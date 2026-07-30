//! Minimal foreground-window query used by the worker's fresh focus check.

pub fn foreground_window_matches(target_hwnd: isize) -> bool {
    if target_hwnd == 0 {
        return false;
    }
    #[cfg(windows)]
    {
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
