use super::super::super::{
    DurationTicks, LatencyClass, PlaybackClockState, QpcClock, RtTraceRecord,
    RuntimeDispatchCoordinator, STRICT_SATURATION_ABORT_STREAK, SendLatencyEstimator,
    SharedMetrics, TRACE_FLAG_ANOMALY, TRACE_FLAG_DEFERRED, TRACE_FLAG_RECOVERY,
    TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN, TRACE_KIND_UP, TelemetryCollector, TimelineTicks,
    TraceContext, TraceDelivery, TraceTiming, TrackedKeyState, trace_outcome_code,
    try_publish_metrics,
};
use super::super::{
    DispatchHealthObservation, DispatchPath, WorkerConfig, WorkerHealthState, WorkerMetricsLocal,
    WorkerRuntime, WorkerTimingState, observe_dispatch_health, publish_backend_metrics,
    record_lateness, record_lead_saturation, release_runtime_outcome, signed_delta,
    update_estimator_after_send_class,
};
use super::authored::resolve_slo_terminal_step;
use super::release::{ReleaseOutcomeFlags, ReleaseReconciliation, ReleaseSend};
#[cfg(any(test, feature = "test-support"))]
use super::release::{take_release_observer_failure, take_release_telemetry_failure};
use super::timing::{
    DownSendTiming, EstimatorObservationEvidence, is_clean_estimator_observation, read_qpc_us,
};
use super::{AuthoredBatchView, DispatchStep};
use sky_dispatch_core::coordinator::{PendingDispatchPlan, PendingRelease};
use sky_dispatch_win32::input::PacketRetryReason;
use smallvec::SmallVec;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[allow(clippy::too_many_arguments)]
pub(crate) fn publisher_down_send_outcome(
    view: &AuthoredBatchView,
    runtime: &mut WorkerRuntime,
    health: &mut WorkerHealthState,
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
        estimator_evidence,
        recovered_zero_progress,
        recovered_partial_up,
        recovered_retry_late,
        retry_late_abort,
        strict_completion_late,
        saturation_abort,
        ..
    } = *timing_proof;
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
        Err(error) => {
            return DispatchStep::Terminate(format!(
                "dispatch-ready QPC ordering failure: {error:?}"
            ));
        }
    };
    // HARD DISPATCH READY BOUNDARY:
    // gameplay-critical dispatch ownership ends here.
    if recovered_retry_late {
        local_metrics.recovered_zero_progress_but_late = local_metrics
            .recovered_zero_progress_but_late
            .saturating_add(1);
    }
    if recovered_zero_progress {
        local_metrics.recovered_zero_progress_retries = local_metrics
            .recovered_zero_progress_retries
            .saturating_add(1);
    }
    if recovered_partial_up {
        local_metrics.recovered_partial_up_retries =
            local_metrics.recovered_partial_up_retries.saturating_add(1);
    }
    match view.dispatch_path {
        DispatchPath::UpOnly { .. } => {
            health.down_saturation_positive_streak = 0;
            health.up_saturation_positive_streak = timing_proof.saturation_streak;
        }
        DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
            health.up_saturation_positive_streak = 0;
            health.down_saturation_positive_streak = timing_proof.saturation_streak;
        }
    }
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
        core_post_send_us,
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
            estimator_evidence,
            completed_effective,
            authored_batch_scheduled_us: view.authored_batch_scheduled_us,
            batch_scheduled_us: view.batch_scheduled_us,
            core_post_send_us,
            send_warn_us: frozen_budget.send_warn_us,
            core_post_send_warn_us: frozen_budget.core_post_send_warn_us,
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
#[allow(clippy::too_many_arguments)]
pub(super) fn record_down_send_telemetry(
    view: &AuthoredBatchView,
    telemetry: &mut TelemetryCollector,
    trace_kind: u8,
    effective_now_ticks: TimelineTicks,
    lead_down_ticks: DurationTicks,
    result_success: bool,
    _completed_effective: u64,
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
    core_post_send_us: u64,
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
                core_post_send_duration_us: core_post_send_us,
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
                    core_post_send_duration_us: 0,
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
                core_post_send_duration_us: 0,
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
#[allow(clippy::too_many_arguments)]
pub(super) fn record_release_telemetry(
    telemetry: &mut TelemetryCollector,
    due_pending: &SmallVec<[PendingRelease; 15]>,
    send: &ReleaseSend,
    reconciliation: &ReleaseReconciliation,
    lead_up_ticks: DurationTicks,
    core_post_send_us: u64,
) -> Result<(), DispatchStep> {
    let Some(first) = due_pending.first() else {
        return Err(DispatchStep::Terminate(
            "empty pending release batch in telemetry recording".to_string(),
        ));
    };
    #[cfg(any(test, feature = "test-support"))]
    if take_release_telemetry_failure(reconciliation.recovery_required) {
        return Err(DispatchStep::Terminate(
            "injected release telemetry failure".to_string(),
        ));
    }
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
                core_post_send_duration_us: core_post_send_us,
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
pub(super) fn observe_release_send_health(
    _qpc_clock: QpcClock,
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
    core_post_send_us: u64,
) -> Result<ReleaseOutcomeFlags, DispatchStep> {
    #[cfg(any(test, feature = "test-support"))]
    if take_release_observer_failure(reconciliation.recovery_required) {
        return Err(DispatchStep::Terminate(
            "injected release observer failure".to_string(),
        ));
    }
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
        && is_clean_estimator_observation(reconciliation.estimator_evidence)
        && reconciliation
            .up_completion_lateness_ticks
            .is_some_and(|late| late > timing.strict_up_completion_late_ticks);
    runtime.pending_pre_send_spin_us = 0;
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
            estimator_evidence: reconciliation.estimator_evidence,
            core_post_send_us,
            send_warn_us: frozen_budget.send_warn_us,
            core_post_send_warn_us: frozen_budget.core_post_send_warn_us,
            force_publish: !is_clean_estimator_observation(reconciliation.estimator_evidence)
                || reconciliation.recovery_required,
        }),
        &mut local_metrics.observer_dropped_samples,
        &mut local_metrics.observer_queue_high_watermark,
    );
    Ok(ReleaseOutcomeFlags {
        strict_up_completion_late,
        saturation_abort,
    })
}
pub const OBSERVATION_QUEUE_CAPACITY: usize = 64;
#[derive(Clone, Copy, Debug)]
pub enum DispatchObservation {
    Down(DownObservation),
    Up(UpObservation),
}
#[derive(Clone, Copy, Debug)]
pub struct DownObservation {
    pub path: DispatchPath,
    pub latency_class: LatencyClass,
    pub lead_down_saturated: bool,
    pub lead_down: u64,
    pub sender_duration_us: u64,
    pub delivered_count: usize,
    pub batch_intent_count: usize,
    pub completion_error_us: i64,
    pub estimator_evidence: EstimatorObservationEvidence,
    pub completed_effective: u64,
    pub authored_batch_scheduled_us: u64,
    pub batch_scheduled_us: u64,
    pub core_post_send_us: u64,
    pub send_warn_us: u64,
    pub core_post_send_warn_us: u64,
    pub force_publish: bool,
}
#[derive(Clone, Copy, Debug)]
pub struct UpObservation {
    pub latency_class: LatencyClass,
    pub sender_duration_us: u64,
    pub sent_count: usize,
    pub scan_count: usize,
    pub lead_up: u64,
    pub lead_up_saturated: bool,
    pub completed_effective: u64,
    pub scheduled_us: u64,
    pub deferred_by_us: u64,
    pub up_completion_error_us: i64,
    pub estimator_evidence: EstimatorObservationEvidence,
    pub core_post_send_us: u64,
    pub send_warn_us: u64,
    pub core_post_send_warn_us: u64,
    pub force_publish: bool,
}
#[derive(Debug)]
pub struct PendingObservationQueue {
    entries: [Option<DispatchObservation>; OBSERVATION_QUEUE_CAPACITY],
    head: usize,
    len: usize,
}
impl Default for PendingObservationQueue {
    fn default() -> Self {
        Self {
            entries: [None; OBSERVATION_QUEUE_CAPACITY],
            head: 0,
            len: 0,
        }
    }
}
impl PendingObservationQueue {
    pub fn push(
        &mut self,
        observation: DispatchObservation,
        dropped_samples: &mut u64,
        high_watermark: &mut u64,
    ) {
        if self.len == self.entries.len() {
            self.entries[self.head] = None;
            self.head = (self.head + 1) % self.entries.len();
            *dropped_samples = dropped_samples.saturating_add(1);
        } else {
            self.len += 1;
        }
        let tail = (self.head + self.len - 1) % self.entries.len();
        self.entries[tail] = Some(observation);
        *high_watermark = (*high_watermark).max(self.len as u64);
    }
    pub fn pop_front(&mut self) -> Option<DispatchObservation> {
        if self.len == 0 {
            return None;
        }
        let entry = self.entries[self.head].take();
        self.head = (self.head + 1) % self.entries.len();
        self.len -= 1;
        entry
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}
pub(crate) fn observer_has_safe_slack(
    deadline_ticks: Option<TimelineTicks>,
    effective_now_ticks: TimelineTicks,
    budget_us: u64,
    margin_us: u64,
    qpc_clock: QpcClock,
) -> bool {
    let Some(deadline) = deadline_ticks else {
        return true;
    };
    if deadline.as_u64() <= effective_now_ticks.as_u64() {
        return false;
    }
    let slack_ticks = DurationTicks::from_raw(deadline.as_u64() - effective_now_ticks.as_u64());
    match qpc_clock.duration_to_us(slack_ticks) {
        Ok(slack_us) => slack_us >= budget_us.saturating_add(margin_us),
        Err(_) => false,
    }
}
#[cfg(any(test, feature = "test-support"))]
static OBSERVER_ARTIFICIAL_COST_US: AtomicU64 = AtomicU64::new(0);
#[cfg(any(test, feature = "test-support"))]
static OBSERVER_INITIAL_BUDGET_OVERRIDE_US: AtomicU64 = AtomicU64::new(0);
#[cfg(any(test, feature = "test-support"))]
#[cfg(any(test, feature = "test-support"))]
static OBSERVER_TEST_HOOK_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
#[cfg(any(test, feature = "test-support"))]
pub struct ObserverTestHookGuard {
    _lock: parking_lot::MutexGuard<'static, ()>,
}
#[cfg(any(test, feature = "test-support"))]
impl Drop for ObserverTestHookGuard {
    fn drop(&mut self) {
        reset_observer_test_hooks();
        super::release::reset_release_test_hooks();
    }
}
#[cfg(any(test, feature = "test-support"))]
pub fn observer_test_hook_guard() -> ObserverTestHookGuard {
    let lock = OBSERVER_TEST_HOOK_LOCK.lock();
    reset_observer_test_hooks();
    super::release::reset_release_test_hooks();
    ObserverTestHookGuard { _lock: lock }
}
#[cfg(any(test, feature = "test-support"))]
pub fn set_observer_artificial_cost_us(us: u64) {
    OBSERVER_ARTIFICIAL_COST_US.store(us, Ordering::SeqCst);
}
#[cfg(any(test, feature = "test-support"))]
pub fn set_observer_initial_budget_override_us(us: u64) {
    OBSERVER_INITIAL_BUDGET_OVERRIDE_US.store(us, Ordering::SeqCst);
}
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn observer_initial_budget_override_us() -> u64 {
    OBSERVER_INITIAL_BUDGET_OVERRIDE_US.load(Ordering::SeqCst)
}
#[cfg(any(test, feature = "test-support"))]
pub fn reset_observer_test_hooks() {
    OBSERVER_ARTIFICIAL_COST_US.store(0, Ordering::SeqCst);
    OBSERVER_INITIAL_BUDGET_OVERRIDE_US.store(0, Ordering::SeqCst);
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_down_send_outcome(
    observation: &DownObservation,
    config: &WorkerConfig,
    health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    last_published_error: &mut Option<String>,
    metrics: &SharedMetrics,
    backend: &mut TrackedKeyState,
    estimator: &mut SendLatencyEstimator,
    now_us: u64,
) {
    local_metrics.sendinput_warn_threshold_us = observation.send_warn_us;
    local_metrics.core_post_send_warn_threshold_us = observation.core_post_send_warn_us;
    match observation.path {
        DispatchPath::DownOnly { .. } => {
            local_metrics.send_down_warn_threshold_us = observation.send_warn_us;
        }
        DispatchPath::UpOnly { .. } => {
            local_metrics.send_up_warn_threshold_us = observation.send_warn_us;
        }
        DispatchPath::Mixed { .. } => {
            local_metrics.send_mixed_warn_threshold_us = observation.send_warn_us;
        }
    }
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
    local_metrics.core_post_send_max_us = local_metrics
        .core_post_send_max_us
        .max(observation.core_post_send_us);
    if config.estimator.enable_adaptive_lead && observation.lead_down_saturated {
        match observation.path {
            DispatchPath::UpOnly { .. } => record_lead_saturation(
                &mut local_metrics.lead_saturation_count_up,
                &mut local_metrics.positive_residual_at_cap,
                observation.batch_intent_count,
                signed_delta(
                    observation.completed_effective,
                    observation.batch_scheduled_us,
                ),
            ),
            DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => record_lead_saturation(
                &mut local_metrics.lead_saturation_count_down,
                &mut local_metrics.positive_residual_at_cap,
                observation.batch_intent_count,
                signed_delta(
                    observation.completed_effective,
                    observation.batch_scheduled_us,
                ),
            ),
        }
    }
    if config.estimator.enable_adaptive_lead {
        let send_path = super::super::estimator_path_for_dispatch(observation.path);
        let _ = update_estimator_after_send_class(
            estimator,
            send_path,
            observation.sender_duration_us,
            observation.delivered_count,
            observation.batch_intent_count,
            observation.lead_down,
            observation.completion_error_us,
            observation.estimator_evidence,
            observation.latency_class,
        );
    }
    record_lateness(
        signed_delta(
            observation.completed_effective,
            observation.authored_batch_scheduled_us,
        ),
        false,
        false,
        local_metrics,
    );
    observe_dispatch_health(
        DispatchHealthObservation {
            send_duration_us: observation.sender_duration_us,
            post_send_duration_us: observation.core_post_send_us,
            path: observation.path,
            send_warn_us: observation.send_warn_us,
            core_post_send_warn_us: observation.core_post_send_warn_us,
            elapsed_us: observation.completed_effective,
        },
        health.options.window_policy(),
        &mut health.sendinput_window,
        &mut health.core_post_send_window,
        local_metrics,
    );
    publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
    try_publish_metrics(local_metrics, metrics, now_us, observation.force_publish);
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_up_send_outcome(
    observation: &UpObservation,
    config: &WorkerConfig,
    health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    last_published_error: &mut Option<String>,
    metrics: &SharedMetrics,
    backend: &mut TrackedKeyState,
    estimator: &mut SendLatencyEstimator,
    now_us: u64,
) {
    local_metrics.sendinput_warn_threshold_us = observation.send_warn_us;
    local_metrics.core_post_send_warn_threshold_us = observation.core_post_send_warn_us;
    local_metrics.send_up_warn_threshold_us = observation.send_warn_us;
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
    local_metrics.core_post_send_max_us = local_metrics
        .core_post_send_max_us
        .max(observation.core_post_send_us);
    if config.estimator.enable_adaptive_lead && observation.lead_up_saturated {
        record_lead_saturation(
            &mut local_metrics.lead_saturation_count_up,
            &mut local_metrics.positive_residual_at_cap,
            observation.scan_count,
            signed_delta(observation.completed_effective, observation.scheduled_us),
        );
    }
    if config.estimator.enable_adaptive_lead {
        let _ = update_estimator_after_send_class(
            estimator,
            sky_dispatch_core::estimator::SendPath::UpOnly,
            observation.sender_duration_us,
            observation.sent_count,
            observation.scan_count,
            observation.lead_up,
            observation.up_completion_error_us,
            observation.estimator_evidence,
            observation.latency_class,
        );
    }
    record_lateness(
        signed_delta(observation.completed_effective, observation.scheduled_us),
        true,
        observation.deferred_by_us > 0,
        local_metrics,
    );
    observe_dispatch_health(
        DispatchHealthObservation {
            send_duration_us: observation.sender_duration_us,
            post_send_duration_us: observation.core_post_send_us,
            path: DispatchPath::UpOnly {
                up_count: observation.scan_count,
            },
            send_warn_us: observation.send_warn_us,
            core_post_send_warn_us: observation.core_post_send_warn_us,
            elapsed_us: observation.completed_effective,
        },
        health.options.window_policy(),
        &mut health.sendinput_window,
        &mut health.core_post_send_window,
        local_metrics,
    );
    publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
    try_publish_metrics(local_metrics, metrics, now_us, observation.force_publish);
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_one_observer(
    pending: &mut PendingObservationQueue,
    config: &WorkerConfig,
    health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    last_published_error: &mut Option<String>,
    metrics: &SharedMetrics,
    backend: &mut TrackedKeyState,
    estimator: &mut SendLatencyEstimator,
    qpc_clock: QpcClock,
    now_us: u64,
) -> u64 {
    let Some(observation) = pending.pop_front() else {
        return 0;
    };
    let drain_start = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(_) => return 0,
    };
    match &observation {
        DispatchObservation::Down(down) => drain_down_send_outcome(
            down,
            config,
            health,
            local_metrics,
            last_published_error,
            metrics,
            backend,
            estimator,
            now_us,
        ),
        DispatchObservation::Up(up) => drain_up_send_outcome(
            up,
            config,
            health,
            local_metrics,
            last_published_error,
            metrics,
            backend,
            estimator,
            now_us,
        ),
    }
    #[cfg(any(test, feature = "test-support"))]
    {
        let cost_us = OBSERVER_ARTIFICIAL_COST_US.load(Ordering::Relaxed);
        if cost_us > 0 {
            std::thread::sleep(std::time::Duration::from_micros(cost_us));
        }
    }
    let drain_end = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(_) => return 0,
    };
    let drain_us = match drain_end.checked_duration_since(drain_start) {
        Ok(duration) => qpc_clock.duration_to_us(duration).unwrap_or_default(),
        Err(_) => 0,
    };
    super::super::health::observe_observer_health(
        drain_us,
        health.options.observer_warn_us,
        now_us,
        health.options.window_policy(),
        &mut health.observer_window,
        local_metrics,
    );
    drain_us
}
