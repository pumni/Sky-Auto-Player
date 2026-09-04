use crate::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
use std::fs;
use std::path::Path;

const MAX_PUBLIC_KEY_BYTES: usize = 4096;

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
}
