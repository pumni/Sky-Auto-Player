use crate::Result;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output, Stdio};

pub fn run(program: &str, args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> Result<()> {
    let output = capture(program, args, cwd, env)?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed with {}\nstdout: {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

pub fn capture(program: &str, args: &[&str], cwd: &Path, env: &[(&str, &str)]) -> Result<Output> {
    eprintln!("[xtask] {} {}", program, args.join(" "));
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
        .output()
        .map_err(|error| format!("failed to execute {program}: {error}").into())
}

pub fn capture_text(
    program: &str,
    args: &[&str],
    cwd: &Path,
    env: &[(&str, &str)],
) -> Result<String> {
    let output = capture(program, args, cwd, env)?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

pub fn run_owned(
    program: &Path,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
) -> Result<()> {
    let output = capture_owned(program.as_os_str(), args, cwd, env)?;
    if !output.status.success() {
        return Err(format!(
            "{} failed with {}\nstdout: {}\nstderr: {}",
            program.display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(())
}

pub fn capture_owned(
    program: &OsStr,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
) -> Result<Output> {
    let rendered = args.join(" ");
    eprintln!("[xtask] {} {rendered}", program.to_string_lossy());
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
        .output()
        .map_err(|error| format!("failed to execute {}: {error}", program.to_string_lossy()).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_program_is_a_contextual_error() {
        let result = capture_text(
            "xtask-program-that-does-not-exist",
            &[],
            Path::new("."),
            &[],
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("failed to execute")
        );
    }
}
