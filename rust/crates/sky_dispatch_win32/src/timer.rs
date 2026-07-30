//! High-resolution Waitable Timer wrapper for microsecond-accurate kernel sleeps.

pub struct WaitableTimer {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

impl WaitableTimer {
    pub fn new() -> Option<Self> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, CreateWaitableTimerExW, TIMER_ALL_ACCESS,
            };

            // SAFETY: null security attributes and name request an unnamed timer
            // owned solely by this wrapper. Drop closes the returned handle once.
            let handle = unsafe {
                CreateWaitableTimerExW(
                    std::ptr::null(),
                    std::ptr::null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS,
                )
            };
            if !handle.is_null() {
                return Some(WaitableTimer { handle });
            }
            None
        }
        #[cfg(not(windows))]
        {
            None
        }
    }

    pub fn sleep_us(&self, us: u64) -> bool {
        if us == 0 {
            return true;
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{
                INFINITE, SetWaitableTimer, WaitForSingleObject,
            };

            let Some(ticks_100ns) = us.checked_mul(10) else {
                return false;
            };
            let Ok(ticks_100ns) = i64::try_from(ticks_100ns) else {
                return false;
            };
            let due_time_100ns = -ticks_100ns;
            // SAFETY: the handle is a live waitable timer and the due-time
            // pointer remains valid for the duration of this call.
            let armed = unsafe {
                SetWaitableTimer(self.handle, &due_time_100ns, 0, None, std::ptr::null(), 0)
            };
            if armed == 0 {
                return false;
            }
            // SAFETY: waiting borrows the live handle without transferring it.
            let wait_result = unsafe { WaitForSingleObject(self.handle, INFINITE) };
            wait_result == windows_sys::Win32::Foundation::WAIT_OBJECT_0
        }
        #[cfg(not(windows))]
        {
            std::thread::sleep(std::time::Duration::from_micros(us));
            true
        }
    }
}

impl Drop for WaitableTimer {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            if !self.handle.is_null() {
                // SAFETY: this wrapper is the unique owner and Drop runs once.
                unsafe {
                    windows_sys::Win32::Foundation::CloseHandle(self.handle);
                }
            }
        }
    }
}
