//! Windows process-level singleton guard for the desktop shell.
//!
//! The playback activity coordinator is intentionally process-local. Without a
//! process-level guard, two independently launched desktop shells could each
//! own a valid coordinator and emit physical SendInput concurrently.

use std::fmt;
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, SetLastError,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, GetWindowThreadProcessId, SW_RESTORE, SetForegroundWindow, ShowWindow,
};

const INSTANCE_MUTEX_NAME: &str = r"Local\io.github.pumni.skyautoplayer.single-instance";
const MAIN_WINDOW_TITLE: &str = "Sky Auto Player";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AcquireError {
    AlreadyRunning,
    Win32(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExistingInstancePolicy {
    FocusExisting,
    Refuse,
}

pub(crate) fn existing_instance_policy(debug_build: bool) -> ExistingInstancePolicy {
    if debug_build {
        ExistingInstancePolicy::Refuse
    } else {
        ExistingInstancePolicy::FocusExisting
    }
}

impl fmt::Display for AcquireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => {
                formatter.write_str("another desktop instance is already running")
            }
            Self::Win32(code) => write!(
                formatter,
                "single-instance mutex creation failed with Win32 error {code}"
            ),
        }
    }
}

pub(crate) struct SingleInstanceGuard {
    handle: HANDLE,
}

impl SingleInstanceGuard {
    pub(crate) fn acquire() -> Result<Self, AcquireError> {
        Self::acquire_named(INSTANCE_MUTEX_NAME)
    }

    fn acquire_named(name: &str) -> Result<Self, AcquireError> {
        let name = wide_null(name);
        unsafe {
            // CreateMutexW documents ERROR_ALREADY_EXISTS on success when the
            // named mutex already existed. Clear the thread-local last-error
            // slot first so a successful new mutex cannot inherit a stale 183.
            SetLastError(0);
            let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if handle.is_null() {
                return Err(AcquireError::Win32(GetLastError()));
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return Err(AcquireError::AlreadyRunning);
            }
            Ok(Self { handle })
        }
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

pub(crate) fn focus_existing_instance() {
    let title = wide_null(MAIN_WINDOW_TITLE);
    unsafe {
        let window = FindWindowW(std::ptr::null(), title.as_ptr());
        if window.is_null() {
            return;
        }
        let _ = ShowWindow(window, SW_RESTORE);
        let _ = SetForegroundWindow(window);
    }
}

pub(crate) fn existing_instance_details() -> Option<String> {
    let title = wide_null(MAIN_WINDOW_TITLE);
    unsafe {
        let window = FindWindowW(std::ptr::null(), title.as_ptr());
        if window.is_null() {
            return None;
        }

        let mut pid = 0u32;
        if GetWindowThreadProcessId(window, &mut pid) == 0 || pid == 0 {
            return None;
        }

        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Some(format!(" (PID {pid})"));
        }

        let mut path = [0u16; 4096];
        let mut path_length = path.len() as u32;
        let details =
            if QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut path_length) != 0 {
                let path = String::from_utf16_lossy(&path[..path_length as usize]);
                format!(" (PID {pid}, executable {path})")
            } else {
                format!(" (PID {pid})")
            };
        CloseHandle(process);
        Some(details)
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_mutex_rejects_a_second_live_guard_and_releases_on_drop() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let name = format!(
            r"Local\io.github.pumni.skyautoplayer.test.{}.{}",
            std::process::id(),
            suffix
        );

        let first = SingleInstanceGuard::acquire_named(&name).expect("first guard");
        assert_eq!(
            SingleInstanceGuard::acquire_named(&name).err(),
            Some(AcquireError::AlreadyRunning)
        );

        drop(first);
        let _reacquired = SingleInstanceGuard::acquire_named(&name).expect("reacquired guard");
    }

    #[test]
    fn debug_build_refuses_an_existing_instance() {
        assert_eq!(
            existing_instance_policy(true),
            ExistingInstancePolicy::Refuse
        );
    }

    #[test]
    fn packaged_build_focuses_an_existing_instance() {
        assert_eq!(
            existing_instance_policy(false),
            ExistingInstancePolicy::FocusExisting
        );
    }
}
