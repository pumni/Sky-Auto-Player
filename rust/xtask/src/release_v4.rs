#![forbid(unsafe_code)]

use crate::{Result, credential_cli, process, release_metadata, repo, version};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

pub const REPOSITORY: &str = "pumni/Sky-Auto-Player";
const BOOTSTRAP_PATH: &str = ".release-metadata/README.md";
const BOOTSTRAP_CONTENT: &str = "# Sky Auto Player v4 release metadata\n\nbootstrap_contract: sky-auto-player-v4-release-metadata-v1\nThis orphan branch contains deployment-state metadata only.\nThe channel latest.json files are created only by qualified immutable release promotion.";
const EVIDENCE_NAMES: [&str; 6] = [
    "V4_PRODUCTION_RELEASE_EVIDENCE.json",
    "V4_QUALIFICATION_EVIDENCE.json",
    "TAURI_AUTHENTICODE_EVIDENCE.json",
    "INSTALLED_AUTHENTICODE_EVIDENCE.json",
    "TAURI_ARTIFACT_SUMMARY.json",
    "SBOM.spdx.json",
];
const RELEASE_BUILD_STEP_TIMEOUT: Duration = Duration::from_secs(90 * 60);
const RELEASE_DEFENDER_STEP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const ATTESTATION_VERIFY_TIMEOUT: Duration = Duration::from_secs(5 * 60);

fn decode_api_response(method: &str, endpoint: &str, bytes: &[u8]) -> Result<Option<Value>> {
    if bytes.is_empty() {
        Ok(None)
    } else {
        Ok(Some(serde_json::from_slice(bytes).map_err(|error| {
            format!("GitHub API returned malformed JSON for {method} {endpoint}: {error}")
        })?))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum Phase {
    Ready,
    Qualified,
    PublishedPendingMetadata,
    Complete,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Context {
    schema_version: u32,
    repository: String,
    version: String,
    channel: String,
    tag: String,
    release_notes_path: String,
    source_sha: String,
    workflow_sha: String,
    run_id: String,
    created_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CandidateRecord {
    name: String,
    release_name: String,
    source_name: String,
    role: String,
    size: u64,
    sha256: String,
    source_path: String,
    state_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CandidateManifest {
    schema_version: u32,
    source_sha: String,
    version: String,
    channel: String,
    tag: String,
    qualification_assets: Vec<CandidateRecord>,
    public_assets: Vec<CandidateRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseState {
    schema_version: u32,
    repository: String,
    version: String,
    channel: String,
    tag: String,
    source_sha: String,
    workflow_sha: String,
    run_id: String,
    phase: Phase,
    attested: bool,
    qualified_after_download: bool,
    release_id: Option<u64>,
    published_at: Option<String>,
    promoted_at: Option<String>,
    final_verified: bool,
}

#[derive(Clone, Debug)]
struct Options {
    state_root: PathBuf,
    key_path: Option<PathBuf>,
    workflow_sha: Option<String>,
}

#[derive(Debug)]
struct HttpFailure {
    status: Option<u16>,
    detail: String,
}

impl std::fmt::Display for HttpFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GitHub request failed")?;
        if let Some(status) = self.status {
            write!(f, " (HTTP {status})")?;
        }
        write!(f, ": {}", self.detail)
    }
}

impl std::error::Error for HttpFailure {}

trait Transport {
    fn request(
        &mut self,
        method: &str,
        endpoint: &str,
        body: Option<&Value>,
    ) -> Result<Option<Value>>;
    fn upload(&mut self, endpoint: &str, name: &str, path: &Path) -> Result<()>;
    fn download(&mut self, endpoint: &str, destination: &Path) -> Result<()>;
    fn raw_get(&mut self, endpoint: &str) -> Result<Option<Vec<u8>>>;
}

struct GhTransport;

fn canonical_upload_url(endpoint: &str) -> Result<url::Url> {
    let endpoint = endpoint.strip_suffix("{?name,label}").unwrap_or(endpoint);
    let raw_authority = endpoint
        .split_once("://")
        .map(|(_, rest)| rest.split('/').next().unwrap_or_default())
        .unwrap_or_default();
    if raw_authority != "uploads.github.com" {
        return Err(
            "refusing release asset upload URL outside the canonical GitHub uploads host".into(),
        );
    }
    let url = url::Url::parse(endpoint)?;
    let repository_parts = REPOSITORY.split('/').collect::<Vec<_>>();
    let segments = url
        .path_segments()
        .ok_or("release upload URL has no path segments")?
        .collect::<Vec<_>>();
    let canonical_path = segments.len() == 6
        && segments[0] == "repos"
        && segments[1] == repository_parts[0]
        && segments[2] == repository_parts[1]
        && segments[3] == "releases"
        && segments[4]
            .parse::<u64>()
            .is_ok_and(|id| id > 0 && id.to_string() == segments[4])
        && segments[5] == "assets";
    if url.scheme() != "https"
        || url.host_str() != Some("uploads.github.com")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !canonical_path
    {
        return Err(
            "refusing release asset upload URL outside the canonical numeric GitHub release endpoint"
                .into(),
        );
    }
    Ok(url)
}

impl GhTransport {
    fn new() -> Self {
        Self
    }

    fn invoke(&self, args: &[String], payload: Option<&[u8]>) -> Result<Vec<u8>> {
        self.invoke_with_timeout(args, payload, Duration::from_secs(60))
    }

    fn invoke_with_timeout(
        &self,
        args: &[String],
        payload: Option<&[u8]>,
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        let input_path = if let Some(input) = payload {
            let path = unique_temp(&env::temp_dir(), "sky-release-gh-request");
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            use std::io::Write;
            file.write_all(input)?;
            Some(path)
        } else {
            None
        };
        let mut args = args.to_vec();
        if let (Some(path), Some(input_arg)) = (
            input_path.as_ref(),
            args.iter_mut().find(|arg| arg.as_str() == "-"),
        ) {
            *input_arg = path.to_string_lossy().into_owned();
        }
        let output = process::capture_owned_timeout_filtered(
            std::ffi::OsStr::new("gh"),
            &args,
            &repo::root(),
            &[
                ("GH_PAGER".to_owned(), "cat".to_owned()),
                ("PAGER".to_owned(), "cat".to_owned()),
            ],
            &[],
            timeout,
        );
        let cleanup = input_path.map(fs::remove_file).transpose();
        let output = match (output, cleanup) {
            (Ok(output), Ok(_)) => output,
            (Ok(_), Err(error)) => {
                return Err(format!("failed to remove temporary GitHub request: {error}").into());
            }
            (Err(error), Ok(_)) => return Err(error),
            (Err(error), Err(cleanup_error)) => {
                return Err(format!(
                    "GitHub request failed: {error}; temporary request cleanup failed: {cleanup_error}"
                )
                .into());
            }
        };
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let status = extract_http_status(&stderr);
            if status == Some(404) {
                return Err(Box::new(HttpFailure {
                    status,
                    detail: "not found".to_owned(),
                }));
            }
            return Err(Box::new(HttpFailure {
                status,
                detail: safe_cli_error(&stderr),
            }));
        }
        Ok(output.stdout)
    }

    fn invoke_to_file(&self, args: &[String], destination: &Path) -> Result<()> {
        let output = process::capture_owned_timeout_to_file_filtered(
            std::ffi::OsStr::new("gh"),
            args,
            &repo::root(),
            &[
                ("GH_PAGER".to_owned(), "cat".to_owned()),
                ("PAGER".to_owned(), "cat".to_owned()),
            ],
            &[],
            destination,
            Duration::from_secs(10 * 60),
        )?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Box::new(HttpFailure {
                status: extract_http_status(&stderr),
                detail: safe_cli_error(&stderr),
            }));
        }
        Ok(())
    }

    fn api_once(
        &self,
        method: &str,
        endpoint: &str,
        body: Option<&Value>,
    ) -> Result<Option<Value>> {
        let mut args = vec![
            "api".to_owned(),
            "--hostname".to_owned(),
            "github.com".to_owned(),
        ];
        if method != "GET" {
            args.push("--method".to_owned());
            args.push(method.to_owned());
        }
        args.push(endpoint.to_owned());
        if body.is_some() {
            args.extend(["--input".to_owned(), "-".to_owned()]);
        }
        let payload = body.map(serde_json::to_vec).transpose()?;
        match self.invoke(&args, payload.as_deref()) {
            Ok(bytes) => decode_api_response(method, endpoint, &bytes),
            Err(error)
                if error
                    .downcast_ref::<HttpFailure>()
                    .is_some_and(|failure| failure.status == Some(404)) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
}

impl Transport for GhTransport {
    fn request(
        &mut self,
        method: &str,
        endpoint: &str,
        body: Option<&Value>,
    ) -> Result<Option<Value>> {
        retry_github(|| self.api_once(method, endpoint, body), std::thread::sleep)
    }

    fn upload(&mut self, endpoint: &str, name: &str, path: &Path) -> Result<()> {
        if !path.is_file() {
            return Err("release upload source is not a regular file".into());
        }
        let mut url = canonical_upload_url(endpoint)?;
        url.query_pairs_mut().append_pair("name", name);
        let path_query = format!("{}?{}", url.path(), url.query().unwrap_or_default());
        let args = vec![
            "api".to_owned(),
            "--hostname".to_owned(),
            url.host_str().ok_or("upload host missing")?.to_owned(),
            "--method".to_owned(),
            "POST".to_owned(),
            path_query,
            "--input".to_owned(),
            "-".to_owned(),
            "--header".to_owned(),
            "Content-Type: application/octet-stream".to_owned(),
        ];
        retry_github(
            || {
                let mut request = args.clone();
                if let Some(input) = request.iter_mut().find(|arg| arg.as_str() == "-") {
                    *input = path.to_string_lossy().into_owned();
                }
                self.invoke_with_timeout(&request, None, Duration::from_secs(10 * 60))
                    .map(|_| ())
            },
            std::thread::sleep,
        )
    }

    fn download(&mut self, endpoint: &str, destination: &Path) -> Result<()> {
        let args = vec![
            "api".to_owned(),
            "--hostname".to_owned(),
            "github.com".to_owned(),
            endpoint.to_owned(),
            "--header".to_owned(),
            "Accept: application/octet-stream".to_owned(),
        ];
        retry_github(
            || {
                if destination.exists() {
                    fs::remove_file(destination)?;
                }
                match self.invoke_to_file(&args, destination) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        let cleanup_error = if destination.exists() {
                            fs::remove_file(destination).err()
                        } else {
                            None
                        };
                        if let Some(cleanup_error) = cleanup_error {
                            Err(format!(
                                "{error}; failed to remove partial download: {cleanup_error}"
                            )
                            .into())
                        } else {
                            Err(error)
                        }
                    }
                }
            },
            std::thread::sleep,
        )
    }

    fn raw_get(&mut self, endpoint: &str) -> Result<Option<Vec<u8>>> {
        let parsed = url::Url::parse(endpoint)?;
        let expected_path = format!("/{}/release-metadata/channels/", REPOSITORY);
        if parsed.scheme() != "https"
            || parsed.host_str() != Some("raw.githubusercontent.com")
            || parsed
                .path()
                .strip_prefix(&expected_path)
                .is_none_or(|tail| !matches!(tail, "stable/latest.json" | "beta/latest.json"))
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.port().is_some()
        {
            return Err("refusing public metadata GET outside the canonical raw endpoint".into());
        }
        let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
        let args = [
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--max-time",
            "10",
            "--max-filesize",
            "65500",
            "--write-out",
            "\n%{http_code}",
            endpoint,
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        let output = process::capture_owned_timeout_filtered(
            std::ffi::OsStr::new(curl),
            &args,
            &repo::root(),
            &[],
            &[],
            Duration::from_secs(15),
        )?;
        let split = output
            .stdout
            .windows(4)
            .rposition(|window| window[0] == b'\n' && window[1..].iter().all(u8::is_ascii_digit))
            .ok_or("public metadata GET omitted the HTTP status")?;
        let status = &output.stdout[split + 1..split + 4];
        if status == b"404" {
            return Ok(None);
        }
        if !output.status.success() || !status.starts_with(b"2") {
            return Err(format!(
                "public metadata GET failed with HTTP {}",
                String::from_utf8_lossy(status)
            )
            .into());
        }
        Ok(Some(output.stdout[..split].to_vec()))
    }
}

fn extract_http_status(stderr: &str) -> Option<u16> {
    let lower = stderr.to_ascii_lowercase();
    let marker = "http ";
    let start = lower.find(marker)? + marker.len();
    lower[start..]
        .chars()
        .take(3)
        .collect::<String>()
        .parse()
        .ok()
}

fn safe_cli_error(stderr: &str) -> String {
    let mut result = stderr.trim().to_owned();
    for secret_name in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(secret) = env::var(secret_name)
            && secret.len() > 4
        {
            result = result.replace(&secret, "[REDACTED]");
        }
    }
    if result.len() > 600 {
        result.truncate(600);
    }
    result
}

const GITHUB_RETRY_ATTEMPTS: usize = 4;

fn retry_github<T>(
    mut operation: impl FnMut() -> Result<T>,
    mut sleep: impl FnMut(Duration),
) -> Result<T> {
    let mut delay = Duration::from_millis(500);
    for attempt in 0..GITHUB_RETRY_ATTEMPTS {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error) => {
                let retryable = error.downcast_ref::<HttpFailure>().is_some_and(|failure| {
                    matches!(failure.status, Some(429 | 500 | 502 | 503 | 504))
                });
                if !retryable || attempt + 1 == GITHUB_RETRY_ATTEMPTS {
                    return Err(error);
                }
                sleep(delay);
                delay = (delay * 2).min(Duration::from_secs(4));
            }
        }
    }
    Err("GitHub retry loop exhausted unexpectedly".into())
}

pub(crate) fn run(root: &Path, args: &[String]) -> Result<()> {
    let command = args.first().ok_or("release-v4 requires a phase")?.as_str();
    if command == "self-test" {
        return self_test();
    }
    if command == "doctor" {
        return doctor(root, &args[1..]);
    }
    let mut state_root = None;
    let mut key_path = None;
    let mut workflow_sha = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--state-root" => {
                state_root = Some(PathBuf::from(next_value(args, &mut index, "--state-root")?))
            }
            "--key-path" => {
                key_path = Some(PathBuf::from(next_value(args, &mut index, "--key-path")?))
            }
            "--workflow-sha" => {
                workflow_sha = Some(next_value(args, &mut index, "--workflow-sha")?)
            }
            option => return Err(format!("unknown release-v4 option: {option}").into()),
        }
        index += 1;
    }
    let state_root = state_root.ok_or("release-v4 requires --state-root")?;
    ensure_state_root(root, &state_root)?;
    let options = Options {
        state_root,
        key_path,
        workflow_sha,
    };
    let mut transport = GhTransport::new();
    match command {
        "preflight" => preflight(root, &options, &mut transport),
        "build-candidate" => build_candidate(root, &options),
        "verify-attestations" => verify_attestations(root, &options),
        "publish-release" => publish_release(root, &options, &mut transport),
        "promote-metadata" => promote_metadata(root, &options, &mut transport),
        "final-verify" => final_verify(root, &options, &mut transport),
        "cleanup" => cleanup_local_state(root, &options),
        "probe-metadata-token" => probe_metadata_token(&mut transport),
        _ => Err(format!("unknown release-v4 phase: {command}").into()),
    }
}

fn next_value(args: &[String], index: &mut usize, option: &str) -> Result<String> {
    *index += 1;
    args.get(*index)
        .cloned()
        .ok_or_else(|| format!("{option} requires a value").into())
}

fn ensure_state_root(root: &Path, state_root: &Path) -> Result<()> {
    let root = fs::canonicalize(root)?;
    fs::create_dir_all(state_root)?;
    let state_root = fs::canonicalize(state_root)?;
    if state_root == root || state_root.starts_with(&root) {
        return Err("release state root must remain outside the checked-out workspace".into());
    }
    let runner_temp = env::var_os("RUNNER_TEMP")
        .ok_or("RUNNER_TEMP is required for release transaction state")?;
    let runner_temp = fs::canonicalize(PathBuf::from(runner_temp))?;
    if state_root == runner_temp || !state_root.starts_with(&runner_temp) {
        return Err(
            "release state root must remain in a dedicated RUNNER_TEMP child directory".into(),
        );
    }
    Ok(())
}

fn identity(root: &Path, workflow_sha: Option<&str>) -> Result<Context> {
    let repository = env::var("GITHUB_REPOSITORY")
        .map_err(|_| "GITHUB_REPOSITORY is required for a production release")?;
    if repository != REPOSITORY {
        return Err("release pipeline refuses a non-canonical GitHub repository".into());
    }
    if env::var("GITHUB_EVENT_NAME").ok().as_deref() != Some("workflow_dispatch") {
        return Err("official v4 release requires workflow_dispatch".into());
    }
    if env::var("GITHUB_REF").ok().as_deref() != Some("refs/heads/main") {
        return Err("official v4 release must run from the canonical main ref".into());
    }
    let source_sha = git_text(root, &["rev-parse", "HEAD"])?.to_ascii_lowercase();
    if source_sha.len() != 40 || !source_sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("checked-out HEAD is not an exact commit SHA".into());
    }
    let worktree = git_text(root, &["status", "--porcelain"])?;
    if !worktree.is_empty() {
        return Err("release source worktree is not clean".into());
    }
    let version_text = repo::project_version(root)?;
    let parsed = version::parse(&version_text)?;
    if parsed.major != 4 {
        return Err("v4 release requires a major version of 4".into());
    }
    let channel = if parsed.pre.is_empty() {
        "stable"
    } else {
        "beta"
    };
    version::validate_channel(channel, &version_text, &parsed)?;
    let tag = format!("v{version_text}");
    let notes_path = format!("docs/releases/{tag}.md");
    validate_release_notes(root, &notes_path, &version_text)?;
    let env_sha = env::var("GITHUB_SHA")
        .map_err(|_| "GITHUB_SHA is required for a production release")?
        .to_ascii_lowercase();
    let workflow_sha = workflow_sha.unwrap_or(&env_sha).to_ascii_lowercase();
    if workflow_sha.len() != 40 || workflow_sha != source_sha || env_sha != source_sha {
        return Err("workflow provenance SHA must equal the exact checked-out source SHA".into());
    }
    let run_id = env::var("GITHUB_RUN_ID")
        .map_err(|_| "GITHUB_RUN_ID is required for a production release")?;
    if run_id.is_empty() || !run_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("workflow run identity is malformed".into());
    }
    Ok(Context {
        schema_version: 1,
        repository: repository.to_owned(),
        version: version_text,
        channel: channel.to_owned(),
        tag,
        release_notes_path: notes_path,
        source_sha,
        workflow_sha,
        run_id,
        created_at: utc_now(),
    })
}

fn git_text(root: &Path, args: &[&str]) -> Result<String> {
    let output = process::capture("git", args, root, &[])?;
    if !output.status.success() {
        return Err("git identity query failed".into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn validate_release_notes(root: &Path, relative: &str, version: &str) -> Result<PathBuf> {
    let path = root.join(relative);
    let content = fs::read_to_string(&path)
        .map_err(|_| "release notes for the selected version are missing")?;
    let expected = format!("# Sky Auto Player v{version}");
    if content.trim().is_empty() || content.lines().next().map(str::trim) != Some(expected.as_str())
    {
        return Err("release notes heading must match the exact candidate version".into());
    }
    Ok(path)
}

fn utc_now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64);
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        day_seconds / 3600,
        (day_seconds / 60) % 60,
        day_seconds % 60
    )
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn read_context(root: &Path, options: &Options) -> Result<Context> {
    let context: Context = read_json(&options.state_root.join("release-context.json"))?;
    validate_context(root, options, &context)?;
    Ok(context)
}

fn validate_context(root: &Path, options: &Options, context: &Context) -> Result<()> {
    if context.schema_version != 1 || context.repository != REPOSITORY {
        return Err("release context has an unsupported schema or repository".into());
    }
    if context.tag != format!("v{}", context.version)
        || context.release_notes_path != format!("docs/releases/{}.md", context.tag)
        || context.source_sha.len() != 40
        || context.workflow_sha.len() != 40
        || !context
            .source_sha
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || !context
            .workflow_sha
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || context.source_sha != git_text(root, &["rev-parse", "HEAD"])?
    {
        return Err("release context identity differs from the checked-out source".into());
    }
    let parsed = version::parse(&context.version)?;
    version::validate_channel(&context.channel, &context.version, &parsed)?;
    validate_release_notes(root, &context.release_notes_path, &context.version)?;
    if options
        .workflow_sha
        .as_ref()
        .is_some_and(|sha| sha.to_ascii_lowercase() != context.workflow_sha)
    {
        return Err("release workflow SHA differs from release-context.json".into());
    }
    if env::var("GITHUB_SHA").is_ok_and(|sha| sha.to_ascii_lowercase() != context.workflow_sha) {
        return Err("release context workflow SHA differs from GITHUB_SHA".into());
    }
    if env::var("GITHUB_REPOSITORY").is_ok_and(|repository| repository != context.repository) {
        return Err("release context repository differs from GITHUB_REPOSITORY".into());
    }
    if env::var("GITHUB_RUN_ID").is_ok_and(|run_id| run_id != context.run_id) {
        return Err("release context run ID differs from GITHUB_RUN_ID".into());
    }
    if context.run_id.is_empty() || !context.run_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("release context run ID must contain decimal digits only".into());
    }
    if env::var("GITHUB_EVENT_NAME").is_ok_and(|event| event != "workflow_dispatch") {
        return Err("release context is not executing under workflow_dispatch".into());
    }
    if env::var("GITHUB_REF").is_ok_and(|reference| reference != "refs/heads/main") {
        return Err("release context is not executing from refs/heads/main".into());
    }
    Ok(())
}

fn preflight(root: &Path, options: &Options, transport: &mut dyn Transport) -> Result<()> {
    let context = identity(root, options.workflow_sha.as_deref())?;
    let main = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/git/ref/heads/main"),
        None,
    )?;
    if main["ref"] != "refs/heads/main"
        || main["object"]["sha"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase()
            != context.source_sha
    {
        return Err("canonical main does not match exact checked-out release SHA".into());
    }
    let bootstrap = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/contents/{BOOTSTRAP_PATH}?ref=release-metadata"),
        None,
    )?;
    if String::from_utf8(decode_contents(&bootstrap)?)?.trim() != BOOTSTRAP_CONTENT {
        return Err("release-metadata bootstrap contract content mismatch".into());
    }
    for name in ["stable", "beta"] {
        let endpoint =
            format!("repos/{REPOSITORY}/contents/channels/{name}/latest.json?ref=release-metadata");
        if let Some(value) = transport.request("GET", &endpoint, None)? {
            let bytes = decode_contents(&value)?;
            let path = options
                .state_root
                .join(format!("preflight-{name}-latest.json"));
            fs::write(&path, &bytes)?;
            release_metadata::validate(release_metadata::Channel::parse(name)?, &path)?;
        }
    }
    let tag_ref = transport.request(
        "GET",
        &format!("repos/{REPOSITORY}/git/ref/tags/{}", context.tag),
        None,
    )?;
    let mut matching_draft_id = None;
    let release = transport.request(
        "GET",
        &format!("repos/{REPOSITORY}/releases/tags/{}", context.tag),
        None,
    )?;
    if let Some(release) = release {
        if !release["draft"].as_bool().unwrap_or(false) {
            return Err(
                "repository already contains a published release/tag; published tags are immutable"
                    .into(),
            );
        }
        if !matches_transaction(&release, &context, false) {
            return Err(
                "repository contains a conflicting draft for the requested release tag".into(),
            );
        }
        matching_draft_id = release["id"].as_u64();
    } else if tag_ref.is_some() {
        return Err("repository already contains a tag without an associated release; published tags are immutable".into());
    }
    let latest = transport
        .request("GET", &format!("repos/{REPOSITORY}/releases/latest"), None)?
        .ok_or("GitHub Latest release is missing")?;
    validate_stable_latest(&latest)?;
    let context_path = options.state_root.join("release-context.json");
    write_json_atomic(&context_path, &context)?;
    let sentinel = json!({"schema_version":1,"repository":REPOSITORY,"run_id":context.run_id,"source_sha":context.source_sha});
    write_json_atomic(&options.state_root.join(".release-owner.json"), &sentinel)?;
    write_json_atomic(
        &options.state_root.join("preflight-evidence.json"),
        &json!({
            "version": context.version, "channel": context.channel, "tag": context.tag,
            "source_sha": context.source_sha, "release_context": "release-context.json",
            "stale_matching_draft": matching_draft_id.is_some(), "stale_matching_draft_id": matching_draft_id,
            "preflight_status": "PASS", "pre_publication_latest": latest, "timestamp": context.created_at
        }),
    )?;
    println!(
        "V4 Rust release preflight: PASS (version={}, channel={}, source={})",
        context.version, context.channel, context.source_sha
    );
    Ok(())
}

fn build_candidate(root: &Path, options: &Options) -> Result<()> {
    let context = read_context(root, options)?;
    let key_path = options
        .key_path
        .as_ref()
        .ok_or("BuildCandidate requires --key-path")?;
    let key_path =
        fs::canonicalize(key_path).map_err(|_| "updater private key path is not a file")?;
    let workspace = fs::canonicalize(root)?;
    if key_path.starts_with(&workspace) || !key_path.is_file() {
        return Err("updater private key must be a file outside the workspace".into());
    }
    for name in [
        "TAURI_SIGNING_PRIVATE_KEY",
        "TAURI_SIGNING_PRIVATE_KEY_PATH",
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
    ] {
        if env::var_os(name).is_some_and(|value| !value.is_empty()) {
            return Err("ambient Tauri updater signing environment is forbidden".into());
        }
    }
    let source_installer = release_metadata::canonical_installer_name(&context.version);
    let source_signature = format!("{source_installer}.sig");
    let dist = root.join("rust/target/dist");
    let bundle = dist.join("bundle/nsis");
    purge_build_outputs(root, &dist, &bundle, &source_installer, &source_signature)?;
    let credential_args = vec![
        "run-action".to_owned(),
        "--action".to_owned(),
        "build-candidate".to_owned(),
        "--expected-source-sha".to_owned(),
        context.source_sha.clone(),
        "--version".to_owned(),
        context.version.clone(),
        "--channel".to_owned(),
        context.channel.clone(),
        "--key-path".to_owned(),
        key_path.to_string_lossy().into_owned(),
    ];
    credential_cli::run(root, &credential_args)?;

    let candidate_bundle = options.state_root.join("candidate-bundle");
    let candidate_evidence = options.state_root.join("candidate-evidence");
    fs::create_dir_all(&candidate_bundle)?;
    fs::create_dir_all(&candidate_evidence)?;
    let mut actual_bundle = fs::read_dir(&bundle)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().to_string()))
        .collect::<std::io::Result<Vec<_>>>()?;
    actual_bundle.sort();
    let mut expected_bundle = vec![source_installer.clone(), source_signature.clone()];
    expected_bundle.sort();
    if actual_bundle != expected_bundle {
        return Err(
            "Tauri build output must contain exactly the canonical installer and signature".into(),
        );
    }
    for name in [&source_installer, &source_signature] {
        let source = bundle.join(name);
        let file_type = fs::symlink_metadata(&source)?.file_type();
        if !file_type.is_file() {
            return Err(format!("Tauri build output is not a regular file: {name}").into());
        }
        fs::copy(source, candidate_bundle.join(name))?;
    }
    let frozen_installer = candidate_bundle.join(&source_installer);
    let frozen_signature = candidate_bundle.join(&source_signature);
    let frozen_sbom = candidate_evidence.join(EVIDENCE_NAMES[5]);
    let frozen_summary = candidate_evidence.join(EVIDENCE_NAMES[4]);
    let frozen_authenticode = candidate_evidence.join(EVIDENCE_NAMES[2]);
    run_xtask(
        root,
        &[
            "authenticode",
            "verify",
            "--mode",
            "unsigned-zero-budget",
            "--artifact",
            frozen_installer.to_str().ok_or("invalid installer path")?,
            "--evidence",
            frozen_authenticode
                .to_str()
                .ok_or("invalid Authenticode evidence path")?,
        ],
    )?;
    run_xtask(
        root,
        &[
            "updater-trust",
            "verify-signature",
            "--installer",
            frozen_installer.to_str().ok_or("invalid installer path")?,
            "--signature",
            frozen_signature.to_str().ok_or("invalid signature path")?,
        ],
    )?;
    run_xtask(
        root,
        &[
            "sbom",
            "generate",
            "--artifact-dir",
            candidate_bundle.to_str().ok_or("invalid bundle path")?,
            "--output",
            frozen_sbom.to_str().ok_or("invalid SBOM path")?,
        ],
    )?;
    run_xtask(
        root,
        &[
            "sbom",
            "verify",
            "--artifact-dir",
            candidate_bundle.to_str().ok_or("invalid bundle path")?,
            "--sbom",
            frozen_sbom.to_str().ok_or("invalid SBOM path")?,
        ],
    )?;
    run_xtask(
        root,
        &[
            "verify-tauri-bundle",
            "--bundle-dir",
            candidate_bundle.to_str().ok_or("invalid bundle path")?,
            "--summary",
            frozen_summary.to_str().ok_or("invalid summary path")?,
            "--authenticode-evidence",
            frozen_authenticode
                .to_str()
                .ok_or("invalid evidence path")?,
            "--sbom",
            frozen_sbom.to_str().ok_or("invalid SBOM path")?,
        ],
    )?;

    let mut records = Vec::new();
    for (name, role, folder) in [
        (source_installer.as_str(), "installer", "candidate-bundle"),
        (
            source_signature.as_str(),
            "updater-signature",
            "candidate-bundle",
        ),
        (
            EVIDENCE_NAMES[2],
            "authenticode-evidence",
            "candidate-evidence",
        ),
        (EVIDENCE_NAMES[4], "artifact-summary", "candidate-evidence"),
        (EVIDENCE_NAMES[5], "sbom", "candidate-evidence"),
    ] {
        records.push(candidate_record(&options.state_root, name, role, folder)?);
    }

    let canonical_key = options.state_root.join("canonical-updater-public-key.txt");
    run_xtask(
        root,
        &[
            "updater-trust",
            "export-public-key",
            "--output",
            canonical_key.to_str().ok_or("invalid key output path")?,
        ],
    )?;
    let candidate_root = options.state_root.join("rust-updater-candidate-contract");
    run_xtask(
        root,
        &[
            "ci",
            "candidate",
            "create",
            "--bundle-dir",
            options
                .state_root
                .join("candidate-bundle")
                .to_str()
                .ok_or("invalid bundle path")?,
            "--public-key",
            canonical_key.to_str().ok_or("invalid public key path")?,
            "--calibration-binary",
            root.join("desktop/src-tauri/resources/native_calibration.exe")
                .to_str()
                .ok_or("invalid calibration path")?,
            "--output-root",
            candidate_root.to_str().ok_or("invalid candidate path")?,
            "--source-sha",
            &context.source_sha,
        ],
    )?;
    let candidate: Value = read_json(&candidate_root.join("candidate.json"))?;
    let candidate_installer = candidate_root.join(
        candidate["installer"]
            .as_str()
            .ok_or("candidate contract has no installer")?,
    );
    let runner_temp = env::var_os("RUNNER_TEMP")
        .ok_or("RUNNER_TEMP is unavailable for qualification isolation")?;
    let fixture = unique_temp(
        Path::new(&runner_temp),
        "sky-auto-player-v4-updater-fixture",
    );
    run_xtask(
        root,
        &[
            "ci",
            "updater-fixture",
            "run",
            "--candidate-root",
            candidate_root.to_str().ok_or("invalid candidate path")?,
            "--source-sha",
            &context.source_sha,
            "--fixture-root",
            fixture.to_str().ok_or("invalid fixture path")?,
            "--evidence",
            options
                .state_root
                .join("fixture-http-evidence.json")
                .to_str()
                .ok_or("invalid evidence path")?,
            "--build-bridge",
            "--bridge-version",
            "4.1.3",
            "--bridge-publisher",
            "github",
            "--allow-protected-release-runner",
        ],
    )?;
    let defender_evidence = options.state_root.join("defender-evidence.json");
    run_pwsh(
        root,
        &[
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            root.join("scripts/scan_v4_defender_exact.ps1")
                .to_str()
                .ok_or("invalid Defender adapter path")?,
            "-Artifact",
            frozen_installer.to_str().ok_or("invalid installer path")?,
            "-Evidence",
            defender_evidence
                .to_str()
                .ok_or("invalid Defender evidence path")?,
        ],
    )?;
    let defender: Value = read_json(&defender_evidence)?;
    if defender["scan_performed"] != true || defender["detection_result"] != "none" {
        return Err("Defender evidence does not bind a clean exact-candidate scan".into());
    }

    let smoke = unique_temp(
        Path::new(&runner_temp),
        "sky-auto-player-v4-candidate-smoke",
    );
    run_xtask(
        root,
        &[
            "ci",
            "nsis",
            "smoke",
            "--installer",
            candidate_installer
                .to_str()
                .ok_or("invalid installer path")?,
            "--candidate-root",
            candidate_root.to_str().ok_or("invalid candidate path")?,
            "--source-sha",
            &context.source_sha,
            "--fixture-root",
            smoke.to_str().ok_or("invalid smoke path")?,
            "--registry-mode",
            "fresh-install",
            "--evidence-root",
            candidate_evidence
                .to_str()
                .ok_or("invalid evidence directory")?,
            "--authenticode-mode",
            "unsigned-zero-budget",
            "--allow-protected-release-runner",
        ],
    )?;

    write_release_evidence(&context, &records, &candidate_evidence)?;
    records.push(candidate_record(
        &options.state_root,
        EVIDENCE_NAMES[0],
        "production-evidence",
        "candidate-evidence",
    )?);
    records.push(candidate_record(
        &options.state_root,
        EVIDENCE_NAMES[1],
        "qualification-evidence",
        "candidate-evidence",
    )?);
    records.push(candidate_record(
        &options.state_root,
        EVIDENCE_NAMES[3],
        "installed-authenticode-evidence",
        "candidate-evidence",
    )?);
    ensure_exact_candidate_topology(
        &options.state_root,
        &records,
        &source_installer,
        &source_signature,
    )?;
    validate_qualification_evidence(
        &candidate_evidence.join(EVIDENCE_NAMES[1]),
        &context,
        &records,
    )?;
    validate_candidate_evidence(&options.state_root, &records, &context)?;

    let public_names = [
        release_metadata::canonical_release_installer_name(&context.version),
        format!(
            "{}.sig",
            release_metadata::canonical_release_installer_name(&context.version)
        ),
    ];
    let public_assets = records
        .iter()
        .filter(|record| public_names.contains(&record.release_name))
        .cloned()
        .collect::<Vec<_>>();
    if public_assets.len() != 2 {
        return Err(
            "candidate manifest must contain exactly the canonical public installer and signature"
                .into(),
        );
    }
    let manifest = CandidateManifest {
        schema_version: 1,
        source_sha: context.source_sha.clone(),
        version: context.version.clone(),
        channel: context.channel.clone(),
        tag: context.tag.clone(),
        qualification_assets: records,
        public_assets,
    };
    write_json_atomic(
        &options.state_root.join("candidate-manifest.json"),
        &manifest,
    )?;
    let state = ReleaseState {
        schema_version: 1,
        repository: context.repository,
        version: context.version,
        channel: context.channel,
        tag: context.tag,
        source_sha: context.source_sha,
        workflow_sha: context.workflow_sha,
        run_id: context.run_id,
        phase: Phase::Ready,
        attested: false,
        qualified_after_download: false,
        release_id: None,
        published_at: None,
        promoted_at: None,
        final_verified: false,
    };
    write_json_atomic(&options.state_root.join("release-state.json"), &state)?;
    println!(
        "V4 Rust candidate qualification: PASS (frozen candidate, exact evidence, updater fixture, Defender, NSIS smoke)"
    );
    Ok(())
}

fn unique_temp(parent: &Path, prefix: &str) -> PathBuf {
    let entropy = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |value| value.as_nanos())
    );
    parent.join(format!("{prefix}-{entropy}"))
}

fn purge_build_outputs(
    workspace: &Path,
    dist: &Path,
    bundle: &Path,
    installer: &str,
    signature: &str,
) -> Result<()> {
    let canonical_workspace = fs::canonicalize(workspace)?;
    fs::create_dir_all(dist)?;
    let canonical_dist = fs::canonicalize(dist)?;
    if !canonical_dist.starts_with(&canonical_workspace) {
        return Err("Tauri dist output directory escapes the repository workspace".into());
    }
    fs::create_dir_all(bundle)?;
    let canonical_bundle = fs::canonicalize(bundle)?;
    if !canonical_bundle.starts_with(&canonical_dist) {
        return Err("Tauri bundle output directory escapes the Rust dist root".into());
    }
    for path in [
        bundle.join(installer),
        bundle.join(signature),
        dist.join(EVIDENCE_NAMES[0]),
        dist.join(EVIDENCE_NAMES[1]),
        dist.join(EVIDENCE_NAMES[2]),
        dist.join(EVIDENCE_NAMES[3]),
        dist.join(EVIDENCE_NAMES[4]),
        dist.join(EVIDENCE_NAMES[5]),
    ] {
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() || metadata.file_type().is_symlink() => {
                fs::remove_file(&path)?;
            }
            Ok(_) => {
                return Err(format!(
                    "refusing to purge non-file candidate output: {}",
                    path.display()
                )
                .into());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn run_xtask(root: &Path, args: &[&str]) -> Result<()> {
    let executable = env::current_exe()?;
    let args = args
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect::<Vec<_>>();
    let output = process::capture_owned_timeout_filtered(
        executable.as_os_str(),
        &args,
        root,
        &[],
        &[],
        RELEASE_BUILD_STEP_TIMEOUT,
    )?;
    if !output.status.success() {
        return Err(format!(
            "Rust candidate qualification step failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(())
}

fn run_pwsh(root: &Path, args: &[&str]) -> Result<()> {
    let args = args
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect::<Vec<_>>();
    let output = process::capture_owned_timeout_filtered(
        std::ffi::OsStr::new("pwsh"),
        &args,
        root,
        &[],
        &[],
        RELEASE_DEFENDER_STEP_TIMEOUT,
    )?;
    if !output.status.success() {
        return Err(format!(
            "Windows evidence adapter failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let metadata = fs::metadata(path)
        .map_err(|_| format!("required release state file is missing: {}", path.display()))?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Err("release state JSON exceeds the 16 MiB limit".into());
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().ok_or("release state path has no parent")?;
    fs::create_dir_all(parent)?;
    let payload = serde_json::to_vec_pretty(value)?;
    let temp = parent.join(format!(
        ".release-state-{}-{}.tmp",
        std::process::id(),
        utc_now().replace([':', '-'], "")
    ));
    fs::write(&temp, [payload.as_slice(), b"\n"].concat())?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temp, path)?;
    Ok(())
}

fn candidate_record(root: &Path, name: &str, role: &str, folder: &str) -> Result<CandidateRecord> {
    let state_relative = Path::new(folder).join(name);
    let path = safe_state_asset_path(
        root,
        &CandidateRecord {
            name: name.to_owned(),
            release_name: name.replace(' ', "."),
            source_name: name.to_owned(),
            role: role.to_owned(),
            size: 0,
            sha256: String::new(),
            source_path: String::new(),
            state_path: state_relative.to_string_lossy().replace('\\', "/"),
        },
    )?;
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() {
        return Err(format!("candidate evidence is not a regular file: {name}").into());
    }
    let bytes = fs::read(&path)?;
    if bytes.is_empty() {
        return Err(format!("candidate evidence is empty: {name}").into());
    }
    let release_name = name.replace(' ', ".");
    Ok(CandidateRecord {
        name: release_name.clone(),
        release_name,
        source_name: name.to_owned(),
        role: role.to_owned(),
        size: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
        source_path: path.to_string_lossy().into_owned(),
        state_path: state_relative.to_string_lossy().replace('\\', "/"),
    })
}

fn write_release_evidence(
    context: &Context,
    records: &[CandidateRecord],
    evidence_root: &Path,
) -> Result<()> {
    let record = |role: &str| {
        records
            .iter()
            .find(|record| record.role == role)
            .ok_or_else(|| format!("candidate evidence input is missing role {role}"))
    };
    let installer = record("installer")?;
    let signature = record("updater-signature")?;
    let authenticode = record("authenticode-evidence")?;
    let sbom = record("sbom")?;
    let auth_path = safe_state_asset_path(
        evidence_root
            .parent()
            .ok_or("evidence root has no parent")?,
        authenticode,
    )?;
    let auth: Value = read_json(&auth_path)?;
    let files = auth["files"]
        .as_array()
        .ok_or("unsigned-zero-budget Authenticode evidence omitted files")?;
    if files.len() != 1
        || files[0]["name"] != installer.source_name
        || !files[0]["signer_thumbprint"].is_null()
    {
        return Err("production Authenticode evidence did not prove the unsigned installer".into());
    }
    let qualification = json!({
        "schema_version":1,
        "evidence_type":"tauri-nsis-qualified-release",
        "qualified":true,
        "qualification":"install-launch-uninstall",
        "product_name":"Sky Auto Player",
        "identifier":"io.github.pumni.skyautoplayer",
        "version":context.version,
        "target":"nsis",
        "install_mode":"currentUser",
        "installer":installer.source_name,
        "updater_signature":signature.source_name,
        "installer_size":installer.size,
        "signature_size":signature.size,
        "installer_sha256":installer.sha256,
        "updater_signature_sha256":signature.sha256,
        "authenticode_mode":"unsigned-zero-budget",
        "authenticode_evidence":authenticode.source_name,
        "authenticode_evidence_sha256":authenticode.sha256,
        "sbom":sbom.source_name,
        "sbom_sha256":sbom.sha256,
    });
    let production = json!({
        "schema_version":1,
        "evidence_type":"v4-production-release-qualification",
        "source_sha":context.source_sha,
        "version":context.version,
        "channel":context.channel,
        "product_name":"Sky Auto Player",
        "identifier":"io.github.pumni.skyautoplayer",
        "target":"nsis",
        "install_mode":"currentUser",
        "installer":installer.source_name,
        "installer_size":installer.size,
        "installer_sha256":installer.sha256,
        "updater_signature":signature.source_name,
        "signature_size":signature.size,
        "updater_signature_sha256":signature.sha256,
        "authenticode_mode":"unsigned-zero-budget",
        "authenticode_state":"unsigned",
        "authenticode_provider":"none",
        "approved_signer_thumbprint":Value::Null,
        "observed_signer_thumbprint":Value::Null,
        "authenticode_evidence":authenticode.source_name,
        "authenticode_evidence_sha256":authenticode.sha256,
        "updater_key_id":crate::updater_trust::canonical_key_id()?,
        "updater_signature_status":"valid",
        "sbom":sbom.source_name,
        "sbom_sha256":sbom.sha256,
        "qualification_status":"PASS",
    });
    write_json_atomic(&evidence_root.join(EVIDENCE_NAMES[1]), &qualification)?;
    write_json_atomic(&evidence_root.join(EVIDENCE_NAMES[0]), &production)?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn safe_state_asset_path(root: &Path, record: &CandidateRecord) -> Result<PathBuf> {
    let relative = Path::new(&record.state_path);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "candidate manifest has an unsafe state path: {}",
            record.state_path
        )
        .into());
    }
    let root = fs::canonicalize(root)?;
    let candidate = root.join(relative);
    let resolved = fs::canonicalize(&candidate)
        .map_err(|_| format!("candidate asset is missing: {}", record.release_name))?;
    if !resolved.starts_with(&root) || !resolved.is_file() {
        return Err("candidate asset escapes the release state root".into());
    }
    Ok(resolved)
}

fn ensure_exact_candidate_topology(
    root: &Path,
    records: &[CandidateRecord],
    installer: &str,
    signature: &str,
) -> Result<()> {
    let bundle = root.join("candidate-bundle");
    let mut actual = fs::read_dir(&bundle)?
        .map(|entry| entry.map(|e| e.file_name().to_string_lossy().to_string()))
        .collect::<std::io::Result<Vec<_>>>()?;
    actual.sort();
    let mut expected = vec![installer.to_owned(), signature.to_owned()];
    expected.sort();
    if actual != expected {
        return Err(
            "candidate-bundle must contain exactly the installer and updater signature".into(),
        );
    }
    let evidence = root.join("candidate-evidence");
    for name in EVIDENCE_NAMES {
        if !evidence.join(name).is_file() {
            return Err(format!("candidate-evidence is missing {name}").into());
        }
    }
    if records.len() != 8 {
        return Err("candidate manifest must contain exactly eight qualification files".into());
    }
    for record in records {
        verify_record(root, record)?;
    }
    Ok(())
}

fn verify_record(root: &Path, record: &CandidateRecord) -> Result<PathBuf> {
    let path = safe_state_asset_path(root, record)?;
    let bytes = fs::read(&path)?;
    if bytes.len() as u64 != record.size || sha256_hex(&bytes) != record.sha256 {
        return Err(format!(
            "candidate file differs from its frozen manifest record: {}",
            record.release_name
        )
        .into());
    }
    Ok(path)
}

fn validate_candidate_evidence(
    root: &Path,
    records: &[CandidateRecord],
    context: &Context,
) -> Result<()> {
    let production_record = records
        .iter()
        .find(|record| record.source_name == EVIDENCE_NAMES[0])
        .ok_or("production evidence record missing")?;
    let qualification_record = records
        .iter()
        .find(|record| record.source_name == EVIDENCE_NAMES[1])
        .ok_or("qualification evidence record missing")?;
    let production: Value = read_json(&safe_state_asset_path(root, production_record)?)?;
    let qualification: Value = read_json(&safe_state_asset_path(root, qualification_record)?)?;
    if production["source_sha"] != context.source_sha
        || production["version"] != context.version
        || production["channel"] != context.channel
        || production["authenticode_mode"] != "unsigned-zero-budget"
        || production["authenticode_state"] != "unsigned"
        || production["authenticode_provider"] != "none"
        || !production["approved_signer_thumbprint"].is_null()
        || !production["observed_signer_thumbprint"].is_null()
        || production["updater_signature_status"] != "valid"
        || production["qualification_status"] != "PASS"
    {
        return Err("production evidence does not match release identity or unsigned-zero-budget trust state".into());
    }
    for record in records.iter().filter(|record| {
        matches!(
            record.role.as_str(),
            "installer" | "updater-signature" | "authenticode-evidence" | "sbom"
        )
    }) {
        let (name_field, size_field, digest_field) = match record.role.as_str() {
            "installer" => ("installer", "installer_size", "installer_sha256"),
            "updater-signature" => (
                "updater_signature",
                "signature_size",
                "updater_signature_sha256",
            ),
            "authenticode-evidence" => {
                ("authenticode_evidence", "", "authenticode_evidence_sha256")
            }
            "sbom" => ("sbom", "", "sbom_sha256"),
            _ => unreachable!(),
        };
        for evidence in [&production, &qualification] {
            if evidence[name_field] != record.source_name || evidence[digest_field] != record.sha256
            {
                return Err(format!(
                    "candidate evidence identity or digest mismatch for {}",
                    record.source_name
                )
                .into());
            }
            if !size_field.is_empty() && evidence[size_field].as_u64() != Some(record.size) {
                return Err(format!(
                    "candidate evidence byte count mismatch for {}",
                    record.source_name
                )
                .into());
            }
        }
    }
    if qualification["authenticode_mode"] != "unsigned-zero-budget" {
        return Err("qualification evidence has an unsupported Authenticode mode".into());
    }
    Ok(())
}

fn validate_qualification_evidence(
    path: &Path,
    context: &Context,
    records: &[CandidateRecord],
) -> Result<()> {
    let evidence: Value = read_json(path)?;
    let required = [
        "schema_version",
        "evidence_type",
        "qualified",
        "qualification",
        "product_name",
        "identifier",
        "version",
        "target",
        "install_mode",
        "installer",
        "updater_signature",
        "installer_size",
        "signature_size",
        "installer_sha256",
        "updater_signature_sha256",
        "authenticode_mode",
        "authenticode_evidence",
        "authenticode_evidence_sha256",
        "sbom",
        "sbom_sha256",
    ];
    let object = evidence
        .as_object()
        .ok_or("qualification evidence must be one JSON object")?;
    if object.len() != required.len() || object.keys().any(|key| !required.contains(&key.as_str()))
    {
        return Err(
            "qualification evidence schema is not the bounded canonical Tauri schema".into(),
        );
    }
    if evidence["schema_version"].as_u64() != Some(1)
        || evidence["evidence_type"] != "tauri-nsis-qualified-release"
        || evidence["qualification"] != "install-launch-uninstall"
        || evidence["qualified"] != true
        || evidence["authenticode_mode"] != "unsigned-zero-budget"
        || evidence["product_name"] != "Sky Auto Player"
        || evidence["identifier"] != "io.github.pumni.skyautoplayer"
        || evidence["version"] != context.version
        || evidence["target"] != "nsis"
        || evidence["install_mode"] != "currentUser"
    {
        return Err("qualification evidence does not match canonical product, version, or qualification path".into());
    }
    let installer = records
        .iter()
        .find(|record| record.role == "installer")
        .ok_or("installer record missing")?;
    let signature = records
        .iter()
        .find(|record| record.role == "updater-signature")
        .ok_or("signature record missing")?;
    if evidence["installer"] != installer.source_name
        || evidence["updater_signature"] != signature.source_name
        || evidence["installer_size"].as_u64() != Some(installer.size)
        || evidence["signature_size"].as_u64() != Some(signature.size)
        || evidence["installer_sha256"] != installer.sha256
        || evidence["updater_signature_sha256"] != signature.sha256
    {
        return Err(
            "qualification evidence installer/signature identity does not match frozen candidate"
                .into(),
        );
    }
    for field in [
        "installer_sha256",
        "updater_signature_sha256",
        "authenticode_evidence_sha256",
        "sbom_sha256",
    ] {
        if evidence[field].as_str().is_none_or(|digest| {
            digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err(
                format!("qualification evidence has malformed SHA-256 field {field}").into(),
            );
        }
    }
    Ok(())
}

fn required_api(
    transport: &mut dyn Transport,
    method: &str,
    endpoint: &str,
    body: Option<&Value>,
) -> Result<Value> {
    transport.request(method, endpoint, body)?.ok_or_else(|| {
        format!("GitHub API {method} {endpoint} returned 404 or an empty response").into()
    })
}

fn decode_contents(value: &Value) -> Result<Vec<u8>> {
    let content = value["content"]
        .as_str()
        .ok_or("GitHub Contents response omitted content")?;
    let compact = content
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    Ok(base64::engine::general_purpose::STANDARD.decode(compact.as_bytes())?)
}

fn matches_transaction(release: &Value, context: &Context, require_run: bool) -> bool {
    if release["tag_name"] != context.tag
        || release["target_commitish"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase()
            != context.source_sha
        || !release["draft"].as_bool().unwrap_or(false)
    {
        return false;
    }
    let Some(body) = release["body"].as_str() else {
        return false;
    };
    let marker = "<!-- v4-release-tx:";
    let mut found = body.match_indices(marker);
    let Some((start, _)) = found.next() else {
        return false;
    };
    if found.next().is_some() {
        return false;
    }
    let Some(end_relative) = body[start..].find("-->") else {
        return false;
    };
    let raw = body[start + marker.len()..start + end_relative].trim();
    for key in ["repository", "run_id", "source_sha", "version", "tag"] {
        if raw.matches(&format!("\"{key}\"")).count() != 1 {
            return false;
        }
    }
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    value["repository"] == context.repository
        && value["source_sha"]
            .as_str()
            .unwrap_or_default()
            .eq_ignore_ascii_case(&context.source_sha)
        && value["version"] == context.version
        && value["tag"] == context.tag
        && value["run_id"]
            .as_str()
            .is_some_and(|run| !run.is_empty() && (!require_run || run == context.run_id))
}

fn validate_stable_latest(release: &Value) -> Result<()> {
    if release["draft"] == true
        || release["prerelease"] == true
        || release["published_at"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
        || !release["tag_name"]
            .as_str()
            .unwrap_or_default()
            .starts_with('v')
    {
        return Err("GitHub Latest must be a published stable release".into());
    }
    let tag = release["tag_name"].as_str().unwrap_or_default();
    let value = tag.strip_prefix('v').ok_or("GitHub Latest tag malformed")?;
    let parsed = version::parse(value)?;
    if !parsed.pre.is_empty() {
        return Err("GitHub Latest is outside the stable release namespace".into());
    }
    Ok(())
}

fn validate_beta_keeps_latest(
    before: &Value,
    after: &Value,
    published_id: u64,
    tag: &str,
) -> Result<()> {
    validate_stable_latest(after)?;
    for field in [
        "id",
        "tag_name",
        "target_commitish",
        "draft",
        "prerelease",
        "published_at",
    ] {
        if before[field] != after[field] {
            return Err(format!("beta release changed GitHub Latest identity: {field}").into());
        }
    }
    if after["id"].as_u64() == Some(published_id)
        || after["tag_name"] == tag
        || after["prerelease"] == true
    {
        return Err("beta release displaced GitHub Latest".into());
    }
    Ok(())
}

fn read_manifest(root: &Path, options: &Options, context: &Context) -> Result<CandidateManifest> {
    let manifest: CandidateManifest =
        read_json(&options.state_root.join("candidate-manifest.json"))?;
    if manifest.schema_version != 1 || manifest.repository_mismatch(context) {
        return Err("candidate manifest release identity differs from release context".into());
    }
    if manifest.qualification_assets.len() != 8 || manifest.public_assets.len() != 2 {
        return Err(
            "candidate manifest must contain eight qualification files and two public assets"
                .into(),
        );
    }
    let expected_installer = release_metadata::canonical_release_installer_name(&context.version);
    let expected_public = [
        expected_installer.clone(),
        format!("{expected_installer}.sig"),
    ];
    let mut actual = manifest
        .public_assets
        .iter()
        .map(|record| record.release_name.clone())
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = expected_public.to_vec();
    expected.sort();
    if actual != expected {
        return Err("candidate manifest public assets are not the exact canonical pair".into());
    }
    for record in &manifest.qualification_assets {
        verify_record(&options.state_root, record)?;
    }
    for record in &manifest.public_assets {
        if !manifest.qualification_assets.contains(record) {
            return Err("public asset record is not bound by qualification manifest".into());
        }
    }
    validate_candidate_evidence(&options.state_root, &manifest.qualification_assets, context)?;
    let _ = root;
    Ok(manifest)
}

fn verify_attestations(root: &Path, options: &Options) -> Result<()> {
    let context = read_context(root, options)?;
    let mut state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    validate_state(&state, &context)?;
    if state.phase != Phase::Ready {
        return Err("attestation verification requires a READY candidate".into());
    }
    let manifest = read_manifest(root, options, &context)?;
    let installer_record = manifest
        .public_assets
        .iter()
        .find(|record| record.role == "installer")
        .ok_or("installer record missing")?;
    let signature_record = manifest
        .public_assets
        .iter()
        .find(|record| record.role == "updater-signature")
        .ok_or("signature record missing")?;
    let installer = safe_state_asset_path(&options.state_root, installer_record)?;
    let signature = safe_state_asset_path(&options.state_root, signature_record)?;
    let signer = format!("{REPOSITORY}/.github/workflows/release-v4.yml");
    let repo_arg = REPOSITORY.to_owned();
    for subject in [&installer, &signature] {
        let args = vec![
            "attestation".to_owned(),
            "verify".to_owned(),
            subject
                .to_str()
                .ok_or("invalid attestation subject")?
                .to_owned(),
            "-R".to_owned(),
            repo_arg.clone(),
            "--source-digest".to_owned(),
            context.source_sha.clone(),
            "--signer-workflow".to_owned(),
            signer.clone(),
        ];
        let output = process::capture_owned_timeout_filtered(
            std::ffi::OsStr::new("gh"),
            &args,
            root,
            &[],
            &[],
            ATTESTATION_VERIFY_TIMEOUT,
        )?;
        if !output.status.success() {
            return Err(format!(
                "exact-source candidate attestation verification failed: {}",
                safe_cli_error(&String::from_utf8_lossy(&output.stderr))
            )
            .into());
        }
    }
    let args = vec![
        "attestation".to_owned(),
        "verify".to_owned(),
        installer
            .to_str()
            .ok_or("invalid installer attestation path")?
            .to_owned(),
        "-R".to_owned(),
        repo_arg,
        "--predicate-type".to_owned(),
        "https://spdx.dev/Document/v2.3".to_owned(),
        "--source-digest".to_owned(),
        context.source_sha.clone(),
        "--signer-workflow".to_owned(),
        signer,
    ];
    let output = process::capture_owned_timeout_filtered(
        std::ffi::OsStr::new("gh"),
        &args,
        root,
        &[],
        &[],
        ATTESTATION_VERIFY_TIMEOUT,
    )?;
    if !output.status.success() {
        return Err(format!(
            "exact-source SBOM attestation verification failed: {}",
            safe_cli_error(&String::from_utf8_lossy(&output.stderr))
        )
        .into());
    }
    state.attested = true;
    write_json_atomic(&options.state_root.join("release-state.json"), &state)?;
    println!(
        "V4 Rust attestation verification: PASS (installer, updater signature, SBOM bound to exact source SHA)"
    );
    Ok(())
}

impl CandidateManifest {
    fn repository_mismatch(&self, context: &Context) -> bool {
        self.source_sha != context.source_sha
            || self.version != context.version
            || self.channel != context.channel
            || self.tag != context.tag
    }
}

fn publish_release(root: &Path, options: &Options, transport: &mut dyn Transport) -> Result<()> {
    let context = read_context(root, options)?;
    let mut state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    if state.phase != Phase::Ready || !state.attested {
        return Err("PublishRelease requires a READY candidate state".into());
    }
    validate_state(&state, &context)?;
    let manifest = read_manifest(root, options, &context)?;
    let latest_before = read_json::<Value>(&options.state_root.join("preflight-evidence.json"))?["pre_publication_latest"].clone();
    validate_stable_latest(&latest_before)?;
    let notes = fs::read_to_string(root.join(&context.release_notes_path))?;
    let marker = format!(
        "<!-- v4-release-tx: {} -->",
        serde_json::to_string(&json!({
            "repository":context.repository,"run_id":context.run_id,"source_sha":context.source_sha,
            "version":context.version,"tag":context.tag
        }))?
    );
    let body = format!("{}\n\n{marker}", notes.trim());

    // Reconcile only a matching unpublished draft at the first publication mutation boundary.
    if let Some(stale) = transport.request(
        "GET",
        &format!("repos/{REPOSITORY}/releases/tags/{}", context.tag),
        None,
    )? {
        if !matches_transaction(&stale, &context, false) {
            return Err(
                "existing release is published or its draft transaction identity conflicts".into(),
            );
        }
        delete_owned_draft(transport, &stale, &context)?;
    }
    if transport
        .request(
            "GET",
            &format!("repos/{REPOSITORY}/git/ref/tags/{}", context.tag),
            None,
        )?
        .is_some()
    {
        return Err("release tag already exists; immutable tags are never modified".into());
    }

    let payload = json!({
        "tag_name":context.tag,"target_commitish":context.source_sha,"name":context.tag,"body":body,
        "draft":true,"prerelease":context.channel == "beta","make_latest":"false"
    });
    let created = match transport.request(
        "POST",
        &format!("repos/{REPOSITORY}/releases"),
        Some(&payload),
    ) {
        Ok(Some(release)) => release,
        Ok(None) | Err(_) => {
            let reconciled = transport.request(
                "GET",
                &format!("repos/{REPOSITORY}/releases/tags/{}", context.tag),
                None,
            )?;
            match reconciled {
                Some(release) if matches_transaction(&release, &context, true) => release,
                Some(_) => return Err("release draft POST outcome is ambiguous and remote draft identity does not match this transaction".into()),
                None => return Err("release draft POST failed and no exact same-transaction draft could be reconciled".into()),
            }
        }
    };
    let release_id = created["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or("GitHub draft has no valid release id")?;
    if !matches_transaction(&created, &context, true) {
        return Err("GitHub draft response has mismatched transaction identity".into());
    }
    let upload_url = created["upload_url"]
        .as_str()
        .ok_or("GitHub draft has no upload URL")?
        .replace("{?name,label}", "");

    let before_publish = (|| -> Result<()> {
        for record in &manifest.public_assets {
            let path = verify_record(&options.state_root, record)?;
            transport.upload(&upload_url, &record.release_name, &path)?;
        }
        let draft = required_api(
            transport,
            "GET",
            &format!("repos/{REPOSITORY}/releases/{release_id}"),
            None,
        )?;
        verify_public_assets(transport, &draft, &manifest.public_assets, false)?;
        state.qualified_after_download = true;
        write_json_atomic(&options.state_root.join("release-state.json"), &state)?;
        let patch = json!({"draft":false,"make_latest":if context.channel == "stable" { "true" } else { "false" }});
        transport.request(
            "PATCH",
            &format!("repos/{REPOSITORY}/releases/{release_id}"),
            Some(&patch),
        )?;
        Ok(())
    })();

    if let Err(error) = before_publish {
        match transport.request("GET", &format!("repos/{REPOSITORY}/releases/{release_id}"), None) {
            Ok(Some(remote)) if remote["draft"] == true && matches_transaction(&remote, &context, true) => {
                return match delete_owned_draft(transport, &remote, &context) {
                    Ok(()) => Err(error),
                    Err(cleanup_error) => Err(format!(
                        "release transaction failed: {error}; owned draft cleanup failed: {cleanup_error}"
                    )
                    .into()),
                };
            }
            Ok(Some(remote)) if remote["draft"] == false => {
                // The PATCH may have committed while its response was lost. Continue with exact id GET.
            }
            Ok(None) => return Err(format!("release transaction failed; exact release id {release_id} is missing and cleanup was refused: {error}").into()),
            Ok(Some(_)) => return Err(format!("release transaction failed; remote release identity is ambiguous and cleanup was refused: {error}").into()),
            Err(reconcile_error) => return Err(format!("POST_PUBLICATION_INCIDENT: release PATCH outcome is unknown; cleanup was refused ({reconcile_error})").into()),
        }
    }

    // Once publication is attempted, reconcile only by the exact release id. Never clean up an
    // object whose publication status cannot be established.
    let published = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/releases/{release_id}"),
        None,
    )
    .map_err(|error| {
        format!("POST_PUBLICATION_INCIDENT: exact release id reconciliation failed: {error}")
    })?;
    validate_published_release(&published, &context, release_id)?;
    state.phase = Phase::PublishedPendingMetadata;
    state.release_id = Some(release_id);
    state.published_at = Some(canonical_timestamp(
        published["published_at"]
            .as_str()
            .ok_or("published_at missing")?,
    )?);
    write_json_atomic(&options.state_root.join("release-state.json"), &state)?;
    let latest_after = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/releases/latest"),
        None,
    )
    .map_err(|error| {
        format!("POST_PUBLICATION_INCIDENT: GitHub Latest could not be reconciled: {error}")
    })?;
    if context.channel == "stable" {
        if latest_after["id"].as_u64() != Some(release_id)
            || latest_after["tag_name"] != context.tag
            || latest_after["target_commitish"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase()
                != context.source_sha
        {
            return Err(
                "POST_PUBLICATION_INCIDENT: stable release did not become exact GitHub Latest"
                    .into(),
            );
        }
    } else {
        validate_beta_keeps_latest(&latest_before, &latest_after, release_id, &context.tag)
            .map_err(|error| format!("POST_PUBLICATION_INCIDENT: {error}"))?;
    }
    println!(
        "V4 Rust PublishRelease: PASS (tag={}, release_id={}, immutable=true)",
        context.tag, release_id
    );
    Ok(())
}

fn validate_state(state: &ReleaseState, context: &Context) -> Result<()> {
    if state.schema_version != 1
        || state.repository != context.repository
        || state.version != context.version
        || state.channel != context.channel
        || state.tag != context.tag
        || state.source_sha != context.source_sha
        || state.workflow_sha != context.workflow_sha
        || state.run_id != context.run_id
    {
        return Err("persisted release state identity differs from the current transaction".into());
    }
    Ok(())
}

fn delete_owned_draft(
    transport: &mut dyn Transport,
    draft: &Value,
    context: &Context,
) -> Result<()> {
    let id = draft["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or("owned draft has no valid id")?;
    if !matches_transaction(draft, context, false) {
        return Err("refusing draft cleanup because transaction ownership did not match".into());
    }
    transport.request("DELETE", &format!("repos/{REPOSITORY}/releases/{id}"), None)?;
    if transport
        .request("GET", &format!("repos/{REPOSITORY}/releases/{id}"), None)?
        .is_some()
    {
        return Err("owned unpublished draft was not removed by exact release id".into());
    }
    Ok(())
}

fn verify_public_assets(
    transport: &mut dyn Transport,
    release: &Value,
    expected: &[CandidateRecord],
    download: bool,
) -> Result<()> {
    if expected.len() != 2 {
        return Err("public release must contain exactly two candidate assets".into());
    }
    let assets = release["assets"]
        .as_array()
        .ok_or("GitHub release omitted assets array")?;
    let mut expected_names = expected
        .iter()
        .map(|record| record.release_name.as_str())
        .collect::<Vec<_>>();
    let mut actual_names = assets
        .iter()
        .map(|asset| asset["name"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    expected_names.sort();
    actual_names.sort();
    if actual_names != expected_names {
        return Err(
            "GitHub release assets are not the exact canonical installer/signature pair".into(),
        );
    }
    for record in expected {
        let asset = assets
            .iter()
            .find(|asset| asset["name"] == record.release_name)
            .ok_or("release asset missing")?;
        if asset["size"].as_u64() != Some(record.size) || asset["state"] != "uploaded" {
            return Err(format!(
                "GitHub asset size/state mismatch for {}",
                record.release_name
            )
            .into());
        }
        let digest = asset["digest"].as_str().ok_or_else(|| {
            format!(
                "GitHub asset server digest missing for {}",
                record.release_name
            )
        })?;
        let actual = digest
            .strip_prefix("sha256:")
            .ok_or("GitHub asset digest algorithm is not SHA-256")?;
        if actual.len() != 64
            || !actual.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !actual.eq_ignore_ascii_case(&record.sha256)
        {
            return Err(
                format!("GitHub server digest mismatch for {}", record.release_name).into(),
            );
        }
        if download {
            let url = asset["url"].as_str().ok_or("GitHub asset URL missing")?;
            let path = unique_temp(&env::temp_dir(), "sky-release-public-asset");
            let downloaded = transport.download(asset_api_endpoint(url)?, &path);
            let verification = downloaded.and_then(|()| {
                let size = fs::metadata(&path)?.len();
                let digest = crate::hash::sha256(&path)?;
                if size != record.size || digest != record.sha256 {
                    return Err(format!(
                        "downloaded public asset differs from frozen candidate: {}",
                        record.release_name
                    )
                    .into());
                }
                Ok(())
            });
            let cleanup_error = if path.exists() {
                fs::remove_file(&path).err()
            } else {
                None
            };
            if let Err(error) = verification {
                if let Some(cleanup_error) = cleanup_error {
                    return Err(format!(
                        "{error}; public asset temporary-file cleanup failed: {cleanup_error}"
                    )
                    .into());
                }
                return Err(error);
            }
            if let Some(cleanup_error) = cleanup_error {
                return Err(
                    format!("public asset temporary-file cleanup failed: {cleanup_error}").into(),
                );
            }
        }
    }
    Ok(())
}

fn asset_api_endpoint(url: &str) -> Result<&str> {
    let parsed = url::Url::parse(url)?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("api.github.com")
        || parsed
            .path()
            .strip_prefix(&format!("/repos/{REPOSITORY}/releases/assets/"))
            .is_none_or(|id| id.is_empty() || !id.bytes().all(|byte| byte.is_ascii_digit()))
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err("GitHub release asset URL is outside the canonical API host".into());
    }
    Ok(url)
}

fn validate_published_release(release: &Value, context: &Context, release_id: u64) -> Result<()> {
    if release["id"].as_u64() != Some(release_id)
        || release["draft"] != false
        || release["immutable"] != true
        || release["tag_name"] != context.tag
        || release["target_commitish"]
            .as_str()
            .unwrap_or_default()
            .to_ascii_lowercase()
            != context.source_sha
        || release["published_at"]
            .as_str()
            .is_none_or(|value| canonical_timestamp(value).is_err())
        || (context.channel == "beta") != (release["prerelease"] == true)
    {
        return Err("published release exact-id identity, immutability, source, or channel validation failed".into());
    }
    Ok(())
}

fn canonical_timestamp(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || bytes.get(10) != Some(&b'T')
    {
        return Err("timestamp is not RFC3339".into());
    }
    let year = value[0..4].parse::<i64>()?;
    let month = value[5..7].parse::<i64>()?;
    let day = value[8..10].parse::<i64>()?;
    let zone_start = value[11..]
        .char_indices()
        .find_map(|(index, character)| {
            if matches!(character, 'Z' | '+' | '-') {
                Some(index + 11)
            } else {
                None
            }
        })
        .ok_or("RFC3339 timezone is missing")?;
    let time = &value[11..zone_start];
    let mut parts = time.split(':');
    let hour = parts.next().ok_or("RFC3339 hour missing")?.parse::<i64>()?;
    let minute = parts
        .next()
        .ok_or("RFC3339 minute missing")?
        .parse::<i64>()?;
    let second_text = parts.next().ok_or("RFC3339 second missing")?;
    let second = second_text
        .split('.')
        .next()
        .unwrap_or_default()
        .parse::<i64>()?;
    if parts.next().is_some()
        || !(1..=12).contains(&month)
        || hour > 23
        || minute > 59
        || second > 59
        || day < 1
    {
        return Err("RFC3339 date/time fields are outside their valid range".into());
    }
    let month_days = match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day > month_days {
        return Err("RFC3339 day is outside its month".into());
    }
    let zone = &value[zone_start..];
    let offset_seconds = if zone == "Z" {
        0
    } else {
        if zone.len() != 6 || zone.as_bytes()[3] != b':' {
            return Err("RFC3339 timezone offset is malformed".into());
        }
        let sign = if zone.starts_with('+') {
            1
        } else if zone.starts_with('-') {
            -1
        } else {
            return Err("RFC3339 timezone offset is malformed".into());
        };
        let hours = zone[1..3].parse::<i64>()?;
        let minutes = zone[4..6].parse::<i64>()?;
        if hours > 23 || minutes > 59 {
            return Err("RFC3339 timezone offset is outside its valid range".into());
        }
        sign * (hours * 3600 + minutes * 60)
    };
    let epoch = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second
        - offset_seconds;
    let days = epoch.div_euclid(86_400);
    let seconds = epoch.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    ))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn published_release_for_state(
    transport: &mut dyn Transport,
    context: &Context,
    state: &ReleaseState,
) -> Result<Value> {
    if state.phase != Phase::PublishedPendingMetadata && state.phase != Phase::Complete {
        return Err("this operation requires immutable publication".into());
    }
    let id = state
        .release_id
        .ok_or("persisted published state has no release id")?;
    let release = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/releases/{id}"),
        None,
    )?;
    validate_published_release(&release, context, id)?;
    Ok(release)
}

fn promote_metadata(root: &Path, options: &Options, transport: &mut dyn Transport) -> Result<()> {
    let context = read_context(root, options)?;
    let mut state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    validate_state(&state, &context)?;
    let manifest = read_manifest(root, options, &context)?;
    let published = published_release_for_state(transport, &context, &state)?;
    verify_public_assets(transport, &published, &manifest.public_assets, false)?;

    let signature_name = format!(
        "{}.sig",
        release_metadata::canonical_installer_name(&context.version)
    );
    let signature_record = manifest
        .qualification_assets
        .iter()
        .find(|record| record.source_name == signature_name)
        .ok_or("candidate manifest lacks canonical updater signature")?;
    let signature_path = safe_state_asset_path(&options.state_root, signature_record)?;
    let notes_path = root.join(&context.release_notes_path);
    let output_path = options.state_root.join("latest.json");
    let asset_url = release_metadata::canonical_asset_url(&context.version);
    let published_at = canonical_timestamp(
        published["published_at"]
            .as_str()
            .ok_or("published release timestamp missing")?,
    )?;
    release_metadata::generate(release_metadata::GenerateInput {
        channel: release_metadata::Channel::parse(&context.channel)?,
        version: &context.version,
        notes_path: &notes_path,
        pub_date: &published_at,
        platform: release_metadata::WINDOWS_PLATFORM,
        asset_url: &asset_url,
        signature_path: &signature_path,
        output: &output_path,
    })?;
    release_metadata::validate(
        release_metadata::Channel::parse(&context.channel)?,
        &output_path,
    )?;

    let endpoint = format!(
        "repos/{REPOSITORY}/contents/channels/{}/latest.json?ref=release-metadata",
        context.channel
    );
    let existing = transport.request("GET", &endpoint, None)?;
    let current_sha = if let Some(existing) = existing.as_ref() {
        let bytes = decode_contents(existing)?;
        let current = options
            .state_root
            .join(format!("current-{}-latest.json", context.channel));
        fs::write(&current, bytes)?;
        release_metadata::validate(
            release_metadata::Channel::parse(&context.channel)?,
            &current,
        )?;
        if fs::read(&current)? == fs::read(&output_path)? {
            state.promoted_at.get_or_insert_with(utc_now);
            write_json_atomic(&options.state_root.join("release-state.json"), &state)?;
            println!(
                "V4 Rust PromoteMetadata: PASS (channel={} already contains exact candidate bytes)",
                context.channel
            );
            return Ok(());
        }
        release_metadata::validate_monotonic(
            release_metadata::Channel::parse(&context.channel)?,
            &current,
            &output_path,
        )?;
        Some(
            existing["sha"]
                .as_str()
                .ok_or("metadata response omitted blob SHA")?
                .to_owned(),
        )
    } else {
        None
    };
    let content = base64::engine::general_purpose::STANDARD.encode(fs::read(&output_path)?);
    let mut payload = json!({"message":format!("promote {} metadata for {}", context.channel, context.tag),"content":content,"branch":"release-metadata"});
    if let Some(sha) = current_sha {
        payload["sha"] = Value::String(sha);
    }
    let put_endpoint = format!(
        "repos/{REPOSITORY}/contents/channels/{}/latest.json",
        context.channel
    );
    required_api(transport, "PUT", &put_endpoint, Some(&payload))?;
    let stored = required_api(transport, "GET", &endpoint, None)?;
    if decode_contents(&stored)? != fs::read(&output_path)? {
        return Err("release-metadata Contents API readback differs from candidate bytes".into());
    }
    state.promoted_at = Some(utc_now());
    write_json_atomic(&options.state_root.join("release-state.json"), &state)?;
    println!(
        "V4 Rust PromoteMetadata: PASS (channel={}, tag={})",
        context.channel, context.tag
    );
    Ok(())
}

fn final_verify(root: &Path, options: &Options, transport: &mut dyn Transport) -> Result<()> {
    let context = read_context(root, options)?;
    let mut state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    validate_state(&state, &context)?;
    let manifest = read_manifest(root, options, &context)?;
    let id = state
        .release_id
        .ok_or("final verification requires exact release id")?;
    let release = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/releases/{id}"),
        None,
    )?;
    validate_published_release(&release, &context, id)?;
    verify_public_assets(transport, &release, &manifest.public_assets, true)?;

    let signature_name = format!(
        "{}.sig",
        release_metadata::canonical_installer_name(&context.version)
    );
    let signature = manifest
        .qualification_assets
        .iter()
        .find(|record| record.source_name == signature_name)
        .ok_or("candidate manifest lacks canonical updater signature")?;
    let expected_path = options.state_root.join("final-expected-metadata.json");
    let expected_bytes =
        generate_expected_metadata(root, options, &context, &release, signature, &expected_path)?;
    let endpoint = format!(
        "repos/{REPOSITORY}/contents/channels/{}/latest.json?ref=release-metadata",
        context.channel
    );
    let stored = required_api(transport, "GET", &endpoint, None)?;
    let stored_bytes = decode_contents(&stored)?;
    if stored_bytes != expected_bytes {
        return Err(
            "release-metadata branch bytes differ from independently generated candidate".into(),
        );
    }
    let stored_path = options.state_root.join("final-metadata.json");
    fs::write(&stored_path, &stored_bytes)?;
    release_metadata::validate(
        release_metadata::Channel::parse(&context.channel)?,
        &stored_path,
    )?;
    let raw_url = format!(
        "https://raw.githubusercontent.com/{REPOSITORY}/release-metadata/channels/{}/latest.json",
        context.channel
    );
    let raw_bytes = transport
        .raw_get(&raw_url)?
        .ok_or("public metadata raw endpoint returned 404 after promotion")?;
    if raw_bytes != expected_bytes {
        return Err("unauthenticated raw metadata endpoint does not serve the exact qualified metadata bytes".into());
    }

    let latest = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/releases/latest"),
        None,
    )?;
    let before: Value = read_json(&options.state_root.join("preflight-evidence.json"))?;
    if context.channel == "stable" {
        if latest["id"].as_u64() != Some(id)
            || latest["tag_name"] != context.tag
            || latest["target_commitish"]
                .as_str()
                .unwrap_or_default()
                .to_ascii_lowercase()
                != context.source_sha
        {
            return Err("stable release is not the exact GitHub Latest release".into());
        }
    } else {
        validate_beta_keeps_latest(&before["pre_publication_latest"], &latest, id, &context.tag)?;
    }
    state.phase = Phase::Complete;
    state.final_verified = true;
    write_json_atomic(&options.state_root.join("release-state.json"), &state)?;
    println!(
        "V4 Rust FinalVerify: PASS (tag={} channel={} raw endpoint verified)",
        context.tag, context.channel
    );
    Ok(())
}

fn generate_expected_metadata(
    root: &Path,
    options: &Options,
    context: &Context,
    release: &Value,
    signature: &CandidateRecord,
    output: &Path,
) -> Result<Vec<u8>> {
    release_metadata::generate(release_metadata::GenerateInput {
        channel: release_metadata::Channel::parse(&context.channel)?,
        version: &context.version,
        notes_path: &root.join(&context.release_notes_path),
        pub_date: &canonical_timestamp(
            release["published_at"]
                .as_str()
                .ok_or("published_at missing")?,
        )?,
        platform: release_metadata::WINDOWS_PLATFORM,
        asset_url: &release_metadata::canonical_asset_url(&context.version),
        signature_path: &safe_state_asset_path(&options.state_root, signature)?,
        output,
    })?;
    Ok(fs::read(output)?)
}

fn probe_metadata_token(transport: &mut dyn Transport) -> Result<()> {
    let branch = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/git/ref/heads/release-metadata"),
        None,
    )?;
    if branch["ref"] != "refs/heads/release-metadata"
        || branch["object"]["sha"].as_str().unwrap_or_default().len() != 40
    {
        return Err("metadata App token did not read the expected release-metadata branch".into());
    }
    let bootstrap = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/contents/{BOOTSTRAP_PATH}?ref=release-metadata"),
        None,
    )?;
    if String::from_utf8(decode_contents(&bootstrap)?)?.trim() != BOOTSTRAP_CONTENT {
        return Err("metadata App token read the wrong repository or bootstrap state".into());
    }
    println!("V4 Rust metadata App token probe: PASS (canonical repository and branch readable)");
    Ok(())
}

fn cleanup_local_state(root: &Path, options: &Options) -> Result<()> {
    if !options.state_root.exists() {
        return Ok(());
    }
    let marker_path = options.state_root.join(".release-owner.json");
    if !marker_path.exists() {
        println!("V4 Rust runner-local state cleanup: skipped (no Rust ownership marker)");
        return Ok(());
    }
    let context: Context = read_json(&options.state_root.join("release-context.json"))?;
    let marker: Value = read_json(&marker_path)?;
    if context.repository != REPOSITORY
        || marker["repository"] != REPOSITORY
        || marker["run_id"] != context.run_id
        || marker["source_sha"] != context.source_sha
        || options.state_root == root
        || options.state_root.starts_with(root)
    {
        return Err(
            "release state cleanup refused because ownership identity did not match".into(),
        );
    }
    let temp = env::var_os("RUNNER_TEMP")
        .ok_or("release state cleanup requires RUNNER_TEMP ownership boundary")?;
    let temp = fs::canonicalize(PathBuf::from(temp))?;
    let state = fs::canonicalize(&options.state_root)?;
    if state == temp || !state.starts_with(temp) {
        return Err(
            "release state cleanup refused outside a dedicated RUNNER_TEMP child directory".into(),
        );
    }
    fs::remove_dir_all(&options.state_root)?;
    println!("V4 Rust runner-local state cleanup: PASS");
    Ok(())
}

fn doctor(root: &Path, args: &[String]) -> Result<()> {
    let mut transport = GhTransport::new();
    let (report, format) = doctor_report(root, args, &mut transport)?;
    print_doctor_report(&report, &format)
}

fn doctor_report(
    root: &Path,
    args: &[String],
    transport: &mut dyn Transport,
) -> Result<(Value, String)> {
    let mut tag = None;
    let mut version_text = None;
    let mut channel = None;
    let mut state_root = None;
    let mut state_path = None;
    let mut run_id = None;
    let mut workflow_sha = None;
    let mut format = "text".to_owned();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--tag" => tag = Some(next_value(args, &mut i, "--tag")?),
            "--version" => version_text = Some(next_value(args, &mut i, "--version")?),
            "--channel" => channel = Some(next_value(args, &mut i, "--channel")?),
            "--state-root" => {
                state_root = Some(PathBuf::from(next_value(args, &mut i, "--state-root")?))
            }
            "--state-path" => {
                state_path = Some(PathBuf::from(next_value(args, &mut i, "--state-path")?))
            }
            "--run-id" => run_id = Some(next_value(args, &mut i, "--run-id")?),
            "--workflow-sha" => workflow_sha = Some(next_value(args, &mut i, "--workflow-sha")?),
            "--format" => format = next_value(args, &mut i, "--format")?.to_ascii_lowercase(),
            option => return Err(format!("unknown release-doctor option: {option}").into()),
        }
        i += 1;
    }
    if !matches!(format.as_str(), "text" | "json") {
        return Err("release-doctor --format must be text or json".into());
    }
    let version_value = version_text
        .or_else(|| {
            tag.as_deref()
                .map(|value| value.strip_prefix('v').unwrap_or(value).to_owned())
        })
        .unwrap_or(repo::project_version(root)?);
    let tag_value = tag.unwrap_or_else(|| format!("v{version_value}"));
    let parsed = version::parse(&version_value)?;
    let channel_value = channel.unwrap_or_else(|| {
        if parsed.pre.is_empty() {
            "stable".into()
        } else {
            "beta".into()
        }
    });
    version::validate_channel(&channel_value, &version_value, &parsed)?;
    if tag_value != format!("v{version_value}") {
        return Err("release-doctor tag and version do not match".into());
    }
    if run_id
        .as_deref()
        .is_some_and(|value| value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err("release-doctor --run-id must contain decimal digits only".into());
    }
    if workflow_sha.as_deref().is_some_and(|value| {
        value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err(
            "release-doctor --workflow-sha must be an exact 40-character commit SHA".into(),
        );
    }
    let repository = REPOSITORY;
    let mut local_path = state_path;
    if local_path.is_none() {
        local_path = state_root.map(|path| path.join("release-state.json"));
    }
    let local_state = if let Some(path) = local_path.as_deref().filter(|path| path.exists()) {
        let metadata = fs::metadata(path)?;
        if metadata.len() > 16 * 1024 * 1024 {
            return Err("release-doctor local state exceeds the 16 MiB bound".into());
        }
        let bytes = fs::read(path)?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "release-doctor local state JSON is malformed")?;
        if !value.is_object() {
            return Err("release-doctor local state must be a JSON object".into());
        }
        Some(value)
    } else {
        None
    };
    let state_file_found = local_path.as_deref().is_some_and(Path::exists);
    let mut target_sha = workflow_sha
        .clone()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            local_state
                .as_ref()
                .and_then(|state| state["source_sha"].as_str())
                .map(str::to_owned)
        });
    if let Some(local) = local_state.as_ref() {
        for (field, expected) in [
            ("tag", Some(tag_value.as_str())),
            ("version", Some(version_value.as_str())),
            ("channel", Some(channel_value.as_str())),
            ("repository", Some(repository)),
        ] {
            if local
                .get(field)
                .is_some_and(|actual| actual.as_str() != expected)
            {
                return Err(
                    format!("release-doctor local state identity mismatch: {field}").into(),
                );
            }
        }
        if local
            .get("source_sha")
            .and_then(Value::as_str)
            .is_some_and(|sha| sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return Err("release-doctor local state source SHA is malformed".into());
        }
        if let (Some(expected), Some(actual)) = (
            workflow_sha.as_deref(),
            local.get("source_sha").and_then(Value::as_str),
        ) && !actual.eq_ignore_ascii_case(expected)
        {
            return Err("release-doctor local state source SHA differs from --workflow-sha".into());
        }
        if local
            .get("run_id")
            .and_then(Value::as_str)
            .is_some_and(|value| {
                value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return Err("release-doctor local state run ID is malformed".into());
        }
        if local
            .get("schema_version")
            .and_then(Value::as_u64)
            .is_some_and(|schema| schema != 1)
        {
            return Err("release-doctor local state schema version is unsupported".into());
        }
        if local
            .get("phase")
            .and_then(Value::as_str)
            .is_some_and(|phase| {
                !matches!(
                    phase,
                    "READY" | "QUALIFIED" | "PUBLISHED_PENDING_METADATA" | "COMPLETE"
                )
            })
        {
            return Err("release-doctor local state phase is unsupported".into());
        }
        for field in [
            "published_at",
            "promoted_at",
            "final_verified_at",
            "last_reconciled_at",
        ] {
            if let Some(timestamp) = local.get(field).and_then(Value::as_str) {
                canonical_timestamp(timestamp).map_err(|_| {
                    format!("release-doctor local state timestamp is malformed: {field}")
                })?;
            }
        }
    }
    let mut remote_findings = Vec::new();
    let external_release = match transport.request(
        "GET",
        &format!("repos/{repository}/releases/tags/{tag_value}"),
        None,
    ) {
        Ok(value) => value,
        Err(_) => {
            remote_findings.push("GitHub release endpoint could not be read".to_owned());
            None
        }
    };
    if let Some(release) = external_release.as_ref()
        && (release["id"].as_u64().is_none_or(|id| id == 0)
            || release["tag_name"] != tag_value
            || release["draft"].as_bool().is_none()
            || release["target_commitish"].as_str().is_none_or(|sha| {
                sha.len() != 40 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            || release["prerelease"].as_bool() != Some(channel_value == "beta")
            || !release["assets"].is_array())
    {
        remote_findings.push(
            "GitHub release response does not match the requested release identity".to_owned(),
        );
    }
    if target_sha.is_none() {
        target_sha = external_release
            .as_ref()
            .and_then(|release| release["target_commitish"].as_str())
            .map(str::to_owned);
    }
    let tag_ref = match transport.request(
        "GET",
        &format!("repos/{repository}/git/ref/tags/{tag_value}"),
        None,
    ) {
        Ok(value) => value,
        Err(_) => {
            remote_findings.push("GitHub tag endpoint could not be read".to_owned());
            None
        }
    };
    let latest =
        match transport.request("GET", &format!("repos/{repository}/releases/latest"), None) {
            Ok(value) => value,
            Err(_) => {
                remote_findings.push("GitHub Latest endpoint could not be read".to_owned());
                None
            }
        };
    if latest.as_ref().is_some_and(|release| {
        release["id"].as_u64().is_none_or(|id| id == 0)
            || release["tag_name"].as_str().is_none_or(str::is_empty)
            || release["draft"] != false
    }) {
        remote_findings.push("GitHub Latest response is malformed".to_owned());
    }
    let metadata_endpoint = format!(
        "repos/{repository}/contents/channels/{channel_value}/latest.json?ref=release-metadata"
    );
    let metadata_contents = match transport.request("GET", &metadata_endpoint, None) {
        Ok(value) => value,
        Err(_) => {
            remote_findings.push("release-metadata Contents endpoint could not be read".to_owned());
            None
        }
    };
    let mut channel_metadata_bytes = None;
    let channel_metadata = match metadata_contents.as_ref() {
        Some(value) => match decode_contents(value) {
            Ok(bytes) => match doctor_validate_metadata(&bytes, &channel_value) {
                Ok(()) => match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value) if value.is_object() => {
                        channel_metadata_bytes = Some(bytes);
                        Some(value)
                    }
                    _ => {
                        remote_findings
                            .push("release-metadata Contents payload is malformed".to_owned());
                        None
                    }
                },
                Err(_) => {
                    remote_findings
                        .push("release-metadata Contents payload is malformed".to_owned());
                    None
                }
            },
            Err(_) => {
                remote_findings.push("release-metadata Contents payload is malformed".to_owned());
                None
            }
        },
        None => None,
    };
    let raw_url = format!(
        "https://raw.githubusercontent.com/{repository}/release-metadata/channels/{channel_value}/latest.json"
    );
    let mut raw_metadata_bytes = None;
    let raw_metadata = match transport.raw_get(&raw_url) {
        Ok(Some(bytes)) => match doctor_validate_metadata(&bytes, &channel_value) {
            Ok(()) => match serde_json::from_slice::<Value>(&bytes) {
                Ok(value) if value.is_object() => {
                    raw_metadata_bytes = Some(bytes);
                    Some(value)
                }
                _ => {
                    remote_findings.push("public raw metadata payload is malformed".to_owned());
                    None
                }
            },
            Err(_) => {
                remote_findings.push("public raw metadata payload is malformed".to_owned());
                None
            }
        },
        Ok(None) => None,
        Err(_) => {
            remote_findings.push("public raw metadata endpoint could not be read".to_owned());
            None
        }
    };
    let metadata_bytes_match = match (channel_metadata_bytes.as_ref(), raw_metadata_bytes.as_ref())
    {
        (Some(contents), Some(raw)) => contents == raw,
        (None, None) => true,
        _ => false,
    };

    let mut workflow_run = None;
    let mut ambiguous = false;
    if let Some(id) = run_id.as_deref().filter(|value| !value.trim().is_empty()) {
        let value = match transport.request(
            "GET",
            &format!("repos/{repository}/actions/runs/{id}"),
            None,
        ) {
            Ok(value) => value,
            Err(_) => {
                remote_findings.push("specified GitHub Actions run could not be read".to_owned());
                None
            }
        };
        if value.is_none() {
            return Err(
                format!("specified workflow run ID {id} was not found in {repository}").into(),
            );
        }
        workflow_run = value.map(|value| {
            if value["workflow_runs"].is_array() {
                value["workflow_runs"][0].clone()
            } else {
                value
            }
        });
        if workflow_run
            .as_ref()
            .is_some_and(|run| !valid_workflow_run(run, target_sha.as_deref(), repository))
        {
            return Err(format!(
                "specified workflow run {id} does not match release workflow identity"
            )
            .into());
        }
    } else if let Some(sha) = target_sha.as_deref() {
        let response = match transport.request("GET", &format!("repos/{repository}/actions/workflows/release-v4.yml/runs?head_sha={sha}&per_page=10"), None) {
            Ok(value) => value,
            Err(_) => {
                remote_findings.push("release workflow runs endpoint could not be read".to_owned());
                None
            }
        };
        let runs = response
            .and_then(|value| value["workflow_runs"].as_array().cloned())
            .unwrap_or_default();
        let valid = runs
            .into_iter()
            .filter(|run| valid_workflow_run(run, Some(sha), repository))
            .collect::<Vec<_>>();
        match valid.as_slice() {
            [only] => workflow_run = Some(only.clone()),
            [] => {}
            _ => ambiguous = true,
        }
    }

    let release_exists = external_release.is_some();
    let is_draft = external_release
        .as_ref()
        .is_some_and(|release| release["draft"] == true);
    let raw_published_at = external_release
        .as_ref()
        .and_then(|release| release["published_at"].as_str())
        .unwrap_or_default();
    let published_at = canonical_timestamp(raw_published_at).ok();
    let malformed_published_at =
        release_exists && !is_draft && !raw_published_at.is_empty() && published_at.is_none();
    let published =
        release_exists && !is_draft && (published_at.is_some() || malformed_published_at);
    let immutable = external_release
        .as_ref()
        .is_some_and(|release| release["immutable"] == true);
    let target_commitish = external_release
        .as_ref()
        .and_then(|release| release["target_commitish"].as_str())
        .unwrap_or_default();
    let latest_tag = latest
        .as_ref()
        .and_then(|value| value["tag_name"].as_str())
        .unwrap_or_default();
    let is_latest = latest_tag == tag_value;
    let metadata_version = channel_metadata
        .as_ref()
        .and_then(|value| value["version"].as_str())
        .unwrap_or_default();
    let raw_version = raw_metadata
        .as_ref()
        .and_then(|value| value["version"].as_str())
        .unwrap_or_default();
    let public_assets = external_release
        .as_ref()
        .and_then(|value| value["assets"].as_array())
        .map(|assets| {
            assets
                .iter()
                .map(|asset| asset["name"].as_str().unwrap_or_default().to_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let expected_installer = release_metadata::canonical_release_installer_name(&version_value);
    let expected_assets = [
        expected_installer.clone(),
        format!("{expected_installer}.sig"),
    ];
    let exact_assets = public_assets.len() == 2
        && expected_assets
            .iter()
            .all(|name| public_assets.contains(name));

    let workflow_status = workflow_run
        .as_ref()
        .and_then(|run| run["status"].as_str())
        .map(str::to_owned);
    let workflow_conclusion = workflow_run
        .as_ref()
        .and_then(|run| run["conclusion"].as_str())
        .map(str::to_owned);
    let workflow_outcome = match workflow_status.as_deref() {
        Some("completed") => match workflow_conclusion.as_deref() {
            Some("failure") => "failure",
            Some("success") => "success",
            Some(other) => other,
            None => "unknown",
        },
        Some("in_progress" | "queued" | "waiting" | "requested" | "pending") => "in_progress",
        _ => "unknown",
    };
    let persisted_phase = local_state
        .as_ref()
        .and_then(|state| state["phase"].as_str())
        .map(str::to_owned);
    let local_attested = local_state
        .as_ref()
        .is_some_and(|state| state["attested"] == true);
    let local_qualified = local_state
        .as_ref()
        .is_some_and(|state| state["qualified_after_download"] == true);
    let mut external_phase = Value::Null;
    let mut classification = Value::Null;
    let mut operator_review_required = false;
    let mut findings: Vec<String> = Vec::new();
    findings.extend(remote_findings);
    let remote_unknown = !findings.is_empty();
    let assessment;
    let recovery_guidance;
    if remote_unknown {
        classification = json!("REMOTE_STATE_UNKNOWN");
        operator_review_required = true;
        assessment = format!("Release {tag_value} external state could not be fully reconciled.");
        recovery_guidance = "Do not repeat publication or metadata promotion until read-only external state is available and reconciled.".to_owned();
    } else if published {
        let complete = metadata_version == version_value
            && raw_version == version_value
            && metadata_bytes_match
            && (channel_value != "stable" || is_latest)
            && exact_assets
            && immutable
            && !malformed_published_at;
        if complete {
            external_phase = json!("COMPLETE");
            assessment =
                format!("Release {tag_value} is published, immutable, current, and verified.");
            recovery_guidance =
                "Release transaction completed successfully. No recovery action needed.".to_owned();
        } else {
            external_phase = json!("PUBLISHED_PENDING_METADATA");
            if malformed_published_at {
                findings.push(format!(
                    "External published_at is invalid: {raw_published_at}"
                ));
            }
            if workflow_outcome == "failure"
                || malformed_published_at
                || !immutable
                || !exact_assets
                || metadata_version != version_value
                || raw_version != version_value
                || !metadata_bytes_match
                || (channel_value == "stable" && !is_latest)
            {
                classification = json!("POST_PUBLICATION_INCIDENT");
                operator_review_required = true;
                if !metadata_version.eq(&version_value) {
                    findings.push(format!("Channel metadata version is '{metadata_version}', expected '{version_value}'"));
                }
                if !raw_version.eq(&version_value) {
                    findings.push(format!(
                        "Raw metadata version is '{raw_version}', expected '{version_value}'"
                    ));
                }
                if !metadata_bytes_match {
                    findings.push(
                        "Contents API metadata differs from the public raw endpoint".to_owned(),
                    );
                }
                if !exact_assets {
                    findings.push(
                        "Public release assets differ from the exact canonical pair".to_owned(),
                    );
                }
                if !immutable {
                    findings.push("GitHub release is not immutable".to_owned());
                }
                assessment = format!(
                    "Release {tag_value} was published, but post-publication verification found: {}.",
                    findings.join("; ")
                );
                recovery_guidance = "Published tags and releases are immutable. Do not delete, edit, replace, or rerun this release. Use a separately reviewed corrective release with a greater SemVer version.".to_owned();
            } else if workflow_outcome == "in_progress" {
                assessment =
                    format!("Release {tag_value} is published and its workflow is in progress.");
                recovery_guidance =
                    "Await workflow completion. Do not interrupt, cancel, or rerun.".to_owned();
            } else {
                operator_review_required = true;
                assessment = if ambiguous {
                    "Multiple matching release workflow runs exist; explicit --run-id is required."
                        .to_owned()
                } else {
                    "Workflow state is unknown while channel metadata is not promoted.".to_owned()
                };
                recovery_guidance = "Check the exact release workflow run. Published tags and releases must not be mutated.".to_owned();
            }
        }
    } else if is_draft {
        external_phase = json!("QUALIFIED");
        if local_attested && local_qualified {
            assessment =
                format!("Draft release {tag_value} exists and candidate bytes are qualified.");
            recovery_guidance = "Ready for PublishRelease.".to_owned();
        } else {
            classification = json!("RECOVERABLE_PRE_PUBLICATION_FAILURE");
            assessment = format!(
                "Draft release {tag_value} exists without complete local qualification evidence."
            );
            recovery_guidance = "No immutable release has been published. A matching unpublished draft may be discarded or recreated by a fresh dispatch.".to_owned();
        }
    } else if tag_ref.is_some() {
        classification = json!("POST_PUBLICATION_INCIDENT");
        operator_review_required = true;
        assessment = format!("Git tag {tag_value} exists without an associated GitHub release.");
        recovery_guidance =
            "Investigate under the release authority rules. Do not delete the tag.".to_owned();
    } else if !metadata_bytes_match {
        classification = json!("REMOTE_STATE_UNKNOWN");
        operator_review_required = true;
        findings.push("Contents API metadata differs from the public raw endpoint".to_owned());
        assessment =
            "Release metadata endpoints diverge and require operator reconciliation.".to_owned();
        recovery_guidance = "Do not promote metadata until both read-only metadata endpoints serve the same validated bytes.".to_owned();
    } else if persisted_phase.as_deref() == Some("READY") {
        assessment =
            "Candidate build is ready locally; draft release has not yet been created on GitHub."
                .to_owned();
        recovery_guidance = "Ready for PublishRelease.".to_owned();
    } else {
        classification = json!("NOT_READY");
        assessment = "Release request has not yet reached candidate build stage.".to_owned();
        recovery_guidance =
            "Dispatch the release pipeline from the canonical repository main branch.".to_owned();
    }
    let local_truth = if let Some(local) = local_state.as_ref() {
        json!({"state_file_found":true,"schema_version":local.get("schema_version"),"phase":local.get("phase"),
            "attested":local.get("attested"),"qualified_after_download":local.get("qualified_after_download"),
            "published":local.get("published"),"immutable":local.get("immutable"),
            "published_at":local.get("published_at").and_then(Value::as_str).and_then(|value| canonical_timestamp(value).ok()),
            "promoted_at":local.get("promoted_at").and_then(Value::as_str).and_then(|value| canonical_timestamp(value).ok()),
            "final_verified":local.get("final_verified"),
            "final_verified_at":local.get("final_verified_at").and_then(Value::as_str).and_then(|value| canonical_timestamp(value).ok()),
            "reconciled_from_remote":local.get("reconciled_from_remote"),"failure_class":local.get("failure_class"),
            "error_message":local.get("error_message"),
            "last_reconciled_at":local.get("last_reconciled_at").and_then(Value::as_str).and_then(|value| canonical_timestamp(value).ok())})
    } else {
        json!({"state_file_found":state_file_found})
    };
    let report = json!({
        "tag":tag_value,"version":version_value,"channel":channel_value,"repository":repository,
        "persisted_phase":persisted_phase,"external_phase":external_phase,"classification":classification,
        "operator_review_required":operator_review_required,"workflow_outcome":workflow_outcome,
        "workflow_run_id":workflow_run.as_ref().and_then(|run| run["id"].as_u64()).map(|id| id.to_string()),
        "external_truth":{"release_exists":release_exists,"status":if published { "PUBLISHED" } else if is_draft { "DRAFT" } else { "MISSING" },
            "immutable":immutable,"published_at":published_at,"target_commitish":target_commitish,"github_latest_tag":latest_tag,
            "is_latest":is_latest,"public_assets":public_assets,"metadata_bytes_match":metadata_bytes_match,"channel_metadata_version":metadata_version,
            "public_raw_metadata_version":raw_version,"workflow_run_status":workflow_status,
            "workflow_run_conclusion":workflow_conclusion,"workflow_run_url":workflow_run.as_ref().and_then(|run| run["html_url"].as_str())},
        "local_truth":local_truth,"findings":findings,"assessment":assessment,"recovery_guidance":recovery_guidance
    });
    Ok((report, format))
}

fn print_doctor_report(report: &Value, format: &str) -> Result<()> {
    if format == "json" {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }
    let truth = &report["external_truth"];
    let assets = truth["public_assets"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "NONE".to_owned());
    println!("=================================================================");
    println!(" Sky Auto Player V4 - Release Diagnostic Doctor (Read-Only)");
    println!("=================================================================");
    println!(
        "Persisted Phase:          {}",
        report["persisted_phase"].as_str().unwrap_or("NONE")
    );
    println!(
        "External Phase:           {}",
        report["external_phase"].as_str().unwrap_or("NONE")
    );
    println!(
        "Classification:           {}",
        report["classification"].as_str().unwrap_or("NONE")
    );
    println!(
        "Workflow Outcome:         {}",
        report["workflow_outcome"].as_str().unwrap_or("unknown")
    );
    println!(
        "Operator Review Required: {}",
        report["operator_review_required"].as_bool().unwrap_or(true)
    );
    println!();
    println!("External Truth:");
    println!(
        "  Release Status:    {}",
        truth["status"].as_str().unwrap_or("MISSING")
    );
    println!(
        "  Immutable:         {}",
        truth["immutable"].as_bool().unwrap_or(false)
    );
    println!(
        "  Published At:      {}",
        truth["published_at"].as_str().unwrap_or("NONE")
    );
    println!(
        "  Target Commit:     {}",
        truth["target_commitish"]
            .as_str()
            .filter(|value| !value.is_empty())
            .unwrap_or("NONE")
    );
    println!(
        "  GitHub Latest:     {}",
        truth["github_latest_tag"]
            .as_str()
            .filter(|value| !value.is_empty())
            .unwrap_or("NONE")
    );
    println!("  Public Assets:     {assets}");
    println!(
        "  Channel Metadata:  {}",
        truth["channel_metadata_version"]
            .as_str()
            .filter(|value| !value.is_empty())
            .unwrap_or("NONE")
    );
    println!(
        "  Raw Metadata:      {}",
        truth["public_raw_metadata_version"]
            .as_str()
            .filter(|value| !value.is_empty())
            .unwrap_or("NONE")
    );
    let workflow_run_id = report["workflow_run_id"].as_str().unwrap_or("NONE");
    let workflow_status = truth["workflow_run_status"].as_str().unwrap_or("unknown");
    let workflow_conclusion = truth["workflow_run_conclusion"]
        .as_str()
        .unwrap_or("unknown");
    println!(
        "  Workflow Run:      {workflow_run_id} ({workflow_status}, conclusion={workflow_conclusion})"
    );
    println!();
    println!("Assessment:");
    println!(
        "  {}",
        report["assessment"]
            .as_str()
            .unwrap_or("External state is unknown.")
    );
    println!();
    println!("Recovery Guidance:");
    println!(
        "{}",
        report["recovery_guidance"]
            .as_str()
            .unwrap_or("Review the release state.")
    );
    if let Some(findings) = report["findings"].as_array() {
        for finding in findings.iter().filter_map(Value::as_str) {
            println!("Finding: {finding}");
        }
    }
    println!("=================================================================");
    Ok(())
}

fn doctor_validate_metadata(bytes: &[u8], channel: &str) -> Result<()> {
    let path = unique_temp(&env::temp_dir(), "sky-release-doctor-metadata");
    let validation = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        use std::io::Write;
        file.write_all(bytes)?;
        drop(file);
        release_metadata::validate(release_metadata::Channel::parse(channel)?, &path)
    })();
    let cleanup = if path.exists() {
        fs::remove_file(&path).map_err(|error| error.to_string())
    } else {
        Ok(())
    };
    match (validation, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => {
            Err(format!("metadata validation temporary-file cleanup failed: {error}").into())
        }
        (Err(error), Err(cleanup_error)) => Err(format!(
            "{error}; metadata validation temporary-file cleanup failed: {cleanup_error}"
        )
        .into()),
    }
}

fn valid_workflow_run(run: &Value, sha: Option<&str>, repository: &str) -> bool {
    let path = run["path"]
        .as_str()
        .or_else(|| run["workflow_url"].as_str())
        .unwrap_or_default()
        .replace('\\', "/");
    let path = path.split_once('@').map_or(path.as_str(), |(path, _)| path);
    let run_repo = run["repository"]
        .as_str()
        .or_else(|| run["repository"]["full_name"].as_str())
        .unwrap_or_default();
    path == ".github/workflows/release-v4.yml"
        && run["event"] == "workflow_dispatch"
        && run_repo.eq_ignore_ascii_case(repository)
        && sha.is_none_or(|expected| {
            run["head_sha"]
                .as_str()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
        })
}

fn self_test() -> Result<()> {
    let outcomes = [
        mock_publish_succeeds(),
        mock_draft_post_timeout_reconciles_exact_marker(),
        mock_complete_transaction_and_raw_metadata(),
        mock_metadata_promotion_is_idempotent_after_committed_timeout(),
        mock_publication_identity_and_asset_inventory_fail_closed(),
        mock_matching_stale_draft_is_reconciled(),
        mock_conflicting_draft_is_not_deleted(),
        mock_digest_mismatch_cleans_owned_draft(),
        mock_partial_upload_cleans_owned_draft(),
        mock_cleanup_failure_preserves_original_error(),
        mock_patch_timeout_reconciles_exact_release_id(),
        mock_unknown_publication_never_cleans_up(),
        mock_latest_policy_failure_is_post_publication(),
        metadata_token_probe_rejects_wrong_repository_state(),
        mock_stale_evidence_identity_fails_closed(),
        mock_manifest_path_traversal_fails_closed(),
        mock_release_doctor_detects_metadata_endpoint_divergence(),
        doctor_state_input_rejects_malformed_and_oversized_files(),
        retry_policy_retries_only_transient_responses(),
        marker_parser_rejects_duplicates_and_multiple_markers(),
        workflow_run_identity_fails_closed(),
        timestamp_normalization_is_utc_and_strict(),
    ];
    for outcome in outcomes {
        outcome?;
    }
    println!("V4 Rust release state-machine mock self-test: PASS (22 scenarios)");
    Ok(())
}

fn mock_publish_succeeds() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    let old_latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.latest = old_latest.clone();
    write_json_atomic(
        &options.state_root.join("preflight-evidence.json"),
        &json!({"pre_publication_latest":old_latest}),
    )?;
    write_json_atomic(&options.state_root.join("release-context.json"), &context)?;
    write_json_atomic(
        &options.state_root.join("release-state.json"),
        &fixture_state(&context),
    )?;
    write_json_atomic(
        &options.state_root.join("candidate-manifest.json"),
        &manifest,
    )?;
    publish_release(&root, &options, &mut github)?;
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert_eq!(state.phase, Phase::PublishedPendingMetadata);
    assert_eq!(state.release_id, Some(12));
    let post = github
        .events
        .iter()
        .position(|event| event == "PATCH repos/pumni/Sky-Auto-Player/releases/12")
        .ok_or("publish PATCH missing")?;
    let verify = github
        .events
        .iter()
        .rposition(|event| event == "GET repos/pumni/Sky-Auto-Player/releases/12")
        .ok_or("exact id reconciliation missing")?;
    assert!(
        post < verify,
        "exact release id must be read after irreversible publication"
    );
    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_draft_post_timeout_reconciles_exact_marker() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.post_created_then_timeout = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release(&root, &options, &mut github)?;
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert_eq!(state.phase, Phase::PublishedPendingMetadata);
    assert_eq!(state.release_id, Some(12));
    assert_eq!(
        github.releases.len(),
        1,
        "timeout reconciliation must not duplicate the draft"
    );
    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_complete_transaction_and_raw_metadata() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release(&root, &options, &mut github)?;
    promote_metadata(&root, &options, &mut github)?;
    final_verify(&root, &options, &mut github)?;
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert_eq!(state.phase, Phase::Complete);
    assert!(state.final_verified);
    assert!(github.metadata.starts_with(b"{\n"));
    assert!(
        github
            .events
            .iter()
            .any(|event| event.starts_with("PUT repos/pumni/Sky-Auto-Player/contents/channels/"))
    );
    let run_id = "9001";
    github.workflow_runs.insert(
        run_id.to_owned(),
        json!({"id":9001,"path":".github/workflows/release-v4.yml","event":"workflow_dispatch",
            "repository":REPOSITORY,"head_sha":context.source_sha,"status":"completed","conclusion":"success",
            "html_url":"https://github.com/pumni/Sky-Auto-Player/actions/runs/9001"}),
    );
    let event_count = github.events.len();
    let metadata_before = github.metadata_files.get("stable").cloned();
    let (report, _) = doctor_report(
        &root,
        &[
            "--tag".to_owned(),
            context.tag.clone(),
            "--version".to_owned(),
            context.version.clone(),
            "--channel".to_owned(),
            context.channel.clone(),
            "--workflow-sha".to_owned(),
            context.source_sha.clone(),
            "--run-id".to_owned(),
            run_id.to_owned(),
            "--state-path".to_owned(),
            options
                .state_root
                .join("release-state.json")
                .to_string_lossy()
                .into_owned(),
        ],
        &mut github,
    )?;
    assert_eq!(report["external_phase"], "COMPLETE");
    assert!(report["classification"].is_null());
    assert_eq!(report["external_truth"]["metadata_bytes_match"], true);
    assert_eq!(
        github.metadata_files.get("stable"),
        metadata_before.as_ref()
    );
    assert!(github.deletions.is_empty());
    assert!(
        github.events[event_count..]
            .iter()
            .all(|event| event.starts_with("GET ") || event.starts_with("RAW_GET "))
    );
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_metadata_promotion_is_idempotent_after_committed_timeout() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release(&root, &options, &mut github)?;
    github.fail_metadata_put_after_write = true;
    let error = promote_metadata(&root, &options, &mut github)
        .expect_err("simulated response loss after Contents commit must be visible");
    assert!(
        error
            .to_string()
            .contains("metadata write timeout after commit")
    );
    assert!(github.metadata_files.contains_key("stable"));
    assert_eq!(
        github
            .events
            .iter()
            .filter(|event| event.starts_with("PUT repos/pumni/Sky-Auto-Player/contents/channels/"))
            .count(),
        1
    );

    promote_metadata(&root, &options, &mut github)?;
    assert_eq!(
        github
            .events
            .iter()
            .filter(|event| event.starts_with("PUT repos/pumni/Sky-Auto-Player/contents/channels/"))
            .count(),
        1,
        "retry must reconcile the exact bytes without a second metadata write"
    );
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert!(state.promoted_at.is_some());
    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_publication_identity_and_asset_inventory_fail_closed() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release(&root, &options, &mut github)?;
    let published = github
        .releases
        .get(&12)
        .cloned()
        .ok_or("fake published release missing")?;
    verify_public_assets(&mut github, &published, &manifest.public_assets, false)?;
    validate_published_release(&published, &context, 12)?;

    let mut missing_asset = published.clone();
    missing_asset["assets"].as_array_mut().unwrap().pop();
    assert!(
        verify_public_assets(&mut github, &missing_asset, &manifest.public_assets, false).is_err()
    );

    let mut extra_asset = published.clone();
    extra_asset["assets"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"unexpected.bin"}));
    assert!(
        verify_public_assets(&mut github, &extra_asset, &manifest.public_assets, false).is_err()
    );

    let mut wrong_digest = published.clone();
    wrong_digest["assets"][0]["digest"] = json!("sha256:invalid");
    assert!(
        verify_public_assets(&mut github, &wrong_digest, &manifest.public_assets, false).is_err()
    );

    let mut wrong_tag = published.clone();
    wrong_tag["tag_name"] = json!("v4.99.0");
    assert!(validate_published_release(&wrong_tag, &context, 12).is_err());
    let mut wrong_source = published.clone();
    wrong_source["target_commitish"] = json!("c".repeat(40));
    assert!(validate_published_release(&wrong_source, &context, 12).is_err());
    let mut wrong_id = published.clone();
    wrong_id["id"] = json!(13);
    assert!(validate_published_release(&wrong_id, &context, 12).is_err());
    let mut mixed_state = published.clone();
    mixed_state["draft"] = json!(true);
    assert!(validate_published_release(&mixed_state, &context, 12).is_err());
    let mut wrong_channel = published.clone();
    wrong_channel["prerelease"] = json!(true);
    assert!(validate_published_release(&wrong_channel, &context, 12).is_err());
    let mut mutable_release = published;
    mutable_release["immutable"] = json!(false);
    assert!(validate_published_release(&mutable_release, &context, 12).is_err());

    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_release_doctor_detects_metadata_endpoint_divergence() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release(&root, &options, &mut github)?;
    promote_metadata(&root, &options, &mut github)?;
    final_verify(&root, &options, &mut github)?;
    github.workflow_runs.insert(
        "9001".to_owned(),
        json!({"id":9001,"path":".github/workflows/release-v4.yml","event":"workflow_dispatch",
            "repository":REPOSITORY,"head_sha":context.source_sha,"status":"completed","conclusion":"success"}),
    );
    let mut divergent = github
        .raw_metadata_files
        .get("stable")
        .cloned()
        .ok_or("fake raw metadata was not promoted")?;
    divergent.push(b' ');
    github
        .raw_metadata_files
        .insert("stable".to_owned(), divergent);
    let (report, _) = doctor_report(
        &root,
        &[
            "--tag".to_owned(),
            context.tag.clone(),
            "--version".to_owned(),
            context.version.clone(),
            "--channel".to_owned(),
            context.channel.clone(),
            "--workflow-sha".to_owned(),
            context.source_sha.clone(),
            "--run-id".to_owned(),
            "9001".to_owned(),
        ],
        &mut github,
    )?;
    assert_eq!(report["classification"], "POST_PUBLICATION_INCIDENT");
    assert_eq!(report["external_truth"]["metadata_bytes_match"], false);
    assert!(report["findings"].as_array().is_some_and(|items| {
        items
            .iter()
            .any(|item| item == "Contents API metadata differs from the public raw endpoint")
    }));
    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn doctor_state_input_rejects_malformed_and_oversized_files() -> Result<()> {
    let (_root, options, context, _) = fixture()?;
    let state_path = options.state_root.join("doctor-input.json");
    let args = vec![
        "--tag".to_owned(),
        context.tag.clone(),
        "--version".to_owned(),
        context.version.clone(),
        "--channel".to_owned(),
        context.channel.clone(),
        "--state-path".to_owned(),
        state_path.to_string_lossy().into_owned(),
    ];
    let mut github = FakeTransport::new();
    fs::write(&state_path, b"{ malformed")?;
    assert!(doctor_report(&repo::root(), &args, &mut github).is_err());
    assert!(
        github.events.is_empty(),
        "malformed local state must fail before remote reads"
    );

    fs::write(&state_path, vec![b' '; 16 * 1024 * 1024 + 1])?;
    assert!(doctor_report(&repo::root(), &args, &mut github).is_err());
    assert!(
        github.events.is_empty(),
        "oversized local state must fail before remote reads"
    );

    fs::write(&state_path, serde_json::to_vec(&json!({"tag":"v4.0.0"}))?)?;
    assert!(doctor_report(&repo::root(), &args, &mut github).is_err());
    assert!(
        github.events.is_empty(),
        "divergent local identity must fail before remote reads"
    );
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn retry_policy_retries_only_transient_responses() -> Result<()> {
    let mut attempts = 0;
    let mut delays = Vec::new();
    let result = retry_github(
        || {
            attempts += 1;
            if attempts < 3 {
                Err(Box::new(HttpFailure {
                    status: Some(if attempts == 1 { 429 } else { 503 }),
                    detail: "injected transient response".to_owned(),
                })
                    as Box<dyn std::error::Error + Send + Sync>)
            } else {
                Ok("ok")
            }
        },
        |delay| delays.push(delay),
    )?;
    assert_eq!(result, "ok");
    assert_eq!(attempts, 3);
    assert_eq!(delays, [Duration::from_millis(500), Duration::from_secs(1)]);

    for status in [403, 404] {
        let mut attempts = 0;
        let error = retry_github::<()>(
            || {
                attempts += 1;
                Err(Box::new(HttpFailure {
                    status: Some(status),
                    detail: "injected permanent response".to_owned(),
                })
                    as Box<dyn std::error::Error + Send + Sync>)
            },
            |_| panic!("permanent HTTP failures must not sleep"),
        )
        .expect_err("403 and 404 must fail without retry");
        assert_eq!(
            error
                .downcast_ref::<HttpFailure>()
                .and_then(|failure| failure.status),
            Some(status)
        );
        assert_eq!(attempts, 1);
    }

    let mut attempts = 0;
    let error = retry_github::<()>(
        || {
            attempts += 1;
            Err(Box::new(HttpFailure {
                status: Some(500),
                detail: "injected persistent server error".to_owned(),
            }) as Box<dyn std::error::Error + Send + Sync>)
        },
        |_| {},
    )
    .expect_err("retryable server failures must eventually exhaust");
    assert_eq!(
        error
            .downcast_ref::<HttpFailure>()
            .and_then(|failure| failure.status),
        Some(500)
    );
    assert_eq!(attempts, GITHUB_RETRY_ATTEMPTS);
    Ok(())
}

fn metadata_token_probe_rejects_wrong_repository_state() -> Result<()> {
    for (bad_ref, bad_bootstrap) in [(true, false), (false, true)] {
        let mut github = FakeTransport::new();
        github.bad_metadata_ref = bad_ref;
        github.bad_metadata_bootstrap = bad_bootstrap;
        assert!(probe_metadata_token(&mut github).is_err());
        assert!(github.events.iter().all(|event| event.starts_with("GET ")));
        assert!(!github.events.iter().any(|event| {
            event.starts_with("PUT ") || event.starts_with("POST ") || event.starts_with("DELETE ")
        }));
    }
    Ok(())
}

fn mock_matching_stale_draft_is_reconciled() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    let marker = format!(
        "<!-- v4-release-tx: {} -->",
        serde_json::to_string(&json!({
            "repository":context.repository,"run_id":"previous-run","source_sha":context.source_sha,
            "version":context.version,"tag":context.tag
        }))?
    );
    let mut stale = fake_release(12, &context.tag, &context.source_sha, true, false, false);
    stale["body"] = json!(marker);
    stale["upload_url"] = json!(
        "https://uploads.github.test/repos/pumni/Sky-Auto-Player/releases/12/assets{?name,label}"
    );
    github.releases.insert(12, stale);
    github.next_id = 13;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release(&root, &options, &mut github)?;
    assert_eq!(github.deletions, vec![12]);
    assert_eq!(
        read_json::<ReleaseState>(&options.state_root.join("release-state.json"))?.release_id,
        Some(13)
    );
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_conflicting_draft_is_not_deleted() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    let mut conflict = fake_release(12, &context.tag, &context.source_sha, true, false, false);
    conflict["body"] = json!(format!(
        "<!-- v4-release-tx: {{\"repository\":\"{REPOSITORY}\",\"run_id\":\"other\",\"source_sha\":\"{}\",\"version\":\"{}\",\"tag\":\"{}\"}} -->",
        "c".repeat(40),
        context.version,
        context.tag
    ));
    github.releases.insert(12, conflict);
    github.next_id = 13;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    assert!(publish_release(&root, &options, &mut github).is_err());
    assert!(github.deletions.is_empty());
    assert!(!github.events.iter().any(|event| event.starts_with("POST ")));
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_digest_mismatch_cleans_owned_draft() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.bad_digest = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    assert!(publish_release(&root, &options, &mut github).is_err());
    assert_eq!(github.deletions, vec![12]);
    assert!(!github.releases.contains_key(&12));
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_partial_upload_cleans_owned_draft() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.fail_upload_after_first = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    let error = publish_release(&root, &options, &mut github)
        .expect_err("partial asset upload must fail the pre-publication transaction");
    assert!(error.to_string().contains("injected upload failure"));
    assert_eq!(github.deletions, vec![12]);
    assert!(!github.releases.contains_key(&12));
    assert_eq!(
        github
            .events
            .iter()
            .filter(|event| event.starts_with("UPLOAD "))
            .count(),
        2
    );
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_cleanup_failure_preserves_original_error() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.bad_digest = true;
    github.fail_delete = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    let error = publish_release(&root, &options, &mut github)
        .expect_err("draft cleanup failure must remain visible with the original failure");
    let message = error.to_string();
    assert!(message.contains("GitHub server digest mismatch"));
    assert!(message.contains("owned draft cleanup failed"));
    assert!(message.contains("injected delete failure"));
    assert!(github.releases.contains_key(&12));
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_patch_timeout_reconciles_exact_release_id() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.patch_applies_then_fails = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release(&root, &options, &mut github)?;
    assert!(
        github
            .events
            .iter()
            .any(|event| event == "GET repos/pumni/Sky-Auto-Player/releases/12")
    );
    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_unknown_publication_never_cleans_up() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.fail_reconcile_after_patch = true;
    github.patch_applies_then_fails = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    assert!(publish_release(&root, &options, &mut github).is_err());
    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_latest_policy_failure_is_post_publication() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.preserve_latest_on_publish = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    assert!(publish_release(&root, &options, &mut github).is_err());
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert_eq!(state.phase, Phase::PublishedPendingMetadata);
    assert!(github.deletions.is_empty());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_stale_evidence_identity_fails_closed() -> Result<()> {
    let (_root, options, context, manifest) = fixture()?;
    let production = options
        .state_root
        .join("candidate-evidence")
        .join(EVIDENCE_NAMES[0]);
    let mut evidence: Value = read_json(&production)?;
    evidence["source_sha"] = json!("e".repeat(40));
    fs::write(&production, serde_json::to_vec(&evidence)?)?;
    assert!(
        validate_candidate_evidence(
            &options.state_root,
            &manifest.qualification_assets,
            &context
        )
        .is_err()
    );
    let mut qualification: Value = read_json(
        &options
            .state_root
            .join("candidate-evidence")
            .join(EVIDENCE_NAMES[1]),
    )?;
    qualification["version"] = json!("4.1.4");
    let qualification_path = options.state_root.join("tampered-qualification.json");
    fs::write(&qualification_path, serde_json::to_vec(&qualification)?)?;
    assert!(
        validate_qualification_evidence(
            &qualification_path,
            &context,
            &manifest.qualification_assets
        )
        .is_err()
    );
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_manifest_path_traversal_fails_closed() -> Result<()> {
    let (_root, options, _context, _manifest) = fixture()?;
    let record = CandidateRecord {
        name: "attacker.bin".to_owned(),
        release_name: "attacker.bin".to_owned(),
        source_name: "attacker.bin".to_owned(),
        role: "installer".to_owned(),
        size: 1,
        sha256: "0".repeat(64),
        source_path: String::new(),
        state_path: "../outside.bin".to_owned(),
    };
    assert!(safe_state_asset_path(&options.state_root, &record).is_err());
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn marker_parser_rejects_duplicates_and_multiple_markers() -> Result<()> {
    let (_, options, context, _) = fixture()?;
    let mut release = fake_release(12, &context.tag, &context.source_sha, true, false, false);
    release["body"] = json!(format!(
        "<!-- v4-release-tx: {{\"repository\":\"{REPOSITORY}\",\"repository\":\"{REPOSITORY}\",\"run_id\":\"run\",\"source_sha\":\"{}\",\"version\":\"{}\",\"tag\":\"{}\"}} -->",
        context.source_sha, context.version, context.tag
    ));
    assert!(!matches_transaction(&release, &context, false));
    release["body"] = json!(format!(
        "<!-- v4-release-tx: {{\"repository\":\"{REPOSITORY}\",\"run_id\":\"run\",\"source_sha\":\"{}\",\"version\":\"{}\",\"tag\":\"{}\"}} --><!-- v4-release-tx: {{}} -->",
        context.source_sha, context.version, context.tag
    ));
    assert!(!matches_transaction(&release, &context, false));
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn workflow_run_identity_fails_closed() -> Result<()> {
    let good = json!({"path":".github/workflows/release-v4.yml@refs/heads/main","event":"workflow_dispatch","head_sha":"a".repeat(40),"repository":{"full_name":REPOSITORY}});
    assert!(valid_workflow_run(&good, Some(&"a".repeat(40)), REPOSITORY));
    for bad in [
        json!({"path":".github/workflows/rehearse-v4.yml","event":"workflow_dispatch","head_sha":"a".repeat(40),"repository":{"full_name":REPOSITORY}}),
        json!({"path":"attacker/.github/workflows/release-v4.yml@refs/heads/main","event":"workflow_dispatch","head_sha":"a".repeat(40),"repository":{"full_name":REPOSITORY}}),
        json!({"path":".github/workflows/release-v4.yml","event":"push","head_sha":"a".repeat(40),"repository":{"full_name":REPOSITORY}}),
        json!({"path":".github/workflows/release-v4.yml","event":"workflow_dispatch","head_sha":"b".repeat(40),"repository":{"full_name":REPOSITORY}}),
        json!({"path":".github/workflows/release-v4.yml","event":"workflow_dispatch","head_sha":"a".repeat(40)}),
    ] {
        assert!(!valid_workflow_run(&bad, Some(&"a".repeat(40)), REPOSITORY));
    }
    Ok(())
}

fn timestamp_normalization_is_utc_and_strict() -> Result<()> {
    assert_eq!(
        canonical_timestamp("2026-10-25T12:34:56+02:30")?,
        "2026-10-25T10:04:56Z"
    );
    assert_eq!(
        canonical_timestamp("2026-01-01T00:00:00.123Z")?,
        "2026-01-01T00:00:00Z"
    );
    assert!(canonical_timestamp("not-a-date").is_err());
    Ok(())
}

#[derive(Default)]
struct FakeTransport {
    latest: Value,
    releases: std::collections::BTreeMap<u64, Value>,
    next_id: u64,
    assets: std::collections::BTreeMap<String, Vec<u8>>,
    events: Vec<String>,
    deletions: Vec<u64>,
    bad_digest: bool,
    patch_applies_then_fails: bool,
    fail_reconcile_after_patch: bool,
    preserve_latest_on_publish: bool,
    fail_upload_after_first: bool,
    fail_delete: bool,
    post_created_then_timeout: bool,
    fail_metadata_put_after_write: bool,
    bad_metadata_ref: bool,
    bad_metadata_bootstrap: bool,
    metadata: Vec<u8>,
    metadata_files: std::collections::BTreeMap<String, Vec<u8>>,
    raw_metadata_files: std::collections::BTreeMap<String, Vec<u8>>,
    workflow_runs: std::collections::BTreeMap<String, Value>,
}

impl FakeTransport {
    fn new() -> Self {
        Self {
            next_id: 12,
            ..Self::default()
        }
    }
}

impl Transport for FakeTransport {
    fn request(
        &mut self,
        method: &str,
        endpoint: &str,
        body: Option<&Value>,
    ) -> Result<Option<Value>> {
        self.events.push(format!("{method} {endpoint}"));
        if method == "GET" && endpoint.starts_with("repos/pumni/Sky-Auto-Player/actions/runs/") {
            let id = endpoint.rsplit('/').next().unwrap_or_default();
            return Ok(self.workflow_runs.get(id).cloned());
        }
        if method == "GET" && endpoint == "repos/pumni/Sky-Auto-Player/releases/latest" {
            return Ok(Some(self.latest.clone()));
        }
        if method == "GET"
            && endpoint == "repos/pumni/Sky-Auto-Player/git/ref/heads/release-metadata"
        {
            let expected_ref = if self.bad_metadata_ref {
                "refs/heads/main"
            } else {
                "refs/heads/release-metadata"
            };
            return Ok(Some(
                json!({"ref":expected_ref,"object":{"sha":"a".repeat(40)}}),
            ));
        }
        if method == "GET"
            && endpoint
                == "repos/pumni/Sky-Auto-Player/contents/.release-metadata/README.md?ref=release-metadata"
        {
            let bootstrap = if self.bad_metadata_bootstrap {
                "wrong repository or branch"
            } else {
                BOOTSTRAP_CONTENT
            };
            return Ok(Some(
                json!({"encoding":"base64","content":base64::engine::general_purpose::STANDARD.encode(bootstrap),"sha":"b".repeat(40)}),
            ));
        }
        if endpoint.starts_with("repos/pumni/Sky-Auto-Player/contents/channels/") {
            let channel = endpoint
                .strip_prefix("repos/pumni/Sky-Auto-Player/contents/channels/")
                .and_then(|value| value.split('/').next())
                .ok_or("fake metadata channel missing")?;
            if !matches!(channel, "stable" | "beta") {
                return Err("fake metadata request had an unsupported channel".into());
            }
            match method {
                "GET" => {
                    return Ok(self.metadata_files.get(channel).map(|bytes| {
                        json!({"encoding":"base64","content":base64::engine::general_purpose::STANDARD.encode(bytes),"sha":"c".repeat(40)})
                    }));
                }
                "PUT" => {
                    let payload = body.ok_or("fake metadata PUT body missing")?;
                    let encoded = payload["content"]
                        .as_str()
                        .ok_or("fake metadata PUT content missing")?;
                    let bytes = base64::engine::general_purpose::STANDARD.decode(encoded)?;
                    self.metadata = bytes.clone();
                    self.metadata_files.insert(channel.to_owned(), bytes);
                    if let Some(contents) = self.metadata_files.get(channel).cloned() {
                        self.raw_metadata_files.insert(channel.to_owned(), contents);
                    }
                    if self.fail_metadata_put_after_write {
                        self.fail_metadata_put_after_write = false;
                        return Err(Box::new(HttpFailure {
                            status: Some(504),
                            detail: "injected metadata write timeout after commit".to_owned(),
                        }));
                    }
                    return Ok(Some(
                        json!({"content":{"path":endpoint,"sha":"c".repeat(40)}}),
                    ));
                }
                _ => {}
            }
        }
        if method == "GET" && endpoint.starts_with("repos/pumni/Sky-Auto-Player/releases/tags/") {
            let tag = endpoint.rsplit('/').next().unwrap_or_default();
            return Ok(self
                .releases
                .values()
                .find(|release| release["tag_name"] == tag)
                .cloned());
        }
        if let Some(id_text) = endpoint.strip_prefix("repos/pumni/Sky-Auto-Player/releases/")
            && let Ok(id) = id_text.parse::<u64>()
        {
            match method {
                "GET" => {
                    if self.fail_reconcile_after_patch
                        && self
                            .releases
                            .get(&id)
                            .is_some_and(|release| release["draft"] == false)
                    {
                        return Err(Box::new(HttpFailure {
                            status: Some(503),
                            detail: "injected read failure".to_owned(),
                        }));
                    }
                    return Ok(self.releases.get(&id).cloned());
                }
                "DELETE" => {
                    self.deletions.push(id);
                    if self.fail_delete {
                        return Err(Box::new(HttpFailure {
                            status: Some(403),
                            detail: "injected delete failure".to_owned(),
                        }));
                    }
                    self.releases.remove(&id);
                    return Ok(None);
                }
                "PATCH" => {
                    let release = self.releases.get_mut(&id).ok_or("release missing")?;
                    release["draft"] = body.ok_or("PATCH body missing")?["draft"].clone();
                    if release["draft"] == false {
                        release["immutable"] = json!(true);
                        release["published_at"] = json!("2026-10-25T09:00:00Z");
                        if !self.preserve_latest_on_publish {
                            self.latest = release.clone();
                        }
                    }
                    if self.patch_applies_then_fails {
                        self.patch_applies_then_fails = false;
                        return Err(Box::new(HttpFailure {
                            status: Some(504),
                            detail: "injected patch timeout".to_owned(),
                        }));
                    }
                    return Ok(Some(release.clone()));
                }
                _ => {}
            }
        }
        if method == "POST" && endpoint == "repos/pumni/Sky-Auto-Player/releases" {
            let body = body.ok_or("draft body missing")?;
            let id = self.next_id;
            self.next_id += 1;
            let release = json!({"id":id,"tag_name":body["tag_name"],"target_commitish":body["target_commitish"],
                "name":body["name"],"body":body["body"],"draft":true,"prerelease":body["prerelease"],"immutable":false,
                "upload_url":format!("https://uploads.github.test/repos/{REPOSITORY}/releases/{id}/assets{{?name,label}}"),"assets":[]});
            self.releases.insert(id, release.clone());
            if self.post_created_then_timeout {
                self.post_created_then_timeout = false;
                return Err(Box::new(HttpFailure {
                    status: Some(504),
                    detail: "injected draft POST timeout after commit".to_owned(),
                }));
            }
            return Ok(Some(release));
        }
        if method == "GET" && endpoint.starts_with("repos/pumni/Sky-Auto-Player/git/ref/tags/") {
            return Ok(None);
        }
        Err(format!("unhandled fake GitHub request: {method} {endpoint}").into())
    }

    fn upload(&mut self, endpoint: &str, name: &str, path: &Path) -> Result<()> {
        self.events.push(format!("UPLOAD {name}"));
        if self.fail_upload_after_first
            && self
                .events
                .iter()
                .filter(|event| event.starts_with("UPLOAD "))
                .count()
                > 1
        {
            return Err("injected upload failure after partial publication".into());
        }
        let id = endpoint
            .split("/releases/")
            .nth(1)
            .and_then(|value| value.split('/').next())
            .ok_or("fake upload release id missing")?
            .parse::<u64>()?;
        let bytes = fs::read(path)?;
        let digest = if self.bad_digest {
            "sha256:".to_owned() + &"0".repeat(64)
        } else {
            format!("sha256:{}", sha256_hex(&bytes))
        };
        let asset_index = self
            .releases
            .get(&id)
            .and_then(|release| release["assets"].as_array())
            .map(Vec::len)
            .ok_or("fake release assets missing")?;
        let asset_id = id
            .checked_mul(1000)
            .and_then(|value| value.checked_add(asset_index as u64 + 1))
            .ok_or("fake asset identifier overflow")?;
        let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/assets/{asset_id}");
        self.assets.insert(url.clone(), bytes.clone());
        let release = self.releases.get_mut(&id).ok_or("fake release missing")?;
        let assets = release["assets"]
            .as_array_mut()
            .ok_or("fake assets missing")?;
        assets.push(
            json!({"name":name,"size":bytes.len(),"state":"uploaded","digest":digest,"url":url}),
        );
        Ok(())
    }

    fn download(&mut self, endpoint: &str, destination: &Path) -> Result<()> {
        let bytes = self.assets.get(endpoint).ok_or("fake asset not found")?;
        fs::write(destination, bytes)?;
        Ok(())
    }

    fn raw_get(&mut self, endpoint: &str) -> Result<Option<Vec<u8>>> {
        self.events.push(format!("RAW_GET {endpoint}"));
        let channel = endpoint
            .rsplit("/channels/")
            .next()
            .and_then(|value| value.strip_suffix("/latest.json"))
            .ok_or("fake raw metadata URL was not canonical")?;
        Ok(self.raw_metadata_files.get(channel).cloned())
    }
}

fn fake_release(
    id: u64,
    tag: &str,
    source: impl AsRef<str>,
    draft: bool,
    prerelease: bool,
    immutable: bool,
) -> Value {
    json!({"id":id,"tag_name":tag,"target_commitish":source.as_ref(),"draft":draft,"prerelease":prerelease,
        "immutable":immutable,"published_at":if draft { Value::Null } else { json!("2026-01-01T00:00:00Z") },"assets":[]})
}

fn fixture() -> Result<(PathBuf, Options, Context, CandidateManifest)> {
    let root = repo::root();
    let version = repo::project_version(&root)?;
    let parsed = version::parse(&version)?;
    let channel = if parsed.pre.is_empty() {
        "stable"
    } else {
        "beta"
    };
    let source_sha = git_text(&root, &["rev-parse", "HEAD"])?;
    let tag = format!("v{version}");
    let context = Context {
        schema_version: 1,
        repository: REPOSITORY.to_owned(),
        version: version.clone(),
        channel: channel.to_owned(),
        tag: tag.clone(),
        release_notes_path: format!("docs/releases/{tag}.md"),
        source_sha: source_sha.clone(),
        workflow_sha: source_sha.clone(),
        run_id: env::var("GITHUB_RUN_ID").unwrap_or_else(|_| "9001".to_owned()),
        created_at: "2026-10-25T00:00:00Z".to_owned(),
    };
    static NEXT_FIXTURE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let fixture_id = NEXT_FIXTURE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let state_root = env::temp_dir().join(format!(
        "sky-v4-release-mock-{}-{fixture_id}",
        std::process::id()
    ));
    fs::create_dir_all(state_root.join("candidate-bundle"))?;
    fs::create_dir_all(state_root.join("candidate-evidence"))?;
    let options = Options {
        state_root: state_root.clone(),
        key_path: None,
        workflow_sha: None,
    };
    let installer_name = release_metadata::canonical_installer_name(&version);
    let signature_name = format!("{installer_name}.sig");
    let mut base = Vec::new();
    for (name, role, dir, bytes) in [
        (
            installer_name.as_str(),
            "installer",
            "candidate-bundle",
            b"installer-fixture".as_slice(),
        ),
        (
            signature_name.as_str(),
            "updater-signature",
            "candidate-bundle",
            b"c2lnbmF0dXJl".as_slice(),
        ),
        (
            EVIDENCE_NAMES[2],
            "authenticode-evidence",
            "candidate-evidence",
            b"auth evidence".as_slice(),
        ),
        (
            EVIDENCE_NAMES[5],
            "sbom",
            "candidate-evidence",
            b"sbom fixture".as_slice(),
        ),
    ] {
        let path = state_root.join(dir).join(name);
        fs::write(&path, bytes)?;
        base.push(CandidateRecord {
            name: name.replace(' ', "."),
            release_name: name.replace(' ', "."),
            source_name: name.to_owned(),
            role: role.to_owned(),
            size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
            source_path: path.to_string_lossy().into_owned(),
            state_path: Path::new(dir)
                .join(name)
                .to_string_lossy()
                .replace('\\', "/"),
        });
    }
    let installer = base
        .iter()
        .find(|record| record.role == "installer")
        .unwrap()
        .clone();
    let signature = base
        .iter()
        .find(|record| record.role == "updater-signature")
        .unwrap()
        .clone();
    let auth = base
        .iter()
        .find(|record| record.role == "authenticode-evidence")
        .unwrap()
        .clone();
    let sbom = base
        .iter()
        .find(|record| record.role == "sbom")
        .unwrap()
        .clone();
    let production = json!({"source_sha":source_sha,"version":version,"channel":channel,"authenticode_mode":"unsigned-zero-budget","authenticode_state":"unsigned","authenticode_provider":"none","approved_signer_thumbprint":null,"observed_signer_thumbprint":null,
        "updater_signature_status":"valid","qualification_status":"PASS","installer":installer.source_name,"installer_size":installer.size,"installer_sha256":installer.sha256,
        "updater_signature":signature.source_name,"signature_size":signature.size,"updater_signature_sha256":signature.sha256,"authenticode_evidence":auth.source_name,"authenticode_evidence_sha256":auth.sha256,"sbom":sbom.source_name,"sbom_sha256":sbom.sha256});
    let qualification = json!({"schema_version":1,"evidence_type":"tauri-nsis-qualified-release","qualified":true,"qualification":"install-launch-uninstall","product_name":"Sky Auto Player","identifier":"io.github.pumni.skyautoplayer","version":version,"target":"nsis","install_mode":"currentUser","installer":installer.source_name,"updater_signature":signature.source_name,"installer_size":installer.size,"signature_size":signature.size,"installer_sha256":installer.sha256,"updater_signature_sha256":signature.sha256,"authenticode_mode":"unsigned-zero-budget","authenticode_evidence":auth.source_name,"authenticode_evidence_sha256":auth.sha256,"sbom":sbom.source_name,"sbom_sha256":sbom.sha256});
    for (name, value) in [
        (EVIDENCE_NAMES[0], &production),
        (EVIDENCE_NAMES[1], &qualification),
    ] {
        let path = state_root.join("candidate-evidence").join(name);
        fs::write(&path, serde_json::to_vec(value)?)?;
        let bytes = fs::read(&path)?;
        base.push(CandidateRecord {
            name: name.to_owned(),
            release_name: name.to_owned(),
            source_name: name.to_owned(),
            role: if name == EVIDENCE_NAMES[0] {
                "production-evidence"
            } else {
                "qualification-evidence"
            }
            .to_owned(),
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            source_path: path.to_string_lossy().into_owned(),
            state_path: format!("candidate-evidence/{name}"),
        });
    }
    for name in [EVIDENCE_NAMES[3], EVIDENCE_NAMES[4]] {
        let path = state_root.join("candidate-evidence").join(name);
        fs::write(&path, b"fixture evidence")?;
        let bytes = fs::read(&path)?;
        base.push(CandidateRecord {
            name: name.to_owned(),
            release_name: name.to_owned(),
            source_name: name.to_owned(),
            role: if name == EVIDENCE_NAMES[3] {
                "installed-authenticode-evidence"
            } else {
                "artifact-summary"
            }
            .to_owned(),
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            source_path: path.to_string_lossy().into_owned(),
            state_path: format!("candidate-evidence/{name}"),
        });
    }
    base.sort_by(|a, b| a.source_name.cmp(&b.source_name));
    validate_candidate_evidence(&state_root, &base, &context)?;
    let public_names = [
        release_metadata::canonical_release_installer_name(&version),
        format!(
            "{}.sig",
            release_metadata::canonical_release_installer_name(&version)
        ),
    ];
    let public_assets = base
        .iter()
        .filter(|record| public_names.contains(&record.release_name))
        .cloned()
        .collect();
    Ok((
        root,
        options,
        context.clone(),
        CandidateManifest {
            schema_version: 1,
            source_sha,
            version,
            channel: channel.to_owned(),
            tag,
            qualification_assets: base,
            public_assets,
        },
    ))
}

fn fixture_state(context: &Context) -> ReleaseState {
    ReleaseState {
        schema_version: 1,
        repository: context.repository.clone(),
        version: context.version.clone(),
        channel: context.channel.clone(),
        tag: context.tag.clone(),
        source_sha: context.source_sha.clone(),
        workflow_sha: context.workflow_sha.clone(),
        run_id: context.run_id.clone(),
        phase: Phase::Ready,
        // Publish tests begin after the separately guarded exact-source attestation phase.
        attested: true,
        qualified_after_download: false,
        release_id: None,
        published_at: None,
        promoted_at: None,
        final_verified: false,
    }
}

fn prepare_publish_fixture(
    options: &Options,
    context: &Context,
    manifest: &CandidateManifest,
    latest: &Value,
) -> Result<()> {
    write_json_atomic(&options.state_root.join("release-context.json"), context)?;
    write_json_atomic(
        &options.state_root.join("release-state.json"),
        &fixture_state(context),
    )?;
    write_json_atomic(
        &options.state_root.join("candidate-manifest.json"),
        manifest,
    )?;
    write_json_atomic(
        &options.state_root.join("preflight-evidence.json"),
        &json!({"pre_publication_latest":latest}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_github_api_json_fails_closed_with_endpoint_context() {
        assert_eq!(
            decode_api_response("DELETE", "repos/example/releases/1", b"").unwrap(),
            None
        );
        let error = decode_api_response("GET", "repos/example/releases/latest", b"{not-json")
            .expect_err("malformed API JSON must be rejected");
        assert!(error.to_string().contains("repos/example/releases/latest"));
        assert!(
            decode_api_response("GET", "repos/example", br#"{"ok":true}"#)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn release_upload_url_is_bound_to_canonical_numeric_release_route() {
        let endpoint = canonical_upload_url(
            "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets{?name,label}",
        )
        .expect("canonical GitHub upload template must be accepted");
        assert_eq!(
            endpoint.as_str(),
            "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets"
        );

        for endpoint in [
            "http://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets",
            "https://uploads.github.test/repos/pumni/Sky-Auto-Player/releases/123/assets",
            "https://uploads.github.com/repos/other/Sky-Auto-Player/releases/123/assets",
            "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/latest/assets",
            "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/0/assets",
            "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets/456",
            "https://uploads.github.com:443/repos/pumni/Sky-Auto-Player/releases/123/assets",
            "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets?name=other.exe",
            "https://user@uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets",
        ] {
            assert!(
                canonical_upload_url(endpoint).is_err(),
                "noncanonical upload endpoint must be rejected: {endpoint}"
            );
        }
    }

    #[test]
    fn mock_release_success_and_reconciliation() {
        mock_publish_succeeds().unwrap();
        mock_draft_post_timeout_reconciles_exact_marker().unwrap();
        mock_patch_timeout_reconciles_exact_release_id().unwrap();
        mock_complete_transaction_and_raw_metadata().unwrap();
        mock_metadata_promotion_is_idempotent_after_committed_timeout().unwrap();
        mock_publication_identity_and_asset_inventory_fail_closed().unwrap();
        metadata_token_probe_rejects_wrong_repository_state().unwrap();
    }
    #[test]
    fn mock_failure_cleanup_and_immutability() {
        mock_conflicting_draft_is_not_deleted().unwrap();
        mock_digest_mismatch_cleans_owned_draft().unwrap();
        mock_unknown_publication_never_cleans_up().unwrap();
        mock_latest_policy_failure_is_post_publication().unwrap();
        mock_stale_evidence_identity_fails_closed().unwrap();
        mock_manifest_path_traversal_fails_closed().unwrap();
        mock_release_doctor_detects_metadata_endpoint_divergence().unwrap();
        doctor_state_input_rejects_malformed_and_oversized_files().unwrap();
        retry_policy_retries_only_transient_responses().unwrap();
    }
    #[test]
    fn doctor_and_timestamp_negative_contracts() {
        marker_parser_rejects_duplicates_and_multiple_markers().unwrap();
        workflow_run_identity_fails_closed().unwrap();
        timestamp_normalization_is_utc_and_strict().unwrap();
    }
}
