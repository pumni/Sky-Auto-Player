use std::path::{Path, PathBuf};

use crate::error::{Result, UpdaterError};
use crate::version::{Pep440Version, require_upgrade};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdaterArgs {
    pub install_root: PathBuf,
    pub parent_pid: u32,
    pub current_version: String,
    pub target_version: String,
    pub channel: Channel,
    pub restart: bool,
    pub dry_run: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    Stable,
    Beta,
}

impl Channel {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "stable" => Ok(Self::Stable),
            "beta" => Ok(Self::Beta),
            _ => Err(UpdaterError::InvalidArgument(
                "channel must be exactly stable or beta".into(),
            )),
        }
    }
}

pub enum ParseResult {
    Args(UpdaterArgs),
    Help,
    Version,
}

pub fn parse<I>(mut values: I) -> Result<ParseResult>
where
    I: Iterator<Item = String>,
{
    let _program = values.next();
    let mut install_root = None;
    let mut parent_pid = None;
    let mut current_version = None;
    let mut target_version = None;
    let mut channel = None;
    let mut restart = false;
    let mut dry_run = false;
    while let Some(argument) = values.next() {
        match argument.as_str() {
            "--help" | "-h" => return Ok(ParseResult::Help),
            "--version" => return Ok(ParseResult::Version),
            "--restart" => set_once(&mut restart, "--restart")?,
            "--dry-run" => set_once(&mut dry_run, "--dry-run")?,
            "--install-root" => set_value(
                &mut install_root,
                value(&mut values, "--install-root")?,
                "--install-root",
            )?,
            "--parent-pid" => set_value(
                &mut parent_pid,
                parse_pid(&value(&mut values, "--parent-pid")?)?,
                "--parent-pid",
            )?,
            "--current-version" => set_value(
                &mut current_version,
                value(&mut values, "--current-version")?,
                "--current-version",
            )?,
            "--target-version" => set_value(
                &mut target_version,
                value(&mut values, "--target-version")?,
                "--target-version",
            )?,
            "--channel" => set_value(
                &mut channel,
                Channel::parse(&value(&mut values, "--channel")?)?,
                "--channel",
            )?,
            other => {
                return Err(UpdaterError::InvalidArgument(format!(
                    "unknown flag: {other}"
                )));
            }
        }
    }
    let args = UpdaterArgs {
        install_root: PathBuf::from(install_root.ok_or_else(|| missing("--install-root"))?),
        parent_pid: parent_pid.ok_or_else(|| missing("--parent-pid"))?,
        current_version: current_version.ok_or_else(|| missing("--current-version"))?,
        target_version: target_version.ok_or_else(|| missing("--target-version"))?,
        channel: channel.ok_or_else(|| missing("--channel"))?,
        restart,
        dry_run,
    };
    validate(&args)?;
    Ok(ParseResult::Args(args))
}

fn value<I>(values: &mut I, flag: &str) -> Result<String>
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

fn set_once(value: &mut bool, flag: &str) -> Result<()> {
    if *value {
        return Err(UpdaterError::InvalidArgument(format!(
            "duplicate flag: {flag}"
        )));
    }
    *value = true;
    Ok(())
}

fn set_value<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<()> {
    if slot.is_some() {
        return Err(UpdaterError::InvalidArgument(format!(
            "duplicate flag: {flag}"
        )));
    }
    *slot = Some(value);
    Ok(())
}

fn parse_pid(value: &str) -> Result<u32> {
    let pid = value
        .parse::<u32>()
        .map_err(|_| UpdaterError::InvalidArgument("parent PID must be a u32".into()))?;
    if pid == 0 {
        return Err(UpdaterError::InvalidArgument(
            "parent PID must be nonzero".into(),
        ));
    }
    Ok(pid)
}

fn missing(flag: &str) -> UpdaterError {
    UpdaterError::InvalidArgument(format!("missing required flag: {flag}"))
}

pub fn validate(args: &UpdaterArgs) -> Result<()> {
    if !args.install_root.is_absolute() {
        return Err(UpdaterError::InstallRootInvalid(
            "install root must be absolute".into(),
        ));
    }
    if !args.install_root.is_dir() {
        return Err(UpdaterError::InstallRootInvalid(
            "install root must exist and be a directory".into(),
        ));
    }
    if args.parent_pid == 0 {
        return Err(UpdaterError::InvalidArgument(
            "parent PID must be nonzero".into(),
        ));
    }
    let _current = Pep440Version::parse(&args.current_version)?;
    let target = Pep440Version::parse(&args.target_version)?;
    require_upgrade(&args.current_version, &args.target_version)?;
    if args.channel == Channel::Stable && target.is_prerelease() {
        return Err(UpdaterError::ReleasePolicyRejected(
            "stable channel cannot install a prerelease".into(),
        ));
    }
    let primary = args.install_root.join(crate::PRIMARY_EXE);
    if !is_within(&args.install_root, &primary) || !primary.is_file() {
        return Err(UpdaterError::InstallRootInvalid(
            "canonical primary executable is missing from install root".into(),
        ));
    }
    Ok(())
}

fn is_within(root: &Path, path: &Path) -> bool {
    let root = root.components().collect::<Vec<_>>();
    let path = path.components().collect::<Vec<_>>();
    path.len() >= root.len() && path[..root.len()] == root[..]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_or_missing_flags() {
        let result = parse(["updater".into(), "--wat".into()].into_iter());
        assert!(matches!(result, Err(UpdaterError::InvalidArgument(_))));
    }

    #[test]
    fn parses_help_without_runtime_arguments() {
        assert!(matches!(
            parse(["updater".into(), "--help".into()].into_iter()),
            Ok(ParseResult::Help)
        ));
    }

    #[test]
    fn rejects_noncanonical_channel() {
        let result = Channel::parse("STABLE");
        assert!(result.is_err());
    }
}
