use super::super::{
    ActionKind, BackendConfig, CORE_WARMUP_SPIN_MAX_US, CPU_METRICS_SAMPLE_INTERVAL_US,
    CoordinatorError, Duration, DurationTicks, HARD_LATE_ABORT_THRESHOLD_US, LatencyClass,
    PAUSED_POLL_US, PlaybackClockState, QpcClock, QpcError, QpcTicks, RELEASE_RETRY_BACKOFF_US,
    RtTraceRecord, RuntimeDispatchCoordinator, SEND_COLD_THRESHOLD_US, STARTUP_WAKE_GUARD_US,
    STRICT_RETRY_LATE_THRESHOLD_US, STRICT_SATURATION_ABORT_STREAK, SendLatencyEstimator,
    SharedMetrics, TRACE_FLAG_ANOMALY, TRACE_FLAG_DEFERRED, TRACE_FLAG_RECOVERY,
    TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN, TRACE_KIND_UP, TelemetryCollector, TimelineTicks,
    TraceContext, TraceDelivery, TraceTiming, TrackedKeyState, WaitFailure, WaitOutcome,
    current_process_cpu_time_us, current_thread_cpu_time_us, qpc_frequency_checked,
    trace_outcome_code, try_publish_metrics,
};
#[cfg(any(test, feature = "test-support"))]
use super::super::{CommandTimingCleanup, create_mock_backend};
use super::{
    CommandControl, CommandControlClock, CommandControlInput, CommandControlMetrics,
    CommandControlRuntime, CommandControlSignals, DispatchHealthObservation, DispatchHealthOptions,
    DispatchPath, DownAdmission, FinalizeInput, FinalizePublication, FinalizeResources,
    FinalizeSignals, FinalizeState, FinalizeTiming, HealthWindow, StartupResources, WaitBoundary,
    WaitBoundaryInput, WaitDeadline, WaitMutable, WaitSignals, WaitTiming, Worker,
    WorkerHealthState, WorkerResources, WorkerTimingState, anchored_dispatch_target_ticks_typed,
    build_dispatch_budget, cancel_coordinator_or_terminal, classify_latency_class,
    cpu_metrics_sample_due, derive_spin_threshold_us, describe_release_outcome,
    ensure_preflight_for_target, estimator_kind_for_path, final_down_admission, finalize_worker,
    focus_matches, focus_matches_hwnd, initialize_startup, lease_bounded_ticks, load_target_stamp,
    observe_dispatch_health, plan_next_dispatch, process_command_control, publish_backend_metrics,
    publish_wake_error_stats, record_lateness, record_lead_saturation, record_termination_error,
    release_runtime_outcome, release_state_verified, signed_delta, signed_ticks_to_us,
    signed_timeline_delta_ticks, suspend_live_input, target_stamp_still_current,
    update_estimator_after_send_class, wait_failure_message, wait_for_next_boundary,
};
use crate::engine::telemetry::TRACE_KIND_MIXED;
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
    let local_metrics = &mut core.metrics;
    let runtime = &mut core.runtime;
    let secondary_errors = &mut core.errors.secondary;
    let last_published_error = &mut core.errors.last_published;
    let abort_counts = &mut core.errors.abort_counts;
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
    local_metrics.power_throttling_disabled = power_throttling_disabled;
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
    local_metrics.total_us = match coordinator.effective_total_ticks().and_then(|ticks| {
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
    abort_counts.reserve(6);
    let mut effective_spin_threshold_us = config.timing.spin_threshold_us;
    let _ = interrupt.try_take();
    if config.wait.enable_adaptive_spin
        && let Some(stats) = waiter.probe_wake_error_stats(qpc_clock, interrupt, 10)
    {
        publish_wake_error_stats(stats, local_metrics);
        effective_spin_threshold_us =
            derive_spin_threshold_us(stats.p95_us, config.timing.spin_floor_us);
    }
    local_metrics.effective_spin_threshold_us = effective_spin_threshold_us;
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
    let health = core.health.as_mut().expect("worker health initialized");
    local_metrics.send_warn_threshold_us = health.options.send_warn_floor_us;
    local_metrics.bookkeeping_warn_threshold_us = health.options.bookkeeping_warn_us;
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
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
    runtime.startup_gate = coordinator
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
    let timing = core.timing.as_mut().expect("worker timing initialized");
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
        runtime.force_full_cleanup = true;
        runtime.terminal_error = Some(error);
    }
    if wait_fault {
        runtime.force_full_cleanup = true;
        runtime.terminal_error = Some("wait failure injected".to_string());
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

    if runtime.terminal_error.is_none() {
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
                runtime.force_full_cleanup = true;
                runtime.terminal_error =
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
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error = Some(format!("QPC runtime failure: {error:?}"));
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
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error = Some(format!("QPC runtime failure: {error:?}"));
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
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error = Some(format!("QPC conversion failure: {error:?}"));
                    break;
                }
            }
        }};
    }

    let resources = core
        .resources
        .as_mut()
        .expect("worker resources initialized");
    let qpc_clock = resources.clock;
    let waiter = &resources.waiter;
    let backend = &mut resources.backend;
    let coordinator = &mut resources.coordinator;
    let clock_state = &mut resources.playback;
    let estimator = &mut resources.estimator;
    let telemetry = &mut resources.telemetry;

    let worker_result = catch_unwind(AssertUnwindSafe(|| {
        if runtime.terminal_error.is_some() {
            return;
        }
        while !coordinator.is_finished() {
            let loop_start_ticks = qpc_ticks_or_terminal!();
            let loop_start_us = qpc_ticks_to_us_or_terminal!(loop_start_ticks);
            local_metrics.playback_wall_time_us =
                loop_start_us.saturating_sub(timing.start_wall_time_us);
            if cpu_metrics_sample_due(
                loop_start_us,
                timing.last_cpu_metrics_sample_us,
                CPU_METRICS_SAMPLE_INTERVAL_US,
            ) {
                local_metrics.worker_cpu_time_us =
                    current_thread_cpu_time_us().saturating_sub(timing.start_thread_cpu_us);
                local_metrics.process_cpu_time_us =
                    current_process_cpu_time_us().saturating_sub(timing.start_process_cpu_us);
                timing.last_cpu_metrics_sample_us = loop_start_us;
            }
            if local_metrics.playback_wall_time_us > 0 {
                local_metrics.spin_duty_cycle_ppm = (local_metrics.spin_time_us as u128 * 1_000_000
                    / local_metrics.playback_wall_time_us as u128)
                    as u64;
            }
            try_publish_metrics(local_metrics, metrics, loop_start_us, false);
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
                    backend,
                    coordinator,
                    force_full_cleanup: &mut runtime.force_full_cleanup,
                    terminal_error: &mut runtime.terminal_error,
                    secondary_errors,
                    abort_counts,
                },
                metrics: CommandControlMetrics {
                    local_metrics,
                    metrics,
                    last_published_error,
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
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
                            Some(format!("QPC pause observation failed: {error:?}"));
                        break;
                    }
                };
                command_timing.observe_pause(observed_ticks);
            }

            if !focus_ok {
                runtime.verified_target = None;
                runtime.focus_restore_started_ticks = None;
                if !clock_state.has_pause_reason("focus") {
                    runtime.verified_target = None;
                    if let Err(error) = suspend_live_input(
                        backend,
                        coordinator,
                        target_hwnd.load(Ordering::Acquire),
                    ) {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!("focus suspension failed: {error}"));
                        break;
                    }
                    *abort_counts.entry("focus_lost").or_insert(0) += 1;
                    if let Err(error) = clock_state.enter_pause("focus", now_ticks) {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!("playback clock failure: {error}"));
                        break;
                    }
                    publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
                    try_publish_metrics(local_metrics, metrics, qpc_us_or_terminal!(), true);
                }
            } else if clock_state.has_pause_reason("focus") {
                let restored_at = *runtime.focus_restore_started_ticks.get_or_insert(now_ticks);
                let focus_grace_elapsed = match now_ticks.checked_duration_since(restored_at) {
                    Ok(elapsed) => elapsed,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
                            Some(format!("focus grace clock failure: {error}"));
                        break;
                    }
                };
                if focus_grace_elapsed >= timing.focus_restore_grace_ticks {
                    // Second idempotent release happens while the restored
                    // target is foreground, before playback can resume.
                    let preflight_target = load_target_stamp(target_hwnd, target_generation);
                    runtime.verified_target = None;
                    if let Err(error) =
                        suspend_live_input(backend, coordinator, preflight_target.hwnd)
                    {
                        runtime.verified_target = None;
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!("focus restoration failed: {error}"));
                        break;
                    }
                    if let Err(error) = ensure_preflight_for_target(
                        backend,
                        preflight_target,
                        &mut runtime.verified_target,
                    ) {
                        runtime.verified_target = None;
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!(
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
                        runtime.verified_target = None;
                        runtime.focus_restore_started_ticks = None;
                        continue;
                    }
                    // Cleanup can include bounded backend retries. Re-sample
                    // QPC after it completes so that the cleanup interval is
                    // included in the focus pause rather than lost from the
                    // playback clock.
                    let resumed_ticks = qpc_ticks_or_terminal!();
                    if let Err(error) = clock_state.exit_pause("focus", resumed_ticks) {
                        runtime.verified_target = None;
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!("playback clock failure: {error}"));
                        break;
                    }
                    if desired_pause.load(Ordering::Acquire) {
                        // Focus restoration is not the final admission when
                        // manual pause is still active. Require a separate
                        // manual-resume preflight for that epoch.
                        runtime.verified_target = None;
                    }
                    runtime.focus_restore_started_ticks = None;
                    publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
                    try_publish_metrics(local_metrics, metrics, qpc_us_or_terminal!(), true);
                }
            }

            if manual_pause && !clock_state.has_pause_reason("manual") {
                runtime.verified_target = None;
                if !clock_state.is_paused() {
                    if let Err(error) = suspend_live_input(
                        backend,
                        coordinator,
                        target_hwnd.load(Ordering::Acquire),
                    ) {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
                            Some(format!("manual pause suspension failed: {error}"));
                        break;
                    }
                    *abort_counts.entry("manual_pause").or_insert(0) += 1;
                    publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
                    try_publish_metrics(local_metrics, metrics, qpc_us_or_terminal!(), true);
                }
                if let Err(error) = clock_state.enter_pause("manual", now_ticks) {
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error = Some(format!("playback clock failure: {error}"));
                    break;
                }
            } else if !manual_pause && clock_state.has_pause_reason("manual") {
                if !clock_state.has_pause_reason("focus") {
                    // Manual resume is a new admission boundary. Keep the
                    // authored clock paused while physical cleanup/preflight
                    // runs and sample QPC only after those operations finish.
                    let preflight_target = load_target_stamp(target_hwnd, target_generation);
                    if let Err(error) = ensure_preflight_for_target(
                        backend,
                        preflight_target,
                        &mut runtime.verified_target,
                    ) {
                        runtime.verified_target = None;
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!(
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
                        runtime.verified_target = None;
                        continue;
                    }
                    let resumed_ticks = qpc_ticks_or_terminal!();
                    if let Err(error) = clock_state.exit_pause("manual", resumed_ticks) {
                        runtime.verified_target = None;
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!("playback clock failure: {error}"));
                        break;
                    }
                } else {
                    // Keep the manual pause reason until focus restoration
                    // has completed; an old QPC sample must not admit a
                    // partially resumed session.
                    runtime.verified_target = None;
                }
            }

            #[cfg(any(test, feature = "test-support"))]
            if clock_state.has_pause_reason("manual") && command_timing.needs_acknowledgment() {
                let acknowledged_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
                            Some(format!("QPC pause acknowledgment failed: {error:?}"));
                        break;
                    }
                };
                command_timing.acknowledge_pause(acknowledged_ticks);
            }

            let paused = clock_state.is_paused();
            metrics.is_paused.store(paused, Ordering::Relaxed);
            if paused {
                let pause_target = match now_ticks.checked_add_duration(timing.paused_poll_ticks) {
                    Ok(target) => target,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
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
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
                            Some(format!("pause lease deadline failure: {error:?}"));
                        break;
                    }
                };
                if let WaitOutcome::Failed(failure) = waiter
                    .wait_until_ticks_with_metrics_typed(
                        qpc_clock,
                        pause_target,
                        DurationTicks::ZERO,
                        interrupt,
                    )
                    .outcome
                {
                    if matches!(failure, WaitFailure::Clock) {
                        local_metrics.wait_clock_failures =
                            local_metrics.wait_clock_failures.saturating_add(1);
                    } else {
                        local_metrics.wait_backend_failures =
                            local_metrics.wait_backend_failures.saturating_add(1);
                    }
                    if config.timing.strict_timing || matches!(failure, WaitFailure::Clock) {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(wait_failure_message(failure));
                        break;
                    }
                    std::thread::sleep(Duration::from_micros(500));
                }
                continue;
            }

            if let Some((startup_scheduled_ticks, startup_lead_ticks)) = runtime.startup_gate {
                let target_sample_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
                            Some(format!("QPC failure before startup wait: {error:?}"));
                        break;
                    }
                };
                let target_qpc = match anchored_dispatch_target_ticks_typed(
                    target_sample_ticks,
                    clock_state.epoch,
                    startup_scheduled_ticks,
                    startup_lead_ticks,
                ) {
                    Ok(target) => target,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
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
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("lease deadline failure: {error:?}"));
                            break;
                        }
                    };
                    let wait_result = waiter.wait_until_ticks_with_metrics_typed(
                        qpc_clock,
                        bounded_target_qpc,
                        timing.effective_spin_threshold_ticks,
                        interrupt,
                    );
                    local_metrics.idle_wake_count = local_metrics.idle_wake_count.saturating_add(1);
                    local_metrics.spin_time_us = local_metrics
                        .spin_time_us
                        .saturating_add(wait_result.spin_us);
                    match wait_result.outcome {
                        WaitOutcome::Interrupted => continue,
                        WaitOutcome::Deadline => continue,
                        WaitOutcome::Failed(failure) => {
                            if matches!(failure, WaitFailure::Clock) {
                                local_metrics.wait_clock_failures =
                                    local_metrics.wait_clock_failures.saturating_add(1);
                            } else {
                                local_metrics.wait_backend_failures =
                                    local_metrics.wait_backend_failures.saturating_add(1);
                            }
                            if config.timing.strict_timing || matches!(failure, WaitFailure::Clock)
                            {
                                runtime.force_full_cleanup = true;
                                runtime.terminal_error = Some(wait_failure_message(failure));
                                break;
                            }
                            std::thread::sleep(Duration::from_micros(500));
                            continue;
                        }
                    }
                }
                runtime.startup_gate = None;
                // A first note at authored t=0 may be dispatched before the
                // future physical epoch by its lead. The typed timeline has
                // no negative value, so this one startup sample is defined as
                // logical zero; later underflow remains terminal.
                runtime.allow_pre_epoch_startup_dispatch = true;
                now_ticks = qpc_ticks_or_terminal!();
            }

            let effective_now_ticks = if runtime.allow_pre_epoch_startup_dispatch
                && now_ticks < clock_state.epoch
            {
                TimelineTicks::ZERO
            } else {
                runtime.allow_pre_epoch_startup_dispatch = false;
                match clock_state.get_elapsed_allow_pre_epoch(
                    now_ticks,
                    runtime.allow_pre_epoch_startup_dispatch,
                ) {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!("playback clock failure: {error}"));
                        break;
                    }
                }
            };
            let effective_now_us = qpc_ticks_to_us_or_terminal!(effective_now_ticks);
            local_metrics.elapsed_us = effective_now_us;
            let latency_class = match classify_latency_class(
                runtime.last_send_qpc_ticks,
                now_ticks,
                timing.cold_threshold_ticks,
            ) {
                Ok(class) => class,
                Err(error) => {
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error = Some(format!("QPC ordering failure: {error}"));
                    break;
                }
            };

            let dispatch_plan = match plan_next_dispatch(
                coordinator,
                estimator,
                qpc_clock,
                latency_class,
                &config.timing,
                config.estimator.enable_adaptive_lead,
            ) {
                Ok(plan) => plan,
                Err(error) => {
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error = Some(format!("planning failure: {error}"));
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
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error =
                        Some(format!("lead telemetry conversion failure: {error:?}"));
                    break;
                }
            };
            let due_pending = match pending_plan.as_ref() {
                Some(plan) => match coordinator.pop_due_pending_ticks(effective_now_ticks, plan) {
                    Ok(due) => due,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
                            Some(format!("coordinator pending-pop failure: {error}"));
                        break;
                    }
                },
                None => SmallVec::new(),
            };
            if !due_pending.is_empty() {
                let scan_codes: SmallVec<[u16; 15]> =
                    due_pending.iter().map(|p| p.scan_code).collect();
                let frozen_budget = build_dispatch_budget(
                    estimator,
                    DispatchPath::UpOnly {
                        up_count: scan_codes.len(),
                    },
                    latency_class,
                    health.options,
                    config.timing.strict_timing,
                );
                let started_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
                            Some(format!("QPC failure before note-off: {error:?}"));
                        break;
                    }
                };
                let started_us = qpc_ticks_to_us_or_terminal!(started_ticks);
                let actual_ticks = match clock_state.get_elapsed_allow_pre_epoch(
                    started_ticks,
                    runtime.allow_pre_epoch_startup_dispatch,
                ) {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!("playback clock failure: {error}"));
                        break;
                    }
                };
                let result = backend.key_up(&scan_codes);
                if let Some(error) = backend.timing_error.take() {
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error = Some(format!("QPC failure after note-off: {error:?}"));
                    break;
                }
                let completed_qpc_ticks = match result.send_completed_ticks {
                    Some(ticks) => ticks,
                    None => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(
                            "SendInput note-off completed without a QPC completion boundary"
                                .to_string(),
                        );
                        break;
                    }
                };
                let completed_effective_ticks = match clock_state.get_elapsed_allow_pre_epoch(
                    completed_qpc_ticks,
                    runtime.allow_pre_epoch_startup_dispatch,
                ) {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!("playback clock failure: {error}"));
                        break;
                    }
                };
                let sender_started_effective_ticks = match result.send_started_ticks {
                    Some(ticks) => match clock_state.get_elapsed_allow_pre_epoch(
                        ticks,
                        runtime.allow_pre_epoch_startup_dispatch,
                    ) {
                        Ok(value) => Some(value),
                        Err(error) => {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("playback clock failure: {error}"));
                            break;
                        }
                    },
                    None => None,
                };
                let completed_effective = qpc_ticks_to_us_or_terminal!(completed_effective_ticks);
                runtime.last_send_qpc_ticks = Some(completed_qpc_ticks);
                let recovery_required = match coordinator.requeue_failed_releases_ticks(
                    &due_pending,
                    &result.sent,
                    &result.skipped_duplicates,
                    actual_ticks,
                    completed_effective_ticks,
                    &timing.retry_backoff_ticks,
                    result.last_win32_error,
                ) {
                    Ok(required) => required,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
                            Some(format!("coordinator recovery failure: {error}"));
                        break;
                    }
                };
                if let Err(error) = coordinator.complete_releases(
                    &due_pending,
                    &result.sent,
                    &result.skipped_duplicates,
                ) {
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error =
                        Some(format!("coordinator release completion failure: {error}"));
                    break;
                }
                // A successful retry closes the recovery pause. The
                // coordinator advances one immutable timeline offset, so
                // overdue work cannot burst immediately after recovery.
                if !recovery_required {
                    match coordinator.finish_release_recovery_ticks(completed_effective_ticks) {
                        Ok(Some(recovery_pause_ticks)) => {
                            let recovery_pause_us =
                                match qpc_clock.duration_to_us(recovery_pause_ticks) {
                                    Ok(value) => value,
                                    Err(error) => {
                                        runtime.force_full_cleanup = true;
                                        runtime.terminal_error = Some(format!(
                                            "recovery telemetry conversion failure: {error:?}"
                                        ));
                                        break;
                                    }
                                };
                            local_metrics.total_us =
                                local_metrics.total_us.saturating_add(recovery_pause_us);
                        }
                        Ok(None) => {}
                        Err(error) => {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("coordinator recovery completion failure: {error}"));
                            break;
                        }
                    }
                }
                let bookkeeping_completed_us = qpc_us_or_terminal!();
                let mut first_index: Option<usize> = None;
                let mut first_deadline: Option<TimelineTicks> = None;
                for (index, pending) in due_pending.iter().enumerate() {
                    let deadline = match pending.get_effective_release_ticks(lead_up_ticks) {
                        Ok(deadline) => deadline,
                        Err(error) => {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("pending release deadline failure: {error}"));
                            break;
                        }
                    };
                    let is_better = match first_index {
                        None => true,
                        Some(best_index) => match first_deadline {
                            Some(best) => {
                                (deadline, pending.source_action_index, pending.scan_code)
                                    < (
                                        best,
                                        due_pending[best_index].source_action_index,
                                        due_pending[best_index].scan_code,
                                    )
                            }
                            None => {
                                runtime.force_full_cleanup = true;
                                runtime.terminal_error = Some(
                                    "pending release first-deadline state is inconsistent"
                                        .to_string(),
                                );
                                false
                            }
                        },
                    };
                    if is_better {
                        first_index = Some(index);
                        first_deadline = Some(deadline);
                    }
                }
                if runtime.terminal_error.is_some() {
                    break;
                }
                let Some(first_index) = first_index else {
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error =
                        Some("coordinator returned an empty pending release batch".to_string());
                    break;
                };
                let Some(effective_deadline_ticks) = first_deadline else {
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error =
                        Some("coordinator returned no release deadline".to_string());
                    break;
                };
                let first = &due_pending[first_index];
                let Some(scheduled_ticks) = due_pending
                    .iter()
                    .map(|pending| pending.scheduled_release_ticks)
                    .min()
                else {
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error =
                        Some("pending release batch has no scheduled timestamp".to_string());
                    break;
                };
                let scheduled_us = match qpc_clock.timeline_to_us(scheduled_ticks) {
                    Ok(value) => value,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!(
                            "pending release telemetry conversion failure: {error:?}"
                        ));
                        break;
                    }
                };
                let mut deferred_by_us = 0u64;
                for pending in &due_pending {
                    let ready_ticks = pending
                        .release_not_before_ticks
                        .max(pending.next_retry_ticks);
                    let deferred_ticks =
                        match ready_ticks.checked_duration_since(pending.scheduled_release_ticks) {
                            Ok(value) => value,
                            Err(sky_dispatch_core::time::TimeArithmeticError::NegativeOrder) => {
                                DurationTicks::ZERO
                            }
                            Err(error) => {
                                runtime.force_full_cleanup = true;
                                runtime.terminal_error =
                                    Some(format!("pending deferral arithmetic failure: {error}"));
                                break;
                            }
                        };
                    let deferred_us = match qpc_clock.duration_to_us(deferred_ticks) {
                        Ok(value) => value,
                        Err(error) => {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("pending deferral conversion failure: {error:?}"));
                            break;
                        }
                    };
                    deferred_by_us = deferred_by_us.max(deferred_us);
                }
                if runtime.terminal_error.is_some() {
                    break;
                }
                let mixed_source = due_pending.iter().any(|pending| {
                    pending.source_action_index != first.source_action_index
                        || pending.reason_id != first.reason_id
                });
                let up_completion_lateness_ticks = completed_effective_ticks
                    .checked_duration_since(scheduled_ticks)
                    .ok();
                let up_completion_error_ticks = match signed_timeline_delta_ticks(
                    completed_effective_ticks,
                    effective_deadline_ticks,
                ) {
                    Ok(value) => value,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
                            Some(format!("note-off timing conversion failure: {error}"));
                        break;
                    }
                };
                let up_authored_completion_error_ticks =
                    match signed_timeline_delta_ticks(completed_effective_ticks, scheduled_ticks) {
                        Ok(value) => value,
                        Err(error) => {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error = Some(format!(
                                "note-off authored timing conversion failure: {error}"
                            ));
                            break;
                        }
                    };
                let up_completion_error_us =
                    match signed_ticks_to_us(qpc_clock, up_completion_error_ticks) {
                        Ok(value) => value,
                        Err(error) => {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("note-off timing conversion failure: {error}"));
                            break;
                        }
                    };
                let clean_up_sample = result.success
                    && result.sent.len() == scan_codes.len()
                    && result.skipped_duplicates.is_empty()
                    && result.send_attempts == 1
                    && deferred_by_us == 0
                    && !mixed_source;
                let strict_up_completion_late = config.timing.strict_timing
                    && clean_up_sample
                    && up_completion_lateness_ticks
                        .is_some_and(|late| late > timing.strict_up_completion_late_ticks);
                let up_saturated_positive = pending_plan
                    .as_ref()
                    .is_some_and(|plan| plan.lead_saturated)
                    && up_completion_lateness_ticks.is_some();
                health.up_saturation_positive_streak = if up_saturated_positive {
                    health.up_saturation_positive_streak.saturating_add(1)
                } else {
                    0
                };
                let saturation_abort = config.timing.strict_timing
                    && health.up_saturation_positive_streak >= STRICT_SATURATION_ABORT_STREAK;
                let release_outcome = if strict_up_completion_late {
                    "strict_completion_slo_exceeded"
                } else {
                    release_runtime_outcome(
                        deferred_by_us,
                        result.sent.len(),
                        scan_codes.len(),
                        recovery_required,
                    )
                };
                let mut trace_flags = 0;
                if result.sent.len() == scan_codes.len() {
                    trace_flags |= TRACE_FLAG_SENT_FULL;
                }
                if release_outcome == "deferred_release" || release_outcome == "failed_note_off" {
                    trace_flags |= TRACE_FLAG_RECOVERY;
                }
                if deferred_by_us > 0 {
                    trace_flags |= TRACE_FLAG_DEFERRED;
                }
                if release_outcome != "sent" {
                    trace_flags |= TRACE_FLAG_ANOMALY;
                }
                if let Err(error) = telemetry.try_push(|| {
                    RtTraceRecord::dispatched(
                        TraceContext {
                            event_index: first.source_action_index,
                            kind: TRACE_KIND_UP,
                            outcome: trace_outcome_code(release_outcome),
                            polyphony: scan_codes.len(),
                            flags: trace_flags,
                            win32_error: result.last_win32_error.unwrap_or(0),
                        },
                        TraceTiming {
                            authored_ticks: scheduled_ticks,
                            effective_deadline_ticks,
                            wake_ticks: actual_ticks,
                            send_started_ticks: sender_started_effective_ticks,
                            send_completed_ticks: Some(completed_effective_ticks),
                            bookkeeping_duration_us: bookkeeping_completed_us
                                .saturating_sub(result.send_completed_us),
                            completion_error_ticks: up_completion_error_ticks,
                            authored_completion_error_ticks: up_authored_completion_error_ticks,
                            applied_lead_ticks: lead_up_ticks,
                        },
                        TraceDelivery {
                            requested: scan_codes.len(),
                            sent: result.sent.len(),
                            skipped: result.skipped_duplicates.len(),
                            send_attempts: usize::from(result.send_attempts),
                        },
                    )
                }) {
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error =
                        Some(format!("native telemetry record overflow: {error}"));
                    break;
                }
                if config.estimator.enable_adaptive_lead
                    && pending_plan
                        .as_ref()
                        .is_some_and(|plan| plan.lead_saturated)
                {
                    record_lead_saturation(
                        &mut local_metrics.lead_saturation_count_up,
                        &mut local_metrics.positive_residual_at_cap,
                        scan_codes.len(),
                        signed_delta(completed_effective, scheduled_us),
                    );
                }
                runtime.pending_pre_send_spin_us = 0;
                let send_warn_threshold_us = frozen_budget.send_warn_us;
                local_metrics.send_warn_threshold_us = frozen_budget.send_warn_us;
                local_metrics.bookkeeping_warn_threshold_us = frozen_budget.bookkeeping_warn_us;
                local_metrics.send_up_warn_threshold_us = frozen_budget.send_warn_us;
                local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
                let send_duration_us = result.send_completed_us.saturating_sub(started_us);
                if config.estimator.enable_adaptive_lead
                    && let Err(error) = update_estimator_after_send_class(
                        estimator,
                        ActionKind::Up,
                        result.send_completed_us.saturating_sub(started_us),
                        result.sent.len(),
                        scan_codes.len(),
                        lead_up,
                        up_completion_error_us,
                        clean_up_sample,
                        latency_class,
                    )
                {
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error = Some(format!("estimator update failure: {error}"));
                    break;
                }
                let deferred_release = deferred_by_us > 0;
                record_lateness(
                    signed_delta(completed_effective, scheduled_us),
                    true,
                    deferred_release,
                    local_metrics,
                );
                publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
                try_publish_metrics(
                    local_metrics,
                    metrics,
                    qpc_us_or_terminal!(),
                    !clean_up_sample || recovery_required,
                );
                let iteration_ready_us = qpc_us_or_terminal!();
                observe_dispatch_health(
                    DispatchHealthObservation {
                        send_duration_us,
                        post_send_duration_us: iteration_ready_us
                            .saturating_sub(result.send_completed_us),
                        path: frozen_budget.path,
                        send_warn_us: send_warn_threshold_us,
                        bookkeeping_warn_us: frozen_budget.bookkeeping_warn_us,
                        elapsed_us: completed_effective,
                    },
                    health.options.window_policy(),
                    &mut health.send_pure_window,
                    &mut health.bookkeeping_window,
                    local_metrics,
                );
                if recovery_required {
                    runtime.verified_target = None;
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error = Some(format!(
                        "note-off recovery exhausted after {} retries{}",
                        sky_dispatch_core::coordinator::MAX_RELEASE_RETRIES,
                        result
                            .last_win32_error
                            .map_or(String::new(), |error| format!(" (Win32 error {error})"))
                    ));
                    let recovery_cleanup =
                        backend.release_all_full_instrument(target_hwnd.load(Ordering::Acquire));
                    if !release_state_verified(backend, &recovery_cleanup) {
                        record_termination_error(
                            &mut runtime.terminal_error,
                            secondary_errors,
                            format!(
                                "recovery cleanup release verification failed: {}",
                                describe_release_outcome(&recovery_cleanup)
                            ),
                        );
                    }
                    cancel_coordinator_or_terminal(
                        coordinator,
                        &mut runtime.force_full_cleanup,
                        &mut runtime.terminal_error,
                        secondary_errors,
                    );
                    break;
                }
                if strict_up_completion_late {
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error = Some(format!(
                        "strict timing completion SLO exceeded for note-off at action {}: completion was {}us late",
                        first.source_action_index, up_completion_error_us
                    ));
                    break;
                }
                if saturation_abort {
                    runtime.force_full_cleanup = true;
                    runtime.terminal_error = Some(format!(
                        "strict timing SLO exceeded: note-off lead saturated with positive residual for {} consecutive dispatches",
                        STRICT_SATURATION_ABORT_STREAK
                    ));
                    break;
                }
                continue;
            }

            let (lead_down, lead_down_saturated, lead_down_ticks) =
                match dispatch_plan.authored.as_ref() {
                    Some(authored) => (
                        authored.lead_us,
                        authored.lead_saturated,
                        authored.lead_ticks,
                    ),
                    None => (0, false, DurationTicks::ZERO),
                };

            let prepared_batch =
                match coordinator.prepare_next_due_authored(effective_now_ticks, lead_down_ticks) {
                    Ok(value) => value,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
                            Some(format!("coordinator authored-prepare failure: {error}"));
                        break;
                    }
                };
            local_metrics.timeline_rebase_count = coordinator.timeline_rebase_count();
            local_metrics.timeline_rebase_total_us =
                match qpc_clock.duration_to_us(coordinator.timeline_rebase_total_ticks()) {
                    Ok(value) => value,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!(
                            "timeline rebase telemetry conversion failure: {error:?}"
                        ));
                        break;
                    }
                };
            local_metrics.timeline_rebase_max_us =
                match qpc_clock.duration_to_us(coordinator.timeline_rebase_max_ticks()) {
                    Ok(value) => value,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!(
                            "timeline rebase telemetry conversion failure: {error:?}"
                        ));
                        break;
                    }
                };
            local_metrics.timeline_rebase_last_reason = match coordinator
                .last_timeline_rebase_reason()
            {
                None => 0,
                Some(sky_dispatch_core::coordinator::TimelineRebaseReason::WorkerLate) => 1,
                Some(sky_dispatch_core::coordinator::TimelineRebaseReason::ReleaseFloor) => 2,
                Some(sky_dispatch_core::coordinator::TimelineRebaseReason::ReleaseRecovery) => 3,
            };
            if let Some(prepared_batch) = prepared_batch {
                let batch_index = prepared_batch.index;
                // --- Borrow scope: extract all scalar and stack data before any &mut call ---
                // `batch_view` borrows from `coordinator.schedule`. We must not call any
                // `&mut coordinator` method until this scope ends. Pull every field we need
                // into Copy / stack-owned values here.
                let batch_scheduled_ticks = prepared_batch.effective_scheduled_ticks;
                let packet_mode = match prepared_batch.packet_kind {
                    Some(sky_dispatch_core::model::PhysicalPacketKind::DownOnly)
                        if prepared_batch.packet_batch_count == 1 =>
                    {
                        false
                    }
                    Some(_) => true,
                    None => false,
                };
                let (
                    batch_kind,
                    dispatch_path,
                    batch_source_action_index,
                    batch_intent_count,
                    conflict_mask,
                    scan_batch,
                    packet_masks,
                ) = if packet_mode {
                    let packet_view = match coordinator
                        .schedule
                        .view_packet_ticks(prepared_batch.packet_index, batch_scheduled_ticks)
                    {
                        Ok(value) => value,
                        Err(error) => {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("runtime packet view failure: {error}"));
                            break;
                        }
                    };
                    let conflict_mask = coordinator.check_packet_down_conflicts(
                        packet_view.up_mask(),
                        packet_view.down_intents,
                    );
                    let up_count = packet_view.up_mask().count_ones() as usize;
                    let down_count = packet_view.down_mask().count_ones() as usize;
                    let dispatch_path = match prepared_batch.packet_kind {
                        Some(sky_dispatch_core::model::PhysicalPacketKind::UpOnly) => {
                            DispatchPath::UpOnly { up_count }
                        }
                        Some(sky_dispatch_core::model::PhysicalPacketKind::DownOnly) => {
                            DispatchPath::DownOnly { down_count }
                        }
                        Some(sky_dispatch_core::model::PhysicalPacketKind::Mixed) => {
                            DispatchPath::Mixed {
                                up_count,
                                down_count,
                            }
                        }
                        None => DispatchPath::DownOnly { down_count: 0 },
                    };
                    (
                        if matches!(dispatch_path, DispatchPath::UpOnly { .. }) {
                            ActionKind::Up
                        } else {
                            ActionKind::Down
                        },
                        dispatch_path,
                        packet_view
                            .header
                            .down_source_action_index
                            .or_else(|| {
                                coordinator
                                    .schedule
                                    .batches
                                    .get(packet_view.header.first_batch_index as usize)
                                    .map(|batch| batch.source_action_index)
                            })
                            .unwrap_or(0),
                        up_count + down_count,
                        conflict_mask,
                        packet_view.down_scan_code_batch(),
                        Some(sky_dispatch_win32::input::PhysicalPacket::new(
                            packet_view.up_mask(),
                            packet_view.down_mask(),
                        )),
                    )
                } else {
                    let batch_view = match coordinator
                        .schedule
                        .view_batch_ticks(batch_index, batch_scheduled_ticks)
                    {
                        Ok(value) => value,
                        Err(error) => {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("runtime schedule view failure: {error}"));
                            break;
                        }
                    };
                    let conflict_mask =
                        coordinator.check_down_conflicts_compact(batch_view.intents);
                    (
                        batch_view.kind(),
                        match batch_view.kind() {
                            ActionKind::Up => DispatchPath::UpOnly {
                                up_count: batch_view.intents.len(),
                            },
                            ActionKind::Down => DispatchPath::DownOnly {
                                down_count: batch_view.intents.len(),
                            },
                        },
                        batch_view.source_action_index(),
                        batch_view.intents.len(),
                        conflict_mask,
                        batch_view.scan_code_batch_excluding_mask(conflict_mask),
                        None,
                    )
                };
                let batch_scheduled_us = match qpc_clock.timeline_to_us(batch_scheduled_ticks) {
                    Ok(value) => value,
                    Err(error) => {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
                            Some(format!("schedule telemetry conversion failure: {error:?}"));
                        break;
                    }
                };
                let authored_batch_scheduled_ticks = coordinator.batch_scheduled_ticks[batch_index];
                let authored_batch_scheduled_us =
                    match qpc_clock.timeline_to_us(authored_batch_scheduled_ticks) {
                        Ok(value) => value,
                        Err(error) => {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error = Some(format!(
                                "authored schedule telemetry conversion failure: {error:?}"
                            ));
                            break;
                        }
                    };
                let has_conflicts = conflict_mask != 0;
                // --- End of borrow scope: all data is now in stack-local copies ---

                if packet_masks.is_some() || batch_kind == ActionKind::Down {
                    let mut force_dispatch_publish;
                    // Repeat the foreground comparison at the final boundary
                    // immediately before SendInput. If focus changed after
                    // the outer-loop sample, terminalize this authored batch;
                    // it must not be replayed after the focus grace period.
                    if !packet_masks.is_some_and(|packet| packet.down_mask == 0)
                        && !focus_matches(config.focus.require_focus, focus_active, target_hwnd)
                    {
                        // The batch was only prepared. Leave the cursor and generation
                        // ledger untouched so the same authored chord can be prepared again
                        // after focus restoration.
                        if let Err(error) = suspend_live_input(
                            backend,
                            coordinator,
                            target_hwnd.load(Ordering::Acquire),
                        ) {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("focus suspension failed: {error}"));
                            break;
                        }
                        if let Err(error) = clock_state.enter_pause("focus", now_ticks) {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("playback clock failure: {error}"));
                            break;
                        }
                        runtime.focus_restore_started_ticks = None;
                        if let Err(error) = telemetry.try_push(|| {
                            RtTraceRecord::dispatched(
                                TraceContext {
                                    event_index: batch_source_action_index,
                                    kind: TRACE_KIND_DOWN,
                                    outcome: trace_outcome_code("blocked_unfocused"),
                                    polyphony: batch_intent_count,
                                    flags: TRACE_FLAG_ANOMALY,
                                    win32_error: 0,
                                },
                                TraceTiming {
                                    authored_ticks: authored_batch_scheduled_ticks,
                                    effective_deadline_ticks: batch_scheduled_ticks,
                                    wake_ticks: effective_now_ticks,
                                    send_started_ticks: None,
                                    send_completed_ticks: None,
                                    bookkeeping_duration_us: 0,
                                    completion_error_ticks: 0,
                                    authored_completion_error_ticks: 0,
                                    applied_lead_ticks: lead_down_ticks,
                                },
                                TraceDelivery {
                                    requested: 0,
                                    sent: 0,
                                    skipped: 0,
                                    send_attempts: 0,
                                },
                            )
                        }) {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("native telemetry record overflow: {error}"));
                            break;
                        }
                        publish_backend_metrics(
                            backend,
                            local_metrics,
                            metrics,
                            last_published_error,
                        );
                        try_publish_metrics(local_metrics, metrics, qpc_us_or_terminal!(), true);
                        continue;
                    }
                    if !packet_masks.is_some_and(|packet| packet.down_mask == 0)
                        && focus_loss_fault
                        && !runtime.focus_loss_fault_injected
                    {
                        runtime.focus_loss_fault_injected = true;
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(
                            "focus lost after due check before SendInput boundary".to_string(),
                        );
                        break;
                    }
                    let preflight_target = load_target_stamp(target_hwnd, target_generation);
                    if let Err(error) = ensure_preflight_for_target(
                        backend,
                        preflight_target,
                        &mut runtime.verified_target,
                    ) {
                        runtime.verified_target = None;
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!(
                            "instrument key preflight failed; release the 15 instrument keys before playback: {error}"
                        ));
                        break;
                    }
                    if !target_stamp_still_current(target_hwnd, target_generation, preflight_target)
                    {
                        runtime.verified_target = None;
                        continue;
                    }
                    if config.timing.strict_timing
                        && effective_now_ticks
                            .checked_duration_since(authored_batch_scheduled_ticks)
                            .is_ok_and(|late| late > timing.hard_late_abort_threshold_ticks)
                    {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!(
                            "authored Down exceeded hard lateness safety threshold of {}us",
                            HARD_LATE_ABORT_THRESHOLD_US
                        ));
                        break;
                    }
                    // A runtime conflict means coordinator/backend state no
                    // longer matches the compiled schedule. It is an
                    // invariant failure, never a user-selectable drop mode.
                    if has_conflicts {
                        local_metrics.authored_conflict_events =
                            local_metrics.authored_conflict_events.saturating_add(1);
                        local_metrics.authored_chords_rejected =
                            local_metrics.authored_chords_rejected.saturating_add(1);
                        local_metrics.authored_keys_rejected = local_metrics
                            .authored_keys_rejected
                            .saturating_add(batch_intent_count as u64);
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error = Some(format!(
                            "unexpected blocked authored Down at action {}",
                            batch_source_action_index
                        ));
                        break;
                    }

                    if packet_masks.is_some() || !scan_batch.is_empty() {
                        // Preflight can perform multiple Win32 calls. Keep
                        // the final admission bound to the exact stamp that
                        // was verified and let command races return to the
                        // worker control path without becoming send failures.
                        let admission = if packet_masks.is_some_and(|packet| packet.down_mask == 0)
                        {
                            DownAdmission::Allowed
                        } else {
                            final_down_admission(
                                preflight_target,
                                config.focus.require_focus,
                                focus_active,
                                target_hwnd,
                                target_generation,
                                quit_requested,
                                skip_requested,
                                panic_requested,
                                desired_pause,
                            )
                        };
                        match admission {
                            DownAdmission::Allowed => {}
                            DownAdmission::FocusLost => {
                                runtime.verified_target = None;
                                let focus_ticks = qpc_ticks_or_terminal!();
                                if let Err(error) = suspend_live_input(
                                    backend,
                                    coordinator,
                                    target_hwnd.load(Ordering::Acquire),
                                ) {
                                    runtime.force_full_cleanup = true;
                                    runtime.terminal_error =
                                        Some(format!("focus suspension failed: {error}"));
                                    break;
                                }
                                if let Err(error) = clock_state.enter_pause("focus", focus_ticks) {
                                    runtime.force_full_cleanup = true;
                                    runtime.terminal_error = Some(format!(
                                        "playback clock failure after final focus check: {error}"
                                    ));
                                    break;
                                }
                                runtime.focus_restore_started_ticks = None;
                                publish_backend_metrics(
                                    backend,
                                    local_metrics,
                                    metrics,
                                    last_published_error,
                                );
                                try_publish_metrics(
                                    local_metrics,
                                    metrics,
                                    qpc_us_or_terminal!(),
                                    true,
                                );
                                continue;
                            }
                            DownAdmission::TargetChanged
                            | DownAdmission::PauseRequested
                            | DownAdmission::QuitRequested
                            | DownAdmission::SkipRequested
                            | DownAdmission::PanicRequested => {
                                runtime.verified_target = None;
                                continue;
                            }
                        }
                        // A compiled same-timestamp packet crosses the
                        // platform boundary exactly once. Legacy single
                        // batches retain their existing sender seam until
                        // the compatibility path is removed.
                        let frozen_budget = build_dispatch_budget(
                            estimator,
                            dispatch_path,
                            latency_class,
                            health.options,
                            config.timing.strict_timing,
                        );
                        let result = if let Some(packet) = packet_masks {
                            backend.key_down_physical_packet(packet)
                        } else {
                            backend.key_down(scan_batch.as_slice())
                        };
                        if let Some(error) = backend.timing_error.take() {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("QPC failure after note-on: {error:?}"));
                            break;
                        }

                        let trace_kind = match prepared_batch.packet_kind {
                            Some(sky_dispatch_core::model::PhysicalPacketKind::UpOnly) => {
                                TRACE_KIND_UP
                            }
                            Some(sky_dispatch_core::model::PhysicalPacketKind::DownOnly) => {
                                TRACE_KIND_DOWN
                            }
                            Some(sky_dispatch_core::model::PhysicalPacketKind::Mixed) => {
                                TRACE_KIND_MIXED
                            }
                            None => TRACE_KIND_DOWN,
                        };

                        let (
                            result_started_ticks,
                            result_completed_us,
                            result_completed_ticks,
                            result_sent,
                            result_skipped_duplicates,
                            result_send_attempts,
                            _result_zero_progress_retries,
                            result_retry_reason,
                            result_chord_integrity_lost,
                            _result_first_win32_error,
                            result_last_win32_error,
                            result_success,
                        ) = match result {
                            sky_dispatch_win32::input::DownSendOutcome::Complete {
                                started_ticks,
                                completed_us,
                                completed_ticks,
                                sent,
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                retry_reason,
                                ..
                            } => (
                                started_ticks,
                                completed_us,
                                completed_ticks,
                                sent,
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                retry_reason,
                                false,
                                None,
                                None,
                                true,
                            ),
                            sky_dispatch_win32::input::DownSendOutcome::ZeroProgress {
                                started_ticks,
                                completed_us,
                                completed_ticks,
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                retry_reason,
                                first_error,
                                last_error,
                                ..
                            } => (
                                started_ticks,
                                completed_us,
                                completed_ticks,
                                smallvec::SmallVec::<[u16; 15]>::new(),
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                retry_reason,
                                false,
                                first_error,
                                last_error,
                                false,
                            ),
                            sky_dispatch_win32::input::DownSendOutcome::IntegrityLost {
                                started_ticks,
                                completed_us,
                                completed_ticks,
                                sent,
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                retry_reason,
                                first_error,
                                last_error,
                                ..
                            } => (
                                started_ticks,
                                completed_us,
                                completed_ticks,
                                sent,
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                retry_reason,
                                true,
                                first_error,
                                last_error,
                                false,
                            ),
                        };

                        if !result_success {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error = Some(format!(
                                "authored Down send integrity failure at action {}",
                                batch_source_action_index
                            ));
                            break;
                        }

                        // The sender owns the exact syscall boundary. The
                        // admission QPC sample was intentionally removed so
                        // coordinator activation and sender-duration metrics
                        // cannot use a timestamp taken before final checks.
                        let sender_started_ticks = match result_started_ticks {
                            Some(ticks) => ticks,
                            None => {
                                runtime.force_full_cleanup = true;
                                runtime.terminal_error = Some(
                                    "SendInput note-on succeeded without a QPC start boundary"
                                        .to_string(),
                                );
                                break;
                            }
                        };
                        let completed_qpc_ticks = match result_completed_ticks {
                            Some(ticks) => ticks,
                            None => {
                                runtime.force_full_cleanup = true;
                                runtime.terminal_error = Some(
                                    "SendInput note-on completed without a QPC completion boundary"
                                        .to_string(),
                                );
                                break;
                            }
                        };
                        let sender_duration_ticks = match completed_qpc_ticks
                            .checked_duration_since(sender_started_ticks)
                        {
                            Ok(duration) => duration,
                            Err(error) => {
                                runtime.force_full_cleanup = true;
                                runtime.terminal_error =
                                    Some(format!("note-on QPC ordering failure: {error}"));
                                break;
                            }
                        };
                        let sender_duration_us =
                            match qpc_clock.duration_to_us(sender_duration_ticks) {
                                Ok(duration) => duration,
                                Err(error) => {
                                    runtime.force_full_cleanup = true;
                                    runtime.terminal_error = Some(format!(
                                        "note-on sender duration conversion failure: {error:?}"
                                    ));
                                    break;
                                }
                            };
                        let sender_started_effective_ticks = match clock_state
                            .get_elapsed_allow_pre_epoch(
                                sender_started_ticks,
                                runtime.allow_pre_epoch_startup_dispatch,
                            ) {
                            Ok(ticks) => ticks,
                            Err(error) => {
                                runtime.force_full_cleanup = true;
                                runtime.terminal_error =
                                    Some(format!("playback clock failure: {error}"));
                                break;
                            }
                        };
                        let completed_effective_ticks = match clock_state
                            .get_elapsed_allow_pre_epoch(
                                completed_qpc_ticks,
                                runtime.allow_pre_epoch_startup_dispatch,
                            ) {
                            Ok(ticks) => ticks,
                            Err(error) => {
                                runtime.force_full_cleanup = true;
                                runtime.terminal_error =
                                    Some(format!("playback clock failure: {error}"));
                                break;
                            }
                        };
                        let completed_effective =
                            qpc_ticks_to_us_or_terminal!(completed_effective_ticks);
                        runtime.last_send_qpc_ticks = Some(completed_qpc_ticks);
                        let commit_result = if packet_mode {
                            coordinator.commit_packet_success(
                                prepared_batch,
                                sender_started_effective_ticks,
                                completed_effective_ticks,
                            )
                        } else {
                            coordinator.commit_down_success(
                                prepared_batch,
                                &result_sent,
                                sender_started_effective_ticks,
                                completed_effective_ticks,
                            )
                        };
                        if let Err(error) = commit_result {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("coordinator activation failure: {error}"));
                            break;
                        }
                        let completion_lateness_ticks = completed_effective_ticks
                            .checked_duration_since(batch_scheduled_ticks)
                            .ok();
                        let completion_error_ticks_value = match signed_timeline_delta_ticks(
                            completed_effective_ticks,
                            batch_scheduled_ticks,
                        ) {
                            Ok(value) => value,
                            Err(error) => {
                                runtime.force_full_cleanup = true;
                                runtime.terminal_error =
                                    Some(format!("note-on timing conversion failure: {error}"));
                                break;
                            }
                        };
                        let authored_completion_error_ticks_value =
                            match signed_timeline_delta_ticks(
                                completed_effective_ticks,
                                authored_batch_scheduled_ticks,
                            ) {
                                Ok(value) => value,
                                Err(error) => {
                                    runtime.force_full_cleanup = true;
                                    runtime.terminal_error = Some(format!(
                                        "note-on authored timing conversion failure: {error}"
                                    ));
                                    break;
                                }
                            };
                        let completion_error_us =
                            match signed_ticks_to_us(qpc_clock, completion_error_ticks_value) {
                                Ok(value) => value,
                                Err(error) => {
                                    runtime.force_full_cleanup = true;
                                    runtime.terminal_error =
                                        Some(format!("note-on timing conversion failure: {error}"));
                                    break;
                                }
                            };
                        let requested_count = dispatch_path.event_count();
                        let delivered_count = if packet_mode {
                            usize::from(result_success) * requested_count
                        } else {
                            result_sent.len()
                        };
                        let estimator_kind = estimator_kind_for_path(dispatch_path);
                        let clean_directional_sample = result_success
                            && result_skipped_duplicates.is_empty()
                            && result_send_attempts == 1
                            && !result_chord_integrity_lost
                            && !matches!(dispatch_path, DispatchPath::Mixed { .. })
                            && estimator_kind.is_some()
                            && delivered_count == requested_count;
                        let recovered_zero_progress = matches!(
                            result_retry_reason,
                            sky_dispatch_win32::input::PacketRetryReason::ZeroProgress
                        );
                        let recovered_partial_up = matches!(
                            (dispatch_path, result_retry_reason),
                            (
                                DispatchPath::UpOnly { .. },
                                sky_dispatch_win32::input::PacketRetryReason::PartialProgress { .. }
                            )
                        ) && result_success;
                        let recovered_retry_late = recovered_zero_progress
                            && result_success
                            && completion_lateness_ticks
                                .is_some_and(|late| late > timing.retry_late_threshold_ticks);
                        let retry_late_abort = config.timing.strict_timing && recovered_retry_late;
                        let strict_completion_late = config.timing.strict_timing
                            && clean_directional_sample
                            && completion_lateness_ticks.is_some_and(|late| {
                                late > match dispatch_path {
                                    DispatchPath::UpOnly { .. } => {
                                        timing.strict_up_completion_late_ticks
                                    }
                                    DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
                                        timing.strict_down_completion_late_ticks
                                    }
                                }
                            });
                        if recovered_retry_late {
                            local_metrics.recovered_zero_progress_but_late = local_metrics
                                .recovered_zero_progress_but_late
                                .saturating_add(1);
                        }
                        let saturation_abort = match dispatch_path {
                            DispatchPath::UpOnly { .. } => {
                                health.down_saturation_positive_streak = 0;
                                health.up_saturation_positive_streak =
                                    if lead_down_saturated && completion_lateness_ticks.is_some() {
                                        health.up_saturation_positive_streak.saturating_add(1)
                                    } else {
                                        0
                                    };
                                config.timing.strict_timing
                                    && health.up_saturation_positive_streak
                                        >= STRICT_SATURATION_ABORT_STREAK
                            }
                            DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
                                health.up_saturation_positive_streak = 0;
                                health.down_saturation_positive_streak =
                                    if lead_down_saturated && completion_lateness_ticks.is_some() {
                                        health.down_saturation_positive_streak.saturating_add(1)
                                    } else {
                                        0
                                    };
                                config.timing.strict_timing
                                    && health.down_saturation_positive_streak
                                        >= STRICT_SATURATION_ABORT_STREAK
                            }
                        };
                        let bookkeeping_completed_us = qpc_us_or_terminal!();
                        if recovered_zero_progress && result_success {
                            local_metrics.recovered_zero_progress_retries = local_metrics
                                .recovered_zero_progress_retries
                                .saturating_add(1);
                        }
                        if recovered_partial_up {
                            local_metrics.recovered_partial_up_retries =
                                local_metrics.recovered_partial_up_retries.saturating_add(1);
                        }
                        let down_outcome = if recovered_retry_late {
                            "recovered_zero_progress_but_late"
                        } else if recovered_partial_up {
                            "recovered_partial_up_retry"
                        } else if strict_completion_late {
                            "strict_completion_slo_exceeded"
                        } else if result_chord_integrity_lost {
                            "chord_integrity_lost"
                        } else if packet_masks.is_some_and(|_| result_success)
                            || (packet_masks.is_none() && result_sent.len() == scan_batch.len())
                        {
                            "sent"
                        } else {
                            "partial_note_on"
                        };
                        force_dispatch_publish = !result_success
                            || !matches!(
                                result_retry_reason,
                                sky_dispatch_win32::input::PacketRetryReason::None
                            )
                            || result_chord_integrity_lost;
                        let mut trace_flags = 0;
                        let send_completed_fully = packet_masks.is_some_and(|_| result_success)
                            || (packet_masks.is_none() && result_sent.len() == scan_batch.len());
                        if send_completed_fully {
                            trace_flags |= TRACE_FLAG_SENT_FULL;
                        }
                        if recovered_retry_late || result_chord_integrity_lost {
                            trace_flags |= TRACE_FLAG_RECOVERY;
                        }
                        if down_outcome != "sent" {
                            trace_flags |= TRACE_FLAG_ANOMALY;
                        }
                        if let Err(error) = telemetry.try_push(|| {
                            RtTraceRecord::dispatched(
                                TraceContext {
                                    event_index: batch_source_action_index,
                                    kind: trace_kind,
                                    outcome: trace_outcome_code(down_outcome),
                                    polyphony: batch_intent_count,
                                    flags: trace_flags,
                                    win32_error: result_last_win32_error.unwrap_or(0),
                                },
                                TraceTiming {
                                    authored_ticks: authored_batch_scheduled_ticks,
                                    effective_deadline_ticks: batch_scheduled_ticks,
                                    wake_ticks: effective_now_ticks,
                                    send_started_ticks: Some(sender_started_effective_ticks),
                                    send_completed_ticks: Some(completed_effective_ticks),
                                    bookkeeping_duration_us: bookkeeping_completed_us
                                        .saturating_sub(result_completed_us),
                                    completion_error_ticks: completion_error_ticks_value,
                                    authored_completion_error_ticks:
                                        authored_completion_error_ticks_value,
                                    applied_lead_ticks: lead_down_ticks,
                                },
                                TraceDelivery {
                                    requested: batch_intent_count,
                                    sent: if packet_masks.is_some() && result_success {
                                        requested_count
                                    } else {
                                        result_sent.len()
                                    },
                                    skipped: result_skipped_duplicates.len(),
                                    send_attempts: usize::from(result_send_attempts),
                                },
                            )
                        }) {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("native telemetry record overflow: {error}"));
                            break;
                        }
                        if config.estimator.enable_adaptive_lead && lead_down_saturated {
                            match dispatch_path {
                                DispatchPath::UpOnly { .. } => record_lead_saturation(
                                    &mut local_metrics.lead_saturation_count_up,
                                    &mut local_metrics.positive_residual_at_cap,
                                    batch_intent_count,
                                    signed_delta(completed_effective, batch_scheduled_us),
                                ),
                                DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
                                    record_lead_saturation(
                                        &mut local_metrics.lead_saturation_count_down,
                                        &mut local_metrics.positive_residual_at_cap,
                                        batch_intent_count,
                                        signed_delta(completed_effective, batch_scheduled_us),
                                    )
                                }
                            }
                        }
                        runtime.pending_pre_send_spin_us = 0;
                        let send_warn_threshold_us = frozen_budget.send_warn_us;
                        local_metrics.send_warn_threshold_us = frozen_budget.send_warn_us;
                        local_metrics.bookkeeping_warn_threshold_us =
                            frozen_budget.bookkeeping_warn_us;
                        match dispatch_path {
                            DispatchPath::DownOnly { .. } => {
                                local_metrics.send_down_warn_threshold_us =
                                    frozen_budget.send_warn_us;
                            }
                            DispatchPath::UpOnly { .. } => {
                                local_metrics.send_up_warn_threshold_us =
                                    frozen_budget.send_warn_us;
                            }
                            DispatchPath::Mixed { .. } => {
                                local_metrics.send_mixed_warn_threshold_us =
                                    frozen_budget.send_warn_us;
                            }
                        }
                        local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
                        if config.estimator.enable_adaptive_lead
                            && let Some(kind) = estimator_kind
                            && let Err(error) = update_estimator_after_send_class(
                                estimator,
                                kind,
                                sender_duration_us,
                                delivered_count,
                                batch_intent_count,
                                lead_down,
                                completion_error_us,
                                clean_directional_sample,
                                latency_class,
                            )
                        {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("estimator update failure: {error}"));
                            break;
                        }
                        record_lateness(
                            signed_delta(completed_effective, authored_batch_scheduled_us),
                            false,
                            false,
                            local_metrics,
                        );
                        let terminal_dispatch = result_chord_integrity_lost
                            || retry_late_abort
                            || strict_completion_late
                            || saturation_abort;
                        if terminal_dispatch {
                            force_dispatch_publish = true;
                        }
                        publish_backend_metrics(
                            backend,
                            local_metrics,
                            metrics,
                            last_published_error,
                        );
                        try_publish_metrics(
                            local_metrics,
                            metrics,
                            qpc_us_or_terminal!(),
                            force_dispatch_publish,
                        );
                        let iteration_ready_us = qpc_us_or_terminal!();
                        observe_dispatch_health(
                            DispatchHealthObservation {
                                send_duration_us: sender_duration_us,
                                post_send_duration_us: iteration_ready_us
                                    .saturating_sub(result_completed_us),
                                path: frozen_budget.path,
                                send_warn_us: send_warn_threshold_us,
                                bookkeeping_warn_us: frozen_budget.bookkeeping_warn_us,
                                elapsed_us: completed_effective,
                            },
                            health.options.window_policy(),
                            &mut health.send_pure_window,
                            &mut health.bookkeeping_window,
                            local_metrics,
                        );
                        if result_chord_integrity_lost {
                            runtime.verified_target = None;
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error = Some(format!(
                                "SendInput split authored chord at action {}",
                                batch_source_action_index
                            ));
                            break;
                        }
                        if retry_late_abort {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error = Some(format!(
                                "strict timing rejected zero-progress retry at action {}: completion was {}us late",
                                batch_source_action_index, completion_error_us
                            ));
                            break;
                        }
                        if strict_completion_late {
                            runtime.force_full_cleanup = true;
                            let timing_label =
                                if matches!(dispatch_path, DispatchPath::UpOnly { .. }) {
                                    "note-off"
                                } else {
                                    "note-on"
                                };
                            runtime.terminal_error = Some(format!(
                                "strict timing completion SLO exceeded for {timing_label} at action {}: completion was {}us late",
                                batch_source_action_index, completion_error_us
                            ));
                            break;
                        }
                        if saturation_abort {
                            runtime.force_full_cleanup = true;
                            let timing_label =
                                if matches!(dispatch_path, DispatchPath::UpOnly { .. }) {
                                    "note-off"
                                } else {
                                    "note-on"
                                };
                            runtime.terminal_error = Some(format!(
                                "strict timing SLO exceeded: {timing_label} lead saturated with positive residual for {} consecutive dispatches",
                                STRICT_SATURATION_ABORT_STREAK
                            ));
                            break;
                        }
                    }
                } else {
                    let (_, suppressed) = match coordinator.commit_up_request(prepared_batch) {
                        Ok(value) => value,
                        Err(error) => {
                            runtime.force_full_cleanup = true;
                            runtime.terminal_error =
                                Some(format!("coordinator release request failure: {error}"));
                            break;
                        }
                    };
                    if !suppressed.is_empty()
                        && let Err(error) = telemetry.try_push(|| {
                            RtTraceRecord::dispatched(
                                TraceContext {
                                    event_index: batch_source_action_index,
                                    kind: TRACE_KIND_UP,
                                    outcome: trace_outcome_code("suppressed_stale_up"),
                                    polyphony: suppressed.len(),
                                    flags: TRACE_FLAG_ANOMALY,
                                    win32_error: 0,
                                },
                                TraceTiming {
                                    authored_ticks: authored_batch_scheduled_ticks,
                                    effective_deadline_ticks: batch_scheduled_ticks,
                                    wake_ticks: effective_now_ticks,
                                    send_started_ticks: None,
                                    send_completed_ticks: None,
                                    bookkeeping_duration_us: 0,
                                    completion_error_ticks: 0,
                                    authored_completion_error_ticks: 0,
                                    applied_lead_ticks: lead_up_ticks,
                                },
                                TraceDelivery {
                                    requested: 0,
                                    sent: 0,
                                    skipped: 0,
                                    send_attempts: 0,
                                },
                            )
                        })
                    {
                        runtime.force_full_cleanup = true;
                        runtime.terminal_error =
                            Some(format!("native telemetry record overflow: {error}"));
                        break;
                    }
                    publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
                    try_publish_metrics(
                        local_metrics,
                        metrics,
                        qpc_us_or_terminal!(),
                        !suppressed.is_empty(),
                    );
                }
                continue;
            }

            let deadline_ticks = dispatch_plan.deadline_ticks;

            match wait_for_next_boundary(WaitBoundaryInput {
                deadline: WaitDeadline {
                    deadline_ticks,
                    qpc_clock,
                    clock_state,
                    allow_pre_epoch_startup_dispatch: runtime.allow_pre_epoch_startup_dispatch,
                    last_send_qpc_ticks: runtime.last_send_qpc_ticks,
                },
                timing: WaitTiming {
                    core_warmup_ticks: timing.core_warmup_ticks,
                    cold_threshold_ticks: timing.cold_threshold_ticks,
                    effective_spin_threshold_ticks: timing.effective_spin_threshold_ticks,
                    lease_timeout_ticks: timing.lease_timeout_ticks,
                    supervisor_heartbeat_ticks,
                },
                signals: WaitSignals {
                    waiter,
                    interrupt,
                    strict_timing: config.timing.strict_timing,
                    wait_warn_us: health.options.wait_warn_us,
                    wait_policy: health.options.window_policy(),
                },
                mutable: WaitMutable {
                    local_metrics,
                    pending_pre_send_spin_us: &mut runtime.pending_pre_send_spin_us,
                    force_full_cleanup: &mut runtime.force_full_cleanup,
                    terminal_error: &mut runtime.terminal_error,
                    wait_window: &mut health.wait_window,
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
