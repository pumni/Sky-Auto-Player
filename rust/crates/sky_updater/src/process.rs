use std::path::Path;
use std::time::Duration;

use crate::error::{Result, UpdaterError};

pub fn wait_for_parent(
    parent_pid: u32,
    timeout: Duration,
    expected_executable: &Path,
) -> Result<()> {
    if parent_pid == 0 {
        return Err(UpdaterError::InvalidArgument(
            "parent PID must be nonzero".into(),
        ));
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{
            CloseHandle, ERROR_INVALID_PARAMETER, GetLastError, WAIT_OBJECT_0, WAIT_TIMEOUT,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            WaitForSingleObject,
        };
        let handle = unsafe {
            OpenProcess(
                PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                parent_pid,
            )
        };
        if handle.is_null() {
            // ERROR_INVALID_PARAMETER means the PID no longer exists. Other
            // failures (for example access denied) are not evidence that the
            // parent exited, so fail closed before any update work.
            if unsafe { GetLastError() } == ERROR_INVALID_PARAMETER {
                return Ok(());
            }
            return Err(UpdaterError::NetworkFailure(
                "could not open parent process for bounded wait".into(),
            ));
        }
        verify_parent_image(handle, expected_executable)?;
        let result = unsafe {
            WaitForSingleObject(handle, timeout.as_millis().min(u32::MAX as u128) as u32)
        };
        unsafe { CloseHandle(handle) };
        match result {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => Err(UpdaterError::ParentTimeout),
            _ => Err(UpdaterError::NetworkFailure("parent wait failed".into())),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (timeout, expected_executable);
        Err(UpdaterError::NetworkFailure(
            "native updater requires Windows".into(),
        ))
    }
}

#[cfg(windows)]
fn verify_parent_image(
    handle: windows_sys::Win32::Foundation::HANDLE,
    expected_executable: &Path,
) -> Result<()> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
    use windows_sys::Win32::System::Threading::QueryFullProcessImageNameW;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    let mut buffer = [0u16; 32_768];
    let mut length = buffer.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
        // The parent may terminate after OpenProcess succeeds but before its
        // image is queried. A signaled handle proves that this is the bounded
        // parent-exit race; a live handle still fails closed on query errors.
        if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
            return Ok(());
        }
        return Err(UpdaterError::NetworkFailure(
            "could not query parent process image".into(),
        ));
    }
    if length == 0 || length as usize > buffer.len() {
        return Err(UpdaterError::NetworkFailure(
            "parent image path length is invalid".into(),
        ));
    }
    let actual = std::ffi::OsString::from_wide(&buffer[..length as usize]);
    let expected = expected_executable
        .canonicalize()
        .map_err(|_| UpdaterError::InstallRootInvalid("parent executable is missing".into()))?;
    let actual = Path::new(&actual)
        .canonicalize()
        .map_err(|_| UpdaterError::NetworkFailure("parent image path is unavailable".into()))?;
    if !actual
        .to_string_lossy()
        .eq_ignore_ascii_case(&expected.to_string_lossy())
    {
        return Err(UpdaterError::InstallRootInvalid(
            "parent process is not the canonical installed app".into(),
        ));
    }
    Ok(())
}
