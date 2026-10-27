#![forbid(unsafe_code)]

use crate::{Result, process, release_handoff, release_metadata, release_signing, repo, version};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

mod recovery;

pub const REPOSITORY: &str = "pumni/Sky-Auto-Player";
const RELEASE_WORKFLOW_REF: &str =
    "pumni/Sky-Auto-Player/.github/workflows/release-v4.yml@refs/heads/main";
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
const RELEASE_DEPENDENCY_STEP_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const RELEASE_DEFENDER_STEP_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const ATTESTATION_VERIFY_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const RAW_METADATA_RETRY_BUDGET: Duration = Duration::from_secs(60);
const RAW_METADATA_RETRY_INTERVAL: Duration = Duration::from_secs(5);

pub(crate) fn linux_process_smoke(args: &[String]) -> Result<()> {
    recovery::linux_process_smoke(args)
}

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
    run_attempt: u32,
    created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeIdentity {
    github_sha: String,
    github_repository: String,
    github_run_id: String,
    github_run_attempt: String,
    github_event_name: String,
    github_ref: String,
    github_workflow_ref: String,
}

impl RuntimeIdentity {
    fn from_environment() -> Result<Self> {
        Self::from_values(
            env::var("GITHUB_SHA").ok(),
            env::var("GITHUB_REPOSITORY").ok(),
            env::var("GITHUB_RUN_ID").ok(),
            env::var("GITHUB_RUN_ATTEMPT").ok(),
            env::var("GITHUB_EVENT_NAME").ok(),
            env::var("GITHUB_REF").ok(),
            env::var("GITHUB_WORKFLOW_REF").ok(),
        )
    }

    fn from_values(
        github_sha: Option<String>,
        github_repository: Option<String>,
        github_run_id: Option<String>,
        github_run_attempt: Option<String>,
        github_event_name: Option<String>,
        github_ref: Option<String>,
        github_workflow_ref: Option<String>,
    ) -> Result<Self> {
        Ok(Self {
            github_sha: github_sha.ok_or("GITHUB_SHA is required")?,
            github_repository: github_repository.ok_or("GITHUB_REPOSITORY is required")?,
            github_run_id: github_run_id.ok_or("GITHUB_RUN_ID is required")?,
            github_run_attempt: github_run_attempt.ok_or("GITHUB_RUN_ATTEMPT is required")?,
            github_event_name: github_event_name.ok_or("GITHUB_EVENT_NAME is required")?,
            github_ref: github_ref.ok_or("GITHUB_REF is required")?,
            github_workflow_ref: github_workflow_ref.ok_or("GITHUB_WORKFLOW_REF is required")?,
        })
    }
}

fn validate_runtime_identity(
    context: &Context,
    workflow_sha_arg: Option<&str>,
    runtime: &RuntimeIdentity,
) -> Result<()> {
    let workflow_sha_arg =
        workflow_sha_arg.ok_or("--workflow-sha is required for this release phase")?;
    let run_attempt = runtime
        .github_run_attempt
        .parse::<u32>()
        .map_err(|_| "GITHUB_RUN_ATTEMPT must be a positive decimal integer")?;
    if runtime.github_repository != context.repository
        || runtime.github_event_name != "workflow_dispatch"
        || runtime.github_ref != "refs/heads/main"
        || runtime.github_workflow_ref != RELEASE_WORKFLOW_REF
        || runtime.github_run_id != context.run_id
        || run_attempt != context.run_attempt
        || run_attempt == 0
        || run_attempt > 100
        || !runtime
            .github_sha
            .eq_ignore_ascii_case(&context.workflow_sha)
        || !runtime.github_sha.eq_ignore_ascii_case(&context.source_sha)
        || !workflow_sha_arg.eq_ignore_ascii_case(&context.workflow_sha)
        || runtime.github_run_id.is_empty()
        || !runtime
            .github_run_id
            .bytes()
            .all(|byte| byte.is_ascii_digit())
    {
        return Err(
            "release runtime identity does not match the exact canonical workflow context".into(),
        );
    }
    Ok(())
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
    state_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CandidateManifest {
    schema_version: u32,
    repository: String,
    source_sha: String,
    workflow_sha: String,
    run_id: String,
    run_attempt: u32,
    version: String,
    channel: String,
    tag: String,
    unsigned_handoff: release_handoff::UnsignedArtifactManifest,
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
    run_attempt: u32,
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

#[derive(Debug)]
struct MutationOutcomeUnknown {
    method: String,
    endpoint: String,
    detail: String,
}

impl std::fmt::Display for MutationOutcomeUnknown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} outcome is unknown and must be reconciled before another mutation: {}",
            self.method, self.endpoint, self.detail
        )
    }
}

impl std::error::Error for MutationOutcomeUnknown {}

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

const METADATA_WRITE_TOKEN_ENV: &str = "V4_METADATA_WRITE_TOKEN";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecoveryTokenClass {
    ReadOnly,
    MetadataWrite,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SelectedRecoveryToken<'a> {
    class: RecoveryTokenClass,
    value: &'a str,
}

struct RecoveryCredentials {
    read_only_token: String,
    metadata_write_token: Option<String>,
}

impl RecoveryCredentials {
    fn new(read_only_token: String, metadata_write_token: Option<String>) -> Result<Self> {
        let valid_token = |token: &str| {
            !token.is_empty() && !token.bytes().any(|byte| byte.is_ascii_whitespace())
        };
        if !valid_token(&read_only_token)
            || metadata_write_token
                .as_deref()
                .is_some_and(|token| !valid_token(token))
        {
            return Err("metadata recovery GitHub credentials are malformed".into());
        }
        Ok(Self {
            read_only_token,
            metadata_write_token,
        })
    }

    fn select(&self, method: &str, endpoint: &str) -> Result<SelectedRecoveryToken<'_>> {
        if method.eq_ignore_ascii_case("GET") {
            return Ok(SelectedRecoveryToken {
                class: RecoveryTokenClass::ReadOnly,
                value: &self.read_only_token,
            });
        }
        let stable_contents_endpoint =
            format!("repos/{REPOSITORY}/contents/channels/stable/latest.json");
        if method.eq_ignore_ascii_case("PUT") && endpoint == stable_contents_endpoint {
            return Ok(SelectedRecoveryToken {
                class: RecoveryTokenClass::MetadataWrite,
                value: self
                    .metadata_write_token
                    .as_deref()
                    .ok_or("metadata recovery stable write token is unavailable")?,
            });
        }
        Err(
            "metadata recovery transport permits only GitHub reads and one stable metadata PUT"
                .into(),
        )
    }
}

struct GhTransport {
    recovery_credentials: Option<RecoveryCredentials>,
}

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
        Self {
            recovery_credentials: None,
        }
    }

    fn for_metadata_recovery(
        read_only_token: String,
        metadata_write_token: Option<String>,
    ) -> Result<Self> {
        Ok(Self {
            recovery_credentials: Some(RecoveryCredentials::new(
                read_only_token,
                metadata_write_token,
            )?),
        })
    }

    fn selected_recovery_token(
        &self,
        method: &str,
        endpoint: &str,
    ) -> Result<Option<SelectedRecoveryToken<'_>>> {
        self.recovery_credentials
            .as_ref()
            .map(|credentials| credentials.select(method, endpoint))
            .transpose()
    }

    fn invoke_with_timeout(
        &self,
        args: &[String],
        payload: Option<&[u8]>,
        selected_token: Option<&str>,
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
        let output = if self.recovery_credentials.is_some() {
            let token = selected_token
                .ok_or("metadata recovery GitHub child requires an operation-selected token")?;
            process::capture_owned_timeout_runtime_only(
                std::ffi::OsStr::new("gh"),
                &args,
                &repo::root(),
                &[
                    ("GH_TOKEN".to_owned(), token.to_owned()),
                    ("GH_PAGER".to_owned(), "cat".to_owned()),
                    ("PAGER".to_owned(), "cat".to_owned()),
                ],
                timeout,
            )
        } else {
            process::capture_owned_timeout_filtered(
                std::ffi::OsStr::new("gh"),
                &args,
                &repo::root(),
                &[
                    ("GH_PAGER".to_owned(), "cat".to_owned()),
                    ("PAGER".to_owned(), "cat".to_owned()),
                ],
                &[],
                timeout,
            )
        };
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

    fn invoke_to_file(
        &self,
        args: &[String],
        destination: &Path,
        selected_token: Option<&str>,
    ) -> Result<()> {
        let output = if self.recovery_credentials.is_some() {
            let token = selected_token
                .ok_or("metadata recovery asset download requires its read-only GitHub token")?;
            process::capture_owned_timeout_to_file_runtime_only(
                std::ffi::OsStr::new("gh"),
                args,
                &repo::root(),
                &[
                    ("GH_TOKEN".to_owned(), token.to_owned()),
                    ("GH_PAGER".to_owned(), "cat".to_owned()),
                    ("PAGER".to_owned(), "cat".to_owned()),
                ],
                destination,
                Duration::from_secs(10 * 60),
            )?
        } else {
            process::capture_owned_timeout_to_file_filtered(
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
            )?
        };
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
        let selected = self.selected_recovery_token(method, endpoint)?;
        match self.invoke_with_timeout(
            &args,
            payload.as_deref(),
            selected.map(|credential| credential.value),
            Duration::from_secs(60),
        ) {
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
        request_with_retry_policy(
            method,
            endpoint,
            || self.api_once(method, endpoint, body),
            std::thread::sleep,
        )
    }

    fn upload(&mut self, endpoint: &str, name: &str, path: &Path) -> Result<()> {
        if self.recovery_credentials.is_some() {
            return Err("metadata recovery transport cannot upload release assets".into());
        }
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
            path_query.clone(),
            "--input".to_owned(),
            "-".to_owned(),
            "--header".to_owned(),
            "Content-Type: application/octet-stream".to_owned(),
        ];
        request_with_retry_policy(
            "POST",
            &path_query,
            || {
                let mut request = args.clone();
                if let Some(input) = request.iter_mut().find(|arg| arg.as_str() == "-") {
                    *input = path.to_string_lossy().into_owned();
                }
                self.invoke_with_timeout(&request, None, None, Duration::from_secs(10 * 60))
                    .map(|_| ())
            },
            std::thread::sleep,
        )
    }

    fn download(&mut self, endpoint: &str, destination: &Path) -> Result<()> {
        let selected = self.selected_recovery_token("GET", endpoint)?;
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
                match self.invoke_to_file(
                    &args,
                    destination,
                    selected.map(|credential| credential.value),
                ) {
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
        let output = if self.recovery_credentials.is_some() {
            process::capture_owned_timeout_runtime_only(
                std::ffi::OsStr::new(curl),
                &args,
                &repo::root(),
                &[],
                Duration::from_secs(15),
            )?
        } else {
            process::capture_owned_timeout_filtered(
                std::ffi::OsStr::new(curl),
                &args,
                &repo::root(),
                &[],
                &[],
                Duration::from_secs(15),
            )?
        };
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
    for secret_name in ["GH_TOKEN", "GITHUB_TOKEN", METADATA_WRITE_TOKEN_ENV] {
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

fn request_with_retry_policy<T>(
    method: &str,
    endpoint: &str,
    mut operation: impl FnMut() -> Result<T>,
    sleep: impl FnMut(Duration),
) -> Result<T> {
    if method.eq_ignore_ascii_case("GET") {
        retry_github(operation, sleep)
    } else {
        operation().map_err(|error| {
            Box::new(MutationOutcomeUnknown {
                method: method.to_owned(),
                endpoint: endpoint.to_owned(),
                detail: error.to_string(),
            }) as Box<dyn std::error::Error + Send + Sync>
        })
    }
}

pub(crate) fn run(root: &Path, args: &[String]) -> Result<()> {
    let command = args.first().ok_or("release-v4 requires a phase")?.as_str();
    if command == "self-test" {
        return self_test();
    }
    if command == "cleanup-signing-material" {
        return release_signing::cleanup_run_material();
    }
    if command == "doctor" {
        return doctor(root, &args[1..]);
    }
    if matches!(
        command,
        "metadata-recovery-plan" | "metadata-recovery-apply"
    ) {
        return recovery::run(root, command, &args[1..]);
    }
    let mut state_root = None;
    let mut workflow_sha = None;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--state-root" => {
                state_root = Some(PathBuf::from(next_value(args, &mut index, "--state-root")?))
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
        workflow_sha,
    };
    let expected_job = expected_job_for_command(command);
    if expected_job.is_some() {
        require_runtime_context(root, &options, command)?;
    }
    let mut transport = GhTransport::new();
    match command {
        "preflight" => preflight(root, &options, &mut transport),
        "build-unsigned" => build_unsigned(root, &options),
        "sign-candidate" => sign_candidate(root, &options),
        "qualify-candidate" => qualify_candidate(root, &options),
        "verify-candidate" | "probe-verify-candidate" => verify_candidate_handoff(root, &options),
        "verify-attestations" => verify_attestations(root, &options),
        "publish-release" => publish_release(root, &options, &mut transport),
        "promote-metadata" => promote_metadata(root, &options, &mut transport),
        "final-verify" => final_verify(root, &options, &mut transport),
        "cleanup" => cleanup_local_state(root, &options),
        "probe-metadata-token" => {
            let contents_permission =
                env::var("V4_RELEASE_METADATA_TOKEN_REQUESTED_CONTENTS_PERMISSION")
                    .map_err(|_| "pre-publication metadata App permission evidence is required")?;
            probe_metadata_token(&contents_permission, &mut transport)
        }
        _ => Err(format!("unknown release-v4 phase: {command}").into()),
    }
}

pub(crate) fn usage() -> &'static str {
    "Usage:\n  cargo xtask release-v4 preflight --state-root <dir> --workflow-sha <sha>\n  cargo xtask release-v4 <build-unsigned|sign-candidate|qualify-candidate|verify-candidate|probe-verify-candidate|verify-attestations|probe-metadata-token|publish-release|promote-metadata|final-verify> --state-root <dir> --workflow-sha <sha>\n  cargo xtask release-v4 cleanup --state-root <dir>\n  cargo xtask release-v4 cleanup-signing-material\n  cargo xtask release-v4 doctor [--tag <tag>] [--version <semver>] [--channel <stable|beta>] [--run-id <id>] [--workflow-sha <sha>] [--state-root <dir>] [--format <text|json>]\n  cargo xtask release-v4 self-test\n  cargo xtask release-v4 metadata-recovery-plan --tag <v4.x.y> --output <path>\n  cargo xtask release-v4 metadata-recovery-apply --tag <v4.x.y> --plan <path> --plan-run-id <id> --plan-sha256 <sha256> --confirm-metadata-only-recovery\nRecovery is operator-gated by .github/workflows/v4-metadata-recovery.yml; plan is read-only and apply writes stable channel metadata only."
}

fn expected_job_for_command(command: &str) -> Option<&'static str> {
    match command {
        "build-unsigned" => Some("build_unsigned"),
        "sign-candidate" | "qualify-candidate" => Some("sign_qualify"),
        "verify-candidate" => Some("attest"),
        "probe-verify-candidate" | "probe-metadata-token" => Some("metadata_probe"),
        "verify-attestations" | "publish-release" => Some("publish"),
        "promote-metadata" => Some("metadata_promote"),
        "final-verify" => Some("final_verify"),
        _ => None,
    }
}

fn require_runtime_context(root: &Path, options: &Options, command: &str) -> Result<()> {
    let context = read_context(root, options)?;
    let head = repo::git_head(root, true)?;
    if !head.eq_ignore_ascii_case(&context.source_sha) {
        return Err("release phase checkout is not the exact clean source SHA".into());
    }
    let runtime = RuntimeIdentity::from_environment()?;
    validate_runtime_identity(&context, options.workflow_sha.as_deref(), &runtime)?;
    let job = env::var("GITHUB_JOB").map_err(|_| "GITHUB_JOB is required for release phases")?;
    validate_dispatch_job(command, &job)?;
    Ok(())
}

fn validate_dispatch_job(command: &str, actual_job: &str) -> Result<&'static str> {
    let expected_job = expected_job_for_command(command)
        .ok_or_else(|| format!("release command has no workflow job authorization: {command}"))?;
    if actual_job != expected_job {
        return Err(format!("release phase requires canonical workflow job {expected_job}").into());
    }
    Ok(expected_job)
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
    let run_id = env::var("GITHUB_RUN_ID").map_err(|_| "GITHUB_RUN_ID is required")?;
    let run_attempt =
        env::var("GITHUB_RUN_ATTEMPT").map_err(|_| "GITHUB_RUN_ATTEMPT is required")?;
    if run_id.is_empty()
        || !run_id.bytes().all(|byte| byte.is_ascii_digit())
        || run_attempt.is_empty()
        || !run_attempt.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("release state root run identity is malformed".into());
    }
    let expected = runner_temp.join(format!("sky-v4-release-{run_id}-{run_attempt}"));
    if state_root != fs::canonicalize(expected)?
        || state_root == runner_temp
        || state_root.parent() != Some(runner_temp.as_path())
    {
        return Err(
            "release state root must be the exact run-and-attempt child of RUNNER_TEMP".into(),
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
    if env::var("GITHUB_JOB").ok().as_deref() != Some("prepare") {
        return Err("release preflight must run from the prepare job".into());
    }
    let workflow_ref = env::var("GITHUB_WORKFLOW_REF")
        .map_err(|_| "GITHUB_WORKFLOW_REF is required for v4 qualification")?;
    if workflow_ref != RELEASE_WORKFLOW_REF {
        return Err("v4 release must run from the canonical release workflow on main".into());
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
    let workflow_sha = workflow_sha
        .ok_or("--workflow-sha is required for Preflight")?
        .to_ascii_lowercase();
    if workflow_sha.len() != 40 || workflow_sha != source_sha || env_sha != source_sha {
        return Err("workflow provenance SHA must equal the exact checked-out source SHA".into());
    }
    let run_id = env::var("GITHUB_RUN_ID")
        .map_err(|_| "GITHUB_RUN_ID is required for a production release")?;
    if run_id.is_empty() || !run_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("workflow run identity is malformed".into());
    }
    let run_attempt = env::var("GITHUB_RUN_ATTEMPT")
        .map_err(|_| "GITHUB_RUN_ATTEMPT is required for a production release")?
        .parse::<u32>()
        .map_err(|_| "workflow run attempt is malformed")?;
    if run_attempt == 0 || run_attempt > 100 {
        return Err("workflow run attempt is outside the supported bound".into());
    }
    Ok(Context {
        schema_version: 2,
        repository: repository.to_owned(),
        version: version_text,
        channel: channel.to_owned(),
        tag,
        release_notes_path: notes_path,
        source_sha,
        workflow_sha,
        run_id,
        run_attempt,
        created_at: utc_now(),
    })
}

fn git_text(root: &Path, args: &[&str]) -> Result<String> {
    process::capture_text_runtime_only("git", args, root)
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
    if context.schema_version != 2 || context.repository != REPOSITORY {
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
    if context.run_id.is_empty()
        || context.run_id.len() > 20
        || !context.run_id.bytes().all(|byte| byte.is_ascii_digit())
        || context.run_attempt == 0
        || context.run_attempt > 100
    {
        return Err("release context run ID must contain decimal digits only".into());
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
        if !matches_transaction(&release, &context) {
            return Err(
                "repository contains a conflicting draft for the requested release tag".into(),
            );
        }
        if release["id"].as_u64().filter(|id| *id > 0).is_none()
            || release["immutable"].as_bool() != Some(false)
        {
            return Err(
                "repository contains an owned draft with invalid id or immutable state".into(),
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
    let sentinel = json!({"schema_version":1,"repository":REPOSITORY,"run_id":context.run_id,"run_attempt":context.run_attempt,"source_sha":context.source_sha});
    write_json_atomic(&options.state_root.join("release-owner.json"), &sentinel)?;
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

fn build_unsigned(root: &Path, options: &Options) -> Result<()> {
    let context = read_context(root, options)?;
    if env::var_os("V4_TAURI_UPDATER_PRIVATE_KEY").is_some()
        || env::var_os("V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD").is_some()
    {
        return Err("unsigned build job must not receive production signing secrets".into());
    }
    let head = repo::git_head(root, true)?;
    if !head.eq_ignore_ascii_case(&context.source_sha) {
        return Err("unsigned build checkout differs from the preflight source SHA".into());
    }
    let source_installer = release_metadata::canonical_installer_name(&context.version);
    let source_signature = format!("{source_installer}.sig");
    let dist = root.join("rust/target/dist");
    let bundle = dist.join("bundle/nsis");
    purge_build_outputs(root, &dist, &bundle, &source_installer, &source_signature)?;
    let desktop = root.join("desktop");
    let remove_env = [
        "TAURI_SIGNING_PRIVATE_KEY",
        "TAURI_SIGNING_PRIVATE_KEY_PATH",
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
        "SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT",
        "SKY_AUTHENTICODE_PROVIDER",
        "SKY_AUTHENTICODE_PROVIDER_SCRIPT",
        "SKY_AUTHENTICODE_PROVIDER_COMMAND",
        "SKY_AUTHENTICODE_TEST_PFX_PATH",
        "SKY_AUTHENTICODE_TEST_PFX_PASSWORD",
        "SKY_AUTHENTICODE_TEST_THUMBPRINT",
    ]
    .map(str::to_owned);
    let install = process::capture_owned_timeout_filtered(
        std::ffi::OsStr::new("bun"),
        &["install".to_owned(), "--frozen-lockfile".to_owned()],
        &desktop,
        &[],
        &remove_env,
        RELEASE_DEPENDENCY_STEP_TIMEOUT,
    )?;
    if !install.status.success() {
        return Err(format!(
            "pinned desktop dependency installation failed with {}",
            install.status
        )
        .into());
    }
    let config_path = options.state_root.join("unsigned-build-config.json");
    fs::write(
        &config_path,
        br#"{"bundle":{"createUpdaterArtifacts":false}}"#,
    )?;
    let build_args = vec![
        "run".to_owned(),
        "tauri".to_owned(),
        "build".to_owned(),
        "--ci".to_owned(),
        "--bundles".to_owned(),
        "nsis".to_owned(),
        "--config".to_owned(),
        config_path.to_string_lossy().into_owned(),
        "--".to_owned(),
        "--profile".to_owned(),
        "dist".to_owned(),
    ];
    let build_env = vec![(
        "SKY_AUTHENTICODE_MODE".to_owned(),
        "unsigned-zero-budget".to_owned(),
    )];
    let build = process::capture_owned_timeout_filtered(
        std::ffi::OsStr::new("bun"),
        &build_args,
        &desktop,
        &build_env,
        &remove_env,
        RELEASE_BUILD_STEP_TIMEOUT,
    )?;
    if !build.status.success() {
        return Err(format!(
            "unsigned Tauri NSIS build failed with {}: {}",
            build.status,
            String::from_utf8_lossy(&build.stderr).trim()
        )
        .into());
    }
    let mut actual_bundle = fs::read_dir(&bundle)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().to_string()))
        .collect::<std::io::Result<Vec<_>>>()?;
    actual_bundle.sort();
    if actual_bundle != [source_installer.clone()] {
        return Err(
            "unsigned Tauri output must contain exactly the canonical installer and no updater sidecar".into(),
        );
    }
    let installer = bundle.join(&source_installer);
    if !fs::symlink_metadata(&installer)?.file_type().is_file() {
        return Err("unsigned Tauri installer is not a regular file".into());
    }
    let after = repo::git_head(root, true)?;
    if !after.eq_ignore_ascii_case(&context.source_sha) {
        return Err(
            "unsigned candidate build changed the checked-out release source identity".into(),
        );
    }
    let manifest = release_handoff::create_unsigned_handoff(
        &options.state_root,
        &installer,
        &root.join("desktop/src-tauri/resources/native_calibration.exe"),
        &release_handoff::ExpectedHandoffIdentity {
            repository: &context.repository,
            source_sha: &context.source_sha,
            workflow_sha: &context.workflow_sha,
            run_id: &context.run_id,
            run_attempt: context.run_attempt,
            version: &context.version,
        },
    )?;
    println!(
        "V4 Rust unsigned build handoff: PASS (installer={}, size={}, sha256={}, run={} attempt={})",
        manifest.installer_name,
        manifest.installer_size,
        manifest.installer_sha256,
        manifest.run_id,
        manifest.run_attempt
    );
    Ok(())
}

fn sign_candidate(root: &Path, options: &Options) -> Result<()> {
    let context = read_context(root, options)?;
    let runtime = release_handoff::WorkflowIdentity::from_environment()?;
    let incoming = options.state_root.join("incoming-unsigned");
    let expected = release_handoff::ExpectedHandoffIdentity {
        repository: &context.repository,
        source_sha: &context.source_sha,
        workflow_sha: &context.workflow_sha,
        run_id: &context.run_id,
        run_attempt: context.run_attempt,
        version: &context.version,
    };
    let (handoff, input_installer) =
        release_handoff::validate_unsigned_handoff(root, &incoming, &expected, &runtime)?;
    let input_signature = release_signing::sign_installer(
        root,
        &input_installer,
        &context.version,
        handoff.installer_size,
        &handoff.installer_sha256,
        &context.run_id,
        context.run_attempt,
    )?;

    let candidate_bundle = options.state_root.join("candidate-bundle");
    fs::create_dir(&candidate_bundle)?;
    let source_installer = handoff.installer_name.clone();
    let source_signature = format!("{source_installer}.sig");
    let signature_sha256 = crate::hash::sha256(&input_signature)?;
    let installer_metadata = fs::symlink_metadata(&input_installer)?;
    if !installer_metadata.file_type().is_file() {
        return Err("unsigned handoff installer is not a regular file".into());
    }
    let frozen_installer = candidate_bundle.join(&source_installer);
    let frozen_signature = candidate_bundle.join(&source_signature);
    fs::copy(&input_installer, &frozen_installer)?;
    fs::copy(&input_signature, &frozen_signature)?;
    fs::remove_file(input_signature)?;
    if crate::hash::sha256(&frozen_installer)? != handoff.installer_sha256 {
        return Err("frozen installer changed while entering the signing output state".into());
    }
    if crate::hash::sha256(&frozen_signature)? != signature_sha256 {
        return Err(
            "frozen updater signature changed while entering the signing output state".into(),
        );
    }
    println!(
        "V4 Rust protected signing phase: PASS (signature verified; unsigned installer SHA-256 preserved)"
    );
    Ok(())
}

fn qualify_candidate(root: &Path, options: &Options) -> Result<()> {
    let context = read_context(root, options)?;
    let runtime = release_handoff::WorkflowIdentity::from_environment()?;
    let incoming = options.state_root.join("incoming-unsigned");
    let expected = release_handoff::ExpectedHandoffIdentity {
        repository: &context.repository,
        source_sha: &context.source_sha,
        workflow_sha: &context.workflow_sha,
        run_id: &context.run_id,
        run_attempt: context.run_attempt,
        version: &context.version,
    };
    let (handoff, _input_installer) =
        release_handoff::validate_unsigned_handoff(root, &incoming, &expected, &runtime)?;
    let candidate_bundle = options.state_root.join("candidate-bundle");
    let candidate_evidence = options.state_root.join("candidate-evidence");
    let source_installer = handoff.installer_name.clone();
    let source_signature = format!("{source_installer}.sig");
    let mut bundle_names = fs::read_dir(&candidate_bundle)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<Vec<_>>>()?;
    bundle_names.sort();
    let mut expected_names = vec![source_installer.clone(), source_signature.clone()];
    expected_names.sort();
    if bundle_names != expected_names {
        return Err(
            "signed candidate bundle must contain exactly the canonical installer and signature"
                .into(),
        );
    }
    fs::create_dir(&candidate_evidence)?;
    let frozen_installer = candidate_bundle.join(&source_installer);
    let frozen_signature = candidate_bundle.join(&source_signature);
    let signature_metadata = crate::ci_artifacts::require_regular_file(
        &frozen_signature,
        "signed candidate updater signature",
    )?;
    if signature_metadata.len() == 0 || signature_metadata.len() > 16 * 1024 {
        return Err("signed candidate signature is empty or exceeds its size bound".into());
    }
    if crate::hash::sha256(&frozen_installer)? != handoff.installer_sha256 {
        return Err("signed installer differs from the original unsigned build handoff".into());
    }
    crate::updater_trust::verify_updater_signature_for_version(
        &frozen_installer,
        &frozen_signature,
        &context.version,
    )?;
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
            "--expected-version",
            &context.version,
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
            incoming
                .join(&handoff.calibration_binary)
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
        schema_version: 2,
        repository: context.repository.clone(),
        source_sha: context.source_sha.clone(),
        workflow_sha: context.workflow_sha.clone(),
        run_id: context.run_id.clone(),
        run_attempt: context.run_attempt,
        version: context.version.clone(),
        channel: context.channel.clone(),
        tag: context.tag.clone(),
        unsigned_handoff: handoff,
        qualification_assets: records,
        public_assets,
    };
    write_json_atomic(
        &options.state_root.join("candidate-manifest.json"),
        &manifest,
    )?;
    let state = ReleaseState {
        schema_version: 2,
        repository: context.repository,
        version: context.version,
        channel: context.channel,
        tag: context.tag,
        source_sha: context.source_sha,
        workflow_sha: context.workflow_sha,
        run_id: context.run_id,
        run_attempt: context.run_attempt,
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

fn verify_candidate_handoff(root: &Path, options: &Options) -> Result<()> {
    let context = read_context(root, options)?;
    let head = repo::git_head(root, true)?;
    if !head.eq_ignore_ascii_case(&context.source_sha) {
        return Err("attestation checkout is not the exact clean release source SHA".into());
    }
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    validate_state(&state, &context)?;
    if state.phase != Phase::Ready
        || state.attested
        || state.release_id.is_some()
        || state.published_at.is_some()
        || state.promoted_at.is_some()
        || state.final_verified
    {
        return Err("candidate handoff is not an unpublished, unattested READY state".into());
    }
    let manifest = read_manifest(root, options, &context)?;
    let installer = manifest
        .public_assets
        .iter()
        .find(|record| record.role == "installer")
        .ok_or("candidate handoff has no installer")?;
    let signature = manifest
        .public_assets
        .iter()
        .find(|record| record.role == "updater-signature")
        .ok_or("candidate handoff has no updater signature")?;
    crate::updater_trust::verify_updater_signature_for_version(
        &safe_state_asset_path(&options.state_root, installer)?,
        &safe_state_asset_path(&options.state_root, signature)?,
        &context.version,
    )?;
    println!(
        "V4 Rust qualified artifact handoff: PASS (source={}, run={} attempt={}, installer and signature rehashed against the typed manifest)",
        context.source_sha, context.run_id, context.run_attempt
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

static NEXT_DOCTOR_METADATA_TEMP_SEQUENCE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

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
        &[
            "V4_TAURI_UPDATER_PRIVATE_KEY".to_owned(),
            "V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD".to_owned(),
        ],
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
        &[
            "V4_TAURI_UPDATER_PRIVATE_KEY".to_owned(),
            "V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD".to_owned(),
        ],
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
        "repository":context.repository,
        "source_sha":context.source_sha,
        "workflow_sha":context.workflow_sha,
        "run_id":context.run_id,
        "run_attempt":context.run_attempt,
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
        "repository":context.repository,
        "source_sha":context.source_sha,
        "workflow_sha":context.workflow_sha,
        "run_id":context.run_id,
        "run_attempt":context.run_attempt,
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
        || production["repository"] != context.repository
        || production["workflow_sha"] != context.workflow_sha
        || production["run_id"] != context.run_id
        || production["run_attempt"].as_u64() != Some(u64::from(context.run_attempt))
        || production["version"] != context.version
        || production["channel"] != context.channel
        || production["authenticode_mode"] != "unsigned-zero-budget"
        || production["authenticode_state"] != "unsigned"
        || production["authenticode_provider"] != "none"
        || !production["approved_signer_thumbprint"].is_null()
        || !production["observed_signer_thumbprint"].is_null()
        || production["updater_signature_status"] != "valid"
        || production["updater_key_id"] != crate::updater_trust::canonical_key_id()?
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
        "repository",
        "source_sha",
        "workflow_sha",
        "run_id",
        "run_attempt",
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
        || evidence["repository"] != context.repository
        || evidence["source_sha"] != context.source_sha
        || evidence["workflow_sha"] != context.workflow_sha
        || evidence["run_id"] != context.run_id
        || evidence["run_attempt"].as_u64() != Some(u64::from(context.run_attempt))
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

fn matches_transaction(release: &Value, context: &Context) -> bool {
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
    for key in [
        "repository",
        "run_id",
        "run_attempt",
        "source_sha",
        "version",
        "tag",
    ] {
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
        && value["run_attempt"].as_u64() == Some(u64::from(context.run_attempt))
        && value["run_id"]
            .as_str()
            .is_some_and(|run| !run.is_empty() && run == context.run_id)
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
    if manifest.schema_version != 2 || manifest.repository_mismatch(context) {
        return Err("candidate manifest release identity differs from release context".into());
    }
    if manifest.qualification_assets.len() != 8 || manifest.public_assets.len() != 2 {
        return Err(
            "candidate manifest must contain eight qualification files and two public assets"
                .into(),
        );
    }
    let expected_handoff = release_handoff::ExpectedHandoffIdentity {
        repository: &context.repository,
        source_sha: &context.source_sha,
        workflow_sha: &context.workflow_sha,
        run_id: &context.run_id,
        run_attempt: context.run_attempt,
        version: &context.version,
    };
    release_handoff::validate_manifest_identity(&manifest.unsigned_handoff, &expected_handoff)?;
    let signed_installer = manifest
        .public_assets
        .iter()
        .find(|record| record.role == "installer")
        .ok_or("candidate manifest has no signed installer")?;
    if signed_installer.size != manifest.unsigned_handoff.installer_size
        || signed_installer.sha256 != manifest.unsigned_handoff.installer_sha256
        || signed_installer.source_name != manifest.unsigned_handoff.installer_name
    {
        return Err("signed installer does not preserve the build-job handoff bytes".into());
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
        self.repository != context.repository
            || self.source_sha != context.source_sha
            || self.workflow_sha != context.workflow_sha
            || self.run_id != context.run_id
            || self.run_attempt != context.run_attempt
            || self.version != context.version
            || self.channel != context.channel
            || self.tag != context.tag
    }
}

fn publish_release(root: &Path, options: &Options, transport: &mut dyn Transport) -> Result<()> {
    publish_release_with_signature_verifier(
        root,
        options,
        transport,
        crate::updater_trust::verify_updater_signature_for_version,
    )
}

fn publish_release_with_mock_verifier(
    root: &Path,
    options: &Options,
    transport: &mut dyn Transport,
) -> Result<()> {
    publish_release_with_signature_verifier(
        root,
        options,
        transport,
        |installer, signature, version| {
            let installer_name = release_metadata::canonical_installer_name(version);
            let signature_name = format!("{installer_name}.sig");
            if installer.file_name().and_then(|name| name.to_str()) != Some(&installer_name)
                || signature.file_name().and_then(|name| name.to_str()) != Some(&signature_name)
                || !installer.is_file()
                || !signature.is_file()
            {
                return Err(
                    "mock signature verifier received unexpected downloaded asset names".into(),
                );
            }
            Ok(())
        },
    )
}

fn publish_release_with_signature_verifier(
    root: &Path,
    options: &Options,
    transport: &mut dyn Transport,
    mut verify_signature: impl FnMut(&Path, &Path, &str) -> Result<()>,
) -> Result<()> {
    let context = read_context(root, options)?;
    let mut state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    if state.phase != Phase::Ready || !state.attested {
        return Err("PublishRelease requires a READY candidate state".into());
    }
    validate_state(&state, &context)?;
    let manifest = read_manifest(root, options, &context)?;
    let latest_before = read_json::<Value>(&options.state_root.join("preflight-evidence.json"))?["pre_publication_latest"].clone();
    validate_stable_latest(&latest_before)?;
    prepublication_metadata_qualification(
        root,
        options,
        &context,
        &manifest,
        transport,
        &latest_before,
    )?;
    let notes = fs::read_to_string(root.join(&context.release_notes_path))?;
    let marker = format!(
        "<!-- v4-release-tx: {} -->",
        serde_json::to_string(&json!({
            "repository":context.repository,"run_id":context.run_id,"source_sha":context.source_sha,
            "run_attempt":context.run_attempt,"version":context.version,"tag":context.tag
        }))?
    );
    let body = format!("{}\n\n{marker}", notes.trim());

    // Reconcile only a matching unpublished draft at the first publication mutation boundary.
    if let Some(stale) = transport.request(
        "GET",
        &format!("repos/{REPOSITORY}/releases/tags/{}", context.tag),
        None,
    )? {
        if !matches_transaction(&stale, &context) {
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
                Some(release) if matches_transaction(&release, &context) => release,
                Some(_) => return Err("release draft POST outcome is ambiguous and remote draft identity does not match this transaction".into()),
                None => return Err("release draft POST failed and no exact same-transaction draft could be reconciled".into()),
            }
        }
    };
    let release_id = created["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or("GitHub draft has no valid release id")?;
    if !matches_transaction(&created, &context) {
        return Err("GitHub draft response has mismatched transaction identity".into());
    }
    let upload_url = created["upload_url"]
        .as_str()
        .ok_or("GitHub draft has no upload URL")?
        .replace("{?name,label}", "");

    let before_publish = (|| -> Result<()> {
        for record in &manifest.public_assets {
            let path = verify_record(&options.state_root, record)?;
            if let Err(upload_error) = transport.upload(&upload_url, &record.release_name, &path)
                && !reconcile_uncertain_upload(transport, release_id, &context, record)?
            {
                return Err(upload_error);
            }
        }
        let draft = required_api(
            transport,
            "GET",
            &format!("repos/{REPOSITORY}/releases/{release_id}"),
            None,
        )?;
        verify_draft_assets_before_publication(
            options,
            transport,
            &draft,
            &manifest.public_assets,
            &context,
            release_id,
            &mut verify_signature,
        )?;
        prepublication_metadata_qualification(
            root,
            options,
            &context,
            &manifest,
            transport,
            &latest_before,
        )?;
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
            Ok(Some(remote)) if remote["draft"] == true && matches_transaction(&remote, &context) => {
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

fn verify_draft_assets_before_publication(
    options: &Options,
    transport: &mut dyn Transport,
    draft: &Value,
    expected: &[CandidateRecord],
    context: &Context,
    release_id: u64,
    verify_signature: &mut dyn FnMut(&Path, &Path, &str) -> Result<()>,
) -> Result<()> {
    if draft["id"].as_u64() != Some(release_id)
        || draft["draft"] != true
        || draft["immutable"] != false
        || !matches_transaction(draft, context)
    {
        return Err("draft asset readback requires the exact owned unpublished release".into());
    }
    verify_public_assets(transport, draft, expected, false)?;
    let installer_name = release_metadata::canonical_installer_name(&context.version);
    let installer_release_name =
        release_metadata::canonical_release_installer_name(&context.version);
    let signature_name = format!("{installer_name}.sig");
    let signature_release_name = format!("{installer_release_name}.sig");
    let installer = expected
        .iter()
        .find(|record| record.role == "installer")
        .ok_or("candidate manifest lacks the installer for draft readback")?;
    let signature = expected
        .iter()
        .find(|record| record.role == "updater-signature")
        .ok_or("candidate manifest lacks the updater signature for draft readback")?;
    if expected.len() != 2
        || installer.source_name != installer_name
        || installer.release_name != installer_release_name
        || signature.source_name != signature_name
        || signature.release_name != signature_release_name
    {
        return Err(
            "draft readback asset source/public names differ from the canonical v4 pair".into(),
        );
    }

    let directory = options.state_root.join("publication-readback");
    fs::create_dir(&directory)?;
    let verification = (|| -> Result<()> {
        for (record, source_name) in [
            (installer, installer_name.as_str()),
            (signature, signature_name.as_str()),
        ] {
            let asset = draft["assets"]
                .as_array()
                .and_then(|assets| {
                    assets
                        .iter()
                        .find(|asset| asset["name"] == record.release_name)
                })
                .ok_or("draft asset disappeared before exact-byte readback")?;
            let url = asset["url"]
                .as_str()
                .ok_or("draft asset API URL is missing")?;
            let destination = directory.join(source_name);
            transport.download(asset_api_endpoint(url)?, &destination)?;
            let metadata = fs::symlink_metadata(&destination)?;
            if !metadata.file_type().is_file()
                || metadata.len() != record.size
                || crate::hash::sha256(&destination)? != record.sha256
            {
                return Err(format!(
                    "draft asset readback differs from the frozen candidate: {}",
                    record.release_name
                )
                .into());
            }
        }
        verify_signature(
            &directory.join(&installer_name),
            &directory.join(&signature_name),
            &context.version,
        )?;
        Ok(())
    })();
    let cleanup = fs::remove_dir_all(&directory);
    match (verification, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(format!("draft readback cleanup failed: {error}").into()),
        (Err(error), Err(cleanup_error)) => {
            Err(format!("{error}; draft readback cleanup failed: {cleanup_error}").into())
        }
    }
}

fn prepublication_metadata_qualification(
    root: &Path,
    options: &Options,
    context: &Context,
    manifest: &CandidateManifest,
    transport: &mut dyn Transport,
    preflight_latest: &Value,
) -> Result<()> {
    let signature_name = format!(
        "{}.sig",
        release_metadata::canonical_installer_name(&context.version)
    );
    let signature = manifest
        .qualification_assets
        .iter()
        .find(|record| record.role == "updater-signature" && record.source_name == signature_name)
        .ok_or("candidate manifest lacks the canonical signature for metadata dry-run")?;
    let candidate_path = options
        .state_root
        .join("prepublication-candidate-metadata.json");
    generate_expected_metadata(
        root,
        options,
        context,
        &json!({"published_at":"2000-01-01T00:00:00Z"}),
        signature,
        &candidate_path,
    )?;
    let channel = release_metadata::Channel::parse(&context.channel)?;
    release_metadata::validate(channel, &candidate_path)?;

    let endpoint = format!(
        "repos/{REPOSITORY}/contents/channels/{}/latest.json?ref=release-metadata",
        context.channel
    );
    if let Some(existing) = transport.request("GET", &endpoint, None)? {
        let current_path = options
            .state_root
            .join(format!("prepublication-current-{}.json", context.channel));
        fs::write(&current_path, decode_contents(&existing)?)?;
        release_metadata::validate(channel, &current_path)?;
        release_metadata::validate_monotonic(channel, &current_path, &candidate_path)?;
    }

    let current_latest = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/releases/latest"),
        None,
    )?;
    validate_stable_latest(&current_latest)?;
    if !same_github_latest(preflight_latest, &current_latest) {
        return Err(
            "GitHub Latest changed after Preflight; refusing release publication on stale qualification evidence"
                .into(),
        );
    }
    if context.channel == "stable" {
        let current_tag = current_latest["tag_name"]
            .as_str()
            .and_then(|tag| tag.strip_prefix('v'))
            .ok_or("GitHub Latest tag is not a canonical v-prefixed SemVer")?;
        if version::parse(&context.version)? <= version::parse(current_tag)? {
            return Err(format!(
                "stable candidate {} is not strictly newer than GitHub Latest {}",
                context.version, current_tag
            )
            .into());
        }
    }
    println!(
        "V4 pre-publication metadata qualification: PASS (channel={}, candidate={}, GitHub Latest unchanged)",
        context.channel, context.version
    );
    Ok(())
}

fn same_github_latest(before: &Value, after: &Value) -> bool {
    before["id"].as_u64().is_some()
        && before["id"] == after["id"]
        && before["tag_name"] == after["tag_name"]
        && before["target_commitish"]
            .as_str()
            .zip(after["target_commitish"].as_str())
            .is_some_and(|(before, after)| before.eq_ignore_ascii_case(after))
        && before["published_at"] == after["published_at"]
}

fn reconcile_uncertain_upload(
    transport: &mut dyn Transport,
    release_id: u64,
    context: &Context,
    record: &CandidateRecord,
) -> Result<bool> {
    let endpoint = format!("repos/{REPOSITORY}/releases/{release_id}");
    let release = required_api(transport, "GET", &endpoint, None).map_err(|error| {
        format!(
            "release asset upload outcome is unknown and exact release readback failed: {error}"
        )
    })?;
    if release["id"].as_u64() != Some(release_id)
        || release["draft"] != true
        || release["immutable"].as_bool() != Some(false)
        || !matches_transaction(&release, context)
    {
        return Err(
            "release asset upload outcome is unknown and exact draft identity failed validation"
                .into(),
        );
    }
    let assets = release["assets"]
        .as_array()
        .ok_or("release asset upload reconciliation omitted the assets array")?;
    let matching = assets
        .iter()
        .filter(|asset| asset["name"] == record.release_name)
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return Ok(false);
    }
    if matching.len() != 1 {
        return Err(format!(
            "release asset upload outcome is ambiguous: duplicate {} assets",
            record.release_name
        )
        .into());
    }
    let asset = matching[0];
    let digest = asset["digest"]
        .as_str()
        .and_then(|value| value.strip_prefix("sha256:"));
    if asset["size"].as_u64() != Some(record.size)
        || asset["state"] != "uploaded"
        || digest.is_none_or(|value| !value.eq_ignore_ascii_case(&record.sha256))
    {
        return Err(format!(
            "release asset upload outcome is unknown and exact asset validation failed: {}",
            record.release_name
        )
        .into());
    }
    Ok(true)
}

fn validate_state(state: &ReleaseState, context: &Context) -> Result<()> {
    if state.schema_version != 2
        || state.repository != context.repository
        || state.version != context.version
        || state.channel != context.channel
        || state.tag != context.tag
        || state.source_sha != context.source_sha
        || state.workflow_sha != context.workflow_sha
        || state.run_id != context.run_id
        || state.run_attempt != context.run_attempt
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
    if draft["id"].as_u64() != Some(id)
        || draft["immutable"].as_bool() != Some(false)
        || !matches_transaction(draft, context)
    {
        return Err("refusing draft cleanup because transaction ownership did not match".into());
    }
    let endpoint = format!("repos/{REPOSITORY}/releases/{id}");
    let current = transport.request("GET", &endpoint, None)?;
    let Some(current) = current else {
        return Ok(());
    };
    if current["id"].as_u64() != Some(id)
        || current["draft"] != true
        || current["immutable"].as_bool() != Some(false)
        || !matches_transaction(&current, context)
    {
        return Err(
            "refusing draft deletion because exact-id ownership/draft state changed".into(),
        );
    }
    let delete_result = transport.request("DELETE", &endpoint, None);
    match transport.request("GET", &endpoint, None) {
        Ok(None) => Ok(()),
        Ok(Some(_remote)) => {
            let detail = delete_result
                .err()
                .map(|error| format!("; DELETE outcome: {error}"))
                .unwrap_or_default();
            Err(
                format!("owned unpublished draft was not removed by exact release id{detail}")
                    .into(),
            )
        }
        Err(read_error) => Err(format!(
            "draft DELETE outcome is unknown and exact-id readback failed: {read_error}"
        )
        .into()),
    }
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

fn await_raw_metadata_convergence(
    expected: &[u8],
    budget: Duration,
    interval: Duration,
    mut get: impl FnMut() -> Result<Option<Vec<u8>>>,
    mut sleep: impl FnMut(Duration),
    mut elapsed: impl FnMut() -> Duration,
) -> Result<()> {
    if budget.is_zero() || interval.is_zero() || interval > budget {
        return Err("raw metadata retry budget and interval must be positive and bounded".into());
    }
    let expected_sha = sha256_hex(expected);
    let mut last_http = "not-observed".to_owned();
    let mut last_sha = "none".to_owned();
    loop {
        if elapsed() >= budget {
            break;
        }
        match get() {
            Ok(Some(bytes)) => {
                last_http = "200".to_owned();
                last_sha = sha256_hex(&bytes);
                if bytes == expected {
                    println!("V4 raw metadata convergence: PASS (sha256={expected_sha})");
                    return Ok(());
                }
            }
            Ok(None) => {
                last_http = "404".to_owned();
                last_sha = "none".to_owned();
            }
            Err(error) => {
                last_http = error
                    .downcast_ref::<HttpFailure>()
                    .and_then(|failure| failure.status)
                    .map(|status| status.to_string())
                    .unwrap_or_else(|| "transport-error".to_owned());
                last_sha = "none".to_owned();
            }
        }
        let now = elapsed();
        if now >= budget {
            break;
        }
        sleep(interval.min(budget.saturating_sub(now)));
    }
    Err(format!(
        "public raw metadata endpoint did not converge within {}s: expected_sha256={expected_sha} last_http={last_http} last_observed_sha256={last_sha}",
        budget.as_secs_f64()
    )
    .into())
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
    validate_latest_for_metadata_promotion(options, &context, &state, &published, transport)?;
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
    let expected_bytes = fs::read(&output_path)?;
    let content = base64::engine::general_purpose::STANDARD.encode(&expected_bytes);
    let mut payload = json!({"message":format!("promote {} metadata for {}", context.channel, context.tag),"content":content,"branch":"release-metadata"});
    if let Some(sha) = current_sha {
        payload["sha"] = Value::String(sha);
    }
    let put_endpoint = format!(
        "repos/{REPOSITORY}/contents/channels/{}/latest.json",
        context.channel
    );
    let put_result = transport.request("PUT", &put_endpoint, Some(&payload));
    let stored = match put_result {
        Ok(Some(_)) => required_api(transport, "GET", &endpoint, None)?,
        Ok(None) => reconcile_metadata_put(
            transport,
            &endpoint,
            &expected_bytes,
            "successful PUT response omitted JSON",
        )?,
        Err(error) => {
            reconcile_metadata_put(transport, &endpoint, &expected_bytes, &error.to_string())?
        }
    };
    if decode_contents(&stored)? != expected_bytes {
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

fn validate_latest_for_metadata_promotion(
    options: &Options,
    context: &Context,
    state: &ReleaseState,
    published: &Value,
    transport: &mut dyn Transport,
) -> Result<()> {
    let current_latest = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/releases/latest"),
        None,
    )?;
    validate_stable_latest(&current_latest)?;
    if context.channel == "stable" {
        if state.release_id != current_latest["id"].as_u64()
            || current_latest["tag_name"] != context.tag
            || !current_latest["target_commitish"]
                .as_str()
                .is_some_and(|sha| sha.eq_ignore_ascii_case(&context.source_sha))
            || !same_github_latest(published, &current_latest)
        {
            return Err(
                "GitHub Latest changed after publication; refusing channel metadata promotion"
                    .into(),
            );
        }
    } else {
        let preflight_latest: Value = read_json::<Value>(
            &options.state_root.join("preflight-evidence.json"),
        )?["pre_publication_latest"]
            .clone();
        validate_stable_latest(&preflight_latest)?;
        if !same_github_latest(&preflight_latest, &current_latest) {
            return Err(
                "GitHub Latest changed while beta metadata promotion was waiting; refusing channel metadata promotion"
                    .into(),
            );
        }
    }
    Ok(())
}

fn reconcile_metadata_put(
    transport: &mut dyn Transport,
    endpoint: &str,
    expected_bytes: &[u8],
    put_error: &str,
) -> Result<Value> {
    let readback = transport
        .request("GET", endpoint, None)
        .map_err(|error| {
            format!(
                "metadata PUT outcome is unknown ({put_error}) and exact Contents readback failed: {error}"
            )
        })?
        .ok_or_else(|| {
            format!(
                "metadata PUT outcome is unknown ({put_error}); candidate bytes are absent and no retry was issued"
            )
        })?;
    if decode_contents(&readback)? != expected_bytes {
        return Err(format!(
            "metadata PUT outcome is unknown ({put_error}); exact Contents readback does not match candidate bytes and no retry was issued"
        )
        .into());
    }
    Ok(readback)
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
    let started = Instant::now();
    await_raw_metadata_convergence(
        &expected_bytes,
        RAW_METADATA_RETRY_BUDGET,
        RAW_METADATA_RETRY_INTERVAL,
        || transport.raw_get(&raw_url),
        std::thread::sleep,
        || started.elapsed(),
    )?;

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

fn probe_metadata_token(contents_permission: &str, transport: &mut dyn Transport) -> Result<()> {
    if contents_permission != "write" {
        return Err(
            "pre-publication metadata App token was not minted with contents: write".into(),
        );
    }
    let repository = required_api(transport, "GET", &format!("repos/{REPOSITORY}"), None)?;
    if !repository["full_name"]
        .as_str()
        .is_some_and(|name| name.eq_ignore_ascii_case(REPOSITORY))
    {
        return Err("metadata App token returned the wrong canonical repository identity".into());
    }
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
    for channel in ["stable", "beta"] {
        let endpoint = format!(
            "repos/{REPOSITORY}/contents/channels/{channel}/latest.json?ref=release-metadata"
        );
        if let Some(existing) = transport.request("GET", &endpoint, None)? {
            doctor_validate_metadata(&decode_contents(&existing)?, channel).map_err(|error| {
                format!("metadata App token returned invalid {channel} channel metadata: {error}")
            })?;
        }
    }
    println!(
        "V4 Rust metadata App token probe: PASS (contents:write mint succeeded; exact repository scope; GET-only branch, bootstrap, stable, and beta access)"
    );
    Ok(())
}

fn cleanup_local_state(root: &Path, options: &Options) -> Result<()> {
    if !options.state_root.exists() {
        return Ok(());
    }
    let marker_path = options.state_root.join("release-owner.json");
    if !marker_path.exists() {
        println!("V4 Rust runner-local state cleanup: skipped (no Rust ownership marker)");
        return Ok(());
    }
    let context: Context = read_json(&options.state_root.join("release-context.json"))?;
    let marker: Value = read_json(&marker_path)?;
    if context.repository != REPOSITORY
        || marker["repository"] != REPOSITORY
        || marker["run_id"] != context.run_id
        || marker["run_attempt"].as_u64() != Some(u64::from(context.run_attempt))
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
    let expected_state = temp.join(format!(
        "sky-v4-release-{}-{}",
        context.run_id, context.run_attempt
    ));
    if state != fs::canonicalize(expected_state)? || state.parent() != Some(temp.as_path()) {
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
            .is_some_and(|schema| !matches!(schema, 1 | 2))
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
    let (path, mut file) = loop {
        let base = unique_temp(&env::temp_dir(), "sky-release-doctor-metadata");
        let sequence =
            NEXT_DOCTOR_METADATA_TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let file_name = format!(
            "{}-{sequence}",
            base.file_name()
                .and_then(|name| name.to_str())
                .ok_or("metadata validation temporary filename is invalid")?
        );
        let path = base.with_file_name(file_name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => break (path, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    };
    let validation = (|| -> Result<()> {
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
        mock_draft_readback_failures_never_patch(),
        mock_non_monotonic_candidate_never_mutates(),
        mock_draft_post_timeout_reconciles_exact_marker(),
        mock_upload_timeout_reconciles_exact_asset(),
        mock_complete_transaction_and_raw_metadata(),
        mock_metadata_promotion_is_idempotent_after_committed_timeout(),
        mock_metadata_promotion_rejects_changed_latest(),
        mock_publication_identity_and_asset_inventory_fail_closed(),
        mock_pr_merge_sha_does_not_break_release_or_owned_cleanup(),
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
        mutation_adapter_is_at_most_once(),
        raw_metadata_retry_converges_after_cache_lag(),
        raw_metadata_retry_fails_with_bounded_diagnostics(),
        runtime_identity_fails_closed_without_exact_workflow_context(),
        workflow_run_identity_fails_closed(),
        timestamp_normalization_is_utc_and_strict(),
    ];
    let scenario_count = outcomes.len();
    for outcome in outcomes {
        outcome?;
    }
    println!(
        "V4 Rust release state-machine mock self-test: PASS ({} scenarios)",
        scenario_count
    );
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
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
    assert_eq!(state.phase, Phase::PublishedPendingMetadata);
    assert_eq!(state.release_id, Some(12));
    assert!(state.qualified_after_download);
    let post = github
        .events
        .iter()
        .position(|event| event == "PATCH repos/pumni/Sky-Auto-Player/releases/12")
        .ok_or("publish PATCH missing")?;
    let downloads = github
        .events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.starts_with("DOWNLOAD "))
        .collect::<Vec<_>>();
    assert_eq!(
        downloads.len(),
        2,
        "draft installer and signature are read back"
    );
    assert!(downloads.iter().all(|(index, _)| *index < post));
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

fn mock_draft_readback_failures_never_patch() -> Result<()> {
    for failure in 0..4 {
        let (root, options, context, manifest) = fixture()?;
        let mut github = FakeTransport::new();
        github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
        prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
        let result = match failure {
            0 => {
                github.fail_download = true;
                publish_release_with_mock_verifier(&root, &options, &mut github)
            }
            1 => {
                github.corrupt_download = true;
                publish_release_with_mock_verifier(&root, &options, &mut github)
            }
            2 => {
                github.corrupt_signature_download = true;
                publish_release_with_mock_verifier(&root, &options, &mut github)
            }
            _ => publish_release_with_signature_verifier(
                &root,
                &options,
                &mut github,
                |installer, signature, version| {
                    let expected_installer = release_metadata::canonical_installer_name(version);
                    let expected_signature = format!("{expected_installer}.sig");
                    if installer.file_name().and_then(|name| name.to_str())
                        != Some(expected_installer.as_str())
                        || signature.file_name().and_then(|name| name.to_str())
                            != Some(expected_signature.as_str())
                    {
                        return Err(
                            "downloaded verifier input filenames were not source names".into()
                        );
                    }
                    Err("injected cryptographic signature verification failure".into())
                },
            ),
        };
        let error = result.expect_err("failed draft readback must block publication");
        match failure {
            0 => assert!(
                error
                    .to_string()
                    .contains("injected draft asset download failure")
            ),
            1 => assert!(error.to_string().contains("draft asset readback differs")),
            2 => {
                assert!(!github.corrupt_signature_download);
                assert!(error.to_string().contains("draft asset readback differs"));
            }
            _ => assert!(
                error
                    .to_string()
                    .contains("injected cryptographic signature")
            ),
        }
        assert!(
            github
                .events
                .iter()
                .all(|event| event != "PATCH repos/pumni/Sky-Auto-Player/releases/12")
        );
        let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
        assert!(!state.qualified_after_download);
        assert_eq!(github.deletions, [12]);
        let _ = fs::remove_dir_all(&options.state_root);
    }
    Ok(())
}

fn mock_non_monotonic_candidate_never_mutates() -> Result<()> {
    {
        let (root, options, context, manifest) = fixture()?;
        let mut github = FakeTransport::new();
        github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
        let signature = manifest
            .qualification_assets
            .iter()
            .find(|record| record.role == "updater-signature")
            .ok_or("fixture signature missing")?;
        let current_path = options.state_root.join("same-version-latest.json");
        release_metadata::generate(release_metadata::GenerateInput {
            channel: release_metadata::Channel::parse(&context.channel)?,
            version: &context.version,
            notes_path: &root.join(&context.release_notes_path),
            pub_date: "2026-01-01T00:00:00Z",
            platform: release_metadata::WINDOWS_PLATFORM,
            asset_url: &release_metadata::canonical_asset_url(&context.version),
            signature_path: &safe_state_asset_path(&options.state_root, signature)?,
            output: &current_path,
        })?;
        github
            .metadata_files
            .insert(context.channel.clone(), fs::read(&current_path)?);
        prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
        assert!(publish_release_with_mock_verifier(&root, &options, &mut github).is_err());
        assert!(github.events.iter().all(|event| {
            !(event.starts_with("POST ")
                || event.starts_with("PATCH ")
                || event.starts_with("PUT ")
                || event.starts_with("DELETE ")
                || event.starts_with("UPLOAD "))
        }));
        let _ = fs::remove_dir_all(&options.state_root);
    }
    {
        let (root, options, context, manifest) = fixture()?;
        let mut github = FakeTransport::new();
        github.latest = fake_release(11, &context.tag, &context.source_sha, false, false, true);
        prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
        assert!(publish_release_with_mock_verifier(&root, &options, &mut github).is_err());
        assert!(github.events.iter().all(|event| {
            !(event.starts_with("POST ")
                || event.starts_with("PATCH ")
                || event.starts_with("PUT ")
                || event.starts_with("DELETE ")
                || event.starts_with("UPLOAD "))
        }));
        let _ = fs::remove_dir_all(&options.state_root);
    }
    Ok(())
}

fn mock_draft_post_timeout_reconciles_exact_marker() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.post_created_then_timeout = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
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

fn mock_upload_timeout_reconciles_exact_asset() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    github.fail_upload_after_commit = true;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    assert_eq!(
        github
            .events
            .iter()
            .filter(|event| event.starts_with("UPLOAD "))
            .count(),
        manifest.public_assets.len(),
        "committed upload timeout must be read back rather than blindly replayed"
    );
    let first_upload = github
        .events
        .iter()
        .position(|event| event.starts_with("UPLOAD "))
        .ok_or("expected first upload event")?;
    let reconcile = github
        .events
        .iter()
        .position(|event| event == "GET repos/pumni/Sky-Auto-Player/releases/12")
        .ok_or("exact release readback after upload timeout missing")?;
    let second_upload = github
        .events
        .iter()
        .enumerate()
        .skip(first_upload + 1)
        .find(|(_, event)| event.starts_with("UPLOAD "))
        .map(|(index, _)| index)
        .ok_or("expected second upload event")?;
    assert!(first_upload < reconcile && reconcile < second_upload);
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_complete_transaction_and_raw_metadata() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
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
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    github.fail_metadata_put_after_write = true;
    promote_metadata(&root, &options, &mut github)?;
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

fn mock_metadata_promotion_rejects_changed_latest() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
    github.latest = fake_release(13, "v4.4.0", "c".repeat(40), false, false, true);

    let error = promote_metadata(&root, &options, &mut github)
        .expect_err("changed GitHub Latest must block metadata promotion");
    assert!(
        error
            .to_string()
            .contains("GitHub Latest changed after publication")
    );
    assert!(github.events.iter().all(|event| !event.starts_with("PUT ")));
    let _ = fs::remove_dir_all(&options.state_root);
    Ok(())
}

fn mock_publication_identity_and_asset_inventory_fail_closed() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
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
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
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

fn mutation_adapter_is_at_most_once() -> Result<()> {
    for method in ["POST", "PATCH", "PUT", "DELETE"] {
        let attempts = std::cell::Cell::new(0);
        let error = request_with_retry_policy(
            method,
            "repos/pumni/Sky-Auto-Player/releases/12",
            || {
                attempts.set(attempts.get() + 1);
                Err::<(), _>(Box::new(HttpFailure {
                    status: Some(504),
                    detail: "injected committed mutation timeout".to_owned(),
                })
                    as Box<dyn std::error::Error + Send + Sync>)
            },
            |_| panic!("mutations must never enter the retry sleep path"),
        )
        .expect_err("mutation failures must be marked for reconciliation");
        assert_eq!(attempts.get(), 1, "{method} was replayed");
        let unknown = error
            .downcast_ref::<MutationOutcomeUnknown>()
            .ok_or("mutation error did not carry the typed uncertain outcome")?;
        assert_eq!(unknown.method, method);
    }
    Ok(())
}

fn raw_metadata_retry_converges_after_cache_lag() -> Result<()> {
    let expected = b"exact-qualified-metadata".to_vec();
    let mut responses = std::collections::VecDeque::from([
        Ok(None),
        Ok(Some(b"stale-cdn-bytes".to_vec())),
        Ok(Some(expected.clone())),
    ]);
    let reads = std::cell::Cell::new(0usize);
    let elapsed = std::cell::Cell::new(Duration::ZERO);
    await_raw_metadata_convergence(
        &expected,
        Duration::from_secs(15),
        Duration::from_secs(5),
        || {
            reads.set(reads.get() + 1);
            responses
                .pop_front()
                .ok_or("raw metadata test response queue exhausted")?
        },
        |delay| elapsed.set(elapsed.get() + delay),
        || elapsed.get(),
    )?;
    assert_eq!(reads.get(), 3, "404 and stale bytes must be retried");
    assert_eq!(elapsed.get(), Duration::from_secs(10));
    Ok(())
}

fn raw_metadata_retry_fails_with_bounded_diagnostics() -> Result<()> {
    let expected = b"expected-metadata";
    let stale = b"permanently-stale-metadata".to_vec();
    let reads = std::cell::Cell::new(0usize);
    let elapsed = std::cell::Cell::new(Duration::ZERO);
    let error = await_raw_metadata_convergence(
        expected,
        Duration::from_secs(10),
        Duration::from_secs(5),
        || {
            reads.set(reads.get() + 1);
            Ok(Some(stale.clone()))
        },
        |delay| elapsed.set(elapsed.get() + delay),
        || elapsed.get(),
    )
    .expect_err("permanently stale public metadata must hit the deadline");
    assert_eq!(reads.get(), 2);
    assert!(
        error
            .to_string()
            .contains(&format!("expected_sha256={}", sha256_hex(expected)))
    );
    assert!(error.to_string().contains("last_http=200"));
    assert!(
        error
            .to_string()
            .contains(&format!("last_observed_sha256={}", sha256_hex(&stale)))
    );
    Ok(())
}

fn metadata_token_probe_rejects_wrong_repository_state() -> Result<()> {
    let mut read_only = FakeTransport::new();
    assert!(probe_metadata_token("read", &mut read_only).is_err());
    assert!(read_only.events.is_empty());

    for (bad_ref, bad_bootstrap, bad_target) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        let mut github = FakeTransport::new();
        github.bad_metadata_ref = bad_ref;
        github.bad_metadata_bootstrap = bad_bootstrap;
        github.bad_metadata_target = bad_target;
        assert!(probe_metadata_token("write", &mut github).is_err());
        assert!(github.events.iter().all(|event| event.starts_with("GET ")));
        assert!(!github.events.iter().any(|event| {
            event.starts_with("PUT ") || event.starts_with("POST ") || event.starts_with("DELETE ")
        }));
    }
    let mut readable = FakeTransport::new();
    probe_metadata_token("write", &mut readable)?;
    for channel in ["stable", "beta"] {
        assert!(readable.events.iter().any(|event| {
            event == &format!(
                "GET repos/{REPOSITORY}/contents/channels/{channel}/latest.json?ref=release-metadata"
            )
        }));
    }
    assert!(
        readable
            .events
            .iter()
            .all(|event| event.starts_with("GET "))
    );
    for channel in ["stable", "beta"] {
        let mut invalid = FakeTransport::new();
        invalid.bad_channel_metadata = Some(channel.to_owned());
        assert!(probe_metadata_token("write", &mut invalid).is_err());
        assert!(invalid.events.iter().all(|event| event.starts_with("GET ")));
    }
    Ok(())
}

fn mock_pr_merge_sha_does_not_break_release_or_owned_cleanup() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let runtime = RuntimeIdentity::from_values(
        Some(context.workflow_sha.clone()),
        Some(context.repository.clone()),
        Some(context.run_id.clone()),
        Some(context.run_attempt.to_string()),
        Some("workflow_dispatch".to_owned()),
        Some("refs/heads/main".to_owned()),
        Some(RELEASE_WORKFLOW_REF.to_owned()),
    )?;
    let pr_merge_runtime = RuntimeIdentity {
        github_sha: "0".repeat(40),
        ..runtime
    };
    assert!(
        validate_runtime_identity(&context, Some(&context.workflow_sha), &pr_merge_runtime,)
            .is_err()
    );
    validate_context(&root, &options, &context)?;

    let mut github = FakeTransport::new();
    github.latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
    let marker = format!(
        "<!-- v4-release-tx: {} -->",
        serde_json::to_string(&json!({
            "repository":context.repository,"run_id":context.run_id,"source_sha":context.source_sha,
            "run_attempt":context.run_attempt,"version":context.version,"tag":context.tag
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
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
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
        context.source_sha, context.version, context.tag
    ));
    github.releases.insert(12, conflict);
    github.next_id = 13;
    prepare_publish_fixture(&options, &context, &manifest, &github.latest)?;
    assert!(publish_release_with_mock_verifier(&root, &options, &mut github).is_err());
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
    assert!(publish_release_with_mock_verifier(&root, &options, &mut github).is_err());
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
    let error = publish_release_with_mock_verifier(&root, &options, &mut github)
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
    let error = publish_release_with_mock_verifier(&root, &options, &mut github)
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
    publish_release_with_mock_verifier(&root, &options, &mut github)?;
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
    assert!(publish_release_with_mock_verifier(&root, &options, &mut github).is_err());
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
    assert!(publish_release_with_mock_verifier(&root, &options, &mut github).is_err());
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
    assert!(!matches_transaction(&release, &context));
    release["body"] = json!(format!(
        "<!-- v4-release-tx: {{\"repository\":\"{REPOSITORY}\",\"run_id\":\"run\",\"source_sha\":\"{}\",\"version\":\"{}\",\"tag\":\"{}\"}} --><!-- v4-release-tx: {{}} -->",
        context.source_sha, context.version, context.tag
    ));
    assert!(!matches_transaction(&release, &context));
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

fn runtime_identity_fails_closed_without_exact_workflow_context() -> Result<()> {
    let (_, options, context, _) = fixture()?;
    let alternate_run_id = format!("{}0", context.run_id);
    let alternate_attempt = if context.run_attempt == 1 { "2" } else { "1" };
    let valid = RuntimeIdentity::from_values(
        Some(context.workflow_sha.clone()),
        Some(context.repository.clone()),
        Some(context.run_id.clone()),
        Some(context.run_attempt.to_string()),
        Some("workflow_dispatch".to_owned()),
        Some("refs/heads/main".to_owned()),
        Some(RELEASE_WORKFLOW_REF.to_owned()),
    )?;
    assert!(validate_runtime_identity(&context, Some(&context.workflow_sha), &valid).is_ok());
    assert!(validate_runtime_identity(&context, None, &valid).is_err());
    assert!(
        RuntimeIdentity::from_values(
            None,
            Some(context.repository.clone()),
            Some(context.run_id.clone()),
            Some(context.run_attempt.to_string()),
            Some("workflow_dispatch".to_owned()),
            Some("refs/heads/main".to_owned()),
            Some(RELEASE_WORKFLOW_REF.to_owned()),
        )
        .is_err()
    );
    for invalid in [
        RuntimeIdentity {
            github_sha: "f".repeat(40),
            ..valid.clone()
        },
        RuntimeIdentity {
            github_repository: "attacker/other".to_owned(),
            ..valid.clone()
        },
        RuntimeIdentity {
            github_run_id: alternate_run_id,
            ..valid.clone()
        },
        RuntimeIdentity {
            github_run_attempt: alternate_attempt.to_owned(),
            ..valid.clone()
        },
        RuntimeIdentity {
            github_event_name: "push".to_owned(),
            ..valid.clone()
        },
        RuntimeIdentity {
            github_ref: "refs/heads/feature".to_owned(),
            ..valid.clone()
        },
        RuntimeIdentity {
            github_workflow_ref: format!("{RELEASE_WORKFLOW_REF}@refs/heads/feature"),
            ..valid.clone()
        },
    ] {
        assert!(
            validate_runtime_identity(&context, Some(&context.workflow_sha), &invalid).is_err()
        );
    }
    let _ = fs::remove_dir_all(&options.state_root);
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
    fail_download: bool,
    corrupt_download: bool,
    corrupt_signature_download: bool,
    patch_applies_then_fails: bool,
    fail_reconcile_after_patch: bool,
    preserve_latest_on_publish: bool,
    fail_upload_after_first: bool,
    fail_upload_after_commit: bool,
    fail_delete: bool,
    post_created_then_timeout: bool,
    fail_metadata_put_after_write: bool,
    fail_metadata_put_without_write: bool,
    bad_metadata_ref: bool,
    bad_metadata_bootstrap: bool,
    bad_metadata_target: bool,
    bad_channel_metadata: Option<String>,
    metadata: Vec<u8>,
    metadata_files: std::collections::BTreeMap<String, Vec<u8>>,
    raw_metadata_files: std::collections::BTreeMap<String, Vec<u8>>,
    workflow_runs: std::collections::BTreeMap<String, Value>,
    workflow_jobs: std::collections::BTreeMap<(String, u32), Vec<Value>>,
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
            let path = endpoint
                .strip_prefix("repos/pumni/Sky-Auto-Player/actions/runs/")
                .unwrap_or_default();
            if let Some((run_id, attempt_query)) = path.split_once("/attempts/") {
                let Some((attempt, query)) = attempt_query.split_once("/jobs?") else {
                    return Err("fake workflow job endpoint omitted attempt jobs path".into());
                };
                let run_attempt = attempt.parse::<u32>()?;
                let query_value = |name: &str| {
                    query
                        .split('&')
                        .find_map(|part| part.strip_prefix(&format!("{name}=")))
                        .and_then(|value| value.parse::<usize>().ok())
                };
                let per_page = query_value("per_page").ok_or("fake jobs page size missing")?;
                let page = query_value("page").ok_or("fake jobs page number missing")?;
                if per_page == 0 || page == 0 {
                    return Err("fake jobs pagination is invalid".into());
                }
                let Some(jobs) = self.workflow_jobs.get(&(run_id.to_owned(), run_attempt)) else {
                    return Ok(None);
                };
                let start = (page - 1).saturating_mul(per_page);
                let items = jobs
                    .iter()
                    .skip(start)
                    .take(per_page)
                    .cloned()
                    .collect::<Vec<_>>();
                return Ok(Some(json!({"total_count":jobs.len(),"jobs":items})));
            }
            let id = path.split('?').next().unwrap_or_default();
            return Ok(self.workflow_runs.get(id).cloned());
        }
        if method == "GET" && endpoint == "repos/pumni/Sky-Auto-Player" {
            let target = if self.bad_metadata_target {
                "pumni/Other-Repository"
            } else {
                REPOSITORY
            };
            return Ok(Some(json!({"full_name":target})));
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
                    if self.bad_channel_metadata.as_deref() == Some(channel) {
                        return Ok(Some(json!({
                            "encoding":"base64",
                            "content":base64::engine::general_purpose::STANDARD.encode(b"not valid release metadata"),
                            "sha":"d".repeat(40)
                        })));
                    }
                    return Ok(self.metadata_files.get(channel).map(|bytes| {
                        json!({"encoding":"base64","content":base64::engine::general_purpose::STANDARD.encode(bytes),"sha":"c".repeat(40)})
                    }));
                }
                "PUT" => {
                    if self.fail_metadata_put_without_write {
                        self.fail_metadata_put_without_write = false;
                        return Err(Box::new(HttpFailure {
                            status: Some(504),
                            detail: "injected metadata write timeout before commit".to_owned(),
                        }));
                    }
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
        if self.fail_upload_after_commit {
            self.fail_upload_after_commit = false;
            return Err(Box::new(HttpFailure {
                status: Some(504),
                detail: "injected upload timeout after commit".to_owned(),
            }));
        }
        Ok(())
    }

    fn download(&mut self, endpoint: &str, destination: &Path) -> Result<()> {
        self.events.push(format!("DOWNLOAD {endpoint}"));
        if self.fail_download {
            self.fail_download = false;
            return Err("injected draft asset download failure".into());
        }
        if self.corrupt_signature_download
            && self.releases.values().any(|release| {
                release["assets"].as_array().is_some_and(|assets| {
                    assets.iter().any(|asset| {
                        asset["url"] == endpoint
                            && asset["name"]
                                .as_str()
                                .is_some_and(|name| name.ends_with(".sig"))
                    })
                })
            })
        {
            self.corrupt_signature_download = false;
            fs::write(destination, b"corrupted draft signature")?;
            return Ok(());
        }
        if self.corrupt_download {
            self.corrupt_download = false;
            fs::write(destination, b"corrupted draft asset")?;
            return Ok(());
        }
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
        schema_version: 2,
        repository: REPOSITORY.to_owned(),
        version: version.clone(),
        channel: channel.to_owned(),
        tag: tag.clone(),
        release_notes_path: format!("docs/releases/{tag}.md"),
        source_sha: source_sha.clone(),
        workflow_sha: source_sha.clone(),
        run_id: env::var("GITHUB_RUN_ID").unwrap_or_else(|_| "9001".to_owned()),
        run_attempt: env::var("GITHUB_RUN_ATTEMPT")
            .ok()
            .and_then(|value| value.parse().ok())
            .filter(|value| *value > 0 && *value <= 100)
            .unwrap_or(1),
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
    let production = json!({"repository":context.repository,"source_sha":source_sha,"workflow_sha":context.workflow_sha,"run_id":context.run_id,"run_attempt":context.run_attempt,"version":version,"channel":channel,"authenticode_mode":"unsigned-zero-budget","authenticode_state":"unsigned","authenticode_provider":"none","approved_signer_thumbprint":null,"observed_signer_thumbprint":null,
        "updater_signature_status":"valid","updater_key_id":crate::updater_trust::canonical_key_id()?,"qualification_status":"PASS","installer":installer.source_name,"installer_size":installer.size,"installer_sha256":installer.sha256,
        "updater_signature":signature.source_name,"signature_size":signature.size,"updater_signature_sha256":signature.sha256,"authenticode_evidence":auth.source_name,"authenticode_evidence_sha256":auth.sha256,"sbom":sbom.source_name,"sbom_sha256":sbom.sha256});
    let qualification = json!({"schema_version":1,"evidence_type":"tauri-nsis-qualified-release","repository":context.repository,"source_sha":context.source_sha,"workflow_sha":context.workflow_sha,"run_id":context.run_id,"run_attempt":context.run_attempt,"qualified":true,"qualification":"install-launch-uninstall","product_name":"Sky Auto Player","identifier":"io.github.pumni.skyautoplayer","version":version,"target":"nsis","install_mode":"currentUser","installer":installer.source_name,"updater_signature":signature.source_name,"installer_size":installer.size,"signature_size":signature.size,"installer_sha256":installer.sha256,"updater_signature_sha256":signature.sha256,"authenticode_mode":"unsigned-zero-budget","authenticode_evidence":auth.source_name,"authenticode_evidence_sha256":auth.sha256,"sbom":sbom.source_name,"sbom_sha256":sbom.sha256});
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
    let installer_record = base
        .iter()
        .find(|record| record.role == "installer")
        .ok_or("mock fixture installer record missing")?;
    let unsigned_handoff = release_handoff::UnsignedArtifactManifest {
        schema_version: 2,
        repository: context.repository.clone(),
        source_sha: context.source_sha.clone(),
        workflow_sha: context.workflow_sha.clone(),
        run_id: context.run_id.clone(),
        run_attempt: context.run_attempt,
        version: context.version.clone(),
        installer_name: installer_record.source_name.clone(),
        installer_size: installer_record.size,
        installer_sha256: installer_record.sha256.clone(),
        calibration_binary: "native_calibration.exe".to_owned(),
        calibration_binary_size: 1,
        calibration_binary_sha256: "c".repeat(64),
        calibration_binary_architecture: "x86_64".to_owned(),
        producer_job: "build_unsigned".to_owned(),
        artifact_name: release_handoff::artifact_name(&context.run_id, context.run_attempt)?,
    };
    Ok((
        root,
        options,
        context.clone(),
        CandidateManifest {
            schema_version: 2,
            repository: context.repository.clone(),
            source_sha: context.source_sha.clone(),
            workflow_sha: context.workflow_sha.clone(),
            run_id: context.run_id.clone(),
            run_attempt: context.run_attempt,
            version,
            channel: channel.to_owned(),
            tag,
            unsigned_handoff,
            qualification_assets: base,
            public_assets,
        },
    ))
}

fn fixture_state(context: &Context) -> ReleaseState {
    ReleaseState {
        schema_version: 2,
        repository: context.repository.clone(),
        version: context.version.clone(),
        channel: context.channel.clone(),
        tag: context.tag.clone(),
        source_sha: context.source_sha.clone(),
        workflow_sha: context.workflow_sha.clone(),
        run_id: context.run_id.clone(),
        run_attempt: context.run_attempt,
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
    use std::process::Command;

    #[test]
    fn candidate_verification_dispatcher_requires_the_exact_workflow_job() {
        assert_eq!(expected_job_for_command("verify-candidate"), Some("attest"));
        assert_eq!(
            expected_job_for_command("probe-verify-candidate"),
            Some("metadata_probe")
        );
        assert_eq!(
            validate_dispatch_job("probe-verify-candidate", "metadata_probe").unwrap(),
            "metadata_probe"
        );
        assert!(validate_dispatch_job("probe-verify-candidate", "attest").is_err());
        assert!(validate_dispatch_job("verify-candidate", "metadata_probe").is_err());
        assert!(validate_dispatch_job("probe-verify-candidate", "*").is_err());
        assert!(validate_dispatch_job("unknown", "metadata_probe").is_err());
    }

    #[test]
    fn sign_candidate_entrypoint_does_not_leak_updater_secrets_to_git_child() {
        const CHILD_MODE: &str = "SKY_XTASK_RELEASE_GIT_CHILD_TEST";
        const RUNNER_TEMP: &str = "SKY_XTASK_RELEASE_GIT_CHILD_RUNNER_TEMP";
        const SOURCE_SHA: &str = "SKY_XTASK_RELEASE_GIT_CHILD_SOURCE_SHA";
        const CAPTURE_PATH: &str = "SKY_XTASK_RELEASE_GIT_CHILD_CAPTURE";
        const COMMAND: &str = "SKY_XTASK_RELEASE_GIT_CHILD_COMMAND";

        if env::var_os(CHILD_MODE).is_some() {
            let root = repo::root();
            let source_sha = env::var(SOURCE_SHA).expect("test source SHA");
            let runner_temp = PathBuf::from(env::var_os(RUNNER_TEMP).expect("test runner temp"));
            let state_root = runner_temp.join("sky-v4-release-123456-1");
            fs::create_dir_all(&state_root).expect("create release state root");
            let version = repo::project_version(&root).expect("project version");
            let parsed = version::parse(&version).expect("parse project version");
            let channel = if parsed.pre.is_empty() {
                "stable"
            } else {
                "beta"
            };
            let tag = format!("v{version}");
            let context = Context {
                schema_version: 2,
                repository: REPOSITORY.to_owned(),
                version,
                channel: channel.to_owned(),
                tag: tag.clone(),
                release_notes_path: format!("docs/releases/{tag}.md"),
                source_sha: source_sha.clone(),
                workflow_sha: source_sha.clone(),
                run_id: "123456".to_owned(),
                run_attempt: 1,
                created_at: "2026-10-27T00:00:00Z".to_owned(),
            };
            write_json_atomic(&state_root.join("release-context.json"), &context)
                .expect("write release context");
            let command = env::var(COMMAND).expect("test release command");
            let actual_job = env::var("GITHUB_JOB").expect("test GitHub job identity");
            let args = [
                command.clone(),
                "--state-root".to_owned(),
                state_root.to_string_lossy().into_owned(),
                "--workflow-sha".to_owned(),
                source_sha,
            ];
            let error =
                run(&root, &args).expect_err("dispatcher fixture must stop before mutation");
            let expected_error = match (command.as_str(), actual_job.as_str()) {
                ("sign-candidate", "sign_qualify") => "unsigned handoff manifest",
                ("verify-candidate", "attest") | ("probe-verify-candidate", "metadata_probe") => {
                    "required release state file is missing"
                }
                ("verify-candidate", _) => "canonical workflow job attest",
                ("probe-verify-candidate", _) => "canonical workflow job metadata_probe",
                _ => panic!("unexpected dispatcher fixture command or job"),
            };
            assert!(
                error.to_string().contains(expected_error),
                "dispatcher `{command}` under `{actual_job}` stopped unexpectedly: {error}"
            );
            return;
        }

        let test_root = env::temp_dir().join(format!(
            "sky-release-git-child-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&test_root).expect("create test root");
        struct RemoveTestRoot(PathBuf);
        impl Drop for RemoveTestRoot {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let _remove_root = RemoveTestRoot(test_root.clone());
        let fake_git_dir = test_root.join("fake-git");
        fs::create_dir(&fake_git_dir).expect("create fake git directory");
        let capture_path = test_root.join("git-child-environments.txt");
        let source_sha = repo::git_head(&repo::root(), false).expect("read current source SHA");
        let fake_source = format!(
            r#"
use std::{{env, fs::OpenOptions, io::Write}};
const CAPTURE: &str = {capture_path:?};
const HEAD: &str = {source_sha:?};
fn main() {{
    let mut file = OpenOptions::new().create(true).append(true).open(CAPTURE).unwrap();
    writeln!(file, "--- child ---").unwrap();
    let mut values = env::vars().collect::<Vec<_>>();
    values.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, value) in values {{ writeln!(file, "{{name}}={{value}}").unwrap(); }}
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("rev-parse") {{ println!("{{HEAD}}"); }}
}}
"#
        );
        let fake_source_path = test_root.join("fake-git.rs");
        fs::write(&fake_source_path, fake_source).expect("write fake git source");
        let fake_git_path = fake_git_dir.join(if cfg!(windows) { "git.exe" } else { "git" });
        let compile = Command::new("rustc")
            .arg("--edition=2021")
            .arg(&fake_source_path)
            .arg("-o")
            .arg(&fake_git_path)
            .output()
            .expect("compile fake git child");
        assert!(
            compile.status.success(),
            "fake git compilation failed: {}",
            String::from_utf8_lossy(&compile.stderr)
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&fake_git_path)
                .expect("fake git metadata")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&fake_git_path, permissions).expect("make fake git executable");
        }
        let mut search_path = vec![fake_git_dir.clone()];
        search_path.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
        let path = env::join_paths(search_path).expect("join fake git search path");
        let runner_temp = test_root.join("runner-temp");
        fs::create_dir(&runner_temp).expect("create test runner temp");
        let executable = env::current_exe().expect("test executable path");
        for (command, job) in [
            ("sign-candidate", "sign_qualify"),
            ("verify-candidate", "attest"),
            ("probe-verify-candidate", "metadata_probe"),
            ("probe-verify-candidate", "attest"),
            ("verify-candidate", "metadata_probe"),
        ] {
            let child = Command::new(&executable)
                .args([
                    "--exact",
                    "release_v4::tests::sign_candidate_entrypoint_does_not_leak_updater_secrets_to_git_child",
                    "--nocapture",
                ])
                .env(CHILD_MODE, "1")
                .env("RUNNER_TEMP", &runner_temp)
                .env(RUNNER_TEMP, &runner_temp)
                .env(SOURCE_SHA, &source_sha)
                .env(CAPTURE_PATH, &capture_path)
                .env(COMMAND, command)
                .env("PATH", &path)
                .env("GITHUB_SHA", &source_sha)
                .env("GITHUB_REPOSITORY", REPOSITORY)
                .env("GITHUB_RUN_ID", "123456")
                .env("GITHUB_RUN_ATTEMPT", "1")
                .env("GITHUB_EVENT_NAME", "workflow_dispatch")
                .env("GITHUB_REF", "refs/heads/main")
                .env("GITHUB_WORKFLOW_REF", RELEASE_WORKFLOW_REF)
                .env("GITHUB_JOB", job)
                .env("V4_TAURI_UPDATER_PRIVATE_KEY", "synthetic-private-key")
                .env(
                    "V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD",
                    "synthetic-updater-passphrase",
                )
                .env("GITHUB_TOKEN", "synthetic-github-token")
                .env("GH_TOKEN", "synthetic-gh-token")
                .env("NPM_TOKEN", "synthetic-package-token")
                .output()
                .expect("run release dispatcher entrypoint test child");
            assert!(
                child.status.success(),
                "dispatcher regression for `{command}` / `{job}` failed: {}",
                String::from_utf8_lossy(&child.stderr)
            );
        }
        let observed = fs::read_to_string(&capture_path).expect("captured fake Git environments");
        assert!(observed.matches("--- child ---").count() >= 20);
        for forbidden in [
            "V4_TAURI_UPDATER_PRIVATE_KEY=",
            "V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD=",
            "GITHUB_TOKEN=",
            "GH_TOKEN=",
            "NPM_TOKEN=",
            "synthetic-private-key",
            "synthetic-updater-passphrase",
            "synthetic-github-token",
            "synthetic-gh-token",
            "synthetic-package-token",
        ] {
            assert!(
                !observed.contains(forbidden),
                "fake Git child observed a forbidden signing secret or token"
            );
        }
    }

    #[test]
    fn metadata_recovery_auth_routes_reads_and_one_stable_put_to_separate_tokens() {
        let transport = GhTransport::for_metadata_recovery(
            "synthetic-read-only-token".to_owned(),
            Some("synthetic-metadata-write-token".to_owned()),
        )
        .unwrap();
        for endpoint in [
            "repos/pumni/Sky-Auto-Player/actions/runs/7001",
            "repos/pumni/Sky-Auto-Player/actions/runs/7001/attempts/1/jobs",
            "repos/pumni/Sky-Auto-Player/releases/latest",
            "repos/pumni/Sky-Auto-Player/releases/assets/42",
        ] {
            let selected = transport
                .selected_recovery_token("GET", endpoint)
                .unwrap()
                .expect("recovery reads require an explicit credential");
            assert_eq!(selected.class, RecoveryTokenClass::ReadOnly);
            assert_eq!(selected.value, "synthetic-read-only-token");
        }
        let stable_put = transport
            .selected_recovery_token(
                "PUT",
                "repos/pumni/Sky-Auto-Player/contents/channels/stable/latest.json",
            )
            .unwrap()
            .expect("the canonical stable metadata write selects the App token");
        assert_eq!(stable_put.class, RecoveryTokenClass::MetadataWrite);
        assert_eq!(stable_put.value, "synthetic-metadata-write-token");
        for (method, endpoint) in [
            (
                "PUT",
                "repos/pumni/Sky-Auto-Player/contents/channels/beta/latest.json",
            ),
            (
                "PUT",
                "repos/pumni/Sky-Auto-Player/contents/channels/stable/latest.json?ref=main",
            ),
            (
                "POST",
                "repos/pumni/Sky-Auto-Player/contents/channels/stable/latest.json",
            ),
            ("DELETE", "repos/pumni/Sky-Auto-Player/releases/42"),
        ] {
            assert!(
                transport.selected_recovery_token(method, endpoint).is_err(),
                "recovery must reject {method} {endpoint}"
            );
        }
        let read_only =
            GhTransport::for_metadata_recovery("synthetic-read-only-token".to_owned(), None)
                .unwrap();
        assert!(
            read_only
                .selected_recovery_token(
                    "PUT",
                    "repos/pumni/Sky-Auto-Player/contents/channels/stable/latest.json",
                )
                .is_err(),
            "a read-only plan cannot synthesize metadata write authority"
        );
    }

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
        mock_draft_readback_failures_never_patch().unwrap();
        mock_pr_merge_sha_does_not_break_release_or_owned_cleanup().unwrap();
        mock_draft_post_timeout_reconciles_exact_marker().unwrap();
        mock_patch_timeout_reconciles_exact_release_id().unwrap();
        mock_complete_transaction_and_raw_metadata().unwrap();
        mock_metadata_promotion_is_idempotent_after_committed_timeout().unwrap();
        mock_metadata_promotion_rejects_changed_latest().unwrap();
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
