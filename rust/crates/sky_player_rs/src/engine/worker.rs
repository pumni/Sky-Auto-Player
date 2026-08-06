mod admission;
mod cleanup;
mod control;
pub(super) mod dispatch;
mod estimator;
mod health;
mod orchestration;
mod planning;
mod startup;
mod timing;
mod wait;

pub(super) use dispatch::{
    AuthoredPacketContext, DispatchStep, PendingReleaseContext, dispatch_authored_packet,
    dispatch_due_pending_releases,
};
pub(crate) use planning::plan_next_dispatch;

pub(crate) use admission::{
    DownAdmission, TargetStamp, ensure_preflight_for_target, final_down_admission, focus_matches,
    focus_matches_hwnd, load_target_stamp, target_stamp_still_current,
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
#[cfg(test)]
pub(crate) use estimator::update_estimator_after_send;
pub(crate) use estimator::{record_lead_saturation, update_estimator_after_send_class};
pub(crate) use health::HealthWindowPolicy;
#[cfg(test)]
pub(crate) use health::record_input_path_health;
pub(crate) use health::{
    DispatchHealthObservation, DispatchHealthOptions, DispatchPath, HEALTH_WINDOW_CAPACITY,
    HealthWindow, build_dispatch_budget, estimator_kind_for_path, focus_gate_matches,
    observe_dispatch_health, observe_wait_health, publish_backend_metrics, record_lateness,
};
use startup::{StartupResources, initialize_startup};
#[cfg(test)]
pub(crate) use timing::{
    adjust_spin_threshold, anchored_dispatch_target_ticks, deadline_target_ticks,
    exact_sender_durations,
};
pub(crate) use timing::{
    anchored_dispatch_target_ticks_typed, classify_latency_class, derive_spin_threshold_us,
    lease_bounded_ticks, publish_wake_error_stats, signed_delta, signed_ticks_to_us,
    signed_timeline_delta_ticks, supervisor_lease_expired, wait_failure_message,
    wake_lateness_ticks,
};
use wait::{
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
pub(super) struct WorkerRuntime {
    verified_target: Option<TargetStamp>,
    startup_gate: Option<(TimelineTicks, DurationTicks)>,
    focus_restore_started_ticks: Option<QpcTicks>,
    last_send_qpc_ticks: Option<QpcTicks>,
    force_full_cleanup: bool,
    terminal_error: Option<String>,
    focus_loss_fault_injected: bool,
    allow_pre_epoch_startup_dispatch: bool,
    pending_pre_send_spin_us: u64,
}

#[derive(Default)]
pub(super) struct WorkerErrorState {
    secondary: Vec<String>,
    last_published: Option<String>,
    abort_counts: HashMap<&'static str, u64>,
}

#[derive(Clone, Copy)]
pub(super) struct WorkerTimingState {
    pub(super) hard_late_abort_threshold_ticks: DurationTicks,
    pub(super) retry_late_threshold_ticks: DurationTicks,
    pub(super) strict_down_completion_late_ticks: DurationTicks,
    pub(super) strict_up_completion_late_ticks: DurationTicks,
    pub(super) focus_restore_grace_ticks: DurationTicks,
    pub(super) paused_poll_ticks: DurationTicks,
    pub(super) cold_threshold_ticks: DurationTicks,
    pub(super) core_warmup_ticks: DurationTicks,
    pub(super) lease_timeout_ticks: DurationTicks,
    pub(super) retry_backoff_ticks: [DurationTicks; RELEASE_RETRY_BACKOFF_US.len()],
    pub(super) effective_spin_threshold_ticks: DurationTicks,
    pub(super) start_wall_time_us: u64,
    pub(super) start_thread_cpu_us: u64,
    pub(super) start_process_cpu_us: u64,
    pub(super) last_cpu_metrics_sample_us: u64,
}

pub(super) struct WorkerHealthState {
    pub(super) down_saturation_positive_streak: u8,
    pub(super) up_saturation_positive_streak: u8,
    pub(super) options: DispatchHealthOptions,
    pub(super) send_pure_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
    pub(super) bookkeeping_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
    pub(super) wait_window: HealthWindow<HEALTH_WINDOW_CAPACITY>,
}

pub(super) struct WorkerResources {
    pub(super) clock: QpcClock,
    pub(super) waiter: HybridWaiter,
    pub(super) backend: TrackedKeyState,
    pub(super) coordinator: RuntimeDispatchCoordinator,
    pub(super) playback: PlaybackClockState,
    pub(super) estimator: SendLatencyEstimator,
    pub(super) telemetry: TelemetryCollector,
}

#[derive(Default)]
pub(super) struct WorkerCore {
    pub(super) resources: Option<WorkerResources>,
    pub(super) metrics: WorkerMetricsLocal,
    pub(super) health: Option<WorkerHealthState>,
    pub(super) timing: Option<WorkerTimingState>,
    pub(super) runtime: WorkerRuntime,
    pub(super) errors: WorkerErrorState,
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
