use std::path::{Path, PathBuf};
use std::process::Command;

use super::supervisor::SupervisorError;

const CORE_EXE_NAME: &str = "Sky-Auto-Player-Core.exe";

fn repository_root() -> PathBuf {
    let configured = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..\\..");
    configured.canonicalize().unwrap_or(configured)
}

fn apply_no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
}

pub(crate) fn desktop_install_root() -> Result<PathBuf, SupervisorError> {
    let current_exe =
        std::env::current_exe().map_err(|error| SupervisorError::Launch(error.to_string()))?;
    Ok(if cfg!(debug_assertions) {
        repository_root()
    } else {
        current_exe
            .parent()
            .ok_or_else(|| {
                SupervisorError::Launch("desktop executable has no parent directory".into())
            })?
            .to_path_buf()
    })
}

pub(crate) fn check_startup_update_guard() -> Result<(), SupervisorError> {
    let root = desktop_install_root()?;
    let active = sky_updater::active_state::active_update_for_install(&root).map_err(|error| {
        SupervisorError::Launch(format!("update startup guard failed: {error}"))
    })?;
    enforce_update_startup_admission(active.as_ref())
}

fn enforce_update_startup_admission(
    active: Option<&sky_updater::active_state::ActiveUpdateState>,
) -> Result<(), SupervisorError> {
    if let Some(state) = active {
        return Err(SupervisorError::Launch(format!(
            "an updater transaction is already active for this installation ({})",
            state.run_id
        )));
    }
    Ok(())
}

pub(crate) fn build_core_command() -> Result<Command, SupervisorError> {
    let root = desktop_install_root()?;
    let mut command = if cfg!(debug_assertions) {
        let python = root.join(".venv\\Scripts\\python.exe");
        if !python.is_file() {
            return Err(SupervisorError::Launch(format!(
                "debug Python interpreter not found: {}",
                python.display()
            )));
        }
        let entrypoint = root.join("src\\core_main.py");
        if !entrypoint.is_file() {
            return Err(SupervisorError::Launch(format!(
                "desktop Core entrypoint not found: {}",
                entrypoint.display()
            )));
        }
        let mut command = Command::new(python);
        command.arg(entrypoint);
        command
    } else {
        let core = root.join(CORE_EXE_NAME);
        if !core.is_file() {
            return Err(SupervisorError::Launch(format!(
                "packaged Core sidecar not found: {}",
                core.display()
            )));
        }
        Command::new(core)
    };
    command
        .arg("--desktop-worker")
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .arg("--install-root")
        .arg(root.as_os_str())
        .current_dir(Path::new(&root));
    apply_no_window(&mut command);
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::enforce_update_startup_admission;
    use sky_updater::active_state::ActiveUpdateState;

    fn active_state() -> ActiveUpdateState {
        ActiveUpdateState {
            schema_version: 1,
            install_id: "a".repeat(64),
            run_id: format!("run-{}", "b".repeat(32)),
            updater_pid: 4711,
            target_version: "3.6.0".into(),
            phase: "WaitingForParent".into(),
            started_at_utc: "2026-08-30T00:00:00Z".into(),
            updated_at_utc: "2026-08-30T00:00:00Z".into(),
        }
    }

    #[test]
    fn startup_admission_allows_clear_state() {
        enforce_update_startup_admission(None).expect("clear state starts normally");
    }

    #[test]
    fn startup_admission_rejects_live_owned_transaction_before_gui_start() {
        let state = active_state();
        let error = enforce_update_startup_admission(Some(&state))
            .expect_err("active updater must block ordinary GUI startup");
        assert!(error.to_string().contains("already active"));
        assert!(error.to_string().contains(&state.run_id));
    }
}
