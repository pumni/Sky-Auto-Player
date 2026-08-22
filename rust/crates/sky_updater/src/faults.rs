//! Deterministic fault hooks for the separately built E2E binary only.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::error::{Result, UpdaterError};

#[derive(Clone, Debug)]
struct FaultSpec {
    phase: String,
    checkpoint: String,
    target: String,
}

#[derive(Clone, Debug, Default)]
struct FaultConfig {
    failures: Vec<FaultSpec>,
    pause_at: Option<String>,
    fail_restart: bool,
}

static CONFIG: OnceLock<Mutex<FaultConfig>> = OnceLock::new();

pub fn configure(fail_at: Option<&str>, pause_at: Option<&str>) -> Result<()> {
    let mut failures = Vec::new();
    if let Some(specs) = fail_at {
        for spec in specs.split(',') {
            let mut parts = spec.splitn(3, ':');
            let phase = parts.next().unwrap_or_default();
            let checkpoint = parts.next().unwrap_or_default();
            let target = parts.next().unwrap_or_default();
            if !matches!(phase, "apply" | "rollback")
                || !matches!(
                    checkpoint,
                    "before-replace" | "after-replace" | "after-restore"
                )
                || target.is_empty()
                || (checkpoint == "after-restore" && phase != "rollback")
            {
                return Err(UpdaterError::InvalidArgument(
                    "--fail-at must be <phase>:<before-replace|after-replace|after-restore>:<relative-path>; separate multiple specs with commas".into(),
                ));
            }
            failures.push(FaultSpec {
                phase: phase.into(),
                checkpoint: checkpoint.into(),
                target: target.into(),
            });
        }
    }
    let config = FaultConfig {
        failures,
        pause_at: pause_at.map(str::to_owned),
        fail_restart: false,
    };
    CONFIG
        .get_or_init(|| Mutex::new(config.clone()))
        .lock()
        .map_err(|_| UpdaterError::InstallCopyFailed("fault configuration poisoned".into()))?
        .clone_from(&config);
    Ok(())
}

pub fn set_restart_failure(enabled: bool) -> Result<()> {
    let config = CONFIG
        .get_or_init(|| Mutex::new(FaultConfig::default()))
        .lock()
        .map_err(|_| UpdaterError::InstallCopyFailed("fault configuration poisoned".into()))?;
    let mut config = config;
    config.fail_restart = enabled;
    Ok(())
}

pub fn restart_should_fail() -> bool {
    CONFIG
        .get()
        .and_then(|config| config.lock().ok())
        .is_some_and(|config| config.fail_restart)
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
            config.failures.iter().any(|failure| {
                failure.phase == phase
                    && failure.checkpoint == "before-replace"
                    && (failure.target == path || failure.target == index.to_string())
            })
        });
    if should_fail {
        return Err(UpdaterError::InstallCopyFailed(format!(
            "deterministic E2E fault at {phase}:before-replace:{path}"
        )));
    }
    Ok(())
}

pub fn after_replace(phase: &str, path: &str) -> Result<()> {
    if matches_checkpoint(phase, "after-replace", path) {
        return Err(UpdaterError::InstallCopyFailed(format!(
            "deterministic E2E fault at {phase}:after-replace:{path}"
        )));
    }
    Ok(())
}

pub fn after_restore(path: &str) -> Result<()> {
    if matches_checkpoint("rollback", "after-restore", path) {
        return Err(UpdaterError::RollbackAtomicReplaceFailed {
            path: path.into(),
            os_code: 0,
            message: format!("deterministic E2E fault at rollback:after-restore:{path}"),
        });
    }
    Ok(())
}

fn matches_checkpoint(phase: &str, checkpoint: &str, path: &str) -> bool {
    CONFIG
        .get()
        .and_then(|config| config.lock().ok())
        .is_some_and(|config| {
            config.failures.iter().any(|failure| {
                failure.phase == phase && failure.checkpoint == checkpoint && failure.target == path
            })
        })
}
