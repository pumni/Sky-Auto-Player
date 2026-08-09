use std::cmp::Ordering;

use crate::error::{Result, UpdaterError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Pep440Version {
    release: Vec<u64>,
    pre: Option<(PreKind, u64)>,
    post: Option<u64>,
    dev: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
enum PreKind {
    Alpha,
    Beta,
    Rc,
}

impl Pep440Version {
    pub fn parse(raw: &str) -> Result<Self> {
        let value = raw.trim();
        if value.is_empty() || value != raw || value.starts_with('v') || value.starts_with('V') {
            return Err(UpdaterError::InvalidArgument(format!(
                "invalid PEP 440 version: {raw:?}"
            )));
        }
        let (without_local, local) = value
            .split_once('+')
            .map_or((value, None), |(a, b)| (a, Some(b)));
        if local.is_some_and(|part| part.is_empty()) {
            return Err(UpdaterError::InvalidArgument("empty local version".into()));
        }
        let mut core = without_local;
        let dev = parse_suffix_number(&mut core, ".dev", "dev")?;
        let post = parse_suffix_number(&mut core, ".post", "post")?;
        let pre = parse_pre(&mut core)?;
        let release = core
            .replace(['_', '-'], ".")
            .split('.')
            .map(|part| {
                if part.is_empty() {
                    return Err(UpdaterError::InvalidArgument(format!(
                        "invalid release segment in {raw:?}"
                    )));
                }
                part.parse::<u64>().map_err(|_| {
                    UpdaterError::InvalidArgument(format!("invalid release segment in {raw:?}"))
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if release.is_empty() {
            return Err(UpdaterError::InvalidArgument(format!(
                "missing release segment in {raw:?}"
            )));
        }
        Ok(Self {
            release,
            pre,
            post,
            dev,
        })
    }

    pub fn is_prerelease(&self) -> bool {
        self.pre.is_some() || self.dev.is_some()
    }
}

fn parse_suffix_number(core: &mut &str, marker: &str, label: &str) -> Result<Option<u64>> {
    if let Some((before, suffix)) = core.rsplit_once(marker) {
        if suffix.is_empty() || !suffix.chars().all(|ch| ch.is_ascii_digit()) {
            return Err(UpdaterError::InvalidArgument(format!(
                "invalid {label} version"
            )));
        }
        *core = before;
        return suffix
            .parse()
            .map(Some)
            .map_err(|_| UpdaterError::InvalidArgument(format!("invalid {label} version")));
    }
    Ok(None)
}

fn parse_pre(core: &mut &str) -> Result<Option<(PreKind, u64)>> {
    let mut found: Option<(usize, PreKind, &str)> = None;
    for (marker, kind) in [
        ("rc", PreKind::Rc),
        ("beta", PreKind::Beta),
        ("b", PreKind::Beta),
        ("alpha", PreKind::Alpha),
        ("a", PreKind::Alpha),
    ] {
        if let Some(index) = core.rfind(marker)
            && found.is_none_or(|old| index > old.0)
        {
            found = Some((index, kind, marker));
        }
    }
    let Some((index, kind, marker)) = found else {
        return Ok(None);
    };
    let suffix = &core[index + marker.len()..];
    let suffix = suffix.strip_prefix('.').unwrap_or(suffix);
    let number = if suffix.is_empty() {
        0
    } else {
        suffix
            .parse()
            .map_err(|_| UpdaterError::InvalidArgument("invalid prerelease version".into()))?
    };
    *core = core[..index].trim_end_matches(['.', '-', '_']);
    Ok(Some((kind, number)))
}

impl Ord for Pep440Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let release_len = self.release.len().max(other.release.len());
        for index in 0..release_len {
            let left = self.release.get(index).copied().unwrap_or(0);
            let right = other.release.get(index).copied().unwrap_or(0);
            match left.cmp(&right) {
                Ordering::Equal => {}
                order => return order,
            }
        }
        match (self.pre, other.pre) {
            (Some(left), Some(right)) => match left.cmp(&right) {
                Ordering::Equal => {}
                order => return order,
            },
            (Some(_), None) if other.dev.is_none() => return Ordering::Less,
            (None, Some(_)) if self.dev.is_none() => return Ordering::Greater,
            _ => {}
        }
        match (self.dev, other.dev) {
            (Some(left), Some(right)) => match left.cmp(&right) {
                Ordering::Equal => {}
                order => return order,
            },
            (Some(_), None) => return Ordering::Less,
            (None, Some(_)) => return Ordering::Greater,
            _ => {}
        }
        self.post.unwrap_or(0).cmp(&other.post.unwrap_or(0))
    }
}

impl PartialOrd for Pep440Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
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
    fn accepts_common_pep440_prerelease_spellings() {
        for value in ["3.2.0rc1", "3.2.0-rc1", "3.2.0.rc1", "3.2.0.dev1"] {
            assert!(Pep440Version::parse(value).is_ok(), "{value}");
        }
    }

    #[test]
    fn compares_release_candidates_before_final() {
        assert!(Pep440Version::parse("3.2.0rc1").unwrap() < Pep440Version::parse("3.2.0").unwrap());
        assert!(
            Pep440Version::parse("3.2.0").unwrap() < Pep440Version::parse("3.2.0.post1").unwrap()
        );
    }

    #[test]
    fn rejects_whitespace_and_v_prefixes() {
        assert!(Pep440Version::parse(" 3.2.0").is_err());
        assert!(Pep440Version::parse("v3.2.0").is_err());
    }
}
