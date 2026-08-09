use super::super::super::{
    DurationTicks, LatencyClass, PlaybackClockState, QpcClock, RtTraceRecord,
    RuntimeDispatchCoordinator, SendLatencyEstimator, SharedMetrics, TRACE_FLAG_ANOMALY,
    TRACE_KIND_DOWN, TRACE_KIND_UP, TelemetryCollector, TimelineTicks, TraceContext, TraceDelivery,
    TraceTiming, TrackedKeyState, trace_outcome_code, try_publish_metrics,
};
use super::super::{
    DispatchHealthObservation, DispatchPath, WorkerConfig, WorkerHealthState, WorkerMetricsLocal,
    WorkerRuntime, WorkerTimingState, observe_dispatch_health, publish_backend_metrics,
    record_lateness, record_lead_saturation, signed_delta, signed_ticks_to_us,
    update_estimator_after_send_class,
};
use super::authored::resolve_slo_terminal_step;
use super::observation::{
    DispatchObservation, DownObservation, DownTraceObservation, OBSERVATION_QUEUE_CAPACITY,
    UpObservation, record_down_send_telemetry, record_release_telemetry,
};
#[cfg(any(test, feature = "test-support"))]
use super::release::take_release_observer_failure;
use super::timing::{DownSendTiming, is_clean_estimator_observation, read_qpc_us};
use super::{AuthoredBatchView, DispatchStep};
use sky_dispatch_win32::input::PacketRetryReason;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicU64, Ordering};
pub(crate) fn take_deadline_wake_qpc(
    runtime: &mut WorkerRuntime,
    _sender_started_qpc: sky_dispatch_win32::clock::QpcTicks,
) -> Option<sky_dispatch_win32::clock::QpcTicks> {
    runtime.last_dispatch_deadline_wake_qpc.take()
}
#[cfg(test)]
mod wake_tests {
    use super::take_deadline_wake_qpc;
    use crate::engine::worker::WorkerRuntime;
    use sky_dispatch_win32::clock::QpcTicks;

    #[test]
    fn deadline_wake_is_consumed_by_only_one_observation() {
        let mut runtime = WorkerRuntime {
            last_dispatch_deadline_wake_qpc: Some(QpcTicks::from_raw(100)),
            ..WorkerRuntime::default()
        };
        assert_eq!(
            take_deadline_wake_qpc(&mut runtime, QpcTicks::from_raw(125)),
            Some(QpcTicks::from_raw(100))
        );
        assert_eq!(
            take_deadline_wake_qpc(&mut runtime, QpcTicks::from_raw(150)),
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
    result_confirmed_mask: u16,
    result_skipped_mask: u16,
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
        sender_duration_ticks,
        requested_count,
        delivered_count,
        completion_error_ticks_value,
        authored_completion_error_ticks_value,
        estimator_evidence,
        recovered_zero_progress,
        recovered_partial_up,
        recovered_retry_late,
        retry_late_abort,
        strict_completion_late,
        saturation_abort,
        ..
    } = *timing_proof;
    let wake_qpc = take_deadline_wake_qpc(runtime, sender_started_qpc);
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
    let (up_streak, down_streak) = match view.dispatch_path {
        DispatchPath::UpOnly { .. } => (timing_proof.saturation_streak, 0),
        DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
            (0, timing_proof.saturation_streak)
        }
    };
    health.up_saturation_positive_streak = up_streak;
    health.down_saturation_positive_streak = down_streak;
    let dispatch_ready_qpc = match qpc_clock.now() {
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
        sender_started_qpc,
        sender_completed_qpc,
        dispatch_ready_qpc,
        sender_duration_ticks,
        wake_qpc,
        delivered_count,
        batch_intent_count: view.batch_intent_count,
        completed_effective_ticks,
        estimator_evidence,
        send_warn_us: frozen_budget.send_warn_us,
        core_post_send_warn_us: frozen_budget.core_post_send_warn_us,
        trace: DownTraceObservation {
            event_index: view.batch_source_action_index,
            trace_kind,
            result_success,
            requested_count,
            sent_count: result_confirmed_mask.count_ones() as usize,
            skipped_count: result_skipped_mask.count_ones() as usize,
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
    let completion_error_us = match signed_ticks_to_us(qpc_clock, completion_error_ticks_value) {
        Ok(value) => value,
        Err(error) => {
            return DispatchStep::Terminate(format!(
                "note-on terminal timing conversion failure: {error}"
            ));
        }
    };
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
        self.entries[(self.head + self.len - 1) % self.entries.len()] = Some(observation);
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
    observer_guard_ticks: DurationTicks,
) -> bool {
    let Some(deadline) = deadline_ticks else {
        return true;
    };
    let Some(slack_ticks) = deadline
        .as_u64()
        .checked_sub(effective_now_ticks.as_u64())
        .map(DurationTicks::from_raw)
    else {
        return false;
    };
    slack_ticks >= observer_guard_ticks
}
#[cfg(any(test, feature = "test-support"))]
static OBSERVER_ARTIFICIAL_COST_US: AtomicU64 = AtomicU64::new(0);
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
pub fn reset_observer_test_hooks() {
    OBSERVER_ARTIFICIAL_COST_US.store(0, Ordering::SeqCst);
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
    let sender_duration_us = qpc_clock
        .duration_to_us(observation.sender_duration_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-on duration conversion failure: {error:?}"))
        })?;
    let completed_effective_us = qpc_clock
        .timeline_to_us(observation.completed_effective_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-on completion conversion failure: {error:?}"))
        })?;
    let batch_scheduled_us = qpc_clock
        .timeline_to_us(observation.trace.effective_deadline_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-on deadline conversion failure: {error:?}"))
        })?;
    let authored_batch_scheduled_us = qpc_clock
        .timeline_to_us(observation.trace.authored_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-on authored conversion failure: {error:?}"))
        })?;
    let completion_error_us =
        signed_ticks_to_us(qpc_clock, observation.trace.completion_error_ticks).map_err(
            |error| DispatchStep::Terminate(format!("note-on error conversion failure: {error:?}")),
        )?;
    let core_post_send_us = qpc_clock
        .duration_to_us(
            observation
                .dispatch_ready_qpc
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
    let wake_to_send_start_us = observation
        .wake_qpc
        .and_then(|wake| {
            observation
                .sender_started_qpc
                .checked_duration_since(wake)
                .ok()
        })
        .map(|ticks| qpc_clock.duration_to_us(ticks))
        .transpose()
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-on wake conversion failure: {error:?}"))
        })?;
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
    let (_, force_publish) = record_down_send_telemetry(observation, telemetry, core_post_send_us)?;
    local_metrics.core_post_send_max_us =
        local_metrics.core_post_send_max_us.max(core_post_send_us);
    if let Some(wake_to_send_us) = wake_to_send_start_us {
        local_metrics.wake_to_send_max_us = local_metrics.wake_to_send_max_us.max(wake_to_send_us);
    }
    if config.estimator.enable_adaptive_lead && observation.lead_down_saturated {
        match observation.path {
            DispatchPath::UpOnly { .. } => record_lead_saturation(
                &mut local_metrics.lead_saturation_count_up,
                &mut local_metrics.positive_residual_at_cap,
                observation.batch_intent_count,
                signed_delta(completed_effective_us, batch_scheduled_us),
            ),
            DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => record_lead_saturation(
                &mut local_metrics.lead_saturation_count_down,
                &mut local_metrics.positive_residual_at_cap,
                observation.batch_intent_count,
                signed_delta(completed_effective_us, batch_scheduled_us),
            ),
        }
    }
    if config.estimator.enable_adaptive_lead {
        let send_path = super::super::estimator_path_for_dispatch(observation.path);
        let _ = update_estimator_after_send_class(
            estimator,
            send_path,
            sender_duration_us,
            observation.delivered_count,
            observation.batch_intent_count,
            observation.lead_down,
            observation.lead_down_saturated,
            completion_error_us,
            observation.estimator_evidence,
            observation.latency_class,
        );
    }
    record_lateness(
        signed_delta(completed_effective_us, authored_batch_scheduled_us),
        false,
        false,
        local_metrics,
    );
    observe_dispatch_health(
        DispatchHealthObservation {
            send_duration_us: sender_duration_us,
            post_send_duration_us: core_post_send_us,
            path: observation.path,
            send_warn_us: observation.send_warn_us,
            core_post_send_warn_us: observation.core_post_send_warn_us,
            elapsed_us: completed_effective_us,
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
    let sender_duration_us = qpc_clock
        .duration_to_us(observation.sender_duration_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-off duration conversion failure: {error:?}"))
        })?;
    let completed_effective_us = qpc_clock
        .timeline_to_us(observation.completed_effective_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-off completion conversion failure: {error:?}"))
        })?;
    let scheduled_us = qpc_clock
        .timeline_to_us(observation.scheduled_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-off scheduled conversion failure: {error:?}"))
        })?;
    let deferred_by_us = qpc_clock
        .duration_to_us(observation.deferred_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-off deferral conversion failure: {error:?}"))
        })?;
    let up_completion_error_us =
        signed_ticks_to_us(qpc_clock, observation.up_completion_error_ticks).map_err(|error| {
            DispatchStep::Terminate(format!("note-off error conversion failure: {error:?}"))
        })?;
    let core_post_send_us = qpc_clock
        .duration_to_us(
            observation
                .dispatch_ready_qpc
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
    let wake_to_send_start_us = observation
        .wake_qpc
        .and_then(|wake| {
            observation
                .sender_started_qpc
                .checked_duration_since(wake)
                .ok()
        })
        .map(|ticks| qpc_clock.duration_to_us(ticks))
        .transpose()
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-off wake conversion failure: {error:?}"))
        })?;
    local_metrics.sendinput_warn_threshold_us = observation.send_warn_us;
    local_metrics.core_post_send_warn_threshold_us = observation.core_post_send_warn_us;
    local_metrics.send_up_warn_threshold_us = observation.send_warn_us;
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
    let lead_up = qpc_clock
        .duration_to_us(observation.lead_up_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-off lead conversion failure: {error:?}"))
        })?;
    record_release_telemetry(telemetry, observation, qpc_clock, core_post_send_us)?;
    if let Some(wake_to_send_us) = wake_to_send_start_us {
        local_metrics.wake_to_send_max_us = local_metrics.wake_to_send_max_us.max(wake_to_send_us);
    }
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
            up_completion_error_us,
        );
    }
    if config.estimator.enable_adaptive_lead {
        let _ = update_estimator_after_send_class(
            estimator,
            sky_dispatch_core::estimator::SendPath::UpOnly,
            sender_duration_us,
            observation.sent_count,
            observation.scan_count,
            lead_up,
            observation.lead_up_saturated,
            up_completion_error_us,
            observation.estimator_evidence,
            observation.latency_class,
        );
    }
    record_lateness(
        signed_delta(completed_effective_us, scheduled_us),
        true,
        deferred_by_us > 0,
        local_metrics,
    );
    observe_dispatch_health(
        DispatchHealthObservation {
            send_duration_us: sender_duration_us,
            post_send_duration_us: core_post_send_us,
            path: DispatchPath::UpOnly {
                up_count: observation.scan_count,
            },
            send_warn_us: observation.send_warn_us,
            core_post_send_warn_us: observation.core_post_send_warn_us,
            elapsed_us: completed_effective_us,
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
    now_qpc_ticks: sky_dispatch_win32::clock::QpcTicks,
    timing: &mut WorkerTimingState,
) -> Result<u64, DispatchStep> {
    let Some(observation) = pending.pop_front() else {
        return Ok(0);
    };
    let now_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(now_qpc_ticks.as_u64()))
        .map_err(|error| {
            DispatchStep::Terminate(format!("observer now conversion failure: {error:?}"))
        })?;
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
