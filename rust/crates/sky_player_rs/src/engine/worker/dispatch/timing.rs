//! Pure timing projection for the authored note-on path: resolves the QPC
//! evidence into typed timeline values, flags, and deferred observation data used by
//! telemetry, estimator updates, and the terminal SLO decision.
//!
//! Structural ownership: this module must not import `SharedMetrics`,
//! `TelemetryCollector`, `Mutex`, or any Python type.  It stays pure and
//! unit-testable (no wall-clock beyond the QPC samples already captured by the
//! caller into `result_started_ticks` / `result_completed_ticks`).

use super::super::super::{
    ActionKind, DurationTicks, PlaybackClockState, QpcClock, QpcTicks, RuntimeDispatchCoordinator,
    STRICT_SATURATION_ABORT_STREAK, TimelineTicks,
};
use super::super::health::{DispatchLeadEstimate, estimate_dispatch_path_lead};
use super::super::{
    DispatchPath, WorkerConfig, WorkerHealthState, WorkerRuntime, WorkerTimingState,
    signed_timeline_delta_ticks,
};
use super::{AuthoredBatchView, BatchViewResult, DispatchStep};
use crate::engine::config::TimingOptions;
use sky_dispatch_core::coordinator::{PreparedBatch, physical_packet_kind};
use sky_dispatch_core::estimator::{DispatchCostEstimator, SendPath};
use sky_dispatch_core::model::PhysicalPacketKind;
use sky_dispatch_win32::input::{PacketRetryReason, PhysicalPacket, SendTransactionStatus};

/// Snapshot of the next authored packet's dispatch path and lead.
///
/// Built once per worker loop epoch and reused for both
/// `prepare_next_due_authored` and wait-boundary planning.  The deadline is
/// the physical target for this authored work; `NextDispatchPlan::deadline_ticks`
/// may be earlier when a pending release is also present.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AuthoredDispatchPlan {
    pub(crate) path: DispatchPath,
    pub(crate) lead_us: u64,
    pub(crate) lead_ticks: DurationTicks,
    pub(crate) lead_saturated: bool,
    pub(crate) deadline_ticks: TimelineTicks,
}

/// Resolve the path-aware lead for one authored packet without reading QPC.
pub(crate) fn resolve_authored_lead(
    estimator: &DispatchCostEstimator,
    path: DispatchPath,
    timing: &TimingOptions,
    enable_dispatch_cost_lead: bool,
) -> DispatchLeadEstimate {
    if timing.dispatch_lead_us > 0 {
        return DispatchLeadEstimate {
            applied_us: timing.dispatch_lead_us,
            saturated: false,
        };
    }
    if !enable_dispatch_cost_lead {
        return DispatchLeadEstimate {
            applied_us: 0,
            saturated: false,
        };
    }
    estimate_dispatch_path_lead(estimator, path, timing.strict_timing, timing.max_lead_us)
}

/// Classify the next authored packet into a [`DispatchPath`].
///
/// Leading empty physical masks are stale-Up suppression metadata. Skip them
/// when selecting the startup/next physical path, while retaining the legacy
/// Down fallback if the remaining authored stream contains only stale Ups.
pub(crate) fn next_authored_path(coordinator: &RuntimeDispatchCoordinator) -> Option<DispatchPath> {
    let (up_mask, down_mask) = coordinator.next_physical_authored_packet().map_or_else(
        || coordinator.next_authored_packet_masks(),
        |(_, up_mask, down_mask)| Some((up_mask, down_mask)),
    )?;
    let up_count = up_mask.count_ones() as usize;
    let down_count = down_mask.count_ones() as usize;
    match physical_packet_kind(up_mask, down_mask) {
        Ok(PhysicalPacketKind::UpOnly) => Some(DispatchPath::UpOnly { up_count }),
        Ok(PhysicalPacketKind::DownOnly) => Some(DispatchPath::DownOnly { down_count }),
        Ok(PhysicalPacketKind::Mixed) => Some(DispatchPath::Mixed {
            up_count,
            down_count,
        }),
        Err(_) => {
            let polyphony = coordinator.next_authored_polyphony().max(1);
            Some(DispatchPath::DownOnly {
                down_count: polyphony,
            })
        }
    }
}

pub(crate) fn pending_lead_for_polyphony(
    estimator: &DispatchCostEstimator,
    qpc_clock: QpcClock,
    polyphony: usize,
    timing: &TimingOptions,
    enable_dispatch_cost_lead: bool,
) -> Result<(DurationTicks, bool), sky_dispatch_win32::clock::TimeConversionError> {
    let (lead_us, saturated) = if timing.dispatch_lead_us > 0 {
        (timing.dispatch_lead_us, false)
    } else if enable_dispatch_cost_lead {
        let estimate = estimator.estimate_lead(SendPath::UpOnly, polyphony, timing.strict_timing);
        (estimate.applied_us, estimate.saturated)
    } else {
        (0, false)
    };
    qpc_clock
        .duration_from_us(lead_us)
        .map(|ticks| (ticks, saturated))
}

/// Resolve the path-aware lead used to anchor the startup wait before the
/// first main-loop `NextDispatchPlan` is built.
///
/// The normal loop derives lead from the next authored packet's path; startup
/// must use the same policy instead of a hard-coded Down lead, otherwise a
/// first authored `UpOnly`/`Mixed` packet anchors its physical boundary with
/// the wrong directional lead.
pub(crate) fn startup_lead_for_first_packet(
    coordinator: &RuntimeDispatchCoordinator,
    estimator: &DispatchCostEstimator,
    timing: &TimingOptions,
    enable_dispatch_cost_lead: bool,
) -> u64 {
    let path = next_authored_path(coordinator).unwrap_or_else(|| DispatchPath::DownOnly {
        down_count: coordinator.next_authored_polyphony().max(1),
    });
    resolve_authored_lead(estimator, path, timing, enable_dispatch_cost_lead).applied_us
}

/// Typed transport/timing evidence shared by DownOnly, Mixed, and UpOnly
/// estimator observations.  The observer applies the one canonical predicate
/// after the hard dispatch-ready boundary; dispatch code only constructs this
/// immutable evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EstimatorObservationEvidence {
    pub status: SendTransactionStatus,
    pub attempts: u8,
    pub retry_reason: PacketRetryReason,
    pub requested_count: usize,
    pub confirmed_count: usize,
    pub skipped_count: usize,
    pub timing_valid: bool,
    pub transport_anomaly: bool,
    pub recovery_used: bool,
    pub chord_integrity_lost: bool,
}

/// Canonical clean estimator eligibility.  Every transport direction uses
/// this exact predicate, including Mixed, with no weaker UpOnly shortcut.
pub fn is_clean_estimator_observation(evidence: EstimatorObservationEvidence) -> bool {
    evidence.status == SendTransactionStatus::Complete
        && evidence.attempts == 1
        && evidence.retry_reason == PacketRetryReason::None
        && evidence.skipped_count == 0
        && evidence.confirmed_count == evidence.requested_count
        && evidence.timing_valid
        && !evidence.transport_anomaly
        && !evidence.recovery_used
        && !evidence.chord_integrity_lost
}

#[inline]
fn physical_event_counts(
    packet_masks: Option<PhysicalPacket>,
    dispatch_path: DispatchPath,
    result_success: bool,
    result_confirmed_mask: u16,
) -> (usize, usize) {
    let requested_count = packet_masks
        .map(PhysicalPacket::event_count)
        .map_or_else(|| dispatch_path.event_count(), usize::from);
    let confirmed_count = if result_success {
        // A Complete transaction confirms every event in the packet.  Keep
        // directional packet identity here: a union mask cannot represent a
        // same-key Up+Down retrigger as two INPUT events.
        requested_count
    } else {
        result_confirmed_mask.count_ones() as usize
    };
    (requested_count, confirmed_count)
}

/// Project the prepared authored batch into a snapshot used by admission, send,
/// and telemetry.  Built once per epoch so the worker does not re-query the
/// coordinator schedule across multiple invariants within a single loop epoch
/// (D1 — one immutable dispatch plan per epoch).
pub(crate) fn prepare_authored_batch_view(
    coordinator: &mut RuntimeDispatchCoordinator,
    prepared_batch: PreparedBatch,
) -> BatchViewResult {
    let batch_index = prepared_batch.index;
    let batch_scheduled_ticks = prepared_batch.effective_scheduled_ticks;
    // Every physical authored packet, including a single Down, uses the same
    // immutable Up-before-Down transport.  `None` is reserved for a stale Up
    // request whose physical packet was suppressed by the coordinator.
    let packet_mode = prepared_batch.packet_kind.is_some();
    let (
        batch_kind,
        dispatch_path,
        batch_source_action_index,
        batch_intent_count,
        conflict_mask,
        packet_masks,
    ) = if packet_mode {
        let packet_view = match coordinator
            .schedule
            .view_packet_ticks(prepared_batch.packet_index, batch_scheduled_ticks)
        {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "runtime packet view failure: {error}"
                )));
            }
        };
        let conflict_mask =
            coordinator.check_packet_down_conflicts(packet_view.up_mask(), packet_view.down_mask());
        let up_count = packet_view.up_mask().count_ones() as usize;
        let down_count = packet_view.down_mask().count_ones() as usize;
        let dispatch_path = match prepared_batch.packet_kind {
            Some(sky_dispatch_core::model::PhysicalPacketKind::UpOnly) => {
                DispatchPath::UpOnly { up_count }
            }
            Some(sky_dispatch_core::model::PhysicalPacketKind::DownOnly) => {
                DispatchPath::DownOnly { down_count }
            }
            Some(sky_dispatch_core::model::PhysicalPacketKind::Mixed) => DispatchPath::Mixed {
                up_count,
                down_count,
            },
            None => DispatchPath::DownOnly { down_count: 0 },
        };
        (
            if matches!(dispatch_path, DispatchPath::UpOnly { .. }) {
                ActionKind::Up
            } else {
                ActionKind::Down
            },
            dispatch_path,
            packet_view
                .header
                .down_source_action_index
                .or_else(|| {
                    coordinator
                        .schedule
                        .batches
                        .get(packet_view.header.first_batch_index as usize)
                        .map(|batch| batch.source_action_index)
                })
                .unwrap_or(0),
            up_count + down_count,
            conflict_mask,
            Some(sky_dispatch_win32::input::PhysicalPacket::new(
                packet_view.up_mask(),
                packet_view.down_mask(),
            )),
        )
    } else {
        let batch_view = match coordinator
            .schedule
            .view_batch_ticks(batch_index, batch_scheduled_ticks)
        {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "runtime schedule view failure: {error}"
                )));
            }
        };
        let conflict_mask = coordinator.check_down_conflicts_compact(batch_view.intents);
        (
            batch_view.kind(),
            match batch_view.kind() {
                ActionKind::Up => DispatchPath::UpOnly {
                    up_count: batch_view.intents.len(),
                },
                ActionKind::Down => DispatchPath::DownOnly {
                    down_count: batch_view.intents.len(),
                },
            },
            batch_view.source_action_index(),
            batch_view.intents.len(),
            conflict_mask,
            None,
        )
    };
    let authored_batch_scheduled_ticks = coordinator.batch_scheduled_ticks[batch_index];
    Ok(Some(AuthoredBatchView {
        prepared_batch,
        batch_source_action_index,
        batch_intent_count,
        batch_kind,
        batch_scheduled_ticks,
        authored_batch_scheduled_ticks,
        conflict_mask,
        dispatch_path,
        packet_masks,
    }))
}

/// Timing-derived evidence captured from the note-on SendInput call:
/// projections used across telemetry, estimator update, and the terminal
/// SLO decision.
pub(crate) struct DownSendTiming {
    pub(crate) sender_started_qpc: QpcTicks,
    pub(crate) sender_completed_qpc: QpcTicks,
    pub(crate) sender_started_effective_ticks: TimelineTicks,
    pub(crate) completed_effective_ticks: TimelineTicks,
    pub(crate) sender_duration_ticks: DurationTicks,
    pub(crate) completion_error_ticks_value: i64,
    pub(crate) authored_completion_error_ticks_value: i64,
    pub(crate) estimator_evidence: EstimatorObservationEvidence,
    pub(crate) recovered_partial_up: bool,
    pub(crate) recovered_retry_late: bool,
    pub(crate) strict_completion_late: bool,
    pub(crate) retry_late_abort: bool,
    pub(crate) saturation_abort: bool,
    pub(crate) saturation_streak: u8,
}

/// Resolves the QPC evidence, commits the prepared batch, and computes the
/// boundary values shared by the SLO flags, telemetry, and the estimator.
/// Mutates `coordinator` (commit) and `runtime` (last-send boundary).
#[allow(clippy::too_many_arguments)]
fn resolve_send_boundaries(
    view: &AuthoredBatchView,
    clock_state: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    coordinator: &mut RuntimeDispatchCoordinator,
    result_started_ticks: Option<QpcTicks>,
    result_completed_ticks: Option<QpcTicks>,
) -> Result<
    (
        TimelineTicks,
        TimelineTicks,
        DurationTicks,
        Option<DurationTicks>,
    ),
    DispatchStep,
> {
    let sender_started_ticks = match result_started_ticks {
        Some(ticks) => ticks,
        None => {
            return Err(DispatchStep::Terminate(
                "SendInput note-on succeeded without a QPC start boundary".to_string(),
            ));
        }
    };
    let completed_qpc_ticks = match result_completed_ticks {
        Some(ticks) => ticks,
        None => {
            return Err(DispatchStep::Terminate(
                "SendInput note-on completed without a QPC completion boundary".to_string(),
            ));
        }
    };
    let sender_duration_ticks =
        match completed_qpc_ticks.checked_duration_since(sender_started_ticks) {
            Ok(duration) => duration,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "note-on QPC ordering failure: {error}"
                )));
            }
        };
    let sender_started_effective_ticks = match clock_state.get_elapsed_allow_pre_epoch(
        sender_started_ticks,
        runtime.allow_pre_epoch_startup_dispatch,
    ) {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "playback clock failure: {error}"
            )));
        }
    };
    let completed_effective_ticks = match clock_state.get_elapsed_allow_pre_epoch(
        completed_qpc_ticks,
        runtime.allow_pre_epoch_startup_dispatch,
    ) {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "playback clock failure: {error}"
            )));
        }
    };
    let commit_result = coordinator.commit_packet_success(
        view.prepared_batch,
        sender_started_effective_ticks,
        completed_effective_ticks,
    );
    if let Err(error) = commit_result {
        return Err(DispatchStep::Terminate(format!(
            "coordinator activation failure: {error}"
        )));
    }
    let completion_lateness_ticks = completed_effective_ticks
        .checked_duration_since(view.batch_scheduled_ticks)
        .ok();
    Ok((
        sender_started_effective_ticks,
        completed_effective_ticks,
        sender_duration_ticks,
        completion_lateness_ticks,
    ))
}

/// Resolves the QPC evidence, commits the prepared batch, computes timing
/// SLO flags and the saturation-observation value.  Mutates `coordinator`
/// (commit) and `runtime` (last-send boundary), but does not mutate health,
/// record telemetry, or call the estimator.
#[allow(clippy::too_many_arguments)]
pub(crate) fn interpret_down_send_timing(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    clock_state: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    _qpc_clock: QpcClock,
    physical_target_qpc: QpcTicks,
    coordinator: &mut RuntimeDispatchCoordinator,
    health: &WorkerHealthState,
    timing: &WorkerTimingState,
    result_success: bool,
    result_status: SendTransactionStatus,
    result_started_ticks: Option<QpcTicks>,
    result_completed_ticks: Option<QpcTicks>,
    result_confirmed_mask: u16,
    result_skipped_mask: u16,
    result_send_attempts: u8,
    result_retry_reason: PacketRetryReason,
    result_chord_integrity_lost: bool,
    result_last_win32_error: Option<u32>,
    lead_down_saturated: bool,
) -> Result<DownSendTiming, DispatchStep> {
    if let Some(completed_qpc) = result_completed_ticks
        && completed_qpc
            .checked_duration_since(physical_target_qpc)
            .is_err()
    {
        return Err(DispatchStep::Terminate(
            "note-on completion precedes physical target boundary".to_string(),
        ));
    }
    let (
        sender_started_effective_ticks,
        completed_effective_ticks,
        sender_duration_ticks,
        completion_lateness_ticks,
    ) = resolve_send_boundaries(
        view,
        clock_state,
        runtime,
        coordinator,
        result_started_ticks,
        result_completed_ticks,
    )?;
    // Expose the raw QPC sender-completion boundary for the deferred observer.
    // Guaranteed `Some` here: a missing boundary already terminated inside
    // `resolve_send_boundaries`.
    let sender_started_qpc = result_started_ticks.unwrap_or(QpcTicks::ZERO);
    let sender_completed_qpc = result_completed_ticks.unwrap_or(QpcTicks::ZERO);
    let completion_error_ticks_value =
        match signed_timeline_delta_ticks(completed_effective_ticks, view.batch_scheduled_ticks) {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "note-on timing conversion failure: {error}"
                )));
            }
        };
    let authored_completion_error_ticks_value = match signed_timeline_delta_ticks(
        completed_effective_ticks,
        view.authored_batch_scheduled_ticks,
    ) {
        Ok(value) => value,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "note-on authored timing conversion failure: {error}"
            )));
        }
    };
    let (requested_count, delivered_count) = physical_event_counts(
        view.packet_masks,
        view.dispatch_path,
        result_success,
        result_confirmed_mask,
    );
    let recovered_zero_progress = matches!(result_retry_reason, PacketRetryReason::ZeroProgress);
    let recovered_partial_up = matches!(
        (view.dispatch_path, result_retry_reason),
        (
            DispatchPath::UpOnly { .. },
            PacketRetryReason::PartialProgress { .. }
        )
    ) && result_success;
    let recovered_retry_late = recovered_zero_progress
        && result_success
        && completion_lateness_ticks.is_some_and(|late| late > timing.retry_late_threshold_ticks);
    let retry_late_abort = config.timing.strict_timing && recovered_retry_late;
    let estimator_evidence = EstimatorObservationEvidence {
        status: result_status,
        attempts: result_send_attempts,
        retry_reason: result_retry_reason,
        requested_count,
        confirmed_count: delivered_count,
        skipped_count: result_skipped_mask.count_ones() as usize,
        timing_valid: true,
        transport_anomaly: result_last_win32_error.is_some(),
        recovery_used: !matches!(result_retry_reason, PacketRetryReason::None),
        chord_integrity_lost: result_chord_integrity_lost,
    };
    let clean_estimator_sample = is_clean_estimator_observation(estimator_evidence);
    let strict_completion_late = config.timing.strict_timing
        && clean_estimator_sample
        && completion_lateness_ticks.is_some_and(|late| {
            late > match view.dispatch_path {
                DispatchPath::UpOnly { .. } => timing.strict_up_completion_late_ticks,
                DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
                    timing.strict_down_completion_late_ticks
                }
            }
        });
    let saturation_positive =
        is_positive_saturation_residual(lead_down_saturated, completion_error_ticks_value);
    let saturation_streak = match view.dispatch_path {
        DispatchPath::UpOnly { .. } => {
            if saturation_positive {
                health.up_saturation_positive_streak.saturating_add(1)
            } else {
                0
            }
        }
        DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
            if saturation_positive {
                health.down_saturation_positive_streak.saturating_add(1)
            } else {
                0
            }
        }
    };
    let saturation_abort =
        config.timing.strict_timing && saturation_streak >= STRICT_SATURATION_ABORT_STREAK;
    Ok(DownSendTiming {
        sender_started_qpc,
        sender_completed_qpc,
        sender_started_effective_ticks,
        completed_effective_ticks,
        sender_duration_ticks,
        completion_error_ticks_value,
        authored_completion_error_ticks_value,
        estimator_evidence,
        recovered_partial_up,
        recovered_retry_late,
        strict_completion_late,
        retry_late_abort,
        saturation_abort,
        saturation_streak,
    })
}

#[inline]
fn is_positive_saturation_residual(lead_saturated: bool, completion_error_ticks: i64) -> bool {
    lead_saturated && completion_error_ticks > 0
}

pub(crate) fn read_qpc_us(
    qpc_clock: QpcClock,
    clock_state: &PlaybackClockState,
) -> Result<u64, DispatchStep> {
    match qpc_clock.now() {
        Ok(now) => {
            match qpc_clock.duration_to_us(match now.checked_duration_since(clock_state.epoch) {
                Ok(dur) => dur,
                Err(_) => DurationTicks::ZERO,
            }) {
                Ok(us) => Ok(us),
                Err(error) => Err(DispatchStep::Terminate(format!(
                    "QPC us conversion failure: {error:?}"
                ))),
            }
        }
        Err(error) => Err(DispatchStep::Terminate(format!("QPC failure: {error:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EstimatorObservationEvidence, is_clean_estimator_observation,
        is_positive_saturation_residual, physical_event_counts,
    };
    use crate::engine::worker::DispatchPath;
    use sky_dispatch_win32::input::{PacketRetryReason, PhysicalPacket, SendTransactionStatus};

    #[test]
    fn down_saturation_requires_strictly_positive_residual() {
        assert!(!is_positive_saturation_residual(true, -1));
        assert!(!is_positive_saturation_residual(true, 0));
        assert!(is_positive_saturation_residual(true, 1));
        assert!(!is_positive_saturation_residual(false, 1));
    }

    #[test]
    fn mixed_retrigger_retains_directional_event_cardinality() {
        let same_key = PhysicalPacket::new(0b001, 0b001);
        let three_events = PhysicalPacket::new(0b001, 0b011);

        assert_eq!(same_key.event_count(), 2);
        assert_eq!(three_events.event_count(), 3);
        assert_eq!(
            physical_event_counts(
                Some(same_key),
                DispatchPath::Mixed {
                    up_count: 1,
                    down_count: 1,
                },
                true,
                0b001,
            ),
            (2, 2)
        );
        assert!(is_clean_estimator_observation(
            EstimatorObservationEvidence {
                status: SendTransactionStatus::Complete,
                attempts: 1,
                retry_reason: PacketRetryReason::None,
                requested_count: 2,
                confirmed_count: 2,
                skipped_count: 0,
                timing_valid: true,
                transport_anomaly: false,
                recovery_used: false,
                chord_integrity_lost: false,
            }
        ));
    }
}
