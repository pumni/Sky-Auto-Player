use super::{
    OUTCOME_ERROR, OUTCOME_FINISHED, OUTCOME_QUIT, OUTCOME_SKIPPED, RuntimeDispatchCoordinator,
    TrackedKeyState, WorkerMetricsLocal, WorkerSchedulingGuards, current_process_cpu_time_us,
    current_thread_cpu_time_us, publish_backend_metrics, try_publish_metrics,
};
use crate::engine::shared::SharedProgressClock;
use crate::engine::telemetry::{NativeTelemetryOutput, SharedMetrics, TelemetryCollector};
use parking_lot::Mutex;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::{QpcClock, QpcError};
use sky_dispatch_win32::input::{ReleaseAllOutcome, ReleaseScope};
use std::any::Any;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

pub(super) struct FinalizeResources {
    pub(super) backend: TrackedKeyState,
    pub(super) coordinator: RuntimeDispatchCoordinator,
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
    pub(super) force_full_cleanup: bool,
    pub(super) terminal_error: Option<String>,
    pub(super) secondary_errors: Vec<String>,
    pub(super) last_published_error: Option<String>,
}

pub(super) struct FinalizeSignals<'a> {
    pub(super) target_hwnd: &'a AtomicIsize,
    pub(super) skip_requested: &'a AtomicBool,
    pub(super) quit_requested: &'a AtomicBool,
}

pub(super) struct FinalizePublication<'a> {
    pub(super) metrics: &'a SharedMetrics,
    pub(super) telemetry_output: &'a Mutex<Option<NativeTelemetryOutput>>,
    pub(super) estimator_output: &'a Mutex<Option<String>>,
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
}

pub(super) fn finalize_worker(context: FinalizeInput<'_>) -> u8 {
    let FinalizeInput {
        resources,
        state,
        signals,
        publication,
        timing,
    } = context;
    let FinalizeResources {
        mut backend,
        mut coordinator,
        telemetry,
        playback,
        qpc_clock,
        scheduling,
        mut observer,
    } = resources;
    let FinalizeState {
        worker_result,
        mut local_metrics,
        mut abort_counts,
        mut force_full_cleanup,
        mut terminal_error,
        mut secondary_errors,
        mut last_published_error,
    } = state;
    let FinalizeSignals {
        target_hwnd,
        skip_requested,
        quit_requested,
    } = signals;
    let FinalizePublication {
        metrics,
        telemetry_output,
        estimator_output,
        priority_acquired,
        progress_clock,
    } = publication;
    let FinalizeTiming {
        start_wall_time_us,
        start_thread_cpu_us,
        start_process_cpu_us,
    } = timing;

    // Validate before either cleanup operation can erase the evidence of a
    // coordinator mismatch. The first failure remains primary; later cleanup
    // and accounting failures are retained as secondary diagnostics.
    if let Err(error) = coordinator.check_invariants() {
        force_full_cleanup = true;
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            format!("coordinator pre-cleanup invariant failure: {error}"),
        );
    }

    if worker_result.is_err() {
        force_full_cleanup = true;
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            "worker panicked before terminal cleanup".to_string(),
        );
    }

    // This cleanup sits outside the contained loop so it also runs when an
    // unexpected panic crosses the orchestration/backend seam. The release
    // scope is decided once, up front: a terminal/full state releases the
    // whole instrument, a normal completion releases only the tracked set.
    // A second cleanup FSM is never chained, so cleanup latency is bounded to
    // a single FSM invocation.
    let release_scope = if worker_result.is_err() || force_full_cleanup {
        ReleaseScope::FullInstrument
    } else {
        ReleaseScope::Tracked
    };
    let cleanup_result = catch_unwind(AssertUnwindSafe(|| {
        backend.release_scope(release_scope, target_hwnd.load(Ordering::Acquire))
    }));
    if let Ok(outcome) = &cleanup_result {
        *metrics.terminal_release_outcome.lock() = Some(outcome.clone());
        if !release_state_verified(&backend, outcome) {
            record_termination_error(
                &mut terminal_error,
                &mut secondary_errors,
                format!(
                    "terminal release verification failed: {}",
                    describe_release_outcome(outcome)
                ),
            );
        }
    } else {
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            "terminal backend cleanup panicked".to_string(),
        );
    }

    if terminal_error.is_none()
        && !skip_requested.load(Ordering::Acquire)
        && !quit_requested.load(Ordering::Acquire)
        && !clean_completion_proven(&coordinator, &backend)
    {
        terminal_error = Some(
            "clean completion contract failed: authored generations or backend state were not fully released"
                .to_string(),
        );
    }

    if let Err(error) = coordinator.cancel_all() {
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            format!("coordinator cancellation failure: {error}"),
        );
    }
    if let Err(error) = coordinator.check_post_cleanup_invariants() {
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            format!("coordinator post-cleanup invariant failure: {error}"),
        );
    }
    if let Some(observer_runtime) = observer.take() {
        let observer_output = observer_runtime.stop();
        local_metrics.merge_observer(&observer_output.metrics);
        if let Some(error) = observer_output.terminal_error {
            record_termination_error(
                &mut terminal_error,
                &mut secondary_errors,
                format!("observer consumer failed: {error}"),
            );
        }
    }
    let end_ticks = qpc_clock.now();
    let end_qpc = end_ticks.and_then(|ticks| {
        qpc_clock
            .duration_to_us(DurationTicks::from_raw(ticks.as_u64()))
            .map_err(|_| QpcError::ConversionOverflow)
    });
    let end_us = match end_qpc {
        Ok(value) => value,
        Err(error) => {
            record_termination_error(
                &mut terminal_error,
                &mut secondary_errors,
                format!("QPC runtime failure during termination: {error:?}"),
            );
            start_wall_time_us
        }
    };
    if let Ok(terminal_ticks) = end_ticks {
        progress_clock.publish_terminal(&playback, terminal_ticks);
    }
    let terminal_abort_reason =
        if worker_result.is_err() || cleanup_result.is_err() || terminal_error.is_some() {
            "error"
        } else if skip_requested.load(Ordering::Acquire) {
            "skipped"
        } else if quit_requested.load(Ordering::Acquire) {
            "quit"
        } else {
            "finished"
        };
    *abort_counts.entry(terminal_abort_reason).or_insert(0) += 1;
    *metrics.abort_counts_by_reason.lock() = abort_counts
        .into_iter()
        .map(|(reason, count)| (reason.to_string(), count))
        .collect();
    *metrics.terminal_error.lock() = terminal_error.clone();
    *metrics.secondary_errors.lock() = secondary_errors;
    *metrics.generation_status_counts.lock() = coordinator.generation_status_counts();
    publish_backend_metrics(
        &backend,
        &mut local_metrics,
        metrics,
        &mut last_published_error,
    );

    local_metrics.playback_wall_time_us = end_us.saturating_sub(start_wall_time_us);
    local_metrics.worker_cpu_time_us =
        current_thread_cpu_time_us().saturating_sub(start_thread_cpu_us);
    local_metrics.process_cpu_time_us =
        current_process_cpu_time_us().saturating_sub(start_process_cpu_us);
    if local_metrics.playback_wall_time_us > 0 {
        local_metrics.spin_duty_cycle_ppm = (local_metrics.spin_time_us as u128 * 1_000_000
            / local_metrics.playback_wall_time_us as u128)
            as u64;
    }
    try_publish_metrics(&local_metrics, metrics, end_us, true);
    metrics.is_paused.store(false, Ordering::Relaxed);
    let telemetry = match Arc::try_unwrap(telemetry) {
        Ok(telemetry) => telemetry.into_inner(),
        Err(_) => panic!("observer telemetry must be uniquely owned after observer shutdown"),
    };
    let mut telemetry = telemetry;
    telemetry.output.qpc_frequency_hz = qpc_clock.frequency_hz().get();
    *telemetry_output.lock() = Some(std::mem::take(&mut telemetry.output));
    // Kept as a stable compatibility publication for older Python callers.
    // Adaptive dispatch estimation is no longer part of the runtime.
    *estimator_output.lock() = Some("{\"deprecated\":true}".to_string());
    drop(scheduling);
    *priority_acquired.lock() = "off".to_string();
    local_metrics.power_throttling_disabled = false;
    try_publish_metrics(&local_metrics, metrics, end_us, true);
    match (worker_result, cleanup_result) {
        (Err(payload), _) | (Ok(_), Err(payload)) => resume_unwind(payload),
        (Ok(_), Ok(_)) => {}
    }
    if terminal_error.is_some() {
        OUTCOME_ERROR
    } else if skip_requested.load(Ordering::Acquire) {
        OUTCOME_SKIPPED
    } else if quit_requested.load(Ordering::Acquire) {
        OUTCOME_QUIT
    } else {
        OUTCOME_FINISHED
    }
}

pub(crate) fn cancel_coordinator_or_terminal(
    coordinator: &mut RuntimeDispatchCoordinator,
    force_full_cleanup: &mut bool,
    terminal_error: &mut Option<String>,
    secondary_errors: &mut Vec<String>,
) {
    if let Err(error) = coordinator.cancel_all() {
        *force_full_cleanup = true;
        record_termination_error(
            terminal_error,
            secondary_errors,
            format!("coordinator cancellation failure: {error}"),
        );
    }
}

pub(crate) fn release_outcome_verified(outcome: &ReleaseAllOutcome) -> bool {
    outcome.released_successfully && outcome.stuck_mask == 0 && !outcome.verification_inconclusive
}

pub(crate) fn release_state_verified(
    backend: &TrackedKeyState,
    outcome: &ReleaseAllOutcome,
) -> bool {
    release_outcome_verified(outcome)
        && backend.active_mask == 0
        && backend.possibly_active_mask == 0
        && backend.failed_release_mask == 0
}

pub(crate) fn clean_completion_proven(
    coordinator: &RuntimeDispatchCoordinator,
    backend: &TrackedKeyState,
) -> bool {
    let counts = coordinator.generation_status_counts();
    let all_released = counts.get("released").copied().unwrap_or_default()
        == coordinator.schedule.generation_count
        && counts.values().sum::<u64>() == coordinator.schedule.generation_count;
    all_released
        && counts.get("scheduled").copied().unwrap_or_default() == 0
        && counts.get("active").copied().unwrap_or_default() == 0
        && counts.get("dropped_backend").copied().unwrap_or_default() == 0
        && counts.get("dropped_conflict").copied().unwrap_or_default() == 0
        && counts.get("dropped_expired").copied().unwrap_or_default() == 0
        && counts.get("cancelled").copied().unwrap_or_default() == 0
        && backend.active_mask == 0
        && backend.possibly_active_mask == 0
        && backend.failed_release_mask == 0
        && backend.keys_dropped == 0
        && backend.chord_split_events == 0
        && backend.sendinput_partial_events == 0
        && backend.sendinput_zero_progress_failures == 0
        && backend.authored_keys_rejected == 0
}

pub(crate) fn describe_release_outcome(outcome: &ReleaseAllOutcome) -> String {
    format!(
        "released_successfully={}, stuck_keys={:?}, verification_inconclusive={}",
        outcome.released_successfully,
        outcome.stuck_keys(),
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

/// Release physical input before cancelling only generations that still own it.
///
/// A suspend is resumable: authored generations that have not reached the
/// backend remain Scheduled. The backend result is checked before coordinator
/// state is changed, so an inconclusive release cannot be mistaken for a clean
/// pause.
pub(crate) fn suspend_live_input(
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    target_hwnd: isize,
) -> Result<Vec<u64>, String> {
    // A suspension is fail-closed: release the whole instrument in a single
    // FSM invocation. The scope is decided before the call so two release
    // FSMs are never chained (which held the total cleanup latency at
    // ~330 ms plus duplicated retries).
    let release = backend.release_all_full_instrument(target_hwnd);
    if !release_state_verified(backend, &release) {
        return Err(format!(
            "release verification failed: {}",
            describe_release_outcome(&release)
        ));
    }

    debug_assert!(release_state_verified(backend, &release));
    let cancelled = coordinator
        .cancel_live_generations()
        .map_err(|error| format!("coordinator live cancellation failed: {error}"))?;
    coordinator
        .check_invariants()
        .map_err(|error| format!("coordinator invariant failure after suspension: {error}"))?;
    Ok(cancelled)
}

pub(crate) fn release_runtime_outcome(
    deferred_by_us: u64,
    sent_count: usize,
    requested_count: usize,
    _recovery_required: bool,
) -> &'static str {
    let deferred = deferred_by_us > 0;
    match (sent_count == requested_count, sent_count > 0, deferred) {
        (true, _, true) => "deferred_release",
        (true, _, false) => "sent",
        (false, true, true) => "deferred_partial_note_off",
        (false, true, false) => "partial_note_off",
        (false, false, true) => "deferred_failed_note_off",
        (false, false, false) => "failed_note_off",
    }
}
