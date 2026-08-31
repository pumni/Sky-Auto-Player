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

/// Resolve the visible Sky window using the same title/process admission
/// boundary as the legacy desktop target adapter.  This is deliberately kept
/// in the Win32 crate so application code never has to own unsafe window API
/// calls.  The returned HWND is only a target hint; the realtime worker still
/// performs the final foreground admission before every physical down.
#[cfg(windows)]
pub fn find_sky_window(process_names: &[String], allow_title_fallback: bool) -> Option<isize> {
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible,
    };

    struct Search {
        names: Vec<String>,
        allow_title_fallback: bool,
        target: Option<isize>,
    }

    unsafe extern "system" fn visit(hwnd: HWND, context: LPARAM) -> i32 {
        let search = unsafe { &mut *(context as *mut Search) };
        if unsafe { IsWindowVisible(hwnd) } == 0 {
            return 1;
        }
        let length = unsafe { GetWindowTextLengthW(hwnd) };
        if length <= 0 {
            return 1;
        }
        let mut title = vec![0u16; length as usize + 1];
        let written = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
        if written <= 0 {
            return 1;
        }
        let title = String::from_utf16_lossy(&title[..written as usize]);
        if title != "Sky" && !title.starts_with("Sky") {
            return 1;
        }
        let mut pid = 0u32;
        if unsafe { GetWindowThreadProcessId(hwnd, &mut pid) } == 0 {
            return 1;
        }
        let mut process_name = None;
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if !handle.is_null() {
            let mut buffer = [0u16; 4096];
            let mut size = buffer.len() as u32;
            if unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) } != 0
            {
                process_name = String::from_utf16(&buffer[..size as usize])
                    .ok()
                    .and_then(|path| {
                        std::path::Path::new(&path)
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                    });
            }
            unsafe {
                CloseHandle(handle);
            }
        }
        if search.allow_title_fallback
            || search.names.is_empty()
            || process_name
                .as_deref()
                .is_some_and(|name| search.names.iter().any(|expected| expected == name))
        {
            search.target = Some(hwnd as isize);
            return 0;
        }
        1
    }

    let mut search = Search {
        names: process_names.to_vec(),
        allow_title_fallback,
        target: None,
    };
    // The callback does not outlive this synchronous EnumWindows call. The
    // search owns its process-name strings, so the callback context has no
    // fabricated reference lifetime.
    let context = &mut search as *mut Search as LPARAM;
    unsafe {
        EnumWindows(Some(visit), context);
    }
    search.target
}

#[cfg(not(windows))]
pub fn find_sky_window(_process_names: &[String], _allow_title_fallback: bool) -> Option<isize> {
    None
}

/// Perform only the documented minimal foreground request.  No input queue
/// attachment, z-order forcing, or repeated activation is performed here.
#[cfg(windows)]
pub fn focus_window(hwnd: isize) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SW_RESTORE, SetForegroundWindow, ShowWindow,
    };
    if hwnd == 0 {
        return false;
    }
    unsafe {
        ShowWindow(hwnd as _, SW_RESTORE);
        SetForegroundWindow(hwnd as _) != 0
    }
}

#[cfg(not(windows))]
pub fn focus_window(_hwnd: isize) -> bool {
    false
}

/// Request the documented minimal focus change and verify the exact HWND for
/// a bounded interval before a physical worker is armed.  The polling is
/// read-only and intentionally does not attach input queues or force z-order.
pub fn focus_window_and_verify(hwnd: isize, budget: std::time::Duration) -> bool {
    if !focus_window(hwnd) {
        return false;
    }
    let deadline = std::time::Instant::now() + budget;
    loop {
        if foreground_window_matches(hwnd) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
