use super::super::super::{PlaybackClockState, QpcTicks};
use super::super::{WorkerConfig, WorkerMetricsLocal, WorkerRuntime};
use super::{AuthoredBatchView, DispatchStep, PhysicalCommit};
use sky_dispatch_core::coordinator::RuntimeDispatchCoordinator;
use sky_dispatch_win32::input::TrackedKeyState;

#[derive(Clone, Copy)]
pub(super) enum DownMissReason {
    Backlog,
    HardLate,
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
) -> DispatchStep {
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
        let Some(prepared_up_packet) = view.prepared_up_recovery_packet.as_ref() else {
            return DispatchStep::TerminateStatic("missing_prepared_up_recovery_packet");
        };
        #[cfg(any(test, feature = "test-support"))]
        let result = backend.send_prepared_physical_packet_with_start_and_cutoff(
            prepared_up_packet,
            observed_qpc,
            None,
        );
        #[cfg(not(any(test, feature = "test-support")))]
        let result = backend.send_prepared_physical_packet_with_cutoff(prepared_up_packet, None);
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
    local_metrics.missed_down_boundaries = local_metrics.missed_down_boundaries.saturating_add(1);
    local_metrics.missed_down_keys = local_metrics
        .missed_down_keys
        .saturating_add(u64::from(view.packet_masks.down_mask.count_ones()));
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
    DispatchStep::Dispatched
}
