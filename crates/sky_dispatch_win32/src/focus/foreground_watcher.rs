//! Narrow, out-of-context foreground WinEvent hint source. A hint is never
//! sender admission evidence; callers must continue to use the exact live
//! foreground and process-identity checks at the input boundary.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, atomic::AtomicPtr, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ForegroundHint {
    pub sequence: u64,
    pub hwnd: Option<isize>,
}

#[derive(Default)]
struct ForegroundWatcherState {
    hwnd: AtomicIsize,
    sequence: AtomicU64,
    closed: AtomicBool,
    message_loop_failed: AtomicBool,
    thread_id: AtomicU64,
    wake_lock: Mutex<()>,
    wake: Condvar,
}

static ACTIVE_WATCHER: AtomicPtr<ForegroundWatcherState> = AtomicPtr::new(std::ptr::null_mut());

pub struct ForegroundWatcher {
    state: Arc<ForegroundWatcherState>,
    last_sequence: AtomicU64,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl ForegroundWatcher {
    /// Start the one process-wide foreground hint watcher. Failure is normal:
    /// the caller must retain its periodic GetForegroundWindow fallback.
    pub fn start() -> Result<Self, String> {
        #[cfg(not(windows))]
        {
            Err("foreground WinEvent hooks are available only on Windows".into())
        }
        #[cfg(windows)]
        {
            let state = Arc::new(ForegroundWatcherState::default());
            let worker_state = Arc::clone(&state);
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let worker = thread::Builder::new()
                .name("sky-foreground-winevent".into())
                .spawn(move || watcher_message_loop(worker_state, ready_tx))
                .map_err(|error| format!("failed to start foreground watcher: {error}"))?;
            match ready_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(Ok(())) => Ok(Self {
                    state,
                    last_sequence: AtomicU64::new(0),
                    worker: Mutex::new(Some(worker)),
                }),
                Ok(Err(error)) => {
                    let _ = worker.join();
                    Err(error)
                }
                Err(error) => {
                    state.closed.store(true, Ordering::Release);
                    let thread_id = state.thread_id.load(Ordering::Acquire) as u32;
                    if thread_id != 0 {
                        // SAFETY: this thread id belongs to the watcher thread;
                        // WM_QUIT asks its GetMessageW loop to exit.
                        unsafe {
                            windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                                thread_id,
                                windows_sys::Win32::UI::WindowsAndMessaging::WM_QUIT,
                                0,
                                0,
                            );
                        }
                    }
                    let _ = worker.join();
                    Err(format!("foreground watcher startup timed out: {error}"))
                }
            }
        }
    }

    /// Return the newest coalesced event hint. The HWND is untrusted as
    /// admission evidence and may be stale or absent.
    pub fn take_hint(&self) -> Option<ForegroundHint> {
        let sequence = self.state.sequence.load(Ordering::Acquire);
        if sequence == 0 || sequence == self.last_sequence.load(Ordering::Relaxed) {
            return None;
        }
        let hwnd = self.state.hwnd.load(Ordering::Acquire);
        self.last_sequence.store(sequence, Ordering::Relaxed);
        Some(ForegroundHint {
            sequence,
            hwnd: (hwnd != 0).then_some(hwnd),
        })
    }

    /// Wait for a coalesced hint, with a finite timeout used by the exact
    /// foreground polling fallback. The hint only wakes the owner; it never
    /// authorizes a sender operation.
    pub fn wait_for_hint(&self, timeout: Duration) -> Option<ForegroundHint> {
        if let Some(hint) = self.take_hint() {
            return Some(hint);
        }
        let guard = self
            .state
            .wake_lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !self.state.closed.load(Ordering::Acquire)
            && !self.state.message_loop_failed.load(Ordering::Acquire)
            && self.state.sequence.load(Ordering::Acquire)
                == self.last_sequence.load(Ordering::Relaxed)
        {
            let (guard, _) = self
                .state
                .wake
                .wait_timeout(guard, timeout)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            drop(guard);
        }
        self.take_hint()
    }

    /// A failed message loop requires immediate polling fallback. The caller
    /// should poll periodically even while the hook reports healthy because
    /// Windows event delivery can still be lost or reordered.
    pub fn polling_fallback_required(&self) -> bool {
        self.state.message_loop_failed.load(Ordering::Acquire)
    }
}

impl Drop for ForegroundWatcher {
    fn drop(&mut self) {
        self.state.closed.store(true, Ordering::Release);
        self.state.wake.notify_all();
        #[cfg(windows)]
        {
            let thread_id = self.state.thread_id.load(Ordering::Acquire) as u32;
            if thread_id != 0 {
                // SAFETY: this id is owned by the message-loop thread. Its
                // teardown unhooks the event source before the JoinHandle is
                // joined below.
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                        thread_id,
                        windows_sys::Win32::UI::WindowsAndMessaging::WM_QUIT,
                        0,
                        0,
                    );
                }
            }
        }
        if let Ok(worker) = self.worker.get_mut()
            && let Some(worker) = worker.take()
        {
            let _ = worker.join();
        }
    }
}

#[cfg(windows)]
fn watcher_message_loop(
    state: Arc<ForegroundWatcherState>,
    ready: mpsc::SyncSender<Result<(), String>>,
) {
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, EVENT_SYSTEM_FOREGROUND, GetMessageW, MSG, PM_NOREMOVE, PeekMessageW,
        TranslateMessage, WINEVENT_OUTOFCONTEXT,
    };

    // Create this thread's message queue before publishing its id to Drop.
    let mut message = MSG::default();
    // SAFETY: MSG is initialized and no message is removed by PM_NOREMOVE.
    unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, PM_NOREMOVE) };
    let thread_id = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
    state
        .thread_id
        .store(u64::from(thread_id), Ordering::Release);

    let state_pointer = Arc::as_ptr(&state).cast_mut();
    if ACTIVE_WATCHER
        .compare_exchange(
            std::ptr::null_mut(),
            state_pointer,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        let _ = ready.send(Err("foreground watcher is already active".into()));
        return;
    }

    // SAFETY: this exact adapter registers only the foreground event range,
    // with no process/thread filter and the required out-of-context flag.
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            std::ptr::null_mut(),
            Some(foreground_event_callback),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if hook.is_null() {
        clear_active_watcher(&state);
        let _ = ready.send(Err("SetWinEventHook failed for foreground event".into()));
        return;
    }
    if ready.send(Ok(())).is_err() {
        state.closed.store(true, Ordering::Release);
    }

    loop {
        if state.closed.load(Ordering::Acquire) {
            break;
        }
        // SAFETY: MSG is a valid output buffer and this thread owns its queue.
        let result = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
        if result <= 0 {
            if !state.closed.load(Ordering::Acquire) {
                state.message_loop_failed.store(true, Ordering::Release);
                state.wake.notify_all();
            }
            break;
        }
        // SAFETY: the message was returned by this thread's GetMessageW call.
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    // SAFETY: hook is the live handle returned by this exact adapter.
    unsafe { UnhookWinEvent(hook) };
    clear_active_watcher(&state);
}

#[cfg(windows)]
fn clear_active_watcher(state: &Arc<ForegroundWatcherState>) {
    let state_pointer = Arc::as_ptr(state).cast_mut();
    let _ = ACTIVE_WATCHER.compare_exchange(
        state_pointer,
        std::ptr::null_mut(),
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

#[cfg(windows)]
unsafe extern "system" fn foreground_event_callback(
    _hook: windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK,
    event: u32,
    hwnd: windows_sys::Win32::Foundation::HWND,
    _object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    use windows_sys::Win32::UI::WindowsAndMessaging::EVENT_SYSTEM_FOREGROUND;
    if event != EVENT_SYSTEM_FOREGROUND {
        return;
    }
    let state_pointer = ACTIVE_WATCHER.load(Ordering::Acquire);
    if state_pointer.is_null() {
        return;
    }
    // SAFETY: the out-of-context hook dispatches on its registering message
    // thread. The owner retains this Arc until after UnhookWinEvent and joins
    // that thread before dropping the state; callback touches only atomics.
    let state = unsafe { &*state_pointer };
    state.hwnd.store(hwnd as isize, Ordering::Relaxed);
    state.sequence.fetch_add(1, Ordering::Release);
    state.wake.notify_one();
}

#[cfg(test)]
mod tests {
    use super::{ForegroundHint, ForegroundWatcher, ForegroundWatcherState};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    #[test]
    fn event_hint_is_a_coalesced_value_not_an_admission_token() {
        let state = ForegroundWatcherState::default();
        state.hwnd.store(17, Ordering::Relaxed);
        state.sequence.store(1, Ordering::Release);
        let observed = ForegroundHint {
            sequence: state.sequence.load(Ordering::Acquire),
            hwnd: Some(state.hwnd.load(Ordering::Acquire)),
        };
        assert_eq!(
            observed,
            ForegroundHint {
                sequence: 1,
                hwnd: Some(17)
            }
        );
        state.hwnd.store(0, Ordering::Relaxed);
        state.sequence.store(2, Ordering::Release);
        assert_eq!(state.sequence.load(Ordering::Acquire), 2);
        assert_eq!(state.hwnd.load(Ordering::Acquire), 0);
    }

    #[test]
    fn callback_wakes_waiter_but_only_returns_a_hint() {
        let watcher = Arc::new(ForegroundWatcher {
            state: Arc::new(ForegroundWatcherState::default()),
            last_sequence: AtomicU64::new(0),
            worker: Mutex::new(None),
        });
        let (ready_tx, ready_rx) = mpsc::channel();
        let waiter = Arc::clone(&watcher);
        let waiting = std::thread::spawn(move || {
            ready_tx.send(()).expect("waiter ready");
            waiter.wait_for_hint(Duration::from_secs(1))
        });
        ready_rx.recv().expect("waiter started");
        std::thread::sleep(Duration::from_millis(10));
        watcher.state.hwnd.store(41, Ordering::Relaxed);
        watcher.state.sequence.fetch_add(1, Ordering::Release);
        watcher.state.wake.notify_one();

        assert_eq!(
            waiting.join().expect("hint waiter thread"),
            Some(ForegroundHint {
                sequence: 1,
                hwnd: Some(41),
            })
        );
    }
}
