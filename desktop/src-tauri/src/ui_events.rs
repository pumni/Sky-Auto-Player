use serde::{Deserialize, Serialize};
use ts_rs::TS;

const MAX_EVENT_TEXT_BYTES: usize = 4096;

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CoreReadyPayload {
    pub app_version: String,
    pub protocol_version: u64,
    pub native_build: NativeBuildPayload,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct NativeBuildPayload {
    pub native_build_commit: String,
    pub native_version: String,
    pub schema_version: u64,
    pub native_abi: String,
    pub rustc_version: String,
    pub win32_backend: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CoreFatalPayload {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CatalogChangedPayload {
    pub generation: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum CatalogReadiness {
    Uninitialized,
    Loading,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CatalogLoadFailedPayload {
    pub message: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    Stable,
    Beta,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum UpdateState {
    Idle,
    Checking,
    Current,
    Available,
    Downloading,
    Ready,
    Installing,
    Error,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum UpdateCheckOrigin {
    Manual,
    Background,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct UpdateCheckRequest {
    pub origin: UpdateCheckOrigin,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum UpdateCheckDisposition {
    Performed,
    Disabled,
    Throttled,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct UpdateCheckAckDto {
    pub disposition: UpdateCheckDisposition,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum UpdateRetryAction {
    None,
    Check,
    Install,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum UpdateErrorCode {
    CheckFailed,
    UpdateServiceUnavailable,
    ChannelUnavailable,
    PlaybackActive,
    CalibrationActive,
    UpdateBusy,
    Closing,
    StaleUpdate,
    UpdateUnavailable,
    DownloadFailed,
    InstallFailed,
    StatePersistenceFailed,
    Unknown,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct UpdateProgressDto {
    pub completed: u64,
    pub total: Option<u64>,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct UpdateSnapshotPayload {
    pub revision: u64,
    pub state: UpdateState,
    pub current_version: String,
    pub available_version: Option<String>,
    pub channel: UpdateChannel,
    pub release_notes: Option<String>,
    pub published_at: Option<String>,
    pub error_code: Option<UpdateErrorCode>,
    pub error_detail: Option<String>,
    pub retry_action: UpdateRetryAction,
    pub operation_id: Option<String>,
    pub progress: Option<UpdateProgressDto>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticsBackendStatus {
    Healthy,
    Degraded,
    Error,
    Unavailable,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsSnapshotDto {
    pub seq: u64,
    pub physical_session: bool,
    pub player_attached: bool,
    pub sender_sample_count: u64,
    pub max_lateness_us: Option<u64>,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub sigma_onset_ms: Option<f64>,
    pub late_2ms: Option<u64>,
    pub late_5ms: Option<u64>,
    pub late_10ms: Option<u64>,
    pub max_sendinput_pre_call_lateness_us: Option<u64>,
    pub pre_call_late_2ms: u64,
    pub pre_call_late_5ms: u64,
    pub pre_call_late_10ms: u64,
    pub fps: u16,
    pub frame_us: u64,
    pub hold_frames: f64,
    pub frame_base_hold_us: u64,
    pub timing_margin_us: u64,
    pub min_hold_us: u64,
    pub min_release_gap_us: u64,
    pub timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto,
    pub pre_call_lt_250us: u64,
    pub pre_call_250_500us: u64,
    pub pre_call_500_750us: u64,
    pub pre_call_750_1000us: u64,
    pub pre_call_1000_1500us: u64,
    pub pre_call_1500_2000us: u64,
    pub pre_call_ge_2000us: u64,
    pub active_keys: u64,
    pub stuck_keys: u64,
    pub keys_dropped: u64,
    pub chord_split_events: u64,
    pub missed_down_boundaries: u64,
    pub missed_down_keys: u64,
    pub missed_unobserved_backlog_boundaries: u64,
    pub missed_physical_window_boundaries: u64,
    pub final_sender_window_expirations: u64,
    pub release_floor_infeasible_boundaries: u64,
    pub hold_floor_delay_boundaries: u64,
    pub max_hold_floor_delay_us: u64,
    pub release_floor_delay_boundaries: u64,
    pub max_release_floor_delay_us: u64,
    pub final_gate_control_rejections: u64,
    pub final_gate_target_changes: u64,
    pub final_gate_focus_losses: u64,
    pub final_gate_lease_expirations: u64,
    pub sendinput_partial_events: u64,
    pub sendinput_zero_progress_failures: u64,
    pub backend_status: DiagnosticsBackendStatus,
    pub release_max_us: Option<u64>,
    pub release_late_2ms: Option<u64>,
    pub session_id: Option<String>,
    pub last_error: Option<String>,
    pub power_request_created: bool,
    pub power_request_active: bool,
    pub power_request_create_failures: u64,
    pub power_request_set_failures: u64,
    pub power_request_clear_failures: u64,
    pub power_request_close_failures: u64,
    pub suspend_resume_registered: bool,
    pub suspend_resume_registration_failures: u64,
    pub suspend_resume_unregistration_failures: u64,
    pub system_suspend_active: bool,
    pub system_suspend_notifications: u64,
    pub system_resume_notifications: u64,
    pub duplicate_system_power_notifications: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationMode {
    Quick,
    Full,
    Diagnostic,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationState {
    Idle,
    Starting,
    Running,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CalibrationProgressPayload {
    pub operation_id: String,
    pub state: CalibrationState,
    pub phase: String,
    pub completed: u64,
    pub total: u64,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct CalibrationFinishedPayload {
    pub operation_id: String,
    pub outcome: CalibrationOutcome,
    pub status: String,
    pub recommended_timing_margin_us: Option<u64>,
    pub recommendation_qualified: bool,
    pub sample_count: u64,
    pub source: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackEventState {
    Starting,
    Playing,
    Paused,
    Stopping,
    Finished,
    Failed,
}

#[cfg(test)]
impl PlaybackEventState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Playing => "playing",
            Self::Paused => "paused",
            Self::Stopping => "stopping",
            Self::Finished => "finished",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackFocusState {
    Focused,
    Unfocused,
    Waiting,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackHealthState {
    Healthy,
    Degraded,
    Error,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlaybackStateChangedPayload {
    pub session_id: String,
    pub song_id: String,
    pub state: PlaybackEventState,
    pub physical: bool,
    pub message: Option<String>,
    pub outcome: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlaybackSnapshotPayload {
    pub session_id: String,
    pub seq: u64,
    pub state: PlaybackEventState,
    pub song_id: String,
    pub title: String,
    pub current_us: u64,
    pub total_us: u64,
    pub pre_roll_remaining_us: u64,
    pub focus_state: PlaybackFocusState,
    pub health: PlaybackHealthState,
    pub input_path_degraded: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlaybackFinishedPayload {
    pub session_id: String,
    pub song_id: String,
    pub outcome: String,
    pub total_us: u64,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlaybackFailedPayload {
    pub session_id: String,
    pub song_id: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, TS, PartialEq)]
#[ts(export)]
#[serde(tag = "name")]
pub enum UiEvent {
    #[serde(rename = "core.ready")]
    CoreReady { v: u64, payload: CoreReadyPayload },
    #[allow(dead_code)]
    #[serde(rename = "core.fatal")]
    CoreFatal { v: u64, payload: CoreFatalPayload },
    #[serde(rename = "catalog.changed")]
    CatalogChanged {
        v: u64,
        payload: CatalogChangedPayload,
    },
    #[serde(rename = "catalog.load_failed")]
    CatalogLoadFailed {
        v: u64,
        payload: CatalogLoadFailedPayload,
    },
    #[serde(rename = "playback.state_changed")]
    PlaybackStateChanged {
        v: u64,
        payload: PlaybackStateChangedPayload,
    },
    #[serde(rename = "playback.snapshot")]
    PlaybackSnapshot {
        v: u64,
        payload: PlaybackSnapshotPayload,
    },
    #[serde(rename = "playback.finished")]
    PlaybackFinished {
        v: u64,
        payload: PlaybackFinishedPayload,
    },
    #[serde(rename = "playback.failed")]
    PlaybackFailed {
        v: u64,
        payload: PlaybackFailedPayload,
    },
    #[serde(rename = "diagnostics.snapshot")]
    DiagnosticsSnapshot {
        v: u64,
        payload: Box<DiagnosticsSnapshotDto>,
    },
    #[serde(rename = "calibration.progress")]
    CalibrationProgress {
        v: u64,
        payload: CalibrationProgressPayload,
    },
    #[serde(rename = "calibration.finished")]
    CalibrationFinished {
        v: u64,
        payload: CalibrationFinishedPayload,
    },
    #[serde(rename = "update.changed")]
    UpdateChanged {
        v: u64,
        payload: UpdateSnapshotPayload,
    },
}

impl UiEvent {
    pub(crate) fn validate_ready(payload: &CoreReadyPayload) -> Result<(), String> {
        validate_text("app_version", &payload.app_version)?;
        validate_text(
            "native_build_commit",
            &payload.native_build.native_build_commit,
        )?;
        validate_text("native_version", &payload.native_build.native_version)?;
        validate_text("native_abi", &payload.native_build.native_abi)?;
        validate_text("rustc_version", &payload.native_build.rustc_version)
    }

    pub(crate) fn validate_fatal(payload: &CoreFatalPayload) -> Result<(), String> {
        validate_text("code", &payload.code)?;
        validate_text("message", &payload.message)
    }

    pub(crate) fn validate_catalog_changed(payload: &CatalogChangedPayload) -> Result<(), String> {
        if payload.generation == 0 {
            return Err("event generation must be positive".into());
        }
        if payload.total > 10_000_000 {
            return Err("event catalog total exceeds the bounded contract".into());
        }
        Ok(())
    }

    pub(crate) fn validate_catalog_load_failed(
        payload: &CatalogLoadFailedPayload,
    ) -> Result<(), String> {
        validate_text("message", &payload.message)
    }

    pub(crate) fn validate_playback_state_changed(
        payload: &PlaybackStateChangedPayload,
    ) -> Result<(), String> {
        validate_session_id(&payload.session_id)?;
        validate_song_id(&payload.song_id)?;
        if let Some(message) = &payload.message {
            validate_text("message", message)?;
        }
        if let Some(outcome) = &payload.outcome {
            validate_text("outcome", outcome)?;
        }
        Ok(())
    }

    pub(crate) fn validate_playback_snapshot(
        payload: &PlaybackSnapshotPayload,
    ) -> Result<(), String> {
        validate_session_id(&payload.session_id)?;
        validate_song_id(&payload.song_id)?;
        if payload.seq == 0 {
            return Err("event playback snapshot sequence must be positive".into());
        }
        validate_text("title", &payload.title)?;
        if payload.total_us > 86_400_000_000 {
            return Err("event playback total exceeds bounds".into());
        }
        if payload.current_us > payload.total_us {
            return Err("event playback current time exceeds total time".into());
        }
        if let Some(message) = &payload.message {
            validate_text("message", message)?;
        }
        Ok(())
    }

    pub(crate) fn validate_playback_finished(
        payload: &PlaybackFinishedPayload,
    ) -> Result<(), String> {
        validate_session_id(&payload.session_id)?;
        validate_song_id(&payload.song_id)?;
        validate_text("outcome", &payload.outcome)?;
        validate_text("message", &payload.message)
    }

    pub(crate) fn validate_playback_failed(payload: &PlaybackFailedPayload) -> Result<(), String> {
        validate_session_id(&payload.session_id)?;
        validate_song_id(&payload.song_id)?;
        validate_text("code", &payload.code)?;
        validate_text("message", &payload.message)
    }

    pub(crate) fn validate_diagnostics_snapshot(
        payload: &DiagnosticsSnapshotDto,
    ) -> Result<(), String> {
        if payload.seq == 0 {
            return Err("diagnostics sequence must be positive".into());
        }
        for (name, value) in [
            ("p50_ms", payload.p50_ms),
            ("p95_ms", payload.p95_ms),
            ("sigma_onset_ms", payload.sigma_onset_ms),
        ] {
            if let Some(value) = value
                && (!value.is_finite() || !(-60_000.0..=60_000.0).contains(&value))
            {
                return Err(format!("diagnostics {name} is outside bounds"));
            }
        }
        if let Some(max_lateness_us) = payload.max_lateness_us
            && max_lateness_us > 60_000_000
        {
            return Err("diagnostics max lateness is outside bounds".into());
        }
        if let Some(last_error) = &payload.last_error {
            validate_text("last_error", last_error)?;
        }
        if let Some(session_id) = &payload.session_id {
            validate_session_id(session_id)?;
        }
        Ok(())
    }

    pub(crate) fn validate_calibration_progress(
        payload: &CalibrationProgressPayload,
    ) -> Result<(), String> {
        validate_operation_id(&payload.operation_id)?;
        validate_text("phase", &payload.phase)?;
        validate_text("message", &payload.message)?;
        if payload.total == 0 || payload.total > 10_000 || payload.completed > payload.total {
            return Err("calibration progress is outside bounds".into());
        }
        Ok(())
    }

    pub(crate) fn validate_calibration_finished(
        payload: &CalibrationFinishedPayload,
    ) -> Result<(), String> {
        validate_operation_id(&payload.operation_id)?;
        validate_text("status", &payload.status)?;
        validate_text("source", &payload.source)?;
        validate_text("message", &payload.message)?;
        if payload.sample_count > 5_000 {
            return Err("calibration sample count exceeds bounds".into());
        }
        if payload
            .recommended_timing_margin_us
            .is_some_and(|value| value > sky_app_core::settings::MAX_TIMING_MARGIN_US)
        {
            return Err("calibration recommendation is outside bounds".into());
        }
        if payload.recommendation_qualified && payload.recommended_timing_margin_us.is_none() {
            return Err("qualified calibration is missing its recommendation".into());
        }
        Ok(())
    }

    pub(crate) fn validate_update_changed(payload: &UpdateSnapshotPayload) -> Result<(), String> {
        if payload.revision == 0 {
            return Err("event update revision must be positive".into());
        }
        validate_text("current_version", &payload.current_version)?;
        if let Some(version) = &payload.available_version {
            validate_text("available_version", version)?;
        }
        if let Some(notes) = &payload.release_notes {
            validate_text("release_notes", notes)?;
        }
        if let Some(published_at) = &payload.published_at {
            validate_text("published_at", published_at)?;
        }
        if let Some(error_detail) = &payload.error_detail {
            validate_text("error_detail", error_detail)?;
        }
        if let Some(operation_id) = &payload.operation_id {
            validate_session_id(operation_id)
                .map_err(|_| "update operation_id is not an opaque ID".to_string())?;
        }
        if let Some(progress) = &payload.progress {
            validate_text("message", &progress.message)?;
            if progress.completed > 2 * 1024 * 1024 * 1024 {
                return Err("update progress exceeds the bounded artifact size".into());
            }
            if progress.total.is_some_and(|total| {
                total == 0 || total > 2 * 1024 * 1024 * 1024 || progress.completed > total
            }) {
                return Err("update progress is outside bounds".into());
            }
        }
        Ok(())
    }
}

fn validate_session_id(value: &str) -> Result<(), String> {
    if value.len() != 32 || !value.bytes().all(is_lower_hex) {
        return Err("event session_id is not an opaque ID".into());
    }
    Ok(())
}

fn validate_song_id(value: &str) -> Result<(), String> {
    if value.len() != 32 || !value.bytes().all(is_lower_hex) {
        return Err("event song_id is not an opaque ID".into());
    }
    Ok(())
}

fn validate_operation_id(value: &str) -> Result<(), String> {
    validate_session_id(value).map_err(|_| "calibration operation_id is not an opaque ID".into())
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn validate_text(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_EVENT_TEXT_BYTES || value.contains('\0') {
        return Err(format!("event {name} is outside the bounded text contract"));
    }
    Ok(())
}
