use std::fs;
use std::path::{Path, PathBuf};

use crate::archive::sha256_file;
use crate::error::{Result, UpdaterError, io_context};
use crate::manifest::Manifest;
use crate::{PRIMARY_EXE, UPDATER_EXE};

pub fn project_owned_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for required in [PRIMARY_EXE, UPDATER_EXE, "native_calibration.exe"] {
        let path = root.join(required);
        if !path.is_file() {
            return Err(UpdaterError::ManifestInvalid(format!(
                "required project file missing: {required}"
            )));
        }
        files.push(path);
    }
    let internal = root.join("_internal");
    if internal.is_dir() {
        collect_project_pyds(&internal, &mut files)?;
    }
    Ok(files)
}

fn collect_project_pyds(root: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(root)
        .map_err(|error| io_context("verify staging", "read project directory", root, error))?
    {
        let entry = entry.map_err(|error| {
            io_context(
                "verify staging",
                "read project directory entry",
                root,
                error,
            )
        })?;
        let path = entry.path();
        if entry
            .file_type()
            .map_err(|error| io_context("verify staging", "read project entry type", &path, error))?
            .is_symlink()
        {
            return Err(UpdaterError::ManifestInvalid(format!(
                "symlink under _internal: {}",
                path.display()
            )));
        }
        if path.is_dir() {
            collect_project_pyds(&path, output)?;
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pyd"))
        {
            output.push(path);
        }
    }
    Ok(())
}

pub fn verify_manifest_scope(root: &Path, manifest: &Manifest) -> Result<()> {
    let project_files = project_owned_files(root)?;
    for path in project_files {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| UpdaterError::ManifestInvalid("project file escaped root".into()))?
            .to_string_lossy()
            .replace('\\', "/");
        if !manifest.files.iter().any(|file| file.path == relative) {
            return Err(UpdaterError::ManifestInvalid(format!(
                "project-owned file is absent from manifest: {relative}"
            )));
        }
    }
    Ok(())
}

pub fn verify_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        UpdaterError::ManifestInvalid(format!("missing project file: {}", path.display()))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(UpdaterError::ManifestInvalid(format!(
            "project file is not a regular file: {}",
            path.display()
        )));
    }
    Ok(())
}

pub fn verify_project_files(root: &Path, manifest: &Manifest) -> Result<()> {
    verify_manifest_scope(root, manifest)?;
    for path in project_owned_files(root)? {
        verify_file(&path)?;
        let relative = path
            .strip_prefix(root)
            .map_err(|_| UpdaterError::ManifestInvalid("project file escaped root".into()))?
            .to_string_lossy()
            .replace('\\', "/");
        let entry = manifest
            .files
            .iter()
            .find(|file| file.path == relative)
            .ok_or_else(|| {
                UpdaterError::ManifestInvalid(format!(
                    "project-owned file is absent from manifest: {relative}"
                ))
            })?;
        let metadata = fs::metadata(&path)
            .map_err(|error| io_context("verify staging", "read project metadata", &path, error))?;
        if metadata.len() != entry.size {
            return Err(UpdaterError::ManifestHashMismatch(relative));
        }
        let actual = sha256_file(&path).map_err(|error| match error {
            UpdaterError::Io(source) => {
                io_context("verify staging", "hash project file", &path, source)
            }
            other => other,
        })?;
        if actual != entry.sha256.to_ascii_lowercase() {
            return Err(UpdaterError::ManifestHashMismatch(relative));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::sha256_bytes;
    use crate::manifest::ManifestFile;
    use crate::{APP_NAME, MANIFEST_NAME, SCHEMA_VERSION};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sky-updater-integrity-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ))
    }

    fn write_file(root: &Path, relative: &str, bytes: &[u8]) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("file parent")).expect("file parent");
        fs::write(path, bytes).expect("write fixture file");
    }

    fn fixture_manifest(files: &[(&str, &[u8])]) -> Manifest {
        Manifest {
            schema_version: SCHEMA_VERSION,
            app: APP_NAME.into(),
            version: "3.3.0.dev0".into(),
            executable: PRIMARY_EXE.into(),
            git_head: "a".repeat(40),
            dirty_worktree: false,
            native_build_commit: "b".repeat(40),
            build_time_utc: "2026-08-10T00:00:00Z".into(),
            files: files
                .iter()
                .map(|(path, bytes)| ManifestFile {
                    path: (*path).into(),
                    size: bytes.len() as u64,
                    sha256: sha256_bytes(bytes),
                })
                .collect(),
        }
    }

    fn release_files() -> [(&'static str, &'static [u8]); 3] {
        [
            (PRIMARY_EXE, b"unsigned app"),
            (UPDATER_EXE, b"unsigned updater"),
            ("native_calibration.exe", b"unsigned calibration"),
        ]
    }

    #[test]
    fn valid_unsigned_project_passes_manifest_integrity() {
        let root = temp_root("valid");
        fs::create_dir_all(&root).expect("fixture root");
        let files = release_files();
        for (path, bytes) in files {
            write_file(&root, path, bytes);
        }
        let manifest = fixture_manifest(&files);
        verify_project_files(&root, &manifest).expect("unsigned bytes should be accepted");
        fs::remove_dir_all(root).expect("cleanup fixture");
    }

    #[test]
    fn project_hash_mismatch_is_rejected() {
        let root = temp_root("hash-mismatch");
        fs::create_dir_all(&root).expect("fixture root");
        let files = release_files();
        for (path, bytes) in files {
            write_file(&root, path, bytes);
        }
        let manifest = fixture_manifest(&files);
        write_file(&root, PRIMARY_EXE, b"tampered app");
        assert!(matches!(
            verify_project_files(&root, &manifest),
            Err(UpdaterError::ManifestHashMismatch(path)) if path == PRIMARY_EXE
        ));
        fs::remove_dir_all(root).expect("cleanup fixture");
    }

    #[test]
    fn missing_project_manifest_entry_is_rejected() {
        let root = temp_root("missing-entry");
        fs::create_dir_all(&root).expect("fixture root");
        let files = release_files();
        for (path, bytes) in files {
            write_file(&root, path, bytes);
        }
        let manifest = fixture_manifest(&files[..2]);
        assert!(matches!(
            verify_project_files(&root, &manifest),
            Err(UpdaterError::ManifestInvalid(message))
                if message.contains("native_calibration.exe")
        ));
        fs::remove_dir_all(root).expect("cleanup fixture");
    }

    #[test]
    fn missing_project_file_is_rejected() {
        let root = temp_root("missing-file");
        fs::create_dir_all(&root).expect("fixture root");
        let files = release_files();
        write_file(&root, PRIMARY_EXE, files[0].1);
        let manifest = fixture_manifest(&files);
        assert!(matches!(
            verify_project_files(&root, &manifest),
            Err(UpdaterError::ManifestInvalid(message))
                if message.contains("Sky-Auto-Player-Updater.exe")
        ));
        fs::write(root.join(MANIFEST_NAME), b"not used by this helper").expect("fixture marker");
        fs::remove_dir_all(root).expect("cleanup fixture");
    }
}
