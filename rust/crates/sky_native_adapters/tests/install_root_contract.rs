use sky_app_core::library::LibraryManifestService;
use sky_app_core::settings::SettingsService;
use sky_native_adapters::{
    AppPaths, FileCatalogSource, JsonLibraryManifestStore, JsonSettingsStore,
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
    let default_songs = paths.resolve_songs_dir("songs");
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
#[allow(clippy::permissions_set_readonly_false)]
fn v4_adapters_operate_with_read_only_install_root() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("sky-appdata-contract-test-{suffix}"));
    let install_root = temp.join("install");
    let app_data_root = temp.join("app_data");

    fs::create_dir_all(&install_root).expect("create install root");
    // Place simulated immutable binaries in install root
    let binary = install_root.join("native_calibration.exe");
    fs::write(&binary, b"simulated-binary-payload").expect("write binary");

    // Make install root and its payload read-only
    let mut perms = fs::metadata(&binary).expect("meta").permissions();
    perms.set_readonly(true);
    fs::set_permissions(&binary, perms).expect("set readonly file");

    let paths = AppPaths::from_app_data_root(install_root.clone(), app_data_root.clone());
    paths
        .ensure_mutable_directories()
        .expect("ensure app data dirs");

    // Settings store loads and saves in app data while install root is immutable
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
    let catalog = FileCatalogSource::new(paths.resolve_songs_dir("songs"));
    assert_eq!(catalog.root(), paths.user_music_root());
    assert!(!install_root.join("songs").exists());

    // Cleanup: restore write permission on install files so temp dir can be cleaned up
    let mut perms = fs::metadata(&binary).expect("meta").permissions();
    perms.set_readonly(false);
    let _ = fs::set_permissions(&binary, perms);
    let _ = fs::remove_dir_all(&temp);
}
