use super::super::admission::{
    focus_matches_hwnd, safety_cleanup_drain_decision, verify_safety_cleanup_target,
};
use super::super::cleanup::CleanupTarget;
use super::super::config::StartupCheckpoint;
use super::super::shared::{StartupAbortReason, StartupPhase};
use super::super::{
    OUTCOME_ERROR, OUTCOME_QUIT, OUTCOME_SKIPPED, SharedMetrics, TerminalTrigger, TrackedKeyState,
    WaitOptions, describe_release_outcome, release_state_verified, wait_failure_message,
};
use sky_dispatch_win32::input::ReleaseAllOutcome;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;
pub(super) fn record_startup_failure(
    shared: &super::super::shared::SessionShared,
    message: String,
) -> u8 {
    let metrics = &shared.publication.metrics;
    shared
        .commands
        .system_power
        .fail_startup(StartupAbortReason::WorkerFailure);
    *metrics.last_error.lock() = Some(message.clone());
    *metrics.terminal_error.lock() = Some(message.clone());
    *metrics.terminal_trigger_reason.lock() = Some("startup_failure".to_string());
    *metrics.terminal_result.lock() = Some(super::super::super::WorkerTerminalResult {
        outcome: super::super::super::WorkerTerminalOutcome::Error,
        cause: super::super::super::WorkerTerminalCause::Failure,
        terminal_error: Some(message),
        residual: super::super::super::TerminalReleaseResidual::empty(),
    });
    OUTCOME_ERROR
}

fn trigger_for_abort(reason: StartupAbortReason) -> Option<TerminalTrigger> {
    match reason {
        StartupAbortReason::None => None,
        StartupAbortReason::Quit => Some(TerminalTrigger::Quit),
        StartupAbortReason::Skip => Some(TerminalTrigger::Skip),
        StartupAbortReason::PanicRelease => Some(TerminalTrigger::PanicReleaseRequested),
        StartupAbortReason::SupervisorLeaseExpired => Some(TerminalTrigger::SupervisorLeaseExpired),
        StartupAbortReason::SystemSuspend => Some(TerminalTrigger::SystemSuspend),
        StartupAbortReason::FocusInvalidated => Some(TerminalTrigger::FocusAuthorityInvalidated),
        StartupAbortReason::TargetInvalidated => Some(TerminalTrigger::TargetAuthorityInvalidated),
        StartupAbortReason::OwnerInvalidated => Some(TerminalTrigger::OwnerAuthorityInvalidated),
        StartupAbortReason::WorkerFailure => Some(TerminalTrigger::WorkerPanic),
    }
}

pub(super) fn startup_abort_trigger(
    shared: &super::super::shared::SessionShared,
    require_focus: bool,
) -> Option<TerminalTrigger> {
    let startup = shared.commands.system_power.startup_snapshot();
    if startup.phase == StartupPhase::Failed {
        return trigger_for_abort(startup.abort_reason);
    }
    if matches!(
        startup.abort_reason,
        StartupAbortReason::PanicRelease
            | StartupAbortReason::SupervisorLeaseExpired
            | StartupAbortReason::FocusInvalidated
            | StartupAbortReason::TargetInvalidated
            | StartupAbortReason::OwnerInvalidated
            | StartupAbortReason::WorkerFailure
    ) {
        return trigger_for_abort(startup.abort_reason);
    }
    if shared.commands.supervisor_expired.is_expired() {
        return Some(TerminalTrigger::SupervisorLeaseExpired);
    }
    if startup.system_suspend_terminal || startup.abort_reason == StartupAbortReason::SystemSuspend
    {
        return Some(TerminalTrigger::SystemSuspend);
    }
    if startup.phase == StartupPhase::Cancelled {
        return trigger_for_abort(startup.abort_reason);
    }
    if startup.phase == StartupPhase::Armed {
        return None;
    }

    let reason = if shared.commands.skip_requested.load(Ordering::Acquire) {
        Some(StartupAbortReason::Skip)
    } else if shared.commands.quit_requested.load(Ordering::Acquire) {
        Some(StartupAbortReason::Quit)
    } else if shared.commands.panic_requested.load(Ordering::Acquire) {
        Some(StartupAbortReason::PanicRelease)
    } else if shared.commands.system_power.down_blocked() {
        Some(StartupAbortReason::SystemSuspend)
    } else if require_focus {
        match shared.target.load_stable() {
            Some((hwnd, generation)) => reason_for_target_authority(shared, hwnd, generation),
            None => Some(StartupAbortReason::TargetInvalidated),
        }
    } else {
        None
    };
    if let Some(reason) = reason {
        shared.commands.system_power.cancel_prearm(reason);
        let startup = shared.commands.system_power.startup_snapshot();
        return (startup.phase == StartupPhase::Cancelled)
            .then(|| trigger_for_abort(startup.abort_reason))
            .flatten();
    }
    None
}

fn reason_for_target_authority(
    shared: &super::super::shared::SessionShared,
    hwnd: isize,
    generation: u64,
) -> Option<StartupAbortReason> {
    if hwnd == 0
        || generation == 0
        || !shared.commands.focus_active.load(Ordering::Acquire)
        || !focus_matches_hwnd(true, &shared.commands.focus_active, hwnd)
    {
        return Some(StartupAbortReason::FocusInvalidated);
    }
    if !matches!(
        shared.target.owner_identity_status(generation),
        super::super::target::OwnerIdentityStatus::NotRequired
            | super::super::target::OwnerIdentityStatus::Bound(_)
    ) {
        return Some(StartupAbortReason::OwnerInvalidated);
    }
    None
}

pub(super) fn record_startup_abort_without_backend(
    shared: &super::super::shared::SessionShared,
    trigger: TerminalTrigger,
    message: String,
) -> u8 {
    let metrics = &shared.publication.metrics;
    *metrics.terminal_trigger_reason.lock() = Some(trigger.as_str().to_string());
    if trigger.requires_error_outcome() {
        *metrics.last_error.lock() = Some(message.clone());
    }
    *metrics.terminal_error.lock() = None;
    let outcome = startup_outcome(trigger);
    let typed_outcome = match outcome {
        OUTCOME_QUIT => super::super::super::WorkerTerminalOutcome::Quit,
        OUTCOME_SKIPPED => super::super::super::WorkerTerminalOutcome::Skipped,
        super::super::super::OUTCOME_SYSTEM_SUSPENDED => {
            super::super::super::WorkerTerminalOutcome::SystemSuspended
        }
        _ => super::super::super::WorkerTerminalOutcome::Error,
    };
    *metrics.terminal_result.lock() = Some(super::super::super::WorkerTerminalResult {
        outcome: typed_outcome,
        cause: if trigger.requires_error_outcome() {
            super::super::super::WorkerTerminalCause::Failure
        } else if matches!(trigger, TerminalTrigger::Quit) {
            super::super::super::WorkerTerminalCause::Quit
        } else if matches!(trigger, TerminalTrigger::Skip) {
            super::super::super::WorkerTerminalCause::Skipped
        } else {
            super::super::super::WorkerTerminalCause::SystemSuspended
        },
        terminal_error: trigger.requires_error_outcome().then_some(message),
        residual: super::super::super::TerminalReleaseResidual::empty(),
    });
    outcome
}

fn startup_outcome(trigger: TerminalTrigger) -> u8 {
    if trigger.requires_error_outcome() {
        OUTCOME_ERROR
    } else if matches!(trigger, TerminalTrigger::SystemSuspend) {
        super::super::super::OUTCOME_SYSTEM_SUSPENDED
    } else if matches!(trigger, TerminalTrigger::Skip) {
        OUTCOME_SKIPPED
    } else if matches!(trigger, TerminalTrigger::Quit) {
        OUTCOME_QUIT
    } else {
        OUTCOME_ERROR
    }
}

fn release_startup_tracked_obligations(
    backend: &mut TrackedKeyState,
    cleanup_target: CleanupTarget<'_>,
) -> ReleaseAllOutcome {
    let session_release_owner = backend.session_release_owner_proof().cloned();
    backend
        .release_terminal_tracked_scope_with_attempt_admission_and_drain_control(
            cleanup_target.verification_hwnd,
            |_, release_owner| {
                let stamp = cleanup_target.expected_target.ok_or_else(|| {
                    "target HWND/generation unavailable during startup cleanup".to_string()
                })?;
                verify_safety_cleanup_target(
                    cleanup_target.target,
                    stamp,
                    release_owner,
                    cleanup_target.require_owner_identity,
                )
            },
            || {
                let stamp = cleanup_target.expected_target.ok_or_else(|| {
                    "target HWND/generation unavailable during startup cleanup".to_string()
                })?;
                let release_owner = session_release_owner.as_ref().ok_or_else(|| {
                    "session release owner proof is missing during startup cleanup".to_string()
                })?;
                safety_cleanup_drain_decision(
                    cleanup_target.target,
                    stamp,
                    release_owner,
                    cleanup_target.require_owner_identity,
                    true,
                )
            },
        )
        .release_outcome
}

pub(super) fn finalize_startup_abort_with_origin(
    backend: &mut TrackedKeyState,
    metrics: &SharedMetrics,
    cleanup_target: CleanupTarget<'_>,
    trigger: TerminalTrigger,
    primary_error: String,
) -> u8 {
    finalize_startup_abort_with_cleanup(backend, metrics, trigger, primary_error, |backend| {
        release_startup_tracked_obligations(backend, cleanup_target)
    })
}

fn finalize_startup_abort_with_cleanup(
    backend: &mut TrackedKeyState,
    metrics: &SharedMetrics,
    trigger: TerminalTrigger,
    primary_error: String,
    cleanup: impl FnOnce(&mut TrackedKeyState) -> ReleaseAllOutcome,
) -> u8 {
    let cleanup_result = catch_unwind(AssertUnwindSafe(|| cleanup(backend)));
    let cleanup_verified = cleanup_result
        .as_ref()
        .is_ok_and(|cleanup| release_state_verified(backend, cleanup));
    let cleanup_error = match &cleanup_result {
        Ok(_) if cleanup_verified => None,
        Ok(cleanup) => Some(format!(
            "startup cancellation cleanup failed: {}",
            describe_release_outcome(backend, cleanup)
        )),
        Err(_) => Some(format!(
            "startup cancellation cleanup panicked with obligation mask {:#06x}",
            backend.release_obligation_mask()
        )),
    };
    if let Ok(cleanup) = &cleanup_result {
        *metrics.terminal_release_outcome.lock() = Some(cleanup.clone());
    }
    metrics.terminal_release_obligation_mask.store(
        u32::from(backend.release_obligation_mask()),
        Ordering::Release,
    );
    metrics
        .terminal_release_attempts_total
        .store(backend.release_attempts_total, Ordering::Release);
    *metrics.terminal_trigger_reason.lock() = Some(trigger.as_str().to_string());
    *metrics.terminal_error.lock() = cleanup_error.clone();
    *metrics.secondary_errors.lock() = Vec::new();
    let (outcome, terminal_error) = if let Some(cleanup_error) = cleanup_error {
        *metrics.last_error.lock() = Some(format!("{primary_error}; {cleanup_error}"));
        (OUTCOME_ERROR, Some(cleanup_error))
    } else if trigger.requires_error_outcome() {
        *metrics.last_error.lock() = Some(primary_error.clone());
        (OUTCOME_ERROR, Some(primary_error))
    } else {
        (startup_outcome(trigger), None)
    };
    let typed_outcome = match outcome {
        OUTCOME_QUIT => super::super::super::WorkerTerminalOutcome::Quit,
        OUTCOME_SKIPPED => super::super::super::WorkerTerminalOutcome::Skipped,
        super::super::super::OUTCOME_SYSTEM_SUSPENDED => {
            super::super::super::WorkerTerminalOutcome::SystemSuspended
        }
        _ => super::super::super::WorkerTerminalOutcome::Error,
    };
    let cause = if cleanup_result.is_err() {
        super::super::super::WorkerTerminalCause::CleanupPanic
    } else if terminal_error.is_some() {
        super::super::super::WorkerTerminalCause::Failure
    } else if matches!(trigger, TerminalTrigger::Quit) {
        super::super::super::WorkerTerminalCause::Quit
    } else if matches!(trigger, TerminalTrigger::Skip) {
        super::super::super::WorkerTerminalCause::Skipped
    } else if matches!(trigger, TerminalTrigger::SystemSuspend) {
        super::super::super::WorkerTerminalCause::SystemSuspended
    } else {
        super::super::super::WorkerTerminalCause::Failure
    };
    *metrics.terminal_result.lock() = Some(super::super::super::WorkerTerminalResult {
        outcome: typed_outcome,
        cause,
        terminal_error,
        residual: super::super::super::TerminalReleaseResidual::from_backend(backend),
    });
    outcome
}

pub(super) fn startup_checkpoint(
    shared: &super::super::shared::SessionShared,
    require_focus: bool,
    checkpoint: StartupCheckpoint,
    #[cfg(any(test, feature = "test-support"))] hook: Option<
        &super::super::config::StartupOrderingHook,
    >,
) -> Result<(), String> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = hook {
        hook.visit_startup_checkpoint(checkpoint);
    }
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = checkpoint;
    if let Some(trigger) = startup_abort_trigger(shared, require_focus) {
        return Err(format!("startup cancelled: {}", trigger.as_str()));
    }
    Ok(())
}

fn finalize_admission_failure_with_cleanup(
    backend: &mut TrackedKeyState,
    metrics: &SharedMetrics,
    primary_error: String,
    cleanup: impl FnOnce(&mut TrackedKeyState) -> ReleaseAllOutcome,
) -> u8 {
    let cleanup_result = catch_unwind(AssertUnwindSafe(|| cleanup(backend)));
    let cleanup_verified = cleanup_result
        .as_ref()
        .is_ok_and(|cleanup| release_state_verified(backend, cleanup));
    let cleanup_error = match &cleanup_result {
        Ok(_) if cleanup_verified => None,
        Ok(cleanup) => Some(format!(
            "admission cleanup failed: {}",
            describe_release_outcome(backend, cleanup)
        )),
        Err(_) => Some(format!(
            "admission cleanup panicked with obligation mask {:#06x}",
            backend.release_obligation_mask()
        )),
    };
    if let Ok(cleanup) = &cleanup_result {
        *metrics.terminal_release_outcome.lock() = Some(cleanup.clone());
    }
    metrics.terminal_release_obligation_mask.store(
        u32::from(backend.release_obligation_mask()),
        Ordering::Release,
    );
    metrics
        .terminal_release_attempts_total
        .store(backend.release_attempts_total, Ordering::Release);
    *metrics.terminal_error.lock() = Some(primary_error.clone());
    *metrics.terminal_trigger_reason.lock() = Some("startup_failure".to_string());
    *metrics.secondary_errors.lock() = cleanup_error.iter().cloned().collect();
    let message = cleanup_error
        .map(|cleanup_error| format!("{primary_error}; {cleanup_error}"))
        .unwrap_or(primary_error);
    *metrics.last_error.lock() = Some(message.clone());
    *metrics.terminal_result.lock() = Some(super::super::super::WorkerTerminalResult {
        outcome: super::super::super::WorkerTerminalOutcome::Error,
        cause: if cleanup_result.is_err() {
            super::super::super::WorkerTerminalCause::CleanupPanic
        } else {
            super::super::super::WorkerTerminalCause::Failure
        },
        terminal_error: Some(message),
        residual: super::super::super::TerminalReleaseResidual::from_backend(backend),
    });
    OUTCOME_ERROR
}

pub(super) fn finalize_admission_failure_with_origin(
    backend: &mut TrackedKeyState,
    metrics: &SharedMetrics,
    cleanup_target: CleanupTarget<'_>,
    primary_error: String,
) -> u8 {
    finalize_admission_failure_with_cleanup(backend, metrics, primary_error, |backend| {
        release_startup_tracked_obligations(backend, cleanup_target)
    })
}

#[cfg(test)]
pub(super) fn finalize_admission_failure(
    backend: &mut TrackedKeyState,
    metrics: &SharedMetrics,
    verification_hwnd: isize,
    primary_error: String,
) -> u8 {
    finalize_admission_failure_with_cleanup(backend, metrics, primary_error, |backend| {
        backend.release_all(verification_hwnd)
    })
}

pub(super) fn validate_production_wait_backend(
    backend_is_production: bool,
    options: &WaitOptions,
    initial_failure: Option<sky_dispatch_win32::wait::WaitFailure>,
) -> Result<(), String> {
    if !backend_is_production {
        return Ok(());
    }
    if !options.enable_waitable_timer {
        return Err("production wait backend requires the high-resolution waitable timer".into());
    }
    if !options.enable_event_wait {
        return Err("production wait backend requires event wait to be enabled".into());
    }
    if let Some(failure) = initial_failure {
        return Err(wait_failure_message(failure));
    }
    Ok(())
}
