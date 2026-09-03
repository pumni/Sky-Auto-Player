use sky_app_core::library::LibraryManifestService;
use sky_app_core::settings::SettingsService;
use sky_native_adapters::{
    AppPaths, FileCatalogSource, JsonLibraryManifestStore, JsonSettingsStore, snapshot_directory,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn v4_adapters_enforce_clean_application_data_boundary() {
    let install_root = PathBuf::from(r"C:\Users\tester\AppData\Local\Programs\Sky Auto Player");
    let app_data_root =
        PathBuf::from(r"C:\Users\tester\AppData\Local\io.github.pumni.skyautoplayer");
    let paths = AppPaths::from_app_data_root(install_root.clone(), app_data_root.clone());

    // Clean boundary: mutable roots must not reside inside install root
    assert!(paths.assert_clean_boundary().is_ok());

    // Settings store uses app-data config path, not install root
    let settings = JsonSettingsStore::new(paths.settings_path());
    assert_eq!(settings.path(), app_data_root.join("config.json").as_path());
    assert!(!settings.path().starts_with(&install_root));

    // Library manifest store uses app-data data path, not install root
    let manifest = JsonLibraryManifestStore::new(paths.library_manifest_path());
    assert_eq!(
        manifest.path(),
        app_data_root.join("library-manifest.json").as_path()
    );
    assert!(!manifest.path().starts_with(&install_root));

    // Catalog source uses resolved songs dir outside install root
    let default_songs = paths.resolve_songs_dir("songs").expect("valid songs dir");
    let catalog = FileCatalogSource::new(&default_songs);
    assert_eq!(catalog.root(), app_data_root.join("songs").as_path());
    assert!(!catalog.root().starts_with(&install_root));

    // Calibration cache lives in app data cache, not install root
    assert_eq!(
        paths.calibration_cache_path(),
        app_data_root.join("cache").join("input_latency.json")
    );
    assert!(!paths.calibration_cache_path().starts_with(&install_root));

    // Only immutable executable payload lives in install root
    assert_eq!(
        paths.calibration_binary_path(),
        install_root.join("native_calibration.exe")
    );
    assert!(paths.calibration_binary_path().starts_with(&install_root));
}

#[test]
fn v4_songs_dir_resolution_fails_closed_on_escapes() {
    let install_root = PathBuf::from(r"C:\Users\tester\AppData\Local\Programs\Sky Auto Player");
    let app_data_root =
        PathBuf::from(r"C:\Users\tester\AppData\Local\io.github.pumni.skyautoplayer");
    let paths = AppPaths::from_app_data_root(install_root.clone(), app_data_root.clone());

    // 1. Rejects relative parent traversal '..'
    let traversal_err = paths.resolve_songs_dir("../escaped").unwrap_err();
    assert!(
        traversal_err.contains("parent directory traversal ('..')"),
        "unexpected error: {traversal_err}"
    );

    // 2. Rejects nested parent traversal
    let nested_traversal_err = paths.resolve_songs_dir("sub/../../escaped").unwrap_err();
    assert!(
        nested_traversal_err.contains("parent directory traversal ('..')"),
        "unexpected error: {nested_traversal_err}"
    );

    // 3. Rejects absolute path pointing inside install root
    let inside_install = install_root.join("songs");
    let inside_err = paths
        .resolve_songs_dir(&inside_install.display().to_string())
        .unwrap_err();
    assert!(
        inside_err.contains("must not reside inside install root"),
        "unexpected error: {inside_err}"
    );

    // 4. Preserves valid external absolute path
    let external_abs = PathBuf::from(r"D:\ExternalLibrary\Sheets");
    let resolved_abs = paths
        .resolve_songs_dir(&external_abs.display().to_string())
        .expect("valid external");
    assert_eq!(resolved_abs, external_abs);

    // 5. Resolves valid relative subfolder under user_music_root
    let sub = paths.resolve_songs_dir("custom").expect("valid sub");
    assert_eq!(sub, app_data_root.join("songs").join("custom"));
}

#[test]
fn v4_adapters_operate_with_immutable_install_payload_snapshot_proof() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("sky-appdata-contract-test-{suffix}"));
    let install_root = temp.join("install");
    let app_data_root = temp.join("app_data");

    fs::create_dir_all(&install_root).expect("create install root");
    // Place simulated immutable binaries and resources in install root
    let binary = install_root.join("native_calibration.exe");
    fs::write(&binary, b"simulated-binary-payload").expect("write binary");
    let resource = install_root.join("resources.pak");
    fs::write(&resource, b"simulated-resource-payload").expect("write resource");

    // Cryptographic snapshot before any runtime adapter interaction
    let install_snapshot_before =
        snapshot_directory(&install_root).expect("snapshot install before");

    let paths = AppPaths::from_app_data_root(install_root.clone(), app_data_root.clone());
    paths
        .ensure_mutable_directories()
        .expect("ensure app data dirs");

    // Settings store loads and saves in app data
    let settings_store = JsonSettingsStore::new(paths.settings_path());
    let mut settings_service = SettingsService::load(settings_store).expect("load settings");
    let patch = sky_app_core::settings::SettingsPatch {
        theme: Some("slate".into()),
        ..Default::default()
    };
    settings_service
        .patch(&patch)
        .expect("patch settings must succeed into app_data");

    // Verify settings was saved into app_data_root, not install_root
    assert!(paths.settings_path().exists());
    assert!(!install_root.join("config.json").exists());

    // Library manifest loads and saves in app data
    let manifest_store = JsonLibraryManifestStore::new(paths.library_manifest_path());
    let mut manifest_service = LibraryManifestService::load(manifest_store).expect("load manifest");
    manifest_service
        .create_collection("0123456789abcdef0123456789abcdef", "Test Playlist")
        .expect("create collection must succeed into app_data");

    assert!(paths.library_manifest_path().exists());
    assert!(!install_root.join("library-manifest.json").exists());

    // Songs dir is created and read from app data
    let test_song = paths.user_music_root().join("sample.json");
    fs::write(&test_song, b"{\"title\":\"Test\"}").expect("write test song");
    let default_songs = paths.resolve_songs_dir("songs").expect("resolve songs");
    let catalog = FileCatalogSource::new(&default_songs);
    assert_eq!(catalog.root(), paths.user_music_root());
    assert!(!install_root.join("songs").exists());

    // Cryptographic proof: entire install payload is completely byte-for-byte unchanged
    let install_snapshot_after = snapshot_directory(&install_root).expect("snapshot install after");
    assert_eq!(
        install_snapshot_before, install_snapshot_after,
        "install payload must remain 100% byte-for-byte identical throughout adapter operations"
    );

    let _ = fs::remove_dir_all(&temp);
}
