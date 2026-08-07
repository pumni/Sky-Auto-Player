//! Pure timing projection for the authored note-on path: resolves the QPC
//! evidence into typed timeline values, flags, and bookkeeping markers used by
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
use super::super::{
    DispatchPath, WorkerConfig, WorkerHealthState, WorkerMetricsLocal, WorkerRuntime,
    WorkerTimingState, estimator_kind_for_path, signed_ticks_to_us, signed_timeline_delta_ticks,
};
use super::{AuthoredBatchView, BatchViewResult, DispatchStep};
use sky_dispatch_core::coordinator::PreparedBatch;
use sky_dispatch_win32::input::PacketRetryReason;
use smallvec::SmallVec;

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
    pub(crate) estimator_kind: Option<ActionKind>,
    pub(crate) clean_directional_sample: bool,
    pub(crate) recovered_partial_up: bool,
    pub(crate) recovered_retry_late: bool,
    pub(crate) strict_completion_late: bool,
    pub(crate) retry_late_abort: bool,
    pub(crate) saturation_abort: bool,
    pub(crate) bookkeeping_completed_us: u64,
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
/// SLO flags, the saturation-abort streak, and the bookkeeping completion
/// marker.  Mutates `coordinator` (commit) and `health` (saturation streak).
/// Does not record telemetry or call the estimator.
#[allow(clippy::too_many_arguments)]
pub(crate) fn interpret_down_send_timing(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    clock_state: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    qpc_clock: QpcClock,
    coordinator: &mut RuntimeDispatchCoordinator,
    health: &mut WorkerHealthState,
    timing: &WorkerTimingState,
    local_metrics: &mut WorkerMetricsLocal,
    result_success: bool,
    result_started_ticks: Option<QpcTicks>,
    result_completed_ticks: Option<QpcTicks>,
    result_sent: &SmallVec<[u16; 15]>,
    result_skipped_duplicates: &SmallVec<[u16; 15]>,
    result_send_attempts: u8,
    result_retry_reason: PacketRetryReason,
    result_chord_integrity_lost: bool,
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
    let estimator_kind = estimator_kind_for_path(view.dispatch_path);
    let clean_directional_sample = result_success
        && result_skipped_duplicates.is_empty()
        && result_send_attempts == 1
        && !result_chord_integrity_lost
        && !matches!(view.dispatch_path, DispatchPath::Mixed { .. })
        && estimator_kind.is_some()
        && delivered_count == requested_count;
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
    let strict_completion_late = config.timing.strict_timing
        && clean_directional_sample
        && completion_lateness_ticks.is_some_and(|late| {
            late > match view.dispatch_path {
                DispatchPath::UpOnly { .. } => timing.strict_up_completion_late_ticks,
                DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
                    timing.strict_down_completion_late_ticks
                }
            }
        });
    if recovered_retry_late {
        local_metrics.recovered_zero_progress_but_late = local_metrics
            .recovered_zero_progress_but_late
            .saturating_add(1);
    }
    let saturation_abort = match view.dispatch_path {
        DispatchPath::UpOnly { .. } => {
            health.down_saturation_positive_streak = 0;
            health.up_saturation_positive_streak =
                if lead_down_saturated && completion_lateness_ticks.is_some() {
                    health.up_saturation_positive_streak.saturating_add(1)
                } else {
                    0
                };
            config.timing.strict_timing
                && health.up_saturation_positive_streak >= STRICT_SATURATION_ABORT_STREAK
        }
        DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
            health.up_saturation_positive_streak = 0;
            health.down_saturation_positive_streak =
                if lead_down_saturated && completion_lateness_ticks.is_some() {
                    health.down_saturation_positive_streak.saturating_add(1)
                } else {
                    0
                };
            config.timing.strict_timing
                && health.down_saturation_positive_streak >= STRICT_SATURATION_ABORT_STREAK
        }
    };
    let bookkeeping_completed_us = read_qpc_us(qpc_clock, clock_state)?;
    if recovered_zero_progress && result_success {
        local_metrics.recovered_zero_progress_retries = local_metrics
            .recovered_zero_progress_retries
            .saturating_add(1);
    }
    if recovered_partial_up {
        local_metrics.recovered_partial_up_retries =
            local_metrics.recovered_partial_up_retries.saturating_add(1);
    }
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
        estimator_kind,
        clean_directional_sample,
        recovered_partial_up,
        recovered_retry_late,
        strict_completion_late,
        retry_late_abort,
        saturation_abort,
        bookkeeping_completed_us,
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
