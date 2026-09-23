use super::{
    RuntimeDispatchCoordinator, TrackedKeyState, WorkerMetricsLocal,
    cancel_coordinator_or_terminal, describe_release_outcome, publish_backend_metrics,
    record_termination_error, release_state_verified, try_publish_metrics,
};
use crate::engine::shared::SupervisorLeaseState;
use crate::engine::telemetry::SharedMetrics;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::{QpcClock, QpcError};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

pub(super) enum CommandControl {
    Continue,
    Exit,
}

pub(super) struct CommandControlClock {
    pub(super) qpc_clock: QpcClock,
}

pub(super) struct CommandControlSignals<'a> {
    pub(super) quit_requested: &'a AtomicBool,
    pub(super) skip_requested: &'a AtomicBool,
    pub(super) panic_requested: &'a AtomicBool,
    pub(super) supervisor_expired: &'a SupervisorLeaseState,
    pub(super) target_hwnd: &'a AtomicIsize,
}

pub(super) struct CommandControlRuntime<'a> {
    pub(super) backend: &'a mut TrackedKeyState,
    pub(super) coordinator: &'a mut RuntimeDispatchCoordinator,
    pub(super) terminal_error: &'a mut Option<String>,
    pub(super) secondary_errors: &'a mut Vec<String>,
    pub(super) abort_counts: &'a mut HashMap<&'static str, u64>,
}

pub(super) struct CommandControlMetrics<'a> {
    pub(super) local_metrics: &'a mut WorkerMetricsLocal,
    pub(super) metrics: &'a SharedMetrics,
    pub(super) last_published_error: &'a mut Option<String>,
}

pub(super) struct CommandControlInput<'a> {
    pub(super) clock: CommandControlClock,
    pub(super) signals: CommandControlSignals<'a>,
    pub(super) runtime: CommandControlRuntime<'a>,
    pub(super) metrics: CommandControlMetrics<'a>,
}

#[inline]
fn terminal_reason_for_hard_stop(
    supervisor_expired_before: bool,
    panic_hard_stop_consumed: bool,
    supervisor_expired_after: bool,
) -> Option<&'static str> {
    if !supervisor_expired_before && !panic_hard_stop_consumed {
        return None;
    }
    Some(if supervisor_expired_before || supervisor_expired_after {
        "supervisor_lease_expired"
    } else {
        "panic_release_requested"
    })
}

#[inline]
fn consume_panic_request_if_pending(panic_requested: &AtomicBool, command_exit: bool) -> bool {
    if command_exit || !panic_requested.load(Ordering::Acquire) {
        return false;
    }
    panic_requested.swap(false, Ordering::AcqRel)
}

pub(super) fn process_command_control(context: CommandControlInput<'_>) -> CommandControl {
    let CommandControlInput {
        clock,
        signals,
        runtime,
        metrics,
    } = context;
    let CommandControlClock { qpc_clock } = clock;
    let CommandControlSignals {
        quit_requested,
        skip_requested,
        panic_requested,
        supervisor_expired,
        target_hwnd,
    } = signals;
    let CommandControlRuntime {
        backend,
        coordinator,
        terminal_error,
        secondary_errors,
        abort_counts,
    } = runtime;
    let CommandControlMetrics {
        local_metrics,
        metrics,
        last_published_error,
    } = metrics;

    let supervisor_expired_before = supervisor_expired.is_expired();
    let command_exit =
        quit_requested.load(Ordering::Acquire) || skip_requested.load(Ordering::Acquire);
    let panic_hard_stop_consumed = consume_panic_request_if_pending(panic_requested, command_exit);
    let panic_requested = supervisor_expired_before || panic_hard_stop_consumed;
    if panic_requested {
        let panic_release = backend.release_all(target_hwnd.load(Ordering::Acquire));
        if !release_state_verified(backend, &panic_release) {
            record_termination_error(
                terminal_error,
                secondary_errors,
                format!(
                    "panic cleanup release verification failed: {}",
                    describe_release_outcome(backend, &panic_release)
                ),
            );
        }
        cancel_coordinator_or_terminal(coordinator, terminal_error, secondary_errors);
        *abort_counts.entry("panic").or_insert(0) += 1;
        publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
        let metrics_us = qpc_clock.now().and_then(|ticks| {
            qpc_clock
                .duration_to_us(DurationTicks::from_raw(ticks.as_u64()))
                .map_err(|_| QpcError::ConversionOverflow)
        });
        match metrics_us {
            Ok(value) => try_publish_metrics(local_metrics, metrics, qpc_clock, value, true),
            Err(error) => {
                *terminal_error = Some(format!("QPC runtime failure: {error:?}"));
                return CommandControl::Exit;
            }
        };
        let supervisor_expired_after = supervisor_expired.is_expired();
        *terminal_error = Some(
            terminal_reason_for_hard_stop(
                supervisor_expired_before,
                panic_hard_stop_consumed,
                supervisor_expired_after,
            )
            .expect("panic cleanup must have a terminal reason")
            .to_string(),
        );
        return CommandControl::Exit;
    }
    if command_exit {
        return CommandControl::Exit;
    }

    CommandControl::Continue
}

#[cfg(test)]
mod tests {
    use super::terminal_reason_for_hard_stop;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn watchdog_expiry_after_worker_sample_keeps_terminal_identity() {
        let supervisor_expired = super::super::super::shared::SupervisorLeaseState::new(
            sky_dispatch_core::time::QpcTicks::from_raw(1),
        );
        let panic_requested = AtomicBool::new(true);

        // Deterministic interleaving: the worker sampled the old state, then
        // the watchdog published expiry before the hard-stop was consumed.
        let sampled_before = supervisor_expired.is_expired();
        supervisor_expired.latch_expired();
        let hard_stop_consumed = panic_requested.swap(false, Ordering::AcqRel);
        let sampled_after = supervisor_expired.is_expired();

        assert_eq!(
            terminal_reason_for_hard_stop(sampled_before, hard_stop_consumed, sampled_after),
            Some("supervisor_lease_expired")
        );
    }

    #[test]
    fn explicit_panic_without_supervisor_expiry_keeps_user_terminal_identity() {
        assert_eq!(
            terminal_reason_for_hard_stop(false, true, false),
            Some("panic_release_requested")
        );
    }

    #[test]
    fn panic_fast_path_does_not_rmw_when_flag_is_clear() {
        let panic_requested = AtomicBool::new(false);

        assert!(!super::consume_panic_request_if_pending(
            &panic_requested,
            false
        ));
        panic_requested.store(true, Ordering::Release);
        assert!(super::consume_panic_request_if_pending(
            &panic_requested,
            false
        ));
        assert!(!panic_requested.load(Ordering::Acquire));
    }

    #[test]
    fn panic_fast_path_source_loads_before_conditional_consume() {
        let source = include_str!("control.rs");
        let helper = source
            .split("fn consume_panic_request_if_pending")
            .nth(1)
            .expect("panic fast path helper")
            .split("pub(super) fn process_command_control")
            .next()
            .expect("panic fast path helper body");
        assert!(helper.find("load(Ordering::Acquire)").unwrap() < helper.find("swap(").unwrap());
    }
}
