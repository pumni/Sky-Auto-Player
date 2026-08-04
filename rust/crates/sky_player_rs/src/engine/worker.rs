mod admission;
mod cleanup;
mod control;
mod estimator;
mod health;
mod orchestration;
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

fn mock_platform_send_result_from_started_ticks(
    qpc_clock: QpcClock,
    started_ticks: Result<QpcTicks, QpcError>,
    requested: u32,
    inserted: u32,
    win32_error: u32,
    latency_ticks: u64,
) -> PlatformSendResult {
    let started_ticks = match started_ticks {
        Ok(ticks) => ticks,
        Err(error) => {
            return PlatformSendResult {
                requested,
                inserted: 0,
                started_ticks: QpcTicks::ZERO,
                completed_ticks: None,
                completed_us: 0,
                win32_error,
                timing_error: Some(error),
            };
        }
    };
    let deadline = match started_ticks.checked_add_duration(DurationTicks::from_raw(latency_ticks))
    {
        Ok(deadline) => deadline,
        Err(_) => {
            return PlatformSendResult {
                requested,
                inserted: 0,
                started_ticks,
                completed_ticks: None,
                completed_us: 0,
                win32_error,
                timing_error: Some(QpcError::DeadlineOverflow),
            };
        }
    };
    loop {
        match qpc_clock.now() {
            Ok(now) if now >= deadline => {
                let (completed_us, timing_error) =
                    match qpc_clock.duration_to_us(DurationTicks::from_raw(now.as_u64())) {
                        Ok(micros) => (micros, None),
                        Err(_) => (0, Some(QpcError::ConversionOverflow)),
                    };
                return PlatformSendResult {
                    requested,
                    inserted,
                    started_ticks,
                    completed_ticks: Some(now),
                    completed_us,
                    win32_error,
                    timing_error,
                };
            }
            Ok(_) => std::hint::spin_loop(),
            Err(error) => {
                return PlatformSendResult {
                    requested,
                    inserted: 0,
                    started_ticks,
                    completed_ticks: None,
                    completed_us: 0,
                    win32_error,
                    timing_error: Some(error),
                };
            }
        }
    }
}

pub(super) struct WorkerInputs<'a> {
    pub(super) interrupt: &'a OwnedEvent,
    pub(super) desired_pause: &'a AtomicBool,
    pub(super) quit_requested: &'a AtomicBool,
    pub(super) skip_requested: &'a AtomicBool,
    pub(super) panic_requested: &'a AtomicBool,
    pub(super) focus_active: &'a AtomicBool,
    pub(super) target_hwnd: &'a AtomicIsize,
    pub(super) target_generation: &'a AtomicU64,
    pub(super) metrics: &'a SharedMetrics,
    pub(super) telemetry_output: &'a Mutex<Option<NativeTelemetryOutput>>,
    pub(super) priority_acquired: &'a Mutex<String>,
    pub(super) estimator_output: &'a Mutex<Option<String>>,
    pub(super) supervisor_heartbeat_ticks: &'a AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) command_timing: &'a CommandTimingState,
}

impl<'a> WorkerInputs<'a> {
    pub(super) fn from_shared(shared: &'a SessionShared) -> Self {
        Self {
            interrupt: &shared.interrupt,
            desired_pause: &shared.desired_pause,
            quit_requested: &shared.quit_requested,
            skip_requested: &shared.skip_requested,
            panic_requested: &shared.panic_requested,
            focus_active: &shared.focus_active,
            target_hwnd: &shared.target_hwnd,
            target_generation: &shared.target_generation,
            metrics: &shared.metrics,
            telemetry_output: &shared.telemetry_output,
            priority_acquired: &shared.priority_acquired,
            estimator_output: &shared.estimator_output,
            supervisor_heartbeat_ticks: &shared.supervisor_heartbeat_ticks,
            #[cfg(any(test, feature = "test-support"))]
            command_timing: &shared.command_timing,
        }
    }
}

pub(super) struct Worker<'a> {
    config: WorkerConfig,
    interrupt: &'a OwnedEvent,
    desired_pause: &'a AtomicBool,
    quit_requested: &'a AtomicBool,
    skip_requested: &'a AtomicBool,
    panic_requested: &'a AtomicBool,
    focus_active: &'a AtomicBool,
    target_hwnd: &'a AtomicIsize,
    target_generation: &'a AtomicU64,
    metrics: &'a SharedMetrics,
    telemetry_output: &'a Mutex<Option<NativeTelemetryOutput>>,
    priority_acquired: &'a Mutex<String>,
    estimator_output: &'a Mutex<Option<String>>,
    supervisor_heartbeat_ticks: &'a AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    command_timing: &'a CommandTimingState,
}

impl<'a> Worker<'a> {
    pub(super) fn new(config: WorkerConfig, inputs: WorkerInputs<'a>) -> Self {
        let WorkerInputs {
            interrupt,
            desired_pause,
            quit_requested,
            skip_requested,
            panic_requested,
            focus_active,
            target_hwnd,
            target_generation,
            metrics,
            telemetry_output,
            priority_acquired,
            estimator_output,
            supervisor_heartbeat_ticks,
            #[cfg(any(test, feature = "test-support"))]
            command_timing,
        } = inputs;
        Self {
            config,
            interrupt,
            desired_pause,
            quit_requested,
            skip_requested,
            panic_requested,
            focus_active,
            target_hwnd,
            target_generation,
            metrics,
            telemetry_output,
            priority_acquired,
            estimator_output,
            supervisor_heartbeat_ticks,
            #[cfg(any(test, feature = "test-support"))]
            command_timing,
        }
    }

    pub(super) fn run(self) -> u8 {
        orchestration::run(self)
    }
}
