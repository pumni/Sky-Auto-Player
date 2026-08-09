use super::super::super::{
    DurationTicks, LatencyClass, PlaybackClockState, QpcClock, RtTraceRecord,
    RuntimeDispatchCoordinator, SendLatencyEstimator, SharedMetrics, TRACE_FLAG_ANOMALY,
    TRACE_FLAG_DEFERRED, TRACE_FLAG_RECOVERY, TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN, TRACE_KIND_UP,
    TelemetryCollector, TimelineTicks, TraceContext, TraceDelivery, TraceTiming, TrackedKeyState,
    trace_outcome_code, try_publish_metrics,
};
use super::super::{
    DispatchHealthObservation, DispatchPath, WorkerConfig, WorkerHealthState, WorkerMetricsLocal,
    WorkerRuntime, WorkerTimingState, observe_dispatch_health, publish_backend_metrics,
    record_lateness, record_lead_saturation, release_runtime_outcome, signed_delta,
    update_estimator_after_send_class,
};
use super::authored::resolve_slo_terminal_step;
use super::observation::{
    DispatchObservation, DownObservation, DownTraceObservation, OBSERVATION_QUEUE_CAPACITY,
    UpObservation,
};
#[cfg(any(test, feature = "test-support"))]
use super::release::{take_release_observer_failure, take_release_telemetry_failure};
use super::timing::{DownSendTiming, is_clean_estimator_observation, read_qpc_us};
use super::{AuthoredBatchView, DispatchStep};
use sky_dispatch_win32::input::PacketRetryReason;
use smallvec::SmallVec;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicU64, Ordering};
pub(crate) fn take_wake_to_send_start_us(
    runtime: &mut WorkerRuntime,
    qpc_clock: QpcClock,
    sender_started_qpc: sky_dispatch_win32::clock::QpcTicks,
) -> Option<u64> {
    runtime
        .last_dispatch_deadline_wake_qpc
        .take()
        .and_then(|wake| sender_started_qpc.checked_duration_since(wake).ok())
        .and_then(|ticks| qpc_clock.duration_to_us(ticks).ok())
}
#[cfg(test)]
mod wake_tests {
    use super::take_wake_to_send_start_us;
    use crate::engine::worker::WorkerRuntime;
    use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
    use std::num::NonZeroU64;

    #[test]
    fn deadline_wake_is_consumed_by_only_one_observation() {
        let mut runtime = WorkerRuntime {
            last_dispatch_deadline_wake_qpc: Some(QpcTicks::from_raw(100)),
            ..WorkerRuntime::default()
        };
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).expect("frequency"));
        assert_eq!(
            take_wake_to_send_start_us(&mut runtime, clock, QpcTicks::from_raw(125)),
            Some(25)
        );
        assert_eq!(
            take_wake_to_send_start_us(&mut runtime, clock, QpcTicks::from_raw(150)),
            None
        );
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn publisher_down_send_outcome(
    view: &AuthoredBatchView,
    runtime: &mut WorkerRuntime,
    health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    qpc_clock: QpcClock,
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
        sender_started_qpc,
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
    let wake_to_send_start_us = take_wake_to_send_start_us(runtime, qpc_clock, sender_started_qpc);
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
    runtime.pending_pre_send_spin_us = 0;
    let worker_ready_qpc = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return DispatchStep::Terminate(format!("QPC runtime failure: {error:?}"));
        }
    };
    // HARD DISPATCH READY BOUNDARY:
    // physical/coordinator ownership is safe for the next dispatch.  From
    // here on, only a fixed raw observation enqueue and terminal policy may
    // run on this call stack.
    let observation = DownObservation {
        path: frozen_budget.path,
        latency_class,
        lead_down_saturated,
        lead_down,
        timeline_rebase_count: local_metrics.timeline_rebase_count,
        timeline_rebase_total_ticks: local_metrics.timeline_rebase_total_ticks,
        timeline_rebase_max_ticks: local_metrics.timeline_rebase_max_ticks,
        timeline_rebase_last_reason: local_metrics.timeline_rebase_last_reason,
        sender_duration_us,
        wake_to_send_start_us,
        delivered_count,
        batch_intent_count: view.batch_intent_count,
        completion_error_us,
        estimator_evidence,
        completed_effective,
        authored_batch_scheduled_us: view.authored_batch_scheduled_us,
        batch_scheduled_us: view.batch_scheduled_us,
        sender_completed_qpc,
        worker_ready_qpc,
        send_warn_us: frozen_budget.send_warn_us,
        core_post_send_warn_us: frozen_budget.core_post_send_warn_us,
        trace: DownTraceObservation {
            event_index: view.batch_source_action_index,
            trace_kind,
            result_success,
            requested_count,
            sent_count: if view.packet_masks.is_some() && result_success {
                requested_count
            } else {
                result_sent.len()
            },
            skipped_count: result_skipped_duplicates.len(),
            send_attempts: result_send_attempts,
            retry_reason: result_retry_reason,
            chord_integrity_lost: result_chord_integrity_lost,
            last_win32_error: result_last_win32_error.unwrap_or(0),
            authored_ticks: view.authored_batch_scheduled_ticks,
            effective_deadline_ticks: view.batch_scheduled_ticks,
            wake_ticks: effective_now_ticks,
            sender_started_ticks: Some(sender_started_effective_ticks),
            sender_completed_ticks: Some(completed_effective_ticks),
            completion_error_ticks: completion_error_ticks_value,
            authored_completion_error_ticks: authored_completion_error_ticks_value,
            applied_lead_ticks: lead_down_ticks,
            recovered_retry_late,
            recovered_partial_up,
            strict_completion_late,
        },
    };
    observer.push(
        DispatchObservation::Down(observation),
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
    observation: &DownObservation,
    telemetry: &mut TelemetryCollector,
    core_post_send_us: u64,
) -> Result<(&'static str, bool), DispatchStep> {
    let trace = observation.trace;
    let result_retry_reason = trace.retry_reason;
    let result_chord_integrity_lost = trace.chord_integrity_lost;
    let result_success = trace.result_success;
    let requested_count = trace.requested_count;
    let result_sent_count = trace.sent_count;
    let result_skipped_count = trace.skipped_count;
    let result_send_attempts = trace.send_attempts;
    let recovered_retry_late = trace.recovered_retry_late;
    let recovered_partial_up = trace.recovered_partial_up;
    let strict_completion_late = trace.strict_completion_late;
    let down_outcome = if recovered_retry_late {
        "recovered_zero_progress_but_late"
    } else if recovered_partial_up {
        "recovered_partial_up_retry"
    } else if strict_completion_late {
        "strict_completion_slo_exceeded"
    } else if result_chord_integrity_lost {
        "chord_integrity_lost"
    } else if result_success && result_sent_count == requested_count {
        "sent"
    } else {
        "partial_note_on"
    };
    let force_publish = !result_success
        || !matches!(result_retry_reason, PacketRetryReason::None)
        || result_chord_integrity_lost;
    let mut trace_flags = 0u8;
    let send_completed_fully = result_success && result_sent_count == requested_count;
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
                event_index: trace.event_index,
                kind: trace.trace_kind,
                outcome: trace_outcome_code(down_outcome),
                polyphony: observation.batch_intent_count,
                flags: trace_flags,
                win32_error: trace.last_win32_error,
            },
            TraceTiming {
                authored_ticks: trace.authored_ticks,
                effective_deadline_ticks: trace.effective_deadline_ticks,
                wake_ticks: trace.wake_ticks,
                send_started_ticks: trace.sender_started_ticks,
                send_completed_ticks: trace.sender_completed_ticks,
                core_post_send_duration_us: core_post_send_us,
                completion_error_ticks: trace.completion_error_ticks,
                authored_completion_error_ticks: trace.authored_completion_error_ticks,
                applied_lead_ticks: trace.applied_lead_ticks,
            },
            TraceDelivery {
                requested: requested_count,
                sent: result_sent_count,
                skipped: result_skipped_count,
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
    observation: &UpObservation,
    core_post_send_us: u64,
) -> Result<(), DispatchStep> {
    let trace = observation.trace;
    #[cfg(any(test, feature = "test-support"))]
    if take_release_telemetry_failure(trace.recovery_required) {
        return Err(DispatchStep::Terminate(
            "injected release telemetry failure".to_string(),
        ));
    }
    let scan_count = trace.scan_count;
    let release_outcome = release_runtime_outcome(
        trace.deferred_by_us,
        trace.sent_count,
        scan_count,
        trace.recovery_required,
    );
    let mut trace_flags = 0u8;
    if trace.sent_count == scan_count {
        trace_flags |= TRACE_FLAG_SENT_FULL;
    }
    if release_outcome == "deferred_release" || release_outcome == "failed_note_off" {
        trace_flags |= TRACE_FLAG_RECOVERY;
    }
    if trace.deferred_by_us > 0 {
        trace_flags |= TRACE_FLAG_DEFERRED;
    }
    if release_outcome != "sent" {
        trace_flags |= TRACE_FLAG_ANOMALY;
    }
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: trace.event_index,
                kind: trace.trace_kind,
                outcome: trace_outcome_code(release_outcome),
                polyphony: scan_count,
                flags: trace_flags,
                win32_error: trace.last_win32_error,
            },
            TraceTiming {
                authored_ticks: trace.authored_ticks,
                effective_deadline_ticks: trace.effective_deadline_ticks,
                wake_ticks: trace.wake_ticks,
                send_started_ticks: trace.sender_started_ticks,
                send_completed_ticks: trace.sender_completed_ticks,
                core_post_send_duration_us: core_post_send_us,
                completion_error_ticks: trace.completion_error_ticks,
                authored_completion_error_ticks: trace.authored_completion_error_ticks,
                applied_lead_ticks: trace.applied_lead_ticks,
            },
            TraceDelivery {
                requested: scan_count,
                sent: trace.sent_count,
                skipped: trace.skipped_count,
                send_attempts: usize::from(trace.send_attempts),
            },
        )
    }) {
        return Err(DispatchStep::Terminate(format!(
            "native telemetry record overflow: {error}"
        )));
    }
    Ok(())
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_full(&self) -> bool {
        self.len == self.entries.len()
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
    telemetry: &mut TelemetryCollector,
    qpc_clock: QpcClock,
    now_us: u64,
) -> Result<(), DispatchStep> {
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
    local_metrics.timeline_rebase_count = observation.timeline_rebase_count;
    local_metrics.timeline_rebase_total_us = qpc_clock
        .duration_to_us(observation.timeline_rebase_total_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "timeline rebase total conversion failure: {error:?}"
            ))
        })?;
    local_metrics.timeline_rebase_max_us = qpc_clock
        .duration_to_us(observation.timeline_rebase_max_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "timeline rebase maximum conversion failure: {error:?}"
            ))
        })?;
    local_metrics.timeline_rebase_last_reason = observation.timeline_rebase_last_reason;
    let core_post_send_us = qpc_clock
        .duration_to_us(
            observation
                .worker_ready_qpc
                .checked_duration_since(observation.sender_completed_qpc)
                .map_err(|error| {
                    DispatchStep::Terminate(format!(
                        "note-on observer QPC ordering failure: {error:?}"
                    ))
                })?,
        )
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "note-on observer post-send conversion failure: {error:?}"
            ))
        })?;
    let (_, force_publish) = record_down_send_telemetry(observation, telemetry, core_post_send_us)?;
    local_metrics.core_post_send_max_us =
        local_metrics.core_post_send_max_us.max(core_post_send_us);
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
            observation.lead_down_saturated,
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
            post_send_duration_us: core_post_send_us,
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
    try_publish_metrics(local_metrics, metrics, now_us, force_publish);
    Ok(())
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
    telemetry: &mut TelemetryCollector,
    qpc_clock: QpcClock,
    now_us: u64,
) -> Result<(), DispatchStep> {
    #[cfg(any(test, feature = "test-support"))]
    if take_release_observer_failure(observation.trace.recovery_required) {
        return Err(DispatchStep::Terminate(
            "injected release observer failure".to_string(),
        ));
    }
    local_metrics.sendinput_warn_threshold_us = observation.send_warn_us;
    local_metrics.core_post_send_warn_threshold_us = observation.core_post_send_warn_us;
    local_metrics.send_up_warn_threshold_us = observation.send_warn_us;
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
    let lead_up = qpc_clock
        .duration_to_us(observation.lead_up_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-off lead conversion failure: {error:?}"))
        })?;
    let core_post_send_us = qpc_clock
        .duration_to_us(
            observation
                .worker_ready_qpc
                .checked_duration_since(observation.sender_completed_qpc)
                .map_err(|error| {
                    DispatchStep::Terminate(format!(
                        "note-off observer QPC ordering failure: {error:?}"
                    ))
                })?,
        )
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "note-off observer post-send conversion failure: {error:?}"
            ))
        })?;
    record_release_telemetry(telemetry, observation, core_post_send_us)?;
    if let Some(pause_ticks) = observation.recovery_pause_ticks {
        let recovery_pause_us = qpc_clock.duration_to_us(pause_ticks).map_err(|error| {
            DispatchStep::Terminate(format!("recovery telemetry conversion failure: {error:?}"))
        })?;
        local_metrics.total_us = local_metrics.total_us.saturating_add(recovery_pause_us);
    }
    local_metrics.core_post_send_max_us =
        local_metrics.core_post_send_max_us.max(core_post_send_us);
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
            lead_up,
            observation.lead_up_saturated,
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
            post_send_duration_us: core_post_send_us,
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
    try_publish_metrics(
        local_metrics,
        metrics,
        now_us,
        !is_clean_estimator_observation(observation.estimator_evidence)
            || observation.trace.recovery_required,
    );
    Ok(())
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
    telemetry: &mut TelemetryCollector,
    qpc_clock: QpcClock,
    now_us: u64,
    timing: &mut WorkerTimingState,
) -> Result<u64, DispatchStep> {
    let Some(observation) = pending.pop_front() else {
        return Ok(0);
    };
    let drain_start = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "observer start QPC failure: {error:?}"
            )));
        }
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
            telemetry,
            qpc_clock,
            now_us,
        )?,
        DispatchObservation::Up(up) => drain_up_send_outcome(
            up,
            config,
            health,
            local_metrics,
            last_published_error,
            metrics,
            backend,
            estimator,
            telemetry,
            qpc_clock,
            now_us,
        )?,
        DispatchObservation::Wait(wait) => {
            super::observation::drain_wait_observation(wait, health, local_metrics, qpc_clock)?;
        }
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
        Err(error) => {
            return Err(DispatchStep::Terminate(format!(
                "observer end QPC failure: {error:?}"
            )));
        }
    };
    let drain_us = match drain_end.checked_duration_since(drain_start) {
        Ok(duration) => qpc_clock.duration_to_us(duration).unwrap_or_default(),
        Err(_) => 0,
    };
    if let Ok(wall_now_us) = qpc_clock.duration_to_us(DurationTicks::from_raw(drain_end.as_u64())) {
        super::super::update_deferred_worker_metrics(local_metrics, timing, wall_now_us);
    }
    super::super::health::observe_observer_health(
        drain_us,
        health.options.observer_warn_us,
        now_us,
        health.options.window_policy(),
        &mut health.observer_window,
        local_metrics,
    );
    if matches!(observation, DispatchObservation::Wait(_)) {
        publish_backend_metrics(backend, local_metrics, metrics, last_published_error);
        try_publish_metrics(local_metrics, metrics, now_us, false);
    }
    Ok(drain_us)
}
