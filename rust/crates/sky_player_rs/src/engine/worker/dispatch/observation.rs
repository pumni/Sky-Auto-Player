use super::super::super::{
    QpcClock, RtTraceRecord, TRACE_FLAG_ANOMALY, TRACE_FLAG_DEFERRED, TRACE_FLAG_RECOVERY,
    TRACE_FLAG_SENT_FULL, TelemetryCollector, TraceContext, TraceDelivery, TraceTiming,
    trace_outcome_code,
};
use super::super::wait::WaitObservation;
use super::super::{
    DispatchPath, DispatchStep, WorkerHealthState, observe_wait_health, release_runtime_outcome,
    wake_lateness_ticks,
};
use super::timing::DispatchObservationEvidence;
use crate::engine::telemetry::WorkerMetricsLocal;
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use sky_dispatch_win32::input::{PacketRetryReason, PhysicalPacket, SendTransactionStatus};
use sky_dispatch_win32::wait::WaitOutcome;

pub const OBSERVATION_QUEUE_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug)]
pub enum DispatchObservation {
    Down(DownObservation),
    // Retained for the observer schema and test/support scenarios; the
    // production path currently does not construct this variant.
    #[allow(dead_code)]
    Up(UpObservation),
    Wait(WaitObservation),
    StaleMetadata(StaleMetadataObservation),
    BlockedUnfocused(BlockedUnfocusedObservation),
}

#[derive(Clone, Copy, Debug)]
pub struct StaleMetadataObservation {
    pub source_action_index: u32,
    pub effective_scheduled_ticks: TimelineTicks,
    pub effective_now_ticks: TimelineTicks,
    pub suppressed_intent_count: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct BlockedUnfocusedObservation {
    pub event_index: u32,
    pub authored_ticks: TimelineTicks,
    pub effective_deadline_ticks: TimelineTicks,
    pub effective_now_ticks: TimelineTicks,
    pub polyphony: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct DownTraceObservation {
    pub event_index: u32,
    pub trace_kind: u8,
    pub result_status: SendTransactionStatus,
    pub send_attempts: u8,
    pub retry_reason: PacketRetryReason,
    pub chord_integrity_lost: bool,
    pub last_win32_error: u32,
    pub authored_ticks: TimelineTicks,
    pub effective_deadline_ticks: TimelineTicks,
    pub wake_ticks: TimelineTicks,
    pub final_admission_ticks: Option<TimelineTicks>,
    pub sendinput_completed_ticks: Option<TimelineTicks>,
    pub recovered_retry_late: bool,
    pub recovered_partial_up: bool,
    pub strict_completion_late: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct DownObservation {
    pub path: DispatchPath,
    pub physical_target_qpc: QpcTicks,
    pub final_admission_qpc: QpcTicks,
    pub sendinput_completed_qpc: QpcTicks,
    pub dispatch_ready_qpc: Option<QpcTicks>,
    pub admission_to_completion_ticks: DurationTicks,
    /// Raw QPC wake sample; derivation is deferred to the observer drain.
    pub wake_qpc: Option<QpcTicks>,
    pub requested_packet: PhysicalPacket,
    pub confirmed_mask: u16,
    pub skipped_mask: u16,
    pub completed_effective_ticks: TimelineTicks,
    pub trace: DownTraceObservation,
}

impl DownTraceObservation {
    pub(super) const fn result_success(self) -> bool {
        matches!(self.result_status, SendTransactionStatus::Complete)
    }
}

impl DownObservation {
    pub(super) const fn requested_count(self) -> usize {
        down_transport_counts(
            self.requested_packet,
            self.confirmed_mask,
            self.skipped_mask,
            self.trace.result_success(),
        )
        .0
    }

    pub(super) const fn confirmed_count(self) -> usize {
        down_transport_counts(
            self.requested_packet,
            self.confirmed_mask,
            self.skipped_mask,
            self.trace.result_success(),
        )
        .1
    }

    pub(super) const fn skipped_count(self) -> usize {
        down_transport_counts(
            self.requested_packet,
            self.confirmed_mask,
            self.skipped_mask,
            self.trace.result_success(),
        )
        .2
    }
}

pub(super) const fn down_transport_counts(
    requested_packet: PhysicalPacket,
    confirmed_mask: u16,
    skipped_mask: u16,
    confirmed_all_events: bool,
) -> (usize, usize, usize) {
    let requested_count = requested_packet.event_count() as usize;
    (
        requested_count,
        if confirmed_all_events {
            requested_count
        } else {
            confirmed_mask.count_ones() as usize
        },
        skipped_mask.count_ones() as usize,
    )
}

#[derive(Clone, Copy, Debug)]
pub struct UpTraceObservation {
    pub event_index: u32,
    pub trace_kind: u8,
    pub retry_reason: PacketRetryReason,
    pub send_attempts: u8,
    pub last_win32_error: u32,
    pub authored_ticks: TimelineTicks,
    pub effective_deadline_ticks: TimelineTicks,
    pub wake_ticks: TimelineTicks,
    pub final_admission_ticks: Option<TimelineTicks>,
    pub sendinput_completed_ticks: Option<TimelineTicks>,
    pub dispatch_start_error_ticks: i64,
    pub completion_error_ticks: i64,
    pub authored_completion_error_ticks: i64,
    pub deferred_ticks: DurationTicks,
    pub recovery_required: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct UpObservation {
    pub physical_target_qpc: QpcTicks,
    pub final_admission_qpc: QpcTicks,
    pub sendinput_completed_qpc: QpcTicks,
    pub dispatch_ready_qpc: Option<QpcTicks>,
    pub admission_to_completion_ticks: DurationTicks,
    pub wake_qpc: Option<QpcTicks>,
    pub requested_mask: u16,
    pub confirmed_mask: u16,
    pub skipped_mask: u16,
    pub result_status: SendTransactionStatus,
    pub completed_effective_ticks: TimelineTicks,
    pub scheduled_ticks: TimelineTicks,
    pub deferred_ticks: DurationTicks,
    pub up_completion_error_ticks: i64,
    pub trace: UpTraceObservation,
    pub recovery_pause_ticks: Option<DurationTicks>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn record_down_send_telemetry(
    observation: &DownObservation,
    telemetry: &mut TelemetryCollector,
    core_post_send_us: u64,
    completion_residual_us: u64,
    post_send_metrics_available: bool,
    dispatch_start_error_ticks: i64,
    completion_error_ticks: i64,
    authored_completion_error_ticks: i64,
) -> Result<(&'static str, bool), DispatchStep> {
    let trace = observation.trace;
    let down_outcome = if trace.recovered_retry_late {
        "recovered_zero_progress_but_late"
    } else if trace.recovered_partial_up {
        "recovered_partial_up_retry"
    } else if trace.strict_completion_late {
        "strict_completion_slo_exceeded"
    } else if trace.chord_integrity_lost {
        "chord_integrity_lost"
    } else if trace.result_success()
        && observation.confirmed_count() == observation.requested_count()
    {
        "sent"
    } else {
        "partial_note_on"
    };
    let force_publish = !trace.result_success()
        || !matches!(trace.retry_reason, PacketRetryReason::None)
        || trace.chord_integrity_lost;
    let mut trace_flags = 0u8;
    if trace.result_success() && observation.confirmed_count() == observation.requested_count() {
        trace_flags |= TRACE_FLAG_SENT_FULL;
    }
    if trace.recovered_retry_late || trace.chord_integrity_lost {
        trace_flags |= TRACE_FLAG_RECOVERY;
    }
    if down_outcome != "sent" {
        trace_flags |= TRACE_FLAG_ANOMALY;
    }
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: trace.event_index,
                kind: trace.trace_kind,
                outcome: trace_outcome_code(down_outcome),
                polyphony: observation.requested_count(),
                flags: trace_flags,
                win32_error: trace.last_win32_error,
            },
            TraceTiming {
                authored_ticks: trace.authored_ticks,
                effective_deadline_ticks: trace.effective_deadline_ticks,
                wake_ticks: trace.wake_ticks,
                final_admission_ticks: trace.final_admission_ticks,
                sendinput_completed_ticks: trace.sendinput_completed_ticks,
                completion_residual_us,
                core_post_send_duration_us: core_post_send_us,
                post_send_metrics_available,
                dispatch_start_error_ticks,
                completion_error_ticks,
                authored_completion_error_ticks,
            },
            TraceDelivery {
                requested: observation.requested_count(),
                sent: observation.confirmed_count(),
                skipped: observation.skipped_count(),
                send_attempts: usize::from(trace.send_attempts),
            },
        )
    }) {
        return Err(DispatchStep::Terminate(format!(
            "native telemetry record overflow: {error}"
        )));
    }
    Ok((down_outcome, force_publish))
}

pub(super) fn record_release_telemetry(
    telemetry: &mut TelemetryCollector,
    observation: &UpObservation,
    qpc_clock: QpcClock,
    core_post_send_us: u64,
    completion_residual_us: u64,
    post_send_metrics_available: bool,
) -> Result<(), DispatchStep> {
    let trace = observation.trace;
    let deferred_by_us = qpc_clock
        .duration_to_us(trace.deferred_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-off deferral conversion failure: {error:?}"))
        })?;
    let (scan_count, sent_count, skipped_count) = up_transport_counts(observation);
    let release_outcome = release_runtime_outcome(
        deferred_by_us,
        sent_count,
        scan_count,
        trace.recovery_required,
    );
    let mut trace_flags = 0u8;
    if sent_count == scan_count {
        trace_flags |= TRACE_FLAG_SENT_FULL;
    }
    if release_outcome == "deferred_release" || release_outcome == "failed_note_off" {
        trace_flags |= TRACE_FLAG_RECOVERY;
    }
    if deferred_by_us > 0 {
        trace_flags |= TRACE_FLAG_DEFERRED;
    }
    if release_outcome != "sent" {
        trace_flags |= TRACE_FLAG_ANOMALY;
    }
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: trace.event_index,
                kind: trace.trace_kind,
                outcome: trace_outcome_code(release_outcome),
                polyphony: scan_count,
                flags: trace_flags,
                win32_error: trace.last_win32_error,
            },
            TraceTiming {
                authored_ticks: trace.authored_ticks,
                effective_deadline_ticks: trace.effective_deadline_ticks,
                wake_ticks: trace.wake_ticks,
                final_admission_ticks: trace.final_admission_ticks,
                sendinput_completed_ticks: trace.sendinput_completed_ticks,
                completion_residual_us,
                core_post_send_duration_us: core_post_send_us,
                post_send_metrics_available,
                dispatch_start_error_ticks: trace.dispatch_start_error_ticks,
                completion_error_ticks: trace.completion_error_ticks,
                authored_completion_error_ticks: trace.authored_completion_error_ticks,
            },
            TraceDelivery {
                requested: scan_count,
                sent: sent_count,
                skipped: skipped_count,
                send_attempts: usize::from(trace.send_attempts),
            },
        )
    }) {
        return Err(DispatchStep::Terminate(format!(
            "native telemetry record overflow: {error}"
        )));
    }
    Ok(())
}

pub(super) const fn up_transport_counts(observation: &UpObservation) -> (usize, usize, usize) {
    up_transport_counts_from_masks(
        observation.requested_mask,
        observation.confirmed_mask,
        observation.skipped_mask,
    )
}

pub(super) const fn up_transport_counts_from_masks(
    requested_mask: u16,
    confirmed_mask: u16,
    skipped_mask: u16,
) -> (usize, usize, usize) {
    (
        requested_mask.count_ones() as usize,
        confirmed_mask.count_ones() as usize,
        skipped_mask.count_ones() as usize,
    )
}

pub(super) fn up_dispatch_evidence(observation: &UpObservation) -> DispatchObservationEvidence {
    let (requested_count, confirmed_count, skipped_count) = up_transport_counts(observation);
    DispatchObservationEvidence {
        status: observation.result_status,
        attempts: observation.trace.send_attempts,
        retry_reason: observation.trace.retry_reason,
        requested_count,
        confirmed_count,
        skipped_count,
        timing_valid: true,
        transport_anomaly: observation.trace.last_win32_error != 0
            || !matches!(observation.result_status, SendTransactionStatus::Complete),
        recovery_used: observation.trace.recovery_required
            || observation.deferred_ticks > DurationTicks::ZERO,
        chord_integrity_lost: false,
    }
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
    fn raw_down_masks_derive_transport_counts_when_observed() {
        assert_eq!(
            down_transport_counts(PhysicalPacket::new(0b001, 0b001), 0b001, 0, true,),
            (2, 2, 0)
        );
        assert_eq!(
            down_transport_counts(PhysicalPacket::new(0b001, 0b011), 0b001, 0, true,),
            (3, 3, 0)
        );
    }

    #[test]
    fn raw_up_masks_derive_transport_counts_when_observed() {
        assert_eq!(
            up_transport_counts_from_masks(0b111, 0b101, 0b010),
            (3, 2, 1)
        );
    }

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
