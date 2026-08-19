use std::io;
use std::path::{Path, PathBuf};

#[cfg(not(windows))]
use std::process::Command;

use crate::error::{Result, UpdaterError};
use crate::install::installed_manifest;
use crate::signature::verify_project_files;
use crate::transaction::safe_join;
use crate::{APP_NAME, PRIMARY_EXE};

pub fn restart_verified(install_root: &Path) -> Result<()> {
    let executable = safe_join(install_root, PRIMARY_EXE)?;
    if !executable.is_file()
        || executable.file_name().and_then(|name| name.to_str()) != Some(PRIMARY_EXE)
    {
        return Err(UpdaterError::RestartFailed(
            "canonical app executable is missing".into(),
        ));
    }
    let manifest = installed_manifest(install_root).map_err(|error| {
        UpdaterError::RestartFailed(format!("installed manifest is not valid: {error}"))
    })?;
    verify_project_files(install_root, &manifest).map_err(|error| {
        UpdaterError::RestartFailed(format!(
            "installed project files failed integrity check: {error}"
        ))
    })?;
    launch_verified_process(&executable, install_root).map_err(|err| {
        UpdaterError::RestartFailed(format!("{APP_NAME} could not be started: {err}"))
    })
}

#[cfg(windows)]
#[derive(Debug, Eq, PartialEq)]
struct ProcessLaunchSpec {
    application_name: PathBuf,
    current_directory: PathBuf,
    creation_flags: u32,
    inherit_handles: bool,
}

#[cfg(windows)]
fn process_launch_spec(executable: &Path, install_root: &Path) -> ProcessLaunchSpec {
    ProcessLaunchSpec {
        application_name: executable.to_owned(),
        current_directory: install_root.to_owned(),
        creation_flags: windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE,
        inherit_handles: false,
    }
}

#[cfg(windows)]
fn launch_verified_process(executable: &Path, install_root: &Path) -> io::Result<()> {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, PROCESS_INFORMATION, STARTUPINFOW,
    };

    let spec = process_launch_spec(executable, install_root);
    let application_name = spec
        .application_name
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    let current_directory = spec
        .current_directory
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    let mut startup_info: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup_info.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut process_info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    // The updater is deliberately launched without a console and with all
    // standard handles redirected. Start the verified app in a fresh console
    // instead of inheriting that hidden/non-interactive handle environment.
    let created = unsafe {
        CreateProcessW(
            application_name.as_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            spec.inherit_handles as i32,
            spec.creation_flags,
            std::ptr::null_mut(),
            current_directory.as_ptr(),
            &startup_info,
            &mut process_info,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }

    unsafe {
        CloseHandle(process_info.hProcess);
        CloseHandle(process_info.hThread);
    }
    Ok(())
}

#[cfg(not(windows))]
fn launch_verified_process(executable: &Path, install_root: &Path) -> io::Result<()> {
    Command::new(executable)
        .current_dir(install_root)
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_canonical_executable_is_rejected_before_launch() {
        let root = std::env::temp_dir().join(format!(
            "sky-updater-restart-verification-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("restart fixture root");

        let error = restart_verified(&root).expect_err("missing canonical executable");
        assert!(matches!(
            error,
            UpdaterError::RestartFailed(message)
                if message == "canonical app executable is missing"
        ));

        std::fs::remove_dir_all(root).expect("cleanup restart fixture");
    }

    #[cfg(windows)]
    #[test]
    fn windows_restart_uses_fresh_console_without_inherited_handles() {
        let executable = PathBuf::from(r"C:\install\Sky-Auto-Player.exe");
        let install_root = PathBuf::from(r"C:\install");
        let spec = process_launch_spec(&executable, &install_root);

        assert_eq!(spec.application_name, executable);
        assert_eq!(spec.current_directory, install_root);
        assert_eq!(
            spec.creation_flags,
            windows_sys::Win32::System::Threading::CREATE_NEW_CONSOLE
        );
        assert!(!spec.inherit_handles);
    }
}
