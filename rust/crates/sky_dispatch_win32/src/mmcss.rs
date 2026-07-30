//! MMCSS thread priority registration ("Pro Audio") with RAII guard.

#[derive(Debug)]
pub struct MmcssGuard {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

impl MmcssGuard {
    pub fn join_pro_audio() -> Self {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{
                AVRT_PRIORITY_HIGH, AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
                AvSetMmThreadPriority,
            };

            let task_name: Vec<u16> = "Pro Audio\0".encode_utf16().collect();
            let mut task_index: u32 = 0;
            // SAFETY: the UTF-16 task name is NUL-terminated and lives through
            // the call; task_index is a valid writable out-parameter.
            let handle =
                unsafe { AvSetMmThreadCharacteristicsW(task_name.as_ptr(), &mut task_index) };
            if !handle.is_null() {
                // SAFETY: handle was returned by AvSetMmThreadCharacteristicsW.
                if unsafe { AvSetMmThreadPriority(handle, AVRT_PRIORITY_HIGH) } != 0 {
                    return MmcssGuard { handle };
                }
                // SAFETY: priority acquisition failed, so release the partial
                // registration immediately.
                unsafe {
                    AvRevertMmThreadCharacteristics(handle);
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
                use windows_sys::Win32::System::Threading::AvRevertMmThreadCharacteristics;
                // SAFETY: this guard uniquely owns the registration and Drop is
                // executed by the worker thread that created it.
                unsafe {
                    AvRevertMmThreadCharacteristics(self.handle);
                }
            }
        }
    }
}
