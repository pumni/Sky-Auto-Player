#[cfg(test)]
use crate::core::CoreSupervisor;
#[cfg(test)]
use crate::core::protocol::CoreEvent;
#[cfg(test)]
use crate::core::supervisor::CoreEventObserver;
use crate::native_runtime::{NativeDesktopRuntime, TestSeams};
#[cfg(test)]
use crate::ui_events::CalibrationOutcome;
#[cfg(test)]
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(test)]
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};

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

    #[cfg(test)]
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
}

impl Drop for CalibrationReservation {
    fn drop(&mut self) {
        self.coordinator.release_calibration();
    }
}

#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    #[cfg(test)]
    core: Mutex<CoreState>,
    native: Mutex<Option<Arc<NativeDesktopRuntime>>>,
    settings_writes: Mutex<()>,
    coherence: Mutex<()>,
    #[cfg(test)]
    core_event_route: Mutex<CoreEventRoute>,
    activity: ActivityCoordinator,
    test_seams: Mutex<TestSeams>,
    closing: AtomicBool,
    gui_smoke_exit: AtomicBool,
    gui_smoke_failed: AtomicBool,
}

#[derive(Default)]
#[cfg(test)]
struct CoreEventRoute {
    phase: CoreEventRoutePhase,
    queue: VecDeque<crate::ui_events::UiEvent>,
    draining: bool,
}

#[derive(Default)]
#[cfg(test)]
enum CoreEventRoutePhase {
    #[default]
    Pending,
    Transitioning,
    Live(Arc<NativeDesktopRuntime>),
    Failed,
}

#[cfg(test)]
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
                #[cfg(test)]
                core: Mutex::new(CoreState::Idle),
                native: Mutex::new(None),
                settings_writes: Mutex::new(()),
                coherence: Mutex::new(()),
                #[cfg(test)]
                core_event_route: Mutex::new(CoreEventRoute::default()),
                activity: ActivityCoordinator {
                    state: Arc::new(Mutex::new(ActivityState::default())),
                },
                test_seams: Mutex::new(TestSeams::Disabled),
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
        let test_seams = *self
            .inner
            .test_seams
            .lock()
            .map_err(|_| "native test-seam state poisoned".to_string())?;
        let runtime = Arc::new(
            NativeDesktopRuntime::from_install_root_with_activity_and_seams(
                crate::native_runtime::resolve_install_root()?,
                self.activity(),
                test_seams,
            )?,
        );
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

    /// Serialize settings writes and playback starts at the application
    /// coherence boundary.  The same guard is used by both production Native
    /// command routes, so a start cannot consume a stale prepared plan.
    pub(crate) fn lock_coherence(&self) -> MutexGuard<'_, ()> {
        self.inner
            .coherence
            .lock()
            .expect("coherence gate poisoned")
    }

    #[cfg(test)]
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
        self.queue_core_event(event.clone().into_ui_event())
    }

    #[cfg(test)]
    pub(crate) fn begin_core_event_route_transition(&self) -> Result<(), String> {
        let mut route = self
            .inner
            .core_event_route
            .lock()
            .map_err(|_| "Core event route poisoned".to_string())?;
        match &route.phase {
            CoreEventRoutePhase::Pending => {
                route.phase = CoreEventRoutePhase::Transitioning;
                Ok(())
            }
            CoreEventRoutePhase::Transitioning => {
                Err("Core event route transition already in progress".into())
            }
            CoreEventRoutePhase::Live(_) => Err("Core event route is already live".into()),
            CoreEventRoutePhase::Failed => Err("Core event route is failed closed".into()),
        }
    }

    #[cfg(test)]
    pub(crate) fn complete_core_event_route_transition(
        &self,
        native: Arc<NativeDesktopRuntime>,
    ) -> Result<(), String> {
        let should_drain = {
            let mut route = self
                .inner
                .core_event_route
                .lock()
                .map_err(|_| "Core event route poisoned".to_string())?;
            if !matches!(&route.phase, CoreEventRoutePhase::Transitioning) {
                return Err("Core event route was not transitioning".into());
            }
            route.phase = CoreEventRoutePhase::Live(native);
            if route.queue.is_empty() || route.draining {
                false
            } else {
                route.draining = true;
                true
            }
        };
        if should_drain {
            self.drain_core_event_route()?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn fail_core_event_route(&self) {
        if let Ok(mut route) = self.inner.core_event_route.lock() {
            route.phase = CoreEventRoutePhase::Failed;
            route.queue.clear();
            route.draining = false;
        }
    }

    #[cfg(test)]
    fn queue_core_event(&self, event: crate::ui_events::UiEvent) -> Result<(), String> {
        const MAX_CORE_ROUTE_EVENTS: usize = 128;
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
        let should_drain = {
            let mut route = self
                .inner
                .core_event_route
                .lock()
                .map_err(|_| "Core event route poisoned".to_string())?;
            if matches!(&route.phase, CoreEventRoutePhase::Failed) {
                return Err("unified UI event delivery failed".into());
            }
            if let Some(key) = snapshot_key {
                if let Some(index) = route.queue.iter().position(|candidate| {
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
                    route.queue[index] = event;
                    false
                } else {
                    if route.queue.len() >= MAX_CORE_ROUTE_EVENTS
                        && let Some(index) = route.queue.iter().position(|candidate| {
                            matches!(
                                candidate,
                                crate::ui_events::UiEvent::PlaybackSnapshot { .. }
                                    | crate::ui_events::UiEvent::DiagnosticsSnapshot { .. }
                                    | crate::ui_events::UiEvent::CalibrationProgress { .. }
                            )
                        })
                    {
                        route.queue.remove(index);
                    }
                    if route.queue.len() >= MAX_CORE_ROUTE_EVENTS {
                        route.phase = CoreEventRoutePhase::Failed;
                        return Err("Core lifecycle event buffer overflow".into());
                    }
                    route.queue.push_back(event);
                    if matches!(&route.phase, CoreEventRoutePhase::Live(_)) && !route.draining {
                        route.draining = true;
                        true
                    } else {
                        false
                    }
                }
            } else {
                if route.queue.len() >= MAX_CORE_ROUTE_EVENTS
                    && let Some(index) = route.queue.iter().position(|candidate| {
                        matches!(
                            candidate,
                            crate::ui_events::UiEvent::PlaybackSnapshot { .. }
                                | crate::ui_events::UiEvent::DiagnosticsSnapshot { .. }
                                | crate::ui_events::UiEvent::CalibrationProgress { .. }
                        )
                    })
                {
                    route.queue.remove(index);
                }
                if route.queue.len() >= MAX_CORE_ROUTE_EVENTS {
                    route.phase = CoreEventRoutePhase::Failed;
                    return Err("Core lifecycle event buffer overflow".into());
                }
                route.queue.push_back(event);
                if matches!(&route.phase, CoreEventRoutePhase::Live(_)) && !route.draining {
                    route.draining = true;
                    true
                } else {
                    false
                }
            }
        };
        if should_drain {
            self.drain_core_event_route()?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn drain_core_event_route(&self) -> Result<(), String> {
        loop {
            let next = {
                let mut route = self
                    .inner
                    .core_event_route
                    .lock()
                    .map_err(|_| "Core event route poisoned".to_string())?;
                let CoreEventRoutePhase::Live(native) = &route.phase else {
                    route.draining = false;
                    return Ok(());
                };
                let native = Arc::clone(native);
                let Some(event) = route.queue.pop_front() else {
                    route.draining = false;
                    return Ok(());
                };
                (native, event)
            };
            if next.0.relay_core_event(next.1).is_err() {
                self.fail_core_event_route();
                self.inner.activity.release_calibration();
                return Err("unified UI event delivery failed".into());
            }
        }
    }

    #[cfg(test)]
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
                let observer: CoreEventObserver = Arc::new(move |event: &CoreEvent| {
                    if let Some(inner) = weak_inner.upgrade() {
                        let state = AppState { inner };
                        state.observe_core_event(event)
                    } else {
                        Err("desktop state was dropped".into())
                    }
                });
                if let Err(error) = supervisor.install_event_observer_and_drain(observer) {
                    supervisor.shutdown();
                    return Err(error);
                }
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

    pub(crate) fn with_test_seams(test_seams: TestSeams) -> Self {
        let state = Self::default();
        *state
            .inner
            .test_seams
            .lock()
            .expect("test-seam state poisoned") = test_seams;
        state
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
    use crate::native_runtime::NativeDesktopRuntime;
    use crate::ui_events::{CalibrationFinishedPayload, CalibrationOutcome, CatalogChangedPayload};
    use serde_json::Value;
    use std::fs;
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        let _calibration = activity.reserve_calibration().expect("calibration slot");
        assert!(activity.reserve_playback("other").is_err());
        activity.release_calibration();
        let playback = activity.reserve_playback("other").expect("released slot");
        drop(playback);
        assert!(!activity.is_calibration_active());
    }

    #[test]
    fn terminal_core_calibration_event_releases_the_shared_activity_slot() {
        let state = AppState::default();
        let _reservation = state.activity().reserve_calibration().expect("slot");
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

    #[test]
    fn core_route_replays_backlog_before_transition_and_live_events() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-core-route-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        let native =
            Arc::new(NativeDesktopRuntime::from_install_root(root.clone()).expect("native"));
        let state = AppState::default();

        let catalog_event = |generation| {
            CoreEvent::CatalogChanged(CatalogChangedPayload {
                generation,
                total: generation,
            })
        };
        // Event one is the pre-observer/backlog equivalent. Event two arrives
        // after the route enters Transitioning, and event three is live.
        state
            .observe_core_event(&catalog_event(1))
            .expect("backlog event");
        state
            .begin_core_event_route_transition()
            .expect("begin route transition");
        state
            .observe_core_event(&catalog_event(2))
            .expect("transition event");

        let delivered = Arc::new(Mutex::new(Vec::new()));
        let delivered_for_channel = Arc::clone(&delivered);
        let channel = tauri::ipc::Channel::<crate::ui_events::UiEvent>::new(move |body| {
            let raw = match body {
                tauri::ipc::InvokeResponseBody::Json(raw) => raw,
                tauri::ipc::InvokeResponseBody::Raw(raw) => {
                    String::from_utf8(raw).map_err(|error| tauri::Error::Anyhow(error.into()))?
                }
            };
            let value: Value =
                serde_json::from_str(&raw).map_err(|error| tauri::Error::Anyhow(error.into()))?;
            delivered_for_channel
                .lock()
                .expect("delivered events")
                .push(value["payload"]["generation"].as_u64().expect("generation"));
            Ok(())
        });
        native.subscribe(channel).expect("native subscription");

        let complete_state = state.clone();
        let complete_native = Arc::clone(&native);
        let complete = thread::spawn(move || {
            complete_state.complete_core_event_route_transition(complete_native)
        });
        complete
            .join()
            .expect("route transition thread")
            .expect("route complete");
        state
            .observe_core_event(&catalog_event(3))
            .expect("live event");

        assert_eq!(*delivered.lock().expect("delivered events"), vec![1, 2, 3]);
        native.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn core_route_serializes_live_delivery_while_backlog_drain_is_in_flight() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-core-route-drain-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        let native =
            Arc::new(NativeDesktopRuntime::from_install_root(root.clone()).expect("native"));
        let state = AppState::default();
        let catalog_event = |generation| {
            CoreEvent::CatalogChanged(CatalogChangedPayload {
                generation,
                total: generation,
            })
        };
        state
            .observe_core_event(&catalog_event(1))
            .expect("backlog event");
        state
            .begin_core_event_route_transition()
            .expect("begin route transition");

        let delivered = Arc::new(Mutex::new(Vec::new()));
        let delivered_for_channel = Arc::clone(&delivered);
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let release_receiver = Arc::new(Mutex::new(release_receiver));
        let release_receiver_for_channel = Arc::clone(&release_receiver);
        let channel = tauri::ipc::Channel::<crate::ui_events::UiEvent>::new(move |body| {
            let raw = match body {
                tauri::ipc::InvokeResponseBody::Json(raw) => raw,
                tauri::ipc::InvokeResponseBody::Raw(raw) => {
                    String::from_utf8(raw).map_err(|error| tauri::Error::Anyhow(error.into()))?
                }
            };
            let value: Value =
                serde_json::from_str(&raw).map_err(|error| tauri::Error::Anyhow(error.into()))?;
            delivered_for_channel
                .lock()
                .expect("delivered events")
                .push(value["payload"]["generation"].as_u64().expect("generation"));
            if value["payload"]["generation"] == 1 {
                entered_sender.send(()).expect("entered receiver");
                release_receiver_for_channel
                    .lock()
                    .expect("release receiver")
                    .recv()
                    .expect("release sender");
            }
            Ok(())
        });
        native.subscribe(channel).expect("native subscription");

        let complete_state = state.clone();
        let complete_native = Arc::clone(&native);
        let complete = thread::spawn(move || {
            complete_state.complete_core_event_route_transition(complete_native)
        });
        entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("backlog drain entered");

        let live_state = state.clone();
        let live = thread::spawn(move || live_state.observe_core_event(&catalog_event(2)));
        // The route drain owns delivery of event one, but the route lock is
        // released before invoking the Channel. Event two can therefore join
        // the ordered queue without overtaking event one.
        live.join().expect("live event thread").expect("live event");
        release_sender.send(()).expect("release drain");
        complete
            .join()
            .expect("route transition thread")
            .expect("route complete");
        assert_eq!(*delivered.lock().expect("delivered events"), vec![1, 2]);
        native.shutdown();
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn settings_patch_and_native_start_share_one_linearization_gate() {
        let state = Arc::new(AppState::default());
        let order = Arc::new(Mutex::new(Vec::new()));
        let (patch_entered_sender, patch_entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let patch_state = Arc::clone(&state);
        let patch_order = Arc::clone(&order);
        let patch = thread::spawn(move || {
            let _guard = patch_state.lock_coherence();
            patch_entered_sender.send(()).expect("patch entered");
            release_receiver.recv().expect("release patch");
            patch_order.lock().expect("order").push("settings.patch");
        });
        patch_entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("patch gate entered");

        let (start_entered_sender, start_entered_receiver) = mpsc::channel();
        let start_state = Arc::clone(&state);
        let start_order = Arc::clone(&order);
        let start = thread::spawn(move || {
            let _guard = start_state.lock_coherence();
            start_entered_sender.send(()).expect("start entered");
            start_order.lock().expect("order").push("playback.start");
        });
        assert!(
            start_entered_receiver
                .recv_timeout(Duration::from_millis(50))
                .is_err()
        );
        release_sender.send(()).expect("release patch");
        start_entered_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("start gate entered");
        patch.join().expect("patch thread");
        start.join().expect("start thread");
        assert_eq!(
            *order.lock().expect("order"),
            vec!["settings.patch", "playback.start"]
        );
    }

    #[test]
    fn calibration_success_invalidates_native_prepared_plan_before_release() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-calibration-invalidation-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        fs::write(
            root.join("songs/demo.json"),
            r#"{"name":"Demo","songNotes":[{"time":0,"key":"Key0"}]}"#,
        )
        .expect("song");
        let native =
            Arc::new(NativeDesktopRuntime::from_install_root(root.clone()).expect("native"));
        let state = AppState::default();
        *state.inner.native.lock().expect("native state") = Some(Arc::clone(&native));
        let bootstrap = native.bootstrap().expect("bootstrap");
        let search: crate::commands::CatalogSearchDto = serde_json::from_value(
            native
                .dispatch(
                    "catalog.search",
                    serde_json::json!({
                        "query": "",
                        "offset": 0,
                        "limit": 10,
                        "generation": bootstrap.catalog_generation
                    }),
                )
                .expect("search"),
        )
        .expect("search DTO");
        let prepared = native
            .dispatch(
                "playback.prepare",
                serde_json::json!({
                    "songId": search.items[0].song_id,
                    "generation": bootstrap.catalog_generation,
                    "config": {"hold_frames":1.0,"tempo_scale":1.0,"fps":60,"dry_run":true}
                }),
            )
            .expect("prepare");
        let prepared_id = prepared["prepared_id"]
            .as_str()
            .expect("prepared ID")
            .to_owned();
        let _reservation = state
            .activity()
            .reserve_calibration()
            .expect("calibration slot");
        state
            .observe_core_event(&CoreEvent::CalibrationFinished(
                CalibrationFinishedPayload {
                    operation_id: "a".repeat(32),
                    outcome: CalibrationOutcome::Succeeded,
                    status: "succeeded".into(),
                    margin_us: Some(777),
                    sample_count: 10,
                    source: "fixture".into(),
                    message: "published".into(),
                    applied: true,
                },
            ))
            .expect("calibration event");
        assert!(!state.activity().is_calibration_active());
        assert!(
            native
                .dispatch(
                    "playback.start",
                    serde_json::json!({"preparedId":prepared_id,"decisions":[]}),
                )
                .is_err()
        );
        native.shutdown();
        let _ = fs::remove_dir_all(root);
    }
}
