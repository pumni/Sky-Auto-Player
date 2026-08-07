use super::super::{
    ActionKind, BackendConfig, CORE_WARMUP_SPIN_MAX_US, CPU_METRICS_SAMPLE_INTERVAL_US,
    CoordinatorError, Duration, DurationTicks, HARD_LATE_ABORT_THRESHOLD_US, LatencyClass,
    PAUSED_POLL_US, PlaybackClockState, QpcClock, QpcError, QpcTicks, RELEASE_RETRY_BACKOFF_US,
    RuntimeDispatchCoordinator, SEND_COLD_THRESHOLD_US, STARTUP_WAKE_GUARD_US,
    STRICT_RETRY_LATE_THRESHOLD_US, SendLatencyEstimator, SharedMetrics, TelemetryCollector,
    TimelineTicks, TrackedKeyState, WaitFailure, WaitOutcome, current_process_cpu_time_us,
    current_thread_cpu_time_us, qpc_frequency_checked, try_publish_metrics,
};
#[cfg(any(test, feature = "test-support"))]
use super::super::{CommandTimingCleanup, create_mock_backend};
use super::{
    CommandControl, CommandControlClock, CommandControlInput, CommandControlMetrics,
    CommandControlRuntime, CommandControlSignals, DispatchHealthOptions, FinalizeInput,
    FinalizePublication, FinalizeResources, FinalizeSignals, FinalizeState, FinalizeTiming,
    HealthWindow, StartupResources, WaitBoundary, WaitBoundaryInput, WaitDeadline, WaitMutable,
    WaitSignals, WaitTiming, Worker, WorkerHealthState, WorkerResources, WorkerTimingState,
    anchored_dispatch_target_ticks_typed, classify_latency_class, cpu_metrics_sample_due,
    derive_spin_threshold_us, describe_release_outcome, ensure_preflight_for_target,
    finalize_worker, focus_matches, focus_matches_hwnd, initialize_startup, lease_bounded_ticks,
    load_target_stamp, plan_next_dispatch, process_command_control, publish_backend_metrics,
    publish_wake_error_stats, release_state_verified, suspend_live_input,
    target_stamp_still_current, wait_failure_message, wait_for_next_boundary,
};
use smallvec::SmallVec;
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;

pub(super) fn run(worker: &mut Worker<'_>) -> u8 {
    let shared = worker.shared;

    let schedule = match worker.take_schedule() {
        Ok(schedule) => schedule,
        Err(error) => {
            *shared.publication.metrics.last_error.lock() = Some(error.to_string());
            return 1;
        }
    };

    let core = &mut worker.core;
    let interrupt = &shared.commands.interrupt;
    let desired_pause = &shared.commands.desired_pause;
    let quit_requested = &shared.commands.quit_requested;
    let skip_requested = &shared.commands.skip_requested;
    let panic_requested = &shared.commands.panic_requested;
    let focus_active = &shared.commands.focus_active;
    let target_hwnd = &shared.target.target_hwnd;
    let target_generation = &shared.target.target_generation;
    let metrics = &shared.publication.metrics;
    let priority_acquired = &shared.publication.priority_acquired;
    let supervisor_heartbeat_ticks = &shared.publication.supervisor_heartbeat_ticks;

    let config = &worker.config;

    #[cfg(any(test, feature = "test-support"))]
    let command_timing = &shared.commands.command_timing;
    #[cfg(any(test, feature = "test-support"))]
    let _command_timing_cleanup = CommandTimingCleanup(&shared.commands.command_timing);
    let qpc_clock = match QpcClock::initialize() {
        Ok(clock) => clock,
        Err(error) => {
            *metrics.last_error.lock() = Some(format!("QPC admission failed: {error:?}"));
            return 1;
        }
    };
    #[cfg(any(test, feature = "test-support"))]
    let (focus_loss_fault, wait_fault) = match &config.backend {
        BackendConfig::Production => (false, false),
        BackendConfig::Mock { fault_script, .. } => (
            fault_script.focus_loss_after_due_before_send,
            fault_script.wait_failure,
        ),
    };
    #[cfg(not(any(test, feature = "test-support")))]
    let (focus_loss_fault, wait_fault) = (false, false);
    let mut backend = match &config.backend {
        #[cfg(any(test, feature = "test-support"))]
        BackendConfig::Mock {
            latency_base_us,
            latency_per_key_us,
            fault_script,
        } => create_mock_backend(
            qpc_clock,
            *latency_base_us,
            *latency_per_key_us,
            fault_script.clone(),
        ),
        BackendConfig::Production => TrackedKeyState::with_qpc_clock(qpc_clock),
    };
    let admission_failure =
        |backend: &mut TrackedKeyState, metrics: &SharedMetrics, primary_error: String| {
            let verification_hwnd = target_hwnd.load(Ordering::Acquire);
            let cleanup = backend.release_all_full_instrument(verification_hwnd);
            let message = if release_state_verified(backend, &cleanup) {
                primary_error
            } else {
                format!(
                    "{primary_error}; admission cleanup failed: {}",
                    describe_release_outcome(&cleanup)
                )
            };
            *metrics.last_error.lock() = Some(message);
            1
        };
    let StartupResources {
        power_guard: _power_guard,
        priority_guard: _priority_guard,
        waiter,
        power_throttling_disabled,
    } = initialize_startup(
        config.priority.mode,
        config.wait.enable_waitable_timer,
        config.wait.enable_event_wait,
        priority_acquired,
        metrics,
    );
    core.metrics.power_throttling_disabled = power_throttling_disabled;
    let mut estimator =
        match SendLatencyEstimator::try_new(0.2, config.timing.max_lead_us, config.allowed_count) {
            Ok(estimator) => estimator,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("invalid estimator configuration: {error}"),
                );
            }
        };
    if let Some(raw) = &config.estimator.state_json {
        // Timing caches are disposable runtime evidence. Any schema or
        // provenance mismatch starts from the conservative prior; it must not
        // turn a playback session into a keyboard cleanup failure.
        let _ = estimator.import_state(raw);
    }
    let frame_period_us = 1_000_000u64.div_ceil(u64::from(config.timing.game_fps));
    // The native worker owns the physical visibility floor. Python supplies
    // the resolved FPS, but note-on timestamps remain authored timestamps;
    // only the minimum post-Down hold is frame-safe.
    let effective_min_hold_us = config
        .timing
        .min_hold_us
        .max(frame_period_us.saturating_add(500));
    let min_hold_ticks = match qpc_clock.duration_from_us(effective_min_hold_us) {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("min-hold conversion failed: {error:?}"),
            );
        }
    };
    let hard_late_abort_threshold_ticks =
        match qpc_clock.duration_from_us(HARD_LATE_ABORT_THRESHOLD_US) {
            Ok(ticks) => ticks,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("hard late-abort threshold conversion failed: {error:?}"),
                );
            }
        };
    let retry_late_threshold_ticks =
        match qpc_clock.duration_from_us(STRICT_RETRY_LATE_THRESHOLD_US) {
            Ok(ticks) => ticks,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("retry-late threshold conversion failed: {error:?}"),
                );
            }
        };
    let strict_down_completion_late_ticks =
        match qpc_clock.duration_from_us(config.timing.strict_down_completion_late_us) {
            Ok(ticks) => ticks,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("strict note-on threshold conversion failed: {error:?}"),
                );
            }
        };
    let strict_up_completion_late_ticks =
        match qpc_clock.duration_from_us(config.timing.strict_up_completion_late_us) {
            Ok(ticks) => ticks,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("strict note-off threshold conversion failed: {error:?}"),
                );
            }
        };
    let focus_restore_grace_ticks =
        match qpc_clock.duration_from_us(config.focus.focus_restore_grace_us) {
            Ok(ticks) => ticks,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("focus grace conversion failed: {error:?}"),
                );
            }
        };
    let paused_poll_ticks = match qpc_clock.duration_from_us(PAUSED_POLL_US) {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("paused polling conversion failed: {error:?}"),
            );
        }
    };
    let cold_threshold_ticks = match qpc_clock.duration_from_us(SEND_COLD_THRESHOLD_US) {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("cold threshold conversion failed: {error:?}"),
            );
        }
    };
    let core_warmup_ticks = match qpc_clock.duration_from_us(
        config
            .timing
            .core_warmup_budget_us
            .min(CORE_WARMUP_SPIN_MAX_US),
    ) {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("core warmup conversion failed: {error:?}"),
            );
        }
    };
    let lease_timeout_ticks =
        match qpc_clock.duration_from_us(config.wait.supervisor_lease_timeout_us) {
            Ok(ticks) => ticks,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("lease timeout conversion failed: {error:?}"),
                );
            }
        };
    let mut retry_backoff_ticks = [DurationTicks::ZERO; RELEASE_RETRY_BACKOFF_US.len()];
    for (target, delay_us) in retry_backoff_ticks.iter_mut().zip(RELEASE_RETRY_BACKOFF_US) {
        *target = match qpc_clock.duration_from_us(delay_us) {
            Ok(value) => value,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("retry backoff conversion failed: {error:?}"),
                );
            }
        };
    }
    let delivery_margin_ticks = DurationTicks::ZERO;
    let mut coordinator = match RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        effective_min_hold_us,
        min_hold_ticks,
        0,
        delivery_margin_ticks,
        |us| {
            qpc_clock
                .timeline_from_us(us)
                .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
        },
    ) {
        Ok(coordinator) => coordinator,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("coordinator construction failed: {error}"),
            );
        }
    };
    let frame_period_ticks = match qpc_clock.duration_from_us(frame_period_us) {
        Ok(value) => value,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("frame period conversion failed: {error:?}"),
            );
        }
    };
    coordinator.set_frame_period_ticks(frame_period_ticks);
    core.metrics.total_us = match coordinator.effective_total_ticks().and_then(|ticks| {
        qpc_clock
            .duration_to_us(DurationTicks::from_raw(ticks.as_u64()))
            .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
    }) {
        Ok(total_us) => total_us,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("total timeline conversion failed: {error}"),
            );
        }
    };
    let telemetry = TelemetryCollector::new(config.telemetry.mode, config.telemetry.capacity);
    core.errors.abort_counts.reserve(6);
    let mut effective_spin_threshold_us = config.timing.spin_threshold_us;
    let _ = interrupt.try_take();
    if config.wait.enable_adaptive_spin
        && let Some(stats) = waiter.probe_wake_error_stats(qpc_clock, interrupt, 10)
    {
        publish_wake_error_stats(stats, &mut core.metrics);
        effective_spin_threshold_us =
            derive_spin_threshold_us(stats.p95_us, config.timing.spin_floor_us);
    }
    core.metrics.effective_spin_threshold_us = effective_spin_threshold_us;
    let initial_now_ticks = match qpc_clock.now() {
        Ok(now) => now,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("QPC admission failed: {error:?}"),
            );
        }
    };
    let initial_now_us =
        match qpc_clock.duration_to_us(DurationTicks::from_raw(initial_now_ticks.as_u64())) {
            Ok(now) => now,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("QPC admission conversion failed: {error:?}"),
                );
            }
        };
    let effective_spin_threshold_ticks =
        match qpc_clock.duration_from_us(effective_spin_threshold_us) {
            Ok(ticks) => ticks,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("spin threshold conversion failed: {error:?}"),
                );
            }
        };
    // Cold/hot classification must use physical QPC time.  The authored
    // playback clock deliberately freezes during pause/focus recovery, so a
    // logical gap cannot tell us whether the CPU/input path has gone cold.
    let health_options = DispatchHealthOptions {
        wait_warn_us: config.timing.input_path_warn_us,
        ..DispatchHealthOptions::default()
    };
    core.health = Some(WorkerHealthState {
        down_saturation_positive_streak: 0,
        up_saturation_positive_streak: 0,
        options: health_options,
        send_pure_window: HealthWindow::default(),
        bookkeeping_window: HealthWindow::default(),
        wait_window: HealthWindow::default(),
    });
    core.metrics.send_warn_threshold_us = health_options.send_warn_floor_us;
    core.metrics.bookkeeping_warn_threshold_us = health_options.bookkeeping_warn_us;
    core.metrics.wait_warn_threshold_us = health_options.wait_warn_us;
    // Keep the logical authored timeline at zero while placing the physical
    // anchor in the future.  This gives a t=0 action a real opportunity to
    // dispatch early by its measured lead instead of being forced late by the
    // worker prologue.
    let startup_class = LatencyClass::Cold;
    let startup_lead_us = if config.timing.dispatch_lead_us > 0 {
        config.timing.dispatch_lead_us
    } else if config.estimator.enable_adaptive_lead {
        estimator
            .estimate_lead_with_class_and_policy(
                ActionKind::Down,
                coordinator.next_authored_polyphony(),
                startup_class,
                config.timing.strict_timing,
            )
            .applied_us
    } else {
        0
    };
    let startup_lead_ticks = match qpc_clock.duration_from_us(startup_lead_us) {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("startup lead conversion failed: {error:?}"),
            );
        }
    };
    let startup_guard_ticks = (|| {
        let wake_guard = qpc_clock
            .duration_from_us(STARTUP_WAKE_GUARD_US)
            .map_err(|error| format!("{error:?}"))?;
        let with_spin = wake_guard
            .checked_add(effective_spin_threshold_ticks)
            .map_err(|error| error.to_string())?;
        with_spin
            .checked_add(core_warmup_ticks)
            .map_err(|error| error.to_string())
    })();
    let startup_guard_ticks = match startup_guard_ticks {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("startup guard conversion failed: {error}"),
            );
        }
    };
    let startup_anchor_ticks = match initial_now_ticks
        .checked_add_duration(startup_guard_ticks)
        .and_then(|ticks| ticks.checked_add_duration(startup_lead_ticks))
    {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("startup anchor arithmetic failed: {error}"),
            );
        }
    };
    let clock_state = match PlaybackClockState::new(
        startup_anchor_ticks,
        sky_dispatch_core::time::DurationTicks::from_raw(0),
    ) {
        Ok(clock) => clock,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("playback clock initialization failed: {error}"),
            );
        }
    };
    core.runtime.startup_gate = coordinator
        .batch_scheduled_ticks
        .first()
        .copied()
        .map(|scheduled_ticks| (scheduled_ticks, startup_lead_ticks));
    let start_wall_time_us = initial_now_us;
    let start_thread_cpu_us = current_thread_cpu_time_us();
    let start_process_cpu_us = current_process_cpu_time_us();
    core.timing = Some(WorkerTimingState {
        hard_late_abort_threshold_ticks,
        retry_late_threshold_ticks,
        strict_down_completion_late_ticks,
        strict_up_completion_late_ticks,
        focus_restore_grace_ticks,
        paused_poll_ticks,
        cold_threshold_ticks,
        core_warmup_ticks,
        lease_timeout_ticks,
        retry_backoff_ticks,
        effective_spin_threshold_ticks,
        start_wall_time_us,
        start_thread_cpu_us,
        start_process_cpu_us,
        last_cpu_metrics_sample_us: start_wall_time_us,
    });
    let mut timing = *core.timing.as_ref().expect("worker timing initialized");
    let qpc_admission_error = qpc_frequency_checked()
        .err()
        .map(|error| format!("QPC frequency unavailable: {error:?}"))
        .or_else(|| {
            qpc_clock
                .now()
                .err()
                .map(|error| format!("QPC counter unavailable: {error:?}"))
        })
        .or_else(|| {
            config
                .timing
                .strict_timing
                .then(|| waiter.initial_failure().map(wait_failure_message))
                .flatten()
        });
    if let Some(error) = qpc_admission_error {
        core.runtime.force_full_cleanup = true;
        core.runtime.terminal_error = Some(error);
    }
    if wait_fault {
        core.runtime.force_full_cleanup = true;
        core.runtime.terminal_error = Some("wait failure injected".to_string());
    }

    core.resources = Some(WorkerResources {
        clock: qpc_clock,
        waiter,
        backend,
        coordinator,
        playback: clock_state,
        estimator,
        telemetry,
    });

    if core.runtime.terminal_error.is_none() {
        let startup_ready_result = qpc_clock.now().and_then(|ready_ticks| {
            let requested_raw = shared
                .publication
                .startup_requested_ticks
                .load(Ordering::Acquire);
            if requested_raw == 0 {
                return Err(sky_dispatch_win32::clock::QpcError::CounterUnavailable);
            }
            let requested_ticks = QpcTicks::from_raw(requested_raw);
            let elapsed_ticks = ready_ticks
                .checked_duration_since(requested_ticks)
                .map_err(|_| sky_dispatch_win32::clock::QpcError::CounterUnavailable)?;
            let elapsed_us = qpc_clock
                .duration_to_us(elapsed_ticks)
                .map_err(|_| sky_dispatch_win32::clock::QpcError::CounterUnavailable)?;
            Ok((ready_ticks, elapsed_us))
        });
        match startup_ready_result {
            Ok((startup_ready_ticks, startup_latency_us)) => {
                shared
                    .publication
                    .startup_ready_ticks
                    .store(startup_ready_ticks.as_u64(), Ordering::Relaxed);
                shared
                    .publication
                    .startup_latency_us
                    .store(startup_latency_us, Ordering::Relaxed);
                shared
                    .publication
                    .startup_ready
                    .store(true, Ordering::Release);
            }
            Err(error) => {
                core.runtime.force_full_cleanup = true;
                core.runtime.terminal_error =
                    Some(format!("QPC startup-ready publication failed: {error:?}"));
            }
        }
    }

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

    let worker_result = catch_unwind(AssertUnwindSafe(|| {
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

            let effective_now_ticks = if core.runtime.allow_pre_epoch_startup_dispatch
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
            let effective_now_us = qpc_ticks_to_us_or_terminal!(effective_now_ticks);
            core.metrics.elapsed_us = effective_now_us;
            let latency_class = match classify_latency_class(
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

            let dispatch_plan = match plan_next_dispatch(
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
                    },
                    config,
                    resources,
                    core.health.as_mut().unwrap(),
                    &timing,
                    &mut core.runtime,
                    &mut core.metrics,
                    &mut core.errors.secondary,
                    &mut core.errors.last_published,
                    target_hwnd,
                    metrics,
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
    }));

    worker.finalize(worker_result)
}

impl Worker<'_> {
    fn finalize(&mut self, worker_result: Result<(), Box<dyn Any + Send>>) -> u8 {
        let shared = self.shared;
        let core = &mut self.core;
        let resources = core
            .resources
            .take()
            .expect("worker resources available for finalization");
        let timing = core
            .timing
            .as_ref()
            .expect("worker timing available for finalization");

        finalize_worker(FinalizeInput {
            resources: FinalizeResources {
                backend: resources.backend,
                coordinator: resources.coordinator,
                telemetry: resources.telemetry,
                estimator: resources.estimator,
                qpc_clock: resources.clock,
            },
            state: FinalizeState {
                worker_result,
                local_metrics: std::mem::take(&mut core.metrics),
                abort_counts: std::mem::take(&mut core.errors.abort_counts),
                force_full_cleanup: core.runtime.force_full_cleanup,
                terminal_error: std::mem::take(&mut core.runtime.terminal_error),
                secondary_errors: std::mem::take(&mut core.errors.secondary),
                last_published_error: std::mem::take(&mut core.errors.last_published),
            },
            signals: FinalizeSignals {
                target_hwnd: &shared.target.target_hwnd,
                skip_requested: &shared.commands.skip_requested,
                quit_requested: &shared.commands.quit_requested,
            },
            publication: FinalizePublication {
                metrics: &shared.publication.metrics,
                telemetry_output: &shared.publication.telemetry_output,
                estimator_output: &shared.publication.estimator_output,
            },
            timing: FinalizeTiming {
                start_wall_time_us: timing.start_wall_time_us,
                start_thread_cpu_us: timing.start_thread_cpu_us,
                start_process_cpu_us: timing.start_process_cpu_us,
            },
        })
    }
}
