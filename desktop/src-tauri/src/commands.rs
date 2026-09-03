use crate::app_state::AppState;
use crate::ui_events::{CalibrationMode, CalibrationState, UiEvent, UpdateChannel, UpdateState};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri::ipc::Channel;
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CatalogSearchRequest {
    pub query: String,
    pub offset: u64,
    pub limit: u16,
    pub generation: Option<u64>,
    #[serde(default)]
    pub source: LibrarySource,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum CatalogSourceId {
    #[default]
    All,
    Liked,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LibrarySource {
    Smart { id: CatalogSourceId },
    Collection { id: String },
}

impl Default for LibrarySource {
    fn default() -> Self {
        Self::Smart {
            id: CatalogSourceId::All,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CatalogDetailRequest {
    pub song_id: String,
    pub generation: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CatalogViewportRequest {
    pub generation: u64,
    pub first_index: u64,
    pub last_index: i64,
    pub selected_song_id: Option<String>,
    pub song_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CatalogSetLikedRequest {
    pub song_id: String,
    pub liked: bool,
    pub generation: Option<u64>,
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
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    pub theme: Option<String>,
    pub telemetry_enabled: Option<bool>,
    pub verbose_hud: Option<bool>,
    pub playback_defaults: Option<PlaybackPatch>,
    pub update_preferences: Option<UpdatePreferencesPatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct UpdatePreferencesPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_check: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<UpdateChannel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlaybackPrepareRequest {
    pub song_id: String,
    pub generation: u64,
    pub config: PlaybackConfigDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlaybackConfigDto {
    pub hold_frames: f64,
    pub tempo_scale: f64,
    pub fps: u16,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlaybackDecisionAcceptanceDto {
    pub decision: PlaybackDecision,
    pub accepted: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackAdmission {
    Ready,
    ConfirmationRequired,
    Blocked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq, Hash)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackDecision {
    Proceed,
    UseRecommended,
    DryRun,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackSessionState {
    Starting,
    Playing,
    Paused,
    Stopping,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackControl {
    Stop,
    Pause,
    Resume,
    Skip,
}

impl PlaybackControl {
    fn method(self) -> &'static str {
        match self {
            Self::Stop => "playback.stop",
            Self::Pause => "playback.pause",
            Self::Resume => "playback.resume",
            Self::Skip => "playback.skip",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackPendingControl {
    Pause,
    Resume,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlaybackStartRequest {
    pub prepared_id: String,
    pub decisions: Vec<PlaybackDecisionAcceptanceDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlaybackSessionCommandRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct UpdateBeginHandoffRequest {
    pub target_version: String,
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
    pub channel: UpdateChannel,
    pub skip_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct UpdateCheckDto {
    pub state: UpdateState,
    pub current_version: String,
    pub available_version: Option<String>,
    pub channel: UpdateChannel,
    pub release_notes: Option<String>,
    pub published_at: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct UpdateHandoffDto {
    pub handoff_id: String,
    pub target_version: String,
    pub state: UpdateState,
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
    pub format_label: String,
    pub duration_us: Option<u64>,
    pub note_count: Option<u64>,
    pub risk_level: String,
    pub metadata_state: String,
    pub liked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogSearchDto {
    pub items: Vec<CatalogRowDto>,
    pub offset: u64,
    pub limit: u16,
    pub total: u64,
    pub liked_total: u64,
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
#[serde(deny_unknown_fields)]
pub struct RiskDecisionDto {
    pub decision: PlaybackDecision,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlaybackPlanVariantDto {
    pub decision: PlaybackDecision,
    pub config: PlaybackConfigDto,
    pub plan_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PreparedPlaybackDto {
    pub prepared_id: Option<String>,
    pub song: SongDetailDto,
    pub config: PlaybackConfigDto,
    pub admission: PlaybackAdmission,
    pub risk: RiskSummaryDto,
    pub decisions: Vec<RiskDecisionDto>,
    pub plan_fingerprint: Option<String>,
    pub variants: Vec<PlaybackPlanVariantDto>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlaybackSessionDto {
    pub session_id: String,
    pub prepared_id: String,
    pub song_id: String,
    pub state: PlaybackSessionState,
    pub config: PlaybackConfigDto,
    pub plan_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlaybackCommandAckDto {
    pub accepted: bool,
    pub session_id: String,
    pub state: PlaybackSessionState,
    pub pending_command: Option<PlaybackPendingControl>,
    pub reason: Option<String>,
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
    pub items: Vec<CatalogRowDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogSetLikedDto {
    pub song_id: String,
    pub liked: bool,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LibraryCollectionDto {
    pub id: String,
    pub name: String,
    pub song_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LibraryCollectionsDto {
    pub collections: Vec<LibraryCollectionDto>,
    pub imported_source_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LibraryCreateCollectionRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LibraryRenameCollectionRequest {
    pub collection_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LibraryCollectionSongsRequest {
    pub collection_id: String,
    pub song_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LibraryCollectionIdRequest {
    pub collection_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LibraryRemoveImportRequest {
    pub source_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LibraryImportDto {
    pub source_ids: Vec<String>,
    pub imported_count: u64,
    pub catalog_generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsSetEnabledRequest {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsEnabledDto {
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CalibrationStartRequest {
    pub mode: CalibrationMode,
    pub class_name: Option<String>,
    pub polyphony: Option<u8>,
    pub samples: Option<u32>,
    pub timeout_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CalibrationStartAckDto {
    pub operation_id: String,
    pub state: CalibrationState,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CalibrationCancelRequest {
    pub operation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CalibrationCancelAckDto {
    pub operation_id: String,
    pub state: CalibrationState,
    pub accepted: bool,
}

async fn blocking_request<P, R>(
    state: State<'_, AppState>,
    method: &'static str,
    params: P,
) -> Result<R, String>
where
    P: Serialize + Send + 'static,
    R: DeserializeOwned + Send + 'static,
{
    let app_state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _coherence_guard = (method == "playback.start").then(|| app_state.lock_coherence());
        if crate::command_ownership::owner_for(method)
            != Some(crate::command_ownership::CommandOwner::Native)
        {
            return Err(format!("unowned non-Native desktop command: {method}"));
        }
        let runtime = app_state.ensure_native_blocking()?;
        let params = serde_json::to_value(params).map_err(|error| error.to_string())?;
        let value = runtime.dispatch(method, params)?;
        serde_json::from_value(value)
            .map_err(|error| format!("invalid native {method} response: {error}"))
    })
    .await
    .map_err(|error| format!("Native command worker failed: {error}"))?
}

async fn blocking_settings_request<P, R>(
    state: State<'_, AppState>,
    method: &'static str,
    params: P,
) -> Result<R, String>
where
    P: Serialize + Send + 'static,
    R: DeserializeOwned + Send + 'static,
{
    let app_state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        // Tauri may run multiple async commands concurrently. Keep the
        // persistence boundary serialized/exclusive even if callers bypass the
        // frontend queue. The frontend Promise tail owns user-intent order;
        // this guard only prevents overlapping persistence operations.
        let _write_guard = app_state.lock_settings_writes();
        let _coherence_guard = (method == "settings.patch").then(|| app_state.lock_coherence());
        if crate::command_ownership::owner_for(method)
            != Some(crate::command_ownership::CommandOwner::Native)
        {
            return Err(format!("unowned non-Native desktop command: {method}"));
        }
        let runtime = app_state.ensure_native_blocking()?;
        let params = serde_json::to_value(params).map_err(|error| error.to_string())?;
        let value = runtime.dispatch(method, params)?;
        serde_json::from_value(value)
            .map_err(|error| format!("invalid native {method} response: {error}"))
    })
    .await
    .map_err(|error| format!("Native settings worker failed: {error}"))?
}

#[tauri::command]
pub async fn bootstrap(state: State<'_, AppState>) -> Result<BootstrapDto, String> {
    blocking_request(state, "app.bootstrap", serde_json::json!({})).await
}

#[tauri::command]
pub async fn search_songs(
    state: State<'_, AppState>,
    params: CatalogSearchRequest,
) -> Result<CatalogSearchDto, String> {
    blocking_request(state, "catalog.search", params).await
}

#[tauri::command]
pub async fn get_song_detail(
    state: State<'_, AppState>,
    params: CatalogDetailRequest,
) -> Result<SongDetailDto, String> {
    blocking_request(state, "catalog.detail", params).await
}

#[tauri::command]
pub async fn reload_library(state: State<'_, AppState>) -> Result<CatalogReloadDto, String> {
    blocking_request(state, "catalog.reload", serde_json::json!({})).await
}

#[tauri::command]
pub async fn set_library_viewport(
    state: State<'_, AppState>,
    params: CatalogViewportRequest,
) -> Result<CatalogViewportDto, String> {
    blocking_request(state, "catalog.set_viewport", params).await
}

#[tauri::command]
pub async fn set_song_liked(
    state: State<'_, AppState>,
    params: CatalogSetLikedRequest,
) -> Result<CatalogSetLikedDto, String> {
    blocking_request(state, "catalog.set_liked", params).await
}

#[tauri::command]
pub async fn library_list_collections(
    state: State<'_, AppState>,
) -> Result<LibraryCollectionsDto, String> {
    blocking_request(state, "library.list_collections", serde_json::json!({})).await
}

#[tauri::command]
pub async fn library_create_collection(
    state: State<'_, AppState>,
    params: LibraryCreateCollectionRequest,
) -> Result<LibraryCollectionDto, String> {
    blocking_request(state, "library.create_collection", params).await
}

#[tauri::command]
pub async fn library_rename_collection(
    state: State<'_, AppState>,
    params: LibraryRenameCollectionRequest,
) -> Result<LibraryCollectionDto, String> {
    blocking_request(state, "library.rename_collection", params).await
}

#[tauri::command]
pub async fn library_delete_collection(
    state: State<'_, AppState>,
    params: LibraryCollectionIdRequest,
) -> Result<bool, String> {
    blocking_request(state, "library.delete_collection", params).await
}

#[tauri::command]
pub async fn library_add_songs(
    state: State<'_, AppState>,
    params: LibraryCollectionSongsRequest,
) -> Result<LibraryCollectionDto, String> {
    blocking_request(state, "library.add_songs", params).await
}

#[tauri::command]
pub async fn library_remove_songs(
    state: State<'_, AppState>,
    params: LibraryCollectionSongsRequest,
) -> Result<LibraryCollectionDto, String> {
    blocking_request(state, "library.remove_songs", params).await
}

#[tauri::command]
pub async fn library_import_local_files(
    app: tauri::AppHandle<super::ShellRuntime>,
    state: State<'_, AppState>,
) -> Result<LibraryImportDto, String> {
    use tauri_plugin_dialog::DialogExt;

    let paths = tauri::async_runtime::spawn_blocking(move || {
        let selected = app
            .dialog()
            .file()
            .add_filter("Sky sheets", &["json", "skysheet", "txt"])
            .blocking_pick_files();
        selected
            .unwrap_or_default()
            .into_iter()
            .map(|path| {
                path.into_path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .map_err(|error| format!("selected import path is invalid: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()
    })
    .await
    .map_err(|error| format!("Native dialog worker failed: {error}"))??;
    blocking_request(
        state,
        "library.import_local_files",
        serde_json::json!({ "paths": paths }),
    )
    .await
}

#[tauri::command]
pub async fn library_import_local_folder(
    app: tauri::AppHandle<super::ShellRuntime>,
    state: State<'_, AppState>,
) -> Result<LibraryImportDto, String> {
    use tauri_plugin_dialog::DialogExt;

    let path = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .map(|path| {
                path.into_path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .map_err(|error| format!("selected import folder is invalid: {error}"))
            })
            .transpose()
    })
    .await
    .map_err(|error| format!("Native dialog worker failed: {error}"))??;
    blocking_request(
        state,
        "library.import_local_folder",
        serde_json::json!({ "paths": path.into_iter().collect::<Vec<_>>() }),
    )
    .await
}

#[tauri::command]
pub async fn library_remove_import(
    state: State<'_, AppState>,
    params: LibraryRemoveImportRequest,
) -> Result<LibraryImportDto, String> {
    blocking_request(state, "library.remove_import", params).await
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<SettingsDto, String> {
    blocking_request(state, "settings.get", serde_json::json!({})).await
}

#[tauri::command]
pub async fn patch_settings(
    state: State<'_, AppState>,
    params: SettingsPatch,
) -> Result<SettingsDto, String> {
    blocking_settings_request(state, "settings.patch", params).await
}

#[tauri::command]
pub async fn check_for_update(state: State<'_, AppState>) -> Result<UpdateCheckDto, String> {
    blocking_request(state, "update.check", serde_json::json!({})).await
}

#[tauri::command]
pub async fn get_update_preferences(
    state: State<'_, AppState>,
) -> Result<UpdatePreferencesDto, String> {
    blocking_request(state, "update.preferences.get", serde_json::json!({})).await
}

#[tauri::command]
pub async fn patch_update_preferences(
    state: State<'_, AppState>,
    params: UpdatePreferencesPatch,
) -> Result<UpdatePreferencesDto, String> {
    blocking_settings_request(state, "update.preferences.patch", params).await
}

#[tauri::command]
pub async fn begin_update_handoff(
    state: State<'_, AppState>,
    params: UpdateBeginHandoffRequest,
) -> Result<UpdateHandoffDto, String> {
    blocking_request(state, "update.begin_handoff", params).await
}

#[tauri::command]
pub async fn set_diagnostics_enabled(
    state: State<'_, AppState>,
    params: DiagnosticsSetEnabledRequest,
) -> Result<DiagnosticsEnabledDto, String> {
    blocking_request(state, "diagnostics.set_enabled", params).await
}

#[tauri::command]
pub async fn start_calibration(
    state: State<'_, AppState>,
    params: CalibrationStartRequest,
) -> Result<CalibrationStartAckDto, String> {
    blocking_request(state, "calibration.start", params).await
}

#[tauri::command]
pub async fn cancel_calibration(
    state: State<'_, AppState>,
    params: CalibrationCancelRequest,
) -> Result<CalibrationCancelAckDto, String> {
    blocking_request(state, "calibration.cancel", params).await
}

#[tauri::command]
pub async fn prepare_playback(
    state: State<'_, AppState>,
    params: PlaybackPrepareRequest,
) -> Result<PreparedPlaybackDto, String> {
    blocking_request(state, "playback.prepare", params).await
}

#[tauri::command]
pub async fn start_playback(
    state: State<'_, AppState>,
    params: PlaybackStartRequest,
) -> Result<PlaybackSessionDto, String> {
    blocking_request(state, "playback.start", params).await
}

async fn playback_session_command(
    state: State<'_, AppState>,
    control: PlaybackControl,
    params: PlaybackSessionCommandRequest,
) -> Result<PlaybackCommandAckDto, String> {
    blocking_request(state, control.method(), params).await
}

#[tauri::command]
pub async fn stop_playback(
    state: State<'_, AppState>,
    params: PlaybackSessionCommandRequest,
) -> Result<PlaybackCommandAckDto, String> {
    playback_session_command(state, PlaybackControl::Stop, params).await
}

#[tauri::command]
pub async fn pause_playback(
    state: State<'_, AppState>,
    params: PlaybackSessionCommandRequest,
) -> Result<PlaybackCommandAckDto, String> {
    playback_session_command(state, PlaybackControl::Pause, params).await
}

#[tauri::command]
pub async fn resume_playback(
    state: State<'_, AppState>,
    params: PlaybackSessionCommandRequest,
) -> Result<PlaybackCommandAckDto, String> {
    playback_session_command(state, PlaybackControl::Resume, params).await
}

#[tauri::command]
pub async fn skip_playback(
    state: State<'_, AppState>,
    params: PlaybackSessionCommandRequest,
) -> Result<PlaybackCommandAckDto, String> {
    playback_session_command(state, PlaybackControl::Skip, params).await
}

#[tauri::command]
pub async fn subscribe_ui_events(
    state: State<'_, AppState>,
    channel: Channel<UiEvent>,
) -> Result<(), String> {
    let _command_name = crate::ipc_contract::UI_EVENTS_COMMAND;
    let app_state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let native = app_state.ensure_native_blocking()?;
        native.subscribe(channel)
    })
    .await
    .map_err(|error| format!("Native event worker failed: {error}"))?
}

#[tauri::command]
pub async fn shutdown(
    window: tauri::WebviewWindow<super::ShellRuntime>,
    state: State<'_, AppState>,
    params: Option<ShutdownRequest>,
) -> Result<(), String> {
    // This command is used only after an authoritative update handoff. Keep
    // shell exit under the same prevent-close -> bounded Native cleanup ->
    // destroy lifecycle as a user-initiated close; React never destroys the
    // native window directly.
    if params.is_some_and(|request| request.failed) {
        state.inner().set_gui_smoke_failed();
    }
    crate::lifecycle::close_window(window.as_ref().window());
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShutdownRequest {
    pub failed: bool,
}

#[cfg(test)]
mod tests {
    use super::PlaybackCommandAckDto;
    use serde_json::json;

    #[test]
    fn playback_command_ack_is_typed_and_rejects_unknown_fields() {
        let value = serde_json::from_value::<PlaybackCommandAckDto>(json!({
            "accepted": true,
            "session_id": "a".repeat(32),
            "state": "playing",
            "pending_command": "pause",
            "reason": null,
        }))
        .expect("valid playback acknowledgement");
        assert!(value.accepted);

        let unknown = serde_json::from_value::<PlaybackCommandAckDto>(json!({
            "accepted": true,
            "session_id": "a".repeat(32),
            "state": "playing",
            "pending_command": null,
            "reason": null,
            "extra": true,
        }));
        assert!(unknown.is_err());
    }
}
