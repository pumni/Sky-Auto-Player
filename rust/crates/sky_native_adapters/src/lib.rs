//! Outer adapters for Wave 2 application services.
//!
//! This crate owns filesystem enumeration and persisted config compatibility.
//! It does not contain Tauri delivery policy, Python transport, realtime
//! playback, or updater transaction logic.

use serde_json::{Map, Value};
use sky_app_core::catalog::{CatalogError, CatalogSourceEntry, SUPPORTED_EXTENSIONS, SongSource};
use sky_app_core::settings::{
    ApplicationSettings, DEFAULT_GAME_FPS, DEFAULT_HOLD_FRAMES, DEFAULT_PROCESS_NAMES,
    DEFAULT_SONGS_DIR, DEFAULT_UPDATE_INTERVAL_S, HOLD_FRAME_OPTIONS, HotkeySettings,
    SafetySettings, SettingsError, SettingsStore, UpdateChannel, UpdatePreferences, VALID_FPS,
    normalize_settings,
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
        let raw = self.read_object();
        let migrated = migrate_raw(&raw);
        if migrated != raw {
            let encoded = serde_json::to_vec_pretty(&Value::Object(migrated.clone()))
                .map_err(|error| SettingsError::Storage(error.to_string()))?;
            atomic_replace(&self.path, &encoded)?;
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
        atomic_replace(&self.path, &encoded)
    }
}

pub struct FileCatalogSource {
    root: PathBuf,
}

impl FileCatalogSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl SongSource for FileCatalogSource {
    fn entries(&self) -> Result<Vec<CatalogSourceEntry>, CatalogError> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        if !self.root.is_dir() {
            return Err(CatalogError::SourceUnavailable(
                "songs directory is not a directory".into(),
            ));
        }
        let mut entries = Vec::new();
        let read_dir = fs::read_dir(&self.root)
            .map_err(|error| CatalogError::SourceUnavailable(error.to_string()))?;
        for item in read_dir {
            let path = item
                .map_err(|error| CatalogError::SourceUnavailable(error.to_string()))?
                .path();
            if !path.is_file() || !is_supported(&path) {
                continue;
            }
            let canonical = fs::canonicalize(&path)
                .map_err(|error| CatalogError::SourceUnavailable(error.to_string()))?;
            let title = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_owned();
            entries.push(CatalogSourceEntry {
                canonical_path: canonical.to_string_lossy().into_owned(),
                title,
            });
        }
        entries.sort_by(|left, right| left.canonical_path.cmp(&right.canonical_path));
        Ok(entries)
    }
}

fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn settings_from_raw(raw: &Map<String, Value>) -> ApplicationSettings {
    let mut settings = ApplicationSettings::default();
    settings.theme = raw_string(raw, "theme", &settings.theme);
    settings.ui_background_mode =
        raw_string(raw, "ui_background_mode", &settings.ui_background_mode);
    settings.playback_defaults.hold_frames =
        raw_f64(raw, "default_hold_frames", DEFAULT_HOLD_FRAMES);
    settings.playback_defaults.tempo_scale = raw_f64(raw, "default_tempo_scale", 1.0);
    settings.playback_defaults.fps = raw_fps(raw, "game_fps", DEFAULT_GAME_FPS);
    settings.telemetry_enabled = raw_bool(raw, "telemetry_enabled_by_default", false);
    settings.verbose_hud = raw_bool(raw, "verbose_hud", false);
    settings.songs_dir = raw_string(raw, "songs_dir", DEFAULT_SONGS_DIR);
    settings.allow_title_fallback = raw_bool(raw, "allow_title_fallback", false);
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
            last_notified_version: object_string_only(update, "last_notified_version", ""),
            legacy_old_dir_sweep_pending: object_bool(
                update,
                "legacy_old_dir_sweep_pending",
                false,
            ) || update.contains_key("pending_update_version")
                || update.contains_key("auto_apply"),
        };
    }
    normalize_settings(settings)
}

fn migrate_raw(raw: &Map<String, Value>) -> Map<String, Value> {
    let mut migrated = raw.clone();
    let fps = resolve_fps(
        raw.get("game_fps")
            .and_then(python_int)
            .unwrap_or(i64::from(DEFAULT_GAME_FPS)),
    );
    let profile = raw
        .get("default_timing_profile")
        .map(python_string)
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
                    .and_then(legacy_float)
                    .and_then(nearest_hold_frames)
            })
    });
    if candidate.is_none() {
        let frame_us = (1_000_000_i64 + i64::from(fps) - 1) / i64::from(fps);
        candidate = selected.and_then(|profile| {
            ["min_hold_us", "hold_us"].into_iter().find_map(|key| {
                profile
                    .get(key)
                    .and_then(legacy_float)
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
    migrated.insert("schema_version".into(), Value::from(3_u32));
    migrated.insert(
        "default_hold_frames".into(),
        Value::from(if raw.contains_key("default_hold_frames") {
            hold
        } else {
            candidate
        }),
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
    migrated
}

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
            // Python keeps a truthy non-dict exact match and then discards it;
            // it does not continue looking for an alias in that case.
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
        .map(python_string)
        .unwrap_or_else(|| default.into())
}

fn raw_bool(raw: &Map<String, Value>, key: &str, default: bool) -> bool {
    raw.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn raw_f64(raw: &Map<String, Value>, key: &str, default: f64) -> f64 {
    raw.get(key)
        .and_then(python_float)
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}

fn raw_fps(raw: &Map<String, Value>, key: &str, default: u16) -> u16 {
    resolve_fps(
        raw.get(key)
            .and_then(python_int)
            .unwrap_or(i64::from(default)),
    )
}

fn raw_string_list(value: Option<&Value>) -> Option<Vec<String>> {
    value?
        .as_array()
        .map(|items| items.iter().map(python_string).collect())
}

fn object_string(object: &Map<String, Value>, key: &str, default: &str) -> String {
    object
        .get(key)
        .map(python_string)
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

fn python_string(value: &Value) -> String {
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
                .map(python_repr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!("{}: {}", python_repr_string(key), python_repr(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn python_repr(value: &Value) -> String {
    match value {
        Value::String(value) => python_repr_string(value),
        _ => python_string(value),
    }
}

fn python_repr_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn python_float(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(_) | Value::Null => None,
        Value::Number(value) => value.as_f64(),
        Value::String(value) => value.trim().parse::<f64>().ok(),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn legacy_float(value: &Value) -> Option<f64> {
    match value {
        Value::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        _ => python_float(value),
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

fn python_int(value: &Value) -> Option<i64> {
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
    raw.insert("theme".into(), Value::from(settings.theme.clone()));
    raw.insert(
        "ui_background_mode".into(),
        Value::from(settings.ui_background_mode.clone()),
    );
    raw.insert(
        "default_hold_frames".into(),
        Value::from(settings.playback_defaults.hold_frames),
    );
    raw.insert(
        "default_tempo_scale".into(),
        Value::from(settings.playback_defaults.tempo_scale),
    );
    raw.insert(
        "game_fps".into(),
        Value::from(settings.playback_defaults.fps),
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
    raw.insert("hotkeys".into(), serde_json::json!({"pause": settings.hotkeys.pause, "skip": settings.hotkeys.skip, "quit": settings.hotkeys.quit, "refocus": settings.hotkeys.refocus, "panic": settings.hotkeys.panic}));
    raw.insert("safety".into(), serde_json::json!({"prompt_on_medium_risk": settings.safety.prompt_on_medium_risk, "prompt_on_high_risk": settings.safety.prompt_on_high_risk}));
    let mut update = raw
        .remove("update")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
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
    update.insert(
        "last_notified_version".into(),
        Value::from(settings.update.last_notified_version.clone()),
    );
    update.insert(
        "legacy_old_dir_sweep_pending".into(),
        Value::from(settings.update.legacy_old_dir_sweep_pending),
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

fn atomic_replace(path: &Path, contents: &[u8]) -> Result<(), SettingsError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| SettingsError::Storage(error.to_string()))?;
    }
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, contents).map_err(|error| SettingsError::Storage(error.to_string()))?;
    replace_file(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        SettingsError::Storage(error)
    })
}

#[cfg(windows)]
fn replace_file(temp: &Path, target: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source = temp
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file(temp: &Path, target: &Path) -> Result<(), String> {
    fs::rename(temp, target).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_app_core::settings::{PlaybackDefaultsPatch, SettingsPatch, SettingsService};

    #[test]
    fn settings_store_preserves_unknown_keys_and_writes_atomically() {
        let root = std::env::temp_dir().join(format!("sky-w2-settings-{}", std::process::id()));
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
        let raw: Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("json");
        assert_eq!(raw["future_field"], 42);
        assert_eq!(raw["update"]["future"], true);
        assert_eq!(raw["game_fps"], 120);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn catalog_source_handles_missing_directory_as_empty() {
        let source = FileCatalogSource::new(std::env::temp_dir().join("sky-w2-no-such-directory"));
        assert!(source.entries().expect("missing root").is_empty());
    }

    #[test]
    fn settings_store_reads_legacy_timing_profile_and_preserves_unknown_data() {
        let root =
            std::env::temp_dir().join(format!("sky-w2-legacy-settings-{}", std::process::id()));
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
        store.save(&settings).expect("save migrated settings");
        let raw: Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("json");
        assert_eq!(raw["future"], true);
        assert!(raw.get("default_timing_profile").is_none());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn catalog_source_rejects_non_directory_and_ignores_unsupported_files() {
        let root = std::env::temp_dir().join(format!("sky-w2-catalog-{}", std::process::id()));
        fs::create_dir_all(&root).expect("temp root");
        fs::write(root.join("song.txt"), "notes").expect("song");
        fs::write(root.join("ignored.csv"), "ignored").expect("ignored");
        let entries = FileCatalogSource::new(&root).entries().expect("entries");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].canonical_path.ends_with("song.txt"));
        let file = root.join("not-a-directory");
        fs::write(&file, "file").expect("file");
        assert!(matches!(
            FileCatalogSource::new(file).entries(),
            Err(CatalogError::SourceUnavailable(_))
        ));
        let _ = fs::remove_dir_all(root);
    }
}
