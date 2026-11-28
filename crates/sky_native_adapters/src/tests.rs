use super::*;
use serde_json::Value;
use sky_app_core::settings::{
    DEFAULT_TIMING_MARGIN_US, PlaybackDefaultsPatch, SettingsPatch, SettingsService, SettingsStore,
};
use std::fs;
#[test]
fn settings_palette_migration_is_precedence_preserving_and_idempotent() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-palette-migration-{suffix}"));
    let path = root.join("config.json");
    fs::create_dir_all(&root).expect("temp root");
    fs::write(
        &path,
        br#"{
                "schema_version": 8,
                "theme": " SLATE ",
                "future_root": {"preserve": true}
            }"#,
    )
    .expect("seed v8 settings");

    let store = JsonSettingsStore::new(&path);
    let settings = store.load().expect("load v8 settings");
    assert_eq!(settings.palette, "slate");
    let migrated = serde_json::from_slice::<Value>(&fs::read(&path).expect("read migrated"))
        .expect("valid migrated json");
    assert_eq!(migrated["schema_version"], 10);
    assert_eq!(migrated["palette"], "slate");
    assert!(migrated.get("theme").is_none());
    assert_eq!(migrated["future_root"]["preserve"], true);

    let first_bytes = fs::read(&path).expect("read first migration");
    let reloaded = store.load().expect("reload migrated settings");
    assert_eq!(reloaded.palette, "slate");
    assert_eq!(
        fs::read(&path).expect("read idempotent migration"),
        first_bytes
    );

    fs::write(
        &path,
        br#"{
                "schema_version": 8,
                "palette": " classic ",
                "theme": "slate",
                "future_root": 42
            }"#,
    )
    .expect("seed precedence settings");
    let precedence = store.load().expect("load precedence settings");
    assert_eq!(precedence.palette, "classic");
    let precedence_raw =
        serde_json::from_slice::<Value>(&fs::read(&path).expect("read precedence"))
            .expect("valid precedence json");
    assert_eq!(precedence_raw["palette"], "classic");
    assert!(precedence_raw.get("theme").is_none());
    assert_eq!(precedence_raw["future_root"], 42);

    fs::write(&path, br#"{"schema_version":8,"palette":"unknown"}"#).expect("seed invalid palette");
    let invalid = store.load().expect("load invalid palette");
    assert_eq!(invalid.palette, "aurora");
    let invalid_raw = serde_json::from_slice::<Value>(&fs::read(&path).expect("read invalid"))
        .expect("valid invalid-palette migration");
    assert_eq!(invalid_raw["palette"], "aurora");

    let mut service = SettingsService::load(store.clone()).expect("load canonical settings");
    service
        .patch(&SettingsPatch {
            palette: Some("cyberpunk".into()),
            ..Default::default()
        })
        .expect("patch palette");
    let patched_raw = serde_json::from_slice::<Value>(&fs::read(&path).expect("read patched"))
        .expect("valid patched json");
    assert_eq!(patched_raw["palette"], "cyberpunk");
    assert!(patched_raw.get("theme").is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn graphite_is_the_new_settings_default_and_saved_palettes_round_trip() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-settings-palette-round-trip-{suffix}"));
    let path = root.join("config.json");
    fs::create_dir_all(&root).expect("temp root");
    let store = JsonSettingsStore::new(&path);

    let fresh = store.load().expect("initialize new settings");
    assert_eq!(fresh.palette, "graphite");
    let fresh_raw = serde_json::from_slice::<Value>(&fs::read(&path).expect("read defaults"))
        .expect("valid defaults json");
    assert_eq!(fresh_raw["palette"], "graphite");

    fs::write(
        &path,
        serde_json::to_vec(&serde_json::json!({
            "schema_version": sky_app_core::settings::SCHEMA_VERSION,
        }))
        .expect("encode existing settings without a palette"),
    )
    .expect("seed existing settings without a palette");
    assert_eq!(
        store
            .load()
            .expect("existing settings keep the previous palette default")
            .palette,
        "aurora"
    );

    for palette in ["aurora", "minimalist", "slate", "cyberpunk", "classic"] {
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "schema_version": sky_app_core::settings::SCHEMA_VERSION,
                "palette": palette,
            }))
            .expect("encode existing settings"),
        )
        .expect("seed existing palette");
        let loaded = store.load().expect("load existing palette");
        assert_eq!(loaded.palette, palette);

        let mut service = SettingsService::load(store.clone()).expect("load service");
        service
            .patch(&SettingsPatch {
                palette: Some("graphite".into()),
                ..Default::default()
            })
            .expect("save Graphite choice");
        assert_eq!(store.load().expect("reload Graphite").palette, "graphite");

        service
            .patch(&SettingsPatch {
                palette: Some(palette.into()),
                ..Default::default()
            })
            .expect("restore existing palette");
        assert_eq!(store.load().expect("reload saved palette").palette, palette);
    }

    fs::remove_dir_all(root).expect("remove temp root");
}

#[test]
fn settings_store_preserves_unknown_keys_and_writes_atomically() {
    let root = std::env::temp_dir().join(format!("sky-settings-{}", std::process::id()));
    let path = root.join("config.json");
    fs::create_dir_all(&root).expect("temp root");
    fs::write(
        &path,
        br#"{"theme":"aurora","future_field":42,"update":{"channel":"stable","future":true}}"#,
    )
    .expect("seed");
    let store = JsonSettingsStore::new(&path);
    let mut service = SettingsService::load(store.clone()).expect("load");
    service
        .patch(&SettingsPatch {
            playback_defaults: Some(PlaybackDefaultsPatch {
                fps: Some(120),
                ..Default::default()
            }),
            ..Default::default()
        })
        .expect("patch");
    let raw: Value = serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("json");
    assert_eq!(raw["future_field"], 42);
    assert_eq!(raw["update"]["future"], true);
    assert_eq!(raw["game_fps"], 120);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn settings_store_drops_retired_update_fields_while_preserving_unknown_data() {
    let root =
        std::env::temp_dir().join(format!("sky-retired-update-fields-{}", std::process::id()));
    let path = root.join("config.json");
    fs::create_dir_all(&root).expect("temp root");
    fs::write(
        &path,
        br#"{
                "theme": "aurora",
                "future_root": "keep_me",
                "update": {
                    "auto_check": true,
                    "channel": "beta",
                    "skip_version": "v3.9.0",
                    "check_interval_s": 43200,
                    "last_check_ts": 12345,
                    "last_error_ts": 0,
                    "last_notified_version": "v3.8.0",
                    "legacy_old_dir_sweep_pending": true,
                    "pending_update_version": "v3.9.0",
                    "auto_apply": true,
                    "unknown_nested": {"preserve": 99}
                }
            }"#,
    )
    .expect("seed old config");

    let store = JsonSettingsStore::new(&path);
    let settings = store.load().expect("load legacy config safely");
    assert_eq!(settings.update.channel.as_str(), "beta");
    assert_eq!(settings.update.skip_version, "v3.9.0");
    assert_eq!(settings.update.check_interval_s, 43200);
    assert_eq!(settings.update.last_check_ts, 12345);

    // Save back / overlay settings
    store
        .save(&settings)
        .expect("save settings drops retired keys");

    let raw: Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("valid json");
    let update = raw["update"].as_object().expect("update is object");

    // Assert retired keys are strictly absent
    assert!(update.get("last_notified_version").is_none());
    assert!(update.get("legacy_old_dir_sweep_pending").is_none());
    assert!(update.get("pending_update_version").is_none());
    assert!(update.get("auto_apply").is_none());

    // Assert active keys are preserved correctly
    assert_eq!(update["auto_check"], true);
    assert_eq!(update["channel"], "beta");
    assert_eq!(update["skip_version"], "v3.9.0");
    assert_eq!(update["check_interval_s"], 43200);
    assert_eq!(update["last_check_ts"], 12345);
    assert_eq!(update["last_error_ts"], 0);

    // Assert unknown/future fields are preserved
    assert_eq!(raw["future_root"], "keep_me");
    assert_eq!(update["unknown_nested"]["preserve"], 99);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn settings_store_reads_legacy_timing_profile_and_preserves_unknown_data() {
    let root = std::env::temp_dir().join(format!("sky-legacy-settings-{}", std::process::id()));
    let path = root.join("config.json");
    fs::create_dir_all(&root).expect("temp root");
    fs::write(
            &path,
            br#"{"schema_version":2,"default_timing_profile":"audience-safe","timing_profiles":{"audience_safe":{"min_hold_frames":1.5}},"future":true}"#,
        )
        .expect("seed");
    let store = JsonSettingsStore::new(&path);
    let settings = store.load().expect("load legacy settings");
    assert_eq!(settings.playback_defaults.hold_frames, 1.5);
    assert_eq!(
        settings.playback_defaults.timing_margin_us,
        DEFAULT_TIMING_MARGIN_US
    );
    store.save(&settings).expect("save migrated settings");
    let raw: Value = serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("json");
    assert_eq!(raw["future"], true);
    assert_eq!(
        raw["schema_version"],
        sky_app_core::settings::SCHEMA_VERSION
    );
    assert_eq!(raw["default_timing_margin_us"], DEFAULT_TIMING_MARGIN_US);
    assert!(raw.get("default_timing_profile").is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn settings_store_round_trips_exact_timing_margin() {
    let root =
        std::env::temp_dir().join(format!("sky-timing-margin-settings-{}", std::process::id()));
    let path = root.join("config.json");
    fs::create_dir_all(&root).expect("temp root");
    fs::write(
        &path,
        br#"{"schema_version":3,"default_timing_margin_us":1200}"#,
    )
    .expect("seed prior schema with margin");
    let store = JsonSettingsStore::new(&path);
    let settings = store.load().expect("load settings");
    assert_eq!(settings.playback_defaults.timing_margin_us, 1_200);
    store.save(&settings).expect("save settings");
    let raw: Value = serde_json::from_slice(&fs::read(&path).expect("read")).expect("valid JSON");
    assert_eq!(raw["default_timing_margin_us"], 1_200);
    assert_eq!(
        raw["schema_version"],
        sky_app_core::settings::SCHEMA_VERSION
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn settings_store_persists_auto_play_without_changing_timing() {
    let root = std::env::temp_dir().join(format!("sky-auto-play-settings-{}", std::process::id()));
    let path = root.join("config.json");
    fs::create_dir_all(&root).expect("temp root");
    fs::write(
        &path,
        br#"{"schema_version":5,"default_timing_margin_us":1200}"#,
    )
    .expect("seed prior schema");

    let store = JsonSettingsStore::new(&path);
    let mut service = SettingsService::load(store.clone()).expect("load settings");
    assert!(service.snapshot().playback_behavior.auto_play);
    let original_defaults = service.snapshot().playback_defaults.clone();
    service
        .patch(&SettingsPatch {
            auto_play: Some(false),
            ..Default::default()
        })
        .expect("disable auto play");
    assert_eq!(service.snapshot().playback_defaults, original_defaults);
    let raw: Value =
        serde_json::from_slice(&fs::read(&path).expect("read settings")).expect("valid json");
    assert_eq!(
        raw["schema_version"],
        sky_app_core::settings::SCHEMA_VERSION
    );
    assert_eq!(raw["auto_play"], false);

    let reloaded = store.load().expect("reload settings");
    assert!(!reloaded.playback_behavior.auto_play);
    assert_eq!(reloaded.playback_defaults, original_defaults);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn settings_migration_canonicalizes_invalid_timing_margin_in_persisted_file() {
    for invalid_margin in [799_u64, 3_001_u64] {
        let root = std::env::temp_dir().join(format!(
            "sky-invalid-timing-margin-{}-{invalid_margin}",
            std::process::id()
        ));
        let path = root.join("config.json");
        fs::create_dir_all(&root).expect("temp root");
        fs::write(
            &path,
            format!("{{\"schema_version\":3,\"default_timing_margin_us\":{invalid_margin}}}"),
        )
        .expect("seed invalid persisted margin");

        let store = JsonSettingsStore::new(&path);
        let settings = store.load().expect("load settings");
        assert_eq!(
            settings.playback_defaults.timing_margin_us,
            DEFAULT_TIMING_MARGIN_US
        );
        let raw: Value = serde_json::from_slice(&fs::read(&path).expect("read migrated config"))
            .expect("valid migrated json");
        assert_eq!(
            raw["schema_version"],
            sky_app_core::settings::SCHEMA_VERSION
        );
        assert_eq!(raw["default_timing_margin_us"], DEFAULT_TIMING_MARGIN_US);

        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn settings_v6_migration_discards_down_late_grace_without_changing_margin() {
    let cases = [
        ("missing", None),
        ("default", Some("2000")),
        ("minimum", Some("0")),
        ("maximum", Some("5000")),
        ("malformed_step", Some("799")),
        ("malformed_type", Some(r#""not-a-number""#)),
    ];
    for (case, value) in cases {
        let root = std::env::temp_dir().join(format!(
            "sky-v6-down-late-grace-{}-{case}",
            std::process::id(),
        ));
        let path = root.join("config.json");
        fs::create_dir_all(&root).expect("temp root");
        let raw_value = value
            .map(|value| format!(",\"default_down_late_grace_us\":{value}"))
            .unwrap_or_default();
        fs::write(
            &path,
            format!("{{\"schema_version\":6,\"default_timing_margin_us\":1200{raw_value}}}"),
        )
        .expect("seed schema v6 settings");

        let settings = JsonSettingsStore::new(&path)
            .load()
            .expect("migrate settings");
        assert_eq!(settings.playback_defaults.timing_margin_us, 1_200);
        assert!(settings.playback_behavior.auto_play);
        let raw: Value = serde_json::from_slice(&fs::read(&path).expect("read migrated config"))
            .expect("valid migrated json");
        assert_eq!(
            raw["schema_version"],
            sky_app_core::settings::SCHEMA_VERSION
        );
        assert!(raw.get("default_down_late_grace_us").is_none());
        assert_eq!(raw["default_timing_margin_us"], 1_200);
        assert_eq!(raw["auto_play"], true);
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn legacy_normal_tolerance_is_ignored_and_removed_without_changing_timing_policy() {
    let mut snapshots = Vec::new();
    for (suffix, legacy_value) in [("low", 2_000), ("high", 10_000)] {
        let root = std::env::temp_dir().join(format!(
            "sky-legacy-tolerance-{}-{suffix}",
            std::process::id()
        ));
        let path = root.join("config.json");
        fs::create_dir_all(&root).expect("temp root");
        fs::write(
                &path,
                format!(
                    "{{\"schema_version\":7,\"default_timing_margin_us\":800,\"default_normal_down_start_tolerance_us\":{legacy_value}}}"
                ),
            )
            .expect("seed legacy settings");

        let store = JsonSettingsStore::new(&path);
        let settings = store.load().expect("load legacy settings");
        assert_eq!(settings.playback_defaults.timing_margin_us, 800);
        store.save(&settings).expect("save canonical settings");
        let raw: Value = serde_json::from_slice(&fs::read(&path).expect("read canonical settings"))
            .expect("valid canonical JSON");
        assert!(raw.get("default_normal_down_start_tolerance_us").is_none());
        snapshots.push((settings, raw));
        let _ = fs::remove_dir_all(root);
    }
    assert_eq!(snapshots[0].0, snapshots[1].0);
    assert_eq!(snapshots[0].1, snapshots[1].1);
}
