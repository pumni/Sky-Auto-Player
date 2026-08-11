use super::super::super::{
    DurationTicks, QpcClock, RtTraceRecord, RuntimeDispatchCoordinator, SharedMetrics,
    TRACE_FLAG_ANOMALY, TRACE_KIND_DOWN, TRACE_KIND_UP, TelemetryCollector, TimelineTicks,
    TraceContext, TraceDelivery, TraceTiming, trace_outcome_code, try_publish_metrics,
};
use super::super::wait::WaitObservation;
use super::super::{
    DispatchHealthObservation, DispatchHealthOptions, DispatchPath, WorkerHealthState,
    WorkerMetricsLocal, WorkerRuntime, WorkerTimingState, observe_dispatch_health, record_lateness,
    signed_delta, signed_ticks_to_us,
};
use super::authored::resolve_slo_terminal_step;
use super::observation::{
    BlockedUnfocusedObservation, DispatchObservation, DownObservation, DownTraceObservation,
    OBSERVATION_QUEUE_CAPACITY, StaleMetadataObservation, UpObservation,
    record_down_send_telemetry, record_release_telemetry, up_dispatch_evidence,
    up_transport_counts,
};
#[cfg(any(test, feature = "test-support"))]
use super::release::take_release_observer_failure;
use super::timing::{DispatchObservationEvidence, DownSendTiming, is_clean_dispatch_observation};
use super::{AuthoredBatchView, DispatchStep};
use crossbeam_queue::ArrayQueue;
use sky_dispatch_win32::input::PacketRetryReason;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
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
    _health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    qpc_clock: QpcClock,
    effective_now_ticks: TimelineTicks,
    physical_target_qpc: sky_dispatch_win32::clock::QpcTicks,
    capture_dispatch_ready_qpc: bool,
    frozen_budget: &crate::engine::worker::health::FrozenDispatchBudget,
    trace_kind: u8,
    result_confirmed_mask: u16,
    result_skipped_mask: u16,
    result_send_attempts: u8,
    result_retry_reason: PacketRetryReason,
    result_chord_integrity_lost: bool,
    result_last_win32_error: Option<u32>,
    observer: &mut PendingObservationQueue,
    timing_proof: &DownSendTiming,
) -> DispatchStep {
    let Some(requested_packet) = view.packet_masks else {
        return DispatchStep::Terminate(
            "authored physical dispatch has no canonical packet evidence".to_string(),
        );
    };
    let DownSendTiming {
        sender_started_qpc,
        sender_completed_qpc,
        sender_started_effective_ticks,
        completed_effective_ticks,
        sender_duration_ticks,
        completion_error_ticks_value,
        authored_completion_error_ticks_value,
        recovered_partial_up,
        recovered_retry_late,
        retry_late_abort,
        strict_completion_late,
        ..
    } = *timing_proof;
    let wake_qpc = take_deadline_wake_qpc(runtime, sender_started_qpc);
    let dispatch_ready_qpc = if capture_dispatch_ready_qpc {
        match qpc_clock.now() {
            Ok(ticks) => Some(ticks),
            Err(error) => {
                return DispatchStep::Terminate(format!("QPC runtime failure: {error:?}"));
            }
        }
    } else {
        None
    };
    // HARD DISPATCH READY BOUNDARY:
    // physical/coordinator ownership is safe for the next dispatch.  From
    // here on, only a fixed raw observation enqueue and terminal policy may
    // run on this call stack.
    let observation = DownObservation {
        path: frozen_budget.path,
        physical_target_qpc,
        timeline_rebase_count: local_metrics.timeline_rebase_count,
        timeline_rebase_total_ticks: local_metrics.timeline_rebase_total_ticks,
        timeline_rebase_max_ticks: local_metrics.timeline_rebase_max_ticks,
        timeline_rebase_last_reason: local_metrics.timeline_rebase_last_reason,
        sender_started_qpc,
        sender_completed_qpc,
        dispatch_ready_qpc,
        sender_duration_ticks,
        wake_qpc,
        requested_packet,
        confirmed_mask: result_confirmed_mask,
        skipped_mask: result_skipped_mask,
        completed_effective_ticks,
        send_warn_us: frozen_budget.send_warn_us,
        core_post_send_warn_us: frozen_budget.core_post_send_warn_us,
        trace: DownTraceObservation {
            event_index: view.batch_source_action_index,
            trace_kind,
            result_status: timing_proof.observation_evidence.status,
            send_attempts: result_send_attempts,
            retry_reason: result_retry_reason,
            chord_integrity_lost: result_chord_integrity_lost,
            last_win32_error: result_last_win32_error.unwrap_or(0),
            authored_ticks: view.authored_batch_scheduled_ticks,
            effective_deadline_ticks: view.batch_scheduled_ticks,
            wake_ticks: effective_now_ticks,
            sender_started_ticks: Some(sender_started_effective_ticks),
            sender_completed_ticks: Some(completed_effective_ticks),
            dispatch_start_error_ticks: timing_proof.dispatch_start_error_ticks,
            completion_error_ticks: completion_error_ticks_value,
            authored_completion_error_ticks: authored_completion_error_ticks_value,
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
        false,
        qpc_clock,
        completion_error_ticks_value,
        view,
        runtime,
    )
}
pub(crate) fn dispatch_stale_packet(
    prepared: sky_dispatch_core::coordinator::PreparedStalePacket,
    coordinator: &mut RuntimeDispatchCoordinator,
    observer: &PendingObservationQueue,
    dropped_samples: &mut u64,
    queue_high_watermark: &mut u64,
    effective_now_ticks: TimelineTicks,
) -> DispatchStep {
    if let Err(error) = coordinator.commit_stale_packet(prepared) {
        return DispatchStep::Terminate(format!(
            "coordinator stale-packet commit failure: {error}"
        ));
    }
    observer.push(
        DispatchObservation::StaleMetadata(StaleMetadataObservation {
            source_action_index: prepared.source_action_index,
            effective_scheduled_ticks: prepared.effective_scheduled_ticks,
            effective_now_ticks,
            suppressed_intent_count: prepared.suppressed_intent_count,
        }),
        dropped_samples,
        queue_high_watermark,
    );
    DispatchStep::Dispatched
}
fn drain_stale_metadata_observation(
    observation: &StaleMetadataObservation,
    telemetry: &mut TelemetryCollector,
) -> Result<(), DispatchStep> {
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: observation.source_action_index,
                kind: TRACE_KIND_UP,
                outcome: trace_outcome_code("suppressed_stale_up"),
                polyphony: observation.suppressed_intent_count,
                flags: TRACE_FLAG_ANOMALY,
                win32_error: 0,
            },
            TraceTiming {
                authored_ticks: observation.effective_scheduled_ticks,
                effective_deadline_ticks: observation.effective_scheduled_ticks,
                wake_ticks: observation.effective_now_ticks,
                send_started_ticks: None,
                send_completed_ticks: None,
                completion_residual_us: 0,
                core_post_send_duration_us: 0,
                post_send_metrics_available: false,
                dispatch_start_error_ticks: 0,
                completion_error_ticks: 0,
                authored_completion_error_ticks: 0,
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

fn drain_blocked_unfocused_observation(
    observation: &BlockedUnfocusedObservation,
    telemetry: &mut TelemetryCollector,
) -> Result<(), DispatchStep> {
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: observation.event_index,
                kind: TRACE_KIND_DOWN,
                outcome: trace_outcome_code("blocked_unfocused"),
                polyphony: observation.polyphony,
                flags: TRACE_FLAG_ANOMALY,
                win32_error: 0,
            },
            TraceTiming {
                authored_ticks: observation.authored_ticks,
                effective_deadline_ticks: observation.effective_deadline_ticks,
                wake_ticks: observation.effective_now_ticks,
                send_started_ticks: None,
                send_completed_ticks: None,
                completion_residual_us: 0,
                core_post_send_duration_us: 0,
                post_send_metrics_available: false,
                dispatch_start_error_ticks: 0,
                completion_error_ticks: 0,
                authored_completion_error_ticks: 0,
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
#[derive(Clone, Debug)]
pub struct PendingObservationQueue {
    queue: Arc<ArrayQueue<DispatchObservation>>,
}
impl Default for PendingObservationQueue {
    fn default() -> Self {
        Self {
            queue: Arc::new(ArrayQueue::new(OBSERVATION_QUEUE_CAPACITY)),
        }
    }
}
impl PendingObservationQueue {
    pub fn push(
        &self,
        observation: DispatchObservation,
        dropped_samples: &mut u64,
        high_watermark: &mut u64,
    ) {
        if self.queue.push(observation).is_err() {
            *dropped_samples = dropped_samples.saturating_add(1);
        }
        *high_watermark = (*high_watermark).max(self.queue.len() as u64);
    }
    pub fn push_wait(
        &self,
        observation: WaitObservation,
        dropped_samples: &mut u64,
        high_watermark: &mut u64,
    ) {
        if self
            .queue
            .push(DispatchObservation::Wait(observation))
            .is_err()
        {
            *dropped_samples = dropped_samples.saturating_add(1);
        }
        *high_watermark = (*high_watermark).max(self.queue.len() as u64);
    }
    pub fn pop_front(&self) -> Option<DispatchObservation> {
        self.queue.pop()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn len(&self) -> usize {
        self.queue.len()
    }
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

/// Dedicated consumer for deferred dispatch observations. The producer side
/// only performs a bounded `ArrayQueue::push`; all telemetry and health work
/// runs here, off the physical dispatch thread.
pub(crate) struct ObserverRuntime {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<ObserverOutput>>,
}

pub(crate) struct ObserverOutput {
    pub(crate) metrics: WorkerMetricsLocal,
    pub(crate) terminal_error: Option<String>,
}

impl ObserverRuntime {
    pub(crate) fn start(
        pending: PendingObservationQueue,
        qpc_clock: QpcClock,
        shared_metrics: Arc<SharedMetrics>,
        telemetry: Arc<parking_lot::Mutex<TelemetryCollector>>,
        timing: WorkerTimingState,
        health_options: DispatchHealthOptions,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let consumer_queue = pending.clone();
        let handle = std::thread::Builder::new()
            .name("sky-dispatch-observer".to_string())
            .spawn(move || {
                let mut local_metrics = WorkerMetricsLocal::default();
                let mut health = WorkerHealthState::new(health_options);
                let mut timing = timing;
                let mut terminal_error = None;
                let mut consumer_queue = consumer_queue;
                loop {
                    if consumer_queue.is_empty() {
                        if stop_thread.load(Ordering::Acquire) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    }
                    let now_qpc_ticks = match qpc_clock.now() {
                        Ok(ticks) => ticks,
                        Err(error) => {
                            terminal_error =
                                Some(format!("observer consumer QPC failure: {error:?}"));
                            break;
                        }
                    };
                    match drain_one_observer(
                        &mut consumer_queue,
                        &mut health,
                        &mut local_metrics,
                        shared_metrics.as_ref(),
                        &mut telemetry.lock(),
                        qpc_clock,
                        now_qpc_ticks,
                        &mut timing,
                    ) {
                        Ok(Some(drain_us)) => {
                            local_metrics.observer_duration_max_us =
                                local_metrics.observer_duration_max_us.max(drain_us);
                        }
                        Ok(None) => {}
                        Err(DispatchStep::Terminate(error)) => {
                            terminal_error = Some(error);
                            break;
                        }
                        Err(DispatchStep::NoWork | DispatchStep::Continue) => {}
                        Err(DispatchStep::Dispatched) => {}
                    }
                }
                ObserverOutput {
                    metrics: local_metrics,
                    terminal_error,
                }
            })
            .expect("observer thread must start");
        Self {
            stop,
            handle: Some(handle),
        }
    }

    pub(crate) fn stop(mut self) -> ObserverOutput {
        self.stop.store(true, Ordering::Release);
        self.handle
            .take()
            .expect("observer handle available")
            .join()
            .unwrap_or_else(|_| ObserverOutput {
                metrics: WorkerMetricsLocal::default(),
                terminal_error: Some("observer consumer panicked".to_string()),
            })
    }
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

fn down_observer_evidence(
    observation: &DownObservation,
) -> (usize, usize, usize, DispatchObservationEvidence) {
    let counts = (
        observation.requested_count(),
        observation.confirmed_count(),
        observation.skipped_count(),
    );
    let evidence = DispatchObservationEvidence {
        status: observation.trace.result_status,
        attempts: observation.trace.send_attempts,
        retry_reason: observation.trace.retry_reason,
        requested_count: counts.0,
        confirmed_count: counts.1,
        skipped_count: counts.2,
        timing_valid: true,
        transport_anomaly: observation.trace.last_win32_error != 0,
        recovery_used: !matches!(observation.trace.retry_reason, PacketRetryReason::None),
        chord_integrity_lost: observation.trace.chord_integrity_lost,
    };
    (counts.0, counts.1, counts.2, evidence)
}

fn record_down_recovery_metrics(
    observation: &DownObservation,
    local_metrics: &mut WorkerMetricsLocal,
) {
    if observation.trace.recovered_retry_late {
        local_metrics.recovered_zero_progress_but_late = local_metrics
            .recovered_zero_progress_but_late
            .saturating_add(1);
    }
    if matches!(
        observation.trace.retry_reason,
        PacketRetryReason::ZeroProgress
    ) {
        local_metrics.recovered_zero_progress_retries = local_metrics
            .recovered_zero_progress_retries
            .saturating_add(1);
    }
    if observation.trace.recovered_partial_up {
        local_metrics.recovered_partial_up_retries =
            local_metrics.recovered_partial_up_retries.saturating_add(1);
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_down_send_outcome(
    observation: &DownObservation,
    health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    metrics: &SharedMetrics,
    telemetry: &mut TelemetryCollector,
    qpc_clock: QpcClock,
    now_us: u64,
) -> Result<(), DispatchStep> {
    let (_requested_count, _confirmed_count, _skipped_count, _observation_evidence) =
        down_observer_evidence(observation);
    let sender_duration_us = qpc_clock
        .duration_to_us(observation.sender_duration_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-on duration conversion failure: {error:?}"))
        })?;
    let completion_residual_us = qpc_clock
        .duration_to_us(
            observation
                .sender_completed_qpc
                .checked_duration_since(observation.physical_target_qpc)
                .map_err(|error| {
                    DispatchStep::Terminate(format!(
                        "note-on completion precedes physical target: {error:?}"
                    ))
                })?,
        )
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "note-on completion-residual conversion failure: {error:?}"
            ))
        })?;
    let completed_effective_us = qpc_clock
        .timeline_to_us(observation.completed_effective_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-on completion conversion failure: {error:?}"))
        })?;
    let _batch_scheduled_us = qpc_clock
        .timeline_to_us(observation.trace.effective_deadline_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-on deadline conversion failure: {error:?}"))
        })?;
    let authored_batch_scheduled_us = qpc_clock
        .timeline_to_us(observation.trace.authored_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-on authored conversion failure: {error:?}"))
        })?;
    let core_post_send_us = observation
        .dispatch_ready_qpc
        .map(|ready| {
            ready
                .checked_duration_since(observation.sender_completed_qpc)
                .map_err(|error| {
                    DispatchStep::Terminate(format!(
                        "note-on observer QPC ordering failure: {error:?}"
                    ))
                })
                .and_then(|ticks| {
                    qpc_clock.duration_to_us(ticks).map_err(|error| {
                        DispatchStep::Terminate(format!(
                            "note-on observer post-send conversion failure: {error:?}"
                        ))
                    })
                })
        })
        .transpose()?
        .unwrap_or(0);
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
    local_metrics.post_send_metrics_available |= observation.dispatch_ready_qpc.is_some();
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
    record_down_recovery_metrics(observation, local_metrics);
    let (_, force_publish) = record_down_send_telemetry(
        observation,
        telemetry,
        core_post_send_us,
        completion_residual_us,
        observation.dispatch_ready_qpc.is_some(),
    )?;
    local_metrics.core_post_send_max_us =
        local_metrics.core_post_send_max_us.max(core_post_send_us);
    if let Some(wake_to_send_us) = wake_to_send_start_us {
        local_metrics.wake_to_send_max_us = local_metrics.wake_to_send_max_us.max(wake_to_send_us);
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
            post_send_metrics_available: observation.dispatch_ready_qpc.is_some(),
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
    try_publish_metrics(local_metrics, metrics, now_us, force_publish);
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_up_send_outcome(
    observation: &UpObservation,
    health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    metrics: &SharedMetrics,
    telemetry: &mut TelemetryCollector,
    qpc_clock: QpcClock,
    now_us: u64,
) -> Result<(), DispatchStep> {
    let (scan_count, _sent_count, _skipped_count) = up_transport_counts(observation);
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
    let completion_residual_us = qpc_clock
        .duration_to_us(
            observation
                .sender_completed_qpc
                .checked_duration_since(observation.physical_target_qpc)
                .map_err(|error| {
                    DispatchStep::Terminate(format!(
                        "note-off completion precedes physical target: {error:?}"
                    ))
                })?,
        )
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "note-off completion-residual conversion failure: {error:?}"
            ))
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
    let _up_completion_error_us =
        signed_ticks_to_us(qpc_clock, observation.up_completion_error_ticks).map_err(|error| {
            DispatchStep::Terminate(format!("note-off error conversion failure: {error:?}"))
        })?;
    let core_post_send_us = observation
        .dispatch_ready_qpc
        .map(|ready| {
            ready
                .checked_duration_since(observation.sender_completed_qpc)
                .map_err(|error| {
                    DispatchStep::Terminate(format!(
                        "note-off observer QPC ordering failure: {error:?}"
                    ))
                })
                .and_then(|ticks| {
                    qpc_clock.duration_to_us(ticks).map_err(|error| {
                        DispatchStep::Terminate(format!(
                            "note-off observer post-send conversion failure: {error:?}"
                        ))
                    })
                })
        })
        .transpose()?
        .unwrap_or(0);
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
    local_metrics.post_send_metrics_available |= observation.dispatch_ready_qpc.is_some();
    local_metrics.send_up_warn_threshold_us = observation.send_warn_us;
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
    record_release_telemetry(
        telemetry,
        observation,
        qpc_clock,
        core_post_send_us,
        completion_residual_us,
        observation.dispatch_ready_qpc.is_some(),
    )?;
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
            post_send_metrics_available: observation.dispatch_ready_qpc.is_some(),
            path: DispatchPath::UpOnly {
                up_count: scan_count,
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
    try_publish_metrics(
        local_metrics,
        metrics,
        now_us,
        !is_clean_dispatch_observation(up_dispatch_evidence(observation))
            || observation.trace.recovery_required,
    );
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn drain_one_observer(
    pending: &mut PendingObservationQueue,
    health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    metrics: &SharedMetrics,
    telemetry: &mut TelemetryCollector,
    qpc_clock: QpcClock,
    now_qpc_ticks: sky_dispatch_win32::clock::QpcTicks,
    timing: &mut WorkerTimingState,
) -> Result<Option<u64>, DispatchStep> {
    let Some(observation) = pending.pop_front() else {
        return Ok(None);
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
            health,
            local_metrics,
            metrics,
            telemetry,
            qpc_clock,
            now_us,
        )?,
        DispatchObservation::Up(up) => drain_up_send_outcome(
            up,
            health,
            local_metrics,
            metrics,
            telemetry,
            qpc_clock,
            now_us,
        )?,
        DispatchObservation::Wait(wait) => {
            super::observation::drain_wait_observation(wait, health, local_metrics, qpc_clock)?;
        }
        DispatchObservation::StaleMetadata(stale) => {
            drain_stale_metadata_observation(stale, telemetry)?;
        }
        DispatchObservation::BlockedUnfocused(blocked) => {
            drain_blocked_unfocused_observation(blocked, telemetry)?;
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
    if matches!(
        observation,
        DispatchObservation::Wait(_)
            | DispatchObservation::StaleMetadata(_)
            | DispatchObservation::BlockedUnfocused(_)
    ) {
        try_publish_metrics(local_metrics, metrics, now_us, false);
    }
    Ok(Some(drain_us))
}
