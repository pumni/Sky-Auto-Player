use super::{lease_bounded_ticks, wait_failure_message};
use crate::engine::telemetry::WorkerMetricsLocal;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::wait::{HybridWaiter, WaitFailure, WaitOutcome, WaitResult};
use std::sync::atomic::AtomicU64;

pub(crate) enum WaitBoundary {
    Due {
        wait_result: Option<WaitResult>,
        target_qpc: QpcTicks,
    },
    Replan {
        wait_result: WaitResult,
    },
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

pub(crate) struct WaitDeadline<'a> {
    pub(crate) deadline_ticks: Option<TimelineTicks>,
    pub(crate) qpc_clock: QpcClock,
    pub(crate) clock_state: &'a mut PlaybackClockState,
    pub(crate) allow_pre_epoch_startup_dispatch: bool,
}

pub(crate) struct WaitTiming<'a> {
    pub(crate) effective_spin_threshold_ticks: DurationTicks,
    pub(crate) lease_timeout_ticks: DurationTicks,
    pub(crate) supervisor_heartbeat_ticks: &'a AtomicU64,
}

pub(crate) struct WaitSignals<'a> {
    pub(crate) waiter: &'a HybridWaiter,
    pub(crate) interrupt: &'a OwnedEvent,
}

pub(crate) struct WaitMutable<'a> {
    pub(crate) local_metrics: &'a mut WorkerMetricsLocal,
    pub(crate) force_full_cleanup: &'a mut bool,
    pub(crate) terminal_error: &'a mut Option<String>,
}

pub(crate) struct WaitBoundaryInput<'a> {
    pub(crate) deadline: WaitDeadline<'a>,
    pub(crate) timing: WaitTiming<'a>,
    pub(crate) signals: WaitSignals<'a>,
    pub(crate) mutable: WaitMutable<'a>,
}

pub(crate) fn record_wait_failure(
    failure: WaitFailure,
    local_metrics: &mut WorkerMetricsLocal,
    force_full_cleanup: &mut bool,
    terminal_error: &mut Option<String>,
) {
    if matches!(failure, WaitFailure::Clock) {
        local_metrics.wait_clock_failures = local_metrics.wait_clock_failures.saturating_add(1);
    } else {
        local_metrics.wait_backend_failures = local_metrics.wait_backend_failures.saturating_add(1);
    }
    *force_full_cleanup = true;
    *terminal_error = Some(wait_failure_message(failure));
}

fn dispatch_deadline_wake_is_due(bounded_target: QpcTicks, target_qpc: QpcTicks) -> bool {
    bounded_target == target_qpc
}

pub(crate) fn wait_for_next_boundary(context: WaitBoundaryInput<'_>) -> WaitBoundary {
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
    let WaitSignals { waiter, interrupt } = signals;
    let WaitMutable {
        local_metrics,
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
        return WaitBoundary::Due {
            wait_result: None,
            target_qpc,
        };
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
                target_qpc,
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
            record_wait_failure(failure, local_metrics, force_full_cleanup, terminal_error);
            WaitBoundary::Exit
        }
        WaitOutcome::Interrupted => {
            local_metrics.wait_interrupted_count =
                local_metrics.wait_interrupted_count.saturating_add(1);
            WaitBoundary::Replan { wait_result }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        WaitBoundary, WaitBoundaryInput, WaitDeadline, WaitMutable, WaitSignals, WaitTiming,
        dispatch_deadline_wake_is_due, record_wait_failure, wait_for_next_boundary,
    };
    use crate::engine::telemetry::WorkerMetricsLocal;
    use sky_dispatch_core::clock::PlaybackClockState;
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
    use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
    use sky_dispatch_win32::event::OwnedEvent;
    use sky_dispatch_win32::wait::{HybridWaiter, WaitFailure};
    use std::sync::atomic::AtomicU64;

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

    #[test]
    fn every_wait_failure_is_terminal_and_counted() {
        let failures = [
            (WaitFailure::Clock, true),
            (WaitFailure::TimerCreate { win32_error: 1 }, false),
            (WaitFailure::TimerArm { win32_error: 2 }, false),
            (WaitFailure::TimerWait { win32_error: 3 }, false),
            (WaitFailure::MultiWait { win32_error: 4 }, false),
        ];

        for (failure, is_clock_failure) in failures {
            let mut local_metrics = WorkerMetricsLocal::default();
            let mut force_full_cleanup = false;
            let mut terminal_error = None;

            record_wait_failure(
                failure,
                &mut local_metrics,
                &mut force_full_cleanup,
                &mut terminal_error,
            );

            assert!(force_full_cleanup);
            assert!(terminal_error.is_some());
            assert_eq!(
                local_metrics.wait_clock_failures,
                u64::from(is_clock_failure)
            );
            assert_eq!(
                local_metrics.wait_backend_failures,
                u64::from(!is_clock_failure)
            );
        }
    }

    #[test]
    fn lease_only_timer_wake_replans_instead_of_dispatching() {
        let qpc_clock = QpcClock::initialize().expect("qpc clock");
        let epoch = qpc_clock.now().expect("qpc epoch");
        let deadline = TimelineTicks::from_raw(
            qpc_clock
                .duration_from_us(50_000)
                .expect("deadline conversion")
                .as_u64(),
        );
        let mut clock_state =
            PlaybackClockState::new(epoch, DurationTicks::ZERO).expect("playback clock");
        let heartbeat = AtomicU64::new(epoch.as_u64());
        let waiter = HybridWaiter::new();
        let interrupt = OwnedEvent::new_auto_reset().expect("interrupt event");
        let mut local_metrics = WorkerMetricsLocal::default();
        let mut force_full_cleanup = false;
        let mut terminal_error = None;

        let boundary = wait_for_next_boundary(WaitBoundaryInput {
            deadline: WaitDeadline {
                deadline_ticks: Some(deadline),
                qpc_clock,
                clock_state: &mut clock_state,
                allow_pre_epoch_startup_dispatch: false,
            },
            timing: WaitTiming {
                effective_spin_threshold_ticks: DurationTicks::ZERO,
                lease_timeout_ticks: qpc_clock.duration_from_us(1_000).expect("lease conversion"),
                supervisor_heartbeat_ticks: &heartbeat,
            },
            signals: WaitSignals {
                waiter: &waiter,
                interrupt: &interrupt,
            },
            mutable: WaitMutable {
                local_metrics: &mut local_metrics,
                force_full_cleanup: &mut force_full_cleanup,
                terminal_error: &mut terminal_error,
            },
        });

        assert!(matches!(
            boundary,
            WaitBoundary::Replan { wait_result, .. }
                if matches!(wait_result.outcome, sky_dispatch_win32::wait::WaitOutcome::Interrupted)
        ));
        assert!(!force_full_cleanup);
        assert!(terminal_error.is_none());
    }
}
