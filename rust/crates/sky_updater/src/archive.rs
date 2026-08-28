use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::{Component, Path};

use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::error::{Result, UpdaterError, io_context};
use crate::{
    ZIP_MAX_COMPRESSED_BYTES, ZIP_MAX_ENTRIES, ZIP_MAX_ENTRY_BYTES, ZIP_MAX_UNCOMPRESSED_BYTES,
};

use std::fmt::Write as _;

fn format_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        let _ = write!(hex, "{:02x}", b);
    }
    hex
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format_hex(&hasher.finalize())
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format_hex(&hasher.finalize()))
}

pub fn parse_sha_sidecar(bytes: &[u8], expected_zip_name: &str) -> Result<String> {
    if bytes.len() > crate::SIDECAR_MAX_BYTES {
        return Err(UpdaterError::ChecksumInvalid(
            "sidecar exceeds size bound".into(),
        ));
    }
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|_| UpdaterError::ChecksumInvalid("sidecar is not UTF-8".into()))?;
    let records = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>();
    if records.len() != 1 {
        return Err(UpdaterError::ChecksumInvalid(
            "sidecar must contain exactly one meaningful record".into(),
        ));
    }
    let fields = records[0].split_whitespace().collect::<Vec<_>>();
    if fields.len() != 2 {
        return Err(UpdaterError::ChecksumInvalid(
            "sidecar record must have hash and filename".into(),
        ));
    }
    if fields[1] != expected_zip_name
        || fields[0].len() != 64
        || !fields[0].chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return Err(UpdaterError::ChecksumInvalid(
            "sidecar is not bound to the expected ZIP filename".into(),
        ));
    }
    Ok(fields[0].to_ascii_lowercase())
}

pub fn validate_relative_path(raw: &str) -> Result<String> {
    if raw.is_empty()
        || raw.contains('\0')
        || raw.contains('\\')
        || raw.starts_with('/')
        || raw.starts_with("//")
    {
        return Err(UpdaterError::ArchiveUnsafe(format!("unsafe path: {raw:?}")));
    }
    if raw.len() >= 2 && raw.as_bytes()[1] == b':' {
        return Err(UpdaterError::ArchiveUnsafe(format!(
            "drive-qualified path: {raw:?}"
        )));
    }
    let mut normalized = Vec::new();
    for component in raw.split('/') {
        if component.is_empty() {
            return Err(UpdaterError::ArchiveUnsafe(format!(
                "unsafe path component: {raw:?}"
            )));
        }
        if component == "."
            || component == ".."
            || component.ends_with(['.', ' '])
            || component.contains(':')
        {
            return Err(UpdaterError::ArchiveUnsafe(format!(
                "unsafe path component: {raw:?}"
            )));
        }
        if is_reserved_device(component) {
            return Err(UpdaterError::ArchiveUnsafe(format!(
                "reserved device path: {raw:?}"
            )));
        }
        normalized.push(component);
    }
    if normalized.is_empty() {
        return Err(UpdaterError::ArchiveUnsafe(format!(
            "empty relative path: {raw:?}"
        )));
    }
    Ok(normalized.join("/"))
}

fn is_reserved_device(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_uppercase();
    matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "CLOCK$"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

pub fn validate_zip(bytes: &[u8]) -> Result<Vec<String>> {
    validate_zip_reader(std::io::Cursor::new(bytes), bytes.len() as u64)
}

pub fn validate_zip_file(path: &Path) -> Result<Vec<String>> {
    let compressed_size = fs::metadata(path)
        .map_err(|error| io_context("verify archive", "read archive metadata", path, error))?
        .len();
    let file = File::open(path)
        .map_err(|error| io_context("verify archive", "open archive", path, error))?;
    validate_zip_reader(file, compressed_size)
}

fn validate_zip_reader<R: Read + Seek>(reader: R, compressed_size: u64) -> Result<Vec<String>> {
    if compressed_size > ZIP_MAX_COMPRESSED_BYTES {
        return Err(UpdaterError::ArchiveUnsafe(
            "compressed archive exceeds size bound".into(),
        ));
    }
    let mut archive =
        ZipArchive::new(reader).map_err(|err| UpdaterError::ArchiveUnsafe(err.to_string()))?;
    if archive.len() > ZIP_MAX_ENTRIES {
        return Err(UpdaterError::ArchiveUnsafe(
            "archive has too many entries".into(),
        ));
    }
    let mut paths = Vec::new();
    let mut entries = HashMap::<String, ZipEntryKind>::new();
    let mut total = 0u64;
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|err| UpdaterError::ArchiveUnsafe(err.to_string()))?;
        let raw = file.name().to_owned();
        let normalized = validate_relative_path(raw.trim_end_matches('/'))?;
        let kind = if file.is_dir() || raw.ends_with('/') {
            ZipEntryKind::Directory
        } else {
            ZipEntryKind::File
        };
        if file
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(UpdaterError::ArchiveUnsafe(format!("symlink entry: {raw}")));
        }
        if file.size() > ZIP_MAX_ENTRY_BYTES {
            return Err(UpdaterError::ArchiveUnsafe(format!(
                "entry exceeds size bound: {raw}"
            )));
        }
        total = total.saturating_add(file.size());
        if total > ZIP_MAX_UNCOMPRESSED_BYTES {
            return Err(UpdaterError::ArchiveUnsafe(
                "archive exceeds uncompressed size bound".into(),
            ));
        }
        let key = windows_path_key(&normalized);
        if entries.insert(key, kind).is_some() {
            return Err(UpdaterError::ArchiveUnsafe(format!(
                "duplicate/case-colliding path: {raw}"
            )));
        }
        paths.push(normalized);
    }
    for path in &paths {
        let mut current = Path::new(path);
        while let Some(parent) = current.parent() {
            if parent == Path::new("") {
                break;
            }
            let parent_string = parent.to_string_lossy().replace('\\', "/");
            if entries.get(&windows_path_key(&parent_string)) == Some(&ZipEntryKind::File) {
                return Err(UpdaterError::ArchiveUnsafe(format!(
                    "file/directory collision: {path}"
                )));
            }
            current = parent;
        }
    }
    Ok(paths)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ZipEntryKind {
    File,
    Directory,
}

fn windows_path_key(path: &str) -> String {
    path.to_lowercase()
}

pub fn extract_zip(bytes: &[u8], staging: &Path) -> Result<()> {
    let expected_paths = validate_zip(bytes)?;
    fs::create_dir_all(staging)
        .map_err(|error| io_context("extract", "create staging directory", staging, error))?;
    let reader = std::io::Cursor::new(bytes);
    let mut archive =
        ZipArchive::new(reader).map_err(|err| UpdaterError::ArchiveUnsafe(err.to_string()))?;
    for (index, relative) in expected_paths.iter().enumerate() {
        let mut file = archive
            .by_index(index)
            .map_err(|err| UpdaterError::ArchiveUnsafe(err.to_string()))?;
        let destination = staging.join(relative);
        if file.is_dir() || file.name().ends_with('/') {
            fs::create_dir_all(&destination).map_err(|error| {
                io_context("extract", "create child directory", &destination, error)
            })?;
            continue;
        }
        let parent = destination
            .parent()
            .ok_or_else(|| UpdaterError::ArchiveUnsafe(relative.clone()))?;
        fs::create_dir_all(parent)
            .map_err(|error| io_context("extract", "create staged parent", parent, error))?;
        let mut output = File::create(&destination)
            .map_err(|error| io_context("extract", "create staged file", &destination, error))?;
        std::io::copy(&mut file, &mut output)
            .map_err(|error| io_context("extract", "copy archive entry", &destination, error))?;
        output
            .flush()
            .map_err(|error| io_context("extract", "flush staged file", &destination, error))?;
    }
    Ok(())
}

pub fn extract_zip_file(path: &Path, staging: &Path) -> Result<()> {
    let expected_paths = validate_zip_file(path)?;
    fs::create_dir_all(staging)
        .map_err(|error| io_context("extract", "create staging directory", staging, error))?;
    let archive_file = File::open(path)
        .map_err(|error| io_context("extract", "open release archive", path, error))?;
    let mut archive = ZipArchive::new(archive_file)
        .map_err(|err| UpdaterError::ArchiveUnsafe(err.to_string()))?;
    for (index, relative) in expected_paths.iter().enumerate() {
        let mut file = archive
            .by_index(index)
            .map_err(|err| UpdaterError::ArchiveUnsafe(err.to_string()))?;
        let destination = staging.join(relative);
        if file.is_dir() || file.name().ends_with('/') {
            fs::create_dir_all(&destination).map_err(|error| {
                io_context("extract", "create child directory", &destination, error)
            })?;
            continue;
        }
        let parent = destination
            .parent()
            .ok_or_else(|| UpdaterError::ArchiveUnsafe(relative.clone()))?;
        fs::create_dir_all(parent)
            .map_err(|error| io_context("extract", "create staged parent", parent, error))?;
        let mut output = File::create(&destination)
            .map_err(|error| io_context("extract", "create staged file", &destination, error))?;
        std::io::copy(&mut file, &mut output)
            .map_err(|error| io_context("extract", "copy archive entry", &destination, error))?;
        output
            .flush()
            .map_err(|error| io_context("extract", "flush staged file", &destination, error))?;
    }
    Ok(())
}

pub fn path_is_safe_under(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zip_with_entries(entries: &[(&str, bool)]) -> Vec<u8> {
        let mut output = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut output);
        let options = zip::write::SimpleFileOptions::default();
        for (path, directory) in entries {
            if *directory {
                writer.add_directory(*path, options).expect("directory");
            } else {
                writer.start_file(*path, options).expect("file");
                writer.write_all(b"payload").expect("payload");
            }
        }
        writer.finish().expect("finish");
        output.into_inner()
    }

    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            sha256_bytes(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_bytes(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sidecar_requires_one_exact_record() {
        let hash = "A".repeat(64);
        assert_eq!(
            parse_sha_sidecar(format!("{hash}  app.zip\n").as_bytes(), "app.zip").unwrap(),
            hash.to_ascii_lowercase()
        );
        assert!(parse_sha_sidecar(format!("{hash}  other.zip\n").as_bytes(), "app.zip").is_err());
    }

    #[test]
    fn rejects_windows_ambiguous_paths() {
        for path in [
            "../x", "/x", "C:/x", "x:y", "CON.txt", "x. ", "a\\b", "a//b",
        ] {
            assert!(validate_relative_path(path).is_err(), "accepted {path}");
        }
        assert_eq!(
            crate::manifest::classify_preserved("songs-old/file"),
            crate::manifest::PreserveClass::Managed
        );
    }

    #[test]
    fn explicit_directory_entries_are_legal_parents() {
        let archive = zip_with_entries(&[("songs/", true), ("songs/foo.txt", false)]);
        assert!(validate_zip(&archive).is_ok());
    }

    #[test]
    fn file_parent_and_case_folded_collisions_are_rejected() {
        let archive = zip_with_entries(&[("Foo", false), ("foo/bar", false)]);
        assert!(validate_zip(&archive).is_err());
    }
}
