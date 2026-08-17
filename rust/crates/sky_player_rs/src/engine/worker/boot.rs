#[cfg(any(test, feature = "test-support"))]
use super::super::create_mock_backend;
use super::super::{
    BackendConfig, CoordinatorError, DurationTicks, HARD_LATE_ABORT_THRESHOLD_US, PAUSED_POLL_US,
    PlaybackClockState, QpcClock, QpcError, QpcTicks, RuntimeDispatchCoordinator,
    STARTUP_WAKE_GUARD_US, STRICT_RETRY_LATE_THRESHOLD_US, SharedMetrics, TelemetryCollector,
    TrackedKeyState, WaitOptions, current_process_cpu_time_us, current_thread_cpu_time_us,
    qpc_frequency_checked,
};
use super::{
    DispatchHealthOptions, HealthWindow, StartupResources, Worker, WorkerHealthState,
    WorkerResources, WorkerTimingState, describe_release_outcome, initialize_startup,
    release_state_verified, wait_failure_message,
};
use std::sync::atomic::Ordering;

fn validate_production_wait_backend(
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

/// Assembles the worker's admission state: backend, coordinator,
/// timing frame, health window, startup anchor, and resource bundle.
///
/// Returns a non-zero code only for hard admission errors (the shared
/// `last_error` is already populated in that case).  Soft terminal conditions
/// (QPC admission, injected wait fault) are recorded on the core runtime and
/// surfaced when the dispatch loop first inspects `terminal_error`.
#[allow(clippy::too_many_arguments)]
pub(super) fn initialize(worker: &mut Worker<'_>, wait_fault: bool) -> u8 {
    let shared = worker.shared;
    let metrics = &shared.publication.metrics;

    let schedule = match worker.take_schedule() {
        Ok(schedule) => schedule,
        Err(error) => {
            *metrics.last_error.lock() = Some(error.to_string());
            return 1;
        }
    };

    let core = &mut worker.core;

    let qpc_clock = match QpcClock::initialize() {
        Ok(clock) => clock,
        Err(error) => {
            *metrics.last_error.lock() = Some(format!("QPC admission failed: {error:?}"));
            return 1;
        }
    };
    let mut backend = match &worker.config.backend {
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
    let target_hwnd = &shared.target.target_hwnd;
    let priority_acquired = &shared.publication.priority_acquired;
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
        scheduling,
        waiter,
        power_throttling_disabled,
    } = initialize_startup(
        worker.config.priority.mode,
        matches!(&worker.config.backend, &BackendConfig::Production),
        worker.config.wait.enable_waitable_timer,
        worker.config.wait.enable_event_wait,
        priority_acquired,
        metrics,
    );
    let config = &worker.config;
    let backend_is_production = matches!(&config.backend, &BackendConfig::Production);
    if let Err(error) = validate_production_wait_backend(
        backend_is_production,
        &config.wait,
        waiter.initial_failure(),
    ) {
        return admission_failure(&mut backend, metrics, error);
    }
    core.metrics.power_throttling_disabled = power_throttling_disabled;
    // Python materializes the frame-rate floor before crossing the FFI
    // boundary. The worker consumes that effective minimum hold verbatim and
    // converts it once into the captured QPC tick domain.
    let effective_min_hold_us = config.timing.min_hold_us;
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
    let coordinator = match RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        effective_min_hold_us,
        min_hold_ticks,
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
    core.metrics.total_us = match qpc_clock.duration_to_us(DurationTicks::from_raw(
        match coordinator.effective_total_ticks() {
            Ok(ticks) => ticks.as_u64(),
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("total timeline conversion failed: {error}"),
                );
            }
        },
    )) {
        Ok(total_us) => total_us,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("total timeline conversion failed: {error:?}"),
            );
        }
    };
    let telemetry = std::sync::Arc::new(parking_lot::Mutex::new(TelemetryCollector::new(
        config.telemetry.mode,
        config.telemetry.capacity,
    )));
    core.errors.abort_counts.reserve(6);
    // Production dispatch uses one fixed QPC spin handoff.  Wake probing and
    // adaptive lead control are diagnostic-only and cannot alter this path.
    let effective_spin_threshold_us = super::super::config::DEFAULT_SPIN_THRESHOLD_US;
    let interrupt = &shared.commands.interrupt;
    let _ = interrupt.try_take();
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
    let health_options = DispatchHealthOptions {
        wait_warn_us: config.timing.input_path_warn_us,
        ..DispatchHealthOptions::default()
    };
    core.health = Some(WorkerHealthState {
        options: health_options,
        sendinput_window: HealthWindow::default(),
        core_post_send_window: HealthWindow::default(),
        observer_window: HealthWindow::default(),
        wait_window: HealthWindow::default(),
    });
    core.metrics.sendinput_warn_threshold_us = health_options.sendinput_warn_floor_us;
    core.metrics.core_post_send_warn_threshold_us = health_options.core_post_send_warn_us;
    core.metrics.observer_warn_threshold_us = health_options.observer_warn_us;
    core.metrics.wait_warn_threshold_us = health_options.wait_warn_us;
    let startup_guard_ticks: Result<DurationTicks, String> = (|| {
        let wake_guard = qpc_clock
            .duration_from_us(STARTUP_WAKE_GUARD_US)
            .map_err(|error| format!("{error:?}"))?;
        let with_spin = wake_guard
            .checked_add(effective_spin_threshold_ticks)
            .map_err(|error| error.to_string())?;
        Ok(with_spin)
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
    let startup_anchor_ticks = match initial_now_ticks.checked_add_duration(startup_guard_ticks) {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("startup anchor arithmetic failed: {error}"),
            );
        }
    };
    let clock_state =
        match PlaybackClockState::new(startup_anchor_ticks, DurationTicks::from_raw(0)) {
            Ok(clock) => clock,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("playback clock initialization failed: {error}"),
                );
            }
        };
    let start_wall_time_us = initial_now_us;
    let start_thread_cpu_us = current_thread_cpu_time_us();
    let start_process_cpu_us = current_process_cpu_time_us();
    core.timing = Some(WorkerTimingState {
        strict_timing: config.timing.strict_timing,
        hard_late_abort_threshold_ticks,
        retry_late_threshold_ticks,
        strict_down_completion_late_ticks,
        strict_up_completion_late_ticks,
        focus_restore_grace_ticks,
        paused_poll_ticks,
        lease_timeout_ticks,
        effective_spin_threshold_ticks,
        start_wall_time_us,
        start_thread_cpu_us,
        start_process_cpu_us,
        last_cpu_metrics_sample_us: start_wall_time_us,
    });
    let qpc_admission_error = qpc_frequency_checked()
        .err()
        .map(|error| format!("QPC frequency unavailable: {error:?}"))
        .or_else(|| {
            qpc_clock
                .now()
                .err()
                .map(|error| format!("QPC counter unavailable: {error:?}"))
        });
    if let Some(error) = qpc_admission_error {
        core.runtime.force_full_cleanup = true;
        core.runtime.terminal_error = Some(error);
    }
    if wait_fault {
        core.runtime.force_full_cleanup = true;
        core.runtime.terminal_error = Some("wait failure injected".to_string());
    }

    let observer_timing = core.timing.expect("worker timing initialized");
    core.observer.runtime = Some(super::ObserverRuntime::start(
        core.observer.pending.clone(),
        qpc_clock,
        std::sync::Arc::clone(&shared.publication.metrics),
        std::sync::Arc::clone(&telemetry),
        observer_timing,
        health_options,
    ));
    core.resources = Some(WorkerResources {
        clock: qpc_clock,
        waiter,
        backend,
        coordinator,
        playback: clock_state,
        telemetry,
        scheduling,
    });
    shared.publication.progress_clock.publish(
        &core
            .resources
            .as_ref()
            .expect("worker resources published")
            .playback,
    );

    if core.runtime.terminal_error.is_none() {
        let startup_ready_result = qpc_clock.now().and_then(|ready_ticks| {
            let requested_raw = shared
                .publication
                .startup_requested_ticks
                .load(Ordering::Acquire);
            if requested_raw == 0 {
                return Err(QpcError::CounterUnavailable);
            }
            let requested_ticks = QpcTicks::from_raw(requested_raw);
            let elapsed_ticks = ready_ticks
                .checked_duration_since(requested_ticks)
                .map_err(|_| QpcError::CounterUnavailable)?;
            let elapsed_us = qpc_clock
                .duration_to_us(elapsed_ticks)
                .map_err(|_| QpcError::CounterUnavailable)?;
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
    0
}

#[cfg(test)]
mod tests {
    use super::validate_production_wait_backend;
    use crate::engine::WaitOptions;
    use sky_dispatch_win32::wait::WaitFailure;

    fn options(enable_waitable_timer: bool, enable_event_wait: bool) -> WaitOptions {
        WaitOptions {
            enable_waitable_timer,
            enable_event_wait,
            supervisor_lease_timeout_us: 0,
        }
    }

    #[test]
    fn production_wait_backend_requires_both_precision_primitives() {
        assert!(validate_production_wait_backend(true, &options(true, true), None).is_ok());
        assert!(validate_production_wait_backend(true, &options(false, true), None).is_err());
        assert!(validate_production_wait_backend(true, &options(true, false), None).is_err());
    }

    #[test]
    fn production_wait_backend_rejects_startup_failure() {
        let result = validate_production_wait_backend(
            true,
            &options(true, true),
            Some(WaitFailure::TimerCreate { win32_error: 5 }),
        );

        assert!(result.is_err());
        assert!(
            result
                .expect_err("initial wait failure must reject production")
                .contains("high-resolution waitable timer creation failed")
        );
    }

    #[test]
    fn non_production_wait_backend_can_keep_explicit_fallback_support() {
        assert!(
            validate_production_wait_backend(
                false,
                &options(false, false),
                Some(WaitFailure::TimerCreate { win32_error: 5 }),
            )
            .is_ok()
        );
    }
}
