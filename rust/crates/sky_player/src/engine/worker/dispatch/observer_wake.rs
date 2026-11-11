use super::super::WorkerRuntime;
use super::super::wait::{WaitBoundary, WaitObservation};
use super::observation::PrecisionHandoffEvidence;
use super::observer::PendingObservationQueue;
use crate::engine::telemetry::WorkerMetricsLocal;
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::wait::{HybridWaiter, WaitOutcome};

impl WaitBoundary {
    pub(crate) fn wake_qpc(&self) -> Option<QpcTicks> {
        match self {
            Self::Due { wait_result, .. } => {
                wait_result.as_ref().and_then(|result| result.wake_qpc)
            }
            Self::Interrupted(wait_result, _, _) => wait_result.wake_qpc,
            Self::Exit => None,
        }
    }
}

pub(crate) fn wait_for_metadata_only_deadline(
    metadata_target_qpc: QpcTicks,
    qpc_clock: QpcClock,
    waiter: &HybridWaiter,
    interrupt: &OwnedEvent,
    local_metrics: &mut WorkerMetricsLocal,
    terminal_error: &mut Option<String>,
) -> WaitBoundary {
    let wait_result = waiter.wait_until_ticks_with_metrics_typed(
        qpc_clock,
        metadata_target_qpc,
        DurationTicks::ZERO,
        interrupt,
    );
    match wait_result.outcome {
        WaitOutcome::Deadline => WaitBoundary::Due {
            dispatch_qpc: wait_result.wake_qpc.unwrap_or(metadata_target_qpc),
            planned_wait_ticks: DurationTicks::ZERO,
            wait_result: Some(wait_result),
            target_qpc: metadata_target_qpc,
        },
        WaitOutcome::Failed(failure) => {
            super::super::wait::record_wait_failure(failure, local_metrics, terminal_error);
            WaitBoundary::Exit
        }
        WaitOutcome::Interrupted => {
            local_metrics.wait_interrupted_count =
                local_metrics.wait_interrupted_count.saturating_add(1);
            WaitBoundary::Interrupted(wait_result, metadata_target_qpc, DurationTicks::ZERO)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn publish_interrupted_physical_wait(
    runtime: &mut WorkerRuntime,
    observer: Option<&PendingObservationQueue>,
    local_metrics: &mut WorkerMetricsLocal,
    wait_result: sky_dispatch_win32::wait::WaitResult,
    target_qpc: QpcTicks,
    planned_wait_ticks: DurationTicks,
    deadline_ticks: TimelineTicks,
    epoch_qpc: QpcTicks,
) {
    publish_pending_wait_observation(runtime, observer, local_metrics);
    let observation = record_wait_attempt(
        observer,
        Some(wait_result),
        target_qpc,
        planned_wait_ticks,
        deadline_ticks,
        epoch_qpc,
        None,
        local_metrics,
    );
    #[cfg(any(test, feature = "test-support"))]
    if let (Some(hook), Some(observation)) =
        (runtime.startup_ordering_hook.as_ref(), observation.as_ref())
    {
        hook.record_interrupted_wait(crate::engine::config::InterruptedWaitEvidence {
            sequence: observation.sequence,
            outcome: observation.outcome,
            spin_ticks: observation.spin_ticks.as_u64(),
            physical_target_qpc_ticks: observation.physical_target_qpc.as_u64(),
            planned_wait_ticks: observation.planned_wait_ticks.as_u64(),
            wake_qpc_ticks: observation.wake_qpc.map(|ticks| ticks.as_u64()),
            deadline_crossing_qpc_ticks: observation
                .deadline_crossing_qpc
                .map(|ticks| ticks.as_u64()),
            waiter_return_qpc_ticks: observation.waiter_return_qpc.map(|ticks| ticks.as_u64()),
        });
    }
    runtime.pending_wait_observation = observation;
    publish_pending_wait_observation(runtime, observer, local_metrics);
}

pub(crate) fn precision_handoff(
    runtime: &WorkerRuntime,
    admission_wake_qpc: Option<QpcTicks>,
    target_crossing_qpc: Option<QpcTicks>,
    final_policy_qpc: Option<QpcTicks>,
) -> Option<PrecisionHandoffEvidence> {
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = runtime;
    #[cfg(any(test, feature = "test-support"))]
    let pending_wait = runtime.pending_wait_observation;
    #[cfg(any(test, feature = "test-support"))]
    let waiter_deadline_crossing_qpc = pending_wait.and_then(|wait| wait.deadline_crossing_qpc);
    #[cfg(any(test, feature = "test-support"))]
    let timer_arm_qpc = pending_wait.and_then(|wait| wait.timer_arm_qpc);
    #[cfg(any(test, feature = "test-support"))]
    let timer_arm_target_qpc = pending_wait.and_then(|wait| wait.timer_arm_target_qpc);
    #[cfg(any(test, feature = "test-support"))]
    let timer_arm_generation = pending_wait.and_then(|wait| wait.timer_arm_generation);
    #[cfg(any(test, feature = "test-support"))]
    let timer_return_generation = pending_wait.and_then(|wait| wait.timer_return_generation);
    #[cfg(any(test, feature = "test-support"))]
    let timer_return_qpc = pending_wait.and_then(|wait| wait.timer_return_qpc);
    #[cfg(any(test, feature = "test-support"))]
    let waiter_return_qpc = pending_wait.and_then(|wait| wait.waiter_return_qpc);
    let base_handoff_available =
        admission_wake_qpc.is_some() || target_crossing_qpc.is_some() || final_policy_qpc.is_some();
    #[cfg(any(test, feature = "test-support"))]
    let has_precision_handoff = base_handoff_available
        || waiter_deadline_crossing_qpc.is_some()
        || timer_arm_qpc.is_some()
        || timer_arm_target_qpc.is_some()
        || timer_arm_generation.is_some()
        || timer_return_generation.is_some()
        || timer_return_qpc.is_some()
        || waiter_return_qpc.is_some();
    #[cfg(not(any(test, feature = "test-support")))]
    let has_precision_handoff = base_handoff_available;
    has_precision_handoff.then_some(PrecisionHandoffEvidence {
        admission_wake_qpc,
        target_crossing_qpc,
        final_policy_qpc,
        #[cfg(any(test, feature = "test-support"))]
        waiter_deadline_crossing_qpc,
        #[cfg(any(test, feature = "test-support"))]
        timer_arm_qpc,
        #[cfg(any(test, feature = "test-support"))]
        timer_arm_target_qpc,
        #[cfg(any(test, feature = "test-support"))]
        timer_arm_generation,
        #[cfg(any(test, feature = "test-support"))]
        timer_return_generation,
        #[cfg(any(test, feature = "test-support"))]
        timer_return_qpc,
        #[cfg(any(test, feature = "test-support"))]
        waiter_return_qpc,
    })
}

pub(crate) fn take_deadline_wake_qpc(runtime: &mut WorkerRuntime) -> Option<QpcTicks> {
    runtime.last_dispatch_deadline_wake_qpc.take()
}

/// Capture a real wait result before control transitions or another plan can
/// replace its deadline attribution. Queue insertion happens after dispatch,
/// except interrupted replans, which have no following dispatch attempt. An
/// already-due target has no wait result; failed waits are not observer samples.
#[allow(clippy::too_many_arguments)]
pub(crate) fn record_wait_attempt(
    observer: Option<&PendingObservationQueue>,
    wait_result: Option<sky_dispatch_win32::wait::WaitResult>,
    physical_target_qpc: sky_dispatch_win32::clock::QpcTicks,
    planned_wait_ticks: DurationTicks,
    deadline_ticks: TimelineTicks,
    epoch_qpc: sky_dispatch_win32::clock::QpcTicks,
    _waiter_return_qpc: Option<QpcTicks>,
    local_metrics: &mut WorkerMetricsLocal,
) -> Option<WaitObservation> {
    let result = wait_result?;
    if matches!(
        result.outcome,
        sky_dispatch_win32::wait::WaitOutcome::Failed(_)
    ) {
        return None;
    }
    local_metrics.spin_observation_expected_samples = local_metrics
        .spin_observation_expected_samples
        .saturating_add(1);
    if observer.is_none() {
        local_metrics.spin_observation_missing_samples = local_metrics
            .spin_observation_missing_samples
            .saturating_add(1);
        return None;
    }
    let observation = WaitObservation {
        sequence: local_metrics.spin_observation_expected_samples,
        outcome: result.outcome,
        wake_qpc: result.wake_qpc,
        spin_ticks: result.spin_ticks,
        physical_target_qpc,
        planned_wait_ticks,
        deadline_ticks,
        epoch_qpc,
        allow_pre_epoch_startup_dispatch: true,
        #[cfg(any(test, feature = "test-support"))]
        timer_arm_qpc: result.timer_arm_qpc,
        #[cfg(any(test, feature = "test-support"))]
        timer_arm_target_qpc: result.timer_arm_target_qpc,
        #[cfg(any(test, feature = "test-support"))]
        timer_arm_generation: result.timer_arm_generation,
        #[cfg(any(test, feature = "test-support"))]
        timer_return_generation: result.timer_return_generation,
        #[cfg(any(test, feature = "test-support"))]
        timer_return_qpc: result.timer_return_qpc,
        #[cfg(any(test, feature = "test-support"))]
        deadline_crossing_qpc: (result.outcome == sky_dispatch_win32::wait::WaitOutcome::Deadline)
            .then_some(result.wake_qpc)
            .flatten(),
        #[cfg(any(test, feature = "test-support"))]
        waiter_return_qpc: _waiter_return_qpc,
    };
    Some(observation)
}

pub(crate) fn publish_pending_wait_observation(
    runtime: &mut WorkerRuntime,
    observer: Option<&PendingObservationQueue>,
    local_metrics: &mut WorkerMetricsLocal,
) {
    let Some(observation) = runtime.pending_wait_observation.take() else {
        return;
    };
    let Some(observer) = observer else {
        local_metrics.spin_observation_missing_samples = local_metrics
            .spin_observation_missing_samples
            .saturating_add(1);
        return;
    };
    observer.push_wait(
        observation,
        &mut local_metrics.observer_dropped_samples,
        &mut local_metrics.spin_observation_dropped_samples,
    );
}

#[cfg(test)]
mod tests {
    use super::super::observation::{
        DispatchObservation, OBSERVATION_QUEUE_CAPACITY, StaleMetadataObservation,
    };
    use super::super::observer::PendingObservationQueue;
    use super::{publish_pending_wait_observation, record_wait_attempt, take_deadline_wake_qpc};
    use crate::engine::telemetry::WorkerMetricsLocal;
    use crate::engine::worker::WorkerRuntime;
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
    use sky_dispatch_win32::clock::QpcTicks;
    use sky_dispatch_win32::wait::{WaitOutcome, WaitResult};

    #[test]
    fn deadline_wake_is_consumed_by_only_one_observation() {
        let mut runtime = WorkerRuntime {
            last_dispatch_deadline_wake_qpc: Some(QpcTicks::from_raw(100)),
            ..WorkerRuntime::default()
        };
        assert_eq!(
            take_deadline_wake_qpc(&mut runtime),
            Some(QpcTicks::from_raw(100))
        );
        assert_eq!(take_deadline_wake_qpc(&mut runtime), None);
    }

    #[test]
    fn queue_pressure_counts_drops_without_producer_occupancy_sampling() {
        let queue = PendingObservationQueue::default();
        let mut dropped = 0;

        for index in 0..OBSERVATION_QUEUE_CAPACITY {
            queue.push(
                DispatchObservation::StaleMetadata(StaleMetadataObservation {
                    source_action_index: u32::try_from(index).unwrap_or(u32::MAX),
                    effective_scheduled_ticks: TimelineTicks::from_raw(index as u64),
                    suppressed_intent_count: 1,
                }),
                &mut dropped,
            );
        }

        assert_eq!(queue.len(), OBSERVATION_QUEUE_CAPACITY);
        assert_eq!(dropped, 0);

        let mut wait_metrics = WorkerMetricsLocal::default();
        let observation = record_wait_attempt(
            Some(&queue),
            Some(WaitResult::interrupted()),
            QpcTicks::from_raw(31_000),
            DurationTicks::from_raw(9_000),
            TimelineTicks::from_raw(22_000),
            QpcTicks::from_raw(10_000),
            None,
            &mut wait_metrics,
        )
        .expect("wait observation should be captured before handoff");
        let mut runtime = WorkerRuntime {
            pending_wait_observation: Some(observation),
            ..WorkerRuntime::default()
        };
        publish_pending_wait_observation(&mut runtime, Some(&queue), &mut wait_metrics);
        assert_eq!(
            wait_metrics.observer_dropped_samples, 1,
            "full queue uses drop-new semantics for wait observations"
        );
        assert_eq!(wait_metrics.spin_observation_expected_samples, 1);
        assert_eq!(wait_metrics.spin_observation_dropped_samples, 1);
        assert_eq!(wait_metrics.spin_observation_missing_samples, 0);
        assert_eq!(wait_metrics.spin_observation_pending_samples, 0);
        assert_eq!(wait_metrics.observer_queue_high_watermark, 0);
        assert_eq!(queue.len(), OBSERVATION_QUEUE_CAPACITY);
    }

    #[test]
    fn wait_attempt_is_enqueued_once_with_its_original_deadline() {
        let queue = PendingObservationQueue::default();
        let mut metrics = WorkerMetricsLocal::default();
        let target = QpcTicks::from_raw(31_000);
        let planned = DurationTicks::from_raw(9_000);
        let deadline = TimelineTicks::from_raw(22_000);
        let epoch = QpcTicks::from_raw(10_000);
        let wait = record_wait_attempt(
            Some(&queue),
            Some(WaitResult::interrupted()),
            target,
            planned,
            deadline,
            epoch,
            None,
            &mut metrics,
        )
        .expect("interrupted wait attempt should be observed");

        assert_eq!(metrics.observer_dropped_samples, 0);
        assert_eq!(
            queue.len(),
            0,
            "raw capture must not publish before dispatch"
        );
        let mut runtime = WorkerRuntime {
            pending_wait_observation: Some(wait),
            ..WorkerRuntime::default()
        };
        publish_pending_wait_observation(&mut runtime, Some(&queue), &mut metrics);
        assert_eq!(metrics.observer_queue_high_watermark, 0);
        match queue.pop_front() {
            Some(DispatchObservation::Wait(observed)) => {
                assert!(matches!(observed.outcome, WaitOutcome::Interrupted));
                assert_eq!(observed.wake_qpc, None);
                assert_eq!(observed.physical_target_qpc, wait.physical_target_qpc);
                assert_eq!(observed.planned_wait_ticks, wait.planned_wait_ticks);
                assert_eq!(observed.deadline_ticks, wait.deadline_ticks);
                assert_eq!(observed.epoch_qpc, wait.epoch_qpc);
            }
            other => panic!("expected exactly one wait observation, got {other:?}"),
        }
        assert!(queue.is_empty(), "wait evidence must not be duplicated");
    }

    #[test]
    fn consecutive_interrupted_waits_keep_separate_deadlines() {
        let queue = PendingObservationQueue::default();
        let mut metrics = WorkerMetricsLocal::default();
        let mut runtime = WorkerRuntime::default();
        for target in [31_000, 42_000] {
            runtime.pending_wait_observation = record_wait_attempt(
                Some(&queue),
                Some(WaitResult::interrupted()),
                QpcTicks::from_raw(target),
                DurationTicks::from_raw(9_000),
                TimelineTicks::from_raw(target - 10_000),
                QpcTicks::from_raw(10_000),
                None,
                &mut metrics,
            );
            publish_pending_wait_observation(&mut runtime, Some(&queue), &mut metrics);
            publish_pending_wait_observation(&mut runtime, Some(&queue), &mut metrics);
        }
        for target in [31_000, 42_000] {
            match queue.pop_front() {
                Some(DispatchObservation::Wait(wait)) => {
                    assert_eq!(wait.physical_target_qpc, QpcTicks::from_raw(target));
                    assert_eq!(
                        wait.deadline_ticks,
                        TimelineTicks::from_raw(target - 10_000)
                    );
                }
                other => panic!("expected wait observation, got {other:?}"),
            }
        }
        assert!(queue.is_empty());
        assert_eq!(metrics.spin_observation_expected_samples, 2);
        assert_eq!(metrics.spin_observation_dropped_samples, 0);
        assert_eq!(metrics.spin_observation_missing_samples, 0);
    }

    #[test]
    fn observer_removed_after_capture_counts_missing_once() {
        let queue = PendingObservationQueue::default();
        let mut metrics = WorkerMetricsLocal::default();
        let mut runtime = WorkerRuntime {
            pending_wait_observation: record_wait_attempt(
                Some(&queue),
                Some(WaitResult::interrupted()),
                QpcTicks::from_raw(31_000),
                DurationTicks::from_raw(9_000),
                TimelineTicks::from_raw(22_000),
                QpcTicks::from_raw(10_000),
                None,
                &mut metrics,
            ),
            ..WorkerRuntime::default()
        };
        publish_pending_wait_observation(&mut runtime, None, &mut metrics);
        publish_pending_wait_observation(&mut runtime, None, &mut metrics);
        assert_eq!(metrics.spin_observation_expected_samples, 1);
        assert_eq!(metrics.spin_observation_missing_samples, 1);
        assert_eq!(metrics.spin_observation_dropped_samples, 0);
        assert!(queue.is_empty());
    }

    #[test]
    fn no_wait_result_and_failed_wait_do_not_claim_a_sample() {
        let queue = PendingObservationQueue::default();
        let mut metrics = WorkerMetricsLocal::default();
        let target = QpcTicks::from_raw(31_000);
        let planned = DurationTicks::ZERO;
        let deadline = TimelineTicks::from_raw(22_000);
        let epoch = QpcTicks::from_raw(10_000);

        assert!(
            record_wait_attempt(
                Some(&queue),
                None,
                target,
                planned,
                deadline,
                epoch,
                None,
                &mut metrics,
            )
            .is_none()
        );
        assert!(
            record_wait_attempt(
                Some(&queue),
                Some(WaitResult::failed(
                    sky_dispatch_win32::wait::WaitFailure::Clock
                )),
                target,
                planned,
                deadline,
                epoch,
                None,
                &mut metrics,
            )
            .is_none()
        );
        assert!(queue.is_empty());
        assert_eq!(metrics.observer_dropped_samples, 0);
        assert_eq!(metrics.observer_queue_high_watermark, 0);
        assert_eq!(metrics.spin_observation_expected_samples, 0);
        assert_eq!(metrics.spin_observation_missing_samples, 0);
    }

    #[test]
    fn wait_observation_preserves_relative_timer_arm_target_and_generation() {
        let queue = PendingObservationQueue::default();
        let mut metrics = WorkerMetricsLocal::default();
        let physical_target = QpcTicks::from_raw(31_000);
        let observation = record_wait_attempt(
            Some(&queue),
            Some(WaitResult {
                outcome: WaitOutcome::Deadline,
                wake_qpc: Some(QpcTicks::from_raw(29_000)),
                spin_ticks: DurationTicks::from_raw(0),
                timer_arm_qpc: Some(QpcTicks::from_raw(20_000)),
                timer_arm_target_qpc: Some(QpcTicks::from_raw(25_000)),
                timer_arm_generation: Some(3),
                timer_return_qpc: Some(QpcTicks::from_raw(26_000)),
                timer_return_generation: Some(3),
            }),
            physical_target,
            DurationTicks::from_raw(11_000),
            TimelineTicks::from_raw(22_000),
            QpcTicks::from_raw(10_000),
            Some(QpcTicks::from_raw(29_000)),
            &mut metrics,
        )
        .expect("real timer wait should be observed");

        assert_eq!(observation.physical_target_qpc, physical_target);
        assert_eq!(
            observation.timer_arm_target_qpc,
            Some(QpcTicks::from_raw(25_000))
        );
        assert_ne!(observation.timer_arm_target_qpc, Some(physical_target));
        assert_eq!(observation.timer_arm_generation, Some(3));
        assert_eq!(observation.timer_return_generation, Some(3));
        assert_eq!(
            observation.timer_return_qpc,
            Some(QpcTicks::from_raw(26_000))
        );
    }

    #[test]
    fn observer_disabled_marks_spin_sample_missing_instead_of_zero() {
        let mut metrics = WorkerMetricsLocal::default();
        let result = WaitResult {
            outcome: WaitOutcome::Deadline,
            wake_qpc: Some(QpcTicks::from_raw(4_100)),
            spin_ticks: DurationTicks::from_raw(100),
            timer_return_qpc: Some(QpcTicks::from_raw(4_000)),
            timer_arm_target_qpc: Some(QpcTicks::from_raw(3_900)),
            #[cfg(any(test, feature = "test-support"))]
            timer_arm_qpc: Some(QpcTicks::from_raw(3_890)),
            #[cfg(any(test, feature = "test-support"))]
            timer_arm_generation: Some(1),
            #[cfg(any(test, feature = "test-support"))]
            timer_return_generation: Some(1),
        };

        assert!(
            record_wait_attempt(
                None,
                Some(result),
                QpcTicks::from_raw(4_000),
                DurationTicks::from_raw(2_000),
                TimelineTicks::from_raw(3_000),
                QpcTicks::from_raw(1_000),
                Some(QpcTicks::from_raw(4_100)),
                &mut metrics,
            )
            .is_none()
        );

        assert_eq!(metrics.spin_observation_expected_samples, 1);
        assert_eq!(metrics.spin_observation_missing_samples, 1);
        assert_eq!(metrics.spin_time_us, 0);
        assert_eq!(metrics.idle_wake_count, 0);
    }

    #[test]
    fn interrupted_pause_wait_and_restored_mixed_wait_keep_separate_attribution() {
        let queue = PendingObservationQueue::default();
        let mut metrics = WorkerMetricsLocal::default();

        let paused = record_wait_attempt(
            Some(&queue),
            Some(WaitResult::interrupted()),
            QpcTicks::from_raw(3_020),
            DurationTicks::from_raw(1_500),
            TimelineTicks::from_raw(2_020),
            QpcTicks::from_raw(1_000),
            None,
            &mut metrics,
        )
        .expect("interrupted pre-pause wait should remain observable");
        let mut runtime = WorkerRuntime {
            pending_wait_observation: Some(paused),
            ..WorkerRuntime::default()
        };
        publish_pending_wait_observation(&mut runtime, Some(&queue), &mut metrics);
        let restored = record_wait_attempt(
            Some(&queue),
            Some(WaitResult {
                outcome: WaitOutcome::Deadline,
                wake_qpc: Some(QpcTicks::from_raw(5_010)),
                spin_ticks: DurationTicks::from_raw(8),
                timer_return_qpc: Some(QpcTicks::from_raw(5_000)),
                timer_arm_target_qpc: Some(QpcTicks::from_raw(4_990)),
                #[cfg(any(test, feature = "test-support"))]
                timer_arm_qpc: Some(QpcTicks::from_raw(4_980)),
                #[cfg(any(test, feature = "test-support"))]
                timer_arm_generation: Some(1),
                #[cfg(any(test, feature = "test-support"))]
                timer_return_generation: Some(1),
            }),
            QpcTicks::from_raw(5_000),
            DurationTicks::from_raw(2_400),
            TimelineTicks::from_raw(2_500),
            QpcTicks::from_raw(2_000),
            Some(QpcTicks::from_raw(5_005)),
            &mut metrics,
        )
        .expect("restored mixed-boundary wait should remain observable");
        runtime.pending_wait_observation = Some(restored);
        publish_pending_wait_observation(&mut runtime, Some(&queue), &mut metrics);

        assert_eq!(metrics.observer_queue_high_watermark, 0);
        match queue.pop_front() {
            Some(DispatchObservation::Wait(observed)) => {
                assert!(matches!(observed.outcome, WaitOutcome::Interrupted));
                assert_eq!(observed.physical_target_qpc, paused.physical_target_qpc);
                assert_eq!(observed.deadline_ticks, TimelineTicks::from_raw(2_020));
                assert_eq!(observed.epoch_qpc, QpcTicks::from_raw(1_000));
            }
            other => panic!("expected paused wait first, got {other:?}"),
        }
        match queue.pop_front() {
            Some(DispatchObservation::Wait(observed)) => {
                assert!(matches!(observed.outcome, WaitOutcome::Deadline));
                assert_eq!(observed.wake_qpc, Some(QpcTicks::from_raw(5_010)));
                assert_eq!(observed.physical_target_qpc, restored.physical_target_qpc);
                assert_eq!(observed.deadline_ticks, TimelineTicks::from_raw(2_500));
                assert_eq!(observed.epoch_qpc, QpcTicks::from_raw(2_000));
            }
            other => panic!("expected restored mixed wait second, got {other:?}"),
        }
        assert!(queue.is_empty());
    }
}
