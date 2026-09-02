use crate::{Result, repo, update_trust};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::env;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const APP_NAME: &str = "Sky-Auto-Player";
const PRIMARY_EXE: &str = "Sky-Auto-Player.exe";
const REQUIRED: &[&str] = &[
    PRIMARY_EXE,
    "native_calibration.exe",
    "Sky-Auto-Player-Updater.exe",
    "MANIFEST.json",
];

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct DetachedManifestSignature {
    key_id: String,
    signature: String,
}

fn validate_signing_key(
    signing_key: &SigningKey,
    key_id: &str,
    trust: &update_trust::UpdateTrustConfig,
) -> Result<()> {
    if key_id != trust.key_id {
        return Err(format!(
            "manifest signing key id {key_id} does not match configured {}",
            trust.key_id
        )
        .into());
    }
    let derived_public_key = encode_hex(signing_key.verifying_key().to_bytes());
    if derived_public_key != trust.public_key_hex {
        return Err(format!(
            "manifest signing key does not match configured public key for {}",
            trust.key_id
        )
        .into());
    }
    Ok(())
}

pub fn sign(
    manifest_path: &Path,
    output_path: &Path,
    requested_key_id: Option<&str>,
) -> Result<()> {
    let trust = update_trust::load(&repo::root())?;
    let key_id = requested_key_id.unwrap_or(&trust.key_id);
    let key_hex = env::var("SKY_UPDATE_SIGNING_KEY_HEX")
        .map_err(|_| "SKY_UPDATE_SIGNING_KEY_HEX is required for manifest signing")?;
    let key_bytes = decode_hex::<32>(&key_hex)
        .ok_or("SKY_UPDATE_SIGNING_KEY_HEX must be exactly 32 bytes of hex")?;
    let signing_key = SigningKey::from_bytes(&key_bytes);
    validate_signing_key(&signing_key, key_id, &trust)?;
    let manifest = std::fs::read(manifest_path)?;
    let signature = signing_key.sign(&manifest);
    let envelope = DetachedManifestSignature {
        key_id: key_id.to_owned(),
        signature: encode_hex(signature.to_bytes()),
    };
    let mut bytes = serde_json::to_vec_pretty(&envelope)?;
    bytes.push(b'\n');
    std::fs::write(output_path, bytes)?;
    println!(
        "[xtask] signed {} with key {} -> {}",
        manifest_path.display(),
        key_id,
        output_path.display()
    );
    Ok(())
}

pub fn verify_signature(manifest_path: &Path, signature_path: &Path) -> Result<()> {
    let trust = update_trust::load(&repo::root())?;
    let manifest = std::fs::read(manifest_path)?;
    let envelope: DetachedManifestSignature =
        serde_json::from_slice(&std::fs::read(signature_path)?)?;
    if envelope.key_id != trust.key_id {
        return Err(format!(
            "manifest signature key id {} does not match configured {}",
            envelope.key_id, trust.key_id
        )
        .into());
    }
    let signature = decode_hex::<64>(&envelope.signature)
        .ok_or("manifest signature must be exactly 64 bytes of hex")?;
    let verifying_key = VerifyingKey::from_bytes(&trust.public_key)?;
    verifying_key.verify_strict(&manifest, &Signature::from_bytes(&signature))?;
    println!(
        "[xtask] verified manifest signature {} with key {}",
        manifest_path.display(),
        trust.key_id
    );
    Ok(())
}

fn decode_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut output = [0u8; N];
    let bytes = value.as_bytes();
    for (index, output_byte) in output.iter_mut().enumerate() {
        let offset = index * 2;
        *output_byte = (hex_digit(bytes[offset])? << 4) | hex_digit(bytes[offset + 1])?;
    }
    Some(output)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn encode_hex<const N: usize>(bytes: [u8; N]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(N * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

pub fn sha256(path: &Path) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(std::fs::read(path)?);
    Ok(repo::hex_digest(digest.finalize()))
}

fn relative_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut result = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type().is_symlink() {
            return Err(format!("release contains symlink: {}", path.display()).into());
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        result.push((relative, path.to_path_buf()));
    }
    result.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(result)
}

pub fn forbidden_paths(root: &Path) -> Result<Vec<String>> {
    let mut forbidden = Vec::new();
    for (relative, _path) in relative_files(root)? {
        let folded = relative.to_ascii_lowercase();
        let name = Path::new(&relative)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let extension = Path::new(&relative)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let components = folded.split('/').collect::<Vec<_>>();
        if folded == "sky-auto-player-core.exe"
            || name == "sky-player.exe"
            || folded.starts_with("_internal/")
            || name.starts_with("python")
            || name.starts_with("base_library")
            || matches!(extension.as_str(), "pyd" | "py" | "pyc")
            || folded.contains("sky_player_rs")
            || name.starts_with("sky_updater_e2e")
            || components.contains(&"installer")
            || components.contains(&".pytest_cache")
            || components.contains(&"__pycache__")
            || name == "updater.bat"
            || name == "updater.ps1"
            || extension == "bat"
            || extension == "ps1"
            || name == "testresults.xml"
        {
            forbidden.push(relative);
        }
    }
    Ok(forbidden)
}

pub fn write(
    release_dir: &Path,
    version: &str,
    head: &str,
    native_build_commit: &str,
) -> Result<()> {
    let files = relative_files(release_dir)?
        .into_iter()
        .filter(|(relative, _)| relative != "MANIFEST.json")
        .map(|(relative, path)| {
            Ok(json!({
                "path": relative,
                "size": std::fs::metadata(&path)?.len(),
                "sha256": sha256(&path)?,
            }))
        })
        .collect::<Result<Vec<_>>>()?;
    let build_time_utc = repo::commit_time_utc(&repo::root())?;
    let manifest = json!({
        "schema_version": 2,
        "app": APP_NAME,
        "version": version,
        "executable": PRIMARY_EXE,
        "git_head": head,
        "dirty_worktree": false,
        "native_build_commit": native_build_commit,
        "build_time_utc": build_time_utc,
        "files": files,
    });
    std::fs::write(
        release_dir.join("MANIFEST.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(())
}

pub fn file_count(release_dir: &Path) -> Result<usize> {
    Ok(relative_files(release_dir)?.len())
}

pub fn managed_count(release_dir: &Path) -> Result<usize> {
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(release_dir.join("MANIFEST.json"))?)?;
    Ok(manifest
        .get("files")
        .and_then(Value::as_array)
        .map_or(0, Vec::len))
}

pub fn verify_release(release_dir: &Path) -> Result<()> {
    let release_dir = release_dir.canonicalize()?;
    if !release_dir.is_dir() {
        return Err(format!("release directory is missing: {}", release_dir.display()).into());
    }
    let files = relative_files(&release_dir)?;
    let actual: std::collections::BTreeSet<String> =
        files.iter().map(|(relative, _)| relative.clone()).collect();
    let mut folded = std::collections::BTreeMap::new();
    for relative in &actual {
        if let Some(previous) = folded.insert(relative.to_ascii_lowercase(), relative) {
            return Err(format!("case-colliding release paths: {previous}, {relative}").into());
        }
    }
    let missing: Vec<&str> = REQUIRED
        .iter()
        .copied()
        .filter(|required| !actual.contains(*required))
        .collect();
    if !missing.is_empty() {
        return Err(format!("release is missing required files: {missing:?}").into());
    }
    let forbidden = forbidden_paths(&release_dir)?;
    if !forbidden.is_empty() {
        return Err(format!("release contains forbidden artifacts: {forbidden:?}").into());
    }
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(release_dir.join("MANIFEST.json"))?)?;
    if manifest.get("schema_version") != Some(&json!(2))
        || manifest.get("app") != Some(&json!(APP_NAME))
        || manifest.get("executable") != Some(&json!(PRIMARY_EXE))
        || manifest.get("dirty_worktree") != Some(&json!(false))
    {
        return Err("manifest schema/app/executable/clean-worktree contract failed".into());
    }
    for key in ["git_head", "native_build_commit"] {
        let value = manifest
            .get(key)
            .and_then(Value::as_str)
            .ok_or(format!("manifest {key} missing"))?;
        if value.len() != 40 || !value.chars().all(|character| character.is_ascii_hexdigit()) {
            return Err(format!("manifest {key} is not a full commit SHA").into());
        }
    }
    if manifest.get("git_head") != manifest.get("native_build_commit") {
        return Err("manifest git_head and native_build_commit must match".into());
    }
    if manifest
        .get("build_time_utc")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err("manifest build_time_utc is missing".into());
    }
    let version = manifest
        .get("version")
        .and_then(Value::as_str)
        .ok_or("manifest version missing")?;
    let expected_version = repo::project_version(&repo::root())?;
    if expected_version != version {
        return Err(format!(
            "manifest version {version} does not match Cargo version {expected_version}"
        )
        .into());
    }
    let entries = manifest
        .get("files")
        .and_then(Value::as_array)
        .ok_or("manifest files must be an array")?;
    let mut listed = std::collections::BTreeSet::new();
    for entry in entries {
        let object = entry
            .as_object()
            .ok_or("manifest file entry must be an object")?;
        let relative = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or("manifest path missing")?;
        let size = object
            .get("size")
            .and_then(Value::as_u64)
            .ok_or("manifest size missing")?;
        let digest = object
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or("manifest sha256 missing")?;
        if relative.is_empty()
            || relative.contains('\\')
            || relative.starts_with('/')
            || relative.as_bytes().get(1) == Some(&b':')
            || relative
                .split('/')
                .any(|part| part == ".." || part.is_empty())
            || relative == "MANIFEST.json"
            || digest.len() != 64
            || !digest
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(format!("unsafe manifest entry: {relative}").into());
        }
        if !listed.insert(relative.to_owned()) {
            return Err(format!("duplicate manifest entry: {relative}").into());
        }
        let path = release_dir.join(relative);
        if !path.is_file()
            || path.metadata()?.len() != size
            || sha256(&path)? != digest.to_ascii_lowercase()
        {
            return Err(format!("manifest hash/size mismatch: {relative}").into());
        }
    }
    if listed
        != actual
            .into_iter()
            .filter(|path| path != "MANIFEST.json")
            .collect()
    {
        return Err("manifest file set does not match release tree".into());
    }
    println!(
        "Release manifest verified: {} (version {version})",
        release_dir.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> tempfile_like::TempDir {
        tempfile_like::TempDir::new()
    }

    mod tempfile_like {
        use std::path::{Path, PathBuf};
        pub struct TempDir(PathBuf);
        impl TempDir {
            pub fn new() -> Self {
                let nonce = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |duration| duration.as_nanos());
                let path = std::env::temp_dir()
                    .join(format!("sky-xtask-manifest-{}-{nonce}", std::process::id()));
                let _ = std::fs::remove_dir_all(&path);
                std::fs::create_dir_all(&path).unwrap();
                Self(path)
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn rejects_python_runtime_paths() {
        let temp = fixture();
        fs::write(temp.path().join(PRIMARY_EXE), b"app").unwrap();
        fs::write(temp.path().join("python314.dll"), b"bad").unwrap();
        assert!(
            forbidden_paths(temp.path())
                .unwrap()
                .contains(&"python314.dll".into())
        );
    }

    #[test]
    fn rejects_a_valid_but_wrong_signing_key() {
        let trust = update_trust::load(&repo::root()).expect("update trust metadata");
        let wrong_key = SigningKey::from_bytes(&[0u8; 32]);
        assert!(validate_signing_key(&wrong_key, &trust.key_id, &trust).is_err());
    }

    #[test]
    fn writes_and_verifies_schema_two_manifest() {
        let temp = fixture();
        for name in [
            PRIMARY_EXE,
            "native_calibration.exe",
            "Sky-Auto-Player-Updater.exe",
        ] {
            fs::write(temp.path().join(name), name.as_bytes()).unwrap();
        }
        write(temp.path(), "3.5.0", &"a".repeat(40), &"a".repeat(40)).unwrap();
        let written: Value =
            serde_json::from_slice(&fs::read(temp.path().join("MANIFEST.json")).unwrap()).unwrap();
        assert_ne!(
            written.get("build_time_utc").and_then(Value::as_str),
            Some("1970-01-01T00:00:00Z")
        );
        verify_release(temp.path()).unwrap();
        fs::write(temp.path().join(PRIMARY_EXE), b"tampered").unwrap();
        assert!(verify_release(temp.path()).is_err());
    }

    #[test]
    fn rejects_legacy_and_test_artifacts_anywhere_in_tree() {
        for relative in [
            "nested/Sky-Player.exe",
            "songs/installer/payload.txt",
            "songs/cleanup.ps1",
            "songs/cleanup.bat",
            "songs/TestResults.xml",
            ".pytest_cache/state",
            "songs/__pycache__/state",
            "songs/sky_updater_e2e-helper.exe",
        ] {
            let temp = fixture();
            for name in [
                PRIMARY_EXE,
                "native_calibration.exe",
                "Sky-Auto-Player-Updater.exe",
            ] {
                fs::write(temp.path().join(name), name.as_bytes()).unwrap();
            }
            write(temp.path(), "3.5.0", &"a".repeat(40), &"a".repeat(40)).unwrap();
            let path = temp.path().join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, b"forbidden").unwrap();
            assert!(verify_release(temp.path()).is_err(), "{relative}");
        }
    }

    #[test]
    fn rejects_manifest_provenance_drift() {
        let temp = fixture();
        for name in [
            PRIMARY_EXE,
            "native_calibration.exe",
            "Sky-Auto-Player-Updater.exe",
        ] {
            fs::write(temp.path().join(name), name.as_bytes()).unwrap();
        }
        write(temp.path(), "3.5.0", &"a".repeat(40), &"a".repeat(40)).unwrap();
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(temp.path().join("MANIFEST.json")).unwrap()).unwrap();
        manifest["native_build_commit"] = json!("b".repeat(40));
        fs::write(
            temp.path().join("MANIFEST.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(verify_release(temp.path()).is_err());
    }

    #[test]
    fn rejects_parent_traversal_in_manifest() {
        let temp = fixture();
        for name in [
            PRIMARY_EXE,
            "native_calibration.exe",
            "Sky-Auto-Player-Updater.exe",
        ] {
            fs::write(temp.path().join(name), name.as_bytes()).unwrap();
        }
        let payload = json!({
            "schema_version": 2, "app": APP_NAME, "version": "3.5.0", "executable": PRIMARY_EXE,
            "dirty_worktree": false, "files": [{"path":"../escape", "size":0, "sha256":"0".repeat(64)}]
        });
        fs::write(
            temp.path().join("MANIFEST.json"),
            serde_json::to_vec(&payload).unwrap(),
        )
        .unwrap();
        assert!(verify_release(temp.path()).is_err());
    }
}
