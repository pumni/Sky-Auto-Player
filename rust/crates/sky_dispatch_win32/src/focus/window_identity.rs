#[cfg(feature = "test-support")]
use super::WINDOW_IDENTITY_AUTHORITY_DROP_COUNT;
use super::filetime_ticks;
#[cfg(feature = "test-support")]
use std::sync::atomic::Ordering;

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

/// Retained authority for one window owner process. The process handle is
/// opened with `PROCESS_QUERY_LIMITED_INFORMATION` only and remains owned by
/// the session until terminal teardown. Full identity checks are control
/// plane work; the precision path uses only the frozen PID and atomic live
/// state published by the session.
pub struct WindowIdentityAuthority {
    pub(super) identity: WindowIdentity,
    pub(super) image_identity: String,
    pub(super) process_handle: Option<isize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowIdentityContinuity {
    Continuous,
    ProcessTerminated,
    OwnerMismatch,
    WindowUnavailable,
    ProcessIdentityMismatch,
    QueryUnavailable,
}

// A process HANDLE is a kernel object handle that may be used for concurrent
// read-only queries from the supervisor thread and closed by the owning
// session thread after that supervisor has joined.
unsafe impl Send for WindowIdentityAuthority {}

impl WindowIdentityAuthority {
    pub fn identity(&self) -> &WindowIdentity {
        &self.identity
    }

    pub fn process_image_identity(&self) -> &str {
        &self.image_identity
    }

    #[cfg(feature = "test-support")]
    pub fn synthetic_for_test(hwnd: isize, owner_pid: u32) -> Self {
        Self {
            identity: WindowIdentity {
                hwnd,
                owner_pid,
                title: "Sky test window".to_string(),
                process_start_time_filetime: 1,
                process_image_basename: "sky-test.exe".to_string(),
            },
            image_identity: "C:\\Sky\\sky-test.exe".to_string(),
            process_handle: None,
        }
    }

    pub(super) fn process_continuity(&self) -> WindowIdentityContinuity {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::FILETIME;
            use windows_sys::Win32::System::Threading::{
                GetProcessTimes, QueryFullProcessImageNameW,
            };

            let Some(handle) = self.process_handle else {
                #[cfg(feature = "test-support")]
                {
                    return WindowIdentityContinuity::Continuous;
                }
                #[cfg(not(feature = "test-support"))]
                {
                    return WindowIdentityContinuity::QueryUnavailable;
                }
            };
            let handle = handle as windows_sys::Win32::Foundation::HANDLE;
            let mut creation = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let mut exit = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let mut kernel = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let mut user = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) }
                == 0
            {
                return WindowIdentityContinuity::QueryUnavailable;
            }
            if filetime_ticks(exit) != 0 {
                return WindowIdentityContinuity::ProcessTerminated;
            }
            if filetime_ticks(creation) != self.identity.process_start_time_filetime {
                return WindowIdentityContinuity::ProcessIdentityMismatch;
            }
            let mut image = [0u16; 4096];
            let mut image_length = image.len() as u32;
            if unsafe {
                QueryFullProcessImageNameW(handle, 0, image.as_mut_ptr(), &mut image_length)
            } == 0
            {
                return WindowIdentityContinuity::QueryUnavailable;
            }
            let Ok(image) = String::from_utf16(&image[..image_length as usize]) else {
                return WindowIdentityContinuity::QueryUnavailable;
            };
            if !image.eq_ignore_ascii_case(&self.image_identity) {
                return WindowIdentityContinuity::ProcessIdentityMismatch;
            }
            WindowIdentityContinuity::Continuous
        }
        #[cfg(not(windows))]
        {
            WindowIdentityContinuity::QueryUnavailable
        }
    }

    /// Recheck that this exact HWND is still owned by the retained process
    /// object, and that its creation time and full image path still match the
    /// frozen startup identity. Called only by the desktop supervisor.
    pub fn continuity(&self) -> WindowIdentityContinuity {
        #[cfg(feature = "test-support")]
        if self.process_handle.is_none() {
            return WindowIdentityContinuity::Continuous;
        }
        let process_status = self.process_continuity();
        if process_status != WindowIdentityContinuity::Continuous {
            return process_status;
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::Foundation::HWND;
            use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindow};
            let hwnd = self.identity.hwnd as HWND;
            if unsafe { IsWindow(hwnd) } == 0 {
                return WindowIdentityContinuity::WindowUnavailable;
            }
            let mut owner_pid = 0u32;
            if unsafe { GetWindowThreadProcessId(hwnd, &mut owner_pid) } == 0 {
                WindowIdentityContinuity::QueryUnavailable
            } else if owner_pid != self.identity.owner_pid {
                WindowIdentityContinuity::OwnerMismatch
            } else {
                WindowIdentityContinuity::Continuous
            }
        }
        #[cfg(not(windows))]
        {
            WindowIdentityContinuity::QueryUnavailable
        }
    }
}

/// Terminates only the live, identity-retained project ReceiveOnly test sink.
/// This is available to the native acceptance harness through `test-support`;
/// production builds never request process-termination rights.
#[cfg(all(windows, feature = "test-support"))]
pub fn terminate_receive_only_sink_process_for_test(
    identity: &WindowIdentity,
    expected_image_identity: &str,
) -> Result<u32, String> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, FILETIME, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, QueryFullProcessImageNameW, TerminateProcess,
        WaitForSingleObject,
    };

    const SINK_TITLE: &str = "Sky Auto Player — Native Acceptance Sink";
    const TEST_EXIT_CODE: u32 = 0x646;

    if identity.title != SINK_TITLE
        || !identity
            .process_image_basename
            .eq_ignore_ascii_case("pwsh.exe")
        || identity.owner_pid == std::process::id()
    {
        return Err(
            "refusing to terminate a process that is not the retained ReceiveOnly sink".into(),
        );
    }
    // SAFETY: The retained window identity was validated by the acceptance
    // harness. The additional creation-time and image checks below bind this
    // termination-capable handle to that same PID incarnation.
    let process = unsafe {
        OpenProcess(
            PROCESS_TERMINATE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            identity.owner_pid,
        )
    };
    if process.is_null() {
        return Err("could not open the retained ReceiveOnly process for termination".into());
    }

    let result = (|| {
        let mut creation = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exit = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut kernel = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut user = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        // SAFETY: Each output points to a live, writable FILETIME value.
        if unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }
            == 0
            || filetime_ticks(exit) != 0
            || filetime_ticks(creation) != identity.process_start_time_filetime
        {
            return Err("ReceiveOnly process creation identity changed before termination".into());
        }

        let mut image = [0u16; 4096];
        let mut image_length = image.len() as u32;
        // SAFETY: `image` is writable and its capacity is supplied to Win32.
        if unsafe { QueryFullProcessImageNameW(process, 0, image.as_mut_ptr(), &mut image_length) }
            == 0
        {
            return Err("ReceiveOnly process image could not be revalidated".into());
        }
        let image = String::from_utf16(&image[..image_length as usize])
            .map_err(|_| "ReceiveOnly process image path was invalid UTF-16")?;
        if !image.eq_ignore_ascii_case(expected_image_identity) {
            return Err("ReceiveOnly process image identity changed before termination".into());
        }

        // SAFETY: `process` is the exact identity-revalidated sink handle.
        match unsafe { WaitForSingleObject(process, 0) } {
            WAIT_TIMEOUT => {}
            WAIT_OBJECT_0 => {
                return Err("ReceiveOnly process exited before the termination fixture".into());
            }
            _ => return Err("could not query ReceiveOnly process wait state".into()),
        }
        // SAFETY: This deliberately terminates only the project-owned sink
        // process whose title, PID incarnation, image path, and window
        // identity were verified above.
        if unsafe { TerminateProcess(process, TEST_EXIT_CODE) } == 0 {
            return Err(
                "TerminateProcess failed for the identity-verified ReceiveOnly sink".into(),
            );
        }
        // SAFETY: `process` is a valid synchronization handle.
        if unsafe { WaitForSingleObject(process, 5_000) } != WAIT_OBJECT_0 {
            return Err(
                "ReceiveOnly process did not signal termination within five seconds".into(),
            );
        }
        let mut exit_code = 0u32;
        // SAFETY: `exit_code` is writable and `process` is signaled.
        if unsafe { GetExitCodeProcess(process, &mut exit_code) } == 0
            || exit_code != TEST_EXIT_CODE
        {
            return Err(
                "ReceiveOnly process exit code did not match the termination fixture".into(),
            );
        }
        Ok(exit_code)
    })();

    // SAFETY: This function opened `process` and closes it exactly once here.
    unsafe { CloseHandle(process as HANDLE) };
    result
}

impl Drop for WindowIdentityAuthority {
    fn drop(&mut self) {
        #[cfg(feature = "test-support")]
        WINDOW_IDENTITY_AUTHORITY_DROP_COUNT.fetch_add(1, Ordering::AcqRel);
        #[cfg(windows)]
        if let Some(handle) = self.process_handle.take() {
            // SAFETY: this authority exclusively owns the process handle and
            // Drop is the single close point for the retained session handle.
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(
                    handle as windows_sys::Win32::Foundation::HANDLE,
                );
            }
        }
    }
}

#[cfg(windows)]
pub fn retain_window_identity(hwnd: isize) -> Result<WindowIdentityAuthority, String> {
    use std::path::Path;
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HWND};
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
    // SAFETY: the retained handle requests only PROCESS_QUERY_LIMITED_INFORMATION
    // and inheritance is disabled.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, owner_pid) };
    if process.is_null() {
        return Err("window owner process could not be opened".to_string());
    }
    let result = (|| {
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
        let mut creation_time = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exit_time = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut kernel_time = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut user_time = FILETIME {
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
            || filetime_ticks(exit_time) != 0
        {
            return Err("window owner process is not live".to_string());
        }
        let mut image_path = [0u16; 4096];
        let mut image_length = image_path.len() as u32;
        if unsafe {
            QueryFullProcessImageNameW(process, 0, image_path.as_mut_ptr(), &mut image_length)
        } == 0
        {
            return Err("process image path could not be read".to_string());
        }
        let image_identity = String::from_utf16(&image_path[..image_length as usize])
            .map_err(|_| "process image path is not valid UTF-16".to_string())?;
        let image_basename = Path::new(&image_identity)
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "process image basename is unavailable".to_string())?;
        let mut confirmed_owner_pid = 0u32;
        if unsafe { IsWindow(hwnd as HWND) } == 0
            || unsafe { GetWindowThreadProcessId(hwnd as HWND, &mut confirmed_owner_pid) } == 0
            || confirmed_owner_pid != owner_pid
        {
            return Err("window owner changed during identity capture".to_string());
        }
        Ok((
            WindowIdentity {
                hwnd,
                owner_pid,
                title,
                process_start_time_filetime: filetime_ticks(creation_time),
                process_image_basename: image_basename,
            },
            image_identity,
        ))
    })();
    match result {
        Ok((identity, image_identity)) => Ok(WindowIdentityAuthority {
            identity,
            image_identity,
            process_handle: Some(process as isize),
        }),
        Err(error) => {
            unsafe { CloseHandle(process) };
            Err(error)
        }
    }
}

#[cfg(not(windows))]
pub fn retain_window_identity(_hwnd: isize) -> Result<WindowIdentityAuthority, String> {
    Err("retained window identity is Windows-only".to_string())
}
