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

#[cfg(test)]
mod tests {
    use super::PowerThrottlingGuard;

    #[test]
    fn guard_can_be_constructed_and_dropped() {
        let guard = PowerThrottlingGuard::disable_current_thread();
        let _ = guard.is_active();
    }
}
