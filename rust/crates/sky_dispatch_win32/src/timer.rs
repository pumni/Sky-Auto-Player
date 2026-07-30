//! High-resolution Waitable Timer wrapper for microsecond-accurate kernel sleeps.

pub struct WaitableTimer {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

unsafe impl Send for WaitableTimer {}
unsafe impl Sync for WaitableTimer {}

impl WaitableTimer {
    pub fn new() -> Option<Self> {
        #[cfg(windows)]
        {
            use std::ffi::CString;
            use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

            unsafe {
                let kernel32 = LoadLibraryA(c"kernel32.dll".as_ptr() as *const u8);
                if !kernel32.is_null() {
                    let fn_name = CString::new("CreateWaitableTimerExW").unwrap();
                    let func = GetProcAddress(kernel32, fn_name.as_ptr() as *const u8);
                    if let Some(func) = func {
                        type CreateTimerExFn =
                            unsafe extern "system" fn(
                                *const std::ffi::c_void,
                                *const u16,
                                u32,
                                u32,
                            )
                                -> windows_sys::Win32::Foundation::HANDLE;
                        let func: CreateTimerExFn = std::mem::transmute(func);

                        // CREATE_WAITABLE_TIMER_HIGH_RESOLUTION = 0x00000002
                        // TIMER_ALL_ACCESS = 0x001F0003
                        let handle =
                            func(std::ptr::null(), std::ptr::null(), 0x00000002, 0x001F0003);
                        if !handle.is_null() {
                            return Some(WaitableTimer { handle });
                        }
                    }
                }
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

            let due_time_100ns: i64 = -((us as i64) * 10);
            unsafe {
                let res =
                    SetWaitableTimer(self.handle, &due_time_100ns, 0, None, std::ptr::null(), 0);
                if res != 0 {
                    WaitForSingleObject(self.handle, INFINITE);
                    true
                } else {
                    false
                }
            }
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
                unsafe {
                    windows_sys::Win32::Foundation::CloseHandle(self.handle);
                }
            }
        }
    }
}
