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
            other => {
                standard.push(other.to_owned());
            }
        }
    }
    let release_dir = release_dir.ok_or_else(|| {
        UpdaterError::InvalidArgument("missing required flag: --release-dir".into())
    })?;
    sky_updater::faults::configure(fail_at.as_deref(), pause_at.as_deref())?;
    let source = LocalReleaseSource::new(&release_dir)?;
    let args = match sky_updater::cli::parse(standard.into_iter())? {
        sky_updater::cli::ParseResult::Args(args) => args,
        sky_updater::cli::ParseResult::Help | sky_updater::cli::ParseResult::Version => {
            return Err(UpdaterError::InvalidArgument(
                "E2E updater requires update arguments".into(),
            ));
        }
    };
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
