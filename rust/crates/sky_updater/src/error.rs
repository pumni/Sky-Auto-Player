use std::io;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum UpdaterError {
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("parent process did not exit before timeout")]
    ParentTimeout,
    #[error("network failure: {0}")]
    NetworkFailure(String),
    #[error("redirect rejected: {0}")]
    RedirectRejected(String),
    #[error("release not found: {0}")]
    ReleaseNotFound(String),
    #[error("release policy rejected: {0}")]
    ReleasePolicyRejected(String),
    #[error("asset missing: {0}")]
    AssetMissing(String),
    #[error("checksum invalid: {0}")]
    ChecksumInvalid(String),
    #[error("checksum mismatch")]
    ChecksumMismatch,
    #[error("archive unsafe: {0}")]
    ArchiveUnsafe(String),
    #[error("manifest invalid: {0}")]
    ManifestInvalid(String),
    #[error("manifest hash mismatch: {0}")]
    ManifestHashMismatch(String),
    #[error("install root invalid: {0}")]
    InstallRootInvalid(String),
    #[error("transaction recovery required: {0}")]
    TransactionRecoveryRequired(String),
    #[error("backup failed: {0}")]
    BackupFailed(String),
    #[error("install copy failed: {0}")]
    InstallCopyFailed(String),
    #[error("an install target is busy: {path} (Win32 error {os_code}): {message}")]
    InstallTargetBusy {
        path: String,
        os_code: u32,
        message: String,
    },
    #[error("another updater is already running")]
    UpdateAlreadyRunning,
    #[error("atomic install replacement failed for {path} (Win32 error {os_code}): {message}")]
    InstallAtomicReplaceFailed {
        path: String,
        os_code: u32,
        message: String,
    },
    #[error("atomic rollback replacement failed for {path} (Win32 error {os_code}): {message}")]
    RollbackAtomicReplaceFailed {
        path: String,
        os_code: u32,
        message: String,
    },
    #[error("post-install verification failed: {0}")]
    PostInstallVerifyFailed(String),
    #[error("rollback failed: {0}")]
    RollbackFailed(String),
    #[error("restart failed: {0}")]
    RestartFailed(String),
    #[error("I/O failure: {0}")]
    Io(#[from] io::Error),
    #[error("JSON failure: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, UpdaterError>;
