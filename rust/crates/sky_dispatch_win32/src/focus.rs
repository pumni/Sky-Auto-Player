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

/// Identity of a live window and the process that owns it.
///
/// This is deliberately a read-only boundary for qualification tooling. It
/// does not inspect process memory or command lines, and it does not grant
/// permission to send input by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowIdentity {
    pub hwnd: isize,
    pub owner_pid: u32,
    pub title: String,
    /// Windows FILETIME ticks since 1601-01-01 UTC, represented as one u64.
    pub process_start_time_filetime: u64,
    pub process_image_basename: String,
}

/// Result of the control-plane integrity comparison used to identify a
/// likely Windows UIPI block before a physical session is armed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetIntegrityCompatibility {
    Compatible,
    Mismatch,
    Unknown,
}

fn compare_integrity_rids(
    current_integrity_rid: u32,
    target_integrity_rid: u32,
) -> TargetIntegrityCompatibility {
    if target_integrity_rid > current_integrity_rid {
        TargetIntegrityCompatibility::Mismatch
    } else {
        TargetIntegrityCompatibility::Compatible
    }
}

/// Compare the current process token with the exact target window owner's
/// token. This is a read-only, control-plane preflight; it does not authorize
/// input and it never runs on the realtime dispatch thread.
#[cfg(windows)]
pub fn target_integrity_compatibility(hwnd: isize) -> TargetIntegrityCompatibility {
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{CloseHandle, HWND};
    use windows_sys::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL,
        TOKEN_QUERY, TokenIntegrityLevel,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    fn token_integrity_rid(token: windows_sys::Win32::Foundation::HANDLE) -> Option<u32> {
        let mut required = 0u32;
        // SAFETY: The first call intentionally supplies a null output buffer
        // to obtain the required TOKEN_MANDATORY_LABEL size.
        unsafe {
            GetTokenInformation(token, TokenIntegrityLevel, null_mut(), 0, &mut required);
        }
        if required < std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 {
            return None;
        }
        let word_count = (required as usize).div_ceil(std::mem::size_of::<u64>());
        let mut buffer = vec![0u64; word_count];
        // SAFETY: `buffer` is writable and sized by the preceding Win32 query;
        // the returned structure is read only while `buffer` remains alive.
        let success = unsafe {
            GetTokenInformation(
                token,
                TokenIntegrityLevel,
                buffer.as_mut_ptr().cast(),
                required,
                &mut required,
            )
        };
        if success == 0 {
            return None;
        }
        // SAFETY: GetTokenInformation returned a TOKEN_MANDATORY_LABEL in the
        // caller-owned buffer and the buffer is aligned by Vec<u64>'s allocator
        // for the structure's ABI alignment.
        let label = unsafe { &*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
        let sid = label.Label.Sid;
        if sid.is_null() {
            return None;
        }
        // SAFETY: The SID pointer belongs to the token information buffer and
        // is valid for the duration of this read-only query.
        let count = unsafe { GetSidSubAuthorityCount(sid).as_ref().copied()? };
        if count == 0 {
            return None;
        }
        // SAFETY: The final subauthority is the integrity RID for a valid
        // mandatory-label SID.
        unsafe {
            GetSidSubAuthority(sid, u32::from(count - 1))
                .as_ref()
                .copied()
        }
    }

    if hwnd == 0 {
        return TargetIntegrityCompatibility::Unknown;
    }
    let mut target_pid = 0u32;
    // SAFETY: The HWND is supplied by the existing target discovery boundary;
    // this call only resolves its owner PID into caller-owned storage.
    if unsafe { GetWindowThreadProcessId(hwnd as HWND, &mut target_pid) } == 0 || target_pid == 0 {
        return TargetIntegrityCompatibility::Unknown;
    }
    // SAFETY: Only PROCESS_QUERY_LIMITED_INFORMATION is requested for the
    // target process; no process memory or execution rights are acquired.
    let target_process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, target_pid) };
    if target_process.is_null() {
        return TargetIntegrityCompatibility::Unknown;
    }
    let mut current_token = null_mut();
    let mut target_token = null_mut();
    // SAFETY: Both process handles are valid for the duration of these token
    // queries and TOKEN_QUERY is the minimum required token right.
    let current_opened =
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut current_token) } != 0;
    let target_opened = current_opened
        && unsafe { OpenProcessToken(target_process, TOKEN_QUERY, &mut target_token) } != 0;
    let result = if target_opened {
        match (
            token_integrity_rid(current_token),
            token_integrity_rid(target_token),
        ) {
            (Some(current), Some(target)) => compare_integrity_rids(current, target),
            _ => TargetIntegrityCompatibility::Unknown,
        }
    } else {
        TargetIntegrityCompatibility::Unknown
    };
    // SAFETY: OpenProcessToken returned owned token handles when successful;
    // the pseudo-handle from GetCurrentProcess is never closed here.
    unsafe {
        if !target_token.is_null() {
            CloseHandle(target_token);
        }
        if !current_token.is_null() {
            CloseHandle(current_token);
        }
        CloseHandle(target_process);
    }
    result
}

#[cfg(not(windows))]
pub fn target_integrity_compatibility(_hwnd: isize) -> TargetIntegrityCompatibility {
    TargetIntegrityCompatibility::Unknown
}

#[cfg(windows)]
fn filetime_ticks(value: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

/// Inspect the exact owner identity of a live window.
///
/// The Windows implementation uses only the already-approved window and
/// limited-process-information APIs. The non-Windows implementation is an
/// explicit failure so callers cannot accidentally treat a host without
/// Win32 as a qualified physical-input host.
#[cfg(windows)]
pub fn inspect_window_identity(hwnd: isize) -> Result<WindowIdentity, String> {
    use std::path::Path;
    use windows_sys::Win32::Foundation::{CloseHandle, HWND};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
    };

    if hwnd == 0 || unsafe { IsWindow(hwnd as HWND) } == 0 {
        return Err("window HWND is not live".to_string());
    }

    let mut owner_pid = 0u32;
    if unsafe { GetWindowThreadProcessId(hwnd as HWND, &mut owner_pid) } == 0 || owner_pid == 0 {
        return Err("window owner PID could not be resolved".to_string());
    }

    let title_length = unsafe { GetWindowTextLengthW(hwnd as HWND) };
    if title_length <= 0 {
        return Err("window title is empty or unavailable".to_string());
    }
    let mut title = vec![0u16; title_length as usize + 1];
    let title_written =
        unsafe { GetWindowTextW(hwnd as HWND, title.as_mut_ptr(), title.len() as i32) };
    if title_written != title_length {
        return Err("window title could not be read consistently".to_string());
    }
    let title = String::from_utf16(&title[..title_written as usize])
        .map_err(|_| "window title is not valid UTF-16".to_string())?;

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, owner_pid) };
    if process.is_null() {
        return Err("window owner process could not be opened".to_string());
    }

    let result = (|| {
        let mut creation_time = windows_sys::Win32::Foundation::FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exit_time = windows_sys::Win32::Foundation::FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut kernel_time = windows_sys::Win32::Foundation::FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut user_time = windows_sys::Win32::Foundation::FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        if unsafe {
            GetProcessTimes(
                process,
                &mut creation_time,
                &mut exit_time,
                &mut kernel_time,
                &mut user_time,
            )
        } == 0
        {
            return Err("process creation time could not be read".to_string());
        }

        let mut image_path = [0u16; 4096];
        let mut image_length = image_path.len() as u32;
        if unsafe {
            QueryFullProcessImageNameW(process, 0, image_path.as_mut_ptr(), &mut image_length)
        } == 0
        {
            return Err("process image path could not be read".to_string());
        }
        let image_path = String::from_utf16(&image_path[..image_length as usize])
            .map_err(|_| "process image path is not valid UTF-16".to_string())?;
        let image_basename = Path::new(&image_path)
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "process image basename is unavailable".to_string())?;

        Ok(WindowIdentity {
            hwnd,
            owner_pid,
            title,
            process_start_time_filetime: filetime_ticks(creation_time),
            process_image_basename: image_basename,
        })
    })();
    unsafe {
        CloseHandle(process);
    }
    result
}

#[cfg(not(windows))]
pub fn inspect_window_identity(_hwnd: isize) -> Result<WindowIdentity, String> {
    Err("window identity inspection is Windows-only".to_string())
}

/// Resolve one non-extended scan code using the keyboard context of a target
/// window. This is observation-only evidence for the receive-only sink; it is
/// not an alternate input path.
#[cfg(windows)]
pub fn virtual_key_for_scan_code(hwnd: isize, scan_code: u16) -> Result<i32, String> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardLayout, MAPVK_VSC_TO_VK_EX, MapVirtualKeyExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

    if hwnd == 0 || scan_code == 0 {
        return Err("scan-code mapping requires nonzero HWND and scan code".to_string());
    }
    let thread_id = unsafe {
        GetWindowThreadProcessId(
            hwnd as windows_sys::Win32::Foundation::HWND,
            std::ptr::null_mut(),
        )
    };
    if thread_id == 0 {
        return Err("target keyboard thread could not be resolved".to_string());
    }
    let layout = unsafe { GetKeyboardLayout(thread_id) };
    if layout.is_null() {
        return Err("target keyboard layout could not be resolved".to_string());
    }
    let virtual_key = unsafe { MapVirtualKeyExW(u32::from(scan_code), MAPVK_VSC_TO_VK_EX, layout) };
    if virtual_key == 0 {
        return Err(format!(
            "scan code {scan_code:#x} has no target virtual key"
        ));
    }
    Ok(virtual_key as i32)
}

#[cfg(not(windows))]
pub fn virtual_key_for_scan_code(_hwnd: isize, _scan_code: u16) -> Result<i32, String> {
    Err("keyboard-layout mapping is Windows-only".to_string())
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

#[cfg(test)]
mod tests {
    use super::{TargetIntegrityCompatibility, compare_integrity_rids};

    #[test]
    fn higher_target_integrity_is_the_only_mismatch() {
        assert_eq!(
            compare_integrity_rids(0x2000, 0x1000),
            TargetIntegrityCompatibility::Compatible
        );
        assert_eq!(
            compare_integrity_rids(0x2000, 0x2000),
            TargetIntegrityCompatibility::Compatible
        );
        assert_eq!(
            compare_integrity_rids(0x2000, 0x3000),
            TargetIntegrityCompatibility::Mismatch
        );
    }
}
