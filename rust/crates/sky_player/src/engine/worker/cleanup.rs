use super::{
    OUTCOME_ERROR, OUTCOME_FINISHED, OUTCOME_QUIT, OUTCOME_SKIPPED, OUTCOME_SYSTEM_SUSPENDED,
    TerminalTrigger, TrackedKeyState, WorkerMetricsLocal, WorkerSchedulingGuards,
    current_process_cpu_time_us, current_thread_cpu_time_us, publish_backend_metrics,
};
use crate::engine::shared::SessionTarget;
use crate::engine::shared::SharedProgressClock;
use crate::engine::shared::SupervisorLeaseState;
use crate::engine::shared::SystemPowerState;
use crate::engine::telemetry::{
    NativeTelemetryOutput, SharedMetrics, TelemetryCollector, publish_terminal_metrics,
};
use parking_lot::Mutex;
use sky_dispatch_core::accounting::GenerationAccounting;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::executor::DispatchExecutor;
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::input::ReleaseAllOutcome;
#[cfg(any(test, feature = "test-support"))]
use sky_dispatch_win32::input::TerminalReleaseEvidence;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub(super) struct FinalizeResources {
    pub(super) backend: TrackedKeyState,
    pub(super) scheduler: DispatchExecutor,
    pub(super) telemetry: Arc<Mutex<TelemetryCollector>>,
    pub(super) playback: PlaybackClockState,
    pub(super) qpc_clock: QpcClock,
    pub(super) scheduling: WorkerSchedulingGuards,
    pub(super) observer: Option<super::ObserverRuntime>,
}

pub(super) struct FinalizeState {
    pub(super) worker_result: Result<(), Box<dyn Any + Send>>,
    pub(super) local_metrics: WorkerMetricsLocal,
    pub(super) abort_counts: HashMap<&'static str, u64>,
    pub(super) terminal_error: Option<String>,
    pub(super) secondary_errors: Vec<String>,
    pub(super) last_published_error: Option<String>,
    pub(super) terminal_trigger: Option<TerminalTrigger>,
}

pub(super) struct FinalizeSignals<'a> {
    pub(super) target: &'a SessionTarget,
    pub(super) skip_requested: &'a AtomicBool,
    pub(super) quit_requested: &'a AtomicBool,
    pub(super) panic_requested: &'a AtomicBool,
    pub(super) supervisor_expired: &'a SupervisorLeaseState,
    pub(super) system_power: &'a SystemPowerState,
}

pub(super) struct FinalizePublication<'a> {
    pub(super) metrics: &'a SharedMetrics,
    pub(super) telemetry_output: &'a Mutex<Option<NativeTelemetryOutput>>,
    pub(super) priority_acquired: &'a Mutex<String>,
    pub(super) progress_clock: &'a SharedProgressClock,
}

pub(super) struct FinalizeTiming {
    pub(super) start_wall_time_us: u64,
    pub(super) start_thread_cpu_us: u64,
    pub(super) start_process_cpu_us: u64,
}

pub(super) struct FinalizeInput<'a> {
    pub(super) resources: FinalizeResources,
    pub(super) state: FinalizeState,
    pub(super) signals: FinalizeSignals<'a>,
    pub(super) publication: FinalizePublication<'a>,
    pub(super) timing: FinalizeTiming,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) cleanup_observation: Option<&'a mut FinalizeTestObservation>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
pub(crate) struct FinalizeTestObservation {
    pub(crate) release_obligation_mask: u16,
    pub(crate) active_mask: u16,
    pub(crate) possibly_active_mask: u16,
    pub(crate) failed_release_mask: u16,
    pub(crate) in_flight_mask: u16,
    pub(crate) attempted_mask: u16,
    pub(crate) terminal_release_attempts: u8,
    pub(crate) total_release_attempts: u64,
    pub(crate) cleanup_outcome: Option<ReleaseAllOutcome>,
    pub(crate) terminal_cleanup_evidence: Option<TerminalReleaseEvidence>,
    pub(crate) generation_accounting: sky_dispatch_core::accounting::GenerationAccounting,
    pub(crate) pre_cancel_generation_accounting:
        sky_dispatch_core::accounting::GenerationAccounting,
    pub(crate) terminal_trigger_reason: Option<String>,
    pub(crate) terminal_error: Option<String>,
    pub(crate) secondary_errors: Vec<String>,
    pub(crate) operation_trace: Option<Arc<Mutex<Vec<&'static str>>>>,
}

mod finalize;
pub(super) use finalize::finalize_worker;
pub(crate) fn release_outcome_verified(outcome: &ReleaseAllOutcome) -> bool {
    outcome.released_successfully && outcome.stuck_mask == 0 && !outcome.verification_inconclusive
}

pub(crate) fn release_state_verified(
    backend: &TrackedKeyState,
    outcome: &ReleaseAllOutcome,
) -> bool {
    release_outcome_verified(outcome) && backend.release_obligation_mask() == 0
}

pub(crate) fn clean_completion_proven(
    accounting: GenerationAccounting,
    backend: &TrackedKeyState,
) -> bool {
    clean_generation_accounting_proven(accounting)
        && backend.release_obligation_mask() == 0
        && backend.keys_dropped == 0
        && backend.chord_split_events == 0
        && backend.sendinput_partial_events == 0
        && backend.sendinput_zero_progress_failures == 0
        && backend.authored_keys_rejected == 0
}

fn clean_generation_accounting_proven(accounting: GenerationAccounting) -> bool {
    accounting.total == accounting.activated
        && accounting.total == accounting.released + accounting.cancelled
        && accounting.scheduled == 0
        && accounting.active == 0
        && accounting.dropped_conflict == 0
        && accounting.dropped_backend == 0
        && accounting.dropped_expired == 0
}

pub(crate) fn describe_release_outcome(
    backend: &TrackedKeyState,
    outcome: &ReleaseAllOutcome,
) -> String {
    format!(
        "released_successfully={}, stuck_keys={:?}, verification_inconclusive={}",
        outcome.released_successfully,
        outcome.stuck_keys_with_profile(backend.instrument_key_profile()),
        outcome.verification_inconclusive
    )
}

pub(crate) fn record_termination_error(
    primary: &mut Option<String>,
    secondary: &mut Vec<String>,
    error: String,
) {
    if primary.is_none() {
        *primary = Some(error);
    } else if primary.as_deref() != Some(error.as_str()) && !secondary.contains(&error) {
        secondary.push(error);
    }
}

pub(crate) fn suspend_scheduler_input(
    backend: &mut TrackedKeyState,
    scheduler: &mut DispatchExecutor,
    target_hwnd: isize,
    post_cleanup_qpc: impl FnOnce() -> Result<QpcTicks, String>,
) -> Result<usize, String> {
    let release = backend.release_all(target_hwnd);
    if !release_state_verified(backend, &release) {
        return Err(format!(
            "release verification failed: {}",
            describe_release_outcome(backend, &release)
        ));
    }
    let cleanup_completed_qpc = post_cleanup_qpc()
        .map_err(|error| format!("post-cleanup resume-barrier QPC failure: {error}"))?;
    let cancelled = scheduler.on_verified_cleanup_at(cleanup_completed_qpc)?;
    Ok(cancelled as usize)
}
