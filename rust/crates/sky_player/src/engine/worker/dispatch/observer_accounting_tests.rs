use super::{
    DispatchObservation, ObserverRuntime, PendingObservationQueue, StaleMetadataObservation,
    observer_test_hook_guard, set_observer_abort_after_drains,
};
use crate::engine::worker::dispatch::observation::DownObservation;
use crate::engine::{
    DispatchHealthOptions, NativeTelemetryOutput, SharedMetrics, TelemetryCollector, TelemetryMode,
    WorkerTimingState,
};
use sky_dispatch_core::time::TimelineTicks;
use sky_dispatch_win32::clock::QpcClock;
use std::sync::Arc;

fn queue_down_with_up_slots(queue: &PendingObservationQueue, sequence: u64, count: u8) {
    let mut observation = DownObservation::test_fixture(sequence);
    observation.up_slot_count = count;
    if count > 0 {
        observation.up_slots = queue.acquire_up_slots();
        assert!(
            observation.up_slots.is_some(),
            "preallocated Up-slot storage"
        );
    }
    let mut dropped_samples = 0;
    assert!(queue.push(DispatchObservation::Down(observation), &mut dropped_samples,));
    assert_eq!(dropped_samples, 0);
}

fn queue_stale_metadata(queue: &PendingObservationQueue, source_action_index: u32) {
    let mut dropped_samples = 0;
    assert!(queue.push(
        DispatchObservation::StaleMetadata(StaleMetadataObservation {
            source_action_index,
            effective_scheduled_ticks: TimelineTicks::from_raw(u64::from(source_action_index)),
            suppressed_intent_count: 1,
        }),
        &mut dropped_samples,
    ));
    assert_eq!(dropped_samples, 0);
}

fn start_observer(
    queue: &PendingObservationQueue,
    expected_up_slots: u64,
) -> (ObserverRuntime, Arc<parking_lot::Mutex<TelemetryCollector>>) {
    let telemetry = Arc::new(parking_lot::Mutex::new(TelemetryCollector::new(
        TelemetryMode::Ring,
        16,
    )));
    telemetry.lock().output.up_slot_records_expected = expected_up_slots;
    let runtime = ObserverRuntime::start(
        queue.clone(),
        QpcClock::initialize().expect("QPC clock"),
        Arc::new(SharedMetrics::default()),
        Arc::clone(&telemetry),
        WorkerTimingState::create_test_timing(),
        DispatchHealthOptions::default(),
    )
    .expect("observer thread");
    (runtime, telemetry)
}

fn finalized_output(
    telemetry: Arc<parking_lot::Mutex<TelemetryCollector>>,
    accounting_complete: bool,
) -> NativeTelemetryOutput {
    let mut output = match Arc::try_unwrap(telemetry) {
        Ok(telemetry) => telemetry.into_inner().output,
        Err(_) => panic!("observer releases telemetry"),
    };
    output.finalize_up_slot_accounting(accounting_complete);
    output
}

#[test]
fn observer_abort_before_first_drain_accounts_all_queued_up_slots() {
    let _guard = observer_test_hook_guard();
    let queue = PendingObservationQueue::default();
    queue_down_with_up_slots(&queue, 1, 2);
    queue_down_with_up_slots(&queue, 2, 1);
    set_observer_abort_after_drains(0);
    let (runtime, telemetry) = start_observer(&queue, 3);

    let observer_output = runtime.stop();
    assert_eq!(
        observer_output.terminal_error.as_deref(),
        Some("injected observer consumer failure")
    );
    assert!(observer_output.up_slot_accounting_complete);
    let output = finalized_output(telemetry, observer_output.up_slot_accounting_complete);
    let json = serde_json::to_value(&output).expect("telemetry JSON");
    assert_eq!(json["up_slot_records_expected"], 3);
    assert_eq!(json["up_slot_records_dropped_by_observer_abort"], 3);
    assert_eq!(json["up_slot_records_dropped"], 3);
    assert_eq!(json["up_slot_accounting_status"], "valid");
    assert!(json["up_slot_accounting_invalid_reason"].is_null());
}

#[test]
fn observer_abort_after_partial_drain_counts_only_stranded_up_slots() {
    let _guard = observer_test_hook_guard();
    let queue = PendingObservationQueue::default();
    queue_stale_metadata(&queue, 1);
    queue_down_with_up_slots(&queue, 2, 2);
    set_observer_abort_after_drains(1);
    let (runtime, telemetry) = start_observer(&queue, 2);

    let observer_output = runtime.stop();
    assert_eq!(
        observer_output.terminal_error.as_deref(),
        Some("injected observer consumer failure")
    );
    assert!(observer_output.up_slot_accounting_complete);
    let output = finalized_output(telemetry, observer_output.up_slot_accounting_complete);
    let json = serde_json::to_value(&output).expect("telemetry JSON");
    assert_eq!(json["records"].as_array().map(Vec::len), Some(1));
    assert_eq!(json["up_slot_records_expected"], 2);
    assert_eq!(json["up_slot_records_dropped_by_observer_abort"], 2);
    assert_eq!(json["up_slot_records_dropped"], 2);
    assert_eq!(json["up_slot_accounting_status"], "valid");
}
