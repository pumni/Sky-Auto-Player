#[cfg(any(test, feature = "test-support"))]
use super::super::create_mock_backend;
mod epoch;
mod startup_admission;
#[cfg(test)]
use startup_admission::finalize_admission_failure;
use startup_admission::{
    finalize_admission_failure_with_origin, finalize_startup_abort_with_origin,
    record_startup_abort_without_backend, record_startup_failure, startup_abort_trigger,
    startup_checkpoint, validate_production_wait_backend,
};
mod startup_calibration;
mod telemetry_admission;
mod timing_admission;
use super::super::config::{EffectiveRuntimePolicy, StartupCheckpoint};
use super::super::shared::StartupAbortReason;
use super::super::{
    BackendConfig, DurationTicks, QpcClock, QpcTicks, SharedMetrics, TrackedKeyState,
    current_process_cpu_time_us, current_thread_cpu_time_us, qpc_frequency_checked,
};
use super::admission::{TargetStamp, focus_matches_hwnd};
use super::{
    DispatchHealthOptions, StartupResources, Worker, WorkerResources, WorkerSchedulingGuards,
    ensure_preflight_for_target, initialize_startup, load_target_stamp,
};
use crate::engine::config::StartupCalibrationPlacement;
use epoch::compute_playback_epoch;
use startup_calibration::{
    StartupCalibrationMeasurement, apply_startup_calibration_result, measure_startup_calibration,
};
use std::sync::atomic::Ordering;
use telemetry_admission::try_new_worker_telemetry;
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
    let effective_policy = EffectiveRuntimePolicy::for_worker(config);
    #[cfg(any(test, feature = "test-support"))]
    let startup_hook = core.runtime.startup_ordering_hook.as_deref();
    let production_wait = effective_policy.production_wait;
    let requested_wait_policy = config
        .wait
        .requested_wait_policy_label(matches!(&config.backend, &BackendConfig::Production));
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
    let Some((release_hwnd, release_generation)) = target.load_stable() else {
        return record_startup_failure(
            shared,
            "session target unavailable while binding worker release provenance".into(),
        );
    };
    let session_release_owner =
        match target.try_release_owner_proof_for_generation(release_hwnd, release_generation) {
            Ok(proof) => proof,
            Err(error) => {
                return record_startup_failure(
                    shared,
                    format!("session release owner binding failed: {error:?}"),
                );
            }
        };
    if let Err(error) = backend.bind_session_release_owner(session_release_owner) {
        return record_startup_failure(
            shared,
            format!("session release owner binding failed: {error}"),
        );
    }
    let release_target_stamp = TargetStamp {
        hwnd: release_hwnd,
        generation: release_generation,
    };
    let require_owner_identity = target.owner_process_identity_required();
    let priority_acquired = &shared.publication.priority_acquired;
    let schedule_has_down = schedule.has_down();
    let admission_failure =
        |backend: &mut TrackedKeyState, metrics: &SharedMetrics, primary_error: String| {
            if let Some(trigger) =
                startup_abort_trigger(shared, config.focus.require_focus && schedule_has_down)
            {
                finalize_startup_abort_with_origin(
                    backend,
                    metrics,
                    super::cleanup::CleanupTarget::for_session(
                        target,
                        Some(release_target_stamp),
                        require_owner_identity,
                    ),
                    trigger,
                    format!("startup cancelled: {}", trigger.as_str()),
                )
            } else {
                shared
                    .commands
                    .system_power
                    .fail_startup(StartupAbortReason::WorkerFailure);
                finalize_admission_failure_with_origin(
                    backend,
                    metrics,
                    super::cleanup::CleanupTarget::for_session(
                        target,
                        Some(release_target_stamp),
                        require_owner_identity,
                    ),
                    primary_error,
                )
            }
        };
    let StartupResources { waiter } = initialize_startup(
        worker.config.priority.mode,
        effective_policy,
        config.wait.enable_waitable_timer,
        config.wait.enable_event_wait,
        requested_wait_policy,
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
        min_hold_ticks,
        mut worker_timing,
        ..
    } = match convert_timing(config, qpc_clock, &effective_policy) {
        Ok(timing) => timing,
        Err(error) => return admission_failure(&mut backend, metrics, error),
    };
    let lease_timeout_ticks = worker_timing.lease_timeout_ticks;
    core.runtime
        .production_forensics
        .set_timing_policies(min_hold_ticks);
    #[cfg(any(test, feature = "test-support"))]
    if let Err(error) = startup_checkpoint(
        shared,
        config.focus.require_focus && schedule_has_down,
        StartupCheckpoint::StreamBuild,
        startup_hook,
    ) {
        return admission_failure(&mut backend, metrics, error);
    }
    let prepared_schedule = schedule;
    if !prepared_schedule.is_compatible(qpc_clock.frequency_hz(), config.timing.min_hold_us) {
        return admission_failure(
            &mut backend,
            metrics,
            "prepared dispatch plan no longer matches the worker QPC domain or hold policy"
                .to_string(),
        );
    }
    if prepared_schedule
        .memory_metrics()
        .overlapping_peak_capacity_bytes
        > sky_dispatch_core::model::MAX_RETAINED_DISPATCH_BYTES
    {
        return admission_failure(
            &mut backend,
            metrics,
            "prepared dispatch plan exceeds the retained capacity budget".to_string(),
        );
    }
    let telemetry = match try_new_worker_telemetry(&effective_policy) {
        Ok(telemetry) => std::sync::Arc::new(parking_lot::Mutex::new(telemetry)),
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("telemetry admission failed: {error}"),
            );
        }
    };
    let scheduler =
        sky_dispatch_core::executor::DispatchExecutor::from_prepared_schedule(prepared_schedule);
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = startup_hook {
        hook.record_prepared_schedule_executor_consumption();
    }
    core.metrics.total_us = match qpc_clock.duration_to_us(DurationTicks::from_raw(
        scheduler.schedule_end_offset_ticks().as_u64(),
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
    let mut effective_spin_threshold_ticks = match apply_startup_calibration_result(
        config,
        &calibration,
        &mut core.metrics,
        qpc_clock,
    ) {
        Ok(ticks) => ticks,
        Err(error) => return admission_failure(&mut backend, metrics, error),
    };
    worker_timing.effective_spin_threshold_ticks = effective_spin_threshold_ticks;

    let health_options = DispatchHealthOptions {
        wait_warn_us: config.timing.input_path_warn_us,
        ..DispatchHealthOptions::default()
    };
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
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = startup_hook {
        hook.record_startup_cpu_baseline(start_thread_cpu_us, start_process_cpu_us);
    }
    worker_timing.start_wall_time_us = initial_now_us;
    worker_timing.start_thread_cpu_us = start_thread_cpu_us;
    worker_timing.start_process_cpu_us = start_process_cpu_us;
    worker_timing.last_cpu_metrics_sample_ticks = initial_now_ticks.as_u64();
    core.timing = Some(worker_timing);
    if effective_policy.observer_enabled {
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
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hook) = startup_hook {
            hook.record_observer_cpu_baseline(
                observer_timing.start_thread_cpu_us,
                observer_timing.start_process_cpu_us,
            );
        }
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
        #[cfg(any(test, feature = "test-support"))]
        if startup_hook.is_some_and(|hook| hook.take_guarded_calibration_origin_failure()) {
            return admission_failure(
                &mut backend,
                metrics,
                "QPC counter unavailable before guarded calibration: InjectedFailure".to_string(),
            );
        }
        let calibration_origin_ticks = match qpc_clock.now() {
            Ok(ticks) => ticks,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("QPC counter unavailable before guarded calibration: {error:?}"),
                );
            }
        };
        calibration = measure_startup_calibration(
            shared,
            config.focus.require_focus && schedule_has_down,
            &waiter,
            qpc_clock,
            interrupt,
            calibration_origin_ticks,
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
        effective_spin_threshold_ticks = match apply_startup_calibration_result(
            config,
            &calibration,
            &mut core.metrics,
            qpc_clock,
        ) {
            Ok(ticks) => ticks,
            Err(error) => return admission_failure(&mut backend, metrics, error),
        };
        if let Some(timing) = core.timing.as_mut() {
            timing.effective_spin_threshold_ticks = effective_spin_threshold_ticks;
            let post_cal_thread_cpu_us = current_thread_cpu_time_us();
            let post_cal_process_cpu_us = current_process_cpu_time_us();
            timing.start_thread_cpu_us = post_cal_thread_cpu_us;
            timing.start_process_cpu_us = post_cal_process_cpu_us;
            if let Ok(now) = qpc_clock.now()
                && let Ok(now_us) = qpc_clock.duration_to_us(DurationTicks::from_raw(now.as_u64()))
            {
                timing.start_wall_time_us = now_us;
                timing.last_cpu_metrics_sample_ticks = now.as_u64();
            }
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

    // Capture playback CPU attribution after every pre-arm gate but before
    // the selected epoch QPC sample. The observer keeps its earlier copied
    // startup pair, which describes its own lifetime.
    let playback_start_thread_cpu_us = current_thread_cpu_time_us();
    let playback_start_process_cpu_us = current_process_cpu_time_us();
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = startup_hook {
        hook.record_playback_cpu_baseline(
            playback_start_thread_cpu_us,
            playback_start_process_cpu_us,
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
    if let Some(hook) = startup_hook {
        hook.record_epoch_qpc_sample_completed();
    }
    let (epoch_qpc, playback, epoch_wall_us) = match compute_playback_epoch(
        qpc_clock,
        epoch_base,
        shared.publication.pre_roll_us.load(Ordering::Acquire),
        #[cfg(any(test, feature = "test-support"))]
        startup_hook.is_some_and(|hook| hook.take_startup_epoch_conversion_failure()),
    ) {
        Ok(state) => state,
        Err(error) => return admission_failure(&mut backend, metrics, error),
    };
    worker.epoch_qpc = epoch_qpc;
    if let Some(timing) = core.timing.as_mut() {
        timing.start_wall_time_us = epoch_wall_us;
        timing.last_cpu_metrics_sample_ticks = epoch_qpc.as_u64();
        timing.start_thread_cpu_us = playback_start_thread_cpu_us;
        timing.start_process_cpu_us = playback_start_process_cpu_us;
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
    // Capture the actual successor session's successful Ready-to-Armed
    // admission boundary on the worker QPC clock. This acceptance-only
    // observation follows the publication CAS; it is not a process-launch or
    // caller-side arm timestamp. A failed QPC read leaves the metric absent.
    #[cfg(feature = "real-input-acceptance")]
    metrics.playback_armed_qpc_ticks.store(
        qpc_clock.now().map_or(0, |ticks| ticks.as_u64()),
        Ordering::Release,
    );
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = startup_hook {
        hook.visit_startup_checkpoint(StartupCheckpoint::ArmedPublished);
    }

    core.resources = Some(WorkerResources {
        clock: qpc_clock,
        waiter,
        backend,
        scheduler,
        playback,
        telemetry,
        scheduling,
    });
    0
}

#[cfg(test)]
mod admission_tests;

#[cfg(test)]
mod timing_admission_tests;
