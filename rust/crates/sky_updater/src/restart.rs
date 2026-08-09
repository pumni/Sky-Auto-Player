use std::path::Path;
use std::process::Command;

use crate::error::{Result, UpdaterError};
use crate::{APP_NAME, PRIMARY_EXE};

pub fn restart_verified(install_root: &Path) -> Result<()> {
    let executable = install_root.join(PRIMARY_EXE);
    if !executable.is_file()
        || executable.file_name().and_then(|name| name.to_str()) != Some(PRIMARY_EXE)
    {
        return Err(UpdaterError::RestartFailed(
            "canonical app executable is missing".into(),
        ));
    }
    Command::new(&executable)
        .current_dir(install_root)
        .spawn()
        .map(|_| ())
        .map_err(|err| {
            UpdaterError::RestartFailed(format!("{APP_NAME} could not be started: {err}"))
        })
}
