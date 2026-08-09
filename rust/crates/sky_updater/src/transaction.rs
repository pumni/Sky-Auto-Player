use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::archive::{path_is_safe_under, sha256_file, validate_relative_path};
use crate::error::{Result, UpdaterError};
use crate::manifest::{Manifest, PreserveClass, classify_preserved};
use crate::{MANIFEST_NAME, SCHEMA_VERSION};

pub const TRANSACTION_DIR: &str = ".sky-update-transaction";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionPlan {
    pub files_to_replace: Vec<String>,
    pub files_to_add: Vec<String>,
    pub managed_orphans_to_delete: Vec<String>,
    pub backup_paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub schema_version: u32,
    pub state: JournalState,
    pub install_root: String,
    pub new_paths: Vec<String>,
    pub backups: Vec<BackupEntry>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub enum JournalState {
    Prepared,
    Committed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupEntry {
    pub path: String,
    pub backup_path: String,
    pub sha256: String,
}

pub fn build_plan(old: Option<&Manifest>, new: &Manifest) -> Result<TransactionPlan> {
    new.validate(Some(&new.version))?;
    let new_files = new.files_by_path();
    validate_no_file_directory_collisions(new_files.keys())?;
    let old_files = old.map(Manifest::files_by_path).unwrap_or_default();
    let mut replace = BTreeSet::new();
    let mut add = BTreeSet::new();
    let mut orphans = BTreeSet::new();
    for path in new_files.keys() {
        if classify_preserved(path) == PreserveClass::Preserved {
            continue;
        }
        if old_files.contains_key(path) {
            replace.insert(path.clone());
        } else {
            add.insert(path.clone());
        }
    }
    if old_files.contains_key(MANIFEST_NAME) {
        replace.insert(MANIFEST_NAME.to_owned());
    } else {
        add.insert(MANIFEST_NAME.to_owned());
    }
    for path in old_files.keys() {
        if !new_files.contains_key(path) && classify_preserved(path) == PreserveClass::Managed {
            orphans.insert(path.clone());
        }
    }
    let backup_paths = replace
        .union(&add)
        .chain(&orphans)
        .cloned()
        .collect::<Vec<_>>();
    Ok(TransactionPlan {
        files_to_replace: replace.into_iter().collect(),
        files_to_add: add.into_iter().collect(),
        managed_orphans_to_delete: orphans.into_iter().collect(),
        backup_paths,
    })
}

fn validate_no_file_directory_collisions<'a>(
    paths: impl Iterator<Item = &'a String>,
) -> Result<()> {
    let path_set = paths.map(String::as_str).collect::<BTreeSet<_>>();
    for path in &path_set {
        let mut current = Path::new(path);
        while let Some(parent) = current.parent() {
            if parent == Path::new("") {
                break;
            }
            let parent_string = parent.to_string_lossy().replace('\\', "/");
            if path_set.contains(parent_string.as_str()) {
                return Err(UpdaterError::ManifestInvalid(format!(
                    "file/directory collision: {path}"
                )));
            }
            current = parent;
        }
    }
    Ok(())
}

pub fn transaction_root(install_root: &Path) -> PathBuf {
    install_root.join(TRANSACTION_DIR)
}

pub fn journal_path(install_root: &Path) -> PathBuf {
    transaction_root(install_root).join("journal.json")
}

pub fn prepare_journal(install_root: &Path, plan: &TransactionPlan) -> Result<Journal> {
    let root = transaction_root(install_root);
    if root.exists() {
        return Err(UpdaterError::TransactionRecoveryRequired(
            "transaction directory already exists".into(),
        ));
    }
    fs::create_dir_all(root.join("backup"))
        .map_err(|err| UpdaterError::BackupFailed(err.to_string()))?;
    let mut backups = Vec::new();
    for (index, relative) in plan.backup_paths.iter().enumerate() {
        let source = install_root.join(relative);
        if !source.is_file() {
            continue;
        }
        let backup_relative = format!("backup/{index:08}.bin");
        let backup = root.join(&backup_relative);
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| UpdaterError::BackupFailed(err.to_string()))?;
        }
        fs::copy(&source, &backup).map_err(|err| {
            UpdaterError::BackupFailed(format!(
                "could not back up {} to {}: {err}",
                source.display(),
                backup.display()
            ))
        })?;
        #[cfg(windows)]
        {
            let mut permissions = fs::metadata(&backup)
                .map_err(|err| UpdaterError::BackupFailed(err.to_string()))?
                .permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(&backup, permissions)
                .map_err(|err| UpdaterError::BackupFailed(err.to_string()))?;
        }
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&backup)
            .and_then(|file| file.sync_all())
            .map_err(|err| {
                UpdaterError::BackupFailed(format!(
                    "could not flush backup {}: {err}",
                    backup.display()
                ))
            })?;
        backups.push(BackupEntry {
            path: relative.clone(),
            backup_path: backup_relative,
            sha256: sha256_file(&source)
                .map_err(|err| UpdaterError::BackupFailed(err.to_string()))?,
        });
    }
    let new_paths = plan
        .files_to_replace
        .iter()
        .chain(plan.files_to_add.iter())
        .cloned()
        .collect();
    let journal = Journal {
        schema_version: SCHEMA_VERSION,
        state: JournalState::Prepared,
        install_root: install_root.to_string_lossy().into_owned(),
        new_paths,
        backups,
    };
    write_json_atomic(&journal_path(install_root), &journal)?;
    Ok(journal)
}

pub fn apply(
    install_root: &Path,
    staging: &Path,
    new_manifest: &Manifest,
    plan: &TransactionPlan,
) -> Result<()> {
    let journal = read_journal(install_root)?;
    if journal.state != JournalState::Prepared {
        return Err(UpdaterError::TransactionRecoveryRequired(
            "transaction is not prepared".into(),
        ));
    }
    for relative in plan.files_to_replace.iter().chain(plan.files_to_add.iter()) {
        copy_managed_file(install_root, staging, relative)?;
    }
    for relative in &plan.managed_orphans_to_delete {
        let destination = safe_join(install_root, relative)?;
        if destination.is_file() {
            fs::remove_file(destination)
                .map_err(|err| UpdaterError::InstallCopyFailed(err.to_string()))?;
        }
    }
    verify_installed_managed(install_root, new_manifest)?;
    let installed_manifest = Manifest::parse(&fs::read(install_root.join(MANIFEST_NAME))?)
        .map_err(|error| UpdaterError::PostInstallVerifyFailed(error.to_string()))?;
    if installed_manifest != *new_manifest {
        return Err(UpdaterError::PostInstallVerifyFailed(
            "installed MANIFEST.json does not match staged manifest".into(),
        ));
    }
    let mut committed = journal;
    committed.state = JournalState::Committed;
    write_json_atomic(&journal_path(install_root), &committed)?;
    Ok(())
}

fn copy_managed_file(install_root: &Path, staging: &Path, relative: &str) -> Result<()> {
    if classify_preserved(relative) == PreserveClass::Preserved {
        return Err(UpdaterError::InstallCopyFailed(format!(
            "attempted to replace preserved path: {relative}"
        )));
    }
    let source = safe_join(staging, relative)?;
    let destination = safe_join(install_root, relative)?;
    if !source.is_file() {
        return Err(UpdaterError::InstallCopyFailed(format!(
            "missing staged file: {relative}"
        )));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination)
        .map_err(|err| UpdaterError::InstallCopyFailed(err.to_string()))?;
    Ok(())
}

pub fn verify_installed_managed(install_root: &Path, manifest: &Manifest) -> Result<()> {
    for file in &manifest.files {
        if classify_preserved(&file.path) == PreserveClass::Preserved {
            continue;
        }
        let path = safe_join(install_root, &file.path)?;
        let metadata = fs::metadata(&path)
            .map_err(|_| UpdaterError::PostInstallVerifyFailed(file.path.clone()))?;
        if metadata.len() != file.size || sha256_file(&path)? != file.sha256.to_ascii_lowercase() {
            return Err(UpdaterError::PostInstallVerifyFailed(file.path.clone()));
        }
    }
    Ok(())
}

pub fn read_journal(install_root: &Path) -> Result<Journal> {
    let journal: Journal = serde_json::from_slice(&fs::read(journal_path(install_root))?)?;
    if journal.schema_version != SCHEMA_VERSION
        || journal.install_root != install_root.to_string_lossy()
    {
        return Err(UpdaterError::TransactionRecoveryRequired(
            "journal identity/schema mismatch".into(),
        ));
    }
    let mut new_paths = BTreeSet::new();
    for path in &journal.new_paths {
        validate_relative_path(path)
            .map_err(|err| UpdaterError::TransactionRecoveryRequired(err.to_string()))?;
        if classify_preserved(path) == PreserveClass::Preserved || !new_paths.insert(path) {
            return Err(UpdaterError::TransactionRecoveryRequired(
                "journal contains an invalid or duplicate new path".into(),
            ));
        }
    }
    let mut backup_paths = BTreeSet::new();
    let mut backup_files = BTreeSet::new();
    for backup in &journal.backups {
        validate_relative_path(&backup.path)
            .map_err(|err| UpdaterError::TransactionRecoveryRequired(err.to_string()))?;
        validate_relative_path(&backup.backup_path)
            .map_err(|err| UpdaterError::TransactionRecoveryRequired(err.to_string()))?;
        if classify_preserved(&backup.path) == PreserveClass::Preserved
            || !backup_paths.insert(&backup.path)
            || !backup_files.insert(&backup.backup_path)
            || backup.sha256.len() != 64
            || !backup.sha256.chars().all(|ch| ch.is_ascii_hexdigit())
        {
            return Err(UpdaterError::TransactionRecoveryRequired(
                "journal contains an invalid or duplicate backup entry".into(),
            ));
        }
    }
    Ok(journal)
}

pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| UpdaterError::Io(std::io::Error::other("JSON path has no parent")))?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    file.write_all(&bytes)?;
    file.flush()?;
    file.sync_all()?;
    atomic_replace(&temporary, path)?;
    Ok(())
}

fn atomic_replace(temporary: &Path, destination: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let source = temporary
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let target = destination
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let flags = MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH;
        if unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), flags) } == 0 {
            return Err(UpdaterError::Io(std::io::Error::last_os_error()));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(temporary, destination)?;
        Ok(())
    }
}

pub fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = root.join(relative);
    if !path_is_safe_under(root, &path) {
        return Err(UpdaterError::InstallRootInvalid(format!(
            "path escapes root: {relative}"
        )));
    }
    Ok(path)
}

pub fn cleanup_committed(install_root: &Path) -> Result<()> {
    let root = transaction_root(install_root);
    if root.exists() {
        let journal = read_journal(install_root)?;
        if journal.state != JournalState::Committed {
            return Err(UpdaterError::TransactionRecoveryRequired(
                "prepared transaction must be recovered".into(),
            ));
        }
        fs::remove_dir_all(root)?;
    }
    Ok(())
}

pub fn remove_manifest_from_set(paths: &mut BTreeSet<String>) {
    paths.remove(MANIFEST_NAME);
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::archive::sha256_bytes;
    use crate::manifest::{ManifestFile, PreserveClass};

    fn manifest(version: &str, files: &[&str]) -> Manifest {
        Manifest {
            schema_version: SCHEMA_VERSION,
            app: crate::APP_NAME.into(),
            version: version.into(),
            executable: crate::PRIMARY_EXE.into(),
            git_head: "a".repeat(40),
            dirty_worktree: false,
            native_build_commit: "a".repeat(40),
            build_time_utc: "2026-01-01T00:00:00Z".into(),
            files: files
                .iter()
                .map(|path| ManifestFile {
                    path: (*path).into(),
                    size: 1,
                    sha256: "a".repeat(64),
                })
                .collect(),
        }
    }

    #[test]
    fn preserve_paths_are_not_orphaned() {
        assert_eq!(
            classify_preserved("songs/custom.skysheet"),
            PreserveClass::Preserved
        );
        let old = manifest("1.0.0", &[crate::PRIMARY_EXE, "old.dll", "config.json"]);
        let new = manifest("2.0.0", &[crate::PRIMARY_EXE, "new.dll", "config.json"]);
        let plan = build_plan(Some(&old), &new).unwrap();
        assert_eq!(plan.managed_orphans_to_delete, vec!["old.dll"]);
        assert!(
            !plan
                .files_to_replace
                .iter()
                .any(|path| path == "config.json")
        );
    }

    #[test]
    fn unknown_files_do_not_appear_in_plan() {
        let old = manifest("1.0.0", &[crate::PRIMARY_EXE]);
        let new = manifest("2.0.0", &[crate::PRIMARY_EXE]);
        let plan = build_plan(Some(&old), &new).unwrap();
        assert!(plan.managed_orphans_to_delete.is_empty());
    }

    #[test]
    fn transaction_replaces_managed_files_and_preserves_user_state() {
        let root = std::env::temp_dir().join(format!(
            "sky-updater-transaction-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let staging = root.with_file_name(format!("sky-updater-staging-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("install root");
        std::fs::create_dir_all(&staging).expect("staging");
        let old_app = b"old app";
        let new_app = b"new app";
        std::fs::write(root.join(crate::PRIMARY_EXE), old_app).expect("old app");
        std::fs::write(root.join("old.dll"), b"old dll").expect("old dll");
        std::fs::write(root.join("config.json"), b"user config").expect("config");
        std::fs::write(root.join("unknown-user-file.txt"), b"user file").expect("unknown");
        let old = Manifest {
            schema_version: SCHEMA_VERSION,
            app: crate::APP_NAME.into(),
            version: "1.0.0".into(),
            executable: crate::PRIMARY_EXE.into(),
            git_head: "a".repeat(40),
            dirty_worktree: false,
            native_build_commit: "a".repeat(40),
            build_time_utc: "2026-01-01T00:00:00Z".into(),
            files: vec![
                ManifestFile {
                    path: crate::PRIMARY_EXE.into(),
                    size: old_app.len() as u64,
                    sha256: sha256_bytes(old_app),
                },
                ManifestFile {
                    path: "old.dll".into(),
                    size: 7,
                    sha256: sha256_bytes(b"old dll"),
                },
            ],
        };
        let new = Manifest {
            schema_version: SCHEMA_VERSION,
            app: crate::APP_NAME.into(),
            version: "2.0.0".into(),
            executable: crate::PRIMARY_EXE.into(),
            git_head: "b".repeat(40),
            dirty_worktree: false,
            native_build_commit: "b".repeat(40),
            build_time_utc: "2026-01-02T00:00:00Z".into(),
            files: vec![ManifestFile {
                path: crate::PRIMARY_EXE.into(),
                size: new_app.len() as u64,
                sha256: sha256_bytes(new_app),
            }],
        };
        std::fs::write(staging.join(crate::PRIMARY_EXE), new_app).expect("staged app");
        std::fs::write(
            staging.join(crate::MANIFEST_NAME),
            serde_json::to_vec(&new).expect("manifest json"),
        )
        .expect("staged manifest");

        let plan = build_plan(Some(&old), &new).expect("plan");
        prepare_journal(&root, &plan).expect("prepare");
        apply(&root, &staging, &new, &plan).expect("apply");
        cleanup_committed(&root).expect("cleanup");

        assert_eq!(
            std::fs::read(root.join(crate::PRIMARY_EXE)).unwrap(),
            new_app
        );
        assert!(!root.join("old.dll").exists());
        assert_eq!(
            std::fs::read(root.join("config.json")).unwrap(),
            b"user config"
        );
        assert_eq!(
            std::fs::read(root.join("unknown-user-file.txt")).unwrap(),
            b"user file"
        );
        assert!(!transaction_root(&root).exists());
        std::fs::remove_dir_all(root).expect("remove test root");
        std::fs::remove_dir_all(staging).expect("remove staging root");
    }
}
