use super::super::{
    CPU_METRICS_SAMPLE_INTERVAL_US, Duration, DurationTicks, QpcError, TimelineTicks, WaitFailure,
    WaitOutcome, current_process_cpu_time_us, current_thread_cpu_time_us, try_publish_metrics,
};
use super::{
    CommandControl, CommandControlClock, CommandControlInput, CommandControlMetrics,
    CommandControlRuntime, CommandControlSignals, WaitBoundary, WaitBoundaryInput, WaitDeadline,
    WaitMutable, WaitSignals, WaitTiming, Worker, anchored_dispatch_target_ticks_typed,
    classify_latency_class, cpu_metrics_sample_due, ensure_preflight_for_target, focus_matches,
    focus_matches_hwnd, lease_bounded_ticks, load_target_stamp, plan_next_dispatch,
    process_command_control, publish_backend_metrics, suspend_live_input,
    target_stamp_still_current, wait_failure_message, wait_for_next_boundary,
};
use smallvec::SmallVec;
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;

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

    macro_rules! qpc_ticks_to_us_or_terminal {
        ($ticks:expr) => {{
            match qpc_clock.duration_to_us(DurationTicks::from_raw($ticks.as_u64())) {
                Ok(value) => value,
                Err(error) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error =
                        Some(format!("QPC conversion failure: {error:?}"));
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
            let loop_start_ticks = qpc_ticks_or_terminal!();
            let loop_start_us = qpc_ticks_to_us_or_terminal!(loop_start_ticks);
            core.metrics.playback_wall_time_us =
                loop_start_us.saturating_sub(timing.start_wall_time_us);
            if cpu_metrics_sample_due(
                loop_start_us,
                timing.last_cpu_metrics_sample_us,
                CPU_METRICS_SAMPLE_INTERVAL_US,
            ) {
                core.metrics.worker_cpu_time_us =
                    current_thread_cpu_time_us().saturating_sub(timing.start_thread_cpu_us);
                core.metrics.process_cpu_time_us =
                    current_process_cpu_time_us().saturating_sub(timing.start_process_cpu_us);
                timing.last_cpu_metrics_sample_us = loop_start_us;
            }
            if core.metrics.playback_wall_time_us > 0 {
                core.metrics.spin_duty_cycle_ppm = (core.metrics.spin_time_us as u128 * 1_000_000
                    / core.metrics.playback_wall_time_us as u128)
                    as u64;
            }
            try_publish_metrics(&core.metrics, metrics, loop_start_us, false);
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
            let focus_ok = focus_matches(config.focus.require_focus, focus_active, target_hwnd);
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

            if let Some((startup_scheduled_ticks, startup_lead_ticks)) = core.runtime.startup_gate {
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
                    startup_scheduled_ticks,
                    startup_lead_ticks,
                ) {
                    Ok(target) => target,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("startup deadline failure: {error:?}"));
                        break;
                    }
                };
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
                    core.metrics.idle_wake_count = core.metrics.idle_wake_count.saturating_add(1);
                    core.metrics.spin_time_us = core
                        .metrics
                        .spin_time_us
                        .saturating_add(wait_result.spin_us);
                    match wait_result.outcome {
                        WaitOutcome::Interrupted => continue,
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
            let mut effective_now_us = qpc_ticks_to_us_or_terminal!(effective_now_ticks);
            core.metrics.elapsed_us = effective_now_us;
            let mut latency_class = match classify_latency_class(
                core.runtime.last_send_qpc_ticks,
                now_ticks,
                timing.cold_threshold_ticks,
            ) {
                Ok(class) => class,
                Err(error) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(format!("QPC ordering failure: {error}"));
                    break;
                }
            };

            // §8.7: fresh QPC → immutable plan → inspect slack → maybe drain
            // one observation → if drained, discard plan and rebuild from a
            // fresh QPC sample before any admit/dispatch/wait.
            let mut dispatch_plan = match plan_next_dispatch(
                &resources.coordinator,
                &resources.estimator,
                qpc_clock,
                latency_class,
                &config.timing,
                config.estimator.enable_adaptive_lead,
            ) {
                Ok(plan) => plan,
                Err(error) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(format!("planning failure: {error}"));
                    break;
                }
            };
            if !core.observer.pending.is_empty()
                && super::observer_has_safe_slack(
                    dispatch_plan.deadline_ticks,
                    effective_now_ticks,
                    core.observer.budget_us,
                    super::OBSERVER_MARGIN_US,
                    qpc_clock,
                )
            {
                let drain_us = super::drain_one_observer(
                    &mut core.observer.pending,
                    config,
                    core.health.as_mut().unwrap(),
                    &mut core.metrics,
                    &mut core.errors.last_published,
                    metrics,
                    &mut resources.backend,
                    &mut resources.estimator,
                    qpc_clock,
                    effective_now_us,
                );
                if drain_us > 0 {
                    core.metrics.observer_duration_max_us =
                        core.metrics.observer_duration_max_us.max(drain_us);
                    // §8.9 adaptive budget: clamp(2 * observed, FLOOR..CAP).
                    core.observer.budget_us = drain_us.saturating_mul(2).clamp(
                        super::OBSERVER_BUDGET_FLOOR_US,
                        super::OBSERVER_BUDGET_CAP_US,
                    );
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
                    effective_now_us = qpc_ticks_to_us_or_terminal!(effective_now_ticks);
                    core.metrics.elapsed_us = effective_now_us;
                    latency_class = match classify_latency_class(
                        core.runtime.last_send_qpc_ticks,
                        now_ticks,
                        timing.cold_threshold_ticks,
                    ) {
                        Ok(class) => class,
                        Err(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error =
                                Some(format!("QPC ordering failure: {error}"));
                            break;
                        }
                    };
                    dispatch_plan = match plan_next_dispatch(
                        &resources.coordinator,
                        &resources.estimator,
                        qpc_clock,
                        latency_class,
                        &config.timing,
                        config.estimator.enable_adaptive_lead,
                    ) {
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
            let pending_plan = dispatch_plan.pending;

            let lead_up_ticks = match pending_plan.as_ref() {
                Some(plan) => plan.lead_ticks,
                None => DurationTicks::ZERO,
            };
            let lead_up = match qpc_clock.duration_to_us(lead_up_ticks) {
                Ok(lead) => lead,
                Err(error) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error =
                        Some(format!("lead telemetry conversion failure: {error:?}"));
                    break;
                }
            };
            let due_pending = match pending_plan.as_ref() {
                Some(plan) => match resources
                    .coordinator
                    .pop_due_pending_ticks(effective_now_ticks, plan)
                {
                    Ok(due) => due,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("coordinator pending-pop failure: {error}"));
                        break;
                    }
                },
                None => SmallVec::new(),
            };
            if !due_pending.is_empty() {
                let step = super::dispatch_due_pending_releases(
                    super::PendingReleaseContext {
                        due_pending,
                        pending_plan: pending_plan.as_ref(),
                        lead_up_ticks,
                        lead_up,
                        latency_class,
                        observer: &mut core.observer.pending,
                    },
                    config,
                    resources,
                    core.health.as_mut().unwrap(),
                    &timing,
                    &mut core.runtime,
                    &mut core.metrics,
                    &mut core.errors.secondary,
                    target_hwnd,
                );
                match step {
                    super::DispatchStep::Dispatched | super::DispatchStep::Continue => continue,
                    super::DispatchStep::NoWork => {}
                    super::DispatchStep::Terminate(err) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error = Some(err);
                        break;
                    }
                }
            }

            let authored_step = super::dispatch_authored_packet(
                super::AuthoredPacketContext {
                    dispatch_plan: &dispatch_plan,
                    effective_now_ticks,
                    now_ticks,
                    latency_class,
                    focus_loss_fault,
                },
                config,
                resources,
                core.health.as_mut().unwrap(),
                &timing,
                &mut core.runtime,
                &mut core.metrics,
                &mut core.errors.last_published,
                focus_active,
                target_hwnd,
                target_generation,
                quit_requested,
                skip_requested,
                panic_requested,
                desired_pause,
                metrics,
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

            let deadline_ticks = dispatch_plan.deadline_ticks;

            match wait_for_next_boundary(WaitBoundaryInput {
                deadline: WaitDeadline {
                    deadline_ticks,
                    qpc_clock,
                    clock_state: &mut resources.playback,
                    allow_pre_epoch_startup_dispatch: core.runtime.allow_pre_epoch_startup_dispatch,
                    last_send_qpc_ticks: core.runtime.last_send_qpc_ticks,
                },
                timing: WaitTiming {
                    core_warmup_ticks: timing.core_warmup_ticks,
                    cold_threshold_ticks: timing.cold_threshold_ticks,
                    effective_spin_threshold_ticks: timing.effective_spin_threshold_ticks,
                    lease_timeout_ticks: timing.lease_timeout_ticks,
                    supervisor_heartbeat_ticks,
                },
                signals: WaitSignals {
                    waiter: &resources.waiter,
                    interrupt,
                    strict_timing: config.timing.strict_timing,
                    wait_warn_us: core.health.as_ref().unwrap().options.wait_warn_us,
                    wait_policy: core.health.as_ref().unwrap().options.window_policy(),
                },
                mutable: WaitMutable {
                    local_metrics: &mut core.metrics,
                    pending_pre_send_spin_us: &mut core.runtime.pending_pre_send_spin_us,
                    force_full_cleanup: &mut core.runtime.force_full_cleanup,
                    terminal_error: &mut core.runtime.terminal_error,
                    wait_window: &mut core.health.as_mut().unwrap().wait_window,
                },
            }) {
                WaitBoundary::Ready => {}
                WaitBoundary::Continue => continue,
                WaitBoundary::Exit => break,
            }
        }
    }))
}
