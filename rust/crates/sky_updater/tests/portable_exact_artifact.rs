//! Exact portable artifact qualification.
//!
//! The normal updater tests use small synthetic payloads.  When
//! `SKY_PORTABLE_ARTIFACT_DIR` is supplied, this test consumes the exact ZIP,
//! sidecar, external manifest, and embedded manifest emitted by the portable
//! assembler, then runs the real install/rollback transaction against a
//! 3.4.5-style install.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{SystemTime, UNIX_EPOCH};

use sky_updater::archive::{extract_zip_file, sha256_bytes, sha256_file};
use sky_updater::error::Result;
use sky_updater::github::ReleaseSource;
use sky_updater::install::{install_verified, read_staged_manifest};
use sky_updater::local_source::LocalReleaseSource;
use sky_updater::manifest::{Manifest, ManifestFile, PreserveClass, classify_preserved};
use sky_updater::progress::NoopProgressSink;
use sky_updater::recovery::{has_unresolved_transaction, recover_before_update, rollback_prepared};
use sky_updater::transaction::verify_installed_managed;
use sky_updater::transaction::{build_plan, prepare_journal};
use sky_updater::{
    APP_NAME, CALIBRATION_EXE, MANIFEST_NAME, PRIMARY_EXE, SCHEMA_VERSION, UPDATER_EXE,
};

const PREVIOUS_VERSION: &str = "3.4.5";
const TARGET_VERSION: &str = "3.5.0";

struct EnvGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &Path) -> Self {
        let previous = std::env::var_os(key);
        // These integration tests run single-threaded because the updater
        // contract deliberately uses process-wide LOCALAPPDATA state.
        unsafe { std::env::set_var(key, value) };
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "sky-portable-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ))
}

fn write_file(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("file parent")).expect("parent");
    fs::write(path, bytes).expect("fixture file");
}

fn write_manifest(root: &Path, manifest: &Manifest) {
    write_file(
        root,
        MANIFEST_NAME,
        &serde_json::to_vec_pretty(manifest).expect("manifest JSON"),
    );
}

fn old_manifest() -> Manifest {
    let files = [
        (PRIMARY_EXE, b"previous Tauri replacement".as_slice()),
        (UPDATER_EXE, b"previous updater".as_slice()),
        (CALIBRATION_EXE, b"previous calibration".as_slice()),
        (
            "Sky-Auto-Player-Core.exe",
            b"previous Python Core".as_slice(),
        ),
        (
            "_internal/python314.dll",
            b"previous CPython runtime".as_slice(),
        ),
        ("Sky-Player.exe", b"obsolete v3 identity".as_slice()),
    ];
    Manifest {
        schema_version: SCHEMA_VERSION,
        app: APP_NAME.into(),
        version: PREVIOUS_VERSION.into(),
        executable: PRIMARY_EXE.into(),
        git_head: "a".repeat(40),
        dirty_worktree: false,
        native_build_commit: "b".repeat(40),
        build_time_utc: "2026-08-30T00:00:00Z".into(),
        files: files
            .into_iter()
            .map(|(path, bytes)| ManifestFile {
                path: path.into(),
                size: bytes.len() as u64,
                sha256: sha256_bytes(bytes),
            })
            .collect(),
    }
}

fn old_file_bytes(path: &str) -> &'static [u8] {
    match path {
        PRIMARY_EXE => b"previous Tauri replacement",
        UPDATER_EXE => b"previous updater",
        CALIBRATION_EXE => b"previous calibration",
        "Sky-Auto-Player-Core.exe" => b"previous Python Core",
        "_internal/python314.dll" => b"previous CPython runtime",
        "Sky-Player.exe" => b"obsolete v3 identity",
        _ => unreachable!("unknown previous fixture path"),
    }
}

fn assert_preserved(install: &Path) {
    assert_preserved_config(install, br#"{"theme":"aurora"}"#);
}

fn assert_preserved_config(install: &Path, expected_config: &[u8]) {
    assert_eq!(
        fs::read(install.join("config.json")).expect("config"),
        expected_config
    );
    assert_eq!(
        fs::read(install.join(".env")).expect("env"),
        b"USER_SECRET=preserve"
    );
    assert_eq!(
        fs::read(install.join("songs/user.skysheet")).expect("song"),
        b"user song"
    );
    assert_eq!(
        fs::read(install.join("logs/user.log")).expect("log"),
        b"user log"
    );
}

fn assert_preserved_config_semantics(install: &Path, expected_config: &[u8]) {
    let actual: serde_json::Value = serde_json::from_slice(
        &fs::read(install.join("config.json")).expect("config"),
    )
    .expect("actual config JSON");
    let expected: serde_json::Value =
        serde_json::from_slice(expected_config).expect("expected config JSON");
    assert_eq!(actual, expected, "user configuration values changed");
}

fn stop_child(child: &mut Child) {
    if child.try_wait().expect("child status").is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}

struct ChildGuard(Child);

impl ChildGuard {
    fn id(&self) -> u32 {
        self.0.id()
    }

    fn stop(&mut self) {
        stop_child(&mut self.0);
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.0.wait()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        stop_child(&mut self.0);
    }
}

fn wait_for_ready_handoff(local_app_data: &Path, updater_pid: u32) -> PathBuf {
    let runs = local_app_data.join(APP_NAME).join("update-runs");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if let Ok(entries) = fs::read_dir(&runs) {
            for entry in entries.flatten() {
                let handoff = entry.path().join("handoff.json");
                let Ok(bytes) = fs::read(&handoff) else {
                    continue;
                };
                let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                    continue;
                };
                if value.get("state").and_then(|v| v.as_str()) == Some("ready")
                    && value.get("updater_pid").and_then(|v| v.as_u64()) == Some(updater_pid as u64)
                {
                    return entry.path();
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("native updater did not publish READY within the bounded handoff budget");
}

#[cfg(windows)]
fn no_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0800_0000);
}

#[cfg(not(windows))]
fn no_window(_command: &mut Command) {}

#[test]
fn exact_portable_artifact_updates_previous_stable_and_preserves_user_state() -> Result<()> {
    let artifact_dir = match std::env::var_os("SKY_PORTABLE_ARTIFACT_DIR") {
        Some(value) => PathBuf::from(value),
        None => {
            eprintln!("SKY_PORTABLE_ARTIFACT_DIR is not set; exact artifact qualification skipped");
            return Ok(());
        }
    };
    let zip = artifact_dir.join(format!("Sky-Auto-Player-v{TARGET_VERSION}.zip"));
    let sidecar = artifact_dir.join(format!("Sky-Auto-Player-v{TARGET_VERSION}.zip.sha256"));
    let external_manifest = artifact_dir.join(MANIFEST_NAME);
    assert!(zip.is_file(), "exact ZIP missing");
    assert!(sidecar.is_file(), "exact ZIP sidecar missing");
    assert!(
        external_manifest.is_file(),
        "exact external manifest missing"
    );
    let sidecar_text = fs::read_to_string(&sidecar).expect("sidecar");
    let actual_hash = sha256_file(&zip)?;
    assert!(sidecar_text.starts_with(&actual_hash));

    let source = LocalReleaseSource::new(&artifact_dir).expect("local exact release source");
    let download_root = temp_root("download");
    fs::create_dir_all(&download_root).expect("download root");
    let downloaded = download_root.join("downloaded.zip");
    let payload = source.fetch_exact_release(
        TARGET_VERSION,
        sky_updater::cli::Channel::Stable,
        &downloaded,
    )?;
    assert_eq!(payload.zip_sha256, actual_hash);

    let staging = temp_root("staging");
    fs::create_dir_all(&staging).expect("staging");
    extract_zip_file(&downloaded, &staging)?;
    let staged = read_staged_manifest(&staging, TARGET_VERSION)?;
    assert_eq!(
        staged, payload.manifest,
        "embedded/external manifests differ"
    );
    assert_eq!(staged.executable, PRIMARY_EXE);
    assert!(
        staged
            .files
            .iter()
            .all(|file| file.path != "Sky-Auto-Player-Core.exe")
    );
    assert!(
        staged
            .files
            .iter()
            .all(|file| !file.path.starts_with("_internal/"))
    );

    let install = temp_root("install with spaces");
    fs::create_dir_all(&install).expect("install");
    let previous = old_manifest();
    for file in &previous.files {
        write_file(&install, &file.path, old_file_bytes(&file.path));
    }
    write_manifest(&install, &previous);
    write_file(&install, "config.json", br#"{"theme":"aurora"}"#);
    write_file(&install, ".env", b"USER_SECRET=preserve");
    write_file(&install, "songs/user.skysheet", b"user song");
    write_file(&install, "logs/user.log", b"user log");

    install_verified(&install, &staging, &staged, &previous, &NoopProgressSink)?;
    verify_installed_managed(&install, &staged)?;
    assert_preserved(&install);
    assert!(
        !install.join("Sky-Player.exe").exists(),
        "obsolete managed identity removed"
    );
    assert_eq!(
        Manifest::parse(&fs::read(install.join(MANIFEST_NAME)).expect("installed manifest"))?,
        staged
    );

    fs::remove_dir_all(download_root).expect("download cleanup");
    fs::remove_dir_all(staging).expect("staging cleanup");
    fs::remove_dir_all(install).expect("install cleanup");
    Ok(())
}

#[test]
fn exact_portable_artifact_interrupted_transaction_recovers_and_preserves_user_state() -> Result<()>
{
    let artifact_dir = match std::env::var_os("SKY_PORTABLE_ARTIFACT_DIR") {
        Some(value) => PathBuf::from(value),
        None => return Ok(()),
    };
    let zip = artifact_dir.join(format!("Sky-Auto-Player-v{TARGET_VERSION}.zip"));
    assert!(zip.is_file(), "exact ZIP missing");

    let install = temp_root("exact-recovery-install");
    fs::create_dir_all(&install).expect("install");
    let previous = old_manifest();
    for file in &previous.files {
        write_file(&install, &file.path, old_file_bytes(&file.path));
    }
    write_manifest(&install, &previous);
    write_file(&install, "config.json", br#"{"theme":"aurora"}"#);
    write_file(&install, ".env", b"USER_SECRET=preserve");
    write_file(&install, "songs/user.skysheet", b"user song");
    write_file(&install, "logs/user.log", b"user log");

    let staging = temp_root("exact-recovery-staging");
    fs::create_dir_all(&staging).expect("staging");
    extract_zip_file(&zip, &staging)?;
    let target = read_staged_manifest(&staging, TARGET_VERSION)?;
    let plan = build_plan(Some(&previous), &target)?;
    prepare_journal(&install, &plan, &NoopProgressSink)?;
    assert!(has_unresolved_transaction(&install));

    // Simulate an interrupted apply after target files were written but before
    // the transaction could commit. Copy only manifest-owned files here: the
    // real transaction never replaces preserved user state such as
    // config.json, songs/, or logs/.
    for file in &target.files {
        if classify_preserved(&file.path) == PreserveClass::Preserved {
            continue;
        }
        let source = staging.join(&file.path);
        let destination = install.join(&file.path);
        fs::create_dir_all(destination.parent().expect("managed file parent"))?;
        fs::copy(source, destination)?;
    }
    recover_before_update(&install)?;

    assert!(!has_unresolved_transaction(&install));
    for file in &previous.files {
        assert_eq!(
            fs::read(install.join(&file.path)).expect("restored managed file"),
            old_file_bytes(&file.path),
            "restored {}",
            file.path
        );
    }
    assert_preserved(&install);
    assert!(install.join("Sky-Auto-Player-Core.exe").is_file());
    assert!(install.join("_internal/python314.dll").is_file());

    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_dir_all(&install);
    Ok(())
}

#[cfg(feature = "e2e-fault-injection")]
#[test]
fn exact_portable_artifact_injected_apply_failure_rolls_back_and_preserves_user_state() -> Result<()>
{
    let artifact_dir = match std::env::var_os("SKY_PORTABLE_ARTIFACT_DIR") {
        Some(value) => PathBuf::from(value),
        None => return Ok(()),
    };
    let zip = artifact_dir.join(format!("Sky-Auto-Player-v{TARGET_VERSION}.zip"));
    assert!(zip.is_file(), "exact ZIP missing");

    let install = temp_root("exact-fault-install");
    fs::create_dir_all(&install).expect("install");
    let previous = old_manifest();
    for file in &previous.files {
        write_file(&install, &file.path, old_file_bytes(&file.path));
    }
    write_manifest(&install, &previous);
    write_file(&install, "config.json", br#"{"theme":"aurora"}"#);
    write_file(&install, ".env", b"USER_SECRET=preserve");
    write_file(&install, "songs/user.skysheet", b"user song");
    write_file(&install, "logs/user.log", b"user log");

    let staging = temp_root("exact-fault-staging");
    fs::create_dir_all(&staging).expect("staging");
    extract_zip_file(&zip, &staging)?;
    let target = read_staged_manifest(&staging, TARGET_VERSION)?;

    sky_updater::faults::configure(
        Some("apply:after-replace:Sky-Auto-Player-Updater.exe"),
        None,
        None,
    )
    .expect("fault config");
    let error = install_verified(&install, &staging, &target, &previous, &NoopProgressSink)
        .expect_err("exact artifact apply fault");
    assert!(matches!(
        error,
        sky_updater::error::UpdaterError::InstallCopyFailed(_)
    ));
    assert!(has_unresolved_transaction(&install));

    sky_updater::faults::configure(None, None, None).expect("clear fault");
    rollback_prepared(&install)?;
    for file in &previous.files {
        assert_eq!(
            fs::read(install.join(&file.path)).expect("restored managed file"),
            old_file_bytes(&file.path),
            "restored {}",
            file.path
        );
    }
    assert_preserved(&install);
    assert!(!has_unresolved_transaction(&install));

    let _ = fs::remove_dir_all(&staging);
    let _ = fs::remove_dir_all(&install);
    Ok(())
}

#[test]
fn exact_packaged_updater_handoff_transaction_and_restart() -> Result<()> {
    let artifact_dir = match std::env::var_os("SKY_PORTABLE_ARTIFACT_DIR") {
        Some(value) => PathBuf::from(value),
        None => return Ok(()),
    };
    let e2e_updater = match std::env::var_os("SKY_PORTABLE_E2E_UPDATER") {
        Some(value) => PathBuf::from(value),
        None => panic!("exact package updater qualification runner is missing"),
    };
    let release_dir = artifact_dir.join(format!("Sky-Auto-Player-v{TARGET_VERSION}"));
    let actual_updater = release_dir.join(UPDATER_EXE);
    let primary = release_dir.join(PRIMARY_EXE);
    assert!(actual_updater.is_file(), "packaged updater is missing");
    assert!(primary.is_file(), "packaged Tauri executable is missing");
    let version = Command::new(&actual_updater).arg("--version").output()?;
    assert!(
        version.status.success(),
        "packaged updater --version failed"
    );

    let zip = artifact_dir.join(format!("Sky-Auto-Player-v{TARGET_VERSION}.zip"));
    let local_app_data = temp_root("localappdata");
    let _local_app_data = EnvGuard::set("LOCALAPPDATA", &local_app_data);
    fs::create_dir_all(&local_app_data).expect("local app data");
    let update_runs = local_app_data.join(APP_NAME).join("update-runs");
    fs::create_dir_all(&update_runs).expect("update runs");
    let install = temp_root("exact-install-with-spaces");
    fs::create_dir_all(&install).expect("install");
    extract_zip_file(&zip, &install)?;
    let target = read_staged_manifest(&install, TARGET_VERSION)?;
    let packaged_config = fs::read(install.join("config.json")).expect("packaged config");
    let user_config = String::from_utf8(packaged_config)
        .expect("packaged config UTF-8")
        .replace(r#""theme": "aurora""#, r#""theme": "classic""#)
        .into_bytes();
    assert_ne!(
        user_config,
        fs::read(install.join("config.json")).expect("packaged config")
    );
    write_file(&install, "config.json", &user_config);
    let mut previous = target.clone();
    previous.version = PREVIOUS_VERSION.into();
    previous.git_head = "a".repeat(40);
    previous.native_build_commit = "b".repeat(40);
    previous.files.push(ManifestFile {
        path: "Sky-Player.exe".into(),
        size: b"obsolete v3 identity".len() as u64,
        sha256: sha256_bytes(b"obsolete v3 identity"),
    });
    write_file(&install, "Sky-Player.exe", b"obsolete v3 identity");
    write_manifest(&install, &previous);
    write_file(&install, ".env", b"USER_SECRET=preserve");
    write_file(&install, "songs/user.skysheet", b"user song");
    write_file(&install, "logs/user.log", b"user log");

    let restart_marker = temp_root("restart-marker");
    let mut parent = Command::new(install.join(PRIMARY_EXE));
    parent
        .arg("--selftest-desktop-parent")
        .current_dir(&install)
        .env("SKY_DESKTOP_RESTART_MARKER", &restart_marker);
    no_window(&mut parent);
    let mut parent = ChildGuard(parent.spawn()?);

    // Exercise the shipped updater binary through its real READY/parent-wait
    // boundary before the offline transaction runner performs the exact
    // artifact install. The production binary intentionally remains
    // GitHub/HTTPS-only; the feature-gated runner below is the only local
    // transport used for the deterministic offline transaction.
    let packaged_run_root = update_runs.join("run-abcdef0123456789abcdef0123456789");
    fs::create_dir_all(&packaged_run_root).expect("packaged updater run root");
    let packaged_updater = packaged_run_root.join(UPDATER_EXE);
    fs::copy(&actual_updater, &packaged_updater).expect("packaged updater qualification copy");
    let mut packaged = Command::new(&packaged_updater);
    packaged
        .arg("--install-root")
        .arg(&install)
        .arg("--parent-pid")
        .arg(parent.id().to_string())
        .arg("--current-version")
        .arg(PREVIOUS_VERSION)
        .arg("--target-version")
        .arg(TARGET_VERSION)
        .arg("--channel")
        .arg("stable")
        .current_dir(&install);
    no_window(&mut packaged);
    let mut packaged = ChildGuard(packaged.spawn()?);
    let _packaged_run_root = wait_for_ready_handoff(&local_app_data, packaged.id());
    packaged.stop();

    let e2e_run_root = update_runs.join("run-0123456789abcdef0123456789abcdef");
    fs::create_dir_all(&e2e_run_root).expect("E2E updater run root");
    let e2e_updater_canonical = e2e_run_root.join(UPDATER_EXE);
    fs::copy(&e2e_updater, &e2e_updater_canonical).expect("canonical E2E updater copy");
    let mut updater = Command::new(&e2e_updater_canonical);
    updater
        .arg("--release-dir")
        .arg(&artifact_dir)
        .arg("--install-root")
        .arg(&install)
        .arg("--parent-pid")
        .arg(parent.id().to_string())
        .arg("--current-version")
        .arg(PREVIOUS_VERSION)
        .arg("--target-version")
        .arg(TARGET_VERSION)
        .arg("--channel")
        .arg("stable")
        .arg("--restart")
        .current_dir(&install)
        .env("SKY_DESKTOP_RESTART_SELFTEST", "1")
        // The restarted packaged shell runs the same deterministic safe
        // seams as the portable selftest.  Propagate those seams through the
        // updater so the child does not attempt live GitHub/calibration I/O.
        .env("SKY_PACKAGED_SAFE_CALIBRATION", "1")
        .env("SKY_PACKAGED_SAFE_UPDATE", "1")
        .env("SKY_DESKTOP_RESTART_MARKER", &restart_marker);
    no_window(&mut updater);
    let mut updater = ChildGuard(updater.spawn()?);
    let _run_root = wait_for_ready_handoff(&local_app_data, updater.id());
    parent.stop();
    let status = updater.wait()?;
    assert!(
        status.success(),
        "local-source native updater failed: {status}"
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !restart_marker.is_file() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        restart_marker.is_file(),
        "canonical Tauri restart did not bootstrap the new native desktop"
    );
    verify_installed_managed(&install, &target)?;
    // Native bootstrap may normalize persisted JSON formatting on read; the
    // updater contract is preservation of user state, not incidental key
    // ordering or whitespace.
    assert_preserved_config_semantics(&install, &user_config);
    assert!(!install.join("Sky-Player.exe").exists());
    assert!(!install.join("Sky-Auto-Player-Core.exe").exists());
    assert!(!install.join("_internal").exists());
    assert!(
        !local_app_data
            .join(APP_NAME)
            .join("update-state")
            .join("active-update.json")
            .exists()
    );

    let _ = fs::remove_dir_all(&install);
    let _ = fs::remove_dir_all(&local_app_data);
    let _ = fs::remove_file(&restart_marker);
    Ok(())
}
