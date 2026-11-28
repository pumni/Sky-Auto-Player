use crate::{Result, repo};
use semver::Version;

pub fn parse(value: &str) -> Result<Version> {
    if value.is_empty() || value.trim() != value || value.starts_with(['v', 'V']) {
        return Err(format!("invalid SemVer version: {value:?}").into());
    }
    let parsed =
        Version::parse(value).map_err(|error| format!("invalid SemVer version: {error}"))?;
    if !parsed.build.is_empty() {
        return Err(format!("SemVer build metadata is not allowed: {value:?}").into());
    }
    Ok(parsed)
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct VersionCheckOptions<'a> {
    pub version: Option<&'a str>,
    pub channel: Option<&'a str>,
    pub tag: Option<&'a str>,
    pub no_repo_match: bool,
}

pub fn check(options: VersionCheckOptions<'_>) -> Result<()> {
    let root = repo::root();
    let (effective_version, parsed) = if let Some(v) = options.version {
        let parsed = parse(v)?;
        if !options.no_repo_match {
            let cargo_version = repo::project_version(&root)?;
            if v != cargo_version {
                return Err(format!(
                    "Specified version '{v}' does not match Cargo.toml version '{cargo_version}'"
                )
                .into());
            }
        }
        (v.to_owned(), parsed)
    } else {
        let version = repo::project_version(&root)?;
        let parsed = parse(&version)?;
        (version, parsed)
    };

    if let Some(channel) = options.channel {
        validate_channel(channel, &effective_version, &parsed)?;
    }

    if let Some(tag) = options.tag {
        check_tag(tag, &effective_version, &parsed)?;
    }

    println!("version={effective_version}");
    println!("is_prerelease={}", !parsed.pre.is_empty());
    if let Some(channel) = options.channel {
        println!("channel={channel}");
    }
    Ok(())
}

pub fn validate_channel(channel: &str, version: &str, parsed: &Version) -> Result<()> {
    match channel {
        "stable" => {
            if !parsed.pre.is_empty() {
                return Err(format!(
                    "Channel 'stable' rejects prerelease version '{version}' (SemVer without hyphen required)"
                )
                .into());
            }
        }
        "beta" => {
            if parsed.pre.is_empty() {
                return Err(format!(
                    "Channel 'beta' requires a prerelease SemVer version (e.g. '{version}-beta.1')"
                )
                .into());
            }
        }
        _ => {
            return Err(format!(
                "Missing or invalid channel: '{channel}' (must be 'stable' or 'beta')"
            )
            .into());
        }
    }
    Ok(())
}

fn check_tag(tag: &str, version: &str, parsed: &Version) -> Result<()> {
    let expected = format!("v{version}");
    if tag != expected {
        return Err(format!("tag {tag:?} does not exactly match Cargo version {version:?}").into());
    }
    let tag_version = tag
        .strip_prefix('v')
        .ok_or("release tag must start with lowercase v")?;
    let tag_parsed = parse(tag_version)?;
    if &tag_parsed != parsed {
        return Err(format!("tag {tag:?} does not match Cargo version {version:?}").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_v4_semver_prerelease_corpus() {
        let values = [
            ("4.0.0-alpha.1", true),
            ("4.0.0-beta.1", true),
            ("4.0.0-rc.1", true),
            ("4.0.0", false),
        ];
        for (value, expected) in values {
            assert_eq!(!parse(value).unwrap().pre.is_empty(), expected, "{value}");
        }
    }

    #[test]
    fn rejects_pep440_and_untrusted_forms() {
        for value in [
            " 4.0.0",
            "v4.0.0",
            "4.0.0rc1",
            "4.0",
            "4.0.0+local",
            "4.0.0-alpha.1+build",
        ] {
            assert!(parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn release_tag_rejects_semver_build_metadata() {
        for version in ["4.0.0+local", "4.0.0-alpha.1+build"] {
            let parsed = Version::parse(version).unwrap();
            assert!(parse(version).is_err(), "Cargo version: {version}");
            assert!(
                check_tag(&format!("v{version}"), version, &parsed).is_err(),
                "release tag: v{version}"
            );
        }
    }

    #[test]
    fn release_tag_requires_exact_cargo_text() {
        let version = "4.0.0-alpha.1";
        let parsed = parse(version).unwrap();
        assert!(check_tag("v4.0.0-alpha.1", version, &parsed).is_ok());
        for tag in [
            "4.0.0-alpha.1",
            "V4.0.0-alpha.1",
            "v4.0.0",
            "v4.0.00-alpha.1",
            "v4.0.0-alpha1",
            "v4.0.0rc1",
        ] {
            assert!(check_tag(tag, version, &parsed).is_err(), "{tag}");
        }
    }

    #[test]
    fn release_tag_keeps_exact_semver_prerelease_text() {
        for version in ["4.0.0-alpha.1", "4.0.0-beta.1", "4.0.0-rc.1"] {
            let parsed = parse(version).unwrap();
            assert!(check_tag(&format!("v{version}"), version, &parsed).is_ok());
        }
    }

    #[test]
    fn channel_policy_enforces_stable_and_beta_invariants() {
        let stable_version = "4.0.0";
        let parsed_stable = parse(stable_version).unwrap();
        assert!(validate_channel("stable", stable_version, &parsed_stable).is_ok());
        let err = validate_channel("beta", stable_version, &parsed_stable).unwrap_err();
        assert!(
            err.to_string()
                .contains("Channel 'beta' requires a prerelease SemVer version")
        );

        let beta_version = "4.0.0-beta.1";
        let parsed_beta = parse(beta_version).unwrap();
        assert!(validate_channel("beta", beta_version, &parsed_beta).is_ok());
        let err = validate_channel("stable", beta_version, &parsed_beta).unwrap_err();
        assert!(
            err.to_string()
                .contains("Channel 'stable' rejects prerelease version")
        );

        let err = validate_channel("nightly", stable_version, &parsed_stable).unwrap_err();
        assert!(err.to_string().contains("Missing or invalid channel"));
    }

    #[test]
    fn version_check_standalone_no_repo_match() {
        assert!(
            check(VersionCheckOptions {
                version: Some("9.9.9"),
                channel: Some("stable"),
                tag: None,
                no_repo_match: true,
            })
            .is_ok()
        );

        assert!(
            check(VersionCheckOptions {
                version: Some("9.9.9-beta.2"),
                channel: Some("beta"),
                tag: Some("v9.9.9-beta.2"),
                no_repo_match: true,
            })
            .is_ok()
        );
    }
}
