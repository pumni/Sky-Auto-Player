use super::{
    FinalizeInput, FinalizePublication, FinalizeResources, FinalizeSignals, FinalizeState,
    FinalizeTiming, OUTCOME_ERROR, OUTCOME_FINISHED, OUTCOME_QUIT, OUTCOME_SKIPPED,
    OUTCOME_SYSTEM_SUSPENDED, TerminalTrigger, clean_completion_proven,
    current_process_cpu_time_us, current_thread_cpu_time_us, describe_release_outcome,
    publish_backend_metrics, publish_terminal_metrics, record_termination_error,
    release_state_verified,
};
#[cfg(any(test, feature = "test-support"))]
use parking_lot::Mutex;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::QpcError;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub(crate) fn finalize_worker(context: FinalizeInput<'_>) -> u8 {
    let FinalizeInput {
        resources,
        state,
        signals,
        publication,
        timing,
        #[cfg(any(test, feature = "test-support"))]
        mut cleanup_observation,
    } = context;
    let FinalizeResources {
        mut backend,
        mut scheduler,
        telemetry,
        playback,
        qpc_clock,
        scheduling,
        mut observer,
    } = resources;
    let FinalizeState {
        worker_result,
        mut local_metrics,
        mut abort_counts,
        mut terminal_error,
        mut secondary_errors,
        mut last_published_error,
        mut terminal_trigger,
    } = state;
    let FinalizeSignals {
        target,
        skip_requested,
        quit_requested,
        panic_requested,
        supervisor_expired,
        system_power,
    } = signals;
    let FinalizePublication {
        metrics,
        telemetry_output,
        priority_acquired,
        progress_clock,
    } = publication;
    let FinalizeTiming {
        start_wall_time_us,
        start_thread_cpu_us,
        start_process_cpu_us,
    } = timing;
    let worker_panicked = worker_result.is_err();
    #[cfg(any(test, feature = "test-support"))]
    let operation_trace = cleanup_observation
        .as_ref()
        .and_then(|observation| observation.operation_trace.clone());

    if terminal_trigger.is_none() {
        terminal_trigger = if supervisor_expired.is_expired() {
            Some(TerminalTrigger::SupervisorLeaseExpired)
        } else if system_power.system_suspend_terminal() {
            Some(TerminalTrigger::SystemSuspend)
        } else if skip_requested.load(Ordering::Acquire) {
            Some(TerminalTrigger::Skip)
        } else if quit_requested.load(Ordering::Acquire) {
            Some(TerminalTrigger::Quit)
        } else if panic_requested.load(Ordering::Acquire) {
            Some(if supervisor_expired.is_expired() {
                TerminalTrigger::SupervisorLeaseExpired
            } else {
                TerminalTrigger::PanicReleaseRequested
            })
        } else if worker_result.is_err() {
            Some(TerminalTrigger::WorkerPanic)
        } else {
            None
        };
    }

    // This scalar sample does not traverse state or delay release; keeping it
    // before cleanup excludes terminal work from the dispatch-worker metric.
    let final_worker_cpu_time_us = current_thread_cpu_time_us().saturating_sub(start_thread_cpu_us);

    // This cleanup sits outside the contained loop so it also runs when an
    // unexpected panic crosses the orchestration/backend seam. It always
    // releases the persistent backend's tracked obligations, including any
    // bounded in-flight packet mask. This is the single terminal release
    // owner; resumable cleanup remains a separate lifecycle operation.
    #[cfg(any(test, feature = "test-support"))]
    record_finalization_event(&operation_trace, "terminal_release_started");
    let release_scope = sky_dispatch_win32::input::ReleaseScope::Tracked;
    let cleanup_result = catch_unwind(AssertUnwindSafe(|| {
        backend.release_scope(release_scope, target.hwnd_for_safety_release())
    }));
    if let Ok(outcome) = &cleanup_result {
        *metrics.terminal_release_outcome.lock() = Some(outcome.clone());
        if !release_state_verified(&backend, outcome) {
            record_termination_error(
                &mut terminal_error,
                &mut secondary_errors,
                format!(
                    "terminal release verification failed: {}",
                    describe_release_outcome(&backend, outcome)
                ),
            );
        }
    } else {
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            "terminal backend cleanup panicked".to_string(),
        );
    }
    #[cfg(any(test, feature = "test-support"))]
    record_finalization_event(&operation_trace, "terminal_release_returned");
    metrics.terminal_release_obligation_mask.store(
        u32::from(backend.release_obligation_mask()),
        Ordering::Release,
    );
    metrics
        .terminal_release_attempts_total
        .store(backend.release_attempts_total, Ordering::Release);

    #[cfg(any(test, feature = "test-support"))]
    if let Some(observation) = cleanup_observation.as_deref_mut() {
        observation.release_obligation_mask = backend.release_obligation_mask();
        observation.active_mask = backend.active_mask;
        observation.possibly_active_mask = backend.possibly_active_mask;
        observation.failed_release_mask = backend.failed_release_mask;
        observation.in_flight_mask = backend.in_flight_mask;
        observation.attempted_mask = cleanup_result
            .as_ref()
            .ok()
            .map_or(0, |outcome| outcome.attempted_mask);
        observation.terminal_release_attempts = cleanup_result
            .as_ref()
            .ok()
            .map_or(0, |outcome| outcome.attempts);
        observation.total_release_attempts = backend.release_attempts_total;
        observation.cleanup_outcome = cleanup_result.as_ref().ok().cloned();
    }

    // Preserve the pre-cancel ledger evidence, but only after the first
    // physical release has been attempted from the backend obligation mask.
    #[cfg(any(test, feature = "test-support"))]
    record_finalization_event(&operation_trace, "pre_cancel_ledger_audit");
    let pre_cancel_accounting = {
        if let Err(error) = scheduler.check_invariants() {
            record_termination_error(
                &mut terminal_error,
                &mut secondary_errors,
                format!("scheduler pre-cancel invariant failure: {error}"),
            );
        }
        scheduler.accounting()
    };

    if let Err(payload) = &worker_result {
        let message = payload
            .downcast_ref::<&str>()
            .map(|value| (*value).to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "native worker panicked".to_string());
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            format!("worker panicked before terminal cleanup: {message}"),
        );
    }

    if let Some(trigger) = terminal_trigger {
        *metrics.terminal_trigger_reason.lock() = Some(trigger.as_str().to_string());
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(observation) = cleanup_observation.as_deref_mut() {
        observation.pre_cancel_generation_accounting = pre_cancel_accounting;
        observation.terminal_trigger_reason =
            terminal_trigger.map(|trigger| trigger.as_str().to_string());
    }

    if terminal_error.is_none()
        && terminal_trigger.is_none()
        && !skip_requested.load(Ordering::Acquire)
        && !quit_requested.load(Ordering::Acquire)
        && !clean_completion_proven(pre_cancel_accounting, &backend)
    {
        terminal_error = Some(
            "clean completion contract failed: authored generations or backend state were not fully released"
                .to_string(),
        );
    }

    #[cfg(any(test, feature = "test-support"))]
    record_finalization_event(&operation_trace, "terminal_cancellation");
    let terminal_cleanup_result = scheduler.on_verified_terminal_cleanup().map(|_| ());
    if let Err(error) = terminal_cleanup_result {
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            format!("scheduler terminal cancellation failure: {error}"),
        );
    }
    #[cfg(any(test, feature = "test-support"))]
    record_finalization_event(&operation_trace, "post_cleanup_ledger_audit");
    let post_cleanup_check = scheduler.check_invariants();
    if let Err(error) = post_cleanup_check {
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            format!("scheduler post-cleanup invariant failure: {error}"),
        );
    }
    #[cfg(any(test, feature = "test-support"))]
    record_finalization_event(&operation_trace, "observer_shutdown");
    let mut observer_up_slot_accounting_complete = true;
    if let Some(observer_runtime) = observer.take() {
        let observer_output = observer_runtime.stop();
        local_metrics.merge_observer(&observer_output.metrics);
        observer_up_slot_accounting_complete = observer_output.up_slot_accounting_complete;
        if let Some(error) = observer_output.terminal_error {
            record_termination_error(
                &mut terminal_error,
                &mut secondary_errors,
                format!("observer consumer failed: {error}"),
            );
        }
    }
    let end_ticks = qpc_clock.now();
    let end_qpc = end_ticks.and_then(|ticks| {
        qpc_clock
            .duration_to_us(DurationTicks::from_raw(ticks.as_u64()))
            .map_err(|_| QpcError::ConversionOverflow)
    });
    let end_us = match end_qpc {
        Ok(value) => value,
        Err(error) => {
            record_termination_error(
                &mut terminal_error,
                &mut secondary_errors,
                format!("QPC runtime failure during termination: {error:?}"),
            );
            start_wall_time_us
        }
    };
    if let Ok(terminal_ticks) = end_ticks {
        progress_clock.publish_terminal(&playback, terminal_ticks);
    }
    // A callback may arrive while terminal cleanup is running. Re-read the
    // monotonic session latch before publishing the outcome so Stop/Skip or
    // clean completion cannot hide an accepted suspend boundary.
    if supervisor_expired.is_expired()
        && !terminal_trigger.is_some_and(TerminalTrigger::requires_error_outcome)
    {
        terminal_trigger = Some(TerminalTrigger::SupervisorLeaseExpired);
    } else if system_power.system_suspend_terminal()
        && !terminal_trigger.is_some_and(TerminalTrigger::requires_error_outcome)
    {
        terminal_trigger = Some(TerminalTrigger::SystemSuspend);
    } else if panic_requested.load(Ordering::Acquire)
        && !terminal_trigger.is_some_and(TerminalTrigger::requires_error_outcome)
    {
        terminal_trigger = Some(TerminalTrigger::PanicReleaseRequested);
    }
    if let Some(trigger) = terminal_trigger {
        *metrics.terminal_trigger_reason.lock() = Some(trigger.as_str().to_string());
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(observation) = cleanup_observation.as_deref_mut() {
        observation.terminal_trigger_reason =
            terminal_trigger.map(|trigger| trigger.as_str().to_string());
    }
    if matches!(
        terminal_trigger,
        Some(TerminalTrigger::PanicReleaseRequested | TerminalTrigger::SupervisorLeaseExpired)
    ) {
        *abort_counts.entry("panic").or_insert(0) += 1;
    }
    let terminal_abort_reason = if worker_panicked
        || cleanup_result.is_err()
        || terminal_error.is_some()
        || terminal_trigger.is_some_and(TerminalTrigger::requires_error_outcome)
    {
        "error"
    } else if matches!(terminal_trigger, Some(TerminalTrigger::SystemSuspend)) {
        "system_suspend"
    } else if matches!(terminal_trigger, Some(TerminalTrigger::Skip))
        || skip_requested.load(Ordering::Acquire)
    {
        "skipped"
    } else if matches!(terminal_trigger, Some(TerminalTrigger::Quit))
        || quit_requested.load(Ordering::Acquire)
    {
        "quit"
    } else {
        "finished"
    };
    *abort_counts.entry(terminal_abort_reason).or_insert(0) += 1;
    *metrics.abort_counts_by_reason.lock() = abort_counts
        .into_iter()
        .map(|(reason, count)| (reason.to_string(), count))
        .collect();
    *metrics.terminal_error.lock() = terminal_error.clone();
    *metrics.secondary_errors.lock() = secondary_errors.clone();
    #[cfg(any(test, feature = "test-support"))]
    if let Some(observation) = cleanup_observation {
        observation.terminal_error = terminal_error.clone();
        observation.secondary_errors = secondary_errors.clone();
    }
    #[cfg(any(test, feature = "test-support"))]
    record_finalization_event(&operation_trace, "generation_metrics_export");
    let generation_status_counts = scheduler.generation_status_counts();
    *metrics.generation_status_counts.lock() = generation_status_counts;
    #[cfg(any(test, feature = "test-support", feature = "real-input-acceptance"))]
    {
        let generation_accounting = scheduler.accounting();
        *metrics.generation_accounting.lock() = generation_accounting;
    }
    publish_backend_metrics(
        &backend,
        &mut local_metrics,
        metrics,
        &mut last_published_error,
    );

    local_metrics.playback_wall_time_us = end_us.saturating_sub(start_wall_time_us);
    local_metrics.worker_cpu_time_us = final_worker_cpu_time_us;
    local_metrics.process_cpu_time_us =
        current_process_cpu_time_us().saturating_sub(start_process_cpu_us);
    if local_metrics.idle_wake_count > 0 && local_metrics.playback_wall_time_us > 0 {
        local_metrics.spin_duty_cycle_ppm = (local_metrics.spin_time_us as u128 * 1_000_000
            / local_metrics.playback_wall_time_us as u128)
            as u64;
    }
    publish_terminal_metrics(&local_metrics, metrics, qpc_clock, end_us);
    metrics.is_paused.store(false, Ordering::Relaxed);
    let telemetry = match Arc::try_unwrap(telemetry) {
        Ok(telemetry) => telemetry.into_inner(),
        Err(_) => panic!("observer telemetry must be uniquely owned after observer shutdown"),
    };
    let mut telemetry = telemetry;
    telemetry.output.qpc_frequency_hz = qpc_clock.frequency_hz().get();
    telemetry.output.observer_queue_dropped = local_metrics.observer_dropped_samples;
    telemetry.output.up_slot_records_expected = local_metrics.up_slot_records_expected;
    telemetry.output.account_up_slot_loss(
        crate::engine::telemetry::UpSlotLossReason::ObserverQueue,
        local_metrics.observer_dropped_up_slot_records,
    );
    telemetry.output.account_up_slot_loss(
        crate::engine::telemetry::UpSlotLossReason::PoolStorage,
        local_metrics.observer_dropped_up_slot_pool_records,
    );
    telemetry.output.account_up_slot_loss(
        crate::engine::telemetry::UpSlotLossReason::CaptureModeOff,
        local_metrics.up_slot_records_not_captured_by_mode,
    );
    telemetry
        .output
        .finalize_up_slot_accounting(observer_up_slot_accounting_complete);
    if telemetry.output.observer_queue_dropped > 0 {
        telemetry.output.truncated = true;
    }
    #[cfg(any(test, feature = "test-support"))]
    record_finalization_event(&operation_trace, "telemetry_export");
    *telemetry_output.lock() = Some(std::mem::take(&mut telemetry.output));
    drop(scheduling);
    *priority_acquired.lock() = "off".to_string();
    local_metrics.power_throttling_disabled = false;
    #[cfg(any(test, feature = "test-support"))]
    record_finalization_event(&operation_trace, "terminal_metrics_publication");
    publish_terminal_metrics(&local_metrics, metrics, qpc_clock, end_us);
    let cleanup_panicked = cleanup_result.is_err();
    match (worker_result, cleanup_result) {
        (Err(payload), _) | (Ok(_), Err(payload)) => resume_unwind(payload),
        (Ok(_), Ok(_)) => {}
    }
    if terminal_error.is_some()
        || cleanup_panicked
        || worker_panicked
        || terminal_trigger.is_some_and(TerminalTrigger::requires_error_outcome)
    {
        OUTCOME_ERROR
    } else if matches!(terminal_trigger, Some(TerminalTrigger::SystemSuspend)) {
        OUTCOME_SYSTEM_SUSPENDED
    } else if matches!(terminal_trigger, Some(TerminalTrigger::Skip))
        || skip_requested.load(Ordering::Acquire)
    {
        OUTCOME_SKIPPED
    } else if matches!(terminal_trigger, Some(TerminalTrigger::Quit))
        || quit_requested.load(Ordering::Acquire)
    {
        OUTCOME_QUIT
    } else {
        OUTCOME_FINISHED
    }
}

#[cfg(any(test, feature = "test-support"))]
fn record_finalization_event(trace: &Option<Arc<Mutex<Vec<&'static str>>>>, event: &'static str) {
    if let Some(trace) = trace {
        trace.lock().push(event);
    }
}
