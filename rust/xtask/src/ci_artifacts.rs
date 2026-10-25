#![forbid(unsafe_code)]

use crate::Result;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn validate_commit_sha(value: &str, name: &str) -> Result<String> {
    if value.len() != 40
        || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        || value.bytes().all(|byte| byte == b'0')
    {
        return Err(format!("{name} must be a non-zero 40-character commit SHA").into());
    }
    Ok(value.to_ascii_lowercase())
}

pub(crate) fn validate_digest(value: &str, name: &str) -> Result<String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{name} must be a SHA-256 digest").into());
    }
    Ok(value.to_ascii_lowercase())
}

pub(crate) fn validate_semver(value: &str, name: &str) -> Result<()> {
    let parsed = crate::version::parse(value)
        .map_err(|error| format!("{name} is not canonical SemVer: {error}"))?;
    if parsed.to_string() != value {
        return Err(format!("{name} is not canonical SemVer: {value}").into());
    }
    Ok(())
}

pub(crate) fn validate_safe_filename(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.encode_utf16().count() > 260
        || value.starts_with('/')
        || value.starts_with('\\')
        || value
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':' | '\0'))
        || value.contains("..")
        || value == "."
        || value == ".."
    {
        return Err(format!("{field} is not a safe artifact-relative filename: {value}").into());
    }
    Ok(())
}

pub(crate) fn is_reparse_point(path: &Path, metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        let _ = path;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        false
    }
}

pub(crate) fn require_directory(path: &Path, missing_message: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!("{missing_message}: {}", path.display())
        } else {
            format!("could not inspect directory {}: {error}", path.display())
        }
    })?;
    if !metadata.is_dir() || is_reparse_point(path, &metadata) {
        return Err(format!("{} is not a regular directory", path.display()).into());
    }
    Ok(())
}

pub(crate) fn require_regular_file(path: &Path, description: &str) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "{description} is missing or unavailable at {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() || is_reparse_point(path, &metadata) {
        return Err(format!("{description} must be a regular file: {}", path.display()).into());
    }
    Ok(metadata)
}

pub(crate) fn direct_files(root: &Path, missing_message: &str) -> Result<Vec<PathBuf>> {
    require_directory(root, missing_message)?;
    let mut files = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.is_dir() || is_reparse_point(&path, &metadata) {
            return Err(format!(
                "artifact directory contains an unexpected directory or reparse point: {}",
                entry.file_name().to_string_lossy()
            )
            .into());
        }
        if !metadata.is_file() {
            return Err(format!(
                "artifact directory contains a non-file entry: {}",
                entry.file_name().to_string_lossy()
            )
            .into());
        }
        if entry.file_name().to_str().is_none() {
            return Err("artifact directory contains a non-Unicode filename".into());
        }
        files.push(path);
    }
    files.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    Ok(files)
}

pub(crate) fn sha256_file(path: &Path) -> Result<String> {
    require_regular_file(path, "artifact file")?;
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(crate::repo::hex_digest(digest.finalize()))
}

pub(crate) fn pe_machine(path: &Path, description: &str) -> Result<u16> {
    require_regular_file(path, description)?;
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    if size < 0x40 {
        return Err(format!("{description} is not a complete PE file").into());
    }
    let mut dos = [0_u8; 2];
    file.read_exact(&mut dos)?;
    if &dos != b"MZ" {
        return Err(format!("{description} is missing the DOS header").into());
    }
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0x3c))?;
    let mut offset_bytes = [0_u8; 4];
    file.read_exact(&mut offset_bytes)?;
    let offset = i32::from_le_bytes(offset_bytes);
    if offset < 0x40 || offset as u64 > size.saturating_sub(6) {
        return Err(format!("{description} has an invalid PE header offset").into());
    }
    file.seek(SeekFrom::Start(offset as u64))?;
    let mut header = [0_u8; 6];
    file.read_exact(&mut header)?;
    if &header[..4] != b"PE\0\0" {
        return Err(format!("{description} is missing the PE signature").into());
    }
    Ok(u16::from_le_bytes([header[4], header[5]]))
}

pub(crate) fn validate_public_key(path: &Path) -> Result<String> {
    let metadata = require_regular_file(path, "candidate public key")?;
    if metadata.len() == 0 || metadata.len() > 4096 {
        return Err("candidate public key is empty or unbounded".into());
    }
    let value = fs::read_to_string(path)
        .map_err(|error| format!("candidate public key is not UTF-8 text: {error}"))?;
    let value = value.trim();
    if value.is_empty() || value.to_ascii_uppercase().contains("PRIVATE KEY") {
        return Err("candidate public key is missing or contains private key material".into());
    }
    if !value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(byte, b'+' | b'/' | b'=' | b'\r' | b'\n' | b'\t' | b' ')
    }) {
        return Err("candidate public key contains unexpected material".into());
    }
    Ok(value.to_owned())
}

pub(crate) fn unique_sibling(path: &Path, suffix: &str) -> PathBuf {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    parent.join(format!(
        ".{name}.{suffix}.{}.{time}.{sequence}",
        std::process::id()
    ))
}

pub(crate) struct StagingDirectory {
    path: PathBuf,
    committed: bool,
}

impl StagingDirectory {
    pub(crate) fn create(destination: &Path) -> Result<Self> {
        if fs::symlink_metadata(destination).is_ok() {
            return Err(
                format!("output directory already exists: {}", destination.display()).into(),
            );
        }
        let parent = destination
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)?;
        for _ in 0..8 {
            let path = unique_sibling(destination, "staging");
            match fs::create_dir(&path) {
                Ok(()) => {
                    return Ok(Self {
                        path,
                        committed: false,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err("could not allocate a unique artifact staging directory".into())
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn commit(mut self, destination: &Path) -> Result<()> {
        if fs::symlink_metadata(destination).is_ok() {
            return Err(
                format!("output directory already exists: {}", destination.display()).into(),
            );
        }
        fs::rename(&self.path, destination)?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub(crate) fn replace_file_with_rollback(staged: &Path, destination: &Path) -> Result<()> {
    let had_destination = match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.is_file() && !is_reparse_point(destination, &metadata) => true,
        Ok(_) => {
            return Err(format!(
                "replacement target must be a regular file: {}",
                destination.display()
            )
            .into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    let backup = unique_sibling(destination, "backup");
    if had_destination {
        fs::rename(destination, &backup)?;
    }
    if let Err(error) = fs::rename(staged, destination) {
        if had_destination && let Err(restore_error) = fs::rename(&backup, destination) {
            return Err(format!(
                "replacement failed ({error}) and original file restoration failed ({restore_error})"
            )
            .into());
        }
        return Err(error.into());
    }
    if had_destination && let Err(error) = fs::remove_file(&backup) {
        let rollback = fs::remove_file(destination).and_then(|()| fs::rename(&backup, destination));
        if let Err(restore_error) = rollback {
            return Err(format!(
                "replacement cleanup failed ({error}) and original file restoration failed ({restore_error})"
            )
            .into());
        }
        return Err(error.into());
    }
    Ok(())
}

pub(crate) fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(name: &str) -> PathBuf {
        let path = unique_sibling(&std::env::temp_dir().join(name), "test");
        fs::create_dir(&path).unwrap();
        path
    }

    fn valid_pe(machine: u16) -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x86];
        bytes[0..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_i32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        bytes
    }

    #[test]
    fn pe_reader_accepts_x64_and_rejects_corruption_and_wrong_machine() {
        let root = temp_dir("sky-xtask-pe-test");
        let path = root.join("native_calibration.exe");
        fs::write(&path, valid_pe(0x8664)).unwrap();
        assert_eq!(pe_machine(&path, "test PE").unwrap(), 0x8664);
        fs::write(&path, valid_pe(0x014c)).unwrap();
        assert_eq!(pe_machine(&path, "test PE").unwrap(), 0x014c);
        let mut corrupt = valid_pe(0x8664);
        corrupt[0x80] = b'X';
        fs::write(&path, corrupt).unwrap();
        assert!(pe_machine(&path, "test PE").is_err());
        fs::write(&path, b"not a PE").unwrap();
        assert!(pe_machine(&path, "test PE").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn direct_inventory_rejects_directories_and_symlinks() {
        let root = temp_dir("sky-xtask-inventory-test");
        fs::write(root.join("one.bin"), b"one").unwrap();
        assert_eq!(direct_files(&root, "missing").unwrap().len(), 1);
        fs::create_dir(root.join("nested")).unwrap();
        assert!(direct_files(&root, "missing").is_err());
        fs::remove_dir(root.join("nested")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("one.bin"), root.join("link.bin")).unwrap();
            assert!(direct_files(&root, "missing").is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn staging_directory_is_removed_after_an_error() {
        let root = temp_dir("sky-xtask-staging-test");
        let output = root.join("candidate");
        let staged_path;
        {
            let staging = StagingDirectory::create(&output).unwrap();
            staged_path = staging.path().to_owned();
            fs::write(staging.path().join("partial.json"), b"partial").unwrap();
            assert!(direct_files(staging.path(), "missing").is_ok());
            // Simulate any subsequent validation or artifact-write failure.
        }
        assert!(!staged_path.exists());
        assert!(!output.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn replacement_restores_the_original_when_staging_rename_fails() {
        let root = temp_dir("sky-xtask-replace-test");
        let destination = root.join("native_calibration.exe");
        fs::write(&destination, b"original").unwrap();
        let missing_stage = root.join("missing-stage.exe");
        assert!(replace_file_with_rollback(&missing_stage, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);

        let staged = root.join("staged.exe");
        let mut file = File::create(&staged).unwrap();
        file.write_all(b"replacement").unwrap();
        file.sync_all().unwrap();
        replace_file_with_rollback(&staged, &destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"replacement");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
