#![forbid(unsafe_code)]

use crate::{Result, ci_bridge, ci_candidate, ci_classifier};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct CandidateSourceIdentity {
    #[serde(rename = "Version")]
    pub(crate) version: String,
    #[serde(rename = "SourceSha")]
    pub(crate) source_sha: String,
}

fn parse_options(args: &[String]) -> Result<(BTreeMap<String, String>, bool)> {
    let mut options = BTreeMap::new();
    let mut require_worktree_version = false;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        if option == "--require-worktree-version" {
            if require_worktree_version {
                return Err("--require-worktree-version was provided more than once".into());
            }
            require_worktree_version = true;
            index += 1;
            continue;
        }
        let name = option
            .strip_prefix("--")
            .ok_or_else(|| format!("CI option must start with --: {option}"))?;
        if ![
            "repository-root",
            "candidate-version",
            "source-sha",
            "format",
        ]
        .contains(&name)
        {
            return Err(format!("unknown CI option: --{name}").into());
        }
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("missing value for --{name}"))?
            .clone();
        if options.insert(name.to_owned(), value).is_some() {
            return Err(format!("CI option was provided more than once: --{name}").into());
        }
        index += 1;
    }
    Ok((options, require_worktree_version))
}

fn required<'a>(options: &'a BTreeMap<String, String>, name: &str) -> Result<&'a str> {
    options
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("source-identity requires --{name}").into())
}

fn package_version(manifest_source: &str) -> Result<String> {
    let manifest: toml::Value = toml::from_str(manifest_source)
        .map_err(|error| format!("Candidate source manifest is invalid TOML: {error}"))?;
    let version = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .ok_or("Candidate source manifest does not declare one literal package version")?;
    crate::ci_artifacts::validate_semver(version, "Candidate source package version")?;
    Ok(version.to_owned())
}

pub(crate) fn capture_git_text(repository_root: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repository_root)
        .output()
        .map_err(|error| format!("could not run git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!("git {} failed", args.join(" ")).into());
    }
    String::from_utf8(output.stdout).map_err(|error| {
        format!("git {} returned non-UTF-8 output: {error}", args.join(" ")).into()
    })
}

pub(crate) fn candidate_source_identity(
    repository_root: &Path,
    candidate_version: &str,
    source_sha: Option<&str>,
    require_worktree_version: bool,
) -> Result<CandidateSourceIdentity> {
    crate::ci_artifacts::validate_semver(candidate_version, "Candidate source identity version")?;
    let head_sha = capture_git_text(repository_root, &["rev-parse", "--verify", "HEAD"])?;
    let head_sha = crate::ci_artifacts::validate_commit_sha(
        head_sha.trim(),
        "Candidate source identity Git HEAD SHA",
    )?;
    let source_sha = if let Some(source_sha) = source_sha {
        let source_sha = crate::ci_artifacts::validate_commit_sha(
            source_sha,
            "Candidate source identity source SHA",
        )?;
        if source_sha != head_sha {
            return Err(
                "Candidate source identity SHA does not match the checked-out Git HEAD".into(),
            );
        }
        source_sha
    } else {
        head_sha
    };
    let revision_path = format!("{source_sha}:desktop/src-tauri/Cargo.toml");
    let manifest_source = capture_git_text(repository_root, &["show", revision_path.as_str()])
        .map_err(
            |_| "Candidate source identity could not read the desktop manifest from Git HEAD",
        )?;
    let source_version = package_version(&manifest_source)?;
    if candidate_version != source_version {
        return Err(format!(
            "Candidate version {candidate_version} does not match the release manifest at Git HEAD ({source_version})"
        )
        .into());
    }
    if require_worktree_version {
        let current_version = crate::repo::project_version(repository_root)?;
        if candidate_version != current_version {
            return Err(format!(
                "Candidate version {candidate_version} does not match the checked-out release source and lockfile"
            )
            .into());
        }
    }
    Ok(CandidateSourceIdentity {
        version: source_version,
        source_sha,
    })
}

fn source_identity(repository_root: &Path, args: &[String]) -> Result<()> {
    let (options, require_worktree_version) = parse_options(args)?;
    let root = options
        .get("repository-root")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository_root.to_owned());
    let candidate_version = required(&options, "candidate-version")?;
    let source_sha = options.get("source-sha").map(String::as_str);
    let identity = candidate_source_identity(
        &root,
        candidate_version,
        source_sha,
        require_worktree_version,
    )?;
    match options.get("format").map(String::as_str).unwrap_or("text") {
        "text" => println!(
            "[xtask] candidate source identity: PASS (source={}; version={})",
            identity.source_sha, identity.version
        ),
        "json" => println!("{}", serde_json::to_string(&identity)?),
        format => return Err(format!("unsupported source-identity format: {format}").into()),
    }
    Ok(())
}

pub(crate) fn run(repository_root: &Path, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("classify") => ci_classifier::run(&args[1..]),
        Some("candidate") => ci_candidate::run(repository_root, &args[1..]),
        Some("bridge") if args.get(1).map(String::as_str) == Some("build-fixture") => {
            crate::updater_fixture::build_bridge(repository_root, &args[1..])
        }
        Some("bridge") => ci_bridge::run(&args[1..]),
        Some("nsis") => crate::nsis_harness::run(repository_root, &args[1..]),
        Some("updater-fixture") => crate::updater_fixture::run(repository_root, &args[1..]),
        Some("source-identity") => source_identity(repository_root, &args[1..]),
        Some(operation) => Err(format!("unknown CI operation: {operation}").into()),
        None => Err(
            "CI requires classify, candidate, bridge, nsis, updater-fixture, or source-identity"
                .into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_source_identity_from_typed_manifest_and_rejects_malformed_contracts() {
        assert_eq!(
            package_version("[package]\nname = \"fixture\"\nversion = \"4.0.0-beta.1\"\n").unwrap(),
            "4.0.0-beta.1"
        );
        assert!(
            package_version("[package]\nname = \"fixture\"\nversion = \"4.0.0+build\"\n").is_err()
        );
        assert!(package_version("[workspace]\nmembers = []\n").is_err());
    }

    #[test]
    fn explicit_source_identity_requires_exact_checked_out_commit_and_version() {
        let root = crate::repo::root();
        let head = capture_git_text(&root, &["rev-parse", "--verify", "HEAD"]).unwrap();
        let head = head.trim();
        let version = crate::repo::project_version(&root).unwrap();
        let valid = candidate_source_identity(&root, &version, Some(head), true).unwrap();
        assert_eq!(valid.source_sha, head);
        assert!(candidate_source_identity(&root, &version, Some(&"0".repeat(40)), false).is_err());
        assert!(candidate_source_identity(&root, &version, Some(&"a".repeat(40)), false).is_err());
        assert!(candidate_source_identity(&root, "4.0.0+build", Some(head), false).is_err());
    }

    #[test]
    fn source_identity_options_reject_unknown_flags() {
        let args = vec![
            "--candidate-version".to_owned(),
            "4.0.0".to_owned(),
            "--skip-git".to_owned(),
            "true".to_owned(),
        ];
        assert!(
            parse_options(&args)
                .unwrap_err()
                .to_string()
                .contains("unknown CI option")
        );
    }
}
