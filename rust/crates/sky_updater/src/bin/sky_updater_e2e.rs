//! Disposable local-release E2E executable.
//!
//! This binary is feature-gated in Cargo and is never built by the public
//! release pipeline. It intentionally requires an explicit local release
//! directory and uses the same transaction runner as production.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use sky_updater::error::{Result, UpdaterError};
use sky_updater::local_source::LocalReleaseSource;
use sky_updater::runner::run_update_with_source;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!(
                "Sky Auto Player E2E updater failed code={}: {error}",
                sky_updater::result::error_code(&error)
            );
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<()> {
    let mut standard = vec!["Sky-Auto-Player-Updater.exe".to_owned()];
    let mut release_dir = None;
    let mut fail_at = None;
    let mut pause_at = None;
    let mut resume_file = None;
    let mut fail_restart = false;
    let mut cleanup_only = false;
    let handshake_only =
        env::var("SKY_AUTO_PLAYER_E2E_HANDSHAKE_ONLY").is_ok_and(|value| value == "1");
    let mut values = env::args().skip(1);
    while let Some(argument) = values.next() {
        match argument.as_str() {
            "--release-dir" => {
                if release_dir.is_some() {
                    return Err(UpdaterError::InvalidArgument(
                        "duplicate flag: --release-dir".into(),
                    ));
                }
                release_dir = Some(PathBuf::from(next_value(&mut values, &argument)?));
            }
            "--fail-at" => {
                if fail_at.is_some() {
                    return Err(UpdaterError::InvalidArgument(
                        "duplicate flag: --fail-at".into(),
                    ));
                }
                fail_at = Some(next_value(&mut values, &argument)?);
            }
            "--pause-at" => {
                if pause_at.is_some() {
                    return Err(UpdaterError::InvalidArgument(
                        "duplicate flag: --pause-at".into(),
                    ));
                }
                pause_at = Some(next_value(&mut values, &argument)?);
            }
            "--resume-file" => {
                if resume_file.is_some() {
                    return Err(UpdaterError::InvalidArgument(
                        "duplicate flag: --resume-file".into(),
                    ));
                }
                resume_file = Some(PathBuf::from(next_value(&mut values, &argument)?));
            }
            "--fail-restart" => {
                if fail_restart {
                    return Err(UpdaterError::InvalidArgument(
                        "duplicate flag: --fail-restart".into(),
                    ));
                }
                fail_restart = true;
            }
            "--cleanup-only" => {
                if cleanup_only {
                    return Err(UpdaterError::InvalidArgument(
                        "duplicate flag: --cleanup-only".into(),
                    ));
                }
                cleanup_only = true;
            }
            other => {
                standard.push(other.to_owned());
            }
        }
    }
    let args = match sky_updater::cli::parse(standard.into_iter())? {
        sky_updater::cli::ParseResult::Args(args) => args,
        sky_updater::cli::ParseResult::Help | sky_updater::cli::ParseResult::Version => {
            return Err(UpdaterError::InvalidArgument(
                "E2E updater requires update arguments".into(),
            ));
        }
    };

    if handshake_only {
        let run_root = sky_updater::runner::updater_run_root()?;
        return sky_updater::handoff::write_ready(&run_root, &args.target_version);
    }

    if cleanup_only {
        let report = sky_updater::file_replace::cleanup_stale_artifacts_report(&args.install_root);
        if let Some(failure) = report.failures.into_iter().next() {
            return Err(UpdaterError::Io(failure.error));
        }
        return Ok(());
    }

    let release_dir = release_dir.ok_or_else(|| {
        UpdaterError::InvalidArgument("missing required flag: --release-dir".into())
    })?;
    sky_updater::faults::configure(
        fail_at.as_deref(),
        pause_at.as_deref(),
        resume_file.as_deref(),
    )?;
    sky_updater::faults::set_restart_failure(fail_restart)?;
    let source = LocalReleaseSource::new(&release_dir)?;
    run_update_with_source(&args, &source)
}

fn next_value<I>(values: &mut I, flag: &str) -> Result<String>
where
    I: Iterator<Item = String>,
{
    let value = values
        .next()
        .ok_or_else(|| UpdaterError::InvalidArgument(format!("{flag} requires a value")))?;
    if value.is_empty() || value.starts_with('-') {
        return Err(UpdaterError::InvalidArgument(format!(
            "{flag} requires a non-empty value"
        )));
    }
    Ok(value)
}
