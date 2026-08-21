use super::super::super::{PlaybackClockState, QpcTicks};
use super::super::{WorkerConfig, WorkerMetricsLocal, WorkerRuntime};
use super::DownBoundaryAdmission;
use super::observation::{DispatchObservation, ObserverLifecycle};
use super::{
    AuthoredBatchView, DispatchStep, PendingObservationQueue, PhysicalCommit, RecoveryDescriptor,
};
use sky_dispatch_core::coordinator::RuntimeDispatchCoordinator;
use sky_dispatch_win32::input::TrackedKeyState;

pub(super) fn record_rescue_admission(
    down_admission: DownBoundaryAdmission,
    admission: &super::authored::AdmissionOutcome,
    local_metrics: &mut WorkerMetricsLocal,
) {
    if !down_admission.is_late_rescue() {
        return;
    }
    match admission {
        super::authored::AdmissionOutcome::BlockedUnfocused
        | super::authored::AdmissionOutcome::FocusLost
        | super::authored::AdmissionOutcome::TargetChanged => {
            local_metrics.late_discovery_rescue_blocked_focus_or_target = local_metrics
                .late_discovery_rescue_blocked_focus_or_target
                .saturating_add(1);
        }
        super::authored::AdmissionOutcome::ControlRejected => {
            local_metrics.late_discovery_rescue_blocked_control = local_metrics
                .late_discovery_rescue_blocked_control
                .saturating_add(1);
        }
        super::authored::AdmissionOutcome::Allowed { .. }
        | super::authored::AdmissionOutcome::Guarded { .. } => {}
    }
}

pub(super) fn record_rescue_send(
    local_metrics: &mut WorkerMetricsLocal,
    down_admission: DownBoundaryAdmission,
    sender_cutoff: bool,
) {
    if !down_admission.is_late_rescue() {
        return;
    }
    if sender_cutoff {
        local_metrics.late_discovery_rescue_sender_cutoff_misses = local_metrics
            .late_discovery_rescue_sender_cutoff_misses
            .saturating_add(1);
    } else {
        local_metrics.late_discovery_rescue_sent =
            local_metrics.late_discovery_rescue_sent.saturating_add(1);
    }
}

#[derive(Clone, Copy)]
pub(super) enum DownMissReason {
    Backlog,
    HardLate,
}

fn record_last_missed_down_sample(
    local_metrics: &mut WorkerMetricsLocal,
    source_action_index: u32,
    down_mask: u16,
    physical_target_qpc: QpcTicks,
    observed_qpc: QpcTicks,
    reason: DownMissReason,
) {
    local_metrics.last_missed_down_valid = true;
    local_metrics.last_missed_down_reason_code = match reason {
        DownMissReason::Backlog => 1,
        DownMissReason::HardLate => 2,
    };
    local_metrics.last_missed_down_source_action_index = source_action_index;
    local_metrics.last_missed_down_mask = down_mask;
    local_metrics.last_missed_down_lateness_ticks = observed_qpc
        .checked_duration_since(physical_target_qpc)
        .map_or(0, |lateness| lateness.as_u64());
}

pub(super) fn record_missed_down_classification(
    local_metrics: &mut WorkerMetricsLocal,
    source_action_index: u32,
    down_mask: u16,
    physical_target_qpc: QpcTicks,
    observed_qpc: QpcTicks,
    reason: DownMissReason,
) {
    record_last_missed_down_sample(
        local_metrics,
        source_action_index,
        down_mask,
        physical_target_qpc,
        observed_qpc,
        reason,
    );
    local_metrics.missed_down_boundaries = local_metrics.missed_down_boundaries.saturating_add(1);
    local_metrics.missed_down_keys = local_metrics
        .missed_down_keys
        .saturating_add(u64::from(down_mask.count_ones()));
    match reason {
        DownMissReason::Backlog => {
            local_metrics.missed_backlog_boundaries =
                local_metrics.missed_backlog_boundaries.saturating_add(1);
        }
        DownMissReason::HardLate => {
            local_metrics.missed_hard_late_boundaries =
                local_metrics.missed_hard_late_boundaries.saturating_add(1);
        }
    }
    if let Ok(lateness) = observed_qpc.checked_duration_since(physical_target_qpc) {
        local_metrics.max_missed_lateness_ticks = local_metrics
            .max_missed_lateness_ticks
            .max(lateness.as_u64());
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn recover_missed_down_boundary(
    view: &AuthoredBatchView,
    config: &WorkerConfig,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    physical_target_qpc: QpcTicks,
    observed_qpc: QpcTicks,
    reason: DownMissReason,
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    record_missed_down_classification(
        local_metrics,
        view.batch_source_action_index,
        view.packet_masks.down_mask,
        physical_target_qpc,
        observed_qpc,
        reason,
    );
    if config.timing.strict_timing {
        return DispatchStep::TerminateStatic(match reason {
            DownMissReason::Backlog => "down_deadline_missed_before_send",
            DownMissReason::HardLate => "down_hard_late_abort",
        });
    }
    let up_mask = view.packet_masks.up_mask;
    let (started_qpc, _completed_qpc) = if up_mask == 0 {
        (observed_qpc, observed_qpc)
    } else {
        let RecoveryDescriptor::UpPrefix {
            up_len,
            up_mask: descriptor_up_mask,
        } = view.recovery
        else {
            return DispatchStep::TerminateStatic("missing_prepared_up_recovery_descriptor");
        };
        if descriptor_up_mask != up_mask || up_len != up_mask.count_ones() as u8 {
            return DispatchStep::TerminateStatic("invalid_prepared_up_recovery_descriptor");
        }
        let Some(prepared_up_packet) = view.prepared_packet.up_recovery_view() else {
            return DispatchStep::TerminateStatic("missing_prepared_up_recovery_view");
        };
        if prepared_up_packet.packet() != sky_dispatch_win32::input::PhysicalPacket::new(up_mask, 0)
            || prepared_up_packet.packet().event_count() != up_len
        {
            return DispatchStep::TerminateStatic("invalid_prepared_up_recovery_view");
        };
        #[cfg(any(test, feature = "test-support"))]
        let result = backend.send_prepared_physical_packet_view_with_start_and_cutoff(
            prepared_up_packet,
            observed_qpc,
            None,
        );
        #[cfg(not(any(test, feature = "test-support")))]
        let result =
            backend.send_prepared_physical_packet_view_with_cutoff(prepared_up_packet, None);
        if backend.timing_error.take().is_some() {
            return DispatchStep::TerminateStatic("QPC failure during missed Down Up recovery");
        }
        if !result.is_success()
            || result.evidence.confirmed_mask != up_mask
            || result.evidence.skipped_mask != 0
        {
            return DispatchStep::TerminateStatic("missed Down safety Up transport failure");
        }
        let Some(started) = result.evidence.started_ticks else {
            return DispatchStep::TerminateStatic("missed Down safety Up missing start boundary");
        };
        let Some(completed) = result.evidence.completed_ticks else {
            return DispatchStep::TerminateStatic(
                "missed Down safety Up missing completion boundary",
            );
        };
        runtime
            .production_forensics
            .observe_lifecycle(ObserverLifecycle::RecoveryUp { up_mask });
        if let Some(observer) = observer {
            observer.push(
                DispatchObservation::Lifecycle(ObserverLifecycle::RecoveryUp { up_mask }),
                &mut local_metrics.observer_dropped_samples,
                &mut local_metrics.observer_queue_high_watermark,
            );
        }
        (started, completed)
    };
    let started_effective = match clock_state
        .get_elapsed_allow_pre_epoch(started_qpc, runtime.allow_pre_epoch_startup_dispatch)
    {
        Ok(ticks) => ticks,
        Err(error) => {
            return DispatchStep::Terminate(format!(
                "playback clock failure during missed Down recovery: {error}"
            ));
        }
    };
    let commit_result = match &view.commit {
        PhysicalCommit::Authored(commit) => coordinator
            .commit_prepared_authored_frame_deadline_miss(
                commit,
                up_mask,
                view.packet_masks.down_mask,
                started_effective,
            ),
        PhysicalCommit::Coalesced {
            authored,
            release_mask,
            due_ticks,
        } => {
            if started_effective < *due_ticks {
                return DispatchStep::TerminateStatic(
                    "coalesced missed Down recovery started before Up due boundary",
                );
            }
            coordinator
                .commit_pending_release_success(*release_mask, started_effective)
                .and_then(|()| {
                    coordinator.commit_prepared_authored_frame_deadline_miss(
                        authored,
                        authored.frame.immediate_up_mask,
                        authored.frame.down_mask,
                        started_effective,
                    )
                })
        }
        PhysicalCommit::PendingRelease { .. } => {
            return DispatchStep::TerminateStatic(
                "pending release cannot carry a missed authored Down",
            );
        }
    };
    if let Err(error) = commit_result {
        return DispatchStep::Terminate(format!("coordinator missed Down commit failure: {error}"));
    }

    backend.last_error = None;
    runtime.last_dispatch_was_missed_down = true;
    runtime.mark_down_boundary_missed();
    DispatchStep::Dispatched
}

#[cfg(test)]
mod tests {
    use super::{
        DownMissReason, record_last_missed_down_sample, record_missed_down_classification,
    };
    use crate::engine::telemetry::WorkerMetricsLocal;
    use sky_dispatch_win32::clock::QpcTicks;

    #[test]
    fn last_missed_down_sample_records_backlog_evidence() {
        let mut metrics = WorkerMetricsLocal::default();

        record_last_missed_down_sample(
            &mut metrics,
            41,
            0b101,
            QpcTicks::from_raw(1_000),
            QpcTicks::from_raw(1_250),
            DownMissReason::Backlog,
        );

        assert!(metrics.last_missed_down_valid);
        assert_eq!(metrics.last_missed_down_reason_code, 1);
        assert_eq!(metrics.last_missed_down_source_action_index, 41);
        assert_eq!(metrics.last_missed_down_mask, 0b101);
        assert_eq!(metrics.last_missed_down_lateness_ticks, 250);
    }

    #[test]
    fn last_missed_down_sample_records_hard_late_evidence() {
        let mut metrics = WorkerMetricsLocal::default();

        record_last_missed_down_sample(
            &mut metrics,
            7,
            0b010,
            QpcTicks::from_raw(10_000),
            QpcTicks::from_raw(10_005),
            DownMissReason::HardLate,
        );

        assert!(metrics.last_missed_down_valid);
        assert_eq!(metrics.last_missed_down_reason_code, 2);
        assert_eq!(metrics.last_missed_down_source_action_index, 7);
        assert_eq!(metrics.last_missed_down_mask, 0b010);
        assert_eq!(metrics.last_missed_down_lateness_ticks, 5);
    }

    #[test]
    fn classified_miss_keeps_last_sample_and_counters_consistent() {
        let mut metrics = WorkerMetricsLocal::default();

        record_missed_down_classification(
            &mut metrics,
            9,
            0b101,
            QpcTicks::from_raw(2_000),
            QpcTicks::from_raw(2_007),
            DownMissReason::HardLate,
        );

        assert!(metrics.last_missed_down_valid);
        assert_eq!(metrics.missed_down_boundaries, 1);
        assert_eq!(metrics.missed_hard_late_boundaries, 1);
        assert_eq!(metrics.missed_backlog_boundaries, 0);
        assert_eq!(metrics.last_missed_down_lateness_ticks, 7);
    }
}
