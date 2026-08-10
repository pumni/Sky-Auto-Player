use super::super::{
    Duration, DurationTicks, QpcError, TimelineTicks, WaitFailure, WaitOutcome, try_publish_metrics,
};
use super::wait::WaitObservation;
use super::{
    CommandControl, CommandControlClock, CommandControlInput, CommandControlMetrics,
    CommandControlRuntime, CommandControlSignals, PlanningInput, WaitBoundary, WaitBoundaryInput,
    WaitDeadline, WaitMutable, WaitSignals, WaitTiming, Worker,
    anchored_dispatch_target_ticks_typed, ensure_preflight_for_target, focus_matches,
    focus_matches_hwnd, lease_bounded_ticks, load_target_stamp, plan_next_dispatch_projected,
    plan_structure_is_valid, process_command_control, publish_backend_metrics,
    record_termination_error, suspend_live_input, target_stamp_still_current, wait_failure_message,
    wait_for_next_boundary,
};
use smallvec::SmallVec;
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PhysicalWork {
    Pending,
    Authored,
}

fn physical_target_qpc_for_work(
    plan: &super::planning::NextDispatchPlan,
    epoch: sky_dispatch_win32::clock::QpcTicks,
    now: sky_dispatch_win32::clock::QpcTicks,
    startup_dispatch_target_qpc: Option<sky_dispatch_win32::clock::QpcTicks>,
    work: PhysicalWork,
) -> Result<Option<sky_dispatch_win32::clock::QpcTicks>, String> {
    let deadline = match work {
        PhysicalWork::Pending => plan.pending.as_ref().map(|pending| pending.deadline_ticks),
        PhysicalWork::Authored => plan
            .authored
            .as_ref()
            .map(|authored| authored.deadline_ticks),
    };
    let Some(deadline) = deadline else {
        return Ok(None);
    };
    let target = match (work, startup_dispatch_target_qpc) {
        (PhysicalWork::Authored, Some(startup_target)) => startup_target,
        _ => epoch
            .checked_add_duration(DurationTicks::from_raw(deadline.as_u64()))
            .map_err(|error| format!("physical target arithmetic failure: {error}"))?,
    };
    Ok((target <= now).then_some(target))
}

/// Dispatch the work represented by one immutable plan.  This helper is used
/// for both an already-due plan and a successful blocking deadline wake, so a
/// normal timer wake never re-enters general orchestration before transport.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_due_from_plan(
    plan: &super::planning::NextDispatchPlan,
    effective_now_ticks: TimelineTicks,
    now_ticks: sky_dispatch_win32::clock::QpcTicks,
    epoch_qpc: sky_dispatch_win32::clock::QpcTicks,
    focus_loss_fault: bool,
    config: &super::WorkerConfig,
    resources: &mut super::WorkerResources,
    health: &mut super::WorkerHealthState,
    timing: &super::WorkerTimingState,
    runtime: &mut super::WorkerRuntime,
    local_metrics: &mut super::WorkerMetricsLocal,
    last_published_error: &mut Option<String>,
    secondary_errors: &mut Vec<String>,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    supervisor_heartbeat_ticks: &AtomicU64,
    lease_timeout_ticks: DurationTicks,
    metrics: &crate::engine::telemetry::SharedMetrics,
    progress_clock: &crate::engine::shared::SharedProgressClock,
    observer: &mut super::dispatch::PendingObservationQueue,
) -> super::DispatchStep {
    if !plan_structure_is_valid(plan) {
        return super::DispatchStep::Terminate(
            "dispatch plan has inconsistent physical-work budgets".to_string(),
        );
    }
    let pending_plan = plan.pending.as_ref();
    let lead_up_ticks = pending_plan.map_or(DurationTicks::ZERO, |pending| pending.lead_ticks);
    let due_pending = match pending_plan {
        Some(pending) => match resources
            .coordinator
            .pop_due_pending_ticks(effective_now_ticks, pending)
        {
            Ok(due) => due,
            Err(error) => {
                return super::DispatchStep::Terminate(format!(
                    "coordinator pending-pop failure: {error}"
                ));
            }
        },
        None => SmallVec::new(),
    };
    if !due_pending.is_empty() {
        let physical_target_qpc = match physical_target_qpc_for_work(
            plan,
            epoch_qpc,
            now_ticks,
            None,
            PhysicalWork::Pending,
        ) {
            Ok(Some(target)) => target,
            Ok(None) => return super::DispatchStep::NoWork,
            Err(error) => return super::DispatchStep::Terminate(error),
        };
        let Some(frozen_budget) = plan.pending_budget.as_ref() else {
            return super::DispatchStep::Terminate(
                "pending dispatch plan has no health budget".to_string(),
            );
        };
        match super::dispatch_due_pending_releases(
            super::PendingReleaseContext {
                due_pending,
                pending_plan,
                lead_up_ticks,
                physical_target_qpc,
                frozen_budget: *frozen_budget,
                quit_requested,
                skip_requested,
                panic_requested,
                desired_pause,
                supervisor_heartbeat_ticks,
                lease_timeout_ticks,
                observer,
            },
            config,
            resources,
            health,
            timing,
            runtime,
            local_metrics,
            secondary_errors,
            target_hwnd,
        ) {
            super::DispatchStep::Dispatched => return super::DispatchStep::Dispatched,
            super::DispatchStep::Continue => return super::DispatchStep::Continue,
            super::DispatchStep::NoWork => {}
            super::DispatchStep::Terminate(error) => {
                return super::DispatchStep::Terminate(error);
            }
        }
    }

    if plan.authored.is_none() {
        return super::DispatchStep::NoWork;
    }
    if plan.authored_budget.is_none() {
        return super::DispatchStep::Terminate(
            "authored dispatch plan has no health budget".to_string(),
        );
    }
    /* stale authored metadata is drained by the outer pre-precision loop */
    let startup_target_selected = runtime.startup_dispatch_target_qpc.is_some();
    let physical_target_qpc = match physical_target_qpc_for_work(
        plan,
        epoch_qpc,
        now_ticks,
        runtime.startup_dispatch_target_qpc,
        PhysicalWork::Authored,
    ) {
        Ok(Some(target)) => target,
        Ok(None) => return super::DispatchStep::NoWork,
        Err(error) => return super::DispatchStep::Terminate(error),
    };

    super::dispatch_authored_packet(
        super::AuthoredPacketContext {
            dispatch_plan: plan,
            effective_now_ticks,
            now_ticks,
            physical_target_qpc,
            startup_target_selected,
            focus_loss_fault,
            supervisor_heartbeat_ticks,
            lease_timeout_ticks,
        },
        config,
        resources,
        health,
        timing,
        runtime,
        local_metrics,
        last_published_error,
        focus_active,
        target_hwnd,
        target_generation,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        metrics,
        progress_clock,
        observer,
    )
}

pub(super) fn dispatch(
    worker: &mut Worker<'_>,
    focus_loss_fault: bool,
) -> Result<(), Box<dyn Any + Send>> {
    let shared = worker.shared;
    let core = &mut worker.core;
    let config = &worker.config;
    let interrupt = &shared.commands.interrupt;
    let desired_pause = &shared.commands.desired_pause;
    let quit_requested = &shared.commands.quit_requested;
    let skip_requested = &shared.commands.skip_requested;
    let panic_requested = &shared.commands.panic_requested;
    let focus_active = &shared.commands.focus_active;
    let target_hwnd = &shared.target.target_hwnd;
    let target_generation = &shared.target.target_generation;
    let metrics = &shared.publication.metrics;
    let supervisor_heartbeat_ticks = &shared.publication.supervisor_heartbeat_ticks;
    #[cfg(any(test, feature = "test-support"))]
    let command_timing = &shared.commands.command_timing;

    let mut timing = *core.timing.as_ref().expect("worker timing initialized");
    let qpc_clock = core
        .resources
        .as_ref()
        .expect("worker resources initialized")
        .clock;

    // Every QPC query after admission is part of the worker's correctness
    // boundary. A failed query is terminal and must take the cleanup path;
    // it must never become timestamp zero or a best-effort continuation.
    macro_rules! qpc_us_or_terminal {
        () => {{
            match qpc_clock.now().and_then(|ticks| {
                qpc_clock
                    .duration_to_us(DurationTicks::from_raw(ticks.as_u64()))
                    .map_err(|_| QpcError::ConversionOverflow)
            }) {
                Ok(value) => value,
                Err(error) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(format!("QPC runtime failure: {error:?}"));
                    break;
                }
            }
        }};
    }

    macro_rules! qpc_ticks_or_terminal {
        () => {{
            match qpc_clock.now() {
                Ok(value) => value,
                Err(error) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(format!("QPC runtime failure: {error:?}"));
                    break;
                }
            }
        }};
    }

    catch_unwind(AssertUnwindSafe(|| {
        if core.runtime.terminal_error.is_some() {
            return;
        }
        let resources = core
            .resources
            .as_mut()
            .expect("worker resources initialized");
        let qpc_clock = resources.clock;
        while !resources.coordinator.is_finished() {
            // A deadline-wake sample belongs to exactly one physical send.
            // Re-entering the non-precision loop clears any stale sample
            // after interrupts, replans, command transitions, or failures.
            core.runtime.last_dispatch_deadline_wake_qpc = None;
            let loop_start_ticks = qpc_ticks_or_terminal!();
            if let CommandControl::Exit = process_command_control(CommandControlInput {
                clock: CommandControlClock {
                    loop_start_ticks,
                    qpc_clock,
                    lease_timeout_ticks: timing.lease_timeout_ticks,
                    supervisor_heartbeat_ticks,
                },
                signals: CommandControlSignals {
                    quit_requested,
                    skip_requested,
                    panic_requested,
                    target_hwnd,
                },
                runtime: CommandControlRuntime {
                    backend: &mut resources.backend,
                    coordinator: &mut resources.coordinator,
                    force_full_cleanup: &mut core.runtime.force_full_cleanup,
                    terminal_error: &mut core.runtime.terminal_error,
                    secondary_errors: &mut core.errors.secondary,
                    abort_counts: &mut core.errors.abort_counts,
                },
                metrics: CommandControlMetrics {
                    local_metrics: &mut core.metrics,
                    metrics,
                    last_published_error: &mut core.errors.last_published,
                },
            }) {
                break;
            }

            let mut now_ticks = qpc_ticks_or_terminal!();
            let focus_ok = focus_matches(config.focus.require_focus, focus_active);
            let manual_pause = desired_pause.load(Ordering::Acquire);
            #[cfg(any(test, feature = "test-support"))]
            if command_timing.needs_observation() {
                let observed_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("QPC pause observation failed: {error:?}"));
                        break;
                    }
                };
                command_timing.observe_pause(observed_ticks);
            }

            if !focus_ok {
                core.runtime.startup_dispatch_target_qpc = None;
                core.runtime.verified_target = None;
                core.runtime.focus_restore_started_ticks = None;
                if !resources.playback.has_pause_reason("focus") {
                    core.runtime.verified_target = None;
                    if let Err(error) = suspend_live_input(
                        &mut resources.backend,
                        &mut resources.coordinator,
                        target_hwnd.load(Ordering::Acquire),
                    ) {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("focus suspension failed: {error}"));
                        break;
                    }
                    *core.errors.abort_counts.entry("focus_lost").or_insert(0) += 1;
                    if let Err(error) = resources.playback.enter_pause("focus", now_ticks) {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("playback clock failure: {error}"));
                        break;
                    }
                    shared
                        .publication
                        .progress_clock
                        .publish(&resources.playback);
                    publish_backend_metrics(
                        &resources.backend,
                        &mut core.metrics,
                        metrics,
                        &mut core.errors.last_published,
                    );
                    try_publish_metrics(&core.metrics, metrics, qpc_us_or_terminal!(), true);
                }
            } else if resources.playback.has_pause_reason("focus") {
                let restored_at = *core
                    .runtime
                    .focus_restore_started_ticks
                    .get_or_insert(now_ticks);
                let focus_grace_elapsed = match now_ticks.checked_duration_since(restored_at) {
                    Ok(elapsed) => elapsed,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("focus grace clock failure: {error}"));
                        break;
                    }
                };
                if focus_grace_elapsed >= timing.focus_restore_grace_ticks {
                    let preflight_target = load_target_stamp(target_hwnd, target_generation);
                    core.runtime.verified_target = None;
                    if let Err(error) = suspend_live_input(
                        &mut resources.backend,
                        &mut resources.coordinator,
                        preflight_target.hwnd,
                    ) {
                        core.runtime.startup_dispatch_target_qpc = None;
                        core.runtime.verified_target = None;
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("focus restoration failed: {error}"));
                        break;
                    }
                    if let Err(error) = ensure_preflight_for_target(
                        &resources.backend,
                        preflight_target,
                        &mut core.runtime.verified_target,
                    ) {
                        core.runtime.verified_target = None;
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error = Some(format!(
                            "instrument key preflight failed during focus restoration; release the 15 instrument keys before playback: {error}"
                        ));
                        break;
                    }
                    if !focus_matches_hwnd(
                        config.focus.require_focus,
                        focus_active,
                        preflight_target.hwnd,
                    ) || !target_stamp_still_current(
                        target_hwnd,
                        target_generation,
                        preflight_target,
                    ) {
                        core.runtime.startup_dispatch_target_qpc = None;
                        core.runtime.verified_target = None;
                        core.runtime.focus_restore_started_ticks = None;
                        continue;
                    }
                    let resumed_ticks = qpc_ticks_or_terminal!();
                    if let Err(error) = resources.playback.exit_pause("focus", resumed_ticks) {
                        core.runtime.verified_target = None;
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("playback clock failure: {error}"));
                        break;
                    }
                    shared
                        .publication
                        .progress_clock
                        .publish(&resources.playback);
                    if desired_pause.load(Ordering::Acquire) {
                        core.runtime.startup_dispatch_target_qpc = None;
                        core.runtime.verified_target = None;
                    }
                    core.runtime.focus_restore_started_ticks = None;
                    publish_backend_metrics(
                        &resources.backend,
                        &mut core.metrics,
                        metrics,
                        &mut core.errors.last_published,
                    );
                    try_publish_metrics(&core.metrics, metrics, qpc_us_or_terminal!(), true);
                }
            }

            if manual_pause && !resources.playback.has_pause_reason("manual") {
                core.runtime.startup_dispatch_target_qpc = None;
                core.runtime.verified_target = None;
                if !resources.playback.is_paused() {
                    if let Err(error) = suspend_live_input(
                        &mut resources.backend,
                        &mut resources.coordinator,
                        target_hwnd.load(Ordering::Acquire),
                    ) {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("manual pause suspension failed: {error}"));
                        break;
                    }
                    *core.errors.abort_counts.entry("manual_pause").or_insert(0) += 1;
                    publish_backend_metrics(
                        &resources.backend,
                        &mut core.metrics,
                        metrics,
                        &mut core.errors.last_published,
                    );
                    try_publish_metrics(&core.metrics, metrics, qpc_us_or_terminal!(), true);
                }
                if let Err(error) = resources.playback.enter_pause("manual", now_ticks) {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(format!("playback clock failure: {error}"));
                    break;
                }
                shared
                    .publication
                    .progress_clock
                    .publish(&resources.playback);
            } else if !manual_pause && resources.playback.has_pause_reason("manual") {
                if !resources.playback.has_pause_reason("focus") {
                    let preflight_target = load_target_stamp(target_hwnd, target_generation);
                    if let Err(error) = ensure_preflight_for_target(
                        &resources.backend,
                        preflight_target,
                        &mut core.runtime.verified_target,
                    ) {
                        core.runtime.startup_dispatch_target_qpc = None;
                        core.runtime.verified_target = None;
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error = Some(format!(
                            "instrument key preflight failed on manual resume; release the 15 instrument keys before playback: {error}"
                        ));
                        break;
                    }
                    if !focus_matches_hwnd(
                        config.focus.require_focus,
                        focus_active,
                        preflight_target.hwnd,
                    ) || !target_stamp_still_current(
                        target_hwnd,
                        target_generation,
                        preflight_target,
                    ) {
                        core.runtime.startup_dispatch_target_qpc = None;
                        core.runtime.verified_target = None;
                        continue;
                    }
                    let resumed_ticks = qpc_ticks_or_terminal!();
                    if let Err(error) = resources.playback.exit_pause("manual", resumed_ticks) {
                        core.runtime.verified_target = None;
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("playback clock failure: {error}"));
                        break;
                    }
                    shared
                        .publication
                        .progress_clock
                        .publish(&resources.playback);
                } else {
                    core.runtime.verified_target = None;
                }
            }

            #[cfg(any(test, feature = "test-support"))]
            if resources.playback.has_pause_reason("manual")
                && command_timing.needs_acknowledgment()
            {
                let acknowledged_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("QPC pause acknowledgment failed: {error:?}"));
                        break;
                    }
                };
                command_timing.acknowledge_pause(acknowledged_ticks);
            }

            let paused = resources.playback.is_paused();
            metrics.is_paused.store(paused, Ordering::Relaxed);
            if paused {
                let pause_target = match now_ticks.checked_add_duration(timing.paused_poll_ticks) {
                    Ok(target) => target,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("pause deadline arithmetic failure: {error}"));
                        break;
                    }
                };
                let pause_target = match lease_bounded_ticks(
                    pause_target,
                    timing.lease_timeout_ticks,
                    supervisor_heartbeat_ticks,
                ) {
                    Ok(target) => target,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("pause lease deadline failure: {error:?}"));
                        break;
                    }
                };
                if let WaitOutcome::Failed(failure) = resources
                    .waiter
                    .wait_until_ticks_with_metrics_typed(
                        qpc_clock,
                        pause_target,
                        DurationTicks::ZERO,
                        interrupt,
                    )
                    .outcome
                {
                    if matches!(failure, WaitFailure::Clock) {
                        core.metrics.wait_clock_failures =
                            core.metrics.wait_clock_failures.saturating_add(1);
                    } else {
                        core.metrics.wait_backend_failures =
                            core.metrics.wait_backend_failures.saturating_add(1);
                    }
                    if config.timing.strict_timing || matches!(failure, WaitFailure::Clock) {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error = Some(wait_failure_message(failure));
                        break;
                    }
                    std::thread::sleep(Duration::from_micros(500));
                }
                continue;
            }

            // Startup metadata is drained before precision planning and wait.
            // Commit at most one compiled packet per outer iteration so all
            // control, focus, and lease gates are re-admitted between packets.
            if matches!(
                core.runtime.startup_precision_phase,
                super::StartupPrecisionPhase::PrePrecision
            ) {
                match resources.coordinator.prepare_current_stale_packet() {
                    Ok(Some(prepared)) => {
                        match super::dispatch_stale_packet(
                            prepared,
                            &mut resources.coordinator,
                            &mut resources.telemetry,
                            TimelineTicks::ZERO,
                        ) {
                            super::DispatchStep::Dispatched => {
                                #[cfg(any(test, feature = "test-support"))]
                                if let Some(hook) = core.runtime.startup_ordering_hook.as_ref() {
                                    hook.mark_stale_packet_committed();
                                }
                                continue;
                            }
                            super::DispatchStep::Terminate(error) => {
                                core.runtime.force_full_cleanup = true;
                                core.runtime.terminal_error = Some(error);
                                break;
                            }
                            super::DispatchStep::Continue | super::DispatchStep::NoWork => {
                                core.runtime.force_full_cleanup = true;
                                core.runtime.terminal_error = Some(
                                    "stale packet did not complete its metadata commit".to_string(),
                                );
                                break;
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error = Some(format!(
                            "coordinator stale-packet preparation failure: {error}"
                        ));
                        break;
                    }
                }
            }

            // Freeze the physical plan before the precision wait.  Once that
            // wait succeeds, the handoff below may perform only final safety
            // admission and physical transport.
            let mut precision_plan = if matches!(
                core.runtime.startup_precision_phase,
                super::StartupPrecisionPhase::PrePrecision
            ) && core.runtime.startup_gate.is_some()
            {
                Some(
                    match plan_next_dispatch_projected(PlanningInput {
                        coordinator: &resources.coordinator,
                        estimator: &resources.estimator,
                        qpc_clock,
                        timing: &config.timing,
                        health_options: core
                            .health
                            .as_ref()
                            .expect("worker health initialized")
                            .options,
                        enable_dispatch_cost_lead: config.estimator.enable_dispatch_cost_lead,
                    }) {
                        Ok(plan) => plan,
                        Err(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error =
                                Some(format!("startup planning failure: {error}"));
                            break;
                        }
                    },
                )
            } else {
                None
            };

            if let Some(startup_gate) = core.runtime.startup_gate {
                if !matches!(
                    core.runtime.startup_precision_phase,
                    super::StartupPrecisionPhase::PrePrecision
                ) {
                    core.runtime.startup_gate = None;
                } else {
                    let current_physical = resources.coordinator.next_physical_authored_packet();
                    if current_physical
                        != Some((
                            startup_gate.scheduled_ticks,
                            startup_gate.up_mask,
                            startup_gate.down_mask,
                        ))
                    {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error = Some(
                            "startup gate no longer identifies the current physical packet"
                                .to_string(),
                        );
                        break;
                    }
                    let Some(plan) = precision_plan.as_ref() else {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some("startup gate has no frozen physical dispatch plan".to_string());
                        break;
                    };
                    if plan.pending.is_some()
                        || plan
                            .authored
                            .as_ref()
                            .map(|authored| authored.path)
                            .is_none()
                    {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error = Some(
                            "startup precision plan contains non-authored physical work"
                                .to_string(),
                        );
                        break;
                    }
                    if plan
                        .authored
                        .as_ref()
                        .is_some_and(|authored| authored.lead_ticks != startup_gate.lead_ticks)
                    {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some("startup gate lead differs from frozen physical plan".to_string());
                        break;
                    }
                }
            }

            if matches!(
                core.runtime.startup_precision_phase,
                super::StartupPrecisionPhase::PrePrecision
            ) && core.runtime.startup_gate.is_some()
            {
                let startup_gate = core.runtime.startup_gate.expect("startup gate checked");
                let target_sample_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("QPC failure before startup wait: {error:?}"));
                        break;
                    }
                };
                let target_qpc = match anchored_dispatch_target_ticks_typed(
                    target_sample_ticks,
                    resources.playback.epoch,
                    startup_gate.scheduled_ticks,
                    startup_gate.lead_ticks,
                ) {
                    Ok(target) => target,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("startup deadline failure: {error:?}"));
                        break;
                    }
                };
                core.runtime.startup_dispatch_target_qpc = Some(target_qpc);
                if target_sample_ticks < target_qpc {
                    let bounded_target_qpc = match lease_bounded_ticks(
                        target_qpc,
                        timing.lease_timeout_ticks,
                        supervisor_heartbeat_ticks,
                    ) {
                        Ok(target) => target,
                        Err(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error =
                                Some(format!("lease deadline failure: {error:?}"));
                            break;
                        }
                    };
                    let wait_result = resources.waiter.wait_until_ticks_with_metrics_typed(
                        qpc_clock,
                        bounded_target_qpc,
                        timing.effective_spin_threshold_ticks,
                        interrupt,
                    );
                    let spin_us = match qpc_clock.duration_to_us(wait_result.spin_ticks) {
                        Ok(value) => value,
                        Err(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error =
                                Some(format!("startup wait spin conversion failure: {error:?}"));
                            break;
                        }
                    };
                    core.metrics.idle_wake_count = core.metrics.idle_wake_count.saturating_add(1);
                    core.metrics.spin_time_us = core.metrics.spin_time_us.saturating_add(spin_us);
                    match wait_result.outcome {
                        WaitOutcome::Interrupted => continue,
                        WaitOutcome::Deadline if bounded_target_qpc == target_qpc => {}
                        WaitOutcome::Deadline => continue,
                        WaitOutcome::Failed(failure) => {
                            if matches!(failure, WaitFailure::Clock) {
                                core.metrics.wait_clock_failures =
                                    core.metrics.wait_clock_failures.saturating_add(1);
                            } else {
                                core.metrics.wait_backend_failures =
                                    core.metrics.wait_backend_failures.saturating_add(1);
                            }
                            if config.timing.strict_timing || matches!(failure, WaitFailure::Clock)
                            {
                                core.runtime.force_full_cleanup = true;
                                core.runtime.terminal_error = Some(wait_failure_message(failure));
                                break;
                            }
                            std::thread::sleep(Duration::from_micros(500));
                            continue;
                        }
                    }
                }
                core.runtime.startup_precision_phase = super::StartupPrecisionPhase::PostPrecision;
                #[cfg(any(test, feature = "test-support"))]
                if let Some(hook) = core.runtime.startup_ordering_hook.as_ref() {
                    hook.mark_precision_wait_completed();
                }
                core.runtime.startup_gate = None;
                core.runtime.allow_pre_epoch_startup_dispatch = true;
                now_ticks = qpc_ticks_or_terminal!();
            }

            let mut effective_now_ticks = if core.runtime.allow_pre_epoch_startup_dispatch
                && now_ticks < resources.playback.epoch
            {
                TimelineTicks::ZERO
            } else {
                core.runtime.allow_pre_epoch_startup_dispatch = false;
                match resources.playback.get_elapsed_allow_pre_epoch(
                    now_ticks,
                    core.runtime.allow_pre_epoch_startup_dispatch,
                ) {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("playback clock failure: {error}"));
                        break;
                    }
                }
            };
            // §8.7: fresh QPC → immutable plan → inspect slack → maybe drain
            // one observation → if drained, discard plan and rebuild from a
            // fresh QPC sample before any admit/dispatch/wait.  A startup
            // precision plan was frozen before its wait and must bypass this
            // general observer/planning path after the wait succeeds.
            let precision_plan_frozen = precision_plan.is_some();
            if !precision_plan_frozen
                && let Some(wait_observation) = core.runtime.pending_wait_observation.take()
            {
                core.observer.pending.push_wait(wait_observation);
            }
            let mut dispatch_plan = match precision_plan.take() {
                Some(plan) => plan,
                None => match plan_next_dispatch_projected(PlanningInput {
                    coordinator: &resources.coordinator,
                    estimator: &resources.estimator,
                    qpc_clock,
                    timing: &config.timing,
                    health_options: core
                        .health
                        .as_ref()
                        .expect("worker health initialized")
                        .options,
                    enable_dispatch_cost_lead: config.estimator.enable_dispatch_cost_lead,
                }) {
                    Ok(plan) => plan,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error = Some(format!("planning failure: {error}"));
                        break;
                    }
                },
            };
            if !precision_plan_frozen
                && !core.observer.pending.is_empty()
                && super::observer_has_safe_slack(
                    dispatch_plan.deadline_ticks,
                    effective_now_ticks,
                    timing.observer_guard_ticks,
                )
            {
                let drain_result = match super::drain_one_observer(
                    &mut core.observer.pending,
                    config,
                    core.health.as_mut().unwrap(),
                    &mut core.metrics,
                    &mut core.errors.last_published,
                    metrics,
                    &mut resources.backend,
                    &mut resources.estimator,
                    &mut resources.telemetry,
                    qpc_clock,
                    now_ticks,
                    &mut timing,
                ) {
                    Ok(value) => value,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("observer drain failed: {error:?}"));
                        break;
                    }
                };
                if observer_drain_requires_replan(drain_result) {
                    let drain_us = drain_result.expect("Some drain result must carry duration");
                    core.metrics.observer_duration_max_us =
                        core.metrics.observer_duration_max_us.max(drain_us);
                    // Observer work invalidates the plan. Rebuild from fresh QPC.
                    now_ticks = qpc_ticks_or_terminal!();
                    effective_now_ticks = if core.runtime.allow_pre_epoch_startup_dispatch
                        && now_ticks < resources.playback.epoch
                    {
                        TimelineTicks::ZERO
                    } else {
                        core.runtime.allow_pre_epoch_startup_dispatch = false;
                        match resources.playback.get_elapsed_allow_pre_epoch(
                            now_ticks,
                            core.runtime.allow_pre_epoch_startup_dispatch,
                        ) {
                            Ok(ticks) => ticks,
                            Err(error) => {
                                core.runtime.force_full_cleanup = true;
                                core.runtime.terminal_error =
                                    Some(format!("playback clock failure: {error}"));
                                break;
                            }
                        }
                    };
                    dispatch_plan = match plan_next_dispatch_projected(PlanningInput {
                        coordinator: &resources.coordinator,
                        estimator: &resources.estimator,
                        qpc_clock,
                        timing: &config.timing,
                        health_options: core
                            .health
                            .as_ref()
                            .expect("worker health initialized")
                            .options,
                        enable_dispatch_cost_lead: config.estimator.enable_dispatch_cost_lead,
                    }) {
                        Ok(plan) => plan,
                        Err(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error =
                                Some(format!("planning failure: {error}"));
                            break;
                        }
                    };
                }
            }
            let authored_step = dispatch_due_from_plan(
                &dispatch_plan,
                effective_now_ticks,
                now_ticks,
                resources.playback.epoch,
                focus_loss_fault,
                config,
                resources,
                core.health.as_mut().unwrap(),
                &timing,
                &mut core.runtime,
                &mut core.metrics,
                &mut core.errors.last_published,
                &mut core.errors.secondary,
                focus_active,
                target_hwnd,
                target_generation,
                quit_requested,
                skip_requested,
                panic_requested,
                desired_pause,
                supervisor_heartbeat_ticks,
                timing.lease_timeout_ticks,
                metrics,
                &shared.publication.progress_clock,
                &mut core.observer.pending,
            );
            match authored_step {
                super::DispatchStep::Dispatched | super::DispatchStep::Continue => continue,
                super::DispatchStep::NoWork if precision_plan_frozen => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(
                        "startup precision handoff found no physical work at the selected target"
                            .to_string(),
                    );
                    break;
                }
                super::DispatchStep::NoWork => {}
                super::DispatchStep::Terminate(err) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(err);
                    break;
                }
            }

            let deadline_ticks = dispatch_plan.deadline_ticks;

            match wait_for_next_boundary(WaitBoundaryInput {
                deadline: WaitDeadline {
                    deadline_ticks,
                    qpc_clock,
                    clock_state: &mut resources.playback,
                    allow_pre_epoch_startup_dispatch: core.runtime.allow_pre_epoch_startup_dispatch,
                },
                timing: WaitTiming {
                    effective_spin_threshold_ticks: timing.effective_spin_threshold_ticks,
                    lease_timeout_ticks: timing.lease_timeout_ticks,
                    supervisor_heartbeat_ticks,
                },
                signals: WaitSignals {
                    waiter: &resources.waiter,
                    interrupt,
                    strict_timing: config.timing.strict_timing,
                },
                mutable: WaitMutable {
                    local_metrics: &mut core.metrics,
                    force_full_cleanup: &mut core.runtime.force_full_cleanup,
                    terminal_error: &mut core.runtime.terminal_error,
                },
            }) {
                WaitBoundary::Due {
                    wait_result,
                    target_qpc,
                } => {
                    // This is the global wait boundary only. Physical target
                    // attribution is resolved from the selected work below.
                    let _wait_boundary_target_qpc = target_qpc;
                    let Some(wait_deadline_ticks) = deadline_ticks else {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some("wait returned a result without a dispatch deadline".to_string());
                        break;
                    };
                    if let Some(wait_result) = wait_result {
                        core.runtime.last_dispatch_deadline_wake_qpc = wait_result.wake_qpc;
                        core.runtime.pending_wait_observation = Some(WaitObservation {
                            outcome: wait_result.outcome,
                            wake_qpc: wait_result.wake_qpc,
                            spin_ticks: wait_result.spin_ticks,
                            deadline_ticks: wait_deadline_ticks,
                            epoch_qpc: resources.playback.epoch,
                            allow_pre_epoch_startup_dispatch: core
                                .runtime
                                .allow_pre_epoch_startup_dispatch,
                        });
                    }
                    let dispatch_now_ticks = match qpc_clock.now() {
                        Ok(ticks) => ticks,
                        Err(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error =
                                Some(format!("QPC failure after deadline wake: {error:?}"));
                            break;
                        }
                    };
                    let dispatch_effective_now =
                        match resources.playback.get_elapsed_allow_pre_epoch(
                            dispatch_now_ticks,
                            core.runtime.allow_pre_epoch_startup_dispatch,
                        ) {
                            Ok(ticks) => ticks,
                            Err(error) => {
                                core.runtime.force_full_cleanup = true;
                                core.runtime.terminal_error = Some(format!(
                                    "playback clock failure after deadline wake: {error}"
                                ));
                                break;
                            }
                        };
                    match dispatch_due_from_plan(
                        &dispatch_plan,
                        dispatch_effective_now,
                        dispatch_now_ticks,
                        resources.playback.epoch,
                        focus_loss_fault,
                        config,
                        resources,
                        core.health.as_mut().unwrap(),
                        &timing,
                        &mut core.runtime,
                        &mut core.metrics,
                        &mut core.errors.last_published,
                        &mut core.errors.secondary,
                        focus_active,
                        target_hwnd,
                        target_generation,
                        quit_requested,
                        skip_requested,
                        panic_requested,
                        desired_pause,
                        supervisor_heartbeat_ticks,
                        timing.lease_timeout_ticks,
                        metrics,
                        &shared.publication.progress_clock,
                        &mut core.observer.pending,
                    ) {
                        super::DispatchStep::Terminate(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error = Some(error);
                            break;
                        }
                        super::DispatchStep::Dispatched
                        | super::DispatchStep::Continue
                        | super::DispatchStep::NoWork => continue,
                    }
                }
                WaitBoundary::Replan { wait_result } => {
                    let Some(wait_deadline_ticks) = deadline_ticks else {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some("replan wait result without a dispatch deadline".to_string());
                        break;
                    };
                    core.runtime.pending_wait_observation = Some(WaitObservation {
                        outcome: wait_result.outcome,
                        wake_qpc: wait_result.wake_qpc,
                        spin_ticks: wait_result.spin_ticks,
                        deadline_ticks: wait_deadline_ticks,
                        epoch_qpc: resources.playback.epoch,
                        allow_pre_epoch_startup_dispatch: core
                            .runtime
                            .allow_pre_epoch_startup_dispatch,
                    });
                    // Event consumption is intentionally outside the
                    // precision boundary.  A signal racing this drain is
                    // harmless; the next blocking wait will replan.
                    let _ = interrupt.try_take();
                    continue;
                }
                WaitBoundary::Exit => break,
            }
        }
        // A terminal transition can end the coordinator loop immediately
        // after a successful physical send.  Drain the fixed raw queue before
        // finalization so deferred telemetry is not lost, while keeping this
        // work outside the normal dispatch critical section.
        while !core.observer.pending.is_empty() {
            let now_ticks = match qpc_clock.now() {
                Ok(ticks) => ticks,
                Err(error) => {
                    core.runtime.force_full_cleanup = true;
                    record_termination_error(
                        &mut core.runtime.terminal_error,
                        &mut core.errors.secondary,
                        format!("observer finalization QPC failure: {error:?}"),
                    );
                    break;
                }
            };
            let drain_result = match super::drain_one_observer(
                &mut core.observer.pending,
                config,
                core.health.as_mut().unwrap(),
                &mut core.metrics,
                &mut core.errors.last_published,
                metrics,
                &mut resources.backend,
                &mut resources.estimator,
                &mut resources.telemetry,
                qpc_clock,
                now_ticks,
                &mut timing,
            ) {
                Ok(value) => value,
                Err(error) => {
                    core.runtime.force_full_cleanup = true;
                    record_termination_error(
                        &mut core.runtime.terminal_error,
                        &mut core.errors.secondary,
                        format!("observer finalization failed: {error:?}"),
                    );
                    break;
                }
            };
            if let Some(drain_us) = drain_result {
                core.metrics.observer_duration_max_us =
                    core.metrics.observer_duration_max_us.max(drain_us);
            }
        }
    }))
}

#[inline]
fn observer_drain_requires_replan(result: Option<u64>) -> bool {
    result.is_some()
}

#[cfg(test)]
mod tests {
    use super::{PhysicalWork, observer_drain_requires_replan, physical_target_qpc_for_work};
    use crate::engine::worker::dispatch::timing::AuthoredDispatchPlan;
    use crate::engine::worker::health::DispatchPath;
    use crate::engine::worker::planning::NextDispatchPlan;
    use sky_dispatch_core::coordinator::PendingDispatchPlan;
    use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};

    #[test]
    fn zero_microsecond_observer_drain_still_invalidates_the_plan() {
        assert!(observer_drain_requires_replan(Some(0)));
        assert!(observer_drain_requires_replan(Some(7)));
        assert!(!observer_drain_requires_replan(None));
    }

    fn plan_with_deadlines(authored: u64, pending: u64) -> NextDispatchPlan {
        NextDispatchPlan {
            authored: Some(AuthoredDispatchPlan {
                path: DispatchPath::DownOnly { down_count: 1 },
                lead_us: 0,
                lead_ticks: DurationTicks::ZERO,
                lead_saturated: false,
                deadline_ticks: TimelineTicks::from_raw(authored),
            }),
            authored_budget: None,
            pending: Some(PendingDispatchPlan {
                deadline_ticks: TimelineTicks::from_raw(pending),
                lead_ticks: DurationTicks::ZERO,
                polyphony: 1,
                lead_saturated: false,
            }),
            pending_budget: None,
            deadline_ticks: Some(TimelineTicks::from_raw(authored.min(pending))),
        }
    }

    #[test]
    fn pending_overdue_work_uses_its_own_physical_target() {
        let plan = plan_with_deadlines(1_000, 1_100);
        let epoch = QpcTicks::ZERO;

        assert_eq!(
            physical_target_qpc_for_work(
                &plan,
                epoch,
                QpcTicks::from_raw(1_200),
                None,
                PhysicalWork::Pending
            )
            .expect("pending target")
            .expect("pending deadline"),
            QpcTicks::from_raw(1_100)
        );
        assert_eq!(
            physical_target_qpc_for_work(
                &plan,
                epoch,
                QpcTicks::from_raw(1_200),
                None,
                PhysicalWork::Authored
            )
            .expect("authored target")
            .expect("authored deadline"),
            QpcTicks::from_raw(1_000)
        );
    }

    #[test]
    fn pending_target_remains_earlier_when_authored_is_later() {
        let plan = plan_with_deadlines(1_100, 1_000);
        let target = physical_target_qpc_for_work(
            &plan,
            QpcTicks::ZERO,
            QpcTicks::from_raw(1_200),
            None,
            PhysicalWork::Pending,
        )
        .expect("pending target")
        .expect("pending deadline");

        assert_eq!(target, QpcTicks::from_raw(1_000));
    }

    #[test]
    fn equal_authored_and_pending_deadlines_share_the_same_target() {
        let plan = plan_with_deadlines(1_000, 1_000);
        let epoch = QpcTicks::from_raw(500);
        let pending = physical_target_qpc_for_work(
            &plan,
            epoch,
            QpcTicks::from_raw(1_500),
            None,
            PhysicalWork::Pending,
        )
        .expect("pending target")
        .expect("pending deadline");
        let authored = physical_target_qpc_for_work(
            &plan,
            epoch,
            QpcTicks::from_raw(1_500),
            None,
            PhysicalWork::Authored,
        )
        .expect("authored target")
        .expect("authored deadline");

        assert_eq!(pending, QpcTicks::from_raw(1_500));
        assert_eq!(authored, pending);
    }

    #[test]
    fn startup_target_is_preserved_through_physical_target_selection() {
        let plan = plan_with_deadlines(0, 1_000);
        let target = physical_target_qpc_for_work(
            &plan,
            QpcTicks::from_raw(10_000),
            QpcTicks::from_raw(9_540),
            Some(QpcTicks::from_raw(9_500)),
            PhysicalWork::Authored,
        )
        .expect("startup target")
        .expect("startup target is due");

        assert_eq!(target, QpcTicks::from_raw(9_500));
        assert_eq!(
            QpcTicks::from_raw(9_700)
                .checked_duration_since(target)
                .expect("completion follows exact startup target")
                .as_u64(),
            200
        );
    }

    #[test]
    fn future_selected_target_is_not_replaced_with_now() {
        let plan = plan_with_deadlines(1_000, 2_000);
        let target = physical_target_qpc_for_work(
            &plan,
            QpcTicks::ZERO,
            QpcTicks::from_raw(900),
            None,
            PhysicalWork::Authored,
        )
        .expect("target arithmetic");

        assert_eq!(target, None);
    }
}
