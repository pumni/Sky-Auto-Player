#![forbid(unsafe_code)]

use crate::{Result, ci_artifacts, hash, process, updater_trust};
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
        if !private_key.lines().next().is_some_and(|line| {
            line.to_ascii_lowercase()
                .contains("minisign encrypted secret key")
        }) {
            return Err(
                "protected updater key must be encrypted Minisign private-key material".into(),
            );
        }
        Ok(Self {
            private_key: Zeroizing::new(private_key),
            passphrase: Zeroizing::new(passphrase),
        })
    }
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
    let mut cleanup = RemoveOnFailure {
        path: signature.clone(),
        keep: false,
    };
    if signature.exists() {
        return Err("refusing to overwrite a pre-existing updater signature".into());
    }
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
    let remove_env = vec![
        "TAURI_SIGNING_PRIVATE_KEY".to_owned(),
        PRIVATE_KEY_SECRET.to_owned(),
        PASSPHRASE_SECRET.to_owned(),
    ];
    let output = process::capture_owned_timeout_filtered(
        std::ffi::OsStr::new("bun"),
        &args,
        &root.join("desktop"),
        &env_values,
        &remove_env,
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
    let output = process::capture_owned_timeout_filtered(
        std::ffi::OsStr::new("bun"),
        &["run".to_owned(), "tauri".to_owned(), "--version".to_owned()],
        &root.join("desktop"),
        &[],
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

    const PRIVATE_KEY: &str = "untrusted comment: minisign encrypted secret key\nRWQAAAA=";

    #[test]
    fn signing_secret_adapter_accepts_bounded_encrypted_key_pair() {
        let secrets = SigningSecrets::from_values(PRIVATE_KEY.to_owned(), "passphrase".to_owned())
            .expect("bounded encrypted key pair");
        assert_eq!(secrets.private_key.len(), PRIVATE_KEY.len());
        assert_eq!(secrets.passphrase.as_str(), "passphrase");
    }

    #[test]
    fn signing_secret_adapter_rejects_unencrypted_empty_and_oversized_values() {
        assert!(SigningSecrets::from_values(String::new(), "passphrase".to_owned()).is_err());
        assert!(
            SigningSecrets::from_values("plain text".to_owned(), "passphrase".to_owned()).is_err()
        );
        assert!(SigningSecrets::from_values(PRIVATE_KEY.to_owned(), "  ".to_owned()).is_err());
        assert!(
            SigningSecrets::from_values(
                format!("{}{}", PRIVATE_KEY, "x".repeat(MAX_PRIVATE_KEY_BYTES)),
                "passphrase".to_owned(),
            )
            .is_err()
        );
        assert!(
            SigningSecrets::from_values(
                PRIVATE_KEY.to_owned(),
                "p".repeat(MAX_PASSPHRASE_BYTES + 1),
            )
            .is_err()
        );
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
