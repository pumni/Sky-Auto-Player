use crate::app_state::AppState;
use crate::ui_events::UiEvent;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogSearchRequest {
    pub query: String,
    pub offset: u64,
    pub limit: u16,
    pub generation: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDetailRequest {
    pub song_id: String,
    pub generation: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogViewportRequest {
    pub generation: u64,
    pub first_index: u64,
    pub last_index: i64,
    pub selected_song_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackPatch {
    pub hold_frames: Option<f64>,
    pub tempo_scale: Option<f64>,
    pub fps: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPatch {
    pub theme: Option<String>,
    pub telemetry_enabled: Option<bool>,
    pub verbose_hud: Option<bool>,
    pub playback_defaults: Option<PlaybackPatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeBuildDto {
    pub native_build_commit: String,
    pub native_version: String,
    pub schema_version: u64,
    pub native_abi: String,
    pub rustc_version: String,
    pub win32_backend: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackDefaultsDto {
    pub hold_frames: f64,
    pub tempo_scale: f64,
    pub fps: u16,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackOptionSetsDto {
    pub hold_frames: Vec<f64>,
    pub tempo_scales: Vec<f64>,
    pub fps: Vec<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdatePreferencesDto {
    pub auto_check: bool,
    pub channel: String,
    pub skip_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogRowDto {
    pub song_id: String,
    pub title: String,
    pub duration_us: Option<u64>,
    pub note_count: Option<u64>,
    pub risk_level: String,
    pub metadata_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogSearchDto {
    pub items: Vec<CatalogRowDto>,
    pub offset: u64,
    pub limit: u16,
    pub total: u64,
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskSummaryDto {
    pub level: String,
    pub headline: String,
    pub reasons: Vec<String>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackRecommendationDto {
    pub recommended_hold_frames: Option<f64>,
    pub recommended_tempo_scale: Option<f64>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SongDetailDto {
    pub song_id: String,
    pub title: String,
    pub duration_us: u64,
    pub note_count: u64,
    pub format_label: String,
    pub risk: RiskSummaryDto,
    pub recommendation: Option<PlaybackRecommendationDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsDto {
    pub theme: String,
    pub ui_background_mode: String,
    pub playback_defaults: PlaybackDefaultsDto,
    pub telemetry_enabled: bool,
    pub verbose_hud: bool,
    pub update_preferences: UpdatePreferencesDto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogReloadDto {
    pub generation: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    request(&state, "catalog.detail", params)
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
    request(&state, "catalog.set_viewport", params)
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
    request(&state, "settings.patch", params)
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
