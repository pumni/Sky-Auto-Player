//! Owned auto-reset event used to interrupt the dispatch worker.

#[cfg(not(windows))]
use std::sync::atomic::{AtomicBool, Ordering};

pub struct OwnedEvent {
    #[cfg(windows)]
    handle: isize,
    #[cfg(not(windows))]
    signalled: AtomicBool,
}

impl OwnedEvent {
    pub fn new_auto_reset() -> Option<Self> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::CreateEventW;

            // SAFETY: null security attributes and name create an unnamed
            // auto-reset event whose sole owning handle is stored below.
            let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
            if handle.is_null() {
                None
            } else {
                Some(Self {
                    handle: handle as isize,
                })
            }
        }
        #[cfg(not(windows))]
        {
            Some(Self {
                signalled: AtomicBool::new(false),
            })
        }
    }

    pub fn signal(&self) -> bool {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::SetEvent;

            // SAFETY: raw_handle reconstructs the live event handle without
            // transferring ownership.
            unsafe { SetEvent(self.raw_handle()) != 0 }
        }
        #[cfg(not(windows))]
        {
            self.signalled.store(true, Ordering::Release);
            true
        }
    }

    pub fn try_take(&self) -> bool {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
            use windows_sys::Win32::System::Threading::WaitForSingleObject;

            // SAFETY: waiting borrows the live event handle.
            unsafe { WaitForSingleObject(self.raw_handle(), 0) == WAIT_OBJECT_0 }
        }
        #[cfg(not(windows))]
        {
            self.signalled.swap(false, Ordering::AcqRel)
        }
    }

    #[cfg(windows)]
    pub(crate) fn raw_handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
        self.handle as windows_sys::Win32::Foundation::HANDLE
    }
}

impl Drop for OwnedEvent {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            let handle = self.handle as windows_sys::Win32::Foundation::HANDLE;
            if !handle.is_null() {
                // SAFETY: this wrapper owns the handle and Drop runs once.
                unsafe {
                    windows_sys::Win32::Foundation::CloseHandle(handle);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OwnedEvent;

    #[test]
    fn auto_reset_event_consumes_one_signal() {
        let event = OwnedEvent::new_auto_reset().expect("event");
        assert!(!event.try_take());
        assert!(event.signal());
        assert!(event.try_take());
        assert!(!event.try_take());
    }
}
