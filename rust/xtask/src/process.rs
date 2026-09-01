use crate::Result;
use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_CAPTURE_BYTES: usize = 64 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const READER_DRAIN_TIMEOUT: Duration = Duration::from_millis(500);

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

pub fn run_owned_timeout(
    program: &Path,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    timeout: Duration,
) -> Result<()> {
    let output = capture_owned_timeout(program.as_os_str(), args, cwd, env, timeout)?;
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

struct BoundedReader {
    bytes: Arc<Mutex<Vec<u8>>>,
    finished: mpsc::Receiver<()>,
    handle: JoinHandle<()>,
}

fn spawn_bounded_reader<R>(mut reader: R) -> BoundedReader
where
    R: Read + Send + 'static,
{
    let bytes = Arc::new(Mutex::new(Vec::with_capacity(MAX_CAPTURE_BYTES.min(8192))));
    let captured = Arc::clone(&bytes);
    let (finished_sender, finished) = mpsc::channel();
    let handle = thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if let Ok(mut captured) = captured.lock() {
                        let remaining = MAX_CAPTURE_BYTES.saturating_sub(captured.len());
                        if remaining > 0 {
                            captured.extend_from_slice(&buffer[..count.min(remaining)]);
                        }
                    }
                }
            }
        }
        let _ = finished_sender.send(());
    });
    BoundedReader {
        bytes,
        finished,
        handle,
    }
}

fn collect_bounded_reader(reader: BoundedReader, stream: &str) -> Result<Vec<u8>> {
    if reader.finished.recv_timeout(READER_DRAIN_TIMEOUT).is_err() {
        return Ok(reader
            .bytes
            .lock()
            .map(|bytes| bytes.clone())
            .unwrap_or_default());
    }
    if reader.handle.join().is_err() {
        return Err(format!("{stream} reader thread panicked").into());
    }
    reader
        .bytes
        .lock()
        .map(|bytes| bytes.clone())
        .map_err(|_| format!("{stream} reader lock poisoned").into())
}

fn terminate_and_reap(child: &mut Child) -> Result<std::process::ExitStatus> {
    let _ = child.kill();
    child
        .wait()
        .map_err(|error| format!("failed to reap timed-out child: {error}").into())
}

pub fn capture_owned_timeout(
    program: &OsStr,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    timeout: Duration,
) -> Result<Output> {
    let rendered = args.join(" ");
    eprintln!(
        "[xtask] {} {rendered} (timeout={}ms)",
        program.to_string_lossy(),
        timeout.as_millis()
    );
    let mut command = Command::new(program);
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
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = terminate_and_reap(&mut child);
            return Err("timed child has no stdout pipe".into());
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            let _ = terminate_and_reap(&mut child);
            return Err("timed child has no stderr pipe".into());
        }
    };
    let stdout_reader = spawn_bounded_reader(stdout);
    let stderr_reader = spawn_bounded_reader(stderr);
    let started = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= timeout {
            timed_out = true;
            break terminate_and_reap(&mut child)?;
        }
        thread::sleep(POLL_INTERVAL.min(timeout.saturating_sub(started.elapsed())));
    };
    let stdout = collect_bounded_reader(stdout_reader, "stdout")?;
    let stderr = collect_bounded_reader(stderr_reader, "stderr")?;
    if timed_out {
        return Err(format!(
            "process {} timed out after {}ms\nstdout: {}\nstderr: {}",
            program.to_string_lossy(),
            timeout.as_millis(),
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        )
        .into());
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
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
        assert!(started.elapsed() < Duration::from_secs(2));
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
}
