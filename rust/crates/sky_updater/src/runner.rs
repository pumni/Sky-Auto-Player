use std::env;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use crate::active_state::ActiveUpdateGuard;
use crate::archive::extract_zip_file;
use crate::cli::{self, ParseResult, UpdaterArgs};
use crate::error::{Result, UpdaterError};
use crate::file_replace::{
    CleanupFailure, CleanupReport, remove_owned_run_file, remove_owned_tree,
};
use crate::github::{GitHubReleaseSource, ReleaseSource};
use crate::handoff;
use crate::install::{inspect_archive, install_verified, installed_manifest, read_staged_manifest};
use crate::process::wait_for_parent;
use crate::progress::{ProgressEvent, ProgressSink, UpdatePhase};
use crate::progress_ui::NativeProgressUi;
use crate::recovery::{has_unresolved_transaction, recover_before_update, rollback_prepared};
use crate::restart::restart_verified;
use crate::result;
use crate::signature::verify_project_files;
use crate::transaction::{cleanup_committed, safe_join};
use crate::update_lock::UpdateLock;

const PARENT_WAIT: Duration = Duration::from_secs(30);

struct UpdateProgress<'a> {
    ui: &'a NativeProgressUi,
    active: &'a ActiveUpdateGuard,
    last_persisted_phase: Mutex<Option<UpdatePhase>>,
}

impl ProgressSink for UpdateProgress<'_> {
    fn publish(&self, event: ProgressEvent) -> Result<()> {
        // Counters are UI telemetry, not active-state boundaries. Persisting
        // active-update.json for every managed file would turn a large update
        // into hundreds of read/flush/replace operations and make a transient
        // state-file contention fail the transaction. Keep the durable state
        // at phase boundaries while still rendering every counter.
        let mut last_phase = self
            .last_persisted_phase
            .lock()
            .map_err(|_| UpdaterError::Io(std::io::Error::other("progress phase lock poisoned")))?;
        if *last_phase != Some(event.phase) {
            self.active.set_phase(event.phase)?;
            *last_phase = Some(event.phase);
        }
        drop(last_phase);
        self.ui.set_phase(event.phase, event.current, event.total);
        Ok(())
    }
}

#[derive(Debug)]
pub struct ExecutionFailure {
    pub error: UpdaterError,
    pub rolled_back: bool,
}

#[derive(Debug)]
pub enum UpdateExecutionOutcome {
    Success(UpdateSuccess),
    RolledBack(UpdateRollback),
    Failure(UpdateFailure),
    DryRun,
}

#[derive(Debug)]
pub struct UpdateSuccess {
    pub warnings: Vec<result::UpdateWarning>,
    pub cleanup_pending: bool,
}

#[derive(Debug)]
pub struct UpdateRollback {
    pub cause: UpdaterError,
}

#[derive(Debug)]
pub struct UpdateFailure {
    pub error: UpdaterError,
}

impl From<std::result::Result<(), ExecutionFailure>> for UpdateExecutionOutcome {
    fn from(value: std::result::Result<(), ExecutionFailure>) -> Self {
        match value {
            Ok(()) => Self::Success(UpdateSuccess {
                warnings: Vec::new(),
                cleanup_pending: false,
            }),
            Err(failure) if failure.rolled_back => Self::RolledBack(UpdateRollback {
                cause: failure.error,
            }),
            Err(failure) => Self::Failure(UpdateFailure {
                error: failure.error,
            }),
        }
    }
}

impl From<UpdaterError> for ExecutionFailure {
    fn from(error: UpdaterError) -> Self {
        Self {
            error,
            rolled_back: false,
        }
    }
}

pub fn run_production<I>(values: I) -> Result<()>
where
    I: Iterator<Item = String>,
{
    match cli::parse(values)? {
        ParseResult::Help => {
            print_help();
            Ok(())
        }
        ParseResult::Version => {
            println!("{} {}", crate::APP_NAME, env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        ParseResult::Args(args) => run_update_with_source(&args, &GitHubReleaseSource),
    }
}

pub fn run_update_with_source<S: ReleaseSource>(args: &UpdaterArgs, source: &S) -> Result<()> {
    let run_root = updater_run_root()?;
    // This guard deliberately surrounds parent wait, recovery, network,
    // preflight, transaction, result write, and restart.
    let lock = match UpdateLock::acquire(&args.install_root) {
        Ok(lock) => lock,
        Err(UpdaterError::UpdateAlreadyRunning) => {
            handoff::write_rejected(
                &run_root,
                &args.target_version,
                "UPDATE_ALREADY_RUNNING",
                "another updater is already running",
            )?;
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let mut ui = match NativeProgressUi::start(&args.current_version, &args.target_version) {
        Ok(ui) => ui,
        Err(error) => {
            let _ = handoff::write_rejected(
                &run_root,
                &args.target_version,
                "UI_INITIALIZATION_FAILED",
                &error.to_string(),
            );
            drop(lock);
            return Err(error);
        }
    };
    let active =
        match ActiveUpdateGuard::create(&args.install_root, &run_root, &args.target_version) {
            Ok(active) => active,
            Err(error) => {
                let _ = handoff::write_rejected(
                    &run_root,
                    &args.target_version,
                    "IO_FAILURE",
                    &error.to_string(),
                );
                return Err(error);
            }
        };
    let progress = UpdateProgress {
        ui: &ui,
        active: &active,
        // ActiveUpdateGuard::create already persisted Starting before the
        // handoff. Treat that durable write as the initial phase boundary so
        // READY cannot be followed by a redundant fallible filesystem write.
        last_persisted_phase: Mutex::new(Some(UpdatePhase::Starting)),
    };
    handoff::write_ready(&run_root, &args.target_version)?;
    #[cfg(feature = "e2e-fault-injection")]
    crate::faults::pause_at("after-lock");
    let outcome = execute_update(args, source, &progress, &run_root);
    let terminal = match &outcome {
        UpdateExecutionOutcome::Failure(failure) => {
            Some(("Update failed", failure.error.to_string()))
        }
        UpdateExecutionOutcome::RolledBack(rollback) => {
            Some(("Update rolled back", rollback.cause.to_string()))
        }
        _ => None,
    };
    let was_success = matches!(&outcome, UpdateExecutionOutcome::Success(_));
    let was_rollback = matches!(&outcome, UpdateExecutionOutcome::RolledBack(_));
    let result = finalize_update(args, outcome, |root| {
        active.set_phase(UpdatePhase::Restarting)?;
        ui.show_restarting();
        active.remove();
        #[cfg(feature = "e2e-fault-injection")]
        if crate::faults::restart_should_fail() {
            return Err(UpdaterError::RestartFailed(
                "deterministic E2E restart failure".into(),
            ));
        }
        restart_verified(root)
    });
    if let Some((title, message)) = terminal {
        if was_rollback {
            ui.show_rolled_back(&message);
        } else {
            ui.show_failure(title, &message);
        }
        ui.wait_for_user_close();
    } else if was_success && result.is_err() {
        ui.show_restart_failure(
            &result
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_default(),
        );
        ui.wait_for_user_close();
    } else if result.is_ok() {
        ui.close_after_success();
        ui.wait_for_user_close();
    }
    result
}

pub fn finalize_update<F, O>(args: &UpdaterArgs, outcome: O, restart: F) -> Result<()>
where
    F: FnOnce(&Path) -> Result<()>,
    O: Into<UpdateExecutionOutcome>,
{
    let outcome = outcome.into();
    let record = match &outcome {
        UpdateExecutionOutcome::DryRun => {
            result::dry_run(&args.current_version, &args.target_version)
        }
        UpdateExecutionOutcome::Success(success) => result::success_with_warnings(
            &args.current_version,
            &args.target_version,
            success.warnings.clone(),
            success.cleanup_pending,
        ),
        UpdateExecutionOutcome::RolledBack(rollback) => {
            result::rolled_back(&args.current_version, &args.target_version, &rollback.cause)
        }
        UpdateExecutionOutcome::Failure(failure) => {
            result::failure(&args.current_version, &args.target_version, &failure.error)
        }
    };
    if let Err(write_error) = result::write_result(&record) {
        eprintln!("could not write updater result: {write_error}");
        return Err(write_error);
    }
    if let Err(log_error) = result::append_log(&record) {
        eprintln!("could not append updater log: {log_error}");
    }
    let should_restart = args.restart
        && !args.dry_run
        && matches!(
            outcome,
            UpdateExecutionOutcome::Success(_) | UpdateExecutionOutcome::RolledBack(_)
        );
    if should_restart && let Err(restart_error) = restart(&args.install_root) {
        eprintln!("could not restart verified application: {restart_error}");
        if let UpdateExecutionOutcome::Success(success) = &outcome {
            let restart_record = result::failure_with_warnings(
                &args.current_version,
                &args.target_version,
                &restart_error,
                success.warnings.clone(),
                success.cleanup_pending,
            );
            if let Err(write_error) = result::write_result(&restart_record) {
                eprintln!("could not write restart-failure result: {write_error}");
                return Err(write_error);
            }
            if let Err(log_error) = result::append_log(&restart_record) {
                eprintln!("could not append restart-failure log: {log_error}");
            }
            return Err(restart_error);
        }
    }
    match outcome {
        UpdateExecutionOutcome::Success(_) | UpdateExecutionOutcome::DryRun => Ok(()),
        UpdateExecutionOutcome::RolledBack(rollback) => Err(rollback.cause),
        UpdateExecutionOutcome::Failure(failure) => Err(failure.error),
    }
}

fn execute_update<S: ReleaseSource>(
    args: &UpdaterArgs,
    source: &S,
    progress: &dyn ProgressSink,
    run_root: &Path,
) -> UpdateExecutionOutcome {
    let result = execute_update_inner(args, source, progress, run_root);
    match result {
        Ok(_success) if args.dry_run => UpdateExecutionOutcome::DryRun,
        Ok(success) => UpdateExecutionOutcome::Success(success),
        Err(failure) if failure.rolled_back => UpdateExecutionOutcome::RolledBack(UpdateRollback {
            cause: failure.error,
        }),
        Err(failure) => UpdateExecutionOutcome::Failure(UpdateFailure {
            error: failure.error,
        }),
    }
}

fn execute_update_inner<S: ReleaseSource>(
    args: &UpdaterArgs,
    source: &S,
    progress: &dyn ProgressSink,
    run_root: &Path,
) -> std::result::Result<UpdateSuccess, ExecutionFailure> {
    let primary_exe = safe_join(&args.install_root, crate::PRIMARY_EXE)?;
    progress.publish(ProgressEvent {
        phase: UpdatePhase::WaitingForParent,
        current: None,
        total: None,
    })?;
    wait_for_parent(args.parent_pid, PARENT_WAIT, &primary_exe)?;
    if has_unresolved_transaction(&args.install_root) {
        if args.dry_run {
            return Err(UpdaterError::TransactionRecoveryRequired(
                "dry-run refuses to inspect an unresolved transaction".into(),
            )
            .into());
        }
        recover_before_update(&args.install_root)?;
    }

    let updater_path =
        env::current_exe().map_err(|error| UpdaterError::InstallRootInvalid(error.to_string()))?;
    crate::signature::verify_file(&updater_path)?;

    let zip_path = run_root.join("release.zip");
    let staging = run_root.join("staging");
    (|| -> std::result::Result<UpdateSuccess, ExecutionFailure> {
        progress.publish(ProgressEvent {
            phase: UpdatePhase::FetchingRelease,
            current: None,
            total: None,
        })?;
        let payload = source.fetch_exact_release(&args.target_version, args.channel, &zip_path)?;
        progress.publish(ProgressEvent {
            phase: UpdatePhase::VerifyingRelease,
            current: None,
            total: None,
        })?;
        inspect_archive(&payload.zip_path)?;
        progress.publish(ProgressEvent {
            phase: UpdatePhase::Extracting,
            current: None,
            total: None,
        })?;
        extract_zip_file(&payload.zip_path, &staging)?;
        progress.publish(ProgressEvent {
            phase: UpdatePhase::VerifyingStaging,
            current: None,
            total: None,
        })?;
        let staged_manifest = read_staged_manifest(&staging, &args.target_version)?;
        if staged_manifest != payload.manifest {
            return Err(UpdaterError::ManifestHashMismatch(
                "embedded MANIFEST.json differs from release MANIFEST.json".into(),
            )
            .into());
        }
        verify_project_files(&staging, &staged_manifest)?;
        let old_manifest = installed_manifest(&args.install_root)?;
        if args.dry_run {
            return Ok(UpdateSuccess {
                warnings: Vec::new(),
                cleanup_pending: false,
            });
        }
        #[cfg(feature = "e2e-fault-injection")]
        crate::faults::pause_at("before-apply");
        let install = match install_verified(
            &args.install_root,
            &staging,
            &staged_manifest,
            &old_manifest,
            progress,
        ) {
            Ok(report) => report,
            Err(error) => {
                if has_unresolved_transaction(&args.install_root) {
                    if let Err(rollback_error) = rollback_prepared(&args.install_root) {
                        let combined = match rollback_error {
                            UpdaterError::RollbackAtomicReplaceFailed {
                                path,
                                os_code,
                                message,
                            } => UpdaterError::RollbackAtomicReplaceFailed {
                                path,
                                os_code,
                                message: format!("{error}; rollback failed: {message}"),
                            },
                            other => UpdaterError::RollbackFailed(format!(
                                "{error}; rollback failed: {other}"
                            )),
                        };
                        return Err(ExecutionFailure {
                            error: combined,
                            rolled_back: false,
                        });
                    }
                    return Err(ExecutionFailure {
                        error,
                        rolled_back: true,
                    });
                }
                return Err(error.into());
            }
        };
        publish_best_effort(progress, UpdatePhase::CleaningUp, None, None);
        let committed_cleanup = cleanup_committed(&args.install_root)?;
        let run_cleanup = cleanup_run_files(run_root);
        let mut warnings = Vec::new();
        // cleanup_committed() retries updater-owned stale artifacts after the
        // transaction is committed. A replacement failure recorded during
        // apply is only still pending if that exact path survived the later
        // sweep; do not surface a warning for an artifact already removed.
        let unresolved_install_cleanup = install
            .transaction
            .cleanup_failures
            .into_iter()
            .filter(|failure| failure.path.exists())
            .collect();
        warnings.extend(cleanup_warnings(
            "ARTIFACT_CLEANUP_FAILED",
            unresolved_install_cleanup,
        ));
        for failure in committed_cleanup.failures {
            let code = if failure
                .path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|value| value.starts_with(".sky-update-"))
            {
                "ARTIFACT_CLEANUP_FAILED"
            } else {
                "COMMITTED_TRANSACTION_CLEANUP_FAILED"
            };
            warnings.extend(cleanup_warnings(code, vec![failure]));
        }
        warnings.extend(cleanup_warnings("RUN_CLEANUP_FAILED", run_cleanup.failures));
        Ok(UpdateSuccess {
            cleanup_pending: !warnings.is_empty(),
            warnings,
        })
    })()
}

fn publish_best_effort(
    progress: &dyn ProgressSink,
    phase: UpdatePhase,
    current: Option<u64>,
    total: Option<u64>,
) {
    let _ = progress.publish(ProgressEvent {
        phase,
        current,
        total,
    });
}

fn cleanup_warnings(code: &str, failures: Vec<CleanupFailure>) -> Vec<result::UpdateWarning> {
    failures
        .into_iter()
        .map(|failure| result::UpdateWarning {
            code: code.into(),
            message: failure.error.to_string(),
            phase: Some("cleanup".into()),
            operation: Some("remove updater-owned artifact".into()),
            path: Some(failure.path.display().to_string()),
            os_error: failure
                .error
                .raw_os_error()
                .map(|value| value.unsigned_abs()),
        })
        .collect()
}

pub fn updater_run_root() -> Result<PathBuf> {
    let current_exe =
        env::current_exe().map_err(|error| UpdaterError::InstallRootInvalid(error.to_string()))?;
    if current_exe.file_name().and_then(|name| name.to_str()) != Some(crate::UPDATER_EXE) {
        return Err(UpdaterError::InstallRootInvalid(
            "updater executable has a noncanonical name".into(),
        ));
    }
    let run_root = current_exe
        .parent()
        .ok_or_else(|| UpdaterError::InstallRootInvalid("updater has no parent directory".into()))?
        .to_owned();
    let local_app_data = env::var_os("LOCALAPPDATA")
        .ok_or_else(|| UpdaterError::InstallRootInvalid("LOCALAPPDATA is unavailable".into()))?;
    let runs = PathBuf::from(local_app_data)
        .join(crate::APP_NAME)
        .join("update-runs")
        .canonicalize()
        .map_err(|error| UpdaterError::InstallRootInvalid(error.to_string()))?;
    let relative = run_root
        .canonicalize()
        .map_err(|error| UpdaterError::InstallRootInvalid(error.to_string()))?
        .strip_prefix(&runs)
        .map_err(|_| {
            UpdaterError::InstallRootInvalid("updater run directory is not allow-listed".into())
        })?
        .to_owned();
    let mut components = relative.components();
    let Some(component) = components.next() else {
        return Err(UpdaterError::InstallRootInvalid(
            "updater run directory is missing".into(),
        ));
    };
    if components.next().is_some()
        || !component.as_os_str().to_string_lossy().starts_with("run-")
        || component.as_os_str().to_string_lossy().len() != 36
        || !component
            .as_os_str()
            .to_string_lossy()
            .as_bytes()
            .iter()
            .skip(4)
            .all(u8::is_ascii_hexdigit)
    {
        return Err(UpdaterError::InstallRootInvalid(
            "updater run directory name is invalid".into(),
        ));
    }
    Ok(run_root)
}

fn cleanup_run_files(run_root: &Path) -> CleanupReport {
    let mut report = CleanupReport::default();
    for name in ["release.zip", "staging"] {
        let path = run_root.join(name);
        if path.is_dir() {
            if let Err(error) = remove_owned_tree(&path) {
                report.failures.push(CleanupFailure { path, error });
            }
        } else if path.is_file()
            && let Err(error) = remove_owned_run_file(&path)
        {
            report.failures.push(CleanupFailure { path, error });
        }
    }
    report
}

fn print_help() {
    println!(
        "Sky Auto Player updater\n\nUsage:\n  Sky-Auto-Player-Updater.exe --install-root <absolute-path> --parent-pid <pid> --current-version <version> --target-version <version> --channel <stable|beta> [--restart] [--dry-run]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_ENV_LOCK: Mutex<()> = Mutex::new(());

    struct LocalAppDataGuard {
        previous: Option<OsString>,
    }

    impl LocalAppDataGuard {
        fn set(path: &Path) -> Self {
            let previous = std::env::var_os("LOCALAPPDATA");
            unsafe { std::env::set_var("LOCALAPPDATA", path) };
            Self { previous }
        }
    }

    impl Drop for LocalAppDataGuard {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(value) => unsafe { std::env::set_var("LOCALAPPDATA", value) },
                None => unsafe { std::env::remove_var("LOCALAPPDATA") },
            }
        }
    }

    #[test]
    fn restart_failure_overwrites_success_result_without_rollback() {
        let _env_lock = TEST_ENV_LOCK.lock().expect("test environment lock");
        let root = std::env::temp_dir().join(format!(
            "sky-updater-restart-failure-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let install_root = root.join("install");
        std::fs::create_dir_all(&install_root).expect("install root");
        let marker = install_root.join("new-version-marker");
        std::fs::write(&marker, b"new files stay installed").expect("marker");
        let _local_app_data = LocalAppDataGuard::set(&root.join("local"));
        let args = UpdaterArgs {
            install_root: install_root.clone(),
            parent_pid: 1,
            current_version: "1.0.0".into(),
            target_version: "2.0.0".into(),
            channel: cli::Channel::Stable,
            restart: true,
            dry_run: false,
        };

        let result = finalize_update(&args, Ok(()), |_root| {
            Err(UpdaterError::RestartFailed(
                "injected restart failure".into(),
            ))
        });

        assert!(matches!(result, Err(UpdaterError::RestartFailed(_))));
        assert_eq!(
            std::fs::read(&marker).expect("marker remains"),
            b"new files stay installed"
        );
        let result_path = result::result_dir()
            .expect("result directory")
            .join("last-result.json");
        let saved: result::UpdateResult =
            serde_json::from_slice(&std::fs::read(result_path).expect("result file"))
                .expect("valid result JSON");
        assert_eq!(saved.status, "failure");
        assert_eq!(saved.error_code.as_deref(), Some("RESTART_FAILED"));
        assert!(
            saved
                .message
                .as_deref()
                .is_some_and(|message| message.contains("injected restart failure"))
        );

        std::fs::remove_dir_all(root).expect("cleanup restart-failure fixture");
    }

    #[test]
    fn restart_failure_preserves_success_warnings_and_cleanup_pending() {
        let _env_lock = TEST_ENV_LOCK.lock().expect("test environment lock");
        let root = std::env::temp_dir().join(format!(
            "sky-updater-restart-warning-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let install_root = root.join("install");
        std::fs::create_dir_all(&install_root).expect("install root");
        let _local_app_data = LocalAppDataGuard::set(&root.join("local"));
        let args = UpdaterArgs {
            install_root,
            parent_pid: 1,
            current_version: "1.0.0".into(),
            target_version: "2.0.0".into(),
            channel: cli::Channel::Stable,
            restart: true,
            dry_run: false,
        };
        let warning = result::UpdateWarning {
            code: "CLEANUP_FAILED".into(),
            message: "backup remains locked".into(),
            phase: Some("CleaningUp".into()),
            operation: Some("remove backup".into()),
            path: Some(r"C:\install\.sky-update-1.bak".into()),
            os_error: Some(32),
        };

        let outcome = UpdateExecutionOutcome::Success(UpdateSuccess {
            warnings: vec![warning],
            cleanup_pending: true,
        });
        let result = finalize_update(&args, outcome, |_root| {
            Err(UpdaterError::RestartFailed("restart still failed".into()))
        });

        assert!(matches!(result, Err(UpdaterError::RestartFailed(_))));
        let result_path = result::result_dir()
            .expect("result directory")
            .join("last-result.json");
        let saved: result::UpdateResult =
            serde_json::from_slice(&std::fs::read(result_path).expect("result file"))
                .expect("valid result JSON");
        assert_eq!(saved.status, "failure");
        assert!(saved.cleanup_pending);
        assert_eq!(saved.warnings.len(), 1);
        assert_eq!(
            saved.warnings[0].path.as_deref(),
            Some(r"C:\install\.sky-update-1.bak")
        );

        std::fs::remove_dir_all(root).expect("cleanup restart-warning fixture");
    }
}
