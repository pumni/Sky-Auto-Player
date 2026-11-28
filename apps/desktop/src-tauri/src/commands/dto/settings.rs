use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::update::{UpdatePreferencesDto, UpdatePreferencesPatch};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceMode {
    System,
    Light,
    Dark,
}

impl AppearanceMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackPatch {
    pub hold_frames: Option<f64>,
    pub timing_margin_us: Option<u64>,
    pub tempo_scale: Option<f64>,
    pub fps: Option<u16>,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub appearance_mode: Option<AppearanceMode>,
    pub palette: Option<String>,
    pub telemetry_enabled: Option<bool>,
    pub verbose_hud: Option<bool>,
    pub playback_defaults: Option<PlaybackPatch>,
    pub auto_play: Option<bool>,
    pub update_preferences: Option<UpdatePreferencesPatch>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct SettingsPatchWire {
    appearance_mode: Option<AppearanceMode>,
    #[serde(alias = "theme")]
    palette: Option<String>,
    telemetry_enabled: Option<bool>,
    verbose_hud: Option<bool>,
    playback_defaults: Option<PlaybackPatch>,
    auto_play: Option<bool>,
    update_preferences: Option<UpdatePreferencesPatch>,
}

impl From<SettingsPatchWire> for SettingsPatch {
    fn from(value: SettingsPatchWire) -> Self {
        Self {
            appearance_mode: value.appearance_mode,
            palette: value.palette,
            telemetry_enabled: value.telemetry_enabled,
            verbose_hud: value.verbose_hud,
            playback_defaults: value.playback_defaults,
            auto_play: value.auto_play,
            update_preferences: value.update_preferences,
        }
    }
}

impl<'de> Deserialize<'de> for SettingsPatch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        SettingsPatchWire::deserialize(deserializer).map(Into::into)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlaybackDefaultsDto {
    pub hold_frames: f64,
    pub timing_margin_us: u64,
    pub tempo_scale: f64,
    pub fps: u16,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlaybackOptionSetsDto {
    pub hold_frames: Vec<f64>,
    pub tempo_scales: Vec<f64>,
    pub fps: Vec<u16>,
    pub timing_margin_min_us: u64,
    pub timing_margin_max_us: u64,
    pub timing_margin_step_us: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SettingsDto {
    pub appearance_mode: AppearanceMode,
    pub palette: String,
    pub ui_background_mode: String,
    pub playback_defaults: PlaybackDefaultsDto,
    pub auto_play: bool,
    pub telemetry_enabled: bool,
    pub verbose_hud: bool,
    pub update_preferences: UpdatePreferencesDto,
}
