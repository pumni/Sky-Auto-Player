use crate::Result;
use std::ffi::OsStr;
use std::fmt::Display;
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
const HISTORICAL_PREFIXES: &[&str] = &["docs/adr", "docs/releases", "docs/evidence"];

// Build obsolete identifiers from fragments so this audit can scan its own
// source without matching the terms it is responsible for detecting. Keep the
// retired operational strings specific; current security prohibitions remain
// valid maintained content.
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
    concat!("pythonio", "encoding"),
    concat!("uv_cache", "_dir"),
    concat!("uv_link", "_mode"),
    concat!("uv", " sync"),
    concat!("uv", " run"),
    concat!("build", "_app", ".run_smoke_test"),
];

fn is_text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| TEXT_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
}

fn is_root_configuration_example_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let mut has_example_marker = false;
    let mut has_configuration_marker = false;
    for segment in name
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
    {
        has_example_marker |= matches!(segment, "example" | "sample" | "template");
        has_configuration_marker |= matches!(
            segment,
            "env" | "environment" | "config" | "configuration" | "settings"
        );
    }
    has_example_marker && has_configuration_marker
}

fn is_historical_exception(relative: &str) -> bool {
    HISTORICAL_PATHS.contains(&relative)
        || HISTORICAL_PREFIXES.iter().any(|prefix| {
            relative == *prefix
                || relative
                    .strip_prefix(prefix)
                    .is_some_and(|remainder| remainder.starts_with('/'))
        })
}

fn retired_term_in(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    RETIRED_TERMS
        .iter()
        .copied()
        .find(|term| lower.contains(&term.to_ascii_lowercase()))
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn root_file_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for file in ROOT_FILES {
        let path = root.join(file);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_file() => paths.push(path),
            Ok(_) => {
                return Err(format!(
                    "legacy compatibility audit expected `{file}` to be a regular file"
                )
                .into());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(
                    format!("legacy compatibility audit cannot inspect `{file}`: {error}").into(),
                );
            }
        }
    }

    let entries = fs::read_dir(root).map_err(|error| {
        format!(
            "legacy compatibility audit cannot enumerate repository root `{}`: {error}",
            root.display()
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "legacy compatibility audit cannot enumerate repository root `{}`: {error}",
                root.display()
            )
        })?;
        let name = entry.file_name();
        if !is_root_configuration_example_name(&name) {
            continue;
        }
        let relative = name.to_string_lossy();
        let file_type = entry.file_type().map_err(|error| {
            format!("legacy compatibility audit cannot inspect `{relative}`: {error}")
        })?;
        if !file_type.is_file() {
            return Err(format!(
                "legacy compatibility audit cannot inspect `{relative}`: expected a regular file"
            )
            .into());
        }
        paths.push(entry.path());
    }
    Ok(paths)
}

fn scan_root_directory(root: &Path, scan_root: &str) -> Result<Option<PathBuf>> {
    let base = root.join(scan_root);
    match fs::metadata(&base) {
        Ok(metadata) if metadata.is_dir() => return Ok(Some(base)),
        Ok(_) => {
            return Err(format!(
                "legacy compatibility audit cannot traverse `{scan_root}`: expected a directory"
            )
            .into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "legacy compatibility audit cannot inspect `{scan_root}`: {error}"
            )
            .into());
        }
    }

    // An absent optional scan root is fine. An existing non-directory or an
    // inaccessible ancestor is not: it can hide maintained files from the
    // audit and therefore must fail closed.
    let mut ancestor = base.parent();
    while let Some(path) = ancestor {
        if path == root {
            break;
        }
        match fs::metadata(path) {
            Ok(metadata) if metadata.is_dir() => return Ok(None),
            Ok(_) => {
                return Err(format!(
                    "legacy compatibility audit cannot traverse `{scan_root}`: ancestor `{}` is not a directory",
                    relative_path(root, path)
                )
                .into());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                ancestor = path.parent();
            }
            Err(error) => {
                return Err(format!(
                    "legacy compatibility audit cannot inspect `{}`: {error}",
                    relative_path(root, path)
                )
                .into());
            }
        }
    }
    Ok(None)
}

fn collect_walk_paths<I, E>(entries: I) -> Result<Vec<PathBuf>>
where
    I: IntoIterator<Item = std::result::Result<Option<PathBuf>, E>>,
    E: Display,
{
    let mut paths = Vec::new();
    for entry in entries {
        if let Some(path) = entry
            .map_err(|error| format!("legacy compatibility audit traversal failed: {error}"))?
        {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn scan_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = root_file_paths(root)?;
    for scan_root in SCAN_ROOTS {
        let Some(base) = scan_root_directory(root, scan_root)? else {
            continue;
        };
        let walker = WalkDir::new(base)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| {
                let name = entry.file_name().to_str();
                let relative = relative_path(root, entry.path());
                !name.is_some_and(|name| IGNORED_DIRS.contains(&name))
                    && !is_historical_exception(&relative)
            });
        let entries = walker.map(|entry| match entry {
            Ok(entry) => {
                let path = entry.path();
                let include = entry.file_type().is_file() && is_text_file(path);
                Ok(include.then(|| entry.into_path()))
            }
            Err(error) => {
                let path = error
                    .path()
                    .map(|path| relative_path(root, path))
                    .unwrap_or_else(|| (*scan_root).to_owned());
                Err(format!("{path}: {error}"))
            }
        });
        paths.extend(collect_walk_paths(entries)?);
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn audited_contents(relative: &str, contents: std::io::Result<String>) -> Result<String> {
    contents.map_err(|error| {
        format!("legacy compatibility audit cannot read `{relative}`: {error}").into()
    })
}

pub(crate) fn run(root: &Path) -> Result<()> {
    let mut violations = Vec::new();
    for path in scan_paths(root)? {
        let relative = relative_path(root, &path);
        if is_historical_exception(&relative) {
            continue;
        }
        let contents = audited_contents(&relative, fs::read_to_string(&path))?;
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
    use std::io;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_REPO_ID: AtomicU64 = AtomicU64::new(0);

    struct TempRepo(PathBuf);

    impl TempRepo {
        fn new() -> Self {
            for _ in 0..100 {
                let id = TEMP_REPO_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("sky-xtask-audit-{}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create isolated audit fixture: {error}"),
                }
            }
            panic!("could not allocate isolated audit fixture directory");
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, relative: &str, contents: impl AsRef<[u8]>) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().expect("fixture has parent"))
                .expect("create fixture parent");
            fs::write(path, contents).expect("write isolated audit fixture");
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn detects_retired_names_and_specific_operational_tokens() {
        let obsolete_names = [
            format!("{}{}", "python", "_string"),
            format!("{}{}", "python", " oracle"),
            format!("{}{}", "python", "'s round"),
            format!("{}{}", "oracle", "_cases.json"),
            format!("{}{}", "legacy", "_float"),
            concat!("PYTHONIO", "ENCODING").to_owned(),
            concat!("UV_CACHE", "_DIR").to_owned(),
            concat!("UV_LINK", "_MODE").to_owned(),
            concat!("uv", " sync").to_owned(),
            concat!("uv", " run").to_owned(),
            concat!("build", "_app", ".run_smoke_test").to_owned(),
        ];
        for term in obsolete_names {
            assert!(
                retired_term_in(&term).is_some(),
                "missed retired token {term}"
            );
        }
        assert_eq!(retired_term_in("python-keyboard is forbidden"), None);
        assert_eq!(
            retired_term_in("No separate scripting runtime is required."),
            None
        );
        assert!(!is_root_configuration_example_name(OsStr::new(".env")));
    }

    #[test]
    fn root_environment_and_configuration_examples_are_scanned() {
        let repo = TempRepo::new();
        let env_tokens = [
            concat!("PYTHONIO", "ENCODING"),
            concat!("UV_CACHE", "_DIR"),
            concat!("UV_LINK", "_MODE"),
            concat!("uv", " sync"),
        ];
        repo.write(".env.example", env_tokens.join("\n"));
        repo.write(
            "app.config.example.toml",
            concat!("build", "_app", ".run_smoke_test()"),
        );
        repo.write("preview.env.staging.example", concat!("uv", " run"));

        let error = run(repo.path())
            .expect_err("retired operations in root config examples must fail")
            .to_string()
            .to_ascii_lowercase();
        for (file, token) in [
            (".env.example", concat!("pythonio", "encoding")),
            (".env.example", concat!("uv_cache", "_dir")),
            (".env.example", concat!("uv_link", "_mode")),
            (".env.example", concat!("uv", " sync")),
            (
                "app.config.example.toml",
                concat!("build", "_app", ".run_smoke_test"),
            ),
            ("preview.env.staging.example", concat!("uv", " run")),
        ] {
            assert!(
                error.contains(file),
                "diagnostic omitted path {file}: {error}"
            );
            assert!(
                error.contains(token),
                "diagnostic omitted token {token}: {error}"
            );
        }
    }

    #[test]
    fn traversal_failures_are_returned_with_path_and_cause() {
        let entries = [Err::<Option<PathBuf>, _>(
            "rust/crates/hidden.rs: permission denied".to_owned(),
        )];
        let error = collect_walk_paths(entries)
            .expect_err("walker errors must not be discarded")
            .to_string();
        assert!(error.contains("rust/crates/hidden.rs"));
        assert!(error.contains("permission denied"));

        let repo = TempRepo::new();
        repo.write("docs", "this file blocks the docs scan root");
        let error = scan_paths(repo.path())
            .expect_err("an existing non-directory scan root must fail closed")
            .to_string();
        assert!(error.contains("docs"));
        assert!(error.contains("directory"));
    }

    #[test]
    fn unreadable_and_invalid_utf8_files_fail_closed() {
        let read_error = audited_contents(
            "docs/current.md",
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "synthetic denial",
            )),
        )
        .expect_err("read failures must be reported")
        .to_string();
        assert!(read_error.contains("docs/current.md"));
        assert!(read_error.contains("synthetic denial"));

        let repo = TempRepo::new();
        repo.write(".env.example", [0xff, 0xfe]);
        let utf8_error = run(repo.path())
            .expect_err("invalid UTF-8 in a scanned example must fail")
            .to_string()
            .to_ascii_lowercase();
        assert!(utf8_error.contains(".env.example"));
        assert!(utf8_error.contains("utf-8"));
    }

    #[test]
    fn historical_paths_are_exempt_while_current_docs_are_checked() {
        let repo = TempRepo::new();
        let retired_name = format!("{}{}", "python", "_retired");
        repo.write("docs/adr/ADR-0001.md", &retired_name);
        repo.write("docs/releases/v1.md", &retired_name);
        repo.write("docs/evidence/dated.md", &retired_name);
        repo.write("docs/maintenance-test-pipeline-report.md", &retired_name);
        assert!(run(repo.path()).is_ok(), "historical records stay exempt");

        repo.write("docs/architecture.md", &retired_name);
        let error = run(repo.path())
            .expect_err("active documentation must still be checked")
            .to_string();
        assert!(error.contains("docs/architecture.md"));
    }

    #[test]
    fn current_security_and_toolchain_prohibitions_remain_allowed() {
        let repo = TempRepo::new();
        repo.write(
            "SECURITY.md",
            "`python-keyboard`, `pynput`, and keyboard injection are forbidden.",
        );
        repo.write(
            "AGENTS.md",
            "Do not ship a Python product runtime or toolchain.",
        );
        assert!(run(repo.path()).is_ok());
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
