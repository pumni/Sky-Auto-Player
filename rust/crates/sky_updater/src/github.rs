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

    #[test]
    fn expected_names_are_exact() {
        assert_eq!(expected_zip_name("3.2.0"), "Sky-Auto-Player-v3.2.0.zip");
        assert!(release_api_url("3.2.0").ends_with("/releases/tags/v3.2.0"));
    }
}
