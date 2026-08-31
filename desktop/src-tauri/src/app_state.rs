use crate::core::CoreSupervisor;
use crate::core::protocol::CoreEvent;
use crate::native_runtime::NativeDesktopRuntime;
use crate::ui_events::CalibrationOutcome;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};

/// Single non-realtime admission authority for operations that can own the
/// physical input/calibration boundary.  The Python Core and native runtime
/// are separate processes/owners during the strangler phase, so two local
/// booleans cannot safely implement this contract.
#[derive(Clone)]
pub(crate) struct ActivityCoordinator {
    state: Arc<Mutex<ActivityState>>,
}

impl Default for ActivityCoordinator {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(ActivityState::default())),
        }
    }
}

#[derive(Default)]
struct ActivityState {
    physical_playback: Option<String>,
    calibration_active: bool,
    closing: bool,
}

pub(crate) struct PhysicalActivityLease {
    coordinator: ActivityCoordinator,
    session_id: String,
}

impl Drop for PhysicalActivityLease {
    fn drop(&mut self) {
        self.coordinator.release_playback(&self.session_id);
    }
}

impl ActivityCoordinator {
    pub(crate) fn reserve_playback(
        &self,
        session_id: impl Into<String>,
    ) -> Result<PhysicalActivityLease, String> {
        let session_id = session_id.into();
        let mut state = self
            .state
            .lock()
            .map_err(|_| "activity gate poisoned".to_string())?;
        if state.closing {
            return Err("desktop application is closing".into());
        }
        if state.calibration_active {
            return Err("calibration is active".into());
        }
        if state.physical_playback.is_some() {
            return Err("another physical playback session is active".into());
        }
        state.physical_playback = Some(session_id.clone());
        Ok(PhysicalActivityLease {
            coordinator: self.clone(),
            session_id,
        })
    }

    pub(crate) fn reserve_calibration(&self) -> Result<CalibrationReservation, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "activity gate poisoned".to_string())?;
        if state.closing {
            return Err("desktop application is closing".into());
        }
        if state.calibration_active {
            return Err("a calibration operation is already active".into());
        }
        if state.physical_playback.is_some() {
            return Err("calibration cannot run during physical playback".into());
        }
        state.calibration_active = true;
        Ok(CalibrationReservation {
            coordinator: self.clone(),
            committed: false,
        })
    }

    fn release_playback(&self, session_id: &str) {
        if let Ok(mut state) = self.state.lock()
            && state.physical_playback.as_deref() == Some(session_id)
        {
            state.physical_playback = None;
        }
    }

    pub(crate) fn release_calibration(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.calibration_active = false;
        }
    }

    pub(crate) fn observe_core_event(&self, event: &CoreEvent) {
        match event {
            CoreEvent::CalibrationFinished(_) => self.release_calibration(),
            CoreEvent::Fatal(_) => self.release_calibration(),
            _ => {}
        }
    }

    pub(crate) fn begin_shutdown(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closing = true;
            state.physical_playback = None;
            state.calibration_active = false;
        }
    }

    #[cfg(test)]
    pub(crate) fn is_calibration_active(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.calibration_active)
            .unwrap_or(true)
    }
}

pub(crate) struct CalibrationReservation {
    coordinator: ActivityCoordinator,
    committed: bool,
}

impl CalibrationReservation {
    pub(crate) fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for CalibrationReservation {
    fn drop(&mut self) {
        if !self.committed {
            self.coordinator.release_calibration();
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    core: Mutex<CoreState>,
    native: Mutex<Option<Arc<NativeDesktopRuntime>>>,
    settings_writes: Mutex<()>,
    coherence: Mutex<()>,
    pending_core_events: Mutex<VecDeque<crate::ui_events::UiEvent>>,
    activity: ActivityCoordinator,
    closing: AtomicBool,
    gui_smoke_exit: AtomicBool,
    gui_smoke_failed: AtomicBool,
}

enum CoreState {
    Idle,
    Starting(Vec<mpsc::Sender<Result<Arc<CoreSupervisor>, String>>>),
    Ready(Arc<CoreSupervisor>),
    Failed(String),
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                core: Mutex::new(CoreState::Idle),
                native: Mutex::new(None),
                settings_writes: Mutex::new(()),
                coherence: Mutex::new(()),
                pending_core_events: Mutex::new(VecDeque::new()),
                activity: ActivityCoordinator {
                    state: Arc::new(Mutex::new(ActivityState::default())),
                },
                closing: AtomicBool::new(false),
                gui_smoke_exit: AtomicBool::new(false),
                gui_smoke_failed: AtomicBool::new(false),
            }),
        }
    }
}

impl AppState {
    /// Lazily construct the single native application runtime.  Production
    /// commands use this path; Core startup remains available only for the
    /// transitional Python-owned/test protocol surface.
    pub(crate) fn ensure_native_blocking(&self) -> Result<Arc<NativeDesktopRuntime>, String> {
        if self.is_closing() {
            return Err("Desktop application is closing".into());
        }
        let mut native = self.inner.native.lock().expect("native state poisoned");
        if let Some(runtime) = &*native {
            return Ok(Arc::clone(runtime));
        }
        let runtime = Arc::new(NativeDesktopRuntime::for_current_install_with_activity(
            self.activity(),
        )?);
        *native = Some(Arc::clone(&runtime));
        Ok(runtime)
    }

    pub fn shutdown_native(&self) {
        if let Ok(native) = self.inner.native.lock()
            && let Some(runtime) = &*native
        {
            let _ = runtime.dispatch("app.shutdown", serde_json::json!({}));
        }
    }

    pub(crate) fn activity(&self) -> ActivityCoordinator {
        self.inner.activity.clone()
    }

    pub(crate) fn native_if_present(&self) -> Option<Arc<NativeDesktopRuntime>> {
        self.inner
            .native
            .lock()
            .ok()
            .and_then(|runtime| runtime.as_ref().cloned())
    }

    /// Complete the cross-process settings mutation barrier after the Python
    /// owner has committed its atomic config write.  The caller holds the
    /// shared coherence lock, so a Native start cannot observe the write
    /// between the Python response and this invalidation.
    pub(crate) fn invalidate_native_after_python_settings_patch(&self) {
        if let Some(native) = self.native_if_present() {
            native.invalidate_prepared_for_external_mutation();
        }
    }

    pub(crate) fn lock_coherence(&self) -> MutexGuard<'_, ()> {
        self.inner
            .coherence
            .lock()
            .expect("coherence gate poisoned")
    }

    pub(crate) fn observe_core_event(&self, event: &CoreEvent) -> Result<(), String> {
        // Terminal calibration events are the linearization point between the
        // Python-owned calibration operation and a Native playback start.  Do
        // not release the activity slot first and invalidate prepared plans
        // later: a waiting start could otherwise acquire the slot and consume
        // the old plan in that gap.  The same non-realtime coherence gate is
        // used by settings.patch/playback.start.
        let calibration_terminal = matches!(event, CoreEvent::CalibrationFinished(_));
        let _coherence_guard = calibration_terminal.then(|| self.lock_coherence());
        let calibration_succeeded = matches!(
            event,
            CoreEvent::CalibrationFinished(payload)
                if payload.outcome == CalibrationOutcome::Succeeded
        );
        // Invalidate first, then release the shared slot.  Native playback
        // start is serialized by the same coherence gate, so no caller can
        // acquire the released slot while an old prepared plan is still
        // usable.
        if calibration_succeeded
            && let Ok(native) = self.inner.native.lock()
            && let Some(runtime) = &*native
        {
            runtime.invalidate_prepared_for_calibration();
        }
        self.inner.activity.observe_core_event(event);
        let ui_event = event.clone().into_ui_event();
        if let Some(runtime) = self.native_if_present() {
            if runtime.relay_core_event(ui_event).is_err() {
                // A failed UI delivery must not leave the cross-owner
                // calibration reservation held forever.  The native hub has
                // already entered its fail-closed state.
                self.inner.activity.release_calibration();
                return Err("unified UI event delivery failed".into());
            }
        } else {
            self.queue_core_event(ui_event)?;
        }
        Ok(())
    }

    pub(crate) fn take_pending_core_events(
        &self,
    ) -> Result<Vec<crate::ui_events::UiEvent>, String> {
        let mut pending = self
            .inner
            .pending_core_events
            .lock()
            .map_err(|_| "pending Core event queue poisoned".to_string())?;
        Ok(pending.drain(..).collect())
    }

    fn queue_core_event(&self, event: crate::ui_events::UiEvent) -> Result<(), String> {
        const MAX_PENDING_CORE_EVENTS: usize = 128;
        let snapshot_key = match &event {
            crate::ui_events::UiEvent::PlaybackSnapshot { payload, .. } => {
                Some((1_u8, payload.session_id.clone()))
            }
            crate::ui_events::UiEvent::DiagnosticsSnapshot { payload, .. } => {
                Some((2_u8, payload.session_id.clone().unwrap_or_default()))
            }
            crate::ui_events::UiEvent::CalibrationProgress { payload, .. } => {
                Some((3_u8, payload.operation_id.clone()))
            }
            _ => None,
        };
        let mut pending = self
            .inner
            .pending_core_events
            .lock()
            .map_err(|_| "pending Core event queue poisoned".to_string())?;
        if let Some(key) = snapshot_key {
            if let Some(index) = pending.iter().position(|candidate| {
                let candidate_key = match candidate {
                    crate::ui_events::UiEvent::PlaybackSnapshot { payload, .. } => {
                        Some((1_u8, payload.session_id.clone()))
                    }
                    crate::ui_events::UiEvent::DiagnosticsSnapshot { payload, .. } => {
                        Some((2_u8, payload.session_id.clone().unwrap_or_default()))
                    }
                    crate::ui_events::UiEvent::CalibrationProgress { payload, .. } => {
                        Some((3_u8, payload.operation_id.clone()))
                    }
                    _ => None,
                };
                candidate_key == Some(key.clone())
            }) {
                pending[index] = event;
                return Ok(());
            }
            if pending.len() >= MAX_PENDING_CORE_EVENTS
                && let Some(index) = pending.iter().position(|candidate| {
                    matches!(
                        candidate,
                        crate::ui_events::UiEvent::PlaybackSnapshot { .. }
                            | crate::ui_events::UiEvent::DiagnosticsSnapshot { .. }
                            | crate::ui_events::UiEvent::CalibrationProgress { .. }
                    )
                })
            {
                pending.remove(index);
            }
        }
        if pending.len() >= MAX_PENDING_CORE_EVENTS {
            return Err("pending Core lifecycle event buffer overflow".into());
        }
        pending.push_back(event);
        Ok(())
    }

    /// Start the Core at most once and let concurrent callers share its result.
    /// The caller must invoke this from a blocking worker because Core startup
    /// waits for the child process readiness event.
    pub fn ensure_core_blocking(&self) -> Result<Arc<CoreSupervisor>, String> {
        self.ensure_core_blocking_with(|| {
            CoreSupervisor::spawn().map_err(|error| error.to_string())
        })
    }

    fn ensure_core_blocking_with<F>(&self, starter: F) -> Result<Arc<CoreSupervisor>, String>
    where
        F: FnOnce() -> Result<Arc<CoreSupervisor>, String>,
    {
        let receiver = {
            let mut state = self.inner.core.lock().expect("desktop state poisoned");
            match &mut *state {
                CoreState::Ready(supervisor) => return Ok(Arc::clone(supervisor)),
                CoreState::Failed(error) => return Err(error.clone()),
                CoreState::Starting(waiters) => {
                    let (sender, receiver) = mpsc::channel();
                    waiters.push(sender);
                    Some(receiver)
                }
                CoreState::Idle => {
                    *state = CoreState::Starting(Vec::new());
                    None
                }
            }
        };

        if let Some(receiver) = receiver {
            return receiver
                .recv()
                .unwrap_or_else(|_| Err("Core startup worker stopped unexpectedly".into()));
        }

        let result = if self.is_closing() {
            Err("Desktop application is closing".into())
        } else {
            starter()
        };
        let result = match result {
            Ok(supervisor) if self.is_closing() => {
                supervisor.shutdown();
                Err("Desktop application is closing".into())
            }
            Ok(supervisor) => {
                let weak_inner = Arc::downgrade(&self.inner);
                supervisor.set_event_observer(Arc::new(move |event| {
                    if let Some(inner) = weak_inner.upgrade() {
                        let state = AppState { inner };
                        state.observe_core_event(event)
                    } else {
                        Err("desktop state was dropped".into())
                    }
                }));
                let weak_inner = Arc::downgrade(&self.inner);
                supervisor.set_failure_observer(Arc::new(move || {
                    if let Some(inner) = weak_inner.upgrade() {
                        // An abrupt Core exit has no trustworthy terminal
                        // calibration event. Release the shared admission
                        // slot so the shell cannot remain permanently locked.
                        inner.activity.release_calibration();
                    }
                }));
                Ok(supervisor)
            }
            Err(error) => Err(error),
        };
        let mut state = self.inner.core.lock().expect("desktop state poisoned");
        let waiters = match std::mem::replace(&mut *state, CoreState::Idle) {
            CoreState::Starting(waiters) => waiters,
            other => {
                *state = other;
                return Err("Core startup state was corrupted".into());
            }
        };
        match &result {
            Ok(supervisor) => *state = CoreState::Ready(Arc::clone(supervisor)),
            Err(error) => *state = CoreState::Failed(error.clone()),
        }
        for waiter in waiters {
            let _ = waiter.send(result.clone());
        }
        result
    }

    pub fn supervisor(&self) -> Result<Arc<CoreSupervisor>, String> {
        match &*self.inner.core.lock().expect("desktop state poisoned") {
            CoreState::Ready(supervisor) => Ok(Arc::clone(supervisor)),
            CoreState::Starting(_) => Err("Desktop Core is still starting".into()),
            CoreState::Idle => Err("Desktop Core has not started".into()),
            CoreState::Failed(error) => Err(error.clone()),
        }
    }

    pub fn lock_settings_writes(&self) -> MutexGuard<'_, ()> {
        self.inner
            .settings_writes
            .lock()
            .expect("settings write queue poisoned")
    }

    pub fn begin_close(&self) -> bool {
        let first = !self.inner.closing.swap(true, Ordering::AcqRel);
        if first {
            self.inner.activity.begin_shutdown();
        }
        first
    }

    pub fn set_gui_smoke_exit(&self, enabled: bool) {
        self.inner.gui_smoke_exit.store(enabled, Ordering::Release);
    }

    pub fn should_exit_after_close(&self) -> bool {
        self.inner.gui_smoke_exit.load(Ordering::Acquire)
    }

    pub fn set_gui_smoke_failed(&self) {
        self.inner.gui_smoke_failed.store(true, Ordering::Release);
    }

    pub fn gui_smoke_exit_code(&self) -> i32 {
        if self.inner.gui_smoke_failed.load(Ordering::Acquire) {
            1
        } else {
            0
        }
    }

    fn is_closing(&self) -> bool {
        self.inner.closing.load(Ordering::Acquire)
    }

    #[cfg(all(test, feature = "tauri-test", not(feature = "desktop-runtime")))]
    pub(crate) fn install_ready_for_test(&self, supervisor: Arc<CoreSupervisor>) {
        *self.inner.core.lock().expect("desktop state poisoned") = CoreState::Ready(supervisor);
    }

    #[cfg(all(test, feature = "tauri-test", not(feature = "desktop-runtime")))]
    pub(crate) fn is_closing_for_test(&self) -> bool {
        self.is_closing()
    }
}

#[cfg(test)]
mod tests {
    use super::{ActivityCoordinator, AppState};
    use crate::core::protocol::CoreEvent;

    #[test]
    fn close_transition_is_idempotent() {
        let state = AppState::default();
        assert!(state.begin_close());
        assert!(!state.begin_close());
        assert!(state.is_closing());
    }

    #[test]
    fn startup_is_single_flight_and_waiters_share_failure() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        let state = Arc::new(AppState::default());
        let barrier = Arc::new(Barrier::new(2));
        let first_state = Arc::clone(&state);
        let first_barrier = Arc::clone(&barrier);
        let first = thread::spawn(move || {
            first_state.ensure_core_blocking_with(|| {
                first_barrier.wait();
                Err("deterministic startup failure".into())
            })
        });
        barrier.wait();
        let second = state.ensure_core_blocking_with(|| {
            panic!("a concurrent caller must not launch a second Core")
        });
        let first_result = first.join().expect("startup worker panicked");
        let first_error = match first_result {
            Ok(_) => panic!("startup must fail"),
            Err(error) => error,
        };
        let second_error = match second {
            Ok(_) => panic!("waiter must observe startup failure"),
            Err(error) => error,
        };
        assert_eq!(first_error, "deterministic startup failure");
        assert_eq!(second_error, "deterministic startup failure");
    }

    #[test]
    fn startup_does_not_launch_when_close_wins_the_race() {
        let state = AppState::default();
        assert!(state.begin_close());
        let result =
            state.ensure_core_blocking_with(|| panic!("closing state must prevent Core launch"));
        let error = match result {
            Ok(_) => panic!("closing state must reject startup"),
            Err(error) => error,
        };
        assert_eq!(error, "Desktop application is closing");
    }

    #[test]
    fn physical_playback_and_calibration_are_one_atomic_exclusion_gate() {
        let state = AppState::default();
        let activity = state.activity();
        let playback = activity
            .reserve_playback("session")
            .expect("playback lease");
        assert!(activity.reserve_calibration().is_err());
        drop(playback);
        let calibration = activity.reserve_calibration().expect("calibration slot");
        assert!(activity.reserve_playback("other").is_err());
        calibration.commit();
        activity.release_calibration();
        let playback = activity.reserve_playback("other").expect("released slot");
        drop(playback);
        assert!(!activity.is_calibration_active());
    }

    #[test]
    fn terminal_core_calibration_event_releases_the_shared_activity_slot() {
        let state = AppState::default();
        let reservation = state.activity().reserve_calibration().expect("slot");
        reservation.commit();
        assert!(state.activity().is_calibration_active());

        state
            .observe_core_event(&CoreEvent::CalibrationFinished(
                crate::ui_events::CalibrationFinishedPayload {
                    operation_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
                    outcome: crate::ui_events::CalibrationOutcome::Failed,
                    status: "failed".into(),
                    margin_us: None,
                    sample_count: 0,
                    source: "none".into(),
                    message: "test failure".into(),
                    applied: false,
                },
            ))
            .expect("event is relayed/queued");
        assert!(!state.activity().is_calibration_active());
    }

    #[test]
    fn concurrent_playback_and_calibration_reservation_has_one_winner() {
        use std::sync::{Arc, Barrier};
        use std::thread;

        for _ in 0..64 {
            let coordinator = ActivityCoordinator::default();
            let barrier = Arc::new(Barrier::new(2));
            let playback_gate = coordinator.clone();
            let calibration_gate = coordinator.clone();
            let playback_barrier = Arc::clone(&barrier);
            let calibration_barrier = Arc::clone(&barrier);
            let playback = thread::spawn(move || {
                playback_barrier.wait();
                playback_gate.reserve_playback("native-session")
            });
            let calibration = thread::spawn(move || {
                calibration_barrier.wait();
                calibration_gate.reserve_calibration()
            });
            // Keep successful reservations alive until both contenders have
            // completed. Dropping the first lease before joining the second
            // would turn this into a sequential test and allow two success
            // results even though the gate itself is atomic.
            let playback_result = playback.join().expect("playback thread");
            let calibration_result = calibration.join().expect("calibration thread");
            let playback_won = playback_result.is_ok();
            let calibration_won = calibration_result.is_ok();
            assert_ne!(playback_won, calibration_won);
        }
    }
}
