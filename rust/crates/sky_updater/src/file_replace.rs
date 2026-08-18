//! Same-volume, prepared-file replacement primitives.
//!
//! Every managed-file write is staged beside its destination, flushed, hashed,
//! and then committed with an atomic Windows replacement.  The destination is
//! never removed as part of this operation.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::archive::sha256_file;
use crate::error::{Result, UpdaterError};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct PreparedReplacement {
    temporary: PathBuf,
    destination: PathBuf,
    label: String,
}

impl Drop for PreparedReplacement {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.temporary);
    }
}

/// Prove that an existing regular file can be replaced or deleted.
pub fn probe_replaceable(path: &Path, label: &str) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(target_busy(label, &error)),
    };
    if !metadata.file_type().is_file() {
        return Err(UpdaterError::InstallTargetBusy {
            path: label.into(),
            os_code: 1,
            message: "destination is not a regular file".into(),
        });
    }

    #[cfg(windows)]
    probe_windows_handle(path, label)?;
    Ok(true)
}

/// A package addition must not overwrite an unknown existing file.
pub fn probe_new_destination(path: &Path, label: &str) -> Result<()> {
    if probe_replaceable(path, label)? {
        return Err(UpdaterError::InstallTargetBusy {
            path: label.into(),
            os_code: 183,
            message: "new managed destination already exists".into(),
        });
    }
    Ok(())
}

pub fn prepare_replacement(
    source: &Path,
    destination: &Path,
    expected_hash: &str,
    label: &str,
) -> Result<PreparedReplacement> {
    if !source.is_file() {
        return Err(UpdaterError::InstallCopyFailed(format!(
            "missing staged file: {label}"
        )));
    }
    let parent = destination.parent().ok_or_else(|| {
        UpdaterError::InstallCopyFailed(format!("destination has no parent: {label}"))
    })?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".sky-update-{}-{}.tmp",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| UpdaterError::InstallAtomicReplaceFailed {
            path: label.into(),
            os_code: os_code(&error),
            message: format!("could not create same-volume temporary: {error}"),
        })?;
    if let Err(error) = copy_and_flush(&mut input, &mut output) {
        let _ = fs::remove_file(&temporary);
        return Err(UpdaterError::InstallAtomicReplaceFailed {
            path: label.into(),
            os_code: os_code(&error),
            message: format!("could not stage replacement: {error}"),
        });
    }
    if sha256_file(&temporary)? != expected_hash.to_ascii_lowercase() {
        let _ = fs::remove_file(&temporary);
        return Err(UpdaterError::InstallAtomicReplaceFailed {
            path: label.into(),
            os_code: 0,
            message: "staged replacement hash mismatch".into(),
        });
    }
    Ok(PreparedReplacement {
        temporary,
        destination: destination.to_owned(),
        label: label.into(),
    })
}

pub fn atomic_replace_existing(
    replacement: PreparedReplacement,
    phase: &str,
    index: usize,
) -> Result<()> {
    before_replace(phase, index, &replacement.label)?;
    if !replacement.destination.is_file() {
        return Err(atomic_failure(
            &replacement.label,
            2,
            "existing destination disappeared before atomic replace",
        ));
    }
    atomic_replace_paths(&replacement.temporary, &replacement.destination).map_err(|error| {
        atomic_failure(
            &replacement.label,
            os_code(&error),
            &format!("atomic replacement failed: {error}"),
        )
    })
}

pub fn atomic_install_new(
    replacement: PreparedReplacement,
    phase: &str,
    index: usize,
) -> Result<()> {
    before_replace(phase, index, &replacement.label)?;
    if replacement.destination.exists() {
        return Err(atomic_failure(
            &replacement.label,
            183,
            "new destination appeared before atomic install",
        ));
    }
    atomic_move_new(&replacement.temporary, &replacement.destination).map_err(|error| {
        atomic_failure(
            &replacement.label,
            os_code(&error),
            &format!("atomic install failed: {error}"),
        )
    })
}

pub fn atomic_restore(
    source: &Path,
    destination: &Path,
    expected_hash: &str,
    label: &str,
    index: usize,
) -> Result<()> {
    let replacement = prepare_restore(source, destination, expected_hash, label)?;
    if replacement.destination.is_file() {
        atomic_restore_existing(replacement, index)
    } else {
        atomic_restore_new(replacement, index)
    }
}

fn prepare_restore(
    source: &Path,
    destination: &Path,
    expected_hash: &str,
    label: &str,
) -> Result<PreparedReplacement> {
    prepare_replacement(source, destination, expected_hash, label).map_err(|error| match error {
        UpdaterError::InstallAtomicReplaceFailed {
            path,
            os_code,
            message,
        } => UpdaterError::RollbackAtomicReplaceFailed {
            path,
            os_code,
            message,
        },
        other => other,
    })
}

fn atomic_restore_existing(replacement: PreparedReplacement, index: usize) -> Result<()> {
    before_replace("rollback", index, &replacement.label).map_err(|error| match error {
        UpdaterError::InstallCopyFailed(message) => UpdaterError::RollbackAtomicReplaceFailed {
            path: replacement.label.clone(),
            os_code: 0,
            message,
        },
        other => other,
    })?;
    atomic_replace_paths(&replacement.temporary, &replacement.destination).map_err(|error| {
        UpdaterError::RollbackAtomicReplaceFailed {
            path: replacement.label.clone(),
            os_code: os_code(&error),
            message: format!("atomic restore failed: {error}"),
        }
    })
}

fn atomic_restore_new(replacement: PreparedReplacement, index: usize) -> Result<()> {
    before_replace("rollback", index, &replacement.label).map_err(|error| match error {
        UpdaterError::InstallCopyFailed(message) => UpdaterError::RollbackAtomicReplaceFailed {
            path: replacement.label.clone(),
            os_code: 0,
            message,
        },
        other => other,
    })?;
    atomic_move_new(&replacement.temporary, &replacement.destination).map_err(|error| {
        UpdaterError::RollbackAtomicReplaceFailed {
            path: replacement.label.clone(),
            os_code: os_code(&error),
            message: format!("atomic restore install failed: {error}"),
        }
    })
}

fn copy_and_flush(input: &mut File, output: &mut File) -> io::Result<()> {
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
    }
    output.flush()?;
    output.sync_all()
}

fn atomic_failure(path: &str, os_code: u32, message: &str) -> UpdaterError {
    UpdaterError::InstallAtomicReplaceFailed {
        path: path.into(),
        os_code,
        message: message.into(),
    }
}

fn target_busy(path: &str, error: &io::Error) -> UpdaterError {
    UpdaterError::InstallTargetBusy {
        path: path.into(),
        os_code: os_code(error),
        message: error.to_string(),
    }
}

fn os_code(error: &io::Error) -> u32 {
    error.raw_os_error().unwrap_or(1).unsigned_abs()
}

fn before_replace(phase: &str, index: usize, path: &str) -> Result<()> {
    #[cfg(feature = "e2e-fault-injection")]
    crate::faults::pause_at(&format!("before-replace:{phase}:{index}"));
    #[cfg(feature = "e2e-fault-injection")]
    crate::faults::before_replace(phase, index, path)?;
    let _ = (phase, index, path);
    Ok(())
}

#[cfg(windows)]
fn probe_windows_handle(path: &Path, label: &str) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING,
    };

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE | windows_sys::Win32::Storage::FileSystem::DELETE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(target_busy(label, &io::Error::last_os_error()));
    }
    unsafe { CloseHandle(handle) };
    Ok(())
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn atomic_replace_paths(temporary: &Path, destination: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
    let temporary = wide(temporary);
    let destination = wide(destination);
    if unsafe {
        ReplaceFileW(
            destination.as_ptr(),
            temporary.as_ptr(),
            std::ptr::null(),
            1,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace_paths(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary, destination)
}

#[cfg(windows)]
fn atomic_move_new(temporary: &Path, destination: &Path) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
    let temporary = wide(temporary);
    let destination = wide(destination);
    if unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn atomic_move_new(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary, destination)
}
