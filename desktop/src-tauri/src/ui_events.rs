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
    Error,
    HandoffInProgress,
    HandoffReady,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct UpdateAvailablePayload {
    pub current_version: String,
    pub available_version: String,
    pub channel: UpdateChannel,
    pub release_notes: Option<String>,
    pub published_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct UpdateResultPayload {
    pub state: UpdateState,
    pub current_version: String,
    pub available_version: Option<String>,
    pub channel: UpdateChannel,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct UpdateHandoffReadyPayload {
    pub handoff_id: String,
    pub target_version: String,
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
    pub max_lateness_us: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub sigma_onset_ms: f64,
    pub late_2ms: u64,
    pub late_5ms: u64,
    pub late_10ms: u64,
    pub active_keys: u64,
    pub stuck_keys: u64,
    pub keys_dropped: u64,
    pub chord_split_events: u64,
    pub backend_status: DiagnosticsBackendStatus,
    pub release_max_us: Option<u64>,
    pub release_late_2ms: Option<u64>,
    pub session_id: Option<String>,
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
    pub margin_us: Option<u64>,
    pub sample_count: u64,
    pub source: String,
    pub message: String,
    pub applied: bool,
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
        payload: DiagnosticsSnapshotDto,
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
    #[serde(rename = "update.available")]
    UpdateAvailable {
        v: u64,
        payload: UpdateAvailablePayload,
    },
    #[serde(rename = "update.result")]
    UpdateResult {
        v: u64,
        payload: UpdateResultPayload,
    },
    #[serde(rename = "update.handoff_ready")]
    UpdateHandoffReady {
        v: u64,
        payload: UpdateHandoffReadyPayload,
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
            if !value.is_finite() || !(-60_000.0..=60_000.0).contains(&value) {
                return Err(format!("diagnostics {name} is outside bounds"));
            }
        }
        if payload.max_lateness_us > 60_000_000 {
            return Err("diagnostics max lateness is outside bounds".into());
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
        if payload.margin_us.is_some_and(|value| value > 60_000_000) {
            return Err("calibration margin is outside bounds".into());
        }
        Ok(())
    }

    pub(crate) fn validate_update_available(
        payload: &UpdateAvailablePayload,
    ) -> Result<(), String> {
        validate_text("current_version", &payload.current_version)?;
        validate_text("available_version", &payload.available_version)?;
        if let Some(notes) = &payload.release_notes {
            validate_text("release_notes", notes)?;
        }
        if let Some(published_at) = &payload.published_at {
            validate_text("published_at", published_at)?;
        }
        Ok(())
    }

    pub(crate) fn validate_update_result(payload: &UpdateResultPayload) -> Result<(), String> {
        validate_text("current_version", &payload.current_version)?;
        if let Some(version) = &payload.available_version {
            validate_text("available_version", version)?;
        }
        if let Some(error) = &payload.error {
            validate_text("error", error)?;
        }
        Ok(())
    }

    pub(crate) fn validate_update_handoff(
        payload: &UpdateHandoffReadyPayload,
    ) -> Result<(), String> {
        validate_operation_id(&payload.handoff_id)?;
        validate_text("target_version", &payload.target_version)
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
