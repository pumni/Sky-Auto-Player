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
    emergency_backup: PathBuf,
    expected_hash: String,
    label: String,
    cleanup_temporary: bool,
    cleanup_backup: bool,
}

impl Drop for PreparedReplacement {
    fn drop(&mut self) {
        if self.cleanup_temporary {
            let _ = fs::remove_file(&self.temporary);
        }
        if self.cleanup_backup {
            let _ = fs::remove_file(&self.emergency_backup);
        }
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

/// Remove only temporary names reserved by this replacement primitive after a
/// committed or fully recovered transaction.  A killed process can leave an
/// emergency backup beside its destination after ReplaceFileW has succeeded.
pub fn cleanup_stale_artifacts(install_root: &Path) -> io::Result<()> {
    cleanup_stale_artifacts_in(install_root)
}

fn cleanup_stale_artifacts_in(directory: &Path) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(is_reserved_artifact_name)
        {
            fs::remove_file(entry.path())?;
        } else if file_type.is_dir() {
            let path = entry.path();
            if is_reparse_point(&path)? {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.eq_ignore_ascii_case("songs")
                && !name.eq_ignore_ascii_case("logs")
                && !name.eq_ignore_ascii_case(".sky-update-transaction")
            {
                cleanup_stale_artifacts_in(&path)?;
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(path: &Path) -> io::Result<bool> {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    Ok(fs::symlink_metadata(path)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

#[cfg(not(windows))]
fn is_reparse_point(_path: &Path) -> io::Result<bool> {
    Ok(false)
}

fn is_reserved_artifact_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(".sky-update-") else {
        return false;
    };
    let (rest, suffix) = if let Some(rest) = rest.strip_suffix("-reconcile.bak") {
        (rest, "bak")
    } else if let Some(rest) = rest.strip_suffix(".tmp") {
        (rest, "tmp")
    } else if let Some(rest) = rest.strip_suffix(".bak") {
        (rest, "bak")
    } else {
        return false;
    };
    let mut numbers = rest.split('-');
    numbers
        .next()
        .is_some_and(|value| !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()))
        && numbers
            .next()
            .is_some_and(|value| !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()))
        && numbers.next().is_none()
        && matches!(suffix, "tmp" | "bak")
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
    let emergency_backup = parent.join(format!(
        ".sky-update-{}-{}.bak",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    if emergency_backup.exists() {
        return Err(UpdaterError::InstallAtomicReplaceFailed {
            path: label.into(),
            os_code: 183,
            message: "emergency backup name already exists".into(),
        });
    }
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
        emergency_backup,
        expected_hash: expected_hash.to_ascii_lowercase(),
        label: label.into(),
        cleanup_temporary: true,
        cleanup_backup: true,
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
    let mut replacement = replacement;
    match replacement.commit_existing() {
        Ok(()) => {
            after_replace(phase, &replacement.label)?;
            Ok(())
        }
        Err(error) => {
            let reconciliation = replacement.reconcile_after_failure();
            Err(atomic_failure(
                &replacement.label,
                os_code(&error),
                &format!("atomic replacement failed: {error}; {reconciliation}"),
            ))
        }
    }
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
    let mut replacement = replacement;
    atomic_move_new(&replacement.temporary, &replacement.destination).map_err(|error| {
        atomic_failure(
            &replacement.label,
            os_code(&error),
            &format!("atomic install failed: {error}"),
        )
    })?;
    replacement.verify_destination()?;
    replacement.cleanup_temporary = true;
    replacement.cleanup_backup = true;
    after_replace(phase, &replacement.label)
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
    let mut replacement = replacement;
    match replacement.commit_existing() {
        Ok(()) => {
            after_restore(&replacement.label)?;
            Ok(())
        }
        Err(error) => {
            let reconciliation = replacement.reconcile_after_failure();
            Err(UpdaterError::RollbackAtomicReplaceFailed {
                path: replacement.label.clone(),
                os_code: os_code(&error),
                message: format!("atomic restore failed: {error}; {reconciliation}"),
            })
        }
    }
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
    let mut replacement = replacement;
    atomic_move_new(&replacement.temporary, &replacement.destination).map_err(|error| {
        UpdaterError::RollbackAtomicReplaceFailed {
            path: replacement.label.clone(),
            os_code: os_code(&error),
            message: format!("atomic restore install failed: {error}"),
        }
    })?;
    replacement.verify_destination().map_err(|error| {
        UpdaterError::RollbackAtomicReplaceFailed {
            path: replacement.label.clone(),
            os_code: os_code(&error),
            message: format!("restored hash verification failed: {error}"),
        }
    })?;
    replacement.cleanup_temporary = true;
    replacement.cleanup_backup = true;
    after_restore(&replacement.label)
}

impl PreparedReplacement {
    fn commit_existing(&mut self) -> io::Result<()> {
        atomic_replace_paths(
            &self.temporary,
            &self.destination,
            Some(&self.emergency_backup),
        )?;
        self.verify_destination().map_err(|error| {
            io::Error::other(format!("replacement hash verification failed: {error}"))
        })?;
        self.cleanup_temporary = true;
        self.cleanup_backup = true;
        Ok(())
    }

    fn verify_destination(&self) -> io::Result<()> {
        if !self.destination.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "canonical destination is missing after replacement",
            ));
        }
        let actual =
            sha256_file(&self.destination).map_err(|error| io::Error::other(error.to_string()))?;
        if actual != self.expected_hash {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "canonical destination hash does not match prepared replacement",
            ));
        }
        Ok(())
    }

    /// Reconcile a failed ReplaceFileW call before Drop can clean anything.
    ///
    /// The emergency backup and temporary replacement stay on disk whenever
    /// the filesystem state is ambiguous.  Cleanup is enabled only after a
    /// canonical destination has been observed or re-established.
    fn reconcile_after_failure(&mut self) -> String {
        self.cleanup_temporary = false;
        self.cleanup_backup = false;

        if self.destination.is_file() {
            if self.verify_destination().is_ok() {
                self.cleanup_temporary = true;
                self.cleanup_backup = true;
                return "destination contains the verified replacement".into();
            }

            if self.emergency_backup.is_file() {
                let recovery_backup = self.destination.with_file_name(format!(
                    ".sky-update-{}-{}-reconcile.bak",
                    std::process::id(),
                    TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
                ));
                let reconciliation = atomic_replace_paths(
                    &self.emergency_backup,
                    &self.destination,
                    Some(&recovery_backup),
                );
                match reconciliation {
                    Ok(()) if self.destination.is_file() => {
                        let _ = fs::remove_file(&recovery_backup);
                        self.cleanup_temporary = true;
                        self.cleanup_backup = true;
                        return "destination restored from emergency backup".into();
                    }
                    Ok(()) | Err(_) if !self.destination.is_file() => {
                        for backup in [&self.emergency_backup, &recovery_backup] {
                            if backup.is_file()
                                && atomic_move_new(backup, &self.destination).is_ok()
                                && self.destination.is_file()
                            {
                                self.cleanup_temporary = true;
                                self.cleanup_backup = true;
                                return "destination restored after replacement reconciliation"
                                    .into();
                            }
                        }
                    }
                    Ok(()) | Err(_) => {}
                }
            }

            // The original destination is still present.  Preserve any
            // emergency artifacts, but it is safe to remove the unused temp.
            self.cleanup_temporary = true;
            return "canonical destination remains; emergency artifacts preserved".into();
        }

        if self.emergency_backup.is_file()
            && atomic_move_new(&self.emergency_backup, &self.destination).is_ok()
            && self.destination.is_file()
        {
            self.cleanup_temporary = true;
            self.cleanup_backup = true;
            return "destination restored from emergency backup".into();
        }

        if self.temporary.is_file()
            && self.verify_temporary().is_ok()
            && atomic_move_new(&self.temporary, &self.destination).is_ok()
            && self.destination.is_file()
        {
            self.cleanup_temporary = true;
            self.cleanup_backup = true;
            return "destination established from verified temporary replacement".into();
        }

        "canonical destination could not be re-established; artifacts preserved".into()
    }

    fn verify_temporary(&self) -> io::Result<()> {
        let actual =
            sha256_file(&self.temporary).map_err(|error| io::Error::other(error.to_string()))?;
        if actual == self.expected_hash {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "temporary replacement hash mismatch",
            ))
        }
    }
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
    crate::faults::pause_at(&format!("before-replace:{phase}:{path}"));
    #[cfg(feature = "e2e-fault-injection")]
    crate::faults::pause_at(&format!("before-replace:{phase}:{index}"));
    #[cfg(feature = "e2e-fault-injection")]
    crate::faults::before_replace(phase, index, path)?;
    let _ = (phase, index, path);
    Ok(())
}

fn after_replace(phase: &str, path: &str) -> Result<()> {
    #[cfg(feature = "e2e-fault-injection")]
    {
        crate::faults::pause_at(&format!("after-replace:{phase}:{path}"));
        crate::faults::after_replace(phase, path)?;
    }
    let _ = (phase, path);
    Ok(())
}

fn after_restore(path: &str) -> Result<()> {
    #[cfg(feature = "e2e-fault-injection")]
    {
        crate::faults::pause_at(&format!("after-restore:rollback:{path}"));
        crate::faults::after_restore(path)?;
    }
    let _ = path;
    Ok(())
}

#[cfg(windows)]
fn probe_windows_handle(path: &Path, label: &str) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, INVALID_HANDLE_VALUE};
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
            GENERIC_READ
                | windows_sys::Win32::Storage::FileSystem::DELETE
                | windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE,
            windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ
                | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_WRITE
                | windows_sys::Win32::Storage::FileSystem::FILE_SHARE_DELETE,
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
fn atomic_replace_paths(
    temporary: &Path,
    destination: &Path,
    backup: Option<&Path>,
) -> io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::ReplaceFileW;
    let temporary = wide(temporary);
    let destination = wide(destination);
    let backup = backup.map(wide);
    if unsafe {
        ReplaceFileW(
            destination.as_ptr(),
            temporary.as_ptr(),
            backup
                .as_ref()
                .map_or(std::ptr::null(), |path| path.as_ptr()),
            0,
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
fn atomic_replace_paths(
    temporary: &Path,
    destination: &Path,
    _backup: Option<&Path>,
) -> io::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::sha256_bytes;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sky-updater-reconcile-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn prepared(
        root: &Path,
        destination: &str,
        temporary: &str,
        emergency_backup: &str,
        expected: &[u8],
    ) -> PreparedReplacement {
        PreparedReplacement {
            temporary: root.join(temporary),
            destination: root.join(destination),
            emergency_backup: root.join(emergency_backup),
            expected_hash: sha256_bytes(expected),
            label: destination.into(),
            cleanup_temporary: false,
            cleanup_backup: false,
        }
    }

    #[test]
    fn reconcile_missing_destination_prefers_verified_emergency_backup() {
        let root = temp_root("missing-destination");
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join("replacement.tmp"), b"new bytes").expect("temp");
        fs::write(root.join("replacement.bak"), b"old bytes").expect("backup");
        let destination = root.join("managed.dll");
        let mut replacement = prepared(
            &root,
            "managed.dll",
            "replacement.tmp",
            "replacement.bak",
            b"new bytes",
        );

        let message = replacement.reconcile_after_failure();

        assert!(message.contains("emergency backup"));
        assert_eq!(fs::read(&destination).expect("destination"), b"old bytes");
        drop(replacement);
        assert!(!root.join("replacement.tmp").exists());
        assert!(!root.join("replacement.bak").exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn reconcile_wrong_destination_restores_emergency_backup() {
        let root = temp_root("wrong-destination");
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join("managed.dll"), b"wrong bytes").expect("destination");
        fs::write(root.join("replacement.tmp"), b"new bytes").expect("temp");
        fs::write(root.join("replacement.bak"), b"old bytes").expect("backup");
        let mut replacement = prepared(
            &root,
            "managed.dll",
            "replacement.tmp",
            "replacement.bak",
            b"new bytes",
        );

        let message = replacement.reconcile_after_failure();

        assert!(message.contains("restored"));
        assert_eq!(
            fs::read(root.join("managed.dll")).expect("destination"),
            b"old bytes"
        );
        drop(replacement);
        assert!(!root.join("replacement.tmp").exists());
        assert!(!root.join("replacement.bak").exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn reconcile_verified_destination_cleans_duplicate_backup() {
        let root = temp_root("verified-destination");
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join("managed.dll"), b"new bytes").expect("destination");
        fs::write(root.join("replacement.tmp"), b"new bytes").expect("temp");
        fs::write(root.join("replacement.bak"), b"old bytes").expect("backup");
        let mut replacement = prepared(
            &root,
            "managed.dll",
            "replacement.tmp",
            "replacement.bak",
            b"new bytes",
        );

        let message = replacement.reconcile_after_failure();

        assert!(message.contains("verified replacement"));
        drop(replacement);
        assert_eq!(
            fs::read(root.join("managed.dll")).expect("destination"),
            b"new bytes"
        );
        assert!(!root.join("replacement.tmp").exists());
        assert!(!root.join("replacement.bak").exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn reconcile_missing_destination_can_establish_from_verified_temp() {
        let root = temp_root("temp-only");
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join("replacement.tmp"), b"new bytes").expect("temp");
        let destination = root.join("managed.dll");
        let mut replacement = prepared(
            &root,
            "managed.dll",
            "replacement.tmp",
            "replacement.bak",
            b"new bytes",
        );

        let message = replacement.reconcile_after_failure();

        assert!(message.contains("verified temporary"));
        assert_eq!(fs::read(&destination).expect("destination"), b"new bytes");
        drop(replacement);
        assert!(!root.join("replacement.tmp").exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn stale_artifact_cleanup_recurses_without_touching_preserved_directories() {
        let root = temp_root("recursive-cleanup");
        let nested = root.join("_internal").join("bin");
        fs::create_dir_all(&nested).expect("nested");
        fs::write(nested.join(".sky-update-7-8.tmp"), b"stale").expect("nested stale");

        let preserved = ["songs", "Songs", "SONGS", "logs", "Logs", "LOGS"];
        for (index, name) in preserved.iter().enumerate() {
            let directory = root.join(format!("case-{index}")).join(name);
            fs::create_dir_all(&directory).expect("preserved directory");
            fs::write(
                directory.join(format!(".sky-update-{index}-{index}.tmp")),
                b"user file",
            )
            .expect("preserved file");
        }

        cleanup_stale_artifacts(&root).expect("cleanup");

        assert!(!nested.join(".sky-update-7-8.tmp").exists());
        for (index, name) in preserved.iter().enumerate() {
            let directory = root.join(format!("case-{index}")).join(name);
            assert!(
                directory
                    .join(format!(".sky-update-{index}-{index}.tmp"))
                    .exists(),
                "preserved path was cleaned: {name}"
            );
        }
        fs::remove_dir_all(root).expect("cleanup");
    }
}
