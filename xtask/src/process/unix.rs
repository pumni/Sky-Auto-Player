use super::*;
use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process_group};
use std::fs;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Child, ExitStatus};

struct OwnedUnixChild {
    child: Child,
    process_group: Pid,
}

impl OwnedUnixChild {
    fn spawn(
        program: &OsStr,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        remove_env: &[String],
        clear_environment: bool,
        stdout: Stdio,
        stderr: Stdio,
    ) -> Result<Self> {
        let mut command = Command::new(program);
        if clear_environment {
            command.env_clear();
        }
        for name in remove_env {
            command.env_remove(name);
        }
        command
            .args(args)
            .current_dir(cwd)
            .envs(env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .process_group(0);
        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to execute {}: {error}", program.to_string_lossy()))?;
        let process_group = i32::try_from(child.id()).ok().and_then(Pid::from_raw);
        let Some(process_group) = process_group else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("spawned Unix process returned an invalid process-group ID".into());
        };
        Ok(Self {
            child,
            process_group,
        })
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        self.child
            .try_wait()
            .map_err(|error| format!("failed to inspect Unix child process: {error}").into())
    }

    fn terminate(&mut self) -> Result<()> {
        let group_error = match kill_process_group(self.process_group, Signal::KILL) {
            Ok(()) => None,
            Err(Errno::SRCH) => None,
            Err(error) => Some(error),
        };
        if self.try_wait()?.is_none() {
            let _ = self.child.kill();
        }

        let deadline = Instant::now() + PROCESS_TEARDOWN_TIMEOUT;
        while Instant::now() < deadline {
            if self.try_wait()?.is_some() {
                return group_error.map_or(Ok(()), |error| {
                    Err(format!("Unix process-group termination failed: {error}").into())
                });
            }
            thread::sleep(POLL_INTERVAL);
        }
        Err("Unix child did not exit within the process teardown deadline".into())
    }

    fn wait_timeout(&mut self, timeout: Duration) -> Result<ExitStatus> {
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
                        .map(|error| format!("; process-group teardown failed: {error}"))
                        .unwrap_or_default()
                )
                .into());
            }
            thread::sleep(POLL_INTERVAL.min(timeout.saturating_sub(started.elapsed())));
        }
    }
}

impl Drop for OwnedUnixChild {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.terminate();
        }
    }
}

pub(super) fn capture_timeout(
    program: &OsStr,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    remove_env: &[String],
    clear_environment: bool,
    timeout: Duration,
) -> Result<Output> {
    eprintln!(
        "[xtask] {} {} (timeout={}ms)",
        program.to_string_lossy(),
        args.join(" "),
        timeout.as_millis()
    );
    let mut child = OwnedUnixChild::spawn(
        program,
        args,
        cwd,
        env,
        remove_env,
        clear_environment,
        Stdio::piped(),
        Stdio::piped(),
    )?;
    let mut stdout_reader = Some(spawn_bounded_reader(
        child
            .child
            .stdout
            .take()
            .ok_or("timed Unix child has no stdout pipe")?,
    ));
    let mut stderr_reader = Some(spawn_bounded_reader(
        child
            .child
            .stderr
            .take()
            .ok_or("timed Unix child has no stderr pipe")?,
    ));
    let status = match child.wait_timeout(timeout) {
        Ok(status) => status,
        Err(error) => {
            let _ = child.terminate();
            let stdout = drain_after_termination(&mut stdout_reader, "stdout");
            let stderr = drain_after_termination(&mut stderr_reader, "stderr");
            let secrets = diagnostic_secrets(env);
            return Err(format!(
                "{error}\nstdout: {}\nstderr: {}",
                redact_diagnostics(&String::from_utf8_lossy(&stdout), &secrets),
                redact_diagnostics(&String::from_utf8_lossy(&stderr), &secrets)
            )
            .into());
        }
    };
    let stdout = collect_bounded_reader(&mut stdout_reader, READER_DRAIN_TIMEOUT, "stdout")?;
    let stderr = collect_bounded_reader(&mut stderr_reader, READER_DRAIN_TIMEOUT, "stderr")?;
    if stdout.is_none() || stderr.is_none() {
        let teardown = child.terminate();
        let _ = collect_bounded_reader(&mut stdout_reader, PROCESS_TEARDOWN_TIMEOUT, "stdout");
        let _ = collect_bounded_reader(&mut stderr_reader, PROCESS_TEARDOWN_TIMEOUT, "stderr");
        return Err(format!(
            "process {} exited while a descendant kept a captured pipe open{}",
            program.to_string_lossy(),
            teardown
                .err()
                .map(|error| format!("; process-group teardown failed: {error}"))
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

pub(super) fn capture_timeout_to_file(
    program: &OsStr,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    remove_env: &[String],
    clear_environment: bool,
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
    let result = capture_to_file_created(
        program,
        args,
        cwd,
        env,
        remove_env,
        clear_environment,
        stdout_file,
        timeout,
    );
    match result {
        Ok(output) if output.status.success() => Ok(output),
        Ok(output) => {
            remove_partial_file(stdout_path)?;
            Ok(output)
        }
        Err(error) => match remove_partial_file(stdout_path) {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(format!(
                "{error}; failed to remove partial child output: {cleanup_error}"
            )
            .into()),
        },
    }
}

fn capture_to_file_created(
    program: &OsStr,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    remove_env: &[String],
    clear_environment: bool,
    stdout_file: fs::File,
    timeout: Duration,
) -> Result<Output> {
    let mut child = OwnedUnixChild::spawn(
        program,
        args,
        cwd,
        env,
        remove_env,
        clear_environment,
        Stdio::from(stdout_file),
        Stdio::piped(),
    )?;
    let mut stderr_reader = Some(spawn_bounded_reader(
        child
            .child
            .stderr
            .take()
            .ok_or("timed Unix child has no stderr pipe")?,
    ));
    let status = match child.wait_timeout(timeout) {
        Ok(status) => status,
        Err(error) => {
            let _ = child.terminate();
            let stderr = drain_after_termination(&mut stderr_reader, "stderr");
            let secrets = diagnostic_secrets(env);
            return Err(format!(
                "{error}\nstderr: {}",
                redact_diagnostics(&String::from_utf8_lossy(&stderr), &secrets)
            )
            .into());
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
                .map(|error| format!("; process-group teardown failed: {error}"))
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

fn remove_partial_file(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn drain_after_termination(reader: &mut Option<BoundedReader>, stream: &str) -> Vec<u8> {
    collect_bounded_reader(reader, PROCESS_TEARDOWN_TIMEOUT, stream)
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn diagnostic_secrets(explicit_env: &[(String, String)]) -> Vec<String> {
    let mut secrets = env::vars_os()
        .filter_map(|(name, value)| {
            let name = name.to_str()?;
            is_sensitive_name(name)
                .then(|| value.into_string().ok())
                .flatten()
        })
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    secrets.extend(
        explicit_env
            .iter()
            .filter(|(name, _)| is_sensitive_name(name))
            .map(|(_, value)| value)
            .filter(|value| !value.is_empty())
            .cloned(),
    );
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    secrets.dedup();
    secrets
}

fn is_sensitive_name(name: &str) -> bool {
    let uppercase = name.to_ascii_uppercase();
    ["TOKEN", "SECRET", "PASSWORD", "PRIVATE_KEY"]
        .iter()
        .any(|marker| uppercase.contains(marker))
}

fn redact_diagnostics(diagnostics: &str, secrets: &[String]) -> String {
    secrets.iter().fold(diagnostics.to_owned(), |text, secret| {
        text.replace(secret, "[REDACTED]")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn create() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "sky-xtask-unix-process-{}-{nonce:x}",
                std::process::id()
            ));
            fs::create_dir(&root).expect("create process adapter test root");
            Self(root)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn shell(script: &str) -> (OsString, Vec<String>) {
        (
            OsString::from("sh"),
            ["-c", script].into_iter().map(str::to_owned).collect(),
        )
    }

    fn call(script: &str, timeout: Duration) -> Result<Output> {
        let (program, args) = shell(script);
        capture_timeout(&program, &args, Path::new("."), &[], &[], false, timeout)
    }

    #[test]
    fn unix_capture_keeps_only_the_bounded_output_prefix() {
        let output = call(
            "head -c 100000 /dev/zero; head -c 100000 /dev/zero >&2",
            Duration::from_secs(5),
        )
        .expect("bounded Unix process output");
        assert!(output.status.success());
        assert_eq!(output.stdout.len(), MAX_CAPTURE_BYTES);
        assert_eq!(output.stderr.len(), MAX_CAPTURE_BYTES);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn timeout_terminates_same_process_group_descendants() {
        let root = TestRoot::create();
        let pid_file = root.0.join("descendant.pid");
        let script = "sleep 30 & child_pid=$!; printf '%s' \"$child_pid\" > \"$PID_FILE\"; wait \"$child_pid\"";
        let (program, args) = shell(script);
        let error = capture_timeout(
            &program,
            &args,
            Path::new("."),
            &[(
                "PID_FILE".to_owned(),
                pid_file.to_string_lossy().into_owned(),
            )],
            &[],
            false,
            Duration::from_millis(100),
        )
        .expect_err("sleeping descendant must hit its process deadline");
        assert!(error.to_string().contains("timed out after"));
        let pid = fs::read_to_string(pid_file).expect("descendant PID");
        let stat = PathBuf::from(format!("/proc/{}/stat", pid.trim()));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let state = fs::read_to_string(&stat).ok().and_then(|stat| {
                stat.rsplit_once(") ")
                    .and_then(|(_, tail)| tail.chars().next())
            });
            if state.is_none() || state == Some('Z') || state == Some('X') {
                break;
            }
            assert!(Instant::now() < deadline, "descendant is still running");
            thread::sleep(POLL_INTERVAL);
        }
    }

    #[test]
    fn timeout_diagnostics_redact_explicit_token_values() {
        let (program, args) = shell("printf '%s' \"$GH_TOKEN\"; sleep 2");
        let error = capture_timeout(
            &program,
            &args,
            Path::new("."),
            &[("GH_TOKEN".to_owned(), "synthetic-secret-token".to_owned())],
            &[],
            false,
            Duration::from_millis(100),
        )
        .expect_err("sleep must hit its process deadline");
        let rendered = error.to_string();
        assert!(rendered.contains("[REDACTED]"));
        assert!(!rendered.contains("synthetic-secret-token"));
    }

    #[test]
    fn file_capture_preserves_success_bytes_and_removes_failed_output() {
        let root = TestRoot::create();
        let success_path = root.0.join("success.bin");
        let (program, args) = shell("printf '\\000\\001payload'");
        let output = capture_timeout_to_file(
            &program,
            &args,
            Path::new("."),
            &[],
            &[],
            false,
            &success_path,
            Duration::from_secs(2),
        )
        .expect("successful file capture");
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(
            fs::read(&success_path).expect("captured bytes"),
            b"\0\x01payload"
        );

        let failed_path = root.0.join("failed.bin");
        let (program, args) = shell("printf partial; exit 19");
        let output = capture_timeout_to_file(
            &program,
            &args,
            Path::new("."),
            &[],
            &[],
            false,
            &failed_path,
            Duration::from_secs(2),
        )
        .expect("nonzero status remains observable");
        assert_eq!(output.status.code(), Some(19));
        assert!(!failed_path.exists());
    }

    #[test]
    fn file_capture_timeout_cleans_partial_data_and_preserves_existing_destination() {
        let root = TestRoot::create();
        let partial_path = root.0.join("partial.bin");
        let (program, args) = shell("printf partial; sleep 2");
        let error = capture_timeout_to_file(
            &program,
            &args,
            Path::new("."),
            &[],
            &[],
            false,
            &partial_path,
            Duration::from_millis(100),
        )
        .expect_err("sleep must hit the file capture deadline");
        assert!(error.to_string().contains("timed out after"));
        assert!(!partial_path.exists());

        let existing_path = root.0.join("existing.bin");
        fs::write(&existing_path, b"existing-bytes").expect("create protected destination");
        let (program, args) = shell("printf must-not-run");
        assert!(
            capture_timeout_to_file(
                &program,
                &args,
                Path::new("."),
                &[],
                &[],
                false,
                &existing_path,
                Duration::from_secs(2),
            )
            .is_err()
        );
        assert_eq!(
            fs::read(existing_path).expect("preserved destination"),
            b"existing-bytes"
        );
    }
}
