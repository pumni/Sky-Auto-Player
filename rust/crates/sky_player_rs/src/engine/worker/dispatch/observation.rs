use super::super::wait::WaitObservation;
use super::super::{
    DispatchPath, DispatchStep, LatencyClass, WorkerHealthState, observe_wait_health,
    wake_lateness_ticks,
};
use super::timing::EstimatorObservationEvidence;
use crate::engine::telemetry::WorkerMetricsLocal;
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use sky_dispatch_win32::input::PacketRetryReason;
use sky_dispatch_win32::wait::WaitOutcome;

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
    pub timeline_rebase_count: u64,
    pub timeline_rebase_total_ticks: DurationTicks,
    pub timeline_rebase_max_ticks: DurationTicks,
    pub timeline_rebase_last_reason: u8,
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
    pub lead_up_ticks: DurationTicks,
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

/// Drain-side wait conversion. Raw wait evidence stays on the worker-owned
/// queue until observer slack is available; the wait facade only executes the
/// kernel wait and hands back raw timing.
pub(crate) fn drain_wait_observation(
    observation: &WaitObservation,
    health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    qpc_clock: sky_dispatch_win32::clock::QpcClock,
) -> Result<(), DispatchStep> {
    let spin_us = qpc_clock
        .duration_to_us(observation.spin_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("wait observer spin conversion failure: {error:?}"))
        })?;
    local_metrics.idle_wake_count = local_metrics.idle_wake_count.saturating_add(1);
    local_metrics.spin_time_us = local_metrics.spin_time_us.saturating_add(spin_us);
    if !matches!(observation.outcome, WaitOutcome::Deadline) {
        return Ok(());
    }
    let wake_qpc = observation.wake_qpc.ok_or_else(|| {
        DispatchStep::Terminate("wait observer missing deadline QPC evidence".to_string())
    })?;
    let wake_elapsed_ticks = if observation.allow_pre_epoch_startup_dispatch
        && wake_qpc < observation.epoch_qpc
    {
        TimelineTicks::ZERO
    } else {
        let elapsed = wake_qpc
            .checked_duration_since(observation.epoch_qpc)
            .map_err(|error| {
                DispatchStep::Terminate(format!("wait observer QPC ordering failure: {error:?}"))
            })?;
        TimelineTicks::from_raw(elapsed.as_u64())
    };
    let wake_error_ticks = wake_lateness_ticks(wake_elapsed_ticks, observation.deadline_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("wait observer target arithmetic failure: {error}"))
        })?;
    let wake_error_us = qpc_clock
        .duration_to_us(wake_error_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("wait observer wake conversion failure: {error:?}"))
        })?;
    local_metrics.wait_target_error_us = local_metrics.wait_target_error_us.max(wake_error_us);
    let elapsed_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(wake_elapsed_ticks.as_u64()))
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "wait observer elapsed conversion failure: {error:?}"
            ))
        })?;
    observe_wait_health(
        wake_error_us,
        health.options.wait_warn_us,
        elapsed_us,
        health.options.window_policy(),
        &mut health.wait_window,
        local_metrics,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::DispatchHealthOptions;
    use std::num::NonZeroU64;

    #[test]
    fn wait_observation_defers_conversion_and_health_updates() {
        let qpc_clock = sky_dispatch_win32::clock::QpcClock::from_frequency_hz(
            NonZeroU64::new(1_000_000).unwrap(),
        );
        let mut health = WorkerHealthState::new(DispatchHealthOptions::default());
        let mut local_metrics = WorkerMetricsLocal::default();
        let observation = WaitObservation {
            outcome: WaitOutcome::Deadline,
            wake_qpc: Some(QpcTicks::from_raw(2_500)),
            spin_ticks: DurationTicks::from_raw(100),
            deadline_ticks: TimelineTicks::from_raw(1_000),
            epoch_qpc: QpcTicks::from_raw(1_000),
            allow_pre_epoch_startup_dispatch: false,
        };

        drain_wait_observation(&observation, &mut health, &mut local_metrics, qpc_clock)
            .expect("raw wait observation should be drainable");

        assert_eq!(local_metrics.idle_wake_count, 1);
        assert_eq!(local_metrics.spin_time_us, 100);
        assert_eq!(local_metrics.wait_target_error_us, 500);
        assert_eq!(local_metrics.wait_degraded_samples, 1);
        assert_eq!(local_metrics.wait_window_sample_count, 1);
    }
}
