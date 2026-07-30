//! MMCSS thread priority registration ("Pro Audio") with RAII guard.

#[derive(Debug)]
pub struct MmcssGuard {
    #[cfg(windows)]
    handle: *mut std::ffi::c_void,
}

unsafe impl Send for MmcssGuard {}
unsafe impl Sync for MmcssGuard {}

impl MmcssGuard {
    pub fn join_pro_audio() -> Self {
        #[cfg(windows)]
        {
            use std::ffi::CString;
            use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

            unsafe {
                let avrt = LoadLibraryA(c"avrt.dll".as_ptr() as *const u8);
                if !avrt.is_null() {
                    let set_fn_name = CString::new("AvSetMmThreadCharacteristicsW").unwrap();
                    let set_fn = GetProcAddress(avrt, set_fn_name.as_ptr() as *const u8);
                    if let Some(set_fn) = set_fn {
                        type SetFn = unsafe extern "system" fn(
                            *const u16,
                            *mut u32,
                        )
                            -> *mut std::ffi::c_void;
                        let set_fn: SetFn = std::mem::transmute(set_fn);

                        let task_name: Vec<u16> = "Pro Audio\0".encode_utf16().collect();
                        let mut task_index: u32 = 0;
                        let handle = set_fn(task_name.as_ptr(), &mut task_index);
                        if !handle.is_null() {
                            return MmcssGuard { handle };
                        }
                    }
                }
            }
            MmcssGuard {
                handle: std::ptr::null_mut(),
            }
        }
        #[cfg(not(windows))]
        {
            MmcssGuard {}
        }
    }

    pub fn is_active(&self) -> bool {
        #[cfg(windows)]
        {
            !self.handle.is_null()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
}

impl Drop for MmcssGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            if !self.handle.is_null() {
                use std::ffi::CString;
                use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

                unsafe {
                    let avrt = LoadLibraryA(c"avrt.dll".as_ptr() as *const u8);
                    if !avrt.is_null() {
                        let revert_fn_name =
                            CString::new("AvRevertMmThreadCharacteristics").unwrap();
                        let revert_fn = GetProcAddress(avrt, revert_fn_name.as_ptr() as *const u8);
                        if let Some(revert_fn) = revert_fn {
                            type RevertFn = unsafe extern "system" fn(*mut std::ffi::c_void) -> i32;
                            let revert_fn: RevertFn = std::mem::transmute(revert_fn);
                            revert_fn(self.handle);
                        }
                    }
                }
            }
        }
    }
}
