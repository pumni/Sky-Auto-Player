use std::fs;
use std::path::Path;

use crate::archive::sha256_file;
use crate::error::{Result, UpdaterError};
use crate::manifest::{classify_preserved, load_installed};
use crate::transaction::{
    JournalState, TRANSACTION_DIR, read_journal, safe_join, transaction_root,
    verify_installed_managed,
};

pub fn recover_before_update(install_root: &Path) -> Result<()> {
    let root = transaction_root(install_root);
    if !root.exists() {
        return Ok(());
    }
    let journal = read_journal(install_root)?;
    match journal.state {
        JournalState::Committed => {
            let manifest = load_installed(install_root).map_err(|err| {
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
    for relative in &journal.new_paths {
        if classify_preserved(relative) == crate::manifest::PreserveClass::Preserved {
            return Err(UpdaterError::RollbackFailed(format!(
                "journal contains preserved path: {relative}"
            )));
        }
        let path = safe_join(install_root, relative)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        if path.is_file() {
            fs::remove_file(path).map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        }
    }
    let transaction_root = transaction_root(install_root);
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
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        }
        fs::copy(&source, &destination)
            .map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?;
        if sha256_file(&destination).map_err(|err| UpdaterError::RollbackFailed(err.to_string()))?
            != backup.sha256
        {
            return Err(UpdaterError::RollbackFailed(format!(
                "restored hash mismatch: {}",
                backup.path
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
