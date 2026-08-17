use super::super::outcome::{
    PacketRetryReason, PhysicalPacket, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
};
use super::super::tracked::TrackedKeyState;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn same_key_retrigger_packet_contains_two_physical_events() {
    let mut state = TrackedKeyState::with_packet_emitter(|packet| {
        assert_eq!(packet.up_mask, 0b001);
        assert_eq!(packet.down_mask, 0b001);
        assert_eq!(packet.event_count(), 2);
        SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask: 0b001,
                confirmed_mask: 0b001,
                skipped_mask: 0,
                first_inserted: 2,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: Some(crate::clock::QpcTicks::from_raw(10)),
                completed_ticks: Some(crate::clock::QpcTicks::from_raw(20)),
                timing_error: None,
            },
        }
    });
    let outcome = state.key_down_physical_packet(PhysicalPacket::new(0b001, 0b001));
    assert_eq!(outcome.status, SendTransactionStatus::Complete);
}

fn prepared_success_emitter(
    calls: Arc<AtomicUsize>,
) -> impl Fn(PhysicalPacket) -> SendTransactionOutcome + Send + Sync + 'static {
    move |packet| {
        calls.fetch_add(1, Ordering::Relaxed);
        SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: packet.up_mask | packet.down_mask,
                skipped_mask: 0,
                first_inserted: packet.event_count(),
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
}

#[test]
fn prepared_down_cutoff_exact_boundary_sends_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut state = TrackedKeyState::with_packet_emitter(prepared_success_emitter(calls.clone()));
    let prepared = super::super::packet::PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 1))
        .expect("valid prepared Down packet");

    let outcome = state.send_prepared_physical_packet_with_start_and_cutoff(
        &prepared,
        crate::clock::QpcTicks::from_raw(100),
        Some(crate::clock::QpcTicks::from_raw(100)),
    );

    assert_eq!(outcome.status, SendTransactionStatus::Complete);
    assert_eq!(outcome.evidence.attempts, 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn prepared_down_cutoff_one_tick_late_never_calls_emitter() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut state = TrackedKeyState::with_packet_emitter(prepared_success_emitter(calls.clone()));
    let prepared = super::super::packet::PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 1))
        .expect("valid prepared Down packet");

    let outcome = state.send_prepared_physical_packet_with_start_and_cutoff(
        &prepared,
        crate::clock::QpcTicks::from_raw(101),
        Some(crate::clock::QpcTicks::from_raw(100)),
    );

    assert_eq!(
        outcome.status,
        SendTransactionStatus::DeadlineMissedBeforeSend
    );
    assert_eq!(outcome.evidence.attempts, 0);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert_eq!(state.active_mask, 0);
}

#[test]
fn prepared_up_only_late_cutoff_remains_release_eligible() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut state = TrackedKeyState::with_packet_emitter(prepared_success_emitter(calls.clone()));
    let prepared = super::super::packet::PreparedPhysicalPacket::try_new(PhysicalPacket::new(1, 0))
        .expect("valid prepared Up packet");

    let outcome = state.send_prepared_physical_packet_with_start_and_cutoff(
        &prepared,
        crate::clock::QpcTicks::from_raw(101),
        None,
    );

    assert_eq!(outcome.status, SendTransactionStatus::Complete);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}
