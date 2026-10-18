#[cfg(any(test, feature = "test-support"))]
use super::super::create_mock_backend;
mod startup_admission;
use startup_admission::{
    finalize_admission_failure, finalize_startup_abort, record_startup_abort_without_backend,
    record_startup_failure, startup_abort_trigger, startup_checkpoint, startup_wake_probe,
    validate_production_wait_backend,
};
mod timing_admission;
use super::super::config::StartupCheckpoint;
use super::super::shared::StartupAbortReason;
use super::super::{
    BackendConfig, CoordinatorError, DurationTicks, PlaybackClockState, QpcClock, QpcTicks,
    RuntimeDispatchCoordinator, SharedMetrics, TelemetryCollector, TrackedKeyState,
    current_process_cpu_time_us, current_thread_cpu_time_us, qpc_frequency_checked,
};
use super::admission::focus_matches_hwnd;
use super::{
    DispatchHealthOptions, HealthWindow, PreparedDispatchStream, StartupResources, Worker,
    WorkerHealthState, WorkerResources, WorkerSchedulingGuards, ensure_preflight_for_target,
    initialize_startup, load_target_stamp,
};
use crate::engine::config::{
    CALIBRATION_MAX_STARTUP_BUDGET_US, CALIBRATION_SAMPLES, SpinThresholdSource,
    StartupCalibrationPlacement, StartupCalibrationResult,
};
use std::sync::atomic::Ordering;
use timing_admission::{ConvertedTiming, convert as convert_timing};

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
    if !shared.commands.system_power.begin_preparing() {
        if let Some(trigger) = startup_abort_trigger(shared, false) {
            return record_startup_abort_without_backend(
                shared,
                trigger,
                format!("startup cancelled before preparation: {}", trigger.as_str()),
            );
        }
        return record_startup_failure(shared, "worker could not claim startup preparation".into());
    }
    if let Some(trigger) = startup_abort_trigger(shared, false) {
        return record_startup_abort_without_backend(
            shared,
            trigger,
            format!("startup cancelled before preparation: {}", trigger.as_str()),
        );
    }

    let schedule = match worker.take_schedule() {
        Ok(schedule) => schedule,
        Err(error) => {
            return record_startup_failure(shared, error.to_string());
        }
    };

    let core = &mut worker.core;
    let config = &worker.config;
    #[cfg(any(test, feature = "test-support"))]
    let startup_hook = core.runtime.startup_ordering_hook.as_deref();
    let backend_is_production = matches!(&config.backend, &BackendConfig::Production);
    let production_wait = config.wait.production_wait_policy(backend_is_production);
    let requested_wait_policy = config
        .wait
        .requested_wait_policy_label(backend_is_production);
    let effective_wait_policy = if production_wait {
        "production_calibrated"
    } else {
        "legacy_test_wide_spin"
    };
    #[cfg(any(test, feature = "test-support"))]
    let calibration_placement = config.startup_calibration_placement;
    #[cfg(not(any(test, feature = "test-support")))]
    let calibration_placement = StartupCalibrationPlacement::BeforeSchedulingGuards;

    let qpc_clock = match QpcClock::initialize() {
        Ok(clock) => clock,
        Err(error) => {
            if let Some(trigger) = startup_abort_trigger(shared, false) {
                return record_startup_abort_without_backend(
                    shared,
                    trigger,
                    format!(
                        "startup cancelled during QPC admission: {}",
                        trigger.as_str()
                    ),
                );
            }
            return record_startup_failure(shared, format!("QPC admission failed: {error:?}"));
        }
    };
    let instrument_key_profile = worker
        .instrument_key_profile
        .take()
        .expect("worker instrument profile is admitted before boot");
    let mut backend = match &worker.config.backend {
        #[cfg(any(test, feature = "test-support"))]
        BackendConfig::Mock {
            latency_base_us,
            latency_per_key_us,
            fault_script,
        } => create_mock_backend(
            qpc_clock,
            instrument_key_profile,
            *latency_base_us,
            *latency_per_key_us,
            fault_script.clone(),
        ),
        BackendConfig::Production => {
            TrackedKeyState::with_qpc_clock_and_profile(qpc_clock, instrument_key_profile)
        }
    };
    #[cfg(any(test, feature = "test-support"))]
    if let Some(mask) = worker.config.prepared_packet_ambiguity_mask {
        backend.set_prepared_packet_ambiguity_mask(mask);
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(mask) = worker.config.preflight_user_held_mask {
        backend.set_force_preflight_user_held_mask(mask);
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(query) = worker.config.modifier_key_state_query_for_test.as_ref() {
        let query = std::sync::Arc::clone(query);
        backend.set_modifier_key_state_query_for_test(move |virtual_key| query(virtual_key));
    }
    let target = &shared.target;
    let priority_acquired = &shared.publication.priority_acquired;
    let mut schedule_has_down = false;
    for (index, packet) in schedule.packets.iter().enumerate() {
        if index.is_multiple_of(64)
            && let Some(trigger) = startup_abort_trigger(shared, false)
        {
            return finalize_startup_abort(
                &mut backend,
                metrics,
                target.hwnd_for_safety_release(),
                trigger,
                format!(
                    "startup cancelled while classifying schedule: {}",
                    trigger.as_str()
                ),
            );
        }
        schedule_has_down |= packet.down_mask != 0;
    }
    let admission_failure =
        |backend: &mut TrackedKeyState, metrics: &SharedMetrics, primary_error: String| {
            if let Some(trigger) =
                startup_abort_trigger(shared, config.focus.require_focus && schedule_has_down)
            {
                finalize_startup_abort(
                    backend,
                    metrics,
                    target.hwnd_for_safety_release(),
                    trigger,
                    format!("startup cancelled: {}", trigger.as_str()),
                )
            } else {
                shared
                    .commands
                    .system_power
                    .fail_startup(StartupAbortReason::WorkerFailure);
                finalize_admission_failure(
                    backend,
                    metrics,
                    target.hwnd_for_safety_release(),
                    primary_error,
                )
            }
        };
    let StartupResources { waiter } = initialize_startup(
        worker.config.priority.mode,
        production_wait,
        config.wait.enable_waitable_timer,
        config.wait.enable_event_wait,
        requested_wait_policy,
        effective_wait_policy,
        priority_acquired,
        metrics,
    );
    if let Err(error) =
        validate_production_wait_backend(production_wait, &config.wait, waiter.initial_failure())
    {
        return admission_failure(&mut backend, metrics, error);
    }
    if let Err(error) = startup_checkpoint(
        shared,
        config.focus.require_focus && schedule_has_down,
        StartupCheckpoint::StreamBuild,
        #[cfg(any(test, feature = "test-support"))]
        startup_hook,
    ) {
        return admission_failure(&mut backend, metrics, error);
    }
    if config.focus.require_focus && schedule_has_down {
        core.runtime.preparation_probe.record_preflight();
        let Some(target_stamp) = load_target_stamp(target) else {
            return admission_failure(
                &mut backend,
                metrics,
                "target publication changed during preroll admission".to_string(),
            );
        };
        if !focus_matches_hwnd(true, &shared.commands.focus_active, target_stamp.hwnd) {
            return admission_failure(
                &mut backend,
                metrics,
                "focus_lost_during_preroll".to_string(),
            );
        }
        if let Err(error) =
            ensure_preflight_for_target(&backend, target_stamp, &mut core.runtime.verified_target)
        {
            return admission_failure(
                &mut backend,
                metrics,
                format!(
                    "instrument key preflight failed during preroll; release the 15 instrument keys before playback: {error}"
                ),
            );
        }
    }
    let ConvertedTiming {
        effective_min_hold_us,
        min_hold_ticks,
        frame_ticks,
        mut worker_timing,
    } = match convert_timing(config, qpc_clock) {
        Ok(timing) => timing,
        Err(error) => return admission_failure(&mut backend, metrics, error),
    };
    let lease_timeout_ticks = worker_timing.lease_timeout_ticks;
    core.runtime
        .production_forensics
        .set_frame_policies(min_hold_ticks, frame_ticks);
    core.runtime.physical_timing_guard = Some(
        super::physical_timing_guard::PhysicalTimingGuard::new(min_hold_ticks, frame_ticks),
    );
    let mut coordinator = match RuntimeDispatchCoordinator::try_new_ticks_with_checkpoint(
        schedule,
        effective_min_hold_us,
        min_hold_ticks,
        |us| {
            qpc_clock
                .timeline_from_us(us)
                .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
        },
        || {
            startup_checkpoint(
                shared,
                config.focus.require_focus && schedule_has_down,
                StartupCheckpoint::StreamBuild,
                #[cfg(any(test, feature = "test-support"))]
                startup_hook,
            )
            .map_err(|_| CoordinatorError::PreparationCancelled)?;
            Ok(())
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
    let prepared_stream = if !config.profile.strict_timing() && !config.timing.strict_timing {
        match PreparedDispatchStream::build_with_checkpoints(
            coordinator,
            qpc_clock,
            &core.runtime.preparation_probe,
            backend.instrument_key_profile(),
            || {
                startup_checkpoint(
                    shared,
                    config.focus.require_focus && schedule_has_down,
                    StartupCheckpoint::StreamMaterialization,
                    #[cfg(any(test, feature = "test-support"))]
                    startup_hook,
                )
            },
            || {
                startup_checkpoint(
                    shared,
                    config.focus.require_focus && schedule_has_down,
                    StartupCheckpoint::CoordinatorReset,
                    #[cfg(any(test, feature = "test-support"))]
                    startup_hook,
                )
            },
        ) {
            Ok((stream, restored_coordinator)) => {
                coordinator = restored_coordinator;
                Some(stream)
            }
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("normal prepared stream admission failed: {error}"),
                );
            }
        }
    } else {
        None
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
    let telemetry =
        match TelemetryCollector::try_new(config.telemetry.mode, config.telemetry.capacity) {
            Ok(telemetry) => std::sync::Arc::new(parking_lot::Mutex::new(telemetry)),
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("telemetry admission failed: {error}"),
                );
            }
        };
    if let Err(error) = core.errors.abort_counts.try_reserve(6) {
        return admission_failure(
            &mut backend,
            metrics,
            format!("worker terminal accounting allocation failed: {error}"),
        );
    }

    let interrupt = &shared.commands.interrupt;
    let _ = interrupt.try_take();
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
    if let Err(error) = startup_checkpoint(
        shared,
        config.focus.require_focus && schedule_has_down,
        StartupCheckpoint::StreamBuild,
        #[cfg(any(test, feature = "test-support"))]
        startup_hook,
    ) {
        return admission_failure(&mut backend, metrics, error);
    }
    let mut calibration = StartupCalibrationMeasurement::not_requested();
    if production_wait
        && calibration_placement == StartupCalibrationPlacement::BeforeSchedulingGuards
    {
        calibration = measure_startup_calibration(
            shared,
            config.focus.require_focus && schedule_has_down,
            &waiter,
            qpc_clock,
            interrupt,
            initial_now_ticks,
            #[cfg(any(test, feature = "test-support"))]
            startup_hook,
        );
    }
    if let Err(error) = startup_checkpoint(
        shared,
        config.focus.require_focus && schedule_has_down,
        StartupCheckpoint::StreamBuild,
        #[cfg(any(test, feature = "test-support"))]
        startup_hook,
    ) {
        return admission_failure(&mut backend, metrics, error);
    }
    let (mut effective_spin_threshold_us, mut spin_threshold_source) =
        select_startup_spin_threshold(config, calibration.stats);
    if production_wait
        && calibration_placement == StartupCalibrationPlacement::BeforeSchedulingGuards
    {
        core.metrics.startup_calibration_executed = calibration.stats.is_some();
        core.metrics.startup_calibration_sample_count = calibration
            .stats
            .as_ref()
            .map(|_| CALIBRATION_SAMPLES as u64)
            .unwrap_or(0);
        core.metrics.startup_calibration_duration_us = calibration.duration_us;
        core.metrics.startup_calibration_result = calibration.result as u8;
        if let Some(stats) = calibration.stats {
            core.metrics.wake_error_p50_us = stats.p50_us;
            core.metrics.wake_error_p95_us = stats.p95_us;
            core.metrics.wake_error_p99_us = stats.p99_us;
            core.metrics.wake_error_max_us = stats.max_us;
            core.metrics.startup_wake_error_robust_us = stats.robust_us;
        }
    }
    core.metrics.spin_threshold_source = spin_threshold_source as u8;
    core.metrics.effective_spin_threshold_us = effective_spin_threshold_us;
    let mut effective_spin_threshold_ticks =
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
    worker_timing.effective_spin_threshold_ticks = effective_spin_threshold_ticks;

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
        return admission_failure(&mut backend, metrics, error);
    }
    if wait_fault {
        return admission_failure(&mut backend, metrics, "wait failure injected".to_string());
    }

    let start_thread_cpu_us = current_thread_cpu_time_us();
    let start_process_cpu_us = current_process_cpu_time_us();
    worker_timing.start_wall_time_us = initial_now_us;
    worker_timing.start_thread_cpu_us = start_thread_cpu_us;
    worker_timing.start_process_cpu_us = start_process_cpu_us;
    worker_timing.last_cpu_metrics_sample_us = initial_now_us;
    core.timing = Some(worker_timing);
    if config.observer_enabled() {
        #[cfg(any(test, feature = "test-support"))]
        if startup_hook.is_some_and(|hook| hook.take_observer_spawn_failure()) {
            return admission_failure(
                &mut backend,
                metrics,
                "injected observer thread creation failure".to_string(),
            );
        }
        let pending = super::dispatch::PendingObservationQueue::default();
        core.observer.pending = Some(pending.clone());
        let observer_timing = core.timing.expect("worker timing initialized");
        core.observer.runtime = match super::ObserverRuntime::start(
            pending,
            qpc_clock,
            std::sync::Arc::clone(&shared.publication.metrics),
            std::sync::Arc::clone(&telemetry),
            observer_timing,
            health_options,
        ) {
            Ok(observer) => Some(observer),
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("observer admission failed: {error}"),
                );
            }
        };
    }

    // Scheduling changes happen only after all large schedule and telemetry
    // allocations are complete. The guards are acquired and later dropped on
    // this worker thread.
    let scheduling = WorkerSchedulingGuards::acquire(config.priority.mode);
    #[cfg(any(test, feature = "test-support"))]
    let mut scheduling = scheduling;
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = core.runtime.startup_ordering_hook.as_ref() {
        hook.record_scheduling_guard_acquired();
        scheduling.startup_ordering_hook = Some(std::sync::Arc::clone(hook));
    }
    core.metrics.power_throttling_disabled = scheduling.is_power_active();
    *priority_acquired.lock() = scheduling.priority_label().to_string();
    *shared.publication.startup_priority_mode_acquired.lock() =
        scheduling.priority_label().to_string();
    shared
        .publication
        .startup_power_guard_acquired
        .store(scheduling.is_power_active(), Ordering::Release);

    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = startup_hook {
        let delay_us = hook.boot_delay_us.load(Ordering::Acquire);
        if delay_us != 0 {
            std::thread::sleep(std::time::Duration::from_micros(delay_us));
        }
    }
    if production_wait
        && calibration_placement == StartupCalibrationPlacement::AfterSchedulingGuards
    {
        calibration = measure_startup_calibration(
            shared,
            config.focus.require_focus && schedule_has_down,
            &waiter,
            qpc_clock,
            interrupt,
            initial_now_ticks,
            #[cfg(any(test, feature = "test-support"))]
            startup_hook,
        );
    }
    if let Err(error) = startup_checkpoint(
        shared,
        config.focus.require_focus && schedule_has_down,
        StartupCheckpoint::StreamBuild,
        #[cfg(any(test, feature = "test-support"))]
        startup_hook,
    ) {
        return admission_failure(&mut backend, metrics, error);
    }
    if production_wait
        && calibration_placement == StartupCalibrationPlacement::AfterSchedulingGuards
    {
        let calibration_stats = calibration.stats;
        (effective_spin_threshold_us, spin_threshold_source) =
            select_startup_spin_threshold(config, calibration_stats);
        core.metrics.startup_calibration_executed = calibration_stats.is_some();
        core.metrics.startup_calibration_sample_count = calibration_stats
            .as_ref()
            .map(|_| CALIBRATION_SAMPLES as u64)
            .unwrap_or(0);
        core.metrics.startup_calibration_duration_us = calibration.duration_us;
        core.metrics.startup_calibration_result = calibration.result as u8;
        core.metrics.spin_threshold_source = spin_threshold_source as u8;
        core.metrics.effective_spin_threshold_us = effective_spin_threshold_us;
        if let Some(stats) = calibration_stats {
            core.metrics.wake_error_p50_us = stats.p50_us;
            core.metrics.wake_error_p95_us = stats.p95_us;
            core.metrics.wake_error_p99_us = stats.p99_us;
            core.metrics.wake_error_max_us = stats.max_us;
            core.metrics.startup_wake_error_robust_us = stats.robust_us;
        }
        effective_spin_threshold_ticks =
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
        if let Some(timing) = core.timing.as_mut() {
            timing.effective_spin_threshold_ticks = effective_spin_threshold_ticks;
        }
    }
    if let Err(error) = startup_checkpoint(
        shared,
        config.focus.require_focus && schedule_has_down,
        StartupCheckpoint::BeforeReady,
        #[cfg(any(test, feature = "test-support"))]
        startup_hook,
    ) {
        return admission_failure(&mut backend, metrics, error);
    }
    let requested_raw = shared
        .publication
        .startup_requested_ticks
        .load(Ordering::Acquire);
    if requested_raw == 0 {
        return admission_failure(
            &mut backend,
            metrics,
            "startup request QPC was not published".to_string(),
        );
    }
    let ready_ticks = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("QPC startup-ready sample failed: {error:?}"),
            );
        }
    };
    let startup_latency_us = match ready_ticks
        .checked_duration_since(QpcTicks::from_raw(requested_raw))
        .and_then(|elapsed| {
            qpc_clock
                .duration_to_us(elapsed)
                .map_err(|_| sky_dispatch_core::time::TimeArithmeticError::Overflow)
        }) {
        Ok(value) => value,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("startup latency conversion failed: {error}"),
            );
        }
    };
    shared
        .publication
        .startup_ready_ticks
        .store(ready_ticks.as_u64(), Ordering::Relaxed);
    shared
        .publication
        .startup_latency_us
        .store(startup_latency_us, Ordering::Relaxed);
    if !shared.commands.system_power.publish_ready() {
        return admission_failure(
            &mut backend,
            metrics,
            "worker could not publish the Ready startup state".to_string(),
        );
    }
    if let Err(error) = startup_checkpoint(
        shared,
        config.focus.require_focus && schedule_has_down,
        StartupCheckpoint::ReadyToArmed,
        #[cfg(any(test, feature = "test-support"))]
        startup_hook,
    ) {
        return admission_failure(&mut backend, metrics, error);
    }
    if !shared.commands.system_power.begin_arming() {
        return admission_failure(
            &mut backend,
            metrics,
            "worker could not claim the Ready-to-Armed transition".to_string(),
        );
    }
    if let Err(error) = startup_checkpoint(
        shared,
        config.focus.require_focus && schedule_has_down,
        StartupCheckpoint::ReadyToArmed,
        #[cfg(any(test, feature = "test-support"))]
        startup_hook,
    ) {
        return admission_failure(&mut backend, metrics, error);
    }

    let lease_now = match qpc_clock.now() {
        Ok(now) => now,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("QPC final startup admission failed: {error:?}"),
            );
        }
    };
    let lease_proof = shared
        .commands
        .supervisor_expired
        .prove_down_age(lease_now, lease_timeout_ticks);
    if !matches!(
        lease_proof.admission,
        super::super::shared::SupervisorLeaseAdmission::Fresh
    ) {
        shared
            .commands
            .system_power
            .cancel_prearm(StartupAbortReason::SupervisorLeaseExpired);
        shared.commands.supervisor_expired.latch_expired();
        return admission_failure(
            &mut backend,
            metrics,
            "supervisor lease is not fresh at startup arm admission".to_string(),
        );
    }

    if config.focus.require_focus && schedule_has_down {
        let Some(target_stamp) = load_target_stamp(target) else {
            shared
                .commands
                .system_power
                .cancel_prearm(StartupAbortReason::TargetInvalidated);
            return admission_failure(
                &mut backend,
                metrics,
                "target changed before Ready-to-Armed publication".to_string(),
            );
        };
        if !focus_matches_hwnd(true, &shared.commands.focus_active, target_stamp.hwnd) {
            shared
                .commands
                .system_power
                .cancel_prearm(StartupAbortReason::FocusInvalidated);
            return admission_failure(
                &mut backend,
                metrics,
                "focus changed before Ready-to-Armed publication".to_string(),
            );
        }
        if let Err(error) =
            ensure_preflight_for_target(&backend, target_stamp, &mut core.runtime.verified_target)
        {
            shared
                .commands
                .system_power
                .cancel_prearm(StartupAbortReason::OwnerInvalidated);
            return admission_failure(
                &mut backend,
                metrics,
                format!("owner preflight changed before Ready-to-Armed publication: {error}"),
            );
        }
        if !super::target_stamp_still_current(target, target_stamp) {
            shared
                .commands
                .system_power
                .cancel_prearm(StartupAbortReason::TargetInvalidated);
            return admission_failure(
                &mut backend,
                metrics,
                "target generation changed before Ready-to-Armed publication".to_string(),
            );
        }
    }
    if let Some(trigger) =
        startup_abort_trigger(shared, config.focus.require_focus && schedule_has_down)
    {
        return admission_failure(
            &mut backend,
            metrics,
            format!(
                "startup cancelled before epoch publication: {}",
                trigger.as_str()
            ),
        );
    }

    let epoch_base = match qpc_clock.now() {
        Ok(now) => now,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("QPC playback epoch sample failed: {error:?}"),
            );
        }
    };
    #[cfg(any(test, feature = "test-support"))]
    let inject_epoch_conversion_failure =
        startup_hook.is_some_and(|hook| hook.take_startup_epoch_conversion_failure());
    #[cfg(not(any(test, feature = "test-support")))]
    let inject_epoch_conversion_failure = false;
    let pre_roll_ticks = match if inject_epoch_conversion_failure {
        Err("injected QPC pre-roll conversion failure".to_string())
    } else {
        qpc_clock
            .duration_from_us(shared.publication.pre_roll_us.load(Ordering::Acquire))
            .map_err(|error| format!("{error:?}"))
    } {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("pre-roll conversion failed before epoch publication: {error}"),
            );
        }
    };
    let epoch_qpc = match epoch_base.checked_add_duration(pre_roll_ticks) {
        Ok(epoch) => epoch,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("playback epoch arithmetic failed: {error}"),
            );
        }
    };
    let playback = match PlaybackClockState::new(epoch_qpc, DurationTicks::ZERO) {
        Ok(playback) => playback,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("playback clock initialization failed: {error}"),
            );
        }
    };
    worker.epoch_qpc = epoch_qpc;
    let epoch_wall_us = match qpc_clock.duration_to_us(DurationTicks::from_raw(epoch_qpc.as_u64()))
    {
        Ok(value) => value,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("playback epoch wall-time conversion failed: {error:?}"),
            );
        }
    };
    if let Some(timing) = core.timing.as_mut() {
        timing.start_wall_time_us = epoch_wall_us;
        timing.last_cpu_metrics_sample_us = epoch_wall_us;
        timing.start_thread_cpu_us = current_thread_cpu_time_us();
        timing.start_process_cpu_us = current_process_cpu_time_us();
    }

    shared
        .publication
        .epoch_qpc
        .store(epoch_qpc.as_u64(), Ordering::Relaxed);
    shared.publication.progress_clock.publish(&playback);
    if let Err(error) = startup_checkpoint(
        shared,
        config.focus.require_focus && schedule_has_down,
        StartupCheckpoint::EpochPrepared,
        #[cfg(any(test, feature = "test-support"))]
        startup_hook,
    ) {
        return admission_failure(&mut backend, metrics, error);
    }
    if !shared.commands.system_power.publish_armed() {
        return admission_failure(
            &mut backend,
            metrics,
            "worker could not atomically publish the Armed epoch".to_string(),
        );
    }
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = startup_hook {
        hook.visit_startup_checkpoint(StartupCheckpoint::ArmedPublished);
    }

    core.resources = Some(WorkerResources {
        clock: qpc_clock,
        waiter,
        backend,
        coordinator,
        prepared_stream,
        playback,
        telemetry,
        scheduling,
    });
    0
}

struct StartupCalibrationMeasurement {
    stats: Option<sky_dispatch_win32::wait::WakeErrorStats>,
    duration_us: u64,
    result: StartupCalibrationResult,
}

fn select_startup_spin_threshold(
    config: &super::WorkerConfig,
    stats: Option<sky_dispatch_win32::wait::WakeErrorStats>,
) -> (u64, SpinThresholdSource) {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(threshold_us) = config.wait.test_spin_threshold_us {
        let source = if threshold_us == 20_000 {
            SpinThresholdSource::LegacyTestWideSpin
        } else {
            SpinThresholdSource::TestFixedOverride
        };
        return (threshold_us, source);
    }

    #[cfg(not(any(test, feature = "test-support")))]
    let _ = config;

    if stats.is_some() {
        (
            super::timing::select_spin_threshold_us(stats),
            SpinThresholdSource::ProductionStartupCalibration,
        )
    } else {
        (
            super::timing::select_spin_threshold_us(None),
            SpinThresholdSource::ProductionFallback,
        )
    }
}

impl StartupCalibrationMeasurement {
    fn not_requested() -> Self {
        Self {
            stats: None,
            duration_us: 0,
            result: StartupCalibrationResult::NotRequested,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn measure_startup_calibration(
    shared: &super::super::shared::SessionShared,
    require_focus: bool,
    waiter: &sky_dispatch_win32::wait::HybridWaiter,
    qpc_clock: QpcClock,
    interrupt: &sky_dispatch_win32::event::OwnedEvent,
    deadline_origin: QpcTicks,
    #[cfg(any(test, feature = "test-support"))] startup_hook: Option<
        &super::super::config::StartupOrderingHook,
    >,
) -> StartupCalibrationMeasurement {
    let started = qpc_clock.now().ok();
    let stats = startup_wake_probe(
        waiter,
        qpc_clock,
        interrupt,
        deadline_origin,
        #[cfg(any(test, feature = "test-support"))]
        startup_hook,
    );
    let finished = qpc_clock.now().ok();
    let duration_us = started
        .zip(finished)
        .and_then(|(start, end)| end.checked_duration_since(start).ok())
        .and_then(|duration| qpc_clock.duration_to_us(duration).ok())
        .unwrap_or(0);
    let result = if stats.is_some() {
        StartupCalibrationResult::Completed
    } else if startup_abort_trigger(shared, require_focus).is_some() {
        StartupCalibrationResult::Interrupted
    } else {
        let budget_ticks = qpc_clock
            .duration_from_us(CALIBRATION_MAX_STARTUP_BUDGET_US)
            .ok();
        let deadline = budget_ticks
            .and_then(|budget| startup_admission::startup_probe_deadline(deadline_origin, budget));
        if deadline.is_some_and(|deadline| finished.is_some_and(|now| now >= deadline)) {
            StartupCalibrationResult::BudgetExpired
        } else {
            StartupCalibrationResult::Unavailable
        }
    };
    StartupCalibrationMeasurement {
        stats,
        duration_us,
        result,
    }
}

#[cfg(test)]
mod admission_tests;

#[cfg(test)]
mod timing_admission_tests;
