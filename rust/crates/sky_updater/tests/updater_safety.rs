use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sky_updater::archive::sha256_bytes;
use sky_updater::error::UpdaterError;
use sky_updater::install::install_verified;
use sky_updater::manifest::{Manifest, ManifestFile};
use sky_updater::recovery::rollback_prepared;
use sky_updater::transaction::{
    JournalState, build_plan, prepare_journal, read_journal, transaction_root, write_json_atomic,
};
use sky_updater::update_lock::UpdateLock;
use sky_updater::{
    APP_NAME, CALIBRATION_EXE, MANIFEST_NAME, PRIMARY_EXE, SCHEMA_VERSION, UPDATER_EXE,
};

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "sky-updater-safety-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
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
        build_time_utc: "2026-08-18T00:00:00Z".into(),
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
    fs::create_dir_all(path.parent().expect("parent")).expect("parent");
    fs::write(path, bytes).expect("file");
}

fn write_manifest(root: &Path, manifest: &Manifest) {
    write_file(
        root,
        MANIFEST_NAME,
        &serde_json::to_vec(manifest).expect("manifest JSON"),
    );
}

type FixtureFiles = [(&'static str, &'static [u8]); 5];

fn seed_install(root: &Path, staging: &Path) -> (Manifest, Manifest, FixtureFiles, FixtureFiles) {
    let old_files = [
        ("README.md", b"old readme".as_slice()),
        (UPDATER_EXE, b"old updater".as_slice()),
        (PRIMARY_EXE, b"old app".as_slice()),
        (CALIBRATION_EXE, b"old calibration".as_slice()),
        ("old.dll", b"old orphan".as_slice()),
    ];
    let new_files = [
        ("README.md", b"new readme".as_slice()),
        (UPDATER_EXE, b"new updater".as_slice()),
        (PRIMARY_EXE, b"new app".as_slice()),
        (CALIBRATION_EXE, b"new calibration".as_slice()),
        ("new.dll", b"new addition".as_slice()),
    ];
    for (path, bytes) in old_files {
        write_file(root, path, bytes);
    }
    for (path, bytes) in new_files {
        write_file(staging, path, bytes);
    }
    let old = manifest("3.3.0", &old_files);
    let new = manifest("3.4.0", &new_files);
    write_manifest(root, &old);
    write_manifest(staging, &new);
    (old, new, old_files, new_files)
}

#[test]
fn concurrent_updater_is_rejected_before_transaction_creation() {
    let root = temp_root("lock-install");
    let local = root.join("local");
    let install = root.join("install");
    fs::create_dir_all(&install).expect("install");
    let previous = std::env::var_os("LOCALAPPDATA");
    unsafe { std::env::set_var("LOCALAPPDATA", &local) };

    let first = UpdateLock::acquire(&install).expect("first updater lock");
    let second = UpdateLock::acquire(&install).expect_err("second updater must be rejected");
    assert!(matches!(second, UpdaterError::UpdateAlreadyRunning));
    assert!(!transaction_root(&install).exists());
    drop(first);
    let third = UpdateLock::acquire(&install).expect("lock is released after RAII drop");
    drop(third);

    match previous {
        Some(value) => unsafe { std::env::set_var("LOCALAPPDATA", value) },
        None => unsafe { std::env::remove_var("LOCALAPPDATA") },
    }
    fs::remove_dir_all(root).expect("cleanup");
}

#[cfg(feature = "e2e-fault-injection")]
mod fault_regressions {
    use super::*;
    use sky_updater::faults;
    use std::sync::{Mutex, OnceLock};

    static FAULT_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn fault_test_guard() -> std::sync::MutexGuard<'static, ()> {
        FAULT_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn exact_readme_updater_primary_failure_preserves_updater_and_recovers() {
        let _guard = fault_test_guard();
        let root = temp_root("exact-order");
        let staging = root.join("staging");
        fs::create_dir_all(&root).expect("root");
        fs::create_dir_all(&staging).expect("staging");
        let (old, new, old_files, new_files) = seed_install(&root, &staging);
        let plan = build_plan(Some(&old), &new).expect("plan");
        prepare_journal(&root, &plan).expect("journal");
        let mut journal = read_journal(&root).expect("journal reads");
        journal.new_paths = vec![
            "README.md".into(),
            UPDATER_EXE.into(),
            PRIMARY_EXE.into(),
            CALIBRATION_EXE.into(),
            MANIFEST_NAME.into(),
            "new.dll".into(),
        ];
        write_json_atomic(&transaction_root(&root).join("journal.json"), &journal)
            .expect("rewrite exact journal order");
        assert_eq!(
            read_journal(&root).expect("journal").state,
            JournalState::Prepared
        );

        for (path, bytes) in new_files {
            write_file(&root, path, bytes);
        }
        // The new manifest is a managed payload and is part of the simulated
        // partially applied installation.
        write_manifest(&root, &new);

        faults::configure(Some("rollback:before-replace:Sky-Auto-Player.exe"), None)
            .expect("fault config");
        let error = rollback_prepared(&root).expect_err("primary restore fault");
        assert!(matches!(
            error,
            UpdaterError::RollbackAtomicReplaceFailed { .. }
        ));
        assert_eq!(
            fs::read(root.join(UPDATER_EXE)).expect("updater"),
            b"old updater"
        );
        assert_eq!(
            fs::read(root.join(PRIMARY_EXE)).expect("primary"),
            b"new app"
        );
        assert!(transaction_root(&root).exists());

        faults::configure(None, None).expect("clear fault");
        rollback_prepared(&root).expect("second recovery");
        for (path, bytes) in old_files {
            assert_eq!(
                fs::read(root.join(path)).expect("old file"),
                bytes,
                "{path}"
            );
        }
        assert!(!root.join("new.dll").exists());
        assert!(!transaction_root(&root).exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn apply_failure_rolls_back_without_delete_sweep() {
        let _guard = fault_test_guard();
        let root = temp_root("apply-fault");
        let staging = root.join("staging");
        fs::create_dir_all(&root).expect("root");
        fs::create_dir_all(&staging).expect("staging");
        let (old, new, old_files, _) = seed_install(&root, &staging);

        faults::configure(
            Some("apply:after-replace:Sky-Auto-Player-Updater.exe"),
            None,
        )
        .expect("fault config");
        let error = install_verified(&root, &staging, &new, &old).expect_err("primary apply fault");
        assert!(matches!(error, UpdaterError::InstallCopyFailed(_)));
        assert!(transaction_root(&root).exists());
        assert!(root.join(UPDATER_EXE).is_file());
        faults::configure(None, None).expect("clear fault");
        rollback_prepared(&root).expect("rollback");
        for (path, bytes) in old_files {
            assert_eq!(
                fs::read(root.join(path)).expect("old file"),
                bytes,
                "{path}"
            );
        }
        assert!(!transaction_root(&root).exists());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn rollback_fault_keeps_transaction_and_current_files() {
        let _guard = fault_test_guard();
        let root = temp_root("rollback-fault");
        let staging = root.join("staging");
        fs::create_dir_all(&root).expect("root");
        fs::create_dir_all(&staging).expect("staging");
        let (old, new, _, new_files) = seed_install(&root, &staging);
        let plan = build_plan(Some(&old), &new).expect("plan");
        prepare_journal(&root, &plan).expect("journal");
        for (path, bytes) in new_files {
            write_file(&root, path, bytes);
        }
        write_manifest(&root, &new);

        faults::configure(
            Some("rollback:after-restore:Sky-Auto-Player-Updater.exe"),
            None,
        )
        .expect("fault config");
        assert!(rollback_prepared(&root).is_err());
        assert_eq!(
            fs::read(root.join(UPDATER_EXE)).expect("updater"),
            b"old updater"
        );
        assert!(root.join(PRIMARY_EXE).is_file());
        assert!(transaction_root(&root).exists());
        faults::configure(None, None).expect("clear fault");
        rollback_prepared(&root).expect("recovery after injected fault");
        assert!(!transaction_root(&root).exists());
        fs::remove_dir_all(root).expect("cleanup");
    }
}

#[cfg(windows)]
#[test]
fn locked_primary_fails_preflight_without_transaction_or_mutation() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING,
    };

    let root = temp_root("locked-primary");
    let staging = root.join("staging");
    fs::create_dir_all(&root).expect("root");
    fs::create_dir_all(&staging).expect("staging");
    let (old, new, old_files, _) = seed_install(&root, &staging);
    let primary = root.join(PRIMARY_EXE);
    let wide = primary
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(handle, INVALID_HANDLE_VALUE, "lock fixture handle");
    let error = install_verified(&root, &staging, &new, &old).expect_err("busy primary");
    assert!(matches!(error, UpdaterError::InstallTargetBusy { .. }));
    assert!(!transaction_root(&root).exists());
    unsafe { CloseHandle(handle) };
    for (path, bytes) in old_files {
        assert_eq!(
            fs::read(root.join(path)).expect("old file"),
            bytes,
            "{path}"
        );
    }
    fs::remove_dir_all(root).expect("cleanup");
}
