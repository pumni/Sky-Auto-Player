use super::super::super::{
    DurationTicks, LatencyClass, PlaybackClockState, QpcClock, RtTraceRecord,
    RuntimeDispatchCoordinator, STRICT_SATURATION_ABORT_STREAK, SharedMetrics, TRACE_FLAG_ANOMALY,
    TRACE_FLAG_DEFERRED, TRACE_FLAG_RECOVERY, TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN, TRACE_KIND_UP,
    TelemetryCollector, TimelineTicks, TraceContext, TraceDelivery, TraceTiming, TrackedKeyState,
    trace_outcome_code, try_publish_metrics,
};
use super::super::{
    DispatchPath, WorkerConfig, WorkerHealthState, WorkerMetricsLocal, WorkerRuntime,
    WorkerTimingState, release_runtime_outcome,
};
use super::observer_drain::{
    DispatchObservation, DownObservation, PendingObservationQueue, UpObservation,
};
use super::release::{ReleaseOutcomeFlags, ReleaseReconciliation, ReleaseSend};
use super::timing::{DownSendTiming, read_qpc_us};
use super::{AuthoredBatchView, DispatchStep};
use sky_dispatch_core::coordinator::{PendingDispatchPlan, PendingRelease};
use sky_dispatch_win32::input::PacketRetryReason;
use smallvec::SmallVec;

/// Hard-path suffix of the note-on dispatch.  Everything from the coordinator
/// commit onward must leave the worker thread as fast as possible.  The only
/// work that stays here is: (1) the mandatory telemetry trace append, (2) the
/// `dispatch_ready` boundary sample, and (3) enqueueing an allocation-free
/// `DispatchObservation`.  The estimator update, health windows, lateness
/// accounting, and metric publication are consumed later by
/// `drain_down_send_outcome` from the dispatch loop's deferred observer.
#[allow(clippy::too_many_arguments)]
pub(super) fn publisher_down_send_outcome(
    view: &AuthoredBatchView,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    qpc_clock: QpcClock,
    telemetry: &mut TelemetryCollector,
    effective_now_ticks: TimelineTicks,
    lead_down: u64,
    lead_down_saturated: bool,
    lead_down_ticks: DurationTicks,
    latency_class: LatencyClass,
    frozen_budget: &crate::engine::worker::health::FrozenDispatchBudget,
    trace_kind: u8,
    result_success: bool,
    result_sent: &SmallVec<[u16; 15]>,
    result_skipped_duplicates: &SmallVec<[u16; 15]>,
    result_send_attempts: u8,
    result_retry_reason: PacketRetryReason,
    result_chord_integrity_lost: bool,
    result_last_win32_error: Option<u32>,
    observer: &mut PendingObservationQueue,
    timing_proof: &DownSendTiming,
) -> DispatchStep {
    let DownSendTiming {
        sender_completed_qpc,
        sender_started_effective_ticks,
        completed_effective_ticks,
        completed_effective,
        sender_duration_us,
        requested_count,
        delivered_count,
        completion_error_ticks_value,
        authored_completion_error_ticks_value,
        completion_error_us,
        clean_directional_sample,
        recovered_partial_up,
        recovered_retry_late,
        retry_late_abort,
        strict_completion_late,
        saturation_abort,
        bookkeeping_completed_us,
        ..
    } = *timing_proof;
    // (1) Mandatory trace append — hard path, O(1), no allocation.
    let mut force_dispatch_publish = match record_down_send_telemetry(
        view,
        telemetry,
        trace_kind,
        effective_now_ticks,
        lead_down_ticks,
        result_success,
        completed_effective,
        result_sent,
        result_skipped_duplicates,
        result_send_attempts,
        result_retry_reason,
        result_chord_integrity_lost,
        result_last_win32_error,
        sender_started_effective_ticks,
        completed_effective_ticks,
        completion_error_ticks_value,
        authored_completion_error_ticks_value,
        bookkeeping_completed_us,
        requested_count,
        recovered_retry_late,
        recovered_partial_up,
        strict_completion_late,
    ) {
        Ok(value) => value.1,
        Err(step) => return step,
    };
    if result_chord_integrity_lost || retry_late_abort || strict_completion_late || saturation_abort
    {
        force_dispatch_publish = true;
    }
    runtime.pending_pre_send_spin_us = 0;
    // (2) dispatch_ready boundary: typed QPC sample taken immediately after the
    // coordinator commit and trace append, and strictly before the observer
    // work (estimator / health / publish) which now lives in the drain.
    let dispatch_ready_qpc = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return DispatchStep::Terminate(format!("QPC runtime failure: {error:?}"));
        }
    };
    let core_post_send_us = match dispatch_ready_qpc.checked_duration_since(sender_completed_qpc) {
        Ok(duration) => match qpc_clock.duration_to_us(duration) {
            Ok(us) => us,
            Err(error) => {
                return DispatchStep::Terminate(format!(
                    "note-on post-send conversion failure: {error:?}"
                ));
            }
        },
        Err(_) => 0,
    };
    // (3) Enqueue an allocation-free snapshot for the deferred drain.  Drops
    // the oldest entry when the fixed queue is full (see queue docs).
    observer.push(
        DispatchObservation::Down(DownObservation {
            path: frozen_budget.path,
            latency_class,
            lead_down_saturated,
            lead_down,
            sender_duration_us,
            delivered_count,
            batch_intent_count: view.batch_intent_count,
            completion_error_us,
            clean_directional_sample,
            completed_effective,
            authored_batch_scheduled_us: view.authored_batch_scheduled_us,
            batch_scheduled_us: view.batch_scheduled_us,
            core_post_send_us,
            send_warn_us: frozen_budget.send_warn_us,
            bookkeeping_warn_us: frozen_budget.bookkeeping_warn_us,
            force_publish: force_dispatch_publish,
        }),
        &mut local_metrics.observer_dropped_samples,
        &mut local_metrics.observer_queue_high_watermark,
    );
    resolve_slo_terminal_step(
        result_chord_integrity_lost,
        retry_late_abort,
        strict_completion_late,
        saturation_abort,
        completion_error_us,
        view,
        runtime,
    )
}

/// Maps the post-send SLO flags to a `DispatchStep` terminal (or repeat)
/// decision.  Kept separate so the publish function stays under the dispatch
/// per-function line limit.
fn resolve_slo_terminal_step(
    result_chord_integrity_lost: bool,
    retry_late_abort: bool,
    strict_completion_late: bool,
    saturation_abort: bool,
    completion_error_us: i64,
    view: &AuthoredBatchView,
    runtime: &mut WorkerRuntime,
) -> DispatchStep {
    if result_chord_integrity_lost {
        runtime.verified_target = None;
        return DispatchStep::Terminate(format!(
            "SendInput split authored chord at action {}",
            view.batch_source_action_index
        ));
    }
    if retry_late_abort {
        return DispatchStep::Terminate(format!(
            "strict timing rejected zero-progress retry at action {}: completion was {}us late",
            view.batch_source_action_index, completion_error_us
        ));
    }
    if strict_completion_late {
        let timing_label = if matches!(view.dispatch_path, DispatchPath::UpOnly { .. }) {
            "note-off"
        } else {
            "note-on"
        };
        return DispatchStep::Terminate(format!(
            "strict timing completion SLO exceeded for {timing_label} at action {}: completion was {}us late",
            view.batch_source_action_index, completion_error_us
        ));
    }
    if saturation_abort {
        let timing_label = if matches!(view.dispatch_path, DispatchPath::UpOnly { .. }) {
            "note-off"
        } else {
            "note-on"
        };
        return DispatchStep::Terminate(format!(
            "strict timing SLO exceeded: {timing_label} lead saturated with positive residual for {} consecutive dispatches",
            STRICT_SATURATION_ABORT_STREAK
        ));
    }
    DispatchStep::Dispatched
}

/// Computes the down_outcome label, the boolean force-publish flag, and the
/// trace flags; pushes the `RtTraceRecord::dispatched` entry for one note-on
/// packet.  Returns the outcome label so callers can use it for additional
/// accounting without recomputing the predicate tree.
#[allow(clippy::too_many_arguments)]
pub(super) fn record_down_send_telemetry(
    view: &AuthoredBatchView,
    telemetry: &mut TelemetryCollector,
    trace_kind: u8,
    effective_now_ticks: TimelineTicks,
    lead_down_ticks: DurationTicks,
    result_success: bool,
    completed_effective: u64,
    result_sent: &SmallVec<[u16; 15]>,
    result_skipped_duplicates: &SmallVec<[u16; 15]>,
    result_send_attempts: u8,
    result_retry_reason: PacketRetryReason,
    result_chord_integrity_lost: bool,
    result_last_win32_error: Option<u32>,
    sender_started_effective_ticks: TimelineTicks,
    completed_effective_ticks: TimelineTicks,
    completion_error_ticks_value: i64,
    authored_completion_error_ticks_value: i64,
    bookkeeping_completed_us: u64,
    requested_count: usize,
    recovered_retry_late: bool,
    recovered_partial_up: bool,
    strict_completion_late: bool,
) -> Result<(&'static str, bool), DispatchStep> {
    let down_outcome = if recovered_retry_late {
        "recovered_zero_progress_but_late"
    } else if recovered_partial_up {
        "recovered_partial_up_retry"
    } else if strict_completion_late {
        "strict_completion_slo_exceeded"
    } else if result_chord_integrity_lost {
        "chord_integrity_lost"
    } else if view.packet_masks.is_some_and(|_| result_success)
        || (view.packet_masks.is_none() && result_sent.len() == view.scan_batch.len())
    {
        "sent"
    } else {
        "partial_note_on"
    };
    let force_publish = !result_success
        || !matches!(result_retry_reason, PacketRetryReason::None)
        || result_chord_integrity_lost;
    let mut trace_flags = 0u8;
    let send_completed_fully = view.packet_masks.is_some_and(|_| result_success)
        || (view.packet_masks.is_none() && result_sent.len() == view.scan_batch.len());
    if send_completed_fully {
        trace_flags |= TRACE_FLAG_SENT_FULL;
    }
    if recovered_retry_late || result_chord_integrity_lost {
        trace_flags |= TRACE_FLAG_RECOVERY;
    }
    if down_outcome != "sent" {
        trace_flags |= TRACE_FLAG_ANOMALY;
    }
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: view.batch_source_action_index,
                kind: trace_kind,
                outcome: trace_outcome_code(down_outcome),
                polyphony: view.batch_intent_count,
                flags: trace_flags,
                win32_error: result_last_win32_error.unwrap_or(0),
            },
            TraceTiming {
                authored_ticks: view.authored_batch_scheduled_ticks,
                effective_deadline_ticks: view.batch_scheduled_ticks,
                wake_ticks: effective_now_ticks,
                send_started_ticks: Some(sender_started_effective_ticks),
                send_completed_ticks: Some(completed_effective_ticks),
                bookkeeping_duration_us: bookkeeping_completed_us
                    .saturating_sub(completed_effective),
                completion_error_ticks: completion_error_ticks_value,
                authored_completion_error_ticks: authored_completion_error_ticks_value,
                applied_lead_ticks: lead_down_ticks,
            },
            TraceDelivery {
                requested: view.batch_intent_count,
                sent: if view.packet_masks.is_some() && result_success {
                    requested_count
                } else {
                    result_sent.len()
                },
                skipped: result_skipped_duplicates.len(),
                send_attempts: usize::from(result_send_attempts),
            },
        )
    }) {
        return Err(DispatchStep::Terminate(format!(
            "native telemetry record overflow: {error}"
        )));
    }
    Ok((down_outcome, force_publish))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn commit_suppressed_up_request(
    view: &AuthoredBatchView,
    coordinator: &mut RuntimeDispatchCoordinator,
    clock_state: &mut PlaybackClockState,
    qpc_clock: QpcClock,
    telemetry: &mut TelemetryCollector,
    backend: &TrackedKeyState,
    local_metrics: &mut WorkerMetricsLocal,
    metrics: &SharedMetrics,
    last_published_error: &mut Option<String>,
    effective_now_ticks: TimelineTicks,
    lead_down_ticks: DurationTicks,
) -> DispatchStep {
    let (_, suppressed) = match coordinator.commit_up_request(view.prepared_batch) {
        Ok(value) => value,
        Err(error) => {
            return DispatchStep::Terminate(format!(
                "coordinator note-off request failure: {error}"
            ));
        }
    };
    if !suppressed.is_empty()
        && let Err(error) = telemetry.try_push(|| {
            RtTraceRecord::dispatched(
                TraceContext {
                    event_index: view.batch_source_action_index,
                    kind: TRACE_KIND_UP,
                    outcome: trace_outcome_code("suppressed_stale_up"),
                    polyphony: suppressed.len(),
                    flags: TRACE_FLAG_ANOMALY,
                    win32_error: 0,
                },
                TraceTiming {
                    authored_ticks: view.authored_batch_scheduled_ticks,
                    effective_deadline_ticks: view.batch_scheduled_ticks,
                    wake_ticks: effective_now_ticks,
                    send_started_ticks: None,
                    send_completed_ticks: None,
                    bookkeeping_duration_us: 0,
                    completion_error_ticks: 0,
                    authored_completion_error_ticks: 0,
                    applied_lead_ticks: lead_down_ticks,
                },
                TraceDelivery {
                    requested: 0,
                    sent: 0,
                    skipped: 0,
                    send_attempts: 0,
                },
            )
        })
    {
        return DispatchStep::Terminate(format!("native telemetry record overflow: {error}"));
    }
    super::publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
    let current_us = match read_qpc_us(qpc_clock, clock_state) {
        Ok(us) => us,
        Err(step) => return step,
    };
    try_publish_metrics(local_metrics, metrics, current_us, !suppressed.is_empty());
    DispatchStep::Dispatched
}

pub(super) fn record_blocked_unfocused_telemetry(
    telemetry: &mut TelemetryCollector,
    view: &AuthoredBatchView,
    effective_now_ticks: TimelineTicks,
    lead_down_ticks: DurationTicks,
) -> Result<(), DispatchStep> {
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: view.batch_source_action_index,
                kind: TRACE_KIND_DOWN,
                outcome: trace_outcome_code("blocked_unfocused"),
                polyphony: view.batch_intent_count,
                flags: TRACE_FLAG_ANOMALY,
                win32_error: 0,
            },
            TraceTiming {
                authored_ticks: view.authored_batch_scheduled_ticks,
                effective_deadline_ticks: view.batch_scheduled_ticks,
                wake_ticks: effective_now_ticks,
                send_started_ticks: None,
                send_completed_ticks: None,
                bookkeeping_duration_us: 0,
                completion_error_ticks: 0,
                authored_completion_error_ticks: 0,
                applied_lead_ticks: lead_down_ticks,
            },
            TraceDelivery {
                requested: 0,
                sent: 0,
                skipped: 0,
                send_attempts: 0,
            },
        )
    }) {
        return Err(DispatchStep::Terminate(format!(
            "native telemetry record overflow: {error}"
        )));
    }
    Ok(())
}

/// Pushes the `RtTraceRecord::dispatched` entry for one note-off batch using
/// the reconciliation-derived values; returns the outcome label.
#[allow(clippy::too_many_arguments)]
pub(super) fn record_release_telemetry(
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

/// Note-off hard-path observer: computes the mandatory terminal SLO flags
/// (`saturation_abort`, `strict_up_completion_late`) and captures the
/// `dispatch_ready` boundary immediately after the coordinator commit and the
/// mandatory telemetry trace (`record_release_telemetry`).  The estimator
/// update, lead-saturation accounting, lateness metric, health-window
/// observation, and diagnostic metric publication are deferred to the observer
/// drain (`drain_up_send_outcome`) via the queued `UpObservation`.
#[allow(clippy::too_many_arguments)]
pub(super) fn observe_release_send_health(
    qpc_clock: QpcClock,
    config: &WorkerConfig,
    timing: &WorkerTimingState,
    health: &mut WorkerHealthState,
    runtime: &mut WorkerRuntime,
    local_metrics: &mut WorkerMetricsLocal,
    observer: &mut PendingObservationQueue,
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
    runtime.pending_pre_send_spin_us = 0;
    // §8.5 DISPATCH_READY boundary: typed QPC sample taken immediately after
    // the coordinator commit (reconcile_release_recovery) and trace append, and
    // strictly before the estimator/health/publish observer work (now deferred).
    let dispatch_ready_qpc = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "note-off dispatch_ready QPC failure: {error:?}"
            )));
        }
    };
    let core_post_send_us =
        match dispatch_ready_qpc.checked_duration_since(send.sender_completed_qpc) {
            Ok(duration) => match qpc_clock.duration_to_us(duration) {
                Ok(us) => us,
                Err(error) => {
                    return Err(DispatchStep::Terminate(format!(
                        "note-off post-send conversion failure: {error:?}"
                    )));
                }
            },
            Err(_) => 0,
        };
    observer.push(
        DispatchObservation::Up(UpObservation {
            latency_class,
            sender_duration_us: send.sender_duration_us,
            sent_count: send.sent_count,
            scan_count,
            lead_up,
            lead_up_saturated: up_saturated_positive,
            completed_effective: send.completed_effective_us,
            scheduled_us: reconciliation.scheduled_us,
            deferred_by_us: reconciliation.deferred_by_us,
            up_completion_error_us: reconciliation.up_completion_error_us,
            clean_up_sample: reconciliation.clean_up_sample,
            core_post_send_us,
            send_warn_us: frozen_budget.send_warn_us,
            bookkeeping_warn_us: frozen_budget.bookkeeping_warn_us,
            force_publish: !reconciliation.clean_up_sample || reconciliation.recovery_required,
        }),
        &mut local_metrics.observer_dropped_samples,
        &mut local_metrics.observer_queue_high_watermark,
    );
    Ok(ReleaseOutcomeFlags {
        strict_up_completion_late,
        saturation_abort,
    })
}
