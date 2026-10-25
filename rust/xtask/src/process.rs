use crate::Result;
use std::ffi::OsStr;
#[cfg(windows)]
use std::fs::OpenOptions;
#[cfg(any(test, windows))]
use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
#[cfg(any(test, windows))]
use std::sync::mpsc;
#[cfg(any(test, windows))]
use std::thread::{self, JoinHandle};
#[cfg(any(test, windows))]
use std::time::Duration;
#[cfg(any(test, windows))]
use std::time::Instant;

#[cfg(any(test, windows))]
const MAX_CAPTURE_BYTES: usize = 64 * 1024;
#[cfg(any(test, windows))]
const POLL_INTERVAL: Duration = Duration::from_millis(10);
#[cfg(any(test, windows))]
const READER_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);
#[cfg(any(test, windows))]
const PROCESS_TEARDOWN_TIMEOUT: Duration = Duration::from_secs(2);

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

#[cfg(any(test, windows))]
struct BoundedReader {
    finished: mpsc::Receiver<Vec<u8>>,
    handle: JoinHandle<()>,
}

#[cfg(any(test, windows))]
fn spawn_bounded_reader<R>(mut reader: R) -> BoundedReader
where
    R: Read + Send + 'static,
{
    let (finished_sender, finished) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut captured = Vec::with_capacity(MAX_CAPTURE_BYTES.min(8192));
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    let remaining = MAX_CAPTURE_BYTES.saturating_sub(captured.len());
                    if remaining > 0 {
                        captured.extend_from_slice(&buffer[..count.min(remaining)]);
                    }
                }
            }
        }
        let _ = finished_sender.send(captured);
    });
    BoundedReader { finished, handle }
}

#[cfg(any(test, windows))]
fn collect_bounded_reader(
    reader: &mut Option<BoundedReader>,
    timeout: Duration,
    stream: &str,
) -> Result<Option<Vec<u8>>> {
    let Some(active) = reader.as_ref() else {
        return Ok(None);
    };
    match active.finished.recv_timeout(timeout) {
        Ok(bytes) => {
            let active = reader.take().expect("reader was observed above");
            if active.handle.join().is_err() {
                return Err(format!("{stream} reader thread panicked").into());
            }
            Ok(Some(bytes))
        }
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(format!("{stream} pipe reader ended without a result").into())
        }
    }
}

#[cfg(windows)]
pub(crate) struct OwnedChild {
    child: std::process::Child,
    tree: sky_windows_security::ProcessTree,
}

#[cfg(windows)]
impl OwnedChild {
    fn attach(mut child: std::process::Child) -> Result<Self> {
        match sky_windows_security::ProcessTree::attach(&child) {
            Ok(tree) => Ok(Self { child, tree }),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(format!("could not own spawned Windows process tree: {error}").into())
            }
        }
    }

    pub(crate) fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        self.child
            .try_wait()
            .map_err(|error| format!("failed to inspect child process: {error}").into())
    }

    pub(crate) fn wait_timeout(&mut self, timeout: Duration) -> Result<std::process::ExitStatus> {
        let started = Instant::now();
        loop {
            if let Some(status) = self.try_wait()? {
                return Ok(status);
            }
            if started.elapsed() >= timeout {
                let teardown = self.terminate();
                return Err(format!(
                    "process {} timed out after {}ms{}",
                    self.child.id(),
                    timeout.as_millis(),
                    teardown
                        .err()
                        .map(|error| format!("; teardown error: {error}"))
                        .unwrap_or_default()
                )
                .into());
            }
            thread::sleep(POLL_INTERVAL.min(timeout.saturating_sub(started.elapsed())));
        }
    }

    pub(crate) fn terminate(&mut self) -> Result<()> {
        let tree_result = self.tree.terminate();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let deadline = Instant::now() + PROCESS_TEARDOWN_TIMEOUT;
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return tree_result.map_err(|error| {
                    format!("Windows process-tree termination request failed: {error}").into()
                });
            }
            thread::sleep(POLL_INTERVAL);
        }
        Err("Windows child process did not exit within the teardown deadline".into())
    }
}

#[cfg(windows)]
fn spawn_owned(
    program: &OsStr,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    remove_env: &[String],
    stdout: Stdio,
    stderr: Stdio,
) -> Result<OwnedChild> {
    let mut command = Command::new(program);
    for name in remove_env {
        command.env_remove(name);
    }
    command
        .args(args)
        .current_dir(cwd)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    let child = command
        .spawn()
        .map_err(|error| format!("failed to execute {}: {error}", program.to_string_lossy()))?;
    OwnedChild::attach(child)
}

#[cfg(windows)]
pub(crate) fn spawn_owned_silent_filtered(
    program: &OsStr,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    remove_env: &[String],
) -> Result<OwnedChild> {
    eprintln!("[xtask] {} {}", program.to_string_lossy(), args.join(" "));
    spawn_owned(
        program,
        args,
        cwd,
        env,
        remove_env,
        Stdio::null(),
        Stdio::null(),
    )
}

#[cfg(windows)]
fn capture_windows_timeout(
    program: &OsStr,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    remove_env: &[String],
    timeout: Duration,
) -> Result<Output> {
    eprintln!(
        "[xtask] {} {} (timeout={}ms)",
        program.to_string_lossy(),
        args.join(" "),
        timeout.as_millis()
    );
    let mut child = spawn_owned(
        program,
        args,
        cwd,
        env,
        remove_env,
        Stdio::piped(),
        Stdio::piped(),
    )?;
    let mut stdout_reader = Some(spawn_bounded_reader(
        child
            .child
            .stdout
            .take()
            .ok_or("timed child has no stdout pipe")?,
    ));
    let mut stderr_reader = Some(spawn_bounded_reader(
        child
            .child
            .stderr
            .take()
            .ok_or("timed child has no stderr pipe")?,
    ));
    let status = match child.wait_timeout(timeout) {
        Ok(status) => status,
        Err(error) => {
            let _ = child.terminate();
            let stdout =
                collect_bounded_reader(&mut stdout_reader, PROCESS_TEARDOWN_TIMEOUT, "stdout")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
            let stderr =
                collect_bounded_reader(&mut stderr_reader, PROCESS_TEARDOWN_TIMEOUT, "stderr")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
            return Err(format!(
                "{error}\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            )
            .into());
        }
    };

    let stdout = collect_bounded_reader(&mut stdout_reader, READER_DRAIN_TIMEOUT, "stdout")?;
    let stderr = collect_bounded_reader(&mut stderr_reader, READER_DRAIN_TIMEOUT, "stderr")?;
    if stdout.is_none() || stderr.is_none() {
        // The parent exited but a descendant may still own an inherited pipe.
        // Terminate the entire job before waiting again, keeping both reader
        // threads bounded even on the successful-parent/stranded-pipe path.
        let teardown = child.terminate();
        let _stdout =
            collect_bounded_reader(&mut stdout_reader, PROCESS_TEARDOWN_TIMEOUT, "stdout")?;
        let _stderr =
            collect_bounded_reader(&mut stderr_reader, PROCESS_TEARDOWN_TIMEOUT, "stderr")?;
        return Err(format!(
            "process {} exited while a descendant kept a captured pipe open{}",
            program.to_string_lossy(),
            teardown
                .err()
                .map(|error| format!("; process-tree teardown failed: {error}"))
                .unwrap_or_default()
        )
        .into());
    }
    Ok(Output {
        status,
        stdout: stdout.expect("stdout checked above"),
        stderr: stderr.expect("stderr checked above"),
    })
}

#[cfg(all(test, not(windows)))]
fn capture_non_windows_test_timeout(
    program: &OsStr,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    remove_env: &[String],
    timeout: Duration,
) -> Result<Output> {
    let mut command = Command::new(program);
    for name in remove_env {
        command.env_remove(name);
    }
    command
        .args(args)
        .current_dir(cwd)
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to execute {}: {error}", program.to_string_lossy()))?;
    let mut stdout_reader = Some(spawn_bounded_reader(
        child.stdout.take().ok_or("missing stdout pipe")?,
    ));
    let mut stderr_reader = Some(spawn_bounded_reader(
        child.stderr.take().ok_or("missing stderr pipe")?,
    ));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            child.wait()?;
            let stdout =
                collect_bounded_reader(&mut stdout_reader, PROCESS_TEARDOWN_TIMEOUT, "stdout")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
            let stderr =
                collect_bounded_reader(&mut stderr_reader, PROCESS_TEARDOWN_TIMEOUT, "stderr")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
            return Err(format!(
                "process {} timed out after {}ms\nstdout: {}\nstderr: {}",
                program.to_string_lossy(),
                timeout.as_millis(),
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            )
            .into());
        }
        thread::sleep(POLL_INTERVAL);
    };
    let stdout = collect_bounded_reader(&mut stdout_reader, READER_DRAIN_TIMEOUT, "stdout")?;
    let stderr = collect_bounded_reader(&mut stderr_reader, READER_DRAIN_TIMEOUT, "stderr")?;
    if stdout.is_none() || stderr.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        let _ = collect_bounded_reader(&mut stdout_reader, PROCESS_TEARDOWN_TIMEOUT, "stdout");
        let _ = collect_bounded_reader(&mut stderr_reader, PROCESS_TEARDOWN_TIMEOUT, "stderr");
        return Err("process exited while a descendant kept a captured pipe open".into());
    }
    Ok(Output {
        status,
        stdout: stdout.expect("stdout checked above"),
        stderr: stderr.expect("stderr checked above"),
    })
}

#[cfg(test)]
fn capture_owned_timeout(
    program: &OsStr,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    timeout: Duration,
) -> Result<Output> {
    capture_owned_timeout_filtered(program, args, cwd, env, &[], timeout)
}

#[cfg(any(windows, test))]
pub(crate) fn capture_owned_timeout_filtered(
    program: &OsStr,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    remove_env: &[String],
    timeout: Duration,
) -> Result<Output> {
    #[cfg(windows)]
    {
        capture_windows_timeout(program, args, cwd, env, remove_env, timeout)
    }
    #[cfg(all(test, not(windows)))]
    {
        capture_non_windows_test_timeout(program, args, cwd, env, remove_env, timeout)
    }
    #[cfg(all(not(windows), not(test)))]
    {
        let _ = (program, args, cwd, env, remove_env, timeout);
        Err("bounded Windows process-tree execution is unavailable on this platform".into())
    }
}

#[cfg(windows)]
pub(crate) fn capture_owned_timeout_to_file_filtered(
    program: &OsStr,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    remove_env: &[String],
    stdout_path: &Path,
    timeout: Duration,
) -> Result<Output> {
    eprintln!(
        "[xtask] {} {} (timeout={}ms, stdout=file)",
        program.to_string_lossy(),
        args.join(" "),
        timeout.as_millis()
    );
    let stdout_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(stdout_path)?;
    let mut child = spawn_owned(
        program,
        args,
        cwd,
        env,
        remove_env,
        Stdio::from(stdout_file),
        Stdio::piped(),
    )?;
    let mut stderr_reader = Some(spawn_bounded_reader(
        child
            .child
            .stderr
            .take()
            .ok_or("timed child has no stderr pipe")?,
    ));
    let status = match child.wait_timeout(timeout) {
        Ok(status) => status,
        Err(error) => {
            let _ = child.terminate();
            let stderr =
                collect_bounded_reader(&mut stderr_reader, PROCESS_TEARDOWN_TIMEOUT, "stderr")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
            return Err(format!("{error}\nstderr: {}", String::from_utf8_lossy(&stderr)).into());
        }
    };
    let stderr = collect_bounded_reader(&mut stderr_reader, READER_DRAIN_TIMEOUT, "stderr")?;
    let Some(stderr) = stderr else {
        let teardown = child.terminate();
        let _ = collect_bounded_reader(&mut stderr_reader, PROCESS_TEARDOWN_TIMEOUT, "stderr");
        return Err(format!(
            "process {} exited while a descendant kept stderr open{}",
            program.to_string_lossy(),
            teardown
                .err()
                .map(|error| format!("; process-tree teardown failed: {error}"))
                .unwrap_or_default()
        )
        .into());
    };
    Ok(Output {
        status,
        stdout: Vec::new(),
        stderr,
    })
}

#[cfg(not(windows))]
pub(crate) fn capture_owned_timeout_to_file_filtered(
    _program: &OsStr,
    _args: &[String],
    _cwd: &Path,
    _env: &[(String, String)],
    _remove_env: &[String],
    _stdout_path: &Path,
    _timeout: Duration,
) -> Result<Output> {
    Err("bounded Windows process-tree execution is unavailable on this platform".into())
}

#[cfg(windows)]
pub(crate) fn run_owned_timeout_filtered(
    program: &OsStr,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    remove_env: &[String],
    timeout: Duration,
) -> Result<()> {
    let output = capture_owned_timeout_filtered(program, args, cwd, env, remove_env, timeout)?;
    let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    for (name, secret) in env {
        if name.contains("PRIVATE_KEY") && !secret.is_empty() {
            stdout = stdout.replace(secret, "[REDACTED]");
            stderr = stderr.replace(secret, "[REDACTED]");
        }
    }
    if !stdout.is_empty() {
        print!("{stdout}");
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} exited with {}",
            program.to_string_lossy(),
            output.status
        )
        .into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn command(args: &[&str]) -> (PathBuf, Vec<String>) {
        if cfg!(windows) {
            (
                PathBuf::from("cmd.exe"),
                ["/C"]
                    .into_iter()
                    .chain(args.iter().copied())
                    .map(str::to_owned)
                    .collect(),
            )
        } else {
            (
                PathBuf::from("sh"),
                ["-c"]
                    .into_iter()
                    .chain(args.iter().copied())
                    .map(str::to_owned)
                    .collect(),
            )
        }
    }

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

    #[test]
    fn bounded_child_returns_normal_output() {
        let (program, args) = command(if cfg!(windows) {
            &["echo bounded-output"]
        } else {
            &["printf bounded-output"]
        });
        let output = capture_owned_timeout(
            program.as_os_str(),
            &args,
            Path::new("."),
            &[],
            Duration::from_secs(2),
        )
        .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("bounded-output"));
    }

    #[test]
    fn bounded_child_is_killed_and_reaped_at_deadline() {
        let (program, args) = if cfg!(windows) {
            (
                PathBuf::from("ping.exe"),
                ["-n", "4", "127.0.0.1"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            )
        } else {
            command(&["sleep 2"])
        };
        let started = Instant::now();
        let error = capture_owned_timeout(
            program.as_os_str(),
            &args,
            Path::new("."),
            &[],
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert!(error.to_string().contains("timed out after"));
        assert!(started.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn timeout_keeps_bounded_child_diagnostics() {
        let (program, args) = command(if cfg!(windows) {
            &["(echo before-timeout & ping -n 4 127.0.0.1 >NUL)"]
        } else {
            &["printf before-timeout; sleep 2"]
        });
        let error = capture_owned_timeout(
            program.as_os_str(),
            &args,
            Path::new("."),
            &[],
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert!(error.to_string().contains("before-timeout"));
    }

    #[cfg(windows)]
    #[test]
    fn parent_exit_with_descendant_holding_pipes_is_bounded_and_terminates_job() {
        let program = std::env::current_exe().expect("test executable path");
        let args = vec![
            "--exact".to_owned(),
            "process::tests::descendant_pipe_helper".to_owned(),
            "--nocapture".to_owned(),
        ];
        let started = Instant::now();
        let error = capture_owned_timeout(
            program.as_os_str(),
            &args,
            Path::new("."),
            &[(
                "SKY_XTASK_DESCENDANT_PIPE_HELPER".to_owned(),
                "spawn".to_owned(),
            )],
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("descendant kept a captured pipe open")
        );
        assert!(started.elapsed() < Duration::from_secs(8));
    }

    #[test]
    // This helper intentionally exits without waiting so the child keeps the
    // captured pipe handles open; the outer test owns and terminates the job.
    #[allow(clippy::zombie_processes)]
    fn descendant_pipe_helper() {
        if std::env::var_os("SKY_XTASK_DESCENDANT_PIPE_HELPER").is_none() {
            return;
        }
        let executable = std::env::current_exe().expect("test executable path");
        Command::new(executable)
            .args([
                "--exact",
                "process::tests::pipe_holding_descendant",
                "--nocapture",
            ])
            .env("SKY_XTASK_DESCENDANT_PIPE_HELPER", "hold")
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn descendant inheriting both pipe handles");
    }

    #[test]
    fn pipe_holding_descendant() {
        if std::env::var_os("SKY_XTASK_DESCENDANT_PIPE_HELPER").as_deref()
            == Some(OsStr::new("hold"))
        {
            thread::sleep(Duration::from_secs(30));
        }
    }
}
