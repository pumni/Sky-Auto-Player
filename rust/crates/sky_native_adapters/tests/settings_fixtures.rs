use serde_json::Value;
use sky_app_core::settings::SettingsService;
use sky_native_adapters::JsonSettingsStore;
use std::fs;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../../tests/fixtures/wave2/settings.json"
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
            "sky-w2-settings-fixture-{}-{index}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("fixture root");
        let path = root.join("config.json");
        write_input(&path, &case["input"]);

        let store = JsonSettingsStore::new(&path);
        let service = SettingsService::load(store)
            .unwrap_or_else(|error| panic!("Rust adapter rejected Python fixture {name}: {error}"));
        let actual = serde_json::to_value(service.snapshot()).expect("serialize settings");
        assert_eq!(actual, case["normalized"], "normalized case {name}");

        let migrated: Value = serde_json::from_str(
            &fs::read_to_string(&path).expect("Python-compatible load writes migrated config"),
        )
        .expect("migrated config remains JSON");
        assert_eq!(migrated, case["migrated"], "migrated case {name}");

        fs::remove_dir_all(root).expect("remove fixture root");
    }
}
