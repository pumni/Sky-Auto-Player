//! Normalized application settings and atomic patch semantics.
//!
//! The types here describe the values consumed by application use cases. The
//! persisted `config.json` layout, legacy migration, and atomic file swap stay
//! in an outer adapter so raw storage never becomes the domain model.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SCHEMA_VERSION: u32 = 3;
pub const DEFAULT_GAME_FPS: u16 = 60;
pub const VALID_FPS: [u16; 7] = [30, 60, 90, 120, 144, 165, 240];
pub const DEFAULT_HOLD_FRAMES: f64 = 1.0;
pub const HOLD_FRAME_OPTIONS: [f64; 3] = [1.0, 1.25, 1.5];
pub const TEMPO_SCALE_OPTIONS: [f64; 5] = [0.90, 0.95, 1.00, 1.05, 1.10];
pub const DEFAULT_SONGS_DIR: &str = "songs";
pub const DEFAULT_UPDATE_INTERVAL_S: i64 = 86_400;
pub const MAX_SKIP_VERSION_BYTES: usize = 128;

pub const THEME_IDS: [&str; 5] = ["aurora", "minimalist", "slate", "cyberpunk", "classic"];
pub const BACKGROUND_MODES: [&str; 2] = ["transparent", "painted"];
pub const DEFAULT_PROCESS_NAMES: [&str; 2] = ["Sky.exe", "Sky Children of the Light.exe"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeySettings {
    pub pause: String,
    pub skip: String,
    pub quit: String,
    pub refocus: String,
    pub panic: String,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            pause: "f8".into(),
            skip: "f9".into(),
            quit: "f10".into(),
            refocus: "f6".into(),
            panic: "ctrl+alt+backspace".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafetySettings {
    pub prompt_on_medium_risk: bool,
    pub prompt_on_high_risk: bool,
}

impl Default for SafetySettings {
    fn default() -> Self {
        Self {
            prompt_on_medium_risk: true,
            prompt_on_high_risk: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    #[default]
    Stable,
    Beta,
}

impl UpdateChannel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Beta => "beta",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stable" => Some(Self::Stable),
            "beta" => Some(Self::Beta),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatePreferences {
    pub auto_check: bool,
    pub channel: UpdateChannel,
    pub skip_version: String,
    pub check_interval_s: i64,
    pub last_check_ts: i64,
    pub last_error_ts: i64,
    pub last_notified_version: String,
    pub legacy_old_dir_sweep_pending: bool,
}

impl Default for UpdatePreferences {
    fn default() -> Self {
        Self {
            auto_check: true,
            channel: UpdateChannel::Stable,
            skip_version: String::new(),
            check_interval_s: DEFAULT_UPDATE_INTERVAL_S,
            last_check_ts: 0,
            last_error_ts: 0,
            last_notified_version: String::new(),
            legacy_old_dir_sweep_pending: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaybackDefaults {
    pub hold_frames: f64,
    pub tempo_scale: f64,
    pub fps: u16,
}

impl Default for PlaybackDefaults {
    fn default() -> Self {
        Self {
            hold_frames: DEFAULT_HOLD_FRAMES,
            tempo_scale: 1.0,
            fps: DEFAULT_GAME_FPS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplicationSettings {
    pub theme: String,
    pub ui_background_mode: String,
    pub playback_defaults: PlaybackDefaults,
    pub telemetry_enabled: bool,
    pub verbose_hud: bool,
    pub songs_dir: String,
    pub sky_process_names: Vec<String>,
    pub allow_title_fallback: bool,
    pub hotkeys: HotkeySettings,
    pub safety: SafetySettings,
    pub update: UpdatePreferences,
}

impl Default for ApplicationSettings {
    fn default() -> Self {
        Self {
            theme: "aurora".into(),
            ui_background_mode: "transparent".into(),
            playback_defaults: PlaybackDefaults::default(),
            telemetry_enabled: false,
            verbose_hud: false,
            songs_dir: DEFAULT_SONGS_DIR.into(),
            sky_process_names: DEFAULT_PROCESS_NAMES.iter().map(|v| (*v).into()).collect(),
            allow_title_fallback: false,
            hotkeys: HotkeySettings::default(),
            safety: SafetySettings::default(),
            update: UpdatePreferences::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaybackDefaultsPatch {
    pub hold_frames: Option<f64>,
    pub tempo_scale: Option<f64>,
    pub fps: Option<u16>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePreferencesPatch {
    pub auto_check: Option<bool>,
    pub channel: Option<UpdateChannel>,
    pub skip_version: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    pub theme: Option<String>,
    pub telemetry_enabled: Option<bool>,
    pub verbose_hud: Option<bool>,
    pub playback_defaults: Option<PlaybackDefaultsPatch>,
    pub update: Option<UpdatePreferencesPatch>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SettingsError {
    #[error("invalid settings field {field}: {message}")]
    InvalidField { field: String, message: String },
    #[error("settings storage error: {0}")]
    Storage(String),
}

pub trait SettingsStore {
    fn load(&self) -> Result<ApplicationSettings, SettingsError>;
    fn save(&self, settings: &ApplicationSettings) -> Result<(), SettingsError>;
}

pub struct SettingsService<S> {
    store: S,
    current: ApplicationSettings,
}

impl<S: SettingsStore> SettingsService<S> {
    pub fn load(store: S) -> Result<Self, SettingsError> {
        let current = store.load()?;
        Ok(Self { store, current })
    }

    pub fn snapshot(&self) -> &ApplicationSettings {
        &self.current
    }

    pub fn reload(&mut self) -> Result<&ApplicationSettings, SettingsError> {
        self.current = self.store.load()?;
        Ok(&self.current)
    }

    /// Validate the complete patch before mutating or persisting anything.
    pub fn patch(&mut self, patch: &SettingsPatch) -> Result<&ApplicationSettings, SettingsError> {
        let next = apply_patch(&self.current, patch)?;
        if next != self.current {
            self.store.save(&next)?;
            self.current = next;
        }
        Ok(&self.current)
    }

    /// Record an update check outcome in the same durable settings document
    /// used by the rest of the desktop.  Keeping these mutations here makes
    /// the update service unable to accidentally introduce a second
    /// preference store.
    pub fn record_update_success(
        &mut self,
        timestamp: i64,
    ) -> Result<&ApplicationSettings, SettingsError> {
        let mut next = self.current.clone();
        next.update.last_check_ts = timestamp;
        next.update.last_error_ts = 0;
        self.store.save(&next)?;
        self.current = next;
        Ok(&self.current)
    }

    pub fn record_update_error(
        &mut self,
        timestamp: i64,
    ) -> Result<&ApplicationSettings, SettingsError> {
        let mut next = self.current.clone();
        next.update.last_error_ts = timestamp;
        self.store.save(&next)?;
        self.current = next;
        Ok(&self.current)
    }
}

pub fn normalize_settings(mut settings: ApplicationSettings) -> ApplicationSettings {
    settings.theme = normalized_theme(&settings.theme);
    settings.ui_background_mode = normalized_background(&settings.ui_background_mode);
    settings.playback_defaults.hold_frames =
        normalize_hold_frames(settings.playback_defaults.hold_frames);
    settings.playback_defaults.tempo_scale =
        normalize_tempo(settings.playback_defaults.tempo_scale);
    settings.playback_defaults.fps = normalize_fps(settings.playback_defaults.fps);
    settings.sky_process_names = normalize_process_names(settings.sky_process_names);
    settings.update.check_interval_s = settings.update.check_interval_s.max(0);
    settings
}

pub fn apply_patch(
    current: &ApplicationSettings,
    patch: &SettingsPatch,
) -> Result<ApplicationSettings, SettingsError> {
    let mut next = current.clone();

    if let Some(theme) = &patch.theme {
        next.theme = validate_theme(theme)?;
    }
    if let Some(value) = patch.telemetry_enabled {
        next.telemetry_enabled = value;
    }
    if let Some(value) = patch.verbose_hud {
        next.verbose_hud = value;
    }
    if let Some(playback) = &patch.playback_defaults {
        if let Some(value) = playback.hold_frames {
            next.playback_defaults.hold_frames = validate_hold_frames(value)?;
        }
        if let Some(value) = playback.tempo_scale {
            next.playback_defaults.tempo_scale = validate_tempo(value)?;
        }
        if let Some(value) = playback.fps {
            next.playback_defaults.fps = validate_fps(value)?;
        }
    }
    if let Some(update) = &patch.update {
        if let Some(value) = update.auto_check {
            next.update.auto_check = value;
        }
        if let Some(value) = &update.channel {
            next.update.channel = value.clone();
        }
        if let Some(value) = &update.skip_version {
            next.update.skip_version = validate_skip_version(value)?;
        }
    }

    Ok(next)
}

fn normalized_theme(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    if THEME_IDS.contains(&value.as_str()) {
        value
    } else {
        "aurora".into()
    }
}

fn normalized_background(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    if BACKGROUND_MODES.contains(&value.as_str()) {
        value
    } else {
        "transparent".into()
    }
}

fn normalize_hold_frames(value: f64) -> f64 {
    if value.is_finite() && HOLD_FRAME_OPTIONS.contains(&value) {
        value
    } else {
        DEFAULT_HOLD_FRAMES
    }
}

fn normalize_tempo(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
}

fn normalize_fps(value: u16) -> u16 {
    if VALID_FPS.contains(&value) {
        value
    } else {
        DEFAULT_GAME_FPS
    }
}

fn normalize_process_names(values: Vec<String>) -> Vec<String> {
    let mut names = values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if names.is_empty() {
        names = DEFAULT_PROCESS_NAMES
            .iter()
            .map(|value| (*value).into())
            .collect();
    }
    names
}

fn validate_theme(value: &str) -> Result<String, SettingsError> {
    let normalized = value.trim().to_ascii_lowercase();
    if THEME_IDS.contains(&normalized.as_str()) {
        Ok(normalized)
    } else {
        Err(SettingsError::InvalidField {
            field: "theme".into(),
            message: "must be a known theme ID".into(),
        })
    }
}

fn validate_hold_frames(value: f64) -> Result<f64, SettingsError> {
    if value.is_finite() && HOLD_FRAME_OPTIONS.contains(&value) {
        Ok(value)
    } else {
        Err(SettingsError::InvalidField {
            field: "hold_frames".into(),
            message: "must be one of 1.0, 1.25, or 1.5".into(),
        })
    }
}

fn validate_tempo(value: f64) -> Result<f64, SettingsError> {
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(SettingsError::InvalidField {
            field: "tempo_scale".into(),
            message: "must be a finite positive number".into(),
        })
    }
}

fn validate_fps(value: u16) -> Result<u16, SettingsError> {
    if VALID_FPS.contains(&value) {
        Ok(value)
    } else {
        Err(SettingsError::InvalidField {
            field: "fps".into(),
            message: "must be a supported game FPS".into(),
        })
    }
}

fn validate_skip_version(value: &str) -> Result<String, SettingsError> {
    if value.len() > MAX_SKIP_VERSION_BYTES || value.contains('\0') {
        return Err(SettingsError::InvalidField {
            field: "skip_version".into(),
            message: "must be bounded text".into(),
        });
    }
    Ok(value.trim().to_owned())
}

/// Stable list of patch keys used by delivery-layer contract tests.
pub fn patchable_field_names() -> BTreeSet<&'static str> {
    [
        "theme",
        "telemetry_enabled",
        "verbose_hud",
        "playback_defaults",
        "update",
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MemoryStore(ApplicationSettings);

    impl SettingsStore for MemoryStore {
        fn load(&self) -> Result<ApplicationSettings, SettingsError> {
            Ok(self.0.clone())
        }
        fn save(&self, _settings: &ApplicationSettings) -> Result<(), SettingsError> {
            Ok(())
        }
    }

    #[test]
    fn patch_validates_before_mutating() {
        let mut service = SettingsService::load(MemoryStore::default()).expect("load");
        let before = service.snapshot().clone();
        let error = service
            .patch(&SettingsPatch {
                theme: Some("slate".into()),
                playback_defaults: Some(PlaybackDefaultsPatch {
                    fps: Some(61),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .expect_err("invalid patch");
        assert!(matches!(error, SettingsError::InvalidField { field, .. } if field == "fps"));
        assert_eq!(*service.snapshot(), before);
    }

    #[test]
    fn defaults_match_current_python_normalized_defaults() {
        let settings = normalize_settings(ApplicationSettings::default());
        assert_eq!(settings.theme, "aurora");
        assert_eq!(settings.playback_defaults.fps, 60);
        assert_eq!(settings.update.channel, UpdateChannel::Stable);
    }
}
