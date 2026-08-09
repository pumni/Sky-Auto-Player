use std::str::FromStr;

use pep440_rs::Version;

use crate::error::{Result, UpdaterError};

/// PEP 440 ordering delegated to the maintained `pep440_rs` implementation.
///
/// The updater deliberately rejects the GitHub tag conveniences (`v` prefixes
/// and surrounding whitespace) before handing the value to the library. This
/// keeps the CLI contract identical to the Python update selector while using
/// the same ordering rules for epochs, release padding, pre/dev/post and local
/// versions.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct Pep440Version(Version);

impl Pep440Version {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.is_empty() || raw.trim() != raw || raw.starts_with(['v', 'V']) {
            return Err(UpdaterError::InvalidArgument(format!(
                "invalid PEP 440 version: {raw:?}"
            )));
        }
        Version::from_str(raw).map(Self).map_err(|error| {
            UpdaterError::InvalidArgument(format!("invalid PEP 440 version: {error}"))
        })
    }

    pub fn is_prerelease(&self) -> bool {
        self.0.any_prerelease()
    }
}

pub fn require_upgrade(current: &str, target: &str) -> Result<()> {
    let current = Pep440Version::parse(current)?;
    let target = Pep440Version::parse(target)?;
    if target <= current {
        return Err(UpdaterError::ReleasePolicyRejected(
            "target version must be greater than current version".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_common_pep440_spellings() {
        for value in [
            "3.2.0rc1",
            "3.2.0-rc1",
            "3.2.0.rc1",
            "3.2.0.dev1",
            "1!3.2.0.post0+build.7",
        ] {
            assert!(Pep440Version::parse(value).is_ok(), "{value}");
        }
    }

    #[test]
    fn matches_post_zero_and_release_padding_ordering() {
        assert!(Pep440Version::parse("1.0").unwrap() < Pep440Version::parse("1.0.post0").unwrap());
        assert_eq!(
            Pep440Version::parse("1.0").unwrap(),
            Pep440Version::parse("1.0.0").unwrap()
        );
    }

    #[test]
    fn compares_dev_prerelease_and_final_ordering() {
        assert!(Pep440Version::parse("1.0.dev0").unwrap() < Pep440Version::parse("1.0a0").unwrap());
        assert!(Pep440Version::parse("1.0rc1").unwrap() < Pep440Version::parse("1.0").unwrap());
        assert!(
            Pep440Version::parse("1.0+abc").unwrap() < Pep440Version::parse("1.0+def").unwrap()
        );
    }

    #[test]
    fn rejects_whitespace_and_v_prefixes() {
        assert!(Pep440Version::parse(" 3.2.0").is_err());
        assert!(Pep440Version::parse("3.2.0 ").is_err());
        assert!(Pep440Version::parse("v3.2.0").is_err());
    }
}
