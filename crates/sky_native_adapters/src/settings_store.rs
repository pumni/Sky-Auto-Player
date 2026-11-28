use super::atomic_file;
use serde_json::{Map, Value};
use sky_app_core::library::LikedSongs;
use sky_app_core::settings::{
    AppearanceMode, ApplicationSettings, DEFAULT_GAME_FPS, DEFAULT_HOLD_FRAMES,
    DEFAULT_PROCESS_NAMES, DEFAULT_SONGS_DIR, DEFAULT_TIMING_MARGIN_US, DEFAULT_UPDATE_INTERVAL_S,
    HOLD_FRAME_OPTIONS, HotkeySettings, MAX_TIMING_MARGIN_US, MIN_TIMING_MARGIN_US, SafetySettings,
    SettingsError, SettingsStore, TIMING_MARGIN_STEP_US, UpdateChannel, UpdatePreferences,
    VALID_FPS, normalize_settings,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
#[derive(Clone)]
pub struct JsonSettingsStore {
    path: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

impl JsonSettingsStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn read_object(&self) -> Map<String, Value> {
        let Ok(text) = fs::read_to_string(&self.path) else {
            return Map::new();
        };
        serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default()
    }
}

impl SettingsStore for JsonSettingsStore {
    fn load(&self) -> Result<ApplicationSettings, SettingsError> {
        let is_new_settings_file = !self.path.exists();
        let raw = self.read_object();
        let migrated = migrate_raw(&raw, is_new_settings_file);
        if migrated != raw {
            let encoded = serde_json::to_vec_pretty(&Value::Object(migrated.clone()))
                .map_err(|error| SettingsError::Storage(error.to_string()))?;
            atomic_file::atomic_replace(&self.path, &encoded).map_err(SettingsError::Storage)?;
        }
        Ok(settings_from_raw(&migrated))
    }

    fn save(&self, settings: &ApplicationSettings) -> Result<(), SettingsError> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| SettingsError::Storage("settings write lock poisoned".into()))?;
        let mut raw = self.read_object();
        overlay_settings(&mut raw, settings);
        let encoded = serde_json::to_vec_pretty(&Value::Object(raw))
            .map_err(|error| SettingsError::Storage(error.to_string()))?;
        atomic_file::atomic_replace(&self.path, &encoded).map_err(SettingsError::Storage)
    }
}

fn settings_from_raw(raw: &Map<String, Value>) -> ApplicationSettings {
    let mut settings = ApplicationSettings {
        appearance_mode: raw
            .get("appearance_mode")
            .and_then(Value::as_str)
            .and_then(AppearanceMode::parse)
            .unwrap_or(AppearanceMode::Dark),
        ..ApplicationSettings::default()
    };
    let palette = raw
        .get("palette")
        .or_else(|| raw.get("theme"))
        .map(format_legacy_config_value)
        .unwrap_or_else(|| settings.palette.clone());
    settings.palette = normalized_palette(&palette);
    settings.ui_background_mode =
        raw_string(raw, "ui_background_mode", &settings.ui_background_mode);
    settings.playback_defaults.hold_frames =
        raw_f64(raw, "default_hold_frames", DEFAULT_HOLD_FRAMES);
    settings.playback_defaults.timing_margin_us = raw_u64(
        raw,
        "default_timing_margin_us",
        sky_app_core::settings::DEFAULT_TIMING_MARGIN_US,
    );
    settings.playback_defaults.tempo_scale = raw_f64(raw, "default_tempo_scale", 1.0);
    settings.playback_defaults.fps = raw_fps(raw, "game_fps", DEFAULT_GAME_FPS);
    settings.playback_behavior.auto_play = raw_bool(raw, "auto_play", true);
    settings.telemetry_enabled = raw_bool(raw, "telemetry_enabled_by_default", false);
    settings.verbose_hud = raw_bool(raw, "verbose_hud", false);
    settings.songs_dir = raw_string(raw, "songs_dir", DEFAULT_SONGS_DIR);
    settings.allow_title_fallback = raw_bool(raw, "allow_title_fallback", false);
    settings.liked_songs =
        LikedSongs::from_persisted(raw_string_list(raw.get("liked_song_ids")).unwrap_or_default());
    settings.sky_process_names =
        raw_string_list(raw.get("sky_process_names")).unwrap_or_else(|| {
            DEFAULT_PROCESS_NAMES
                .iter()
                .map(|value| (*value).into())
                .collect()
        });

    if let Some(hotkeys) = raw.get("hotkeys").and_then(Value::as_object) {
        settings.hotkeys = HotkeySettings {
            pause: object_string(hotkeys, "pause", &settings.hotkeys.pause),
            skip: object_string(hotkeys, "skip", &settings.hotkeys.skip),
            quit: object_string(hotkeys, "quit", &settings.hotkeys.quit),
            refocus: object_string(hotkeys, "refocus", &settings.hotkeys.refocus),
            panic: object_string(hotkeys, "panic", &settings.hotkeys.panic),
        };
    }
    if let Some(safety) = raw.get("safety").and_then(Value::as_object) {
        settings.safety = SafetySettings {
            prompt_on_medium_risk: object_bool(safety, "prompt_on_medium_risk", true),
            prompt_on_high_risk: object_bool(safety, "prompt_on_high_risk", true),
        };
    }
    if let Some(update) = raw.get("update").and_then(Value::as_object) {
        settings.update = UpdatePreferences {
            auto_check: object_bool(update, "auto_check", true),
            channel: UpdateChannel::parse(&object_string(update, "channel", "stable"))
                .unwrap_or(UpdateChannel::Stable),
            skip_version: object_string_only(update, "skip_version", ""),
            check_interval_s: object_i64(update, "check_interval_s", DEFAULT_UPDATE_INTERVAL_S)
                .max(0),
            last_check_ts: object_i64(update, "last_check_ts", 0),
            last_error_ts: object_i64(update, "last_error_ts", 0),
        };
    }
    normalize_settings(settings)
}

fn migrate_raw(raw: &Map<String, Value>, is_new_settings_file: bool) -> Map<String, Value> {
    let mut migrated = raw.clone();
    let appearance_mode = raw
        .get("appearance_mode")
        .and_then(Value::as_str)
        .and_then(AppearanceMode::parse)
        .unwrap_or({
            if is_new_settings_file {
                AppearanceMode::System
            } else {
                AppearanceMode::Dark
            }
        });
    let palette = raw
        .get("palette")
        .or_else(|| raw.get("theme"))
        .map(format_legacy_config_value)
        .unwrap_or_else(|| {
            if is_new_settings_file {
                ApplicationSettings::default().palette
            } else {
                "aurora".into()
            }
        });
    let fps = resolve_fps(
        raw.get("game_fps")
            .and_then(parse_legacy_integer_setting)
            .unwrap_or(i64::from(DEFAULT_GAME_FPS)),
    );
    let profile = raw
        .get("default_timing_profile")
        .map(format_legacy_config_value)
        .unwrap_or_else(|| "balanced".into())
        .to_lowercase()
        .replace('-', "_");
    let selected = selected_timing_profile(raw.get("timing_profiles"), &profile);

    let mut candidate = selected.and_then(|profile| {
        ["min_hold_frames", "hold_frames"]
            .into_iter()
            .find_map(|key| {
                profile
                    .get(key)
                    .and_then(parse_legacy_profile_number)
                    .and_then(nearest_hold_frames)
            })
    });
    if candidate.is_none() {
        let frame_us = (1_000_000_i64 + i64::from(fps) - 1) / i64::from(fps);
        candidate = selected.and_then(|profile| {
            ["min_hold_us", "hold_us"].into_iter().find_map(|key| {
                profile
                    .get(key)
                    .and_then(parse_legacy_profile_number)
                    .and_then(|value| nearest_hold_frames(value / frame_us as f64))
            })
        });
    }
    let candidate = candidate.unwrap_or_else(|| legacy_profile_hold(&profile));
    let hold = raw
        .get("default_hold_frames")
        .and_then(numeric_float)
        .and_then(nearest_supported_hold)
        .unwrap_or(DEFAULT_HOLD_FRAMES);
    migrated.insert(
        "schema_version".into(),
        Value::from(sky_app_core::settings::SCHEMA_VERSION),
    );
    migrated.insert("palette".into(), Value::from(normalized_palette(&palette)));
    migrated.insert(
        "appearance_mode".into(),
        Value::from(appearance_mode.as_str()),
    );
    migrated.remove("theme");
    migrated.insert(
        "default_hold_frames".into(),
        Value::from(if raw.contains_key("default_hold_frames") {
            hold
        } else {
            candidate
        }),
    );
    let timing_margin_us = raw_u64(raw, "default_timing_margin_us", DEFAULT_TIMING_MARGIN_US);
    let timing_margin_us = if (MIN_TIMING_MARGIN_US..=MAX_TIMING_MARGIN_US)
        .contains(&timing_margin_us)
        && timing_margin_us.is_multiple_of(TIMING_MARGIN_STEP_US)
    {
        timing_margin_us
    } else {
        DEFAULT_TIMING_MARGIN_US
    };
    migrated.insert(
        "default_timing_margin_us".into(),
        Value::from(timing_margin_us),
    );
    // The legacy normal late-tolerance key is accepted for compatibility but
    // deliberately discarded rather than mapped to any current timing policy.
    migrated.remove("default_normal_down_start_tolerance_us");
    migrated.remove("default_down_late_grace_us");
    migrated.insert(
        "auto_play".into(),
        Value::from(raw_bool(raw, "auto_play", true)),
    );
    for key in [
        "default_timing_profile",
        "timing_profiles",
        "frame_timing",
        "hold_us",
        "min_hold_us",
        "hold_frames",
        "min_hold_frames",
        "hold_unframed_us",
        "min_hold_unframed_us",
    ] {
        migrated.remove(key);
    }
    if let Some(Value::Object(update)) = migrated.get_mut("update") {
        for key in RETIRED_UPDATE_KEYS {
            update.remove(key);
        }
    }
    migrated
}

const RETIRED_UPDATE_KEYS: [&str; 4] = [
    "last_notified_version",
    "legacy_old_dir_sweep_pending",
    "pending_update_version",
    "auto_apply",
];

fn legacy_profile_hold(profile: &str) -> f64 {
    match profile {
        "audience_safe" | "remote_safe" | "online_audible_safe" | "online_audible" => 1.5,
        _ => DEFAULT_HOLD_FRAMES,
    }
}

fn selected_timing_profile<'a>(
    value: Option<&'a Value>,
    profile: &str,
) -> Option<&'a Map<String, Value>> {
    let profiles = value?.as_object()?;
    if let Some(selected) = profiles.get(profile) {
        if let Some(object) = selected.as_object() {
            if !object.is_empty() {
                return Some(object);
            }
        } else if !json_is_falsy(selected) {
            // A truthy non-object exact match is discarded without continuing
            // to an alias, matching the legacy profile lookup order.
            return None;
        }
    }
    profiles
        .iter()
        .find(|(key, _)| key.to_lowercase().replace('-', "_") == profile)
        .and_then(|(_, selected)| selected.as_object())
}

fn json_is_falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(value) => !value,
        Value::Number(value) => value.as_f64().is_some_and(|number| number == 0.0),
        Value::String(value) => value.is_empty(),
        Value::Array(value) => value.is_empty(),
        Value::Object(value) => value.is_empty(),
    }
}

fn raw_string(raw: &Map<String, Value>, key: &str, default: &str) -> String {
    raw.get(key)
        .map(format_legacy_config_value)
        .unwrap_or_else(|| default.into())
}

fn normalized_palette(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    if sky_app_core::settings::PALETTE_IDS.contains(&value.as_str()) {
        value
    } else {
        "aurora".into()
    }
}

fn raw_bool(raw: &Map<String, Value>, key: &str, default: bool) -> bool {
    raw.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn raw_f64(raw: &Map<String, Value>, key: &str, default: f64) -> f64 {
    raw.get(key)
        .and_then(parse_json_number_or_decimal_string)
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}

fn raw_u64(raw: &Map<String, Value>, key: &str, default: u64) -> u64 {
    raw.get(key).and_then(Value::as_u64).unwrap_or(default)
}

fn raw_fps(raw: &Map<String, Value>, key: &str, default: u16) -> u16 {
    resolve_fps(
        raw.get(key)
            .and_then(parse_legacy_integer_setting)
            .unwrap_or(i64::from(default)),
    )
}

fn raw_string_list(value: Option<&Value>) -> Option<Vec<String>> {
    value?
        .as_array()
        .map(|items| items.iter().map(format_legacy_config_value).collect())
}

fn object_string(object: &Map<String, Value>, key: &str, default: &str) -> String {
    object
        .get(key)
        .map(format_legacy_config_value)
        .unwrap_or_else(|| default.into())
}

fn object_string_only(object: &Map<String, Value>, key: &str, default: &str) -> String {
    object
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_owned()
}

fn object_bool(object: &Map<String, Value>, key: &str, default: bool) -> bool {
    object.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn object_i64(object: &Map<String, Value>, key: &str, default: i64) -> i64 {
    object.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn format_legacy_config_value(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(value) => {
            if *value {
                "True".into()
            } else {
                "False".into()
            }
        }
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(format_legacy_config_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| {
                    format!(
                        "{}: {}",
                        quote_legacy_config_string(key),
                        format_legacy_config_repr(value)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn format_legacy_config_repr(value: &Value) -> String {
    match value {
        Value::String(value) => quote_legacy_config_string(value),
        _ => format_legacy_config_value(value),
    }
}

fn quote_legacy_config_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn parse_json_number_or_decimal_string(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(_) | Value::Null => None,
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn parse_legacy_profile_number(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        _ => parse_json_number_or_decimal_string(value),
    }
}

fn numeric_float(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(_) | Value::Null | Value::String(_) | Value::Array(_) | Value::Object(_) => {
            None
        }
        Value::Number(value) => value.as_f64(),
    }
}

fn parse_legacy_integer_setting(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(_) | Value::Null => None,
        Value::Number(value) => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .or_else(|| {
                value
                    .as_f64()
                    .map(|value| value.trunc())
                    .filter(|value| value.is_finite())
                    .and_then(|value| i64::try_from(value as i128).ok())
            }),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn nearest_hold_frames(value: f64) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    let mut best = HOLD_FRAME_OPTIONS[0];
    for option in HOLD_FRAME_OPTIONS {
        if (option - value).abs() <= (best - value).abs() {
            best = option;
        }
    }
    Some(best)
}

fn nearest_supported_hold(value: f64) -> Option<f64> {
    HOLD_FRAME_OPTIONS.contains(&value).then_some(value)
}

fn resolve_fps(value: i64) -> u16 {
    u16::try_from(value)
        .ok()
        .filter(|value| VALID_FPS.contains(value))
        .unwrap_or(DEFAULT_GAME_FPS)
}

fn overlay_settings(raw: &mut Map<String, Value>, settings: &ApplicationSettings) {
    raw.insert(
        "schema_version".into(),
        Value::from(sky_app_core::settings::SCHEMA_VERSION),
    );
    raw.insert("palette".into(), Value::from(settings.palette.clone()));
    raw.insert(
        "appearance_mode".into(),
        Value::from(settings.appearance_mode.as_str()),
    );
    raw.remove("theme");
    raw.insert(
        "ui_background_mode".into(),
        Value::from(settings.ui_background_mode.clone()),
    );
    raw.insert(
        "default_hold_frames".into(),
        Value::from(settings.playback_defaults.hold_frames),
    );
    raw.insert(
        "default_timing_margin_us".into(),
        Value::from(settings.playback_defaults.timing_margin_us),
    );
    raw.remove("default_normal_down_start_tolerance_us");
    raw.remove("default_down_late_grace_us");
    raw.insert(
        "default_tempo_scale".into(),
        Value::from(settings.playback_defaults.tempo_scale),
    );
    raw.insert(
        "game_fps".into(),
        Value::from(settings.playback_defaults.fps),
    );
    raw.insert(
        "auto_play".into(),
        Value::from(settings.playback_behavior.auto_play),
    );
    raw.insert(
        "telemetry_enabled_by_default".into(),
        Value::from(settings.telemetry_enabled),
    );
    raw.insert("verbose_hud".into(), Value::from(settings.verbose_hud));
    raw.insert("songs_dir".into(), Value::from(settings.songs_dir.clone()));
    raw.insert(
        "sky_process_names".into(),
        Value::Array(
            settings
                .sky_process_names
                .iter()
                .cloned()
                .map(Value::from)
                .collect(),
        ),
    );
    raw.insert(
        "allow_title_fallback".into(),
        Value::from(settings.allow_title_fallback),
    );
    if !settings.liked_songs.is_empty() || raw.contains_key("liked_song_ids") {
        raw.insert(
            "liked_song_ids".into(),
            Value::Array(
                settings
                    .liked_songs
                    .ids()
                    .iter()
                    .cloned()
                    .map(Value::from)
                    .collect(),
            ),
        );
    }
    raw.insert("hotkeys".into(), serde_json::json!({"pause": settings.hotkeys.pause, "skip": settings.hotkeys.skip, "quit": settings.hotkeys.quit, "refocus": settings.hotkeys.refocus, "panic": settings.hotkeys.panic}));
    raw.insert("safety".into(), serde_json::json!({"prompt_on_medium_risk": settings.safety.prompt_on_medium_risk, "prompt_on_high_risk": settings.safety.prompt_on_high_risk}));
    let mut update = raw
        .remove("update")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    for key in RETIRED_UPDATE_KEYS {
        update.remove(key);
    }
    update.insert("auto_check".into(), Value::from(settings.update.auto_check));
    update.insert(
        "channel".into(),
        Value::from(settings.update.channel.as_str()),
    );
    update.insert(
        "skip_version".into(),
        Value::from(settings.update.skip_version.clone()),
    );
    update.insert(
        "check_interval_s".into(),
        Value::from(settings.update.check_interval_s),
    );
    update.insert(
        "last_check_ts".into(),
        Value::from(settings.update.last_check_ts),
    );
    update.insert(
        "last_error_ts".into(),
        Value::from(settings.update.last_error_ts),
    );
    raw.insert("update".into(), Value::Object(update));
    for key in [
        "default_timing_profile",
        "timing_profiles",
        "frame_timing",
        "hold_us",
        "min_hold_us",
        "hold_frames",
        "min_hold_frames",
        "hold_unframed_us",
        "min_hold_unframed_us",
    ] {
        raw.remove(key);
    }
}

#[cfg(test)]
#[path = "settings_store_tests.rs"]
mod tests;
