//! Deterministic fault hooks for the separately built E2E binary only.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::error::{Result, UpdaterError};

#[derive(Clone, Debug)]
struct FaultConfig {
    fail_phase: Option<String>,
    fail_index: Option<usize>,
    pause_at: Option<String>,
}

static CONFIG: OnceLock<Mutex<FaultConfig>> = OnceLock::new();

pub fn configure(fail_at: Option<&str>, pause_at: Option<&str>) -> Result<()> {
    let (fail_phase, fail_index) = if let Some(spec) = fail_at {
        let mut parts = spec.split(':');
        let phase = parts.next().unwrap_or_default();
        let operation = parts.next().unwrap_or_default();
        let index = parts.next().unwrap_or_default();
        if operation != "before-replace"
            || phase.is_empty()
            || parts.next().is_some()
            || index
                .parse::<usize>()
                .ok()
                .filter(|value| *value > 0)
                .is_none()
        {
            return Err(UpdaterError::InvalidArgument(
                "--fail-at must be <apply|rollback>:before-replace:<positive-index>".into(),
            ));
        }
        (Some(phase.into()), index.parse().ok())
    } else {
        (None, None)
    };
    let config = FaultConfig {
        fail_phase,
        fail_index,
        pause_at: pause_at.map(str::to_owned),
    };
    CONFIG
        .get_or_init(|| Mutex::new(config.clone()))
        .lock()
        .map_err(|_| UpdaterError::InstallCopyFailed("fault configuration poisoned".into()))?
        .clone_from(&config);
    Ok(())
}

pub fn pause_at(point: &str) {
    let should_pause = CONFIG
        .get()
        .and_then(|config| config.lock().ok())
        .is_some_and(|config| config.pause_at.as_deref() == Some(point));
    if should_pause {
        std::thread::sleep(Duration::from_secs(30));
    }
}

pub fn before_replace(phase: &str, index: usize, path: &str) -> Result<()> {
    let should_fail = CONFIG
        .get()
        .and_then(|config| config.lock().ok())
        .is_some_and(|config| {
            config.fail_phase.as_deref() == Some(phase) && config.fail_index == Some(index)
        });
    if should_fail {
        return Err(UpdaterError::InstallCopyFailed(format!(
            "deterministic E2E fault at {phase}:before-replace:{index} ({path})"
        )));
    }
    Ok(())
}
