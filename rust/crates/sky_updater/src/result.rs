use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{Result, UpdaterError};
use crate::transaction::write_json_atomic;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpdateWarning {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_error: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpdateResult {
    pub schema_version: u32,
    pub status: String,
    pub from_version: String,
    pub target_version: String,
    pub timestamp_utc: String,
    pub error_code: Option<String>,
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_error: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<UpdateWarning>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub cleanup_pending: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

pub fn result_dir() -> Result<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| UpdaterError::RestartFailed("LOCALAPPDATA is unavailable".into()))?;
    let path = PathBuf::from(local_app_data)
        .join("Sky-Auto-Player")
        .join("update-state");
    if !path.is_absolute() {
        return Err(UpdaterError::RestartFailed(
            "LOCALAPPDATA must resolve to an absolute path".into(),
        ));
    }
    Ok(path)
}

pub fn write_result(result: &UpdateResult) -> Result<()> {
    write_json_atomic(&result_dir()?.join("last-result.json"), result)
}

pub fn append_log(result: &UpdateResult) -> Result<()> {
    let path = result_dir()?.join("updater.log");
    if path.is_file() && std::fs::metadata(&path)?.len() > 1024 * 1024 {
        return Ok(());
    }
    let code = result.error_code.as_deref().unwrap_or("OK");
    let line = format!(
        "{} status={} from={} target={} code={} phase={} operation={} path={} os_error={} warning_count={} cleanup_pending={} msg=\"{}\"\n",
        result.timestamp_utc,
        result.status,
        result.from_version,
        result.target_version,
        code,
        result.phase.as_deref().unwrap_or(""),
        result.operation.as_deref().unwrap_or(""),
        result.path.as_deref().unwrap_or(""),
        result
            .os_error
            .map_or(String::new(), |value| value.to_string()),
        result.warnings.len(),
        result.cleanup_pending,
        bounded_log_message(result.message.as_deref().unwrap_or("")),
    );
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    file.flush()?;
    for warning in &result.warnings {
        let line = format!(
            "{} status=warning from={} target={} code={} phase={} operation={} path={} os_error={} msg=\"{}\"\n",
            result.timestamp_utc,
            result.from_version,
            result.target_version,
            bound_message(warning.code.clone()),
            warning
                .phase
                .as_deref()
                .map(str::to_owned)
                .map(bound_message)
                .unwrap_or_default(),
            warning
                .operation
                .as_deref()
                .map(str::to_owned)
                .map(bound_message)
                .unwrap_or_default(),
            warning
                .path
                .as_deref()
                .map(str::to_owned)
                .map(bound_message)
                .unwrap_or_default(),
            warning
                .os_error
                .map_or(String::new(), |value| value.to_string()),
            bounded_log_message(&warning.message),
        );
        file.write_all(line.as_bytes())?;
    }
    Ok(())
}

pub fn success(from: &str, target: &str) -> UpdateResult {
    success_with_warnings(from, target, Vec::new(), false)
}

pub fn success_with_warnings(
    from: &str,
    target: &str,
    warnings: Vec<UpdateWarning>,
    cleanup_pending: bool,
) -> UpdateResult {
    UpdateResult {
        schema_version: 1,
        status: "success".into(),
        from_version: from.into(),
        target_version: target.into(),
        timestamp_utc: timestamp(),
        error_code: None,
        message: None,
        phase: None,
        operation: None,
        path: None,
        os_error: None,
        warnings: bound_warnings(warnings),
        cleanup_pending,
    }
}

pub fn dry_run(from: &str, target: &str) -> UpdateResult {
    UpdateResult {
        schema_version: 1,
        status: "dry_run".into(),
        from_version: from.into(),
        target_version: target.into(),
        timestamp_utc: timestamp(),
        error_code: None,
        message: None,
        phase: None,
        operation: None,
        path: None,
        os_error: None,
        warnings: Vec::new(),
        cleanup_pending: false,
    }
}

pub fn failure(from: &str, target: &str, error: &UpdaterError) -> UpdateResult {
    let details = error_details(error);
    UpdateResult {
        schema_version: 1,
        status: "failure".into(),
        from_version: from.into(),
        target_version: target.into(),
        timestamp_utc: timestamp(),
        error_code: Some(error_code(error).into()),
        message: Some(bound_message(error.to_string())),
        phase: details.phase,
        operation: details.operation,
        path: details.path,
        os_error: details.os_error,
        warnings: Vec::new(),
        cleanup_pending: false,
    }
}

pub fn failure_with_warnings(
    from: &str,
    target: &str,
    error: &UpdaterError,
    warnings: Vec<UpdateWarning>,
    cleanup_pending: bool,
) -> UpdateResult {
    let mut result = failure(from, target, error);
    result.warnings = bound_warnings(warnings);
    result.cleanup_pending = cleanup_pending;
    result
}

pub fn rolled_back(from: &str, target: &str, error: &UpdaterError) -> UpdateResult {
    let details = error_details(error);
    UpdateResult {
        schema_version: 1,
        status: "rolled_back".into(),
        from_version: from.into(),
        target_version: target.into(),
        timestamp_utc: timestamp(),
        error_code: Some("ROLLED_BACK".into()),
        message: Some(bound_message(error.to_string())),
        phase: details.phase,
        operation: details.operation,
        path: details.path,
        os_error: details.os_error,
        warnings: Vec::new(),
        cleanup_pending: false,
    }
}

fn timestamp() -> String {
    timestamp_utc()
}

pub(crate) fn timestamp_utc() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        day_seconds / 3_600,
        (day_seconds % 3_600) / 60,
        day_seconds % 60
    )
}

fn civil_from_days(days: i64) -> (i64, u64, u64) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_part + 2) / 5 + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    year += if month <= 2 { 1 } else { 0 };
    (year, month as u64, day as u64)
}

pub fn error_code(error: &UpdaterError) -> &'static str {
    match error {
        UpdaterError::InvalidArgument(_) => "INVALID_ARGUMENT",
        UpdaterError::ParentTimeout => "PARENT_TIMEOUT",
        UpdaterError::NetworkFailure(_) => "NETWORK_FAILURE",
        UpdaterError::RedirectRejected(_) => "REDIRECT_REJECTED",
        UpdaterError::ReleaseNotFound(_) => "RELEASE_NOT_FOUND",
        UpdaterError::ReleasePolicyRejected(_) => "RELEASE_POLICY_REJECTED",
        UpdaterError::AssetMissing(_) => "ASSET_MISSING",
        UpdaterError::ChecksumInvalid(_) => "CHECKSUM_INVALID",
        UpdaterError::ChecksumMismatch => "CHECKSUM_MISMATCH",
        UpdaterError::ArchiveUnsafe(_) => "ARCHIVE_UNSAFE",
        UpdaterError::ManifestInvalid(_) => "MANIFEST_INVALID",
        UpdaterError::ManifestHashMismatch(_) => "MANIFEST_HASH_MISMATCH",
        UpdaterError::InstallRootInvalid(_) => "INSTALL_ROOT_INVALID",
        UpdaterError::TransactionRecoveryRequired(_) => "TRANSACTION_RECOVERY_REQUIRED",
        UpdaterError::BackupFailed(_) => "BACKUP_FAILED",
        UpdaterError::InstallCopyFailed(_) => "INSTALL_COPY_FAILED",
        UpdaterError::InstallTargetBusy { .. } => "INSTALL_TARGET_BUSY",
        UpdaterError::UpdateAlreadyRunning => "UPDATE_ALREADY_RUNNING",
        UpdaterError::InstallAtomicReplaceFailed { .. } => "INSTALL_ATOMIC_REPLACE_FAILED",
        UpdaterError::RollbackAtomicReplaceFailed { .. } => "ROLLBACK_ATOMIC_REPLACE_FAILED",
        UpdaterError::PostInstallVerifyFailed(_) => "POST_INSTALL_VERIFY_FAILED",
        UpdaterError::RollbackFailed(_) => "ROLLBACK_FAILED",
        UpdaterError::RestartFailed(_) => "RESTART_FAILED",
        UpdaterError::UiInitializationFailed(_) => "UI_INITIALIZATION_FAILED",
        UpdaterError::Io(_) => "IO_FAILURE",
        UpdaterError::IoContext { .. } => "IO_FAILURE",
        UpdaterError::Json(_) => "JSON_FAILURE",
    }
}

#[derive(Default)]
struct ErrorDetails {
    phase: Option<String>,
    operation: Option<String>,
    path: Option<String>,
    os_error: Option<u32>,
}

fn error_details(error: &UpdaterError) -> ErrorDetails {
    match error {
        UpdaterError::InstallTargetBusy { path, os_code, .. } => ErrorDetails {
            phase: Some("preflight".into()),
            operation: Some("probe".into()),
            path: Some(bound_message(path.clone())),
            os_error: Some(*os_code),
        },
        UpdaterError::InstallAtomicReplaceFailed { path, os_code, .. } => ErrorDetails {
            phase: Some("apply".into()),
            operation: Some("replace".into()),
            path: Some(bound_message(path.clone())),
            os_error: Some(*os_code),
        },
        UpdaterError::RollbackAtomicReplaceFailed { path, os_code, .. } => ErrorDetails {
            phase: Some("rollback".into()),
            operation: Some("replace".into()),
            path: Some(bound_message(path.clone())),
            os_error: Some(*os_code),
        },
        UpdaterError::IoContext {
            phase,
            operation,
            path,
            source,
        } => ErrorDetails {
            phase: Some(bound_message(phase.clone())),
            operation: Some(bound_message(operation.clone())),
            path: Some(bound_message(path.clone())),
            os_error: source.raw_os_error().map(|value| value.unsigned_abs()),
        },
        _ => ErrorDetails::default(),
    }
}

const MAX_MESSAGE_CHARS: usize = 512;

fn bound_message(message: String) -> String {
    let normalized = message.replace(['\r', '\n'], " ");
    if normalized.chars().count() <= MAX_MESSAGE_CHARS {
        return normalized;
    }
    normalized
        .chars()
        .take(MAX_MESSAGE_CHARS - 1)
        .chain(std::iter::once('…'))
        .collect()
}

fn bounded_log_message(message: &str) -> String {
    bound_message(message.to_owned()).replace('"', "'")
}

const MAX_WARNINGS: usize = 8;

fn bound_warnings(mut warnings: Vec<UpdateWarning>) -> Vec<UpdateWarning> {
    if warnings.len() <= MAX_WARNINGS {
        return warnings.drain(..).map(bound_warning).collect::<Vec<_>>();
    }
    warnings.truncate(MAX_WARNINGS - 1);
    warnings.push(UpdateWarning {
        code: "ADDITIONAL_WARNINGS_OMITTED".into(),
        message: "Additional updater warnings were omitted from the bounded result.".into(),
        phase: None,
        operation: None,
        path: None,
        os_error: None,
    });
    warnings.into_iter().map(bound_warning).collect()
}

fn bound_warning(mut warning: UpdateWarning) -> UpdateWarning {
    warning.code = bound_message(warning.code);
    warning.message = bound_message(warning.message);
    warning.phase = warning.phase.map(bound_message);
    warning.operation = warning.operation.map(bound_message);
    warning.path = warning.path.map(bound_message);
    warning
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::path::Path;

    use crate::error::io_context;

    use super::{MAX_MESSAGE_CHARS, bound_message};

    #[test]
    fn bound_message_limits_ascii_to_512_characters() {
        let bounded = bound_message("x".repeat(600));
        assert_eq!(bounded.chars().count(), MAX_MESSAGE_CHARS);
        assert_eq!(bounded.chars().last(), Some('…'));
    }

    #[test]
    fn bound_message_is_unicode_safe_at_boundary() {
        let bounded = bound_message(format!("{}tail", "ế".repeat(600)));
        assert_eq!(bounded.chars().count(), MAX_MESSAGE_CHARS);
        assert_eq!(bounded.chars().last(), Some('…'));
        assert!(bounded.is_char_boundary(bounded.len()));
    }

    #[test]
    fn io_context_preserves_failure_provenance_in_result() {
        let error = io_context(
            "path validation",
            "read file attributes",
            Path::new(r"C:\install\deep\file.txt"),
            io::Error::from_raw_os_error(3),
        );

        let result = super::failure("3.3.0", "3.4.0", &error);

        assert_eq!(result.error_code.as_deref(), Some("IO_FAILURE"));
        assert_eq!(result.phase.as_deref(), Some("path validation"));
        assert_eq!(result.operation.as_deref(), Some("read file attributes"));
        assert_eq!(result.path.as_deref(), Some(r"C:\install\deep\file.txt"));
        assert_eq!(result.os_error, Some(3));
    }
}
