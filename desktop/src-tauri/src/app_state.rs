use crate::native_runtime::{NativeDesktopRuntime, TestSeams};
use crate::native_update::UpdateService;
use sky_native_adapters::AppPaths;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// Single non-realtime admission authority for operations that can own the
/// physical input/calibration boundary.
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
    update_installing: bool,
    closing: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActivityReservationError {
    Closing,
    CalibrationAlreadyActive,
    PhysicalPlaybackActive,
    UpdateAlreadyActive,
}

impl std::fmt::Display for ActivityReservationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Closing => "desktop application is closing",
            Self::CalibrationAlreadyActive => "a calibration operation is already active",
            Self::PhysicalPlaybackActive => "physical playback is active",
            Self::UpdateAlreadyActive => "an update installation is already active",
        };
        formatter.write_str(message)
    }
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
    ) -> Result<PhysicalActivityLease, ActivityReservationError> {
        let session_id = session_id.into();
        let mut state = self
            .state
            .lock()
            .map_err(|_| ActivityReservationError::Closing)?;
        if state.closing {
            return Err(ActivityReservationError::Closing);
        }
        if state.calibration_active {
            return Err(ActivityReservationError::CalibrationAlreadyActive);
        }
        if state.update_installing {
            return Err(ActivityReservationError::UpdateAlreadyActive);
        }
        if state.physical_playback.is_some() {
            return Err(ActivityReservationError::PhysicalPlaybackActive);
        }
        state.physical_playback = Some(session_id.clone());
        Ok(PhysicalActivityLease {
            coordinator: self.clone(),
            session_id,
        })
    }

    pub(crate) fn reserve_calibration(
        &self,
    ) -> Result<CalibrationReservation, ActivityReservationError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ActivityReservationError::Closing)?;
        if state.closing {
            return Err(ActivityReservationError::Closing);
        }
        if state.calibration_active {
            return Err(ActivityReservationError::CalibrationAlreadyActive);
        }
        if state.physical_playback.is_some() {
            return Err(ActivityReservationError::PhysicalPlaybackActive);
        }
        if state.update_installing {
            return Err(ActivityReservationError::UpdateAlreadyActive);
        }
        state.calibration_active = true;
        Ok(CalibrationReservation {
            coordinator: self.clone(),
        })
    }

    pub(crate) fn reserve_update(&self) -> Result<UpdateInstallLease, ActivityReservationError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ActivityReservationError::Closing)?;
        if state.closing {
            return Err(ActivityReservationError::Closing);
        }
        if state.physical_playback.is_some() {
            return Err(ActivityReservationError::PhysicalPlaybackActive);
        }
        if state.calibration_active {
            return Err(ActivityReservationError::CalibrationAlreadyActive);
        }
        if state.update_installing {
            return Err(ActivityReservationError::UpdateAlreadyActive);
        }
        state.update_installing = true;
        Ok(UpdateInstallLease {
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

    fn release_update(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.update_installing = false;
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

pub(crate) struct UpdateInstallLease {
    coordinator: ActivityCoordinator,
}

impl Drop for UpdateInstallLease {
    fn drop(&mut self) {
        self.coordinator.release_update();
    }
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
    native: Mutex<Option<Arc<NativeDesktopRuntime>>>,
    update_service: Mutex<Option<Arc<UpdateService<crate::ShellRuntime>>>>,
    settings_writes: Mutex<()>,
    coherence: Mutex<()>,
    activity: ActivityCoordinator,
    test_seams: Mutex<TestSeams>,
    #[cfg(any(test, feature = "tauri-test"))]
    paths_override: Mutex<Option<AppPaths>>,
    closing: AtomicBool,
    gui_smoke_exit: AtomicBool,
    gui_smoke_failed: AtomicBool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            inner: Arc::new(AppStateInner {
                native: Mutex::new(None),
                update_service: Mutex::new(None),
                settings_writes: Mutex::new(()),
                coherence: Mutex::new(()),
                activity: ActivityCoordinator::default(),
                test_seams: Mutex::new(TestSeams::Disabled),
                #[cfg(any(test, feature = "tauri-test"))]
                paths_override: Mutex::new(None),
                closing: AtomicBool::new(false),
                gui_smoke_exit: AtomicBool::new(false),
                gui_smoke_failed: AtomicBool::new(false),
            }),
        }
    }
}

impl AppState {
    /// Lazily construct the single native application runtime.
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
        #[cfg(any(test, feature = "tauri-test"))]
        let paths = self
            .inner
            .paths_override
            .lock()
            .map_err(|_| "native paths override state poisoned".to_string())?
            .clone();
        #[cfg(not(any(test, feature = "tauri-test")))]
        let paths = None;
        let paths = match paths {
            Some(p) => p,
            None => sky_native_adapters::AppPaths::resolve()?,
        };
        let runtime = Arc::new(
            NativeDesktopRuntime::from_paths_with_activity_and_seams_and_update_service(
                paths,
                self.activity(),
                test_seams,
                self.update_service()
                    .map_err(|_| "native update service state poisoned".to_string())?,
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

    pub(crate) fn configure_update_service(
        &self,
        app_handle: tauri::AppHandle<crate::ShellRuntime>,
    ) -> Result<(), String> {
        let service = Arc::new(UpdateService::new(app_handle, self.activity()));
        *self
            .inner
            .update_service
            .lock()
            .map_err(|_| "native update service state poisoned".to_string())? = Some(service);
        Ok(())
    }

    pub(crate) fn update_service(
        &self,
    ) -> Result<Option<Arc<UpdateService<crate::ShellRuntime>>>, String> {
        Ok(self
            .inner
            .update_service
            .lock()
            .map_err(|_| "native update service state poisoned".to_string())?
            .clone())
    }

    /// Serialize settings writes and playback starts at the application
    /// coherence boundary.
    pub(crate) fn lock_coherence(&self) -> MutexGuard<'_, ()> {
        self.inner
            .coherence
            .lock()
            .expect("coherence gate poisoned")
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

    #[cfg(any(test, feature = "tauri-test"))]
    #[allow(dead_code)]
    pub(crate) fn with_test_paths(paths: AppPaths) -> Self {
        let state = Self::default();
        *state
            .inner
            .paths_override
            .lock()
            .expect("test paths state poisoned") = Some(paths);
        state
    }

    #[cfg(any(test, feature = "tauri-test"))]
    #[allow(dead_code)]
    pub(crate) fn with_test_install_root(install_root: PathBuf) -> Self {
        let paths = AppPaths::from_app_data_root(install_root.clone(), install_root);
        Self::with_test_paths(paths)
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
}

#[cfg(test)]
mod tests {
    use super::{ActivityCoordinator, ActivityReservationError, AppState};
    use std::sync::{Arc, Barrier, Mutex, mpsc};
    use std::thread;

    #[test]
    fn close_transition_is_idempotent() {
        let state = AppState::default();
        assert!(state.begin_close());
        assert!(!state.begin_close());
        assert!(state.is_closing());
    }

    #[test]
    fn physical_playback_and_calibration_are_one_atomic_exclusion_gate() {
        let state = AppState::default();
        let activity = state.activity();
        let playback = activity
            .reserve_playback("session")
            .expect("playback lease");
        assert!(matches!(
            activity.reserve_calibration(),
            Err(ActivityReservationError::PhysicalPlaybackActive)
        ));
        drop(playback);
        let _calibration = activity.reserve_calibration().expect("calibration slot");
        assert!(matches!(
            activity.reserve_playback("other"),
            Err(ActivityReservationError::CalibrationAlreadyActive)
        ));
        activity.release_calibration();
        let playback = activity.reserve_playback("other").expect("released slot");
        drop(playback);
        assert!(!activity.is_calibration_active());
    }

    #[test]
    fn update_installation_is_rejected_while_physical_playback_is_active() {
        let activity = ActivityCoordinator::default();
        let _playback = activity
            .reserve_playback("session")
            .expect("playback lease");
        assert!(matches!(
            activity.reserve_update(),
            Err(ActivityReservationError::PhysicalPlaybackActive)
        ));
    }

    #[test]
    fn concurrent_playback_and_calibration_reservation_has_one_winner() {
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
            let playback_result = playback.join().expect("playback thread");
            let calibration_result = calibration.join().expect("calibration thread");
            assert_ne!(playback_result.is_ok(), calibration_result.is_ok());
        }
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
        patch_entered_receiver.recv().expect("patch gate entered");

        let (start_entered_sender, start_entered_receiver) = mpsc::channel();
        let start_state = Arc::clone(&state);
        let start_order = Arc::clone(&order);
        let start = thread::spawn(move || {
            let _guard = start_state.lock_coherence();
            start_entered_sender.send(()).expect("start entered");
            start_order.lock().expect("order").push("playback.start");
        });
        assert!(start_entered_receiver.try_recv().is_err());
        release_sender.send(()).expect("release patch");
        start_entered_receiver.recv().expect("start gate entered");
        patch.join().expect("patch thread");
        start.join().expect("start thread");
        assert_eq!(
            *order.lock().expect("order"),
            vec!["settings.patch", "playback.start"]
        );
    }
}
