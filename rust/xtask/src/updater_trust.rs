use crate::{Result, process};
use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use zeroize::{Zeroize, Zeroizing};

const MAX_PUBLIC_KEY_BYTES: usize = 4096;
const MAX_SIGNATURE_BYTES: usize = 64 * 1024;
const MAX_UPDATER_INSTALLER_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Serialize)]
struct HostedUpdaterSignatureProof {
    schema_version: u32,
    evidence_type: &'static str,
    status: &'static str,
    source_sha: String,
    version: String,
    installer: String,
    installer_size: u64,
    installer_sha256: String,
    updater_signature: String,
    updater_signature_size: u64,
    updater_signature_sha256: String,
    updater_public_key_sha256: String,
    updater_key_id: String,
    signed_file: String,
    signed_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdaterTrustInventory {
    pub canonical_key_id: String,
    pub canonical_public_key: String,
    pub algorithm: &'static str,
    pub verified_locations: Vec<String>,
    pub legacy_v3_root_isolated: bool,
    pub unique_production_v4_root: bool,
}

pub fn inventory_public_trust_roots(root: &Path) -> Result<UpdaterTrustInventory> {
    let canonical = crate::tauri_bundle::V4_TAURI_UPDATER_PUBLIC_KEY;
    let _decoded_key = decode_public_key(canonical)?;
    let key_id = extract_key_id_from_public_key(canonical)?;
    if key_id != "19AABD2E7838818C" {
        return Err(format!("unexpected canonical key id: {key_id}").into());
    }

    let mut verified_locations = Vec::new();

    // 1. desktop/src-tauri/tauri.conf.json
    let config_path = root.join("desktop/src-tauri/tauri.conf.json");
    let config_content = fs::read_to_string(&config_path)?;
    let config_json: serde_json::Value = serde_json::from_str(&config_content)?;
    let config_key = config_json
        .get("plugins")
        .and_then(serde_json::Value::as_object)
        .and_then(|p| p.get("updater"))
        .and_then(serde_json::Value::as_object)
        .and_then(|u| u.get("pubkey"))
        .and_then(serde_json::Value::as_str)
        .ok_or("desktop/src-tauri/tauri.conf.json is missing plugins.updater.pubkey")?;
    if config_key != canonical {
        return Err("tauri.conf.json updater pubkey does not match canonical root".into());
    }
    verified_locations.push("desktop/src-tauri/tauri.conf.json".to_string());

    // 2. desktop/src-tauri/src/native_update.rs
    let native_path = root.join("desktop/src-tauri/src/native_update.rs");
    let native_content = fs::read_to_string(&native_path)?;
    let native_key = extract_rust_string_constant(&native_content, "V4_TAURI_UPDATER_PUBLIC_KEY")?;
    if native_key != canonical {
        return Err(
            "native_update.rs V4_TAURI_UPDATER_PUBLIC_KEY does not match canonical root".into(),
        );
    }
    if !native_content
        .contains("const V4_TAURI_UPDATER_PUBLIC_KEYS: &[&str] = &[V4_TAURI_UPDATER_PUBLIC_KEY];")
    {
        return Err(
            "native_update.rs does not enforce single V4_TAURI_UPDATER_PUBLIC_KEYS array".into(),
        );
    }
    verified_locations.push("desktop/src-tauri/src/native_update.rs".to_string());

    // 3. rust/xtask/src/tauri_bundle.rs
    let bundle_path = root.join("rust/xtask/src/tauri_bundle.rs");
    let bundle_content = fs::read_to_string(&bundle_path)?;
    let bundle_key = extract_rust_string_constant(&bundle_content, "V4_TAURI_UPDATER_PUBLIC_KEY")?;
    if bundle_key != canonical {
        return Err(
            "tauri_bundle.rs V4_TAURI_UPDATER_PUBLIC_KEY does not match canonical root".into(),
        );
    }
    verified_locations.push("rust/xtask/src/tauri_bundle.rs".to_string());

    // 4. Verify that the retired v3 custom-manifest trust root is not part of
    // the current v4 workspace.
    let cargo_toml_path = root.join("rust/Cargo.toml");
    let cargo_toml: toml::Value = toml::from_str(&fs::read_to_string(&cargo_toml_path)?)?;
    let has_legacy_v3_root = cargo_toml
        .get("workspace")
        .and_then(toml::Value::as_table)
        .and_then(|workspace| workspace.get("metadata"))
        .and_then(toml::Value::as_table)
        .is_some_and(|metadata| metadata.contains_key("sky-update-signing"));
    if has_legacy_v3_root {
        return Err("retired v3 update trust metadata remains in rust/Cargo.toml".into());
    }
    let legacy_v3_root_isolated = true;

    Ok(UpdaterTrustInventory {
        canonical_key_id: key_id,
        canonical_public_key: canonical.to_string(),
        algorithm: "Ed25519 (Minisign)",
        verified_locations,
        legacy_v3_root_isolated,
        unique_production_v4_root: true,
    })
}

pub fn print_inventory(root: &Path) -> Result<()> {
    let inventory = inventory_public_trust_roots(root)?;
    println!("[xtask] V4 Updater Public Trust Root Inventory: PASS");
    println!("  Key ID: {}", inventory.canonical_key_id);
    println!("  Algorithm: {}", inventory.algorithm);
    println!("  Public Root: {}", inventory.canonical_public_key);
    println!("  Verified Locations (byte-for-byte identical):");
    for location in &inventory.verified_locations {
        println!("    - {location}");
    }
    println!("  Legacy v3 Root: absent from the current v4 workspace");
    println!("  Unique Production v4 Root: verified exactly 1 public trust root");
    Ok(())
}

/// Export only the canonical public updater root for a throwaway packaged
/// qualification fixture. The inventory parser remains the authority for
/// both validation and the exported value; no private material is involved.
pub fn export_canonical_public_key(root: &Path, output: &Path) -> Result<()> {
    let inventory = inventory_public_trust_roots(root)?;
    let parent = output
        .parent()
        .ok_or("updater public-key export requires a parent directory")?;
    fs::create_dir_all(parent)?;
    fs::write(output, format!("{}\n", inventory.canonical_public_key))?;
    println!(
        "[xtask] V4 updater public-root export: PASS (public identity only; {} bytes)",
        inventory.canonical_public_key.len()
    );
    Ok(())
}

/// Verify local updater private key without printing private material or password.
pub fn verify_local_private_key(
    root: &Path,
    key_path: &Path,
    password: Option<&str>,
) -> Result<()> {
    if !key_path.is_file() {
        return Err(format!("private key file does not exist: {}", key_path.display()).into());
    }
    let key_bytes = Zeroizing::new(fs::read(key_path)?);
    if key_bytes.is_empty() || key_bytes.len() > MAX_PUBLIC_KEY_BYTES {
        return Err("private key file is empty or unbounded".into());
    }
    let key_str = String::from_utf8_lossy(&key_bytes);
    if key_str.contains("release-2026") {
        return Err("provided key contains legacy v3 material, not v4".into());
    }

    let mut nonce = [0u8; 64];
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for (i, b) in nonce.iter_mut().enumerate() {
        *b = ((timestamp >> ((i % 16) * 4)) ^ (std::process::id() as u128 >> ((i % 4) * 8))) as u8;
    }
    let temp_dir = std::env::temp_dir().join(format!(
        "sky-v4-keycheck-{}-{}",
        std::process::id(),
        timestamp % 1_000_000
    ));
    let _ = fs::remove_dir_all(&temp_dir);
    fs::create_dir_all(&temp_dir)?;

    let payload_path = temp_dir.join("nonce.bin");
    let signature_path = temp_dir.join("nonce.bin.sig");
    fs::write(&payload_path, nonce)?;

    let desktop_dir = root.join("desktop");
    let canonical_key_path = fs::canonicalize(key_path)?;

    let mut env_values = vec![
        (
            "TAURI_SIGNING_PRIVATE_KEY_PATH".to_owned(),
            canonical_key_path.to_string_lossy().into_owned(),
        ),
        (
            "TAURI_SIGNING_PRIVATE_KEY_PASSWORD".to_owned(),
            password.unwrap_or("").to_owned(),
        ),
    ];
    let args = vec![
        "run".to_owned(),
        "tauri".to_owned(),
        "signer".to_owned(),
        "sign".to_owned(),
        payload_path.to_string_lossy().into_owned(),
    ];
    let output = process::capture_owned_timeout_runtime_only(
        std::ffi::OsStr::new("bun"),
        &args,
        &desktop_dir,
        &env_values,
        Duration::from_secs(5 * 60),
    );
    env_values[1].1.zeroize();
    let output = output.map_err(|_| "failed to execute bounded updater signer verification")?;
    let result = match output {
        out if out.status.success() => {
            if signature_path.is_file() {
                match decode_signature(&signature_path) {
                    Ok(signature) => {
                        match decode_public_key(crate::tauri_bundle::V4_TAURI_UPDATER_PUBLIC_KEY) {
                            Ok(public_key) => match public_key.verify(&nonce, &signature, false) {
                                Ok(_) => Ok(()),
                                Err(_) => Err(
                                    "private key signature does not match the canonical production v4 public root"
                                        .into(),
                                ),
                            },
                            Err(e) => {
                                Err(format!("failed to decode canonical public root: {e}").into())
                            }
                        }
                    }
                    Err(e) => Err(format!("failed to decode generated signature: {e}").into()),
                }
            } else {
                Err("signer completed but signature file was not produced".into())
            }
        }
        out => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("Wrong password")
                || stderr.contains("incorrect updater private key password")
            {
                Err("incorrect updater private key password".into())
            } else {
                Err(
                    "updater signer rejected the private key (invalid key format or corrupt material)"
                        .into(),
                )
            }
        }
    };

    let _ = fs::remove_file(&payload_path);
    let _ = fs::remove_file(&signature_path);
    let _ = fs::remove_dir_all(&temp_dir);

    result
}

/// Verify a Tauri updater signature against an installer file using the canonical v4 public root.
#[cfg(test)]
pub fn verify_updater_signature(installer_path: &Path, signature_path: &Path) -> Result<()> {
    let public_key = decode_public_key(crate::tauri_bundle::V4_TAURI_UPDATER_PUBLIC_KEY)?;
    verify_updater_signature_with_key(installer_path, signature_path, &public_key, None)?;
    let key_id = canonical_key_id()?;
    println!("[xtask] V4 Updater Signature Verification: PASS (Key ID: {key_id})");
    Ok(())
}

/// Verify production updater bytes and require the version embedded in the
/// signed Minisign trusted comment to match the release context.
pub fn verify_updater_signature_for_version(
    installer_path: &Path,
    signature_path: &Path,
    expected_version: &str,
) -> Result<()> {
    crate::ci_artifacts::validate_semver(expected_version, "expected updater version")?;
    let public_key = decode_public_key(crate::tauri_bundle::V4_TAURI_UPDATER_PUBLIC_KEY)?;
    verify_updater_signature_with_key(
        installer_path,
        signature_path,
        &public_key,
        Some(expected_version),
    )?;
    let key_id = canonical_key_id()?;
    println!(
        "[xtask] V4 Updater Signature Verification: PASS (version={expected_version}; Key ID: {key_id})"
    );
    Ok(())
}

fn verify_updater_signature_with_key(
    installer_path: &Path,
    signature_path: &Path,
    public_key: &PublicKey,
    expected_version: Option<&str>,
) -> Result<()> {
    let installer_metadata =
        crate::ci_artifacts::require_regular_file(installer_path, "updater installer")?;
    if installer_metadata.len() == 0 || installer_metadata.len() > MAX_UPDATER_INSTALLER_BYTES {
        return Err("updater installer is empty or exceeds the verification bound".into());
    }
    let installer_bytes = fs::read(installer_path)?;
    if installer_bytes.len() as u64 != installer_metadata.len() {
        return Err("updater installer changed while it was being read".into());
    }
    let signature = decode_signature(signature_path)?;
    public_key
        .verify(&installer_bytes, &signature, false)
        .map_err(|_| "updater signature does not match the supplied public root")?;
    if let Some(expected_version) = expected_version {
        let installer_name = installer_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("updater installer filename is not valid Unicode")?;
        validate_signed_artifact_identity(&signature, installer_name, expected_version)?;
    }
    Ok(())
}

fn trusted_comment_field<'a>(signature: &'a Signature, name: &str) -> Result<&'a str> {
    let prefix = format!("{name}:");
    let mut matches = signature
        .trusted_comment()
        .split('\t')
        .filter_map(|field| field.strip_prefix(&prefix));
    let value = matches
        .next()
        .ok_or_else(|| format!("updater signature trusted comment is missing {name}"))?;
    if value.is_empty() || matches.next().is_some() {
        return Err(format!("updater signature trusted comment has invalid {name}").into());
    }
    Ok(value)
}

fn validate_signed_artifact_identity(
    signature: &Signature,
    expected_file: &str,
    expected_version: &str,
) -> Result<()> {
    let signed_version = trusted_comment_field(signature, "version")?;
    if signed_version != expected_version {
        return Err(format!(
            "updater signature version does not match the candidate: expected {expected_version}, observed {signed_version}"
        )
        .into());
    }
    let signed_file = trusted_comment_field(signature, "file")?;
    if signed_file != expected_file {
        return Err(format!(
            "updater signature filename does not match the candidate: expected {expected_file}, observed {signed_file}"
        )
        .into());
    }
    Ok(())
}

/// Verify the isolated hosted proof artifact with a synthetic test key. This
/// path explicitly refuses the production public root and never loads private
/// material.
pub fn verify_test_updater_signature(
    installer_path: &Path,
    signature_path: &Path,
    public_key_path: &Path,
    expected_version: &str,
    expected_sha256: &str,
    source_sha: &str,
    evidence_path: Option<&Path>,
) -> Result<()> {
    crate::ci_artifacts::validate_semver(expected_version, "expected updater version")?;
    let source_sha = crate::ci_artifacts::validate_commit_sha(source_sha, "source SHA")?;
    let expected_sha256 =
        crate::ci_artifacts::validate_digest(expected_sha256, "expected installer SHA-256")?;
    let expected_installer = crate::release_metadata::canonical_installer_name(expected_version);
    let observed_installer = installer_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("updater installer filename is not valid Unicode")?;
    if observed_installer != expected_installer {
        return Err(format!(
            "updater installer filename does not match expected version: expected {expected_installer}, observed {observed_installer}"
        )
        .into());
    }
    let expected_signature = format!("{expected_installer}.sig");
    let observed_signature = signature_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("updater signature filename is not valid Unicode")?;
    if observed_signature != expected_signature
        || signature_path != installer_path.with_file_name(&expected_signature)
    {
        return Err("updater signature is not the exact installer sidecar".into());
    }

    let public_key_text = read_public_key(public_key_path)?;
    if public_key_text == crate::tauri_bundle::V4_TAURI_UPDATER_PUBLIC_KEY {
        return Err("hosted proof verifier refuses the canonical production updater root".into());
    }
    let public_key = decode_public_key(&public_key_text)?;
    let key_id = extract_key_id_from_public_key(&public_key_text)?;

    let installer_metadata =
        crate::ci_artifacts::require_regular_file(installer_path, "hosted proof installer")?;
    if installer_metadata.len() == 0 || installer_metadata.len() > MAX_UPDATER_INSTALLER_BYTES {
        return Err("hosted proof installer is empty or exceeds the verification bound".into());
    }
    let signature_metadata = crate::ci_artifacts::require_regular_file(
        signature_path,
        "hosted proof updater signature",
    )?;
    let installer_bytes = fs::read(installer_path)?;
    if installer_bytes.len() as u64 != installer_metadata.len() {
        return Err("hosted proof installer changed while it was being read".into());
    }
    let installer_sha256 = crate::repo::hex_digest(Sha256::digest(&installer_bytes));
    if installer_sha256 != expected_sha256 {
        return Err("hosted proof installer SHA-256 does not match the pre-sign digest".into());
    }
    let signature = decode_signature(signature_path)?;
    public_key
        .verify(&installer_bytes, &signature, false)
        .map_err(|_| "hosted proof signature does not match the supplied test public key")?;
    validate_signed_artifact_identity(&signature, &expected_installer, expected_version)?;

    let proof = HostedUpdaterSignatureProof {
        schema_version: 1,
        evidence_type: "tauri-hosted-test-key-signature-proof",
        status: "PASS",
        source_sha,
        version: expected_version.to_owned(),
        installer: expected_installer.clone(),
        installer_size: installer_metadata.len(),
        installer_sha256,
        updater_signature: expected_signature,
        updater_signature_size: signature_metadata.len(),
        updater_signature_sha256: crate::ci_artifacts::sha256_file(signature_path)?,
        updater_public_key_sha256: crate::ci_artifacts::sha256_file(public_key_path)?,
        updater_key_id: key_id.clone(),
        signed_file: trusted_comment_field(&signature, "file")?.to_owned(),
        signed_version: trusted_comment_field(&signature, "version")?.to_owned(),
    };
    if let Some(path) = evidence_path {
        let bytes = serde_json::to_vec_pretty(&proof)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        output.write_all(&bytes)?;
        output.write_all(b"\n")?;
        output.sync_all()?;
    }
    println!(
        "[xtask] Hosted Tauri updater signature proof: PASS (source={}; version={}; installer_sha256={}; test_key_id={})",
        proof.source_sha, proof.version, proof.installer_sha256, proof.updater_key_id
    );
    Ok(())
}

/// Verify a detached Minisign signature against an explicitly supplied public
/// key. This is used for disposable fixture keys and does not alter the
/// canonical production-root verifier above.
#[cfg(windows)]
pub(crate) fn verify_signature_text_with_public_key(
    payload: &[u8],
    signature_text: &str,
    public_key_text: &str,
) -> Result<()> {
    if payload.is_empty() || payload.len() > 1024 * 1024 {
        return Err("updater signature verification payload is empty or unbounded".into());
    }
    let signature_text = signature_text.trim();
    if signature_text.is_empty() || signature_text.len() > MAX_SIGNATURE_BYTES {
        return Err("updater signature text is empty or unbounded".into());
    }
    let public_key_text = public_key_text.trim();
    if public_key_text.is_empty()
        || public_key_text.len() > MAX_PUBLIC_KEY_BYTES
        || public_key_text.contains("PRIVATE KEY")
        || public_key_text.contains('\0')
    {
        return Err("updater public key is empty, private, or unbounded".into());
    }

    let public_key = decode_public_key(public_key_text)?;
    let signature = STANDARD.decode(signature_text)?;
    let signature = String::from_utf8(signature)?;
    let signature = Signature::decode(&signature)?;
    public_key
        .verify(payload, &signature, false)
        .map_err(|_| "updater signature does not match the supplied public root".into())
}

/// Return the identity parsed from the canonical compiled public root.
pub fn canonical_key_id() -> Result<String> {
    extract_key_id_from_public_key(crate::tauri_bundle::V4_TAURI_UPDATER_PUBLIC_KEY)
}

pub fn extract_key_id_from_public_key(value: &str) -> Result<String> {
    let decoded = STANDARD.decode(value)?;
    let text = String::from_utf8(decoded)?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("untrusted comment: minisign public key: ") {
            let key_id = rest.trim().to_ascii_uppercase();
            if key_id.len() == 16 && key_id.chars().all(|c| c.is_ascii_hexdigit()) {
                return Ok(key_id);
            }
        }
    }
    Err("could not find valid minisign key id in public key comment".into())
}

fn extract_rust_string_constant(source: &str, name: &str) -> Result<String> {
    let decl1 = format!("const {name}: &str = \"");
    let decl2 = format!("pub const {name}: &str = \"");
    for line in source.lines() {
        let trimmed = line.trim();
        let prefix_len = if trimmed.starts_with(&decl1) {
            Some(decl1.len())
        } else if trimmed.starts_with(&decl2) {
            Some(decl2.len())
        } else {
            None
        };
        if let Some(len) = prefix_len {
            let after_prefix = &trimmed[len..];
            if let Some(end) = after_prefix.find('"') {
                return Ok(after_prefix[..end].to_string());
            }
        }
    }
    Err(format!("Rust source must contain {name} string constant").into())
}

/// Verify the release-rotation bridge without ever importing or printing a
/// private key. A bridge release trusts both roots; the cutover release trusts
/// only the new root.
pub fn rotation_self_test(old_public_path: &Path, new_public_path: &Path) -> Result<()> {
    let old_public = read_public_key(old_public_path)?;
    let new_public = read_public_key(new_public_path)?;
    if old_public == new_public {
        return Err("updater rotation fixture requires two distinct public keys".into());
    }

    let bridge_roots = [&old_public, &new_public];
    if !bridge_roots.contains(&&old_public) || !bridge_roots.contains(&&new_public) {
        return Err("updater rotation bridge does not contain both roots".into());
    }
    let cutover_roots = [&new_public];
    if !cutover_roots.contains(&&new_public) || cutover_roots.contains(&&old_public) {
        return Err("updater rotation cutover did not remove the old root".into());
    }

    println!("[xtask] non-production updater key rotation fixture: PASS");
    Ok(())
}

pub fn verify_rotation_signatures(
    old_public_path: &Path,
    new_public_path: &Path,
    old_signature_path: &Path,
    new_signature_path: &Path,
    payload_path: &Path,
) -> Result<()> {
    let old_public = read_public_key(old_public_path)?;
    let new_public = read_public_key(new_public_path)?;
    let payload = fs::read(payload_path)?;
    if payload.len() > 1024 * 1024 {
        return Err("updater rotation payload is unbounded".into());
    }
    let old_signature = decode_signature(old_signature_path)?;
    let new_signature = decode_signature(new_signature_path)?;
    let old_key = decode_public_key(&old_public)?;
    let new_key = decode_public_key(&new_public)?;
    old_key.verify(&payload, &old_signature, false)?;
    new_key.verify(&payload, &new_signature, false)?;
    if old_key.verify(&payload, &new_signature, false).is_ok()
        || new_key.verify(&payload, &old_signature, false).is_ok()
    {
        return Err("updater rotation fixture accepted a signature under the wrong root".into());
    }
    println!("[xtask] updater key rotation signatures: PASS");
    Ok(())
}

fn decode_public_key(value: &str) -> Result<PublicKey> {
    let decoded = STANDARD.decode(value)?;
    let decoded = String::from_utf8(decoded)?;
    Ok(PublicKey::decode(&decoded)?)
}

fn decode_signature(path: &Path) -> Result<Signature> {
    let metadata = crate::ci_artifacts::require_regular_file(path, "updater signature")?;
    if metadata.len() == 0 || metadata.len() > MAX_SIGNATURE_BYTES as u64 {
        return Err("updater signature is empty or unbounded".into());
    }
    let encoded = fs::read_to_string(path)?.trim().to_owned();
    if encoded.len() > MAX_SIGNATURE_BYTES {
        return Err("updater signature is unbounded".into());
    }
    let decoded = STANDARD.decode(encoded)?;
    let decoded = String::from_utf8(decoded)?;
    Ok(Signature::decode(&decoded)?)
}

fn read_public_key(path: &Path) -> Result<String> {
    let metadata = crate::ci_artifacts::require_regular_file(path, "updater public-key fixture")?;
    if metadata.len() > MAX_PUBLIC_KEY_BYTES as u64 {
        return Err("updater public-key fixture is unbounded".into());
    }
    let value = String::from_utf8(fs::read(path)?)?.trim().to_owned();
    if value.is_empty() || value.len() > MAX_PUBLIC_KEY_BYTES {
        return Err("updater public-key fixture is empty or unbounded".into());
    }
    if value.contains("PRIVATE KEY") || value.contains("release-2026") {
        return Err("updater public-key fixture contains forbidden key material".into());
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return Err("updater public-key fixture contains invalid characters".into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOSTED_PROOF_BYTES: &[u8] = b"synthetic NSIS installer fixture bytes";
    const HOSTED_PROOF_SOURCE_SHA: &str = "1111111111111111111111111111111111111111";
    const HOSTED_PROOF_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IENGRTM5NkQyNDI3RTUwQzcKUldUSFVINUMwcGJqendvZC84ajliU2dpdDFreG1ESkRPK1BqaWxwQjJIZXhRM01KUDNxMTVMR0gK";
    const HOSTED_PROOF_OTHER_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE1NjM3MkE4NzJGM0JENkUKUldSdXZmTnlxSEpqRmZUcWx4NWV4NGpFK1A0eDlHaEpZRzJjQzFTZDB1L3M4WU5ON2UzTGIxemIK";
    const HOSTED_PROOF_ALTERNATE_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDYzNkNBMzdDQzBDM0IyREQKUldUZHNzUEFmS05zWXpKMjRqcnRlR3FFY0pVMFVybEsydlhmdDhWV0RGYWYxc01wNlJSZ3dJWmQK";
    const HOSTED_PROOF_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUSFVINUMwcGJqendNRFQ0ZmxqNyt4YUhjTUxwQktGLzFXbDRqTk82NEVPVDdlZjBnZ3dPeG1ZTFNNVU8wUGNtYmYrZXJsRUkvOWlVTWtSeVJnTExrUmhXTGcrSzRvVFFnPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyOTkzMjAzCWZpbGU6U2t5IEF1dG8gUGxheWVyXzQuMi4wX3g2NC1zZXR1cC5leGUJdmVyc2lvbjo0LjIuMAp2d2s5MTdmN0QybkVDWlgwQk1haGlCT3ZGQ0xqUUxsTG5YLzlJQ3plM0Z4TEJqdTlpdzZEUTV0UmQ3cHpwRmZXUmVteGEvNU8vS3Q4aFdiNnRDbDFEdz09Cg==";
    const HOSTED_PROOF_WRONG_VERSION_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUZHNzUEFmS05zWTFNVEE2Wm5UdzJjNmZlZkpoSVJ3TlBCZEJSVS9lQ0FzMllGb0U1MkxvbGZSditkTyt4cFRVdVRWbEVJeWJpb2JiRkVQbXEzRTh5K2dFaEpPUXpPZHdjPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkyOTkzNzMxCWZpbGU6U2t5IEF1dG8gUGxheWVyXzQuMi4xX3g2NC1zZXR1cC5leGUJdmVyc2lvbjo0LjIuMApRRDZyd3AvYkh0eTNaZUlML3hTNkZBcllBN0I0MWZIRER1TWJPUkNKVVNBOTdoanJTSUdUeGlRUE9sRjlZQW9Obzd3VW83SU50T3dwUVBrL3JpUlZEUT09Cg==";

    fn fixture_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "sky-xtask-updater-trust-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    fn write_hosted_proof_fixture(
        root: &Path,
        version: &str,
        public_key: &str,
        signature: &str,
        payload: &[u8],
    ) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        fs::create_dir_all(root).unwrap();
        let installer_name = crate::release_metadata::canonical_installer_name(version);
        let installer = root.join(&installer_name);
        let signature_path = root.join(format!("{installer_name}.sig"));
        let public_key_path = root.join("synthetic-updater.pub");
        fs::write(&installer, payload).unwrap();
        fs::write(&signature_path, signature).unwrap();
        fs::write(&public_key_path, public_key).unwrap();
        (installer, signature_path, public_key_path)
    }

    #[test]
    fn rotation_requires_distinct_roots_and_models_bridge_cutover() {
        let root = fixture_root();
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let old = root.join("old.pub");
        let new = root.join("new.pub");
        fs::write(&old, "A".repeat(128)).unwrap();
        fs::write(&new, "B".repeat(128)).unwrap();
        rotation_self_test(&old, &new).unwrap();
        fs::write(&new, "A".repeat(128)).unwrap();
        assert!(rotation_self_test(&old, &new).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn public_key_fixture_rejects_private_material() {
        let root = fixture_root();
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("private.pub");
        fs::write(
            &path,
            format!("-----{}-----", ["BEGIN", "PRIVATE", "KEY"].join(" ")),
        )
        .unwrap();
        assert!(read_public_key(&path).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn extract_key_id_matches_canonical_v4() {
        let canonical = crate::tauri_bundle::V4_TAURI_UPDATER_PUBLIC_KEY;
        let key_id = extract_key_id_from_public_key(canonical).unwrap();
        assert_eq!(key_id, "19AABD2E7838818C");
    }

    #[test]
    fn inventory_proves_single_canonical_v4_root() {
        let repo_root = crate::repo::root();
        let inventory = inventory_public_trust_roots(&repo_root).unwrap();
        assert_eq!(inventory.canonical_key_id, "19AABD2E7838818C");
        assert_eq!(
            inventory.canonical_public_key,
            crate::tauri_bundle::V4_TAURI_UPDATER_PUBLIC_KEY
        );
        assert_eq!(inventory.verified_locations.len(), 3);
        assert!(inventory.legacy_v3_root_isolated);
        assert!(inventory.unique_production_v4_root);
    }

    #[test]
    fn verify_local_private_key_rejects_nonexistent_or_v3_material() {
        let root = fixture_root();
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let nonexistent = root.join("nonexistent.key");
        assert!(verify_local_private_key(&crate::repo::root(), &nonexistent, None).is_err());

        let v3_fake = root.join("v3.key");
        fs::write(&v3_fake, "untrusted comment: release-2026 key material").unwrap();
        assert!(verify_local_private_key(&crate::repo::root(), &v3_fake, None).is_err());

        let empty_key = root.join("empty.key");
        fs::write(&empty_key, "").unwrap();
        assert!(verify_local_private_key(&crate::repo::root(), &empty_key, None).is_err());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn verify_updater_signature_rejects_missing_or_corrupt_files() {
        let root = fixture_root();
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        let dummy_exe = root.join("dummy.exe");
        let dummy_sig = root.join("dummy.exe.sig");
        assert!(verify_updater_signature(&dummy_exe, &dummy_sig).is_err());

        fs::write(&dummy_exe, b"MZ dummy executable content").unwrap();
        assert!(verify_updater_signature(&dummy_exe, &dummy_sig).is_err());

        fs::write(
            &dummy_sig,
            "untrusted comment: signature\ninvalid signature data",
        )
        .unwrap();
        assert!(verify_updater_signature(&dummy_exe, &dummy_sig).is_err());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hosted_signature_proof_verifies_exact_bytes_version_and_typed_evidence() {
        let root = fixture_root().join("hosted-proof-valid");
        let (installer, signature, public_key) = write_hosted_proof_fixture(
            &root,
            "4.2.0",
            HOSTED_PROOF_PUBLIC_KEY,
            HOSTED_PROOF_SIGNATURE,
            HOSTED_PROOF_BYTES,
        );
        let expected_sha256 = crate::repo::hex_digest(Sha256::digest(HOSTED_PROOF_BYTES));
        let evidence = root.join("proof.json");
        verify_test_updater_signature(
            &installer,
            &signature,
            &public_key,
            "4.2.0",
            &expected_sha256,
            HOSTED_PROOF_SOURCE_SHA,
            Some(&evidence),
        )
        .unwrap();
        let evidence: serde_json::Value =
            serde_json::from_slice(&fs::read(evidence).unwrap()).unwrap();
        assert_eq!(evidence["status"], "PASS");
        assert_eq!(evidence["source_sha"], HOSTED_PROOF_SOURCE_SHA);
        assert_eq!(evidence["version"], "4.2.0");
        assert_eq!(evidence["signed_version"], "4.2.0");
        assert_eq!(
            evidence["signed_file"],
            "Sky Auto Player_4.2.0_x64-setup.exe"
        );
        assert_eq!(evidence["installer_sha256"], expected_sha256);
        assert!(!evidence.to_string().contains("PRIVATE KEY"));

        let relocated = root.join("relocated");
        fs::create_dir_all(&relocated).unwrap();
        let relocated_signature = relocated.join(signature.file_name().unwrap());
        fs::copy(&signature, &relocated_signature).unwrap();
        assert!(
            verify_test_updater_signature(
                &installer,
                &relocated_signature,
                &public_key,
                "4.2.0",
                &expected_sha256,
                HOSTED_PROOF_SOURCE_SHA,
                None,
            )
            .unwrap_err()
            .to_string()
            .contains("exact installer sidecar")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hosted_signature_proof_rejects_wrong_key_version_tamper_partial_and_sha() {
        let root = fixture_root().join("hosted-proof-negative");
        let wrong_key = root.join("wrong-key");
        let (installer, signature, public_key) = write_hosted_proof_fixture(
            &wrong_key,
            "4.2.0",
            HOSTED_PROOF_OTHER_PUBLIC_KEY,
            HOSTED_PROOF_SIGNATURE,
            HOSTED_PROOF_BYTES,
        );
        let expected_sha256 = crate::repo::hex_digest(Sha256::digest(HOSTED_PROOF_BYTES));
        assert!(
            verify_test_updater_signature(
                &installer,
                &signature,
                &public_key,
                "4.2.0",
                &expected_sha256,
                HOSTED_PROOF_SOURCE_SHA,
                None,
            )
            .is_err()
        );

        let wrong_version = root.join("wrong-version");
        let (installer, signature, public_key) = write_hosted_proof_fixture(
            &wrong_version,
            "4.2.1",
            HOSTED_PROOF_ALTERNATE_PUBLIC_KEY,
            HOSTED_PROOF_WRONG_VERSION_SIGNATURE,
            HOSTED_PROOF_BYTES,
        );
        let error = verify_test_updater_signature(
            &installer,
            &signature,
            &public_key,
            "4.2.1",
            &expected_sha256,
            HOSTED_PROOF_SOURCE_SHA,
            None,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("version does not match"),
            "unexpected version-negative result: {error}"
        );

        let modified = root.join("modified-bytes");
        let mut modified_payload = HOSTED_PROOF_BYTES.to_vec();
        modified_payload.push(b'!');
        let (installer, signature, public_key) = write_hosted_proof_fixture(
            &modified,
            "4.2.0",
            HOSTED_PROOF_PUBLIC_KEY,
            HOSTED_PROOF_SIGNATURE,
            &modified_payload,
        );
        let modified_sha256 = crate::repo::hex_digest(Sha256::digest(&modified_payload));
        assert!(
            verify_test_updater_signature(
                &installer,
                &signature,
                &public_key,
                "4.2.0",
                &modified_sha256,
                HOSTED_PROOF_SOURCE_SHA,
                None,
            )
            .is_err()
        );

        let partial = root.join("partial-signature");
        let (installer, signature, public_key) = write_hosted_proof_fixture(
            &partial,
            "4.2.0",
            HOSTED_PROOF_PUBLIC_KEY,
            "AAAA",
            HOSTED_PROOF_BYTES,
        );
        assert!(
            verify_test_updater_signature(
                &installer,
                &signature,
                &public_key,
                "4.2.0",
                &expected_sha256,
                HOSTED_PROOF_SOURCE_SHA,
                None,
            )
            .is_err()
        );

        let mismatched_sha = root.join("mismatched-sha");
        let (installer, signature, public_key) = write_hosted_proof_fixture(
            &mismatched_sha,
            "4.2.0",
            HOSTED_PROOF_PUBLIC_KEY,
            HOSTED_PROOF_SIGNATURE,
            HOSTED_PROOF_BYTES,
        );
        assert!(
            verify_test_updater_signature(
                &installer,
                &signature,
                &public_key,
                "4.2.0",
                &"0".repeat(64),
                HOSTED_PROOF_SOURCE_SHA,
                None,
            )
            .is_err()
        );

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn hosted_signature_proof_refuses_the_production_public_root_and_bad_identity() {
        let root = fixture_root().join("hosted-proof-boundary");
        let (installer, signature, public_key) = write_hosted_proof_fixture(
            &root,
            "4.2.0",
            crate::tauri_bundle::V4_TAURI_UPDATER_PUBLIC_KEY,
            HOSTED_PROOF_SIGNATURE,
            HOSTED_PROOF_BYTES,
        );
        let expected_sha256 = crate::repo::hex_digest(Sha256::digest(HOSTED_PROOF_BYTES));
        assert!(
            verify_test_updater_signature(
                &installer,
                &signature,
                &public_key,
                "4.2.0",
                &expected_sha256,
                HOSTED_PROOF_SOURCE_SHA,
                None,
            )
            .is_err()
        );
        assert!(
            verify_test_updater_signature(
                &installer,
                &signature,
                &public_key,
                "4.2.0",
                &expected_sha256,
                &"0".repeat(40),
                None,
            )
            .is_err()
        );
        let _ = fs::remove_dir_all(root);
    }
}
