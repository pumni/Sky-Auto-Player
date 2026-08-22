use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, UpdaterError};
use crate::transaction::write_json_atomic;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Handoff {
    pub schema_version: u32,
    pub state: String,
    pub run_id: String,
    pub updater_pid: u32,
    pub target_version: String,
    pub error_code: String,
    pub message: String,
}

pub fn handoff_path(run_root: &Path) -> PathBuf {
    run_root.join("handoff.json")
}

pub fn write_ready(run_root: &Path, target_version: &str) -> Result<()> {
    write_handoff(
        run_root,
        Handoff {
            schema_version: 1,
            state: "ready".into(),
            run_id: run_id(run_root)?,
            updater_pid: std::process::id(),
            target_version: bounded(target_version),
            error_code: String::new(),
            message: String::new(),
        },
    )
}

pub fn write_rejected(
    run_root: &Path,
    target_version: &str,
    error_code: &str,
    message: &str,
) -> Result<()> {
    write_handoff(
        run_root,
        Handoff {
            schema_version: 1,
            state: "rejected".into(),
            run_id: run_id(run_root)?,
            updater_pid: std::process::id(),
            target_version: bounded(target_version),
            error_code: bounded(error_code),
            message: bounded(message),
        },
    )
}

fn write_handoff(run_root: &Path, handoff: Handoff) -> Result<()> {
    let path = handoff_path(run_root);
    write_json_atomic(&path, &handoff)
}

fn run_id(run_root: &Path) -> Result<String> {
    let value = run_root
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            UpdaterError::InstallRootInvalid("run directory has no valid name".into())
        })?;
    if value.len() != 36
        || !value.starts_with("run-")
        || !value[4..].bytes().all(|byte| byte.is_ascii_hexdigit())
        || value[4..].bytes().any(|byte| byte.is_ascii_uppercase())
    {
        return Err(UpdaterError::InstallRootInvalid(
            "run directory name is invalid".into(),
        ));
    }
    Ok(value.into())
}

fn bounded(value: &str) -> String {
    value
        .replace(['\r', '\n', '\0'], " ")
        .chars()
        .take(512)
        .collect()
}
