use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sky_updater::archive::sha256_bytes;
use sky_updater::error::UpdaterError;
use sky_updater::install::install_verified;
use sky_updater::manifest::{Manifest, ManifestFile};
use sky_updater::progress::NoopProgressSink;
use sky_updater::recovery::rollback_prepared;
use sky_updater::result::{self, UpdateResult};
use sky_updater::transaction::{build_plan, cleanup_committed, prepare_journal, transaction_root};
use sky_updater::{APP_NAME, MANIFEST_NAME, PRIMARY_EXE, SCHEMA_VERSION, UPDATER_EXE};

const CALIBRATION_EXE: &str = "native_calibration.exe";
const PRESERVED_FILES: [(&str, &[u8]); 5] = [
    ("config.json", br#"{"theme":"aurora"}"#),
    (".env", b"SKY_TEST_SECRET=not-a-release-secret"),
    ("songs/user.skysheet", b"user song data"),
    ("logs/old.log", b"user log data"),
    ("unknown-user-file.txt", b"unknown file data"),
];

struct LocalAppDataGuard {
    previous: Option<OsString>,
}

impl LocalAppDataGuard {
    fn set(path: &Path) -> Self {
        let previous = std::env::var_os("LOCALAPPDATA");
        // Rust 2024 marks environment mutation unsafe because concurrent code
        // may observe the process-wide environment. This test is intentionally
        // single-threaded around its result-file assertions.
        unsafe { std::env::set_var("LOCALAPPDATA", path) };
        Self { previous }
    }
}

impl Drop for LocalAppDataGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var("LOCALAPPDATA", value) },
            None => unsafe { std::env::remove_var("LOCALAPPDATA") },
        }
    }
}

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "sky-updater-e2e-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos()
    ))
}

fn manifest(version: &str, files: &[(&str, &[u8])]) -> Manifest {
    Manifest {
        schema_version: SCHEMA_VERSION,
        app: APP_NAME.into(),
        version: version.into(),
        executable: PRIMARY_EXE.into(),
        git_head: "a".repeat(40),
        dirty_worktree: false,
        native_build_commit: "b".repeat(40),
        build_time_utc: "2026-08-09T00:00:00Z".into(),
        files: files
            .iter()
            .map(|(path, bytes)| ManifestFile {
                path: (*path).into(),
                size: bytes.len() as u64,
                sha256: sha256_bytes(bytes),
            })
            .collect(),
    }
}

fn write_file(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("file parent")).expect("file parent");
    fs::write(path, bytes).expect("write fixture file");
}

fn write_manifest(root: &Path, manifest: &Manifest) {
    write_file(
        root,
        MANIFEST_NAME,
        &serde_json::to_vec_pretty(manifest).expect("serialize manifest"),
    );
}

fn seed_preserved_files(root: &Path) {
    for (path, bytes) in PRESERVED_FILES {
        write_file(root, path, bytes);
    }
}

fn assert_files(root: &Path, files: &[(&str, &[u8])]) {
    for (path, expected) in files {
        assert_eq!(
            fs::read(root.join(path)).expect("read fixture file"),
            *expected,
            "{path}"
        );
    }
}

fn assert_preserved_files(root: &Path) {
    assert_files(root, &PRESERVED_FILES);
}

fn read_result() -> UpdateResult {
    let path = result::result_dir()
        .expect("result directory")
        .join("last-result.json");
    serde_json::from_slice(&fs::read(path).expect("result file")).expect("valid result JSON")
}

#[test]
fn packaged_update_and_injected_failure_rollback_preserve_user_state() {
    let result_root = temp_root("result");
    let _local_app_data = LocalAppDataGuard::set(&result_root);

    let a_files: [(&str, &[u8]); 4] = [
        (PRIMARY_EXE, b"A app"),
        (UPDATER_EXE, b"A updater"),
        (CALIBRATION_EXE, b"A calibration"),
        ("old.dll", b"A old managed orphan"),
    ];
    let b_files: [(&str, &[u8]); 4] = [
        (PRIMARY_EXE, b"B app"),
        (UPDATER_EXE, b"B updater"),
        (CALIBRATION_EXE, b"B calibration"),
        ("new.dll", b"B new managed file"),
    ];
    // Controlled previous-stable -> v4-candidate fixture. The final portable
    // package remains a Phase 8 qualification concern; this test exercises the
    // existing transaction contract against the architecture's target layout.
    let manifest_a = manifest("3.4.5", &a_files);
    let manifest_b = manifest("3.5.0", &b_files);

    let install_a = temp_root("install-a");
    let staging_b = temp_root("staging-b");
    fs::create_dir_all(&install_a).expect("install A");
    fs::create_dir_all(&staging_b).expect("staging B");
    for (path, bytes) in a_files {
        write_file(&install_a, path, bytes);
    }
    seed_preserved_files(&install_a);
    write_manifest(&install_a, &manifest_a);
    for (path, bytes) in b_files {
        write_file(&staging_b, path, bytes);
    }
    write_manifest(&staging_b, &manifest_b);
    manifest_b
        .verify_staged(&staging_b)
        .expect("staged B payload");

    install_verified(
        &install_a,
        &staging_b,
        &manifest_b,
        &manifest_a,
        &NoopProgressSink,
    )
    .expect("A to B installation");
    assert_files(&install_a, &b_files);
    assert!(
        !install_a.join("old.dll").exists(),
        "managed orphan removed"
    );
    assert_preserved_files(&install_a);
    assert_eq!(
        Manifest::parse(&fs::read(install_a.join(MANIFEST_NAME)).expect("installed manifest"))
            .expect("installed manifest parses"),
        manifest_b
    );
    result::write_result(&result::success("3.4.5", "3.5.0")).expect("success result");
    assert_eq!(read_result().status, "success");
    cleanup_committed(&install_a).expect("cleanup committed transaction");
    assert!(!transaction_root(&install_a).exists());

    let install_rollback = temp_root("install-rollback");
    fs::create_dir_all(&install_rollback).expect("rollback install");
    for (path, bytes) in a_files {
        write_file(&install_rollback, path, bytes);
    }
    seed_preserved_files(&install_rollback);
    write_manifest(&install_rollback, &manifest_a);

    let plan = build_plan(Some(&manifest_a), &manifest_b).expect("rollback plan");
    prepare_journal(&install_rollback, &plan, &NoopProgressSink).expect("rollback journal");
    // Inject the failure after mutation has started: the B files and manifest
    // are present, the old orphan is removed, and recovery must restore A.
    for (path, bytes) in b_files {
        write_file(&install_rollback, path, bytes);
    }
    write_manifest(&install_rollback, &manifest_b);
    fs::remove_file(install_rollback.join("old.dll")).expect("remove old managed file");

    rollback_prepared(&install_rollback).expect("rollback to A");
    assert_files(&install_rollback, &a_files);
    assert!(
        !install_rollback.join("new.dll").exists(),
        "new managed file removed"
    );
    assert_preserved_files(&install_rollback);
    assert_eq!(
        Manifest::parse(
            &fs::read(install_rollback.join(MANIFEST_NAME)).expect("rolled-back manifest")
        )
        .expect("rolled-back manifest parses"),
        manifest_a
    );
    let injected = UpdaterError::InstallCopyFailed("injected failure after mutation".into());
    result::write_result(&result::rolled_back("3.4.5", "3.5.0", &injected))
        .expect("rollback result");
    assert_eq!(read_result().status, "rolled_back");
    assert!(!transaction_root(&install_rollback).exists());

    fs::remove_dir_all(&install_a).expect("cleanup A");
    fs::remove_dir_all(&staging_b).expect("cleanup staging");
    fs::remove_dir_all(&install_rollback).expect("cleanup rollback");
    fs::remove_dir_all(&result_root).expect("cleanup result");
}
