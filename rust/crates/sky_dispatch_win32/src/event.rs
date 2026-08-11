//! Owned auto-reset event used to interrupt the dispatch worker.

use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(not(windows))]
use std::sync::atomic::AtomicBool;

pub struct OwnedEvent {
    signal_generation: AtomicU64,
    #[cfg(windows)]
    handle: isize,
    #[cfg(not(windows))]
    signalled: AtomicBool,
    #[cfg(test)]
    take_count: AtomicU64,
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
                    signal_generation: AtomicU64::new(0),
                    handle: handle as isize,
                    #[cfg(test)]
                    take_count: AtomicU64::new(0),
                })
            }
        }
        #[cfg(not(windows))]
        {
            Some(Self {
                signal_generation: AtomicU64::new(0),
                signalled: AtomicBool::new(false),
                #[cfg(test)]
                take_count: AtomicU64::new(0),
            })
        }
    }

    pub fn signal(&self) -> bool {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::SetEvent;

            // SAFETY: raw_handle reconstructs the live event handle without
            // transferring ownership. The generation is published only
            // after SetEvent succeeds, so a generation observer can never
            // outrun the event signal and miss it during the spin handoff.
            let signalled = unsafe { SetEvent(self.raw_handle()) != 0 };
            if signalled {
                self.signal_generation.fetch_add(1, Ordering::Release);
            }
            signalled
        }
        #[cfg(not(windows))]
        {
            self.signalled.store(true, Ordering::Release);
            self.signal_generation.fetch_add(1, Ordering::Release);
            true
        }
    }

    /// Monotonic hint used to observe a signal without polling the Win32
    /// event object during the final spin. The event remains the authoritative
    /// interrupt source for long waits and is still consumed by `try_take`.
    pub fn signal_generation(&self) -> u64 {
        self.signal_generation.load(Ordering::Acquire)
    }

    /// Cheap polling hint for the bounded spin phase. The final deadline
    /// admission still uses [`Self::signal_generation`] with Acquire.
    pub fn signal_generation_relaxed(&self) -> u64 {
        self.signal_generation.load(Ordering::Relaxed)
    }

    pub fn try_take(&self) -> bool {
        #[cfg(test)]
        self.take_count.fetch_add(1, Ordering::Relaxed);
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

    #[cfg(test)]
    pub(crate) fn take_count(&self) -> u64 {
        self.take_count.load(Ordering::Relaxed)
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
        assert_eq!(event.signal_generation(), 0);
        assert!(!event.try_take());
        assert!(event.signal());
        assert_eq!(event.signal_generation(), 1);
        assert!(event.try_take());
        assert!(!event.try_take());
    }

    #[test]
    fn repeated_signals_advance_generation_even_when_event_is_auto_reset() {
        let event = OwnedEvent::new_auto_reset().expect("event");
        assert!(event.signal());
        assert!(event.signal());
        assert_eq!(event.signal_generation(), 2);
        assert!(event.try_take());
        assert!(!event.try_take());
    }
}
