#![forbid(unsafe_code)]

use crate::Result;
use sky_windows_security::credential::{
    CredentialStore, CredentialTarget, SessionCredentialOps, reject_ambient_password, run_one_shot,
};
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

const ACTION_PASSWORD_ENV: &str = "TAURI_SIGNING_PRIVATE_KEY_PASSWORD";
const CAPTURE_LIMIT: usize = 8 * 1024 * 1024;
const RELEASE_ACTION_SCRIPT: &str = "scripts/orchestrate_v4_production_release.ps1";
const VERIFY_PRIVATE_KEY_ACTION_SCRIPT: &str = "scripts/verify_v4_updater_private_key.ps1";

pub(crate) fn run(root: &Path, args: &[String]) -> Result<()> {
    let action = args.first().ok_or("credential action is required")?;
    let mut test_target = None;
    let mut action_kind = None;
    let mut action_values = Vec::new();
    let mut options = BTreeSet::new();
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--test-target" => {
                if !options.insert("test-target") {
                    return Err("duplicate --test-target option".into());
                }
                index += 1;
                test_target = Some(
                    args.get(index)
                        .ok_or("--test-target requires a value")?
                        .clone(),
                );
            }
            "--action" => {
                if !options.insert("action") {
                    return Err("duplicate --action option".into());
                }
                index += 1;
                action_kind = Some(args.get(index).ok_or("--action requires a value")?.clone());
            }
            option @ ("--key-path" | "--expected-source-sha" | "--version" | "--channel") => {
                if !options.insert(option) {
                    return Err(format!("duplicate {option} option").into());
                }
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| format!("{option} requires a value"))?;
                action_values.push((option.to_owned(), value.clone()));
            }
            option => return Err(format!("unknown credential option: {option}").into()),
        }
        index += 1;
    }

    let target = match test_target {
        Some(target) => CredentialTarget::test_target(target)?,
        None => CredentialTarget::canonical(),
    };
    let store = CredentialStore::new(target);
    match action.as_str() {
        "status" if action_kind.is_none() && action_values.is_empty() => {
            println!(
                "{}",
                if store.has_nonempty()? {
                    "PRESENT_NONEMPTY"
                } else {
                    "ABSENT"
                }
            );
            Ok(())
        }
        "read-check" if action_kind.is_none() && action_values.is_empty() => {
            let secret = store.read()?;
            drop(secret);
            println!("[PASS] V4 updater credential read successfully");
            Ok(())
        }
        "delete" if action_kind.is_none() && action_values.is_empty() => {
            let _ = store.delete()?;
            println!("[PASS] V4 updater session credential deleted");
            Ok(())
        }
        "write-stdin" if action_kind.is_none() && action_values.is_empty() => {
            write_from_stdin(&store)
        }
        "run-action" => run_action(root, &store, action_kind.as_deref(), &action_values),
        "status" | "read-check" | "delete" | "write-stdin" => {
            Err(format!("unexpected option for credential {action}").into())
        }
        _ => Err(format!("unknown credential action: {action}").into()),
    }
}

fn write_from_stdin(store: &CredentialStore) -> Result<()> {
    let mut bytes = Zeroizing::new(Vec::new());
    std::io::stdin().take(2561).read_to_end(&mut bytes)?;
    if bytes.len() > 2560 {
        return Err("credential input exceeds the Windows Credential Manager blob limit".into());
    }
    if bytes.is_empty() {
        return Err("credential input cannot be empty".into());
    }
    let secret = match String::from_utf8(bytes.to_vec()) {
        Ok(secret) => Zeroizing::new(secret),
        Err(error) => {
            let mut invalid = error.into_bytes();
            zeroize::Zeroize::zeroize(&mut invalid);
            return Err("credential input must be valid UTF-8".into());
        }
    };
    store.write_session(&secret)?;
    println!(
        "[PASS] V4 updater session credential provisioned for target: {}",
        store.target().as_str()
    );
    Ok(())
}

fn run_action(
    root: &Path,
    store: &CredentialStore,
    action_kind: Option<&str>,
    values: &[(String, String)],
) -> Result<()> {
    validate_action_environment()?;
    let action_kind = action_kind.ok_or("run-action requires --action")?;
    let mut key_path = None;
    let mut source_sha = None;
    let mut version = None;
    let mut channel = None;
    for (option, value) in values {
        match option.as_str() {
            "--key-path" => key_path = Some(value.as_str()),
            "--expected-source-sha" => source_sha = Some(value.as_str()),
            "--version" => version = Some(value.as_str()),
            "--channel" => channel = Some(value.as_str()),
            _ => unreachable!("validated credential option"),
        }
    }

    let action_script = action_script_relative_path(action_kind)?;
    let mut command = Command::new("pwsh");
    command.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
    ]);
    command.arg(root.join(action_script));
    match action_kind {
        "release" => {
            command
                .arg("-ExpectedSourceSha")
                .arg(source_sha.ok_or("release action requires --expected-source-sha")?)
                .arg("-Version")
                .arg(version.ok_or("release action requires --version")?)
                .arg("-Channel")
                .arg(channel.ok_or("release action requires --channel")?)
                .arg("-UpdaterPrivateKeyPath")
                .arg(key_path.ok_or("release action requires --key-path")?);
        }
        "verify-private-key" => {
            command
                .arg("-KeyPath")
                .arg(key_path.ok_or("verify-private-key action requires --key-path")?);
        }
        _ => unreachable!("action script mapping accepts only supported action kinds"),
    }

    execute_one_shot_action(store, &mut command, root, |output, stderr| {
        write_process_output(output, stderr)
    })
}

pub(crate) fn action_script_relative_path(action_kind: &str) -> Result<&'static str> {
    match action_kind {
        "release" => Ok(RELEASE_ACTION_SCRIPT),
        "verify-private-key" => Ok(VERIFY_PRIVATE_KEY_ACTION_SCRIPT),
        _ => Err("credential run-action accepts only release or verify-private-key".into()),
    }
}

fn validate_action_environment() -> Result<()> {
    validate_action_environment_values(
        std::env::var_os(ACTION_PASSWORD_ENV).as_deref(),
        std::env::var_os("TAURI_SIGNING_PRIVATE_KEY").as_deref(),
        std::env::var_os("TAURI_SIGNING_PRIVATE_KEY_PATH").as_deref(),
    )
}

fn validate_action_environment_values(
    password: Option<&std::ffi::OsStr>,
    private_key: Option<&std::ffi::OsStr>,
    private_key_path: Option<&std::ffi::OsStr>,
) -> Result<()> {
    reject_ambient_password(password)?;
    for variable in [
        ("TAURI_SIGNING_PRIVATE_KEY", private_key),
        ("TAURI_SIGNING_PRIVATE_KEY_PATH", private_key_path),
    ] {
        if variable.1.is_some_and(|value| !value.is_empty()) {
            return Err(format!(
                "ambient {} is forbidden for credential-backed actions",
                variable.0
            )
            .into());
        }
    }
    Ok(())
}

fn execute_one_shot_action(
    store: &impl SessionCredentialOps,
    command: &mut Command,
    root: &Path,
    mut emit: impl FnMut(&[u8], bool) -> std::io::Result<()>,
) -> Result<()> {
    let boundary = run_one_shot(store, |secret| {
        let (status, stdout, stderr) = run_child(command, root, secret)?;
        let stdout = redact_secret_bytes(&stdout, secret);
        let stderr = redact_secret_bytes(&stderr, secret);
        emit(&stdout, false).map_err(|error| error.to_string())?;
        emit(&stderr, true).map_err(|error| error.to_string())?;
        if status == 0 {
            Ok(())
        } else {
            Err(format!("credential action exited with code {status}"))
        }
    });
    boundary.map_err(Into::into)
}

fn run_child(
    command: &mut Command,
    root: &Path,
    secret: &str,
) -> std::result::Result<(i32, Vec<u8>, Vec<u8>), String> {
    command
        .current_dir(root)
        .env(ACTION_PASSWORD_ENV, secret)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("credential action did not start: {error}"))?;
    #[cfg(windows)]
    let process_tree = match sky_windows_security::ProcessTree::attach(&child) {
        Ok(owner) => Some(owner),
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "credential action process-tree ownership failed: {error}"
            ));
        }
    };
    #[cfg(not(windows))]
    let process_tree: Option<sky_windows_security::ProcessTree> = None;

    let stdout = child
        .stdout
        .take()
        .ok_or("credential action stdout was not captured")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("credential action stderr was not captured")?;
    let stdout_reader = capture_pipe(stdout);
    let stderr_reader = capture_pipe(stderr);
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            if let Some(owner) = &process_tree {
                let _ = owner.terminate();
            }
            let _ = child.kill();
            return Err(format!("credential action wait failed: {error}"));
        }
    };
    let pipe_deadline = Instant::now() + Duration::from_secs(3);
    if !wait_for_reader(&stdout_reader, pipe_deadline)
        || !wait_for_reader(&stderr_reader, pipe_deadline)
    {
        if let Some(owner) = &process_tree {
            owner.terminate().map_err(|error| {
                format!("credential action descendant teardown failed: {error}")
            })?;
        }
        let cleanup_deadline = Instant::now() + Duration::from_secs(2);
        if !wait_for_reader(&stdout_reader, cleanup_deadline)
            || !wait_for_reader(&stderr_reader, cleanup_deadline)
        {
            return Err(
                "credential action descendants retained output pipes after bounded teardown".into(),
            );
        }
    }
    let stdout = join_pipe(stdout_reader)?;
    let stderr = join_pipe(stderr_reader)?;
    Ok((status.code().unwrap_or(-1), stdout, stderr))
}

fn capture_pipe(
    mut pipe: impl Read + Send + 'static,
) -> JoinHandle<std::result::Result<Vec<u8>, String>> {
    thread::spawn(move || {
        let mut captured = Vec::new();
        let mut buffer = [0; 8192];
        let mut truncated = false;
        loop {
            let count = pipe.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            let remaining = CAPTURE_LIMIT.saturating_sub(captured.len());
            if count > remaining {
                truncated = true;
            }
            captured.extend_from_slice(&buffer[..count.min(remaining)]);
        }
        if truncated {
            let keep = captured.len().saturating_sub(5120);
            captured.truncate(keep);
            captured.extend_from_slice(b"\n[output truncated]\n");
        }
        Ok(captured)
    })
}

fn wait_for_reader(
    reader: &JoinHandle<std::result::Result<Vec<u8>, String>>,
    deadline: Instant,
) -> bool {
    while !reader.is_finished() {
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
    true
}

fn join_pipe(
    reader: JoinHandle<std::result::Result<Vec<u8>, String>>,
) -> std::result::Result<Vec<u8>, String> {
    reader
        .join()
        .map_err(|_| "credential action output reader panicked".to_owned())?
}

fn write_process_output(output: &[u8], stderr: bool) -> std::io::Result<()> {
    if stderr {
        let mut stream = std::io::stderr().lock();
        stream.write_all(output)?;
        stream.flush()
    } else {
        let mut stream = std::io::stdout().lock();
        stream.write_all(output)?;
        stream.flush()
    }
}

fn redact_secret_bytes(output: &[u8], secret: &str) -> Vec<u8> {
    if secret.is_empty() {
        return output.to_vec();
    }
    let marker = b"[REDACTED]";
    let mut patterns = vec![secret.as_bytes().to_vec()];
    let mut utf16le = Vec::with_capacity(secret.len() * 2);
    for unit in secret.encode_utf16() {
        utf16le.extend_from_slice(&unit.to_le_bytes());
    }
    patterns.push(utf16le);

    // Scan the original output once, longest patterns first. Newly inserted
    // markers are never scanned, even if the secret itself is `[REDACTED]`.
    patterns.sort_by_key(|pattern| std::cmp::Reverse(pattern.len()));
    let mut redacted = Vec::with_capacity(output.len());
    let mut offset = 0;
    while offset < output.len() {
        if let Some(pattern) = patterns.iter().find(|pattern| {
            !pattern.is_empty()
                && output
                    .get(offset..offset + pattern.len())
                    .is_some_and(|candidate| candidate == pattern.as_slice())
        }) {
            redacted.extend_from_slice(marker);
            offset += pattern.len();
        } else {
            redacted.push(output[offset]);
            offset += 1;
        }
    }
    redacted
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    #[cfg(windows)]
    use std::path::PathBuf;
    #[cfg(windows)]
    use std::time::{SystemTime, UNIX_EPOCH};

    const CHILD_MODE_ENV: &str = "SKY_XTASK_SYNTHETIC_ACTION_CHILD_MODE";
    const CHILD_PID_FILE_ENV: &str = "SKY_XTASK_SYNTHETIC_ACTION_CHILD_PID_FILE";

    #[test]
    fn release_action_routes_to_the_canonical_orchestrator() {
        assert_eq!(
            action_script_relative_path("release").expect("release action must be supported"),
            "scripts/orchestrate_v4_production_release.ps1"
        );
        assert_eq!(
            action_script_relative_path("verify-private-key")
                .expect("private-key verification must be supported"),
            "scripts/verify_v4_updater_private_key.ps1"
        );
        assert!(action_script_relative_path("release-preview").is_err());
    }

    struct SyntheticStore {
        secret: Zeroizing<String>,
        deleted: Cell<bool>,
    }

    impl SyntheticStore {
        fn new(secret: &str) -> Self {
            Self {
                secret: Zeroizing::new(secret.to_owned()),
                deleted: Cell::new(false),
            }
        }
    }

    impl SessionCredentialOps for SyntheticStore {
        fn read(&self) -> std::result::Result<Zeroizing<String>, String> {
            Ok(self.secret.clone())
        }

        fn delete(&self) -> std::result::Result<(), String> {
            self.deleted.set(true);
            Ok(())
        }
    }

    fn synthetic_child_command(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().expect("current test executable"));
        command
            .args([
                "--exact",
                "credential_cli::tests::synthetic_child_entry",
                "--nocapture",
            ])
            .env(CHILD_MODE_ENV, mode)
            .env_remove(ACTION_PASSWORD_ENV);
        command
    }

    #[test]
    fn synthetic_child_entry() {
        let Ok(mode) = std::env::var(CHILD_MODE_ENV) else {
            return;
        };
        match mode.as_str() {
            "action-success" | "action-failure" => {
                let secret = std::env::var(ACTION_PASSWORD_ENV)
                    .expect("action password must be supplied only through the child environment");
                println!("synthetic stdout secret: {secret}");
                eprintln!("synthetic stderr secret: {secret}");
                println!("synthetic argv: {:?}", std::env::args().collect::<Vec<_>>());
                if mode == "action-failure" {
                    std::process::exit(37);
                }
            }
            "early-parent-exit" => {
                let pid_file = std::env::var_os(CHILD_PID_FILE_ENV)
                    .expect("descendant test requires a PID file");
                let executable =
                    std::env::current_exe().expect("current test executable must be available");
                let mut descendant = Command::new(executable);
                descendant
                    .args([
                        "--exact",
                        "credential_cli::tests::synthetic_child_entry",
                        "--nocapture",
                    ])
                    .env(CHILD_MODE_ENV, "pipe-descendant")
                    .env(CHILD_PID_FILE_ENV, pid_file)
                    .stdin(Stdio::null())
                    .stdout(Stdio::inherit())
                    .stderr(Stdio::inherit());
                drop(
                    descendant
                        .spawn()
                        .expect("synthetic pipe-holding descendant must start"),
                );
            }
            "pipe-descendant" => {
                let pid_file = std::env::var_os(CHILD_PID_FILE_ENV)
                    .expect("pipe descendant requires a PID file");
                fs::write(pid_file, std::process::id().to_string())
                    .expect("write synthetic descendant PID");
                std::thread::sleep(Duration::from_secs(30));
            }
            other => panic!("unknown synthetic child mode: {other}"),
        }
    }

    fn capture_synthetic_action(mode: &str, secret: &str) -> (Result<()>, Vec<u8>, Vec<u8>, bool) {
        let store = SyntheticStore::new(secret);
        let mut command = synthetic_child_command(mode);
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let result = execute_one_shot_action(&store, &mut command, root, |output, is_stderr| {
            if is_stderr {
                stderr.extend_from_slice(output);
            } else {
                stdout.extend_from_slice(output);
            }
            Ok(())
        });
        (result, stdout, stderr, store.deleted.get())
    }

    #[test]
    fn actual_child_launcher_redacts_environment_secret_and_deletes_after_success_or_failure() {
        let secret = "BENIGN_SYNTHETIC_ACTION_SECRET";
        let (success, stdout, stderr, deleted) = capture_synthetic_action("action-success", secret);
        assert!(success.is_ok());
        assert!(deleted);
        let stdout_text = String::from_utf8(stdout).expect("synthetic child output is UTF-8");
        assert!(stdout_text.contains("synthetic argv:"));
        assert!(stdout_text.contains("[REDACTED]"));
        assert!(!stdout_text.contains(secret));
        assert!(
            !stdout_text
                .split("synthetic argv:")
                .nth(1)
                .is_some_and(|argv| argv.contains(secret)),
            "the action secret must not be present in the child argument vector"
        );
        let stderr_text = String::from_utf8(stderr).expect("synthetic child output is UTF-8");
        assert!(stderr_text.contains("[REDACTED]"));
        assert!(!stderr_text.contains(secret));

        let (failure, stdout, stderr, deleted) = capture_synthetic_action("action-failure", secret);
        let failure = failure.unwrap_err().to_string();
        assert!(
            failure.contains("exited with code 37"),
            "unexpected child failure: {failure}"
        );
        assert!(deleted);
        for output in [stdout, stderr] {
            let output = String::from_utf8(output).expect("synthetic child output is UTF-8");
            assert!(output.contains("[REDACTED]"));
            assert!(!output.contains(secret));
        }
    }

    #[cfg(windows)]
    struct SyntheticDescendantGuard {
        root: PathBuf,
        pid_file: PathBuf,
    }

    #[cfg(windows)]
    impl Drop for SyntheticDescendantGuard {
        fn drop(&mut self) {
            if let Ok(pid) = fs::read_to_string(&self.pid_file)
                && let Ok(pid) = pid.trim().parse::<u32>()
            {
                let _ = Command::new("taskkill.exe")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .output();
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[cfg(windows)]
    fn synthetic_process_is_running(pid: u32) -> bool {
        let Ok(output) = Command::new("tasklist.exe")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
        else {
            return true;
        };
        String::from_utf8_lossy(&output.stdout).lines().any(|line| {
            line.split(',')
                .nth(1)
                .is_some_and(|field| field.trim_matches('"') == pid.to_string())
        })
    }

    #[cfg(windows)]
    #[test]
    fn actual_child_launcher_kills_early_parent_descendant_holding_output_pipes() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sky-xtask-credential-child-{}-{nonce:032x}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create isolated synthetic child directory");
        let pid_file = root.join("descendant.pid");
        let _guard = SyntheticDescendantGuard {
            root: root.clone(),
            pid_file: pid_file.clone(),
        };
        let mut command = synthetic_child_command("early-parent-exit");
        command.env(CHILD_PID_FILE_ENV, &pid_file);
        let started = Instant::now();
        let result = run_child(&mut command, &root, "synthetic-only-action-secret")
            .expect("bounded process-tree teardown should complete");
        assert_eq!(result.0, 0);
        assert!(
            started.elapsed() < Duration::from_secs(7),
            "pipe-holder teardown exceeded its bound"
        );
        let pid: u32 = fs::read_to_string(&pid_file)
            .expect("descendant must have published its process ID")
            .trim()
            .parse()
            .expect("descendant process ID must be numeric");
        let deadline = Instant::now() + Duration::from_secs(2);
        while synthetic_process_is_running(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(
            !synthetic_process_is_running(pid),
            "pipe-holding descendant process {pid} survived process-tree teardown"
        );
    }

    #[test]
    fn action_environment_rejects_ambient_password_and_private_key_material() {
        use std::ffi::OsStr;

        assert!(
            validate_action_environment_values(Some(OsStr::new("ambient sentinel")), None, None)
                .is_err()
        );
        assert!(
            validate_action_environment_values(None, Some(OsStr::new("ambient key")), None)
                .is_err()
        );
        assert!(
            validate_action_environment_values(None, None, Some(OsStr::new("ambient path")))
                .is_err()
        );
        assert!(validate_action_environment_values(None, None, None).is_ok());
    }

    #[test]
    fn redacts_utf8_and_utf16_secret_bytes_before_forwarding_action_output() {
        let secret = "passphrase-中";
        let mut output = b"prefix: ".to_vec();
        output.extend_from_slice(secret.as_bytes());
        output.extend_from_slice(b" :middle: ");
        for unit in secret.encode_utf16() {
            output.extend_from_slice(&unit.to_le_bytes());
        }
        let sanitized = String::from_utf8(redact_secret_bytes(&output, secret)).unwrap();
        assert!(!sanitized.contains(secret));
        assert_eq!(sanitized.matches("[REDACTED]").count(), 2);
    }

    #[test]
    fn redaction_marker_secret_does_not_loop_or_rewrite_inserted_markers() {
        let secret = "[REDACTED]";
        let output = b"before [REDACTED] after";
        assert_eq!(
            redact_secret_bytes(output, secret),
            b"before [REDACTED] after"
        );
    }
}
