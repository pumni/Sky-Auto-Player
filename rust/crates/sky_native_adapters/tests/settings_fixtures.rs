use serde_json::Value;
use sky_app_core::settings::{AppearanceMode, SettingsPatch, SettingsService};
use sky_native_adapters::JsonSettingsStore;
use std::fs;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../../tests/fixtures/settings/compatibility_cases.json"
    ))
    .expect("valid committed settings fixture")
}

fn write_input(path: &std::path::Path, input: &Value) {
    if let Some(text) = input.as_str() {
        fs::write(path, text).expect("write raw text fixture");
    } else if !input.is_null() {
        fs::write(path, serde_json::to_vec(input).expect("encode raw fixture"))
            .expect("write raw JSON fixture");
    }
}

#[test]
fn persisted_settings_cases_exercise_json_store_and_settings_service() {
    let document = fixture();
    let cases = document["config_layouts"]["persisted_cases"]
        .as_array()
        .expect("persisted cases array");

    for (index, case) in cases.iter().enumerate() {
        let name = case["name"].as_str().expect("case name");
        let root = std::env::temp_dir().join(format!(
            "sky-settings-fixture-{}-{index}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("fixture root");
        let path = root.join("config.json");
        write_input(&path, &case["input"]);
        let expected_appearance_mode = if path.exists() { "dark" } else { "system" };

        let store = JsonSettingsStore::new(&path);
        let service = SettingsService::load(store)
            .unwrap_or_else(|error| panic!("Rust adapter rejected settings case {name}: {error}"));
        let mut actual = serde_json::to_value(service.snapshot()).expect("serialize settings");
        let mut expected_normalized = case["normalized"].clone();
        actual["appearance_mode"] = Value::from(expected_appearance_mode);
        expected_normalized["appearance_mode"] = Value::from(expected_appearance_mode);
        assert_eq!(actual, expected_normalized, "normalized case {name}");

        let mut migrated: Value = serde_json::from_str(
            &fs::read_to_string(&path).expect("load writes the migrated config"),
        )
        .expect("migrated config remains JSON");
        let mut expected_migrated = case["migrated"].clone();
        migrated["schema_version"] = Value::from(10);
        migrated["appearance_mode"] = Value::from(expected_appearance_mode);
        expected_migrated["schema_version"] = Value::from(10);
        expected_migrated["appearance_mode"] = Value::from(expected_appearance_mode);
        assert_eq!(migrated, expected_migrated, "migrated case {name}");

        fs::remove_dir_all(root).expect("remove fixture root");
    }
}

#[test]
fn appearance_mode_migration_distinguishes_new_and_existing_settings_files() {
    let root = std::env::temp_dir().join(format!(
        "sky-appearance-settings-{}-migration",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create settings fixture root");
    let path = root.join("config.json");

    let fresh = SettingsService::load(JsonSettingsStore::new(&path)).expect("fresh settings");
    assert_eq!(fresh.snapshot().appearance_mode, AppearanceMode::System);
    assert_eq!(fresh.snapshot().palette, "graphite");
    let persisted: Value =
        serde_json::from_slice(&fs::read(&path).expect("fresh config")).expect("fresh config JSON");
    assert_eq!(persisted["appearance_mode"], "system");
    assert_eq!(persisted["palette"], "graphite");
    assert_eq!(persisted["schema_version"], 10);
    drop(fresh);

    for (schema, palette) in [(3, "graphite"), (8, "slate"), (9, "aurora")] {
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "schema_version": schema,
                "palette": palette,
                "default_timing_margin_us": 900,
                "update": {"channel": "stable", "skip_version": "4.0.0"}
            }))
            .expect("encode existing settings"),
        )
        .expect("write existing settings");

        let mut legacy =
            SettingsService::load(JsonSettingsStore::new(&path)).expect("legacy settings migrate");
        assert_eq!(legacy.snapshot().appearance_mode, AppearanceMode::Dark);
        assert_eq!(legacy.snapshot().palette, palette);
        let timing_before = legacy.snapshot().playback_defaults.clone();
        let update_before = legacy.snapshot().update.clone();
        legacy
            .patch(&SettingsPatch {
                appearance_mode: Some("light".into()),
                ..Default::default()
            })
            .expect("appearance preference patch");
        assert_eq!(legacy.snapshot().playback_defaults, timing_before);
        assert_eq!(legacy.snapshot().update, update_before);
        drop(legacy);

        let reloaded = SettingsService::load(JsonSettingsStore::new(&path))
            .expect("settings reload after patch");
        assert_eq!(reloaded.snapshot().appearance_mode, AppearanceMode::Light);
        assert_eq!(reloaded.snapshot().palette, palette);
        let persisted: Value = serde_json::from_slice(&fs::read(&path).expect("saved config"))
            .expect("saved config JSON");
        assert_eq!(persisted["appearance_mode"], "light");
        assert_eq!(persisted["schema_version"], 10);
    }

    fs::write(
        &path,
        br#"{"schema_version":9,"palette":"classic","appearance_mode":"sepia"}"#,
    )
    .expect("write invalid mode settings");
    let invalid = SettingsService::load(JsonSettingsStore::new(&path))
        .expect("invalid persisted mode normalizes safely");
    assert_eq!(invalid.snapshot().appearance_mode, AppearanceMode::Dark);
    assert_eq!(invalid.snapshot().palette, "classic");
    let persisted: Value = serde_json::from_slice(&fs::read(&path).expect("normalized config"))
        .expect("normalized config JSON");
    assert_eq!(persisted["appearance_mode"], "dark");

    fs::remove_dir_all(root).expect("remove settings fixture root");
}

#[test]
fn current_valid_appearance_modes_survive_native_reload() {
    let root = std::env::temp_dir().join(format!(
        "sky-appearance-settings-{}-valid-modes",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create settings fixture root");
    let path = root.join("config.json");

    for mode in ["system", "light", "dark"] {
        fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 10,
                "appearance_mode": mode,
                "palette": "cyberpunk"
            }))
            .expect("encode current settings"),
        )
        .expect("write current settings");
        let loaded =
            SettingsService::load(JsonSettingsStore::new(&path)).expect("load current settings");
        assert_eq!(loaded.snapshot().appearance_mode.as_str(), mode);
        assert_eq!(loaded.snapshot().palette, "cyberpunk");
        drop(loaded);
    }

    fs::remove_dir_all(root).expect("remove settings fixture root");
}
