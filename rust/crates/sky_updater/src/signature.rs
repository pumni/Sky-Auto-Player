use std::fs;
use std::path::{Path, PathBuf};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::Deserialize;

use crate::archive::sha256_file;
use crate::error::{Result, UpdaterError, io_context};
use crate::manifest::Manifest;
use crate::{PRIMARY_EXE, UPDATER_EXE};

const RELEASE_KEY_ID: &str = "release-2026";
#[cfg(not(test))]
const RELEASE_PUBLIC_KEY: [u8; 32] = [
    0xf2, 0x91, 0x25, 0xc7, 0x1b, 0xdc, 0xb3, 0x21, 0xdd, 0xd3, 0x67, 0x22, 0x01, 0x68, 0x93, 0xf9,
    0x1b, 0x0b, 0xcb, 0x68, 0x4e, 0x7a, 0x04, 0x99, 0xb4, 0xbd, 0x53, 0x53, 0xbe, 0x35, 0x4c, 0xca,
];

#[cfg(test)]
const RELEASE_PUBLIC_KEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DetachedManifestSignature {
    key_id: String,
    signature: String,
}

/// Verify the detached signature over the exact bytes downloaded as MANIFEST.json.
///
/// The trusted key set is compiled into the updater. A release may identify a
/// key, but it cannot replace or extend this set through remote metadata.
pub fn verify_manifest_signature(manifest_bytes: &[u8], signature_bytes: &[u8]) -> Result<String> {
    if signature_bytes.len() > 16 * 1024 {
        return Err(UpdaterError::ManifestSignatureInvalid(
            "signature exceeds size bound".into(),
        ));
    }
    let detached: DetachedManifestSignature =
        serde_json::from_slice(signature_bytes).map_err(|_| {
            UpdaterError::ManifestSignatureInvalid("signature envelope is not valid JSON".into())
        })?;
    if detached.key_id != RELEASE_KEY_ID {
        return Err(UpdaterError::ManifestSignatureInvalid(format!(
            "untrusted key id: {}",
            detached.key_id
        )));
    }
    let signature = decode_hex::<64>(&detached.signature).ok_or_else(|| {
        UpdaterError::ManifestSignatureInvalid("signature must be exactly 64 hex bytes".into())
    })?;
    let verifying_key = VerifyingKey::from_bytes(&RELEASE_PUBLIC_KEY).map_err(|_| {
        UpdaterError::ManifestSignatureInvalid("embedded public key is invalid".into())
    })?;
    verifying_key
        .verify(manifest_bytes, &Signature::from_bytes(&signature))
        .map_err(|_| {
            UpdaterError::ManifestSignatureInvalid(
                "signature does not match the exact manifest bytes".into(),
            )
        })?;
    Ok(detached.key_id)
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

#[cfg(test)]
mod signed_manifest_tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;

    const TEST_SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];

    fn detached(message: &[u8]) -> Vec<u8> {
        let signature = SigningKey::from_bytes(&TEST_SEED).sign(message);
        serde_json::to_vec(&json!({
            "key_id": RELEASE_KEY_ID,
            "signature": signature.to_bytes().iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        }))
        .expect("signature envelope")
    }

    #[test]
    fn valid_signature_is_accepted() {
        assert_eq!(
            verify_manifest_signature(b"canonical manifest", &detached(b"canonical manifest"))
                .expect("valid signature"),
            RELEASE_KEY_ID
        );
    }

    #[test]
    fn modified_manifest_is_rejected() {
        let signature = detached(b"canonical manifest");
        assert!(matches!(
            verify_manifest_signature(b"modified manifest", &signature),
            Err(UpdaterError::ManifestSignatureInvalid(message))
                if message.contains("exact manifest bytes")
        ));
    }

    #[test]
    fn wrong_key_truncated_signature_and_unknown_id_are_rejected() {
        let mut wrong_key = serde_json::from_slice::<serde_json::Value>(&detached(b"message"))
            .expect("signature JSON");
        wrong_key["signature"] = json!("00".repeat(64));
        assert!(
            verify_manifest_signature(b"message", &serde_json::to_vec(&wrong_key).unwrap())
                .is_err()
        );

        let truncated = json!({"key_id": RELEASE_KEY_ID, "signature": "00"});
        assert!(
            verify_manifest_signature(b"message", &serde_json::to_vec(&truncated).unwrap())
                .is_err()
        );

        let unknown = json!({
            "key_id": "future-key",
            "signature": serde_json::from_slice::<serde_json::Value>(&detached(b"message"))
                .unwrap()["signature"]
        });
        assert!(
            verify_manifest_signature(b"message", &serde_json::to_vec(&unknown).unwrap()).is_err()
        );
    }
}

pub fn project_owned_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for required in [PRIMARY_EXE, UPDATER_EXE, "native_calibration.exe"] {
        let path = root.join(required);
        if !path.is_file() {
            return Err(UpdaterError::ManifestInvalid(format!(
                "required project file missing: {required}"
            )));
        }
        files.push(path);
    }
    let internal = root.join("_internal");
    if internal.is_dir() {
        collect_project_pyds(&internal, &mut files)?;
    }
    Ok(files)
}

fn collect_project_pyds(root: &Path, output: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(root)
        .map_err(|error| io_context("verify staging", "read project directory", root, error))?
    {
        let entry = entry.map_err(|error| {
            io_context(
                "verify staging",
                "read project directory entry",
                root,
                error,
            )
        })?;
        let path = entry.path();
        if entry
            .file_type()
            .map_err(|error| io_context("verify staging", "read project entry type", &path, error))?
            .is_symlink()
        {
            return Err(UpdaterError::ManifestInvalid(format!(
                "symlink under _internal: {}",
                path.display()
            )));
        }
        if path.is_dir() {
            collect_project_pyds(&path, output)?;
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("pyd"))
        {
            output.push(path);
        }
    }
    Ok(())
}

pub fn verify_manifest_scope(root: &Path, manifest: &Manifest) -> Result<()> {
    let project_files = project_owned_files(root)?;
    for path in project_files {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| UpdaterError::ManifestInvalid("project file escaped root".into()))?
            .to_string_lossy()
            .replace('\\', "/");
        if !manifest.files.iter().any(|file| file.path == relative) {
            return Err(UpdaterError::ManifestInvalid(format!(
                "project-owned file is absent from manifest: {relative}"
            )));
        }
    }
    Ok(())
}

pub fn verify_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        UpdaterError::ManifestInvalid(format!("missing project file: {}", path.display()))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(UpdaterError::ManifestInvalid(format!(
            "project file is not a regular file: {}",
            path.display()
        )));
    }
    Ok(())
}

pub fn verify_project_files(root: &Path, manifest: &Manifest) -> Result<()> {
    verify_manifest_scope(root, manifest)?;
    for path in project_owned_files(root)? {
        verify_file(&path)?;
        let relative = path
            .strip_prefix(root)
            .map_err(|_| UpdaterError::ManifestInvalid("project file escaped root".into()))?
            .to_string_lossy()
            .replace('\\', "/");
        let entry = manifest
            .files
            .iter()
            .find(|file| file.path == relative)
            .ok_or_else(|| {
                UpdaterError::ManifestInvalid(format!(
                    "project-owned file is absent from manifest: {relative}"
                ))
            })?;
        let metadata = fs::metadata(&path)
            .map_err(|error| io_context("verify staging", "read project metadata", &path, error))?;
        if metadata.len() != entry.size {
            return Err(UpdaterError::ManifestHashMismatch(relative));
        }
        let actual = sha256_file(&path).map_err(|error| match error {
            UpdaterError::Io(source) => {
                io_context("verify staging", "hash project file", &path, source)
            }
            other => other,
        })?;
        if actual != entry.sha256.to_ascii_lowercase() {
            return Err(UpdaterError::ManifestHashMismatch(relative));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::sha256_bytes;
    use crate::manifest::ManifestFile;
    use crate::{APP_NAME, MANIFEST_NAME, SCHEMA_VERSION};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sky-updater-integrity-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ))
    }

    fn write_file(root: &Path, relative: &str, bytes: &[u8]) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("file parent")).expect("file parent");
        fs::write(path, bytes).expect("write fixture file");
    }

    fn fixture_manifest(files: &[(&str, &[u8])]) -> Manifest {
        Manifest {
            schema_version: SCHEMA_VERSION,
            app: APP_NAME.into(),
            version: "3.3.0.dev0".into(),
            executable: PRIMARY_EXE.into(),
            git_head: "a".repeat(40),
            dirty_worktree: false,
            native_build_commit: "b".repeat(40),
            build_time_utc: "2026-08-10T00:00:00Z".into(),
            files: files
                .iter()
                .map(|(path, bytes)| ManifestFile {
                    path: (*path).into(),
                    size: bytes.len() as u64,
                    sha256: sha256_bytes(bytes),
                })
                .collect(),
        }
    }

    fn release_files() -> [(&'static str, &'static [u8]); 3] {
        [
            (PRIMARY_EXE, b"unsigned app"),
            (UPDATER_EXE, b"unsigned updater"),
            ("native_calibration.exe", b"unsigned calibration"),
        ]
    }

    #[test]
    fn valid_unsigned_project_passes_manifest_integrity() {
        let root = temp_root("valid");
        fs::create_dir_all(&root).expect("fixture root");
        let files = release_files();
        for (path, bytes) in files {
            write_file(&root, path, bytes);
        }
        let manifest = fixture_manifest(&files);
        verify_project_files(&root, &manifest).expect("unsigned bytes should be accepted");
        fs::remove_dir_all(root).expect("cleanup fixture");
    }

    #[test]
    fn project_hash_mismatch_is_rejected() {
        let root = temp_root("hash-mismatch");
        fs::create_dir_all(&root).expect("fixture root");
        let files = release_files();
        for (path, bytes) in files {
            write_file(&root, path, bytes);
        }
        let manifest = fixture_manifest(&files);
        write_file(&root, PRIMARY_EXE, b"tampered app");
        assert!(matches!(
            verify_project_files(&root, &manifest),
            Err(UpdaterError::ManifestHashMismatch(path)) if path == PRIMARY_EXE
        ));
        fs::remove_dir_all(root).expect("cleanup fixture");
    }

    #[test]
    fn missing_project_manifest_entry_is_rejected() {
        let root = temp_root("missing-entry");
        fs::create_dir_all(&root).expect("fixture root");
        let files = release_files();
        for (path, bytes) in files {
            write_file(&root, path, bytes);
        }
        let manifest = fixture_manifest(&files[..2]);
        assert!(matches!(
            verify_project_files(&root, &manifest),
            Err(UpdaterError::ManifestInvalid(message))
                if message.contains("native_calibration.exe")
        ));
        fs::remove_dir_all(root).expect("cleanup fixture");
    }

    #[test]
    fn missing_project_file_is_rejected() {
        let root = temp_root("missing-file");
        fs::create_dir_all(&root).expect("fixture root");
        let files = release_files();
        write_file(&root, PRIMARY_EXE, files[0].1);
        let manifest = fixture_manifest(&files);
        assert!(matches!(
            verify_project_files(&root, &manifest),
            Err(UpdaterError::ManifestInvalid(message))
                if message.contains("Sky-Auto-Player-Updater.exe")
        ));
        fs::write(root.join(MANIFEST_NAME), b"not used by this helper").expect("fixture marker");
        fs::remove_dir_all(root).expect("cleanup fixture");
    }
}
