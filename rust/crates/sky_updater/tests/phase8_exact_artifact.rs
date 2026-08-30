//! Exact Phase 8 artifact qualification.
//!
//! The normal updater tests use small synthetic payloads.  When
//! `SKY_PHASE8_ARTIFACT_DIR` is supplied, this test consumes the exact ZIP,
//! sidecar, external manifest, and embedded manifest emitted by the Phase 8
//! assembler, then runs the real install/rollback transaction against a
//! 3.4.5-style install.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use sky_updater::archive::{extract_zip_file, sha256_bytes, sha256_file};
use sky_updater::error::Result;
use sky_updater::github::ReleaseSource;
use sky_updater::install::{install_verified, read_staged_manifest};
use sky_updater::local_source::LocalReleaseSource;
use sky_updater::manifest::{Manifest, ManifestFile};
use sky_updater::progress::NoopProgressSink;
use sky_updater::transaction::verify_installed_managed;
use sky_updater::{
    APP_NAME, CALIBRATION_EXE, MANIFEST_NAME, PRIMARY_EXE, SCHEMA_VERSION, UPDATER_EXE,
};

const PREVIOUS_VERSION: &str = "3.4.5";
const TARGET_VERSION: &str = "3.5.0";

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "sky-phase8-{label}-{}-{}",
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
        "Sky-Player.exe" => b"obsolete v3 identity",
        _ => unreachable!("unknown previous fixture path"),
    }
}

fn assert_preserved(install: &Path) {
    assert_eq!(
        fs::read(install.join("config.json")).expect("config"),
        br#"{"theme":"aurora"}"#
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

#[test]
fn exact_phase8_artifact_updates_previous_stable_and_preserves_user_state() -> Result<()> {
    let artifact_dir = match std::env::var_os("SKY_PHASE8_ARTIFACT_DIR") {
        Some(value) => PathBuf::from(value),
        None => {
            eprintln!("SKY_PHASE8_ARTIFACT_DIR is not set; exact artifact qualification skipped");
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
            .any(|file| file.path == "Sky-Auto-Player-Core.exe")
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
