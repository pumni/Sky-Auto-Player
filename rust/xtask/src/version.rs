use crate::{Result, repo};
use pep440_rs::Version;
use std::str::FromStr;

pub fn parse(value: &str) -> Result<Version> {
    if value.is_empty() || value.trim() != value || value.starts_with(['v', 'V']) {
        return Err(format!("invalid PEP-440 version: {value:?}").into());
    }
    Version::from_str(value).map_err(|error| format!("invalid PEP-440 version: {error}").into())
}

pub fn check(tag: Option<&str>) -> Result<()> {
    let root = repo::root();
    let version = repo::project_version(&root)?;
    let parsed = parse(&version)?;
    if let Some(tag) = tag {
        check_tag(tag, &version, &parsed)?;
    }
    println!("version={version}");
    println!("is_prerelease={}", parsed.any_prerelease());
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
    fn preserves_project_prerelease_corpus() {
        let values = [
            ("3.5.0.dev1", true),
            ("3.5.0a1", true),
            ("3.5.0-alpha1", true),
            ("3.5.0b1", true),
            ("3.5.0-beta1", true),
            ("3.5.0rc1", true),
            ("3.5.0", false),
        ];
        for (value, expected) in values {
            assert_eq!(parse(value).unwrap().any_prerelease(), expected, "{value}");
        }
    }

    #[test]
    fn rejects_untrusted_tag_forms() {
        assert!(parse(" 3.5.0").is_err());
        assert!(parse("v3.5.0").is_err());
    }

    #[test]
    fn release_tag_requires_exact_cargo_text() {
        let version = "3.5.0";
        let parsed = parse(version).unwrap();
        assert!(check_tag("v3.5.0", version, &parsed).is_ok());
        for tag in [
            "3.5.0",
            "V3.5.0",
            "v3.5",
            "v3.5.00",
            "v3.5.0+local",
            "v3.5.0rc1",
        ] {
            assert!(check_tag(tag, version, &parsed).is_err(), "{tag}");
        }
    }

    #[test]
    fn release_tag_keeps_exact_prerelease_text() {
        for version in ["3.5.0.dev1", "3.5.0a1", "3.5.0b1", "3.5.0rc1"] {
            let parsed = parse(version).unwrap();
            assert!(check_tag(&format!("v{version}"), version, &parsed).is_ok());
        }
    }
}
