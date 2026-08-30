use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::progress::UpdatePhase;
use crate::result::result_dir;
use crate::transaction::write_json_atomic;
use crate::update_lock::install_id;
use crate::{UPDATER_EXE, process};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

/// Inspect the active transaction used by the native updater.
///
/// This is intentionally the same fixed state path, install identity, run
/// identity, and process-image validation used by the updater runtime. Dead
/// or malformed state is cleaned up at that path; a live foreign-install
/// state is ignored so one installation cannot block another.
pub fn active_update_for_install(install_root: &Path) -> Result<Option<ActiveUpdateState>> {
    active_update_for_install_with(install_root, process::query_process_image)
}

/// Testable form of [`active_update_for_install`] with the platform process
/// query supplied by the caller. The production wrapper above always uses the
/// native bounded query; the seam keeps startup-admission tests deterministic.
pub fn active_update_for_install_with<F>(
    install_root: &Path,
    query_process: F,
) -> Result<Option<ActiveUpdateState>>
where
    F: Fn(u32) -> Result<process::ProcessImage>,
{
    let state_path = result_dir()?.join("active-update.json");
    inspect_active_update_for_install(install_root, &state_path, query_process)
}

fn inspect_active_update_for_install<F>(
    install_root: &Path,
    state_path: &Path,
    query_process: F,
) -> Result<Option<ActiveUpdateState>>
where
    F: Fn(u32) -> Result<process::ProcessImage>,
{
    if !state_path.is_file() {
        return Ok(None);
    }
    if state_path.metadata()?.len() > crate::SIDECAR_MAX_BYTES as u64 {
        remove_state(&state_path);
        return Ok(None);
    }
    let state: ActiveUpdateState = match serde_json::from_slice(&std::fs::read(&state_path)?) {
        Ok(state) => state,
        Err(_) => {
            remove_state(&state_path);
            return Ok(None);
        }
    };
    if !valid_state(&state) {
        remove_state(&state_path);
        return Ok(None);
    }
    if state.install_id != install_id(install_root)? {
        return Ok(None);
    }
    let process = match query_process(state.updater_pid)? {
        process::ProcessImage::Exited => {
            remove_state(&state_path);
            return Ok(None);
        }
        process::ProcessImage::Alive(path) => path,
    };
    let image = match process.canonicalize() {
        Ok(path) => path,
        Err(_) => {
            remove_state(&state_path);
            return Ok(None);
        }
    };
    let runs = state_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| crate::error::UpdaterError::InstallRootInvalid("invalid state path".into()))?
        .join("update-runs")
        .canonicalize();
    let run_dir = image.parent().and_then(Path::parent);
    let expected_run = match runs {
        Ok(path) => path,
        Err(_) => {
            remove_state(&state_path);
            return Ok(None);
        }
    };
    if image.file_name().and_then(|name| name.to_str()) != Some(UPDATER_EXE)
        || run_dir != Some(expected_run.as_path())
        || image
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some(state.run_id.as_str())
    {
        remove_state(&state_path);
        return Ok(None);
    }
    Ok(Some(state))
}

fn valid_state(state: &ActiveUpdateState) -> bool {
    state.schema_version == 1
        && state.install_id.len() == 64
        && state
            .install_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        && state.run_id.len() == 36
        && state.run_id.starts_with("run-")
        && state.run_id[4..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        && state.updater_pid > 0
        && !state.target_version.is_empty()
        && state.target_version.len() <= 64
        && matches!(
            state.phase.as_str(),
            "Starting"
                | "WaitingForParent"
                | "FetchingRelease"
                | "VerifyingRelease"
                | "Extracting"
                | "VerifyingStaging"
                | "Preflight"
                | "BackingUp"
                | "Installing"
                | "VerifyingInstall"
                | "Committing"
                | "CleaningUp"
                | "Restarting"
                | "Completed"
                | "Failed"
                | "RolledBack"
        )
        && state.started_at_utc.ends_with('Z')
        && state.updated_at_utc.ends_with('Z')
        && state.started_at_utc.len() <= 64
        && state.updated_at_utc.len() <= 64
}

fn remove_state(path: &Path) {
    let _ = std::fs::remove_file(path);
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

#[cfg(test)]
mod tests {
    use super::{ActiveUpdateState, inspect_active_update_for_install};
    use crate::process::ProcessImage;
    use crate::update_lock::install_id;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sky-updater-active-state-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn state(install: &Path) -> ActiveUpdateState {
        ActiveUpdateState {
            schema_version: 1,
            install_id: install_id(install).expect("install id"),
            run_id: format!("run-{}", "a".repeat(32)),
            updater_pid: 4711,
            target_version: "3.6.0".into(),
            phase: "WaitingForParent".into(),
            started_at_utc: "2026-08-30T00:00:00Z".into(),
            updated_at_utc: "2026-08-30T00:00:00Z".into(),
        }
    }

    #[test]
    fn valid_live_owned_state_blocks_startup() {
        let root = temp_root("live");
        let install = root.join("install");
        let state_root = root.join("Sky-Auto-Player");
        let run = state_root
            .join("update-runs")
            .join(format!("run-{}", "a".repeat(32)));
        fs::create_dir_all(&install).expect("install");
        fs::create_dir_all(&run).expect("run");
        let image = run.join(crate::UPDATER_EXE);
        fs::write(&image, b"fake updater").expect("image");
        let state_path = state_root.join("update-state").join("active-update.json");
        fs::create_dir_all(state_path.parent().expect("state parent")).expect("state dir");
        fs::write(
            &state_path,
            serde_json::to_vec(&state(&install)).expect("state JSON"),
        )
        .expect("state");

        let result = inspect_active_update_for_install(&install, &state_path, |_pid| {
            Ok(ProcessImage::Alive(image.clone()))
        })
        .expect("inspection");
        assert!(result.is_some());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn foreign_state_does_not_block_and_is_not_deleted() {
        let root = temp_root("foreign");
        let install = root.join("install");
        let other = root.join("other");
        fs::create_dir_all(&install).expect("install");
        fs::create_dir_all(&other).expect("other");
        let mut foreign = state(&other);
        foreign.install_id = install_id(&other).expect("other id");
        let state_path = root.join("active-update.json");
        fs::write(
            &state_path,
            serde_json::to_vec(&foreign).expect("state JSON"),
        )
        .expect("state");

        let result = inspect_active_update_for_install(&install, &state_path, |_pid| {
            panic!("foreign state must not query its process")
        })
        .expect("inspection");
        assert!(result.is_none());
        assert!(state_path.is_file());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn malformed_or_dead_owned_state_is_cleaned() {
        let root = temp_root("stale");
        let install = root.join("install");
        fs::create_dir_all(&install).expect("install");
        let state_path = root.join("active-update.json");
        fs::write(&state_path, br#"{"schema_version":1,"unexpected":true}"#)
            .expect("malformed state");
        let result = inspect_active_update_for_install(&install, &state_path, |_pid| {
            panic!("malformed state must not query its process")
        })
        .expect("inspection");
        assert!(result.is_none());
        assert!(!state_path.exists());

        let state_path = root.join("dead-active-update.json");
        fs::write(
            &state_path,
            serde_json::to_vec(&state(&install)).expect("state JSON"),
        )
        .expect("state");
        let result = inspect_active_update_for_install(&install, &state_path, |_pid| {
            Ok(ProcessImage::Exited)
        })
        .expect("inspection");
        assert!(result.is_none());
        assert!(!state_path.exists());
        fs::remove_dir_all(root).expect("cleanup");
    }
}
