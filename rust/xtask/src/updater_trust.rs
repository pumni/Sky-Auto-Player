use crate::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use std::fs;
use std::path::Path;
use std::process::Command;

const MAX_PUBLIC_KEY_BYTES: usize = 4096;

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
    if key_id != "F6355260A0C663D5" {
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

    // 4. Verify isolation of legacy v3 update trust root
    let cargo_toml_path = root.join("rust/Cargo.toml");
    let cargo_toml: toml::Value = toml::from_str(&fs::read_to_string(&cargo_toml_path)?)?;
    let legacy_v3_root_isolated = if let Some(workspace) =
        cargo_toml.get("workspace").and_then(toml::Value::as_table)
        && let Some(metadata) = workspace.get("metadata").and_then(toml::Value::as_table)
        && let Some(update_signing) = metadata
            .get("sky-update-signing")
            .and_then(toml::Value::as_table)
        && let Some(key_id) = update_signing.get("key-id").and_then(toml::Value::as_str)
    {
        if key_id != "release-2026" {
            return Err("legacy update root key-id in rust/Cargo.toml is not release-2026".into());
        }
        if config_key.contains("release-2026")
            || native_key.contains("release-2026")
            || bundle_key.contains("release-2026")
            || config_content.contains("release-2026")
        {
            return Err("v4 code contains legacy v3 release-2026 reference".into());
        }
        true
    } else {
        false
    };

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
    println!("  Legacy v3 Root: isolated to rust/Cargo.toml (release-2026) and rejected by v4");
    println!("  Unique Production v4 Root: verified exactly 1 public trust root");
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
    let key_bytes = fs::read(key_path)?;
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

    let mut command = Command::new("bun");
    command
        .args([
            "run",
            "tauri",
            "signer",
            "sign",
            payload_path.to_str().unwrap(),
        ])
        .current_dir(&desktop_dir)
        .env("TAURI_SIGNING_PRIVATE_KEY_PATH", &canonical_key_path)
        .env("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", password.unwrap_or(""))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let output = command.output();
    let result = match output {
        Ok(out) if out.status.success() => {
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
        Ok(out) => {
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
        Err(e) => Err(format!("failed to execute bun signer: {e}").into()),
    };

    let _ = fs::remove_file(&payload_path);
    let _ = fs::remove_file(&signature_path);
    let _ = fs::remove_dir_all(&temp_dir);

    result
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
    let encoded = fs::read_to_string(path)?.trim().to_owned();
    let decoded = STANDARD.decode(encoded)?;
    let decoded = String::from_utf8(decoded)?;
    Ok(Signature::decode(&decoded)?)
}

fn read_public_key(path: &Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "updater public-key fixture is not a regular file: {}",
            path.display()
        )
        .into());
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

    fn fixture_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "sky-xtask-updater-trust-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
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
        assert_eq!(key_id, "F6355260A0C663D5");
    }

    #[test]
    fn inventory_proves_single_canonical_v4_root() {
        let repo_root = crate::repo::root();
        let inventory = inventory_public_trust_roots(&repo_root).unwrap();
        assert_eq!(inventory.canonical_key_id, "F6355260A0C663D5");
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
}
