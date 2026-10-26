use super::super::{
    METADATA_WRITE_TOKEN_ENV, Transport,
    recovery::{ArtifactVerifier, GitHubArtifactVerifier},
};
use crate::{Result, process, repo};
use serde_json::json;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const READ_TOKEN: &str = "synthetic-linux-recovery-read-token";
const WRITE_TOKEN: &str = "synthetic-linux-stable-write-token";
const GH_SCRIPT: &str = r#"#!/bin/sh
set -eu
directory=${0%/*}
exec 3>>"$directory/invocations.bin"
record_invocation() {
  invocation_command=$1
  shift
  printf '%s\000%s\000%s\000%s\000%s\000%s\000' "$invocation_command" "${GH_TOKEN-}" "${GITHUB_TOKEN-}" "${V4_METADATA_WRITE_TOKEN-}" "${NPM_TOKEN-}" "$#" >&3
  for invocation_argument do printf '%s\000' "$invocation_argument" >&3; done
}
record_invocation gh "$@"
case " $* " in
  *process-smoke-failure*) printf 'synthetic helper failure\n' >&2; exit 23 ;;
  *process-smoke-timeout*|*process-smoke-file-timeout*)
    case " $* " in *process-smoke-file-timeout*) printf 'partial-file' ;; esac
    /bin/sleep 30 &
    child_pid=$!
    printf '%s\n' "$child_pid" > "$directory/descendant.pid"
    wait "$child_pid"
    ;;
  *process-smoke-file-failure*) printf 'partial-file'; exit 19 ;;
  *process-smoke-output-bound*)
    /usr/bin/head -c 100000 /dev/zero
    /usr/bin/head -c 100000 /dev/zero >&2
    ;;
  *attestation\ verify*) printf '{}\n' ;;
  *assets/424242*) printf '\000\001\177\377fixture' ;;
  *) printf '{}\n' ;;
esac
"#;
const CURL_SCRIPT: &str = r#"#!/bin/sh
set -eu
directory=${0%/*}
exec 3>>"$directory/invocations.bin"
record_invocation() {
  invocation_command=$1
  shift
  printf '%s\000%s\000%s\000%s\000%s\000%s\000' "$invocation_command" "${GH_TOKEN-}" "${GITHUB_TOKEN-}" "${V4_METADATA_WRITE_TOKEN-}" "${NPM_TOKEN-}" "$#" >&3
  for invocation_argument do printf '%s\000' "$invocation_argument" >&3; done
}
record_invocation curl "$@"
printf '{"schema":1,"channel":"stable","synthetic":true}\n200'
"#;

struct SmokeRoot(PathBuf);

impl Drop for SmokeRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Invocation {
    command: String,
    gh_token: String,
    github_token: String,
    metadata_write_token: String,
    npm_token: String,
    args: Vec<String>,
}

pub(super) fn run(args: &[String]) -> Result<()> {
    match args {
        [] => run_parent(),
        [mode, root] if mode == "--child" => run_child(Path::new(root)),
        _ => Err("Linux subprocess smoke accepts no arguments outside its child mode".into()),
    }
}

fn run_parent() -> Result<()> {
    let root = create_smoke_root()?;
    let _cleanup = SmokeRoot(root.clone());
    let fake_bin = root.join("bin");
    fs::create_dir(&fake_bin)?;
    write_executable(&fake_bin.join("gh"), GH_SCRIPT)?;
    write_executable(&fake_bin.join("curl"), CURL_SCRIPT)?;

    let executable = env::current_exe()?;
    let args = [
        "ci".to_owned(),
        "linux-process-smoke".to_owned(),
        "--child".to_owned(),
        root.to_string_lossy().into_owned(),
    ];
    let child_environment = [
        ("PATH".to_owned(), fake_bin.to_string_lossy().into_owned()),
        ("HOME".to_owned(), root.to_string_lossy().into_owned()),
        ("GH_TOKEN".to_owned(), READ_TOKEN.to_owned()),
        (METADATA_WRITE_TOKEN_ENV.to_owned(), WRITE_TOKEN.to_owned()),
        (
            "GITHUB_TOKEN".to_owned(),
            "synthetic-inherited-github-token".to_owned(),
        ),
        (
            "NPM_TOKEN".to_owned(),
            "synthetic-inherited-package-token".to_owned(),
        ),
    ];
    let output = process::capture_owned_timeout_runtime_only(
        executable.as_os_str(),
        &args,
        &repo::root(),
        &child_environment,
        Duration::from_secs(60),
    )?;
    if !output.status.success() {
        let redact = |text: &str| {
            [
                READ_TOKEN,
                WRITE_TOKEN,
                "synthetic-inherited-github-token",
                "synthetic-inherited-package-token",
            ]
            .into_iter()
            .fold(text.to_owned(), |text, secret| {
                text.replace(secret, "[REDACTED]")
            })
        };
        return Err(format!(
            "release-mode Linux subprocess smoke child failed with {}\nstdout: {}\nstderr: {}",
            output.status,
            redact(&String::from_utf8_lossy(&output.stdout)),
            redact(&String::from_utf8_lossy(&output.stderr)),
        )
        .into());
    }
    println!("[xtask] Linux release subprocess smoke: PASS");
    Ok(())
}

fn run_child(root: &Path) -> Result<()> {
    if !env::var("GH_TOKEN").is_ok_and(|token| token == READ_TOKEN)
        || !env::var(METADATA_WRITE_TOKEN_ENV).is_ok_and(|token| token == WRITE_TOKEN)
        || !env::var("GITHUB_TOKEN").is_ok_and(|token| token == "synthetic-inherited-github-token")
        || !env::var("NPM_TOKEN").is_ok_and(|token| token == "synthetic-inherited-package-token")
    {
        return Err("smoke child did not receive its disposable credential fixture".into());
    }
    let fake_bin = root.join("bin");
    if env::var_os("PATH").as_deref() != Some(fake_bin.as_os_str()) {
        return Err("smoke child PATH must contain only the fake CLI directory".into());
    }
    let log = fake_bin.join("invocations.bin");
    let installer = root.join("synthetic-installer.exe");
    let subject = root.join("synthetic-attestation-subject.bin");
    fs::write(&subject, b"synthetic attestation subject")?;

    let stable_metadata = format!(
        "repos/{}/contents/channels/stable/latest.json",
        super::super::REPOSITORY
    );
    let mut transport = super::super::GhTransport::for_metadata_recovery(
        READ_TOKEN.to_owned(),
        Some(WRITE_TOKEN.to_owned()),
    )?;
    let get = transport
        .request("GET", "repos/pumni/Sky-Auto-Player/releases/latest", None)?
        .ok_or("fake GitHub GET returned no JSON")?;
    if get != json!({}) {
        return Err("fake GitHub GET returned an unexpected response".into());
    }
    require_latest_token(&log, "gh", "releases/latest", READ_TOKEN)?;

    let body = json!({"message":"synthetic metadata only","content":"bm9wZQ=="});
    let put = transport
        .request("PUT", &stable_metadata, Some(&body))?
        .ok_or("fake stable metadata PUT returned no JSON")?;
    if put != json!({}) {
        return Err("fake stable metadata PUT returned an unexpected response".into());
    }
    require_latest_token(&log, "gh", "channels/stable/latest.json", WRITE_TOKEN)?;

    let call_count = read_invocations(&log)?.len();
    if transport
        .request(
            "PUT",
            "repos/pumni/Sky-Auto-Player/contents/channels/beta/latest.json",
            None,
        )
        .is_ok()
        || transport
            .request(
                "DELETE",
                "repos/pumni/Sky-Auto-Player/releases/424242",
                None,
            )
            .is_ok()
    {
        return Err("recovery transport accepted an unapproved mutation".into());
    }
    let mut read_only =
        super::super::GhTransport::for_metadata_recovery(READ_TOKEN.to_owned(), None)?;
    if read_only
        .request("PUT", &stable_metadata, Some(&body))
        .is_ok()
        || read_invocations(&log)?.len() != call_count
    {
        return Err("stable metadata write escaped token or endpoint gating".into());
    }

    transport.download(
        "repos/pumni/Sky-Auto-Player/releases/assets/424242",
        &installer,
    )?;
    if fs::read(&installer)? != b"\0\x01\x7f\xfffixture" {
        return Err("fake GitHub asset download changed the exact fixture bytes".into());
    }
    require_latest_token(&log, "gh", "assets/424242", READ_TOKEN)?;

    let mut verifier = GitHubArtifactVerifier::new(READ_TOKEN.to_owned())?;
    verifier.verify_attestation(&subject, &"a".repeat(40), None)?;
    require_latest_token(&log, "gh", "attestation verify", READ_TOKEN)?;

    let raw_endpoint = format!(
        "https://raw.githubusercontent.com/{}/release-metadata/channels/stable/latest.json",
        super::super::REPOSITORY
    );
    let raw = transport
        .raw_get(&raw_endpoint)?
        .ok_or("fake stable raw GET returned no content")?;
    if raw != b"{\"schema\":1,\"channel\":\"stable\",\"synthetic\":true}" {
        return Err("fake curl returned an unexpected raw metadata response".into());
    }
    require_latest_token(&log, "curl", "channels/stable/latest.json", "")?;

    let (gh, fail_args) = fake_gh(&["process-smoke-failure"]);
    let failed = process::capture_owned_timeout_runtime_only(
        &gh,
        &fail_args,
        &repo::root(),
        &[("GH_TOKEN".to_owned(), READ_TOKEN.to_owned())],
        Duration::from_secs(2),
    )?;
    if failed.status.code() != Some(23)
        || !String::from_utf8_lossy(&failed.stderr).contains("synthetic helper failure")
    {
        return Err("nonzero fake gh exit was not preserved".into());
    }

    let (gh, bound_args) = fake_gh(&["process-smoke-output-bound"]);
    let bounded = process::capture_owned_timeout_runtime_only(
        &gh,
        &bound_args,
        &repo::root(),
        &[],
        Duration::from_secs(5),
    )?;
    if !bounded.status.success()
        || bounded.stdout.len() != 64 * 1024
        || bounded.stderr.len() != 64 * 1024
    {
        return Err("Linux child capture did not enforce the 64 KiB per-stream bound".into());
    }

    let (gh, timeout_args) = fake_gh(&["process-smoke-timeout"]);
    let timeout = process::capture_owned_timeout_runtime_only(
        &gh,
        &timeout_args,
        &repo::root(),
        &[("GH_TOKEN".to_owned(), READ_TOKEN.to_owned())],
        Duration::from_millis(150),
    )
    .expect_err("fake gh must hit the bounded timeout");
    if !timeout.to_string().contains("timed out after") {
        return Err("Linux child timeout omitted its bounded failure evidence".into());
    }
    require_descendant_stopped(&fake_bin.join("descendant.pid"))?;

    let partial_timeout = root.join("partial-timeout.bin");
    let (gh, file_timeout_args) = fake_gh(&["process-smoke-file-timeout"]);
    let timeout = process::capture_owned_timeout_to_file_runtime_only(
        &gh,
        &file_timeout_args,
        &repo::root(),
        &[("GH_TOKEN".to_owned(), READ_TOKEN.to_owned())],
        &partial_timeout,
        Duration::from_millis(150),
    )
    .expect_err("fake asset download must time out");
    if !timeout.to_string().contains("timed out after") || partial_timeout.exists() {
        return Err("timed-out file capture did not clean its partial output".into());
    }
    require_descendant_stopped(&fake_bin.join("descendant.pid"))?;

    let partial_failure = root.join("partial-failure.bin");
    let (gh, file_failure_args) = fake_gh(&["process-smoke-file-failure"]);
    let failed = process::capture_owned_timeout_to_file_runtime_only(
        &gh,
        &file_failure_args,
        &repo::root(),
        &[("GH_TOKEN".to_owned(), READ_TOKEN.to_owned())],
        &partial_failure,
        Duration::from_secs(2),
    )?;
    if failed.status.code() != Some(19) || partial_failure.exists() {
        return Err("failed file capture did not preserve status and clean partial output".into());
    }

    let preexisting = root.join("preexisting-output.bin");
    fs::write(&preexisting, b"keep-existing-bytes")?;
    let (gh, success_args) = fake_gh(&["process-smoke-file-failure"]);
    if process::capture_owned_timeout_to_file_runtime_only(
        &gh,
        &success_args,
        &repo::root(),
        &[],
        &preexisting,
        Duration::from_secs(2),
    )
    .is_ok()
        || fs::read(&preexisting)? != b"keep-existing-bytes"
    {
        return Err("file capture overwrote a preexisting destination".into());
    }

    for invocation in read_invocations(&log)? {
        if !invocation.github_token.is_empty()
            || !invocation.metadata_write_token.is_empty()
            || !invocation.npm_token.is_empty()
        {
            return Err("isolated fake CLI received an inherited credential".into());
        }
        if invocation.command == "gh"
            && invocation.gh_token == WRITE_TOKEN
            && (!invocation
                .args
                .windows(2)
                .any(|pair| pair[0] == "--method" && pair[1] == "PUT")
                || !invocation
                    .args
                    .iter()
                    .any(|argument| argument.contains("channels/stable/latest.json")))
        {
            return Err("metadata App token reached a non-stable-write gh child".into());
        }
        if invocation.command == "gh"
            && !invocation.gh_token.is_empty()
            && invocation.gh_token != READ_TOKEN
            && invocation.gh_token != WRITE_TOKEN
        {
            return Err("fake gh received an unselected token".into());
        }
    }
    Ok(())
}

fn fake_gh(args: &[&str]) -> (std::ffi::OsString, Vec<String>) {
    (
        std::ffi::OsString::from("gh"),
        args.iter().map(|arg| (*arg).to_owned()).collect(),
    )
}

fn require_latest_token(
    log: &Path,
    command: &str,
    args_marker: &str,
    expected: &str,
) -> Result<()> {
    let invocations = read_invocations(log)?;
    let invocation = invocations
        .iter()
        .rev()
        .find(|invocation| {
            invocation.command == command
                && invocation
                    .args
                    .iter()
                    .any(|argument| argument.contains(args_marker))
        })
        .ok_or("expected fake CLI invocation was not recorded")?;
    if invocation.gh_token != expected
        || !invocation.github_token.is_empty()
        || !invocation.metadata_write_token.is_empty()
        || !invocation.npm_token.is_empty()
    {
        return Err("fake CLI received a token outside its operation-selected scope".into());
    }
    Ok(())
}

fn read_invocations(path: &Path) -> Result<Vec<Invocation>> {
    let Ok(log) = fs::read(path) else {
        return Ok(Vec::new());
    };
    parse_invocations(&log)
}

fn parse_invocations(log: &[u8]) -> Result<Vec<Invocation>> {
    if log.is_empty() || log.last() != Some(&0) {
        return Err("fake CLI wrote a truncated invocation log".into());
    }
    let fields = log.split(|byte| *byte == 0).collect::<Vec<_>>();
    let field_count = fields.len() - 1;
    let mut cursor = 0;
    let mut invocations = Vec::new();
    while cursor < field_count {
        let header = fields
            .get(cursor..cursor + 6)
            .ok_or("fake CLI wrote a malformed invocation header")?;
        let decode =
            |field: &[u8]| -> Result<String> { Ok(std::str::from_utf8(field)?.to_owned()) };
        let command = decode(header[0])?;
        if command != "gh" && command != "curl" {
            return Err("fake CLI wrote an unknown invocation command".into());
        }
        let argument_count = decode(header[5])?
            .parse::<usize>()
            .map_err(|_| "fake CLI wrote an invalid invocation argument count")?;
        if argument_count > 256 {
            return Err("fake CLI invocation argument count exceeds the smoke bound".into());
        }
        cursor += 6;
        let args_end = cursor
            .checked_add(argument_count)
            .filter(|end| *end <= field_count)
            .ok_or("fake CLI wrote a truncated invocation argument list")?;
        let args = fields[cursor..args_end]
            .iter()
            .map(|argument| decode(argument))
            .collect::<Result<Vec<_>>>()?;
        invocations.push(Invocation {
            command,
            gh_token: decode(header[1])?,
            github_token: decode(header[2])?,
            metadata_write_token: decode(header[3])?,
            npm_token: decode(header[4])?,
            args,
        });
        cursor = args_end;
    }
    Ok(invocations)
}

fn require_descendant_stopped(pid_file: &Path) -> Result<()> {
    let pid = fs::read_to_string(pid_file)?;
    let pid = pid.trim();
    if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("fake descendant did not record a numeric process ID".into());
    }
    let stat = PathBuf::from(format!("/proc/{pid}/stat"));
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let state = fs::read_to_string(&stat).ok().and_then(|stat| {
            stat.rsplit_once(") ")
                .and_then(|(_, tail)| tail.chars().next())
        });
        if state.is_none() || state == Some('Z') || state == Some('X') {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("timed-out fake process descendant is still active".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn create_smoke_root() -> Result<PathBuf> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before Unix epoch")?
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "sky-xtask-linux-process-smoke-{}-{nonce:x}",
        std::process::id()
    ));
    fs::create_dir(&root)?;
    Ok(root)
}

fn write_executable(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents)?;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_fields(fields: &[&[u8]]) -> Vec<u8> {
        let mut encoded = Vec::new();
        for field in fields {
            encoded.extend_from_slice(field);
            encoded.push(0);
        }
        encoded
    }

    #[test]
    fn invocation_log_preserves_newline_tab_and_argument_boundaries() {
        let fixture = encode_fields(&[
            b"curl",
            b"read-token",
            b"",
            b"",
            b"",
            b"4",
            b"--write-out",
            b"\n%{http_code}\tstatus",
            b"",
            b"https://raw.example/metadata",
        ]);
        let parsed = parse_invocations(&fixture).expect("control characters are data, not rows");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].command, "curl");
        assert_eq!(parsed[0].gh_token, "read-token");
        assert_eq!(
            parsed[0].args,
            [
                "--write-out",
                "\n%{http_code}\tstatus",
                "",
                "https://raw.example/metadata",
            ]
        );
    }

    #[test]
    fn malformed_invocation_logs_fail_closed() {
        let truncated_header = encode_fields(&[b"gh", b"token"]);
        let truncated_arguments =
            encode_fields(&[b"gh", b"token", b"", b"", b"", b"2", b"--method"]);
        let unknown_command = encode_fields(&[b"other", b"", b"", b"", b"", b"0"]);
        let invalid_utf8 = encode_fields(&[b"gh", b"", b"", b"", b"", b"1", b"\xff"]);
        let mut unterminated = encode_fields(&[b"curl", b"", b"", b"", b"", b"0"]);
        unterminated.pop();

        for fixture in [
            truncated_header,
            truncated_arguments,
            unknown_command,
            invalid_utf8,
            unterminated,
        ] {
            assert!(parse_invocations(&fixture).is_err());
        }
    }
}
