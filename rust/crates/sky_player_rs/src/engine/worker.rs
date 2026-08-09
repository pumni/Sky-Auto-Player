mod admission;
mod boot;
mod cleanup;
mod control;
#[cfg(not(any(test, feature = "test-support")))]
mod dispatch;
// `pub(crate)` under test / test-support so engine.rs can re-export the
// observer queue primitives (§8.11) and slow-observer hooks (§8.12) to the
// public API without a private-module path.
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod dispatch;
mod dispatch_loop;
mod estimator;
#[cfg(not(any(test, feature = "test-support")))]
mod health;
#[cfg(any(test, feature = "test-support"))]
pub(crate) mod health;
mod orchestration;
mod planning;
mod startup;
mod timing;
mod wait;

pub(crate) use admission::{
    DownAdmission, FinalControlAdmission, FinalControlSignals, FinalTargetSignals, TargetStamp,
    ensure_preflight_for_target, final_control_admission_with_lease, final_down_target_admission,
    focus_matches, focus_matches_hwnd, load_target_stamp, target_stamp_still_current,
};
use cleanup::{
    FinalizeInput, FinalizePublication, FinalizeResources, FinalizeSignals, FinalizeState,
    FinalizeTiming, finalize_worker,
};
pub(crate) use cleanup::{
    cancel_coordinator_or_terminal, describe_release_outcome, record_termination_error,
    release_runtime_outcome, release_state_verified, suspend_live_input,
};
use control::{
    CommandControl, CommandControlClock, CommandControlInput, CommandControlMetrics,
    CommandControlRuntime, CommandControlSignals, process_command_control,
};
pub(super) use dispatch::{
    AuthoredPacketContext, DispatchObservation, DispatchStep, PendingReleaseContext,
    dispatch_authored_packet, dispatch_due_pending_releases, drain_one_observer,
    observer_has_safe_slack,
};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use dispatch_loop::dispatch_due_from_plan;

#[cfg(test)]
pub(crate) use estimator::update_estimator_after_send;
pub(crate) use estimator::{record_lead_saturation, update_estimator_after_send_class};
#[cfg(any(test, feature = "test-support"))]
pub(crate) use health::FrozenDispatchBudget;
#[cfg(test)]
pub(crate) use health::HealthWindowPolicy;
#[cfg(test)]
pub(crate) use health::record_input_path_health;
pub(crate) use health::{
    DispatchHealthObservation, DispatchHealthOptions, DispatchPath, HEALTH_WINDOW_CAPACITY,
    HealthWindow, estimator_path_for_dispatch, focus_gate_matches, observe_dispatch_health,
    observe_wait_health, publish_backend_metrics, record_lateness,
};
#[cfg(any(test, feature = "test-support"))]
pub use planning::NextDispatchPlan;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use planning::plan_next_dispatch;
pub(crate) use planning::startup_lead_for_first_packet;
pub(crate) use planning::{
    ProjectedPlanningInput, plan_next_dispatch_projected, plan_structure_is_valid,
};
pub(crate) use startup::WorkerSchedulingGuards;
use startup::{StartupResources, initialize_startup};
#[cfg(test)]
pub(crate) use timing::classify_latency_class;
#[cfg(test)]
pub(crate) use timing::{
    adjust_spin_threshold, anchored_dispatch_target_ticks, deadline_target_ticks,
    exact_sender_durations,
};
pub(crate) use timing::{
    anchored_dispatch_target_ticks_typed, derive_spin_threshold_us, lease_bounded_ticks,
    publish_wake_error_stats, signed_delta, signed_ticks_to_us, signed_timeline_delta_ticks,
    supervisor_lease_expired, wait_failure_message, wake_lateness_ticks,
};
pub(crate) use wait::{
    WaitBoundary, WaitBoundaryInput, WaitDeadline, WaitMutable, WaitSignals, WaitTiming,
    wait_for_next_boundary,
};

use super::shared::SessionShared;
use super::*;
use sky_dispatch_core::model::RuntimeSchedule;

/// Mutable state owned exclusively by the worker thread.
///
/// This state deliberately lives outside the panic boundary so the worker's
/// backend, coordinator, and telemetry can still be finalized after an
/// injected or unexpected panic.
#[derive(Default)]
pub(crate) struct WorkerRuntime {
    verified_target: Option<TargetStamp>,
    startup_gate: Option<(TimelineTicks, DurationTicks)>,
    focus_restore_started_ticks: Option<QpcTicks>,
    last_send_qpc_ticks: Option<QpcTicks>,
    last_dispatch_deadline_wake_qpc: Option<QpcTicks>,
    pub(crate) force_full_cleanup: bool,
    pub(crate) terminal_error: Option<String>,
    focus_loss_fault_injected: bool,
    allow_pre_epoch_startup_dispatch: bool,
    pub(crate) pending_pre_send_spin_us: u64,
    pending_wait_observation: Option<wait::WaitObservation>,
    chord_integrity_lost: u64,
}

#[cfg(any(test, feature = "test-support"))]
impl WorkerRuntime {
    pub(crate) fn create_test_runtime(verified_target: Option<TargetStamp>) -> Self {
        Self {
            verified_target,
            ..Self::default()
        }
    }

    pub(crate) fn chord_integrity_lost_count(&self) -> u64 {
        self.chord_integrity_lost
    }

    pub(crate) fn set_last_send_qpc_for_test(&mut self, ticks: Option<QpcTicks>) {
        self.last_send_qpc_ticks = ticks;
    }

    pub(crate) fn last_send_qpc_for_test(&self) -> Option<QpcTicks> {
        self.last_send_qpc_ticks
    }

    pub(crate) fn set_deadline_wake_qpc_for_test(&mut self, ticks: Option<QpcTicks>) {
        self.last_dispatch_deadline_wake_qpc = ticks;
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
    pub(super) hard_late_abort_threshold_ticks: DurationTicks,
    pub(super) retry_late_threshold_ticks: DurationTicks,
    pub(super) strict_down_completion_late_ticks: DurationTicks,
    pub(super) strict_up_completion_late_ticks: DurationTicks,
    pub(super) focus_restore_grace_ticks: DurationTicks,
    pub(super) paused_poll_ticks: DurationTicks,
    pub(crate) cold_threshold_ticks: DurationTicks,
    pub(crate) lease_timeout_ticks: DurationTicks,
    pub(super) retry_backoff_ticks: [DurationTicks; RELEASE_RETRY_BACKOFF_US.len()],
    pub(crate) effective_spin_threshold_ticks: DurationTicks,
    pub(super) start_wall_time_us: u64,
    pub(super) start_thread_cpu_us: u64,
    pub(super) start_process_cpu_us: u64,
    pub(super) last_cpu_metrics_sample_us: u64,
}

#[cfg(any(test, feature = "test-support"))]
impl WorkerTimingState {
    pub(crate) fn create_test_timing() -> Self {
        Self {
            hard_late_abort_threshold_ticks: DurationTicks::ZERO,
            retry_late_threshold_ticks: DurationTicks::ZERO,
            strict_down_completion_late_ticks: DurationTicks::ZERO,
            strict_up_completion_late_ticks: DurationTicks::ZERO,
            focus_restore_grace_ticks: DurationTicks::ZERO,
            paused_poll_ticks: DurationTicks::ZERO,
            cold_threshold_ticks: DurationTicks::ZERO,
            lease_timeout_ticks: DurationTicks::ZERO,
            retry_backoff_ticks: [DurationTicks::ZERO; RELEASE_RETRY_BACKOFF_US.len()],
            effective_spin_threshold_ticks: DurationTicks::ZERO,
            start_wall_time_us: 0,
            start_thread_cpu_us: 0,
            start_process_cpu_us: 0,
            last_cpu_metrics_sample_us: 0,
        }
    }
}

pub(crate) struct WorkerHealthState {
    pub(super) down_saturation_positive_streak: u8,
    pub(super) up_saturation_positive_streak: u8,
    pub(super) options: DispatchHealthOptions,
    pub(super) sendinput_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
    pub(super) core_post_send_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
    pub(super) observer_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
    pub(super) wait_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
}

#[cfg(any(test, feature = "test-support"))]
impl WorkerHealthState {
    pub(crate) fn new(options: DispatchHealthOptions) -> Self {
        Self {
            down_saturation_positive_streak: 0,
            up_saturation_positive_streak: 0,
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
    pub(super) coordinator: RuntimeDispatchCoordinator,
    pub(super) playback: PlaybackClockState,
    pub(super) estimator: SendLatencyEstimator,
    pub(super) telemetry: TelemetryCollector,
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
/// `pending` holds the fixed observation queue; `budget_us` is the adaptive
/// execution budget a drain step is allowed to consume before a dispatch
/// deadline arrives.  Never placed into shared state.
pub(super) struct WorkerObserverState {
    pub(super) pending: dispatch::PendingObservationQueue,
    pub(super) budget_us: u64,
}

impl Default for WorkerObserverState {
    fn default() -> Self {
        Self {
            pending: dispatch::PendingObservationQueue::default(),
            budget_us: observer_initial_budget_us(),
        }
    }
}

/// Initial deferred-observer execution budget, in microseconds (§8.8).
pub(super) const OBSERVER_INITIAL_BUDGET_US: u64 = 5_000;
/// Margin kept in addition to the observer budget before a drain is allowed,
/// in microseconds (§8.8).
pub(super) const OBSERVER_MARGIN_US: u64 = 500;
/// Lower bound for adaptive observer budget adaptation (§8.9).
pub(super) const OBSERVER_BUDGET_FLOOR_US: u64 = 5_000;
/// Upper bound for adaptive observer budget adaptation (§8.9).
pub(super) const OBSERVER_BUDGET_CAP_US: u64 = 20_000;

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
        local_metrics.worker_cpu_time_us =
            current_thread_cpu_time_us().saturating_sub(timing.start_thread_cpu_us);
        local_metrics.process_cpu_time_us =
            current_process_cpu_time_us().saturating_sub(timing.start_process_cpu_us);
        timing.last_cpu_metrics_sample_us = wall_now_us;
    }
    local_metrics.playback_wall_time_us = wall_now_us.saturating_sub(timing.start_wall_time_us);
    if local_metrics.playback_wall_time_us > 0 {
        local_metrics.spin_duty_cycle_ppm = (local_metrics.spin_time_us as u128 * 1_000_000
            / local_metrics.playback_wall_time_us as u128)
            as u64;
    }
}

fn observer_initial_budget_us() -> u64 {
    #[cfg(any(test, feature = "test-support"))]
    {
        let override_us = dispatch::observer_initial_budget_override_us();
        if override_us > 0 {
            return override_us;
        }
    }
    OBSERVER_INITIAL_BUDGET_US
}

pub(super) struct Worker<'a> {
    schedule: Option<RuntimeSchedule>,
    config: WorkerConfig,
    shared: &'a SessionShared,
    core: WorkerCore,
}

impl<'a> Worker<'a> {
    pub(super) fn new(options: NativeSessionOptions, shared: &'a SessionShared) -> Self {
        let NativeSessionOptions {
            schedule,
            backend,
            allowed_count,
            timing,
            focus,
            wait,
            telemetry,
            priority,
            estimator,
        } = options;
        Self {
            schedule: Some(schedule),
            config: WorkerConfig {
                backend,
                allowed_count,
                timing,
                focus,
                wait,
                telemetry,
                priority,
                estimator,
            },
            shared,
            core: WorkerCore::default(),
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
