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

pub fn check(tag: Option<&str>) -> Result<()> {
    let root = repo::root();
    let version = repo::project_version(&root)?;
    let parsed = parse(&version)?;
    if let Some(tag) = tag {
        check_tag(tag, &version, &parsed)?;
    }
    println!("version={version}");
    println!("is_prerelease={}", !parsed.pre.is_empty());
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
}
