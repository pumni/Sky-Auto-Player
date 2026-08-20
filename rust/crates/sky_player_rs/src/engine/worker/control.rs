use super::{
    RuntimeDispatchCoordinator, TrackedKeyState, WorkerMetricsLocal,
    cancel_coordinator_or_terminal, describe_release_outcome, publish_backend_metrics,
    record_termination_error, release_state_verified, supervisor_lease_expired,
    try_publish_metrics,
};
use crate::engine::telemetry::SharedMetrics;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::{QpcClock, QpcError, QpcTicks};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

pub(super) enum CommandControl {
    Continue,
    Exit,
}

pub(super) struct CommandControlClock<'a> {
    pub(super) loop_start_ticks: QpcTicks,
    pub(super) qpc_clock: QpcClock,
    pub(super) lease_timeout_ticks: DurationTicks,
    pub(super) supervisor_heartbeat_ticks: &'a AtomicU64,
}

pub(super) struct CommandControlSignals<'a> {
    pub(super) quit_requested: &'a AtomicBool,
    pub(super) skip_requested: &'a AtomicBool,
    pub(super) panic_requested: &'a AtomicBool,
    pub(super) target_hwnd: &'a AtomicIsize,
}

pub(super) struct CommandControlRuntime<'a> {
    pub(super) backend: &'a mut TrackedKeyState,
    pub(super) coordinator: &'a mut RuntimeDispatchCoordinator,
    pub(super) force_full_cleanup: &'a mut bool,
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
    pub(super) clock: CommandControlClock<'a>,
    pub(super) signals: CommandControlSignals<'a>,
    pub(super) runtime: CommandControlRuntime<'a>,
    pub(super) metrics: CommandControlMetrics<'a>,
}

pub(super) fn process_command_control(context: CommandControlInput<'_>) -> CommandControl {
    let CommandControlInput {
        clock,
        signals,
        runtime,
        metrics,
    } = context;
    let CommandControlClock {
        loop_start_ticks,
        qpc_clock,
        lease_timeout_ticks,
        supervisor_heartbeat_ticks,
    } = clock;
    let CommandControlSignals {
        quit_requested,
        skip_requested,
        panic_requested,
        target_hwnd,
    } = signals;
    let CommandControlRuntime {
        backend,
        coordinator,
        force_full_cleanup,
        terminal_error,
        secondary_errors,
        abort_counts,
    } = runtime;
    let CommandControlMetrics {
        local_metrics,
        metrics,
        last_published_error,
    } = metrics;

    match supervisor_lease_expired(
        loop_start_ticks,
        lease_timeout_ticks,
        supervisor_heartbeat_ticks,
    ) {
        Ok(true) => {
            *force_full_cleanup = true;
            *terminal_error = Some("supervisor_lease_expired".to_string());
            return CommandControl::Exit;
        }
        Ok(false) => {}
        Err(error) => {
            *force_full_cleanup = true;
            *terminal_error = Some(format!("QPC runtime failure: {error:?}"));
            return CommandControl::Exit;
        }
    }

    if quit_requested.load(Ordering::Acquire) || skip_requested.load(Ordering::Acquire) {
        return CommandControl::Exit;
    }

    if panic_requested.swap(false, Ordering::AcqRel) {
        let panic_release =
            backend.release_all_full_instrument(target_hwnd.load(Ordering::Acquire));
        if !release_state_verified(backend, &panic_release) {
            record_termination_error(
                terminal_error,
                secondary_errors,
                format!(
                    "panic cleanup release verification failed: {}",
                    describe_release_outcome(&panic_release)
                ),
            );
        }
        cancel_coordinator_or_terminal(
            coordinator,
            force_full_cleanup,
            terminal_error,
            secondary_errors,
        );
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
                *force_full_cleanup = true;
                *terminal_error = Some(format!("QPC runtime failure: {error:?}"));
                return CommandControl::Exit;
            }
        }
        *terminal_error = Some("panic_release_requested".to_string());
        return CommandControl::Exit;
    }

    CommandControl::Continue
}
