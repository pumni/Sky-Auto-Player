use std::fs;
use std::path::Path;

use crate::archive::sha256_file;
use crate::error::{Result, UpdaterError};
use crate::manifest::{Manifest, classify_preserved};
use crate::transaction::{
    JournalState, TRANSACTION_DIR, read_journal, safe_join, verify_installed_managed,
};

pub fn recover_before_update(install_root: &Path) -> Result<()> {
    let root = safe_join(install_root, TRANSACTION_DIR)?;
    if !root.exists() {
        return Ok(());
    }
    let journal = read_journal(install_root)?;
    match journal.state {
        JournalState::Committed => {
            let manifest_path = safe_join(install_root, crate::MANIFEST_NAME).map_err(|err| {
                UpdaterError::TransactionRecoveryRequired(format!(
                    "committed manifest path is unsafe: {err}"
                ))
            })?;
            let manifest = Manifest::parse(&fs::read(manifest_path)?).map_err(|err| {
                UpdaterError::TransactionRecoveryRequired(format!(
                    "committed install could not be verified: {err}"
                ))
            })?;
            verify_installed_managed(install_root, &manifest).map_err(|err| {
                UpdaterError::TransactionRecoveryRequired(format!(
                    "committed install hash verification failed: {err}"
                ))
            })?;
            fs::remove_dir_all(root)?;
            Ok(())
        }
        JournalState::Prepared => rollback_prepared(install_root),
    }
}

pub fn rollback_prepared(install_root: &Path) -> Result<()> {
    let journal = read_journal(install_root)?;
    if journal.state != JournalState::Prepared {
        return Err(UpdaterError::TransactionRecoveryRequired(
            "journal is not prepared".into(),
        ));
    }

    let transaction_root = safe_join(install_root, TRANSACTION_DIR)
        .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;

    // Preflight every recovery source and every destructive target before
    // changing the installation. A missing or corrupt backup leaves both the
    // current files and the transaction material intact for diagnosis.
    let mut recovery_files = Vec::with_capacity(journal.backups.len());
    for backup in &journal.backups {
        safe_join(install_root, &backup.path)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        let source = safe_join(&transaction_root, &backup.backup_path)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        if !source.is_file() {
            return Err(UpdaterError::RollbackFailed(format!(
                "backup missing: {}",
                backup.path
            )));
        }
        let actual_hash =
            sha256_file(&source).map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        if !actual_hash.eq_ignore_ascii_case(&backup.sha256) {
            return Err(UpdaterError::RollbackFailed(format!(
                "backup hash mismatch: {}",
                backup.path
            )));
        }
        recovery_files.push((
            backup.path.clone(),
            backup.backup_path.clone(),
            backup.sha256.clone(),
        ));
    }

    let mut removable_paths = Vec::with_capacity(journal.new_paths.len());
    for relative in &journal.new_paths {
        if classify_preserved(relative) == crate::manifest::PreserveClass::Preserved {
            return Err(UpdaterError::RollbackFailed(format!(
                "journal contains preserved path: {relative}"
            )));
        }
        let path = safe_join(install_root, relative)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => {
                removable_paths.push(relative.clone())
            }
            Ok(_) => {
                return Err(UpdaterError::RollbackFailed(format!(
                    "managed rollback path is not a regular file: {relative}"
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(UpdaterError::RollbackFailed(error.to_string())),
        }
    }

    for relative in removable_paths {
        let path = safe_join(install_root, &relative)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        if path.exists() {
            fs::remove_file(path).map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        }
    }

    for (relative, backup_relative, expected_hash) in recovery_files {
        let source = safe_join(&transaction_root, &backup_relative)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        let destination = safe_join(install_root, &relative)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        }
        let destination = safe_join(install_root, &relative)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        fs::copy(&source, &destination)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        if !sha256_file(&destination)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?
            .eq_ignore_ascii_case(&expected_hash)
        {
            return Err(UpdaterError::RollbackFailed(format!(
                "restored hash mismatch: {relative}"
            )));
        }
    }
    fs::remove_dir_all(transaction_root)
        .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
    Ok(())
}

pub fn has_unresolved_transaction(install_root: &Path) -> bool {
    install_root.join(TRANSACTION_DIR).exists()
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::archive::sha256_bytes;
    use crate::manifest::{Manifest, ManifestFile};
    use crate::transaction::{build_plan, prepare_journal, transaction_root};
    use crate::{APP_NAME, MANIFEST_NAME, PRIMARY_EXE, SCHEMA_VERSION};

    fn manifest(version: &str, files: &[(&str, &[u8])]) -> Manifest {
        Manifest {
            schema_version: SCHEMA_VERSION,
            app: APP_NAME.into(),
            version: version.into(),
            executable: PRIMARY_EXE.into(),
            git_head: "a".repeat(40),
            dirty_worktree: false,
            native_build_commit: "a".repeat(40),
            build_time_utc: "2026-01-01T00:00:00Z".into(),
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

    fn temp_root(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "sky-updater-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn corrupt_backup_does_not_delete_current_installation() {
        let root = temp_root("rollback");
        let staging = temp_root("rollback-staging");
        fs::create_dir_all(&root).expect("root");
        fs::create_dir_all(&staging).expect("staging");
        let old_app = b"old app";
        let new_app = b"new app";
        fs::write(root.join(PRIMARY_EXE), old_app).expect("old app");
        let old = manifest("1.0.0", &[(PRIMARY_EXE, old_app)]);
        let new = manifest("2.0.0", &[(PRIMARY_EXE, new_app), ("new.dll", b"new dll")]);
        fs::write(staging.join(PRIMARY_EXE), new_app).expect("new app");
        fs::write(staging.join("new.dll"), b"new dll").expect("new dll");
        fs::write(
            staging.join(MANIFEST_NAME),
            serde_json::to_vec(&new).expect("manifest"),
        )
        .expect("staged manifest");

        let plan = build_plan(Some(&old), &new).expect("plan");
        prepare_journal(&root, &plan).expect("journal");
        fs::write(root.join(PRIMARY_EXE), new_app).expect("simulate install");
        fs::write(root.join("new.dll"), b"new dll").expect("simulate add");
        let backup = fs::read_dir(transaction_root(&root).join("backup"))
            .expect("backup dir")
            .next()
            .expect("backup")
            .expect("backup entry")
            .path();
        fs::write(backup, b"corrupt").expect("corrupt backup");

        assert!(rollback_prepared(&root).is_err());
        assert_eq!(fs::read(root.join(PRIMARY_EXE)).unwrap(), new_app);
        assert_eq!(fs::read(root.join("new.dll")).unwrap(), b"new dll");
        assert!(transaction_root(&root).exists());

        fs::remove_dir_all(root).expect("cleanup root");
        fs::remove_dir_all(staging).expect("cleanup staging");
    }

    #[test]
    fn missing_backup_does_not_delete_current_installation() {
        let root = temp_root("rollback-missing");
        fs::create_dir_all(&root).expect("root");
        let old_app = b"old app";
        let new_app = b"new app";
        fs::write(root.join(PRIMARY_EXE), old_app).expect("old app");
        let old = manifest("1.0.0", &[(PRIMARY_EXE, old_app)]);
        let new = manifest("2.0.0", &[(PRIMARY_EXE, new_app), ("new.dll", b"new dll")]);

        let plan = build_plan(Some(&old), &new).expect("plan");
        prepare_journal(&root, &plan).expect("journal");
        fs::write(root.join(PRIMARY_EXE), new_app).expect("simulate install");
        fs::write(root.join("new.dll"), b"new dll").expect("simulate add");
        let backup = fs::read_dir(transaction_root(&root).join("backup"))
            .expect("backup dir")
            .next()
            .expect("backup")
            .expect("backup entry")
            .path();
        fs::remove_file(backup).expect("remove backup");

        assert!(rollback_prepared(&root).is_err());
        assert_eq!(fs::read(root.join(PRIMARY_EXE)).unwrap(), new_app);
        assert_eq!(fs::read(root.join("new.dll")).unwrap(), b"new dll");
        assert!(transaction_root(&root).exists());

        fs::remove_dir_all(root).expect("cleanup root");
    }
}
