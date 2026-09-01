use crate::{Result, process, repo};
use std::io::Read;
use std::path::Path;

const PACKAGE_FILES: &[&str] = &[
    ".env.example",
    "windows_version_info.txt",
    "rust/Cargo.toml",
    "rust/Cargo.lock",
    "rust/rust-toolchain.toml",
    ".cargo/config.toml",
    "desktop/src-tauri/Cargo.toml",
    "desktop/src-tauri/build.rs",
    "desktop/bun.lock",
    "desktop/package.json",
    "desktop/src-tauri/tauri.conf.json",
];
const PACKAGE_WORKFLOW_FILES: &[&str] =
    &[".github/workflows/ci.yml", ".github/workflows/release.yml"];
const PACKAGE_PREFIXES: &[&str] = &[
    "desktop/src-tauri/capabilities/",
    "desktop/src-tauri/icons/",
    "rust/crates/sky_updater/",
    "scripts/build_",
    "scripts/test_windows_updater_e2e.ps1",
    "rust/xtask/",
];
const CODE_PREFIXES: &[&str] = &["src/", "desktop/", "rust/", "tests/", "scripts/", ".cargo/"];

fn normalize(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_owned()
}

fn package_sensitive(path: &str) -> bool {
    PACKAGE_FILES.contains(&path)
        || PACKAGE_WORKFLOW_FILES.contains(&path)
        || PACKAGE_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

pub fn classify(paths: &[String], full: bool) -> (bool, bool, bool, String) {
    let paths: Vec<String> = paths
        .iter()
        .map(|path| normalize(path))
        .filter(|p| !p.is_empty())
        .collect();
    if full {
        return (true, true, true, "full validation requested".into());
    }
    if paths.is_empty() {
        return (false, false, false, "no changed paths".into());
    }
    let package_required = paths.iter().any(|path| package_sensitive(path));
    let code = paths.iter().any(|path| {
        CODE_PREFIXES.iter().any(|prefix| path.starts_with(prefix))
            || PACKAGE_FILES.contains(&path.as_str())
    }) || package_required;
    let reason = if package_required {
        let package = paths
            .iter()
            .filter(|path| package_sensitive(path))
            .map(String::as_str)
            .collect::<Vec<_>>();
        format!(
            "package-sensitive: {}",
            package.into_iter().take(3).collect::<Vec<_>>().join(", ")
        )
    } else if code {
        format!(
            "code/windows: {}",
            paths.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        )
    } else {
        "static/site/docs only".into()
    };
    (true, code, package_required, reason)
}

fn changed_paths(
    root: &Path,
    base: Option<&str>,
    head: Option<&str>,
    paths_file: Option<&str>,
) -> Result<Vec<String>> {
    if let Some(file) = paths_file {
        return Ok(std::fs::read_to_string(file)?
            .lines()
            .map(str::to_owned)
            .collect());
    }
    if let (Some(base), Some(head)) = (base, head) {
        let output = process::capture_text("git", &["diff", "--name-only", base, head], root, &[])?;
        return Ok(output.lines().map(str::to_owned).collect());
    }
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    Ok(input.lines().map(str::to_owned).collect())
}

pub fn run(
    full: bool,
    base: Option<&str>,
    head: Option<&str>,
    paths_file: Option<&str>,
) -> Result<()> {
    if (base.is_some()) != (head.is_some()) {
        return Err("--base and --head must be supplied together".into());
    }
    let root = repo::root();
    let paths = changed_paths(&root, base, head, paths_file)?;
    let (static_required, code_required, package_required, reason) = classify(&paths, full);
    println!("static_required={}", static_required);
    println!("code_required={}", code_required);
    println!("package_required={}", package_required);
    println!("classification_reason={reason}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(paths: &[&str]) -> (bool, bool, bool, String) {
        classify(
            &paths.iter().map(|p| (*p).into()).collect::<Vec<_>>(),
            false,
        )
    }

    #[test]
    fn classifies_docs_without_expensive_jobs() {
        assert_eq!(
            values(&["docs/guide.md"]),
            (true, false, false, "static/site/docs only".into())
        );
    }

    #[test]
    fn classifies_native_and_package_changes() {
        let result = values(&["rust/crates/sky_player/src/lib.rs"]);
        assert!(result.0 && result.1 && !result.2);
        assert!(values(&["rust/xtask/src/main.rs"]).1);
        assert!(values(&["rust/xtask/src/main.rs"]).2);
    }

    #[test]
    fn package_sensitive_changes_require_code_validation() {
        let result = values(&[".github/workflows/ci.yml"]);
        assert_eq!(result.0, true);
        assert_eq!(result.1, true);
        assert_eq!(result.2, true);
    }

    #[test]
    fn full_and_empty_modes_are_stable() {
        assert_eq!(
            classify(&[], true),
            (true, true, true, "full validation requested".into())
        );
        assert_eq!(
            values(&[]),
            (false, false, false, "no changed paths".into())
        );
    }
}
