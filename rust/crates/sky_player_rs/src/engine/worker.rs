mod admission;
mod cleanup;
mod control;
mod estimator;
mod health;
mod orchestration;
mod startup;
mod timing;
mod wait;

pub(crate) use admission::{
    DownAdmission, TargetStamp, ensure_preflight_for_target, final_down_admission, focus_matches,
    focus_matches_hwnd, load_target_stamp, target_stamp_still_current,
};
use cleanup::{FinalizeContext, finalize_worker};
pub(crate) use cleanup::{
    cancel_coordinator_or_terminal, describe_release_outcome, record_termination_error,
    release_runtime_outcome, release_state_verified, suspend_live_input,
};
use control::{CommandControl, CommandControlContext, process_command_control};
#[cfg(test)]
pub(crate) use estimator::update_estimator_after_send;
pub(crate) use estimator::{record_lead_saturation, update_estimator_after_send_class};
pub(crate) use health::{
    focus_gate_matches, publish_backend_metrics, record_input_path_health, record_lateness,
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
use wait::{WaitBoundary, WaitBoundaryContext, wait_for_next_boundary};

use super::shared::SessionShared;
use super::*;

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

#[derive(Default)]
pub(super) struct WorkerCore {
    pub(super) metrics: WorkerMetricsLocal,
    pub(super) runtime: WorkerRuntime,
    pub(super) errors: WorkerErrorState,
}

pub(super) struct Worker<'a> {
    config: WorkerConfig,
    shared: &'a SessionShared,
    core: WorkerCore,
}

impl<'a> Worker<'a> {
    pub(super) fn new(config: WorkerConfig, shared: &'a SessionShared) -> Self {
        Self {
            config,
            shared,
            core: WorkerCore::default(),
        }
    }

    pub(super) fn run(self) -> u8 {
        orchestration::run(self)
    }
}
