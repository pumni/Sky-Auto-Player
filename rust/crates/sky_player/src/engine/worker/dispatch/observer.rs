use super::super::super::{
    DurationTicks, QpcClock, SharedMetrics, TelemetryCollector, TimelineTicks,
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
    DispatchObservation, DownObservation, OBSERVATION_QUEUE_CAPACITY, down_effective_ticks,
    down_observer_evidence, record_down_recovery_metrics, record_down_send_telemetry,
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
use std::sync::atomic::Ordering;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicU64, AtomicUsize};

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

struct DownPreCallEvidence {
    duration_us: Option<u64>,
    effective_ticks: Option<TimelineTicks>,
    policy_to_pre_call_us: Option<u64>,
    wake_to_pre_call_us: Option<u64>,
}

fn down_pre_call_evidence(
    observation: &DownObservation,
    qpc_clock: QpcClock,
) -> Result<DownPreCallEvidence, DispatchStep> {
    let duration_us = observation
        .pre_call_qpc
        .map(|pre_call| {
            let duration = observation
                .sendinput_completion_qpc
                .checked_duration_since(pre_call)
                .map_err(|error| {
                    DispatchStep::Terminate(format!("note-on QPC ordering failure: {error}"))
                })?;
            qpc_clock.duration_to_us(duration).map_err(|error| {
                DispatchStep::Terminate(format!("note-on duration conversion failure: {error:?}"))
            })
        })
        .transpose()?;
    let effective_ticks = observation
        .pre_call_qpc
        .map(|pre_call| down_effective_ticks(observation, pre_call))
        .transpose()?;
    let policy_to_pre_call_us = observation
        .final_policy_qpc
        .zip(observation.pre_call_qpc)
        .map(|(final_policy, pre_call)| {
            qpc_span_us(
                qpc_clock,
                final_policy,
                pre_call,
                "note-on final-policy-to-pre-call",
            )
        })
        .transpose()?;
    let wake_to_pre_call_us = observation
        .wake_qpc
        .zip(observation.pre_call_qpc)
        .map(|(wake, pre_call)| qpc_span_us(qpc_clock, wake, pre_call, "note-on wake-to-pre-call"))
        .transpose()?;
    Ok(DownPreCallEvidence {
        duration_us,
        effective_ticks,
        policy_to_pre_call_us,
        wake_to_pre_call_us,
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
            pre_call_qpc: Some(pre_call_qpc),
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
    pub fn push(&self, observation: DispatchObservation, dropped_samples: &mut u64) -> bool {
        if self.queue.push(observation).is_err() {
            *dropped_samples = dropped_samples.saturating_add(1);
            false
        } else {
            true
        }
    }
    pub fn push_wait(
        &self,
        observation: WaitObservation,
        dropped_samples: &mut u64,
        spin_dropped_samples: &mut u64,
    ) {
        if self
            .queue
            .push(DispatchObservation::Wait(observation))
            .is_err()
        {
            *dropped_samples = dropped_samples.saturating_add(1);
            *spin_dropped_samples = spin_dropped_samples.saturating_add(1);
        }
    }

    pub fn pop_front(&self) -> Option<DispatchObservation> {
        self.queue.pop()
    }
    fn drain_up_slot_record_count(&self) -> u64 {
        let mut count = 0u64;
        while let Some(observation) = self.pop_front() {
            if let DispatchObservation::Down(down) = observation {
                count = count.saturating_add(u64::from(down.up_slot_count));
            }
        }
        count
    }
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
    pending: PendingObservationQueue,
    telemetry: Arc<parking_lot::Mutex<TelemetryCollector>>,
}

pub(crate) struct ObserverOutput {
    pub(crate) metrics: WorkerMetricsLocal,
    pub(crate) terminal_error: Option<String>,
    pub(crate) up_slot_accounting_complete: bool,
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
        let runtime_pending = pending.clone();
        let runtime_telemetry = Arc::clone(&telemetry);
        let handle = std::thread::Builder::new()
            .name("sky-dispatch-observer".to_string())
            .spawn(move || {
                let mut local_metrics = WorkerMetricsLocal::default();
                let mut health = WorkerHealthState::new(health_options);
                let mut timing = timing;
                let mut terminal_error = None;
                let mut consumer_queue = consumer_queue;
                #[cfg(any(test, feature = "test-support"))]
                let mut completed_drains = 0usize;
                loop {
                    if consumer_queue.is_empty() {
                        if stop_thread.load(Ordering::Acquire) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    }
                    local_metrics.observer_queue_high_watermark = local_metrics
                        .observer_queue_high_watermark
                        .max(u64::try_from(consumer_queue.len()).unwrap_or(u64::MAX));
                    #[cfg(any(test, feature = "test-support"))]
                    if OBSERVER_ABORT_AFTER_DRAINS.load(Ordering::Relaxed) == completed_drains {
                        terminal_error = Some("injected observer consumer failure".to_string());
                        break;
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
                            #[cfg(any(test, feature = "test-support"))]
                            {
                                completed_drains = completed_drains.saturating_add(1);
                            }
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
                        #[cfg(any(test, feature = "test-support"))]
                        Err(DispatchStep::Dispatched) => {}
                    }
                }
                ObserverOutput {
                    metrics: local_metrics,
                    terminal_error,
                    up_slot_accounting_complete: true,
                }
            })?;
        Ok(Self {
            stop,
            handle: Some(handle),
            pending: runtime_pending,
            telemetry: runtime_telemetry,
        })
    }

    pub(crate) fn stop(mut self) -> ObserverOutput {
        self.stop.store(true, Ordering::Release);
        let output = self
            .handle
            .take()
            .expect("observer handle available")
            .join()
            .unwrap_or_else(|_| ObserverOutput {
                metrics: WorkerMetricsLocal::default(),
                terminal_error: Some("observer consumer panicked".to_string()),
                up_slot_accounting_complete: false,
            });
        let stranded_up_slots = self.pending.drain_up_slot_record_count();
        self.telemetry.lock().output.account_up_slot_loss(
            crate::engine::telemetry::UpSlotLossReason::ObserverAbort,
            stranded_up_slots,
        );
        output
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
static OBSERVER_ABORT_AFTER_DRAINS: AtomicUsize = AtomicUsize::new(usize::MAX);
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
    OBSERVER_ABORT_AFTER_DRAINS.store(usize::MAX, Ordering::SeqCst);
}
#[cfg(test)]
fn set_observer_abort_after_drains(count: usize) {
    OBSERVER_ABORT_AFTER_DRAINS.store(count, Ordering::SeqCst);
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
    let pre_call_evidence = down_pre_call_evidence(observation, qpc_clock)?;
    let pre_call_to_completion_us = pre_call_evidence.duration_us;
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
    let pre_call_effective_ticks = pre_call_evidence.effective_ticks;
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
    let final_policy_to_pre_call_us = pre_call_evidence.policy_to_pre_call_us;
    let wake_to_send_start_us = pre_call_evidence.wake_to_pre_call_us;
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
        pre_call_effective_ticks,
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
    if let Some(send_duration_us) = pre_call_to_completion_us {
        local_metrics.sendinput_duration_max_us = local_metrics
            .sendinput_duration_max_us
            .max(send_duration_us);
    }
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
    if let Some(send_duration_us) = pre_call_to_completion_us {
        observe_dispatch_health(
            DispatchHealthObservation {
                send_duration_us,
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
    }
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
    let up_slot_count = match &observation {
        DispatchObservation::Down(down) => u64::from(down.up_slot_count),
        _ => 0,
    };
    let accounted_before = telemetry
        .output
        .accounted_up_slot_records()
        .unwrap_or_default();
    let result = (|| {
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
            #[cfg(any(test, feature = "test-support"))]
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
        if let Ok(wall_now_us) =
            qpc_clock.duration_to_us(DurationTicks::from_raw(drain_end.as_u64()))
        {
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
        #[cfg(any(test, feature = "test-support"))]
        let is_immediate_publish = matches!(
            observation,
            DispatchObservation::Wait(_)
                | DispatchObservation::StaleMetadata(_)
                | DispatchObservation::DownMiss(_)
                | DispatchObservation::BlockedUnfocused(_)
        );
        #[cfg(not(any(test, feature = "test-support")))]
        let is_immediate_publish = matches!(observation, DispatchObservation::Wait(_));
        if is_immediate_publish {
            try_publish_observer_metrics(local_metrics, metrics, qpc_clock, now_us, false);
        }
        Ok(Some(drain_us))
    })();
    if result.is_err() && up_slot_count > 0 {
        let accounted_after = telemetry
            .output
            .accounted_up_slot_records()
            .unwrap_or(accounted_before);
        let accounted_during_drain = accounted_after
            .saturating_sub(accounted_before)
            .min(up_slot_count);
        telemetry.output.account_up_slot_loss(
            crate::engine::telemetry::UpSlotLossReason::ObserverAbort,
            up_slot_count.saturating_sub(accounted_during_drain),
        );
    }
    result
}

#[cfg(test)]
#[path = "observer_accounting_tests.rs"]
mod observer_accounting_tests;

#[cfg(test)]
#[path = "hold_forensics_tests.rs"]
mod hold_forensics_tests;

#[cfg(test)]
#[path = "dispatch_forensics_qualification_tests.rs"]
mod dispatch_forensics_qualification_tests;
