use super::{lease_bounded_ticks, wait_failure_message};
use crate::engine::telemetry::WorkerMetricsLocal;
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::wait::{HybridWaiter, WaitFailure, WaitOutcome, WaitResult};
use std::sync::atomic::AtomicU64;

pub(crate) enum WaitBoundary {
    Due {
        wait_result: Option<WaitResult>,
        target_qpc: QpcTicks,
        dispatch_qpc: QpcTicks,
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

pub(crate) struct WaitDeadline {
    pub(crate) physical_target_qpc: Option<QpcTicks>,
    pub(crate) spin_threshold_ticks: DurationTicks,
    pub(crate) qpc_clock: QpcClock,
}

pub(crate) struct WaitTiming<'a> {
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
    pub(crate) deadline: WaitDeadline,
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

fn spin_threshold_for_bounded_target(
    bounded_target: QpcTicks,
    physical_target_qpc: QpcTicks,
    calibrated_spin_threshold_ticks: DurationTicks,
) -> DurationTicks {
    if dispatch_deadline_wake_is_due(bounded_target, physical_target_qpc) {
        calibrated_spin_threshold_ticks
    } else {
        // A lease-only wake is an orchestration heartbeat, not a musical
        // boundary. Busy-spinning before it spends CPU without improving the
        // physical dispatch contract.
        DurationTicks::ZERO
    }
}

pub(crate) fn wait_for_next_boundary(context: WaitBoundaryInput<'_>) -> WaitBoundary {
    let WaitBoundaryInput {
        deadline,
        timing,
        signals,
        mutable,
    } = context;
    let WaitDeadline {
        physical_target_qpc,
        spin_threshold_ticks,
        qpc_clock,
        ..
    } = deadline;
    let WaitTiming {
        lease_timeout_ticks,
        supervisor_heartbeat_ticks,
    } = timing;
    let WaitSignals { waiter, interrupt } = signals;
    let WaitMutable {
        local_metrics,
        force_full_cleanup,
        terminal_error,
    } = mutable;

    let physical_target_qpc = match physical_target_qpc {
        Some(target) => target,
        None => return WaitBoundary::Exit,
    };
    let target_qpc = physical_target_qpc;
    let target_sample_ticks = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            *force_full_cleanup = true;
            *terminal_error = Some(format!("QPC failure before dispatch wait: {error:?}"));
            return WaitBoundary::Exit;
        }
    };
    if target_sample_ticks >= target_qpc {
        return WaitBoundary::Due {
            wait_result: None,
            target_qpc: physical_target_qpc,
            dispatch_qpc: target_sample_ticks,
        };
    }
    let bounded_target =
        match lease_bounded_ticks(target_qpc, lease_timeout_ticks, supervisor_heartbeat_ticks) {
            Ok(target) => target,
            Err(error) => {
                *force_full_cleanup = true;
                *terminal_error = Some(format!("lease deadline failure: {error:?}"));
                return WaitBoundary::Exit;
            }
        };
    let wait_spin_threshold_ticks =
        spin_threshold_for_bounded_target(bounded_target, target_qpc, spin_threshold_ticks);
    let wait_result = waiter.wait_until_ticks_with_metrics_typed(
        qpc_clock,
        bounded_target,
        wait_spin_threshold_ticks,
        interrupt,
    );
    match wait_result.outcome {
        WaitOutcome::Deadline if dispatch_deadline_wake_is_due(bounded_target, target_qpc) => {
            let dispatch_qpc = wait_result.wake_qpc.unwrap_or(target_qpc);
            WaitBoundary::Due {
                wait_result: Some(wait_result),
                target_qpc: physical_target_qpc,
                dispatch_qpc,
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
        dispatch_deadline_wake_is_due, record_wait_failure, spin_threshold_for_bounded_target,
        wait_for_next_boundary,
    };
    use crate::engine::telemetry::WorkerMetricsLocal;
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
    use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
    use sky_dispatch_win32::event::OwnedEvent;
    use sky_dispatch_win32::wait::{HybridWaiter, WaitFailure};
    use std::sync::atomic::AtomicU64;

    #[test]
    fn physical_wait_uses_the_frozen_precision_spin_threshold() {
        let source = include_str!("wait.rs");
        let body = source
            .split("pub(crate) fn wait_for_next_boundary")
            .nth(1)
            .expect("admission wait implementation");
        assert!(body.contains("physical_target_qpc"));
        assert!(body.contains("wait_until_ticks_with_metrics_typed"));
        assert!(body.contains("wait_spin_threshold_ticks"));
        assert!(!body.contains("wait_to_precision_boundary"));
    }

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
    fn lease_only_wake_does_not_busy_spin() {
        let configured = DurationTicks::from_raw(123);
        assert_eq!(
            spin_threshold_for_bounded_target(
                QpcTicks::from_raw(99),
                QpcTicks::from_raw(100),
                configured,
            ),
            DurationTicks::ZERO
        );
    }

    #[test]
    fn physical_target_wake_keeps_calibrated_spin() {
        let configured = DurationTicks::from_raw(123);
        assert_eq!(
            spin_threshold_for_bounded_target(
                QpcTicks::from_raw(100),
                QpcTicks::from_raw(100),
                configured,
            ),
            configured
        );
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
        let heartbeat = AtomicU64::new(epoch.as_u64());
        let waiter = HybridWaiter::new();
        let interrupt = OwnedEvent::new_auto_reset().expect("interrupt event");
        let mut local_metrics = WorkerMetricsLocal::default();
        let mut force_full_cleanup = false;
        let mut terminal_error = None;

        let boundary = wait_for_next_boundary(WaitBoundaryInput {
            deadline: WaitDeadline {
                physical_target_qpc: Some(
                    epoch
                        .checked_add_duration(DurationTicks::from_raw(deadline.as_u64()))
                        .expect("target"),
                ),
                spin_threshold_ticks: DurationTicks::from_raw(1),
                qpc_clock,
            },
            timing: WaitTiming {
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
