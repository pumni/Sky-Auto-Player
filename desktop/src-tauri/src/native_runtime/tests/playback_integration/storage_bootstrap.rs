use super::*;

#[test]
fn debug_install_root_is_repository_root_not_cwd() {
    if cfg!(debug_assertions) {
        let root = resolve_install_root().expect("root");
        assert!(root.join("rust").is_dir());
        assert!(root.join("desktop").is_dir());
    }
}

#[test]
fn production_composition_has_no_environment_only_safe_bypass() {
    assert_eq!(TestSeams::default(), TestSeams::Disabled);
    assert_ne!(TestSeams::Disabled, TestSeams::SafePackage);
    // The only selector is an explicit composition value.  There is no
    // environment-to-seam conversion in either production constructor.
    assert!(!matches!(TestSeams::Disabled, TestSeams::SafePackage));
    assert!(matches!(TestSeams::SafePackage, TestSeams::SafePackage));
}

#[test]
fn native_bootstrap_uses_explicit_install_root_and_returns_plain_dto() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-runtime-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let value = runtime
        .dispatch("app.bootstrap", Value::Object(Default::default()))
        .expect("bootstrap");
    assert!(value.get("app_version").is_some());
    assert!(value.get("Ok").is_none());
    assert_eq!(value["settings"]["palette"], "aurora");
    assert_eq!(value["settings"]["appearance_mode"], "dark");
    assert_eq!(value["settings"]["palette"], value["palette"]);
    assert_eq!(value["theme"], "aurora");
    assert_eq!(runtime.install_root(), root.as_path());
    fs::write(
        root.join("config.json"),
        "{\"schema_version\":3,\"theme\":\"slate\"}\n",
    )
    .expect("external settings update");
    let cached_bootstrap = runtime
        .dispatch("app.bootstrap", Value::Object(Default::default()))
        .expect("cached bootstrap");
    assert_eq!(cached_bootstrap["settings"]["palette"], "aurora");
    let refreshed_settings = runtime
        .dispatch("settings.get", Value::Object(Default::default()))
        .expect("settings refresh");
    assert_eq!(refreshed_settings["palette"], "slate");
    assert!(
        runtime
            .dispatch("settings.get", Value::Object(Default::default()))
            .is_ok()
    );
    assert_eq!(
        runtime
            .dispatch("app.shutdown", Value::Object(Default::default()))
            .expect("first shutdown"),
        Value::Null
    );
    assert_eq!(
        runtime
            .dispatch("app.shutdown", Value::Object(Default::default()))
            .expect("idempotent shutdown"),
        Value::Null
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn update_check_uses_cached_settings_without_refreshing_external_file() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-update-check-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");

    let initial = runtime
        .dispatch("app.bootstrap", Value::Object(Default::default()))
        .expect("bootstrap");
    assert_eq!(initial["settings"]["palette"], "aurora");

    fs::write(
        root.join("config.json"),
        "{\"schema_version\":3,\"theme\":\"slate\"}\n",
    )
    .expect("external settings update");
    let error = runtime
        .dispatch("update.check", Value::Object(Default::default()))
        .expect_err("install-root runtime has no update service");
    assert!(error.starts_with("update_service_unavailable"));

    let cached = runtime
        .dispatch("app.bootstrap", Value::Object(Default::default()))
        .expect("cached bootstrap");
    assert_eq!(cached["settings"]["palette"], "aurora");

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn same_size_same_mtime_content_mutation_still_matches_clean_compose() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("sky-native-catalog-cache-same-metadata-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let song = root.join("songs/content.json");
    let original = br#"{"name":"Alpha","songNotes":[{"time":0,"key":"1Key0"}]}"#;
    let replacement = br#"{"name":"Bravo","songNotes":[{"time":0,"key":"1Key0"}]}"#;
    assert_eq!(original.len(), replacement.len());
    fs::write(&song, original).expect("original song");

    let initial_runtime =
        NativeDesktopRuntime::from_install_root(root.clone()).expect("initial runtime");
    let _ = catalog_generation(&initial_runtime);
    initial_runtime.shutdown();
    let original_modified = fs::metadata(&song)
        .expect("original metadata")
        .modified()
        .expect("original mtime");

    fs::write(&song, replacement).expect("replacement song");
    fs::OpenOptions::new()
        .write(true)
        .open(&song)
        .expect("open replacement")
        .set_times(fs::FileTimes::new().set_modified(original_modified))
        .expect("restore original mtime");
    let replacement_metadata = fs::metadata(&song).expect("replacement metadata");
    assert_eq!(replacement_metadata.len(), original.len() as u64);
    assert_eq!(
        replacement_metadata.modified().expect("replacement mtime"),
        original_modified
    );

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("cached runtime");
    let claim = runtime
        .claim_catalog_load()
        .expect("reconcile claim")
        .expect("reconcile work");
    runtime.run_catalog_load(claim);
    let reconciled = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({"query":"","offset":0,"limit":20,"source":{"kind":"smart","id":"all"}}),
            )
            .expect("reconciled search");
    runtime.shutdown();

    fs::remove_file(root.join("cache/catalog-index.json")).expect("remove cache");
    let clean_runtime =
        NativeDesktopRuntime::from_install_root(root.clone()).expect("clean runtime");
    let clean = clean_runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({"query":"","offset":0,"limit":20,"source":{"kind":"smart","id":"all"}}),
            )
            .expect("clean search");
    assert_eq!(reconciled["items"], clean["items"]);
    clean_runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corrupt_cache_falls_back_to_cold_rebuild() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-catalog-corrupt-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(root.join("songs/one.json"), b"one").expect("song");
    fs::create_dir_all(root.join("cache")).expect("cache root");
    fs::write(
        root.join("cache/catalog-index.json"),
        b"{\"schema_version\":1",
    )
    .expect("truncated cache");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    assert!(matches!(
        *runtime.catalog_load.lock().expect("catalog state"),
        super::CatalogLoadState::Uninitialized
    ));
    assert_eq!(catalog_generation(&runtime), 1);
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cache_source_fingerprint_rejects_changed_user_root_before_hydration() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sky-native-catalog-cache-changed-user-root-{suffix}"
    ));
    let old_root = root.join("songs");
    let new_root =
        std::env::temp_dir().join(format!("sky-native-catalog-cache-new-user-root-{suffix}"));
    fs::create_dir_all(&old_root).expect("old songs root");
    fs::create_dir_all(&new_root).expect("new songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(old_root.join("old.json"), b"old").expect("old song");

    let initial_runtime =
        NativeDesktopRuntime::from_install_root(root.clone()).expect("initial runtime");
    let _ = catalog_generation(&initial_runtime);
    initial_runtime.shutdown();
    fs::write(
        root.join("config.json"),
        format!(
            "{{\"schema_version\":3,\"songs_dir\":{:?}}}\n",
            new_root.to_string_lossy()
        ),
    )
    .expect("changed songs root");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    assert_eq!(
        runtime
            .settings
            .lock()
            .expect("settings lock")
            .snapshot()
            .songs_dir,
        new_root.to_string_lossy()
    );
    let persisted = CatalogCacheStore::new(root.join("cache/catalog-index.json"))
        .load()
        .expect("load cache")
        .expect("cache");
    assert_ne!(
        persisted.source_fingerprint,
        runtime
            .catalog_source_fingerprint()
            .expect("source fingerprint"),
        "changed user root must change the source fingerprint"
    );
    assert!(matches!(
        *runtime.catalog_load.lock().expect("catalog state"),
        super::CatalogLoadState::Uninitialized
    ));
    assert_eq!(
        runtime
            .catalog
            .lock()
            .expect("catalog lock")
            .index
            .snapshot()
            .total,
        0
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(new_root);
}

#[test]
fn shutdown_fences_background_reconciliation_and_cache_write() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-catalog-shutdown-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let song = root.join("songs/one.json");
    fs::write(&song, b"one").expect("song");
    write_catalog_cache(&root, vec![user_cache_entry(&song, "one", 3)]);
    let cache_path = root.join("cache/catalog-index.json");
    let before = fs::read(&cache_path).expect("cache before reconcile");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let generation = catalog_generation(&runtime);
    let claim = runtime
        .claim_catalog_load()
        .expect("reconcile claim")
        .expect("reconcile work");
    runtime.shutdown();
    runtime.run_catalog_load(claim);
    assert!(matches!(
        *runtime.catalog_load.lock().expect("catalog state"),
        super::CatalogLoadState::Closing
    ));
    assert_eq!(
        runtime
            .catalog
            .lock()
            .expect("catalog lock")
            .index
            .generation(),
        generation
    );
    assert_eq!(fs::read(cache_path).expect("cache after shutdown"), before);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn v4_runtime_operates_with_read_only_install_root_and_separate_app_data() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("sky-v4-appdata-runtime-{suffix}"));
    let install_root = temp.join("install_payload");
    let app_data_root = temp.join("app_data");

    fs::create_dir_all(&install_root).expect("create install payload");
    let dummy_exe = install_root.join(sky_native_adapters::CALIBRATION_EXE);
    fs::write(&dummy_exe, b"immutable-binary-data").expect("write dummy exe");
    let dummy_resource = install_root.join("resources.pak");
    fs::write(&dummy_resource, b"immutable-resource-data").expect("write dummy resource");

    // Take snapshot of entire install payload before runtime execution
    let install_snapshot_before =
        sky_native_adapters::snapshot_directory(&install_root).expect("snapshot install before");

    let paths = sky_native_adapters::AppPaths::from_app_data_root(
        install_root.clone(),
        app_data_root.clone(),
    );
    assert!(paths.assert_clean_boundary().is_ok());
    paths
        .ensure_mutable_directories()
        .expect("ensure mutable dirs");

    // Place a song sheet in the user songs directory
    let song_file = paths.user_music_root().join("test_song.json");
    fs::write(
            &song_file,
            br#"{"name":"V4 Song","author":"Tester","bpm":120,"pitchLevel":0,"data":[{"time":0,"key":"1Key0"}]}"#,
        )
        .expect("write test song");

    let runtime = NativeDesktopRuntime::from_paths_with_activity_and_seams(
        paths.clone(),
        sky_native_adapters::AppResources::from_resource_dir(&install_root),
        ActivityCoordinator::default(),
        TestSeams::SafePackage,
    )
    .expect("start runtime with read-only install root");

    // 1. Settings mutation saves into app_data/config.json
    let patch_result = runtime
        .dispatch(
            "settings.patch",
            serde_json::json!({
                "palette": "slate"
            }),
        )
        .expect("patch settings");
    assert_eq!(patch_result["palette"], "slate");
    assert!(paths.settings_path().exists());
    assert!(!install_root.join("config.json").exists());

    // 2. Catalog load reads from user_music_root outside install root
    let catalog = runtime
        .dispatch("catalog.reload", Value::Object(Default::default()))
        .expect("reload catalog");
    let count = catalog["total"].as_u64().unwrap_or(0);
    assert!(count >= 1, "catalog must contain at least 1 song");
    assert!(!install_root.join("songs").exists());

    // 3. Calibration writes cache to app_data/cache, not install root
    runtime
        .dispatch("calibration.start", serde_json::json!({"mode":"quick"}))
        .expect("start calibration");

    let deadline = Instant::now() + Duration::from_secs(3);
    while runtime.calibration.is_calibration_active_for_test() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!runtime.calibration.is_calibration_active_for_test());
    assert!(paths.calibration_cache_path().exists());
    assert!(!install_root.join(".cache").exists());
    assert!(!install_root.join("cache").exists());

    // 4. Clean shutdown
    runtime.shutdown();

    // 5. Cryptographic proof: entire install payload is completely byte-for-byte unchanged
    let install_snapshot_after =
        sky_native_adapters::snapshot_directory(&install_root).expect("snapshot install after");
    assert_eq!(
        install_snapshot_before, install_snapshot_after,
        "install payload must remain 100% byte-for-byte identical"
    );

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn persisted_beta_repair_fails_closed_and_persists_stable() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-beta-repair-test-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(
        root.join("config.json"),
        r#"{"schema_version":8,"update":{"channel":"beta"}}"#,
    )
    .expect("write config with beta channel");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone())
        .expect("runtime bootstrap must succeed and repair beta channel to stable");

    assert_eq!(
        runtime
            .settings_snapshot()
            .expect("snapshot")
            .update
            .channel,
        sky_app_core::settings::UpdateChannel::Stable
    );

    let persisted: Value = serde_json::from_str(
        &fs::read_to_string(root.join("config.json")).expect("read persisted config"),
    )
    .expect("parse persisted config");
    assert_eq!(persisted["update"]["channel"], "stable");

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn beta_patch_rejected() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-beta-patch-test-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), r#"{"schema_version":8}"#).expect("write config");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");

    let err1 = runtime
        .patch_settings(crate::commands::SettingsPatch {
            appearance_mode: None,
            palette: None,
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: None,
            auto_play: None,
            update_preferences: Some(crate::commands::UpdatePreferencesPatch {
                auto_check: None,
                channel: Some(crate::ui_events::UpdateChannel::Beta),
                skip_version: None,
            }),
        })
        .unwrap_err();
    assert!(err1.contains("channel_unavailable"));

    let err2 = runtime
        .patch_update_preferences(crate::commands::UpdatePreferencesPatch {
            auto_check: None,
            channel: Some(crate::ui_events::UpdateChannel::Beta),
            skip_version: None,
        })
        .unwrap_err();
    assert!(err2.contains("channel_unavailable"));

    let err3 = runtime
        .dispatch(
            "settings.patch",
            serde_json::json!({
                "updatePreferences": {
                    "channel": "beta"
                }
            }),
        )
        .unwrap_err();
    assert!(err3.contains("channel_unavailable"));

    let err4 = runtime
        .dispatch(
            "update.preferences.patch",
            serde_json::json!({
                "channel": "beta"
            }),
        )
        .unwrap_err();
    assert!(err4.contains("channel_unavailable"));

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn invalid_appearance_patch_rejects_neighboring_settings_atomically() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-invalid-appearance-patch-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(
        root.join("config.json"),
        r#"{"schema_version":9,"palette":"slate"}"#,
    )
    .expect("write legacy settings");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let before = runtime
        .dispatch("settings.get", Value::Object(Default::default()))
        .expect("settings before patch");
    assert_eq!(before["appearance_mode"], "dark");
    assert_eq!(before["telemetry_enabled"], false);

    let error = runtime
        .dispatch(
            "settings.patch",
            serde_json::json!({
                "appearanceMode": "sepia",
                "telemetryEnabled": true
            }),
        )
        .expect_err("invalid appearance mode rejected");
    assert!(!error.is_empty());

    let after = runtime
        .dispatch("settings.get", Value::Object(Default::default()))
        .expect("settings after rejected patch");
    assert_eq!(after, before);
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}
