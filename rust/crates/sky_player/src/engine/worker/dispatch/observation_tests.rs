#![cfg(test)]
use super::*;
use crate::engine::DispatchHealthOptions;
use std::num::NonZeroU64;

#[test]
fn raw_down_masks_derive_transport_counts_when_observed() {
    assert_eq!(
        down_transport_counts(PhysicalPacket::new(0b001, 0b010), 0b010, 0, true,),
        (2, 2, 0)
    );
    assert_eq!(
        down_transport_counts(PhysicalPacket::new(0b001, 0b110), 0b110, 0, true,),
        (3, 3, 0)
    );
}

#[test]
fn raw_up_masks_derive_transport_counts_when_observed() {
    assert_eq!(
        up_transport_counts_from_masks(0b111, 0b101, 0b010),
        (3, 2, 1)
    );
}

#[test]
fn wait_observation_defers_conversion_and_health_updates() {
    let qpc_clock =
        sky_dispatch_win32::clock::QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
    let mut health = WorkerHealthState::new(DispatchHealthOptions::default());
    let mut local_metrics = WorkerMetricsLocal::default();
    let observation = WaitObservation {
        sequence: 1,
        outcome: WaitOutcome::Deadline,
        wake_qpc: Some(QpcTicks::from_raw(2_500)),
        spin_ticks: DurationTicks::from_raw(100),
        physical_target_qpc: QpcTicks::from_raw(2_000),
        planned_wait_ticks: DurationTicks::from_raw(25_000),
        deadline_ticks: TimelineTicks::from_raw(1_000),
        epoch_qpc: QpcTicks::from_raw(1_000),
        allow_pre_epoch_startup_dispatch: false,
        timer_arm_qpc: Some(QpcTicks::from_raw(2_100)),
        timer_arm_target_qpc: Some(QpcTicks::from_raw(2_200)),
        timer_arm_generation: Some(1),
        timer_return_generation: Some(1),
        timer_return_qpc: Some(QpcTicks::from_raw(2_400)),
        deadline_crossing_qpc: Some(QpcTicks::from_raw(2_500)),
        waiter_return_qpc: Some(QpcTicks::from_raw(2_500)),
    };

    drain_wait_observation(&observation, &mut health, &mut local_metrics, qpc_clock)
        .expect("raw wait observation should be drainable");

    assert_eq!(local_metrics.idle_wake_count, 1);
    assert_eq!(local_metrics.spin_time_us, 100);
    assert_eq!(local_metrics.wait_target_error_us, 500);
    assert_eq!(local_metrics.wait_planned_gap_cold_count, 1);
    assert_eq!(local_metrics.wait_planned_gap_hot_count, 0);
    assert_eq!(local_metrics.wait_planned_gap_max_us, 0);
    assert_eq!(local_metrics.physical_target_to_wake_max_ticks, 500);
    assert_eq!(local_metrics.wait_degraded_samples, 1);
    assert_eq!(local_metrics.wait_window_sample_count, 1);
}

#[test]
fn wait_gap_classification_uses_existing_cold_threshold_boundary() {
    assert_eq!(
        classify_wait_gap(
            DurationTicks::from_raw(19_999),
            DurationTicks::from_raw(20_000)
        ),
        WaitGapClass::Hot
    );
    assert_eq!(
        classify_wait_gap(
            DurationTicks::from_raw(20_000),
            DurationTicks::from_raw(20_000)
        ),
        WaitGapClass::Cold
    );
}

#[test]
fn interrupted_wait_is_excluded_from_deadline_gap_population() {
    let qpc_clock =
        sky_dispatch_win32::clock::QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
    let mut health = WorkerHealthState::new(DispatchHealthOptions::default());
    let mut local_metrics = WorkerMetricsLocal::default();
    let observation = WaitObservation {
        sequence: 1,
        outcome: WaitOutcome::Interrupted,
        wake_qpc: None,
        spin_ticks: DurationTicks::from_raw(100),
        physical_target_qpc: QpcTicks::from_raw(2_000),
        planned_wait_ticks: DurationTicks::from_raw(25_000),
        deadline_ticks: TimelineTicks::from_raw(1_000),
        epoch_qpc: QpcTicks::from_raw(1_000),
        allow_pre_epoch_startup_dispatch: false,
        timer_arm_qpc: None,
        timer_arm_target_qpc: None,
        timer_arm_generation: None,
        timer_return_generation: None,
        timer_return_qpc: None,
        deadline_crossing_qpc: None,
        waiter_return_qpc: None,
    };

    drain_wait_observation(&observation, &mut health, &mut local_metrics, qpc_clock)
        .expect("interrupted wait observation should be drainable");

    assert_eq!(local_metrics.idle_wake_count, 1);
    assert_eq!(local_metrics.spin_time_us, 100);
    assert_eq!(local_metrics.wait_planned_gap_hot_count, 0);
    assert_eq!(local_metrics.wait_planned_gap_cold_count, 0);
    assert_eq!(local_metrics.wait_planned_gap_max_us, 0);
    assert_eq!(local_metrics.physical_target_to_wake_max_ticks, 0);
    assert_eq!(local_metrics.wait_degraded_samples, 0);
    assert_eq!(local_metrics.wait_window_sample_count, 0);
}
#[test]
fn report_observation_layout_sizes() {
    use super::{DispatchObservation, DownMissObservation, DownObservation, UpObservation};
    use crate::engine::worker::wait::WaitObservation;
    use sky_dispatch_core::executor::UpSlotTiming;
    eprintln!(
        "DispatchObservation={} DownObservation={} DownMissObservation={} UpObservation={} WaitObservation={} UpSlotTiming={}",
        std::mem::size_of::<DispatchObservation>(),
        std::mem::size_of::<DownObservation>(),
        std::mem::size_of::<DownMissObservation>(),
        std::mem::size_of::<UpObservation>(),
        std::mem::size_of::<WaitObservation>(),
        std::mem::size_of::<UpSlotTiming>()
    );
}

#[test]
fn up_slot_pool_is_bounded_and_returns_storage_on_drop() {
    use super::super::observer::PendingObservationQueue;
    let queue = PendingObservationQueue::default();
    let mut borrowed = Vec::new();
    for _ in 0..super::OBSERVATION_QUEUE_CAPACITY + 2 {
        borrowed.push(queue.acquire_up_slots().expect("preallocated slot"));
    }
    assert!(queue.acquire_up_slots().is_none());
    drop(borrowed.pop());
    assert!(queue.acquire_up_slots().is_some());
    borrowed.clear();
    for _ in 0..super::OBSERVATION_QUEUE_CAPACITY + 2 {
        borrowed.push(queue.acquire_up_slots().expect("returned slot"));
    }
}
