use sky_player::engine::NativeDispatchSession;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};

pub(crate) type SuspendResumeCallback =
    unsafe extern "system" fn(*const c_void, u32, *const c_void) -> u32;

pub(crate) trait PowerPlatform: Send + Sync {
    fn create_system_required_request(&self, reason: &str) -> Result<isize, u32>;
    fn set_system_required(&self, handle: isize) -> Result<(), u32>;
    fn clear_system_required(&self, handle: isize) -> Result<(), u32>;
    fn close_request(&self, handle: isize) -> Result<(), u32>;
    fn register_suspend_resume(
        &self,
        callback: SuspendResumeCallback,
        context: *mut c_void,
    ) -> Result<isize, u32>;
    fn unregister_suspend_resume(&self, handle: isize) -> Result<(), u32>;
}

#[derive(Default)]
pub(crate) struct PowerLifecycleDiagnostics {
    request_created: AtomicBool,
    request_active: AtomicBool,
    registration_active: AtomicBool,
    request_create_failures: AtomicU64,
    request_set_failures: AtomicU64,
    request_clear_failures: AtomicU64,
    request_close_failures: AtomicU64,
    registration_failures: AtomicU64,
    unregistration_failures: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct PowerLifecycleSnapshot {
    pub(crate) request_created: bool,
    pub(crate) request_active: bool,
    pub(crate) registration_active: bool,
    pub(crate) request_create_failures: u64,
    pub(crate) request_set_failures: u64,
    pub(crate) request_clear_failures: u64,
    pub(crate) request_close_failures: u64,
    pub(crate) registration_failures: u64,
    pub(crate) unregistration_failures: u64,
}

impl PowerLifecycleDiagnostics {
    pub(crate) fn snapshot(&self) -> PowerLifecycleSnapshot {
        PowerLifecycleSnapshot {
            request_created: self.request_created.load(Ordering::Acquire),
            request_active: self.request_active.load(Ordering::Acquire),
            registration_active: self.registration_active.load(Ordering::Acquire),
            request_create_failures: self.request_create_failures.load(Ordering::Relaxed),
            request_set_failures: self.request_set_failures.load(Ordering::Relaxed),
            request_clear_failures: self.request_clear_failures.load(Ordering::Relaxed),
            request_close_failures: self.request_close_failures.load(Ordering::Relaxed),
            registration_failures: self.registration_failures.load(Ordering::Relaxed),
            unregistration_failures: self.unregistration_failures.load(Ordering::Relaxed),
        }
    }
}

/// Explicit degraded policy: notification registration is mandatory, while a
/// power-request failure is reported and playback continues without keeping
/// the machine awake. A suspend notification still blocks all Down admissions.
const CONTINUE_AFTER_POWER_REQUEST_FAILURE: bool = true;
const POWER_REQUEST_REASON: &str = "Sky Auto Player physical playback";

pub(crate) struct SystemRequiredPowerRequest {
    platform: Arc<dyn PowerPlatform>,
    diagnostics: Arc<PowerLifecycleDiagnostics>,
    handle: Option<isize>,
    set: bool,
}

impl SystemRequiredPowerRequest {
    pub(crate) fn acquire(
        platform: Arc<dyn PowerPlatform>,
        diagnostics: Arc<PowerLifecycleDiagnostics>,
    ) -> Option<Self> {
        let handle = match platform.create_system_required_request(POWER_REQUEST_REASON) {
            Ok(handle) => handle,
            Err(_) => {
                diagnostics
                    .request_create_failures
                    .fetch_add(1, Ordering::Relaxed);
                return None;
            }
        };
        diagnostics.request_created.store(true, Ordering::Release);
        let mut owner = Self {
            platform,
            diagnostics,
            handle: Some(handle),
            set: false,
        };
        match owner.platform.set_system_required(handle) {
            Ok(()) => {
                owner.set = true;
                owner
                    .diagnostics
                    .request_active
                    .store(true, Ordering::Release);
            }
            Err(_) => {
                owner
                    .diagnostics
                    .request_set_failures
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
        Some(owner)
    }

    pub(crate) fn release(&mut self) {
        let Some(handle) = self.handle else {
            return;
        };
        if self.set {
            match self.platform.clear_system_required(handle) {
                Ok(()) => {
                    self.set = false;
                    self.diagnostics
                        .request_active
                        .store(false, Ordering::Release);
                }
                Err(_) => {
                    self.diagnostics
                        .request_clear_failures
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        match self.platform.close_request(handle) {
            Ok(()) => {
                self.handle = None;
                self.set = false;
                self.diagnostics
                    .request_created
                    .store(false, Ordering::Release);
                self.diagnostics
                    .request_active
                    .store(false, Ordering::Release);
            }
            Err(_) => {
                self.diagnostics
                    .request_close_failures
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

impl Drop for SystemRequiredPowerRequest {
    fn drop(&mut self) {
        self.release();
    }
}

struct CallbackContext {
    player: Weak<NativeDispatchSession>,
}

// The Windows unregister API cancels the registration but does not document
// callback rundown. Retain these weak-only contexts for process lifetime so a
// callback already in flight cannot dereference freed memory. The callback
// never takes this lock; retention happens only on the control plane.
static CALLBACK_CONTEXTS: OnceLock<Mutex<Vec<Arc<CallbackContext>>>> = OnceLock::new();

fn retain_callback_context(context: Arc<CallbackContext>) {
    CALLBACK_CONTEXTS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(context);
}

pub(crate) struct SuspendResumeRegistration {
    platform: Arc<dyn PowerPlatform>,
    diagnostics: Arc<PowerLifecycleDiagnostics>,
    handle: Option<isize>,
}

impl SuspendResumeRegistration {
    pub(crate) fn register(
        platform: Arc<dyn PowerPlatform>,
        player: Weak<NativeDispatchSession>,
        diagnostics: Arc<PowerLifecycleDiagnostics>,
    ) -> Result<Self, u32> {
        let context = Arc::new(CallbackContext { player });
        let result = platform
            .register_suspend_resume(on_suspend_resume, Arc::as_ptr(&context) as *mut c_void);
        let handle = match result {
            Ok(handle) => handle,
            Err(error) => {
                diagnostics
                    .registration_failures
                    .fetch_add(1, Ordering::Relaxed);
                return Err(error);
            }
        };
        retain_callback_context(context);
        diagnostics
            .registration_active
            .store(true, Ordering::Release);
        Ok(Self {
            platform,
            diagnostics,
            handle: Some(handle),
        })
    }

    pub(crate) fn unregister(&mut self) {
        let Some(handle) = self.handle else {
            return;
        };
        if self.platform.unregister_suspend_resume(handle).is_ok() {
            self.handle = None;
            self.diagnostics
                .registration_active
                .store(false, Ordering::Release);
        } else {
            self.diagnostics
                .unregistration_failures
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Drop for SuspendResumeRegistration {
    fn drop(&mut self) {
        self.unregister();
    }
}

unsafe extern "system" fn on_suspend_resume(
    context: *const c_void,
    event_type: u32,
    _setting: *const c_void,
) -> u32 {
    if context.is_null() {
        return 0;
    }
    // SAFETY: this pointer is the Arc allocation retained by the successful
    // registration owner until unregistration completes.
    let context = unsafe { &*(context as *const CallbackContext) };
    let Some(player) = context.player.upgrade() else {
        return 0;
    };
    #[cfg(windows)]
    match event_type {
        windows_sys::Win32::UI::WindowsAndMessaging::PBT_APMSUSPEND => {
            player.notify_system_power(true);
        }
        windows_sys::Win32::UI::WindowsAndMessaging::PBT_APMRESUMESUSPEND
        | windows_sys::Win32::UI::WindowsAndMessaging::PBT_APMRESUMEAUTOMATIC => {
            player.notify_system_power(false);
        }
        _ => {}
    }
    #[cfg(not(windows))]
    let _ = (event_type, player);
    0
}

#[cfg(windows)]
struct WindowsPowerPlatform;

#[cfg(windows)]
impl PowerPlatform for WindowsPowerPlatform {
    fn create_system_required_request(&self, reason: &str) -> Result<isize, u32> {
        use windows_sys::Win32::System::Threading::{
            POWER_REQUEST_CONTEXT_SIMPLE_STRING, REASON_CONTEXT, REASON_CONTEXT_0,
        };
        let mut reason_wide = reason.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let context = REASON_CONTEXT {
            Version: 0,
            Flags: POWER_REQUEST_CONTEXT_SIMPLE_STRING,
            Reason: REASON_CONTEXT_0 {
                SimpleReasonString: reason_wide.as_mut_ptr(),
            },
        };
        // SAFETY: PowerCreateRequest copies the supplied reason context during
        // the call; both the context and UTF-16 string remain alive here.
        let handle = unsafe { windows_sys::Win32::System::Power::PowerCreateRequest(&context) };
        if handle.is_null() || handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            Err(unsafe { windows_sys::Win32::Foundation::GetLastError() })
        } else {
            Ok(handle as isize)
        }
    }

    fn set_system_required(&self, handle: isize) -> Result<(), u32> {
        let ok = unsafe {
            windows_sys::Win32::System::Power::PowerSetRequest(
                handle as windows_sys::Win32::Foundation::HANDLE,
                windows_sys::Win32::System::Power::PowerRequestSystemRequired,
            )
        };
        if ok != 0 {
            Ok(())
        } else {
            Err(unsafe { windows_sys::Win32::Foundation::GetLastError() })
        }
    }

    fn clear_system_required(&self, handle: isize) -> Result<(), u32> {
        let ok = unsafe {
            windows_sys::Win32::System::Power::PowerClearRequest(
                handle as windows_sys::Win32::Foundation::HANDLE,
                windows_sys::Win32::System::Power::PowerRequestSystemRequired,
            )
        };
        if ok != 0 {
            Ok(())
        } else {
            Err(unsafe { windows_sys::Win32::Foundation::GetLastError() })
        }
    }

    fn close_request(&self, handle: isize) -> Result<(), u32> {
        let ok = unsafe {
            windows_sys::Win32::Foundation::CloseHandle(
                handle as windows_sys::Win32::Foundation::HANDLE,
            )
        };
        if ok != 0 {
            Ok(())
        } else {
            Err(unsafe { windows_sys::Win32::Foundation::GetLastError() })
        }
    }

    fn register_suspend_resume(
        &self,
        callback: SuspendResumeCallback,
        context: *mut c_void,
    ) -> Result<isize, u32> {
        use windows_sys::Win32::System::Power::DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS;
        let parameters = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(callback),
            Context: context,
        };
        let mut handle = std::ptr::null_mut();
        let result = unsafe {
            windows_sys::Win32::System::Power::PowerRegisterSuspendResumeNotification(
                windows_sys::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_CALLBACK,
                &parameters as *const _ as windows_sys::Win32::Foundation::HANDLE,
                &mut handle,
            )
        };
        if result == windows_sys::Win32::Foundation::ERROR_SUCCESS {
            Ok(handle as isize)
        } else {
            Err(result)
        }
    }

    fn unregister_suspend_resume(&self, handle: isize) -> Result<(), u32> {
        let result = unsafe {
            windows_sys::Win32::System::Power::PowerUnregisterSuspendResumeNotification(handle)
        };
        if result == windows_sys::Win32::Foundation::ERROR_SUCCESS {
            Ok(())
        } else {
            Err(result)
        }
    }
}

#[cfg(not(windows))]
struct WindowsPowerPlatform;

#[cfg(not(windows))]
impl PowerPlatform for WindowsPowerPlatform {
    fn create_system_required_request(&self, _reason: &str) -> Result<isize, u32> {
        Err(50)
    }
    fn set_system_required(&self, _handle: isize) -> Result<(), u32> {
        Err(50)
    }
    fn clear_system_required(&self, _handle: isize) -> Result<(), u32> {
        Err(50)
    }
    fn close_request(&self, _handle: isize) -> Result<(), u32> {
        Err(50)
    }
    fn register_suspend_resume(
        &self,
        _callback: SuspendResumeCallback,
        _context: *mut c_void,
    ) -> Result<isize, u32> {
        Err(50)
    }
    fn unregister_suspend_resume(&self, _handle: isize) -> Result<(), u32> {
        Err(50)
    }
}

pub(crate) fn system_power_platform() -> Arc<dyn PowerPlatform> {
    Arc::new(WindowsPowerPlatform)
}

pub(crate) fn continue_after_power_request_failure() -> bool {
    CONTINUE_AFTER_POWER_REQUEST_FAILURE
}

pub(crate) fn acquire_playback_power_resources(
    player: Option<Weak<NativeDispatchSession>>,
    platform: Arc<dyn PowerPlatform>,
    diagnostics: Arc<PowerLifecycleDiagnostics>,
) -> Result<
    (
        Option<SystemRequiredPowerRequest>,
        Option<SuspendResumeRegistration>,
    ),
    String,
> {
    let Some(player) = player else {
        return Ok((None, None));
    };
    let registration =
        SuspendResumeRegistration::register(platform.clone(), player, diagnostics.clone())
            .map_err(|error| {
                format!("suspend_resume_registration_failed: Windows error {error}")
            })?;
    let request = SystemRequiredPowerRequest::acquire(platform, diagnostics);
    if request.is_none() && !continue_after_power_request_failure() {
        return Err("system_required_power_request_failed".into());
    }
    Ok((request, Some(registration)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakePlatform {
        calls: Mutex<Vec<&'static str>>,
        failures: Mutex<Vec<&'static str>>,
        once_failures: Mutex<Vec<&'static str>>,
        callback_contexts: Mutex<Vec<(SuspendResumeCallback, usize)>>,
    }

    impl FakePlatform {
        fn fail(&self, operation: &'static str) {
            self.failures.lock().unwrap().push(operation);
        }
        fn result(&self, operation: &'static str) -> Result<(), u32> {
            self.calls.lock().unwrap().push(operation);
            let fail_once = {
                let mut failures = self.once_failures.lock().unwrap();
                if let Some(index) = failures.iter().position(|failure| *failure == operation) {
                    failures.remove(index);
                    true
                } else {
                    false
                }
            };
            let fail_always = self.failures.lock().unwrap().contains(&operation);
            if fail_always || fail_once {
                Err(123)
            } else {
                Ok(())
            }
        }
        fn fail_once(&self, operation: &'static str) {
            self.once_failures.lock().unwrap().push(operation);
        }
    }

    impl PowerPlatform for FakePlatform {
        fn create_system_required_request(&self, _reason: &str) -> Result<isize, u32> {
            self.calls.lock().unwrap().push("create");
            if self.failures.lock().unwrap().contains(&"create") {
                Err(123)
            } else {
                Ok(42)
            }
        }
        fn set_system_required(&self, _handle: isize) -> Result<(), u32> {
            self.result("set")
        }
        fn clear_system_required(&self, _handle: isize) -> Result<(), u32> {
            self.result("clear")
        }
        fn close_request(&self, _handle: isize) -> Result<(), u32> {
            self.result("close")
        }
        fn register_suspend_resume(
            &self,
            callback: SuspendResumeCallback,
            context: *mut c_void,
        ) -> Result<isize, u32> {
            self.calls.lock().unwrap().push("register");
            if self.failures.lock().unwrap().contains(&"register") {
                Err(123)
            } else {
                self.callback_contexts
                    .lock()
                    .unwrap()
                    .push((callback, context as usize));
                Ok(7)
            }
        }
        fn unregister_suspend_resume(&self, _handle: isize) -> Result<(), u32> {
            self.result("unregister")
        }
    }

    #[test]
    fn power_request_raii_sets_then_clears_and_closes_once() {
        let platform = Arc::new(FakePlatform::default());
        let diagnostics = Arc::new(PowerLifecycleDiagnostics::default());
        let mut request =
            SystemRequiredPowerRequest::acquire(platform.clone(), diagnostics.clone())
                .expect("power request");
        assert!(diagnostics.snapshot().request_active);
        request.release();
        request.release();
        assert_eq!(
            *platform.calls.lock().unwrap(),
            vec!["create", "set", "clear", "close"]
        );
        let snapshot = diagnostics.snapshot();
        assert!(!snapshot.request_created);
        assert!(!snapshot.request_active);
    }

    #[test]
    fn power_request_failure_points_are_separately_reported_and_cleaned() {
        let create_platform = Arc::new(FakePlatform::default());
        create_platform.fail("create");
        let create_diagnostics = Arc::new(PowerLifecycleDiagnostics::default());
        assert!(
            SystemRequiredPowerRequest::acquire(
                create_platform.clone(),
                create_diagnostics.clone()
            )
            .is_none()
        );
        assert_eq!(create_diagnostics.snapshot().request_create_failures, 1);
        assert_eq!(*create_platform.calls.lock().unwrap(), vec!["create"]);

        let set_platform = Arc::new(FakePlatform::default());
        set_platform.fail("set");
        let set_diagnostics = Arc::new(PowerLifecycleDiagnostics::default());
        let request =
            SystemRequiredPowerRequest::acquire(set_platform.clone(), set_diagnostics.clone())
                .expect("created request is still RAII-owned after set failure");
        drop(request);
        assert_eq!(set_diagnostics.snapshot().request_set_failures, 1);
        assert_eq!(
            *set_platform.calls.lock().unwrap(),
            vec!["create", "set", "close"]
        );

        let release_platform = Arc::new(FakePlatform::default());
        release_platform.fail("clear");
        let release_diagnostics = Arc::new(PowerLifecycleDiagnostics::default());
        drop(
            SystemRequiredPowerRequest::acquire(
                release_platform.clone(),
                release_diagnostics.clone(),
            )
            .expect("power request"),
        );
        let snapshot = release_diagnostics.snapshot();
        assert_eq!(snapshot.request_clear_failures, 1);
        assert_eq!(snapshot.request_close_failures, 0);
        assert!(!snapshot.request_created);
        assert_eq!(
            *release_platform.calls.lock().unwrap(),
            vec!["create", "set", "clear", "close"]
        );
    }

    #[test]
    fn notification_registration_is_raii_and_failure_is_distinct() {
        let platform = Arc::new(FakePlatform::default());
        let diagnostics = Arc::new(PowerLifecycleDiagnostics::default());
        // A dead Weak player is sufficient for registration ownership tests.
        let registration =
            SuspendResumeRegistration::register(platform.clone(), Weak::new(), diagnostics.clone());
        assert!(registration.is_ok());
        drop(registration);
        let (callback, context) = platform.callback_contexts.lock().unwrap()[0];
        // SAFETY: a successful registration's context is retained until
        // process exit, including callbacks that were in flight at unregister.
        assert_eq!(
            unsafe { callback(context as *const c_void, 0, std::ptr::null()) },
            0
        );
        assert_eq!(
            *platform.calls.lock().unwrap(),
            vec!["register", "unregister"]
        );
        assert!(!diagnostics.snapshot().registration_active);

        let failing_platform = Arc::new(FakePlatform::default());
        failing_platform.fail("register");
        let failed_diagnostics = Arc::new(PowerLifecycleDiagnostics::default());
        assert!(
            SuspendResumeRegistration::register(
                failing_platform,
                Weak::new(),
                failed_diagnostics.clone()
            )
            .is_err()
        );
        assert_eq!(failed_diagnostics.snapshot().registration_failures, 1);
    }

    #[test]
    fn dry_run_does_not_register_or_create_a_power_request() {
        let platform = Arc::new(FakePlatform::default());
        let diagnostics = Arc::new(PowerLifecycleDiagnostics::default());

        let (request, registration) =
            acquire_playback_power_resources(None, platform.clone(), diagnostics.clone())
                .expect("dry run has no power resources");

        assert!(request.is_none());
        assert!(registration.is_none());
        assert!(platform.calls.lock().unwrap().is_empty());
        assert_eq!(diagnostics.snapshot(), PowerLifecycleSnapshot::default());
    }

    #[test]
    fn notification_failure_prevents_power_acquisition_and_request_failure_is_explicitly_degraded()
    {
        let registration_platform = Arc::new(FakePlatform::default());
        registration_platform.fail("register");
        let registration_diagnostics = Arc::new(PowerLifecycleDiagnostics::default());
        let result = acquire_playback_power_resources(
            Some(Weak::new()),
            registration_platform.clone(),
            registration_diagnostics.clone(),
        );
        assert!(result.is_err());
        assert_eq!(
            *registration_platform.calls.lock().unwrap(),
            vec!["register"]
        );
        assert_eq!(registration_diagnostics.snapshot().registration_failures, 1);

        let request_platform = Arc::new(FakePlatform::default());
        request_platform.fail("set");
        let request_diagnostics = Arc::new(PowerLifecycleDiagnostics::default());
        let (request, registration) = acquire_playback_power_resources(
            Some(Weak::new()),
            request_platform.clone(),
            request_diagnostics.clone(),
        )
        .expect("documented policy permits physical playback without the power request");
        assert!(continue_after_power_request_failure());
        assert!(request.is_some());
        assert!(registration.is_some());
        assert_eq!(request_diagnostics.snapshot().request_set_failures, 1);
        drop(request);
        drop(registration);
        assert_eq!(
            *request_platform.calls.lock().unwrap(),
            vec!["register", "create", "set", "close", "unregister"]
        );
    }

    #[test]
    fn failed_notification_unregistration_is_reported_and_keeps_callback_state_alive() {
        let platform = Arc::new(FakePlatform::default());
        platform.fail("unregister");
        let diagnostics = Arc::new(PowerLifecycleDiagnostics::default());
        let registration =
            SuspendResumeRegistration::register(platform.clone(), Weak::new(), diagnostics.clone())
                .expect("registration");
        drop(registration);

        let snapshot = diagnostics.snapshot();
        assert!(snapshot.registration_active);
        assert_eq!(snapshot.unregistration_failures, 1);
        assert_eq!(
            *platform.calls.lock().unwrap(),
            vec!["register", "unregister"]
        );
    }

    #[test]
    fn close_and_unregistration_failures_are_retried_during_raii_drop() {
        let request_platform = Arc::new(FakePlatform::default());
        request_platform.fail_once("close");
        let request_diagnostics = Arc::new(PowerLifecycleDiagnostics::default());
        let mut request = SystemRequiredPowerRequest::acquire(
            request_platform.clone(),
            request_diagnostics.clone(),
        )
        .expect("power request");
        request.release();
        assert!(request_diagnostics.snapshot().request_created);
        drop(request);
        assert!(!request_diagnostics.snapshot().request_created);
        assert_eq!(
            *request_platform.calls.lock().unwrap(),
            vec!["create", "set", "clear", "close", "close"]
        );

        let registration_platform = Arc::new(FakePlatform::default());
        registration_platform.fail_once("unregister");
        let registration_diagnostics = Arc::new(PowerLifecycleDiagnostics::default());
        let mut registration = SuspendResumeRegistration::register(
            registration_platform.clone(),
            Weak::new(),
            registration_diagnostics.clone(),
        )
        .expect("registration");
        registration.unregister();
        assert!(registration_diagnostics.snapshot().registration_active);
        drop(registration);
        assert!(!registration_diagnostics.snapshot().registration_active);
        assert_eq!(
            registration_diagnostics.snapshot().unregistration_failures,
            1
        );
        assert_eq!(
            *registration_platform.calls.lock().unwrap(),
            vec!["register", "unregister", "unregister"]
        );
    }
}
