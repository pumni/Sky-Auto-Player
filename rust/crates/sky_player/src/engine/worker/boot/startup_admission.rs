use super::super::admission::focus_matches_hwnd;
use super::super::config::StartupCheckpoint;
use super::super::shared::{StartupAbortReason, StartupPhase};
use super::super::{
    DurationTicks, OUTCOME_ERROR, OUTCOME_QUIT, OUTCOME_SKIPPED, QpcClock, QpcTicks, SharedMetrics,
    TerminalTrigger, TrackedKeyState, WaitOptions, describe_release_outcome,
    release_state_verified, wait_failure_message,
};
use crate::engine::config::{CALIBRATION_MAX_STARTUP_BUDGET_US, CALIBRATION_SAMPLES};
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
    *metrics.terminal_error.lock() = Some(message);
    *metrics.terminal_trigger_reason.lock() = Some("startup_failure".to_string());
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
    if startup.phase == StartupPhase::Cancelled {
        return trigger_for_abort(startup.abort_reason);
    }
    if matches!(startup.phase, StartupPhase::Armed | StartupPhase::Failed) {
        return None;
    }

    let reason = if shared.commands.supervisor_expired.is_expired() {
        Some(StartupAbortReason::SupervisorLeaseExpired)
    } else if shared.commands.skip_requested.load(Ordering::Acquire) {
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
    startup_outcome(trigger)
}

fn startup_outcome(trigger: TerminalTrigger) -> u8 {
    if trigger.requires_error_outcome() {
        OUTCOME_ERROR
    } else if matches!(trigger, TerminalTrigger::Skip) {
        OUTCOME_SKIPPED
    } else if matches!(trigger, TerminalTrigger::Quit) {
        OUTCOME_QUIT
    } else {
        OUTCOME_ERROR
    }
}

pub(super) fn finalize_startup_abort(
    backend: &mut TrackedKeyState,
    metrics: &SharedMetrics,
    verification_hwnd: isize,
    trigger: TerminalTrigger,
    primary_error: String,
) -> u8 {
    let cleanup_result = catch_unwind(AssertUnwindSafe(|| backend.release_all(verification_hwnd)));
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
    if let Some(cleanup_error) = cleanup_error {
        *metrics.last_error.lock() = Some(format!("{primary_error}; {cleanup_error}"));
        OUTCOME_ERROR
    } else if trigger.requires_error_outcome() {
        *metrics.last_error.lock() = Some(primary_error);
        OUTCOME_ERROR
    } else {
        startup_outcome(trigger)
    }
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

pub(super) fn finalize_admission_failure(
    backend: &mut TrackedKeyState,
    metrics: &SharedMetrics,
    verification_hwnd: isize,
    primary_error: String,
) -> u8 {
    let cleanup_result = catch_unwind(AssertUnwindSafe(|| backend.release_all(verification_hwnd)));
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
    *metrics.last_error.lock() = Some(message);
    OUTCOME_ERROR
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

pub(super) fn startup_wake_probe(
    waiter: &sky_dispatch_win32::wait::HybridWaiter,
    qpc_clock: QpcClock,
    interrupt: &sky_dispatch_win32::event::OwnedEvent,
    initial_now_ticks: QpcTicks,
) -> Option<sky_dispatch_win32::wait::WakeErrorStats> {
    let budget_ticks = qpc_clock
        .duration_from_us(CALIBRATION_MAX_STARTUP_BUDGET_US)
        .ok()?;
    let probe_deadline_ticks = startup_probe_deadline(initial_now_ticks, budget_ticks)?;
    waiter.probe_wake_error_stats_until(
        qpc_clock,
        interrupt,
        CALIBRATION_SAMPLES,
        Some(probe_deadline_ticks),
    )
}

pub(super) fn startup_probe_deadline(
    initial_now_ticks: QpcTicks,
    calibration_budget_ticks: DurationTicks,
) -> Option<QpcTicks> {
    initial_now_ticks
        .checked_add_duration(calibration_budget_ticks)
        .ok()
}
