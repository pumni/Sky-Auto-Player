use super::finalize_admission_failure;
use crate::engine::OUTCOME_ERROR;
use crate::engine::telemetry::metrics::SharedMetrics;
use sky_dispatch_win32::input::{
    InstrumentPhysicalState, PacketRetryReason, PhysicalPacket, SendEvidence,
    SendTransactionOutcome, SendTransactionStatus, TrackedKeyState,
};

fn release_outcome(packet: PhysicalPacket, complete: bool) -> SendTransactionOutcome {
    let requested_mask = packet.up_mask | packet.down_mask;
    SendTransactionOutcome {
        status: if complete {
            SendTransactionStatus::Complete
        } else {
            SendTransactionStatus::PartialProgress
        },
        evidence: SendEvidence {
            requested_mask,
            confirmed_mask: if complete { requested_mask } else { 0 },
            skipped_mask: 0,
            first_inserted: if complete { packet.event_count() } else { 0 },
            attempts: 1,
            zero_progress_retries: 0,
            retry_reason: PacketRetryReason::None,
            first_win32_error: None,
            last_win32_error: None,
            started_ticks: None,
            completed_ticks: None,
            timing_error: None,
        },
    }
}

#[test]
fn admission_failure_keeps_startup_cause_and_verified_release_owner() {
    let mut backend = TrackedKeyState::with_packet_emitter(|packet| release_outcome(packet, true));
    backend.active_mask = 1;
    backend.set_probe(|_, _| InstrumentPhysicalState::AllUp);
    let metrics = SharedMetrics::default();

    assert_eq!(
        finalize_admission_failure(&mut backend, &metrics, 0, "startup admission failed".into()),
        OUTCOME_ERROR
    );

    let outcome = metrics
        .terminal_release_outcome
        .lock()
        .clone()
        .expect("admission owner publishes its release outcome");
    assert_eq!(outcome.attempts, 1);
    assert!(outcome.released_successfully);
    assert_eq!(backend.release_attempts_total, 1);
    assert_eq!(backend.release_obligation_mask(), 0);
    assert_eq!(
        metrics
            .terminal_release_obligation_mask
            .load(std::sync::atomic::Ordering::Acquire),
        0
    );
    assert_eq!(
        metrics
            .terminal_release_attempts_total
            .load(std::sync::atomic::Ordering::Acquire),
        1
    );
    assert_eq!(
        metrics.terminal_trigger_reason.lock().as_deref(),
        Some("startup_failure")
    );
    assert_eq!(
        metrics.terminal_error.lock().as_deref(),
        Some("startup admission failed")
    );
    assert!(metrics.secondary_errors.lock().is_empty());
}

#[test]
fn admission_cleanup_uncertainty_keeps_residual_mask_and_secondary_provenance() {
    let mut backend = TrackedKeyState::with_packet_emitter(|packet| release_outcome(packet, false));
    backend.active_mask = 1;
    backend.set_probe(|_, _| InstrumentPhysicalState::Inconclusive);
    let metrics = SharedMetrics::default();

    assert_eq!(
        finalize_admission_failure(&mut backend, &metrics, 0, "startup admission failed".into()),
        OUTCOME_ERROR
    );

    let outcome = metrics
        .terminal_release_outcome
        .lock()
        .clone()
        .expect("admission owner publishes its release outcome");
    assert_eq!(outcome.attempts, 4);
    assert!(outcome.verification_inconclusive);
    assert_eq!(outcome.stuck_mask, 1);
    assert_eq!(backend.release_attempts_total, 4);
    assert_eq!(backend.release_obligation_mask(), 1);
    assert_eq!(
        metrics
            .terminal_release_obligation_mask
            .load(std::sync::atomic::Ordering::Acquire),
        1
    );
    assert_eq!(
        metrics.terminal_error.lock().as_deref(),
        Some("startup admission failed")
    );
    assert!(metrics.secondary_errors.lock()[0].contains("admission cleanup failed"));
    assert!(
        metrics
            .last_error
            .lock()
            .as_deref()
            .unwrap()
            .contains("startup admission failed")
    );
    assert!(
        metrics
            .last_error
            .lock()
            .as_deref()
            .unwrap()
            .contains("admission cleanup failed")
    );
}

#[test]
fn admission_cleanup_panic_retains_in_flight_obligation_without_second_budget() {
    let mut backend =
        TrackedKeyState::with_packet_emitter(|_| panic!("test admission sender panic"));
    backend.active_mask = 1;
    let metrics = SharedMetrics::default();

    assert_eq!(
        finalize_admission_failure(&mut backend, &metrics, 0, "startup admission failed".into()),
        OUTCOME_ERROR
    );

    assert_eq!(backend.release_attempts_total, 1);
    assert_eq!(backend.release_obligation_mask(), 1);
    assert_eq!(
        metrics
            .terminal_release_attempts_total
            .load(std::sync::atomic::Ordering::Acquire),
        1
    );
    assert_eq!(
        metrics
            .terminal_release_obligation_mask
            .load(std::sync::atomic::Ordering::Acquire),
        1
    );
    assert!(metrics.terminal_release_outcome.lock().is_none());
    assert!(metrics.secondary_errors.lock()[0].contains("admission cleanup panicked"));
}
