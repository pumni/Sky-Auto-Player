use super::super::super::{
    DurationTicks, LatencyClass, PlaybackClockState, QpcClock, QpcTicks,
    RuntimeDispatchCoordinator, STRICT_SATURATION_ABORT_STREAK, TimelineTicks, TrackedKeyState,
};
use super::super::{
    DispatchPath, WorkerConfig, WorkerHealthState, WorkerMetricsLocal, WorkerResources,
    WorkerRuntime, WorkerTimingState, build_dispatch_budget, cancel_coordinator_or_terminal,
    describe_release_outcome, record_termination_error, release_state_verified, signed_ticks_to_us,
    signed_timeline_delta_ticks,
};
use super::DispatchStep;
use super::observer::PendingObservationQueue;
use super::observer::{observe_release_send_health, record_release_telemetry};
use super::timing::EstimatorObservationEvidence;
use sky_dispatch_core::coordinator::{PendingDispatchPlan, PendingRelease};
use smallvec::SmallVec;
use std::sync::atomic::{AtomicIsize, Ordering};

pub(crate) struct PendingReleaseContext<'a> {
    pub(crate) due_pending: SmallVec<[PendingRelease; 15]>,
    pub(crate) pending_plan: Option<&'a PendingDispatchPlan>,
    pub(crate) lead_up_ticks: DurationTicks,
    pub(crate) lead_up: u64,
    pub(crate) latency_class: LatencyClass,
    pub(crate) observer: &'a mut PendingObservationQueue,
}

/// Evidence captured from the note-off SendInput call plus the timeline
/// projections used by downstream reconciliation.
pub(super) struct ReleaseSend {
    pub(super) actual_ticks: TimelineTicks,
    pub(super) completed_effective_ticks: TimelineTicks,
    pub(super) completed_effective_us: u64,
    /// §8.6 typed QPC completion boundary used by the deferred observer to
    /// derive `core_post_send_us`.  Replaces the old mixed us/QPC subtraction.
    pub(super) sender_completed_qpc: QpcTicks,
    pub(super) sender_started_effective_ticks: Option<TimelineTicks>,
    pub(super) last_win32_error: Option<u32>,
    pub(super) sender_duration_us: u64,
    pub(super) sent_count: usize,
    pub(super) skipped_count: usize,
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
    pub(super) scheduled_us: u64,
    pub(super) deferred_by_us: u64,
    pub(super) up_completion_lateness_ticks: Option<DurationTicks>,
    pub(super) up_completion_error_ticks: i64,
    pub(super) up_authored_completion_error_ticks: i64,
    pub(super) up_completion_error_us: i64,
    pub(super) estimator_evidence: EstimatorObservationEvidence,
}

/// Strict/SLO flags computed after the health observation stage; the release
/// orchestrator uses them for terminal decisions.
pub(super) struct ReleaseOutcomeFlags {
    pub(super) strict_up_completion_late: bool,
    pub(super) saturation_abort: bool,
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
pub(crate) fn reset_release_order_test_hook() {
    RELEASE_READY_REACHED.store(false, Ordering::SeqCst);
    RELEASE_RECOVERY_COMPLETED_BEFORE_READY.store(false, Ordering::SeqCst);
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
    health: &mut WorkerHealthState,
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
        pending_plan,
        lead_up_ticks,
        lead_up,
        latency_class,
        observer,
    } = ctx;

    let WorkerResources {
        clock: qpc_clock,
        backend,
        coordinator,
        playback: clock_state,
        estimator,
        telemetry,
        ..
    } = resources;
    let qpc_clock = *qpc_clock;

    let scan_codes: SmallVec<[u16; 15]> = due_pending.iter().map(|p| p.scan_code).collect();
    let scan_count = scan_codes.len();
    let frozen_budget = build_dispatch_budget(
        estimator,
        DispatchPath::UpOnly {
            up_count: scan_count,
        },
        latency_class,
        health.options,
        config.timing.strict_timing,
    );

    let send = match prepare_release_send(qpc_clock, backend, clock_state, runtime, &scan_codes) {
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
        lead_up_ticks,
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

    let dispatch_ready_qpc = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return DispatchStep::Terminate(format!(
                "note-off dispatch_ready QPC failure: {error:?}"
            ));
        }
    };
    let core_post_send_us =
        match dispatch_ready_qpc.checked_duration_since(send.sender_completed_qpc) {
            Ok(duration) => match qpc_clock.duration_to_us(duration) {
                Ok(us) => us,
                Err(error) => {
                    return DispatchStep::Terminate(format!(
                        "note-off post-send conversion failure: {error:?}"
                    ));
                }
            },
            Err(error) => {
                return DispatchStep::Terminate(format!(
                    "dispatch-ready QPC ordering failure: {error:?}"
                ));
            }
        };

    // HARD DISPATCH READY BOUNDARY:
    // gameplay-critical dispatch ownership ends here.
    #[cfg(any(test, feature = "test-support"))]
    mark_release_ready();

    if let Some(pause_ticks) = reconciliation.recovery_pause_ticks {
        let recovery_pause_us = match qpc_clock.duration_to_us(pause_ticks) {
            Ok(value) => value,
            Err(error) => {
                return DispatchStep::Terminate(format!(
                    "recovery telemetry conversion failure: {error:?}"
                ));
            }
        };
        local_metrics.total_us = local_metrics.total_us.saturating_add(recovery_pause_us);
    }

    if let Err(step) = record_release_telemetry(
        telemetry,
        &due_pending,
        &send,
        &reconciliation,
        lead_up_ticks,
        core_post_send_us,
    ) {
        return step;
    }

    let flags = match observe_release_send_health(
        qpc_clock,
        config,
        timing,
        health,
        runtime,
        local_metrics,
        observer,
        &send,
        &reconciliation,
        &frozen_budget,
        lead_up,
        latency_class,
        pending_plan,
        scan_count,
        core_post_send_us,
    ) {
        Ok(value) => value,
        Err(step) => return step,
    };

    if let Some(outcome) = recovery_outcome {
        return DispatchStep::Terminate(outcome.terminal_error);
    }
    if flags.strict_up_completion_late {
        let first = &due_pending[reconciliation.first_index];
        return DispatchStep::Terminate(format!(
            "strict timing completion SLO exceeded for note-off at action {}: completion was {}us late",
            first.source_action_index, reconciliation.up_completion_error_us
        ));
    }
    if flags.saturation_abort {
        return DispatchStep::Terminate(format!(
            "strict timing SLO exceeded: note-off lead saturated with positive residual for {} consecutive dispatches",
            STRICT_SATURATION_ABORT_STREAK
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
    scan_codes: &SmallVec<[u16; 15]>,
) -> Result<ReleaseSend, DispatchStep> {
    let started_ticks = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "QPC failure before note-off: {error:?}"
            )));
        }
    };
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
    let result = backend.key_up(scan_codes);
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
    let completed_effective_us = match qpc_clock.duration_to_us(
        match completed_effective_ticks.checked_duration_since(TimelineTicks::ZERO) {
            Ok(dur) => dur,
            Err(_) => DurationTicks::ZERO,
        },
    ) {
        Ok(us) => us,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "playback clock conversion failure: {error:?}"
            )));
        }
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
    let sender_duration_us = match qpc_clock.duration_to_us(sender_duration_ticks) {
        Ok(us) => us,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "note-off duration conversion failure: {error:?}"
            )));
        }
    };
    runtime.last_send_qpc_ticks = Some(completed_qpc_ticks);
    let sent_count = result.sent_scan_codes().len();
    let skipped_count = result.skipped_duplicates().len();
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
        completed_effective_us,
        sender_completed_qpc: completed_qpc_ticks,
        sender_started_effective_ticks,
        last_win32_error,
        sender_duration_us,
        sent_count,
        skipped_count,
        attempts,
        retry_reason,
        transport,
    })
}

#[allow(clippy::too_many_arguments)]
fn reconcile_release_recovery(
    coordinator: &mut RuntimeDispatchCoordinator,
    qpc_clock: QpcClock,
    clock_state: &mut PlaybackClockState,
    timing: &WorkerTimingState,
    due_pending: &SmallVec<[PendingRelease; 15]>,
    send: &ReleaseSend,
    lead_up_ticks: DurationTicks,
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
        qpc_clock,
        clock_state,
        due_pending,
        send,
        lead_up_ticks,
        recovery_required,
        recovery_pause_ticks,
    )
}

#[allow(clippy::too_many_arguments)]
fn reconcile_release_outcome(
    qpc_clock: QpcClock,
    _clock_state: &mut PlaybackClockState,
    due_pending: &SmallVec<[PendingRelease; 15]>,
    send: &ReleaseSend,
    lead_up_ticks: DurationTicks,
    recovery_required: bool,
    recovery_pause_ticks: Option<DurationTicks>,
) -> Result<ReleaseReconciliation, DispatchStep> {
    let mut first_index: Option<usize> = None;
    let mut first_deadline: Option<TimelineTicks> = None;
    for (index, pending) in due_pending.iter().enumerate() {
        let deadline = match pending.get_effective_release_ticks(lead_up_ticks) {
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
    let scheduled_us = match qpc_clock.timeline_to_us(scheduled_ticks) {
        Ok(value) => value,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "pending release telemetry conversion error: {error:?}"
            )));
        }
    };
    let mut deferred_by_us = 0u64;
    for pending in due_pending {
        let ready_ticks = pending
            .release_not_before_ticks
            .max(pending.next_retry_ticks);
        let deferred_ticks = match ready_ticks
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
        let deferred_us = match qpc_clock.duration_to_us(deferred_ticks) {
            Ok(value) => value,
            Err(error) => {
                return Err(DispatchStep::Terminate(format!(
                    "pending deferral conversion failure: {error:?}"
                )));
            }
        };
        deferred_by_us = deferred_by_us.max(deferred_us);
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
    let up_completion_error_us = match signed_ticks_to_us(qpc_clock, up_completion_error_ticks) {
        Ok(value) => value,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "note-off timing conversion failure: {error}"
            )));
        }
    };
    let estimator_evidence = EstimatorObservationEvidence {
        status: send.transport.status,
        attempts: send.attempts,
        retry_reason: send.retry_reason,
        requested_count: send.transport.requested_mask.count_ones() as usize,
        confirmed_count: send.transport.confirmed_mask.count_ones() as usize,
        skipped_count: send.transport.skipped_mask.count_ones() as usize,
        timing_valid: true,
        transport_anomaly: send.last_win32_error.is_some()
            || send.transport.status != sky_dispatch_win32::input::SendTransactionStatus::Complete,
        recovery_used: recovery_required || deferred_by_us > 0,
        chord_integrity_lost: false,
    };
    Ok(ReleaseReconciliation {
        recovery_required,
        recovery_pause_ticks,
        first_index,
        effective_deadline_ticks,
        scheduled_ticks,
        scheduled_us,
        deferred_by_us,
        up_completion_lateness_ticks,
        up_completion_error_ticks,
        up_authored_completion_error_ticks,
        up_completion_error_us,
        estimator_evidence,
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
