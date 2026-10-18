use super::super::WorkerRuntime;
use super::super::wait::WaitObservation;
use super::observer::PendingObservationQueue;
use crate::engine::telemetry::WorkerMetricsLocal;
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::QpcTicks;

pub(crate) fn take_deadline_wake_qpc(runtime: &mut WorkerRuntime) -> Option<QpcTicks> {
    runtime.last_dispatch_deadline_wake_qpc.take()
}

/// Capture a real wait result before control transitions or another plan can
/// replace its deadline attribution. Queue insertion happens after dispatch,
/// except interrupted replans, which have no following dispatch attempt. An
/// already-due target has no wait result; failed waits are not observer samples.
pub(crate) fn record_wait_attempt(
    observer: Option<&PendingObservationQueue>,
    wait_result: Option<sky_dispatch_win32::wait::WaitResult>,
    physical_target_qpc: sky_dispatch_win32::clock::QpcTicks,
    planned_wait_ticks: DurationTicks,
    deadline_ticks: TimelineTicks,
    epoch_qpc: sky_dispatch_win32::clock::QpcTicks,
) -> Option<WaitObservation> {
    observer?;
    let result = wait_result?;
    if matches!(
        result.outcome,
        sky_dispatch_win32::wait::WaitOutcome::Failed(_)
    ) {
        return None;
    }
    let observation = WaitObservation {
        outcome: result.outcome,
        wake_qpc: result.wake_qpc,
        spin_ticks: result.spin_ticks,
        physical_target_qpc,
        planned_wait_ticks,
        deadline_ticks,
        epoch_qpc,
        allow_pre_epoch_startup_dispatch: true,
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
        return;
    };
    observer.push_wait(
        observation,
        &mut local_metrics.observer_dropped_samples,
        &mut local_metrics.observer_queue_high_watermark,
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
    fn queue_pressure_counts_drops_and_tracks_successful_occupancy() {
        let queue = PendingObservationQueue::default();
        let mut dropped = 0;
        let mut high_watermark = 0;

        for index in 0..OBSERVATION_QUEUE_CAPACITY {
            queue.push(
                DispatchObservation::StaleMetadata(StaleMetadataObservation {
                    source_action_index: u32::try_from(index).unwrap_or(u32::MAX),
                    effective_scheduled_ticks: TimelineTicks::from_raw(index as u64),
                    suppressed_intent_count: 1,
                }),
                &mut dropped,
                &mut high_watermark,
            );
        }

        assert_eq!(queue.len(), OBSERVATION_QUEUE_CAPACITY);
        assert_eq!(high_watermark, OBSERVATION_QUEUE_CAPACITY as u64);
        assert_eq!(dropped, 0);

        let mut wait_metrics = WorkerMetricsLocal {
            observer_queue_high_watermark: high_watermark,
            ..WorkerMetricsLocal::default()
        };
        let observation = record_wait_attempt(
            Some(&queue),
            Some(WaitResult::interrupted()),
            QpcTicks::from_raw(31_000),
            DurationTicks::from_raw(9_000),
            TimelineTicks::from_raw(22_000),
            QpcTicks::from_raw(10_000),
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
        assert_eq!(
            wait_metrics.observer_queue_high_watermark,
            OBSERVATION_QUEUE_CAPACITY as u64
        );
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
        assert_eq!(metrics.observer_queue_high_watermark, 1);
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
    fn no_wait_result_and_failed_wait_do_not_claim_a_sample() {
        let queue = PendingObservationQueue::default();
        let metrics = WorkerMetricsLocal::default();
        let target = QpcTicks::from_raw(31_000);
        let planned = DurationTicks::ZERO;
        let deadline = TimelineTicks::from_raw(22_000);
        let epoch = QpcTicks::from_raw(10_000);

        assert!(
            record_wait_attempt(Some(&queue), None, target, planned, deadline, epoch,).is_none()
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
            )
            .is_none()
        );
        assert!(queue.is_empty());
        assert_eq!(metrics.observer_dropped_samples, 0);
        assert_eq!(metrics.observer_queue_high_watermark, 0);
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
            }),
            QpcTicks::from_raw(5_000),
            DurationTicks::from_raw(2_400),
            TimelineTicks::from_raw(2_500),
            QpcTicks::from_raw(2_000),
        )
        .expect("restored mixed-boundary wait should remain observable");
        runtime.pending_wait_observation = Some(restored);
        publish_pending_wait_observation(&mut runtime, Some(&queue), &mut metrics);

        assert_eq!(metrics.observer_queue_high_watermark, 2);
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
