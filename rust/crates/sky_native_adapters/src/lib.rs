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
    SafetySettings, SettingsError, SettingsStore, UpdateChannel, UpdatePreferences,
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
        Ok(settings_from_raw(&self.read_object()))
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
    settings.playback_defaults.hold_frames = raw_hold_frames(raw);
    settings.playback_defaults.tempo_scale = raw_f64(raw, "default_tempo_scale", 1.0);
    settings.playback_defaults.fps = raw_u16(raw, "game_fps", DEFAULT_GAME_FPS);
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
            skip_version: object_string(update, "skip_version", ""),
            check_interval_s: object_i64(update, "check_interval_s", DEFAULT_UPDATE_INTERVAL_S)
                .max(0),
            last_check_ts: object_i64(update, "last_check_ts", 0),
            last_error_ts: object_i64(update, "last_error_ts", 0),
            last_notified_version: object_string(update, "last_notified_version", ""),
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

fn raw_hold_frames(raw: &Map<String, Value>) -> f64 {
    if let Some(value) = raw.get("default_hold_frames").and_then(Value::as_f64)
        && HOLD_FRAME_OPTIONS.contains(&value)
    {
        return value;
    }
    let profile = raw
        .get("default_timing_profile")
        .and_then(Value::as_str)
        .unwrap_or("balanced")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_");
    match profile.as_str() {
        "audience_safe" | "remote_safe" | "online_audible_safe" | "online_audible" => 1.5,
        _ => DEFAULT_HOLD_FRAMES,
    }
}

fn raw_string(raw: &Map<String, Value>, key: &str, default: &str) -> String {
    raw.get(key)
        .and_then(Value::as_str)
        .unwrap_or(default)
        .to_owned()
}
fn raw_bool(raw: &Map<String, Value>, key: &str, default: bool) -> bool {
    raw.get(key).and_then(Value::as_bool).unwrap_or(default)
}
fn raw_f64(raw: &Map<String, Value>, key: &str, default: f64) -> f64 {
    raw.get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}
fn raw_u16(raw: &Map<String, Value>, key: &str, default: u16) -> u16 {
    raw.get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .unwrap_or(default)
}
fn raw_string_list(value: Option<&Value>) -> Option<Vec<String>> {
    value?.as_array().map(|items| {
        items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    })
}
fn object_string(object: &Map<String, Value>, key: &str, default: &str) -> String {
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
}
