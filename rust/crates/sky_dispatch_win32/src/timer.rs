//! High-resolution Waitable Timer wrapper for microsecond-accurate kernel sleeps.

pub struct WaitableTimer {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

pub struct TimerResolutionGuard {
    active: bool,
}

impl TimerResolutionGuard {
    pub fn acquire_1ms() -> Option<Self> {
        #[cfg(windows)]
        {
            // SAFETY: timeBeginPeriod has no pointer parameters; a successful
            // request is paired exactly once with timeEndPeriod in Drop.
            let active = unsafe { windows_sys::Win32::Media::timeBeginPeriod(1) }
                == windows_sys::Win32::Media::TIMERR_NOERROR;
            active.then_some(Self { active })
        }
        #[cfg(not(windows))]
        {
            None
        }
    }
}

impl Drop for TimerResolutionGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.active {
            // SAFETY: this exactly balances this guard's successful 1 ms
            // timeBeginPeriod request.
            unsafe {
                windows_sys::Win32::Media::timeEndPeriod(1);
            }
        }
    }
}

impl WaitableTimer {
    pub fn new() -> Option<Self> {
        Self::new_with_error().ok()
    }

    pub fn new_with_error() -> Result<Self, u32> {
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
                return Ok(WaitableTimer { handle });
            }
            Err(unsafe { windows_sys::Win32::Foundation::GetLastError() })
        }
        #[cfg(not(windows))]
        {
            Err(120)
        }
    }

    pub fn sleep_us(&self, us: u64) -> Result<(), u32> {
        if us == 0 {
            return Ok(());
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{INFINITE, WaitForSingleObject};

            self.arm_relative_us(us)?;
            // SAFETY: waiting borrows the live handle without transferring it.
            let wait_result = unsafe { WaitForSingleObject(self.handle, INFINITE) };
            if wait_result == windows_sys::Win32::Foundation::WAIT_OBJECT_0 {
                Ok(())
            } else {
                Err(unsafe { windows_sys::Win32::Foundation::GetLastError() })
            }
        }
        #[cfg(not(windows))]
        {
            std::thread::sleep(std::time::Duration::from_micros(us));
            Ok(())
        }
    }

    #[cfg(windows)]
    pub(crate) fn arm_relative_us(&self, us: u64) -> Result<(), u32> {
        use windows_sys::Win32::System::Threading::SetWaitableTimer;

        let Some(ticks_100ns) = us.checked_mul(10) else {
            return Err(87);
        };
        let Ok(ticks_100ns) = i64::try_from(ticks_100ns) else {
            return Err(87);
        };
        let due_time_100ns = -ticks_100ns;
        // SAFETY: the handle is a live waitable timer and the due-time
        // pointer remains valid for the duration of this call.
        if unsafe { SetWaitableTimer(self.handle, &due_time_100ns, 0, None, std::ptr::null(), 0) }
            != 0
        {
            Ok(())
        } else {
            Err(unsafe { windows_sys::Win32::Foundation::GetLastError() })
        }
    }

    #[cfg(windows)]
    pub(crate) fn raw_handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.handle
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
