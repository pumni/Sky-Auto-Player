use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub const PACKAGE_FILES: &[&str] = &[
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

pub const PACKAGE_WORKFLOW_FILES: &[&str] =
    &[".github/workflows/ci.yml", ".github/workflows/release.yml"];

pub const PACKAGE_PREFIXES: &[&str] = &[
    "desktop/src-tauri/capabilities/",
    "desktop/src-tauri/icons/",
    "rust/crates/sky_updater/",
    "scripts/build_",
    "scripts/test_windows_updater_e2e.ps1",
    "rust/xtask/",
];

pub const CODE_PREFIXES: &[&str] = &["src/", "desktop/", "rust/", "tests/", "scripts/", ".cargo/"];

pub const BROWSER_PREFIXES: &[&str] = &[
    "desktop/src/",
    "desktop/src-tauri/src/commands.rs",
    "desktop/src-tauri/src/ipc_contract.rs",
    "desktop/src-tauri/src/ui_events.rs",
    "desktop/src/bridge/generated/",
];

pub const BROWSER_FILES: &[&str] = &[
    "desktop/index.html",
    "desktop/package.json",
    "desktop/bun.lock",
    "desktop/vite.config.ts",
    "desktop/vite.config.js",
    "desktop/vitest.config.ts",
    "desktop/vitest.config.js",
    "desktop/playwright.config.ts",
    "desktop/playwright.config.js",
    "desktop/scripts/run-e2e.mjs",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub static_required: bool,
    pub code_required: bool,
    pub package_required: bool,
    pub browser_required: bool,
    pub reason: String,
}

impl Classification {
    pub fn new_full() -> Self {
        Self {
            static_required: true,
            code_required: true,
            package_required: true,
            browser_required: true,
            reason: "full validation requested".to_owned(),
        }
    }

    pub fn print(&self) {
        println!("static_required={}", self.static_required);
        println!("code_required={}", self.code_required);
        println!("package_required={}", self.package_required);
        println!("browser_required={}", self.browser_required);
        println!("classification_reason={}", self.reason);
    }
}

pub fn normalize(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_owned()
}

pub fn package_sensitive(path: &str) -> bool {
    PACKAGE_FILES.contains(&path)
        || PACKAGE_WORKFLOW_FILES.contains(&path)
        || PACKAGE_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

pub fn classify(paths: &[String], full: bool) -> (bool, bool, bool, bool, String) {
    if full {
        return (true, true, true, true, "full validation requested".into());
    }
    let paths: Vec<String> = paths
        .iter()
        .map(|path| normalize(path))
        .filter(|p| !p.is_empty())
        .collect();
    if paths.is_empty() {
        return (false, false, false, false, "no changed paths".into());
    }
    let package_required = paths.iter().any(|path| package_sensitive(path));
    let code = paths.iter().any(|path| {
        CODE_PREFIXES.iter().any(|prefix| path.starts_with(prefix))
            || PACKAGE_FILES.contains(&path.as_str())
    }) || package_required;
    let browser_required = paths.iter().any(|path| {
        BROWSER_FILES.contains(&path.as_str())
            || BROWSER_PREFIXES
                .iter()
                .any(|prefix| path.starts_with(prefix))
    });
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
    (true, code, package_required, browser_required, reason)
}

fn find_repo_root() -> Result<PathBuf> {
    let mut current = std::env::current_dir()?;
    loop {
        if current.join(".git").exists() || current.join("rust/Cargo.toml").is_file() {
            return Ok(current);
        }
        if !current.pop() {
            break;
        }
    }
    Ok(std::env::current_dir()?)
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
        let output = Command::new("git")
            .args(["diff", "--name-only", base, head])
            .current_dir(root)
            .output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git diff failed: {stderr}").into());
        }
        let text = String::from_utf8(output.stdout)?;
        return Ok(text.lines().map(str::to_owned).collect());
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
    if full {
        let c = Classification::new_full();
        c.print();
        return Ok(());
    }

    if (base.is_some()) != (head.is_some()) {
        return Err("--base and --head must be supplied together".into());
    }
    let root = find_repo_root()?;
    let paths = changed_paths(&root, base, head, paths_file)?;
    let (static_required, code_required, package_required, browser_required, reason) =
        classify(&paths, false);
    let c = Classification {
        static_required,
        code_required,
        package_required,
        browser_required,
        reason,
    };
    c.print();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(paths: &[&str]) -> (bool, bool, bool, bool, String) {
        let owned: Vec<String> = paths.iter().map(|path| (*path).to_owned()).collect();
        classify(&owned, false)
    }

    #[test]
    fn full_mode_short_circuits_without_paths() {
        let result = classify(&[], true);
        assert!(result.0);
        assert!(result.1);
        assert!(result.2);
        assert!(result.3);
        assert_eq!(result.4, "full validation requested");

        // Calling run with full=true must succeed without stdin or git repository
        assert!(run(true, None, None, None).is_ok());
    }

    #[test]
    fn empty_paths_returns_all_false() {
        let result = values(&[]);
        assert!(!result.0);
        assert!(!result.1);
        assert!(!result.2);
        assert!(!result.3);
        assert_eq!(result.4, "no changed paths");
    }

    #[test]
    fn acceptance_scenario_a_docs_only() {
        let result = values(&["docs/foo.md"]);
        assert!(result.0);
        assert!(!result.1);
        assert!(!result.2);
        assert!(!result.3);
        assert_eq!(result.4, "static/site/docs only");
    }

    #[test]
    fn acceptance_scenario_b_rust_player_code() {
        let result = values(&["rust/crates/sky_player/src/lib.rs"]);
        assert!(result.0);
        assert!(result.1);
        assert!(!result.2);
        assert!(!result.3);
        assert!(result.4.starts_with("code/windows:"));
    }

    #[test]
    fn acceptance_scenario_c_app_core_rust() {
        let result = values(&["rust/crates/sky_app_core/src/settings.rs"]);
        assert!(result.0);
        assert!(result.1);
        assert!(!result.2);
        assert!(!result.3);
        assert!(result.4.starts_with("code/windows:"));
    }

    #[test]
    fn acceptance_scenario_d_desktop_frontend() {
        let result = values(&["desktop/src/App.tsx"]);
        assert!(result.0);
        assert!(result.1);
        assert!(!result.2);
        assert!(result.3);
        assert!(result.4.starts_with("code/windows:"));
    }

    #[test]
    fn acceptance_scenario_e_cargo_lock() {
        let result = values(&["rust/Cargo.lock"]);
        assert!(result.0);
        assert!(result.1);
        assert!(result.2);
        assert!(!result.3);
        assert!(result.4.starts_with("package-sensitive:"));
    }

    #[test]
    fn acceptance_scenario_e_desktop_tauri_manifest() {
        let result = values(&["desktop/src-tauri/Cargo.toml"]);
        assert!(result.0);
        assert!(result.1);
        assert!(result.2);
        assert!(!result.3);
        assert!(result.4.starts_with("package-sensitive:"));
    }

    #[test]
    fn acceptance_scenario_f_ci_workflow() {
        let result = values(&[".github/workflows/ci.yml"]);
        assert!(result.0);
        assert!(result.1);
        assert!(result.2);
        assert!(!result.3);
        assert!(result.4.starts_with("package-sensitive:"));
    }

    #[test]
    fn package_sensitive_changes_require_code_validation() {
        let result = values(&["rust/xtask/src/main.rs"]);
        assert!(result.0);
        assert!(result.1);
        assert!(result.2);
    }

    #[test]
    fn full_and_empty_modes_are_stable() {
        let full = classify(&["docs/foo.md".into()], true);
        assert!(full.0);
        assert!(full.1);
        assert!(full.2);
        assert!(full.3);

        let empty = classify(&[], false);
        assert!(!empty.0);
        assert!(!empty.1);
        assert!(!empty.2);
        assert!(!empty.3);
    }
}
