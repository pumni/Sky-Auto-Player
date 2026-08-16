use super::super::{DurationTicks, QpcError, TimelineTicks, WaitOutcome, try_publish_metrics};
use super::wait::WaitObservation;
use super::{
    CommandControl, CommandControlClock, CommandControlInput, CommandControlMetrics,
    CommandControlRuntime, CommandControlSignals, PlanningInput, WaitBoundary, WaitBoundaryInput,
    WaitDeadline, WaitMutable, WaitSignals, WaitTiming, Worker, ensure_preflight_for_target,
    focus_matches, focus_matches_hwnd, lease_bounded_ticks, load_target_stamp,
    plan_next_dispatch_projected, plan_structure_is_valid, process_command_control,
    publish_backend_metrics, record_wait_failure, suspend_live_input, target_stamp_still_current,
    wait_for_next_boundary,
};
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

fn physical_target_qpc_for_work(
    target: Option<sky_dispatch_win32::clock::QpcTicks>,
    now: sky_dispatch_win32::clock::QpcTicks,
) -> Result<Option<sky_dispatch_win32::clock::QpcTicks>, String> {
    let Some(target) = target else {
        return Ok(None);
    };
    Ok((target <= now).then_some(target))
}

pub(crate) fn preflight_prepared_plan(
    plan: &mut super::planning::NextDispatchPlan,
    backend: &mut sky_dispatch_win32::input::TrackedKeyState,
    runtime: &mut super::WorkerRuntime,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
) -> Result<bool, super::DispatchStep> {
    let Some(physical) = plan.physical_mut() else {
        return Ok(true);
    };
    let has_down_events = physical.authored_view.packet_masks.down_mask != 0;
    if !has_down_events {
        return Ok(true);
    }
    runtime.preparation_probe.record_preflight();
    let target = super::load_target_stamp(target_hwnd, target_generation);
    if let Err(error) =
        super::ensure_preflight_for_target(backend, target, &mut runtime.verified_target)
    {
        runtime.verified_target = None;
        return Err(super::DispatchStep::Terminate(format!(
            "instrument key preflight failed before timed wait; release the 15 instrument keys before playback: {error}"
        )));
    }
    if !super::target_stamp_still_current(target_hwnd, target_generation, target) {
        runtime.verified_target = None;
        return Ok(false);
    }
    physical.target_proof = super::TargetProof::Verified(target);
    Ok(true)
}

/// Project the current QPC into playback time for stale metadata diagnostics.
///
/// A pre-epoch startup projection is intentionally zero, but once playback has
/// begun this reports the real elapsed timeline. This never enables or mutates
/// the physical pre-epoch startup admission path.
fn stale_metadata_effective_now(
    playback: &sky_dispatch_core::clock::PlaybackClockState,
    now_qpc: sky_dispatch_win32::clock::QpcTicks,
) -> Result<TimelineTicks, sky_dispatch_core::time::TimeArithmeticError> {
    if now_qpc < playback.epoch {
        Ok(TimelineTicks::ZERO)
    } else {
        playback.get_elapsed_allow_pre_epoch(now_qpc, false)
    }
}

/// Dispatch the work represented by one immutable plan.  This helper is used
/// for both an already-due plan and a successful blocking deadline wake, so a
/// normal timer wake never re-enters general orchestration before transport.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_due_from_plan(
    plan: &super::planning::NextDispatchPlan,
    effective_now_ticks: TimelineTicks,
    now_ticks: sky_dispatch_win32::clock::QpcTicks,
    focus_loss_fault: bool,
    config: &super::WorkerConfig,
    resources: &mut super::WorkerResources,
    health: &mut super::WorkerHealthState,
    timing: &super::WorkerTimingState,
    runtime: &mut super::WorkerRuntime,
    local_metrics: &mut super::WorkerMetricsLocal,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    supervisor_heartbeat_ticks: &AtomicU64,
    lease_timeout_ticks: DurationTicks,
    progress_clock: &crate::engine::shared::SharedProgressClock,
    observer: &mut super::dispatch::PendingObservationQueue,
) -> super::DispatchStep {
    if !plan_structure_is_valid(plan) {
        return super::DispatchStep::Terminate(
            "dispatch plan has inconsistent prepared physical state".to_string(),
        );
    }
    if let super::planning::NextDispatchPlan::Metadata(metadata) = plan {
        if metadata.physical_target_qpc > now_ticks {
            return super::DispatchStep::NoWork;
        }
        return resources
            .coordinator
            .commit_authored_frame_metadata(metadata.frame)
            .map(|()| super::DispatchStep::Dispatched)
            .unwrap_or_else(|error| {
                super::DispatchStep::Terminate(format!(
                    "coordinator authored metadata commit failure: {error}"
                ))
            });
    }
    if !matches!(plan, super::planning::NextDispatchPlan::Physical(_)) {
        return super::DispatchStep::NoWork;
    }
    /* stale authored metadata is drained by the outer global metadata phase */
    let startup_target_selected = false;
    let physical_target_qpc =
        match physical_target_qpc_for_work(plan.physical_target_qpc(), now_ticks) {
            Ok(Some(target)) => target,
            Ok(None) => {
                runtime.last_physical_target_qpc = None;
                return super::DispatchStep::NoWork;
            }
            Err(error) => return super::DispatchStep::Terminate(error),
        };

    let arrived_via_deadline_wait = runtime.last_dispatch_deadline_wake_qpc.is_some();
    if runtime
        .last_physical_target_qpc
        .is_some_and(|_| physical_target_qpc <= now_ticks)
        && !arrived_via_deadline_wait
    {
        return super::DispatchStep::Terminate(
            "physical deadline infeasible: refusing overdue catch-up burst".to_string(),
        );
    }

    let step = super::dispatch_authored_packet(
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
        focus_active,
        target_hwnd,
        target_generation,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        progress_clock,
        observer,
    );
    if matches!(step, super::DispatchStep::Dispatched) {
        runtime.last_physical_target_qpc = Some(physical_target_qpc);
    }
    step
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

    let timing = *core.timing.as_ref().expect("worker timing initialized");
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

            let now_ticks = qpc_ticks_or_terminal!();
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
                core.runtime.verified_target = None;
                core.runtime.focus_restore_started_ticks = None;
                if !resources.playback.has_pause_reason("focus") {
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
                    let manual_pause_active =
                        manual_pause || resources.playback.has_pause_reason("manual");
                    core.runtime.verified_target = None;
                    if !manual_pause_active {
                        if let Err(error) = suspend_live_input(
                            &mut resources.backend,
                            &mut resources.coordinator,
                            preflight_target.hwnd,
                        ) {
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
                    }
                    #[cfg(any(test, feature = "test-support"))]
                    if let Some(hook) = core.runtime.restore_race_hook.as_ref() {
                        hook(focus_active, target_hwnd, target_generation);
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
                    record_wait_failure(
                        failure,
                        &mut core.metrics,
                        &mut core.runtime.force_full_cleanup,
                        &mut core.runtime.terminal_error,
                    );
                    break;
                }
                continue;
            }

            // Stale metadata is globally non-physical. Commit at most one
            // compiled packet per outer iteration, regardless of startup
            // phase, so every control/focus/pause/lease gate is re-admitted.
            match resources.coordinator.prepare_current_stale_packet() {
                Ok(Some(prepared)) => {
                    let stale_now =
                        match stale_metadata_effective_now(&resources.playback, now_ticks) {
                            Ok(ticks) => ticks,
                            Err(error) => {
                                core.runtime.force_full_cleanup = true;
                                core.runtime.terminal_error = Some(format!(
                                    "stale metadata clock projection failure: {error}"
                                ));
                                break;
                            }
                        };
                    match super::dispatch_stale_packet(
                        prepared,
                        &mut resources.coordinator,
                        &core.observer.pending,
                        &mut core.metrics.observer_dropped_samples,
                        &mut core.metrics.observer_queue_high_watermark,
                        stale_now,
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

            let effective_now_ticks = if now_ticks < resources.playback.epoch {
                TimelineTicks::ZERO
            } else {
                match resources
                    .playback
                    .get_elapsed_allow_pre_epoch(now_ticks, false)
                {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("playback clock failure: {error}"));
                        break;
                    }
                }
            };
            // Wait evidence is also deferred through the nonblocking producer
            // path. The observer consumer owns all conversion and health work.
            if let Some(wait_observation) = core.runtime.pending_wait_observation.take() {
                core.observer.pending.push_wait(
                    wait_observation,
                    &mut core.metrics.observer_dropped_samples,
                    &mut core.metrics.observer_queue_high_watermark,
                );
            }
            let mut dispatch_plan = match plan_next_dispatch_projected(PlanningInput {
                coordinator: &resources.coordinator,
                epoch_qpc: resources.playback.epoch,
                preparation_probe: &core.runtime.preparation_probe,
            }) {
                Ok(plan) => plan,
                Err(error) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(format!("planning failure: {error}"));
                    break;
                }
            };
            match preflight_prepared_plan(
                &mut dispatch_plan,
                &mut resources.backend,
                &mut core.runtime,
                target_hwnd,
                target_generation,
            ) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(super::DispatchStep::Terminate(error)) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(error);
                    break;
                }
                Err(step) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(format!(
                        "unexpected preflight preparation outcome: {step:?}"
                    ));
                    break;
                }
            }
            let authored_step = dispatch_due_from_plan(
                &dispatch_plan,
                effective_now_ticks,
                now_ticks,
                focus_loss_fault,
                config,
                resources,
                core.health.as_mut().unwrap(),
                &timing,
                &mut core.runtime,
                &mut core.metrics,
                focus_active,
                target_hwnd,
                target_generation,
                quit_requested,
                skip_requested,
                panic_requested,
                desired_pause,
                supervisor_heartbeat_ticks,
                timing.lease_timeout_ticks,
                &shared.publication.progress_clock,
                &mut core.observer.pending,
            );
            match authored_step {
                super::DispatchStep::Dispatched | super::DispatchStep::Continue => continue,
                super::DispatchStep::NoWork => {}
                super::DispatchStep::Terminate(err) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(err);
                    break;
                }
            }

            let deadline_ticks = dispatch_plan.deadline_ticks();

            match wait_for_next_boundary(WaitBoundaryInput {
                deadline: WaitDeadline {
                    physical_target_qpc: dispatch_plan.physical_target_qpc(),
                    qpc_clock,
                },
                timing: WaitTiming {
                    effective_spin_threshold_ticks: timing.effective_spin_threshold_ticks,
                    lease_timeout_ticks: timing.lease_timeout_ticks,
                    supervisor_heartbeat_ticks,
                },
                signals: WaitSignals {
                    waiter: &resources.waiter,
                    interrupt,
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
                    dispatch_qpc,
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
                            allow_pre_epoch_startup_dispatch: true,
                        });
                    }
                    let dispatch_now_ticks = dispatch_qpc;
                    let dispatch_effective_now = match resources
                        .playback
                        .get_elapsed_allow_pre_epoch(dispatch_now_ticks, true)
                    {
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
                        focus_loss_fault,
                        config,
                        resources,
                        core.health.as_mut().unwrap(),
                        &timing,
                        &mut core.runtime,
                        &mut core.metrics,
                        focus_active,
                        target_hwnd,
                        target_generation,
                        quit_requested,
                        skip_requested,
                        panic_requested,
                        desired_pause,
                        supervisor_heartbeat_ticks,
                        timing.lease_timeout_ticks,
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
                        allow_pre_epoch_startup_dispatch: true,
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
    }))
}

#[cfg(test)]
mod tests {
    use super::physical_target_qpc_for_work;
    use sky_dispatch_core::time::QpcTicks;

    #[test]
    fn authored_overdue_work_uses_its_physical_target() {
        assert_eq!(
            physical_target_qpc_for_work(
                Some(QpcTicks::from_raw(1_000)),
                QpcTicks::from_raw(1_200),
            )
            .expect("authored target")
            .expect("authored deadline"),
            QpcTicks::from_raw(1_000)
        );
    }

    #[test]
    fn startup_target_cannot_precede_the_playback_epoch() {
        let target = physical_target_qpc_for_work(
            Some(QpcTicks::from_raw(10_000)),
            QpcTicks::from_raw(9_540),
        )
        .expect("target arithmetic");

        assert_eq!(target, None);
    }

    #[test]
    fn future_selected_target_is_not_replaced_with_now() {
        let target =
            physical_target_qpc_for_work(Some(QpcTicks::from_raw(1_000)), QpcTicks::from_raw(900))
                .expect("prepared target");

        assert_eq!(target, None);
    }
}
