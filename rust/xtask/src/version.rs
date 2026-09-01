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
        let tag_version = tag.strip_prefix('v').unwrap_or(tag);
        if tag_version != version {
            let tag_parsed = parse(tag_version)?;
            if tag_parsed != parsed {
                return Err(format!("tag {tag:?} does not match Cargo version {version:?}").into());
            }
        }
    }
    println!("version={version}");
    println!("is_prerelease={}", parsed.any_prerelease());
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
}
