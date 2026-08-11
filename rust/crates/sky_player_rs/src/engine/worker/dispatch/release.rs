use super::super::super::{
    DurationTicks, PlaybackClockState, QpcClock, QpcTicks, RuntimeDispatchCoordinator,
    TimelineTicks, TrackedKeyState,
};
use super::super::{
    FinalControlAdmission, FinalControlSignals, WorkerConfig, WorkerHealthState,
    WorkerMetricsLocal, WorkerResources, WorkerRuntime, WorkerTimingState,
    cancel_coordinator_or_terminal, describe_release_outcome, final_control_admission_at,
    final_control_precheck, record_termination_error, release_state_verified, signed_ticks_to_us,
    signed_timeline_delta_ticks,
};
use super::DispatchStep;
use super::observation::{DispatchObservation, UpObservation, UpTraceObservation};
use super::observer::{PendingObservationQueue, take_deadline_wake_qpc};
use super::timing::DispatchObservationEvidence;
use sky_dispatch_core::coordinator::PendingRelease;
use sky_dispatch_win32::input::PhysicalPacket;
use smallvec::SmallVec;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

pub(crate) struct PendingReleaseContext<'a> {
    pub(crate) due_pending: SmallVec<[PendingRelease; 15]>,
    pub(crate) physical_target_qpc: QpcTicks,
    pub(crate) frozen_budget: crate::engine::worker::health::FrozenDispatchBudget,
    pub(crate) quit_requested: &'a AtomicBool,
    pub(crate) skip_requested: &'a AtomicBool,
    pub(crate) panic_requested: &'a AtomicBool,
    pub(crate) desired_pause: &'a AtomicBool,
    pub(crate) supervisor_heartbeat_ticks: &'a AtomicU64,
    pub(crate) lease_timeout_ticks: DurationTicks,
    pub(crate) observer: &'a mut PendingObservationQueue,
}

/// Evidence captured from the note-off SendInput call plus the timeline
/// projections used by downstream reconciliation.
pub(super) struct ReleaseSend {
    pub(super) actual_ticks: TimelineTicks,
    pub(super) completed_effective_ticks: TimelineTicks,
    /// §8.6 typed QPC completion boundary used by the deferred observer to
    /// derive `core_post_send_us`.  Replaces the old mixed us/QPC subtraction.
    pub(super) sender_started_qpc: QpcTicks,
    pub(super) sender_completed_qpc: QpcTicks,
    pub(super) sender_started_effective_ticks: Option<TimelineTicks>,
    pub(super) last_win32_error: Option<u32>,
    pub(super) sender_duration_ticks: DurationTicks,
    pub(super) attempts: u8,
    pub(super) retry_reason: sky_dispatch_win32::input::PacketRetryReason,
    pub(super) transport: ReleaseTransportEvidence,
}

/// Independent transport-evidence dimension for a pending release batch.
///
/// `confirmed` is the only acknowledged physical release; `skipped` is *not*
/// physical release confirmation, and `unresolved` deliberately does not
/// subtract `skipped`. The coordinator owns every bit until it is confirmed
/// (or recovery is forced), so a skipped bit is state disagreement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ReleaseTransportEvidence {
    requested_mask: u16,
    confirmed_mask: u16,
    skipped_mask: u16,
    unresolved_mask: u16,
    status: sky_dispatch_win32::input::SendTransactionStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReleaseEvidenceError {
    ConfirmedNotSubsetOfRequested,
    SkippedNotSubsetOfRequested,
    ConfirmedAndSkippedOverlap,
}

impl std::fmt::Display for ReleaseEvidenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfirmedNotSubsetOfRequested => {
                write!(
                    f,
                    "confirmed transport mask is not a subset of the requested mask"
                )
            }
            Self::SkippedNotSubsetOfRequested => {
                write!(
                    f,
                    "skipped transport mask is not a subset of the requested mask"
                )
            }
            Self::ConfirmedAndSkippedOverlap => {
                write!(f, "confirmed and skipped transport masks overlap")
            }
        }
    }
}

impl ReleaseTransportEvidence {
    fn from_outcome(
        outcome: &sky_dispatch_win32::input::SendTransactionOutcome,
    ) -> Result<Self, ReleaseEvidenceError> {
        let requested_mask = outcome.evidence.requested_mask;
        let confirmed_mask = outcome.evidence.confirmed_mask;
        let skipped_mask = outcome.evidence.skipped_mask;
        if confirmed_mask & !requested_mask != 0 {
            return Err(ReleaseEvidenceError::ConfirmedNotSubsetOfRequested);
        }
        if skipped_mask & !requested_mask != 0 {
            return Err(ReleaseEvidenceError::SkippedNotSubsetOfRequested);
        }
        if confirmed_mask & skipped_mask != 0 {
            return Err(ReleaseEvidenceError::ConfirmedAndSkippedOverlap);
        }
        let unresolved_mask = requested_mask & !confirmed_mask;
        Ok(Self {
            requested_mask,
            confirmed_mask,
            skipped_mask,
            unresolved_mask,
            status: outcome.status,
        })
    }
}

/// Per-event timing reconciliation derived from the pending release batch
/// and the SendInput note-off evidence.
pub(super) struct ReleaseReconciliation {
    pub(super) recovery_required: bool,
    pub(super) recovery_pause_ticks: Option<DurationTicks>,
    pub(super) first_index: usize,
    pub(super) effective_deadline_ticks: TimelineTicks,
    pub(super) scheduled_ticks: TimelineTicks,
    pub(super) deferred_ticks: DurationTicks,
    pub(super) up_completion_lateness_ticks: Option<DurationTicks>,
    pub(super) up_completion_error_ticks: i64,
    pub(super) up_authored_completion_error_ticks: i64,
    pub(super) dispatch_start_error_ticks: i64,
    pub(super) observation_evidence: DispatchObservationEvidence,
}

/// Strict/SLO flags computed after the health observation stage; the release
/// orchestrator uses them for terminal decisions.
#[derive(Clone, Copy)]
pub(super) struct ReleaseOutcomeFlags {
    pub(super) strict_up_completion_late: bool,
}

struct ReleaseRecoveryOutcome {
    terminal_error: String,
}

#[cfg(any(test, feature = "test-support"))]
static RELEASE_READY_REACHED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(any(test, feature = "test-support"))]
static RELEASE_RECOVERY_COMPLETED_BEFORE_READY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(any(test, feature = "test-support"))]
static RELEASE_TELEMETRY_FAILURE_ON_RECOVERY: AtomicBool = AtomicBool::new(false);
#[cfg(any(test, feature = "test-support"))]
static RELEASE_OBSERVER_FAILURE_ON_RECOVERY: AtomicBool = AtomicBool::new(false);

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn reset_release_order_test_hook() {
    RELEASE_READY_REACHED.store(false, Ordering::SeqCst);
    RELEASE_RECOVERY_COMPLETED_BEFORE_READY.store(false, Ordering::SeqCst);
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn reset_release_test_hooks() {
    reset_release_order_test_hook();
    RELEASE_TELEMETRY_FAILURE_ON_RECOVERY.store(false, Ordering::SeqCst);
    RELEASE_OBSERVER_FAILURE_ON_RECOVERY.store(false, Ordering::SeqCst);
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn set_release_telemetry_failure_on_recovery(enabled: bool) {
    RELEASE_TELEMETRY_FAILURE_ON_RECOVERY.store(enabled, Ordering::SeqCst);
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn set_release_observer_failure_on_recovery(enabled: bool) {
    RELEASE_OBSERVER_FAILURE_ON_RECOVERY.store(enabled, Ordering::SeqCst);
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn take_release_telemetry_failure(recovery_required: bool) -> bool {
    recovery_required && RELEASE_TELEMETRY_FAILURE_ON_RECOVERY.swap(false, Ordering::SeqCst)
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn take_release_observer_failure(recovery_required: bool) -> bool {
    recovery_required && RELEASE_OBSERVER_FAILURE_ON_RECOVERY.swap(false, Ordering::SeqCst)
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn release_recovery_completed_before_ready() -> bool {
    RELEASE_RECOVERY_COMPLETED_BEFORE_READY.load(Ordering::SeqCst)
}

#[cfg(any(test, feature = "test-support"))]
fn mark_release_recovery_complete() {
    if !RELEASE_READY_REACHED.load(Ordering::SeqCst) {
        RELEASE_RECOVERY_COMPLETED_BEFORE_READY.store(true, Ordering::SeqCst);
    }
}

#[cfg(any(test, feature = "test-support"))]
fn mark_release_ready() {
    RELEASE_READY_REACHED.store(true, Ordering::SeqCst);
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_due_pending_releases(
    ctx: PendingReleaseContext<'_>,
    config: &WorkerConfig,
    resources: &mut WorkerResources,
    _health: &mut WorkerHealthState,
    timing: &WorkerTimingState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    secondary_errors: &mut Vec<String>,
    target_hwnd: &AtomicIsize,
) -> DispatchStep {
    #[cfg(any(test, feature = "test-support"))]
    RELEASE_READY_REACHED.store(false, Ordering::SeqCst);
    let PendingReleaseContext {
        due_pending,
        physical_target_qpc,
        frozen_budget,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        supervisor_heartbeat_ticks,
        lease_timeout_ticks,
        observer,
    } = ctx;
    let WorkerResources {
        clock: qpc_clock,
        backend,
        coordinator,
        playback: clock_state,
        ..
    } = resources;
    let qpc_clock = *qpc_clock;
    let release_mask = due_pending
        .iter()
        .fold(0u16, |mask, pending| mask | (1u16 << pending.key_slot));
    let send = match prepare_release_send(
        qpc_clock,
        backend,
        clock_state,
        runtime,
        release_mask,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        supervisor_heartbeat_ticks,
        lease_timeout_ticks,
    ) {
        Ok(value) => value,
        Err(step) => return step,
    };

    // Fail-closed: backend skipping a key the coordinator still owns is state
    // disagreement. It is not acknowledged release and never unblocks a
    // same-key Down; force full-instrument cleanup and terminate.
    if send.transport.skipped_mask != 0 {
        runtime.force_full_cleanup = true;
        return DispatchStep::Terminate(
            "release transport/coordinator state disagreement".to_string(),
        );
    }

    let reconciliation = match reconcile_release_recovery(
        coordinator,
        qpc_clock,
        clock_state,
        timing,
        &due_pending,
        &send,
        physical_target_qpc,
    ) {
        Ok(value) => value,
        Err(step) => return step,
    };

    // Recovery is correctness-critical ownership, not observer bookkeeping.
    // Full-instrument release, physical verification, and coordinator
    // cancellation must finish before the hard ready boundary is sampled.
    let recovery_outcome = if reconciliation.recovery_required {
        Some(finalize_release_recovery(
            backend,
            coordinator,
            runtime,
            secondary_errors,
            target_hwnd,
            &send,
        ))
    } else {
        None
    };

    let flags = ReleaseOutcomeFlags {
        strict_up_completion_late: config.timing.strict_timing
            && super::timing::is_clean_dispatch_observation(reconciliation.observation_evidence)
            && reconciliation
                .up_completion_lateness_ticks
                .is_some_and(|late| late > timing.strict_up_completion_late_ticks),
    };
    let dispatch_ready_qpc = if config.timing.strict_timing
        || !matches!(config.telemetry.mode, super::super::TelemetryMode::Off)
    {
        match qpc_clock.now() {
            Ok(ticks) => Some(ticks),
            Err(error) => {
                return DispatchStep::Terminate(format!(
                    "note-off worker-ready QPC failure: {error:?}"
                ));
            }
        }
    } else {
        None
    };
    let wake_qpc = take_deadline_wake_qpc(runtime, send.sender_started_qpc);
    // HARD DISPATCH READY BOUNDARY:
    // physical/coordinator ownership is safe for the next dispatch.  From
    // here on, only a fixed raw observation enqueue and terminal policy may
    // run on this call stack.
    let observation = UpObservation {
        physical_target_qpc,
        sender_started_qpc: send.sender_started_qpc,
        sender_completed_qpc: send.sender_completed_qpc,
        dispatch_ready_qpc,
        sender_duration_ticks: send.sender_duration_ticks,
        wake_qpc,
        requested_mask: send.transport.requested_mask,
        confirmed_mask: send.transport.confirmed_mask,
        skipped_mask: send.transport.skipped_mask,
        result_status: send.transport.status,
        completed_effective_ticks: send.completed_effective_ticks,
        scheduled_ticks: reconciliation.scheduled_ticks,
        deferred_ticks: reconciliation.deferred_ticks,
        up_completion_error_ticks: reconciliation.up_completion_error_ticks,
        send_warn_us: frozen_budget.send_warn_us,
        core_post_send_warn_us: frozen_budget.core_post_send_warn_us,
        recovery_pause_ticks: reconciliation.recovery_pause_ticks,
        trace: UpTraceObservation {
            event_index: due_pending[reconciliation.first_index].source_action_index,
            trace_kind: super::super::TRACE_KIND_UP,
            send_attempts: send.attempts,
            retry_reason: send.retry_reason,
            last_win32_error: send.last_win32_error.unwrap_or(0),
            authored_ticks: reconciliation.scheduled_ticks,
            effective_deadline_ticks: reconciliation.effective_deadline_ticks,
            wake_ticks: send.actual_ticks,
            sender_started_ticks: send.sender_started_effective_ticks,
            sender_completed_ticks: Some(send.completed_effective_ticks),
            dispatch_start_error_ticks: reconciliation.dispatch_start_error_ticks,
            completion_error_ticks: reconciliation.up_completion_error_ticks,
            authored_completion_error_ticks: reconciliation.up_authored_completion_error_ticks,
            deferred_ticks: reconciliation.deferred_ticks,
            recovery_required: reconciliation.recovery_required,
        },
    };
    #[cfg(any(test, feature = "test-support"))]
    mark_release_ready();
    observer.push(
        DispatchObservation::Up(observation),
        &mut local_metrics.observer_dropped_samples,
        &mut local_metrics.observer_queue_high_watermark,
    );

    release_terminal_step(
        recovery_outcome,
        flags,
        due_pending[reconciliation.first_index].source_action_index,
        qpc_clock,
        reconciliation.up_completion_error_ticks,
    )
}

fn release_terminal_step(
    recovery_outcome: Option<ReleaseRecoveryOutcome>,
    flags: ReleaseOutcomeFlags,
    first_action_index: u32,
    qpc_clock: QpcClock,
    completion_error_ticks: i64,
) -> DispatchStep {
    if let Some(outcome) = recovery_outcome {
        return DispatchStep::Terminate(outcome.terminal_error);
    }
    if flags.strict_up_completion_late {
        let completion_error_us = match signed_ticks_to_us(qpc_clock, completion_error_ticks) {
            Ok(value) => value,
            Err(error) => {
                return DispatchStep::Terminate(format!(
                    "note-off terminal timing conversion failure: {error}"
                ));
            }
        };
        return DispatchStep::Terminate(format!(
            "strict timing completion SLO exceeded for note-off at action {first_action_index}: completion was {completion_error_us}us late"
        ));
    }
    DispatchStep::Dispatched
}

#[allow(clippy::too_many_arguments)]
fn prepare_release_send(
    qpc_clock: QpcClock,
    backend: &mut TrackedKeyState,
    clock_state: &mut PlaybackClockState,
    runtime: &mut WorkerRuntime,
    release_mask: u16,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
    supervisor_heartbeat_ticks: &AtomicU64,
    lease_timeout_ticks: DurationTicks,
) -> Result<ReleaseSend, DispatchStep> {
    let control_signals = FinalControlSignals {
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        supervisor_heartbeat_ticks,
    };
    let admission = final_control_precheck(FinalControlSignals {
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        supervisor_heartbeat_ticks,
    });
    if !matches!(admission, FinalControlAdmission::Allowed) {
        return Err(DispatchStep::Continue);
    }
    let started_ticks = qpc_clock.now().map_err(|error| {
        DispatchStep::Terminate(format!("release QPC start failure: {error:?}"))
    })?;
    if !matches!(
        final_control_admission_at(started_ticks, lease_timeout_ticks, control_signals).map_err(
            |error| DispatchStep::Terminate(format!("release lease failure: {error:?}"))
        )?,
        FinalControlAdmission::Allowed
    ) {
        return Err(DispatchStep::Continue);
    }
    let actual_ticks = match clock_state
        .get_elapsed_allow_pre_epoch(started_ticks, runtime.allow_pre_epoch_startup_dispatch)
    {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "playback clock failure: {error}"
            )));
        }
    };
    let result = backend
        .send_physical_packet_with_start(PhysicalPacket::new(release_mask, 0), started_ticks);
    if let Some(error) = backend.timing_error.take() {
        return Err(DispatchStep::Terminate(format!(
            "QPC failure after note-off: {error:?}"
        )));
    }
    let completed_qpc_ticks = match result.evidence.completed_ticks {
        Some(ticks) => ticks,
        None => {
            return Err(DispatchStep::Terminate(
                "SendInput note-off completed without a QPC completion boundary".to_string(),
            ));
        }
    };
    let completed_effective_ticks = match clock_state.get_elapsed_allow_pre_epoch(
        completed_qpc_ticks,
        runtime.allow_pre_epoch_startup_dispatch,
    ) {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "playback clock failure: {error}"
            )));
        }
    };
    let sender_started_effective_ticks = match result.evidence.started_ticks {
        Some(ticks) => match clock_state
            .get_elapsed_allow_pre_epoch(ticks, runtime.allow_pre_epoch_startup_dispatch)
        {
            Ok(value) => Some(value),
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "playback clock failure: {error}"
                )));
            }
        },
        None => None,
    };
    let sender_duration_ticks = match result.evidence.duration_ticks() {
        Ok(dur) => dur,
        Err(_) => match completed_qpc_ticks.checked_duration_since(started_ticks) {
            Ok(dur) => dur,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "note-off QPC duration failure: {error:?}"
                )));
            }
        },
    };
    let last_win32_error = result.evidence.last_win32_error;
    let attempts = result.evidence.attempts;
    let retry_reason = result.evidence.retry_reason;
    let transport = ReleaseTransportEvidence::from_outcome(&result).map_err(|error| {
        DispatchStep::Terminate(format!(
            "release transport evidence validation failure: {error}"
        ))
    })?;
    Ok(ReleaseSend {
        actual_ticks,
        completed_effective_ticks,
        sender_completed_qpc: completed_qpc_ticks,
        sender_started_qpc: result.evidence.started_ticks.unwrap_or(QpcTicks::ZERO),
        sender_started_effective_ticks,
        last_win32_error,
        sender_duration_ticks,
        attempts,
        retry_reason,
        transport,
    })
}

#[allow(clippy::too_many_arguments)]
fn reconcile_release_recovery(
    coordinator: &mut RuntimeDispatchCoordinator,
    _qpc_clock: QpcClock,
    clock_state: &mut PlaybackClockState,
    timing: &WorkerTimingState,
    due_pending: &SmallVec<[PendingRelease; 15]>,
    send: &ReleaseSend,
    physical_target_qpc: QpcTicks,
) -> Result<ReleaseReconciliation, DispatchStep> {
    // The caller has already fail-closed on any backend-skipped disagreement
    // (`force_full_cleanup` + terminate). Confirmed-only reconciliation below
    // completes exactly the confirmed keys; unconfirmed keys stay owned by the
    // coordinator and are requeued.
    let confirmed_mask = send.transport.confirmed_mask;
    let recovery_required = match coordinator.requeue_unconfirmed_releases_ticks(
        due_pending,
        confirmed_mask,
        send.actual_ticks,
        send.completed_effective_ticks,
        &timing.retry_backoff_ticks,
        send.last_win32_error,
    ) {
        Ok(required) => required,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "coordinator recovery failure: {error}"
            )));
        }
    };
    if let Err(error) = coordinator.complete_releases_mask(due_pending, confirmed_mask) {
        return Err(DispatchStep::Terminate(format!(
            "coordinator release completion failure: {error}"
        )));
    }
    let mut recovery_pause_ticks = None;
    if !recovery_required {
        match coordinator.finish_release_recovery_ticks(send.completed_effective_ticks) {
            Ok(pause_ticks) => {
                recovery_pause_ticks = pause_ticks;
            }
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "coordinator recovery completion failure: {error}"
                )));
            }
        }
    }
    reconcile_release_outcome(
        _qpc_clock,
        clock_state,
        due_pending,
        send,
        physical_target_qpc,
        recovery_required,
        recovery_pause_ticks,
    )
}

#[allow(clippy::too_many_arguments)]
fn reconcile_release_outcome(
    _qpc_clock: QpcClock,
    _clock_state: &mut PlaybackClockState,
    due_pending: &SmallVec<[PendingRelease; 15]>,
    send: &ReleaseSend,
    physical_target_qpc: QpcTicks,
    recovery_required: bool,
    recovery_pause_ticks: Option<DurationTicks>,
) -> Result<ReleaseReconciliation, DispatchStep> {
    let mut first_index: Option<usize> = None;
    let mut first_deadline: Option<TimelineTicks> = None;
    for (index, pending) in due_pending.iter().enumerate() {
        let deadline = match pending.get_effective_release_ticks() {
            Ok(deadline) => deadline,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "pending release deadline failure: {error}"
                )));
            }
        };
        let is_better = match first_index {
            None => true,
            Some(best_index) => match first_deadline {
                Some(best) => {
                    (deadline, pending.source_action_index, pending.scan_code)
                        < (
                            best,
                            due_pending[best_index].source_action_index,
                            due_pending[best_index].scan_code,
                        )
                }
                None => {
                    return Err(DispatchStep::Terminate(
                        "pending release first-deadline state is inconsistent".to_string(),
                    ));
                }
            },
        };
        if is_better {
            first_index = Some(index);
            first_deadline = Some(deadline);
        }
    }
    let Some(first_index) = first_index else {
        return Err(DispatchStep::Terminate(
            "coordinator returned an empty pending release batch".to_string(),
        ));
    };
    let Some(effective_deadline_ticks) = first_deadline else {
        return Err(DispatchStep::Terminate(
            "coordinator returned no release deadline".to_string(),
        ));
    };
    let Some(scheduled_ticks) = due_pending
        .iter()
        .map(|pending| pending.scheduled_release_ticks)
        .min()
    else {
        return Err(DispatchStep::Terminate(
            "pending release batch has no scheduled timestamp".to_string(),
        ));
    };
    let mut deferred_ticks = DurationTicks::ZERO;
    for pending in due_pending {
        let ready_ticks = pending
            .release_not_before_ticks
            .max(pending.next_retry_ticks);
        let pending_deferred_ticks = match ready_ticks
            .checked_duration_since(pending.scheduled_release_ticks)
        {
            Ok(value) => value,
            Err(sky_dispatch_core::time::TimeArithmeticError::NegativeOrder) => DurationTicks::ZERO,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "pending deferral arithmetic failure: {error}"
                )));
            }
        };
        deferred_ticks = deferred_ticks.max(pending_deferred_ticks);
    }
    let up_completion_lateness_ticks = send
        .completed_effective_ticks
        .checked_duration_since(scheduled_ticks)
        .ok();
    let up_completion_error_ticks =
        match signed_timeline_delta_ticks(send.completed_effective_ticks, effective_deadline_ticks)
        {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "note-off timing conversion failure: {error}"
                )));
            }
        };
    let up_authored_completion_error_ticks =
        match signed_timeline_delta_ticks(send.completed_effective_ticks, scheduled_ticks) {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "note-off authored timing conversion failure: {error}"
                )));
            }
        };
    let dispatch_start_error_ticks = signed_timeline_delta_ticks(
        TimelineTicks::from_raw(send.sender_started_qpc.as_u64()),
        TimelineTicks::from_raw(physical_target_qpc.as_u64()),
    )
    .map_err(|error| {
        DispatchStep::Terminate(format!(
            "note-off dispatch-start timing conversion failure: {error}"
        ))
    })?;
    let observation_evidence = DispatchObservationEvidence {
        status: send.transport.status,
        attempts: send.attempts,
        retry_reason: send.retry_reason,
        requested_count: send.transport.requested_mask.count_ones() as usize,
        confirmed_count: send.transport.confirmed_mask.count_ones() as usize,
        skipped_count: send.transport.skipped_mask.count_ones() as usize,
        timing_valid: true,
        transport_anomaly: send.last_win32_error.is_some()
            || send.transport.status != sky_dispatch_win32::input::SendTransactionStatus::Complete,
        recovery_used: recovery_required || deferred_ticks > DurationTicks::ZERO,
        chord_integrity_lost: false,
    };
    Ok(ReleaseReconciliation {
        recovery_required,
        recovery_pause_ticks,
        first_index,
        effective_deadline_ticks,
        scheduled_ticks,
        deferred_ticks,
        up_completion_lateness_ticks,
        up_completion_error_ticks,
        up_authored_completion_error_ticks,
        dispatch_start_error_ticks,
        observation_evidence,
    })
}

fn finalize_release_recovery(
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    runtime: &mut WorkerRuntime,
    secondary_errors: &mut Vec<String>,
    target_hwnd: &AtomicIsize,
    send: &ReleaseSend,
) -> ReleaseRecoveryOutcome {
    runtime.verified_target = None;
    let mut term_err = Some(format!(
        "note-off recovery exhausted after {} retries{}",
        sky_dispatch_core::coordinator::MAX_RELEASE_RETRIES,
        send.last_win32_error
            .map_or(String::new(), |error| format!(" (Win32 error {error})"))
    ));
    let recovery_cleanup = backend.release_all_full_instrument(target_hwnd.load(Ordering::Acquire));
    if !release_state_verified(backend, &recovery_cleanup) {
        record_termination_error(
            &mut term_err,
            secondary_errors,
            format!(
                "recovery cleanup release verification failed: {}",
                describe_release_outcome(&recovery_cleanup)
            ),
        );
    }
    cancel_coordinator_or_terminal(
        coordinator,
        &mut runtime.force_full_cleanup,
        &mut term_err,
        secondary_errors,
    );
    #[cfg(any(test, feature = "test-support"))]
    mark_release_recovery_complete();
    ReleaseRecoveryOutcome {
        terminal_error: term_err.unwrap_or_else(|| "recovery failure".to_string()),
    }
}

#[cfg(test)]
pub(crate) fn effective_pending_lead(
    pending: &PendingRelease,
    requested_lead_ticks: DurationTicks,
    requested_lead_saturated: bool,
    effective_deadline_ticks: TimelineTicks,
) -> (DurationTicks, bool) {
    let _ = (
        pending,
        requested_lead_ticks,
        requested_lead_saturated,
        effective_deadline_ticks,
    );
    (DurationTicks::ZERO, false)
}

#[cfg(test)]
pub(crate) fn effective_pending_cohort_lead(
    due_pending: &SmallVec<[PendingRelease; 15]>,
    requested_lead_ticks: DurationTicks,
    requested_lead_saturated: bool,
    effective_deadline_ticks: TimelineTicks,
) -> (DurationTicks, bool) {
    let _ = (
        due_pending,
        requested_lead_ticks,
        requested_lead_saturated,
        effective_deadline_ticks,
    );
    (DurationTicks::ZERO, false)
}
