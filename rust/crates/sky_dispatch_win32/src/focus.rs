//! Minimal foreground-window query used by the worker's fresh focus check.

#[cfg(feature = "test-support")]
use std::cell::Cell;
#[cfg(feature = "test-support")]
use std::sync::Mutex;
#[cfg(feature = "test-support")]
use std::sync::atomic::{AtomicIsize, Ordering};

#[cfg(feature = "test-support")]
thread_local! {
    static FOREGROUND_QUERY_COUNT: Cell<u64> = const { Cell::new(0) };
}

#[cfg(feature = "test-support")]
static TEST_FOREGROUND_HWND: AtomicIsize = AtomicIsize::new(isize::MIN);
#[cfg(feature = "test-support")]
static TEST_FOREGROUND_LOCK: Mutex<()> = Mutex::new(());

#[cfg(feature = "test-support")]
pub fn reset_foreground_query_count() {
    FOREGROUND_QUERY_COUNT.with(|count| count.set(0));
}

#[cfg(feature = "test-support")]
pub fn foreground_query_count() -> u64 {
    FOREGROUND_QUERY_COUNT.with(Cell::get)
}

/// Override the foreground HWND for cross-platform worker tests. Production
/// builds do not compile this seam, and the default remains the real Win32
/// foreground query.
#[cfg(feature = "test-support")]
pub fn set_foreground_window_for_test(hwnd: Option<isize>) {
    TEST_FOREGROUND_HWND.store(hwnd.unwrap_or(isize::MIN), Ordering::Release);
}

/// Serialize tests that temporarily override the process-wide foreground
/// seam. The worker reads the atomic without taking this lock.
#[cfg(feature = "test-support")]
pub fn lock_foreground_window_for_test() -> std::sync::MutexGuard<'static, ()> {
    TEST_FOREGROUND_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn foreground_window_matches(target_hwnd: isize) -> bool {
    if target_hwnd == 0 {
        return false;
    }
    #[cfg(feature = "test-support")]
    {
        let overridden = TEST_FOREGROUND_HWND.load(Ordering::Acquire);
        if overridden != isize::MIN {
            return overridden == target_hwnd;
        }
    }
    #[cfg(windows)]
    {
        #[cfg(feature = "test-support")]
        FOREGROUND_QUERY_COUNT.with(|count| count.set(count.get().saturating_add(1)));
        // SAFETY: GetForegroundWindow takes no pointers and returns a borrowed
        // window handle that this function only compares numerically.
        let foreground =
            unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
        foreground as isize == target_hwnd
    }
    #[cfg(not(windows))]
    {
        false
    }
}
