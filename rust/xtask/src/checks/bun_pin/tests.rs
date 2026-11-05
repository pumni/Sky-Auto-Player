use super::*;
use crate::ci_artifacts;
use std::fs;
use std::path::PathBuf;

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
const VERSION: &str = "9.8.7\n";
const PACKAGE: &str = r#"{"name":"fixture","packageManager":"bun@9.8.7"}"#;
const VALID_STEP: &str = "jobs:\n  verify:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Set up Bun\n        uses: oven-sh/setup-bun@0123456789abcdef0123456789abcdef01234567 # v2\n        with:\n          bun-version-file: .bun-version\n      - name: Verify Bun pin\n        run: bun scripts/verify-bun-pin.mjs\n";
const VALID_COMPOSITE: &str = "runs:\n  using: composite\n  steps:\n    - name: Set up Bun\n      uses: oven-sh/setup-bun@0123456789abcdef0123456789abcdef01234567 # v2\n      with:\n        bun-version-file: .bun-version\n    - name: Verify Bun pin\n      shell: bash\n      run: bun scripts/verify-bun-pin.mjs\n";

fn fixture_root() -> PathBuf {
    let path = ci_artifacts::unique_sibling(&std::env::temp_dir().join("sky-bun-pin"), "test");
    fs::create_dir_all(path.join(".github/workflows")).unwrap();
    fs::create_dir_all(path.join(".github/actions/site-validate")).unwrap();
    fs::create_dir_all(path.join("site")).unwrap();
    fs::create_dir_all(path.join("desktop")).unwrap();
    fs::write(path.join(BUN_VERSION_FILE), VERSION).unwrap();
    fs::write(path.join("desktop/package.json"), r#"{"name":"desktop"}"#).unwrap();
    fs::write(path.join("desktop/bun.lock"), "{}").unwrap();
    fs::write(path.join("site/bun.lock"), "{}").unwrap();
    fs::write(path.join(SITE_PACKAGE), PACKAGE).unwrap();
    path
}

fn write_workflow(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

fn assert_rejected(source: &str, name: &str) {
    assert!(
        validate_yaml_source(source, Path::new(name)).is_err(),
        "{name}"
    );
}

#[test]
fn accepts_exact_root_pin_mirror_and_sha_pinned_file_input() {
    let version = canonical_bun_version(VERSION).unwrap();
    assert_eq!(version.to_string(), "9.8.7");
    assert_eq!(mirrored_bun_version(PACKAGE).unwrap(), version);
    assert_eq!(
        validate_yaml_source(VALID_STEP, Path::new("fixture.yml")).unwrap(),
        1
    );
}

#[test]
fn rejects_malformed_root_pin_and_mismatched_site_mirror() {
    for source in ["stable\n", "9.8\n", "09.8.7\n", "9.8.7\n\n"] {
        assert!(canonical_bun_version(source).is_err(), "{source:?}");
    }

    let root = fixture_root();
    write_workflow(&root, ".github/workflows/ci.yml", VALID_STEP);
    fs::write(
        root.join(SITE_PACKAGE),
        r#"{"name":"fixture","packageManager":"bun@9.8.8"}"#,
    )
    .unwrap();
    assert!(validate_repository(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_missing_app_locks_and_root_workspace_artifacts() {
    let root = fixture_root();
    write_workflow(&root, ".github/workflows/ci.yml", VALID_STEP);
    assert!(validate_repository(&root).is_ok());

    for path in ["desktop/bun.lock", "site/bun.lock"] {
        fs::remove_file(root.join(path)).unwrap();
        assert!(validate_repository(&root).is_err(), "{path}");
        fs::write(root.join(path), "{}").unwrap();
    }

    fs::write(root.join("bun.lock"), "{}").unwrap();
    assert!(validate_repository(&root).is_err());
    fs::remove_file(root.join("bun.lock")).unwrap();

    fs::write(root.join("bun.lockb"), "{}").unwrap();
    assert!(validate_repository(&root).is_err());
    fs::remove_file(root.join("bun.lockb")).unwrap();

    fs::write(
        root.join("package.json"),
        r#"{"workspaces":["desktop","site"]}"#,
    )
    .unwrap();
    assert!(validate_repository(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_missing_floating_duplicate_and_ambiguous_package_manager() {
    for source in [
        r#"{"name":"fixture"}"#,
        r#"{"name":"fixture","packageManager":"bun@latest"}"#,
        r#"{"name":"fixture","packageManager":"bun@9.8"}"#,
        r#"{"packageManager":"bun@9.8.7","packageManager":"bun@9.8.8"}"#,
        r#"{"nested":{"packageManager":"bun@9.8.7"},"packageManager":"bun@9.8.7"}"#,
    ] {
        assert!(mirrored_bun_version(source).is_err(), "{source}");
    }
}

#[test]
fn rejects_missing_wrong_literal_and_implicit_latest_inputs() {
    for (name, source) in [
        (
            "missing.yml",
            VALID_STEP.replace("          bun-version-file: .bun-version\n", ""),
        ),
        (
            "wrong.yml",
            VALID_STEP.replace(
                "bun-version-file: .bun-version",
                "bun-version-file: site/package.json",
            ),
        ),
        (
            "literal.yml",
            VALID_STEP.replace(
                "          bun-version-file: .bun-version",
                "          bun-version: 9.8.7\n          bun-version-file: .bun-version",
            ),
        ),
        (
            "latest.yml",
            VALID_STEP.replace(
                "          bun-version-file: .bun-version",
                "          bun-version: latest",
            ),
        ),
    ] {
        assert_rejected(&source, name);
    }
}

#[test]
fn rejects_unpinned_and_unrecognized_new_consumers() {
    let pinned_reference = format!("@{SHA}");
    let unpinned = VALID_STEP.replace(&pinned_reference, "@v2");
    assert_rejected(&unpinned, "tag.yml");

    let inline = VALID_STEP.replace(
        "        uses: oven-sh/setup-bun@",
        "      - uses: oven-sh/setup-bun@",
    );
    assert_rejected(&inline, "inline.yml");
}

#[test]
fn rejects_conditionally_skipped_or_failure_ignored_verifier() {
    for control in ["if: false", "continue-on-error: true"] {
        let source = VALID_STEP.replace(
            "        run: bun scripts/verify-bun-pin.mjs",
            &format!("        {control}\n        run: bun scripts/verify-bun-pin.mjs"),
        );
        assert_rejected(&source, "verification-bypass.yml");
    }
}

#[test]
fn rejects_duplicate_ambiguous_and_unsupported_verifier_definitions() {
    let run = "        run: bun scripts/verify-bun-pin.mjs";
    for source in [
        VALID_STEP.replace(run, &format!("{run}\n{run}")),
        VALID_STEP.replace(
            run,
            &format!("{run}\n      - name: Duplicate verifier\n{run}"),
        ),
        VALID_STEP.replace(run, "        uses: local/verify-bun-pin@v1"),
        VALID_STEP.replace(
            run,
            "        run: |\n          bun scripts/verify-bun-pin.mjs",
        ),
    ] {
        assert_rejected(&source, "ambiguous-verifier.yml");
    }
}

#[test]
fn rejects_verifier_in_a_different_job() {
    let source = "jobs:\n  setup:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Set up Bun\n        uses: oven-sh/setup-bun@0123456789abcdef0123456789abcdef01234567 # v2\n        with:\n          bun-version-file: .bun-version\n  verify:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Verify Bun pin\n        run: bun scripts/verify-bun-pin.mjs\n";
    let error = validate_yaml_source(source, Path::new("two-jobs.yml")).unwrap_err();
    assert!(error.to_string().contains("must be followed immediately"));
}

#[test]
fn repository_scan_discovers_new_workflows_and_composite_actions() {
    let root = fixture_root();
    write_workflow(&root, ".github/workflows/first.yml", VALID_STEP);
    write_workflow(
        &root,
        ".github/actions/site-validate/action.yml",
        VALID_COMPOSITE,
    );
    assert_eq!(validate_repository(&root).unwrap(), 2);

    write_workflow(
        &root,
        ".github/workflows/new-consumer.yml",
        &VALID_STEP.replace(".bun-version", "site/package.json"),
    );
    assert!(validate_repository(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}
