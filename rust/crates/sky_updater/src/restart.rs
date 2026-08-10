use std::path::Path;
use std::process::Command;

use crate::error::{Result, UpdaterError};
use crate::install::installed_manifest;
use crate::signature::verify_project_files;
use crate::transaction::safe_join;
use crate::{APP_NAME, PRIMARY_EXE};

pub fn restart_verified(install_root: &Path) -> Result<()> {
    let executable = safe_join(install_root, PRIMARY_EXE)?;
    if !executable.is_file()
        || executable.file_name().and_then(|name| name.to_str()) != Some(PRIMARY_EXE)
    {
        return Err(UpdaterError::RestartFailed(
            "canonical app executable is missing".into(),
        ));
    }
    let manifest = installed_manifest(install_root).map_err(|error| {
        UpdaterError::RestartFailed(format!("installed manifest is not valid: {error}"))
    })?;
    verify_project_files(install_root, &manifest).map_err(|error| {
        UpdaterError::RestartFailed(format!(
            "installed project files failed integrity check: {error}"
        ))
    })?;
    Command::new(&executable)
        .current_dir(install_root)
        .spawn()
        .map(|_| ())
        .map_err(|err| {
            UpdaterError::RestartFailed(format!("{APP_NAME} could not be started: {err}"))
        })
}
