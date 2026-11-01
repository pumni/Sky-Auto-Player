#![forbid(unsafe_code)]

use crate::{Result, ci_artifacts, hash, process, updater_trust};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use zeroize::{Zeroize, Zeroizing};

const PRIVATE_KEY_SECRET: &str = "V4_TAURI_UPDATER_PRIVATE_KEY";
const PASSPHRASE_SECRET: &str = "V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD";
const MAX_PRIVATE_KEY_BYTES: usize = 32 * 1024;
const MAX_PASSPHRASE_BYTES: usize = 4096;
// Pinned Tauri v2.12.3 uses a fixed 158-byte encrypted Minisign secret-key box.
// The CLI generates "rsign" text but legacy encrypted "minisign" boxes can
// still be parsed, subject to the same production-root cryptographic challenge.
const ENCRYPTED_KEY_BYTES: usize = 158;
const TAURI_RSIGN_HEADER: &str = "untrusted comment: rsign encrypted secret key";
const LEGACY_MINISIGN_HEADER: &str = "untrusted comment: minisign encrypted secret key";
const KEY_FORMAT_ERROR: &str = "protected updater key must contain one Tauri-compatible Base64 envelope of an encrypted secret-key file, or the exact two-line inner text";
const SIGN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const TAURI_CLI_VERSION: &str = "tauri-cli 2.12.3";
struct SigningSecrets {
    private_key: Zeroizing<String>,
    passphrase: Zeroizing<String>,
}

impl SigningSecrets {
    fn from_environment() -> Result<Self> {
        let private_key = env::var(PRIVATE_KEY_SECRET)
            .map_err(|_| "protected updater signing key secret is unavailable")?;
        let passphrase = env::var(PASSPHRASE_SECRET)
            .map_err(|_| "protected updater signing passphrase secret is unavailable")?;
        Self::from_values(private_key, passphrase)
    }

    fn from_values(private_key: String, passphrase: String) -> Result<Self> {
        // Own sensitive values immediately, even if the shape check rejects them.
        let private_key = Zeroizing::new(private_key);
        let passphrase = Zeroizing::new(passphrase);
        if private_key.trim().is_empty()
            || private_key.len() > MAX_PRIVATE_KEY_BYTES
            || private_key.contains('\0')
            || passphrase.trim().is_empty()
            || passphrase.len() > MAX_PASSPHRASE_BYTES
            || passphrase.contains('\0')
        {
            return Err(
                "protected updater signing secret is empty, malformed, or over its size bound"
                    .into(),
            );
        }
        Ok(Self {
            private_key: normalize_tauri_private_key(private_key)?,
            passphrase,
        })
    }
}

/// Pinned Tauri v2.12.3 signer generation returns Base64 of the *entire*
/// two-line encrypted private-key box, and `signer sign --private-key-path`
/// reads a file containing that Base64 string (not the decoded inner text).
/// Accept either canonical Tauri Base64 or the two-line inner text, but ALWAYS
/// return the canonical outer Base64 for materialization. Never recursively
/// decode or relax the production-root cryptographic challenge.
fn normalize_tauri_private_key(value: Zeroizing<String>) -> Result<Zeroizing<String>> {
    if is_encrypted_private_key_text(&value) {
        return canonical_tauri_private_key_file(&value);
    }

    // A transported key is precisely one standard Base64 line, optionally
    // followed by terminal newlines. Other whitespace and nested envelopes
    // are rejected before any secret can reach the Tauri signer.
    let envelope = value.trim_end_matches(['\r', '\n']);
    if envelope.is_empty() || envelope.contains(['\r', '\n']) {
        return Err(KEY_FORMAT_ERROR.into());
    }
    let decoded = Zeroizing::new(STANDARD.decode(envelope).map_err(|_| KEY_FORMAT_ERROR)?);
    if decoded.len() > MAX_PRIVATE_KEY_BYTES {
        return Err(KEY_FORMAT_ERROR.into());
    }
    let inner = std::str::from_utf8(&decoded).map_err(|_| KEY_FORMAT_ERROR)?;
    canonical_tauri_private_key_file(inner)
}

/// Canonicalize only the line separators, not the encrypted payload. Minisign
/// box metadata is untrusted; the original private key must still prove it
/// matches the compiled production public key before the installer is signed.
fn canonical_tauri_private_key_file(inner: &str) -> Result<Zeroizing<String>> {
    if !is_encrypted_private_key_text(inner) {
        return Err(KEY_FORMAT_ERROR.into());
    }
    let mut lines = inner.lines();
    let header = lines.next().ok_or(KEY_FORMAT_ERROR)?;
    let payload = lines.next().ok_or(KEY_FORMAT_ERROR)?;
    let canonical = Zeroizing::new(format!("{header}\n{payload}\n"));
    Ok(Zeroizing::new(STANDARD.encode(canonical.as_bytes())))
}

fn is_encrypted_private_key_text(value: &str) -> bool {
    let mut lines = value.lines();
    let (Some(header), Some(payload)) = (lines.next(), lines.next()) else {
        return false;
    };
    if !matches!(header, TAURI_RSIGN_HEADER | LEGACY_MINISIGN_HEADER) || lines.next().is_some() {
        return false;
    }
    let decoded_payload = match STANDARD.decode(payload) {
        Ok(bytes) => Zeroizing::new(bytes),
        Err(_) => return false,
    };
    decoded_payload.len() == ENCRYPTED_KEY_BYTES
}

struct TemporarySigningKey {
    directory: PathBuf,
    path: PathBuf,
}

impl Drop for TemporarySigningKey {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

struct RemoveOnFailure {
    path: PathBuf,
    keep: bool,
}

impl RemoveOnFailure {
    /// Refuse to adopt or erase a signature that this signer did not create.
    /// symlink_metadata also detects dangling symlinks; exists() does not.
    fn for_new_output(path: PathBuf) -> Result<Self> {
        match fs::symlink_metadata(&path) {
            Ok(_) => return Err("refusing to overwrite a pre-existing updater signature".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(Self { path, keep: false })
    }
}

impl Drop for RemoveOnFailure {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(crate) fn sign_installer(
    root: &Path,
    installer: &Path,
    expected_version: &str,
    expected_size: u64,
    expected_sha256: &str,
    run_id: &str,
    run_attempt: u32,
) -> Result<PathBuf> {
    let metadata = ci_artifacts::require_regular_file(installer, "unsigned candidate installer")?;
    if metadata.len() != expected_size || hash::sha256(installer)? != expected_sha256 {
        return Err("unsigned installer differs from its pre-signing SHA-256 manifest".into());
    }
    ci_artifacts::validate_semver(expected_version, "updater signature version")?;
    if env::var_os("TAURI_SIGNING_PRIVATE_KEY").is_some()
        || env::var_os("TAURI_SIGNING_PRIVATE_KEY_PATH").is_some()
        || env::var_os("TAURI_SIGNING_PRIVATE_KEY_PASSWORD").is_some()
    {
        return Err("ambient Tauri signing variables are forbidden in the protected signer".into());
    }
    if run_id.is_empty()
        || run_id.len() > 20
        || !run_id.bytes().all(|byte| byte.is_ascii_digit())
        || run_attempt == 0
        || run_attempt > 100
    {
        return Err("signing material scope has a malformed workflow run identity".into());
    }
    verify_tauri_cli(root)?;
    let secrets = SigningSecrets::from_environment()?;
    let temporary_key = materialize_key(&secrets, run_id, run_attempt)?;
    let key_id = updater_trust::canonical_key_id()?;
    if key_id != "19AABD2E7838818C" {
        return Err("compiled updater trust root is not the approved production key ID".into());
    }
    updater_trust::verify_local_private_key(root, &temporary_key.path, Some(&secrets.passphrase))?;

    let signature = PathBuf::from(format!("{}.sig", installer.display()));
    let mut cleanup = RemoveOnFailure::for_new_output(signature.clone())?;
    let mut args = vec![
        "run".to_owned(),
        "tauri".to_owned(),
        "signer".to_owned(),
        "sign".to_owned(),
        "--private-key-path".to_owned(),
        temporary_key.path.to_string_lossy().into_owned(),
        "--app-version".to_owned(),
        expected_version.to_owned(),
        installer.to_string_lossy().into_owned(),
    ];
    let mut env_values = vec![
        (
            "TAURI_SIGNING_PRIVATE_KEY_PATH".to_owned(),
            temporary_key.path.to_string_lossy().into_owned(),
        ),
        (
            "TAURI_SIGNING_PRIVATE_KEY_PASSWORD".to_owned(),
            secrets.passphrase.to_string(),
        ),
    ];
    let output = process::capture_owned_timeout_runtime_only(
        std::ffi::OsStr::new("bun"),
        &args,
        &root.join("desktop"),
        &env_values,
        SIGN_TIMEOUT,
    );
    for (name, value) in &mut env_values {
        if name == "TAURI_SIGNING_PRIVATE_KEY_PASSWORD" {
            value.zeroize();
        }
    }
    let output = output?;
    args.clear();
    if !output.status.success() {
        return Err("Tauri standalone updater signing failed".into());
    }
    let signature_metadata = ci_artifacts::require_regular_file(&signature, "updater signature")?;
    if signature_metadata.len() == 0 || signature_metadata.len() > 16 * 1024 {
        return Err("generated updater signature is empty or exceeds its size bound".into());
    }
    if fs::metadata(installer)?.len() != expected_size
        || hash::sha256(installer)? != expected_sha256
    {
        return Err("updater signing changed the exact installer bytes".into());
    }
    updater_trust::verify_updater_signature_for_version(installer, &signature, expected_version)?;
    cleanup.keep = true;
    println!(
        "V4 Rust protected signing: PASS (same installer bytes; version={expected_version}; production key ID={key_id})"
    );
    Ok(signature)
}

fn verify_tauri_cli(root: &Path) -> Result<()> {
    let output = process::capture_owned_timeout_runtime_only(
        std::ffi::OsStr::new("bun"),
        &["run".to_owned(), "tauri".to_owned(), "--version".to_owned()],
        &root.join("desktop"),
        &[],
        Duration::from_secs(60),
    )?;
    let version = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_owned();
    if !output.status.success() || version != TAURI_CLI_VERSION {
        return Err(format!("protected signer requires pinned {TAURI_CLI_VERSION}").into());
    }
    Ok(())
}

fn materialize_key(
    secrets: &SigningSecrets,
    run_id: &str,
    run_attempt: u32,
) -> Result<TemporarySigningKey> {
    let runner_temp = env::var_os("RUNNER_TEMP")
        .ok_or("RUNNER_TEMP is required for protected updater signing")?;
    let runner_temp = fs::canonicalize(PathBuf::from(runner_temp))?;
    let directory = runner_temp.join(format!(
        "sky-v4-signing-material-{run_id}-{run_attempt}-{}",
        std::process::id()
    ));
    fs::create_dir(&directory)?;
    let canonical_directory = fs::canonicalize(&directory)?;
    if canonical_directory.parent() != Some(runner_temp.as_path()) {
        let _ = fs::remove_dir_all(&directory);
        return Err("temporary signing material escaped the direct RUNNER_TEMP boundary".into());
    }
    let path = canonical_directory.join("updater.key");
    let owner = serde_json::json!({"repository":"pumni/Sky-Auto-Player","run_id":run_id,"run_attempt":run_attempt});
    let owner_result = fs::write(
        canonical_directory.join("owner.json"),
        [serde_json::to_vec(&owner)?.as_slice(), b"\n"].concat(),
    );
    if let Err(error) = owner_result {
        let _ = fs::remove_dir_all(&canonical_directory);
        return Err(error.into());
    }
    let mut key_bytes = Zeroizing::new(secrets.private_key.as_bytes().to_vec());
    let write_result = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(&key_bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    key_bytes.fill(0);
    if let Err(error) = write_result {
        let _ = fs::remove_dir_all(&canonical_directory);
        return Err(error);
    }
    Ok(TemporarySigningKey {
        directory: canonical_directory,
        path,
    })
}

pub(crate) fn cleanup_run_material() -> Result<()> {
    let run_id = env::var("GITHUB_RUN_ID").map_err(|_| "GITHUB_RUN_ID is required")?;
    let run_attempt = env::var("GITHUB_RUN_ATTEMPT")
        .map_err(|_| "GITHUB_RUN_ATTEMPT is required")?
        .parse::<u32>()
        .map_err(|_| "GITHUB_RUN_ATTEMPT is malformed")?;
    if run_id.is_empty()
        || run_id.len() > 20
        || !run_id.bytes().all(|byte| byte.is_ascii_digit())
        || run_attempt == 0
        || run_attempt > 100
    {
        return Err("signing cleanup run identity is malformed".into());
    }
    let runner_temp = env::var_os("RUNNER_TEMP").ok_or("RUNNER_TEMP is required")?;
    cleanup_run_material_at(&PathBuf::from(runner_temp), &run_id, run_attempt)
}

fn cleanup_run_material_at(runner_temp: &Path, run_id: &str, run_attempt: u32) -> Result<()> {
    let runner_temp = fs::canonicalize(runner_temp)?;
    let prefix = format!("sky-v4-signing-material-{run_id}-{run_attempt}-");
    for entry in fs::read_dir(&runner_temp)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with(&prefix) {
            continue;
        }
        let path = entry.path();
        ci_artifacts::require_directory(&path, "signing cleanup target is missing")?;
        let resolved = fs::canonicalize(&path)?;
        if resolved.parent() != Some(runner_temp.as_path()) {
            return Err("signing cleanup target escaped RUNNER_TEMP".into());
        }
        let owner_path = resolved.join("owner.json");
        let owner_metadata =
            ci_artifacts::require_regular_file(&owner_path, "signing cleanup owner marker")?;
        if owner_metadata.len() > 1024 {
            return Err("signing cleanup owner marker is unbounded".into());
        }
        let owner: serde_json::Value = serde_json::from_slice(&fs::read(&owner_path)?)
            .map_err(|_| "signing cleanup owner marker is malformed")?;
        if owner["repository"] != "pumni/Sky-Auto-Player"
            || owner["run_id"] != run_id
            || owner["run_attempt"].as_u64() != Some(u64::from(run_attempt))
        {
            return Err("signing cleanup refused a foreign material directory".into());
        }
        let mut names = fs::read_dir(&resolved)?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<std::io::Result<Vec<_>>>()?;
        names.sort();
        if names != ["owner.json".to_owned(), "updater.key".to_owned()] {
            return Err("signing cleanup refused unexpected files in the secret directory".into());
        }
        let key = resolved.join("updater.key");
        let _ = ci_artifacts::require_regular_file(&key, "temporary updater key")?;
        fs::remove_dir_all(resolved)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const PRIVATE_KEY: &str = "untrusted comment: rsign encrypted secret key\nRWQAAAA=";

    // Syntax-only 158-byte encrypted boxes; these can never sign a real update.
    fn encrypted_key_fixture(header: &str) -> String {
        format!(
            "{header}\n{}\n",
            STANDARD.encode([0u8; ENCRYPTED_KEY_BYTES])
        )
    }

    #[test]
    fn signing_secret_adapter_matches_pinned_tauri_base64_file_contract() {
        for header in [TAURI_RSIGN_HEADER, LEGACY_MINISIGN_HEADER] {
            let raw = encrypted_key_fixture(header);
            let canonical_file_bytes = STANDARD.encode(raw.as_bytes());
            let secrets =
                SigningSecrets::from_values(canonical_file_bytes.clone(), "passphrase".to_owned())
                    .expect("outer Base64 from pinned Tauri signer");
            assert!(
                secrets.private_key.as_str() == canonical_file_bytes,
                "outer Base64 must remain the signer-file representation"
            );
            assert_eq!(secrets.passphrase.as_str(), "passphrase");

            let from_raw = SigningSecrets::from_values(raw.clone(), "passphrase".to_owned())
                .expect("raw inner text can be canonicalized");
            assert!(
                from_raw.private_key.as_str() == canonical_file_bytes,
                "raw key must be encoded for the Tauri CLI, not passed as raw text"
            );

            let crlf = raw.replace('\n', "\r\n");
            for input in [crlf.clone(), STANDARD.encode(crlf.as_bytes())] {
                let normalized = SigningSecrets::from_values(input, "passphrase".to_owned())
                    .expect("CRLF is normalized back to Tauri's canonical LF file");
                assert!(
                    normalized.private_key.as_str() == canonical_file_bytes,
                    "line-ending changes may not change the encrypted key payload"
                );
            }
        }
    }

    #[test]
    fn signing_secret_adapter_rejects_foreign_public_and_malformed_formats() {
        let raw = encrypted_key_fixture(TAURI_RSIGN_HEADER);
        let wrapped = STANDARD.encode(raw.as_bytes());
        let public = format!(
            "untrusted comment: minisign public key: synthetic\n{}",
            STANDARD.encode([0u8; 42])
        );
        let tauri_signature = format!(
            "untrusted comment: signature from tauri secret key\n{}\ntrusted comment: test\n{}",
            STANDARD.encode([0u8; 74]),
            STANDARD.encode([0u8; 64])
        );
        for invalid in [
            "plain text".to_owned(),
            "RWQAAAA=".to_owned(),
            STANDARD.encode(public.as_bytes()),
            STANDARD.encode(tauri_signature.as_bytes()),
            STANDARD.encode(wrapped.as_bytes()), // nested Base64 is forbidden
            format!(
                "wrong header\n{}",
                STANDARD.encode([0u8; ENCRYPTED_KEY_BYTES])
            ),
            format!("{TAURI_RSIGN_HEADER}\nnot-base64"),
            format!("{TAURI_RSIGN_HEADER}\n{}", STANDARD.encode([0u8; 12])),
            format!(
                "{TAURI_RSIGN_HEADER}\n{}\nEXTRA",
                STANDARD.encode([0u8; ENCRYPTED_KEY_BYTES])
            ),
            format!(
                "{TAURI_RSIGN_HEADER}\n{}\n\n",
                STANDARD.encode([0u8; ENCRYPTED_KEY_BYTES])
            ),
            format!("\u{feff}{raw}"),
            format!("\n{raw}"),
            format!("{}{}\n", STANDARD.encode(raw.as_bytes()), "garbage"),
        ] {
            assert!(
                SigningSecrets::from_values(invalid, "passphrase".to_owned()).is_err(),
                "foreign or malformed key must fail closed"
            );
        }
    }

    #[test]
    fn signing_secret_adapter_enforces_secret_bounds_and_cleanliness() {
        let raw = encrypted_key_fixture(TAURI_RSIGN_HEADER);
        assert!(SigningSecrets::from_values(String::new(), "passphrase".to_owned()).is_err());
        assert!(SigningSecrets::from_values(raw.clone(), "  ".to_owned()).is_err());
        assert!(SigningSecrets::from_values(format!("{raw}\0"), "passphrase".to_owned()).is_err());
        assert!(SigningSecrets::from_values(raw.clone(), "pass\0phrase".to_owned()).is_err());
        assert!(
            SigningSecrets::from_values(
                format!("{raw}{}", "x".repeat(MAX_PRIVATE_KEY_BYTES)),
                "passphrase".to_owned(),
            )
            .is_err()
        );
        assert!(
            SigningSecrets::from_values(
                STANDARD.encode(raw.as_bytes()),
                "p".repeat(MAX_PASSPHRASE_BYTES + 1),
            )
            .is_err()
        );
    }

    #[test]
    fn signing_failure_cleanup_never_adopts_an_existing_signature() {
        let directory = std::env::temp_dir().join(format!(
            "sky-v4-signing-output-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir(&directory).expect("create isolated test directory");
        let signature = directory.join("candidate.exe.sig");
        let foreign_content = b"pre-existing detached signature";
        fs::write(&signature, foreign_content).expect("write pre-existing signature");

        assert!(RemoveOnFailure::for_new_output(signature.clone()).is_err());
        assert_eq!(
            fs::read(&signature).expect("foreign signature remains readable"),
            foreign_content
        );

        fs::remove_file(&signature).expect("remove test fixture");
        {
            let guard = RemoveOnFailure::for_new_output(signature.clone())
                .expect("empty signature path may be owned");
            fs::write(&signature, b"partially written signer output")
                .expect("simulate signer failure");
            drop(guard);
        }
        assert!(
            fs::symlink_metadata(&signature).is_err(),
            "failure cleanup must remove only the output it owns"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(directory.join("missing-target"), &signature).expect("create dangling symlink");
            assert!(
                RemoveOnFailure::for_new_output(signature.clone()).is_err(),
                "dangling symlinks are occupied paths, not new output"
            );
            assert!(
                fs::symlink_metadata(&signature).is_ok(),
                "rejected symlink must remain untouched"
            );
            fs::remove_file(&signature).expect("remove symlink fixture");
        }

        fs::remove_dir(&directory).expect("clean up isolated test directory");
    }

    #[test]
    fn signing_material_cleanup_requires_owned_directories() {
        let root = std::env::temp_dir().join(format!(
            "sky-v4-signing-cleanup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create runner temp fixture");
        let owned = root.join("sky-v4-signing-material-123456-2-999");
        fs::create_dir(&owned).expect("create owned directory");
        fs::write(
            owned.join("owner.json"),
            br#"{"repository":"pumni/Sky-Auto-Player","run_id":"123456","run_attempt":2}"#,
        )
        .expect("write owner marker");
        fs::write(owned.join("updater.key"), PRIVATE_KEY).expect("write key fixture");
        cleanup_run_material_at(&root, "123456", 2).expect("remove owned temp secret");
        assert!(!owned.exists());

        let foreign = root.join("sky-v4-signing-material-123456-2-1000");
        fs::create_dir(&foreign).expect("create foreign directory");
        fs::write(
            foreign.join("owner.json"),
            br#"{"repository":"pumni/Sky-Auto-Player","run_id":"123456","run_attempt":1}"#,
        )
        .expect("write foreign marker");
        fs::write(foreign.join("updater.key"), PRIVATE_KEY).expect("write fixture key");
        assert!(cleanup_run_material_at(&root, "123456", 2).is_err());
        assert!(foreign.exists());
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(test)]
#[path = "release_signing/hosted_proof.rs"]
mod hosted_proof;
