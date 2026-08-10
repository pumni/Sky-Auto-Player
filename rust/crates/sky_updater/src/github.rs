use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::archive::{parse_sha_sidecar, sha256_bytes, sha256_file};
use crate::cli::Channel;
use crate::error::{Result, UpdaterError};
use crate::http::{HttpClient, validate_https_url};
use crate::manifest::Manifest;
use crate::{
    API_MAX_BYTES, APP_NAME, MANIFEST_MAX_BYTES, MANIFEST_NAME, SIDECAR_MAX_BYTES,
    ZIP_MAX_COMPRESSED_BYTES,
};

#[derive(Clone, Debug)]
pub struct ReleasePayload {
    pub version: String,
    pub zip_name: String,
    pub zip_path: PathBuf,
    pub zip_sha256: String,
    pub manifest: Manifest,
    pub external_manifest_sha256: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseResponse {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

pub fn expected_zip_name(target_version: &str) -> String {
    format!("{APP_NAME}-v{target_version}.zip")
}

pub fn release_api_url(target_version: &str) -> String {
    format!("https://api.github.com/repos/pumni/{APP_NAME}/releases/tags/v{target_version}")
}

pub fn fetch_exact_release<C: HttpClient>(
    client: &C,
    target_version: &str,
    channel: Channel,
    zip_destination: &Path,
) -> Result<ReleasePayload> {
    let metadata_bytes = client.get(&release_api_url(target_version), API_MAX_BYTES)?;
    let release: ReleaseResponse = serde_json::from_slice(&metadata_bytes)
        .map_err(|err| UpdaterError::ReleaseNotFound(err.to_string()))?;
    if release.draft || release.tag_name != format!("v{target_version}") {
        return Err(UpdaterError::ReleasePolicyRejected(
            "release tag/draft policy mismatch".into(),
        ));
    }
    if channel == Channel::Stable && release.prerelease {
        return Err(UpdaterError::ReleasePolicyRejected(
            "stable channel rejects prerelease".into(),
        ));
    }
    let zip_name = expected_zip_name(target_version);
    let sidecar_name = format!("{zip_name}.sha256");
    let zip_url = exact_asset_url(&release.assets, &zip_name)?;
    let sidecar_url = exact_asset_url(&release.assets, &sidecar_name)?;
    let manifest_url = exact_asset_url(&release.assets, MANIFEST_NAME)?;
    if !zip_destination.is_absolute() {
        return Err(UpdaterError::InstallRootInvalid(
            "ZIP staging path must be absolute".into(),
        ));
    }
    client.download_to(zip_url, ZIP_MAX_COMPRESSED_BYTES as usize, zip_destination)?;
    if std::fs::metadata(zip_destination)?.len() > ZIP_MAX_COMPRESSED_BYTES {
        return Err(UpdaterError::NetworkFailure(
            "ZIP exceeds compressed size bound".into(),
        ));
    }
    let sidecar = client.get(sidecar_url, SIDECAR_MAX_BYTES)?;
    let expected_hash = parse_sha_sidecar(&sidecar, &zip_name)?;
    if sha256_file(zip_destination)? != expected_hash {
        return Err(UpdaterError::ChecksumMismatch);
    }
    let external_manifest = client.get(manifest_url, MANIFEST_MAX_BYTES)?;
    let manifest = Manifest::parse(&external_manifest)?;
    manifest.validate(Some(target_version))?;
    Ok(ReleasePayload {
        version: target_version.into(),
        zip_name,
        zip_path: zip_destination.to_owned(),
        zip_sha256: expected_hash,
        manifest,
        external_manifest_sha256: sha256_bytes(&external_manifest),
    })
}

fn exact_asset_url<'a>(assets: &'a [ReleaseAsset], expected_name: &str) -> Result<&'a str> {
    let matches = assets
        .iter()
        .filter(|asset| asset.name == expected_name)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(UpdaterError::AssetMissing(expected_name.into()));
    }
    validate_https_url(&matches[0].browser_download_url)?;
    Ok(&matches[0].browser_download_url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::{Value, json};

    use crate::PRIMARY_EXE;
    use crate::manifest::ManifestFile;

    struct FakeHttpClient {
        responses: HashMap<String, Vec<u8>>,
    }

    impl HttpClient for FakeHttpClient {
        fn get(&self, url: &str, _max_bytes: usize) -> Result<Vec<u8>> {
            self.responses.get(url).cloned().ok_or_else(|| {
                UpdaterError::NetworkFailure(format!("missing fake response: {url}"))
            })
        }
    }

    struct Fixture {
        client: FakeHttpClient,
        metadata: Value,
        sidecar_url: String,
        manifest_url: String,
        manifest_bytes: Vec<u8>,
    }

    fn fixture() -> Fixture {
        let target = "2.0.0";
        let zip_name = expected_zip_name(target);
        let zip_url = "https://objects.githubusercontent.com/sky-player.zip";
        let sidecar_url = "https://objects.githubusercontent.com/sky-player.sha256";
        let manifest_url = "https://release-assets.githubusercontent.com/sky-player-manifest";
        let zip_bytes = b"valid enough for the fetch-layer test".to_vec();
        let manifest = Manifest {
            schema_version: crate::SCHEMA_VERSION,
            app: APP_NAME.into(),
            version: target.into(),
            executable: PRIMARY_EXE.into(),
            git_head: "a".repeat(40),
            dirty_worktree: false,
            native_build_commit: "b".repeat(40),
            build_time_utc: "2026-08-10T00:00:00Z".into(),
            files: vec![ManifestFile {
                path: PRIMARY_EXE.into(),
                size: 3,
                sha256: crate::archive::sha256_bytes(b"app"),
            }],
        };
        let manifest_bytes = serde_json::to_vec(&manifest).expect("manifest JSON");
        let zip_hash = crate::archive::sha256_bytes(&zip_bytes);
        let metadata = json!({
            "tag_name": "v2.0.0",
            "draft": false,
            "prerelease": false,
            "assets": [
                {"name": zip_name, "browser_download_url": zip_url},
                {"name": format!("{zip_name}.sha256"), "browser_download_url": sidecar_url},
                {"name": crate::MANIFEST_NAME, "browser_download_url": manifest_url}
            ]
        });
        let mut responses = HashMap::new();
        responses.insert(zip_url.into(), zip_bytes);
        responses.insert(
            sidecar_url.into(),
            format!("{zip_hash}  {zip_name}\n").into_bytes(),
        );
        responses.insert(manifest_url.into(), manifest_bytes.clone());
        Fixture {
            client: FakeHttpClient { responses },
            metadata,
            sidecar_url: sidecar_url.into(),
            manifest_url: manifest_url.into(),
            manifest_bytes,
        }
    }

    fn fetch_fixture(mut fixture: Fixture, channel: Channel) -> Result<ReleasePayload> {
        let api_url = release_api_url("2.0.0");
        fixture.client.responses.insert(
            api_url,
            serde_json::to_vec(&fixture.metadata).expect("release JSON"),
        );
        let destination = std::env::temp_dir().join(format!(
            "sky-updater-github-test-{}-{}-release.zip",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        let result = fetch_exact_release(&fixture.client, "2.0.0", channel, &destination);
        let _ = std::fs::remove_file(destination);
        result
    }

    fn assert_rejected(fixture: Fixture, channel: Channel, expected: &str) {
        let error = fetch_fixture(fixture, channel).expect_err("fixture should be rejected");
        assert!(
            error.to_string().contains(expected),
            "expected {expected:?}, got {error}"
        );
    }

    fn assets(metadata: &mut Value) -> &mut Vec<Value> {
        metadata["assets"].as_array_mut().expect("asset array")
    }

    #[test]
    fn expected_names_are_exact() {
        assert_eq!(expected_zip_name("3.2.0"), "Sky-Auto-Player-v3.2.0.zip");
        assert!(release_api_url("3.2.0").ends_with("/releases/tags/v3.2.0"));
    }

    #[test]
    fn accepts_valid_exact_release_with_fake_http() {
        let fixture = fixture();
        let payload = fetch_fixture(fixture, Channel::Stable).expect("valid release");
        assert_eq!(payload.version, "2.0.0");
        assert_eq!(payload.zip_name, "Sky-Auto-Player-v2.0.0.zip");
    }

    #[test]
    fn rejects_draft_and_wrong_tag() {
        let mut draft = fixture();
        draft.metadata["draft"] = json!(true);
        assert_rejected(draft, Channel::Stable, "release tag/draft policy mismatch");

        let mut wrong_tag = fixture();
        wrong_tag.metadata["tag_name"] = json!("v2.0.1");
        assert_rejected(
            wrong_tag,
            Channel::Stable,
            "release tag/draft policy mismatch",
        );
    }

    #[test]
    fn rejects_stable_prerelease() {
        let mut fixture = fixture();
        fixture.metadata["prerelease"] = json!(true);
        assert_rejected(
            fixture,
            Channel::Stable,
            "stable channel rejects prerelease",
        );
    }

    #[test]
    fn rejects_missing_or_duplicate_canonical_assets() {
        for index in 0..3 {
            let mut missing = fixture();
            assets(&mut missing.metadata).remove(index);
            assert_rejected(missing, Channel::Stable, "asset missing");

            let mut duplicate = fixture();
            let duplicate_asset = assets(&mut duplicate.metadata)[index].clone();
            assets(&mut duplicate.metadata).push(duplicate_asset);
            assert_rejected(duplicate, Channel::Stable, "asset missing");
        }
    }

    #[test]
    fn rejects_bad_asset_url() {
        let mut fixture = fixture();
        assets(&mut fixture.metadata)[0]["browser_download_url"] =
            json!("http://objects.githubusercontent.com/unsafe");
        assert_rejected(fixture, Channel::Stable, "HTTPS is required");
    }

    #[test]
    fn rejects_malformed_sidecar_and_zip_hash_mismatch() {
        let mut malformed = fixture();
        malformed
            .client
            .responses
            .insert(malformed.sidecar_url.clone(), b"not a sidecar".to_vec());
        assert_rejected(malformed, Channel::Stable, "sidecar record");

        let mut mismatch = fixture();
        mismatch.client.responses.insert(
            mismatch.sidecar_url.clone(),
            format!("{}  Sky-Auto-Player-v2.0.0.zip\n", "0".repeat(64)).into_bytes(),
        );
        assert_rejected(mismatch, Channel::Stable, "checksum mismatch");
    }

    #[test]
    fn rejects_manifest_version_mismatch() {
        let mut fixture = fixture();
        let mut manifest: Manifest =
            serde_json::from_slice(&fixture.manifest_bytes).expect("fixture manifest");
        manifest.version = "2.0.1".into();
        fixture.client.responses.insert(
            fixture.manifest_url.clone(),
            serde_json::to_vec(&manifest).expect("manifest JSON"),
        );
        assert_rejected(
            fixture,
            Channel::Stable,
            "manifest version does not match target",
        );
    }
}
