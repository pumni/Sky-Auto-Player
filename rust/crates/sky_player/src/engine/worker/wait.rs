use super::wait_failure_message;
use crate::engine::telemetry::WorkerMetricsLocal;
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::wait::{HybridWaiter, WaitFailure, WaitOutcome, WaitResult};

pub(crate) enum WaitBoundary {
    Due {
        wait_result: Option<WaitResult>,
        target_qpc: QpcTicks,
        dispatch_qpc: QpcTicks,
        planned_wait_ticks: DurationTicks,
    },
    Replan {
        wait_result: WaitResult,
        target_qpc: QpcTicks,
        planned_wait_ticks: DurationTicks,
    },
    Exit,
}

#[derive(Clone, Copy, Debug)]
pub struct WaitObservation {
    pub outcome: WaitOutcome,
    pub wake_qpc: Option<QpcTicks>,
    pub spin_ticks: DurationTicks,
    pub physical_target_qpc: QpcTicks,
    pub planned_wait_ticks: DurationTicks,
    pub deadline_ticks: TimelineTicks,
    pub epoch_qpc: QpcTicks,
    pub allow_pre_epoch_startup_dispatch: bool,
}

pub(crate) struct WaitDeadline {
    pub(crate) physical_target_qpc: Option<QpcTicks>,
    pub(crate) spin_threshold_ticks: DurationTicks,
    pub(crate) qpc_clock: QpcClock,
}

pub(crate) struct WaitSignals<'a> {
    pub(crate) waiter: &'a HybridWaiter,
    pub(crate) interrupt: &'a OwnedEvent,
}

pub(crate) struct WaitMutable<'a> {
    pub(crate) local_metrics: &'a mut WorkerMetricsLocal,
    pub(crate) terminal_error: &'a mut Option<String>,
}

pub(crate) struct WaitBoundaryInput<'a> {
    pub(crate) deadline: WaitDeadline,
    pub(crate) signals: WaitSignals<'a>,
    pub(crate) mutable: WaitMutable<'a>,
}

pub(crate) fn record_wait_failure(
    failure: WaitFailure,
    local_metrics: &mut WorkerMetricsLocal,
    terminal_error: &mut Option<String>,
) {
    if matches!(failure, WaitFailure::Clock) {
        local_metrics.wait_clock_failures = local_metrics.wait_clock_failures.saturating_add(1);
    } else {
        local_metrics.wait_backend_failures = local_metrics.wait_backend_failures.saturating_add(1);
    }
    *terminal_error = Some(wait_failure_message(failure));
}

pub(crate) fn wait_for_next_boundary(context: WaitBoundaryInput<'_>) -> WaitBoundary {
    let WaitBoundaryInput {
        deadline,
        signals,
        mutable,
    } = context;
    let WaitDeadline {
        physical_target_qpc,
        spin_threshold_ticks,
        qpc_clock,
        ..
    } = deadline;
    let WaitSignals { waiter, interrupt } = signals;
    let WaitMutable {
        local_metrics,
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
            *terminal_error = Some(format!("QPC failure before dispatch wait: {error:?}"));
            return WaitBoundary::Exit;
        }
    };
    if target_sample_ticks >= target_qpc {
        return WaitBoundary::Due {
            wait_result: None,
            target_qpc: physical_target_qpc,
            dispatch_qpc: target_sample_ticks,
            planned_wait_ticks: DurationTicks::ZERO,
        };
    }
    let planned_wait_ticks = match target_qpc.checked_duration_since(target_sample_ticks) {
        Ok(ticks) => ticks,
        Err(error) => {
            *terminal_error = Some(format!("QPC planned wait arithmetic failure: {error:?}"));
            return WaitBoundary::Exit;
        }
    };
    let wait_result = waiter.wait_until_ticks_with_metrics_typed(
        qpc_clock,
        target_qpc,
        spin_threshold_ticks,
        interrupt,
    );
    match wait_result.outcome {
        WaitOutcome::Deadline => {
            let dispatch_qpc = wait_result.wake_qpc.unwrap_or(target_qpc);
            WaitBoundary::Due {
                wait_result: Some(wait_result),
                target_qpc: physical_target_qpc,
                dispatch_qpc,
                planned_wait_ticks,
            }
        }
        WaitOutcome::Failed(failure) => {
            record_wait_failure(failure, local_metrics, terminal_error);
            WaitBoundary::Exit
        }
        WaitOutcome::Interrupted => {
            local_metrics.wait_interrupted_count =
                local_metrics.wait_interrupted_count.saturating_add(1);
            WaitBoundary::Replan {
                wait_result,
                target_qpc: physical_target_qpc,
                planned_wait_ticks,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        WaitBoundary, WaitBoundaryInput, WaitDeadline, WaitMutable, WaitSignals,
        record_wait_failure, wait_for_next_boundary,
    };
    use crate::engine::shared::{SYSTEM_POWER_SUSPEND_PENDING, SystemPowerState};
    use crate::engine::telemetry::WorkerMetricsLocal;
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
    use sky_dispatch_win32::clock::QpcClock;
    use sky_dispatch_win32::event::OwnedEvent;
    use sky_dispatch_win32::wait::{HybridWaiter, WaitFailure};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn physical_wait_uses_the_frozen_precision_spin_threshold() {
        let source = include_str!("wait.rs");
        let body = source
            .split("pub(crate) fn wait_for_next_boundary")
            .nth(1)
            .expect("admission wait implementation");
        assert!(body.contains("physical_target_qpc"));
        assert!(body.contains("wait_until_ticks_with_metrics_typed"));
        assert!(body.contains("spin_threshold_ticks"));
        assert!(!body.contains("lease_bounded_ticks"));
        assert!(!body.contains("supervisor_heartbeat_ticks"));
        assert!(!body.contains("wait_to_precision_boundary"));
    }

    #[test]
    fn physical_wait_does_not_use_lease_deadline_or_heartbeat_arithmetic() {
        let source = include_str!("wait.rs");
        let body = source
            .split("pub(crate) fn wait_for_next_boundary")
            .nth(1)
            .expect("wait implementation");
        assert!(!body.contains("lease_timeout_ticks"));
        assert!(!body.contains("supervisor_heartbeat_ticks"));
        assert!(body.contains("target_qpc"));
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
            let mut terminal_error = None;

            record_wait_failure(failure, &mut local_metrics, &mut terminal_error);

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
    fn physical_wait_reaches_the_authored_target() {
        let qpc_clock = QpcClock::initialize().expect("qpc clock");
        let epoch = qpc_clock.now().expect("qpc epoch");
        let deadline = TimelineTicks::from_raw(
            qpc_clock
                .duration_from_us(50_000)
                .expect("deadline conversion")
                .as_u64(),
        );
        let waiter = HybridWaiter::new();
        let interrupt = OwnedEvent::new_auto_reset().expect("interrupt event");
        let mut local_metrics = WorkerMetricsLocal::default();
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
            signals: WaitSignals {
                waiter: &waiter,
                interrupt: &interrupt,
            },
            mutable: WaitMutable {
                local_metrics: &mut local_metrics,
                terminal_error: &mut terminal_error,
            },
        });

        assert!(matches!(boundary, WaitBoundary::Due { .. }));
        assert!(terminal_error.is_none());
    }

    #[test]
    fn lifecycle_interrupt_wakes_an_armed_relative_wait() {
        let qpc_clock = QpcClock::initialize().expect("qpc clock");
        let epoch = qpc_clock.now().expect("qpc epoch");
        let deadline_delta = qpc_clock
            .duration_from_us(5_000_000)
            .expect("deadline conversion");
        let deadline = TimelineTicks::from_raw(deadline_delta.as_u64());
        let waiter = HybridWaiter::new();
        let interrupt = Arc::new(OwnedEvent::new_auto_reset().expect("interrupt event"));
        let signal_event = Arc::clone(&interrupt);
        let system_power = Arc::new(SystemPowerState::default());
        let signal_power = Arc::clone(&system_power);
        let signaler = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(25));
            assert!(signal_power.notify(true, &signal_event));
        });
        let mut local_metrics = WorkerMetricsLocal::default();
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
            signals: WaitSignals {
                waiter: &waiter,
                interrupt: &interrupt,
            },
            mutable: WaitMutable {
                local_metrics: &mut local_metrics,
                terminal_error: &mut terminal_error,
            },
        });
        signaler.join().expect("signal thread");

        assert!(matches!(
            boundary,
            WaitBoundary::Replan { wait_result, .. }
                if matches!(wait_result.outcome, sky_dispatch_win32::wait::WaitOutcome::Interrupted)
        ));
        assert_eq!(system_power.take_pending(), SYSTEM_POWER_SUSPEND_PENDING);
        assert!(system_power.down_blocked());
        assert!(terminal_error.is_none());
    }

    #[test]
    fn system_suspend_signal_before_wait_arm_is_not_lost() {
        let qpc_clock = QpcClock::initialize().expect("qpc clock");
        let epoch = qpc_clock.now().expect("qpc epoch");
        let deadline_delta = qpc_clock
            .duration_from_us(5_000_000)
            .expect("deadline conversion");
        let deadline = TimelineTicks::from_raw(deadline_delta.as_u64());
        let waiter = HybridWaiter::new();
        let interrupt = OwnedEvent::new_auto_reset().expect("interrupt event");
        let system_power = SystemPowerState::default();
        assert!(system_power.notify(true, &interrupt));
        let mut local_metrics = WorkerMetricsLocal::default();
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
            signals: WaitSignals {
                waiter: &waiter,
                interrupt: &interrupt,
            },
            mutable: WaitMutable {
                local_metrics: &mut local_metrics,
                terminal_error: &mut terminal_error,
            },
        });

        assert!(matches!(
            boundary,
            WaitBoundary::Replan { wait_result, .. }
                if matches!(wait_result.outcome, sky_dispatch_win32::wait::WaitOutcome::Interrupted)
        ));
        assert!(system_power.down_blocked());
        assert!(terminal_error.is_none());
    }
}
