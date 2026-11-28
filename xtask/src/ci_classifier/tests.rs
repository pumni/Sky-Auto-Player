use super::*;
use std::path::PathBuf;

fn is_tauri_module_specifier(statement: &str) -> bool {
    statement.contains("@tauri-apps/api/") || statement.contains("@tauri-apps/plugin-")
}

fn is_type_only_import_statement(statement: &str) -> bool {
    let statement = statement.trim_start();
    if statement.starts_with("import type ") || statement.starts_with("export type ") {
        return true;
    }

    let Some(open) = statement.find('{') else {
        return false;
    };
    if statement.starts_with("import ") && !statement["import ".len()..open].trim().is_empty() {
        return false;
    }
    let Some(close) = statement[open + 1..].find('}') else {
        return false;
    };
    let specifiers = &statement[open + 1..open + 1 + close];
    let specifiers = specifiers
        .split(',')
        .map(str::trim)
        .filter(|specifier| !specifier.is_empty())
        .collect::<Vec<_>>();
    !specifiers.is_empty()
        && specifiers
            .iter()
            .all(|specifier| specifier.starts_with("type "))
}

fn contains_tauri_runtime_import(source: &str) -> bool {
    let mut declaration = None::<String>;
    for raw_line in source.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }

        if let Some(statement) = declaration.as_mut() {
            statement.push(' ');
            statement.push_str(line);
        } else if line.starts_with("import ") || line.starts_with("export ") {
            declaration = Some(line.to_owned());
        } else {
            let is_dynamic_import = line.contains("import(") || line.contains("import (");
            let is_type_query = line.starts_with("type ") || line.starts_with("export type ");
            if is_dynamic_import && !is_type_query && is_tauri_module_specifier(line) {
                return true;
            }
            continue;
        }

        let statement = declaration.as_deref().unwrap_or_default();
        if !is_type_only_import_statement(statement) && is_tauri_module_specifier(statement) {
            return true;
        }
        if statement.contains(';') || statement.contains(" from ") {
            declaration = None;
        }
    }
    false
}

fn temp_dir(name: &str) -> PathBuf {
    let path = ci_artifacts::unique_sibling(&std::env::temp_dir().join(name), "test");
    fs::create_dir(&path).unwrap();
    path
}

fn assert_lanes(classification: &Classification, expected: &[&str]) {
    for lane in LANES {
        assert_eq!(
            classification.lane(lane),
            expected.contains(&lane),
            "{lane}"
        );
    }
}

#[test]
fn working_tree_includes_staged_unstaged_deleted_and_untracked_paths() {
    let root = temp_dir("working-tree-classifier");
    git(&root, &["init"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    fs::create_dir_all(root.join("apps/desktop/src")).unwrap();
    fs::create_dir_all(root.join("apps/website/src")).unwrap();
    fs::write(root.join("apps/desktop/src/App.tsx"), "old").unwrap();
    fs::write(root.join("apps/website/src/page.astro"), "old").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial"]);
    assert_lanes(&classify_working_tree(&root, None).unwrap(), &[]);
    fs::write(root.join("apps/desktop/src/App.tsx"), "new").unwrap();
    git(&root, &["add", "apps/desktop/src/App.tsx"]);
    fs::remove_file(root.join("apps/website/src/page.astro")).unwrap();
    let result = classify_working_tree(&root, None).unwrap();
    assert!(result.lane("desktop_required"));
    assert!(result.lane("site_required"));
    fs::write(root.join("unknown-file"), "new").unwrap();
    assert_lanes(&classify_working_tree(&root, None).unwrap(), &LANES);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn working_tree_with_base_and_renames() {
    let root = temp_dir("working-tree-base-classifier");
    git(&root, &["init"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    fs::create_dir_all(root.join("apps/desktop/src")).unwrap();
    fs::write(root.join("apps/desktop/src/App.tsx"), "initial").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial"]);

    // Create base branch/tag
    git(&root, &["tag", "v-base"]);

    // Commit on branch
    fs::write(root.join("apps/desktop/src/App.tsx"), "v2").unwrap();
    git(&root, &["commit", "-am", "branch commit"]);

    // Working tree preview with base
    let with_base = classify_working_tree(&root, Some("v-base")).unwrap();
    assert!(with_base.lane("desktop_required"));

    // Actual git mv rename
    git(
        &root,
        &[
            "mv",
            "apps/desktop/src/App.tsx",
            "apps/desktop/src/RenamedApp.tsx",
        ],
    );
    let renamed = classify_working_tree(&root, Some("v-base")).unwrap();
    assert!(renamed.lane("desktop_required"));

    // Invalid base fails closed to full lanes
    let invalid_base = classify_working_tree(&root, Some("invalid-ref-sky-9999")).unwrap();
    assert_lanes(&invalid_base, &LANES);
    assert!(
        invalid_base
            .classification_reason
            .contains("invalid or missing base ref")
    );

    // Option-looking base fails closed
    let option_base = classify_working_tree(&root, Some("--option-injection")).unwrap();
    assert_lanes(&option_base, &LANES);
    assert!(
        option_base
            .classification_reason
            .contains("invalid option-looking")
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn workspace_cutover_diff_keeps_old_deletion_and_root_addition_fail_closed() {
    let root = temp_dir("workspace-cutover-classifier");
    git(&root, &["init"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    let old_crate = root.join("rust/crates/sky_player/src/lib.rs");
    let old_policy = root.join("rust/supply-chain/config.toml");
    fs::create_dir_all(old_crate.parent().unwrap()).unwrap();
    fs::create_dir_all(old_policy.parent().unwrap()).unwrap();
    fs::write(&old_crate, "pub fn old_layout() {}\n").unwrap();
    fs::write(&old_policy, "[policy]\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "old workspace layout"]);
    let base = git_sha(&root);

    fs::create_dir_all(root.join("crates/sky_player/src")).unwrap();
    fs::create_dir_all(root.join("supply-chain")).unwrap();
    git(
        &root,
        &[
            "mv",
            "rust/crates/sky_player/src/lib.rs",
            "crates/sky_player/src/lib.rs",
        ],
    );
    git(
        &root,
        &[
            "mv",
            "rust/supply-chain/config.toml",
            "supply-chain/config.toml",
        ],
    );
    git(&root, &["commit", "-m", "root workspace layout"]);
    let head = git_sha(&root);

    let paths = changed_paths_from_git(&root, DiffMode::Push, &base, &head, None).unwrap();
    for expected in [
        "rust/crates/sky_player/src/lib.rs",
        "crates/sky_player/src/lib.rs",
        "rust/supply-chain/config.toml",
        "supply-chain/config.toml",
    ] {
        assert!(paths.iter().any(|path| path == expected), "{expected}");
    }
    assert_lanes(&classify_git_paths(paths, DiffMode::Push), &LANES);
    assert_lanes(
        &classify_paths(["rust/crates/sky_player/src/lib.rs".to_owned()]),
        &LANES,
    );
    assert_lanes(
        &classify_paths(["supply-chain/config.toml".to_owned()]),
        &LANES,
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn supply_chain_policy_only_change_fails_closed_to_full_validation() {
    let root = temp_dir("supply-chain-policy-only-classifier");
    git(&root, &["init"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    fs::create_dir_all(root.join("supply-chain")).unwrap();
    let policy = root.join("supply-chain/config.toml");
    fs::write(&policy, "[policy]\nversion = 1\n").unwrap();
    git(&root, &["add", "supply-chain/config.toml"]);
    git(&root, &["commit", "-m", "initial supply-chain policy"]);
    let base = git_sha(&root);

    fs::write(&policy, "[policy]\nversion = 2\n").unwrap();
    git(&root, &["add", "supply-chain/config.toml"]);
    git(&root, &["commit", "-m", "change supply-chain policy"]);
    let head = git_sha(&root);

    let paths = changed_paths_from_git(&root, DiffMode::Push, &base, &head, None).unwrap();
    assert_eq!(paths, ["supply-chain/config.toml".to_owned()]);
    let classification = classify_git_paths(paths, DiffMode::Push);
    assert_lanes(&classification, &LANES);
    assert!(
        classification
            .classification_reason
            .contains("unknown path requires full validation")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn working_tree_detects_staged_unstaged_cancellation() {
    let root = temp_dir("working-tree-cancellation");
    git(&root, &["init"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    fs::create_dir_all(root.join("apps/desktop/src")).unwrap();
    let target = root.join("apps/desktop/src/App.tsx");
    fs::write(&target, "committed-v1").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "initial"]);

    // Step 1: stage an edit
    fs::write(&target, "staged-modification").unwrap();
    git(&root, &["add", "apps/desktop/src/App.tsx"]);

    // Step 2: in unstaged working tree, revert content back to "committed-v1"
    fs::write(&target, "committed-v1").unwrap();

    // Net diff against HEAD in working tree would be empty if not taking union,
    // but union of index/HEAD and worktree/index must still detect the change!
    let classification = classify_working_tree(&root, None).unwrap();
    assert!(
        classification.lane("desktop_required"),
        "staged change must be detected even if working tree unstaged copy was cancelled"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn parse_arguments_rejects_duplicate_or_ambiguous_working_tree_flags() {
    assert!(parse_arguments(&["--working-tree".into(), "--working-tree".into()]).is_err());
    assert!(
        parse_arguments(&[
            "--working-tree".into(),
            "--base".into(),
            "main".into(),
            "--base".into(),
            "other".into()
        ])
        .is_err()
    );
    assert!(
        parse_arguments(&[
            "--working-tree".into(),
            "--base".into(),
            "--diff-mode".into()
        ])
        .is_err()
    );
    assert!(parse_arguments(&["--working-tree".into(), "--full".into()]).is_err());
    assert!(
        parse_arguments(&[
            "--working-tree".into(),
            "--github-output".into(),
            "sentinel.txt".into()
        ])
        .is_err()
    );
}

#[test]
fn run_working_tree_never_touches_github_output_even_on_failure() {
    let temp = temp_dir("working-tree-output-safety");
    let sentinel_path = temp.join("sentinel-output.txt");
    let initial_bytes = b"IMMUTABLE_SENTINEL_CONTENT_XYZ";
    fs::write(&sentinel_path, initial_bytes).unwrap();

    let nonexistent_path = temp.join("nonexistent-output.txt");

    // Call run with --working-tree and --github-output targeting sentinel
    let res1 = run(&[
        "--working-tree".into(),
        "--github-output".into(),
        sentinel_path.to_str().unwrap().into(),
    ]);
    assert!(res1.is_ok());
    // Verify sentinel was NOT modified
    assert_eq!(fs::read(&sentinel_path).unwrap(), initial_bytes);

    // Call run with --working-tree and nonexistent output
    let res2 = run(&[
        "--working-tree".into(),
        "--github-output".into(),
        nonexistent_path.to_str().unwrap().into(),
    ]);
    assert!(res2.is_ok());
    // Verify nonexistent file was NOT created
    assert!(!nonexistent_path.exists());

    fs::remove_dir_all(temp).unwrap();
}

#[test]
fn preserves_exact_lane_output_shape_and_reason_golden() {
    let classification = classify_paths(["apps/desktop/src/App.tsx".to_owned()]);
    assert_eq!(
        classification.render(),
        "rust_required=false\ndesktop_required=true\ndesktop_native_required=false\ndesktop_e2e_required=true\npackage_required=false\nupdater_required=false\nrelease_required=false\nsigning_proof_required=false\nmalware_proof_required=false\nsupply_chain_required=false\nsite_required=false\nclassification_reason=required lanes (desktop_required, desktop_e2e_required): apps/desktop/src/App.tsx"
    );
}

#[test]
fn matches_positive_and_union_classifier_matrix() {
    for path in ["README.md", "CHANGELOG.md", "docs/architecture.md"] {
        assert_lanes(&classify_paths([path.to_owned()]), &[]);
    }
    assert_lanes(&classify_paths(["docs/releases/v4.0.2.md".to_owned()]), &[]);
    assert_lanes(
        &classify_paths(["apps/website/package.json".to_owned()]),
        &["site_required"],
    );
    assert_lanes(
        &classify_paths(["apps/website/src/pages/index.astro".to_owned()]),
        &["site_required"],
    );
    assert_lanes(
        &classify_paths([
            "apps/desktop/src/App.tsx".to_owned(),
            "apps/website/src/pages/index.astro".to_owned(),
        ]),
        &["desktop_required", "desktop_e2e_required", "site_required"],
    );
    let static_site = classify_paths([
        ".config/rust_architecture_allowlist.json".to_owned(),
        "apps/website/src/pages/index.astro".to_owned(),
    ]);
    assert_lanes(&static_site, &["site_required"]);
    assert!(
        static_site
            .classification_reason
            .starts_with("static-only plus required lanes")
    );

    let desktop = classify_paths([
        "docs/releases/v4.0.2.md".to_owned(),
        "apps/desktop/src/App.tsx".to_owned(),
    ]);
    assert_lanes(&desktop, &["desktop_required", "desktop_e2e_required"]);
    assert!(
        desktop
            .classification_reason
            .starts_with("static-only plus required lanes")
    );

    let lanes = classify_paths([
        "apps/desktop/src-tauri/tauri.conf.json".to_owned(),
        "xtask/src/ci_candidate.rs".to_owned(),
        "scripts/ci_build_updater_bridge.ps1".to_owned(),
    ]);
    assert_lanes(
        &lanes,
        &[
            "rust_required",
            "desktop_required",
            "desktop_native_required",
            "package_required",
            "updater_required",
            "signing_proof_required",
            "malware_proof_required",
        ],
    );
    assert_lanes(
        &classify_paths(["scripts/ci_build_updater_bridge.ps1".to_owned()]),
        &["updater_required"],
    );

    let nsis_owner = classify_paths(["xtask/src/nsis_harness.rs".to_owned()]);
    assert_lanes(
        &nsis_owner,
        &[
            "rust_required",
            "desktop_native_required",
            "package_required",
            "release_required",
        ],
    );
    for path in [
        "xtask/src/process.rs",
        "xtask/src/process/unix.rs",
        "xtask/src/release_v4.rs",
        "xtask/src/release_v4/recovery.rs",
        "xtask/src/release_v4/recovery/process_smoke.rs",
    ] {
        assert_lanes(
            &classify_paths([path.to_owned()]),
            &[
                "rust_required",
                "desktop_native_required",
                "package_required",
                "release_required",
            ],
        );
    }
    let updater_owner = classify_paths(["xtask/src/updater_fixture.rs".to_owned()]);
    assert_lanes(
        &updater_owner,
        &[
            "rust_required",
            "desktop_native_required",
            "package_required",
            "updater_required",
            "release_required",
        ],
    );
    let registry_owner = classify_paths(["crates/sky_windows_security/src/registry.rs".to_owned()]);
    assert_lanes(
        &registry_owner,
        &[
            "rust_required",
            "desktop_native_required",
            "package_required",
            "release_required",
        ],
    );
}

#[test]
fn preserves_current_positive_path_matrix_and_script_categories() {
    let cases: &[(&str, &[&str])] = &[
        ("README.md", &[]),
        ("docs/releases/v4.0.2.md", &[]),
        ("apps/website/package.json", &["site_required"]),
        (".github/workflows/pages.yml", &["site_required"]),
        (".bun-version", &LANES),
        (
            ".github/workflows/release-v4.yml",
            &[
                "release_required",
                "signing_proof_required",
                "malware_proof_required",
            ],
        ),
        (".github/workflows/ci.yml", &LANES),
        (
            "apps/desktop/src/App.tsx",
            &["desktop_required", "desktop_e2e_required"],
        ),
        (
            "apps/desktop/tests/e2e/library.spec.ts",
            &["desktop_required", "desktop_e2e_required"],
        ),
        (
            "apps/desktop/package.json",
            &[
                "desktop_required",
                "desktop_e2e_required",
                "package_required",
                "updater_required",
                "supply_chain_required",
            ],
        ),
        (
            "apps/desktop/src-tauri/src/commands.rs",
            &[
                "rust_required",
                "desktop_required",
                "desktop_native_required",
                "package_required",
            ],
        ),
        (
            "apps/desktop/src-tauri/src/native_update.rs",
            &[
                "rust_required",
                "desktop_required",
                "desktop_native_required",
                "package_required",
                "updater_required",
            ],
        ),
        (
            "apps/desktop/src-tauri/src/main.rs",
            &[
                "rust_required",
                "desktop_required",
                "desktop_native_required",
                "package_required",
            ],
        ),
        (
            "apps/desktop/src-tauri/tauri.conf.json",
            &[
                "desktop_required",
                "desktop_native_required",
                "package_required",
                "updater_required",
                "signing_proof_required",
                "malware_proof_required",
            ],
        ),
        (
            "apps/desktop/src-tauri/nsis-hooks.nsh",
            &[
                "desktop_native_required",
                "package_required",
                "signing_proof_required",
                "malware_proof_required",
            ],
        ),
        (
            "apps/desktop/src-tauri/capabilities/main.json",
            &[
                "desktop_required",
                "desktop_native_required",
                "package_required",
            ],
        ),
        (
            "apps/desktop/src-tauri/icons/icon.ico",
            &["package_required"],
        ),
        (
            "Cargo.toml",
            &[
                "rust_required",
                "package_required",
                "updater_required",
                "release_required",
                "supply_chain_required",
            ],
        ),
        (
            "Cargo.lock",
            &[
                "rust_required",
                "package_required",
                "updater_required",
                "release_required",
                "supply_chain_required",
            ],
        ),
        ("supply-chain/config.toml", &LANES),
        ("build_support/git_metadata.rs", &LANES),
        (
            "crates/sky_player/Cargo.toml",
            &[
                "rust_required",
                "desktop_native_required",
                "package_required",
                "supply_chain_required",
            ],
        ),
        (
            "crates/sky_player/src/lib.rs",
            &[
                "rust_required",
                "desktop_native_required",
                "package_required",
            ],
        ),
        (
            "xtask/src/ci_candidate.rs",
            &[
                "rust_required",
                "desktop_native_required",
                "package_required",
                "updater_required",
            ],
        ),
        (
            "xtask/src/release_signing.rs",
            &[
                "rust_required",
                "package_required",
                "updater_required",
                "release_required",
                "signing_proof_required",
            ],
        ),
        (
            "xtask/src/release_v4/malware.rs",
            &[
                "rust_required",
                "release_required",
                "malware_proof_required",
            ],
        ),
        ("xtask/src/checks/hosted_signing.rs", &LANES),
        ("scripts/ci_build_updater_bridge.ps1", &["updater_required"]),
        (
            "scripts/sign_v4_authenticode.ps1",
            &[
                "package_required",
                "release_required",
                "signing_proof_required",
            ],
        ),
        (
            "scripts/test_v4_authenticode.ps1",
            &[
                "package_required",
                "release_required",
                "signing_proof_required",
            ],
        ),
        (
            "scripts/v4_nsis_smoke_boundary.ps1",
            &[
                "release_required",
                "package_required",
                "signing_proof_required",
                "malware_proof_required",
            ],
        ),
        (
            "scripts/scan_v4_malware_exact.py",
            &["release_required", "malware_proof_required"],
        ),
        (
            "scripts/ci_bootstrap_clamav.sh",
            &["release_required", "malware_proof_required"],
        ),
        ("scripts/v4_release_pipeline.ps1", &["release_required"]),
        (
            "scripts/promote_v4_metadata.ps1",
            &["updater_required", "release_required"],
        ),
    ];
    for (path, expected) in cases {
        let classification = classify_paths([(*path).to_owned()]);
        for lane in LANES {
            assert_eq!(
                classification.lane(lane),
                expected.contains(&lane),
                "{path}: {lane}"
            );
        }
    }

    let main_ui =
        classify_normalized_paths(vec!["apps/desktop/src/App.tsx".to_owned()], DiffMode::Push);
    assert_lanes(
        &main_ui,
        &[
            "desktop_required",
            "desktop_e2e_required",
            "package_required",
        ],
    );
    let main_site =
        classify_normalized_paths(vec!["apps/website/package.json".to_owned()], DiffMode::Push);
    assert_lanes(&main_site, &["site_required"]);
}
#[test]
fn rust_toolchain_resolver_and_setup_action_select_all_validation_lanes() {
    for path in [
        "scripts/toolchain-pin.mjs",
        ".github/actions/setup-rust-pinned/action.yml",
    ] {
        assert_lanes(&classify_paths([path.to_owned()]), &LANES);
    }
    assert_lanes(
        &classify_paths(["rust-toolchain.toml".to_owned()]),
        &[
            "rust_required",
            "desktop_required",
            "desktop_native_required",
            "package_required",
            "updater_required",
            "release_required",
        ],
    );
    assert_lanes(
        &classify_paths(["apps/website/src/pages/index.astro".to_owned()]),
        &["site_required"],
    );
}

#[test]
fn windows_tauri_integration_frontend_paths_select_package_qualification() {
    for path in [
        "apps/desktop/src/bridge/tauriBridge.ts",
        "apps/desktop/src/bridge/startupTelemetry.ts",
        "apps/desktop/src/bridge/generated/commands.ts",
        "apps/desktop/src/bridge/generated/nested/Example.ts",
        "apps/desktop/src/platform/runtime.ts",
        "apps/desktop/src/platform/windowControls.ts",
        "apps/desktop/src/platform/startupWindowReveal.ts",
        "apps/desktop/src/platform/systemAppearance.ts",
        "apps/desktop/src/platform/externalLinks.ts",
        "apps/desktop/src/app/BootstrapGate.tsx",
        "apps/desktop/src/app/usePackagedSmokeTest.ts",
        "apps/desktop/src/main.tsx",
    ] {
        assert_lanes(
            &classify_paths([path.to_owned()]),
            &[
                "desktop_required",
                "desktop_native_required",
                "desktop_e2e_required",
                "package_required",
            ],
        );
    }

    for path in [
        "apps/desktop/src/components/PlaybackControls.tsx",
        "apps/desktop/src/styles/app.css",
        "apps/desktop/src/app/App.tsx",
        "apps/desktop/src/bridge/generatedish/Example.ts",
    ] {
        assert_lanes(
            &classify_paths([path.to_owned()]),
            &["desktop_required", "desktop_e2e_required"],
        );
    }
}

#[test]
fn production_tauri_runtime_imports_are_windows_package_qualified() {
    assert!(contains_tauri_runtime_import(
        "import { invoke } from '@tauri-apps/api/core';"
    ));
    assert!(contains_tauri_runtime_import(
        r"import {
  openUrl,
} from '@tauri-apps/plugin-opener';"
    ));
    assert!(contains_tauri_runtime_import(
        "const api = await import('@tauri-apps/api/window');"
    ));
    assert!(!contains_tauri_runtime_import(
        "import type { Window } from '@tauri-apps/api/window';"
    ));
    assert!(!contains_tauri_runtime_import(
        "import { type Window } from '@tauri-apps/api/window';"
    ));
    assert!(contains_tauri_runtime_import(
        "import WindowApi, { type Window } from '@tauri-apps/api/window';"
    ));
    assert!(!contains_tauri_runtime_import(
        "type WindowApi = import('@tauri-apps/api/window').Window;"
    ));

    let repository_root = crate::repo::root();
    let source_root = repository_root.join("apps/desktop/src");
    let mut observed = Vec::new();
    for entry in walkdir::WalkDir::new(&source_root).follow_links(false) {
        let entry = entry.unwrap();
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let extension = path.extension().and_then(|extension| extension.to_str());
        if !matches!(extension, Some("ts" | "tsx")) {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .unwrap_or_default();
        if file_name.ends_with(".d.ts")
            || file_name.contains(".test.")
            || file_name.contains(".spec.")
        {
            continue;
        }

        let source = fs::read_to_string(path).unwrap();
        if !contains_tauri_runtime_import(&source) {
            continue;
        }
        let relative = path
            .strip_prefix(&repository_root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let classification = classify_paths([relative.clone()]);
        assert!(
            classification.lane("package_required"),
            "direct Tauri runtime import lacks Windows package qualification: {relative}"
        );
        assert!(
            classification.lane("desktop_native_required"),
            "direct Tauri runtime import lacks Windows native validation: {relative}"
        );
        observed.push(relative);
    }

    for expected in [
        "apps/desktop/src/bridge/startupTelemetry.ts",
        "apps/desktop/src/bridge/tauriBridge.ts",
        "apps/desktop/src/platform/externalLinks.ts",
        "apps/desktop/src/platform/runtime.ts",
        "apps/desktop/src/platform/startupWindowReveal.ts",
        "apps/desktop/src/platform/systemAppearance.ts",
        "apps/desktop/src/platform/windowControls.ts",
    ] {
        assert!(
            observed.iter().any(|path| path == expected),
            "expected production Tauri runtime import was not scanned: {expected}"
        );
    }
}

#[test]
fn preserves_legacy_reason_and_mixed_path_regressions() {
    assert!(
        classify_paths(["docs/releases/v4.0.2.md".to_owned()])
            .classification_reason
            .starts_with("static-only")
    );

    for path in [
        "scripts/v4_release_pipeline.ps1",
        ".github/workflows/release-v4.yml",
    ] {
        let classification =
            classify_paths(["docs/releases/v4.0.2.md".to_owned(), path.to_owned()]);
        let expected = if path == ".github/workflows/release-v4.yml" {
            &[
                "release_required",
                "signing_proof_required",
                "malware_proof_required",
            ][..]
        } else {
            &["release_required"][..]
        };
        assert_lanes(&classification, expected);
    }

    let no_changes = classify_paths(std::iter::empty());
    assert_lanes(&no_changes, &[]);
    assert_eq!(no_changes.classification_reason, "no changed paths");
}

#[test]
fn normalizes_slashes_whitespace_and_repeated_dot_prefixes() {
    assert_lanes(&classify_paths(["  .\\./README.md  ".to_owned()]), &[]);
    assert_lanes(
        &classify_paths([".\\apps\\desktop\\src\\App.tsx".to_owned()]),
        &["desktop_required", "desktop_e2e_required"],
    );
}

#[test]
fn git_paths_are_exact_and_malformed_names_fail_closed() {
    for path in [
        "README.md ",
        "README.md\n",
        "apps\\desktop\\src\\App.tsx",
        " README.md",
    ] {
        let classification = classify_git_paths(vec![path.to_owned()], DiffMode::PullRequest);
        assert_lanes(&classification, &LANES);
        assert!(
            classification
                .classification_reason
                .starts_with("malformed changed path")
        );
    }

    assert_lanes(
        &classify_git_paths(
            vec!["apps/desktop/src/App.tsx".to_owned()],
            DiffMode::PullRequest,
        ),
        &["desktop_required", "desktop_e2e_required"],
    );
    // Manual paths-file input deliberately retains its historical normalization contract.
    assert_lanes(
        &classify_paths([" .\\apps\\desktop\\src\\App.tsx ".to_owned()]),
        &["desktop_required", "desktop_e2e_required"],
    );
}

#[test]
fn git_path_decoding_preserves_trailing_space_and_rejects_invalid_utf8() {
    let paths = decode_git_paths(b"README.md \0apps/desktop/src/App.tsx\0").unwrap();
    assert_eq!(paths, ["README.md ", "apps/desktop/src/App.tsx"]);
    assert!(decode_git_paths(b"README.md\xff\0").is_err());
}

#[test]
fn unknown_or_malformed_paths_fail_closed_and_workflow_dispatch_is_full() {
    for path in [
        "unclassified/file.txt",
        "desktop/src/App.tsx",
        "desktop/src-tauri/Cargo.toml",
        "site/package.json",
        "site/src/pages/index.astro",
        ".github/workflows/unknown.yml",
        ".github/actions/unknown/action.yml",
        "xtask/src/ci_classifier.rs",
        "xtask/src/ci.rs",
        "xtask/src/main.rs",
        "scripts/ci_classify.ps1",
        "scripts/test_ci_classify.ps1",
    ] {
        let classification = classify_paths([path.to_owned()]);
        assert_lanes(&classification, &LANES);
        assert!(
            classification
                .classification_reason
                .starts_with("unknown path")
        );
    }
    let malformed = classify_paths(["apps/desktop/src/a\nfile.tsx".to_owned()]);
    assert_lanes(&malformed, &LANES);
    assert!(
        malformed
            .classification_reason
            .starts_with("malformed changed path")
    );
    let full = Classification::full("full validation requested");
    assert_lanes(&full, &LANES);
    assert_eq!(full.classification_reason, "full validation requested");
    let dispatch = parse_arguments(&["--full".to_owned()]).unwrap();
    assert_lanes(&dispatch, &LANES);
    assert_eq!(dispatch.classification_reason, "full validation requested");
}

#[test]
fn invalid_base_head_arguments_fail_closed_and_write_all_github_outputs() {
    let root = temp_dir("sky-xtask-classifier-invalid-sha");
    let output = root.join("github-output.txt");
    let args = vec![
        "--base-sha".to_owned(),
        "0000000000000000000000000000000000000000".to_owned(),
        "--head-sha".to_owned(),
        "1234567890abcdef1234567890abcdef12345678".to_owned(),
        "--diff-mode".to_owned(),
        "pr".to_owned(),
        "--github-output".to_owned(),
        output.to_string_lossy().into_owned(),
    ];
    run(&args).unwrap();
    let contents = fs::read_to_string(&output).unwrap();
    for lane in LANES {
        assert!(contents.contains(&format!("{lane}=true\n")), "{lane}");
    }
    assert!(contents.contains("classification_reason<<"));
    assert!(contents.contains("classifier failure (fail-closed)"));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn github_output_uses_multiline_safe_encoding_for_reason() {
    let root = temp_dir("sky-xtask-github-output-test");
    let path = root.join("output.txt");
    let mut classification = Classification::new("first line\nsecond=value");
    classification.add("rust_required");
    append_github_output(&path, &classification).unwrap();
    let output = fs::read_to_string(&path).unwrap();
    assert!(output.starts_with("rust_required=true\ndesktop_required=false\n"));
    assert!(output.contains("classification_reason<<SKY_XTASK_OUTPUT_"));
    assert!(output.contains("\nfirst line\nsecond=value\nSKY_XTASK_OUTPUT_"));
    fs::remove_dir_all(root).unwrap();
}

fn git(root: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git_sha(root: &Path) -> String {
    String::from_utf8(git(root, &["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned()
}

fn file_url(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap();
    let value = canonical.to_string_lossy();
    let value = value
        .strip_prefix("\\\\?\\")
        .unwrap_or(&value)
        .replace('\\', "/");
    if cfg!(windows) {
        format!("file:///{value}")
    } else {
        format!("file://{value}")
    }
}

#[test]
fn pr_uses_merge_base_when_base_advances_and_diff_keeps_both_rename_paths() {
    let root = temp_dir("sky-xtask-classifier-merge-base");
    let source = root.join("source");
    let remote = root.join("origin.git");
    let shallow = root.join("shallow");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "--bare", "--quiet"]);
    git(&source, &["init", "--quiet"]);
    git(&source, &["config", "user.name", "Classifier Test"]);
    git(
        &source,
        &["config", "user.email", "classifier@example.invalid"],
    );
    fs::create_dir_all(source.join("apps/desktop/src")).unwrap();
    fs::write(source.join("README.md"), "base\n").unwrap();
    fs::write(
        source.join("apps/desktop/src/Old.tsx"),
        "export const Old = 1;\n",
    )
    .unwrap();
    git(&source, &["add", "--all"]);
    git(&source, &["commit", "--quiet", "-m", "common base"]);
    let merge_base = git_sha(&source);
    git(&source, &["branch", "feature"]);

    fs::write(source.join("README.md"), "main advanced\n").unwrap();
    fs::create_dir_all(source.join("apps/website")).unwrap();
    fs::write(source.join("apps/website/package.json"), "{}\n").unwrap();
    git(&source, &["add", "--all"]);
    git(&source, &["commit", "--quiet", "-m", "advance main"]);
    let base = git_sha(&source);
    git(
        &source,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(
        &source,
        &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
    );

    git(&source, &["checkout", "--quiet", "feature"]);
    git(
        &source,
        &["mv", "apps/desktop/src/Old.tsx", "apps/desktop/src/New.tsx"],
    );
    fs::write(
        source.join("apps/desktop/src/App.tsx"),
        "export const App = 1;\n",
    )
    .unwrap();
    git(&source, &["add", "--all"]);
    git(&source, &["commit", "--quiet", "-m", "change feature UI"]);
    let head = git_sha(&source);
    git(
        &source,
        &["push", "--quiet", "origin", "HEAD:refs/heads/feature"],
    );
    git(
        &source,
        &["push", "--quiet", "origin", "HEAD:refs/pull/17/head"],
    );

    let clone = Command::new("git")
        .args([
            "clone",
            "--quiet",
            "--depth=1",
            "--branch",
            "main",
            file_url(&remote).as_str(),
            shallow.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        clone.status.success(),
        "shallow clone failed: {}",
        String::from_utf8_lossy(&clone.stderr)
    );
    git(
        &shallow,
        &["fetch", "--quiet", "--depth=1", "origin", "feature"],
    );
    git(
        &shallow,
        &["checkout", "--quiet", "--detach", head.as_str()],
    );
    let base_expression = format!("{merge_base}^{{commit}}");
    assert!(
        !git_output(&shallow, &["cat-file", "-e", base_expression.as_str()])
            .unwrap()
            .status
            .success()
    );

    let pr_refs = PullRequestRefs {
        number: 17,
        base_ref: "main".to_owned(),
    };
    let paths = changed_paths_from_git(
        &shallow,
        DiffMode::PullRequest,
        &base,
        &head,
        Some(&pr_refs),
    )
    .unwrap();
    assert_eq!(
        paths,
        [
            "apps/desktop/src/App.tsx",
            "apps/desktop/src/New.tsx",
            "apps/desktop/src/Old.tsx",
        ]
    );
    assert!(!paths.iter().any(|path| path == "apps/website/package.json"));
    assert_lanes(
        &classify_git_range_with_refs(
            &shallow,
            DiffMode::PullRequest,
            &base,
            &head,
            Some(&pr_refs),
        ),
        &["desktop_required", "desktop_e2e_required"],
    );
    assert_lanes(
        &classify_git_range(&source, DiffMode::Push, &merge_base, &head),
        &[
            "desktop_required",
            "desktop_e2e_required",
            "package_required",
        ],
    );
    assert!(
        git(&shallow, &["rev-parse", "--abbrev-ref", "HEAD"])
            .stdout
            .starts_with(b"HEAD")
    );

    let mismatch = classify_git_range(&source, DiffMode::PullRequest, &base, &merge_base);
    assert_lanes(&mismatch, &LANES);
    assert!(
        mismatch
            .classification_reason
            .contains("does not match checked-out source HEAD")
    );
    let empty = classify_git_range(&source, DiffMode::Push, &head, &head);
    assert_lanes(&empty, &LANES);
    assert!(
        empty
            .classification_reason
            .contains("empty changed-path diff")
    );
    let unavailable = classify_git_range(
        &source,
        DiffMode::Push,
        "1234567890abcdef1234567890abcdef12345678",
        &head,
    );
    assert_lanes(&unavailable, &LANES);
    assert!(
        unavailable
            .classification_reason
            .contains("could not be fetched")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fork_head_uses_bounded_pull_request_ref_and_keeps_frontend_pr_narrow() {
    let root = temp_dir("sky-xtask-classifier-fork-pr-ref");
    let source = root.join("source");
    let origin = root.join("origin.git");
    let fork = root.join("fork.git");
    let checkout = root.join("checkout");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&origin).unwrap();
    fs::create_dir(&fork).unwrap();
    git(&origin, &["init", "--bare", "--quiet"]);
    git(&fork, &["init", "--bare", "--quiet"]);
    git(&source, &["init", "--quiet", "--initial-branch=main"]);
    git(&source, &["config", "user.name", "Classifier Test"]);
    git(
        &source,
        &["config", "user.email", "classifier@example.invalid"],
    );
    git(
        &source,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&source, &["remote", "add", "fork", fork.to_str().unwrap()]);
    fs::write(source.join("README.md"), "fork point\n").unwrap();
    git(&source, &["add", "--all"]);
    git(&source, &["commit", "--quiet", "-m", "fork point"]);
    let fork_point = git_sha(&source);
    git(
        &source,
        &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
    );
    git(
        &source,
        &["push", "--quiet", "fork", "HEAD:refs/heads/main"],
    );

    git(&source, &["checkout", "--quiet", "-b", "feature"]);
    fs::create_dir_all(source.join("apps/desktop/src")).unwrap();
    fs::write(
        source.join("apps/desktop/src/App.tsx"),
        "export const App = 2;\n",
    )
    .unwrap();
    git(&source, &["add", "--all"]);
    git(&source, &["commit", "--quiet", "-m", "fork UI change"]);
    let head = git_sha(&source);
    git(
        &source,
        &["push", "--quiet", "fork", "HEAD:refs/heads/feature"],
    );
    git(
        &source,
        &["push", "--quiet", "origin", "HEAD:refs/pull/29/head"],
    );

    git(&source, &["checkout", "--quiet", "main"]);
    fs::write(source.join("README.md"), "main advanced\n").unwrap();
    fs::create_dir_all(source.join("apps/website")).unwrap();
    fs::write(source.join("apps/website/package.json"), "{}\n").unwrap();
    git(&source, &["add", "--all"]);
    git(
        &source,
        &["commit", "--quiet", "-m", "advance upstream after fork"],
    );
    let base = git_sha(&source);
    git(
        &source,
        &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
    );

    let clone = Command::new("git")
        .args([
            "clone",
            "--quiet",
            "--depth=1",
            "--branch",
            "main",
            file_url(&origin).as_str(),
            checkout.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        clone.status.success(),
        "shallow clone failed: {}",
        String::from_utf8_lossy(&clone.stderr)
    );
    git(
        &checkout,
        &["remote", "add", "fork", fork.to_str().unwrap()],
    );
    git(
        &checkout,
        &[
            "fetch",
            "--quiet",
            "--depth=1",
            "fork",
            "refs/heads/feature:refs/remotes/fork/feature",
        ],
    );
    git(
        &checkout,
        &["checkout", "--quiet", "--detach", head.as_str()],
    );

    let advertised = git(&checkout, &["ls-remote", "--heads", "origin"]);
    assert!(!String::from_utf8_lossy(&advertised.stdout).contains(&head));
    assert!(!String::from_utf8_lossy(&advertised.stdout).contains("feature"));
    let pr_refs = PullRequestRefs {
        number: 29,
        base_ref: "main".to_owned(),
    };
    let (base_refspec, pr_head_refspec) = pr_refs.fetch_refspecs(&checkout).unwrap();
    assert_eq!(base_refspec, "+refs/heads/main:refs/sky-ci/base");
    assert_eq!(pr_head_refspec, "+refs/pull/29/head:refs/sky-ci/pr-head");
    assert!(!base_refspec.contains(&head));
    assert!(!pr_head_refspec.contains(&head));

    let paths = changed_paths_from_git(
        &checkout,
        DiffMode::PullRequest,
        &base,
        &head,
        Some(&pr_refs),
    )
    .unwrap();
    assert_eq!(paths, ["apps/desktop/src/App.tsx"]);
    assert_ne!(fork_point, base);
    assert_lanes(
        &classify_git_range_with_refs(
            &checkout,
            DiffMode::PullRequest,
            &base,
            &head,
            Some(&pr_refs),
        ),
        &["desktop_required", "desktop_e2e_required"],
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn shallow_history_without_a_merge_base_fails_closed_after_bounded_fetches() {
    let root = temp_dir("sky-xtask-classifier-no-merge-base");
    let main_repo = root.join("main-repo");
    let other_repo = root.join("other-repo");
    let remote = root.join("origin.git");
    let shallow = root.join("shallow");
    fs::create_dir(&main_repo).unwrap();
    fs::create_dir(&other_repo).unwrap();
    fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "--bare", "--quiet"]);

    git(&main_repo, &["init", "--quiet"]);
    git(&main_repo, &["config", "user.name", "Classifier Test"]);
    git(
        &main_repo,
        &["config", "user.email", "classifier@example.invalid"],
    );
    fs::write(main_repo.join("main.txt"), "main\n").unwrap();
    git(&main_repo, &["add", "--all"]);
    git(&main_repo, &["commit", "--quiet", "-m", "main root"]);
    fs::write(main_repo.join("main.txt"), "main advance\n").unwrap();
    git(&main_repo, &["commit", "--quiet", "-am", "advance main"]);
    let base = git_sha(&main_repo);
    git(
        &main_repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(
        &main_repo,
        &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
    );

    git(&other_repo, &["init", "--quiet"]);
    git(&other_repo, &["config", "user.name", "Classifier Test"]);
    git(
        &other_repo,
        &["config", "user.email", "classifier@example.invalid"],
    );
    fs::write(other_repo.join("other.txt"), "unrelated history\n").unwrap();
    git(&other_repo, &["add", "--all"]);
    git(&other_repo, &["commit", "--quiet", "-m", "unrelated root"]);
    let head = git_sha(&other_repo);
    git(
        &other_repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(
        &other_repo,
        &["push", "--quiet", "origin", "HEAD:refs/heads/independent"],
    );
    git(
        &other_repo,
        &["push", "--quiet", "origin", "HEAD:refs/pull/17/head"],
    );

    let clone = Command::new("git")
        .args([
            "clone",
            "--quiet",
            "--depth=1",
            "--branch",
            "main",
            file_url(&remote).as_str(),
            shallow.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        clone.status.success(),
        "shallow clone failed: {}",
        String::from_utf8_lossy(&clone.stderr)
    );
    git(
        &shallow,
        &["fetch", "--quiet", "--depth=1", "origin", "independent"],
    );
    git(
        &shallow,
        &["checkout", "--quiet", "--detach", head.as_str()],
    );
    let pr_refs = PullRequestRefs {
        number: 17,
        base_ref: "main".to_owned(),
    };
    let classification = classify_git_range_with_refs(
        &shallow,
        DiffMode::PullRequest,
        &base,
        &head,
        Some(&pr_refs),
    );
    assert_lanes(&classification, &LANES);
    assert!(
        classification
            .classification_reason
            .contains("bounded shallow-history fetch attempts")
    );
    assert_eq!(SHALLOW_DEEPEN_STEPS.len(), 4);
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn empty_paths_are_static_and_path_normalization_matches_powershell_inputs() {
    assert_eq!(
        classify_paths(Vec::<String>::new()).classification_reason,
        "no changed paths"
    );
    let normalized = classify_paths([" ./apps\\desktop\\src\\App.tsx ".to_owned()]);
    assert_lanes(&normalized, &["desktop_required", "desktop_e2e_required"]);
    assert!(
        normalized
            .classification_reason
            .ends_with("apps/desktop/src/App.tsx")
    );
}
