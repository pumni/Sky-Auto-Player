use super::super::super::{PlaybackClockState, QpcTicks};
use super::super::physical_timing_guard::PhysicalTimingWindow;
use super::super::{WorkerConfig, WorkerMetricsLocal, WorkerRuntime};
use super::observation::{
    DispatchObservation, DownMissKind, DownMissObservation, ObserverLifecycle,
};
use super::{
    AuthoredBatchView, DispatchStep, PendingObservationQueue, PhysicalCommit, RecoveryDescriptor,
};
use sky_dispatch_core::coordinator::RuntimeDispatchCoordinator;
use sky_dispatch_win32::input::TrackedKeyState;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DownMissReason {
    UnobservedBacklog,
    PhysicalWindowExpired,
    DownExpiredBeforeSend,
}

pub(super) fn queue_down_miss_observation(
    view: &AuthoredBatchView,
    local_metrics: &mut WorkerMetricsLocal,
    observer: Option<&PendingObservationQueue>,
    wake_ticks: sky_dispatch_core::time::TimelineTicks,
    physical_timing_window: PhysicalTimingWindow,
    observed_qpc: QpcTicks,
    reason: DownMissReason,
) {
    let Some(observer) = observer else {
        return;
    };
    observer.push(
        DispatchObservation::DownMiss(DownMissObservation {
            source_action_index: view.batch_source_action_index,
            compiled_packet_index: u64::try_from(view.prepared_batch.packet_index).ok(),
            authored_ticks: view.authored_batch_scheduled_ticks,
            effective_deadline_ticks: view.batch_scheduled_ticks,
            wake_ticks,
            physical_timing_window,
            observed_qpc,
            up_mask: view.packet_masks.up_mask,
            down_mask: view.packet_masks.down_mask,
            kind: match reason {
                DownMissReason::UnobservedBacklog => DownMissKind::UnobservedBacklog,
                DownMissReason::PhysicalWindowExpired => DownMissKind::PhysicalWindowExpired,
                DownMissReason::DownExpiredBeforeSend => DownMissKind::DownExpiredBeforeSend,
            },
        }),
        &mut local_metrics.observer_dropped_samples,
        &mut local_metrics.observer_queue_high_watermark,
    );
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
        DownMissReason::UnobservedBacklog => 1,
        DownMissReason::PhysicalWindowExpired => 2,
        DownMissReason::DownExpiredBeforeSend => 3,
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
        DownMissReason::UnobservedBacklog => {
            local_metrics.missed_unobserved_backlog_boundaries = local_metrics
                .missed_unobserved_backlog_boundaries
                .saturating_add(1);
        }
        DownMissReason::PhysicalWindowExpired => {
            local_metrics.missed_physical_window_boundaries = local_metrics
                .missed_physical_window_boundaries
                .saturating_add(1);
        }
        DownMissReason::DownExpiredBeforeSend => {
            local_metrics.final_sender_window_expirations = local_metrics
                .final_sender_window_expirations
                .saturating_add(1);
        }
    }
    if let Ok(lateness) = observed_qpc.checked_duration_since(physical_target_qpc) {
        local_metrics.max_missed_lateness_ticks = local_metrics
            .max_missed_lateness_ticks
            .max(lateness.as_u64());
    }
}

pub(crate) fn classify_missed_down_boundary(
    view: &AuthoredBatchView,
    local_metrics: &mut WorkerMetricsLocal,
    observer: Option<&PendingObservationQueue>,
    wake_ticks: sky_dispatch_core::time::TimelineTicks,
    physical_timing_window: PhysicalTimingWindow,
    observed_qpc: QpcTicks,
    reason: DownMissReason,
) {
    queue_down_miss_observation(
        view,
        local_metrics,
        observer,
        wake_ticks,
        physical_timing_window,
        observed_qpc,
        reason,
    );
    record_physical_floor_delays(local_metrics, physical_timing_window);
    record_release_floor_infeasibility(local_metrics, physical_timing_window, reason);
    record_missed_down_classification(
        local_metrics,
        view.batch_source_action_index,
        view.packet_masks.down_mask,
        physical_timing_window.authored_target_qpc,
        observed_qpc,
        reason,
    );
}

pub(crate) fn record_physical_floor_delays(
    local_metrics: &mut WorkerMetricsLocal,
    window: PhysicalTimingWindow,
) {
    if window.hold_floor_mask != 0 {
        let delay = window
            .musical_up_not_before_qpc
            .as_u64()
            .saturating_sub(window.authored_target_qpc.as_u64());
        if delay != 0 {
            local_metrics.hold_floor_delay_boundaries =
                local_metrics.hold_floor_delay_boundaries.saturating_add(1);
            local_metrics.max_hold_floor_delay_ticks =
                local_metrics.max_hold_floor_delay_ticks.max(delay);
            local_metrics.last_hold_floor_delay_mask = window.hold_floor_mask;
            local_metrics.last_hold_floor_authored_target_qpc_ticks =
                window.authored_target_qpc.as_u64();
            local_metrics.last_hold_floor_not_before_qpc_ticks =
                window.musical_up_not_before_qpc.as_u64();
        }
    }
    if window.release_floor_mask != 0 {
        let delay = window
            .down_not_before_qpc
            .as_u64()
            .saturating_sub(window.authored_target_qpc.as_u64());
        if delay != 0 {
            local_metrics.release_floor_delay_boundaries = local_metrics
                .release_floor_delay_boundaries
                .saturating_add(1);
            local_metrics.max_release_floor_delay_ticks =
                local_metrics.max_release_floor_delay_ticks.max(delay);
            local_metrics.last_release_floor_delay_mask = window.release_floor_mask;
            local_metrics.last_release_floor_authored_target_qpc_ticks =
                window.authored_target_qpc.as_u64();
            local_metrics.last_release_floor_not_before_qpc_ticks =
                window.down_not_before_qpc.as_u64();
        }
    }
}

fn record_release_floor_infeasibility(
    local_metrics: &mut WorkerMetricsLocal,
    window: PhysicalTimingWindow,
    reason: DownMissReason,
) {
    if reason == DownMissReason::PhysicalWindowExpired
        && window.release_floor_mask != 0
        && window
            .latest_down_start_qpc
            .is_some_and(|latest| window.down_not_before_qpc > latest)
    {
        local_metrics.release_floor_infeasible_boundaries = local_metrics
            .release_floor_infeasible_boundaries
            .saturating_add(1);
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
    physical_timing_window: PhysicalTimingWindow,
    observed_qpc: QpcTicks,
    wake_ticks: sky_dispatch_core::time::TimelineTicks,
    reason: DownMissReason,
    already_classified: bool,
    observer: Option<&PendingObservationQueue>,
) -> DispatchStep {
    if !already_classified {
        classify_missed_down_boundary(
            view,
            local_metrics,
            observer,
            wake_ticks,
            physical_timing_window,
            observed_qpc,
            reason,
        );
    }
    if config.timing.strict_timing {
        return DispatchStep::TerminateStatic(match reason {
            DownMissReason::UnobservedBacklog => "down_unobserved_backlog",
            DownMissReason::PhysicalWindowExpired => "down_physical_window_expired",
            DownMissReason::DownExpiredBeforeSend => "down_final_sender_window_expired",
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
            if result.evidence.attempts != 0
                && let Some(guard) = runtime.physical_timing_guard.as_mut()
            {
                guard.invalidate();
            }
            return DispatchStep::TerminateStatic("QPC failure during missed Down Up recovery");
        }
        if !result.is_success()
            || result.evidence.confirmed_mask != up_mask
            || result.evidence.skipped_mask != 0
        {
            if result.evidence.attempts != 0
                && let Some(guard) = runtime.physical_timing_guard.as_mut()
            {
                guard.invalidate();
            }
            return DispatchStep::TerminateStatic(
                "missed Down Up-prefix recovery transport failure",
            );
        }
        let Some(started) = result.evidence.started_ticks else {
            return DispatchStep::TerminateStatic("missed Down safety Up missing start boundary");
        };
        let Some(completed) = result.evidence.completed_ticks else {
            if let Some(guard) = runtime.physical_timing_guard.as_mut() {
                guard.invalidate();
            }
            return DispatchStep::TerminateStatic(
                "missed Down Up-prefix recovery missing completion boundary",
            );
        };
        let Some(guard) = runtime.physical_timing_guard.as_mut() else {
            return DispatchStep::TerminateStatic("physical timing guard is not initialized");
        };
        if let Err(error) = guard.observe_successful_packet(completed, up_mask, 0) {
            return DispatchStep::Terminate(format!(
                "physical timing guard recovery update failed: {error:?}"
            ));
        }
        runtime
            .production_forensics
            .observe_lifecycle(ObserverLifecycle::RecoveryUp { up_mask });
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
    DispatchStep::Dispatched
}

#[cfg(test)]
mod tests {
    use super::{
        DownMissReason, record_last_missed_down_sample, record_missed_down_classification,
        record_physical_floor_delays, record_release_floor_infeasibility,
    };
    use crate::engine::telemetry::WorkerMetricsLocal;
    use crate::engine::worker::physical_timing_guard::PhysicalTimingWindow;
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
            DownMissReason::UnobservedBacklog,
        );

        assert!(metrics.last_missed_down_valid);
        assert_eq!(metrics.last_missed_down_reason_code, 1);
        assert_eq!(metrics.last_missed_down_source_action_index, 41);
        assert_eq!(metrics.last_missed_down_mask, 0b101);
        assert_eq!(metrics.last_missed_down_lateness_ticks, 250);
    }

    #[test]
    fn last_missed_down_sample_records_physical_window_expiration() {
        let mut metrics = WorkerMetricsLocal::default();

        record_last_missed_down_sample(
            &mut metrics,
            7,
            0b010,
            QpcTicks::from_raw(10_000),
            QpcTicks::from_raw(10_005),
            DownMissReason::PhysicalWindowExpired,
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
            DownMissReason::PhysicalWindowExpired,
        );

        assert!(metrics.last_missed_down_valid);
        assert_eq!(metrics.missed_down_boundaries, 1);
        assert_eq!(metrics.missed_physical_window_boundaries, 1);
        assert_eq!(metrics.missed_unobserved_backlog_boundaries, 0);
        assert_eq!(metrics.final_sender_window_expirations, 0);
        assert_eq!(metrics.last_missed_down_lateness_ticks, 7);
    }

    #[test]
    fn every_primary_miss_reason_is_mutually_exclusive_and_counts_once() {
        for reason in [
            DownMissReason::UnobservedBacklog,
            DownMissReason::PhysicalWindowExpired,
            DownMissReason::DownExpiredBeforeSend,
        ] {
            let mut metrics = WorkerMetricsLocal::default();
            record_missed_down_classification(
                &mut metrics,
                9,
                0b101,
                QpcTicks::from_raw(2_000),
                QpcTicks::from_raw(2_007),
                reason,
            );

            assert_eq!(metrics.missed_down_boundaries, 1);
            assert_eq!(metrics.missed_down_keys, 2);
            assert_eq!(
                metrics.missed_unobserved_backlog_boundaries
                    + metrics.missed_physical_window_boundaries
                    + metrics.final_sender_window_expirations,
                1
            );
        }
    }

    #[test]
    fn physical_floor_delay_evidence_keeps_independent_masks_and_timestamps() {
        let mut metrics = WorkerMetricsLocal::default();
        let window = PhysicalTimingWindow {
            authored_target_qpc: QpcTicks::from_raw(100),
            musical_up_not_before_qpc: QpcTicks::from_raw(140),
            down_not_before_qpc: QpcTicks::from_raw(130),
            packet_not_before_qpc: QpcTicks::from_raw(140),
            latest_down_start_qpc: Some(QpcTicks::from_raw(150)),
            hold_floor_mask: 1 << 2,
            release_floor_mask: 1 << 4,
        };

        record_physical_floor_delays(&mut metrics, window);

        assert_eq!(metrics.hold_floor_delay_boundaries, 1);
        assert_eq!(metrics.max_hold_floor_delay_ticks, 40);
        assert_eq!(metrics.last_hold_floor_delay_mask, 1 << 2);
        assert_eq!(metrics.last_hold_floor_authored_target_qpc_ticks, 100);
        assert_eq!(metrics.last_hold_floor_not_before_qpc_ticks, 140);
        assert_eq!(metrics.release_floor_delay_boundaries, 1);
        assert_eq!(metrics.max_release_floor_delay_ticks, 30);
        assert_eq!(metrics.last_release_floor_delay_mask, 1 << 4);
        assert_eq!(metrics.last_release_floor_authored_target_qpc_ticks, 100);
        assert_eq!(metrics.last_release_floor_not_before_qpc_ticks, 130);
    }

    #[test]
    fn release_floor_infeasibility_counts_only_when_that_floor_exceeds_latest_start() {
        let mut metrics = WorkerMetricsLocal::default();
        let mut window = PhysicalTimingWindow {
            authored_target_qpc: QpcTicks::from_raw(100),
            musical_up_not_before_qpc: QpcTicks::from_raw(100),
            down_not_before_qpc: QpcTicks::from_raw(130),
            packet_not_before_qpc: QpcTicks::from_raw(130),
            latest_down_start_qpc: Some(QpcTicks::from_raw(120)),
            hold_floor_mask: 0,
            release_floor_mask: 1 << 4,
        };
        record_release_floor_infeasibility(
            &mut metrics,
            window,
            DownMissReason::PhysicalWindowExpired,
        );
        window.latest_down_start_qpc = Some(QpcTicks::from_raw(140));
        record_release_floor_infeasibility(
            &mut metrics,
            window,
            DownMissReason::PhysicalWindowExpired,
        );
        window.latest_down_start_qpc = Some(QpcTicks::from_raw(120));
        window.release_floor_mask = 0;
        record_release_floor_infeasibility(
            &mut metrics,
            window,
            DownMissReason::PhysicalWindowExpired,
        );
        record_release_floor_infeasibility(&mut metrics, window, DownMissReason::UnobservedBacklog);

        assert_eq!(metrics.release_floor_infeasible_boundaries, 1);
    }
}
