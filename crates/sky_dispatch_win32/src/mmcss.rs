//! Per-thread real-time priority ladder with RAII restoration.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PriorityMode {
    Auto,
    Mmcss,
    TimeCritical,
    Highest,
    Off,
}

#[derive(Debug)]
pub struct MmcssGuard {
    #[cfg(windows)]
    mmcss_handle: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(windows)]
    priority_thread: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(windows)]
    old_priority: Option<i32>,
    acquired: &'static str,
}

impl MmcssGuard {
    pub fn join_games() -> Self {
        Self::acquire(PriorityMode::Mmcss)
    }

    pub fn acquire(mode: PriorityMode) -> Self {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{
                AVRT_PRIORITY_HIGH, AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
                AvSetMmThreadPriority, GetCurrentThread, GetThreadPriority, SetThreadPriority,
                THREAD_PRIORITY_HIGHEST, THREAD_PRIORITY_TIME_CRITICAL,
            };

            if matches!(mode, PriorityMode::Auto | PriorityMode::Mmcss) {
                let task_name: Vec<u16> = "Games\0".encode_utf16().collect();
                let mut task_index: u32 = 0;
                // SAFETY: the task name is NUL-terminated and task_index
                // is a valid writable out-parameter.
                let handle =
                    unsafe { AvSetMmThreadCharacteristicsW(task_name.as_ptr(), &mut task_index) };
                if !handle.is_null() {
                    // SAFETY: handle was returned by the MMCSS registration.
                    if unsafe { AvSetMmThreadPriority(handle, AVRT_PRIORITY_HIGH) } != 0 {
                        return Self {
                            mmcss_handle: handle,
                            priority_thread: std::ptr::null_mut(),
                            old_priority: None,
                            acquired: "mmcss:Games",
                        };
                    }
                    // SAFETY: release a partial registration after priority
                    // acquisition failed.
                    unsafe {
                        AvRevertMmThreadCharacteristics(handle);
                    }
                }
                if mode == PriorityMode::Mmcss {
                    return Self::off();
                }
            }

            let requested_priority = match mode {
                PriorityMode::TimeCritical => {
                    Some((THREAD_PRIORITY_TIME_CRITICAL, "thread:time_critical"))
                }
                PriorityMode::Auto | PriorityMode::Highest => {
                    Some((THREAD_PRIORITY_HIGHEST, "thread:highest"))
                }
                PriorityMode::Mmcss | PriorityMode::Off => None,
            };
            if let Some((priority, acquired)) = requested_priority {
                // SAFETY: GetCurrentThread returns a pseudo-handle valid on
                // this worker thread; it must not be closed.
                let thread = unsafe { GetCurrentThread() };
                // SAFETY: both operations use the valid current-thread handle.
                let old_priority = unsafe { GetThreadPriority(thread) };
                if old_priority != i32::MAX && unsafe { SetThreadPriority(thread, priority) } != 0 {
                    return Self {
                        mmcss_handle: std::ptr::null_mut(),
                        priority_thread: thread,
                        old_priority: Some(old_priority),
                        acquired,
                    };
                }
            }
            Self::off()
        }
        #[cfg(not(windows))]
        {
            let _ = mode;
            Self { acquired: "off" }
        }
    }

    fn off() -> Self {
        Self {
            #[cfg(windows)]
            mmcss_handle: std::ptr::null_mut(),
            #[cfg(windows)]
            priority_thread: std::ptr::null_mut(),
            #[cfg(windows)]
            old_priority: None,
            acquired: "off",
        }
    }

    pub fn acquired(&self) -> &'static str {
        self.acquired
    }

    pub fn is_active(&self) -> bool {
        self.acquired != "off"
    }
}

impl PriorityMode {
    /// Stable provenance label for the requested policy. This is distinct
    /// from the acquired label because Auto may fall back from MMCSS.
    pub const fn requested_label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Mmcss => "mmcss",
            Self::TimeCritical => "time_critical",
            Self::Highest => "highest",
            Self::Off => "off",
        }
    }
}

impl Drop for MmcssGuard {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{
                AvRevertMmThreadCharacteristics, SetThreadPriority,
            };
            if let Some(old_priority) = self.old_priority {
                // SAFETY: restoration occurs on the same worker thread using
                // its still-valid pseudo-handle.
                unsafe {
                    SetThreadPriority(self.priority_thread, old_priority);
                }
            }
            if !self.mmcss_handle.is_null() {
                // SAFETY: this guard uniquely owns the registration and Drop is
                // executed by the worker thread that created it.
                unsafe {
                    AvRevertMmThreadCharacteristics(self.mmcss_handle);
                }
            }
        }
    }
}
