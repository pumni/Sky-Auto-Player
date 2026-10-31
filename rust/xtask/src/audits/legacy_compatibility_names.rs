use crate::Result;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const SCAN_ROOTS: &[&str] = &[
    "rust/crates",
    "rust/xtask",
    "desktop/src",
    "desktop/src-tauri",
    "site/src",
    "tests",
    ".github/workflows",
    ".github/actions",
    "docs",
];

const ROOT_FILES: &[&str] = &["AGENTS.md", "CONTRIBUTING.md", "README.md", "SECURITY.md"];

const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    ".astro",
    ".turbo",
    ".cache",
];

const TEXT_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "ps1", "psm1", "psd1", "yml", "yaml", "toml",
    "json", "md", "mdx", "astro", "html", "sh", "bat", "cmd",
];

// These records describe historical source and evidence. Keep their exact
// wording and fixture paths intact; active-content checks scan all other docs.
const HISTORICAL_PATHS: &[&str] = &["CHANGELOG.md", "docs/maintenance-test-pipeline-report.md"];
const HISTORICAL_PREFIXES: &[&str] = &["docs/adr/", "docs/releases/", "docs/evidence/"];

// Build obsolete identifiers from fragments so this audit can scan its own
// source without matching the terms it is responsible for detecting.
const RETIRED_TERMS: &[&str] = &[
    concat!("python", "_"),
    concat!("python", " oracle"),
    concat!("python", " core"),
    concat!("python", " desktop ipc"),
    concat!("python", " fixture"),
    concat!("python", " contract"),
    concat!("python", "'s round"),
    concat!("python", " loader"),
    concat!("python", " analyzer"),
    concat!("python", " supervisor"),
    concat!("python", " api"),
    concat!("python", " types"),
    concat!("python", " boundary"),
    concat!("python", "-compatible"),
    concat!("python", "-fixture"),
    concat!("oracle", "_cases.json"),
    concat!("legacy", "_float"),
];

fn is_text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| TEXT_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}

fn is_historical_exception(relative: &str) -> bool {
    HISTORICAL_PATHS.contains(&relative)
        || HISTORICAL_PREFIXES
            .iter()
            .any(|prefix| relative.starts_with(prefix))
}

fn retired_term_in(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    RETIRED_TERMS
        .iter()
        .copied()
        .find(|term| lower.contains(&term.to_ascii_lowercase()))
}

fn scan_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = ROOT_FILES
        .iter()
        .map(|file| root.join(file))
        .collect::<Vec<_>>();
    for scan_root in SCAN_ROOTS {
        let base = root.join(scan_root);
        if !base.is_dir() {
            continue;
        }
        paths.extend(
            WalkDir::new(base)
                .follow_links(false)
                .into_iter()
                .filter_entry(|entry| {
                    !entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| IGNORED_DIRS.contains(&name))
                })
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry.file_type().is_file() && is_text_file(entry.path()))
                .map(|entry| entry.into_path()),
        );
    }
    paths.sort();
    paths.dedup();
    paths
}

pub(crate) fn run(root: &Path) -> Result<()> {
    let mut violations = Vec::new();
    for path in scan_paths(root) {
        if !path.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if is_historical_exception(&relative) || !is_text_file(&path) {
            continue;
        }
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
        };
        for (line, text) in contents.lines().enumerate() {
            if let Some(term) = retired_term_in(text) {
                violations.push(format!(
                    "{relative}:{}: retired compatibility name `{term}`",
                    line + 1
                ));
            }
        }
    }
    if !violations.is_empty() {
        return Err(format!(
            "legacy compatibility name audit failed:\n{}",
            violations.join("\n")
        )
        .into());
    }
    println!("[xtask] legacy compatibility names: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_retired_names_but_allows_security_prohibitions() {
        for term in [
            format!("{}{}", "python", "_string"),
            format!("{}{}", "python", " oracle"),
            format!("{}{}", "python", "'s round"),
            format!("{}{}", "oracle", "_cases.json"),
            format!("{}{}", "legacy", "_float"),
        ] {
            assert!(retired_term_in(&format!("old helper: {term}")).is_some());
        }
        assert_eq!(retired_term_in("python-keyboard is forbidden"), None);
        assert_eq!(
            retired_term_in("No separate scripting runtime is required."),
            None
        );
    }

    #[test]
    fn historical_exceptions_are_narrow_and_path_scoped() {
        assert!(is_historical_exception("CHANGELOG.md"));
        assert!(is_historical_exception("docs/adr/ADR-0003.md"));
        assert!(is_historical_exception("docs/releases/v3.0.0.md"));
        assert!(is_historical_exception("docs/evidence/issue-540.json"));
        assert!(is_historical_exception(
            "docs/maintenance-test-pipeline-report.md"
        ));
        assert!(!is_historical_exception("docs/architecture.md"));
        assert!(!is_historical_exception(
            "site/src/content/guides/en/security.md"
        ));
    }

    #[test]
    fn maintained_source_scan_covers_native_desktop_and_website_code() {
        assert!(SCAN_ROOTS.contains(&"desktop/src-tauri"));
        assert!(SCAN_ROOTS.contains(&"site/src"));
    }
}
