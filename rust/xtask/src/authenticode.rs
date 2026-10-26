#![forbid(unsafe_code)]

use crate::Result;
#[cfg(any(windows, test))]
use crate::ci_artifacts::is_reparse_point;
#[cfg(windows)]
use crate::ci_artifacts::{
    replace_file_with_rollback, require_directory, require_regular_file, sha256_file,
    unique_sibling,
};
#[cfg(any(windows, test))]
use serde_json::{Value, json};
#[cfg(any(windows, test))]
use std::collections::BTreeSet;
#[cfg(any(windows, test))]
use std::ffi::OsStr;
#[cfg(any(windows, test))]
use std::fs;
#[cfg(windows)]
use std::fs::OpenOptions;
#[cfg(windows)]
use std::io::Write;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::process::Command;

#[cfg(windows)]
const ADAPTER_FILE: &str = "scripts/verify_v4_authenticode.ps1";
const UNSIGNED_MODE: &str = "unsigned-zero-budget";
#[cfg(any(windows, test))]
const UNSIGNED_POLICY: &str = "unsigned-project-owned-pe-files-and-canonical-nsis";
#[cfg(any(windows, test))]
const SIGNED_POLICY: &str =
    "exact-signer-independent-authenticode-integrity-with-platform-trust-diagnostic";
#[cfg(any(windows, test))]
const PRODUCTION_POLICY: &str =
    "windows-valid-platform-and-exact-approved-signer-independent-authenticode-integrity";
#[cfg(any(windows, test))]
const SIGNATURE_VALID: &str = "signature-valid-independent-cryptographic-integrity";
#[cfg(any(windows, test))]
const INTEGRITY_VERIFIER: &str = "signedcms-spc-indirect-data-authenticode-hash";
#[cfg(any(windows, test))]
const TEST_TRUST_EXCEPTION: &str = "test-platform-status-not-used-for-integrity";
#[cfg(any(windows, test))]
const MAX_AUTHENTICODE_ARTIFACTS: usize = 256;
#[cfg(any(windows, test))]
const MAX_ARTIFACT_MANIFEST_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    UnsignedZeroBudget,
    Test,
    Production,
}

impl Mode {
    fn parse(value: &str) -> Result<Self> {
        match value {
            UNSIGNED_MODE => Ok(Self::UnsignedZeroBudget),
            "test" => Ok(Self::Test),
            "production" => Ok(Self::Production),
            _ => Err("Authenticode mode must be unsigned-zero-budget, test, or production".into()),
        }
    }

    #[cfg(any(windows, test))]
    fn as_str(self) -> &'static str {
        match self {
            Self::UnsignedZeroBudget => UNSIGNED_MODE,
            Self::Test => "test",
            Self::Production => "production",
        }
    }

    #[cfg(any(windows, test))]
    fn verification_policy(self) -> &'static str {
        match self {
            Self::UnsignedZeroBudget => UNSIGNED_POLICY,
            Self::Test => SIGNED_POLICY,
            Self::Production => PRODUCTION_POLICY,
        }
    }
}

#[cfg(any(windows, test))]
struct Policy {
    mode: Mode,
    expected_thumbprint: Option<String>,
}

pub(crate) fn run(root: &Path, args: &[String]) -> Result<()> {
    if args.first().map(String::as_str) != Some("verify") {
        return Err("usage: cargo xtask authenticode verify --mode <mode> --artifact <path> [--artifact <path>] [--evidence <path>]".into());
    }
    let (mode, artifacts, evidence) = parse_args(&args[1..])?;
    verify(root, mode, &artifacts, evidence.as_deref())
}

fn parse_args(args: &[String]) -> Result<(Mode, Vec<PathBuf>, Option<PathBuf>)> {
    let mut mode = None;
    let mut artifacts = Vec::new();
    let mut evidence = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--mode" => {
                if mode.is_some() {
                    return Err("duplicate --mode option".into());
                }
                index += 1;
                mode = Some(Mode::parse(
                    args.get(index).ok_or("--mode requires a value")?,
                )?);
            }
            "--artifact" => {
                index += 1;
                artifacts.push(PathBuf::from(
                    args.get(index).ok_or("--artifact requires a path")?,
                ));
            }
            "--evidence" => {
                if evidence.is_some() {
                    return Err("duplicate --evidence option".into());
                }
                index += 1;
                evidence = Some(PathBuf::from(
                    args.get(index).ok_or("--evidence requires a path")?,
                ));
            }
            option => return Err(format!("unknown Authenticode option: {option}").into()),
        }
        index += 1;
    }
    let mode = mode.ok_or("--mode is required")?;
    if artifacts.is_empty() {
        return Err("at least one Authenticode artifact is required".into());
    }
    Ok((mode, artifacts, evidence))
}

fn verify(root: &Path, mode: Mode, artifacts: &[PathBuf], evidence: Option<&Path>) -> Result<()> {
    #[cfg(not(windows))]
    {
        let _ = (root, mode, artifacts, evidence);
        Err("Authenticode verification requires Windows".into())
    }
    #[cfg(windows)]
    {
        let policy = policy_from_environment(mode)?;
        let expected_files = resolve_artifacts(artifacts)?;
        let temporary = TemporaryDirectory::create(&std::env::temp_dir())?;
        let artifact_manifest = temporary.path.join("authenticode-artifacts.json");
        write_artifact_manifest(&artifact_manifest, &expected_files)?;
        let adapter_evidence = temporary.path.join("authenticode-adapter-evidence.json");
        let adapter = root.join(ADAPTER_FILE);
        if !adapter.is_file() {
            return Err("Windows Authenticode inspection adapter is missing".into());
        }
        let mut command = Command::new("pwsh");
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(adapter)
            .arg("-Mode")
            .arg(mode.as_str());
        if let Some(expected) = &policy.expected_thumbprint {
            command.arg("-ExpectedThumbprint").arg(expected);
        }
        command.arg("-ArtifactManifest").arg(&artifact_manifest);
        let output = command.arg("-Evidence").arg(&adapter_evidence).output()?;
        if !output.status.success() {
            return Err(format!(
                "Windows Authenticode inspection adapter failed with exit code {}",
                output.status.code().unwrap_or(-1)
            )
            .into());
        }
        let adapter_bytes = fs::read(&adapter_evidence)?;
        let adapter_bytes = adapter_bytes
            .strip_prefix(&[0xef, 0xbb, 0xbf])
            .unwrap_or(&adapter_bytes);
        let raw: Value = serde_json::from_slice(adapter_bytes)?;
        let canonical = validate_adapter_evidence(&raw, &policy, &expected_files)?;
        if let Some(path) = evidence {
            write_evidence(path, &canonical)?;
            println!("[xtask] Authenticode evidence: {}", path.display());
        } else {
            println!(
                "[xtask] Authenticode verification: PASS ({} PE file(s), mode={})",
                artifacts.len(),
                mode.as_str()
            );
        }
        Ok(())
    }
}

#[cfg(windows)]
fn policy_from_environment(mode: Mode) -> Result<Policy> {
    let expected_thumbprint = match mode {
        Mode::UnsignedZeroBudget => None,
        Mode::Test => {
            let thumbprint = required_environment("SKY_AUTHENTICODE_TEST_THUMBPRINT")?;
            let thumbprint = validate_thumbprint(&thumbprint, "test signer thumbprint")?;
            validate_test_pfx_password(
                std::env::var_os("SKY_AUTHENTICODE_TEST_PFX_PASSWORD").as_deref(),
            )?;
            let pfx_path = required_environment("SKY_AUTHENTICODE_TEST_PFX_PATH")?;
            validate_test_pfx(
                Path::new(&pfx_path),
                std::env::var_os("RUNNER_TEMP").as_deref().map(Path::new),
            )?;
            Some(thumbprint)
        }
        Mode::Production => {
            let thumbprint = validate_thumbprint(
                &required_environment("SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT")?,
                "approved signer thumbprint",
            )?;
            let test_thumbprint = optional_environment("SKY_AUTHENTICODE_TEST_THUMBPRINT")
                .map(|value| validate_thumbprint(&value, "test signer thumbprint"))
                .transpose()?;
            if test_thumbprint.as_deref() == Some(thumbprint.as_str()) {
                return Err(
                    "production Authenticode policy rejects the CI test signer identity".into(),
                );
            }
            validate_production_provider(
                optional_environment("SKY_AUTHENTICODE_PROVIDER").as_deref(),
                optional_environment("SKY_AUTHENTICODE_PROVIDER_SCRIPT").as_deref(),
                optional_environment("SKY_AUTHENTICODE_PROVIDER_COMMAND").as_deref(),
            )?;
            Some(thumbprint)
        }
    };
    Ok(Policy {
        mode,
        expected_thumbprint,
    })
}

#[cfg(windows)]
fn required_environment(name: &str) -> Result<String> {
    optional_environment(name).ok_or_else(|| format!("Authenticode policy requires {name}").into())
}

#[cfg(windows)]
fn optional_environment(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

#[cfg(any(windows, test))]
fn validate_thumbprint(value: &str, description: &str) -> Result<String> {
    let value = value.trim();
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{description} must be an exact 40-character SHA-1 thumbprint").into());
    }
    Ok(value.to_ascii_uppercase())
}

#[cfg(any(windows, test))]
fn validate_production_provider(
    provider: Option<&str>,
    script: Option<&str>,
    command: Option<&str>,
) -> Result<()> {
    if provider.is_none_or(|value| value.trim().is_empty()) {
        return Err("production Authenticode policy requires SKY_AUTHENTICODE_PROVIDER".into());
    }
    match (script.is_some(), command.is_some()) {
        (true, false) => {
            let path = Path::new(script.expect("matched script option"));
            let metadata = fs::symlink_metadata(path)
                .map_err(|_| "configured Authenticode provider script does not exist")?;
            if !metadata.is_file() || is_reparse_point(path, &metadata) {
                return Err("configured Authenticode provider script is not a regular file".into());
            }
        }
        (false, true) => {}
        (true, true) => {
            return Err("specify exactly one Authenticode provider script or command".into());
        }
        (false, false) => {
            return Err(
                "production Authenticode requires exactly one provider script or command".into(),
            );
        }
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn validate_test_pfx_password(value: Option<&OsStr>) -> Result<()> {
    if value.is_some_and(|value| !value.is_empty()) {
        return Err(
            "test Authenticode PFX passwords are forbidden because signtool exposes argv".into(),
        );
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn validate_test_pfx(path: &Path, runner_temp: Option<&Path>) -> Result<()> {
    let root = match runner_temp {
        Some(path) => path.to_path_buf(),
        None => std::env::temp_dir(),
    };
    let root =
        fs::canonicalize(root).map_err(|_| "Authenticode test temporary root is unavailable")?;
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "ephemeral Authenticode test PFX does not exist")?;
    if !metadata.is_file() || is_reparse_point(path, &metadata) {
        return Err("ephemeral Authenticode test PFX must be a regular file".into());
    }
    let path =
        fs::canonicalize(path).map_err(|_| "ephemeral Authenticode test PFX cannot be resolved")?;
    if !path.starts_with(&root) || path == root {
        return Err("ephemeral Authenticode test PFX must remain under the temporary root".into());
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("ephemeral Authenticode test PFX name is invalid")?;
    if !valid_test_pfx_name(name) {
        return Err("ephemeral Authenticode test PFX has an unexpected name".into());
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn valid_test_pfx_name(name: &str) -> bool {
    name.strip_prefix("sky-v4-test-signing-")
        .and_then(|name| name.strip_suffix(".pfx"))
        .is_some_and(|nonce| {
            nonce.len() == 32 && nonce.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

#[cfg(windows)]
fn resolve_artifacts(paths: &[PathBuf]) -> Result<Vec<(PathBuf, (String, String))>> {
    if paths.is_empty() || paths.len() > MAX_AUTHENTICODE_ARTIFACTS {
        return Err(format!(
            "Authenticode artifact inventory must contain 1 to {MAX_AUTHENTICODE_ARTIFACTS} files"
        )
        .into());
    }
    let mut names = BTreeSet::new();
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let metadata = require_regular_file(path, "Authenticode artifact")?;
        if is_reparse_point(path, &metadata) {
            return Err("Authenticode artifact must not be a reparse point".into());
        }
        let resolved = fs::canonicalize(path)
            .map_err(|_| "Authenticode artifact path could not be resolved")?;
        let resolved_metadata = require_regular_file(&resolved, "Authenticode artifact")?;
        if is_reparse_point(&resolved, &resolved_metadata) {
            return Err("resolved Authenticode artifact must not be a reparse point".into());
        }
        let extension = resolved
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        if !extension.eq_ignore_ascii_case("exe") && !extension.eq_ignore_ascii_case("dll") {
            return Err("Authenticode evidence target must be a regular PE file".into());
        }
        let name = resolved
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Authenticode artifact filename is not Unicode")?
            .to_owned();
        if !names.insert(name.to_ascii_lowercase()) {
            return Err(format!("duplicate Authenticode evidence filename: {name}").into());
        }
        files.push((resolved.clone(), (name, sha256_file(&resolved)?)));
    }
    Ok(files)
}

#[cfg(any(windows, test))]
fn artifact_manifest_bytes(expected_files: &[(PathBuf, (String, String))]) -> Result<Vec<u8>> {
    if expected_files.is_empty() || expected_files.len() > MAX_AUTHENTICODE_ARTIFACTS {
        return Err(format!(
            "Authenticode artifact inventory must contain 1 to {MAX_AUTHENTICODE_ARTIFACTS} files"
        )
        .into());
    }
    let files = expected_files
        .iter()
        .map(|(path, (name, sha256))| {
            let path = path
                .to_str()
                .ok_or("Authenticode artifact path is not Unicode")?;
            Ok(json!({
                "path": path,
                "name": name,
                "sha256": sha256,
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    let bytes = serde_json::to_vec(&json!({
        "schema_version": 1,
        "files": files,
    }))?;
    if bytes.len() > MAX_ARTIFACT_MANIFEST_BYTES {
        return Err(format!(
            "Authenticode artifact manifest exceeds the {MAX_ARTIFACT_MANIFEST_BYTES}-byte limit"
        )
        .into());
    }
    Ok(bytes)
}

#[cfg(windows)]
fn write_artifact_manifest(
    path: &Path,
    expected_files: &[(PathBuf, (String, String))],
) -> Result<()> {
    let bytes = artifact_manifest_bytes(expected_files)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(any(windows, test))]
fn validate_adapter_evidence(
    payload: &Value,
    policy: &Policy,
    expected_files: &[(PathBuf, (String, String))],
) -> Result<Value> {
    if payload.get("schema_version").and_then(Value::as_u64) != Some(1)
        || payload.get("evidence_type").and_then(Value::as_str)
            != Some("authenticode-windows-inspection")
        || payload.get("mode").and_then(Value::as_str) != Some(policy.mode.as_str())
    {
        return Err(
            "Windows Authenticode adapter returned an unsupported inspection schema or mode".into(),
        );
    }
    let files = payload
        .get("files")
        .and_then(Value::as_array)
        .ok_or("Windows Authenticode adapter evidence has no file list")?;
    if files.len() != expected_files.len() {
        return Err("Windows Authenticode adapter did not cover every requested artifact".into());
    }
    let mut records = Vec::with_capacity(files.len());
    let mut seen = BTreeSet::new();
    for (_, (expected_name, expected_hash)) in expected_files {
        let record = files
            .iter()
            .find(|record| record.get("name").and_then(Value::as_str) == Some(expected_name))
            .ok_or("Windows Authenticode adapter omitted a requested artifact")?;
        if !seen.insert(expected_name.to_ascii_lowercase()) {
            return Err("Windows Authenticode adapter returned a duplicate artifact".into());
        }
        records.push(validate_record(
            record,
            policy,
            expected_name,
            expected_hash,
        )?);
    }
    Ok(json!({
        "schema_version": 1,
        "evidence_type": "authenticode-verification",
        "mode": policy.mode.as_str(),
        "expected_signer_thumbprint": policy.expected_thumbprint,
        "verification_policy": policy.mode.verification_policy(),
        "files": records,
    }))
}

#[cfg(any(windows, test))]
fn validate_record(
    record: &Value,
    policy: &Policy,
    expected_name: &str,
    expected_hash: &str,
) -> Result<Value> {
    if record.as_object().map(serde_json::Map::len) != Some(10)
        || record.get("name").and_then(Value::as_str) != Some(expected_name)
        || record
            .get("sha256")
            .and_then(Value::as_str)
            .map(|value| value.eq_ignore_ascii_case(expected_hash))
            != Some(true)
    {
        return Err(
            "Windows Authenticode inspection identity or SHA-256 does not match its artifact"
                .into(),
        );
    }
    let status = required_record_string(record, "platform_status")?;
    let signer_thumbprint = record
        .get("signer_thumbprint")
        .ok_or("inspection signer field is missing")?;
    let signer_subject = record
        .get("signer_subject")
        .ok_or("inspection subject field is missing")?;
    let algorithm = record
        .get("signed_digest_algorithm")
        .ok_or("inspection digest algorithm field is missing")?;
    let signed_digest = record
        .get("signed_digest")
        .ok_or("inspection signed digest field is missing")?;
    let computed_digest = record
        .get("computed_digest")
        .ok_or("inspection computed digest field is missing")?;
    let integrity_verifier = record
        .get("integrity_verifier")
        .ok_or("inspection integrity verifier field is missing")?;
    let integrity_status = record
        .get("integrity_status")
        .ok_or("inspection integrity status field is missing")?;

    if policy.mode == Mode::UnsignedZeroBudget {
        if status != "NotSigned"
            || !signer_thumbprint.is_null()
            || !signer_subject.is_null()
            || !algorithm.is_null()
            || !signed_digest.is_null()
            || !computed_digest.is_null()
            || !integrity_verifier.is_null()
            || !integrity_status.is_null()
        {
            return Err(
                "unsigned-zero-budget accepts only a real NotSigned PE with no signer".into(),
            );
        }
        return Ok(json!({
            "name": expected_name,
            "status": status,
            "platform_status": status,
            "verification": "authenticode-unsigned-zero-budget",
            "trust_exception": "unsigned-zero-budget-policy",
            "integrity_verifier": "not-applicable-unsigned-zero-budget",
            "integrity_status": "NotSigned",
            "signed_digest_algorithm": null,
            "signed_digest": null,
            "computed_digest": null,
            "sha256": expected_hash,
            "signer_thumbprint": null,
            "signer_subject": null,
        }));
    }

    let expected = policy
        .expected_thumbprint
        .as_deref()
        .ok_or("signed Authenticode mode is missing its approved signer")?;
    let signer_thumbprint = signer_thumbprint
        .as_str()
        .ok_or("signed Authenticode inspection has no embedded signer")?;
    let signer_subject = signer_subject
        .as_str()
        .ok_or("signed Authenticode inspection has no signer subject")?;
    if !accepted_status(policy.mode, status)
        || !signer_thumbprint.eq_ignore_ascii_case(expected)
        || integrity_verifier.as_str() != Some(INTEGRITY_VERIFIER)
        || integrity_status.as_str() != Some("Valid")
    {
        return Err(
            "Authenticode inspection has an unexpected status, signer, or integrity result".into(),
        );
    }
    if policy.mode == Mode::Production
        && signer_subject
            .to_ascii_lowercase()
            .contains("ci v4 test code signing")
    {
        return Err("production Authenticode evidence uses the CI test certificate".into());
    }
    let digest_algorithm = algorithm
        .as_str()
        .ok_or("signed image digest algorithm is missing")?;
    let digest_length = match digest_algorithm {
        "1.3.14.3.2.26" => 40,
        "2.16.840.1.101.3.4.2.1" => 64,
        "2.16.840.1.101.3.4.2.2" => 96,
        "2.16.840.1.101.3.4.2.3" => 128,
        _ => {
            return Err(
                "Authenticode evidence uses an unsupported signed image digest algorithm".into(),
            );
        }
    };
    let signed_digest = signed_digest
        .as_str()
        .ok_or("signed CMS digest is missing")?;
    let computed_digest = computed_digest
        .as_str()
        .ok_or("computed PE digest is missing")?;
    if signed_digest.len() != digest_length
        || computed_digest.len() != digest_length
        || !signed_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !computed_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        || !signed_digest.eq_ignore_ascii_case(computed_digest)
    {
        return Err(
            "Authenticode signed CMS digest does not match the computed PE image digest".into(),
        );
    }
    let trust_exception = if status == "Valid" {
        Value::Null
    } else if policy.mode == Mode::Test {
        json!(TEST_TRUST_EXCEPTION)
    } else {
        return Err("production Authenticode verification requires Windows Valid status".into());
    };
    Ok(json!({
        "name": expected_name,
        "status": status,
        "platform_status": status,
        "verification": SIGNATURE_VALID,
        "trust_exception": trust_exception,
        "integrity_verifier": INTEGRITY_VERIFIER,
        "integrity_status": "Valid",
        "signed_digest_algorithm": digest_algorithm,
        "signed_digest": signed_digest,
        "computed_digest": computed_digest,
        "sha256": expected_hash,
        "signer_thumbprint": signer_thumbprint.to_ascii_uppercase(),
        "signer_subject": signer_subject,
    }))
}

#[cfg(any(windows, test))]
fn accepted_status(mode: Mode, status: &str) -> bool {
    match mode {
        Mode::UnsignedZeroBudget => status == "NotSigned",
        Mode::Production => status == "Valid",
        Mode::Test => matches!(
            status,
            "Valid"
                | "NotTrusted"
                | "UnknownError"
                | "RevocationStatusUnknown"
                | "OfflineRevocation"
        ),
    }
}

#[cfg(any(windows, test))]
fn required_record_string<'a>(record: &'a Value, field: &str) -> Result<&'a str> {
    record
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Authenticode evidence {field} is missing or invalid").into())
}

#[cfg(windows)]
fn write_evidence(path: &Path, value: &Value) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    require_directory(parent, "Authenticode evidence directory")?;
    let staging = unique_sibling(path, "authenticode-staging");
    let bytes = serde_json::to_vec_pretty(value)?;
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    let result = replace_file_with_rollback(&staging, path);
    let _ = fs::remove_file(&staging);
    result
}

#[cfg(windows)]
struct TemporaryDirectory {
    path: PathBuf,
}

#[cfg(windows)]
impl TemporaryDirectory {
    fn create(root: &Path) -> Result<Self> {
        for _ in 0..8 {
            let candidate = unique_sibling(&root.join("sky-authenticode-adapter"), "temporary");
            match fs::create_dir(&candidate) {
                Ok(()) => return Ok(Self { path: candidate }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err("could not allocate a temporary Authenticode adapter directory".into())
    }
}

#[cfg(windows)]
impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unsigned_payload(name: &str, digest: &str) -> Value {
        json!({
            "schema_version": 1,
            "evidence_type": "authenticode-windows-inspection",
            "mode": UNSIGNED_MODE,
            "files": [{
                "name": name,
                "platform_status": "NotSigned",
                "sha256": digest,
                "signer_thumbprint": null,
                "signer_subject": null,
                "signed_digest_algorithm": null,
                "signed_digest": null,
                "computed_digest": null,
                "integrity_verifier": null,
                "integrity_status": null
            }]
        })
    }

    fn signed_payload(mode: Mode, status: &str, thumbprint: &str) -> Value {
        let digest = "ab".repeat(32);
        json!({
            "schema_version": 1,
            "evidence_type": "authenticode-windows-inspection",
            "mode": mode.as_str(),
            "files": [{
                "name": "signed.exe",
                "platform_status": status,
                "sha256": "cd".repeat(32),
                "signer_thumbprint": thumbprint,
                "signer_subject": "CN=CI V4 Test Code Signing",
                "signed_digest_algorithm": "2.16.840.1.101.3.4.2.1",
                "signed_digest": digest,
                "computed_digest": digest,
                "integrity_verifier": INTEGRITY_VERIFIER,
                "integrity_status": "Valid"
            }]
        })
    }

    #[test]
    fn exact_thumbprint_and_production_provider_exclusivity_fail_closed() {
        assert!(validate_thumbprint(&"a".repeat(40), "test").is_ok());
        assert!(validate_thumbprint(&"a".repeat(39), "test").is_err());
        assert!(validate_thumbprint(&format!("{}x", "a".repeat(39)), "test").is_err());
        assert!(validate_production_provider(None, Some("provider.ps1"), None).is_err());
        assert!(validate_production_provider(Some("provider"), None, None).is_err());
        assert!(validate_production_provider(Some("provider"), None, Some("sign.exe")).is_ok());
        assert!(
            validate_production_provider(Some("provider"), Some("sign.ps1"), Some("sign.exe"))
                .is_err()
        );
    }

    #[test]
    fn test_pfx_is_confined_and_a_password_cannot_be_configured() {
        let root = std::env::temp_dir().join(format!(
            "sky-authenticode-policy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let path = root.join("sky-v4-test-signing-0123456789abcdef0123456789abcdef.pfx");
        fs::write(&path, b"synthetic path fixture; not a certificate").unwrap();
        assert!(validate_test_pfx(&path, Some(&root)).is_ok());
        let renamed = root.join("unexpected.pfx");
        fs::copy(&path, &renamed).unwrap();
        assert!(validate_test_pfx(&renamed, Some(&root)).is_err());
        assert!(validate_test_pfx_password(None).is_ok());
        assert!(validate_test_pfx_password(Some(OsStr::new(""))).is_ok());
        assert!(validate_test_pfx_password(Some(OsStr::new("must-not-enter-argv"))).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn test_pfx_name_and_status_allowlist_are_exact() {
        assert!(valid_test_pfx_name(
            "sky-v4-test-signing-0123456789abcdef0123456789abcdef.pfx"
        ));
        assert!(!valid_test_pfx_name(
            "sky-v4-test-signing-0123456789abcdef.pfx"
        ));
        assert!(accepted_status(Mode::Test, "NotTrusted"));
        assert!(!accepted_status(Mode::Test, "HashMismatch"));
        assert!(!accepted_status(Mode::Test, "MadeUpStatus"));
        assert!(!accepted_status(Mode::Production, "NotTrusted"));
    }

    #[test]
    fn unsigned_evidence_requires_platform_notsigned_and_exact_artifact_hash() {
        let policy = Policy {
            mode: Mode::UnsignedZeroBudget,
            expected_thumbprint: None,
        };
        let expected = vec![(
            PathBuf::from("signed.exe"),
            ("signed.exe".into(), "aa".repeat(32)),
        )];
        let good = unsigned_payload("signed.exe", &"aa".repeat(32));
        assert!(validate_adapter_evidence(&good, &policy, &expected).is_ok());
        let mut signed = good.clone();
        signed["files"][0]["platform_status"] = json!("Valid");
        assert!(validate_adapter_evidence(&signed, &policy, &expected).is_err());
        let tampered = unsigned_payload("signed.exe", &"bb".repeat(32));
        assert!(validate_adapter_evidence(&tampered, &policy, &expected).is_err());
    }

    #[test]
    fn artifact_manifest_is_bounded_and_preserves_exact_unicode_inventory() {
        let files = vec![(
            PathBuf::from(r"C:\staging\Sky Player 中\one.exe"),
            ("one.exe".into(), "aa".repeat(32)),
        )];
        let manifest: Value =
            serde_json::from_slice(&artifact_manifest_bytes(&files).unwrap()).unwrap();
        assert_eq!(manifest["schema_version"], json!(1));
        assert_eq!(
            manifest["files"][0]["path"],
            json!(r"C:\staging\Sky Player 中\one.exe")
        );
        assert_eq!(manifest["files"][0]["name"], json!("one.exe"));
        assert_eq!(manifest["files"][0]["sha256"], json!("aa".repeat(32)));

        let too_many = vec![
            (
                PathBuf::from("one.exe"),
                ("one.exe".into(), "aa".repeat(32))
            );
            MAX_AUTHENTICODE_ARTIFACTS + 1
        ];
        assert!(artifact_manifest_bytes(&too_many).is_err());

        let too_large = vec![(
            PathBuf::from("x".repeat(MAX_ARTIFACT_MANIFEST_BYTES)),
            ("one.exe".into(), "aa".repeat(32)),
        )];
        assert!(artifact_manifest_bytes(&too_large).is_err());
    }

    #[test]
    fn adapter_evidence_must_cover_every_requested_file_and_exact_hash() {
        let policy = Policy {
            mode: Mode::UnsignedZeroBudget,
            expected_thumbprint: None,
        };
        let expected = vec![
            (
                PathBuf::from("one.exe"),
                ("one.exe".into(), "aa".repeat(32)),
            ),
            (
                PathBuf::from("two.dll"),
                ("two.dll".into(), "bb".repeat(32)),
            ),
        ];
        let mut payload = unsigned_payload("one.exe", &"aa".repeat(32));
        payload["files"]
            .as_array_mut()
            .unwrap()
            .push(unsigned_payload("two.dll", &"bb".repeat(32))["files"][0].clone());
        assert!(validate_adapter_evidence(&payload, &policy, &expected).is_ok());

        let mut missing = payload.clone();
        missing["files"].as_array_mut().unwrap().pop();
        assert!(validate_adapter_evidence(&missing, &policy, &expected).is_err());

        let mut mismatched_hash = payload;
        mismatched_hash["files"][1]["sha256"] = json!("cc".repeat(32));
        assert!(validate_adapter_evidence(&mismatched_hash, &policy, &expected).is_err());
    }

    #[test]
    fn signed_evidence_requires_cms_digest_signer_and_expected_status() {
        let thumbprint = "A".repeat(40);
        let policy = Policy {
            mode: Mode::Test,
            expected_thumbprint: Some(thumbprint.clone()),
        };
        let expected = vec![(
            PathBuf::from("signed.exe"),
            ("signed.exe".into(), "cd".repeat(32)),
        )];
        let good = signed_payload(Mode::Test, "NotTrusted", &thumbprint);
        assert!(validate_adapter_evidence(&good, &policy, &expected).is_ok());
        let mut bad_digest = good.clone();
        bad_digest["files"][0]["computed_digest"] = json!("ef".repeat(32));
        assert!(validate_adapter_evidence(&bad_digest, &policy, &expected).is_err());
        let bad_signer = signed_payload(Mode::Test, "Valid", &"B".repeat(40));
        assert!(validate_adapter_evidence(&bad_signer, &policy, &expected).is_err());
        let unknown_status = signed_payload(Mode::Test, "Other", &thumbprint);
        assert!(validate_adapter_evidence(&unknown_status, &policy, &expected).is_err());
        let production_policy = Policy {
            mode: Mode::Production,
            expected_thumbprint: Some(thumbprint.clone()),
        };
        let mut test_signer = signed_payload(Mode::Production, "Valid", &thumbprint);
        test_signer["files"][0]["signer_subject"] = json!("CN=CI V4 Test Code Signing");
        assert!(validate_adapter_evidence(&test_signer, &production_policy, &expected).is_err());
        let mut valid_production = test_signer.clone();
        valid_production["files"][0]["signer_subject"] = json!("CN=Approved Publisher");
        assert!(
            validate_adapter_evidence(&valid_production, &production_policy, &expected).is_ok()
        );
    }
}
