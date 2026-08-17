//! Pure timing projection for the authored note-on path: resolves the QPC
//! evidence into typed timeline values, flags, and deferred observation data used by
//! telemetry and the terminal SLO decision.
//!
//! Structural ownership: this module must not import `SharedMetrics`,
//! `TelemetryCollector`, `Mutex`, or any Python type.  It stays pure and
//! unit-testable (no wall-clock beyond the QPC samples already captured by the
//! caller into `result_started_ticks` / `result_completed_ticks`).

use super::super::super::{
    ActionKind, DurationTicks, PlaybackClockState, QpcClock, QpcTicks, RuntimeDispatchCoordinator,
    TimelineTicks,
};
use super::super::{
    DispatchPath, DispatchPreparationProbe, WorkerConfig, WorkerHealthState, WorkerRuntime,
    WorkerTimingState, signed_timeline_delta_ticks,
};
use super::{AuthoredBatchView, BatchViewResult, DispatchStep, PhysicalCommit};
use sky_dispatch_core::coordinator::{PreparedAuthoredFrame, PreparedBatch};
use sky_dispatch_win32::input::{PacketRetryReason, PhysicalPacket, SendTransactionStatus};

/// Typed transport/timing evidence shared by DownOnly, Mixed, and UpOnly
/// dispatch observations.  The observer applies the one canonical predicate
/// after the hard dispatch-ready boundary; dispatch code only constructs this
/// immutable evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DispatchObservationEvidence {
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

/// Canonical clean dispatch eligibility. Every transport direction uses
/// this exact predicate, including Mixed, with no weaker UpOnly shortcut.
pub fn is_clean_dispatch_observation(evidence: DispatchObservationEvidence) -> bool {
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
    packet_masks: PhysicalPacket,
    _dispatch_path: DispatchPath,
    result_success: bool,
    result_confirmed_mask: u16,
) -> (usize, usize) {
    let requested_count = usize::from(packet_masks.event_count());
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

#[cfg(any(test, feature = "test-support"))]
#[allow(dead_code)]
pub(crate) fn prepare_authored_batch_view(
    coordinator: &RuntimeDispatchCoordinator,
    prepared_batch: PreparedBatch,
    preparation_probe: &DispatchPreparationProbe,
) -> BatchViewResult {
    let batch_index = prepared_batch.index;
    let batch_scheduled_ticks = prepared_batch.effective_scheduled_ticks;
    // Every authored batch reaching this view is physical work. Stale
    // unmatched-Up metadata uses the dedicated coordinator path and must not
    // acquire a physical DispatchPath.
    let packet_kind = prepared_batch.packet_kind;
    let (
        batch_kind,
        dispatch_path,
        batch_source_action_index,
        batch_intent_count,
        conflict_mask,
        packet_masks,
    ) = {
        preparation_probe.record_packet_view();
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
        preparation_probe.record_conflict();
        let conflict_mask =
            coordinator.check_packet_down_conflicts(packet_view.up_mask(), packet_view.down_mask());
        let down_source_action_index = packet_view.header.down_source_action_index;
        let batch_source_action_index = down_source_action_index
            .or_else(|| {
                coordinator
                    .schedule
                    .batches
                    .get(packet_view.header.first_batch_index as usize)
                    .map(|batch| batch.source_action_index)
            })
            .unwrap_or(0);
        let up_count = packet_view.up_mask().count_ones() as usize;
        let down_count = packet_view.down_mask().count_ones() as usize;
        let dispatch_path = match packet_kind {
            sky_dispatch_core::model::PhysicalPacketKind::UpOnly => {
                DispatchPath::UpOnly { up_count }
            }
            sky_dispatch_core::model::PhysicalPacketKind::DownOnly => {
                DispatchPath::DownOnly { down_count }
            }
            sky_dispatch_core::model::PhysicalPacketKind::Mixed => DispatchPath::Mixed {
                up_count,
                down_count,
            },
        };
        (
            if matches!(dispatch_path, DispatchPath::UpOnly { .. }) {
                ActionKind::Up
            } else {
                ActionKind::Down
            },
            dispatch_path,
            batch_source_action_index,
            up_count + down_count,
            conflict_mask,
            Some(sky_dispatch_win32::input::PhysicalPacket::new(
                packet_view.up_mask(),
                packet_view.down_mask(),
            )),
        )
    };
    preparation_probe.record_input_build();
    let packet_masks = match packet_masks {
        Some(value) => value,
        None => {
            return Err(DispatchStep::Terminate(
                "physical packet preparation received an empty packet".to_string(),
            ));
        }
    };
    let prepared_packet =
        match sky_dispatch_win32::input::PreparedPhysicalPacket::try_new(packet_masks) {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "physical packet preparation failure: {error}"
                )));
            }
        };
    let authored_batch_scheduled_ticks = coordinator.batch_scheduled_ticks[batch_index];
    let authored_ticks = coordinator
        .effective_batch_scheduled_ticks(batch_index)
        .map_err(|error| {
            DispatchStep::Terminate(format!("authored frame timing failure: {error}"))
        })?;
    let prepared = PreparedAuthoredFrame {
        first_batch_index: prepared_batch.index,
        packet_index: prepared_batch.packet_index,
        packet_batch_count: prepared_batch.packet_batch_count,
        authored_ticks,
        immediate_up_mask: packet_masks.up_mask,
        deferred_up_mask: 0,
        down_mask: packet_masks.down_mask,
        stale_up_count: 0,
    };
    let commit = coordinator
        .prepare_authored_commit(prepared)
        .map_err(|error| {
            DispatchStep::Terminate(format!("authored commit preparation failure: {error}"))
        })?;
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
        prepared_packet,
        commit: PhysicalCommit::Authored(commit),
    }))
}

/// Build a physical view from the coordinator's per-key frame classification.
/// Deferred unrelated releases are intentionally absent from the packet.
pub(crate) fn prepare_authored_frame_view(
    coordinator: &RuntimeDispatchCoordinator,
    frame: PreparedAuthoredFrame,
    preparation_probe: &DispatchPreparationProbe,
) -> BatchViewResult {
    prepare_authored_frame_view_with_pending(
        coordinator,
        frame,
        0,
        frame.authored_ticks,
        preparation_probe,
    )
}

pub(crate) fn prepare_authored_frame_view_with_pending(
    coordinator: &RuntimeDispatchCoordinator,
    frame: PreparedAuthoredFrame,
    pending_release_mask: u16,
    pending_due_ticks: TimelineTicks,
    preparation_probe: &DispatchPreparationProbe,
) -> BatchViewResult {
    let packet = coordinator
        .schedule
        .view_packet_ticks(frame.packet_index, frame.authored_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("runtime packet view failure: {error}"))
        })?;
    let selected_up_mask = frame.immediate_up_mask | pending_release_mask;
    let selected_down_mask = frame.down_mask;
    let selected_packet = PhysicalPacket::new(selected_up_mask, selected_down_mask);
    let packet_kind =
        sky_dispatch_core::coordinator::physical_packet_kind(selected_up_mask, selected_down_mask)
            .map_err(|error| {
                DispatchStep::Terminate(format!("physical packet kind failure: {error}"))
            })?;
    preparation_probe.record_packet_view();
    preparation_probe.record_conflict();
    let conflict_mask =
        coordinator.check_packet_down_conflicts(selected_up_mask, selected_down_mask);
    let down_source_action_index = packet.header.down_source_action_index;
    let batch_source_action_index = down_source_action_index
        .or_else(|| {
            coordinator
                .schedule
                .batches
                .get(packet.header.first_batch_index as usize)
                .map(|batch| batch.source_action_index)
        })
        .unwrap_or(0);
    preparation_probe.record_input_build();
    let prepared_packet = sky_dispatch_win32::input::PreparedPhysicalPacket::try_new(
        selected_packet,
    )
    .map_err(|error| {
        DispatchStep::Terminate(format!("physical packet preparation failure: {error}"))
    })?;
    let up_count = selected_up_mask.count_ones() as usize;
    let down_count = selected_down_mask.count_ones() as usize;
    let dispatch_path = match packet_kind {
        sky_dispatch_core::model::PhysicalPacketKind::UpOnly => DispatchPath::UpOnly { up_count },
        sky_dispatch_core::model::PhysicalPacketKind::DownOnly => {
            DispatchPath::DownOnly { down_count }
        }
        sky_dispatch_core::model::PhysicalPacketKind::Mixed => DispatchPath::Mixed {
            up_count,
            down_count,
        },
    };
    let prepared_batch = PreparedBatch {
        index: frame.first_batch_index,
        effective_scheduled_ticks: frame.authored_ticks,
        packet_index: frame.packet_index,
        packet_batch_count: frame.packet_batch_count,
        packet_kind,
    };
    let authored_commit = coordinator
        .prepare_authored_commit(frame)
        .map_err(|error| {
            DispatchStep::Terminate(format!("authored commit preparation failure: {error}"))
        })?;
    Ok(Some(AuthoredBatchView {
        prepared_batch,
        batch_source_action_index,
        batch_intent_count: up_count + down_count,
        batch_kind: if down_count != 0 {
            ActionKind::Down
        } else {
            ActionKind::Up
        },
        batch_scheduled_ticks: frame.authored_ticks,
        authored_batch_scheduled_ticks: frame.authored_ticks,
        conflict_mask,
        dispatch_path,
        packet_masks: selected_packet,
        prepared_packet,
        commit: if pending_release_mask == 0 {
            PhysicalCommit::Authored(authored_commit)
        } else {
            PhysicalCommit::Coalesced {
                authored: authored_commit,
                release_mask: pending_release_mask,
                due_ticks: pending_due_ticks,
            }
        },
    }))
}

pub(crate) fn prepare_pending_release_view(
    coordinator: &RuntimeDispatchCoordinator,
    release_mask: u16,
    due_ticks: TimelineTicks,
    preparation_probe: &DispatchPreparationProbe,
) -> BatchViewResult {
    if release_mask == 0 {
        return Err(DispatchStep::Terminate(
            "pending release view has an empty mask".to_string(),
        ));
    }
    let packet = PhysicalPacket::new(release_mask, 0);
    let prepared_packet = sky_dispatch_win32::input::PreparedPhysicalPacket::try_new(packet)
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "pending release packet preparation failure: {error}"
            ))
        })?;
    preparation_probe.record_input_build();
    let count = release_mask.count_ones() as usize;
    let source_action_index = coordinator
        .pending_release_source_action_index(release_mask)
        .unwrap_or(0);
    let prepared_batch = PreparedBatch {
        index: 0,
        effective_scheduled_ticks: due_ticks,
        packet_index: 0,
        packet_batch_count: 1,
        packet_kind: sky_dispatch_core::model::PhysicalPacketKind::UpOnly,
    };
    Ok(Some(AuthoredBatchView {
        prepared_batch,
        batch_source_action_index: source_action_index,
        batch_intent_count: count,
        batch_kind: ActionKind::Up,
        batch_scheduled_ticks: due_ticks,
        authored_batch_scheduled_ticks: due_ticks,
        conflict_mask: 0,
        dispatch_path: DispatchPath::UpOnly { up_count: count },
        packet_masks: packet,
        prepared_packet,
        commit: PhysicalCommit::PendingRelease {
            release_mask,
            due_ticks,
        },
    }))
}

/// Timing-derived evidence captured from the note-on SendInput call.
pub(crate) struct DownSendTiming {
    pub(crate) final_admission_qpc: QpcTicks,
    pub(crate) sendinput_completed_qpc: QpcTicks,
    pub(crate) final_admission_effective_ticks: TimelineTicks,
    pub(crate) completed_effective_ticks: TimelineTicks,
    pub(crate) admission_to_completion_ticks: DurationTicks,
    pub(crate) completion_error_ticks_value: i64,
    pub(crate) recovered_partial_up: bool,
    pub(crate) recovered_retry_late: bool,
    pub(crate) strict_completion_late: bool,
    pub(crate) retry_late_abort: bool,
}

/// Resolves the QPC evidence, commits the prepared batch, and computes the
/// boundary values shared by the SLO flags and telemetry.
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
    let final_admission_ticks = match result_started_ticks {
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
    let admission_to_completion_ticks =
        match completed_qpc_ticks.checked_duration_since(final_admission_ticks) {
            Ok(duration) => duration,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "note-on QPC ordering failure: {error}"
                )));
            }
        };
    let final_admission_effective_ticks = match clock_state.get_elapsed_allow_pre_epoch(
        final_admission_ticks,
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
    let commit_result = match &view.commit {
        PhysicalCommit::Authored(commit) => coordinator
            .commit_prepared_authored_frame_success_frozen(
                commit,
                final_admission_effective_ticks,
                completed_effective_ticks,
            ),
        PhysicalCommit::PendingRelease {
            release_mask,
            due_ticks,
        } => {
            if final_admission_effective_ticks < *due_ticks {
                return Err(DispatchStep::Terminate(
                    "pending release started before its due boundary".to_string(),
                ));
            }
            coordinator
                .commit_pending_release_success(*release_mask, final_admission_effective_ticks)
        }
        PhysicalCommit::Coalesced {
            authored,
            release_mask,
            due_ticks,
        } => {
            if final_admission_effective_ticks < *due_ticks {
                return Err(DispatchStep::Terminate(
                    "coalesced pending release started before its due boundary".to_string(),
                ));
            }
            coordinator
                .commit_pending_release_success(*release_mask, final_admission_effective_ticks)
                .and_then(|_| {
                    coordinator.commit_prepared_authored_frame_success_frozen(
                        authored,
                        final_admission_effective_ticks,
                        completed_effective_ticks,
                    )
                })
        }
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
        final_admission_effective_ticks,
        completed_effective_ticks,
        admission_to_completion_ticks,
        completion_lateness_ticks,
    ))
}

/// Resolves the QPC evidence, commits the prepared batch, computes timing
/// SLO flags. Mutates `coordinator` (commit) and `runtime` (last-send
/// boundary), but does not mutate health or record telemetry.
#[allow(clippy::too_many_arguments)]
pub(crate) fn interpret_down_send_timing(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    clock_state: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    _qpc_clock: QpcClock,
    physical_target_qpc: QpcTicks,
    coordinator: &mut RuntimeDispatchCoordinator,
    _health: &WorkerHealthState,
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
        final_admission_effective_ticks,
        completed_effective_ticks,
        admission_to_completion_ticks,
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
    let final_admission_qpc = result_started_ticks.unwrap_or(QpcTicks::ZERO);
    let sendinput_completed_qpc = result_completed_ticks.unwrap_or(QpcTicks::ZERO);
    let completion_error_ticks_value =
        match signed_timeline_delta_ticks(completed_effective_ticks, view.batch_scheduled_ticks) {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "note-on timing conversion failure: {error}"
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
    let observation_evidence = DispatchObservationEvidence {
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
    let clean_dispatch_sample = is_clean_dispatch_observation(observation_evidence);
    let strict_completion_late = config.timing.strict_timing
        && clean_dispatch_sample
        && completion_lateness_ticks.is_some_and(|late| {
            late > match view.dispatch_path {
                DispatchPath::UpOnly { .. } => timing.strict_up_completion_late_ticks,
                DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
                    timing.strict_down_completion_late_ticks
                }
            }
        });
    Ok(DownSendTiming {
        final_admission_qpc,
        sendinput_completed_qpc,
        final_admission_effective_ticks,
        completed_effective_ticks,
        admission_to_completion_ticks,
        completion_error_ticks_value,
        recovered_partial_up,
        recovered_retry_late,
        strict_completion_late,
        retry_late_abort,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        DispatchObservationEvidence, is_clean_dispatch_observation, physical_event_counts,
    };
    use crate::engine::worker::DispatchPath;
    use sky_dispatch_win32::input::{PacketRetryReason, PhysicalPacket, SendTransactionStatus};

    #[test]
    fn mixed_retrigger_retains_directional_event_cardinality() {
        let same_key = PhysicalPacket::new(0b001, 0b001);
        let three_events = PhysicalPacket::new(0b001, 0b011);

        assert_eq!(same_key.event_count(), 2);
        assert_eq!(three_events.event_count(), 3);
        assert_eq!(
            physical_event_counts(
                same_key,
                DispatchPath::Mixed {
                    up_count: 1,
                    down_count: 1,
                },
                true,
                0b001,
            ),
            (2, 2)
        );
        assert!(is_clean_dispatch_observation(DispatchObservationEvidence {
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
        }));
    }
}
