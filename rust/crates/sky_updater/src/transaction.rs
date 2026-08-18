use std::collections::{BTreeSet, HashMap};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::archive::{path_is_safe_under, sha256_file, validate_relative_path};
use crate::error::{Result, UpdaterError};
use crate::file_replace::{probe_new_destination, probe_replaceable};
use crate::manifest::{Manifest, PreserveClass, classify_preserved};
use crate::{CALIBRATION_EXE, MANIFEST_NAME, PRIMARY_EXE, SCHEMA_VERSION, UPDATER_EXE};

pub const TRANSACTION_DIR: &str = ".sky-update-transaction";
const JOURNAL_FILE_NAME: &str = "journal.json";

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
    if old.is_some() {
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
    let mut files_to_replace = replace.into_iter().collect::<Vec<_>>();
    let mut files_to_add = add.into_iter().collect::<Vec<_>>();
    order_managed_paths(&mut files_to_replace);
    order_managed_paths(&mut files_to_add);
    Ok(TransactionPlan {
        files_to_replace,
        files_to_add,
        managed_orphans_to_delete: orphans.into_iter().collect(),
        backup_paths,
    })
}

fn order_managed_paths(paths: &mut [String]) {
    paths.sort_by(|left, right| {
        path_priority(left)
            .cmp(&path_priority(right))
            .then_with(|| left.cmp(right))
    });
}

fn path_priority(path: &str) -> u8 {
    match path {
        MANIFEST_NAME => 4,
        UPDATER_EXE => 3,
        CALIBRATION_EXE => 2,
        PRIMARY_EXE => 1,
        _ => 0,
    }
}

fn ordered_payload_paths(plan: &TransactionPlan) -> Vec<(String, bool)> {
    let mut paths = plan
        .files_to_replace
        .iter()
        .map(|path| (path.clone(), true))
        .chain(plan.files_to_add.iter().map(|path| (path.clone(), false)))
        .collect::<Vec<_>>();
    paths.sort_by(|(left, _), (right, _)| {
        path_priority(left)
            .cmp(&path_priority(right))
            .then_with(|| left.cmp(right))
    });
    paths
}

fn validate_no_file_directory_collisions<'a>(
    paths: impl Iterator<Item = &'a String>,
) -> Result<()> {
    let mut path_set = HashMap::<String, &str>::new();
    for path in paths {
        let key = path.to_lowercase();
        if path_set.insert(key, path.as_str()).is_some() {
            return Err(UpdaterError::ManifestInvalid(format!(
                "duplicate/case-colliding path: {path}"
            )));
        }
    }
    for path in path_set.values() {
        let mut current = Path::new(path);
        while let Some(parent) = current.parent() {
            if parent == Path::new("") {
                break;
            }
            let parent_string = parent.to_string_lossy().replace('\\', "/");
            if path_set.contains_key(&parent_string.to_lowercase()) {
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

pub fn prepare_journal(install_root: &Path, plan: &TransactionPlan) -> Result<Journal> {
    let root = safe_join(install_root, TRANSACTION_DIR)?;
    if root.exists() {
        return Err(UpdaterError::TransactionRecoveryRequired(
            "transaction directory already exists".into(),
        ));
    }
    fs::create_dir_all(root.join("backup"))
        .map_err(|err| UpdaterError::BackupFailed(err.to_string()))?;
    let result = prepare_journal_inner(install_root, plan, &root);
    if result.is_err() {
        let _ = fs::remove_dir_all(&root);
    }
    result
}

fn prepare_journal_inner(
    install_root: &Path,
    plan: &TransactionPlan,
    root: &Path,
) -> Result<Journal> {
    let mut backups = Vec::new();
    for (index, relative) in plan.backup_paths.iter().enumerate() {
        let source = safe_join(install_root, relative)?;
        if !source.is_file() {
            continue;
        }
        let backup_relative = format!("backup/{index:08}.bin");
        let backup = root.join(&backup_relative);
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| UpdaterError::BackupFailed(err.to_string()))?;
        }
        let expected_hash = sha256_file(&source).map_err(|err| {
            UpdaterError::BackupFailed(format!("could not hash {source:?}: {err}"))
        })?;
        copy_backup(&source, &backup).map_err(|err| {
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
        backups.push(BackupEntry {
            path: relative.clone(),
            backup_path: backup_relative,
            sha256: {
                let actual = sha256_file(&backup)
                    .map_err(|err| UpdaterError::BackupFailed(err.to_string()))?;
                if actual != expected_hash {
                    return Err(UpdaterError::BackupFailed(format!(
                        "backup changed while being created: {relative}"
                    )));
                }
                expected_hash
            },
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
    let journal_file = safe_join(
        install_root,
        &format!("{TRANSACTION_DIR}/{JOURNAL_FILE_NAME}"),
    )?;
    write_json_atomic(&journal_file, &journal)?;
    Ok(journal)
}

fn copy_backup(source: &Path, backup: &Path) -> io::Result<()> {
    let mut input = fs::File::open(source)?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(backup)?;
    io::copy(&mut input, &mut output)?;
    output.flush()?;
    output.sync_all()
}

/// Prove every managed destination can participate in the upcoming atomic
/// transaction before the prepared journal is created.
pub fn preflight(install_root: &Path, plan: &TransactionPlan) -> Result<()> {
    for relative in &plan.files_to_replace {
        let destination = safe_join(install_root, relative)?;
        if !probe_replaceable(&destination, relative)? {
            return Err(UpdaterError::InstallTargetBusy {
                path: relative.clone(),
                os_code: 2,
                message: "managed replacement target is missing".into(),
            });
        }
    }
    for relative in &plan.files_to_add {
        let destination = safe_join(install_root, relative)?;
        probe_new_destination(&destination, relative)?;
    }
    for relative in &plan.managed_orphans_to_delete {
        let destination = safe_join(install_root, relative)?;
        if destination.exists() {
            probe_replaceable(&destination, relative)?;
        }
    }
    Ok(())
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
    let ordered_payloads = ordered_payload_paths(plan);
    let mut index = 0usize;
    for (relative, replaces_existing) in ordered_payloads
        .iter()
        .filter(|(relative, _)| relative != MANIFEST_NAME)
    {
        index += 1;
        copy_managed_file(
            install_root,
            staging,
            new_manifest,
            relative,
            *replaces_existing,
            index,
        )?;
    }
    for relative in &plan.managed_orphans_to_delete {
        let destination = safe_join(install_root, relative)?;
        if destination.is_file() {
            fs::remove_file(destination)
                .map_err(|err| UpdaterError::InstallCopyFailed(err.to_string()))?;
        }
    }
    if let Some((relative, replaces_existing)) = ordered_payloads
        .iter()
        .find(|(relative, _)| relative == MANIFEST_NAME)
    {
        index += 1;
        copy_managed_file(
            install_root,
            staging,
            new_manifest,
            relative,
            *replaces_existing,
            index,
        )?;
    }
    verify_installed_managed(install_root, new_manifest)?;
    let installed_manifest_path = safe_join(install_root, MANIFEST_NAME)
        .map_err(|error| UpdaterError::PostInstallVerifyFailed(error.to_string()))?;
    let installed_manifest = Manifest::parse(&fs::read(installed_manifest_path)?)
        .map_err(|error| UpdaterError::PostInstallVerifyFailed(error.to_string()))?;
    if installed_manifest != *new_manifest {
        return Err(UpdaterError::PostInstallVerifyFailed(
            "installed MANIFEST.json does not match staged manifest".into(),
        ));
    }
    let mut committed = journal;
    committed.state = JournalState::Committed;
    let journal_file = safe_join(
        install_root,
        &format!("{TRANSACTION_DIR}/{JOURNAL_FILE_NAME}"),
    )?;
    write_json_atomic(&journal_file, &committed)?;
    Ok(())
}

fn copy_managed_file(
    install_root: &Path,
    staging: &Path,
    new_manifest: &Manifest,
    relative: &str,
    replaces_existing: bool,
    index: usize,
) -> Result<()> {
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
    let expected_hash = if relative == MANIFEST_NAME {
        sha256_file(&source)?
    } else {
        new_manifest
            .files_by_path()
            .get(relative)
            .map(|file| file.sha256.clone())
            .ok_or_else(|| {
                UpdaterError::ManifestInvalid(format!(
                    "managed path absent from manifest: {relative}"
                ))
            })?
    };
    let replacement =
        crate::file_replace::prepare_replacement(&source, &destination, &expected_hash, relative)?;
    if replaces_existing && destination.is_file() {
        crate::file_replace::atomic_replace_existing(replacement, "apply", index)
    } else {
        crate::file_replace::atomic_install_new(replacement, "apply", index)
    }
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
    let journal_file = safe_join(
        install_root,
        &format!("{TRANSACTION_DIR}/{JOURNAL_FILE_NAME}"),
    )
    .map_err(|err| UpdaterError::TransactionRecoveryRequired(err.to_string()))?;
    let journal: Journal = serde_json::from_slice(&fs::read(journal_file)?)?;
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
    ensure_no_reparse_components(root, &path)?;
    Ok(path)
}

fn ensure_no_reparse_components(root: &Path, path: &Path) -> Result<()> {
    match fs::symlink_metadata(root) {
        Ok(_) => {
            if is_reparse_point(root)? {
                return Err(UpdaterError::InstallRootInvalid(format!(
                    "install root is a reparse point: {}",
                    root.display()
                )));
            }
        }
        Err(error) => return Err(UpdaterError::Io(error)),
    }
    let relative = path.strip_prefix(root).map_err(|_| {
        UpdaterError::InstallRootInvalid(format!("path escapes root: {}", path.display()))
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(UpdaterError::InstallRootInvalid(format!(
                "non-normal path component: {}",
                path.display()
            )));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(_) => {
                if is_reparse_point(&current)? {
                    return Err(UpdaterError::InstallRootInvalid(format!(
                        "reparse point in protected path: {}",
                        current.display()
                    )));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(UpdaterError::Io(error)),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(path: &Path) -> Result<bool> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, GetFileAttributesW, INVALID_FILE_ATTRIBUTES,
    };

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        return Err(UpdaterError::Io(std::io::Error::last_os_error()));
    }
    Ok(attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

#[cfg(not(windows))]
fn is_reparse_point(path: &Path) -> Result<bool> {
    Ok(fs::symlink_metadata(path)?.file_type().is_symlink())
}

pub fn cleanup_committed(install_root: &Path) -> Result<()> {
    let root = safe_join(install_root, TRANSACTION_DIR)?;
    if root.exists() {
        let journal = read_journal(install_root)?;
        if journal.state != JournalState::Committed {
            return Err(UpdaterError::TransactionRecoveryRequired(
                "prepared transaction must be recovered".into(),
            ));
        }
        fs::remove_dir_all(root)?;
    }
    crate::file_replace::cleanup_stale_artifacts(install_root)?;
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

    #[cfg(windows)]
    #[test]
    fn safe_join_rejects_existing_reparse_ancestor() {
        use std::os::windows::fs::symlink_dir;

        let root = std::env::temp_dir().join(format!(
            "sky-updater-reparse-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let outside = root.with_file_name(format!(
            "sky-updater-reparse-outside-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("install root");
        std::fs::create_dir_all(&outside).expect("outside root");
        let junction = root.join("managed");
        if symlink_dir(&outside, &junction).is_err() {
            std::fs::remove_dir_all(&root).expect("cleanup root");
            std::fs::remove_dir_all(&outside).expect("cleanup outside");
            return;
        }

        assert!(safe_join(&root, "managed/escaped.dll").is_err());
        std::fs::remove_dir(&junction).expect("remove junction");
        std::fs::remove_dir_all(&root).expect("cleanup root");
        std::fs::remove_dir_all(&outside).expect("cleanup outside");
    }
}
