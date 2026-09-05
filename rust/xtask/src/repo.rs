use crate::{Result, process};
use std::path::{Path, PathBuf};
use toml::Value;

pub fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask must live below repository rust/")
        .to_path_buf()
}

pub fn git_head(root: &Path, require_clean: bool) -> Result<String> {
    let head = process::capture_text("git", &["rev-parse", "--verify", "HEAD"], root, &[])?;
    if !head.chars().all(|c| c.is_ascii_hexdigit()) || head.len() != 40 {
        return Err(format!("git HEAD is not a full commit SHA: {head:?}").into());
    }
    let status = process::capture_text("git", &["status", "--porcelain"], root, &[])?;
    if require_clean && !status.is_empty() {
        return Err(format!("release command requires a clean worktree: {status}").into());
    }
    Ok(head)
}

pub fn project_version(root: &Path) -> Result<String> {
    let path = root.join("desktop/src-tauri/Cargo.toml");
    let value: Value = toml::from_str(&std::fs::read_to_string(&path)?)?;
    value
        .get("package")
        .and_then(Value::as_table)
        .and_then(|table| table.get("version"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("missing package.version in {}", path.display()).into())
}

/// Return the commit's committer timestamp as an RFC3339 UTC value. The
/// manifest field is deterministic for a given source commit, while retaining
/// the meaning of `build_time_utc` as a real repository timestamp.
pub fn commit_time_utc(root: &Path) -> Result<String> {
    let value = process::capture_text("git", &["show", "-s", "--format=%ct", "HEAD"], root, &[])?;
    let seconds = value.parse::<i64>().map_err(|error| {
        format!("git commit timestamp is not a Unix timestamp: {value:?}: {error}")
    })?;
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }).div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524
        - day_of_era / 146_096)
        .div_euclid(365);
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2).div_euclid(153);
    let day = day_of_year - (153 * month_part + 2).div_euclid(5) + 1;
    let month = month_part + if month_part < 10 { 3 } else { -9 };
    let year = year + if month <= 2 { 1 } else { 0 };
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_is_repository_root() {
        assert!(root().join("rust/Cargo.toml").is_file());
    }

    #[test]
    fn commit_timestamp_is_deterministic_utc_rfc3339() {
        let timestamp = commit_time_utc(&root()).unwrap();
        assert!(timestamp.ends_with('Z'));
        assert_eq!(timestamp.len(), 20);
        assert_eq!(timestamp.as_bytes().get(4), Some(&b'-'));
        assert_eq!(timestamp.as_bytes().get(7), Some(&b'-'));
        assert_eq!(timestamp.as_bytes().get(10), Some(&b'T'));
    }
}
