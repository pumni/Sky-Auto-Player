//! Best-effort per-thread EcoQoS opt-out with RAII restoration.

pub struct PowerThrottlingGuard {
    #[cfg(windows)]
    thread: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(windows)]
    previous: Option<windows_sys::Win32::System::Threading::THREAD_POWER_THROTTLING_STATE>,
    active: bool,
}

impl std::fmt::Debug for PowerThrottlingGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PowerThrottlingGuard")
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl PowerThrottlingGuard {
    pub fn disable_current_thread() -> Self {
        #[cfg(windows)]
        {
            use std::ffi::c_void;
            use std::mem::size_of;
            use windows_sys::Win32::System::Threading::{
                GetCurrentThread, GetThreadInformation, SetThreadInformation,
                THREAD_POWER_THROTTLING_CURRENT_VERSION, THREAD_POWER_THROTTLING_EXECUTION_SPEED,
                THREAD_POWER_THROTTLING_STATE, ThreadPowerThrottling,
            };

            // SAFETY: GetCurrentThread returns a pseudo-handle valid for the
            // lifetime of this guard on the current worker thread.
            let thread = unsafe { GetCurrentThread() };
            let mut previous = THREAD_POWER_THROTTLING_STATE {
                Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
                ControlMask: 0,
                StateMask: 0,
            };
            // SAFETY: previous is a correctly sized writable structure and
            // the pseudo-handle refers to the current thread.
            let queried = unsafe {
                GetThreadInformation(
                    thread,
                    ThreadPowerThrottling,
                    (&raw mut previous).cast::<c_void>(),
                    size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
                )
            } != 0;
            let disabled = THREAD_POWER_THROTTLING_STATE {
                Version: THREAD_POWER_THROTTLING_CURRENT_VERSION,
                ControlMask: THREAD_POWER_THROTTLING_EXECUTION_SPEED,
                StateMask: 0,
            };
            // SAFETY: disabled is a correctly sized immutable structure and
            // the pseudo-handle is valid on this worker thread.
            let active = unsafe {
                SetThreadInformation(
                    thread,
                    ThreadPowerThrottling,
                    (&raw const disabled).cast::<c_void>(),
                    size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
                )
            } != 0;
            Self {
                thread,
                previous: queried.then_some(previous),
                active,
            }
        }
        #[cfg(not(windows))]
        {
            Self { active: false }
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

impl Drop for PowerThrottlingGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        if let Some(previous) = self.previous {
            use std::ffi::c_void;
            use std::mem::size_of;
            use windows_sys::Win32::System::Threading::{
                SetThreadInformation, THREAD_POWER_THROTTLING_STATE, ThreadPowerThrottling,
            };
            // SAFETY: restoration runs on the worker thread that created the
            // pseudo-handle and previous is a correctly sized structure.
            unsafe {
                SetThreadInformation(
                    self.thread,
                    ThreadPowerThrottling,
                    (&raw const previous).cast::<c_void>(),
                    size_of::<THREAD_POWER_THROTTLING_STATE>() as u32,
                );
            }
        }
    }
}

#[cfg(all(windows, any(test, feature = "test-support")))]
static LAST_LIVE_RESTORE_FAILED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(all(windows, any(test, feature = "test-support")))]
pub fn last_live_restore_failed_for_test() -> bool {
    LAST_LIVE_RESTORE_FAILED.load(std::sync::atomic::Ordering::Acquire)
}

#[cfg(all(windows, any(test, feature = "test-support")))]
pub fn reset_live_restore_failed_for_test() {
    LAST_LIVE_RESTORE_FAILED.store(false, std::sync::atomic::Ordering::Release);
}

/// Process-level Quality of Service policy for diagnostic measurement and lab evaluation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ProcessQosPolicy {
    /// A · Control: Default process state without explicit power throttling overrides.
    #[default]
    Default,
    /// B · Candidate: Opt-out of timer resolution throttling.
    /// Bit IGNORE_TIMER_RESOLUTION in ControlMask with StateMask = 0.
    IgnoreTimerResolution,
    /// C-QoS: Process-level execution speed throttling opt-out.
    /// Bit EXECUTION_SPEED in ControlMask with StateMask = 0.
    DisableExecutionSpeed,
}

impl ProcessQosPolicy {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Default => "control_default_qos",
            Self::IgnoreTimerResolution => "candidate_b_ignore_timer_resolution",
            Self::DisableExecutionSpeed => "candidate_c_disable_execution_speed",
        }
    }
}

#[cfg(windows)]
enum ProcessQosBackend {
    Live {
        process: windows_sys::Win32::Foundation::HANDLE,
        previous: Option<windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_STATE>,
    },
    #[cfg(any(test, feature = "test-support"))]
    Mock {
        adapter: std::sync::Arc<MockProcessQosBackend>,
        previous: Option<windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_STATE>,
    },
    Inert,
}

#[cfg(all(windows, any(test, feature = "test-support")))]
#[derive(Default)]
pub struct MockProcessQosBackend {
    pub simulate_get_fail: std::sync::atomic::AtomicBool,
    pub simulate_set_fail: std::sync::atomic::AtomicBool,
    pub simulate_restore_fail: std::sync::atomic::AtomicBool,
    pub query_count: std::sync::atomic::AtomicUsize,
    pub set_count: std::sync::atomic::AtomicUsize,
    pub restore_count: std::sync::atomic::AtomicUsize,
    pub last_restore_failed: std::sync::atomic::AtomicBool,
    pub initial_state: std::sync::Mutex<
        Option<windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_STATE>,
    >,
    pub configured_states: std::sync::Mutex<
        Vec<windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_STATE>,
    >,
    pub restored_states: std::sync::Mutex<
        Vec<windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_STATE>,
    >,
}

#[cfg(all(windows, any(test, feature = "test-support")))]
impl std::fmt::Debug for MockProcessQosBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockProcessQosBackend")
            .field("simulate_get_fail", &self.simulate_get_fail)
            .field("simulate_set_fail", &self.simulate_set_fail)
            .field("simulate_restore_fail", &self.simulate_restore_fail)
            .field("query_count", &self.query_count)
            .field("set_count", &self.set_count)
            .field("restore_count", &self.restore_count)
            .field("last_restore_failed", &self.last_restore_failed)
            .finish_non_exhaustive()
    }
}

#[cfg(all(windows, any(test, feature = "test-support")))]
impl MockProcessQosBackend {
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::default())
    }

    pub fn with_initial_state(control_mask: u32, state_mask: u32) -> std::sync::Arc<Self> {
        use windows_sys::Win32::System::Threading::{
            PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_STATE,
        };
        let backend = Self::default();
        if let Ok(mut init) = backend.initial_state.lock() {
            *init = Some(PROCESS_POWER_THROTTLING_STATE {
                Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                ControlMask: control_mask,
                StateMask: state_mask,
            });
        }
        std::sync::Arc::new(backend)
    }

    pub fn query_state(
        &self,
    ) -> Option<windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_STATE> {
        self.query_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self
            .simulate_get_fail
            .load(std::sync::atomic::Ordering::Acquire)
        {
            None
        } else {
            let state = self
                .initial_state
                .lock()
                .ok()
                .and_then(|guard| *guard)
                .unwrap_or(
                    windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_STATE {
                        Version:
                            windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                        ControlMask: 0,
                        StateMask: 0,
                    },
                );
            Some(state)
        }
    }

    pub fn set_state(
        &self,
        state: windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_STATE,
    ) -> bool {
        self.set_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut list) = self.configured_states.lock() {
            list.push(state);
        }
        !self
            .simulate_set_fail
            .load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn restore(
        &self,
        state: windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_STATE,
    ) -> bool {
        self.restore_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Ok(mut list) = self.restored_states.lock() {
            list.push(state);
        }
        let fail = self
            .simulate_restore_fail
            .load(std::sync::atomic::Ordering::Acquire);
        if fail {
            self.last_restore_failed
                .store(true, std::sync::atomic::Ordering::Release);
            false
        } else {
            true
        }
    }
}

/// RAII guard for process-level power throttling state.
///
/// Restores the process's prior throttling configuration upon drop.
pub struct ProcessPowerThrottlingGuard {
    #[cfg(windows)]
    backend: ProcessQosBackend,
    policy: ProcessQosPolicy,
    active: bool,
}

impl std::fmt::Debug for ProcessPowerThrottlingGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessPowerThrottlingGuard")
            .field("policy", &self.policy)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl ProcessPowerThrottlingGuard {
    pub fn apply(policy: ProcessQosPolicy) -> Self {
        #[cfg(windows)]
        {
            use std::ffi::c_void;
            use std::mem::size_of;
            use windows_sys::Win32::System::Threading::{
                GetCurrentProcess, GetProcessInformation, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
                PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
                ProcessPowerThrottling, SetProcessInformation,
            };

            // SAFETY: GetCurrentProcess returns a pseudo-handle valid for the process lifetime.
            let process = unsafe { GetCurrentProcess() };

            if policy == ProcessQosPolicy::Default {
                return Self {
                    backend: ProcessQosBackend::Inert,
                    policy,
                    active: false,
                };
            }

            let mut previous = PROCESS_POWER_THROTTLING_STATE {
                Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                ControlMask: 0,
                StateMask: 0,
            };

            // Query existing state first. If the OS query fails, do not attempt to
            // mutate the process power throttling state, preserving safe restoration.
            let queried = unsafe {
                GetProcessInformation(
                    process,
                    ProcessPowerThrottling,
                    (&raw mut previous).cast::<c_void>(),
                    size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
                )
            } != 0;

            if !queried {
                return Self {
                    backend: ProcessQosBackend::Inert,
                    policy,
                    active: false,
                };
            }

            let control_mask = match policy {
                ProcessQosPolicy::Default => 0,
                ProcessQosPolicy::IgnoreTimerResolution => {
                    PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION
                }
                ProcessQosPolicy::DisableExecutionSpeed => PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            };

            let configured = PROCESS_POWER_THROTTLING_STATE {
                Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                ControlMask: control_mask,
                StateMask: 0,
            };

            let active = unsafe {
                SetProcessInformation(
                    process,
                    ProcessPowerThrottling,
                    (&raw const configured).cast::<c_void>(),
                    size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
                )
            } != 0;

            Self {
                backend: if active {
                    ProcessQosBackend::Live {
                        process,
                        previous: Some(previous),
                    }
                } else {
                    ProcessQosBackend::Inert
                },
                policy,
                active,
            }
        }
        #[cfg(not(windows))]
        {
            Self {
                policy,
                active: false,
            }
        }
    }

    #[cfg(all(windows, any(test, feature = "test-support")))]
    pub fn apply_with_mock(
        policy: ProcessQosPolicy,
        mock: std::sync::Arc<MockProcessQosBackend>,
    ) -> Self {
        if policy == ProcessQosPolicy::Default {
            return Self {
                backend: ProcessQosBackend::Mock {
                    adapter: mock,
                    previous: None,
                },
                policy,
                active: false,
            };
        }

        let previous = match mock.query_state() {
            Some(p) => p,
            None => {
                return Self {
                    backend: ProcessQosBackend::Mock {
                        adapter: mock,
                        previous: None,
                    },
                    policy,
                    active: false,
                };
            }
        };

        let control_mask = match policy {
            ProcessQosPolicy::Default => 0,
            ProcessQosPolicy::IgnoreTimerResolution => {
                windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION
            }
            ProcessQosPolicy::DisableExecutionSpeed => {
                windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_EXECUTION_SPEED
            }
        };

        let configured = windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_STATE {
            Version:
                windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: control_mask,
            StateMask: 0,
        };

        let active = mock.set_state(configured);

        Self {
            backend: ProcessQosBackend::Mock {
                adapter: mock,
                previous: active.then_some(previous),
            },
            policy,
            active,
        }
    }

    pub fn policy(&self) -> ProcessQosPolicy {
        self.policy
    }

    pub fn is_active(&self) -> bool {
        self.active
    }
}

impl Drop for ProcessPowerThrottlingGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        match &mut self.backend {
            ProcessQosBackend::Live {
                process,
                previous: Some(previous),
            } => {
                use std::ffi::c_void;
                use std::mem::size_of;
                use windows_sys::Win32::System::Threading::{
                    PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling, SetProcessInformation,
                };

                // SAFETY: process is valid for the current process lifetime and previous is
                // a correctly initialized PROCESS_POWER_THROTTLING_STATE.
                // We ALWAYS invoke real SetProcessInformation unconditionally to restore OS state.
                let restored = unsafe {
                    SetProcessInformation(
                        *process,
                        ProcessPowerThrottling,
                        (&raw const *previous).cast::<c_void>(),
                        size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
                    )
                } != 0;

                if !restored {
                    #[cfg(all(windows, any(test, feature = "test-support")))]
                    LAST_LIVE_RESTORE_FAILED.store(true, std::sync::atomic::Ordering::Release);
                }
            }
            #[cfg(any(test, feature = "test-support"))]
            ProcessQosBackend::Mock {
                adapter,
                previous: Some(previous),
            } => {
                let _ = adapter.restore(*previous);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PowerThrottlingGuard, ProcessPowerThrottlingGuard, ProcessQosPolicy};

    #[test]
    fn guard_can_be_constructed_and_dropped() {
        let guard = PowerThrottlingGuard::disable_current_thread();
        let _ = guard.is_active();
    }

    #[test]
    fn process_power_throttling_guard_lifecycle_mocked() {
        // 1. Default policy must be a no-op on the real apply constructor without any OS syscall.
        let default_guard = ProcessPowerThrottlingGuard::apply(ProcessQosPolicy::Default);
        assert_eq!(default_guard.policy(), ProcessQosPolicy::Default);
        assert!(
            !default_guard.is_active(),
            "default policy must not activate overrides"
        );
        drop(default_guard);

        // 2. Candidate policies lifecycle verified on isolated mock backend without mutating the runner process.
        for policy in [
            ProcessQosPolicy::IgnoreTimerResolution,
            ProcessQosPolicy::DisableExecutionSpeed,
        ] {
            let mock = super::MockProcessQosBackend::with_initial_state(0x1234, 0x5678);
            let guard = ProcessPowerThrottlingGuard::apply_with_mock(policy, mock.clone());
            assert_eq!(guard.policy(), policy);
            assert!(guard.is_active());
            assert_eq!(
                mock.query_count.load(std::sync::atomic::Ordering::SeqCst),
                1
            );
            assert_eq!(mock.set_count.load(std::sync::atomic::Ordering::SeqCst), 1);
            assert_eq!(
                mock.restore_count.load(std::sync::atomic::Ordering::SeqCst),
                0
            );

            // Verify configured state
            let expected_mask = match policy {
                ProcessQosPolicy::Default => 0,
                ProcessQosPolicy::IgnoreTimerResolution => {
                    windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION
                }
                ProcessQosPolicy::DisableExecutionSpeed => {
                    windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                }
            };
            assert_eq!(
                mock.configured_states.lock().unwrap()[0].ControlMask,
                expected_mask
            );
            assert_eq!(mock.configured_states.lock().unwrap()[0].StateMask, 0);

            // Drop guard and verify restoration of original captured policy bits
            drop(guard);
            assert_eq!(
                mock.restore_count.load(std::sync::atomic::Ordering::SeqCst),
                1
            );
            assert!(
                !mock
                    .last_restore_failed
                    .load(std::sync::atomic::Ordering::SeqCst)
            );
            assert_eq!(mock.restored_states.lock().unwrap()[0].ControlMask, 0x1234);
            assert_eq!(mock.restored_states.lock().unwrap()[0].StateMask, 0x5678);
        }
    }

    #[test]
    fn process_power_throttling_guard_failure_injection_mocked() {
        // Test query failure: guard must be inactive, no set or restore called.
        let mock_get_fail = super::MockProcessQosBackend::new();
        mock_get_fail
            .simulate_get_fail
            .store(true, std::sync::atomic::Ordering::Release);
        let guard = ProcessPowerThrottlingGuard::apply_with_mock(
            ProcessQosPolicy::IgnoreTimerResolution,
            mock_get_fail.clone(),
        );
        assert!(
            !guard.is_active(),
            "guard must not be active if GetProcessInformation failed"
        );
        assert_eq!(
            mock_get_fail
                .query_count
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(
            mock_get_fail
                .set_count
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        drop(guard);
        assert_eq!(
            mock_get_fail
                .restore_count
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );

        // Test set failure: query succeeded, set failed, guard must be inactive, no restore on drop.
        let mock_set_fail = super::MockProcessQosBackend::new();
        mock_set_fail
            .simulate_set_fail
            .store(true, std::sync::atomic::Ordering::Release);
        let guard = ProcessPowerThrottlingGuard::apply_with_mock(
            ProcessQosPolicy::IgnoreTimerResolution,
            mock_set_fail.clone(),
        );
        assert!(
            !guard.is_active(),
            "guard must not be active if SetProcessInformation failed"
        );
        assert_eq!(
            mock_set_fail
                .query_count
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(
            mock_set_fail
                .set_count
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        drop(guard);
        assert_eq!(
            mock_set_fail
                .restore_count
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[test]
    fn process_power_throttling_guard_restoration_verification_mocked() {
        // Clean restore
        let mock_clean = super::MockProcessQosBackend::new();
        let guard = ProcessPowerThrottlingGuard::apply_with_mock(
            ProcessQosPolicy::IgnoreTimerResolution,
            mock_clean.clone(),
        );
        assert!(guard.is_active());
        drop(guard);
        assert_eq!(
            mock_clean
                .restore_count
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert!(
            !mock_clean
                .last_restore_failed
                .load(std::sync::atomic::Ordering::Acquire),
            "clean drop must not report restoration failure"
        );

        // Injected restore failure
        let mock_fail = super::MockProcessQosBackend::new();
        mock_fail
            .simulate_restore_fail
            .store(true, std::sync::atomic::Ordering::Release);
        let guard = ProcessPowerThrottlingGuard::apply_with_mock(
            ProcessQosPolicy::IgnoreTimerResolution,
            mock_fail.clone(),
        );
        assert!(guard.is_active());
        drop(guard);
        assert_eq!(
            mock_fail
                .restore_count
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert!(
            mock_fail
                .last_restore_failed
                .load(std::sync::atomic::Ordering::Acquire),
            "injected restore failure must be recorded by diagnostic hook"
        );
    }
}
