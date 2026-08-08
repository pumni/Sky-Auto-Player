use super::super::wait::WaitObservation;
use super::super::{DispatchPath, LatencyClass};
use super::timing::EstimatorObservationEvidence;
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use sky_dispatch_win32::input::PacketRetryReason;

pub const OBSERVATION_QUEUE_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug)]
pub enum DispatchObservation {
    Down(DownObservation),
    Up(UpObservation),
    Wait(WaitObservation),
}

#[derive(Clone, Copy, Debug)]
pub struct DownTraceObservation {
    pub event_index: u32,
    pub trace_kind: u8,
    pub result_success: bool,
    pub requested_count: usize,
    pub sent_count: usize,
    pub skipped_count: usize,
    pub send_attempts: u8,
    pub retry_reason: PacketRetryReason,
    pub chord_integrity_lost: bool,
    pub last_win32_error: u32,
    pub authored_ticks: TimelineTicks,
    pub effective_deadline_ticks: TimelineTicks,
    pub wake_ticks: TimelineTicks,
    pub sender_started_ticks: Option<TimelineTicks>,
    pub sender_completed_ticks: Option<TimelineTicks>,
    pub completion_error_ticks: i64,
    pub authored_completion_error_ticks: i64,
    pub applied_lead_ticks: DurationTicks,
    pub recovered_retry_late: bool,
    pub recovered_partial_up: bool,
    pub strict_completion_late: bool,
    pub retry_late_abort: bool,
    pub saturation_abort: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct DownObservation {
    pub path: DispatchPath,
    pub latency_class: LatencyClass,
    pub lead_down_saturated: bool,
    pub lead_down: u64,
    pub sender_duration_us: u64,
    pub delivered_count: usize,
    pub batch_intent_count: usize,
    pub completion_error_us: i64,
    pub estimator_evidence: EstimatorObservationEvidence,
    pub completed_effective: u64,
    pub authored_batch_scheduled_us: u64,
    pub batch_scheduled_us: u64,
    pub sender_completed_qpc: QpcTicks,
    pub worker_ready_qpc: QpcTicks,
    pub send_warn_us: u64,
    pub core_post_send_warn_us: u64,
    pub trace: DownTraceObservation,
}

#[derive(Clone, Copy, Debug)]
pub struct UpTraceObservation {
    pub event_index: u32,
    pub trace_kind: u8,
    pub scan_count: usize,
    pub sent_count: usize,
    pub skipped_count: usize,
    pub send_attempts: u8,
    pub last_win32_error: u32,
    pub authored_ticks: TimelineTicks,
    pub effective_deadline_ticks: TimelineTicks,
    pub wake_ticks: TimelineTicks,
    pub sender_started_ticks: Option<TimelineTicks>,
    pub sender_completed_ticks: Option<TimelineTicks>,
    pub completion_error_ticks: i64,
    pub authored_completion_error_ticks: i64,
    pub applied_lead_ticks: DurationTicks,
    pub deferred_by_us: u64,
    pub recovery_required: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct UpObservation {
    pub latency_class: LatencyClass,
    pub sender_duration_us: u64,
    pub sent_count: usize,
    pub scan_count: usize,
    pub lead_up: u64,
    pub lead_up_saturated: bool,
    pub completed_effective: u64,
    pub scheduled_us: u64,
    pub deferred_by_us: u64,
    pub up_completion_error_us: i64,
    pub estimator_evidence: EstimatorObservationEvidence,
    pub sender_completed_qpc: QpcTicks,
    pub worker_ready_qpc: QpcTicks,
    pub send_warn_us: u64,
    pub core_post_send_warn_us: u64,
    pub trace: UpTraceObservation,
    pub recovery_pause_ticks: Option<DurationTicks>,
    pub strict_up_completion_late: bool,
    pub saturation_abort: bool,
}
