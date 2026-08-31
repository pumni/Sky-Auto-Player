use serde_json::Value;
use sky_app_core::catalog::{CatalogIndex, CatalogSourceEntry};
use sky_app_core::settings::{
    ApplicationSettings, PlaybackDefaultsPatch, SettingsPatch, SettingsService, SettingsStore,
    UpdateChannel,
};
use sky_app_core::update::{channel_policy, retry_delay, should_auto_check};

fn fixture(name: &str) -> Value {
    let text = match name {
        "settings" => include_str!("../../../../tests/fixtures/wave2/settings.json"),
        "catalog" => include_str!("../../../../tests/fixtures/wave2/catalog.json"),
        "update" => include_str!("../../../../tests/fixtures/wave2/update.json"),
        _ => unreachable!(),
    };
    serde_json::from_str(text).expect("valid committed Wave 2 fixture")
}

#[derive(Default)]
struct MemoryStore(ApplicationSettings);

impl SettingsStore for MemoryStore {
    fn load(&self) -> Result<ApplicationSettings, sky_app_core::settings::SettingsError> {
        Ok(self.0.clone())
    }

    fn save(
        &self,
        _settings: &ApplicationSettings,
    ) -> Result<(), sky_app_core::settings::SettingsError> {
        Ok(())
    }
}

#[test]
fn settings_fixture_preserves_python_patch_and_atomic_failure_semantics() {
    let raw = fixture("settings");
    assert_eq!(
        raw["config_layouts"]["legacy_v2"]["normalized_hold_frames"],
        1.5
    );
    assert_eq!(
        raw["config_layouts"]["legacy_v2"]["migrated_schema_version"],
        3
    );
    assert_eq!(
        raw["config_layouts"]["current_v3"]["normalized_theme"],
        "slate"
    );
    let valid = &raw["valid_patch"];
    let mut service = SettingsService::load(MemoryStore::default()).expect("load settings");
    let patched = service
        .patch(&SettingsPatch {
            theme: Some(valid["theme"].as_str().unwrap().into()),
            telemetry_enabled: Some(valid["telemetry_enabled"].as_bool().unwrap()),
            verbose_hud: Some(valid["verbose_hud"].as_bool().unwrap()),
            playback_defaults: Some(PlaybackDefaultsPatch {
                hold_frames: Some(valid["default_hold_frames"].as_f64().unwrap()),
                tempo_scale: Some(valid["default_tempo_scale"].as_f64().unwrap()),
                fps: Some(valid["game_fps"].as_u64().unwrap() as u16),
            }),
            update: Some(sky_app_core::settings::UpdatePreferencesPatch {
                auto_check: Some(valid["update_preferences"]["auto_check"].as_bool().unwrap()),
                channel: Some(UpdateChannel::Beta),
                skip_version: Some(
                    valid["update_preferences"]["skip_version"]
                        .as_str()
                        .unwrap()
                        .into(),
                ),
            }),
        })
        .expect("valid patch");
    assert_eq!(patched.theme, "slate");
    assert_eq!(patched.playback_defaults.fps, 120);
    assert_eq!(patched.update.channel, UpdateChannel::Beta);
    assert_eq!(patched.update.skip_version, "3.6.0");
    let before = service.snapshot().clone();
    assert!(
        service
            .patch(&SettingsPatch {
                playback_defaults: Some(PlaybackDefaultsPatch {
                    fps: Some(61),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .is_err()
    );
    assert_eq!(*service.snapshot(), before);
}

#[test]
fn catalog_fixture_preserves_ids_order_normalization_and_generation() {
    let raw = fixture("catalog");
    let mut index = CatalogIndex::default();
    let entries = raw["paths"].as_array().unwrap().iter().filter_map(|path| {
        let path = path.as_str()?;
        let extension = std::path::Path::new(path).extension()?.to_str()?;
        if extension.eq_ignore_ascii_case("csv") {
            return None;
        }
        let title = std::path::Path::new(path).file_stem()?.to_str()?.to_owned();
        Some(CatalogSourceEntry {
            canonical_path: path.into(),
            title,
        })
    });
    let snapshot = index.replace_entries(entries).expect("catalog index");
    assert_eq!(
        snapshot,
        serde_json::from_value(raw["snapshot"].clone()).unwrap()
    );
    for (query, expected) in raw["substring_queries"].as_object().unwrap() {
        let page = index
            .search_substrings(query, 0, 20, Some(1))
            .expect("substring query");
        assert_eq!(page, serde_json::from_value(expected.clone()).unwrap());
    }
    assert_eq!(
        sky_app_core::catalog::normalize_search_text("Cà Phê"),
        "ca phe"
    );
    assert_eq!(sky_app_core::catalog::normalize_search_text("Đàn"), "dan");
    for (value, expected) in raw["normalized"].as_object().unwrap() {
        assert_eq!(
            sky_app_core::catalog::normalize_search_text(value),
            expected.as_str().unwrap(),
            "normalization fixture {value}"
        );
    }
    for case in raw["window_cases"].as_array().unwrap() {
        let query = case["query"].as_str().unwrap();
        let offset = case["offset"].as_u64().unwrap() as usize;
        let limit = case["limit"].as_u64().unwrap() as usize;
        let result = index.search_substrings(query, offset, limit, Some(1));
        match case["status"].as_str().unwrap() {
            "ok" => {
                let expected: sky_app_core::catalog::CatalogPage =
                    serde_json::from_value(case["page"].clone()).unwrap();
                assert_eq!(
                    result.expect("accepted catalog window"),
                    expected,
                    "window fixture {case}"
                );
            }
            "error" => {
                let error = result.expect_err("rejected catalog window");
                let expected_error = if case["error"].as_str().unwrap().contains("query exceeds") {
                    sky_app_core::catalog::CatalogError::QueryTooLong
                } else {
                    sky_app_core::catalog::CatalogError::InvalidLimit
                };
                assert_eq!(error, expected_error, "window fixture {case}");
            }
            other => panic!("unexpected catalog fixture status {other}"),
        }
    }
    for case in raw["fuzzy_cases"].as_array().unwrap() {
        let query = case["query"].as_str().unwrap();
        let candidate = case["candidate"].as_str().unwrap();
        let expected = case["score"].as_f64().unwrap();
        let actual = sky_app_core::catalog::wratio_score(query, candidate);
        assert!(
            (actual - expected).abs() < 1e-9,
            "WRatio fixture {query:?} vs {candidate:?}: expected {expected}, got {actual}"
        );
    }
    assert!(
        index
            .search_substrings("", 1_000_000_001, 1, Some(1))
            .expect("large Python-compatible offset")
            .items
            .is_empty()
    );
    assert_eq!(
        index
            .canonical_path_for_song_id("not-a-song-id", Some(1))
            .unwrap_err(),
        sky_app_core::catalog::CatalogError::UnknownSongId
    );
}

#[test]
fn update_fixture_preserves_channel_and_throttle_policy() {
    let raw = fixture("update");
    assert_eq!(
        channel_policy(&UpdateChannel::Stable).github_api_path,
        raw["channels"]["stable"]["github_api_path"]
    );
    assert_eq!(
        channel_policy(&UpdateChannel::Beta).include_prerelease,
        raw["channels"]["beta"]["include_prerelease"]
    );
    let preferences = sky_app_core::settings::UpdatePreferences {
        last_check_ts: 1_000,
        ..Default::default()
    };
    assert_eq!(
        should_auto_check(&preferences, 1_500),
        raw["throttle"]["within_success_interval"]
    );
    assert_eq!(
        should_auto_check(&preferences, 87_400),
        raw["throttle"]["at_success_boundary"]
    );
    let retry = sky_app_core::settings::UpdatePreferences {
        last_error_ts: 1_000,
        ..Default::default()
    };
    assert_eq!(retry_delay(&retry, 1_100), raw["throttle"]["retry_delay"]);
    assert_eq!(
        should_auto_check(&retry, 1_300),
        raw["throttle"]["retry_at_boundary"]
    );
}
