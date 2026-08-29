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

pub(crate) fn build_core_command() -> Result<Command, SupervisorError> {
    let current_exe =
        std::env::current_exe().map_err(|error| SupervisorError::Launch(error.to_string()))?;
    let root = if cfg!(debug_assertions) {
        repository_root()
    } else {
        current_exe
            .parent()
            .ok_or_else(|| {
                SupervisorError::Launch("desktop executable has no parent directory".into())
            })?
            .to_path_buf()
    };
    let mut command = if cfg!(debug_assertions) {
        let python = root.join(".venv\\Scripts\\python.exe");
        if !python.is_file() {
            return Err(SupervisorError::Launch(format!(
                "debug Python interpreter not found: {}",
                python.display()
            )));
        }
        let helper = root.join("scripts\\dev_desktop.py");
        if !helper.is_file() {
            return Err(SupervisorError::Launch(format!(
                "desktop dev helper not found: {}",
                helper.display()
            )));
        }
        let mut command = Command::new(python);
        command.arg(helper);
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
