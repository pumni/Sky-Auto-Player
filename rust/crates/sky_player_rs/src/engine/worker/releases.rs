use super::super::{
    ActionKind, DurationTicks, LatencyClass, PlaybackClockState, QpcClock, RtTraceRecord,
    RuntimeDispatchCoordinator, STRICT_SATURATION_ABORT_STREAK, SendLatencyEstimator,
    SharedMetrics, TRACE_FLAG_ANOMALY, TRACE_FLAG_DEFERRED, TRACE_FLAG_RECOVERY,
    TRACE_FLAG_SENT_FULL, TRACE_KIND_UP, TelemetryCollector, TimelineTicks, TraceContext,
    TraceDelivery, TraceTiming, TrackedKeyState, trace_outcome_code, try_publish_metrics,
};
use super::{
    DispatchHealthObservation, DispatchPath, DispatchStep, WorkerConfig, WorkerHealthState,
    WorkerMetricsLocal, WorkerResources, WorkerRuntime, WorkerTimingState, build_dispatch_budget,
    cancel_coordinator_or_terminal, describe_release_outcome, observe_dispatch_health,
    record_lateness, record_lead_saturation, record_termination_error, release_runtime_outcome,
    release_state_verified, signed_delta, signed_ticks_to_us, signed_timeline_delta_ticks,
    update_estimator_after_send_class,
};
use sky_dispatch_core::coordinator::{PendingDispatchPlan, PendingRelease};
use smallvec::SmallVec;
use std::sync::atomic::{AtomicIsize, Ordering};

pub(crate) struct PendingReleaseContext<'a> {
    pub(crate) due_pending: SmallVec<[PendingRelease; 15]>,
    pub(crate) pending_plan: Option<&'a PendingDispatchPlan>,
    pub(crate) lead_up_ticks: DurationTicks,
    pub(crate) lead_up: u64,
    pub(crate) latency_class: LatencyClass,
}

/// Evidence captured from the note-off SendInput call plus the timeline
/// projections used by downstream reconciliation.
struct ReleaseSend {
    actual_ticks: TimelineTicks,
    completed_effective_ticks: TimelineTicks,
    completed_effective_us: u64,
    sender_started_effective_ticks: Option<TimelineTicks>,
    last_win32_error: Option<u32>,
    sender_duration_us: u64,
    sent_count: usize,
    skipped_count: usize,
    attempts: u8,
    is_success: bool,
    transport: ReleaseTransportEvidence,
}

/// Independent transport-evidence dimension for a pending release batch.
///
/// `confirmed` is the only acknowledged physical release; `skipped` is *not*
/// physical release confirmation, and `unresolved` deliberately does not
/// subtract `skipped`. The coordinator owns every bit until it is confirmed
/// (or recovery is forced), so a skipped bit is state disagreement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReleaseTransportEvidence {
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
struct ReleaseReconciliation {
    recovery_required: bool,
    bookkeeping_completed_us: u64,
    first_index: usize,
    effective_deadline_ticks: TimelineTicks,
    scheduled_ticks: TimelineTicks,
    scheduled_us: u64,
    deferred_by_us: u64,
    up_completion_lateness_ticks: Option<DurationTicks>,
    up_completion_error_ticks: i64,
    up_authored_completion_error_ticks: i64,
    up_completion_error_us: i64,
    clean_up_sample: bool,
}

/// Strict/SLO flags computed after estimator and health state have observed
/// the send; the orchestrator uses them for terminal decisions.
struct ReleaseOutcomeFlags {
    strict_up_completion_late: bool,
    saturation_abort: bool,
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
    last_published_error: &mut Option<String>,
    target_hwnd: &AtomicIsize,
    metrics: &SharedMetrics,
) -> DispatchStep {
    let PendingReleaseContext {
        due_pending,
        pending_plan,
        lead_up_ticks,
        lead_up,
        latency_class,
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
        local_metrics,
        &due_pending,
        &send,
        lead_up_ticks,
    ) {
        Ok(value) => value,
        Err(step) => return step,
    };

    if let Err(step) = record_release_telemetry(
        telemetry,
        &due_pending,
        &send,
        &reconciliation,
        lead_up_ticks,
    ) {
        return step;
    }

    let flags = match observe_release_send_health(
        qpc_clock,
        clock_state,
        config,
        timing,
        estimator,
        health,
        backend,
        runtime,
        local_metrics,
        metrics,
        last_published_error,
        &send,
        &reconciliation,
        &frozen_budget,
        lead_up,
        latency_class,
        pending_plan,
        scan_count,
    ) {
        Ok(value) => value,
        Err(step) => return step,
    };

    if reconciliation.recovery_required {
        return finalize_release_recovery(
            backend,
            coordinator,
            runtime,
            secondary_errors,
            target_hwnd,
            &send,
        );
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
    let is_success = result.is_success();
    let transport = ReleaseTransportEvidence::from_outcome(&result).map_err(|error| {
        DispatchStep::Terminate(format!(
            "release transport evidence validation failure: {error}"
        ))
    })?;
    Ok(ReleaseSend {
        actual_ticks,
        completed_effective_ticks,
        completed_effective_us,
        sender_started_effective_ticks,
        last_win32_error,
        sender_duration_us,
        sent_count,
        skipped_count,
        attempts,
        is_success,
        transport,
    })
}

#[allow(clippy::too_many_arguments)]
fn reconcile_release_recovery(
    coordinator: &mut RuntimeDispatchCoordinator,
    qpc_clock: QpcClock,
    clock_state: &mut PlaybackClockState,
    timing: &WorkerTimingState,
    local_metrics: &mut WorkerMetricsLocal,
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
    if !recovery_required {
        match coordinator.finish_release_recovery_ticks(send.completed_effective_ticks) {
            Ok(Some(recovery_pause_ticks)) => {
                let recovery_pause_us = match qpc_clock.duration_to_us(recovery_pause_ticks) {
                    Ok(value) => value,
                    Err(error) => {
                        return Err(DispatchStep::Terminate(format!(
                            "recovery telemetry conversion failure: {error:?}"
                        )));
                    }
                };
                local_metrics.total_us = local_metrics.total_us.saturating_add(recovery_pause_us);
            }
            Ok(None) => {}
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
    )
}

#[allow(clippy::too_many_arguments)]
fn reconcile_release_outcome(
    qpc_clock: QpcClock,
    clock_state: &mut PlaybackClockState,
    due_pending: &SmallVec<[PendingRelease; 15]>,
    send: &ReleaseSend,
    lead_up_ticks: DurationTicks,
    recovery_required: bool,
) -> Result<ReleaseReconciliation, DispatchStep> {
    let bookkeeping_completed_us = match qpc_clock.now() {
        Ok(now) => {
            match qpc_clock.duration_to_us(match now.checked_duration_since(clock_state.epoch) {
                Ok(dur) => dur,
                Err(_) => DurationTicks::ZERO,
            }) {
                Ok(us) => us,
                Err(error) => {
                    return Err(DispatchStep::Terminate(format!(
                        "bookkeeping QPC us conversion failure: {error:?}"
                    )));
                }
            }
        }
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "bookkeeping QPC failure: {error:?}"
            )));
        }
    };
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
    let first = &due_pending[first_index];
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
    let mixed_source = due_pending.iter().any(|pending| {
        pending.source_action_index != first.source_action_index
            || pending.reason_id != first.reason_id
    });
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
    let scan_count = due_pending.len();
    let clean_up_sample = send.is_success
        && send.sent_count == scan_count
        && send.skipped_count == 0
        && send.attempts == 1
        && deferred_by_us == 0
        && !mixed_source;
    Ok(ReleaseReconciliation {
        recovery_required,
        bookkeeping_completed_us,
        first_index,
        effective_deadline_ticks,
        scheduled_ticks,
        scheduled_us,
        deferred_by_us,
        up_completion_lateness_ticks,
        up_completion_error_ticks,
        up_authored_completion_error_ticks,
        up_completion_error_us,
        clean_up_sample,
    })
}

#[allow(clippy::too_many_arguments)]
fn record_release_telemetry(
    telemetry: &mut TelemetryCollector,
    due_pending: &SmallVec<[PendingRelease; 15]>,
    send: &ReleaseSend,
    reconciliation: &ReleaseReconciliation,
    lead_up_ticks: DurationTicks,
) -> Result<(), DispatchStep> {
    let first = &due_pending[reconciliation.first_index];
    let scan_count = due_pending.len();
    let release_outcome = release_runtime_outcome(
        reconciliation.deferred_by_us,
        send.sent_count,
        scan_count,
        reconciliation.recovery_required,
    );
    let mut trace_flags = 0u8;
    if send.sent_count == scan_count {
        trace_flags |= TRACE_FLAG_SENT_FULL;
    }
    if release_outcome == "deferred_release" || release_outcome == "failed_note_off" {
        trace_flags |= TRACE_FLAG_RECOVERY;
    }
    if reconciliation.deferred_by_us > 0 {
        trace_flags |= TRACE_FLAG_DEFERRED;
    }
    if release_outcome != "sent" {
        trace_flags |= TRACE_FLAG_ANOMALY;
    }
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: first.source_action_index,
                kind: TRACE_KIND_UP,
                outcome: trace_outcome_code(release_outcome),
                polyphony: scan_count,
                flags: trace_flags,
                win32_error: send.last_win32_error.unwrap_or(0),
            },
            TraceTiming {
                authored_ticks: reconciliation.scheduled_ticks,
                effective_deadline_ticks: reconciliation.effective_deadline_ticks,
                wake_ticks: send.actual_ticks,
                send_started_ticks: send.sender_started_effective_ticks,
                send_completed_ticks: Some(send.completed_effective_ticks),
                bookkeeping_duration_us: reconciliation
                    .bookkeeping_completed_us
                    .saturating_sub(send.completed_effective_us),
                completion_error_ticks: reconciliation.up_completion_error_ticks,
                authored_completion_error_ticks: reconciliation.up_authored_completion_error_ticks,
                applied_lead_ticks: lead_up_ticks,
            },
            TraceDelivery {
                requested: scan_count,
                sent: send.sent_count,
                skipped: send.skipped_count,
                send_attempts: usize::from(send.attempts),
            },
        )
    }) {
        return Err(DispatchStep::Terminate(format!(
            "native telemetry record overflow: {error}"
        )));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn observe_release_send_health(
    qpc_clock: QpcClock,
    clock_state: &mut PlaybackClockState,
    config: &WorkerConfig,
    timing: &WorkerTimingState,
    estimator: &mut SendLatencyEstimator,
    health: &mut WorkerHealthState,
    backend: &TrackedKeyState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    metrics: &SharedMetrics,
    last_published_error: &mut Option<String>,
    send: &ReleaseSend,
    reconciliation: &ReleaseReconciliation,
    frozen_budget: &crate::engine::worker::health::FrozenDispatchBudget,
    lead_up: u64,
    latency_class: LatencyClass,
    pending_plan: Option<&PendingDispatchPlan>,
    scan_count: usize,
) -> Result<ReleaseOutcomeFlags, DispatchStep> {
    let up_saturated_positive = pending_plan.is_some_and(|plan| plan.lead_saturated)
        && reconciliation.up_completion_lateness_ticks.is_some();
    health.up_saturation_positive_streak = if up_saturated_positive {
        health.up_saturation_positive_streak.saturating_add(1)
    } else {
        0
    };
    let saturation_abort = config.timing.strict_timing
        && health.up_saturation_positive_streak >= STRICT_SATURATION_ABORT_STREAK;
    let strict_up_completion_late = config.timing.strict_timing
        && reconciliation.clean_up_sample
        && reconciliation
            .up_completion_lateness_ticks
            .is_some_and(|late| late > timing.strict_up_completion_late_ticks);
    if config.estimator.enable_adaptive_lead && pending_plan.is_some_and(|plan| plan.lead_saturated)
    {
        record_lead_saturation(
            &mut local_metrics.lead_saturation_count_up,
            &mut local_metrics.positive_residual_at_cap,
            scan_count,
            signed_delta(send.completed_effective_us, reconciliation.scheduled_us),
        );
    }
    let send_warn_threshold_us = frozen_budget.send_warn_us;
    local_metrics.send_warn_threshold_us = frozen_budget.send_warn_us;
    local_metrics.bookkeeping_warn_threshold_us = frozen_budget.bookkeeping_warn_us;
    local_metrics.send_up_warn_threshold_us = frozen_budget.send_warn_us;
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
    if config.estimator.enable_adaptive_lead
        && let Err(error) = update_estimator_after_send_class(
            estimator,
            ActionKind::Up,
            send.sender_duration_us,
            send.sent_count,
            scan_count,
            lead_up,
            reconciliation.up_completion_error_us,
            reconciliation.clean_up_sample,
            latency_class,
        )
    {
        return Err(DispatchStep::Terminate(format!(
            "estimator update failure: {error}"
        )));
    }
    record_lateness(
        signed_delta(send.completed_effective_us, reconciliation.scheduled_us),
        true,
        reconciliation.deferred_by_us > 0,
        local_metrics,
    );
    super::publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
    let current_us = match qpc_clock.now() {
        Ok(now) => {
            match qpc_clock.duration_to_us(match now.checked_duration_since(clock_state.epoch) {
                Ok(dur) => dur,
                Err(_) => DurationTicks::ZERO,
            }) {
                Ok(us) => us,
                Err(error) => {
                    return Err(DispatchStep::Terminate(format!(
                        "QPC us conversion failure: {error:?}"
                    )));
                }
            }
        }
        Err(error) => return Err(DispatchStep::Terminate(format!("QPC failure: {error:?}"))),
    };
    try_publish_metrics(
        local_metrics,
        metrics,
        current_us,
        !reconciliation.clean_up_sample || reconciliation.recovery_required,
    );
    let iteration_ready_us = match qpc_clock.now() {
        Ok(now) => {
            match qpc_clock.duration_to_us(match now.checked_duration_since(clock_state.epoch) {
                Ok(dur) => dur,
                Err(_) => DurationTicks::ZERO,
            }) {
                Ok(us) => us,
                Err(error) => {
                    return Err(DispatchStep::Terminate(format!(
                        "QPC us conversion failure: {error:?}"
                    )));
                }
            }
        }
        Err(error) => return Err(DispatchStep::Terminate(format!("QPC failure: {error:?}"))),
    };
    runtime.pending_pre_send_spin_us = 0;
    observe_dispatch_health(
        DispatchHealthObservation {
            send_duration_us: send.sender_duration_us,
            post_send_duration_us: iteration_ready_us.saturating_sub(send.completed_effective_us),
            path: frozen_budget.path,
            send_warn_us: send_warn_threshold_us,
            bookkeeping_warn_us: frozen_budget.bookkeeping_warn_us,
            elapsed_us: send.completed_effective_us,
        },
        health.options.window_policy(),
        &mut health.send_pure_window,
        &mut health.bookkeeping_window,
        local_metrics,
    );
    Ok(ReleaseOutcomeFlags {
        strict_up_completion_late,
        saturation_abort,
    })
}

fn finalize_release_recovery(
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    runtime: &mut WorkerRuntime,
    secondary_errors: &mut Vec<String>,
    target_hwnd: &AtomicIsize,
    send: &ReleaseSend,
) -> DispatchStep {
    runtime.verified_target = None;
    runtime.force_full_cleanup = true;
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
    DispatchStep::Terminate(term_err.unwrap_or_else(|| "recovery failure".to_string()))
}
