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
    signed_ticks_to_us, signed_timeline_delta_ticks,
};
use super::{AuthoredBatchView, BatchViewResult, DispatchStep};
use crate::engine::config::TimingOptions;
use sky_dispatch_core::coordinator::{PreparedBatch, physical_packet_kind};
use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator, SendPath};
use sky_dispatch_core::model::PhysicalPacketKind;
use sky_dispatch_win32::input::{PacketRetryReason, SendTransactionStatus};
use smallvec::SmallVec;

/// Snapshot of the next authored packet's dispatch path and lead.
///
/// Built once per worker loop epoch and reused for both
/// `prepare_next_due_authored` and `next_deadline_ticks`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AuthoredDispatchPlan {
    pub(crate) path: DispatchPath,
    pub(crate) lead_us: u64,
    pub(crate) lead_ticks: DurationTicks,
    pub(crate) lead_saturated: bool,
}

/// Resolve the path-aware lead for one authored packet without reading QPC.
pub(crate) fn resolve_authored_lead(
    estimator: &SendLatencyEstimator,
    path: DispatchPath,
    latency_class: LatencyClass,
    timing: &TimingOptions,
    enable_adaptive_lead: bool,
) -> DispatchLeadEstimate {
    if timing.dispatch_lead_us > 0 {
        return DispatchLeadEstimate {
            applied_us: timing.dispatch_lead_us,
            saturated: false,
        };
    }
    if !enable_adaptive_lead {
        return DispatchLeadEstimate {
            applied_us: 0,
            saturated: false,
        };
    }
    estimate_dispatch_path_lead(
        estimator,
        path,
        latency_class,
        timing.strict_timing,
        timing.max_lead_us,
    )
}

/// Classify the next authored packet into a [`DispatchPath`].
///
/// Empty physical masks (stale Up suppression metadata) keep the historical
/// Down-polyphony fallback so wait/prepare stay consistent with prior behavior.
pub(crate) fn next_authored_path(coordinator: &RuntimeDispatchCoordinator) -> Option<DispatchPath> {
    let (up_mask, down_mask) = coordinator.next_authored_packet_masks()?;
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
    estimator: &SendLatencyEstimator,
    qpc_clock: QpcClock,
    polyphony: usize,
    latency_class: LatencyClass,
    timing: &TimingOptions,
    enable_adaptive_lead: bool,
) -> Result<(DurationTicks, bool), sky_dispatch_win32::clock::TimeConversionError> {
    let (lead_us, saturated) = if timing.dispatch_lead_us > 0 {
        (timing.dispatch_lead_us, false)
    } else if enable_adaptive_lead {
        let estimate = estimator.estimate_lead_with_class_and_policy(
            SendPath::UpOnly,
            polyphony,
            latency_class,
            timing.strict_timing,
        );
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
    estimator: &SendLatencyEstimator,
    latency_class: LatencyClass,
    timing: &TimingOptions,
    enable_adaptive_lead: bool,
) -> u64 {
    let path = next_authored_path(coordinator).unwrap_or_else(|| DispatchPath::DownOnly {
        down_count: coordinator.next_authored_polyphony().max(1),
    });
    resolve_authored_lead(estimator, path, latency_class, timing, enable_adaptive_lead).applied_us
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

/// Project the prepared authored batch into a snapshot used by admission, send,
/// and telemetry.  Built once per epoch so the worker does not re-query the
/// coordinator schedule across multiple invariants within a single loop epoch
/// (D1 — one immutable dispatch plan per epoch).
pub(crate) fn prepare_authored_batch_view(
    coordinator: &mut RuntimeDispatchCoordinator,
    qpc_clock: QpcClock,
    prepared_batch: PreparedBatch,
) -> BatchViewResult {
    let batch_index = prepared_batch.index;
    let batch_scheduled_ticks = prepared_batch.effective_scheduled_ticks;
    let packet_mode = match prepared_batch.packet_kind {
        Some(sky_dispatch_core::model::PhysicalPacketKind::DownOnly)
            if prepared_batch.packet_batch_count == 1 =>
        {
            false
        }
        Some(_) => true,
        None => false,
    };
    let (
        batch_kind,
        dispatch_path,
        batch_source_action_index,
        batch_intent_count,
        conflict_mask,
        scan_batch,
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
        let conflict_mask = coordinator
            .check_packet_down_conflicts(packet_view.up_mask(), packet_view.down_intents);
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
            packet_view.down_scan_code_batch(),
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
            batch_view.scan_code_batch_excluding_mask(conflict_mask),
            None,
        )
    };
    let batch_scheduled_us = match qpc_clock.timeline_to_us(batch_scheduled_ticks) {
        Ok(value) => value,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "schedule telemetry conversion failure: {error:?}"
            )));
        }
    };
    let authored_batch_scheduled_ticks = coordinator.batch_scheduled_ticks[batch_index];
    let authored_batch_scheduled_us = match qpc_clock.timeline_to_us(authored_batch_scheduled_ticks)
    {
        Ok(value) => value,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "authored schedule telemetry conversion failure: {error:?}"
            )));
        }
    };
    Ok(Some(AuthoredBatchView {
        prepared_batch,
        batch_source_action_index,
        batch_intent_count,
        batch_kind,
        batch_scheduled_ticks,
        batch_scheduled_us,
        authored_batch_scheduled_ticks,
        authored_batch_scheduled_us,
        conflict_mask,
        dispatch_path,
        packet_mode,
        packet_masks,
        scan_batch,
    }))
}

/// Timing-derived evidence captured from the note-on SendInput call:
/// projections used across telemetry, estimator update, and the terminal
/// SLO decision.
pub(crate) struct DownSendTiming {
    pub(crate) sender_completed_qpc: QpcTicks,
    pub(crate) sender_started_effective_ticks: TimelineTicks,
    pub(crate) completed_effective_ticks: TimelineTicks,
    pub(crate) completed_effective: u64,
    pub(crate) sender_duration_us: u64,
    pub(crate) requested_count: usize,
    pub(crate) delivered_count: usize,
    pub(crate) completion_error_ticks_value: i64,
    pub(crate) authored_completion_error_ticks_value: i64,
    pub(crate) completion_error_us: i64,
    pub(crate) estimator_evidence: EstimatorObservationEvidence,
    pub(crate) recovered_zero_progress: bool,
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
    qpc_clock: QpcClock,
    coordinator: &mut RuntimeDispatchCoordinator,
    result_started_ticks: Option<QpcTicks>,
    result_completed_ticks: Option<QpcTicks>,
    result_sent: &SmallVec<[u16; 15]>,
) -> Result<
    (
        TimelineTicks,
        TimelineTicks,
        u64,
        u64,
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
    let sender_duration_us = match qpc_clock.duration_to_us(sender_duration_ticks) {
        Ok(duration) => duration,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "note-on sender duration conversion failure: {error:?}"
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
    let completed_effective = match qpc_clock.duration_to_us(
        match completed_effective_ticks.checked_duration_since(TimelineTicks::ZERO) {
            Ok(dur) => dur,
            Err(_) => DurationTicks::ZERO,
        },
    ) {
        Ok(us) => us,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "playback clock conversion failure: {error:?}"
            )));
        }
    };
    runtime.last_send_qpc_ticks = Some(completed_qpc_ticks);
    let commit_result = if view.packet_mode {
        coordinator.commit_packet_success(
            view.prepared_batch,
            sender_started_effective_ticks,
            completed_effective_ticks,
        )
    } else {
        coordinator.commit_down_success(
            view.prepared_batch,
            result_sent,
            sender_started_effective_ticks,
            completed_effective_ticks,
        )
    };
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
        completed_effective,
        sender_duration_us,
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
    qpc_clock: QpcClock,
    coordinator: &mut RuntimeDispatchCoordinator,
    health: &WorkerHealthState,
    timing: &WorkerTimingState,
    result_success: bool,
    result_status: SendTransactionStatus,
    result_started_ticks: Option<QpcTicks>,
    result_completed_ticks: Option<QpcTicks>,
    result_sent: &SmallVec<[u16; 15]>,
    result_skipped_duplicates: &SmallVec<[u16; 15]>,
    result_send_attempts: u8,
    result_retry_reason: PacketRetryReason,
    result_chord_integrity_lost: bool,
    result_last_win32_error: Option<u32>,
    lead_down_saturated: bool,
) -> Result<DownSendTiming, DispatchStep> {
    let (
        sender_started_effective_ticks,
        completed_effective_ticks,
        completed_effective,
        sender_duration_us,
        completion_lateness_ticks,
    ) = resolve_send_boundaries(
        view,
        clock_state,
        runtime,
        qpc_clock,
        coordinator,
        result_started_ticks,
        result_completed_ticks,
        result_sent,
    )?;
    // Expose the raw QPC sender-completion boundary for the deferred observer.
    // Guaranteed `Some` here: a missing boundary already terminated inside
    // `resolve_send_boundaries`.
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
    let completion_error_us = match signed_ticks_to_us(qpc_clock, completion_error_ticks_value) {
        Ok(value) => value,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "note-on timing conversion failure: {error}"
            )));
        }
    };
    let requested_count = view.dispatch_path.event_count();
    let delivered_count = if view.packet_mode {
        usize::from(result_success) * requested_count
    } else {
        result_sent.len()
    };
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
        skipped_count: result_skipped_duplicates.len(),
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
    let saturation_positive = lead_down_saturated && completion_lateness_ticks.is_some();
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
        sender_completed_qpc,
        sender_started_effective_ticks,
        completed_effective_ticks,
        completed_effective,
        sender_duration_us,
        requested_count,
        delivered_count,
        completion_error_ticks_value,
        authored_completion_error_ticks_value,
        completion_error_us,
        estimator_evidence,
        recovered_zero_progress,
        recovered_partial_up,
        recovered_retry_late,
        strict_completion_late,
        retry_late_abort,
        saturation_abort,
        saturation_streak,
    })
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
