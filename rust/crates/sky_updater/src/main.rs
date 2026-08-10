use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use sky_updater::archive::extract_zip_file;
use sky_updater::cli::{self, ParseResult, UpdaterArgs};
use sky_updater::error::{Result, UpdaterError};
use sky_updater::github::fetch_exact_release;
use sky_updater::http::WinHttpClient;
use sky_updater::install::{
    inspect_archive, install_verified, installed_manifest, read_staged_manifest,
};
use sky_updater::process::wait_for_parent;
use sky_updater::recovery::{has_unresolved_transaction, recover_before_update, rollback_prepared};
use sky_updater::restart::restart_verified;
use sky_updater::result;
use sky_updater::signature::verify_project_files;
use sky_updater::transaction::{cleanup_committed, safe_join};

const PARENT_WAIT: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct ExecutionFailure {
    error: UpdaterError,
    rolled_back: bool,
}

impl From<UpdaterError> for ExecutionFailure {
    fn from(error: UpdaterError) -> Self {
        Self {
            error,
            rolled_back: false,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Sky Auto Player updater failed: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<()> {
    match cli::parse(env::args())? {
        ParseResult::Help => {
            print_help();
            Ok(())
        }
        ParseResult::Version => {
            println!("{} {}", sky_updater::APP_NAME, env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        ParseResult::Args(args) => run_update(&args),
    }
}

fn run_update(args: &UpdaterArgs) -> Result<()> {
    let outcome = execute_update(args);
    finalize_update(args, outcome, restart_verified)
}

fn finalize_update<F>(
    args: &UpdaterArgs,
    outcome: std::result::Result<(), ExecutionFailure>,
    restart: F,
) -> Result<()>
where
    F: FnOnce(&Path) -> Result<()>,
{
    let record = match &outcome {
        Ok(()) if args.dry_run => result::dry_run(&args.current_version, &args.target_version),
        Ok(()) => result::success(&args.current_version, &args.target_version),
        Err(failure) if failure.rolled_back => {
            result::rolled_back(&args.current_version, &args.target_version, &failure.error)
        }
        Err(failure) => {
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
        && (outcome.is_ok()
            || outcome
                .as_ref()
                .err()
                .is_some_and(|failure| failure.rolled_back));
    if should_restart {
        // The result is durable before the new process starts. This prevents
        // the restarted app from racing its own result consumption.
        if let Err(restart_error) = restart(&args.install_root) {
            eprintln!("could not restart verified application: {restart_error}");
            if outcome.is_ok() {
                // Installation succeeded, but the final operation did not.
                // Keep the new files in place and overwrite the optimistic
                // success record so the next app launch reports reality.
                let restart_record =
                    result::failure(&args.current_version, &args.target_version, &restart_error);
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
    }
    outcome.map_err(|failure| failure.error)
}

fn execute_update(args: &UpdaterArgs) -> std::result::Result<(), ExecutionFailure> {
    let primary_exe = safe_join(&args.install_root, sky_updater::PRIMARY_EXE)?;
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
    sky_updater::signature::verify_file(&updater_path)?;

    let run_root = updater_run_root()?;
    let zip_path = run_root.join("release.zip");
    let staging = run_root.join("staging");
    let execution = (|| -> std::result::Result<(), ExecutionFailure> {
        let payload = fetch_exact_release(
            &WinHttpClient,
            &args.target_version,
            args.channel,
            &zip_path,
        )?;
        inspect_archive(&payload.zip_path)?;
        extract_zip_file(&payload.zip_path, &staging)?;
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
            return Ok(());
        }
        if let Err(error) = install_verified(
            &args.install_root,
            &staging,
            &staged_manifest,
            &old_manifest,
        ) {
            if has_unresolved_transaction(&args.install_root) {
                if let Err(rollback_error) = rollback_prepared(&args.install_root) {
                    return Err(ExecutionFailure {
                        error: UpdaterError::RollbackFailed(format!(
                            "{error}; rollback failed: {rollback_error}"
                        )),
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
        cleanup_committed(&args.install_root)?;
        Ok(())
    })();
    let cleanup = cleanup_run_files(&run_root);
    match execution {
        Ok(()) => {
            cleanup?;
            Ok(())
        }
        Err(error) => {
            if let Err(cleanup_error) = cleanup {
                eprintln!("could not clean updater run directory: {cleanup_error}");
            }
            Err(error)
        }
    }
}

fn updater_run_root() -> Result<PathBuf> {
    let current_exe =
        env::current_exe().map_err(|error| UpdaterError::InstallRootInvalid(error.to_string()))?;
    if current_exe.file_name().and_then(|name| name.to_str()) != Some(sky_updater::UPDATER_EXE) {
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
        .join(sky_updater::APP_NAME)
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

fn cleanup_run_files(run_root: &Path) -> Result<()> {
    for name in ["release.zip", "staging"] {
        let path = run_root.join(name);
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else if path.is_file() {
            fs::remove_file(path)?;
        }
    }
    Ok(())
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
    use std::time::{SystemTime, UNIX_EPOCH};

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
}
