mod admission;
mod boot;
mod cleanup;
mod control;
pub(crate) mod dispatch;
mod dispatch_loop;
#[cfg(not(any(test, feature = "test-support")))]
mod health;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod health;
mod orchestration;
mod startup;
mod timing;
mod wait;

pub(crate) use admission::{
    FinalControlSignals, FinalDownAdmissionResult, TargetStamp, ensure_preflight_for_target,
    enter_focus_pause, focus_matches, focus_matches_hwnd, load_target_stamp,
    target_stamp_still_current, verify_final_down_admission,
};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use cleanup::FinalizeTestObservation;
#[cfg(test)]
pub(crate) use cleanup::record_termination_error;
use cleanup::{
    FinalizeInput, FinalizePublication, FinalizeResources, FinalizeSignals, FinalizeState,
    FinalizeTiming, finalize_worker,
};
pub(crate) use cleanup::{describe_release_outcome, release_state_verified};
pub(crate) use control::TerminalTrigger;
use control::{
    CommandControl, CommandControlInput, CommandControlRuntime, CommandControlSignals,
    process_command_control,
};
pub(super) use dispatch::DispatchStep;
pub(crate) use dispatch::ObserverRuntime;
pub(crate) use dispatch::hold_forensics::ProductionHoldForensics;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use dispatch_loop::apply_scheduler_resumable_lifecycle_transition_for_test;
#[cfg(any(test, feature = "test-support"))]
pub use dispatch_loop::{StagedCandidate, StagedDispatchOutcome};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use dispatch_loop::{
    StagedControlSignals, StagedDispatchContext, dispatch_staged_candidate, stage_candidate,
};

#[cfg(any(test, feature = "test-support"))]
#[cfg(test)]
pub(crate) use health::HealthWindowPolicy;
#[cfg(test)]
pub(crate) use health::record_input_path_health;
pub(crate) use health::{
    DispatchHealthObservation, DispatchHealthOptions, DispatchPath, HEALTH_WINDOW_CAPACITY,
    HealthWindow, focus_gate_matches, observe_dispatch_health, observe_wait_health,
    publish_backend_counters, publish_backend_metrics, record_lateness,
};
pub(crate) use startup::WorkerSchedulingGuards;
use startup::{StartupResources, initialize_startup};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use timing::calibrated_spin_threshold_us;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use timing::derive_spin_threshold_us;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use timing::signed_ticks_to_us;
#[cfg(test)]
pub(crate) use timing::{
    adjust_spin_threshold, anchored_dispatch_target_ticks, deadline_target_ticks,
};
pub(crate) use timing::{
    signed_delta, signed_timeline_delta_ticks, supervisor_lease_expired, wait_failure_message,
    wake_lateness_ticks,
};
pub use wait::WaitObservation;
pub(crate) use wait::{
    WaitBoundary, WaitBoundaryInput, WaitDeadline, WaitMutable, WaitSignals, wait_for_next_boundary,
};

use super::shared::SessionShared;
use super::*;
use sky_dispatch_core::model::RuntimeSchedule;
#[cfg(any(test, feature = "test-support"))]
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
#[cfg(any(test, feature = "test-support"))]
#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn process_command_control_for_test(
    terminal_trigger: &mut Option<TerminalTrigger>,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    supervisor_expired: &crate::engine::shared::SupervisorLeaseState,
    system_power: &crate::engine::shared::SystemPowerState,
) -> bool {
    matches!(
        process_command_control(CommandControlInput {
            signals: CommandControlSignals {
                quit_requested,
                skip_requested,
                panic_requested,
                supervisor_expired,
                system_power,
            },
            runtime: CommandControlRuntime { terminal_trigger },
        }),
        CommandControl::Exit
    )
}

#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn finalize_worker_for_test(
    resources: WorkerResources,
    target: &crate::engine::shared::SessionTarget,
    skip_requested: &AtomicBool,
    quit_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    supervisor_expired: &crate::engine::shared::SupervisorLeaseState,
    system_power: &crate::engine::shared::SystemPowerState,
    metrics: &SharedMetrics,
    progress_clock: &super::shared::SharedProgressClock,
    worker_panicked: bool,
    terminal_trigger: Option<TerminalTrigger>,
    terminal_error: Option<String>,
    cleanup_observation: &mut cleanup::FinalizeTestObservation,
) -> u8 {
    let telemetry_output = parking_lot::Mutex::new(None);
    let priority_acquired = parking_lot::Mutex::new(String::new());
    let result = catch_unwind(AssertUnwindSafe(|| {
        finalize_worker(FinalizeInput {
            resources: FinalizeResources {
                backend: resources.backend,
                scheduler: resources.scheduler,
                telemetry: resources.telemetry,
                playback: resources.playback,
                qpc_clock: resources.clock,
                scheduling: resources.scheduling,
                observer: None,
            },
            state: FinalizeState {
                worker_result: if worker_panicked {
                    Err(Box::new("test worker panic"))
                } else {
                    Ok(())
                },
                local_metrics: WorkerMetricsLocal::default(),
                abort_counts: HashMap::new(),
                terminal_error,
                secondary_errors: Vec::new(),
                last_published_error: None,
                terminal_trigger,
            },
            signals: FinalizeSignals {
                target,
                skip_requested,
                quit_requested,
                panic_requested,
                supervisor_expired,
                system_power,
            },
            publication: FinalizePublication {
                metrics,
                telemetry_output: &telemetry_output,
                priority_acquired: &priority_acquired,
                progress_clock,
            },
            timing: FinalizeTiming {
                start_wall_time_us: 0,
                start_thread_cpu_us: 0,
                start_process_cpu_us: 0,
            },
            cleanup_observation: Some(cleanup_observation),
        })
    }));
    cleanup_observation.generation_accounting = *metrics.generation_accounting.lock();
    match result {
        Ok(outcome) => outcome,
        Err(payload) => resume_unwind(payload),
    }
}

/// Mutable state owned exclusively by the worker thread.
///
/// This state deliberately lives outside the panic boundary so the worker's
/// backend, scheduler, and telemetry can still be finalized after an
/// injected or unexpected panic.
#[derive(Default)]
pub(crate) struct WorkerRuntime {
    pub(crate) production_forensics: ProductionHoldForensics,
    verified_target: Option<TargetStamp>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) startup_ordering_hook: Option<Arc<StartupOrderingHook>>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) restore_race_hook: Option<super::config::RestoreRaceHook>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) focus_pause_hook: Option<super::config::FocusPauseHook>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) final_gate_post_focus_race_hook: Option<super::config::FinalGateRaceHook>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) final_gate_post_lease_race_hook: Option<super::config::FinalGateRaceHook>,
    #[cfg(test)]
    pub(crate) final_lease_qpc_sample_count_for_test: u64,
    #[cfg(test)]
    pub(crate) final_lease_qpc_sample_override_for_test:
        Option<Result<QpcTicks, sky_dispatch_win32::clock::QpcError>>,
    focus_restore_started_ticks: Option<QpcTicks>,
    last_dispatch_deadline_wake_qpc: Option<QpcTicks>,
    pub(crate) pending_wait_observation: Option<WaitObservation>,
    /// Measured completion boundary of the most recent physical sender
    /// transaction, retained so sequential HOL evidence survives queue drops.
    pub(crate) last_physical_send_completion_qpc: Option<QpcTicks>,
    pub(crate) previous_sendinput_completion_qpc: Option<QpcTicks>,
    pub(crate) previous_sender_completion_known: bool,
    last_dispatch_deadline_target_qpc: Option<QpcTicks>,
    pub(crate) terminal_error: Option<String>,
    pub(crate) terminal_trigger: Option<TerminalTrigger>,
    pub(crate) manual_pause_suspension_pending: bool,
    /// True only after the first successful authored musical commit. A
    /// final foreground mismatch before that point is startup failure, not a
    /// pause/rebase event.
    pub(crate) musical_physical_commit_started: bool,
    pub(crate) allow_pre_epoch_startup_dispatch: bool,
}

impl WorkerRuntime {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn create_test_runtime(verified_target: Option<TargetStamp>) -> Self {
        Self {
            verified_target,
            ..Self::default()
        }
    }
}

#[derive(Default)]
pub(super) struct WorkerErrorState {
    secondary: Vec<String>,
    last_published: Option<String>,
    abort_counts: HashMap<&'static str, u64>,
}

#[derive(Clone, Copy)]
pub(crate) struct WorkerTimingState {
    pub(super) strict_timing: bool,
    #[allow(dead_code)]
    pub(super) timing_margin_ticks: DurationTicks,
    pub(super) strict_down_completion_late_ticks: DurationTicks,
    pub(super) strict_up_completion_late_ticks: DurationTicks,
    pub(super) focus_restore_grace_ticks: DurationTicks,
    pub(crate) lease_timeout_ticks: DurationTicks,
    pub(crate) effective_spin_threshold_ticks: DurationTicks,
    /// First QPC tick whose floored public duration is strictly over the
    /// corresponding pre-call lateness bucket.  These are converted once at
    /// worker admission so the physical send path only compares integers.
    pub(super) pre_call_250us_ticks: DurationTicks,
    pub(super) pre_call_500us_ticks: DurationTicks,
    pub(super) pre_call_750us_ticks: DurationTicks,
    pub(super) pre_call_1000us_ticks: DurationTicks,
    pub(super) pre_call_1500us_ticks: DurationTicks,
    pub(super) pre_call_2000us_ticks: DurationTicks,
    pub(super) pre_call_2ms_ticks: DurationTicks,
    pub(super) pre_call_5ms_ticks: DurationTicks,
    pub(super) pre_call_10ms_ticks: DurationTicks,
    pub(super) start_wall_time_us: u64,
    pub(super) start_thread_cpu_us: u64,
    pub(super) start_process_cpu_us: u64,
    pub(super) last_cpu_metrics_sample_us: u64,
}

#[cfg(any(test, feature = "test-support"))]
impl WorkerTimingState {
    pub(crate) fn create_test_timing() -> Self {
        Self {
            strict_timing: false,
            timing_margin_ticks: DurationTicks::ZERO,
            strict_down_completion_late_ticks: DurationTicks::ZERO,
            strict_up_completion_late_ticks: DurationTicks::ZERO,
            focus_restore_grace_ticks: DurationTicks::ZERO,
            lease_timeout_ticks: DurationTicks::ZERO,
            effective_spin_threshold_ticks: DurationTicks::ZERO,
            // Test harnesses replace these with the captured clock-domain
            // values when they exercise lateness buckets.  MAX keeps a
            // synthetic timing state from classifying every sample as late.
            pre_call_250us_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_500us_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_750us_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_1000us_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_1500us_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_2000us_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_2ms_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_5ms_ticks: DurationTicks::from_raw(u64::MAX),
            pre_call_10ms_ticks: DurationTicks::from_raw(u64::MAX),
            start_wall_time_us: 0,
            start_thread_cpu_us: 0,
            start_process_cpu_us: 0,
            last_cpu_metrics_sample_us: 0,
        }
    }
}

/// Fixed observer guard converted to QPC ticks during worker admission.
pub(crate) struct WorkerHealthState {
    pub(super) options: DispatchHealthOptions,
    pub(super) sendinput_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
    pub(super) core_post_send_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
    pub(super) observer_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
    pub(super) wait_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
}

impl WorkerHealthState {
    pub(crate) fn new(options: DispatchHealthOptions) -> Self {
        Self {
            options,
            sendinput_window: HealthWindow::default(),
            core_post_send_window: HealthWindow::default(),
            observer_window: HealthWindow::default(),
            wait_window: HealthWindow::default(),
        }
    }
}

pub(crate) struct WorkerResources {
    pub(super) clock: QpcClock,
    pub(super) waiter: HybridWaiter,
    pub(super) backend: TrackedKeyState,
    pub(super) scheduler: sky_dispatch_core::executor::DispatchExecutor,
    pub(super) playback: PlaybackClockState,
    pub(super) telemetry: Arc<parking_lot::Mutex<TelemetryCollector>>,
    pub(super) scheduling: WorkerSchedulingGuards,
}

#[derive(Default)]
pub(super) struct WorkerCore {
    pub(super) resources: Option<WorkerResources>,
    pub(super) metrics: WorkerMetricsLocal,
    pub(super) health: Option<WorkerHealthState>,
    pub(super) timing: Option<WorkerTimingState>,
    pub(super) runtime: WorkerRuntime,
    pub(super) errors: WorkerErrorState,
    pub(super) observer: WorkerObserverState,
}

/// Deferred dispatch-observer state owned exclusively by the worker thread.
/// Production keeps both fields empty; strict diagnostics own the fixed queue
/// and consumer thread here, never in shared state.
#[derive(Default)]
pub(super) struct WorkerObserverState {
    pub(super) pending: Option<dispatch::PendingObservationQueue>,
    pub(super) runtime: Option<ObserverRuntime>,
}

pub(super) fn update_deferred_worker_metrics(
    local_metrics: &mut WorkerMetricsLocal,
    timing: &mut WorkerTimingState,
    wall_now_us: u64,
) {
    if cpu_metrics_sample_due(
        wall_now_us,
        timing.last_cpu_metrics_sample_us,
        CPU_METRICS_SAMPLE_INTERVAL_US,
    ) {
        // The observer runs on a different thread.  Its sampling point can
        // update the process-wide value, but it must never publish that
        // thread's CPU time as the dispatch-worker CPU metric.  The worker
        // value is captured once during finalization on the owning thread.
        local_metrics.process_cpu_time_us =
            current_process_cpu_time_us().saturating_sub(timing.start_process_cpu_us);
        timing.last_cpu_metrics_sample_us = wall_now_us;
    }
    local_metrics.playback_wall_time_us = wall_now_us.saturating_sub(timing.start_wall_time_us);
    if local_metrics.idle_wake_count > 0 && local_metrics.playback_wall_time_us > 0 {
        local_metrics.spin_duty_cycle_ppm = (local_metrics.spin_time_us as u128 * 1_000_000
            / local_metrics.playback_wall_time_us as u128)
            as u64;
    }
}

pub(super) struct Worker<'a> {
    schedule: Option<RuntimeSchedule>,
    config: WorkerConfig,
    shared: &'a SessionShared,
    epoch_qpc: QpcTicks,
    instrument_key_profile: Option<sky_dispatch_win32::input::MaterializedInstrumentKeyProfile>,
    core: WorkerCore,
}

impl<'a> Worker<'a> {
    pub(super) fn new(
        admitted: super::config::AdmittedNativeSessionOptions,
        shared: &'a SessionShared,
    ) -> Self {
        let super::config::AdmittedNativeSessionOptions {
            options:
                NativeSessionOptions {
                    schedule,
                    backend,
                    profile,
                    timing,
                    focus,
                    wait,
                    telemetry,
                    priority,
                    #[cfg(any(test, feature = "test-support"))]
                    startup_calibration_placement,
                    #[cfg(any(test, feature = "test-support"))]
                    timing_observer_for_qualification,
                    #[cfg(any(test, feature = "test-support"))]
                    startup_ordering_hook,
                    #[cfg(any(test, feature = "test-support"))]
                    restore_race_hook,
                    #[cfg(any(test, feature = "test-support"))]
                    focus_pause_hook,
                    #[cfg(any(test, feature = "test-support"))]
                        timer_lifecycle_context: _,
                    #[cfg(any(test, feature = "test-support"))]
                    prepared_packet_ambiguity_mask,
                    #[cfg(any(test, feature = "test-support"))]
                    preflight_user_held_mask,
                    #[cfg(any(test, feature = "test-support"))]
                    modifier_key_state_query_for_test,
                    instrument_key_profile: _,
                },
            instrument_key_profile,
        } = admitted;
        #[cfg(any(test, feature = "test-support"))]
        let runtime = WorkerRuntime {
            startup_ordering_hook,
            restore_race_hook,
            focus_pause_hook,
            ..WorkerRuntime::default()
        };
        #[cfg(not(any(test, feature = "test-support")))]
        let runtime = WorkerRuntime::default();
        Self {
            schedule: Some(schedule),
            config: WorkerConfig {
                backend,
                profile,
                timing,
                focus,
                wait,
                telemetry,
                priority,
                #[cfg(any(test, feature = "test-support"))]
                startup_calibration_placement,
                #[cfg(any(test, feature = "test-support"))]
                timing_observer_for_qualification,
                #[cfg(any(test, feature = "test-support"))]
                prepared_packet_ambiguity_mask,
                #[cfg(any(test, feature = "test-support"))]
                preflight_user_held_mask,
                #[cfg(any(test, feature = "test-support"))]
                modifier_key_state_query_for_test,
            },
            shared,
            epoch_qpc: QpcTicks::ZERO,
            instrument_key_profile: Some(instrument_key_profile),
            core: WorkerCore {
                runtime,
                ..WorkerCore::default()
            },
        }
    }

    fn take_schedule(&mut self) -> Result<RuntimeSchedule, &'static str> {
        self.schedule
            .take()
            .ok_or("worker runtime schedule was already consumed")
    }

    #[cfg(test)]
    pub(super) fn take_schedule_for_test(&mut self) -> Result<RuntimeSchedule, &'static str> {
        self.take_schedule()
    }

    pub(super) fn run(mut self) -> u8 {
        orchestration::run(&mut self)
    }
}
