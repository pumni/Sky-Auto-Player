//! Per-install updater serialization.
//!
//! The handle is intentionally held by RAII for the whole runner lifecycle.
//! On Windows the file remains as a stable name after the handle closes; the
//! kernel handle, not file deletion, is the lock state.

#[cfg(not(windows))]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use crate::archive::sha256_bytes;
use crate::error::{Result, UpdaterError};

#[derive(Debug)]
pub struct UpdateLock {
    #[cfg(windows)]
    _file: File,
    #[cfg(not(windows))]
    path: PathBuf,
    path_display: PathBuf,
}

impl UpdateLock {
    pub fn acquire(install_root: &Path) -> Result<Self> {
        let canonical = install_root
            .canonicalize()
            .map_err(|error| UpdaterError::InstallRootInvalid(error.to_string()))?;
        if !canonical.is_dir() {
            return Err(UpdaterError::InstallRootInvalid(
                "install root must be a directory".into(),
            ));
        }
        let local_app_data = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
            UpdaterError::InstallRootInvalid("LOCALAPPDATA is unavailable".into())
        })?;
        let lock_dir = PathBuf::from(local_app_data)
            .join(crate::APP_NAME)
            .join("update-locks");
        if !lock_dir.is_absolute() {
            return Err(UpdaterError::InstallRootInvalid(
                "LOCALAPPDATA must be absolute".into(),
            ));
        }
        fs::create_dir_all(&lock_dir)?;
        let identity = canonical
            .to_string_lossy()
            .replace('/', "\\")
            .to_ascii_lowercase();
        let lock_path = lock_dir.join(format!("{}.lock", sha256_bytes(identity.as_bytes())));

        #[cfg(windows)]
        {
            let file = open_windows_exclusive(&lock_path)?;
            Ok(Self {
                _file: file,
                path_display: lock_path,
            })
        }
        #[cfg(not(windows))]
        {
            let _file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&lock_path)
                .map_err(|error| {
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        UpdaterError::UpdateAlreadyRunning
                    } else {
                        UpdaterError::Io(error)
                    }
                })?;
            Ok(Self {
                path: lock_path.clone(),
                path_display: lock_path,
            })
        }
    }

    pub fn path(&self) -> &Path {
        &self.path_display
    }
}

#[cfg(not(windows))]
impl Drop for UpdateLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(windows)]
fn open_windows_exclusive(path: &Path) -> Result<File> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_ALWAYS,
    };

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(32) {
            return Err(UpdaterError::UpdateAlreadyRunning);
        }
        return Err(UpdaterError::Io(error));
    }
    // SAFETY: CreateFileW returned a valid owned handle and File closes it.
    Ok(unsafe { File::from_raw_handle(handle as _) })
}
