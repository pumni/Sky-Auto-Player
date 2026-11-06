use super::*;
use crate::repo;

#[test]
fn current_core_boundary_passes() {
    app_core_boundary(&crate::repo::root()).expect("sky_app_core boundary");
}

#[test]
fn current_adapter_dependency_boundary_passes() {
    adapter_boundary(&crate::repo::root()).expect("sky_native_adapters dependency boundary");
}

#[test]
fn native_runtime_module_budgets_reject_regrowth_and_cover_the_current_tree() {
    assert_eq!(
        native_runtime_line_budget_violation("native_runtime.rs", "one\ntwo\n", 1),
        Some("[native_runtime_module_lines] native_runtime.rs: 2 lines (> 1)".to_owned())
    );
    assert_eq!(
        native_runtime_line_budget_violation("native_runtime.rs", "one\ntwo\n", 2),
        None
    );
    native_runtime_module_budgets(&crate::repo::root())
        .expect("native runtime ownership modules stay within their budgets");
}

#[test]
fn playback_integration_module_budgets_reject_regrowth_and_cover_the_current_tree() {
    assert_eq!(
        module_line_budget_violation(
            "native_playback_test_root_lines",
            PLAYBACK_INTEGRATION_ROOT_PATH,
            "one\ntwo\n",
            1,
        ),
        Some(format!(
            "[native_playback_test_root_lines] {PLAYBACK_INTEGRATION_ROOT_PATH}: 2 lines (> 1)"
        ))
    );
    assert_eq!(
        module_line_budget_violation(
            "native_playback_test_module_lines",
            PLAYBACK_INTEGRATION_MODULE_BUDGETS[0].0,
            "one\ntwo\n",
            1,
        ),
        Some(format!(
            "[native_playback_test_module_lines] {}: 2 lines (> 1)",
            PLAYBACK_INTEGRATION_MODULE_BUDGETS[0].0
        ))
    );
    playback_integration_module_budgets(&crate::repo::root())
        .expect("native playback test owners stay within budgets and exact inventory");
}

#[test]
fn playback_integration_module_ownership_rejects_unbudgeted_siblings() {
    let expected = ["supervisor_lease.rs", "terminal_release.rs"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let actual = ["supervisor_lease.rs", "terminal_release.rs", "unowned.rs"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    assert!(playback_integration_module_inventory_violation(&expected, &actual).is_some());
    assert!(playback_integration_module_inventory_violation(&expected, &expected).is_none());
}

#[test]
fn engine_test_module_budgets_reject_regrowth_and_cover_the_current_tree() {
    assert_eq!(
        module_line_budget_violation(
            "engine_test_root_lines",
            ENGINE_TEST_ROOT_PATH,
            "one\ntwo\n",
            1,
        ),
        Some(format!(
            "[engine_test_root_lines] {ENGINE_TEST_ROOT_PATH}: 2 lines (> 1)"
        ))
    );
    assert_eq!(
        module_line_budget_violation(
            "engine_test_module_lines",
            ENGINE_TEST_MODULE_BUDGETS[0].0,
            "one\ntwo\n",
            1,
        ),
        Some(format!(
            "[engine_test_module_lines] {}: 2 lines (> 1)",
            ENGINE_TEST_MODULE_BUDGETS[0].0
        ))
    );
    engine_test_module_budgets(&crate::repo::root())
        .expect("engine test owners stay within their budgets and exact inventory");
}

#[test]
fn engine_test_module_ownership_rejects_unbudgeted_modules() {
    let expected = ["startup_admission.rs", "support.rs"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let actual = ["startup_admission.rs", "support.rs", "unowned.rs"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    assert!(engine_test_module_inventory_violation(&expected, &actual).is_some());
    assert!(engine_test_module_inventory_violation(&expected, &expected).is_none());
}

#[test]
fn xtask_module_budgets_reject_regrowth_and_cover_the_current_tree() {
    assert_eq!(
        module_line_budget_violation("xtask_module_lines", "checks.rs", "one\ntwo\n", 1),
        Some("[xtask_module_lines] checks.rs: 2 lines (> 1)".to_owned())
    );
    assert_eq!(
        module_line_budget_violation("xtask_module_lines", "checks.rs", "one\ntwo\n", 2),
        None
    );
    xtask_module_budgets(&crate::repo::root())
        .expect("xtask ownership modules stay within their budgets");
}

#[test]
fn xtask_module_ownership_rejects_unbudgeted_modules() {
    let expected = ["bindings.rs", "workflow.rs"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let actual = ["bindings.rs", "workflow.rs", "unowned.rs"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    assert!(xtask_module_inventory_violation(&expected, &actual).is_some());
    assert!(xtask_module_inventory_violation(&expected, &expected).is_none());
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
    assert!(
        WORKER_SCHEDULE_CLONE_PATTERNS
            .iter()
            .any(|pattern| "schedule.clone()".contains(pattern))
    );
}

#[test]
fn metric_allowlist_is_exact_monotonic_and_stale_entries_are_rejected() {
    let key = (
        "rust/crates/example/src/module.rs".to_owned(),
        "regular_module_lines".to_owned(),
        String::new(),
    );
    let allowlist = BTreeMap::from([(
        key.clone(),
        ArchitectureException {
            reason: "existing cohesive module".into(),
            review_trigger: "review before expansion".into(),
            baseline: Some(901),
        },
    )]);
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut used = BTreeSet::new();
    let baseline_metric = ArchitectureMetric {
        path: &key.0,
        rule: &key.1,
        subject: "",
        observed: 901,
        unit: "lines",
        hard_limit: REGULAR_HARD_LIMIT,
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
            observed: 902,
            ..baseline_metric
        },
    );
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("above baseline 901"));
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
            path: "rust/crates/example/src/new_module.rs",
            rule: "regular_module_lines",
            subject: "",
            observed: 901,
            unit: "lines",
            hard_limit: REGULAR_HARD_LIMIT,
        },
    );
    assert!(unlisted_errors[0].contains("without an exact baseline exception"));
}

#[test]
fn architecture_allowlist_rejects_duplicate_and_baseline_free_metric_entries() {
    let root = repo::root();
    let entry = serde_json::json!({
        "path": "rust/xtask/src/audits/architecture.rs",
        "rule": "regular_module_lines",
        "baseline": 901,
        "reason": "existing cohesive module",
        "review_trigger": "review before expansion"
    });
    let duplicate = serde_json::json!({"entries": [entry.clone(), entry.clone()]});
    assert!(parse_architecture_allowlist(&root, duplicate).is_err());

    let missing_baseline = serde_json::json!({
        "entries": [{
            "path": "rust/xtask/src/audits/architecture.rs",
            "rule": "regular_module_lines",
            "reason": "existing cohesive module",
            "review_trigger": "review before expansion"
        }]
    });
    assert!(parse_architecture_allowlist(&root, missing_baseline).is_err());
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
