use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub const CODE_FILES: &[&str] = &["Cargo.toml", "Cargo.lock", ".github/workflows/ci.yml"];

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
    "scripts/sign_v4_authenticode.ps1",
    "scripts/verify_v4_authenticode.ps1",
    "scripts/v4_authenticode_crypto.ps1",
    "scripts/setup_v4_test_signing.ps1",
    "scripts/cleanup_v4_test_signing.ps1",
    "scripts/test_v4_authenticode_integrity.ps1",
    "scripts/test_v4_production_signing_contract.ps1",
];

pub const PACKAGE_PREFIXES: &[&str] = &[
    "desktop/",
    "desktop/src-tauri/capabilities/",
    "desktop/src-tauri/icons/",
    "scripts/build_",
];

pub const CODE_PREFIXES: &[&str] = &["src/", "desktop/", "rust/", "tests/", "scripts/", ".cargo/"];

pub const UPDATER_FILES: &[&str] = &[
    "desktop/src-tauri/src/native_update.rs",
    "desktop/src-tauri/tauri.conf.json",
    "rust/xtask/src/release_metadata.rs",
    "rust/xtask/src/tauri_bundle.rs",
    "scripts/promote_v4_metadata.ps1",
];

pub const UPDATER_PREFIXES: &[&str] = &[
    "scripts/ci_tauri_update_e2e",
    "scripts/test_v4_updater_",
    "scripts/verify_v4_updater_",
];

pub const RELEASE_FILES: &[&str] = &[
    ".github/workflows/release-v4.yml",
    "rust/xtask/src/release_metadata.rs",
    "scripts/ci_v4_release_latest_guard.ps1",
    "scripts/promote_v4_metadata.ps1",
    "scripts/v4_release_pipeline.ps1",
    "scripts/orchestrate_v4_production_release.ps1",
    "scripts/test_v4_production_orchestrator.ps1",
    "scripts/verify_v4_release_runner.ps1",
    "scripts/cleanup_v4_release_state.ps1",
    "scripts/test_v4_release_pipeline.ps1",
    "scripts/v4_updater_credential_broker.ps1",
    "scripts/set_v4_updater_session_credential.ps1",
    "scripts/remove_v4_updater_session_credential.ps1",
];

pub const RELEASE_PREFIXES: &[&str] = &["scripts/v4_release_", "scripts/test_v4_release_"];

pub const SUPPLY_CHAIN_FILES: &[&str] = &[
    "Cargo.toml",
    "rust/Cargo.toml",
    "rust/Cargo.lock",
    "desktop/src-tauri/Cargo.toml",
    "desktop/package.json",
    "desktop/bun.lock",
    ".cargo/config.toml",
    "rust/xtask/src/supply_chain.rs",
    ".github/workflows/ci.yml",
];

pub const SUPPLY_CHAIN_PREFIXES: &[&str] = &["rust/supply-chain/"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub code_required: bool,
    pub package_required: bool,
    pub updater_required: bool,
    pub release_required: bool,
    pub supply_chain_required: bool,
    pub reason: String,
}

impl Classification {
    pub fn new_full() -> Self {
        Self {
            code_required: true,
            package_required: true,
            updater_required: true,
            release_required: true,
            supply_chain_required: true,
            reason: "full validation requested".to_owned(),
        }
    }

    pub fn print(&self) {
        println!("code_required={}", self.code_required);
        println!("package_required={}", self.package_required);
        println!("updater_required={}", self.updater_required);
        println!("release_required={}", self.release_required);
        println!("supply_chain_required={}", self.supply_chain_required);
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
        || PACKAGE_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

pub fn updater_sensitive(path: &str) -> bool {
    UPDATER_FILES.contains(&path)
        || UPDATER_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

pub fn release_sensitive(path: &str) -> bool {
    RELEASE_FILES.contains(&path)
        || RELEASE_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

pub fn supply_chain_sensitive(path: &str) -> bool {
    SUPPLY_CHAIN_FILES.contains(&path)
        || SUPPLY_CHAIN_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
}

pub fn classify(paths: &[String], full: bool) -> (bool, bool, bool, bool, bool, String) {
    if full {
        return (
            true,
            true,
            true,
            true,
            true,
            "full validation requested".into(),
        );
    }
    let paths: Vec<String> = paths
        .iter()
        .map(|path| normalize(path))
        .filter(|p| !p.is_empty())
        .collect();
    if paths.is_empty() {
        return (false, false, false, false, false, "no changed paths".into());
    }
    let package_required = paths.iter().any(|path| package_sensitive(path));
    let updater_required = paths.iter().any(|path| updater_sensitive(path));
    let release_required = paths.iter().any(|path| release_sensitive(path));
    let supply_chain_required = paths.iter().any(|path| supply_chain_sensitive(path));
    let code_required = paths.iter().any(|path| {
        (CODE_FILES.contains(&path.as_str())
            || CODE_PREFIXES.iter().any(|prefix| path.starts_with(prefix)))
            && !release_sensitive(path)
    }) || package_required;
    let reason = if release_required {
        let release = paths
            .iter()
            .filter(|path| release_sensitive(path))
            .map(String::as_str)
            .collect::<Vec<_>>();
        format!(
            "release-sensitive: {}",
            release.into_iter().take(3).collect::<Vec<_>>().join(", ")
        )
    } else if updater_required {
        let updater = paths
            .iter()
            .filter(|path| updater_sensitive(path))
            .map(String::as_str)
            .collect::<Vec<_>>();
        format!(
            "updater-sensitive: {}",
            updater.into_iter().take(3).collect::<Vec<_>>().join(", ")
        )
    } else if package_required {
        let package = paths
            .iter()
            .filter(|path| package_sensitive(path))
            .map(String::as_str)
            .collect::<Vec<_>>();
        format!(
            "package-sensitive: {}",
            package.into_iter().take(3).collect::<Vec<_>>().join(", ")
        )
    } else if code_required {
        format!(
            "code/windows: {}",
            paths.iter().take(3).cloned().collect::<Vec<_>>().join(", ")
        )
    } else if supply_chain_required {
        "supply-chain-sensitive".into()
    } else {
        "docs/site only".into()
    };
    (
        code_required,
        package_required,
        updater_required,
        release_required,
        supply_chain_required,
        reason,
    )
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
    let (
        code_required,
        package_required,
        updater_required,
        release_required,
        supply_chain_required,
        reason,
    ) = classify(&paths, false);
    let c = Classification {
        code_required,
        package_required,
        updater_required,
        release_required,
        supply_chain_required,
        reason,
    };
    c.print();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(paths: &[&str]) -> (bool, bool, bool, bool, bool, String) {
        let owned: Vec<String> = paths.iter().map(|path| (*path).to_owned()).collect();
        classify(&owned, false)
    }

    #[test]
    fn full_mode_short_circuits_without_paths() {
        let result = classify(&[], true);
        assert!(result.0 && result.1 && result.2 && result.3 && result.4);
        assert_eq!(result.5, "full validation requested");
        assert!(run(true, None, None, None).is_ok());
    }

    #[test]
    fn empty_paths_returns_all_false() {
        let result = values(&[]);
        assert!(!result.0 && !result.1 && !result.2 && !result.3 && !result.4);
        assert_eq!(result.5, "no changed paths");
    }

    #[test]
    fn docs_only_skips_expensive_jobs() {
        let result = values(&["docs/foo.md"]);
        assert!(!result.0 && !result.1 && !result.2 && !result.3 && !result.4);
        assert_eq!(result.5, "docs/site only");
    }

    #[test]
    fn native_code_requires_code_validation_only() {
        let result = values(&["rust/crates/sky_player/src/lib.rs"]);
        assert!(result.0 && !result.1 && !result.2 && !result.3 && !result.4);
        assert!(result.5.starts_with("code/windows:"));
    }

    #[test]
    fn desktop_changes_require_package_qualification() {
        let result = values(&["desktop/src/App.tsx"]);
        assert!(result.0 && result.1 && !result.2 && !result.3);
        assert!(result.5.starts_with("package-sensitive:"));
    }

    #[test]
    fn cargo_dependency_changes_require_package_and_supply_chain_validation() {
        let result = values(&["rust/Cargo.lock"]);
        assert!(result.0 && result.1 && !result.2 && !result.3 && result.4);
        assert!(result.5.starts_with("package-sensitive:"));
    }

    #[test]
    fn ci_workflow_is_code_and_supply_chain_sensitive_but_not_package_sensitive() {
        let result = values(&[".github/workflows/ci.yml"]);
        assert!(result.0 && !result.1 && !result.2 && !result.3 && result.4);
        assert!(result.5.starts_with("code/windows:"));
    }

    #[test]
    fn xtask_changes_do_not_imply_package_qualification() {
        let result = values(&["rust/xtask/src/main.rs"]);
        assert!(result.0 && !result.1 && !result.2 && !result.3 && !result.4);
    }

    #[test]
    fn updater_only_changes_select_updater_validation() {
        let result = values(&["scripts/ci_tauri_update_e2e_core.ps1"]);
        assert!(result.0 && !result.1 && result.2 && !result.3);
    }

    #[test]
    fn release_only_changes_select_release_validation() {
        let result = values(&[".github/workflows/release-v4.yml"]);
        assert!(!result.0 && !result.1 && !result.2 && result.3);
    }

    #[test]
    fn release_metadata_paths_are_current_and_legacy_paths_are_not_listed() {
        let result = values(&["rust/xtask/src/release_metadata.rs"]);
        assert!(!result.0 && !result.1 && result.2 && result.3);
        for obsolete in [
            "rust/xtask/src/release_authority.rs",
            "scripts/ci_v4_release_authority_acceptance.ps1",
            "scripts/test_v4_release_authority_rehearsal.ps1",
        ] {
            assert!(!UPDATER_FILES.contains(&obsolete) && !RELEASE_FILES.contains(&obsolete));
        }
        assert!(
            !RELEASE_FILES
                .iter()
                .chain(UPDATER_FILES.iter())
                .any(|path| path.contains("authority"))
        );
    }

    #[test]
    fn full_and_empty_modes_are_stable() {
        let full = classify(&["docs/foo.md".into()], true);
        assert!(full.0 && full.1 && full.2 && full.3 && full.4);
        let empty = classify(&[], false);
        assert!(!empty.0 && !empty.1 && !empty.2 && !empty.3 && !empty.4);
    }
}
