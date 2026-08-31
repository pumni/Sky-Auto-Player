use crate::app_state::AppState;
use crate::core::CoreSupervisor;
use crate::ui_events::{CalibrationMode, CalibrationState, UiEvent, UpdateChannel, UpdateState};
use serde::de::DeserializeOwned;
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
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
pub struct PlaybackStartRequest {
    pub prepared_id: String,
    pub decisions: Vec<PlaybackDecisionAcceptanceDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackSessionCommandRequest {
    pub session_id: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    update_preferences: Option<CoreUpdatePreferencesPatch>,
}

#[derive(Debug, Serialize)]
struct CoreUpdatePreferencesPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_check: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<UpdateChannel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skip_version: Option<String>,
}

#[derive(Debug, Serialize)]
struct CoreUpdateBeginHandoffParams {
    target_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct UpdateBeginHandoffRequest {
    pub target_version: String,
}

#[derive(Debug, Serialize)]
struct CorePlaybackPrepareParams {
    song_id: String,
    generation: u64,
    config: PlaybackConfigDto,
}

#[derive(Debug, Serialize)]
struct CorePlaybackStartParams {
    prepared_id: String,
    decisions: Vec<PlaybackDecisionAcceptanceDto>,
}

#[derive(Debug, Serialize)]
struct CorePlaybackSessionParams {
    session_id: String,
}

#[derive(Debug, Serialize)]
struct CoreDiagnosticsSetEnabledParams {
    enabled: bool,
}

#[derive(Debug, Serialize)]
struct CoreCalibrationStartParams {
    mode: CalibrationMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    class_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    polyphony: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    samples: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_seconds: Option<f64>,
}

#[derive(Debug, Serialize)]
struct CoreCalibrationCancelParams {
    operation_id: String,
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

fn request_with_supervisor<P, R>(
    supervisor: &CoreSupervisor,
    method: &'static str,
    params: P,
) -> Result<R, String>
where
    P: Serialize,
    R: DeserializeOwned,
{
    if crate::command_ownership::owner_for(method).is_none() {
        return Err(format!("unowned desktop command: {method}"));
    }
    let value = supervisor
        .request(method, params)
        .map_err(|error| error.to_string())?;
    serde_json::from_value(value).map_err(|error| format!("invalid {method} response: {error}"))
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
        let supervisor = app_state.ensure_core_blocking()?;
        request_with_supervisor(&supervisor, method, params)
    })
    .await
    .map_err(|error| format!("Core worker failed: {error}"))?
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
        let supervisor = app_state.ensure_core_blocking()?;
        request_with_supervisor(&supervisor, method, params)
    })
    .await
    .map_err(|error| format!("Core settings worker failed: {error}"))?
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
    blocking_request(
        state,
        "catalog.detail",
        CoreDetailParams {
            song_id: params.song_id,
            generation: params.generation,
        },
    )
    .await
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
    blocking_request(
        state,
        "catalog.set_viewport",
        CoreViewportParams {
            generation: params.generation,
            first_index: params.first_index,
            last_index: params.last_index,
            selected_song_id: params.selected_song_id,
        },
    )
    .await
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
    let playback_defaults = params.playback_defaults.map(|playback| CorePlaybackPatch {
        hold_frames: playback.hold_frames,
        tempo_scale: playback.tempo_scale,
        fps: playback.fps,
    });
    blocking_settings_request(
        state,
        "settings.patch",
        CoreSettingsPatch {
            theme: params.theme,
            telemetry_enabled: params.telemetry_enabled,
            verbose_hud: params.verbose_hud,
            playback_defaults,
            update_preferences: params
                .update_preferences
                .map(|value| CoreUpdatePreferencesPatch {
                    auto_check: value.auto_check,
                    channel: value.channel,
                    skip_version: value.skip_version,
                }),
        },
    )
    .await
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
    blocking_settings_request(
        state,
        "update.preferences.patch",
        CoreUpdatePreferencesPatch {
            auto_check: params.auto_check,
            channel: params.channel,
            skip_version: params.skip_version,
        },
    )
    .await
}

#[tauri::command]
pub async fn begin_update_handoff(
    state: State<'_, AppState>,
    params: UpdateBeginHandoffRequest,
) -> Result<UpdateHandoffDto, String> {
    blocking_request(
        state,
        "update.begin_handoff",
        CoreUpdateBeginHandoffParams {
            target_version: params.target_version,
        },
    )
    .await
}

#[tauri::command]
pub async fn set_diagnostics_enabled(
    state: State<'_, AppState>,
    params: DiagnosticsSetEnabledRequest,
) -> Result<DiagnosticsEnabledDto, String> {
    blocking_request(
        state,
        "diagnostics.set_enabled",
        CoreDiagnosticsSetEnabledParams {
            enabled: params.enabled,
        },
    )
    .await
}

#[tauri::command]
pub async fn start_calibration(
    state: State<'_, AppState>,
    params: CalibrationStartRequest,
) -> Result<CalibrationStartAckDto, String> {
    blocking_request(
        state,
        "calibration.start",
        CoreCalibrationStartParams {
            mode: params.mode,
            class_name: params.class_name,
            polyphony: params.polyphony,
            samples: params.samples,
            timeout_seconds: params.timeout_seconds,
        },
    )
    .await
}

#[tauri::command]
pub async fn cancel_calibration(
    state: State<'_, AppState>,
    params: CalibrationCancelRequest,
) -> Result<CalibrationCancelAckDto, String> {
    blocking_request(
        state,
        "calibration.cancel",
        CoreCalibrationCancelParams {
            operation_id: params.operation_id,
        },
    )
    .await
}

#[tauri::command]
pub async fn prepare_playback(
    state: State<'_, AppState>,
    params: PlaybackPrepareRequest,
) -> Result<PreparedPlaybackDto, String> {
    blocking_request(
        state,
        "playback.prepare",
        CorePlaybackPrepareParams {
            song_id: params.song_id,
            generation: params.generation,
            config: params.config,
        },
    )
    .await
}

#[tauri::command]
pub async fn start_playback(
    state: State<'_, AppState>,
    params: PlaybackStartRequest,
) -> Result<PlaybackSessionDto, String> {
    blocking_request(
        state,
        "playback.start",
        CorePlaybackStartParams {
            prepared_id: params.prepared_id,
            decisions: params.decisions,
        },
    )
    .await
}

async fn playback_session_command(
    state: State<'_, AppState>,
    control: PlaybackControl,
    params: PlaybackSessionCommandRequest,
) -> Result<PlaybackCommandAckDto, String> {
    blocking_request(
        state,
        control.method(),
        CorePlaybackSessionParams {
            session_id: params.session_id,
        },
    )
    .await
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
    let app_state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let supervisor = app_state.ensure_core_blocking()?;
        supervisor
            .subscribe(channel)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("Core event worker failed: {error}"))?
}

#[tauri::command]
pub async fn shutdown(
    window: tauri::WebviewWindow<super::ShellRuntime>,
    state: State<'_, AppState>,
    params: Option<ShutdownRequest>,
) -> Result<(), String> {
    // This command is used only after an authoritative update handoff. Keep
    // shell exit under the same prevent-close -> bounded Core cleanup ->
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
