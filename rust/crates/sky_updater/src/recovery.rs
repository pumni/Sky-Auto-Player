use std::fs;
use std::path::Path;

use crate::archive::sha256_file;
use crate::error::{Result, UpdaterError};
use crate::file_replace::{atomic_restore, probe_replaceable};
use crate::manifest::{Manifest, classify_preserved};
use crate::transaction::{
    JournalState, TRANSACTION_DIR, read_journal, safe_join, verify_installed_managed,
};
use crate::{CALIBRATION_EXE, MANIFEST_NAME, PRIMARY_EXE, UPDATER_EXE};

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
            let cleanup = crate::transaction::cleanup_committed(install_root)?;
            for failure in cleanup.failures {
                eprintln!(
                    "committed cleanup deferred: {}: {}",
                    failure.path.display(),
                    failure.error
                );
            }
            if root.exists() {
                return Err(UpdaterError::TransactionRecoveryRequired(
                    "committed transaction cleanup remains pending; refusing to start a new update"
                        .into(),
                ));
            }
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
        let destination = safe_join(install_root, &backup.path)
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
        if destination.exists() {
            probe_replaceable(&destination, &backup.path)
                .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        }
        recovery_files.push((
            backup.path.clone(),
            backup.backup_path.clone(),
            backup.sha256.clone(),
        ));
    }

    let backup_paths = journal
        .backups
        .iter()
        .map(|backup| backup.path.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut removable_paths = Vec::with_capacity(journal.new_paths.len());
    for relative in &journal.new_paths {
        if classify_preserved(relative) == crate::manifest::PreserveClass::Preserved {
            return Err(UpdaterError::RollbackFailed(format!(
                "journal contains preserved path: {relative}"
            )));
        }
        let path = safe_join(install_root, relative)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        if backup_paths.contains(relative.as_str()) {
            continue;
        }
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

    recovery_files.sort_by(|left, right| {
        restore_priority(&left.0)
            .cmp(&restore_priority(&right.0))
            .then_with(|| left.0.cmp(&right.0))
    });
    // Restore-first is the P0 safety property: no destination is deleted
    // before its verified backup has been atomically prepared.
    for (index, (relative, backup_relative, expected_hash)) in
        recovery_files.into_iter().enumerate()
    {
        let source = safe_join(&transaction_root, &backup_relative)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        let destination = safe_join(install_root, &relative)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        atomic_restore(&source, &destination, &expected_hash, &relative, index + 1).map_err(
            |error| match error {
                UpdaterError::RollbackAtomicReplaceFailed { .. } => error,
                other => UpdaterError::RollbackFailed(other.to_string()),
            },
        )?;
    }

    for backup in &journal.backups {
        let destination = safe_join(install_root, &backup.path)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        if !sha256_file(&destination)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?
            .eq_ignore_ascii_case(&backup.sha256)
        {
            return Err(UpdaterError::RollbackFailed(format!(
                "restored hash mismatch: {}",
                backup.path
            )));
        }
    }

    // Pure additions are the only paths that may be removed during rollback,
    // and only after every old backup has been restored and verified.
    for relative in removable_paths {
        let path = safe_join(install_root, &relative)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        if path.exists() {
            fs::remove_file(path).map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        }
    }
    crate::file_replace::cleanup_stale_artifacts(install_root)
        .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
    fs::remove_dir_all(transaction_root)
        .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
    Ok(())
}

fn restore_priority(path: &str) -> u8 {
    match path {
        UPDATER_EXE => 0,
        PRIMARY_EXE => 1,
        CALIBRATION_EXE => 2,
        MANIFEST_NAME => 4,
        _ => 3,
    }
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
    use crate::transaction::{
        build_plan, prepare_journal, read_journal, transaction_root, write_json_atomic,
    };
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
        prepare_journal(&root, &plan, &crate::progress::NoopProgressSink).expect("journal");
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
        prepare_journal(&root, &plan, &crate::progress::NoopProgressSink).expect("journal");
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

    #[cfg(windows)]
    #[test]
    fn committed_recovery_ignores_cleanup_only_access_denied() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

        let root = temp_root("committed-cleanup-warning");
        fs::create_dir_all(&root).expect("root");
        let current = b"current app";
        fs::write(root.join(PRIMARY_EXE), current).expect("app");
        let committed_manifest = manifest("2.0.0", &[(PRIMARY_EXE, current)]);
        fs::write(
            root.join(MANIFEST_NAME),
            serde_json::to_vec(&committed_manifest).expect("manifest"),
        )
        .expect("manifest file");

        let old = manifest("1.0.0", &[(PRIMARY_EXE, b"old app")]);
        let plan = build_plan(Some(&old), &committed_manifest).expect("plan");
        prepare_journal(&root, &plan, &crate::progress::NoopProgressSink).expect("journal");
        let mut journal = read_journal(&root).expect("prepared journal");
        journal.state = JournalState::Committed;
        write_json_atomic(&transaction_root(&root).join("journal.json"), &journal)
            .expect("committed journal");

        let stale = root.join(".sky-update-123-456.bak");
        fs::write(&stale, b"locked backup").expect("stale artifact");
        let blocker = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&stale)
            .expect("delete-blocking handle");

        recover_before_update(&root).expect("cleanup-only failure must be deferred");
        assert!(!transaction_root(&root).exists());
        assert!(stale.exists());

        drop(blocker);
        fs::remove_file(stale).expect("cleanup stale artifact");
        fs::remove_dir_all(root).expect("cleanup root");
    }

    #[cfg(windows)]
    #[test]
    fn committed_recovery_rejects_residual_transaction_root_before_new_update() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

        let root = temp_root("committed-root-residual");
        fs::create_dir_all(&root).expect("root");
        let current = b"current app";
        fs::write(root.join(PRIMARY_EXE), current).expect("app");
        let committed_manifest = manifest("2.0.0", &[(PRIMARY_EXE, current)]);
        fs::write(
            root.join(MANIFEST_NAME),
            serde_json::to_vec(&committed_manifest).expect("manifest"),
        )
        .expect("manifest file");

        let old = manifest("1.0.0", &[(PRIMARY_EXE, b"old app")]);
        let plan = build_plan(Some(&old), &committed_manifest).expect("plan");
        prepare_journal(&root, &plan, &crate::progress::NoopProgressSink).expect("journal");
        let mut journal = read_journal(&root).expect("prepared journal");
        journal.state = JournalState::Committed;
        write_json_atomic(&transaction_root(&root).join("journal.json"), &journal)
            .expect("committed journal");

        let locked = transaction_root(&root).join("locked.tmp");
        fs::write(&locked, b"locked transaction material").expect("locked material");
        let blocker = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&locked)
            .expect("delete-blocking handle");

        let error = recover_before_update(&root).expect_err("residual root must block update");
        assert!(
            matches!(error, UpdaterError::TransactionRecoveryRequired(message) if message.contains("committed transaction cleanup remains pending"))
        );
        assert!(transaction_root(&root).exists());

        drop(blocker);
        fs::remove_dir_all(root).expect("cleanup root");
    }
}
