use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::archive::{sha256_file, validate_relative_path};
use crate::error::{Result, UpdaterError};
use crate::{APP_NAME, MANIFEST_NAME, PRIMARY_EXE, SCHEMA_VERSION};

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub app: String,
    pub version: String,
    pub executable: String,
    pub git_head: String,
    pub dirty_worktree: bool,
    pub native_build_commit: String,
    pub build_time_utc: String,
    pub files: Vec<ManifestFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreserveClass {
    Managed,
    Preserved,
}

pub fn classify_preserved(path: &str) -> PreserveClass {
    let lower = path.to_lowercase();
    if lower == "config.json"
        || lower == ".env"
        || lower == "songs"
        || lower.starts_with("songs/")
        || lower == "logs"
        || lower.starts_with("logs/")
    {
        PreserveClass::Preserved
    } else {
        PreserveClass::Managed
    }
}

impl Manifest {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > crate::MANIFEST_MAX_BYTES {
            return Err(UpdaterError::ManifestInvalid(
                "manifest exceeds size bound".into(),
            ));
        }
        let manifest: Self = serde_json::from_slice(bytes)?;
        manifest.validate(None)?;
        Ok(manifest)
    }

    pub fn validate(&self, target_version: Option<&str>) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(UpdaterError::ManifestInvalid(
                "unsupported schema version".into(),
            ));
        }
        if self.app != APP_NAME {
            return Err(UpdaterError::ManifestInvalid(
                "unexpected app identifier".into(),
            ));
        }
        if self.executable != PRIMARY_EXE || self.files.iter().all(|file| file.path != PRIMARY_EXE)
        {
            return Err(UpdaterError::ManifestInvalid(
                "canonical executable is missing or incorrect".into(),
            ));
        }
        if self.dirty_worktree {
            return Err(UpdaterError::ManifestInvalid(
                "release manifest marks a dirty worktree".into(),
            ));
        }
        if let Some(target) = target_version
            && self.version != target
        {
            return Err(UpdaterError::ManifestInvalid(
                "manifest version does not match target".into(),
            ));
        }
        let mut normalized = BTreeSet::new();
        for file in &self.files {
            let path = validate_relative_path(&file.path)?;
            if path != file.path || file.path == MANIFEST_NAME {
                return Err(UpdaterError::ManifestInvalid(format!(
                    "noncanonical manifest path: {}",
                    file.path
                )));
            }
            if file.sha256.len() != 64 || !file.sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Err(UpdaterError::ManifestInvalid(format!(
                    "invalid SHA256 for {path}"
                )));
            }
            if !normalized.insert(path.to_lowercase()) {
                return Err(UpdaterError::ManifestInvalid(format!(
                    "duplicate manifest path: {path}"
                )));
            }
        }
        Ok(())
    }

    pub fn verify_staged(&self, root: &Path) -> Result<()> {
        self.validate(Some(&self.version))?;
        let expected = self
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<BTreeSet<_>>();
        let mut actual = BTreeSet::new();
        collect_files(root, root, &mut actual)?;
        actual.remove(MANIFEST_NAME);
        if actual != expected {
            return Err(UpdaterError::ManifestInvalid(
                "staged file set does not match manifest".into(),
            ));
        }
        for file in &self.files {
            let path = root.join(&file.path);
            let metadata = fs::metadata(&path).map_err(|_| {
                UpdaterError::ManifestInvalid(format!("missing staged file: {}", file.path))
            })?;
            if metadata.len() != file.size {
                return Err(UpdaterError::ManifestHashMismatch(file.path.clone()));
            }
            if sha256_file(&path)? != file.sha256.to_ascii_lowercase() {
                return Err(UpdaterError::ManifestHashMismatch(file.path.clone()));
            }
        }
        Ok(())
    }

    pub fn files_by_path(&self) -> BTreeMap<String, &ManifestFile> {
        self.files
            .iter()
            .map(|file| (file.path.clone(), file))
            .collect()
    }
}

fn collect_files(root: &Path, current: &Path, output: &mut BTreeSet<String>) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| UpdaterError::ManifestInvalid("staged path escaped root".into()))?;
        if entry.file_type()?.is_symlink() {
            return Err(UpdaterError::ManifestInvalid(format!(
                "staged symlink: {}",
                relative.display()
            )));
        }
        if path.is_dir() {
            collect_files(root, &path, output)?;
        } else {
            output.insert(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}
