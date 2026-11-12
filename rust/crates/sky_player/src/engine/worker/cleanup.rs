use super::{
    OUTCOME_ERROR, OUTCOME_FINISHED, OUTCOME_QUIT, OUTCOME_SKIPPED, OUTCOME_SYSTEM_SUSPENDED,
    TerminalTrigger, TrackedKeyState, WorkerMetricsLocal, WorkerSchedulingGuards,
    current_process_cpu_time_us, current_thread_cpu_time_us, publish_backend_metrics,
};
#[cfg(any(test, feature = "test-support"))]
use super::{RuntimeDispatchCoordinator, WorkerRuntime};
use crate::engine::shared::SessionTarget;
use crate::engine::shared::SharedProgressClock;
use crate::engine::shared::SupervisorLeaseState;
use crate::engine::shared::SystemPowerState;
use crate::engine::telemetry::{
    NativeTelemetryOutput, SharedMetrics, TelemetryCollector, publish_terminal_metrics,
};
use parking_lot::Mutex;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::coordinator::GenerationAccounting;
use sky_dispatch_core::executor::DispatchExecutor;
#[cfg(any(test, feature = "test-support"))]
use sky_dispatch_core::time::TimelineTicks;
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::input::ReleaseAllOutcome;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub(super) struct FinalizeResources {
    pub(super) backend: TrackedKeyState,
    pub(super) scheduler: DispatchExecutor,
    /// Legacy ledger used only by the coordinator-based deterministic test harness.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) legacy_coordinator: Option<RuntimeDispatchCoordinator>,
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
    pub(crate) generation_accounting: sky_dispatch_core::coordinator::GenerationAccounting,
    pub(crate) pre_cancel_generation_accounting:
        sky_dispatch_core::coordinator::GenerationAccounting,
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

/// Release physical input before resolving a pending mixed Down miss and
/// cancelling the remaining live generations.
///
/// A suspend is resumable: authored generations that have not reached the
/// backend remain Scheduled. The backend result is checked before coordinator
/// state is changed, and a frozen miss is committed while its authored Up
/// ownership is still active.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn suspend_live_input(
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    runtime: &mut WorkerRuntime,
    effective_now_ticks: Result<TimelineTicks, String>,
    target_hwnd: isize,
    post_cleanup_qpc: impl FnOnce() -> Result<QpcTicks, String>,
    post_cleanup_timeline_ticks: impl FnOnce(QpcTicks) -> Result<TimelineTicks, String>,
) -> Result<usize, String> {
    runtime.invalidate_down_authorization();
    if let Some(guard) = runtime.physical_timing_guard.as_mut() {
        guard.invalidate();
    }

    // A suspension is fail-closed: release the owned or possibly owned keys
    // in a single FSM invocation. The scope is decided before the call so two
    // release FSMs are never chained (which held the total cleanup latency at
    // ~330 ms plus duplicated retries).
    let release = backend.release_all(target_hwnd);
    if !release_state_verified(backend, &release) {
        return Err(format!(
            "release verification failed: {}",
            describe_release_outcome(backend, &release)
        ));
    }

    let cleanup_completed_qpc = post_cleanup_qpc()
        .map_err(|error| format!("post-cleanup resume-barrier QPC failure: {error}"))?;
    #[cfg(any(test, feature = "test-support"))]
    if let Some(hook) = runtime.startup_ordering_hook.as_ref() {
        hook.record_resume_cleanup_qpc(cleanup_completed_qpc.as_u64());
    }
    let guard = runtime
        .physical_timing_guard
        .as_mut()
        .ok_or_else(|| "physical timing guard is not initialized after cleanup".to_string())?;
    guard
        .establish_resume_down_barrier(cleanup_completed_qpc)
        .map_err(|error| format!("resume Down barrier establishment failed: {error:?}"))?;

    debug_assert!(release_state_verified(backend, &release));
    if let Some(pending) = runtime.pending_up_recovery.as_ref() {
        let frame = pending.authored_commit.frame;
        #[cfg(any(test, feature = "test-support"))]
        let admission_mismatch = !pending.admission.is_missed();
        #[cfg(not(any(test, feature = "test-support")))]
        let admission_mismatch = false;
        if pending.boundary.first_batch_index != frame.first_batch_index
            || pending.boundary.packet_index != frame.packet_index
            || pending.boundary.packet_batch_count != frame.packet_batch_count
            || pending.boundary.down_mask != frame.down_mask
            || pending.boundary.up_mask & frame.immediate_up_mask != frame.immediate_up_mask
            || pending.boundary.down_mask == 0
            || admission_mismatch
        {
            return Err("pending Up recovery no longer matches its frozen authored miss".into());
        }
        let resolved_now_ticks = effective_now_ticks.as_ref().map_err(|error| {
            format!("playback clock failure while resolving missed Down: {error}")
        })?;
        coordinator
            .commit_prepared_authored_frame_deadline_miss(
                &pending.authored_commit,
                frame.immediate_up_mask,
                frame.down_mask,
                *resolved_now_ticks,
            )
            .map_err(|error| format!("coordinator missed Down commit failure: {error}"))?;
    }
    let post_cleanup_timeline = post_cleanup_timeline_ticks(cleanup_completed_qpc)
        .map_err(|error| format!("post-cleanup timeline ticks conversion failure: {error}"))?;
    let cancelled = coordinator
        .cancel_live_generations_after_verified_resumable_cleanup_with_time(post_cleanup_timeline)
        .map_err(|error| format!("coordinator live cancellation failed: {error}"))?;
    runtime.invalidate_down_authorization();
    runtime.pending_up_recovery = None;
    Ok(cancelled)
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

#[cfg(any(test, feature = "test-support"))]
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

#[cfg(test)]
mod tests {
    use super::{TrackedKeyState, clean_completion_proven, clean_generation_accounting_proven};
    use sky_dispatch_core::compile::compile_runtime_intents;
    use sky_dispatch_core::coordinator::{GenerationAccounting, RuntimeDispatchCoordinator};
    use sky_dispatch_core::model::{ActionKind, KeyActionInput};
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};

    fn one_generation_coordinator() -> RuntimeDispatchCoordinator {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "clean-completion-down".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 100,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "clean-completion-up".into(),
                },
            ],
            &[0x15],
        )
        .expect("valid clean-completion schedule");
        RuntimeDispatchCoordinator::try_new_ticks(
            schedule,
            0,
            DurationTicks::ZERO,
            |microseconds| Ok(TimelineTicks::from_raw(microseconds)),
        )
        .expect("valid clean-completion coordinator")
    }

    fn one_released_generation_coordinator() -> RuntimeDispatchCoordinator {
        let mut coordinator = one_generation_coordinator();
        for now in [TimelineTicks::ZERO, TimelineTicks::from_raw(100)] {
            let prepared = coordinator
                .prepare_next_due_authored(now)
                .expect("prepare authored packet")
                .expect("authored packet is due");
            coordinator
                .commit_packet_success(prepared, now, now)
                .expect("commit successful authored packet");
        }
        coordinator
    }

    fn healthy_generation_accounting() -> GenerationAccounting {
        GenerationAccounting {
            total: 1,
            activated: 1,
            scheduled: 0,
            active: 0,
            released: 1,
            dropped_conflict: 0,
            dropped_backend: 0,
            dropped_expired: 0,
            cancelled: 0,
        }
    }

    #[test]
    fn clean_completion_rejects_dropped_expired_even_when_additive_count_matches() {
        let mut coordinator = one_generation_coordinator();
        let down = coordinator
            .schedule
            .try_materialize_batch_authored(0)
            .expect("materialize authored Down");
        coordinator
            .drop_expired_downs(&down.intents)
            .expect("record zero-attempt expired Down");

        let accounting = coordinator.generation_accounting();
        assert_eq!(accounting.total, 1);
        assert_eq!(accounting.activated, 0);
        assert_eq!(accounting.released, 0);
        assert_eq!(accounting.dropped_expired, 1);
        assert_eq!(
            accounting.released + accounting.dropped_expired,
            accounting.total,
            "the former additive shortcut still matches the total"
        );
        assert!(!clean_completion_proven(
            coordinator.generation_accounting(),
            &TrackedKeyState::new()
        ));
    }

    #[test]
    fn clean_generation_accounting_requires_every_generation_to_be_activated_and_released() {
        let healthy = healthy_generation_accounting();
        assert!(clean_generation_accounting_proven(healthy));

        let mut dropped_expired = healthy;
        dropped_expired.activated = 0;
        dropped_expired.released = 0;
        dropped_expired.dropped_expired = 1;

        let mut dropped_conflict = dropped_expired;
        dropped_conflict.dropped_expired = 0;
        dropped_conflict.dropped_conflict = 1;

        let mut dropped_backend = dropped_expired;
        dropped_backend.dropped_expired = 0;
        dropped_backend.dropped_backend = 1;

        let mut cancelled = dropped_expired;
        cancelled.dropped_expired = 0;
        cancelled.cancelled = 1;

        let mut scheduled = dropped_expired;
        scheduled.dropped_expired = 0;
        scheduled.scheduled = 1;

        let mut active = healthy;
        active.released = 0;
        active.active = 1;

        let activated_shortfall = GenerationAccounting {
            total: 2,
            activated: 1,
            released: 1,
            ..GenerationAccounting::default()
        };

        let mut released_shortfall = healthy;
        released_shortfall.released = 0;

        for (label, accounting) in [
            ("DroppedExpired", dropped_expired),
            ("DroppedConflict", dropped_conflict),
            ("DroppedBackend", dropped_backend),
            ("Cancelled", cancelled),
            ("Scheduled", scheduled),
            ("Active", active),
            ("activated < total", activated_shortfall),
            ("released != total", released_shortfall),
        ] {
            assert!(
                !clean_generation_accounting_proven(accounting),
                "{label} must not prove clean natural completion"
            );
        }
    }

    #[test]
    fn clean_completion_rejects_each_backend_residue_and_transport_anomaly() {
        let coordinator = one_released_generation_coordinator();
        assert!(clean_completion_proven(
            coordinator.generation_accounting(),
            &TrackedKeyState::new()
        ));

        for residue in [
            "active",
            "possibly_active",
            "failed_release",
            "in_flight",
            "keys_dropped",
            "chord_split_events",
            "sendinput_partial_events",
            "sendinput_zero_progress_failures",
            "authored_keys_rejected",
        ] {
            let mut backend = TrackedKeyState::new();
            match residue {
                "active" => backend.active_mask = 1,
                "possibly_active" => backend.possibly_active_mask = 1,
                "failed_release" => backend.failed_release_mask = 1,
                "in_flight" => backend.in_flight_mask = 1,
                "keys_dropped" => backend.keys_dropped = 1,
                "chord_split_events" => backend.chord_split_events = 1,
                "sendinput_partial_events" => backend.sendinput_partial_events = 1,
                "sendinput_zero_progress_failures" => backend.sendinput_zero_progress_failures = 1,
                "authored_keys_rejected" => backend.authored_keys_rejected = 1,
                _ => unreachable!("residue cases are listed explicitly"),
            }
            assert!(
                !clean_completion_proven(coordinator.generation_accounting(), &backend),
                "{residue} must prevent clean natural completion"
            );
        }
    }
}
