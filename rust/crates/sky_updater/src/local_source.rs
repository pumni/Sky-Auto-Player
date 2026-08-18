//! Local release source for the separately built E2E updater binary.
//!
//! This module is feature-gated and is not reachable from the production
//! `sky_updater` binary. It applies the same ZIP, sidecar, and external
//! manifest checks as the GitHub source; only the transport is local I/O.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::archive::{parse_sha_sidecar, sha256_bytes, sha256_file};
use crate::cli::Channel;
use crate::error::{Result, UpdaterError};
use crate::github::{ReleasePayload, ReleaseSource, expected_zip_name};
use crate::manifest::Manifest;
use crate::version::Pep440Version;
use crate::{MANIFEST_MAX_BYTES, MANIFEST_NAME, SIDECAR_MAX_BYTES, ZIP_MAX_COMPRESSED_BYTES};

#[derive(Clone, Debug)]
pub struct LocalReleaseSource {
    release_dir: PathBuf,
}

impl LocalReleaseSource {
    pub fn new(release_dir: &Path) -> Result<Self> {
        let release_dir = release_dir
            .canonicalize()
            .map_err(|error| UpdaterError::InstallRootInvalid(error.to_string()))?;
        if !release_dir.is_dir() {
            return Err(UpdaterError::InstallRootInvalid(
                "local release source must be a directory".into(),
            ));
        }
        Ok(Self { release_dir })
    }
}

impl ReleaseSource for LocalReleaseSource {
    fn fetch_exact_release(
        &self,
        target_version: &str,
        channel: Channel,
        zip_destination: &Path,
    ) -> Result<ReleasePayload> {
        let target = Pep440Version::parse(target_version)?;
        if channel == Channel::Stable && target.is_prerelease() {
            return Err(UpdaterError::ReleasePolicyRejected(
                "stable channel cannot install a prerelease".into(),
            ));
        }
        if !zip_destination.is_absolute() {
            return Err(UpdaterError::InstallRootInvalid(
                "ZIP staging path must be absolute".into(),
            ));
        }
        let zip_name = expected_zip_name(target_version);
        let zip_source = self.release_dir.join(&zip_name);
        let sidecar_source = self.release_dir.join(format!("{zip_name}.sha256"));
        let manifest_source = self.release_dir.join(MANIFEST_NAME);
        for path in [&zip_source, &sidecar_source, &manifest_source] {
            let metadata = fs::symlink_metadata(path).map_err(|error| {
                UpdaterError::AssetMissing(format!("{}: {error}", path.display()))
            })?;
            if !metadata.file_type().is_file() {
                return Err(UpdaterError::AssetMissing(path.display().to_string()));
            }
        }
        let sidecar = read_bounded(&sidecar_source, SIDECAR_MAX_BYTES)?;
        let expected_hash = parse_sha_sidecar(&sidecar, &zip_name)?;
        let compressed_size = fs::metadata(&zip_source)?.len();
        if compressed_size > ZIP_MAX_COMPRESSED_BYTES {
            return Err(UpdaterError::NetworkFailure(
                "ZIP exceeds compressed size bound".into(),
            ));
        }
        let parent = zip_destination.parent().ok_or_else(|| {
            UpdaterError::InstallRootInvalid("ZIP staging path has no parent".into())
        })?;
        fs::create_dir_all(parent)?;
        let mut input = File::open(&zip_source)?;
        let mut output = File::create(zip_destination)?;
        io::copy(&mut input, &mut output)?;
        output.flush()?;
        output.sync_all()?;
        if sha256_file(zip_destination)? != expected_hash {
            return Err(UpdaterError::ChecksumMismatch);
        }
        let manifest_bytes = read_bounded(&manifest_source, MANIFEST_MAX_BYTES)?;
        let manifest = Manifest::parse(&manifest_bytes)?;
        manifest.validate(Some(target_version))?;
        Ok(ReleasePayload {
            version: target_version.into(),
            zip_name,
            zip_path: zip_destination.to_owned(),
            zip_sha256: expected_hash,
            manifest,
            external_manifest_sha256: sha256_bytes(&manifest_bytes),
        })
    }
}

fn read_bounded(path: &Path, max_bytes: usize) -> Result<Vec<u8>> {
    let bytes = fs::read(path)?;
    if bytes.len() > max_bytes {
        return Err(UpdaterError::NetworkFailure(format!(
            "release asset exceeds size bound: {}",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("asset")
        )));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::archive::sha256_bytes;
    use crate::manifest::ManifestFile;
    use crate::{APP_NAME, PRIMARY_EXE, SCHEMA_VERSION};

    fn fixture_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "sky-updater-local-source-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    fn write_fixture(root: &Path, sidecar_hash: &str) -> (String, PathBuf) {
        let target = "2.0.0";
        let zip_name = expected_zip_name(target);
        let zip_path = root.join(&zip_name);
        let mut bytes = std::io::Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut bytes);
            writer
                .start_file(PRIMARY_EXE, zip::write::SimpleFileOptions::default())
                .expect("ZIP file");
            writer.write_all(b"app").expect("ZIP payload");
            writer.finish().expect("ZIP finish");
        }
        std::fs::write(&zip_path, bytes.into_inner()).expect("ZIP");
        std::fs::write(
            root.join(format!("{zip_name}.sha256")),
            format!("{sidecar_hash}  {zip_name}\n"),
        )
        .expect("sidecar");
        let manifest = Manifest {
            schema_version: SCHEMA_VERSION,
            app: APP_NAME.into(),
            version: target.into(),
            executable: PRIMARY_EXE.into(),
            git_head: "a".repeat(40),
            dirty_worktree: false,
            native_build_commit: "b".repeat(40),
            build_time_utc: "2026-08-18T00:00:00Z".into(),
            files: vec![ManifestFile {
                path: PRIMARY_EXE.into(),
                size: 3,
                sha256: sha256_bytes(b"app"),
            }],
        };
        std::fs::write(
            root.join(MANIFEST_NAME),
            serde_json::to_vec(&manifest).expect("manifest"),
        )
        .expect("manifest file");
        (target.into(), zip_path)
    }

    #[test]
    fn local_source_verifies_zip_sidecar_and_manifest() {
        let root = fixture_root();
        std::fs::create_dir_all(&root).expect("root");
        let (target, zip_path) = write_fixture(&root, &"0".repeat(64));
        let zip_bytes = std::fs::read(&zip_path).expect("ZIP bytes");
        let hash = sha256_bytes(&zip_bytes);
        let sidecar = root.join(format!("{}.sha256", expected_zip_name(&target)));
        std::fs::write(
            &sidecar,
            format!("{hash}  {}\n", expected_zip_name(&target)),
        )
        .expect("valid sidecar");
        let source = LocalReleaseSource::new(&root).expect("local source");
        let destination = root.join("downloaded.zip");
        let payload = source
            .fetch_exact_release(&target, Channel::Stable, &destination)
            .expect("verified local release");
        assert_eq!(payload.zip_sha256, hash);
        assert_eq!(std::fs::read(destination).expect("download"), zip_bytes);
        assert_eq!(payload.manifest.version, target);
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_file(zip_path);
    }

    #[test]
    fn local_source_rejects_sidecar_mismatch() {
        let root = fixture_root();
        std::fs::create_dir_all(&root).expect("root");
        let (target, _) = write_fixture(&root, &"0".repeat(64));
        let source = LocalReleaseSource::new(&root).expect("local source");
        let error = source
            .fetch_exact_release(&target, Channel::Stable, &root.join("downloaded.zip"))
            .expect_err("mismatched sidecar");
        assert!(matches!(error, UpdaterError::ChecksumMismatch));
        let _ = std::fs::remove_dir_all(root);
    }
}
