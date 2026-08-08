//! Minimal foreground-window query used by the worker's fresh focus check.

#[cfg(feature = "test-support")]
use std::cell::Cell;

#[cfg(feature = "test-support")]
thread_local! {
    static FOREGROUND_QUERY_COUNT: Cell<u64> = const { Cell::new(0) };
}

#[cfg(feature = "test-support")]
pub fn reset_foreground_query_count() {
    FOREGROUND_QUERY_COUNT.with(|count| count.set(0));
}

#[cfg(feature = "test-support")]
pub fn foreground_query_count() -> u64 {
    FOREGROUND_QUERY_COUNT.with(Cell::get)
}

pub fn foreground_window_matches(target_hwnd: isize) -> bool {
    if target_hwnd == 0 {
        return false;
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
