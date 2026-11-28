use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::catalog::{RiskSummaryDto, SheetConversionModeDto, SheetPitchDto, SongDetailDto};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PlaybackPrepareRequest {
    pub song_id: String,
    pub generation: u64,
    pub config: PlaybackConfigDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub conversion_mode: Option<SheetConversionModeDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub target_pitch: Option<SheetPitchDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub conversion_confirmation_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlaybackConfigDto {
    pub hold_frames: f64,
    pub timing_margin_us: u64,
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
    pub(in crate::commands) fn method(self) -> &'static str {
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
#[serde(deny_unknown_fields)]
pub struct RiskDecisionDto {
    pub decision: PlaybackDecision,
    pub label: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum TimingAdvisoryStatus {
    NotAnalyzed,
    Unavailable,
    Analyzed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum DispatchModelStatus {
    NotAnalyzed,
    Unavailable,
    NoModeledDisplacement,
    ModeledDisplacement,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct DispatchModelScenarioDto {
    pub scenario_id: String,
    pub label: String,
    pub estimated_sender_setup_us: u64,
    pub estimated_per_input_us: u64,
    pub estimated_post_send_us: u64,
    pub modeled_send_count: u64,
    pub modeled_down_packet_count: u64,
    pub modeled_up_packet_count: u64,
    pub modeled_late_down_count: u64,
    pub max_modeled_down_lateness_us: u64,
    pub modeled_key_readiness_displaced_down_count: u64,
    pub max_modeled_key_readiness_displacement_us: u64,
    pub modeled_cursor_order_displaced_down_count: u64,
    pub max_modeled_cursor_order_displacement_us: u64,
    pub modeled_worker_occupancy_displaced_down_count: u64,
    pub max_modeled_worker_occupancy_residual_us: u64,
    pub modeled_up_slot_count: u64,
    pub modeled_h_floor_displaced_up_slot_count: u64,
    pub max_modeled_h_floor_displacement_us: u64,
    pub modeled_late_up_packet_count: u64,
    pub max_modeled_up_lateness_us: u64,
    pub max_modeled_up_pre_call_above_floor_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct GenerationTimingAdvisoryDto {
    pub status: TimingAdvisoryStatus,
    pub plan_fingerprint: String,
    pub explanation: String,
    pub total_authored_timestamp_group_count: Option<u64>,
    pub down_generation_count: Option<u64>,
    pub unmatched_up_count: Option<u64>,
    pub generation_limit: u64,
    pub work_limit_units: u64,
    pub working_memory_limit_bytes: u64,
    pub min_same_key_slack_us: Option<u64>,
    pub same_key_repeat_count: u64,
    pub zero_slack_repeat_count: u64,
    pub effective_hold_floor_h_us: Option<u64>,
    pub dispatch_model_status: DispatchModelStatus,
    pub dispatch_model_explanation: String,
    pub model_cost_disclaimer: String,
    pub scenarios: Vec<DispatchModelScenarioDto>,
    pub dry_run_is_hypothetical: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlaybackPlanVariantDto {
    pub decision: PlaybackDecision,
    pub config: PlaybackConfigDto,
    pub plan_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub timing_advisory: Option<GenerationTimingAdvisoryDto>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlaybackActiveStatusDto {
    pub session_id: String,
    pub prepared_id: String,
    pub song_id: String,
    pub title: String,
    pub state: PlaybackSessionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlaybackTerminalStatusDto {
    pub session_id: String,
    pub prepared_id: String,
    pub song_id: String,
    pub state: PlaybackSessionState,
    pub outcome: Option<String>,
    pub failure_code: Option<String>,
    pub failure_message: Option<String>,
    pub terminal_release_verified: Option<bool>,
    pub release_obligation_mask: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct PlaybackStatusDto {
    pub active: Option<PlaybackActiveStatusDto>,
    pub last_terminal: Option<PlaybackTerminalStatusDto>,
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
