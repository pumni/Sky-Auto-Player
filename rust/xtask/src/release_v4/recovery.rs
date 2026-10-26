use super::{
    ATTESTATION_VERIFY_TIMEOUT, REPOSITORY, Transport, asset_api_endpoint, canonical_timestamp,
    decode_contents, required_api, sha256_hex, unique_temp, validate_stable_latest,
};
use crate::{Result, process, release_metadata, repo, version};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const RECOVERY_WORKFLOW: &str =
    "pumni/Sky-Auto-Player/.github/workflows/v4-metadata-recovery.yml@refs/heads/main";
const SIGNER_WORKFLOW: &str = "pumni/Sky-Auto-Player/.github/workflows/release-v4.yml";
const MAX_INSTALLER_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 64 * 1024;
static NEXT_DOWNLOAD_DIRECTORY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    UpdateStable,
    Noop,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct AssetRecord {
    name: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    schema_version: u32,
    action: Action,
    repository: String,
    release_id: u64,
    tag: String,
    version: String,
    source_sha: String,
    workflow_run_id: String,
    workflow_run_attempt: u32,
    published_at: String,
    latest_release_id: u64,
    latest_tag: String,
    latest_source_sha: String,
    assets: Vec<AssetRecord>,
    stable_before_version: Option<String>,
    stable_before_blob_sha: Option<String>,
    stable_before_sha256: Option<String>,
    stable_raw_sha256: Option<String>,
    beta_before_sha256: Option<String>,
    candidate_metadata_sha256: String,
    candidate_metadata: release_metadata::TauriMetadata,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransactionMarker {
    repository: String,
    run_id: String,
    run_attempt: u32,
    source_sha: String,
    version: String,
    tag: String,
}

struct DownloadedAssets {
    directory: PathBuf,
    installer: PathBuf,
    signature: PathBuf,
}

impl Drop for DownloadedAssets {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

trait ArtifactVerifier {
    fn verify_signature(&mut self, installer: &Path, signature: &Path, version: &str)
    -> Result<()>;
    fn verify_attestation(
        &mut self,
        subject: &Path,
        source_sha: &str,
        predicate_type: Option<&str>,
    ) -> Result<()>;
}

struct GitHubArtifactVerifier;

impl ArtifactVerifier for GitHubArtifactVerifier {
    fn verify_signature(
        &mut self,
        installer: &Path,
        signature: &Path,
        version: &str,
    ) -> Result<()> {
        crate::updater_trust::verify_updater_signature_for_version(installer, signature, version)
    }

    fn verify_attestation(
        &mut self,
        subject: &Path,
        source_sha: &str,
        predicate_type: Option<&str>,
    ) -> Result<()> {
        let mut args = vec![
            "attestation".to_owned(),
            "verify".to_owned(),
            subject
                .to_str()
                .ok_or("invalid release attestation subject path")?
                .to_owned(),
            "-R".to_owned(),
            REPOSITORY.to_owned(),
            "--source-digest".to_owned(),
            source_sha.to_owned(),
            "--signer-workflow".to_owned(),
            SIGNER_WORKFLOW.to_owned(),
        ];
        if let Some(predicate_type) = predicate_type {
            args.push("--predicate-type".to_owned());
            args.push(predicate_type.to_owned());
        }
        let output = process::capture_owned_timeout_filtered(
            std::ffi::OsStr::new("gh"),
            &args,
            &repo::root(),
            &[],
            &[],
            ATTESTATION_VERIFY_TIMEOUT,
        )?;
        if !output.status.success() {
            return Err(format!(
                "immutable release attestation verification failed: {}",
                super::safe_cli_error(&String::from_utf8_lossy(&output.stderr))
            )
            .into());
        }
        Ok(())
    }
}

pub(super) fn run(root: &Path, command: &str, args: &[String]) -> Result<()> {
    validate_runtime(root)?;
    let mut tag = None;
    let mut output = None;
    let mut plan_sha256 = None;
    let mut confirmed = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--tag" => tag = Some(super::next_value(args, &mut index, "--tag")?),
            "--output" => {
                output = Some(PathBuf::from(super::next_value(
                    args, &mut index, "--output",
                )?))
            }
            "--plan-sha256" => {
                plan_sha256 = Some(super::next_value(args, &mut index, "--plan-sha256")?)
            }
            "--confirm-metadata-only-recovery" => confirmed = true,
            option => return Err(format!("unknown metadata recovery option: {option}").into()),
        }
        index += 1;
    }
    let tag = tag.ok_or("metadata recovery requires --tag <v4.x.y>")?;
    let mut transport = super::GhTransport::new();
    let mut verifier = GitHubArtifactVerifier;
    match command {
        "metadata-recovery-plan" => {
            let output = output.ok_or("metadata recovery plan requires --output <path>")?;
            if plan_sha256.is_some() || confirmed {
                return Err("metadata recovery plan accepts only --tag and --output".into());
            }
            let plan = build_plan(&tag, &mut transport, &mut verifier)?;
            write_plan(&output, &plan)?;
            println!(
                "V4 metadata recovery plan: PASS (action={:?}; tag={}; plan_sha256={})",
                plan.action,
                plan.tag,
                sha256_hex(&encode_plan(&plan)?)
            );
            Ok(())
        }
        "metadata-recovery-apply" => {
            if output.is_some() || !confirmed {
                return Err(
                    "metadata recovery apply requires --confirm-metadata-only-recovery and no --output".into(),
                );
            }
            let plan_sha256 = plan_sha256
                .as_deref()
                .ok_or("metadata recovery apply requires --plan-sha256 <sha256>")?;
            apply(&tag, plan_sha256, &mut transport, &mut verifier)
        }
        _ => Err("unknown metadata recovery operation".into()),
    }
}

fn validate_runtime(root: &Path) -> Result<()> {
    let repository = env::var("GITHUB_REPOSITORY").map_err(|_| "GITHUB_REPOSITORY is required")?;
    let event = env::var("GITHUB_EVENT_NAME").map_err(|_| "GITHUB_EVENT_NAME is required")?;
    let reference = env::var("GITHUB_REF").map_err(|_| "GITHUB_REF is required")?;
    let workflow =
        env::var("GITHUB_WORKFLOW_REF").map_err(|_| "GITHUB_WORKFLOW_REF is required")?;
    let sha = env::var("GITHUB_SHA").map_err(|_| "GITHUB_SHA is required")?;
    if !repository.eq_ignore_ascii_case(REPOSITORY)
        || event != "workflow_dispatch"
        || reference != "refs/heads/main"
        || workflow != RECOVERY_WORKFLOW
        || sha.len() != 40
        || !sha.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !repo::git_head(root, true)?.eq_ignore_ascii_case(&sha)
    {
        return Err(
            "metadata recovery requires the exact canonical main-only manual workflow context"
                .into(),
        );
    }
    Ok(())
}

fn parse_marker(release: &Value, tag: &str, version: &str) -> Result<TransactionMarker> {
    let body = release["body"]
        .as_str()
        .ok_or("immutable release body is missing the transaction marker")?;
    const MARKER: &str = "<!-- v4-release-tx:";
    let mut matches = body.match_indices(MARKER);
    let (start, _) = matches
        .next()
        .ok_or("immutable release body has no release transaction marker")?;
    if matches.next().is_some() {
        return Err("immutable release body has duplicate transaction markers".into());
    }
    let end = body[start..]
        .find("-->")
        .ok_or("immutable release transaction marker is unterminated")?
        + start;
    if !body[end + 3..].trim().is_empty() {
        return Err("immutable release transaction marker is not the final body content".into());
    }
    let raw = body[start + MARKER.len()..end].trim();
    let marker: TransactionMarker = serde_json::from_str(raw)?;
    if marker.repository != REPOSITORY
        || marker.tag != tag
        || marker.version != version
        || marker.tag != format!("v{}", marker.version)
        || marker.source_sha.len() != 40
        || !marker
            .source_sha
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || marker.run_id.is_empty()
        || marker.run_id.len() > 20
        || !marker.run_id.bytes().all(|byte| byte.is_ascii_digit())
        || marker.run_attempt == 0
        || marker.run_attempt > 100
    {
        return Err("immutable release transaction marker conflicts with the selected tag".into());
    }
    let required_fields = [
        "repository",
        "run_id",
        "run_attempt",
        "source_sha",
        "version",
        "tag",
    ];
    if required_fields
        .iter()
        .any(|field| raw.matches(&format!("\"{field}\"")).count() != 1)
    {
        return Err(
            "immutable release transaction marker contains duplicate identity fields".into(),
        );
    }
    Ok(marker)
}

fn validate_release(release: &Value, id: u64, tag: &str, version: &str) -> Result<()> {
    let parsed = version::parse(version)?;
    version::validate_channel("stable", version, &parsed)?;
    if release["id"].as_u64() != Some(id)
        || id == 0
        || release["tag_name"] != tag
        || tag != format!("v{version}")
        || release["draft"] != false
        || release["prerelease"] != false
        || release["immutable"] != true
    {
        return Err(
            "metadata recovery requires the exact immutable published stable release".into(),
        );
    }
    canonical_timestamp(
        release["published_at"]
            .as_str()
            .ok_or("immutable release timestamp is missing")?,
    )?;
    Ok(())
}

fn validate_run(transport: &mut dyn Transport, marker: &TransactionMarker) -> Result<()> {
    let run = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/actions/runs/{}", marker.run_id),
        None,
    )?;
    if !super::valid_workflow_run(&run, Some(&marker.source_sha), REPOSITORY)
        || run["run_attempt"].as_u64() != Some(u64::from(marker.run_attempt))
        || run["status"] != "completed"
        || run["conclusion"] != "success"
    {
        return Err(
            "release workflow run does not match the immutable release transaction marker".into(),
        );
    }
    Ok(())
}

fn exact_release(value: &Value, expected_id: u64, tag: &str, source_sha: &str) -> Result<()> {
    validate_stable_latest(value)?;
    if value["id"].as_u64() != Some(expected_id)
        || value["tag_name"] != tag
        || !value["target_commitish"]
            .as_str()
            .unwrap_or_default()
            .eq_ignore_ascii_case(source_sha)
    {
        return Err("GitHub Latest conflicts with the selected immutable release".into());
    }
    Ok(())
}

fn read_metadata(
    transport: &mut dyn Transport,
    channel: &str,
) -> Result<(Option<Vec<u8>>, Option<String>)> {
    let endpoint =
        format!("repos/{REPOSITORY}/contents/channels/{channel}/latest.json?ref=release-metadata");
    let Some(value) = transport.request("GET", &endpoint, None)? else {
        return Ok((None, None));
    };
    let bytes = decode_contents(&value)?;
    super::doctor_validate_metadata(&bytes, channel)?;
    let blob_sha = value["sha"]
        .as_str()
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("release-metadata Contents API response omitted a valid blob SHA")?
        .to_owned();
    Ok((Some(bytes), Some(blob_sha)))
}

fn raw_endpoint(channel: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/{REPOSITORY}/release-metadata/channels/{channel}/latest.json"
    )
}

fn build_plan(
    tag: &str,
    transport: &mut dyn Transport,
    verifier: &mut dyn ArtifactVerifier,
) -> Result<Plan> {
    let version_value = tag
        .strip_prefix('v')
        .ok_or("recovery tag must start with v")?;
    let parsed_version = version::parse(version_value)?;
    version::validate_channel("stable", version_value, &parsed_version)?;
    if tag != format!("v{version_value}") {
        return Err("recovery tag is not the canonical stable SemVer tag".into());
    }
    let release = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/releases/tags/{tag}"),
        None,
    )?;
    let release_id = release["id"]
        .as_u64()
        .filter(|id| *id > 0)
        .ok_or("immutable release has no valid numeric id")?;
    validate_release(&release, release_id, tag, version_value)?;
    let source_sha = release["target_commitish"]
        .as_str()
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("immutable release has no exact source SHA")?
        .to_ascii_lowercase();
    let marker = parse_marker(&release, tag, version_value)?;
    if !marker.source_sha.eq_ignore_ascii_case(&source_sha) {
        return Err("release source SHA conflicts with its transaction marker".into());
    }
    validate_run(transport, &marker)?;

    let latest = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/releases/latest"),
        None,
    )?;
    exact_release(&latest, release_id, tag, &source_sha)?;

    let installer_name = release_metadata::canonical_release_installer_name(version_value);
    let signature_name = format!("{installer_name}.sig");
    let assets = release["assets"]
        .as_array()
        .ok_or("immutable release omitted its asset inventory")?;
    let mut names = assets
        .iter()
        .map(|asset| asset["name"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    names.sort_unstable();
    let mut expected_names = vec![installer_name.as_str(), signature_name.as_str()];
    expected_names.sort_unstable();
    if names != expected_names {
        return Err("immutable release assets are not the exact installer/signature pair".into());
    }
    let mut asset_records = Vec::new();
    let directory = loop {
        let sequence = NEXT_DOWNLOAD_DIRECTORY.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = unique_temp(&env::temp_dir(), "sky-v4-metadata-recovery");
        let file_name = format!(
            "{}-{sequence}",
            path.file_name()
                .and_then(|name| name.to_str())
                .ok_or("temporary recovery directory name is invalid")?
        );
        let path = path.with_file_name(file_name);
        match fs::create_dir(&path) {
            Ok(()) => break path,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    };
    let downloaded = DownloadedAssets {
        installer: directory.join(release_metadata::canonical_installer_name(version_value)),
        signature: directory.join(format!(
            "{}.sig",
            release_metadata::canonical_installer_name(version_value)
        )),
        directory,
    };
    for (name, destination, max_size) in [
        (&installer_name, &downloaded.installer, MAX_INSTALLER_BYTES),
        (&signature_name, &downloaded.signature, MAX_SIGNATURE_BYTES),
    ] {
        let asset = assets
            .iter()
            .find(|asset| asset["name"] == *name)
            .ok_or("immutable release asset disappeared during inventory validation")?;
        let size = asset["size"]
            .as_u64()
            .filter(|size| *size > 0 && *size <= max_size)
            .ok_or_else(|| format!("immutable release asset size is invalid: {name}"))?;
        if asset["state"] != "uploaded" {
            return Err(format!("immutable release asset is not uploaded: {name}").into());
        }
        let api_digest = asset["digest"]
            .as_str()
            .and_then(|digest| digest.strip_prefix("sha256:"))
            .filter(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
            .ok_or_else(|| format!("immutable release asset has no valid SHA-256: {name}"))?
            .to_ascii_lowercase();
        let url = asset["url"]
            .as_str()
            .ok_or("release asset API URL is missing")?;
        transport.download(asset_api_endpoint(url)?, destination)?;
        let actual_size = fs::metadata(destination)?.len();
        let actual_digest = sha256_hex(&fs::read(destination)?);
        if actual_size != size || actual_digest != api_digest {
            return Err(format!(
                "immutable release downloaded bytes differ from API digest: {name}"
            )
            .into());
        }
        asset_records.push(AssetRecord {
            name: name.clone(),
            size,
            sha256: api_digest,
        });
    }
    asset_records.sort_by(|left, right| left.name.cmp(&right.name));
    verifier.verify_signature(&downloaded.installer, &downloaded.signature, version_value)?;
    verifier.verify_attestation(&downloaded.installer, &source_sha, None)?;
    verifier.verify_attestation(&downloaded.signature, &source_sha, None)?;
    verifier.verify_attestation(
        &downloaded.installer,
        &source_sha,
        Some("https://spdx.dev/Document/v2.3"),
    )?;

    let notes = release["body"]
        .as_str()
        .and_then(|body| {
            body.split_once("<!-- v4-release-tx:")
                .map(|(notes, _)| notes)
        })
        .ok_or("immutable release notes are missing")?;
    let notes = notes.trim_end_matches(['\r', '\n']);
    let published_at = canonical_timestamp(
        release["published_at"]
            .as_str()
            .ok_or("immutable release timestamp is missing")?,
    )?;
    let signature_text = fs::read_to_string(&downloaded.signature)?;
    let candidate_bytes = release_metadata::generate_bytes(
        release_metadata::Channel::Stable,
        version_value,
        notes,
        &published_at,
        release_metadata::WINDOWS_PLATFORM,
        &release_metadata::canonical_asset_url(version_value),
        &signature_text,
    )?;
    let candidate_metadata: release_metadata::TauriMetadata =
        serde_json::from_slice(&candidate_bytes)?;
    let (stable, stable_blob_sha) = read_metadata(transport, "stable")?;
    let (beta, _) = read_metadata(transport, "beta")?;
    let raw_stable = transport.raw_get(&raw_endpoint("stable"))?;
    if raw_stable != stable {
        return Err(
            "public stable endpoint does not match the release-metadata branch snapshot".into(),
        );
    }
    let stable_before_version = if let Some(bytes) = stable.as_deref() {
        let metadata: release_metadata::TauriMetadata = serde_json::from_slice(bytes)?;
        let current = version::parse(&metadata.version)?;
        let candidate = version::parse(version_value)?;
        if bytes != candidate_bytes && current >= candidate {
            return Err(
                "metadata recovery cannot replace same-version or newer stable metadata".into(),
            );
        }
        Some(metadata.version)
    } else {
        None
    };
    let action = if stable.as_deref() == Some(candidate_bytes.as_slice()) {
        Action::Noop
    } else {
        Action::UpdateStable
    };
    let beta_sha256 = beta.as_deref().map(sha256_hex);
    Ok(Plan {
        schema_version: 1,
        action,
        repository: REPOSITORY.to_owned(),
        release_id,
        tag: tag.to_owned(),
        version: version_value.to_owned(),
        source_sha,
        workflow_run_id: marker.run_id,
        workflow_run_attempt: marker.run_attempt,
        published_at,
        latest_release_id: latest["id"].as_u64().ok_or("GitHub Latest id is missing")?,
        latest_tag: latest["tag_name"]
            .as_str()
            .ok_or("GitHub Latest tag is missing")?
            .to_owned(),
        latest_source_sha: latest["target_commitish"]
            .as_str()
            .ok_or("GitHub Latest source SHA is missing")?
            .to_ascii_lowercase(),
        assets: asset_records,
        stable_before_version,
        stable_before_blob_sha: stable_blob_sha,
        stable_before_sha256: stable.as_deref().map(sha256_hex),
        stable_raw_sha256: raw_stable.as_deref().map(sha256_hex),
        beta_before_sha256: beta_sha256,
        candidate_metadata_sha256: sha256_hex(&candidate_bytes),
        candidate_metadata,
    })
}

fn encode_plan(plan: &Plan) -> Result<Vec<u8>> {
    Ok((serde_json::to_string_pretty(plan)? + "\n").into_bytes())
}

fn write_plan(destination: &Path, plan: &Plan) -> Result<()> {
    let runner_temp = env::var_os("RUNNER_TEMP").ok_or("RUNNER_TEMP is required")?;
    let runner_temp = fs::canonicalize(PathBuf::from(runner_temp))?;
    let parent = destination
        .parent()
        .ok_or("metadata recovery plan path has no parent")?;
    let parent = fs::canonicalize(parent)?;
    if !parent.starts_with(&runner_temp) {
        return Err("metadata recovery plan must be written under RUNNER_TEMP".into());
    }
    let destination = parent.join(
        destination
            .file_name()
            .ok_or("metadata recovery plan path has no file name")?,
    );
    let bytes = encode_plan(plan)?;
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)?;
    file.write_all(&bytes)?;
    Ok(())
}

fn apply(
    tag: &str,
    expected_plan_sha256: &str,
    transport: &mut dyn Transport,
    verifier: &mut dyn ArtifactVerifier,
) -> Result<()> {
    if expected_plan_sha256.len() != 64
        || !expected_plan_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("metadata recovery plan hash must be a SHA-256 digest".into());
    }
    let plan = build_plan(tag, transport, verifier)?;
    let plan_hash = sha256_hex(&encode_plan(&plan)?);
    if !plan_hash.eq_ignore_ascii_case(expected_plan_sha256) {
        return Err(format!(
            "metadata recovery plan is stale or differs from operator-reviewed bytes: observed_sha256={plan_hash}"
        )
        .into());
    }
    if plan.action == Action::Noop {
        println!(
            "V4 metadata recovery apply: NOOP (stable pointer already matches exact release bytes)"
        );
        return Ok(());
    }

    let (stable_before, stable_blob_sha) = read_metadata(transport, "stable")?;
    let (beta_before, _) = read_metadata(transport, "beta")?;
    let raw_before = transport.raw_get(&raw_endpoint("stable"))?;
    let latest = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/releases/latest"),
        None,
    )?;
    exact_release(&latest, plan.release_id, &plan.tag, &plan.source_sha)?;
    if stable_before.as_deref().map(sha256_hex) != plan.stable_before_sha256
        || stable_blob_sha != plan.stable_before_blob_sha
        || raw_before.as_deref().map(sha256_hex) != plan.stable_raw_sha256
        || beta_before.as_deref().map(sha256_hex) != plan.beta_before_sha256
    {
        return Err(
            "metadata branch, public endpoint, or beta pointer changed after the reviewed plan"
                .into(),
        );
    }
    if raw_before != stable_before {
        return Err(
            "stable raw endpoint diverged immediately before metadata recovery write".into(),
        );
    }

    let metadata_bytes = release_metadata::generate_bytes(
        release_metadata::Channel::Stable,
        &plan.version,
        &plan.candidate_metadata.notes,
        &plan.published_at,
        release_metadata::WINDOWS_PLATFORM,
        &release_metadata::canonical_asset_url(&plan.version),
        &plan.candidate_metadata.platforms[release_metadata::WINDOWS_PLATFORM].signature,
    )?;
    if sha256_hex(&metadata_bytes) != plan.candidate_metadata_sha256 {
        return Err(
            "reviewed recovery metadata digest does not match its serialized contract".into(),
        );
    }
    let endpoint =
        format!("repos/{REPOSITORY}/contents/channels/stable/latest.json?ref=release-metadata");
    let current = transport.request("GET", &endpoint, None)?;
    if match (stable_before.as_deref(), current.as_ref()) {
        (Some(expected_bytes), Some(current)) => {
            decode_contents(current)? != expected_bytes
                || current["sha"].as_str() != plan.stable_before_blob_sha.as_deref()
        }
        (None, None) => false,
        _ => true,
    } {
        return Err("stable metadata changed at the final Contents API write boundary".into());
    }
    let content = base64::engine::general_purpose::STANDARD.encode(&metadata_bytes);
    let mut payload = json!({
        "message": format!("recover stable metadata for {} (metadata only)", plan.tag),
        "content": content,
        "branch": "release-metadata"
    });
    if let Some(sha) = plan.stable_before_blob_sha.as_deref() {
        payload["sha"] = json!(sha);
    }
    let put_endpoint = format!("repos/{REPOSITORY}/contents/channels/stable/latest.json");
    let put = transport.request("PUT", &put_endpoint, Some(&payload));
    let stored = match put {
        Ok(Some(_)) => required_api(transport, "GET", &endpoint, None)?,
        Ok(None) => super::reconcile_metadata_put(
            transport,
            &endpoint,
            &metadata_bytes,
            "successful recovery PUT response omitted JSON",
        )?,
        Err(error) => super::reconcile_metadata_put(
            transport,
            &endpoint,
            &metadata_bytes,
            &error.to_string(),
        )?,
    };
    if decode_contents(&stored)? != metadata_bytes {
        return Err(
            "recovery Contents API readback differs from the reviewed metadata bytes".into(),
        );
    }
    let beta_after = read_metadata(transport, "beta")?.0;
    if beta_after.as_deref().map(sha256_hex) != plan.beta_before_sha256 {
        return Err("stable metadata recovery changed or conflicted with beta metadata".into());
    }
    let latest_after = required_api(
        transport,
        "GET",
        &format!("repos/{REPOSITORY}/releases/latest"),
        None,
    )?;
    exact_release(&latest_after, plan.release_id, &plan.tag, &plan.source_sha)?;
    let convergence_started = Instant::now();
    super::await_raw_metadata_convergence(
        &metadata_bytes,
        Duration::from_secs(60),
        Duration::from_secs(5),
        || transport.raw_get(&raw_endpoint("stable")),
        std::thread::sleep,
        || convergence_started.elapsed(),
    )?;
    println!(
        "V4 metadata recovery apply: PASS (stable metadata only; tag={}, sha256={})",
        plan.tag, plan.candidate_metadata_sha256
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release_v4::{FakeTransport, fake_release};
    #[derive(Default)]
    struct FakeVerifier {
        signature_calls: usize,
        attestations: Vec<(String, Option<String>)>,
    }

    impl ArtifactVerifier for FakeVerifier {
        fn verify_signature(
            &mut self,
            installer: &Path,
            signature: &Path,
            version: &str,
        ) -> Result<()> {
            if !installer
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains(version)
                || !signature.is_file()
            {
                return Err("fake signature check received mismatched assets".into());
            }
            self.signature_calls += 1;
            Ok(())
        }

        fn verify_attestation(
            &mut self,
            subject: &Path,
            source_sha: &str,
            predicate_type: Option<&str>,
        ) -> Result<()> {
            if source_sha != "a".repeat(40) || !subject.is_file() {
                return Err(
                    "fake attestation check received a mismatched source or subject".into(),
                );
            }
            self.attestations.push((
                subject.file_name().unwrap().to_string_lossy().to_string(),
                predicate_type.map(str::to_owned),
            ));
            Ok(())
        }
    }

    fn recovery_fixture() -> Result<(FakeTransport, FakeVerifier)> {
        let tag = "v4.2.0";
        let version = "4.2.0";
        let source_sha = "a".repeat(40);
        let installer_name = release_metadata::canonical_release_installer_name(version);
        let signature_name = format!("{installer_name}.sig");
        let installer = b"synthetic immutable installer".to_vec();
        let signature = b"dGVzdC1zaWduYXR1cmU=\n".to_vec();
        let installer_url =
            format!("https://api.github.com/repos/{REPOSITORY}/releases/assets/42001");
        let signature_url =
            format!("https://api.github.com/repos/{REPOSITORY}/releases/assets/42002");
        let marker = json!({
            "repository": REPOSITORY,
            "run_id": "7001",
            "run_attempt": 1,
            "source_sha": source_sha,
            "version": version,
            "tag": tag,
        });
        let body = format!(
            "Qualified v4.2.0 notes\n\n<!-- v4-release-tx: {} -->",
            serde_json::to_string(&marker)?
        );
        let mut release = fake_release(42, tag, &source_sha, false, false, true);
        release["published_at"] = json!("2026-10-21T10:39:42Z");
        release["body"] = json!(body);
        release["assets"] = json!([
            {
                "name": installer_name,
                "size": installer.len(),
                "state": "uploaded",
                "digest": format!("sha256:{}", sha256_hex(&installer)),
                "url": installer_url,
            },
            {
                "name": signature_name,
                "size": signature.len(),
                "state": "uploaded",
                "digest": format!("sha256:{}", sha256_hex(&signature)),
                "url": signature_url,
            }
        ]);
        let mut github = FakeTransport::new();
        github.latest = release.clone();
        github.releases.insert(42, release);
        github.assets.insert(installer_url, installer);
        github.assets.insert(signature_url, signature);
        github.workflow_runs.insert(
            "7001".to_owned(),
            json!({
                "path": ".github/workflows/release-v4.yml",
                "event": "workflow_dispatch",
                "repository": {"full_name": REPOSITORY},
                "head_sha": source_sha,
                "run_attempt": 1,
                "status": "completed",
                "conclusion": "success",
            }),
        );
        let current = release_metadata::generate_bytes(
            release_metadata::Channel::Stable,
            "4.1.4",
            "previous stable release",
            "2026-01-01T00:00:00Z",
            release_metadata::WINDOWS_PLATFORM,
            &release_metadata::canonical_asset_url("4.1.4"),
            "cHJldmlvdXM=",
        )?;
        github
            .metadata_files
            .insert("stable".to_owned(), current.clone());
        github
            .raw_metadata_files
            .insert("stable".to_owned(), current);
        let beta = release_metadata::generate_bytes(
            release_metadata::Channel::Beta,
            "4.2.0-beta.1",
            "qualified beta candidate",
            "2026-10-20T10:00:00Z",
            release_metadata::WINDOWS_PLATFORM,
            &release_metadata::canonical_asset_url("4.2.0-beta.1"),
            "YmV0YS1zaWduYXR1cmU=",
        )?;
        github
            .metadata_files
            .insert("beta".to_owned(), beta.clone());
        github.raw_metadata_files.insert("beta".to_owned(), beta);
        Ok((github, FakeVerifier::default()))
    }

    fn digest(plan: &Plan) -> Result<String> {
        Ok(sha256_hex(&encode_plan(plan)?))
    }

    #[test]
    fn read_only_plan_binds_release_source_assets_latest_and_channel_snapshot() {
        let (mut github, mut verifier) = recovery_fixture().unwrap();
        let plan = build_plan("v4.2.0", &mut github, &mut verifier).unwrap();
        assert_eq!(plan.action, Action::UpdateStable);
        assert_eq!(plan.release_id, 42);
        assert_eq!(plan.source_sha, "a".repeat(40));
        assert_eq!(plan.assets.len(), 2);
        assert_eq!(plan.stable_before_version.as_deref(), Some("4.1.4"));
        assert!(
            github
                .events
                .iter()
                .all(|event| event.starts_with("GET ") || event.starts_with("RAW_GET "))
        );
        assert!(!github.events.iter().any(|event| event.starts_with("PUT ")
            || event.starts_with("POST ")
            || event.starts_with("DELETE ")));
        assert_eq!(verifier.signature_calls, 1);
        assert_eq!(verifier.attestations.len(), 3);
        assert_eq!(
            verifier.attestations[2].1.as_deref(),
            Some("https://spdx.dev/Document/v2.3")
        );
    }

    #[test]
    fn metadata_recovery_updates_only_stable_and_reconciles_timeout_once() {
        let (mut github, mut verifier) = recovery_fixture().unwrap();
        github.fail_metadata_put_after_write = true;
        let beta_before = github.metadata_files.get("beta").cloned().unwrap();
        let plan = build_plan("v4.2.0", &mut github, &mut verifier).unwrap();
        let before = github
            .events
            .iter()
            .filter(|event| event.starts_with("PUT "))
            .count();
        apply(
            &plan.tag,
            &digest(&plan).unwrap(),
            &mut github,
            &mut verifier,
        )
        .unwrap();
        let puts = github
            .events
            .iter()
            .filter(|event| event.starts_with("PUT "))
            .count()
            - before;
        assert_eq!(
            puts, 1,
            "a Contents PUT is never retried after an unknown outcome"
        );
        assert!(github.metadata_files.contains_key("stable"));
        assert_eq!(github.metadata_files.get("beta"), Some(&beta_before));
        assert_eq!(
            github.raw_metadata_files.get("stable"),
            github.metadata_files.get("stable")
        );
        assert!(
            github
                .events
                .iter()
                .all(|event| !event.contains("releases/42") || event.starts_with("GET "))
        );
    }

    #[test]
    fn metadata_recovery_unknown_put_outcome_is_not_retried() {
        let (mut github, mut verifier) = recovery_fixture().unwrap();
        let stable_before = github.metadata_files.get("stable").cloned().unwrap();
        let plan = build_plan("v4.2.0", &mut github, &mut verifier).unwrap();
        github.fail_metadata_put_without_write = true;
        let error = apply(
            &plan.tag,
            &digest(&plan).unwrap(),
            &mut github,
            &mut verifier,
        )
        .expect_err("an uncommitted timed-out write must fail closed");
        assert!(error.to_string().contains("does not match candidate bytes"));
        assert_eq!(
            github
                .events
                .iter()
                .filter(|event| event.starts_with("PUT "))
                .count(),
            1
        );
        assert_eq!(github.metadata_files.get("stable"), Some(&stable_before));
    }

    #[test]
    fn metadata_recovery_noop_is_idempotent_and_stale_plan_fails_before_write() {
        let (mut github, mut verifier) = recovery_fixture().unwrap();
        let plan = build_plan("v4.2.0", &mut github, &mut verifier).unwrap();
        assert!(apply(&plan.tag, &"0".repeat(64), &mut github, &mut verifier).is_err());
        assert!(!github.events.iter().any(|event| event.starts_with("PUT ")));
        apply(
            &plan.tag,
            &digest(&plan).unwrap(),
            &mut github,
            &mut verifier,
        )
        .unwrap();
        let plan = build_plan("v4.2.0", &mut github, &mut verifier).unwrap();
        assert_eq!(plan.action, Action::Noop);
        let before = github
            .events
            .iter()
            .filter(|event| event.starts_with("PUT "))
            .count();
        apply(
            &plan.tag,
            &digest(&plan).unwrap(),
            &mut github,
            &mut verifier,
        )
        .unwrap();
        assert_eq!(
            github
                .events
                .iter()
                .filter(|event| event.starts_with("PUT "))
                .count(),
            before
        );
    }

    #[test]
    fn latest_conflict_duplicate_assets_wrong_tag_and_tampered_bytes_fail_closed() {
        for mutation in 0..5 {
            let (mut github, mut verifier) = recovery_fixture().unwrap();
            match mutation {
                0 => github.latest["id"] = json!(99),
                1 => {
                    github.releases.get_mut(&42).unwrap()["assets"] = json!([
                        github.releases[&42]["assets"][0].clone(),
                        github.releases[&42]["assets"][0].clone()
                    ])
                }
                2 => github.releases.get_mut(&42).unwrap()["tag_name"] = json!("v4.2.1"),
                3 => {
                    github.assets.insert(
                        "https://api.github.com/repos/pumni/Sky-Auto-Player/releases/assets/42001"
                            .to_owned(),
                        b"tampered installer".to_vec(),
                    );
                }
                _ => {
                    github.releases.get_mut(&42).unwrap()["target_commitish"] =
                        json!("b".repeat(40))
                }
            }
            assert!(build_plan("v4.2.0", &mut github, &mut verifier).is_err());
            assert!(!github.events.iter().any(|event| event.starts_with("PUT ")));
        }
        let (mut github, mut verifier) = recovery_fixture().unwrap();
        assert!(build_plan("v4.2.1", &mut github, &mut verifier).is_err());
        assert!(!github.events.iter().any(|event| event.starts_with("PUT ")));
    }

    #[test]
    fn metadata_recovery_refuses_same_or_newer_channel_and_changed_reviewed_snapshot() {
        let (mut github, mut verifier) = recovery_fixture().unwrap();
        let same_version = release_metadata::generate_bytes(
            release_metadata::Channel::Stable,
            "4.2.0",
            "different metadata for the already released version",
            "2026-10-21T10:39:42Z",
            release_metadata::WINDOWS_PLATFORM,
            &release_metadata::canonical_asset_url("4.2.0"),
            "dGVzdC1zaWduYXR1cmU=",
        )
        .expect("valid same-version metadata fixture");
        github
            .metadata_files
            .insert("stable".to_owned(), same_version.clone());
        github
            .raw_metadata_files
            .insert("stable".to_owned(), same_version);
        assert!(build_plan("v4.2.0", &mut github, &mut verifier).is_err());
        assert!(!github.events.iter().any(|event| event.starts_with("PUT ")));

        let (mut github, mut verifier) = recovery_fixture().unwrap();
        let newer_version = release_metadata::generate_bytes(
            release_metadata::Channel::Stable,
            "4.2.1",
            "metadata already points to a newer stable release",
            "2026-10-22T10:39:42Z",
            release_metadata::WINDOWS_PLATFORM,
            &release_metadata::canonical_asset_url("4.2.1"),
            "dGVzdC1zaWduYXR1cmU=",
        )
        .expect("valid newer-version metadata fixture");
        github
            .metadata_files
            .insert("stable".to_owned(), newer_version.clone());
        github
            .raw_metadata_files
            .insert("stable".to_owned(), newer_version);
        assert!(build_plan("v4.2.0", &mut github, &mut verifier).is_err());
        assert!(!github.events.iter().any(|event| event.starts_with("PUT ")));

        let (mut github, mut verifier) = recovery_fixture().unwrap();
        let plan = build_plan("v4.2.0", &mut github, &mut verifier).unwrap();
        github.latest["id"] = json!(77);
        assert!(
            apply(
                &plan.tag,
                &digest(&plan).unwrap(),
                &mut github,
                &mut verifier
            )
            .is_err()
        );
        assert!(!github.events.iter().any(|event| event.starts_with("PUT ")));
    }
}
