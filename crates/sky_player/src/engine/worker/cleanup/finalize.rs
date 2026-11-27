use super::super::admission::{TargetStamp, verify_safety_cleanup_target};
use super::{
    FinalizeInput, FinalizePublication, FinalizeResources, FinalizeSignals, FinalizeState,
    FinalizeTiming, TerminalTrigger, clean_completion_proven, current_process_cpu_time_us,
    current_thread_cpu_time_us, describe_release_outcome, publish_backend_metrics,
    publish_terminal_metrics, record_termination_error, release_state_verified,
};
#[cfg(any(test, feature = "test-support"))]
use parking_lot::Mutex;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::{QpcError, QpcTicks};
#[cfg(any(test, feature = "test-support", feature = "real-input-acceptance"))]
use sky_dispatch_win32::input::GuardedReleaseOutcome;
#[cfg(feature = "real-input-acceptance")]
use sky_dispatch_win32::input::TerminalProbeVerdict;
use sky_dispatch_win32::input::TerminalReleaseEvidence;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::sync::atomic::Ordering;

enum TerminalCleanupFailure {
    Panicked(Box<dyn std::any::Any + Send>),
}

#[cfg(any(test, feature = "test-support", feature = "real-input-acceptance"))]
fn terminal_raw_up_syscall_count(outcome: &GuardedReleaseOutcome) -> u64 {
    outcome
        .send_attempts
        .iter()
        .map(|attempt| u64::from(attempt.raw_calls))
        .sum()
}

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
        mut pending_production_forensics,
        mut abort_counts,
        mut terminal_error,
        mut secondary_errors,
        mut attempted_dispatch_anomaly,
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
        supervisor_expiry_reason_code,
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
    let cleanup_target = terminal_cleanup_target(target);
    let session_release_owner = backend.session_release_owner_proof().cloned();
    let cleanup_result = catch_unwind(AssertUnwindSafe(|| {
        backend.release_terminal_tracked_scope_with_attempt_admission_and_drain_control(
            cleanup_target.hwnd,
            |_, release_owner| {
                let stamp = cleanup_target.stamp.ok_or_else(|| {
                    "target HWND/generation unavailable before tracked safety Up".to_string()
                })?;
                verify_safety_cleanup_target(
                    target,
                    stamp,
                    release_owner,
                    cleanup_target.require_owner_identity,
                )
            },
            || {
                let stamp = cleanup_target.stamp.ok_or_else(|| {
                    "target HWND/generation unavailable during terminal cleanup".to_string()
                })?;
                let release_owner = session_release_owner.as_ref().ok_or_else(|| {
                    "session release owner proof is missing during terminal cleanup".to_string()
                })?;
                super::super::admission::safety_cleanup_drain_decision(
                    target,
                    stamp,
                    release_owner,
                    cleanup_target.require_owner_identity,
                    true,
                )
            },
        )
    }))
    .map_err(TerminalCleanupFailure::Panicked);
    if let Ok(outcome) = &cleanup_result {
        *metrics.terminal_release_outcome.lock() = Some(outcome.release_outcome.clone());
        let release_verified = release_state_verified(&backend, &outcome.release_outcome);
        #[cfg(any(test, feature = "test-support", feature = "real-input-acceptance"))]
        {
            metrics.terminal_release_raw_up_syscall_count.store(
                terminal_raw_up_syscall_count(outcome),
                std::sync::atomic::Ordering::Relaxed,
            );
            metrics
                .terminal_release_raw_up_syscall_count_available
                .store(true, std::sync::atomic::Ordering::Release);
            #[cfg(feature = "real-input-acceptance")]
            if release_verified
                && outcome.needs_up_mask == 0
                && outcome.needs_probe_mask == 0
                && matches!(outcome.probe_verdict, TerminalProbeVerdict::AllUp)
            {
                // This worker-side QPC sample follows cleanup's terminal
                // probe and journal reconciliation. It deliberately precedes
                // telemetry extraction and external ReceiveOnly log settling.
                let release_ticks = qpc_clock.now().map_or(0, |ticks| ticks.as_u64());
                metrics
                    .terminal_release_verified_qpc_ticks
                    .store(release_ticks, std::sync::atomic::Ordering::Release);
            }
        }
        if !release_verified {
            let evidence = outcome.terminal_evidence();
            record_termination_error(
                &mut terminal_error,
                &mut secondary_errors,
                format!(
                    "terminal release verification failed: {}; {}",
                    describe_release_outcome(&backend, &outcome.release_outcome),
                    describe_terminal_release_evidence(&evidence)
                ),
            );
        }
        if outcome
            .send_attempts
            .iter()
            .any(|attempt| attempt.qpc_error.is_some())
        {
            let evidence = outcome.terminal_evidence();
            record_termination_error(
                &mut terminal_error,
                &mut secondary_errors,
                format!(
                    "terminal cleanup QPC evidence unavailable: {}",
                    describe_terminal_release_evidence(&evidence)
                ),
            );
        } else if outcome.release_outcome.transport_anomaly && release_verified {
            let evidence = outcome.terminal_evidence();
            let diagnostic = format!(
                "terminal cleanup transport anomaly: {}",
                describe_terminal_release_evidence(&evidence)
            );
            if !secondary_errors.contains(&diagnostic) {
                secondary_errors.push(diagnostic);
            }
        }
        if let Some(reason) = &outcome.withheld_reason {
            let evidence = outcome.terminal_evidence();
            record_termination_error(
                &mut terminal_error,
                &mut secondary_errors,
                format!(
                    "terminal tracked cleanup attempt withheld: {reason}; {}",
                    describe_terminal_release_evidence(&evidence)
                ),
            );
        }
    } else if let Err(failure) = &cleanup_result {
        let message = match failure {
            TerminalCleanupFailure::Panicked(_) => "terminal backend cleanup panicked".into(),
        };
        record_termination_error(&mut terminal_error, &mut secondary_errors, message);
    }
    if let Some(error) = backend.timing_error {
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            format!("earlier SendInput QPC timing evidence unavailable: {error:?}"),
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
            .map_or(0, |outcome| outcome.release_outcome.attempted_mask);
        observation.terminal_release_attempts = cleanup_result
            .as_ref()
            .ok()
            .map_or(0, |outcome| outcome.release_outcome.attempts);
        observation.total_release_attempts = backend.release_attempts_total;
        observation.terminal_release_raw_up_syscall_count = cleanup_result
            .as_ref()
            .ok()
            .map_or(0, terminal_raw_up_syscall_count);
        observation.cleanup_outcome = cleanup_result
            .as_ref()
            .ok()
            .map(|outcome| outcome.release_outcome.clone());
        observation.terminal_cleanup_evidence = cleanup_result
            .as_ref()
            .ok()
            .map(|outcome| outcome.terminal_evidence());
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

    let resolve_trigger_string = |trigger: TerminalTrigger| -> String {
        let specific = supervisor_expiry_reason_code.and_then(|code| {
            crate::engine::shared::decode_supervisor_expiry_reason(code.load(Ordering::Acquire))
        });
        if let Some(reason_str) = specific
            && matches!(
                trigger,
                TerminalTrigger::PanicReleaseRequested | TerminalTrigger::SupervisorLeaseExpired
            )
        {
            reason_str.to_string()
        } else {
            trigger.as_str().to_string()
        }
    };

    if let Some(trigger) = terminal_trigger {
        let mut reason_slot = metrics.terminal_trigger_reason.lock();
        if reason_slot.is_none() {
            *reason_slot = Some(resolve_trigger_string(trigger));
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(observation) = cleanup_observation.as_deref_mut() {
        observation.pre_cancel_generation_accounting = pre_cancel_accounting;
        observation.terminal_trigger_reason = terminal_trigger.map(resolve_trigger_string);
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
    let terminal_cleanup_result = scheduler
        .cancel_remaining_generations_for_terminal_shutdown()
        .map(|_| ());
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
    // The final packet's compact evidence is reduced after physical cleanup
    // and observer teardown. It cannot delay or alter terminal release.
    pending_production_forensics.drain_after_cleanup(&mut local_metrics);
    let end_ticks = qpc_clock.now();
    let terminal_publish_ticks = end_ticks
        .as_ref()
        .map(|ticks| ticks.as_u64())
        .unwrap_or_default();
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
        let mut reason_slot = metrics.terminal_trigger_reason.lock();
        if reason_slot.is_none() {
            *reason_slot = Some(resolve_trigger_string(trigger));
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(observation) = cleanup_observation.as_deref_mut() {
        observation.terminal_trigger_reason = terminal_trigger.map(resolve_trigger_string);
    }
    if matches!(
        terminal_trigger,
        Some(TerminalTrigger::PanicReleaseRequested | TerminalTrigger::SupervisorLeaseExpired)
    ) {
        *abort_counts.entry("panic").or_insert(0) += 1;
    }
    // Apply the anomaly latch only after stronger worker, cleanup, scheduler,
    // observer, and terminal-QPC errors have been collected. This preserves a
    // more specific primary error while preserving the first attempted
    // anomaly as terminal even after its detail is moved to secondary errors.
    if let Some(summary) = attempted_dispatch_anomaly.terminal_error_summary() {
        record_termination_error(&mut terminal_error, &mut secondary_errors, summary);
    }
    attempted_dispatch_anomaly.append_detail_to(&mut secondary_errors);
    if system_power.system_suspend_terminal()
        && !secondary_errors
            .iter()
            .any(|e| e.contains("system suspend"))
    {
        secondary_errors.push("system suspend requested during worker execution".to_string());
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
    *telemetry_output.lock() = Some(std::mem::take(&mut telemetry.output));
    #[cfg(any(test, feature = "test-support"))]
    record_finalization_event(&operation_trace, "telemetry_export");
    drop(scheduling);
    *priority_acquired.lock() = "off".to_string();
    local_metrics.power_throttling_disabled = false;
    #[cfg(any(test, feature = "test-support"))]
    record_finalization_event(&operation_trace, "scheduling_guards_released");
    #[cfg(any(test, feature = "test-support"))]
    record_finalization_event(&operation_trace, "terminal_metrics_publication");
    // Publish exactly once after terminal ownership cleanup, telemetry export,
    // and scheduling teardown, but before exposing the typed terminal result.
    publish_terminal_metrics(
        &local_metrics,
        metrics,
        QpcTicks::from_raw(terminal_publish_ticks),
    );
    let cleanup_panicked = matches!(&cleanup_result, Err(TerminalCleanupFailure::Panicked(_)));
    let terminal_outcome = if terminal_error.is_some()
        || cleanup_panicked
        || worker_panicked
        || terminal_trigger.is_some_and(TerminalTrigger::requires_error_outcome)
    {
        super::super::super::WorkerTerminalOutcome::Error
    } else if matches!(terminal_trigger, Some(TerminalTrigger::SystemSuspend)) {
        super::super::super::WorkerTerminalOutcome::SystemSuspended
    } else if matches!(terminal_trigger, Some(TerminalTrigger::Skip))
        || skip_requested.load(Ordering::Acquire)
    {
        super::super::super::WorkerTerminalOutcome::Skipped
    } else if matches!(terminal_trigger, Some(TerminalTrigger::Quit))
        || quit_requested.load(Ordering::Acquire)
    {
        super::super::super::WorkerTerminalOutcome::Quit
    } else {
        super::super::super::WorkerTerminalOutcome::Finished
    };
    let terminal_cause = if worker_panicked {
        super::super::super::WorkerTerminalCause::WorkerPanic
    } else if cleanup_panicked {
        super::super::super::WorkerTerminalCause::CleanupPanic
    } else {
        match terminal_trigger {
            Some(TerminalTrigger::SenderAnomaly) => {
                super::super::super::WorkerTerminalCause::SenderAnomaly
            }
            Some(TerminalTrigger::SystemSuspend) => {
                super::super::super::WorkerTerminalCause::SystemSuspended
            }
            Some(TerminalTrigger::Skip) => super::super::super::WorkerTerminalCause::Skipped,
            Some(TerminalTrigger::Quit) => super::super::super::WorkerTerminalCause::Quit,
            Some(TerminalTrigger::WorkerPanic) => {
                super::super::super::WorkerTerminalCause::WorkerPanic
            }
            Some(_) if terminal_error.is_some() => {
                super::super::super::WorkerTerminalCause::Failure
            }
            Some(_) => super::super::super::WorkerTerminalCause::Failure,
            None if terminal_error.is_some() => super::super::super::WorkerTerminalCause::Failure,
            None => super::super::super::WorkerTerminalCause::Finished,
        }
    };
    let residual = super::super::super::TerminalReleaseResidual::from_backend(&backend);
    *metrics.terminal_result.lock() = Some(super::super::super::WorkerTerminalResult {
        outcome: terminal_outcome,
        cause: terminal_cause,
        terminal_error: terminal_error.clone(),
        residual,
    });
    #[cfg(any(test, feature = "test-support"))]
    record_finalization_event(&operation_trace, "terminal_result_publication");
    match (worker_result, cleanup_result) {
        (Err(payload), _) => resume_unwind(payload),
        (Ok(_), Err(TerminalCleanupFailure::Panicked(payload))) => resume_unwind(payload),
        (Ok(_), Ok(_)) => {}
    }
    terminal_outcome.legacy_code()
}

fn terminal_cleanup_target(target: &super::SessionTarget) -> TerminalCleanupTarget {
    let Some((hwnd, generation)) = target.load_stable() else {
        return TerminalCleanupTarget {
            hwnd: target.hwnd_for_safety_release(),
            stamp: None,
            require_owner_identity: target.owner_process_identity_required(),
        };
    };
    TerminalCleanupTarget {
        hwnd,
        stamp: Some(TargetStamp { hwnd, generation }),
        require_owner_identity: target.owner_process_identity_required(),
    }
}

struct TerminalCleanupTarget {
    hwnd: isize,
    stamp: Option<TargetStamp>,
    require_owner_identity: bool,
}

fn describe_terminal_release_evidence(evidence: &TerminalReleaseEvidence) -> String {
    let send = evidence
        .send_attempts
        .iter()
        .map(|attempt| {
            format!(
                "raw_calls={},requested={},inserted={},win32_error={:?}",
                attempt.raw_calls,
                attempt.requested_events,
                attempt.inserted_events,
                attempt.win32_error
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    let qpc = evidence
        .send_attempts
        .iter()
        .map(|attempt| {
            format!(
                "pre={:?},completion={:?},error={:?}",
                attempt.started_ticks, attempt.completed_ticks, attempt.qpc_error
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    format!(
        "send=[{send}], qpc=[{qpc}], physical_probe={:?}",
        evidence.probe_verdict
    )
}

#[cfg(any(test, feature = "test-support"))]
fn record_finalization_event(trace: &Option<Arc<Mutex<Vec<&'static str>>>>, event: &'static str) {
    if let Some(trace) = trace {
        trace.lock().push(event);
    }
}

#[cfg(test)]
mod acceptance_raw_up_count_tests {
    use super::terminal_raw_up_syscall_count;
    use sky_dispatch_win32::input::{
        GuardedReleaseOutcome, ReleaseAllOutcome, TerminalProbeVerdict, TerminalSendAttemptEvidence,
    };
    use smallvec::SmallVec;

    fn evidence(
        raw_calls: u8,
        requested_events: u8,
        inserted_events: u8,
    ) -> TerminalSendAttemptEvidence {
        TerminalSendAttemptEvidence {
            raw_calls,
            requested_events,
            inserted_events,
            win32_error: None,
            started_ticks: None,
            completed_ticks: None,
            qpc_error: None,
        }
    }

    fn cleanup_outcome(
        admitted_attempts: u8,
        send_attempts: Vec<TerminalSendAttemptEvidence>,
        withheld_reason: Option<&str>,
    ) -> GuardedReleaseOutcome {
        GuardedReleaseOutcome {
            release_outcome: ReleaseAllOutcome {
                attempted_mask: 0,
                transport_anomaly: true,
                released_successfully: false,
                stuck_mask: 1,
                verification_inconclusive: true,
                attempts: admitted_attempts,
            },
            send_attempts: SmallVec::from_vec(send_attempts),
            probe_verdict: TerminalProbeVerdict::Inconclusive(1),
            withheld_reason: withheld_reason.map(str::to_owned),
            needs_up_mask: 1,
            needs_probe_mask: 0,
        }
    }

    #[test]
    fn acceptance_raw_up_count_uses_only_recorded_raw_syscalls() {
        let withheld = cleanup_outcome(4, Vec::new(), Some("authority changed"));
        assert_eq!(terminal_raw_up_syscall_count(&withheld), 0);

        let pre_send_failure = cleanup_outcome(1, vec![evidence(0, 1, 0)], None);
        assert_eq!(terminal_raw_up_syscall_count(&pre_send_failure), 0);

        let mut raw_zero_evidence = evidence(1, 1, 0);
        raw_zero_evidence.win32_error = Some(5);
        let raw_zero = cleanup_outcome(1, vec![raw_zero_evidence], None);
        assert_eq!(terminal_raw_up_syscall_count(&raw_zero), 1);

        let mut ambiguous_evidence = evidence(1, 2, 1);
        ambiguous_evidence.win32_error = Some(5);
        let ambiguous_sender_failure = cleanup_outcome(1, vec![ambiguous_evidence], None);
        assert_eq!(terminal_raw_up_syscall_count(&ambiguous_sender_failure), 1);
    }
}
