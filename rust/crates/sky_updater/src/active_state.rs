use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::progress::UpdatePhase;
use crate::result::result_dir;
use crate::transaction::write_json_atomic;
use crate::update_lock::install_id;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ActiveUpdateState {
    pub schema_version: u32,
    pub install_id: String,
    pub run_id: String,
    pub updater_pid: u32,
    pub target_version: String,
    pub phase: String,
    pub started_at_utc: String,
    pub updated_at_utc: String,
}

pub struct ActiveUpdateGuard {
    path: PathBuf,
}

impl ActiveUpdateGuard {
    pub fn create(install_root: &Path, run_root: &Path, target_version: &str) -> Result<Self> {
        let path = result_dir()?.join("active-update.json");
        let run_id = run_root
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                crate::error::UpdaterError::InstallRootInvalid("invalid run root".into())
            })?;
        let now = crate::result::timestamp_utc();
        let state = ActiveUpdateState {
            schema_version: 1,
            install_id: install_id(install_root)?,
            run_id: run_id.into(),
            updater_pid: std::process::id(),
            target_version: target_version.into(),
            phase: UpdatePhase::Starting.as_str().into(),
            started_at_utc: now.clone(),
            updated_at_utc: now,
        };
        write_json_atomic(&path, &state)?;
        Ok(Self { path })
    }

    pub fn set_phase(&self, phase: UpdatePhase) -> Result<()> {
        let mut state: ActiveUpdateState = serde_json::from_slice(&std::fs::read(&self.path)?)?;
        state.phase = phase.as_str().into();
        state.updated_at_utc = crate::result::timestamp_utc();
        write_json_atomic(&self.path, &state)
    }

    pub fn remove(&self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Drop for ActiveUpdateGuard {
    fn drop(&mut self) {
        self.remove();
    }
}
