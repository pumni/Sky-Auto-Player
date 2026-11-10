#[cfg(any(test, feature = "test-support"))]
use super::super::super::TimelineTicks;
use super::super::super::{
    DurationTicks, QpcClock, RuntimeDispatchCoordinator, SharedMetrics, TelemetryCollector,
    try_publish_observer_metrics,
};
#[cfg(any(test, feature = "test-support"))]
use super::super::WorkerRuntime;
use super::super::health::build_dispatch_budget;
#[cfg(any(test, feature = "test-support"))]
use super::super::physical_timing_guard::PhysicalTimingWindow;
use super::super::wait::WaitObservation;
use super::super::{
    DispatchHealthObservation, DispatchHealthOptions, DispatchPath, WorkerHealthState,
    WorkerMetricsLocal, WorkerTimingState, observe_dispatch_health, record_lateness, signed_delta,
    signed_timeline_delta_ticks,
};
#[cfg(any(test, feature = "test-support"))]
use super::AuthoredBatchView;
use super::DispatchStep;
#[cfg(any(test, feature = "test-support"))]
use super::authored::resolve_slo_terminal_step;
#[cfg(any(test, feature = "test-support"))]
use super::observation::DownTraceObservation;
use super::observation::{
    DispatchObservation, DownObservation, OBSERVATION_QUEUE_CAPACITY, StaleMetadataObservation,
    down_effective_ticks, down_observer_evidence, record_down_recovery_metrics,
    record_down_send_telemetry,
};
#[cfg(any(test, feature = "test-support"))]
use super::observer_wake::take_deadline_wake_qpc;
#[cfg(any(test, feature = "test-support"))]
use super::recovery::record_physical_floor_delays;
#[cfg(any(test, feature = "test-support"))]
use super::timing::DownSendTiming;
use super::timing::is_clean_dispatch_observation;
use crossbeam_queue::ArrayQueue;
use sky_dispatch_core::time::QpcTicks;
use sky_dispatch_win32::input::PacketRetryReason;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

pub(super) fn qpc_span_us(
    qpc_clock: QpcClock,
    start: QpcTicks,
    end: QpcTicks,
    context: &'static str,
) -> Result<u64, DispatchStep> {
    let ticks = end.checked_duration_since(start).map_err(|error| {
        DispatchStep::Terminate(format!("{context} QPC ordering failure: {error:?}"))
    })?;
    qpc_clock.duration_to_us(ticks).map_err(|error| {
        DispatchStep::Terminate(format!("{context} duration conversion failure: {error:?}"))
    })
}
#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn publisher_down_send_outcome(
    view: &AuthoredBatchView,
    runtime: &mut WorkerRuntime,
    _health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    qpc_clock: QpcClock,
    _effective_now_ticks: TimelineTicks,
    physical_target_qpc: sky_dispatch_win32::clock::QpcTicks,
    physical_timing_window: PhysicalTimingWindow,
    capture_dispatch_ready_qpc: bool,
    trace_kind: u8,
    result_status: sky_dispatch_win32::input::SendTransactionStatus,
    result_confirmed_mask: u16,
    result_skipped_mask: u16,
    result_send_attempts: u8,
    result_retry_reason: PacketRetryReason,
    result_chord_integrity_lost: bool,
    result_last_win32_error: Option<u32>,
    observer: Option<&PendingObservationQueue>,
    timing_proof: &DownSendTiming,
) -> DispatchStep {
    let requested_packet = view.packet_masks;
    let final_policy_qpc = timing_proof.final_policy_qpc;
    let pre_call_qpc = timing_proof.pre_call_qpc;
    let sendinput_completion_qpc = timing_proof.sendinput_completion_qpc;
    let strict_completion_late = timing_proof.strict_completion_late;
    let completion_error_ticks_value = timing_proof.completion_error_ticks_value;
    let wake_qpc = take_deadline_wake_qpc(runtime);
    let precision_handoff = super::observer_wake::precision_handoff(
        runtime,
        wake_qpc,
        timing_proof.target_crossing_qpc,
        final_policy_qpc,
    );
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
    runtime.production_forensics.observe_packet_result(
        requested_packet,
        view.batch_source_action_index,
        physical_target_qpc,
        pre_call_qpc,
        sendinput_completion_qpc,
        result_status,
        local_metrics,
    );
    if matches!(
        result_status,
        sky_dispatch_win32::input::SendTransactionStatus::Complete
    ) && (physical_timing_window.hold_floor_mask != 0
        || physical_timing_window.release_floor_mask != 0)
    {
        record_physical_floor_delays(local_metrics, physical_timing_window);
    }
    if let Some(observer) = observer {
        let observation = DownObservation {
            wait_sequence: 0,
            wait_spin_ticks: 0,
            up_slots: None,
            up_slot_count: 0,
            up_pre_call_available: false,
            epoch_qpc: timing_proof.epoch_qpc,
            allow_pre_epoch_startup_dispatch: timing_proof.allow_pre_epoch_startup_dispatch,
            physical_target_qpc,
            physical_timing_window,
            final_policy_qpc,
            pre_call_qpc,
            sendinput_completion_qpc,
            previous_sendinput_completion_qpc: timing_proof.previous_sendinput_completion_qpc,
            previous_sender_completion_known: timing_proof.previous_sender_completion_known,
            dispatch_ready_qpc,
            wake_qpc,
            precision_handoff,
            requested_packet,
            confirmed_mask: result_confirmed_mask,
            skipped_mask: result_skipped_mask,
            trace: DownTraceObservation {
                event_index: view.batch_source_action_index,
                compiled_packet_index: u64::try_from(view.prepared_batch.packet_index).ok(),
                trace_kind,
                result_status,
                send_attempts: result_send_attempts,
                retry_reason: result_retry_reason,
                chord_integrity_lost: result_chord_integrity_lost,
                last_win32_error: result_last_win32_error.unwrap_or(0),
                authored_ticks: view.authored_batch_scheduled_ticks,
                effective_deadline_ticks: view.batch_scheduled_ticks,
            },
        };
        observer.push(
            DispatchObservation::Down(observation),
            &mut local_metrics.observer_dropped_samples,
            &mut local_metrics.observer_queue_high_watermark,
        );
        super::observer_wake::publish_pending_wait_observation(
            runtime,
            Some(observer),
            local_metrics,
        );
    }
    resolve_slo_terminal_step(
        result_chord_integrity_lost,
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
    observer: Option<&PendingObservationQueue>,
    dropped_samples: &mut u64,
    queue_high_watermark: &mut u64,
) -> DispatchStep {
    if let Err(error) = coordinator.commit_stale_packet(prepared) {
        return DispatchStep::Terminate(format!(
            "coordinator stale-packet commit failure: {error}"
        ));
    }
    if let Some(observer) = observer {
        observer.push(
            DispatchObservation::StaleMetadata(StaleMetadataObservation {
                source_action_index: prepared.source_action_index,
                effective_scheduled_ticks: prepared.effective_scheduled_ticks,
                suppressed_intent_count: prepared.suppressed_intent_count,
            }),
            dropped_samples,
            queue_high_watermark,
        );
    }
    DispatchStep::Dispatched
}
#[derive(Clone, Debug)]
pub struct PendingObservationQueue {
    queue: Arc<ArrayQueue<DispatchObservation>>,
    up_slot_pool: Arc<ArrayQueue<Box<[sky_dispatch_core::executor::UpSlotTiming; 15]>>>,
}
#[derive(Debug)]
pub struct PooledUpSlots {
    pub slots: Option<Box<[sky_dispatch_core::executor::UpSlotTiming; 15]>>,
    pool: Arc<ArrayQueue<Box<[sky_dispatch_core::executor::UpSlotTiming; 15]>>>,
}
impl Drop for PooledUpSlots {
    fn drop(&mut self) {
        if let Some(slots) = self.slots.take() {
            self.pool
                .push(slots)
                .expect("checked-out Up slot has pool capacity");
        }
    }
}
impl Default for PendingObservationQueue {
    fn default() -> Self {
        let up_slot_pool = Arc::new(ArrayQueue::new(OBSERVATION_QUEUE_CAPACITY + 2));
        for _ in 0..OBSERVATION_QUEUE_CAPACITY + 2 {
            assert!(
                up_slot_pool
                    .push(Box::new(
                        [sky_dispatch_core::executor::UpSlotTiming::EMPTY; 15]
                    ))
                    .is_ok()
            );
        }
        Self {
            queue: Arc::new(ArrayQueue::new(OBSERVATION_QUEUE_CAPACITY)),
            up_slot_pool,
        }
    }
}
impl PendingObservationQueue {
    pub fn acquire_up_slots(&self) -> Option<PooledUpSlots> {
        self.up_slot_pool.pop().map(|slots| PooledUpSlots {
            slots: Some(slots),
            pool: Arc::clone(&self.up_slot_pool),
        })
    }
    pub fn push(
        &self,
        observation: DispatchObservation,
        dropped_samples: &mut u64,
        high_watermark: &mut u64,
    ) {
        if self.queue.push(observation).is_err() {
            *dropped_samples = dropped_samples.saturating_add(1);
        } else {
            self.observe_occupancy(high_watermark);
        }
    }
    pub fn push_wait(
        &self,
        observation: WaitObservation,
        dropped_samples: &mut u64,
        high_watermark: &mut u64,
        spin_dropped_samples: &mut u64,
    ) {
        if self
            .queue
            .push(DispatchObservation::Wait(observation))
            .is_err()
        {
            *dropped_samples = dropped_samples.saturating_add(1);
            *spin_dropped_samples = spin_dropped_samples.saturating_add(1);
        } else {
            self.observe_occupancy(high_watermark);
        }
    }

    fn observe_occupancy(&self, high_watermark: &mut u64) {
        let observed = u64::try_from(self.queue.len()).unwrap_or(u64::MAX);
        *high_watermark = (*high_watermark).max(observed);
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

/// Dedicated consumer for diagnostic observations; all telemetry and health
/// work runs here, off the physical dispatch thread.
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
    ) -> std::io::Result<Self> {
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
                        Err(DispatchStep::TerminateStatic(error)) => {
                            terminal_error = Some((*error).to_string());
                            break;
                        }
                        #[cfg(any(test, feature = "test-support"))]
                        Err(DispatchStep::NoWork | DispatchStep::Continue) => {}
                        Err(DispatchStep::Dispatched) => {}
                    }
                }
                ObserverOutput {
                    metrics: local_metrics,
                    terminal_error,
                }
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
        })
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

impl Drop for ObserverRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
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
    }
}
#[cfg(any(test, feature = "test-support"))]
pub fn observer_test_hook_guard() -> ObserverTestHookGuard {
    let lock = OBSERVER_TEST_HOOK_LOCK.lock();
    reset_observer_test_hooks();
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
    health: &mut WorkerHealthState,
    local_metrics: &mut WorkerMetricsLocal,
    metrics: &SharedMetrics,
    telemetry: &mut TelemetryCollector,
    qpc_clock: QpcClock,
    now_us: u64,
    now_qpc_ticks: sky_dispatch_win32::clock::QpcTicks,
    timing: &WorkerTimingState,
) -> Result<(), DispatchStep> {
    let path = observation.path();
    let health_budget = build_dispatch_budget(path, health.options);
    let send_warn_us = health_budget.send_warn_us;
    let core_post_send_warn_us = health_budget.core_post_send_warn_us;
    let (_requested_count, _confirmed_count, _skipped_count, _observation_evidence) =
        down_observer_evidence(observation);
    let pre_call_to_completion_ticks = observation
        .sendinput_completion_qpc
        .checked_duration_since(observation.pre_call_qpc)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-on QPC ordering failure: {error}"))
        })?;
    let pre_call_to_completion_us = qpc_clock
        .duration_to_us(pre_call_to_completion_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-on duration conversion failure: {error:?}"))
        })?;
    let completion_residual_us = qpc_clock
        .duration_to_us(
            observation
                .sendinput_completion_qpc
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
    let pre_call_effective_ticks = down_effective_ticks(observation, observation.pre_call_qpc)?;
    let completed_effective_ticks =
        down_effective_ticks(observation, observation.sendinput_completion_qpc)?;
    let wake_ticks = observation
        .wake_qpc
        .map(|qpc| down_effective_ticks(observation, qpc))
        .transpose()?;
    let completed_effective_us = qpc_clock
        .timeline_to_us(completed_effective_ticks)
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
                .checked_duration_since(observation.sendinput_completion_qpc)
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
    let wake_to_final_policy_us = match (observation.wake_qpc, observation.final_policy_qpc) {
        (Some(wake), Some(final_policy)) => Some(qpc_span_us(
            qpc_clock,
            wake,
            final_policy,
            "note-on wake-to-final-policy",
        )?),
        _ => None,
    };
    let final_policy_to_pre_call_us = observation
        .final_policy_qpc
        .map(|final_policy| {
            qpc_span_us(
                qpc_clock,
                final_policy,
                observation.pre_call_qpc,
                "note-on final-policy-to-pre-call",
            )
        })
        .transpose()?;
    let wake_to_send_start_us = observation
        .wake_qpc
        .map(|wake| {
            qpc_span_us(
                qpc_clock,
                wake,
                observation.pre_call_qpc,
                "note-on wake-to-pre-call",
            )
        })
        .transpose()?;
    local_metrics.sendinput_warn_threshold_us = send_warn_us;
    local_metrics.core_post_send_warn_threshold_us = core_post_send_warn_us;
    local_metrics.post_send_metrics_available |= observation.dispatch_ready_qpc.is_some();
    match path {
        DispatchPath::DownOnly { .. } => {
            local_metrics.send_down_warn_threshold_us = send_warn_us;
        }
        DispatchPath::UpOnly { .. } => {
            local_metrics.send_up_warn_threshold_us = send_warn_us;
        }
        DispatchPath::Mixed { .. } => {
            local_metrics.send_mixed_warn_threshold_us = send_warn_us;
        }
    }
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
    let completion_lateness_ticks = signed_timeline_delta_ticks(
        completed_effective_ticks,
        observation.trace.effective_deadline_ticks,
    )
    .map_err(|error| {
        DispatchStep::Terminate(format!(
            "note-on observer completion conversion failure: {error}"
        ))
    })?;
    let recovered_partial_up = matches!(
        (path, observation.trace.retry_reason),
        (
            DispatchPath::UpOnly { .. },
            PacketRetryReason::PartialProgress { .. }
        )
    ) && observation.trace.result_success();
    let clean_dispatch_sample = is_clean_dispatch_observation(_observation_evidence);
    let strict_completion_late = timing.strict_timing
        && clean_dispatch_sample
        && completion_lateness_ticks > 0
        && (completion_lateness_ticks as u64
            > match path {
                DispatchPath::UpOnly { .. } => timing.strict_up_completion_late_ticks.as_u64(),
                DispatchPath::DownOnly { .. } | DispatchPath::Mixed { .. } => {
                    timing.strict_down_completion_late_ticks.as_u64()
                }
            });
    record_down_recovery_metrics(observation, recovered_partial_up, local_metrics);
    let completion_error_ticks = signed_timeline_delta_ticks(
        completed_effective_ticks,
        observation.trace.effective_deadline_ticks,
    )
    .map_err(|error| {
        DispatchStep::Terminate(format!(
            "note-on observer completion conversion failure: {error}"
        ))
    })?;
    let authored_completion_error_ticks =
        signed_timeline_delta_ticks(completed_effective_ticks, observation.trace.authored_ticks)
            .map_err(|error| {
                DispatchStep::Terminate(format!(
                    "note-on observer authored-completion conversion failure: {error}"
                ))
            })?;
    let (_, force_publish) = record_down_send_telemetry(
        observation,
        telemetry,
        core_post_send_us,
        completion_residual_us,
        observation.dispatch_ready_qpc.is_some(),
        completion_error_ticks,
        authored_completion_error_ticks,
        Some(pre_call_effective_ticks),
        Some(completed_effective_ticks),
        wake_ticks,
        now_qpc_ticks,
        recovered_partial_up,
        strict_completion_late,
    )?;
    local_metrics.core_post_send_max_us =
        local_metrics.core_post_send_max_us.max(core_post_send_us);
    if let Some(wake_to_send_us) = wake_to_send_start_us {
        local_metrics.wake_to_send_max_us = local_metrics.wake_to_send_max_us.max(wake_to_send_us);
    }
    local_metrics.sendinput_duration_max_us = local_metrics
        .sendinput_duration_max_us
        .max(pre_call_to_completion_us);
    if let Some(final_policy_to_pre_call_us) = final_policy_to_pre_call_us {
        local_metrics.final_policy_to_pre_call_max_us = local_metrics
            .final_policy_to_pre_call_max_us
            .max(final_policy_to_pre_call_us);
    }
    if let Some(wake_to_final_policy_us) = wake_to_final_policy_us {
        local_metrics.wake_to_final_policy_max_us = local_metrics
            .wake_to_final_policy_max_us
            .max(wake_to_final_policy_us);
    }
    record_lateness(
        signed_delta(completed_effective_us, authored_batch_scheduled_us),
        false,
        false,
        local_metrics,
    );
    observe_dispatch_health(
        DispatchHealthObservation {
            send_duration_us: pre_call_to_completion_us,
            post_send_duration_us: core_post_send_us,
            post_send_metrics_available: observation.dispatch_ready_qpc.is_some(),
            path,
            send_warn_us,
            core_post_send_warn_us,
            elapsed_us: completed_effective_us,
        },
        health.options.window_policy(),
        &mut health.sendinput_window,
        &mut health.core_post_send_window,
        local_metrics,
    );
    try_publish_observer_metrics(local_metrics, metrics, qpc_clock, now_us, force_publish);
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
            now_qpc_ticks,
            timing,
        )?,
        #[cfg(any(test, feature = "test-support"))]
        DispatchObservation::DownMiss(miss) => {
            super::observer_trace::drain_down_miss(miss, telemetry)?;
        }
        #[cfg(any(test, feature = "test-support"))]
        DispatchObservation::Up(up) => super::observer_trace::drain_up_send_outcome(
            up,
            super::observer_trace::UpObservationDrainContext {
                health,
                local_metrics,
                metrics,
                telemetry,
                qpc_clock,
                now_us,
            },
        )?,
        DispatchObservation::Wait(wait) => {
            super::observation::drain_wait_observation(wait, health, local_metrics, qpc_clock)?;
        }
        DispatchObservation::StaleMetadata(stale) => {
            super::observer_trace::drain_stale_metadata_observation(stale, telemetry)?;
        }
        #[cfg(any(test, feature = "test-support"))]
        DispatchObservation::BlockedUnfocused(blocked) => {
            super::observer_trace::drain_blocked_unfocused_observation(blocked, telemetry)?;
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
    #[cfg(not(any(test, feature = "test-support")))]
    let is_immediate_publish = matches!(
        observation,
        DispatchObservation::Wait(_) | DispatchObservation::StaleMetadata(_)
    );
    #[cfg(any(test, feature = "test-support"))]
    let is_immediate_publish = matches!(
        observation,
        DispatchObservation::Wait(_)
            | DispatchObservation::StaleMetadata(_)
            | DispatchObservation::DownMiss(_)
            | DispatchObservation::BlockedUnfocused(_)
    );
    if is_immediate_publish {
        try_publish_observer_metrics(local_metrics, metrics, qpc_clock, now_us, false);
    }
    Ok(Some(drain_us))
}

#[cfg(test)]
#[path = "hold_forensics_tests.rs"]
mod hold_forensics_tests;

#[cfg(test)]
#[path = "dispatch_forensics_qualification_tests.rs"]
mod dispatch_forensics_qualification_tests;
