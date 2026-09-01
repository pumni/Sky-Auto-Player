//! Production startup admission for updater transactions.
//!
//! This module deliberately has no dependency on the retired Python Core
//! launcher.  The updater's active-state record is the only startup guard.

use std::path::PathBuf;

use sky_updater::active_state::ActiveUpdateState;

#[derive(Debug)]
pub(crate) struct StartupGuardError(String);

impl std::fmt::Display for StartupGuardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for StartupGuardError {}

fn repository_root() -> PathBuf {
    let configured = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..\\..");
    configured.canonicalize().unwrap_or(configured)
}

pub(crate) fn desktop_install_root() -> Result<PathBuf, StartupGuardError> {
    let current_exe = std::env::current_exe().map_err(|error| {
        StartupGuardError(format!("cannot resolve desktop executable: {error}"))
    })?;
    Ok(if cfg!(debug_assertions) {
        repository_root()
    } else {
        current_exe
            .parent()
            .ok_or_else(|| StartupGuardError("desktop executable has no parent directory".into()))?
            .to_path_buf()
    })
}

pub(crate) fn check_startup_update_guard() -> Result<(), StartupGuardError> {
    let root = desktop_install_root()?;
    let active = sky_updater::active_state::active_update_for_install(&root)
        .map_err(|error| StartupGuardError(format!("update startup guard failed: {error}")))?;
    enforce_update_startup_admission(active.as_ref())
}

fn enforce_update_startup_admission(
    active: Option<&ActiveUpdateState>,
) -> Result<(), StartupGuardError> {
    if let Some(state) = active {
        return Err(StartupGuardError(format!(
            "an updater transaction is already active for this installation ({})",
            state.run_id
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::enforce_update_startup_admission;
    use sky_updater::active_state::ActiveUpdateState;

    #[test]
    fn startup_admission_allows_clear_state() {
        enforce_update_startup_admission(None).expect("clear state starts normally");
    }

    #[test]
    fn startup_admission_rejects_live_owned_transaction() {
        let state = ActiveUpdateState {
            schema_version: 1,
            install_id: "a".repeat(64),
            run_id: format!("run-{}", "b".repeat(32)),
            updater_pid: 4711,
            target_version: "3.6.0".into(),
            phase: "WaitingForParent".into(),
            started_at_utc: "2026-08-30T00:00:00Z".into(),
            updated_at_utc: "2026-08-30T00:00:00Z".into(),
        };
        let error = enforce_update_startup_admission(Some(&state))
            .expect_err("active updater must block ordinary GUI startup");
        assert!(error.to_string().contains("already active"));
        assert!(error.to_string().contains(&state.run_id));
    }
}
