use crate::{Result, version};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use url::Url;

pub const RELEASE_REPOSITORY: &str = "pumni/Sky-Auto-Player";
pub const STABLE_METADATA_PATH: &str = "channels/stable/latest.json";
pub const BETA_METADATA_PATH: &str = "channels/beta/latest.json";
pub const WINDOWS_PLATFORM: &str = "windows-x86_64";
pub const PRODUCT_NAME: &str = "Sky Auto Player";
pub const WINDOWS_ARCH: &str = "x64";
const MAX_NOTES_CHARS: usize = 16 * 1024;
const MAX_SIGNATURE_CHARS: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    Stable,
    Beta,
}

impl Channel {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "stable" => Ok(Self::Stable),
            "beta" => Ok(Self::Beta),
            _ => Err(format!("channel must be stable or beta, got {value:?}").into()),
        }
    }

    pub const fn metadata_path(self) -> &'static str {
        match self {
            Self::Stable => STABLE_METADATA_PATH,
            Self::Beta => BETA_METADATA_PATH,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PlatformMetadata {
    pub signature: String,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TauriMetadata {
    pub version: String,
    pub notes: String,
    pub pub_date: String,
    pub platforms: BTreeMap<String, PlatformMetadata>,
}

#[derive(Clone, Copy)]
pub struct GenerateInput<'a> {
    pub channel: Channel,
    pub version: &'a str,
    pub notes_path: &'a Path,
    pub pub_date: &'a str,
    pub platform: &'a str,
    pub asset_url: &'a str,
    pub signature_path: &'a Path,
    pub output: &'a Path,
}

pub fn canonical_installer_name(version: &str) -> String {
    format!("{PRODUCT_NAME}_{version}_{WINDOWS_ARCH}-setup.exe")
}

pub fn canonical_release_installer_name(version: &str) -> String {
    canonical_installer_name(version).replace(' ', ".")
}

pub fn canonical_asset_url(version: &str) -> String {
    format!(
        "https://github.com/{RELEASE_REPOSITORY}/releases/download/v{version}/{}",
        canonical_release_installer_name(version)
    )
}

pub fn generate(input: GenerateInput<'_>) -> Result<()> {
    let notes = normalise_notes(&fs::read_to_string(input.notes_path)?)?;
    let signature = normalise_signature(&fs::read_to_string(input.signature_path)?)?;
    let metadata = build_metadata(
        input.channel,
        input.version,
        notes,
        input.pub_date,
        input.platform,
        input.asset_url,
        signature,
    )?;
    let payload = serde_json::to_string_pretty(&metadata)? + "\n";
    fs::write(input.output, payload.as_bytes())?;
    println!(
        "[xtask] generated {} v{} metadata: {}",
        channel_name(input.channel),
        metadata.version,
        input.output.display()
    );
    Ok(())
}

pub fn validate(channel: Channel, metadata_path: &Path) -> Result<()> {
    let metadata: TauriMetadata = serde_json::from_slice(&fs::read(metadata_path)?)?;
    validate_metadata(&metadata, channel)?;
    println!(
        "[xtask] v4 {} metadata: PASS (v{}, destination {})",
        channel_name(channel),
        metadata.version,
        channel.metadata_path()
    );
    Ok(())
}

pub fn validate_monotonic(
    channel: Channel,
    current_metadata_path: &Path,
    candidate_metadata_path: &Path,
) -> Result<()> {
    let current: TauriMetadata = serde_json::from_slice(&fs::read(current_metadata_path)?)?;
    let candidate: TauriMetadata = serde_json::from_slice(&fs::read(candidate_metadata_path)?)?;
    validate_roll_forward(channel, Some(&current), &candidate)?;
    println!(
        "[xtask] v4 {} metadata monotonicity: PASS ({} -> {})",
        channel_name(channel),
        current.version,
        candidate.version
    );
    Ok(())
}

fn build_metadata(
    channel: Channel,
    version_value: &str,
    notes: String,
    pub_date: &str,
    platform: &str,
    asset_url: &str,
    signature: String,
) -> Result<TauriMetadata> {
    let mut platforms = BTreeMap::new();
    platforms.insert(
        platform.to_owned(),
        PlatformMetadata {
            signature,
            url: asset_url.to_owned(),
        },
    );
    let metadata = TauriMetadata {
        version: version_value.to_owned(),
        notes,
        pub_date: pub_date.to_owned(),
        platforms,
    };
    validate_metadata(&metadata, channel)?;
    Ok(metadata)
}

fn validate_metadata(metadata: &TauriMetadata, channel: Channel) -> Result<()> {
    let parsed_version = version::parse(&metadata.version)?;
    if parsed_version.major != 4 {
        return Err(format!(
            "v4 updater metadata must contain a major version of 4: {}",
            metadata.version
        )
        .into());
    }
    match channel {
        Channel::Stable if !parsed_version.pre.is_empty() => {
            return Err(format!(
                "stable metadata must not contain a prerelease version: {}",
                metadata.version
            )
            .into());
        }
        Channel::Beta if parsed_version.pre.is_empty() => {
            return Err(format!(
                "beta metadata must contain a prerelease version: {}",
                metadata.version
            )
            .into());
        }
        _ => {}
    }
    if metadata.notes.chars().count() > MAX_NOTES_CHARS || metadata.notes.contains('\0') {
        return Err("updater notes are empty-safe but must be bounded and NUL-free".into());
    }
    if !valid_utc_timestamp(&metadata.pub_date) {
        return Err(format!(
            "pub_date must be a second-precision RFC3339 UTC timestamp: {}",
            metadata.pub_date
        )
        .into());
    }
    if metadata.platforms.len() != 1 || !metadata.platforms.contains_key(WINDOWS_PLATFORM) {
        return Err(format!(
            "metadata must contain exactly the canonical {WINDOWS_PLATFORM} platform"
        )
        .into());
    }
    let platform = metadata
        .platforms
        .get(WINDOWS_PLATFORM)
        .expect("platform presence checked above");
    if !valid_signature(&platform.signature) {
        return Err("Tauri updater signature must be non-empty base64 text".into());
    }
    let expected_asset_url = canonical_asset_url(&metadata.version);
    if platform.url != expected_asset_url {
        return Err(format!(
            "asset URL must be the exact immutable repository URL {expected_asset_url}"
        )
        .into());
    }
    let parsed_url = Url::parse(&platform.url)?;
    if parsed_url.scheme() != "https"
        || parsed_url.host_str() != Some("github.com")
        || parsed_url.username() != ""
        || parsed_url.password().is_some()
        || parsed_url.query().is_some()
        || parsed_url.fragment().is_some()
    {
        return Err(
            "asset URL must be an HTTPS GitHub release URL without credentials or query state"
                .into(),
        );
    }
    Ok(())
}

fn validate_roll_forward(
    channel: Channel,
    current: Option<&TauriMetadata>,
    candidate: &TauriMetadata,
) -> Result<()> {
    validate_metadata(candidate, channel)?;
    if let Some(current) = current {
        validate_metadata(current, channel)?;
        let current_version = version::parse(&current.version)?;
        let candidate_version = version::parse(&candidate.version)?;
        if candidate_version <= current_version {
            return Err(format!(
                "{} metadata promotion must strictly increase SemVer: current {}, candidate {}",
                channel_name(channel),
                current.version,
                candidate.version
            )
            .into());
        }
    }
    Ok(())
}

fn normalise_notes(value: &str) -> Result<String> {
    let normalised = value.replace("\r\n", "\n");
    let normalised = normalised.trim_end_matches('\n').to_owned();
    if normalised.chars().count() > MAX_NOTES_CHARS || normalised.contains('\0') {
        return Err("release notes exceed the bounded metadata input contract".into());
    }
    Ok(normalised)
}

fn normalise_signature(value: &str) -> Result<String> {
    let signature = value.trim();
    if !valid_signature(signature) {
        return Err("Tauri .sig input must be non-empty base64 text".into());
    }
    Ok(signature.to_owned())
}

fn valid_signature(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_SIGNATURE_CHARS
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=')
        })
}

fn valid_utc_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return false;
    }
    if !bytes
        .iter()
        .enumerate()
        .all(|(index, value)| matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || value.is_ascii_digit())
    {
        return false;
    }
    let number = |start: usize, end: usize| {
        value[start..end]
            .parse::<u32>()
            .expect("digit positions were checked")
    };
    let year = number(0, 4);
    let month = number(5, 7);
    let day = number(8, 10);
    let hour = number(11, 13);
    let minute = number(14, 16);
    let second = number(17, 19);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => 0,
    };
    (1..=12).contains(&month)
        && (1..=days_in_month).contains(&day)
        && hour <= 23
        && minute <= 59
        && second <= 59
}

fn channel_name(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => "stable",
        Channel::Beta => "beta",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(version: &str, url: &str) -> TauriMetadata {
        TauriMetadata {
            version: version.into(),
            notes: "qualified candidate".into(),
            pub_date: "2026-09-04T00:00:00Z".into(),
            platforms: BTreeMap::from([(
                WINDOWS_PLATFORM.into(),
                PlatformMetadata {
                    signature: "c2lnbmF0dXJl".into(),
                    url: url.into(),
                },
            )]),
        }
    }

    #[test]
    fn canonical_metadata_is_deterministic_and_uses_the_real_tauri_name() {
        let url = canonical_asset_url("4.0.0-beta.1");
        assert_eq!(
            url,
            "https://github.com/pumni/Sky-Auto-Player/releases/download/v4.0.0-beta.1/Sky.Auto.Player_4.0.0-beta.1_x64-setup.exe"
        );
        let first = serde_json::to_string_pretty(&metadata("4.0.0-beta.1", &url)).unwrap();
        let second = serde_json::to_string_pretty(&metadata("4.0.0-beta.1", &url)).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn generator_normalises_inputs_and_emits_valid_static_metadata() {
        let root =
            std::env::temp_dir().join(format!("sky-v4-release-metadata-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let notes = root.join("notes.md");
        let signature = root.join("installer.exe.sig");
        let output = root.join("latest.json");
        fs::write(&notes, "qualified candidate\r\n").unwrap();
        fs::write(&signature, "c2lnbmF0dXJl\n").unwrap();
        let asset_url = canonical_asset_url("4.0.0-beta.1");
        let input = GenerateInput {
            channel: Channel::Beta,
            version: "4.0.0-beta.1",
            notes_path: &notes,
            pub_date: "2026-09-04T00:00:00Z",
            platform: WINDOWS_PLATFORM,
            asset_url: &asset_url,
            signature_path: &signature,
            output: &output,
        };
        generate(input).unwrap();
        let first = fs::read(&output).unwrap();
        validate(Channel::Beta, &output).unwrap();
        generate(input).unwrap();
        assert_eq!(first, fs::read(&output).unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stable_and_beta_destinations_are_structurally_distinct() {
        assert_ne!(
            Channel::Stable.metadata_path(),
            Channel::Beta.metadata_path()
        );
        assert!(Channel::Stable.metadata_path().contains("channels/stable/"));
        assert!(Channel::Beta.metadata_path().contains("channels/beta/"));
    }

    #[test]
    fn validator_rejects_noncanonical_assets() {
        let valid = canonical_asset_url("4.0.0");
        assert!(validate_metadata(&metadata("4.0.0", &valid), Channel::Stable).is_ok());
        for url in [
            valid.replace(RELEASE_REPOSITORY, "pumni/Other-Repository"),
            valid.replace("https://", "http://"),
            valid.replace("_x64-setup.exe", "_x64-setup.exe?channel=beta"),
            valid.replace("Sky.Auto.Player", "Sky-Auto-Player-v4"),
        ] {
            assert!(
                validate_metadata(&metadata("4.0.0", &url), Channel::Stable).is_err(),
                "{url}"
            );
        }
    }

    #[test]
    fn validator_rejects_build_metadata_bad_dates_and_bad_signatures() {
        let url = canonical_asset_url("4.0.0");
        for version_value in ["4.0.0+build", "3.9.9", "v4.0.0"] {
            assert!(validate_metadata(&metadata(version_value, &url), Channel::Stable).is_err());
        }
        let mut bad_date = metadata("4.0.0", &url);
        bad_date.pub_date = "2026-09-04T00:00:00+00:00".into();
        assert!(validate_metadata(&bad_date, Channel::Stable).is_err());
        let mut bad_signature = metadata("4.0.0", &url);
        bad_signature
            .platforms
            .get_mut(WINDOWS_PLATFORM)
            .unwrap()
            .signature = "signature-path.sig".into();
        assert!(validate_metadata(&bad_signature, Channel::Stable).is_err());
    }

    #[test]
    fn channel_policy_keeps_stable_and_beta_candidates_isolated() {
        let stable_url = canonical_asset_url("4.0.0");
        assert!(validate_metadata(&metadata("4.0.0", &stable_url), Channel::Stable).is_ok());
        for prerelease in ["4.0.0-beta.1", "4.0.0-rc.1"] {
            let url = canonical_asset_url(prerelease);
            assert!(
                validate_metadata(&metadata(prerelease, &url), Channel::Stable).is_err(),
                "stable accepted {prerelease}"
            );
            assert!(
                validate_metadata(&metadata(prerelease, &url), Channel::Beta).is_ok(),
                "beta rejected {prerelease}"
            );
        }
        assert!(validate_metadata(&metadata("4.0.0", &stable_url), Channel::Beta).is_err());
    }

    #[test]
    fn first_promotion_allows_a_valid_candidate_without_current_metadata() {
        let candidate_url = canonical_asset_url("4.0.0-beta.1");
        assert!(
            validate_roll_forward(
                Channel::Beta,
                None,
                &metadata("4.0.0-beta.1", &candidate_url)
            )
            .is_ok()
        );
    }

    #[test]
    fn promotion_requires_a_strict_semver_upgrade() {
        let current_url = canonical_asset_url("4.0.0-beta.1");
        let current = metadata("4.0.0-beta.1", &current_url);
        for candidate_version in ["4.0.0-beta.1", "4.0.0-alpha.9"] {
            let candidate_url = canonical_asset_url(candidate_version);
            assert!(
                validate_roll_forward(
                    Channel::Beta,
                    Some(&current),
                    &metadata(candidate_version, &candidate_url)
                )
                .is_err(),
                "accepted non-increasing candidate {candidate_version}"
            );
        }
    }

    #[test]
    fn promotion_accepts_a_strict_semver_upgrade() {
        let current_url = canonical_asset_url("4.0.0-beta.1");
        let candidate_url = canonical_asset_url("4.0.0-beta.2");
        assert!(
            validate_roll_forward(
                Channel::Beta,
                Some(&metadata("4.0.0-beta.1", &current_url)),
                &metadata("4.0.0-beta.2", &candidate_url)
            )
            .is_ok()
        );
    }

    #[test]
    fn promotion_rejects_malformed_current_metadata() {
        let current_url = canonical_asset_url("4.0.0-beta.1");
        let mut current = metadata("4.0.0-beta.1", &current_url);
        current.platforms.clear();
        let candidate_url = canonical_asset_url("4.0.0-beta.2");
        assert!(
            validate_roll_forward(
                Channel::Beta,
                Some(&current),
                &metadata("4.0.0-beta.2", &candidate_url)
            )
            .is_err()
        );
    }
}
