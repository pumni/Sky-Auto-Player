use super::*;
use crate::repo;
use std::path::{Path, PathBuf};

struct ArchitectureTempRoot(PathBuf);

impl ArchitectureTempRoot {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "sky-xtask-architecture-fixture-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create isolated architecture fixture");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, relative: &str, contents: &str) {
        self.write_bytes(relative, contents.as_bytes());
    }

    fn write_bytes(&self, relative: &str, contents: &[u8]) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().expect("fixture file has a parent"))
            .expect("create fixture parent");
        std::fs::write(path, contents).expect("write architecture fixture");
    }
}

impl Drop for ArchitectureTempRoot {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove isolated architecture fixture");
    }
}

#[test]
fn current_core_boundary_passes() {
    app_core_boundary(&crate::repo::root()).expect("sky_app_core boundary");
}

#[test]
fn current_adapter_dependency_boundary_passes() {
    adapter_boundary(&crate::repo::root()).expect("sky_native_adapters dependency boundary");
}

#[test]
fn required_architecture_scan_roots_fail_closed_when_missing_or_not_directories() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sky-xtask-architecture-scan-root-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create isolated scan-root fixture");

    let missing = required_source_directory(&root, "rust/crates/sky_player/src")
        .expect_err("missing active source roots must fail");
    assert!(missing.to_string().contains("rust/crates/sky_player/src"));

    let source = root.join("rust/crates/sky_player/src");
    std::fs::create_dir_all(&source).expect("create active source root");
    assert_eq!(
        required_source_directory(&root, "rust/crates/sky_player/src").unwrap(),
        source
    );
    std::fs::remove_dir_all(&source).expect("remove active source root");
    std::fs::write(&source, "not a directory").expect("replace scan root with a file");

    let not_directory = required_source_directory(&root, "rust/crates/sky_player/src")
        .expect_err("a non-directory active root must fail");
    assert!(not_directory.to_string().contains("expected a directory"));
    std::fs::remove_dir_all(root).expect("remove isolated scan-root fixture");
}

#[test]
fn declared_new_siblings_are_accepted_and_scanned_without_name_inventories() {
    let root = ArchitectureTempRoot::new();
    root.write(
        "apps/desktop/src-tauri/src/native_runtime.rs",
        "mod tests;\n",
    );
    root.write(
        PLAYBACK_INTEGRATION_ROOT_PATH,
        "#[path = \"playback_integration/additional.rs\"]\nmod additional;\n",
    );
    root.write(
        "apps/desktop/src-tauri/src/native_runtime/tests/playback_integration/additional.rs",
        "#[test]\nfn additional_owner_is_valid() {}\n",
    );
    root.write(
        ENGINE_TEST_ROOT_PATH,
        "#[path = \"tests/additional.rs\"]\nmod additional;\n",
    );
    root.write(
        "rust/crates/sky_player/src/engine/tests/additional.rs",
        "#[test]\nfn additional_owner_is_valid() {}\n",
    );
    root.write(
        "rust/xtask/src/checks.rs",
        "#[path = \"checks/additional.rs\"]\nmod additional;\n",
    );
    root.write(
        "rust/xtask/src/checks/additional.rs",
        "pub(super) fn additional_check() {}\n",
    );

    assert!(
        native_runtime_advisory_warnings(root.path())
            .unwrap()
            .is_empty()
    );
    assert!(
        engine_test_advisory_warnings(root.path())
            .unwrap()
            .is_empty()
    );
    assert!(
        xtask_module_advisory_warnings(root.path())
            .unwrap()
            .is_empty()
    );

    // These fixture strings express declared Rust modules. The advisory scanner
    // discovers files for diagnostics; it does not claim to compile orphan files.
    assert!(is_engine_test_owner_path(
        "rust/crates/sky_player/src/engine/tests/new_owner.rs"
    ));
    assert!(is_engine_test_owner_path(ENGINE_TEST_ROOT_PATH));
    assert!(!is_engine_test_owner_path(
        "rust/crates/sky_player/src/engine/test_support/new.rs"
    ));
}

#[test]
fn invalid_unreferenced_orphans_receive_advisory_diagnostics_only() {
    let root = ArchitectureTempRoot::new();
    root.write(
        "rust/xtask/src/checks/unreferenced_orphan.rs",
        &"this is not valid Rust syntax\n".repeat(2_101),
    );

    let warnings = xtask_module_advisory_warnings(root.path())
        .expect("an orphan source file does not create a hard ownership gate");
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("unreferenced_orphan.rs"));
    assert!(warnings[0].contains("advisory threshold 2100"));

    // This fixture is intentionally invalid and unreferenced. The audit scans
    // file sizes only; Rust compiles declared modules, not arbitrary .rs files.
}

#[test]
fn advisory_scans_warn_for_large_modules_and_fail_closed_on_missing_or_unreadable_roots() {
    let root = ArchitectureTempRoot::new();
    root.write(ENGINE_TEST_ROOT_PATH, "mod oversized;\n");
    root.write(
        "rust/crates/sky_player/src/engine/tests/oversized.rs",
        &"// test line\n".repeat(2_201),
    );
    let warnings = engine_test_advisory_warnings(root.path()).expect("advisory size scan");
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("oversized.rs"));
    assert!(warnings[0].contains("2201 lines"));
    assert!(warnings[0].contains("advisory threshold 2200"));

    let missing = native_runtime_advisory_warnings(root.path())
        .expect_err("missing required native runtime file and directories fail closed");
    assert!(
        missing
            .to_string()
            .contains("required Rust source file is missing")
    );

    let unreadable_root = ArchitectureTempRoot::new();
    unreadable_root.write(ENGINE_TEST_ROOT_PATH, "mod unreadable;\n");
    unreadable_root.write_bytes(
        "rust/crates/sky_player/src/engine/tests/unreadable.rs",
        &[0xff],
    );
    let unreadable = engine_test_advisory_warnings(unreadable_root.path())
        .expect_err("unreadable Rust sources fail closed");
    assert!(unreadable.to_string().to_lowercase().contains("utf-8"));
}

#[test]
fn generic_facade_regular_and_context_metrics_are_nonblocking_diagnostics() {
    let facade = module_line_advisory_warning(
        "facade_lines",
        "rust/crates/example/src/lib.rs",
        &"line\n".repeat(251),
        FACADE_ADVISORY_LIMIT,
    )
    .expect("oversized facade diagnostic");
    assert!(facade.contains("251 lines"));
    assert!(facade.contains("advisory threshold 250"));

    let regular = module_line_advisory_warning(
        "regular_module_lines",
        "rust/crates/example/src/module.rs",
        &"line\n".repeat(701),
        REGULAR_ADVISORY_LIMIT,
    )
    .expect("oversized regular module diagnostic");
    assert!(regular.contains("701 lines"));
    assert!(regular.contains("advisory threshold 700"));

    let context = (0..13)
        .map(|index| format!("    field_{index}: u8,\n"))
        .collect::<String>();
    let context = format!("struct WorkerContext {{\n{context}}}\n");
    let context_warning =
        context_advisory_warnings("rust/crates/example/src/context.rs", &clean_lines(&context));
    assert_eq!(context_warning.len(), 1);
    assert!(context_warning[0].contains("13 fields"));
    assert!(context_warning[0].contains("advisory threshold 12"));
}

#[test]
fn category_advisory_scans_cover_new_native_engine_and_xtask_files() {
    let root = ArchitectureTempRoot::new();
    root.write(
        "apps/desktop/src-tauri/src/native_runtime.rs",
        "mod tests;\n",
    );
    root.write(PLAYBACK_INTEGRATION_ROOT_PATH, &"line\n".repeat(501));
    root.write(
        "apps/desktop/src-tauri/src/native_runtime/tests/playback_integration/new_owner.rs",
        &"line\n".repeat(1_601),
    );
    root.write(ENGINE_TEST_ROOT_PATH, &"line\n".repeat(751));
    root.write(
        "rust/crates/sky_player/src/engine/tests/new_owner.rs",
        &"line\n".repeat(2_201),
    );
    root.write(
        "rust/xtask/src/checks/new_owner.rs",
        &"line\n".repeat(2_101),
    );

    let native = native_runtime_advisory_warnings(root.path()).unwrap();
    assert_eq!(native.len(), 2);
    assert!(
        native
            .iter()
            .any(|warning| warning.contains("advisory threshold 500"))
    );
    assert!(
        native
            .iter()
            .any(|warning| warning.contains("advisory threshold 1600"))
    );

    let engine = engine_test_advisory_warnings(root.path()).unwrap();
    assert_eq!(engine.len(), 2);
    assert!(
        engine
            .iter()
            .any(|warning| warning.contains("advisory threshold 750"))
    );
    assert!(
        engine
            .iter()
            .any(|warning| warning.contains("advisory threshold 2200"))
    );

    let xtask = xtask_module_advisory_warnings(root.path()).unwrap();
    assert_eq!(xtask.len(), 1);
    assert!(xtask[0].contains("advisory threshold 2100"));
}
#[test]
fn adapter_dependency_boundary_rejects_player_and_non_windows_dispatch_edges() {
    let fixtures = [
        r#"
                [dependencies]
                sky_app_core = { path = "../sky_app_core" }
                sky_player = { path = "../sky_player" }
                [target.'cfg(windows)'.dependencies]
                sky_dispatch_win32 = { path = "../sky_dispatch_win32" }
            "#,
        r#"
                [dependencies]
                sky_app_core = { path = "../sky_app_core" }
                [target.'cfg(windows)'.dependencies]
                sky_dispatch_win32 = { path = "../sky_dispatch_win32" }
                [target.'cfg(unix)'.dev-dependencies]
                sky_player = { path = "../sky_player" }
            "#,
        r#"
                [dependencies]
                sky_app_core = { path = "../sky_app_core" }
                sky_dispatch_win32 = { path = "../sky_dispatch_win32" }
                [target.'cfg(windows)'.dependencies]
                sky_dispatch_win32 = { path = "../sky_dispatch_win32" }
            "#,
        r#"
                [dependencies]
                sky_app_core = { path = "../sky_app_core" }
                [target.'cfg(windows)'.dependencies]
                sky_dispatch_win32 = { path = "../sky_dispatch_win32" }
                [target.'cfg(unix)'.dependencies]
                sky_dispatch_win32 = { path = "../sky_dispatch_win32" }
            "#,
    ];
    for fixture in fixtures {
        let manifest: toml::Value = toml::from_str(fixture).expect("fixture manifest");
        assert!(adapter_dependency_boundary(&manifest).is_err());
    }
}

#[test]
fn architecture_allowlist_is_loaded_and_current_tree_passes() {
    let root = repo::root();
    let allowlist = load_architecture_allowlist(&root).unwrap();
    assert_eq!(allowlist.len(), 22);
    let rule_counts = allowlist.keys().fold(
        BTreeMap::<&str, usize>::new(),
        |mut counts, (_, rule, _)| {
            *counts.entry(rule).or_default() += 1;
            counts
        },
    );
    assert_eq!(rule_counts.get("dispatch_function_lines"), Some(&1));
    assert_eq!(rule_counts.get("production_glob_import"), Some(&2));
    assert_eq!(rule_counts.get("unsafe_boundary"), Some(&19));
    assert!(
        ["regular_module_lines", "facade_lines", "context_fields"]
            .iter()
            .all(|rule| !rule_counts.contains_key(rule))
    );
    assert!(!allowlist.contains_key(&(
        ENGINE_TEST_ROOT_PATH.into(),
        "regular_module_lines".into(),
        String::new()
    )));
    let session_glob_key = (
        "rust/crates/sky_player/src/engine/session.rs".to_owned(),
        "production_glob_import".to_owned(),
        String::new(),
    );
    assert_eq!(
            allowlist.get(&session_glob_key),
            Some(&ArchitectureException {
                reason: "pre-existing engine session import surface; preserve qualified session lifecycle while explicit dependency refactor awaits a separately reviewed architecture change".into(),
                review_trigger: "review before adding session dependencies or changing session ownership; remove when explicit imports are qualified".into(),
                baseline: Some(1),
            })
        );
    architecture(&root).expect("current full architecture policy passes");
}

#[test]
fn session_glob_exception_fails_closed_when_missing_mismatched_grown_or_stale() {
    let path = "rust/crates/sky_player/src/engine/session.rs";
    let rule = "production_glob_import";
    let key = (path.to_owned(), rule.to_owned(), String::new());
    let exception = ArchitectureException {
            reason: "pre-existing engine session import surface; preserve qualified session lifecycle while explicit dependency refactor awaits a separately reviewed architecture change".into(),
            review_trigger: "review before adding session dependencies or changing session ownership; remove when explicit imports are qualified".into(),
            baseline: Some(1),
        };
    let allowlist = BTreeMap::from([(key, exception)]);
    let live_session_glob = ArchitectureMetric {
        path,
        rule,
        subject: "",
        observed: 1,
        unit: "top-level `use super::*` imports",
        hard_limit: 0,
    };

    let mut missing_errors = Vec::new();
    architecture_metric_record(
        &mut missing_errors,
        &mut Vec::new(),
        &BTreeMap::new(),
        &mut BTreeSet::new(),
        live_session_glob,
    );
    assert!(missing_errors[0].contains("without an exact baseline exception"));

    let mismatched_allowlist = BTreeMap::from([(
        (
            "rust/crates/sky_player/src/engine/worker.rs".to_owned(),
            rule.to_owned(),
            String::new(),
        ),
        ArchitectureException {
            reason: "wrong path".into(),
            review_trigger: "wrong path".into(),
            baseline: Some(1),
        },
    )]);
    let mut mismatch_errors = Vec::new();
    architecture_metric_record(
        &mut mismatch_errors,
        &mut Vec::new(),
        &mismatched_allowlist,
        &mut BTreeSet::new(),
        live_session_glob,
    );
    assert!(mismatch_errors[0].contains("without an exact baseline exception"));

    let mut growth_errors = Vec::new();
    architecture_metric_record(
        &mut growth_errors,
        &mut Vec::new(),
        &allowlist,
        &mut BTreeSet::new(),
        ArchitectureMetric {
            observed: 2,
            ..live_session_glob
        },
    );
    assert!(growth_errors[0].contains("above baseline 1"));

    let stale = stale_metric_allowlist_entries(&allowlist, &BTreeSet::new());
    assert_eq!(stale.len(), 1);
    assert!(stale[0].contains(path));
    assert!(stale[0].contains("has no live violation"));
}

#[test]
fn architecture_helpers_cover_context_function_glob_and_schedule_rules() {
    let context = (0..13)
        .map(|index| format!("    field_{index}: u8,\n"))
        .collect::<String>();
    let context = format!("struct WorkerContext {{\n{context}}}\n");
    assert_eq!(context_violations(&clean_lines(&context)).len(), 1);
    let function = std::iter::once("fn oversized() {\n".to_owned())
        .chain((0..181).map(|_| "    let _value = 1;\n".to_owned()))
        .chain(std::iter::once("}\n".to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(function_line_violations(&function, 180).len(), 1);
    let worker_function = std::iter::once("fn oversized_worker() {\n".to_owned())
        .chain((0..351).map(|_| "    let _value = 1;\n".to_owned()))
        .chain(std::iter::once("}\n".to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(
        function_line_violations(&worker_function, WORKER_FUNCTION_HARD_LIMIT).len(),
        1
    );
    assert_eq!(
        top_level_glob_import_count(&clean_lines(
            "use super::*;\nmod tests {\n    use super::*;\n}\nfn f() {\n    use super::*;\n}\n"
        )),
        1
    );
    assert!(is_test_source_path(
        "rust/crates/sky_player/src/engine/worker/dispatch/authored_tests.rs",
        ""
    ));
    assert!(!is_test_source_path(
        "rust/crates/sky_player/src/engine/worker/dispatch/authored.rs",
        ""
    ));
    assert!(is_test_source_path(
        "fixture.rs",
        "#![cfg(test)]\nuse super::*;"
    ));
    assert!(contains_unsafe_code("unsafe { value(); }"));
    let dependency_violation = dependency_direction_violations(
        "sky_dispatch_core",
        "use sky_dispatch_win32::input::PhysicalKey;",
    )
    .into_iter()
    .next()
    .expect("core-to-platform dependency remains forbidden");
    let mut dependency_errors = Vec::new();
    architecture_record(
        &mut dependency_errors,
        &mut Vec::new(),
        &BTreeMap::new(),
        "rust/crates/sky_dispatch_core/src/fixture.rs",
        "dependency_direction",
        dependency_violation,
    );
    assert_eq!(dependency_errors.len(), 1);
    assert_eq!(
        dependency_direction_violations("sky_dispatch_win32", "use sky_player::engine::Worker;"),
        ["lower crate imports sky_player"]
    );
    assert_eq!(
        dependency_direction_violations("sky_dispatch_core", "use sky_app_core::timing;"),
        Vec::<&str>::new()
    );
    assert_eq!(
        dependency_direction_violations(
            "sky_dispatch_core",
            "use sky_dispatch_win32::input; use sky_player::engine;"
        ),
        [
            "core imports sky_dispatch_win32",
            "lower crate imports sky_player"
        ]
    );
    let schedule_clone = runtime_schedule_clone_violation(
        "rust/crates/sky_player/src/engine/worker.rs",
        "let duplicate = schedule.clone();",
    );
    assert!(schedule_clone);
    let mut schedule_errors = Vec::new();
    architecture_record(
        &mut schedule_errors,
        &mut Vec::new(),
        &BTreeMap::new(),
        "rust/crates/sky_player/src/engine/worker.rs",
        "runtime_schedule_clone",
        "worker must not clone RuntimeSchedule",
    );
    assert_eq!(schedule_errors.len(), 1);
    assert!(!runtime_schedule_clone_violation(
        "rust/crates/sky_player/src/engine/session.rs",
        "let duplicate = schedule.clone();"
    ));
}

#[test]
fn metric_allowlist_is_exact_monotonic_and_stale_entries_are_rejected() {
    let key = (
        "rust/crates/sky_player/src/engine/worker/dispatch/observer.rs".to_owned(),
        "dispatch_function_lines".to_owned(),
        "drain_down_send_outcome".to_owned(),
    );
    let allowlist = BTreeMap::from([(
        key.clone(),
        ArchitectureException {
            reason: "existing dispatch function".into(),
            review_trigger: "review before expansion".into(),
            baseline: Some(233),
        },
    )]);
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut used = BTreeSet::new();
    let baseline_metric = ArchitectureMetric {
        path: &key.0,
        rule: &key.1,
        subject: &key.2,
        observed: 233,
        unit: "lines",
        hard_limit: DISPATCH_FUNCTION_HARD_LIMIT,
    };
    architecture_metric_record(
        &mut errors,
        &mut warnings,
        &allowlist,
        &mut used,
        baseline_metric,
    );
    assert!(errors.is_empty());
    assert_eq!(warnings.len(), 1);
    assert!(stale_metric_allowlist_entries(&allowlist, &used).is_empty());

    architecture_metric_record(
        &mut errors,
        &mut warnings,
        &allowlist,
        &mut used,
        ArchitectureMetric {
            observed: 234,
            ..baseline_metric
        },
    );
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("above baseline 233"));
    assert_eq!(
        stale_metric_allowlist_entries(&allowlist, &BTreeSet::new()).len(),
        1
    );

    let mut unlisted_errors = Vec::new();
    architecture_metric_record(
        &mut unlisted_errors,
        &mut Vec::new(),
        &allowlist,
        &mut BTreeSet::new(),
        ArchitectureMetric {
            path: "rust/crates/sky_player/src/engine/worker/dispatch/new_module.rs",
            rule: "dispatch_function_lines",
            subject: "new_function",
            observed: 181,
            unit: "lines",
            hard_limit: DISPATCH_FUNCTION_HARD_LIMIT,
        },
    );
    assert!(unlisted_errors[0].contains("without an exact baseline exception"));
}

#[test]
fn architecture_allowlist_rejects_duplicate_and_baseline_free_metric_entries() {
    let root = repo::root();
    let entry = serde_json::json!({
        "path": "rust/xtask/src/audits/architecture.rs",
        "rule": "dispatch_function_lines",
        "baseline": 233,
        "subject": "drain_down_send_outcome",
        "reason": "existing dispatch function",
        "review_trigger": "review before expansion"
    });
    let duplicate = serde_json::json!({"entries": [entry.clone(), entry.clone()]});
    assert!(parse_architecture_allowlist(&root, duplicate).is_err());

    let missing_baseline = serde_json::json!({
        "entries": [{
            "path": "rust/xtask/src/audits/architecture.rs",
            "rule": "dispatch_function_lines",
            "subject": "drain_down_send_outcome",
            "reason": "existing cohesive module",
            "review_trigger": "review before expansion"
        }]
    });
    assert!(parse_architecture_allowlist(&root, missing_baseline).is_err());

    for rule in ["regular_module_lines", "facade_lines", "context_fields"] {
        let advisory = serde_json::json!({
            "entries": [{
                "path": "rust/xtask/src/audits/architecture.rs",
                "rule": rule,
                "baseline": 999,
                "subject": "advisory metric",
                "reason": "advisory metrics cannot be allowlisted",
                "review_trigger": "no hard gate"
            }]
        });
        let error = parse_architecture_allowlist(&root, advisory)
            .expect_err("advisory size and context metrics cannot have exceptions");
        assert!(
            error
                .to_string()
                .contains("is advisory and cannot be allowlisted")
        );
    }
}

#[test]
fn unsafe_inventory_separates_production_from_cfg_test_code() {
    let source = r#"
fn production_boundary() {
    unsafe { core::hint::unreachable_unchecked() }
}

#[cfg(test)]
mod tests {
    fn test_boundary() {
        unsafe { core::hint::unreachable_unchecked() }
    }
}
"#;
    let inventory =
        unsafe_code_inventory("apps/desktop/src-tauri/src/inventory_fixture.rs", source);
    assert_eq!(inventory.production_lines, vec![3]);
    assert_eq!(inventory.cfg_test_lines, vec![9]);

    let file_test = unsafe_code_inventory(
        "rust/crates/sky_native_adapters/src/fixture.rs",
        "#![cfg(test)]\nfn test_boundary() { unsafe { core::hint::unreachable_unchecked() } }\n",
    );
    assert!(file_test.production_lines.is_empty());
    assert_eq!(file_test.cfg_test_lines, vec![2]);
}
#[test]
fn unauthorized_unsafe_is_rejected_in_shell_and_adapter_sources() {
    let source = "fn injected() { unsafe { core::hint::unreachable_unchecked() } }\n";
    for path in [
        "apps/desktop/src-tauri/src/injected.rs",
        "rust/crates/sky_native_adapters/src/injected.rs",
    ] {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        audit_unsafe_source(path, source, &BTreeMap::new(), &mut errors, &mut warnings);
        assert_eq!(warnings.len(), 0, "{path}");
        assert_eq!(errors.len(), 1, "{path}");
        assert!(
            errors[0].contains("[unsafe_boundary]"),
            "{path}: {errors:?}"
        );
    }
}
