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

/// RAII guard for process-level power throttling state.
///
/// Restores the process's prior throttling configuration upon drop.
pub struct ProcessPowerThrottlingGuard {
    #[cfg(windows)]
    process: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(windows)]
    previous: Option<windows_sys::Win32::System::Threading::PROCESS_POWER_THROTTLING_STATE>,
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
                    process,
                    previous: None,
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
                    process,
                    previous: None,
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
                process,
                previous: active.then_some(previous),
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
        if let Some(previous) = self.previous {
            use std::ffi::c_void;
            use std::mem::size_of;
            use windows_sys::Win32::System::Threading::{
                PROCESS_POWER_THROTTLING_STATE, ProcessPowerThrottling, SetProcessInformation,
            };

            unsafe {
                SetProcessInformation(
                    self.process,
                    ProcessPowerThrottling,
                    (&raw const previous).cast::<c_void>(),
                    size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
                );
            }
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
    fn process_power_throttling_guard_lifecycle() {
        let default_guard = ProcessPowerThrottlingGuard::apply(ProcessQosPolicy::Default);
        assert_eq!(default_guard.policy(), ProcessQosPolicy::Default);
        assert!(
            !default_guard.is_active(),
            "default policy must not activate overrides"
        );
        drop(default_guard);

        for policy in [
            ProcessQosPolicy::IgnoreTimerResolution,
            ProcessQosPolicy::DisableExecutionSpeed,
        ] {
            let guard = ProcessPowerThrottlingGuard::apply(policy);
            assert_eq!(guard.policy(), policy);
            drop(guard);
        }
    }
}
