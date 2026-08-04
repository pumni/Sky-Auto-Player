use super::*;

pub(super) enum CommandControl {
    Continue,
    Exit,
}

pub(super) struct CommandControlContext<'a> {
    pub(super) loop_start_ticks: QpcTicks,
    pub(super) qpc_clock: QpcClock,
    pub(super) lease_timeout_ticks: DurationTicks,
    pub(super) supervisor_heartbeat_ticks: &'a AtomicU64,
    pub(super) quit_requested: &'a AtomicBool,
    pub(super) skip_requested: &'a AtomicBool,
    pub(super) panic_requested: &'a AtomicBool,
    pub(super) target_hwnd: &'a AtomicIsize,
    pub(super) backend: &'a mut TrackedKeyState,
    pub(super) coordinator: &'a mut RuntimeDispatchCoordinator,
    pub(super) force_full_cleanup: &'a mut bool,
    pub(super) terminal_error: &'a mut Option<String>,
    pub(super) secondary_errors: &'a mut Vec<String>,
    pub(super) abort_counts: &'a mut HashMap<&'static str, u64>,
    pub(super) local_metrics: &'a mut WorkerMetricsLocal,
    pub(super) metrics: &'a SharedMetrics,
    pub(super) last_published_error: &'a mut Option<String>,
}

pub(super) fn process_command_control(context: CommandControlContext<'_>) -> CommandControl {
    match supervisor_lease_expired(
        context.loop_start_ticks,
        context.lease_timeout_ticks,
        context.supervisor_heartbeat_ticks,
    ) {
        Ok(true) => {
            *context.force_full_cleanup = true;
            *context.terminal_error = Some("supervisor_lease_expired".to_string());
            return CommandControl::Exit;
        }
        Ok(false) => {}
        Err(error) => {
            *context.force_full_cleanup = true;
            *context.terminal_error = Some(format!("QPC runtime failure: {error:?}"));
            return CommandControl::Exit;
        }
    }

    if context.quit_requested.load(Ordering::Acquire)
        || context.skip_requested.load(Ordering::Acquire)
    {
        return CommandControl::Exit;
    }

    if context.panic_requested.swap(false, Ordering::AcqRel) {
        let panic_release = context
            .backend
            .release_all_full_instrument(context.target_hwnd.load(Ordering::Acquire));
        if !release_state_verified(context.backend, &panic_release) {
            record_termination_error(
                context.terminal_error,
                context.secondary_errors,
                format!(
                    "panic cleanup release verification failed: {}",
                    describe_release_outcome(&panic_release)
                ),
            );
        }
        cancel_coordinator_or_terminal(
            context.coordinator,
            context.force_full_cleanup,
            context.terminal_error,
            context.secondary_errors,
        );
        *context.abort_counts.entry("panic").or_insert(0) += 1;
        publish_backend_metrics(
            context.backend,
            context.local_metrics,
            context.metrics,
            context.last_published_error,
        );
        let metrics_us = context.qpc_clock.now().and_then(|ticks| {
            context
                .qpc_clock
                .duration_to_us(DurationTicks::from_raw(ticks.as_u64()))
                .map_err(|_| QpcError::ConversionOverflow)
        });
        match metrics_us {
            Ok(value) => try_publish_metrics(context.local_metrics, context.metrics, value, true),
            Err(error) => {
                *context.force_full_cleanup = true;
                *context.terminal_error = Some(format!("QPC runtime failure: {error:?}"));
                return CommandControl::Exit;
            }
        }
        *context.terminal_error = Some("panic_release_requested".to_string());
        return CommandControl::Exit;
    }

    CommandControl::Continue
}
