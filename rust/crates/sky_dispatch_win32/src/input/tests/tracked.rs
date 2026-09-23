use super::super::outcome::{
    PacketRetryReason, PhysicalPacket, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
};
use super::super::tracked::TrackedKeyState;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn same_key_retrigger_packet_is_rejected_at_preparation_boundary() {
    let error =
        super::super::packet::PreparedPhysicalPacket::try_new(PhysicalPacket::new(0b001, 0b001))
            .expect_err("same-key Up+Down overlap must be invalid");
    assert_eq!(
        error,
        super::super::outcome::PacketPreparationError::OverlappingDirections {
            overlap_mask: 0b001
        }
    );
}

#[test]
fn invalid_packet_is_no_syscall_and_not_sendinput_zero_progress() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_emitter = Arc::clone(&calls);
    let mut state = TrackedKeyState::with_packet_emitter(move |_packet| {
        calls_for_emitter.fetch_add(1, Ordering::Relaxed);
        panic!("invalid packet reached packet emitter");
    });

    let outcome = state.send_physical_packet(PhysicalPacket::new(0b001, 0b001));

    assert_eq!(outcome.status, SendTransactionStatus::PreparationRejected);
    assert_eq!(outcome.evidence.attempts, 0);
    assert_eq!(outcome.evidence.first_win32_error, None);
    assert_eq!(outcome.evidence.last_win32_error, None);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert_eq!(state.sendinput_zero_progress_failures, 0);
    assert_eq!(state.chords_rejected, 1);
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
fn prepared_down_at_authored_boundary_sends_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut state = TrackedKeyState::with_packet_emitter(prepared_success_emitter(calls.clone()));
    let prepared = super::super::packet::PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 1))
        .expect("valid prepared Down packet");

    let outcome = state
        .send_prepared_physical_packet_with_start(&prepared, crate::clock::QpcTicks::from_raw(100));

    assert_eq!(outcome.status, SendTransactionStatus::Complete);
    assert_eq!(outcome.evidence.attempts, 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn prepared_down_one_tick_late_still_calls_emitter() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut state = TrackedKeyState::with_packet_emitter(prepared_success_emitter(calls.clone()));
    let prepared = super::super::packet::PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 1))
        .expect("valid prepared Down packet");

    let outcome = state
        .send_prepared_physical_packet_with_start(&prepared, crate::clock::QpcTicks::from_raw(101));

    assert_eq!(outcome.status, SendTransactionStatus::Complete);
    assert_eq!(outcome.evidence.attempts, 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(state.active_mask, 1);
}

#[test]
fn prepared_sender_panic_retains_exact_packet_mask_until_cleanup() {
    let packets = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_packets = Arc::clone(&packets);
    let success = prepared_success_emitter(Arc::new(AtomicUsize::new(0)));
    let mut state = TrackedKeyState::with_packet_emitter(move |packet| {
        captured_packets
            .lock()
            .expect("packet capture lock")
            .push(packet);
        if packet.down_mask != 0 {
            panic!("sender outcome became unknowable");
        }
        success(packet)
    });
    let packet = PhysicalPacket::new(0, 0b0010_0100);
    let prepared = super::super::packet::PreparedPhysicalPacket::try_new(packet)
        .expect("valid prepared chord");

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        state.send_prepared_physical_packet_with_start(
            &prepared,
            crate::clock::QpcTicks::from_raw(100),
        );
    }));
    assert!(result.is_err());
    assert_eq!(state.in_flight_mask, packet.down_mask);
    assert_eq!(state.release_obligation_mask(), packet.down_mask);
    state.set_probe(|_, _| super::super::physical::InstrumentPhysicalState::AllUp);

    let cleanup = state.release_scope(super::super::tracked::ReleaseScope::Tracked, 0);
    assert_eq!(cleanup.attempted_mask, packet.down_mask);
    assert_eq!(cleanup.attempts, 1);
    assert_eq!(
        *packets.lock().expect("packet capture lock"),
        vec![packet, PhysicalPacket::new(packet.down_mask, 0)],
        "tracked cleanup must release only the packet whose outcome is unknown"
    );
    assert_eq!(state.in_flight_mask, 0);
    assert_eq!(state.release_obligation_mask(), 0);
}

fn complete_packet_outcome(packet: PhysicalPacket) -> SendTransactionOutcome {
    let mask = packet.up_mask | packet.down_mask;
    SendTransactionOutcome {
        status: SendTransactionStatus::Complete,
        evidence: SendEvidence {
            requested_mask: mask,
            confirmed_mask: mask,
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

#[test]
fn prepared_mixed_packet_ambiguity_cleans_prior_and_packet_masks_only() {
    let calls = Arc::new(AtomicUsize::new(0));
    let packets = Arc::new(std::sync::Mutex::new(Vec::new()));
    let calls_for_emitter = Arc::clone(&calls);
    let packets_for_emitter = Arc::clone(&packets);
    let mut state = TrackedKeyState::with_packet_emitter(move |packet| {
        packets_for_emitter
            .lock()
            .expect("packet capture lock")
            .push(packet);
        if calls_for_emitter.fetch_add(1, Ordering::Relaxed) == 0 {
            SendTransactionOutcome {
                status: SendTransactionStatus::PartialProgress,
                evidence: SendEvidence {
                    requested_mask: packet.up_mask | packet.down_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: 1,
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
        } else {
            complete_packet_outcome(packet)
        }
    });
    let prior_ownership = 0b1000;
    let packet = PhysicalPacket::new(0b0001, 0b0100);
    let prepared =
        super::super::packet::PreparedPhysicalPacket::try_new(packet).expect("valid mixed packet");
    state.active_mask = prior_ownership;

    let outcome = state
        .send_prepared_physical_packet_with_start(&prepared, crate::clock::QpcTicks::from_raw(100));

    assert_eq!(outcome.status, SendTransactionStatus::PartialProgress);
    assert_eq!(
        state.possibly_active_mask,
        packet.up_mask | packet.down_mask
    );
    let expected_cleanup_mask = prior_ownership | packet.up_mask | packet.down_mask;
    assert_eq!(state.release_obligation_mask(), expected_cleanup_mask);
    assert_eq!(
        state.in_flight_mask, 0,
        "committed ambiguity moves to tracked state"
    );

    state.set_probe(|_, _| super::super::physical::InstrumentPhysicalState::AllUp);
    let cleanup = state.release_scope(super::super::tracked::ReleaseScope::Tracked, 0);
    assert_eq!(cleanup.attempted_mask, expected_cleanup_mask);
    assert!(cleanup.released_successfully);
    assert_eq!(
        *packets.lock().expect("packet capture lock"),
        vec![packet, PhysicalPacket::new(expected_cleanup_mask, 0)],
        "ambiguous mixed cleanup must cover the whole packet and prior ownership only"
    );
    assert_eq!(state.release_obligation_mask(), 0);
}

#[test]
fn prepared_sender_panic_interleavings_preserve_only_uncommitted_packet_ownership() {
    use super::super::tracked::PreparedSendPanicPoint;

    let packet = PhysicalPacket::new(0, 0b0101_0000);
    let prepared = super::super::packet::PreparedPhysicalPacket::try_new(packet)
        .expect("valid prepared chord");

    for (point, expected_obligation, expected_down_sends) in [
        (PreparedSendPanicPoint::BeforeSenderAuthority, 0, 0),
        (
            PreparedSendPanicPoint::AfterSenderReturn,
            packet.down_mask,
            1,
        ),
        (
            PreparedSendPanicPoint::AfterStateCommit,
            packet.down_mask,
            1,
        ),
    ] {
        let packets = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_packets = Arc::clone(&packets);
        let mut state = TrackedKeyState::with_packet_emitter(move |sent| {
            captured_packets
                .lock()
                .expect("packet capture lock")
                .push(sent);
            complete_packet_outcome(sent)
        });
        state.set_probe(|_, _| super::super::physical::InstrumentPhysicalState::AllUp);
        state.prepared_send_panic_point = Some(point);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            state.send_prepared_physical_packet_with_start(
                &prepared,
                crate::clock::QpcTicks::from_raw(100),
            );
        }));
        assert!(result.is_err(), "{point:?} hook must unwind");
        assert_eq!(
            state.release_obligation_mask(),
            expected_obligation,
            "{point:?}"
        );
        assert_eq!(
            state.in_flight_mask,
            if point == PreparedSendPanicPoint::AfterSenderReturn {
                packet.down_mask
            } else {
                0
            },
            "{point:?}: in-flight residue"
        );

        let cleanup = state.release_scope(super::super::tracked::ReleaseScope::Tracked, 0);
        assert_eq!(cleanup.attempted_mask, expected_obligation, "{point:?}");
        let captured = packets.lock().expect("packet capture lock").clone();
        assert_eq!(
            captured.iter().filter(|sent| sent.down_mask != 0).count(),
            expected_down_sends,
            "{point:?}: gameplay Down count"
        );
        if expected_obligation == 0 {
            assert!(
                captured.is_empty(),
                "{point:?}: no transport before authority"
            );
        } else {
            assert_eq!(
                captured,
                vec![packet, PhysicalPacket::new(packet.down_mask, 0)],
                "{point:?}: exact packet-scoped cleanup"
            );
        }
        assert_eq!(
            state.release_obligation_mask(),
            0,
            "{point:?}: residue after cleanup"
        );
    }
}

#[test]
fn prepared_custom_emitter_records_sender_start_before_invocation() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut state = TrackedKeyState::with_packet_emitter(prepared_success_emitter(calls.clone()));
    let prepared = super::super::packet::PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 1))
        .expect("valid prepared Down packet");

    let outcome = state.send_prepared_physical_packet(&prepared);

    assert_eq!(outcome.status, SendTransactionStatus::Complete);
    assert_eq!(outcome.evidence.attempts, 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(state.active_mask, 1);
}

#[test]
fn prepared_custom_emitter_sends_up_only_packet() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut state = TrackedKeyState::with_packet_emitter(prepared_success_emitter(calls.clone()));
    let prepared = super::super::packet::PreparedPhysicalPacket::try_new(PhysicalPacket::new(1, 0))
        .expect("valid prepared Up packet");

    let outcome = state.send_prepared_physical_packet(&prepared);

    assert_eq!(outcome.status, SendTransactionStatus::Complete);
    assert_eq!(outcome.evidence.attempts, 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn prepared_up_only_remains_release_eligible_when_late() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut state = TrackedKeyState::with_packet_emitter(prepared_success_emitter(calls.clone()));
    let prepared = super::super::packet::PreparedPhysicalPacket::try_new(PhysicalPacket::new(1, 0))
        .expect("valid prepared Up packet");

    let outcome = state
        .send_prepared_physical_packet_with_start(&prepared, crate::clock::QpcTicks::from_raw(101));

    assert_eq!(outcome.status, SendTransactionStatus::Complete);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}
