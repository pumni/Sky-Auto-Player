use super::{lease_bounded_ticks, wait_failure_message};
use crate::engine::telemetry::WorkerMetricsLocal;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::wait::{HybridWaiter, WaitFailure, WaitOutcome, WaitResult};
use std::sync::atomic::AtomicU64;
use std::time::Duration;

pub(super) enum WaitBoundary {
    Due { wait_result: Option<WaitResult> },
    Replan { wait_result: WaitResult },
    Exit,
}

#[derive(Clone, Copy, Debug)]
pub struct WaitObservation {
    pub outcome: WaitOutcome,
    pub wake_qpc: Option<QpcTicks>,
    pub spin_ticks: DurationTicks,
    pub deadline_ticks: TimelineTicks,
    pub epoch_qpc: QpcTicks,
    pub allow_pre_epoch_startup_dispatch: bool,
}

pub(super) struct WaitDeadline<'a> {
    pub(super) deadline_ticks: Option<TimelineTicks>,
    pub(super) qpc_clock: QpcClock,
    pub(super) clock_state: &'a mut PlaybackClockState,
    pub(super) allow_pre_epoch_startup_dispatch: bool,
}

pub(super) struct WaitTiming<'a> {
    pub(super) effective_spin_threshold_ticks: DurationTicks,
    pub(super) lease_timeout_ticks: DurationTicks,
    pub(super) supervisor_heartbeat_ticks: &'a AtomicU64,
}

pub(super) struct WaitSignals<'a> {
    pub(super) waiter: &'a HybridWaiter,
    pub(super) interrupt: &'a OwnedEvent,
    pub(super) strict_timing: bool,
}

pub(super) struct WaitMutable<'a> {
    pub(super) local_metrics: &'a mut WorkerMetricsLocal,
    pub(super) pending_pre_send_spin_us: &'a mut u64,
    pub(super) force_full_cleanup: &'a mut bool,
    pub(super) terminal_error: &'a mut Option<String>,
}

pub(super) struct WaitBoundaryInput<'a> {
    pub(super) deadline: WaitDeadline<'a>,
    pub(super) timing: WaitTiming<'a>,
    pub(super) signals: WaitSignals<'a>,
    pub(super) mutable: WaitMutable<'a>,
}

fn dispatch_deadline_wake_is_due(bounded_target: QpcTicks, target_qpc: QpcTicks) -> bool {
    bounded_target == target_qpc
}

pub(super) fn wait_for_next_boundary(context: WaitBoundaryInput<'_>) -> WaitBoundary {
    let WaitBoundaryInput {
        deadline,
        timing,
        signals,
        mutable,
    } = context;
    let WaitDeadline {
        deadline_ticks,
        qpc_clock,
        clock_state,
        allow_pre_epoch_startup_dispatch,
    } = deadline;
    let WaitTiming {
        effective_spin_threshold_ticks,
        lease_timeout_ticks,
        supervisor_heartbeat_ticks,
    } = timing;
    let WaitSignals {
        waiter,
        interrupt,
        strict_timing,
    } = signals;
    let WaitMutable {
        local_metrics,
        pending_pre_send_spin_us,
        force_full_cleanup,
        terminal_error,
    } = mutable;

    let Some(deadline_ticks) = deadline_ticks else {
        return WaitBoundary::Exit;
    };

    // Sample QPC and logical elapsed time together to avoid shifting the target.
    let target_sample_ticks = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            *force_full_cleanup = true;
            *terminal_error = Some(format!("QPC failure before dispatch wait: {error:?}"));
            return WaitBoundary::Exit;
        }
    };
    let target_sample_elapsed_ticks = match clock_state
        .get_elapsed_allow_pre_epoch(target_sample_ticks, allow_pre_epoch_startup_dispatch)
    {
        Ok(ticks) => ticks,
        Err(error) => {
            *force_full_cleanup = true;
            *terminal_error = Some(format!("playback clock failure: {error}"));
            return WaitBoundary::Exit;
        }
    };
    if deadline_ticks <= target_sample_elapsed_ticks {
        return WaitBoundary::Due { wait_result: None };
    }
    let target_qpc = match clock_state
        .epoch
        .checked_add_duration(DurationTicks::from_raw(deadline_ticks.as_u64()))
    {
        Ok(target) => target,
        Err(error) => {
            *force_full_cleanup = true;
            *terminal_error = Some(format!("deadline arithmetic failure: {error}"));
            return WaitBoundary::Exit;
        }
    };
    let bounded_target =
        match lease_bounded_ticks(target_qpc, lease_timeout_ticks, supervisor_heartbeat_ticks) {
            Ok(target) => target,
            Err(error) => {
                *force_full_cleanup = true;
                *terminal_error = Some(format!("lease deadline failure: {error:?}"));
                return WaitBoundary::Exit;
            }
        };
    let wait_result = waiter.wait_until_ticks_with_metrics_typed(
        qpc_clock,
        bounded_target,
        effective_spin_threshold_ticks,
        interrupt,
    );
    match wait_result.outcome {
        WaitOutcome::Deadline if dispatch_deadline_wake_is_due(bounded_target, target_qpc) => {
            WaitBoundary::Due {
                wait_result: Some(wait_result),
            }
        }
        WaitOutcome::Deadline => WaitBoundary::Replan {
            // A lease-only timer wake is orchestration progress, not a
            // physical dispatch deadline.  Preserve its timing evidence but
            // make that distinction explicit to the observer path.
            wait_result: WaitResult {
                outcome: WaitOutcome::Interrupted,
                ..wait_result
            },
        },
        WaitOutcome::Failed(failure) => {
            if matches!(failure, WaitFailure::Clock) {
                local_metrics.wait_clock_failures =
                    local_metrics.wait_clock_failures.saturating_add(1);
            } else {
                local_metrics.wait_backend_failures =
                    local_metrics.wait_backend_failures.saturating_add(1);
            }
            if strict_timing || matches!(failure, WaitFailure::Clock) {
                *force_full_cleanup = true;
                *terminal_error = Some(wait_failure_message(failure));
                return WaitBoundary::Exit;
            }
            std::thread::sleep(Duration::from_micros(500));
            *pending_pre_send_spin_us = 0;
            WaitBoundary::Replan { wait_result }
        }
        WaitOutcome::Interrupted => {
            local_metrics.wait_interrupted_count =
                local_metrics.wait_interrupted_count.saturating_add(1);
            *pending_pre_send_spin_us = 0;
            WaitBoundary::Replan { wait_result }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::dispatch_deadline_wake_is_due;
    use sky_dispatch_win32::clock::QpcTicks;

    #[test]
    fn lease_boundary_is_not_a_dispatch_deadline() {
        assert!(dispatch_deadline_wake_is_due(
            QpcTicks::ZERO,
            QpcTicks::ZERO
        ));
        assert!(!dispatch_deadline_wake_is_due(
            QpcTicks::from_raw(1),
            QpcTicks::from_raw(2)
        ));
    }
}
