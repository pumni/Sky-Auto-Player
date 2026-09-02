use crate::Result;

#[allow(dead_code)]
pub fn classify(paths: &[String], full: bool) -> (bool, bool, bool, bool, String) {
    sky_ci_classifier::classify(paths, full)
}

pub fn run(
    full: bool,
    base: Option<&str>,
    head: Option<&str>,
    paths_file: Option<&str>,
) -> Result<()> {
    sky_ci_classifier::run(full, base, head, paths_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(paths: &[&str]) -> (bool, bool, bool, bool, String) {
        classify(
            &paths.iter().map(|p| (*p).into()).collect::<Vec<_>>(),
            false,
        )
    }

    #[test]
    fn classifies_docs_without_expensive_jobs() {
        assert_eq!(
            values(&["docs/guide.md"]),
            (true, false, false, false, "static/site/docs only".into())
        );
    }

    #[test]
    fn classifies_native_and_package_changes() {
        let result = values(&["rust/crates/sky_player/src/lib.rs"]);
        assert!(result.0 && result.1 && !result.2 && !result.3);
        assert!(values(&["rust/xtask/src/main.rs"]).1);
        assert!(values(&["rust/xtask/src/main.rs"]).2);
    }

    #[test]
    fn package_sensitive_changes_require_code_validation() {
        let result = values(&[".github/workflows/ci.yml"]);
        assert!(result.0);
        assert!(result.1);
        assert!(result.2);
        assert!(!result.3);
    }

    #[test]
    fn full_and_empty_modes_are_stable() {
        assert_eq!(
            classify(&[], true),
            (true, true, true, true, "full validation requested".into())
        );
        assert_eq!(
            values(&[]),
            (false, false, false, false, "no changed paths".into())
        );
    }

    #[test]
    fn browser_classification_is_path_aware() {
        assert!(values(&["desktop/src/components/App.tsx"]).3);
        assert!(values(&["desktop/src-tauri/src/commands.rs"]).3);
        assert!(!values(&["rust/crates/sky_player/src/lib.rs"]).3);
        assert!(!values(&["docs/architecture.md"]).3);
        assert!(values(&["desktop/package.json"]).3);
    }
}
