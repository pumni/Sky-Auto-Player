#![forbid(unsafe_code)]
// Non-Windows production exposes only the fail-closed command stub; fixture
// internals stay available to Windows builds and cross-platform unit tests.
#![cfg_attr(all(not(windows), not(test)), allow(dead_code))]

use crate::Result;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_HTTP_HEADER_BYTES: usize = 32 * 1024;
const MAX_HTTP_BODY_BYTES: usize = 512 * 1024 * 1024;
const MAX_HTTP_REQUESTS: usize = 4096;
const MAX_HTTP_REQUEST_DURATION: Duration = Duration::from_secs(10);
const MAX_HTTP_RESPONSE_DURATION: Duration = Duration::from_secs(120);
const UPDATER_PRODUCT_IDENTIFIER: &str = "io.github.pumni.skyautoplayer";
const CURRENT_NSIS_INSTALLER_HOOK: &str = "nsis-hooks.nsh";
#[cfg(windows)]
const BRIDGE_BUILD_OWNER_SENTINEL: &str = ".sky-bridge-fixture-owner";
#[cfg(windows)]
const BRIDGE_BUILD_OWNER_BYTES: &[u8] = b"Sky Auto Player bridge build fixture v1\n";
#[cfg(windows)]
const BRIDGE_SIGNER_CHALLENGE: &[u8] = b"Sky Auto Player disposable updater signer check v1\n";
const MAX_FIXTURE_SIGNATURE_BYTES: usize = 64 * 1024;
#[cfg(windows)]
const FIXTURE_ENV_REMOVALS: &[&str] = &[
    "TAURI_SIGNING_PRIVATE_KEY",
    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
    "TAURI_SIGNING_PRIVATE_KEY_PATH",
    "V4_UPDATER_PRIVATE_KEY_PATH",
    "V4_UPDATER_PASSWORD",
    "UPDATER_PRIVATE_KEY",
    "UPDATER_PASSWORD",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct RequestEvidence {
    method: String,
    path: String,
    status_code: u16,
    content_type: String,
    content_length: u64,
    body_length: u64,
    body_sha256: String,
}

#[derive(Clone, Debug)]
struct FixturePayloads {
    manifest: Vec<u8>,
    installer: Vec<u8>,
}

#[derive(Default)]
#[cfg(windows)]
struct FixtureEvidenceData {
    bridge: serde_json::Value,
    manifest: serde_json::Value,
    candidate: serde_json::Value,
    preservation: serde_json::Value,
    migration: serde_json::Value,
}

struct FixtureServer {
    address: SocketAddr,
    payloads: Arc<Mutex<FixturePayloads>>,
    requests: Arc<Mutex<Vec<RequestEvidence>>>,
    stop: Arc<AtomicBool>,
    active_stream: Arc<Mutex<Option<TcpStream>>>,
    thread: Option<JoinHandle<std::io::Result<()>>>,
}

impl FixtureServer {
    fn start(manifest: Vec<u8>, installer: Vec<u8>) -> Result<Self> {
        if manifest.is_empty() || installer.is_empty() {
            return Err("updater fixture responses must be non-empty exact byte arrays".into());
        }
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let payloads = Arc::new(Mutex::new(FixturePayloads {
            manifest,
            installer,
        }));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let active_stream = Arc::new(Mutex::new(None));
        let thread_payloads = Arc::clone(&payloads);
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);
        let thread_active_stream = Arc::clone(&active_stream);
        let server_thread = thread::Builder::new()
            .name("tauri-updater-loopback-fixture".to_owned())
            .spawn(move || {
                while !thread_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _peer)) => {
                            stream.set_nonblocking(false)?;
                            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                            stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                            let interrupt_stream = stream.try_clone()?;
                            {
                                let mut active = thread_active_stream.lock().map_err(|_| {
                                    std::io::Error::other("active fixture stream lock was poisoned")
                                })?;
                                *active = Some(interrupt_stream);
                                if thread_stop.load(Ordering::Acquire)
                                    && let Some(active) = active.as_ref()
                                {
                                    let _ = active.shutdown(Shutdown::Both);
                                }
                            }
                            let response = handle_fixture_connection(
                                stream,
                                &thread_payloads,
                                &thread_requests,
                            );
                            {
                                let mut active = thread_active_stream.lock().map_err(|_| {
                                    std::io::Error::other("active fixture stream lock was poisoned")
                                })?;
                                active.take();
                            }
                            if response.is_err() && !thread_stop.load(Ordering::Acquire) {
                                response?;
                            }
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => return Err(error),
                    }
                }
                Ok(())
            })?;
        Ok(Self {
            address,
            payloads,
            requests,
            stop,
            active_stream,
            thread: Some(server_thread),
        })
    }

    fn port(&self) -> u16 {
        self.address.port()
    }

    #[cfg(windows)]
    fn candidate_url(&self) -> String {
        format!("http://127.0.0.1:{}/candidate/update.exe", self.port())
    }

    fn set_manifest(&self, manifest: Vec<u8>) -> Result<()> {
        if manifest.is_empty() {
            return Err("updater fixture manifest cannot be empty".into());
        }
        self.payloads
            .lock()
            .map_err(|_| "updater fixture payload lock was poisoned")?
            .manifest = manifest;
        Ok(())
    }

    fn requests(&self) -> Result<Vec<RequestEvidence>> {
        Ok(self
            .requests
            .lock()
            .map_err(|_| "updater fixture request lock was poisoned")?
            .clone())
    }

    fn stop(&mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        let mut errors = Vec::new();
        match self.active_stream.lock() {
            Ok(active) => {
                if let Some(stream) = active.as_ref()
                    && let Err(error) = stream.shutdown(Shutdown::Both)
                    && error.kind() != std::io::ErrorKind::NotConnected
                {
                    errors.push(format!("interrupt active fixture connection: {error}"));
                }
            }
            Err(_) => errors.push("active fixture stream lock was poisoned".to_owned()),
        }
        if let Some(handle) = self.thread.take() {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("fixture HTTP thread failed: {error}")),
                Err(_) => errors.push("updater fixture HTTP thread panicked".to_owned()),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "updater fixture server shutdown failed: {}",
                errors.join("; ")
            )
            .into())
        }
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn handle_fixture_connection(
    mut stream: TcpStream,
    payloads: &Arc<Mutex<FixturePayloads>>,
    requests: &Arc<Mutex<Vec<RequestEvidence>>>,
) -> std::io::Result<()> {
    let mut request = Vec::with_capacity(1024);
    let mut chunk = [0_u8; 2048];
    let deadline = Instant::now() + MAX_HTTP_REQUEST_DURATION;
    while request.len() < MAX_HTTP_HEADER_BYTES && !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "updater fixture request exceeded its absolute read deadline",
            ));
        }
        stream.set_read_timeout(Some(remaining.min(Duration::from_secs(2))))?;
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
    }
    let request_line = request
        .split(|byte| *byte == b'\n')
        .next()
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .unwrap_or_default();
    let fields = request_line
        .split(|byte| *byte == b' ')
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    let method = fields
        .first()
        .map(|value| String::from_utf8_lossy(value).into_owned())
        .unwrap_or_default();
    let raw_path = fields.get(1).copied().unwrap_or_default();
    let path = String::from_utf8_lossy(raw_path)
        .split('?')
        .next()
        .unwrap_or_default()
        .to_owned();
    let (status, status_text, content_type, body) = if method != "GET" {
        (
            405,
            "Method Not Allowed",
            "text/plain",
            b"method not allowed".to_vec(),
        )
    } else if request.len() >= MAX_HTTP_HEADER_BYTES
        && !request.windows(4).any(|w| w == b"\r\n\r\n")
    {
        (
            431,
            "Request Header Fields Too Large",
            "text/plain",
            b"request too large".to_vec(),
        )
    } else if path == "/stable" || path == "/beta" {
        let body = payloads
            .lock()
            .map_err(|_| std::io::Error::other("fixture payload lock was poisoned"))?
            .manifest
            .clone();
        (200, "OK", "application/json", body)
    } else if path == "/candidate/update.exe" {
        let body = payloads
            .lock()
            .map_err(|_| std::io::Error::other("fixture payload lock was poisoned"))?
            .installer
            .clone();
        (200, "OK", "application/octet-stream", body)
    } else {
        (404, "Not Found", "text/plain", b"not found".to_vec())
    };
    let evidence = RequestEvidence {
        method,
        path,
        status_code: status,
        content_type: content_type.to_owned(),
        content_length: body.len() as u64,
        body_length: body.len() as u64,
        body_sha256: sha256_bytes(&body),
    };
    {
        let mut log = requests
            .lock()
            .map_err(|_| std::io::Error::other("fixture request lock was poisoned"))?;
        if log.len() >= MAX_HTTP_REQUESTS {
            return Err(std::io::Error::other(
                "updater fixture request limit exceeded",
            ));
        }
        log.push(evidence);
    }
    let response_header = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    let response_deadline = Instant::now() + MAX_HTTP_RESPONSE_DURATION;
    write_all_before_deadline(
        &mut stream,
        response_header.as_bytes(),
        response_deadline,
        |stream, timeout| stream.set_write_timeout(Some(timeout)),
    )?;
    write_all_before_deadline(&mut stream, &body, response_deadline, |stream, timeout| {
        stream.set_write_timeout(Some(timeout))
    })?;
    let _ = stream.shutdown(Shutdown::Write);
    Ok(())
}

fn write_all_before_deadline<W: Write>(
    writer: &mut W,
    bytes: &[u8],
    deadline: Instant,
    mut set_timeout: impl FnMut(&mut W, Duration) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut written = 0;
    while written < bytes.len() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "updater fixture response exceeded its absolute write deadline",
            ));
        }
        set_timeout(writer, remaining.min(Duration::from_secs(2)))?;
        match writer.write(&bytes[written..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "updater fixture response writer accepted zero bytes",
                ));
            }
            Ok(count) => {
                written += count;
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "updater fixture response exceeded its absolute write deadline",
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn make_manifest(
    version: &str,
    signature: &str,
    candidate_url: &str,
    note: &str,
) -> Result<Vec<u8>> {
    if signature.is_empty() || signature.len() > MAX_FIXTURE_SIGNATURE_BYTES {
        return Err("updater fixture signature is empty or exceeds its byte bound".into());
    }
    let value = serde_json::json!({
        "version": version,
        "notes": note,
        "pub_date": "2026-09-04T00:00:00Z",
        "platforms": {
            "windows-x86_64": {
                "signature": signature,
                "url": candidate_url,
            }
        }
    });
    let mut bytes = serde_json::to_vec(&value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn validate_manifest_bytes(bytes: &[u8], expected_version: &str, expected_url: &str) -> Result<()> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| format!("Tauri updater fixture manifest JSON is invalid: {error}"))?;
    let platform = value
        .get("platforms")
        .and_then(|platforms| platforms.get("windows-x86_64"))
        .ok_or("Tauri updater fixture manifest lacks windows-x86_64")?;
    if value.get("version").and_then(serde_json::Value::as_str) != Some(expected_version)
        || platform.get("url").and_then(serde_json::Value::as_str) != Some(expected_url)
        || platform
            .get("signature")
            .and_then(serde_json::Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err(
            "Tauri updater fixture manifest does not bind the exact version, URL, and signature"
                .into(),
        );
    }
    Ok(())
}

fn parse_options(
    args: &[String],
) -> Result<(
    BTreeMap<String, String>,
    crate::nsis_harness::RunnerAuthorization,
)> {
    let mut values = BTreeMap::new();
    let mut authorization = crate::nsis_harness::RunnerAuthorization::None;
    let mut index = 0;
    while index < args.len() {
        let next_authorization = match args[index].as_str() {
            "--allow-hosted-disposable-runner" => {
                Some(crate::nsis_harness::RunnerAuthorization::HostedDisposable)
            }
            "--allow-protected-release-runner" => {
                Some(crate::nsis_harness::RunnerAuthorization::ProtectedRelease)
            }
            _ => None,
        };
        if let Some(next_authorization) = next_authorization {
            if authorization != crate::nsis_harness::RunnerAuthorization::None {
                return Err("runner authorization option was repeated or conflicting".into());
            }
            authorization = next_authorization;
            index += 1;
            continue;
        }
        let name = args[index]
            .strip_prefix("--")
            .ok_or_else(|| format!("updater-fixture option must start with --: {}", args[index]))?;
        if ![
            "candidate-root",
            "bridge-root",
            "source-sha",
            "fixture-root",
            "evidence",
            "build-bridge",
            "bridge-version",
            "bridge-publisher",
        ]
        .contains(&name)
        {
            return Err(format!("unknown updater-fixture option: --{name}").into());
        }
        if name == "build-bridge" {
            if values.insert(name.to_owned(), "true".to_owned()).is_some() {
                return Err("--build-bridge was repeated".into());
            }
            index += 1;
            continue;
        }
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("missing value for --{name}"))?
            .clone();
        if values.insert(name.to_owned(), value).is_some() {
            return Err(
                format!("updater-fixture option was provided more than once: --{name}").into(),
            );
        }
        index += 1;
    }
    Ok((values, authorization))
}

#[cfg(windows)]
fn required<'a>(options: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str> {
    options
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("updater fixture requires --{name}").into())
}

#[cfg(windows)]
fn unique_suffix() -> String {
    let ticks = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{}-{ticks:x}", std::process::id())
}

#[cfg(not(windows))]
pub(crate) fn run(_repository_root: &Path, _args: &[String]) -> Result<()> {
    Err("actual Tauri updater qualification is Windows-only and requires a disposable hosted runner".into())
}

#[cfg(windows)]
pub(crate) fn run(repository_root: &Path, args: &[String]) -> Result<()> {
    if args.first().map(String::as_str) != Some("run") {
        return Err("updater-fixture requires the run operation".into());
    }
    let (options, authorization) = parse_options(&args[1..])?;
    crate::nsis_harness::require_runner_authorization(authorization)?;
    let source_sha = required(&options, "source-sha")?;
    let source_sha =
        crate::ci_artifacts::validate_commit_sha(source_sha, "updater fixture source SHA")?;
    let head = crate::ci::capture_git_text(repository_root, &["rev-parse", "--verify", "HEAD"])?;
    if head.trim() != source_sha {
        return Err("updater fixture source SHA must match the checked-out Git HEAD".into());
    }
    let candidate_root = PathBuf::from(required(&options, "candidate-root")?);
    let candidate_version = crate::repo::project_version(repository_root)?;
    let candidate =
        crate::ci_candidate::read_candidate(&candidate_root, &source_sha, &candidate_version)?;
    let runner_temp = std::env::var_os("RUNNER_TEMP")
        .map(PathBuf::from)
        .ok_or("RUNNER_TEMP is required for updater fixture isolation")?;
    let workspace = std::env::var_os("GITHUB_WORKSPACE")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository_root.to_owned());
    let fixture_root = PathBuf::from(required(&options, "fixture-root")?);
    let bridge_root = if options.contains_key("build-bridge") {
        let nonce = unique_suffix();
        let output = runner_temp.join(format!("sky-auto-player-bridge-contract-{nonce}"));
        let build_root = runner_temp.join(format!("sky-auto-player-bridge-build-{nonce}"));
        build_bridge_internal(
            repository_root,
            &source_sha,
            options
                .get("bridge-version")
                .map(String::as_str)
                .unwrap_or("4.1.3"),
            options
                .get("bridge-publisher")
                .map(String::as_str)
                .unwrap_or("github"),
            &build_root,
            &output,
        )?;
        output
    } else {
        PathBuf::from(required(&options, "bridge-root")?)
    };
    let generated_bridge = options.contains_key("build-bridge");
    let qualification: Result<()> = (|| {
        let bridge = crate::ci_bridge::read_bridge(
            &bridge_root,
            &source_sha,
            &crate::ci_bridge::BridgeBindings {
                version: options.get("bridge-version").cloned().or_else(|| {
                    options
                        .contains_key("build-bridge")
                        .then(|| "4.1.3".to_owned())
                }),
                publisher: options.get("bridge-publisher").cloned().or_else(|| {
                    options
                        .contains_key("build-bridge")
                        .then(|| "github".to_owned())
                }),
                identifier: Some("io.github.pumni.skyautoplayer".to_owned()),
                sentinel_id: None,
                sentinel_content_sha256: None,
            },
        )?;
        validate_historical_bridge_manifest_identity(
            &bridge.manifest.publisher,
            &bridge.manifest.identifier,
        )?;
        let (candidate_publisher, candidate_identifier) =
            current_tauri_bundle_identity(repository_root)?;
        if candidate_publisher != "pumni" || candidate_identifier != bridge.manifest.identifier {
            return Err(
            "updater candidate Tauri identity differs from the canonical pumni package identity"
                .into(),
        );
        }
        if semver::Version::parse(&candidate.manifest.version)?
            <= semver::Version::parse(&bridge.manifest.version)?
        {
            return Err(
                "updater fixture requires a strictly increasing N-to-N+1 candidate version".into(),
            );
        }
        let evidence = options
            .get("evidence")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                runner_temp.join("sky-auto-player-updater-fixture-http-evidence.json")
            });
        if options.contains_key("build-bridge") {
            let selected_bridge_version = options
                .get("bridge-version")
                .map(String::as_str)
                .unwrap_or("4.1.3");
            if selected_bridge_version != bridge.manifest.version {
                return Err(
                    "built bridge version does not match the requested bridge version".into(),
                );
            }
        }
        let mut scope = crate::nsis_harness::WindowsSmokeScope::enter(
            &fixture_root,
            &runner_temp,
            &workspace,
            crate::nsis_harness::RegistryMode::FreshInstall,
        )?;
        let mut server = None;
        let mut details = FixtureEvidenceData::default();
        let operation = prepare_and_run(
            &FixtureRunInputs {
                repository_root,
                bridge_root: &bridge_root,
                bridge: &bridge,
                candidate_root: &candidate_root,
                candidate: &candidate,
            },
            &mut scope,
            &mut server,
            &mut details,
        );
        let mut cleanup_errors = Vec::new();
        if let Some(mut active) = server.take() {
            if let Err(error) = active.stop() {
                cleanup_errors.push(format!("stop loopback HTTP server: {error}"));
            }
            if let Err(error) = write_http_evidence(
                &evidence,
                operation.is_ok(),
                active.port(),
                &active.requests(),
                &details,
            ) {
                cleanup_errors.push(format!("write fixture HTTP evidence: {error}"));
            }
        } else if let Err(error) =
            write_http_evidence(&evidence, false, 0, &Ok(Vec::new()), &details)
        {
            cleanup_errors.push(format!("write fixture HTTP evidence: {error}"));
        }
        if let Err(error) = scope.cleanup() {
            cleanup_errors.push(error.to_string());
        }
        let cleanup = if cleanup_errors.is_empty() {
            Ok(())
        } else {
            Err(cleanup_errors.join("; ").into())
        };
        crate::nsis_harness::combine_operation_and_cleanup(operation, cleanup)
    })();
    let bridge_cleanup = if generated_bridge {
        remove_validated_temporary_bridge(&bridge_root, &runner_temp, &source_sha)
    } else {
        Ok(())
    };
    crate::nsis_harness::combine_operation_and_cleanup(qualification, bridge_cleanup)
}

#[cfg(windows)]
fn remove_validated_temporary_bridge(
    root: &Path,
    runner_temp: &Path,
    source_sha: &str,
) -> Result<()> {
    match std::fs::symlink_metadata(root) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(
                "temporary bridge output is not a regular directory; refusing cleanup".into(),
            );
        }
        Ok(_) => {}
    }
    let runner_temp = std::fs::canonicalize(runner_temp)?;
    let canonical_root = std::fs::canonicalize(root)?;
    if canonical_root.parent() != Some(runner_temp.as_path()) {
        return Err("temporary bridge output escaped its direct RUNNER_TEMP child boundary".into());
    }
    crate::ci_bridge::read_bridge(
        &canonical_root,
        source_sha,
        &crate::ci_bridge::BridgeBindings::default(),
    )?;
    std::fs::remove_dir_all(canonical_root)?;
    Ok(())
}

#[cfg(windows)]
pub(crate) fn build_bridge(repository_root: &Path, args: &[String]) -> Result<()> {
    if args.first().map(String::as_str) != Some("build-fixture") {
        return Err("bridge requires create, validate, or build-fixture".into());
    }
    let allowed = [
        "source-sha",
        "bridge-version",
        "bridge-publisher",
        "fixture-root",
        "output-root",
    ];
    let mut values = BTreeMap::new();
    let mut index = 1;
    while index < args.len() {
        let name = args[index]
            .strip_prefix("--")
            .ok_or_else(|| format!("bridge fixture option must start with --: {}", args[index]))?;
        if !allowed.contains(&name) {
            return Err(format!("unknown bridge fixture option: --{name}").into());
        }
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("missing value for --{name}"))?
            .clone();
        if values.insert(name.to_owned(), value).is_some() {
            return Err(format!("bridge fixture option was repeated: --{name}").into());
        }
        index += 1;
    }
    let source_sha = required(&values, "source-sha")?;
    let bridge_version = values
        .get("bridge-version")
        .map(String::as_str)
        .unwrap_or("4.1.3");
    let publisher = values
        .get("bridge-publisher")
        .map(String::as_str)
        .unwrap_or("github");
    let fixture_root = PathBuf::from(required(&values, "fixture-root")?);
    let output_root = PathBuf::from(required(&values, "output-root")?);
    build_bridge_internal(
        repository_root,
        source_sha,
        bridge_version,
        publisher,
        &fixture_root,
        &output_root,
    )
}

#[cfg(not(windows))]
pub(crate) fn build_bridge(_repository_root: &Path, _args: &[String]) -> Result<()> {
    Err("updater bridge fixture packaging is Windows-only".into())
}

#[cfg(windows)]
fn write_http_evidence(
    path: &Path,
    success: bool,
    port: u16,
    requests: &Result<Vec<RequestEvidence>>,
    details: &FixtureEvidenceData,
) -> Result<()> {
    let requests = requests.as_ref().map_err(|error| error.to_string())?;
    let evidence = serde_json::json!({
        "schema_version": 1,
        "status": if success { "PASS" } else { "FAIL" },
        "port": port,
        "proxy_environment": {
            "http_proxy_set": std::env::var("HTTP_PROXY").ok().is_some_and(|value| !value.trim().is_empty()) || std::env::var("http_proxy").ok().is_some_and(|value| !value.trim().is_empty()),
            "https_proxy_set": std::env::var("HTTPS_PROXY").ok().is_some_and(|value| !value.trim().is_empty()) || std::env::var("https_proxy").ok().is_some_and(|value| !value.trim().is_empty()),
            "no_proxy_set": std::env::var("NO_PROXY").ok().is_some_and(|value| !value.trim().is_empty()) || std::env::var("no_proxy").ok().is_some_and(|value| !value.trim().is_empty()),
            "loopback_client_proxy_disabled": true
        },
        "requests": requests,
        "bridge": details.bridge,
        "manifest": details.manifest,
        "candidate": details.candidate,
        "preservation": details.preservation,
        "migration": details.migration,
        "response_bytes_are_exact_source_bytes": true,
        "bound_address": format!("127.0.0.1:{port}"),
    });
    let bytes = serde_json::to_vec_pretty(&evidence)?;
    let parent = path
        .parent()
        .ok_or("fixture evidence path requires a parent")?;
    std::fs::create_dir_all(parent)?;
    std::fs::write(path, [bytes.as_slice(), b"\n"].concat())?;
    Ok(())
}

#[cfg(windows)]
struct FixtureRunInputs<'a> {
    repository_root: &'a Path,
    bridge_root: &'a Path,
    bridge: &'a crate::ci_bridge::BridgeContract,
    candidate_root: &'a Path,
    candidate: &'a crate::ci_candidate::CandidateContract,
}

#[cfg(windows)]
fn prepare_and_run(
    inputs: &FixtureRunInputs<'_>,
    scope: &mut crate::nsis_harness::WindowsSmokeScope,
    server_out: &mut Option<FixtureServer>,
    details: &mut FixtureEvidenceData,
) -> Result<()> {
    let FixtureRunInputs {
        repository_root,
        bridge_root,
        bridge,
        candidate_root,
        candidate,
    } = inputs;
    let fixture_root = scope.root.clone();
    let app_data = fixture_root.join("app-data");
    let install_root = fixture_root.join("install");
    let bridge_installer = std::fs::canonicalize(bridge_root.join(&bridge.manifest.installer))?;
    let expected_bridge = std::fs::canonicalize(bridge_root)?;
    if !bridge_installer.starts_with(expected_bridge) {
        return Err("bridge installer escaped its validated bridge artifact root".into());
    }
    let candidate_installer =
        std::fs::canonicalize(candidate_root.join(&candidate.manifest.installer))?;
    let expected_candidate = std::fs::canonicalize(candidate_root)?;
    if !candidate_installer.starts_with(&expected_candidate) {
        return Err("candidate installer escaped its validated candidate artifact root".into());
    }
    let candidate_bytes = read_bounded_file(&candidate_installer, 512 * 1024 * 1024)?;
    if sha256_bytes(&candidate_bytes) != candidate.installer_sha256 {
        return Err("candidate installer bytes changed after contract validation".into());
    }
    let candidate_signature =
        std::fs::read(candidate_root.join(&candidate.manifest.updater_signature))?;
    if candidate_signature.is_empty() || candidate_signature.len() > MAX_FIXTURE_SIGNATURE_BYTES {
        return Err("candidate updater signature is empty or exceeds its bound".into());
    }
    let candidate_public_key_path =
        std::fs::canonicalize(candidate_root.join(&candidate.manifest.updater_public_key))?;
    if !candidate_public_key_path.starts_with(&expected_candidate) {
        return Err("candidate public key escaped its validated artifact root".into());
    }
    crate::ci_artifacts::validate_public_key(&candidate_public_key_path)?;
    let candidate_public_key = std::fs::read_to_string(&candidate_public_key_path)?;
    validate_public_key_text(&candidate_public_key)?;

    let manifest_source = std::fs::read(repository_root.join("builtin-songs/manifest.json"))?;
    let source_catalog: sky_app_core::catalog::BuiltinCatalogManifest =
        serde_json::from_slice(&manifest_source)?;
    let sentinel = source_catalog
        .songs
        .iter()
        .find(|song| song.path.to_ascii_lowercase().ends_with(".json"))
        .ok_or("canonical built-in catalog has no JSON song sentinel")?;
    let sentinel_id = sentinel.id.clone();
    let candidate_sentinel_sha = sentinel.sha256.clone();
    if sentinel_id != bridge.manifest.sentinel_id {
        return Err(
            "bridge fixture catalog sentinel ID does not match the canonical stable identity"
                .into(),
        );
    }
    if candidate_sentinel_sha.eq_ignore_ascii_case(&bridge.manifest.sentinel_content_sha256) {
        return Err(
            "updater fixture requires distinct N and N+1 built-in sentinel content hashes".into(),
        );
    }

    let key_root = fixture_root.join("keys");
    std::fs::create_dir(&key_root)?;
    let old_key_path = key_root.join("bridge-old.key");
    let untrusted_key_path = key_root.join("untrusted-failure.key");
    generate_signing_key(repository_root, &old_key_path)?;
    generate_signing_key(repository_root, &untrusted_key_path)?;
    let old_public_key = std::fs::read_to_string(with_pub_extension(&old_key_path))?;
    let untrusted_public_key = std::fs::read_to_string(with_pub_extension(&untrusted_key_path))?;
    validate_public_key_text(&old_public_key)?;
    validate_public_key_text(&untrusted_public_key)?;
    if old_public_key.trim() == candidate_public_key.trim()
        || untrusted_public_key.trim() == candidate_public_key.trim()
        || old_public_key.trim() == untrusted_public_key.trim()
    {
        return Err("disposable updater fixture trust roots must be distinct".into());
    }

    let invalid_signature_path = key_root.join("untrusted-candidate.exe");
    std::fs::copy(&candidate_installer, &invalid_signature_path)?;
    if sha256_file(&invalid_signature_path)? != candidate.installer_sha256 {
        return Err("untrusted-signature copy changed the exact candidate bytes".into());
    }
    sign_with_key(
        repository_root,
        &untrusted_key_path,
        &invalid_signature_path,
    )?;
    let invalid_signature = read_bounded_text(
        &with_sig_extension(&invalid_signature_path),
        MAX_FIXTURE_SIGNATURE_BYTES,
    )?;
    let old_signature_path = key_root.join("old-root-candidate.exe");
    std::fs::copy(&candidate_installer, &old_signature_path)?;
    sign_with_key(repository_root, &old_key_path, &old_signature_path)?;
    if sha256_file(&old_signature_path)? != candidate.installer_sha256 {
        return Err(
            "old-root detached-signature operation changed candidate installer bytes".into(),
        );
    }
    let old_signature = read_bounded_text(
        &with_sig_extension(&old_signature_path),
        MAX_FIXTURE_SIGNATURE_BYTES,
    )?;
    let candidate_signature_text = String::from_utf8(candidate_signature)?;
    for signature in [
        &invalid_signature,
        &old_signature,
        &candidate_signature_text,
    ] {
        if signature.trim().is_empty() || signature.len() > MAX_FIXTURE_SIGNATURE_BYTES {
            return Err("updater fixture signature text is empty or unbounded".into());
        }
    }

    let loopback_url = "http://127.0.0.1:1/candidate/update.exe".to_owned();
    let failure_manifest = make_manifest(
        &candidate.manifest.version,
        &invalid_signature,
        &loopback_url,
        "Untrusted signature rollback fixture.",
    )?;
    let _server = FixtureServer::start(failure_manifest, candidate_bytes.clone())?;
    let server = server_out.insert(_server);
    let failure_manifest = make_manifest(
        &candidate.manifest.version,
        &invalid_signature,
        &server.candidate_url(),
        "Untrusted signature rollback fixture.",
    )?;
    server.set_manifest(failure_manifest.clone())?;
    validate_manifest_bytes(
        &failure_manifest,
        &candidate.manifest.version,
        &server.candidate_url(),
    )?;
    verify_exact_http_source(server, "/stable", &failure_manifest, "application/json")?;
    verify_exact_http_source(
        server,
        "/candidate/update.exe",
        &candidate_bytes,
        "application/octet-stream",
    )?;

    std::fs::create_dir_all(&install_root)?;
    let env = scope.child_environment(&app_data);
    let bridge_installer_copy = fixture_root.join("bridge-installer.exe");
    std::fs::copy(&bridge_installer, &bridge_installer_copy)?;
    if sha256_file(&bridge_installer_copy)? != bridge.installer_sha256 {
        return Err("bridge installer copy differs from the validated bridge artifact".into());
    }
    crate::nsis_harness::run_installer(
        &bridge_installer_copy,
        &install_root,
        &env,
        "bridge N installer",
    )?;
    let bridge_app = install_root.join("sky_desktop_shell.exe");
    let uninstaller = install_root.join("uninstall.exe");
    require_file(&bridge_app, "installed bridge executable")?;
    require_file(&uninstaller, "installed bridge uninstaller")?;
    crate::builtin_catalog::run(
        repository_root,
        "verify-installed",
        &[(
            "root".to_owned(),
            install_root
                .join("builtin-songs")
                .to_string_lossy()
                .into_owned(),
        )],
    )?;
    let bridge_catalog = read_installed_catalog(
        &install_root.join("builtin-songs"),
        &bridge.manifest.sentinel_id,
    )
    .map_err(|error| format!("installed bridge catalog evidence failed: {error}"))?;
    if bridge_catalog.selected_content_sha256 != bridge.manifest.sentinel_content_sha256 {
        return Err("installed bridge catalog does not contain the exact N sentinel bytes".into());
    }
    let bridge_sha = sha256_file(&bridge_app)
        .map_err(|error| format!("installed bridge executable hashing failed: {error}"))?;
    let bridge_registry = assert_publisher_state(
        &bridge.manifest.publisher,
        &bridge.manifest.version,
        &install_root,
    )
    .map_err(|error| format!("installed bridge registry evidence failed: {error}"))?;
    details.bridge = serde_json::json!({
        "status": "PASS",
        "manifest": {
            "source_sha": &bridge.manifest.source_sha,
            "version": &bridge.manifest.version,
            "publisher": &bridge.manifest.publisher,
            "identifier": &bridge.manifest.identifier,
            "installer": &bridge.manifest.installer,
            "installer_sha256": &bridge.installer_sha256
        },
        "installed_registry": {
            "Publisher": &bridge_registry.publisher,
            "DisplayVersion": &bridge_registry.version,
            "InstallLocation": &bridge_registry.install_location_raw,
            "InstallLocationNormalized": &bridge_registry.install_root,
            "UninstallString": &bridge_registry.uninstall_string,
            "uninstall_identity": &bridge_registry.uninstall_identity
        }
    });
    let preserved_bridge_root = fixture_root.join("preserved-bridge");
    copy_directory_tree(&install_root, &preserved_bridge_root)?;
    if sha256_file(&preserved_bridge_root.join("sky_desktop_shell.exe"))? != bridge_sha {
        return Err("preserved N bridge executable copy changed bytes".into());
    }

    let user_song_path = app_data.join("songs/updater-preserved-user.json");
    std::fs::create_dir_all(user_song_path.parent().ok_or("user song has no parent")?)?;
    let user_song = br#"{"name":"Updater preserved user","songNotes":[{"time":0,"key":"1Key0"}]}"#;
    std::fs::write(&user_song_path, user_song)?;
    let user_song_sha = sha256_bytes(user_song);
    if sha256_file(&user_song_path)? != user_song_sha {
        return Err("updater user-song sentinel was not written exactly".into());
    }

    let marker = fixture_root.join("completion.txt");
    let expected_version = fixture_root.join("expected-installed-version.txt");
    let safety_marker = fixture_root.join("safety.txt");
    let runtime_args = vec![
        "--selftest-desktop-update".to_owned(),
        "--selftest-update-marker".to_owned(),
        marker.to_string_lossy().into_owned(),
        "--selftest-update-expected-version-file".to_owned(),
        expected_version.to_string_lossy().into_owned(),
        "--selftest-update-safety-marker".to_owned(),
        safety_marker.to_string_lossy().into_owned(),
        "--selftest-update-fixture-port".to_owned(),
        server.port().to_string(),
        "--selftest-update-fixture-public-key".to_owned(),
        with_pub_extension(&old_key_path)
            .to_string_lossy()
            .into_owned(),
        "--selftest-update-fixture-public-key".to_owned(),
        candidate_public_key_path.to_string_lossy().into_owned(),
    ];

    // A validly signed current candidate under an untrusted root must fail
    // before NSIS runs. The N executable, uninstall identity, AppData bytes,
    // and complete monitored registry subtree are compared after rejection.
    let n_state_registry =
        sky_windows_security::registry::capture(&sky_windows_security::registry::product_scopes())?;
    let old_manifest = make_manifest(
        &candidate.manifest.version,
        &invalid_signature,
        &server.candidate_url(),
        "Untrusted signature rollback fixture.",
    )?;
    server.set_manifest(old_manifest)?;
    let failure_exit = run_app_with_status(
        scope,
        &bridge_app,
        &runtime_args,
        &env,
        Duration::from_secs(180),
    )?;
    if failure_exit.success() {
        return Err("untrusted updater signature unexpectedly exited successfully".into());
    }
    let failure_marker = read_marker(&marker)?;
    if !failure_marker.starts_with("update-failed:") {
        return Err("untrusted updater failure marker was not written".into());
    }
    if sha256_file(&bridge_app)? != bridge_sha {
        return Err("failed updater transaction changed the installed N executable".into());
    }
    n_state_registry.assert_equivalent()?;
    if sha256_file(&user_song_path)? != user_song_sha {
        return Err("failed updater transaction changed the user-song sentinel".into());
    }
    let failed_registry =
        assert_publisher_state("github", &bridge.manifest.version, &install_root)?;
    if failed_registry != bridge_registry {
        return Err("failed updater transaction changed the N publisher/uninstall identity".into());
    }
    details.candidate = serde_json::json!({
        "status": "PASS",
        "failure_rollback": {
            "status": "PASS",
            "untrusted_signature_rejected": true,
            "installed_n_sha256_unchanged": true,
            "registry_state_unchanged": true,
            "user_song_sha256_unchanged": true
        },
        "installer_sha256": candidate.installer_sha256,
        "n_to_n_plus_1_installer_sha256": candidate.installer_sha256,
        "failure_signature_root": "untrusted-ephemeral-root"
    });

    // Reset only fixture markers; the parent process environment is untouched.
    std::fs::remove_file(&marker).ok();
    std::fs::remove_file(&safety_marker).ok();
    let success_manifest = make_manifest(
        &candidate.manifest.version,
        &candidate_signature_text,
        &server.candidate_url(),
        "Deterministic bridge rotation candidate.",
    )?;
    validate_manifest_bytes(
        &success_manifest,
        &candidate.manifest.version,
        &server.candidate_url(),
    )?;
    server.set_manifest(success_manifest.clone())?;
    verify_exact_http_source(server, "/stable", &success_manifest, "application/json")?;
    verify_exact_http_source(
        server,
        "/candidate/update.exe",
        &candidate_bytes,
        "application/octet-stream",
    )?;
    let _bridge_exit = run_app_with_status(
        scope,
        &bridge_app,
        &runtime_args,
        &env,
        Duration::from_secs(180),
    )?;
    wait_for_candidate_install(
        &install_root,
        &candidate.manifest.version,
        "pumni",
        &bridge_sha,
        Duration::from_secs(180),
    )?;
    let candidate_app = install_root.join("sky_desktop_shell.exe");
    let candidate_sha = sha256_file(&candidate_app)?;
    if candidate_sha == bridge_sha {
        return Err("installed N+1 executable is byte-identical to bridge N".into());
    }
    let candidate_registry =
        assert_publisher_state("pumni", &candidate.manifest.version, &install_root)?;
    if candidate_registry.install_root != bridge_registry.install_root
        || candidate_registry.uninstall_identity != bridge_registry.uninstall_identity
        || candidate_registry.uninstall_string != bridge_registry.uninstall_string
    {
        return Err("N-to-N+1 migration changed the install root or uninstall identity".into());
    }
    let candidate_exit = run_app_with_status(
        scope,
        &candidate_app,
        &[
            "--selftest-desktop-update".to_owned(),
            "--selftest-update-marker".to_owned(),
            marker.to_string_lossy().into_owned(),
            "--selftest-update-expected-version-file".to_owned(),
            expected_version.to_string_lossy().into_owned(),
        ],
        &env,
        Duration::from_secs(180),
    )?;
    if !candidate_exit.success()
        || read_marker(&marker)?.trim() != format!("update-complete:{}", candidate.manifest.version)
    {
        return Err("exact installed N+1 candidate self-test did not complete".into());
    }
    let safety = read_bounded_text(&safety_marker, 16 * 1024)?;
    validate_safety_phases(&safety)?;
    if sha256_file(&user_song_path)? != user_song_sha {
        return Err("successful updater transaction changed the user-song sentinel".into());
    }
    let candidate_catalog = read_installed_catalog(
        &install_root.join("builtin-songs"),
        &bridge.manifest.sentinel_id,
    )?;
    if candidate_catalog.manifest_sha256 == bridge_catalog.manifest_sha256
        || candidate_catalog.selected_id != bridge_catalog.selected_id
        || candidate_catalog.selected_content_sha256 == bridge_catalog.selected_content_sha256
        || !candidate_catalog
            .selected_content_sha256
            .eq_ignore_ascii_case(&candidate_sentinel_sha)
    {
        return Err(
            "N-to-N+1 update did not replace catalog bytes while preserving stable ID".into(),
        );
    }
    let migration = assert_publisher_state("pumni", &candidate.manifest.version, &install_root)?;
    if migration.install_root != bridge_registry.install_root
        || migration.uninstall_identity != bridge_registry.uninstall_identity
        || migration.uninstall_string != bridge_registry.uninstall_string
    {
        return Err(
            "candidate migration did not preserve publisher-neutral install identity".into(),
        );
    }
    let n_plus_one_registry =
        sky_windows_security::registry::capture(&sky_windows_security::registry::product_scopes())?;
    details.manifest = serde_json::json!({
        "status": "PASS",
        "schema_status": "PASS",
        "version": candidate.manifest.version,
        "platform": "windows-x86_64",
        "candidate_url": server.candidate_url(),
        "signature_present": true
    });
    details.preservation = serde_json::json!({
        "status": "PASS",
        "transition": "N-to-N+1",
        "user_song": "updater-preserved-user.json",
        "user_song_sha256_before": user_song_sha,
        "user_song_sha256_after": sha256_file(&user_song_path)?,
        "built_in_manifest_sha256_before": bridge_catalog.manifest_sha256,
        "built_in_manifest_sha256_after": candidate_catalog.manifest_sha256,
        "selected_builtin_id_before": bridge_catalog.selected_id,
        "selected_builtin_id_after": candidate_catalog.selected_id,
        "selected_builtin_content_sha256_before": bridge_catalog.selected_content_sha256,
        "selected_builtin_content_sha256_after": candidate_catalog.selected_content_sha256,
        "source_tree_restore": "PASS"
    });
    details.migration = serde_json::json!({
        "status": "PASS",
        "previous": {
            "version": bridge_registry.version,
            "publisher": bridge_registry.publisher,
            "identifier": "io.github.pumni.skyautoplayer",
            "install_root": bridge_registry.install_root,
            "install_location_raw": bridge_registry.install_location_raw,
            "uninstall_identity": bridge_registry.uninstall_identity,
            "uninstall_string": bridge_registry.uninstall_string
        },
        "candidate": {
            "version": migration.version,
            "publisher": migration.publisher,
            "identifier": "io.github.pumni.skyautoplayer",
            "install_root": migration.install_root,
            "install_location_raw": migration.install_location_raw,
            "uninstall_identity": migration.uninstall_identity,
            "uninstall_string": migration.uninstall_string
        },
        "install_root_preserved": true,
        "uninstall_identity_preserved": true,
        "app_data_preserved": true,
        "historical_publisher_registry": "github -> pumni; stale github state absent"
    });

    // Use the byte-identical preserved N binary with only the N+1 public root.
    // A detached old-root signature must be rejected before any installer runs.
    server.set_manifest(make_manifest(
        &candidate.manifest.version,
        &old_signature,
        &server.candidate_url(),
        "Old-root cutover rejection fixture.",
    )?)?;
    let requests_before_cutover = server.requests()?.len();
    let cutover_marker = fixture_root.join("cutover.txt");
    let mut cutover_args = vec![
        "--selftest-desktop-update".to_owned(),
        "--selftest-update-marker".to_owned(),
        cutover_marker.to_string_lossy().into_owned(),
        "--selftest-update-fixture-port".to_owned(),
        server.port().to_string(),
        "--selftest-update-fixture-public-key".to_owned(),
        with_pub_extension(&old_key_path)
            .to_string_lossy()
            .into_owned(),
        "--selftest-update-fixture-public-key".to_owned(),
        candidate_public_key_path.to_string_lossy().into_owned(),
        "--selftest-update-fixture-new-only".to_owned(),
    ];
    cutover_args.dedup();
    let preserved_bridge_app = fixture_root.join("preserved-bridge/sky_desktop_shell.exe");
    let cutover_exit = run_app_with_status(
        scope,
        &preserved_bridge_app,
        &cutover_args,
        &env,
        Duration::from_secs(180),
    )?;
    if cutover_exit.success() || !read_marker(&cutover_marker)?.starts_with("update-failed:") {
        return Err("new-root-only bridge accepted the old-root updater signature".into());
    }
    let all_requests = server.requests()?;
    let negative = &all_requests[requests_before_cutover..];
    if !negative.iter().any(|request| request.path == "/stable")
        || !negative
            .iter()
            .any(|request| request.path == "/candidate/update.exe")
        || negative
            .iter()
            .filter(|request| request.path == "/candidate/update.exe")
            .any(|request| request.body_sha256 != candidate.installer_sha256)
    {
        return Err("old-root rejection did not request and hash the exact candidate bytes".into());
    }
    if sha256_file(&candidate_app)? != candidate_sha {
        return Err("old-root rejection mutated the installed N+1 executable".into());
    }
    n_plus_one_registry.assert_equivalent()?;
    if sha256_file(&user_song_path)? != user_song_sha {
        return Err("old-root rejection changed the user-song sentinel".into());
    }

    let mut uninstaller = crate::nsis_harness::run_uninstaller(&uninstaller, &install_root, &env)?;
    crate::nsis_harness::finish_uninstall(&install_root, &mut uninstaller)?;
    assert_migration_registry_roots_absent()?;
    if sha256_file(&user_song_path)? != user_song_sha {
        return Err("uninstaller changed the user-song sentinel".into());
    }
    let request_count = server.requests()?.len();
    details.candidate["old_root_rejection"] = serde_json::json!({
        "status": "PASS",
        "manifest_requests": negative.iter().filter(|request| request.path == "/stable").count(),
        "candidate_requests": negative.iter().filter(|request| request.path == "/candidate/update.exe").count(),
        "served_candidate_sha256": candidate.installer_sha256,
        "matches_n_to_n_plus_1": true,
        "update_result": read_marker(&cutover_marker)?
    });
    details.migration["uninstall"] = serde_json::json!({
        "status": "PASS",
        "install_root_removed": true,
        "app_data_preserved": true,
        "registry_publisher_state_clean": true,
        "registry_scope_restore": "deferred-to-finalizer",
        "user_song_sha256_after_uninstall": sha256_file(&user_song_path)?
    });
    println!(
        "[xtask] Tauri updater fixture: PASS (N={} -> N+1={}; rollback; exact HTTP bytes; old-root rejection; preserved AppData; requests={request_count})",
        bridge.manifest.version, candidate.manifest.version
    );
    Ok(())
}

#[cfg(windows)]
fn verify_exact_http_source(
    server: &FixtureServer,
    path: &str,
    expected: &[u8],
    content_type: &str,
) -> Result<()> {
    let url = format!("http://127.0.0.1:{}{path}", server.port());
    let response = http_get_loopback(&url)?;
    if response.status != 200
        || response.content_type != content_type
        || response.body != expected
        || response.content_length != expected.len() as u64
        || sha256_bytes(&response.body) != sha256_bytes(expected)
    {
        return Err(
            format!("loopback {path} response differs from exact fixture source bytes").into(),
        );
    }
    Ok(())
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    content_type: String,
    content_length: u64,
    body: Vec<u8>,
}

fn http_get_loopback(url: &str) -> Result<HttpResponse> {
    let parsed = url::Url::parse(url)?;
    if parsed.scheme() != "http"
        || parsed.host_str() != Some("127.0.0.1")
        || parsed.port().is_none()
    {
        return Err("fixture HTTP client accepts only an explicit 127.0.0.1 endpoint".into());
    }
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), parsed.port().unwrap()),
        Duration::from_secs(2),
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    write!(
        stream,
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        parsed.path(),
        parsed.port().unwrap()
    )?;
    stream.flush()?;
    let mut bytes = Vec::with_capacity(4096);
    let mut chunk = [0_u8; 4096];
    let separator = loop {
        if let Some(separator) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break separator;
        }
        if bytes.len() >= MAX_HTTP_HEADER_BYTES {
            return Err("loopback fixture response header exceeds its byte bound".into());
        }
        let remaining = MAX_HTTP_HEADER_BYTES + 4 - bytes.len();
        let read_limit = remaining.min(chunk.len());
        let read = stream.read(&mut chunk[..read_limit])?;
        if read == 0 {
            return Err("loopback fixture returned an incomplete HTTP response header".into());
        }
        bytes.extend_from_slice(&chunk[..read]);
    };
    if separator > MAX_HTTP_HEADER_BYTES {
        return Err("loopback fixture response header exceeds its byte bound".into());
    }
    let header = std::str::from_utf8(&bytes[..separator])?;
    let mut lines = header.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .ok_or("loopback fixture returned an invalid HTTP status line")?
        .parse::<u16>()?;
    let mut content_type = None;
    let mut content_length = None;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-type") {
                if content_type.is_some() {
                    return Err("fixture response has duplicate Content-Type headers".into());
                }
                content_type = Some(value.trim().to_owned());
            } else if name.eq_ignore_ascii_case("content-length") {
                if content_length.is_some() {
                    return Err("fixture response has duplicate Content-Length headers".into());
                }
                let parsed = value.trim().parse::<u64>()?;
                if parsed > MAX_HTTP_BODY_BYTES as u64 {
                    return Err("fixture response body exceeds its byte bound".into());
                }
                content_length = Some(parsed);
            }
        }
    }
    let content_length = content_length.ok_or("fixture response has no Content-Length")?;
    let body_start = separator + 4;
    let mut body = bytes[body_start..].to_vec();
    if body.len() as u64 > content_length {
        return Err("loopback fixture returned bytes beyond Content-Length".into());
    }
    while (body.len() as u64) < content_length {
        let remaining = (content_length - body.len() as u64) as usize;
        let read_limit = remaining.min(chunk.len());
        let read = stream.read(&mut chunk[..read_limit]).map_err(|error| {
            format!(
                "loopback fixture body length mismatch: header={content_length}, received={}; read failed: {error}",
                body.len()
            )
        })?;
        if read == 0 {
            return Err(format!(
                "loopback fixture body length mismatch: header={content_length}, received={}",
                body.len()
            )
            .into());
        }
        body.extend_from_slice(&chunk[..read]);
    }
    if body.len() as u64 != content_length {
        return Err(format!(
            "loopback fixture body length mismatch: header={content_length}, received={}",
            body.len()
        )
        .into());
    }
    Ok(HttpResponse {
        status,
        content_type: content_type.ok_or("fixture response has no Content-Type")?,
        content_length,
        body,
    })
}

#[cfg(windows)]
fn validate_public_key_text(value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 4096
        || value.contains("PRIVATE KEY")
        || value.contains('\0')
    {
        return Err("fixture public key is empty, private, or unbounded".into());
    }
    Ok(())
}

#[cfg(windows)]
fn generate_signing_key(repository_root: &Path, key_path: &Path) -> Result<()> {
    let args = vec![
        "run".to_owned(),
        "tauri".to_owned(),
        "signer".to_owned(),
        "generate".to_owned(),
        "--ci".to_owned(),
        "--password".to_owned(),
        String::new(),
        "--force".to_owned(),
        "-w".to_owned(),
        key_path.to_string_lossy().into_owned(),
    ];
    crate::process::run_owned_timeout_filtered(
        std::ffi::OsStr::new("bun"),
        &args,
        &repository_root.join("desktop"),
        &[],
        &removed_private_environment(),
        Duration::from_secs(120),
    )?;
    require_file(key_path, "ephemeral updater key")?;
    require_file(
        &with_pub_extension(key_path),
        "ephemeral updater public key",
    )
}

#[cfg(windows)]
fn sign_with_key(repository_root: &Path, key_path: &Path, artifact: &Path) -> Result<()> {
    let args = vec![
        "run".to_owned(),
        "tauri".to_owned(),
        "signer".to_owned(),
        "sign".to_owned(),
        "--private-key-path".to_owned(),
        key_path.to_string_lossy().into_owned(),
        "--password".to_owned(),
        String::new(),
        artifact.to_string_lossy().into_owned(),
    ];
    crate::process::run_owned_timeout_filtered(
        std::ffi::OsStr::new("bun"),
        &args,
        &repository_root.join("desktop"),
        &[],
        &removed_private_environment(),
        Duration::from_secs(120),
    )?;
    require_file(&with_sig_extension(artifact), "detached updater signature")
}

#[cfg(windows)]
fn removed_private_environment() -> Vec<String> {
    FIXTURE_ENV_REMOVALS
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
}

#[cfg(windows)]
fn with_pub_extension(path: &Path) -> PathBuf {
    let mut output = path.as_os_str().to_owned();
    output.push(".pub");
    PathBuf::from(output)
}

#[cfg(windows)]
fn with_sig_extension(path: &Path) -> PathBuf {
    let mut output = path.as_os_str().to_owned();
    output.push(".sig");
    PathBuf::from(output)
}

#[cfg(windows)]
fn read_bounded_file(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > limit
    {
        return Err(format!(
            "fixture artifact is not a non-empty bounded regular file: {}",
            path.display()
        )
        .into());
    }
    Ok(std::fs::read(path)?)
}

#[cfg(windows)]
fn read_bounded_text(path: &Path, limit: usize) -> Result<String> {
    let bytes = read_bounded_file(path, limit as u64)?;
    String::from_utf8(bytes)
        .map_err(|_| format!("fixture text is not UTF-8: {}", path.display()).into())
}

#[cfg(windows)]
fn require_file(path: &Path, name: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("{name} is missing: {}: {error}", path.display()))?;
    if !metadata.file_type().is_file() || metadata.len() == 0 {
        return Err(format!("{name} is not a non-empty regular file: {}", path.display()).into());
    }
    Ok(())
}

#[cfg(windows)]
fn sha256_file(path: &Path) -> Result<String> {
    crate::ci_artifacts::sha256_file(path)
}

#[cfg(windows)]
fn run_app_with_status(
    scope: &mut crate::nsis_harness::WindowsSmokeScope,
    app: &Path,
    args: &[String],
    env: &[(String, String)],
    timeout: Duration,
) -> Result<std::process::ExitStatus> {
    let child = crate::process::spawn_owned_silent_filtered(
        app.as_os_str(),
        args,
        app.parent()
            .ok_or("updater fixture executable has no parent")?,
        env,
        &removed_private_environment(),
    )?;
    scope.children.push(child);
    scope
        .children
        .last_mut()
        .expect("owned process appended")
        .wait_timeout(timeout)
}

fn parse_windows_install_location(raw: &str) -> Result<String> {
    if raw.is_empty() || raw.trim() != raw {
        return Err("registry InstallLocation is empty or has surrounding whitespace".into());
    }
    let path = match (raw.starts_with('"'), raw.ends_with('"')) {
        (true, true) => &raw[1..raw.len() - 1],
        (true, false) | (false, true) => {
            return Err("registry InstallLocation has an unmatched quote".into());
        }
        (false, false) => raw,
    };
    if path.is_empty()
        || path.trim() != path
        || path.contains('"')
        || path.chars().any(char::is_control)
    {
        return Err("registry InstallLocation has malformed quoted path syntax".into());
    }
    let normalized = path.replace('/', "\\");
    if !is_absolute_windows_install_location(&normalized) {
        return Err("registry InstallLocation must be a well-formed absolute Windows path".into());
    }
    Ok(normalized)
}

fn is_absolute_windows_install_location(path: &str) -> bool {
    if let Some(rest) = path.strip_prefix(r"\\?\") {
        if rest
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("UNC\\"))
        {
            return rest
                .get(4..)
                .is_some_and(|unc| valid_windows_path_components(unc, 2));
        }
        return valid_windows_drive_absolute(rest);
    }
    if let Some(unc) = path.strip_prefix(r"\\") {
        if path.starts_with(r"\\.\") {
            return false;
        }
        return valid_windows_path_components(unc, 2);
    }
    valid_windows_drive_absolute(path)
}

fn valid_windows_drive_absolute(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'\\'
        && valid_windows_path_components(&path[3..], 0)
}

fn valid_windows_path_components(path: &str, minimum_components: usize) -> bool {
    let mut components = path.split('\\').collect::<Vec<_>>();
    if components.last() == Some(&"") {
        components.pop();
    }
    if components.len() < minimum_components {
        return false;
    }
    components.iter().all(|component| {
        !component.is_empty()
            && *component != "."
            && *component != ".."
            && !component.ends_with(' ')
            && !component.ends_with('.')
            && !component.chars().any(|character| {
                character.is_control()
                    || matches!(character, '<' | '>' | ':' | '"' | '|' | '?' | '*')
            })
    })
}

#[cfg(windows)]
fn validate_registry_install_location(raw: &str, expected_root: &Path) -> Result<PathBuf> {
    let normalized = parse_windows_install_location(raw)?;
    let observed_root = std::fs::canonicalize(PathBuf::from(&normalized)).map_err(|error| {
        let path_hint = raw.chars().take(260).collect::<String>();
        format!("registry InstallLocation {path_hint:?} cannot be resolved: {error}")
    })?;
    if observed_root != expected_root {
        return Err("uninstall InstallLocation does not match the bounded fixture root".into());
    }
    Ok(observed_root)
}

#[cfg(windows)]
fn assert_publisher_state(
    publisher: &str,
    version: &str,
    install_root: &Path,
) -> Result<PublisherEvidence> {
    use sky_windows_security::registry::read_value;
    const REG_SZ: u32 = 1;
    const UNINSTALL: &str =
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Sky Auto Player";
    const DUPLICATE: &str =
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\io.github.pumni.skyautoplayer";
    const LEGACY: &str = "Software\\github\\Sky Auto Player";
    if read_value(DUPLICATE, "DisplayName")?.is_some() {
        return Err("installer created a duplicate identifier-based uninstall key".into());
    }
    let publisher_value =
        required_registry_string(read_value(UNINSTALL, "Publisher")?, "Publisher")?;
    let version_value =
        required_registry_string(read_value(UNINSTALL, "DisplayVersion")?, "DisplayVersion")?;
    let install_location =
        required_registry_string(read_value(UNINSTALL, "InstallLocation")?, "InstallLocation")?;
    let uninstall_string =
        required_registry_string(read_value(UNINSTALL, "UninstallString")?, "UninstallString")?;
    if publisher_value != publisher || version_value != version {
        return Err(format!("uninstall identity publisher/version mismatch: observed {publisher_value}/{version_value}, expected {publisher}/{version}").into());
    }
    let expected_root = std::fs::canonicalize(install_root).map_err(|error| {
        format!("could not canonicalize bounded updater fixture install root: {error}")
    })?;
    validate_registry_install_location(&install_location, &expected_root)?;
    let expected_uninstaller = expected_root.join("uninstall.exe");
    if !uninstall_string
        .to_ascii_lowercase()
        .contains(&expected_uninstaller.to_string_lossy().to_ascii_lowercase())
    {
        return Err("uninstall command does not point at the bounded fixture uninstaller".into());
    }
    if publisher == "pumni" && read_value(LEGACY, "InstallLocation")?.is_some() {
        return Err("publisher migration left a historical github registry pointer".into());
    }
    for (name, expected) in [
        ("Publisher", publisher_value.as_str()),
        ("DisplayVersion", version_value.as_str()),
        ("InstallLocation", install_location.as_str()),
        ("UninstallString", uninstall_string.as_str()),
    ] {
        let value = read_value(UNINSTALL, name)?
            .ok_or_else(|| format!("uninstall value disappeared: {name}"))?;
        if value.kind != REG_SZ {
            return Err(format!("uninstall value kind is not REG_SZ: {name}").into());
        }
        let actual = decode_reg_sz(&value)?;
        if actual != expected {
            return Err(format!("uninstall value changed while verifying: {name}").into());
        }
    }
    Ok(PublisherEvidence {
        publisher: publisher_value,
        version: version_value,
        install_root: expected_root.to_string_lossy().into_owned(),
        install_location_raw: install_location,
        uninstall_identity: UNINSTALL.to_owned(),
        uninstall_string,
    })
}

#[cfg(windows)]
fn assert_migration_registry_roots_absent() -> Result<()> {
    const UNINSTALL: &str =
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Sky Auto Player";
    const DUPLICATE: &str =
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\io.github.pumni.skyautoplayer";
    const LEGACY: &str = "Software\\github\\Sky Auto Player";
    const CURRENT: &str = "Software\\pumni\\Sky Auto Player";
    use sky_windows_security::registry::{KeySpec, assert_roots_absent};
    let scopes = [
        KeySpec::new(
            UNINSTALL,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        )?,
        KeySpec::new(
            DUPLICATE,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        )?,
        KeySpec::new(LEGACY, "Software\\github")?,
        KeySpec::new(CURRENT, "Software\\pumni")?,
    ];
    assert_roots_absent(&scopes)?;
    Ok(())
}

#[cfg(windows)]
fn current_tauri_bundle_identity(repository_root: &Path) -> Result<(String, String)> {
    let path = repository_root.join("desktop/src-tauri/tauri.conf.json");
    let bytes = read_bounded_file(&path, 1024 * 1024)?;
    let config: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("canonical Tauri config is invalid JSON: {error}"))?;
    let publisher = config
        .get("bundle")
        .and_then(|bundle| bundle.get("publisher"))
        .and_then(serde_json::Value::as_str)
        .ok_or("canonical Tauri config has no literal bundle publisher")?;
    let identifier = config
        .get("identifier")
        .and_then(serde_json::Value::as_str)
        .ok_or("canonical Tauri config has no literal package identifier")?;
    if publisher != "pumni" || identifier != "io.github.pumni.skyautoplayer" {
        return Err(
            "canonical Tauri package must retain publisher pumni and its established identifier"
                .into(),
        );
    }
    Ok((publisher.to_owned(), identifier.to_owned()))
}

#[cfg(windows)]
fn required_registry_string(
    value: Option<sky_windows_security::registry::RegistryValue>,
    name: &str,
) -> Result<String> {
    let value =
        value.ok_or_else(|| format!("required uninstall registry value is missing: {name}"))?;
    if value.kind != 1 {
        return Err(format!("uninstall registry value is not REG_SZ: {name}").into());
    }
    decode_reg_sz(&value)
}

#[cfg(windows)]
fn decode_reg_sz(value: &sky_windows_security::registry::RegistryValue) -> Result<String> {
    if value.bytes.len() < 2
        || !value.bytes.len().is_multiple_of(2)
        || value.bytes[value.bytes.len() - 2..] != [0, 0]
    {
        return Err("REG_SZ bytes are malformed or not NUL-terminated".into());
    }
    let (words, _) = value.bytes[..value.bytes.len() - 2].as_chunks::<2>();
    let words = words
        .iter()
        .map(|chunk| u16::from_le_bytes(*chunk))
        .collect::<Vec<_>>();
    String::from_utf16(&words).map_err(|_| "REG_SZ value contains invalid UTF-16".into())
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct PublisherEvidence {
    publisher: String,
    version: String,
    install_root: String,
    install_location_raw: String,
    uninstall_identity: String,
    uninstall_string: String,
}

#[cfg(windows)]
struct CatalogEvidence {
    manifest_sha256: String,
    selected_id: String,
    selected_content_sha256: String,
}

#[cfg(windows)]
fn read_installed_catalog(root: &Path, selected_id: &str) -> Result<CatalogEvidence> {
    let manifest_path = root.join("manifest.json");
    let bytes = read_bounded_file(&manifest_path, 16 * 1024 * 1024)?;
    let manifest: sky_app_core::catalog::BuiltinCatalogManifest = serde_json::from_slice(&bytes)?;
    let selected = manifest
        .songs
        .iter()
        .find(|song| song.id == selected_id)
        .ok_or("installed built-in catalog lost the stable bridge sentinel ID")?;
    let song_relative = selected
        .path
        .strip_prefix("sheets/")
        .ok_or("catalog sentinel path is not rooted under sheets/")?;
    if song_relative.contains("..")
        || song_relative.contains('\\')
        || Path::new(song_relative).is_absolute()
    {
        return Err("catalog sentinel path is unsafe".into());
    }
    let song_path = root.join("sheets").join(song_relative);
    if sha256_file(&song_path)? != selected.sha256.to_ascii_lowercase() {
        return Err("installed catalog sentinel content SHA does not match its manifest".into());
    }
    Ok(CatalogEvidence {
        manifest_sha256: sha256_bytes(&bytes),
        selected_id: selected.id.clone(),
        selected_content_sha256: selected.sha256.to_ascii_lowercase(),
    })
}

#[cfg(windows)]
fn validate_safety_phases(contents: &str) -> Result<()> {
    let required = [
        "activity.admission_closed",
        "playback.keys_released",
        "state.persisted",
        "resources.closed",
    ];
    let phases = contents.lines().collect::<Vec<_>>();
    let mut previous = None;
    for phase in required {
        let index = phases
            .iter()
            .position(|observed| *observed == phase)
            .ok_or_else(|| format!("updater shutdown safety marker is missing {phase}"))?;
        if previous.is_some_and(|prior| index <= prior) {
            return Err("updater shutdown safety markers are out of order".into());
        }
        previous = Some(index);
    }
    Ok(())
}

#[cfg(windows)]
fn wait_for_candidate_install(
    install_root: &Path,
    expected_version: &str,
    expected_publisher: &str,
    previous_app_sha: &str,
    timeout: Duration,
) -> Result<()> {
    let started = Instant::now();
    let mut last_error = "candidate installation has not published registry state".to_owned();
    while started.elapsed() < timeout {
        let app = install_root.join("sky_desktop_shell.exe");
        if app.is_file() {
            match assert_publisher_state(expected_publisher, expected_version, install_root) {
                Ok(_) => match sha256_file(&app) {
                    Ok(digest) if digest != previous_app_sha => return Ok(()),
                    Ok(_) => last_error = "installed app still matches bridge N".to_owned(),
                    Err(error) => last_error = error.to_string(),
                },
                Err(error) => last_error = error.to_string(),
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
    Err(format!("timed out waiting for N+1 install barrier: {last_error}").into())
}

#[cfg(windows)]
fn copy_directory_tree(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir(destination)?;
    let mut pending = vec![(source.to_owned(), destination.to_owned())];
    let mut count = 0usize;
    while let Some((source_dir, target_dir)) = pending.pop() {
        for entry in std::fs::read_dir(&source_dir)? {
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                return Err("installed N bridge tree contains a reparse point".into());
            }
            let target = target_dir.join(entry.file_name());
            if metadata.is_dir() {
                std::fs::create_dir(&target)?;
                pending.push((entry.path(), target));
            } else if metadata.is_file() {
                count += 1;
                if count > 100_000 {
                    return Err("installed N bridge tree exceeds the copy bound".into());
                }
                std::fs::copy(entry.path(), target)?;
            } else {
                return Err(
                    "installed N bridge tree contains an unsupported filesystem object".into(),
                );
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn wait_for_path(path: &Path, timeout: Duration) -> Result<()> {
    let started = Instant::now();
    while !path.is_file() {
        if started.elapsed() >= timeout {
            return Err(format!("timed out waiting for updater marker: {}", path.display()).into());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(windows)]
fn read_marker(path: &Path) -> Result<String> {
    wait_for_path(path, Duration::from_secs(10))?;
    read_bounded_text(path, 4096)
}

#[cfg(windows)]
fn build_bridge_internal(
    repository_root: &Path,
    source_sha: &str,
    bridge_version: &str,
    publisher: &str,
    fixture_root: &Path,
    output_root: &Path,
) -> Result<()> {
    validate_supported_bridge_identity(publisher, UPDATER_PRODUCT_IDENTIFIER)?;
    crate::ci_artifacts::validate_commit_sha(source_sha, "bridge fixture source SHA")?;
    crate::ci_artifacts::validate_semver(bridge_version, "bridge fixture version")?;
    let head = crate::ci::capture_git_text(repository_root, &["rev-parse", "--verify", "HEAD"])?;
    if head.trim() != source_sha {
        return Err("bridge fixture source SHA must match the checked-out Git HEAD".into());
    }
    let runner_temp = std::fs::canonicalize(
        std::env::var_os("RUNNER_TEMP")
            .ok_or("RUNNER_TEMP is required for bridge fixture build")?,
    )?;
    let workspace = std::fs::canonicalize(
        std::env::var_os("GITHUB_WORKSPACE")
            .map(PathBuf::from)
            .unwrap_or_else(|| repository_root.to_owned()),
    )?;
    if runner_temp.starts_with(&workspace) {
        return Err(
            "RUNNER_TEMP must be outside the repository workspace for bridge builds".into(),
        );
    }
    if fixture_root.exists() || output_root.exists() {
        return Err("bridge fixture and output roots must be new paths".into());
    }
    let fixture_parent =
        std::fs::canonicalize(fixture_root.parent().ok_or("fixture root has no parent")?)?;
    let output_parent =
        std::fs::canonicalize(output_root.parent().ok_or("output root has no parent")?)?;
    if fixture_parent != runner_temp || output_parent != runner_temp || fixture_root == output_root
    {
        return Err("bridge build roots must be distinct direct children of RUNNER_TEMP".into());
    }
    std::fs::create_dir(fixture_root)?;
    let canonical_fixture_root = match std::fs::canonicalize(fixture_root) {
        Ok(path) => path,
        Err(error) => {
            let rollback = std::fs::remove_dir(fixture_root);
            return Err(format!(
                "could not canonicalize new bridge fixture root: {error}; empty-root rollback: {}",
                rollback.map_or_else(|rollback| rollback.to_string(), |_| "ok".to_owned())
            )
            .into());
        }
    };
    if canonical_fixture_root.parent() != Some(runner_temp.as_path())
        || canonical_fixture_root.starts_with(&workspace)
    {
        let rollback = std::fs::remove_dir(&canonical_fixture_root);
        return Err(format!(
            "new bridge fixture root escaped RUNNER_TEMP; empty-root rollback: {}",
            rollback.map_or_else(|rollback| rollback.to_string(), |_| "ok".to_owned())
        )
        .into());
    }
    let fixture_root = canonical_fixture_root;
    let ordinary_fixture_root = match verified_ordinary_windows_path(&fixture_root) {
        Ok(path) => path,
        Err(error) => {
            let rollback = std::fs::remove_dir(&fixture_root);
            return Err(format!(
                "bridge fixture root cannot be represented as a verified Git path: {error}; empty-root rollback: {}",
                rollback.map_or_else(|rollback| rollback.to_string(), |_| "ok".to_owned())
            )
            .into());
        }
    };
    let worktree_cli_path = match ordinary_fixture_root
        .join("source")
        .into_os_string()
        .into_string()
    {
        Ok(path) => path,
        Err(_) => {
            let rollback = std::fs::remove_dir(&fixture_root);
            return Err(format!(
                "bridge Git worktree path is not valid Unicode; empty-root rollback: {}",
                rollback.map_or_else(|rollback| rollback.to_string(), |_| "ok".to_owned())
            )
            .into());
        }
    };
    let owner_path = fixture_root.join(BRIDGE_BUILD_OWNER_SENTINEL);
    let mut owner_created = false;
    let owner_setup = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&owner_path)?;
        owner_created = true;
        file.write_all(BRIDGE_BUILD_OWNER_BYTES)?;
        file.sync_all()
    })();
    if let Err(error) = owner_setup {
        let rollback = remove_incomplete_bridge_build_root(
            &fixture_root,
            &runner_temp,
            &workspace,
            owner_created,
        );
        return Err(format!(
            "bridge fixture ownership sentinel setup failed: {error}; root rollback: {}",
            rollback.map_or_else(|rollback| rollback.to_string(), |_| "ok".to_owned())
        )
        .into());
    }
    let worktree = fixture_root.join("source");
    // Tauri's ACL manifest discovery rejects the extended-length Windows
    // prefix produced by `canonicalize`; use the already verified ordinary
    // spelling of the same owned fixture root for Cargo's target directory.
    let target = ordinary_fixture_root.join("target");
    let desktop = worktree.join("desktop");
    let key_path = fixture_root.join("bridge.key");
    let config_path = fixture_root.join("bridge-updater.json");
    let signer_challenge_path = fixture_root.join("bridge-signer-challenge.bin");
    let signer_challenge_signature_path = with_sig_extension(&signer_challenge_path);
    let worktree_add = crate::process::run_owned_timeout_filtered(
        std::ffi::OsStr::new("git"),
        &[
            "worktree".to_owned(),
            "add".to_owned(),
            "--detach".to_owned(),
            worktree_cli_path.clone(),
            source_sha.to_owned(),
        ],
        repository_root,
        &[],
        &removed_private_environment(),
        Duration::from_secs(120),
    );
    if let Err(error) = worktree_add {
        let worktree_cleanup = if worktree.exists() {
            crate::process::run_owned_timeout_filtered(
                std::ffi::OsStr::new("git"),
                &[
                    "worktree".to_owned(),
                    "remove".to_owned(),
                    "--force".to_owned(),
                    worktree_cli_path.clone(),
                ],
                repository_root,
                &[],
                &removed_private_environment(),
                Duration::from_secs(120),
            )
        } else {
            Ok(())
        };
        let mut cleanup_errors = Vec::new();
        let worktree_cleanup_failed = if let Err(cleanup_error) = worktree_cleanup {
            cleanup_errors.push(format!(
                "temporary Git worktree cleanup failed: {cleanup_error}"
            ));
            true
        } else {
            false
        };
        if worktree.exists() || worktree_cleanup_failed {
            cleanup_errors.push(
                "failed temporary Git worktree state is uncertain; fixture root retained"
                    .to_owned(),
            );
        } else if let Err(cleanup_error) =
            remove_owned_bridge_build_root(&fixture_root, &runner_temp, &workspace)
        {
            cleanup_errors.push(format!(
                "bridge fixture root cleanup failed: {cleanup_error}"
            ));
        }
        if cleanup_errors.is_empty() {
            return Err(error);
        }
        return Err(format!(
            "bridge worktree add failed: {error}; cleanup errors: {}",
            cleanup_errors.join("; ")
        )
        .into());
    }
    let operation: Result<()> = (|| {
        let cargo_path = worktree.join("desktop/src-tauri/Cargo.toml");
        let lock_path = worktree.join("rust/Cargo.lock");
        let config_source_path = worktree.join("desktop/src-tauri/tauri.conf.json");
        let catalog_path = worktree.join("builtin-songs/manifest.json");
        let cargo =
            replace_one_unindented_version(&std::fs::read_to_string(&cargo_path)?, bridge_version)?;
        std::fs::write(&cargo_path, cargo)?;
        let lock =
            replace_desktop_lock_version(&std::fs::read_to_string(&lock_path)?, bridge_version)?;
        std::fs::write(&lock_path, lock)?;
        let mut config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&config_source_path)?)?;
        prepare_bridge_tauri_config(&mut config, publisher)?;
        let catalog: sky_app_core::catalog::BuiltinCatalogManifest =
            serde_json::from_slice(&std::fs::read(&catalog_path)?)?;
        let sentinel = catalog
            .songs
            .iter()
            .find(|song| song.path.to_ascii_lowercase().ends_with(".json"))
            .ok_or("bridge build needs a JSON built-in song sentinel")?;
        let sentinel_id = sentinel.id.clone();
        let relative = safe_sentinel_relative_path(&sentinel.path)?;
        let bridge_song = br#"{"name":"Updater bridge catalog sentinel","bpm":120,"songNotes":[{"time":0,"key":"1Key0"},{"time":333,"key":"1Key1"}]}"#;
        let bridge_song_sha = sha256_bytes(bridge_song);
        let mut catalog = catalog;
        let selected = catalog
            .songs
            .iter_mut()
            .find(|song| song.id == sentinel_id)
            .ok_or("bridge sentinel disappeared from catalog")?;
        selected.sha256 = bridge_song_sha.clone();
        std::fs::write(&catalog_path, serde_json::to_vec_pretty(&catalog)?)?;
        std::fs::write(worktree.join("songs").join(&relative), bridge_song)?;

        let updater = config
            .get_mut("plugins")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("Tauri config plugins must be an object")?
            .entry("updater")
            .or_insert_with(|| serde_json::json!({}));
        let updater = updater
            .as_object_mut()
            .ok_or("Tauri updater config must be an object")?;
        updater.insert(
            "dangerousInsecureTransportProtocol".to_owned(),
            serde_json::Value::Bool(true),
        );
        updater.insert(
            "endpoints".to_owned(),
            serde_json::json!(["http://127.0.0.1:1/stable"]),
        );

        run_bun(
            &desktop,
            &["install".to_owned(), "--frozen-lockfile".to_owned()],
            &[],
            &target,
            Duration::from_secs(45 * 60),
        )?;
        let key_args = vec![
            "run".to_owned(),
            "tauri".to_owned(),
            "signer".to_owned(),
            "generate".to_owned(),
            "--ci".to_owned(),
            "--password".to_owned(),
            String::new(),
            "--force".to_owned(),
            "-w".to_owned(),
            key_path.to_string_lossy().into_owned(),
        ];
        run_bun(&desktop, &key_args, &[], &target, Duration::from_secs(120))?;
        let public_key = read_bounded_text(&with_pub_extension(&key_path), 4096)?;
        validate_public_key_text(&public_key)?;
        let private_key = read_bounded_text(&key_path, 16 * 1024)?;
        if private_key.trim().is_empty() || private_key.contains('\0') {
            return Err(
                "bridge builder did not produce the expected ephemeral Tauri signing key".into(),
            );
        }
        std::fs::write(&signer_challenge_path, BRIDGE_SIGNER_CHALLENGE)?;
        sign_with_key(&worktree, &key_path, &signer_challenge_path)?;
        let challenge_signature = read_bounded_text(
            &signer_challenge_signature_path,
            MAX_FIXTURE_SIGNATURE_BYTES,
        )?;
        crate::updater_trust::verify_signature_text_with_public_key(
            BRIDGE_SIGNER_CHALLENGE,
            &challenge_signature,
            &public_key,
        )
        .map_err(|_| "bridge Tauri signer key pair failed Rust Minisign verification")?;
        let build_config = serde_json::json!({
            "plugins": {
                "updater": {
                    "pubkey": public_key.trim(),
                    "dangerousInsecureTransportProtocol": true,
                    "endpoints": ["http://127.0.0.1:1/stable"]
                }
            }
        });
        validate_loopback_build_config(&build_config)?;
        persist_bridge_tauri_config(&worktree, &config_source_path, &config, publisher)?;
        std::fs::write(&config_path, serde_json::to_vec_pretty(&build_config)?)?;
        let build_args = vec![
            "run".to_owned(),
            "tauri".to_owned(),
            "build".to_owned(),
            "--ci".to_owned(),
            "--config".to_owned(),
            config_path.to_string_lossy().into_owned(),
            "--".to_owned(),
            "--profile".to_owned(),
            "dist".to_owned(),
            "--features".to_owned(),
            "desktop-runtime,packaged-assets,tauri-update-fixture".to_owned(),
        ];
        run_bun(
            &desktop,
            &build_args,
            &[
                (
                    "TAURI_SIGNING_PRIVATE_KEY".to_owned(),
                    private_key.trim().to_owned(),
                ),
                (
                    "TAURI_SIGNING_PRIVATE_KEY_PASSWORD".to_owned(),
                    String::new(),
                ),
            ],
            &target,
            Duration::from_secs(75 * 60),
        )?;
        let bundle = target.join("dist/bundle/nsis");
        let entries = direct_regular_files(&bundle)?;
        let installers = entries
            .iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.to_ascii_lowercase().ends_with("-setup.exe"))
            })
            .collect::<Vec<_>>();
        if installers.len() != 1 {
            return Err(format!(
                "bridge build must produce exactly one NSIS setup executable, found {}",
                installers.len()
            )
            .into());
        }
        let installer_name = installers[0]
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("bridge setup filename is not Unicode")?;
        let staged_bundle = fixture_root.join("bundle");
        std::fs::create_dir(&staged_bundle)?;
        std::fs::copy(installers[0], staged_bundle.join(installer_name))?;
        let producer = crate::ci_bridge::BridgeProducer {
            source_sha: source_sha.to_owned(),
            version: bridge_version.to_owned(),
            publisher: publisher.to_owned(),
            identifier: UPDATER_PRODUCT_IDENTIFIER.to_owned(),
            sentinel_id: sentinel_id.clone(),
            sentinel_content_sha256: bridge_song_sha,
        };
        crate::ci_bridge::create_bridge(&staged_bundle, output_root, &producer)?;
        let binding = crate::ci_bridge::BridgeBindings {
            version: Some(bridge_version.to_owned()),
            publisher: Some(publisher.to_owned()),
            identifier: Some(UPDATER_PRODUCT_IDENTIFIER.to_owned()),
            sentinel_id: Some(sentinel_id),
            sentinel_content_sha256: Some(producer.sentinel_content_sha256),
        };
        let contract = crate::ci_bridge::read_bridge(output_root, source_sha, &binding)?;
        println!(
            "[xtask] updater bridge build fixture: PASS (source={}; version={}; publisher={}; installer_sha256={}; sentinel={})",
            contract.manifest.source_sha,
            contract.manifest.version,
            contract.manifest.publisher,
            contract.installer_sha256,
            contract.manifest.sentinel_id
        );
        Ok(())
    })();
    let worktree_remove = crate::process::run_owned_timeout_filtered(
        std::ffi::OsStr::new("git"),
        &[
            "worktree".to_owned(),
            "remove".to_owned(),
            "--force".to_owned(),
            worktree_cli_path,
        ],
        repository_root,
        &[],
        &removed_private_environment(),
        Duration::from_secs(120),
    );
    let operation_description = operation
        .as_ref()
        .map(|_| "success".to_owned())
        .unwrap_or_else(ToString::to_string);
    let mut cleanup_errors = Vec::new();
    let worktree_remove_failed = if let Err(error) = worktree_remove {
        cleanup_errors.push(format!("git worktree cleanup failed: {error}"));
        true
    } else {
        false
    };
    for path in [
        &key_path,
        &with_pub_extension(&key_path),
        &config_path,
        &signer_challenge_path,
        &signer_challenge_signature_path,
    ] {
        if path.exists()
            && let Err(error) = std::fs::remove_file(path)
        {
            cleanup_errors.push(format!(
                "could not remove fixture file {}: {error}",
                path.display()
            ));
        }
    }
    if worktree.exists() || worktree_remove_failed {
        cleanup_errors.push(format!(
            "temporary worktree state is uncertain; build root retained: {}",
            worktree.display()
        ));
    }
    if worktree.exists() {
        cleanup_errors.push(
            "bridge build root retained to avoid deleting a Git-registered worktree".to_owned(),
        );
    } else if let Err(error) =
        remove_owned_bridge_build_root(&fixture_root, &runner_temp, &workspace)
    {
        cleanup_errors.push(format!("bridge build fixture cleanup failed: {error}"));
    }
    if operation.is_err()
        && output_root.exists()
        && let Err(error) = remove_validated_temporary_bridge(output_root, &runner_temp, source_sha)
    {
        cleanup_errors.push(format!("partial bridge artifact cleanup failed: {error}"));
    }
    if !cleanup_errors.is_empty() {
        return Err(format!(
            "bridge build outcome: {operation_description}; cleanup errors: {}",
            cleanup_errors.join("; ")
        )
        .into());
    }
    operation
}

#[cfg(windows)]
fn verified_ordinary_windows_path(canonical_path: &Path) -> Result<PathBuf> {
    let canonical_text = canonical_path
        .to_str()
        .ok_or("canonical bridge path is not valid Unicode")?;
    let ordinary_text = if canonical_text
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"\\?\UNC\"))
    {
        format!(r"\\{}", &canonical_text[8..])
    } else if let Some(path) = canonical_text.strip_prefix(r"\\?\") {
        path.to_owned()
    } else {
        canonical_text.to_owned()
    };
    if ordinary_text.starts_with(r"\\?\") {
        return Err("canonical bridge path uses an unsupported Windows device prefix".into());
    }
    let ordinary_path = PathBuf::from(&ordinary_text);
    if !ordinary_path.is_absolute() {
        return Err("Git worktree path must remain an absolute Windows path".into());
    }
    let resolved = std::fs::canonicalize(&ordinary_path)?;
    let resolved_text = resolved
        .to_str()
        .ok_or("normalized Git path resolved to a non-Unicode path")?
        .replace('/', "\\");
    if !resolved_text.eq_ignore_ascii_case(&canonical_text.replace('/', "\\")) {
        return Err("ordinary Git path does not resolve to its canonical bridge path".into());
    }
    Ok(ordinary_path)
}

#[cfg(windows)]
fn remove_owned_bridge_build_root(root: &Path, runner_temp: &Path, workspace: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(root)?;
    if !metadata.is_dir() || crate::ci_artifacts::is_reparse_point(root, &metadata) {
        return Err(
            "bridge build root is not a regular directory; refusing recursive cleanup".into(),
        );
    }
    let runner_temp = std::fs::canonicalize(runner_temp)?;
    let workspace = std::fs::canonicalize(workspace)?;
    let canonical_root = std::fs::canonicalize(root)?;
    if canonical_root.parent() != Some(runner_temp.as_path())
        || canonical_root.starts_with(&workspace)
    {
        return Err("bridge build root escaped its direct RUNNER_TEMP boundary".into());
    }
    let sentinel = canonical_root.join(BRIDGE_BUILD_OWNER_SENTINEL);
    let sentinel_metadata = std::fs::symlink_metadata(&sentinel)?;
    if !sentinel_metadata.is_file()
        || crate::ci_artifacts::is_reparse_point(&sentinel, &sentinel_metadata)
        || std::fs::read(&sentinel)? != BRIDGE_BUILD_OWNER_BYTES
    {
        return Err("bridge build owner sentinel changed; refusing recursive cleanup".into());
    }
    std::fs::remove_dir_all(canonical_root)?;
    Ok(())
}

#[cfg(windows)]
fn remove_incomplete_bridge_build_root(
    root: &Path,
    runner_temp: &Path,
    workspace: &Path,
    sentinel_created: bool,
) -> Result<()> {
    let metadata = std::fs::symlink_metadata(root)?;
    if !metadata.is_dir() || crate::ci_artifacts::is_reparse_point(root, &metadata) {
        return Err("incomplete bridge root is not a regular directory".into());
    }
    let runner_temp = std::fs::canonicalize(runner_temp)?;
    let workspace = std::fs::canonicalize(workspace)?;
    let canonical_root = std::fs::canonicalize(root)?;
    if canonical_root.parent() != Some(runner_temp.as_path())
        || canonical_root.starts_with(&workspace)
    {
        return Err("incomplete bridge root escaped its direct RUNNER_TEMP boundary".into());
    }
    let sentinel = canonical_root.join(BRIDGE_BUILD_OWNER_SENTINEL);
    match std::fs::symlink_metadata(&sentinel) {
        Ok(metadata)
            if metadata.is_file()
                && !crate::ci_artifacts::is_reparse_point(&sentinel, &metadata)
                && sentinel_created =>
        {
            let bytes = std::fs::read(&sentinel)?;
            if !BRIDGE_BUILD_OWNER_BYTES.starts_with(&bytes) {
                return Err("partial bridge owner sentinel changed; refusing rollback".into());
            }
            std::fs::remove_file(&sentinel)?;
        }
        Ok(_) => {
            return Err(
                "unexpected bridge owner sentinel; refusing incomplete-root rollback".into(),
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !sentinel_created => {}
        Err(error) => return Err(error.into()),
    }
    std::fs::remove_dir(canonical_root)?;
    Ok(())
}

#[cfg(windows)]
fn run_bun(
    cwd: &Path,
    args: &[String],
    extra_env: &[(String, String)],
    target: &Path,
    timeout: Duration,
) -> Result<()> {
    let mut env = extra_env.to_vec();
    env.push((
        "CARGO_TARGET_DIR".to_owned(),
        target.to_string_lossy().into_owned(),
    ));
    env.push((
        "SKY_AUTHENTICODE_MODE".to_owned(),
        "unsigned-zero-budget".to_owned(),
    ));
    crate::process::run_owned_timeout_filtered(
        std::ffi::OsStr::new("bun"),
        args,
        cwd,
        &env,
        &removed_private_environment(),
        timeout,
    )
}

#[cfg(windows)]
fn replace_one_unindented_version(source: &str, version: &str) -> Result<String> {
    let mut matched = 0;
    let mut output = String::new();
    for line in source.split_inclusive('\n') {
        if line.starts_with("version = \"") {
            matched += 1;
            let ending = if line.ends_with("\r\n") {
                "\r\n"
            } else if line.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            output.push_str(&format!("version = \"{version}\"{ending}"));
        } else {
            output.push_str(line);
        }
    }
    if matched != 1 {
        return Err(format!(
            "fixture Cargo manifest requires exactly one root version assignment, found {matched}"
        )
        .into());
    }
    Ok(output)
}

#[cfg(windows)]
fn replace_desktop_lock_version(source: &str, version: &str) -> Result<String> {
    let marker = "[[package]]\nname = \"sky_desktop_shell\"\nversion = \"";
    let marker_crlf = marker.replace('\n', "\r\n");
    let (needle, ending) = if source.matches(&marker).count() == 1 {
        (marker, "\n")
    } else if source.matches(&marker_crlf).count() == 1 {
        (marker_crlf.as_str(), "\r\n")
    } else {
        return Err("Cargo.lock must contain exactly one sky_desktop_shell package record".into());
    };
    let start = source.find(needle).ok_or("desktop lock entry missing")? + needle.len();
    let rest = &source[start..];
    let end = rest
        .find('"')
        .ok_or("desktop lock version is unterminated")?;
    let after = &rest[end + 1..];
    if !after.starts_with(ending) {
        return Err("desktop lock version record has an unexpected line ending".into());
    }
    Ok(format!("{}{version}{}{}", &source[..start], "\"", after))
}

fn validate_supported_bridge_identity(publisher: &str, identifier: &str) -> Result<()> {
    if !matches!(publisher, "github" | "pumni") {
        return Err("bridge fixture publisher must be github or pumni".into());
    }
    if identifier != UPDATER_PRODUCT_IDENTIFIER {
        return Err("bridge fixture identity must retain the canonical Tauri identifier".into());
    }
    Ok(())
}

fn validate_historical_bridge_manifest_identity(publisher: &str, identifier: &str) -> Result<()> {
    if publisher != "github" || identifier != UPDATER_PRODUCT_IDENTIFIER {
        return Err("updater fixture bridge must retain the historical github publisher and canonical identifier".into());
    }
    Ok(())
}

fn validate_bridge_tauri_config(config: &serde_json::Value, publisher: &str) -> Result<()> {
    validate_supported_bridge_identity(
        publisher,
        config
            .get("identifier")
            .and_then(serde_json::Value::as_str)
            .ok_or("bridge Tauri config identifier is missing or invalid")?,
    )?;
    let bundle = config
        .get("bundle")
        .and_then(serde_json::Value::as_object)
        .ok_or("bridge Tauri config bundle must be an object")?;
    if bundle.get("publisher").and_then(serde_json::Value::as_str) != Some(publisher) {
        return Err("bridge Tauri bundle publisher does not match bridge manifest identity".into());
    }
    if bundle.contains_key("installerHooks") {
        return Err(
            "bridge Tauri config contains a misplaced top-level NSIS installer hook".into(),
        );
    }
    let nsis = bundle
        .get("windows")
        .and_then(|windows| windows.get("nsis"))
        .and_then(serde_json::Value::as_object)
        .ok_or("bridge Tauri config is missing bundle.windows.nsis")?;
    match publisher {
        "github" if nsis.contains_key("installerHooks") => Err(
            "historical bridge Tauri config must omit the nested N+1 NSIS migration hook".into(),
        ),
        "pumni"
            if nsis
                .get("installerHooks")
                .and_then(serde_json::Value::as_str)
                != Some(CURRENT_NSIS_INSTALLER_HOOK) =>
        {
            Err("current candidate Tauri config must retain its nested NSIS migration hook".into())
        }
        _ => Ok(()),
    }
}

fn prepare_bridge_tauri_config(config: &mut serde_json::Value, publisher: &str) -> Result<()> {
    validate_supported_bridge_identity(
        publisher,
        config
            .get("identifier")
            .and_then(serde_json::Value::as_str)
            .ok_or("bridge Tauri config identifier is missing or invalid")?,
    )?;
    let hook = {
        let bundle = config
            .get("bundle")
            .and_then(serde_json::Value::as_object)
            .ok_or("Tauri config bundle must be an object")?;
        if bundle.contains_key("installerHooks") {
            return Err("Tauri config has a misplaced top-level NSIS installer hook".into());
        }
        bundle
            .get("windows")
            .and_then(|windows| windows.get("nsis"))
            .and_then(serde_json::Value::as_object)
            .ok_or("Tauri config is missing bundle.windows.nsis")?
            .get("installerHooks")
            .and_then(serde_json::Value::as_str)
            .ok_or("Tauri config is missing bundle.windows.nsis.installerHooks")?
            .to_owned()
    };
    if hook != CURRENT_NSIS_INSTALLER_HOOK {
        return Err(
            "Tauri config NSIS installer hook does not match the current migration hook".into(),
        );
    }
    let bundle = config
        .get_mut("bundle")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or("Tauri config bundle must be an object")?;
    bundle.insert(
        "publisher".to_owned(),
        serde_json::Value::String(publisher.to_owned()),
    );
    if publisher == "github" {
        bundle
            .get_mut("windows")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|windows| windows.get_mut("nsis"))
            .and_then(serde_json::Value::as_object_mut)
            .ok_or("Tauri config is missing bundle.windows.nsis")?
            .remove("installerHooks");
    }
    validate_bridge_tauri_config(config, publisher)
}

fn persist_bridge_tauri_config(
    worktree_root: &Path,
    config_source_path: &Path,
    config: &serde_json::Value,
    publisher: &str,
) -> Result<()> {
    validate_bridge_tauri_config(config, publisher)?;
    let root = std::fs::canonicalize(worktree_root)?;
    let metadata = std::fs::symlink_metadata(config_source_path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("ephemeral bridge Tauri config must be a regular file".into());
    }
    let canonical_config = std::fs::canonicalize(config_source_path)?;
    if !canonical_config.starts_with(&root) {
        return Err("ephemeral bridge Tauri config escaped its temporary Git worktree".into());
    }
    let bytes = serde_json::to_vec_pretty(config)?;
    std::fs::write(&canonical_config, &bytes)?;
    let persisted: serde_json::Value = serde_json::from_slice(&std::fs::read(&canonical_config)?)?;
    validate_bridge_tauri_config(&persisted, publisher)?;
    if persisted != *config {
        return Err(
            "persisted bridge Tauri config differs from the validated in-memory config".into(),
        );
    }
    Ok(())
}

#[cfg(windows)]
fn validate_loopback_build_config(config: &serde_json::Value) -> Result<()> {
    let updater = config
        .get("plugins")
        .and_then(|plugins| plugins.get("updater"))
        .ok_or("generated updater config is missing")?;
    let endpoints = updater
        .get("endpoints")
        .and_then(serde_json::Value::as_array)
        .ok_or("fixture updater endpoints are missing")?;
    if endpoints.len() != 1
        || endpoints[0]
            .as_str()
            .is_none_or(|endpoint| !endpoint.starts_with("http://127.0.0.1:"))
    {
        return Err("bridge build configuration contains a non-loopback updater endpoint".into());
    }
    let pubkey = updater
        .get("pubkey")
        .and_then(serde_json::Value::as_str)
        .ok_or("fixture updater public key is missing")?;
    validate_public_key_text(pubkey)?;
    Ok(())
}

#[cfg(windows)]
fn safe_sentinel_relative_path(path: &str) -> Result<PathBuf> {
    let relative = path
        .strip_prefix("sheets/")
        .ok_or("bridge sentinel path is outside sheets/")?;
    let output = Path::new(relative);
    if output.is_absolute()
        || output
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
        || relative.contains('\\')
    {
        return Err("bridge sentinel relative path is unsafe".into());
    }
    Ok(output.to_owned())
}

#[cfg(windows)]
fn direct_regular_files(root: &Path) -> Result<Vec<PathBuf>> {
    let metadata = std::fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Tauri NSIS bundle path is not a regular directory".into());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err("Tauri NSIS bundle contains an unexpected non-file object".into());
        }
        files.push(entry.path());
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge_tauri_config() -> serde_json::Value {
        serde_json::json!({
            "identifier": UPDATER_PRODUCT_IDENTIFIER,
            "bundle": {
                "publisher": "pumni",
                "windows": {
                    "nsis": {
                        "installMode": "currentUser",
                        "installerHooks": CURRENT_NSIS_INSTALLER_HOOK
                    }
                }
            }
        })
    }

    fn temporary_config_root() -> std::path::PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "sky-bridge-tauri-config-test-{}-{nonce:x}",
            std::process::id()
        ))
    }

    #[test]
    fn windows_install_location_parser_accepts_absolute_nsis_paths_with_optional_quotes() {
        for (raw, expected) in [
            (
                r"D:\Program Files\Sky Auto Player",
                r"D:\Program Files\Sky Auto Player",
            ),
            (
                r#""D:\Program Files\Sky Auto Player""#,
                r"D:\Program Files\Sky Auto Player",
            ),
            (
                r"\\?\D:\a\_temp\fixture\install",
                r"\\?\D:\a\_temp\fixture\install",
            ),
            (
                r#""\\?\D:\a\_temp\fixture\install""#,
                r"\\?\D:\a\_temp\fixture\install",
            ),
            (
                r"\\server\share\Sky Auto Player",
                r"\\server\share\Sky Auto Player",
            ),
            (
                r"\\?\UNC\server\share\Sky Auto Player",
                r"\\?\UNC\server\share\Sky Auto Player",
            ),
            ("D:/Apps/Sky", r"D:\Apps\Sky"),
        ] {
            assert_eq!(
                parse_windows_install_location(raw).expect("valid absolute Windows path"),
                expected,
                "path syntax: {raw:?}"
            );
        }
    }

    #[test]
    fn windows_install_location_parser_rejects_relative_or_malformed_paths() {
        for raw in [
            "",
            " relative\\install",
            "D:\\Apps\\install ",
            "relative\\install",
            r"D:relative\install",
            r"\rooted\without\drive",
            r#""D:\install"#,
            r#"D:\install""#,
            r#""D:\bad"name""#,
            r#""D:\install" suffix"#,
            r"\\.\PhysicalDrive0",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1",
            r"D:\Apps\..\install",
            r"D:\Apps\\install",
            "D:\\Apps\\install\t",
            "D:\\Apps\\install\n",
        ] {
            assert!(
                parse_windows_install_location(raw).is_err(),
                "malformed or non-absolute path must fail closed: {raw:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn registry_install_location_quotes_resolve_only_to_managed_fixture_root() {
        struct TestRoot(std::path::PathBuf);
        impl Drop for TestRoot {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let root = temporary_config_root();
        std::fs::create_dir_all(&root).expect("create isolated InstallLocation test root");
        let _root = TestRoot(root.clone());
        let managed = root.join("managed");
        let sibling = root.join("sibling");
        std::fs::create_dir(&managed).expect("create managed fixture install root");
        std::fs::create_dir(&sibling).expect("create sibling path for negative comparison");
        let expected = std::fs::canonicalize(&managed).expect("canonicalize managed root");
        let sibling = std::fs::canonicalize(&sibling).expect("canonicalize sibling path");

        for raw in [
            expected.to_string_lossy().into_owned(),
            format!("\"{}\"", expected.display()),
        ] {
            assert_eq!(
                validate_registry_install_location(&raw, &expected)
                    .expect("valid registry path resolves to the managed root"),
                expected
            );
        }
        assert!(
            validate_registry_install_location(&sibling.to_string_lossy(), &expected).is_err(),
            "an absolute sibling directory must not be accepted as the fixture install root"
        );
        for raw in [
            r"relative\install".to_owned(),
            format!("\"{}", expected.display()),
        ] {
            assert!(
                validate_registry_install_location(&raw, &expected).is_err(),
                "relative and malformed quoted registry paths must be rejected"
            );
        }
    }

    #[test]
    fn bridge_tauri_config_persists_historical_identity_and_removes_nested_migration_hook() {
        struct TestRoot(std::path::PathBuf);
        impl Drop for TestRoot {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let root = temporary_config_root();
        let _root = TestRoot(root.clone());
        let source_root = root.join("source");
        let config_path = source_root.join("desktop/src-tauri/tauri.conf.json");
        std::fs::create_dir_all(config_path.parent().expect("config parent"))
            .expect("create disposable worktree config path");
        let mut config = bridge_tauri_config();
        std::fs::write(
            &config_path,
            serde_json::to_vec_pretty(&config).expect("serialize source config"),
        )
        .expect("write original disposable worktree config");
        prepare_bridge_tauri_config(&mut config, "github")
            .expect("prepare legacy bridge-only bundle identity");
        persist_bridge_tauri_config(&source_root, &config_path, &config, "github")
            .expect("persist and validate the isolated bridge config");

        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&config_path).expect("read persisted config"))
                .expect("persisted config remains valid JSON");
        validate_bridge_tauri_config(&persisted, "github")
            .expect("persisted config must match bridge metadata identity");
        assert_eq!(
            persisted["bundle"]["publisher"],
            serde_json::Value::String("github".to_owned())
        );
        assert_eq!(
            persisted["identifier"],
            serde_json::Value::String(UPDATER_PRODUCT_IDENTIFIER.to_owned())
        );
        assert!(
            persisted["bundle"]["windows"]["nsis"]
                .get("installerHooks")
                .is_none()
        );
        assert_eq!(
            persisted["bundle"]["windows"]["nsis"]["installMode"],
            "currentUser"
        );
        assert!(persisted["bundle"].get("installerHooks").is_none());

        let outside_config = root.join("outside-worktree-tauri.conf.json");
        let outside_bytes = br#"{"publisher":"must remain unchanged"}"#;
        std::fs::write(&outside_config, outside_bytes).expect("write outside sentinel config");
        assert!(
            persist_bridge_tauri_config(&source_root, &outside_config, &config, "github").is_err()
        );
        assert_eq!(
            std::fs::read(&outside_config).expect("read outside sentinel config"),
            outside_bytes,
            "bridge config writer must not modify a file outside its temporary Git worktree"
        );
    }

    #[test]
    fn bridge_tauri_config_preserves_current_candidate_migration_hook() {
        let mut config = bridge_tauri_config();
        prepare_bridge_tauri_config(&mut config, "pumni")
            .expect("prepare current candidate bundle identity");
        validate_bridge_tauri_config(&config, "pumni")
            .expect("candidate config retains the current nested migration hook");
        assert_eq!(config["bundle"]["publisher"], "pumni");
        assert_eq!(
            config["bundle"]["windows"]["nsis"]["installerHooks"],
            CURRENT_NSIS_INSTALLER_HOOK
        );
    }

    #[test]
    fn bridge_tauri_config_rejects_wrong_hook_path_and_mismatched_manifest_identity() {
        let mut misplaced = bridge_tauri_config();
        let bundle = misplaced
            .get_mut("bundle")
            .and_then(serde_json::Value::as_object_mut)
            .expect("bundle object");
        bundle.insert(
            "installerHooks".to_owned(),
            serde_json::Value::String(CURRENT_NSIS_INSTALLER_HOOK.to_owned()),
        );
        bundle["windows"]["nsis"]
            .as_object_mut()
            .expect("nsis object")
            .remove("installerHooks");
        assert!(
            prepare_bridge_tauri_config(&mut misplaced, "github").is_err(),
            "a top-level hook must not stand in for bundle.windows.nsis.installerHooks"
        );

        let mut wrong_hook = bridge_tauri_config();
        wrong_hook["bundle"]["windows"]["nsis"]["installerHooks"] =
            serde_json::Value::String("wrong-hook.nsh".to_owned());
        assert!(
            prepare_bridge_tauri_config(&mut wrong_hook, "github").is_err(),
            "unknown migration hooks must fail closed"
        );

        let mut bridge_config = bridge_tauri_config();
        prepare_bridge_tauri_config(&mut bridge_config, "github").unwrap();
        assert!(
            validate_bridge_tauri_config(&bridge_config, "pumni").is_err(),
            "Tauri publisher must agree with the bridge manifest publisher"
        );
        assert!(
            validate_historical_bridge_manifest_identity("pumni", UPDATER_PRODUCT_IDENTIFIER)
                .is_err()
        );
        assert!(
            validate_historical_bridge_manifest_identity("github", "wrong.identifier").is_err()
        );
    }

    #[cfg(windows)]
    fn bridge_test_roots() -> (PathBuf, PathBuf, PathBuf) {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "sky-bridge-cleanup-test-{}-{nonce:x}",
            std::process::id()
        ));
        let runner_temp = base.join("runner-temp");
        let workspace = base.join("workspace");
        std::fs::create_dir_all(&runner_temp).expect("create synthetic runner temp");
        std::fs::create_dir_all(&workspace).expect("create synthetic workspace");
        (base, runner_temp, workspace)
    }

    #[cfg(windows)]
    #[test]
    fn git_worktree_add_and_remove_accepts_verified_ordinary_windows_paths() {
        struct TestRoot(PathBuf);
        impl Drop for TestRoot {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let (base, runner_temp, workspace) = bridge_test_roots();
        let _root = TestRoot(base.clone());
        let repository = workspace.join("source repository");
        std::fs::create_dir(&repository).expect("create isolated synthetic Git repository");
        let repository = std::fs::canonicalize(&repository).expect("canonicalize source repo");
        let repository_cli = verified_ordinary_windows_path(&repository)
            .expect("source repository must have an ordinary Git-compatible path");

        let init = std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&repository_cli)
            .output()
            .expect("run isolated git init");
        assert!(init.status.success(), "git init failed: {init:?}");
        for (key, value) in [
            ("user.name", "Sky Auto Player fixture"),
            ("user.email", "fixture@example.invalid"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
        ] {
            let configured = std::process::Command::new("git")
                .args(["config", key, value])
                .current_dir(&repository_cli)
                .output()
                .expect("configure isolated synthetic Git repository");
            assert!(
                configured.status.success(),
                "git config failed: {configured:?}"
            );
        }
        std::fs::write(
            repository.join("fixture.txt"),
            b"synthetic worktree fixture\n",
        )
        .expect("write synthetic Git source file");
        let add_source = std::process::Command::new("git")
            .args(["add", "--", "fixture.txt"])
            .current_dir(&repository_cli)
            .output()
            .expect("stage synthetic Git source file");
        assert!(
            add_source.status.success(),
            "git add failed: {add_source:?}"
        );
        let commit = std::process::Command::new("git")
            .args(["commit", "--quiet", "--no-gpg-sign", "-m", "fixture"])
            .current_dir(&repository_cli)
            .output()
            .expect("commit synthetic Git source file");
        assert!(commit.status.success(), "git commit failed: {commit:?}");
        let head = std::process::Command::new("git")
            .args(["rev-parse", "--verify", "HEAD"])
            .current_dir(&repository_cli)
            .output()
            .expect("read synthetic Git HEAD");
        assert!(head.status.success(), "git rev-parse failed: {head:?}");
        let source_sha = String::from_utf8(head.stdout)
            .expect("synthetic Git SHA is UTF-8")
            .trim()
            .to_owned();

        let fixture_root = runner_temp.join("bridge fixture with spaces");
        std::fs::create_dir(&fixture_root).expect("create synthetic bridge root");
        let canonical_fixture_root =
            std::fs::canonicalize(&fixture_root).expect("canonicalize synthetic bridge root");
        let ordinary_fixture_root = verified_ordinary_windows_path(&canonical_fixture_root)
            .expect("bridge root must normalize to a verified ordinary Git path");
        let cargo_target = ordinary_fixture_root.join("target");
        let cargo_target_text = cargo_target
            .to_str()
            .expect("synthetic Cargo target path is UTF-8");
        assert!(
            !cargo_target_text.starts_with(r"\\?\"),
            "Cargo target paths must not retain the Windows extended-length prefix"
        );
        assert_eq!(
            std::fs::canonicalize(cargo_target.parent().unwrap())
                .expect("normalize Cargo target parent"),
            canonical_fixture_root
        );
        let worktree = ordinary_fixture_root.join("source worktree");
        let worktree_text = worktree
            .to_str()
            .expect("synthetic Git worktree path is UTF-8")
            .to_owned();
        assert!(
            !worktree_text.starts_with(r"\\?\"),
            "Git CLI path must not retain the Windows extended-length prefix"
        );

        let added = std::process::Command::new("git")
            .args(["worktree", "add", "--detach"])
            .arg(&worktree_text)
            .arg(&source_sha)
            .current_dir(&repository_cli)
            .output()
            .expect("add isolated synthetic Git worktree");
        assert!(
            added.status.success(),
            "git worktree add failed: {}",
            String::from_utf8_lossy(&added.stderr)
        );
        assert_eq!(
            std::fs::read(worktree.join("fixture.txt")).expect("read synthetic worktree file"),
            b"synthetic worktree fixture\n"
        );

        let removed = std::process::Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&worktree_text)
            .current_dir(&repository_cli)
            .output()
            .expect("remove isolated synthetic Git worktree");
        assert!(
            removed.status.success(),
            "git worktree remove failed: {}",
            String::from_utf8_lossy(&removed.stderr)
        );
        assert!(!worktree.exists(), "synthetic Git worktree must be removed");
        let remaining = std::process::Command::new("git")
            .args(["worktree", "list", "--porcelain"])
            .current_dir(&repository_cli)
            .output()
            .expect("inspect isolated Git worktree registrations");
        assert!(
            remaining.status.success(),
            "git worktree list failed: {remaining:?}"
        );
        assert_eq!(
            String::from_utf8_lossy(&remaining.stdout)
                .lines()
                .filter(|line| line.starts_with("worktree "))
                .count(),
            1,
            "only the synthetic repository's primary worktree may remain"
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires the pinned Bun desktop dependencies; invoked by updater_contract CI"]
    fn tauri_signer_pair_is_verified_by_rust_and_rejects_malformed_or_wrong_pair() {
        struct TestRoot(PathBuf);
        impl Drop for TestRoot {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sky-tauri-signer-pair-test-{}-{nonce:x}",
            std::process::id()
        ));
        std::fs::create_dir(&root).expect("create disposable signer test root");
        let _root = TestRoot(root.clone());
        let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let first_key = root.join("first.key");
        let second_key = root.join("second.key");
        generate_signing_key(&repository_root, &first_key).expect("generate first Tauri key pair");
        generate_signing_key(&repository_root, &second_key)
            .expect("generate second Tauri key pair");

        let payload = b"disposable Tauri signer verification fixture\n";
        let payload_path = root.join("payload.bin");
        std::fs::write(&payload_path, payload).expect("write harmless signer challenge");
        sign_with_key(&repository_root, &first_key, &payload_path)
            .expect("Tauri signer must sign the harmless challenge");
        let signature = read_bounded_text(
            &with_sig_extension(&payload_path),
            MAX_FIXTURE_SIGNATURE_BYTES,
        )
        .expect("read bounded Tauri signature");
        let first_public = read_bounded_text(&with_pub_extension(&first_key), 4096)
            .expect("read first Tauri public key");
        let second_public = read_bounded_text(&with_pub_extension(&second_key), 4096)
            .expect("read second Tauri public key");

        crate::updater_trust::verify_signature_text_with_public_key(
            payload,
            &signature,
            &first_public,
        )
        .expect("Rust Minisign verifier must accept the actual Tauri signer output");
        assert!(
            crate::updater_trust::verify_signature_text_with_public_key(
                payload,
                &signature,
                "malformed public key",
            )
            .is_err(),
            "malformed public keys must fail closed"
        );
        assert!(
            crate::updater_trust::verify_signature_text_with_public_key(
                payload,
                "malformed signature",
                &first_public,
            )
            .is_err(),
            "malformed signatures must fail closed"
        );
        assert!(
            crate::updater_trust::verify_signature_text_with_public_key(
                payload,
                &signature,
                &second_public,
            )
            .is_err(),
            "a different Tauri key pair must not validate the signature"
        );
    }

    fn response(port: u16, path: &str) -> HttpResponse {
        http_get_loopback(&format!("http://127.0.0.1:{port}{path}")).unwrap()
    }

    #[test]
    fn loopback_fixture_serves_exact_bytes_and_records_status_length_and_digest() {
        let manifest = br#"{"version":"4.2.0","platforms":{"windows-x86_64":{"signature":"exact-signature","url":"http://127.0.0.1:42000/candidate/update.exe"}}}"#.to_vec();
        let installer = b"fixed NSIS candidate bytes\0\xff".to_vec();
        let mut server = FixtureServer::start(manifest.clone(), installer.clone()).unwrap();
        let manifest_response = response(server.port(), "/stable");
        assert_eq!(manifest_response.status, 200);
        assert_eq!(manifest_response.content_type, "application/json");
        assert_eq!(manifest_response.content_length, manifest.len() as u64);
        assert_eq!(manifest_response.body, manifest);
        let candidate_response = response(server.port(), "/candidate/update.exe");
        assert_eq!(candidate_response.status, 200);
        assert_eq!(candidate_response.content_type, "application/octet-stream");
        assert_eq!(candidate_response.content_length, installer.len() as u64);
        assert_eq!(candidate_response.body, installer);
        let missing = response(server.port(), "/unapproved");
        assert_eq!(missing.status, 404);
        server.stop().unwrap();
        let requests = server.requests().unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0].body_sha256, sha256_bytes(&manifest));
        assert_eq!(requests[1].body_sha256, sha256_bytes(&installer));
        assert_eq!(requests[2].status_code, 404);
        assert_eq!(requests[2].content_type, "text/plain");
    }

    #[test]
    fn loopback_client_rejects_truncated_response_bodies() {
        let listener =
            TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let responder = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 256];
            while request.len() < 2048 && !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
            }
            assert!(
                request.windows(4).any(|window| window == b"\r\n\r\n"),
                "synthetic HTTP client must finish its request before the server responds"
            );
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 12\r\nConnection: close\r\n\r\nshort")
                .unwrap();
            stream.shutdown(Shutdown::Write).unwrap();
        });
        let error =
            http_get_loopback(&format!("http://127.0.0.1:{}/stable", address.port())).unwrap_err();
        assert!(error.to_string().contains("body length mismatch"));
        responder.join().unwrap();
    }

    #[test]
    fn fixture_server_teardown_is_bounded_for_a_stalled_partial_request() {
        let mut server = FixtureServer::start(b"manifest".to_vec(), b"candidate".to_vec()).unwrap();
        let mut stalled = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        stalled
            .write_all(b"GET /stable HTTP/1.1\r\nHost: 127.0.0.1\r\n")
            .unwrap();
        thread::sleep(Duration::from_millis(50));
        let started = Instant::now();
        let stop = server.stop();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(
            stop.is_ok(),
            "stop must cancel the partial request cleanly: {stop:?}"
        );
        assert!(
            server.thread.is_none(),
            "stop must reap the listener thread"
        );
        drop(stalled);
    }

    #[test]
    fn fixture_response_writer_observes_an_absolute_deadline() {
        struct SlowWriter {
            writes: usize,
        }
        impl Write for SlowWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.writes += 1;
                thread::sleep(Duration::from_millis(20));
                Ok(bytes.len().min(1))
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let mut writer = SlowWriter { writes: 0 };
        let started = Instant::now();
        let deadline = started + Duration::from_millis(55);
        let error = write_all_before_deadline(&mut writer, b"response", deadline, |_, _| Ok(()))
            .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(
            writer.writes <= 4,
            "writer must stop at the absolute deadline"
        );
    }

    #[test]
    fn fixture_server_shutdown_interrupts_a_slow_response_reader_and_reaps_thread() {
        let body = vec![0x5a; 32 * 1024 * 1024];
        let mut server = FixtureServer::start(b"manifest".to_vec(), body).unwrap();
        let mut slow_reader = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
        slow_reader
            .write_all(b"GET /candidate/update.exe HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();

        let request_deadline = Instant::now() + Duration::from_secs(3);
        while server.requests().unwrap().is_empty() && Instant::now() < request_deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(server.requests().unwrap().len(), 1);
        thread::sleep(Duration::from_millis(75));

        let started = Instant::now();
        server.stop().unwrap();
        assert!(started.elapsed() < Duration::from_secs(3));
        assert!(
            server.thread.is_none(),
            "stop must positively reap the server thread"
        );
        drop(slow_reader);
    }

    #[test]
    fn server_switches_only_manifest_payload_and_request_limit_is_bounded() {
        let mut server =
            FixtureServer::start(b"manifest A".to_vec(), b"candidate".to_vec()).unwrap();
        let first = response(server.port(), "/stable");
        assert_eq!(first.body, b"manifest A");
        server.set_manifest(b"manifest B".to_vec()).unwrap();
        let second = response(server.port(), "/stable");
        assert_eq!(second.body, b"manifest B");
        assert_eq!(
            response(server.port(), "/candidate/update.exe").body,
            b"candidate"
        );
        server.stop().unwrap();
        let evidence = server.requests().unwrap();
        assert!(
            evidence
                .iter()
                .any(|entry| entry.body_sha256 == sha256_bytes(b"manifest A"))
        );
        assert!(
            evidence
                .iter()
                .any(|entry| entry.body_sha256 == sha256_bytes(b"manifest B"))
        );
    }

    #[test]
    fn manifest_contract_fails_closed_on_version_url_signature_and_json_errors() {
        let valid = make_manifest(
            "4.2.0",
            "signature",
            "http://127.0.0.1:42000/candidate/update.exe",
            "test",
        )
        .unwrap();
        validate_manifest_bytes(
            &valid,
            "4.2.0",
            "http://127.0.0.1:42000/candidate/update.exe",
        )
        .unwrap();
        assert!(
            validate_manifest_bytes(
                &valid,
                "4.2.1",
                "http://127.0.0.1:42000/candidate/update.exe"
            )
            .is_err()
        );
        assert!(validate_manifest_bytes(&valid, "4.2.0", "http://127.0.0.1:42000/other").is_err());
        assert!(validate_manifest_bytes(b"not json", "4.2.0", "url").is_err());
        assert!(make_manifest("4.2.0", "", "url", "test").is_err());
    }

    #[test]
    fn updater_options_reject_unknown_duplicate_and_incomplete_values() {
        assert!(parse_options(&["--bad".to_owned(), "x".to_owned()]).is_err());
        assert!(parse_options(&["--candidate-root".to_owned()]).is_err());
        assert!(
            parse_options(&[
                "--source-sha".to_owned(),
                "a".repeat(40),
                "--source-sha".to_owned(),
                "b".repeat(40),
            ])
            .is_err()
        );
        assert!(
            parse_options(&[
                "--allow-hosted-disposable-runner".to_owned(),
                "--allow-protected-release-runner".to_owned(),
            ])
            .is_err()
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn actual_updater_installer_command_fails_closed_off_windows() {
        let result = run(Path::new("."), &["run".to_owned()]);
        assert!(result.unwrap_err().to_string().contains("Windows-only"));
    }

    #[test]
    fn exact_loopback_client_refuses_public_and_proxyable_destinations() {
        assert!(http_get_loopback("http://example.com/stable").is_err());
        assert!(http_get_loopback("https://127.0.0.1:42000/stable").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn bridge_build_cleanup_requires_direct_runner_temp_and_exact_owner_sentinel() {
        let (base, runner_temp, workspace) = bridge_test_roots();
        let root = runner_temp.join("owned-build-root");
        std::fs::create_dir(&root).expect("create synthetic bridge root");
        std::fs::write(
            root.join(BRIDGE_BUILD_OWNER_SENTINEL),
            BRIDGE_BUILD_OWNER_BYTES,
        )
        .expect("write synthetic ownership sentinel");
        std::fs::create_dir(root.join("target")).expect("create synthetic build output");
        remove_owned_bridge_build_root(&root, &runner_temp, &workspace)
            .expect("remove owned synthetic build root");
        assert!(!root.exists());

        let tampered = runner_temp.join("tampered-build-root");
        std::fs::create_dir(&tampered).expect("create tampered root");
        std::fs::write(tampered.join(BRIDGE_BUILD_OWNER_SENTINEL), b"foreign owner")
            .expect("write foreign sentinel");
        assert!(remove_owned_bridge_build_root(&tampered, &runner_temp, &workspace).is_err());
        assert!(
            tampered.exists(),
            "invalid ownership proof must retain the tree"
        );

        let outside = workspace.join("outside-build-root");
        std::fs::create_dir(&outside).expect("create out-of-scope root");
        std::fs::write(
            outside.join(BRIDGE_BUILD_OWNER_SENTINEL),
            BRIDGE_BUILD_OWNER_BYTES,
        )
        .expect("write out-of-scope sentinel");
        assert!(remove_owned_bridge_build_root(&outside, &runner_temp, &workspace).is_err());
        assert!(
            outside.exists(),
            "out-of-scope build roots must be retained"
        );
        std::fs::remove_dir_all(base).expect("remove only synthetic build-test roots");
    }

    #[cfg(windows)]
    #[test]
    fn bridge_build_entry_rollback_accepts_only_a_partial_created_sentinel() {
        let (base, runner_temp, workspace) = bridge_test_roots();
        let root = runner_temp.join("partial-build-root");
        std::fs::create_dir(&root).expect("create partial root");
        std::fs::write(
            root.join(BRIDGE_BUILD_OWNER_SENTINEL),
            &BRIDGE_BUILD_OWNER_BYTES[..9],
        )
        .expect("write partial ownership sentinel");
        remove_incomplete_bridge_build_root(&root, &runner_temp, &workspace, true)
            .expect("rollback partial owner sentinel");
        assert!(!root.exists());

        let foreign = runner_temp.join("foreign-partial-root");
        std::fs::create_dir(&foreign).expect("create foreign partial root");
        std::fs::write(foreign.join(BRIDGE_BUILD_OWNER_SENTINEL), b"wrong")
            .expect("write foreign sentinel");
        assert!(
            remove_incomplete_bridge_build_root(&foreign, &runner_temp, &workspace, true).is_err()
        );
        assert!(foreign.exists());
        std::fs::remove_dir_all(base).expect("remove only synthetic rollback-test roots");
    }

    #[cfg(windows)]
    #[test]
    fn fixture_manifest_and_http_response_bytes_have_no_implicit_encoding() {
        let signature = "untrusted comment: fixture\nSGVsbG8=";
        let manifest = make_manifest(
            "4.2.0",
            signature,
            "http://127.0.0.1:42000/candidate/update.exe",
            "notes",
        )
        .unwrap();
        assert_eq!(manifest.last(), Some(&b'\n'));
        validate_manifest_bytes(
            &manifest,
            "4.2.0",
            "http://127.0.0.1:42000/candidate/update.exe",
        )
        .unwrap();
    }
}
