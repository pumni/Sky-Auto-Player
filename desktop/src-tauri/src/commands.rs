use crate::app_state::AppState;
use crate::ui_events::UiEvent;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSearchRequest {
    pub query: String,
    pub offset: u64,
    pub limit: u16,
    pub generation: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDetailRequest {
    pub song_id: String,
    pub generation: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CatalogViewportRequest {
    pub generation: u64,
    pub first_index: u64,
    pub last_index: i64,
    pub selected_song_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackPatch {
    pub hold_frames: Option<f64>,
    pub tempo_scale: Option<f64>,
    pub fps: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub theme: Option<String>,
    pub telemetry_enabled: Option<bool>,
    pub verbose_hud: Option<bool>,
    pub playback_defaults: Option<PlaybackPatch>,
}

#[derive(Debug, Serialize)]
struct CoreDetailParams {
    song_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation: Option<u64>,
}

#[derive(Debug, Serialize)]
struct CoreViewportParams {
    generation: u64,
    first_index: u64,
    last_index: i64,
    selected_song_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct CorePlaybackPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    hold_frames: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tempo_scale: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fps: Option<u16>,
}

#[derive(Debug, Serialize)]
struct CoreSettingsPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    telemetry_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verbose_hud: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    playback_defaults: Option<CorePlaybackPatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NativeBuildDto {
    pub native_build_commit: String,
    pub native_version: String,
    pub schema_version: u64,
    pub native_abi: String,
    pub rustc_version: String,
    pub win32_backend: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlaybackDefaultsDto {
    pub hold_frames: f64,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct UpdatePreferencesDto {
    pub auto_check: bool,
    pub channel: String,
    pub skip_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BootstrapDto {
    pub app_version: String,
    pub protocol_version: u64,
    pub native_build: NativeBuildDto,
    pub playback_defaults: PlaybackDefaultsDto,
    pub option_sets: PlaybackOptionSetsDto,
    pub theme: String,
    pub telemetry_enabled: bool,
    pub update_preferences: UpdatePreferencesDto,
    pub catalog_generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogRowDto {
    pub song_id: String,
    pub title: String,
    pub duration_us: Option<u64>,
    pub note_count: Option<u64>,
    pub risk_level: String,
    pub metadata_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogSearchDto {
    pub items: Vec<CatalogRowDto>,
    pub offset: u64,
    pub limit: u16,
    pub total: u64,
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RiskSummaryDto {
    pub level: String,
    pub headline: String,
    pub reasons: Vec<String>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlaybackRecommendationDto {
    pub recommended_hold_frames: Option<f64>,
    pub recommended_tempo_scale: Option<f64>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SongDetailDto {
    pub song_id: String,
    pub title: String,
    pub duration_us: u64,
    pub note_count: u64,
    pub format_label: String,
    pub risk: RiskSummaryDto,
    pub recommendation: Option<PlaybackRecommendationDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SettingsDto {
    pub theme: String,
    pub ui_background_mode: String,
    pub playback_defaults: PlaybackDefaultsDto,
    pub telemetry_enabled: bool,
    pub verbose_hud: bool,
    pub update_preferences: UpdatePreferencesDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogReloadDto {
    pub generation: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogViewportDto {
    pub accepted: bool,
    pub generation: u64,
    pub first_index: u64,
    pub last_index: i64,
    pub selected_song_id: Option<String>,
}

fn request<P, R>(state: &State<'_, AppState>, method: &str, params: P) -> Result<R, String>
where
    P: Serialize,
    R: for<'de> Deserialize<'de>,
{
    let value = state
        .supervisor()
        .map_err(|error| error.to_string())?
        .request(method, params)
        .map_err(|error| error.to_string())?;
    serde_json::from_value(value).map_err(|error| format!("invalid {method} response: {error}"))
}

#[tauri::command]
pub fn bootstrap(state: State<'_, AppState>) -> Result<BootstrapDto, String> {
    request(&state, "app.bootstrap", serde_json::json!({}))
}

#[tauri::command]
pub fn search_songs(
    state: State<'_, AppState>,
    params: CatalogSearchRequest,
) -> Result<CatalogSearchDto, String> {
    request(&state, "catalog.search", params)
}

#[tauri::command]
pub fn get_song_detail(
    state: State<'_, AppState>,
    params: CatalogDetailRequest,
) -> Result<SongDetailDto, String> {
    request(
        &state,
        "catalog.detail",
        CoreDetailParams {
            song_id: params.song_id,
            generation: params.generation,
        },
    )
}

#[tauri::command]
pub fn reload_library(state: State<'_, AppState>) -> Result<CatalogReloadDto, String> {
    request(&state, "catalog.reload", serde_json::json!({}))
}

#[tauri::command]
pub fn set_library_viewport(
    state: State<'_, AppState>,
    params: CatalogViewportRequest,
) -> Result<CatalogViewportDto, String> {
    request(
        &state,
        "catalog.set_viewport",
        CoreViewportParams {
            generation: params.generation,
            first_index: params.first_index,
            last_index: params.last_index,
            selected_song_id: params.selected_song_id,
        },
    )
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<SettingsDto, String> {
    request(&state, "settings.get", serde_json::json!({}))
}

#[tauri::command]
pub fn patch_settings(
    state: State<'_, AppState>,
    params: SettingsPatch,
) -> Result<SettingsDto, String> {
    let playback_defaults = params.playback_defaults.map(|playback| CorePlaybackPatch {
        hold_frames: playback.hold_frames,
        tempo_scale: playback.tempo_scale,
        fps: playback.fps,
    });
    request(
        &state,
        "settings.patch",
        CoreSettingsPatch {
            theme: params.theme,
            telemetry_enabled: params.telemetry_enabled,
            verbose_hud: params.verbose_hud,
            playback_defaults,
        },
    )
}

#[tauri::command]
pub fn subscribe_ui_events(
    state: State<'_, AppState>,
    channel: Channel<UiEvent>,
) -> Result<(), String> {
    state
        .supervisor()
        .map_err(|error| error.to_string())?
        .subscribe(channel)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn shutdown(state: State<'_, AppState>) -> Result<(), String> {
    state
        .supervisor()
        .map_err(|error| error.to_string())?
        .shutdown();
    Ok(())
}
