use super::*;
use crate::input::{
    InstrumentKeyProfile, InstrumentKeyProfileSpec, MaterializedInstrumentKeyProfile, PhysicalKey,
};
use std::sync::{Arc, Mutex};

#[test]
fn partial_transport_plus_physical_all_up_is_inconclusive() {
    let reconciled = reconcile_release_observation(0x0003, 0x0001, InstrumentPhysicalState::AllUp);
    assert_eq!(reconciled, ReconciledRelease::Inconclusive(0x0002));
}

#[test]
fn zero_progress_plus_physical_all_up_is_inconclusive() {
    let reconciled = reconcile_release_observation(0x0003, 0x0000, InstrumentPhysicalState::AllUp);
    assert_eq!(reconciled, ReconciledRelease::Inconclusive(0x0003));
}

#[test]
fn full_transport_and_all_up_are_verified_success() {
    let reconciled = reconcile_release_observation(0x0003, 0x0003, InstrumentPhysicalState::AllUp);
    assert_eq!(reconciled, ReconciledRelease::VerifiedAllUp);
}

#[test]
fn focus_loss_probe_cannot_report_cleanup_success() {
    let mut state = TrackedKeyState::with_emitter(|codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: codes.len() as u8,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 0,
        timing_error: None,
    });
    state.set_probe(|mask, _| instrument_physical_state_for_mask(isize::MAX, mask));
    state.active_mask = 0x0001;

    let outcome = state.release_all(isize::MAX);
    assert!(!outcome.released_successfully);
    assert!(outcome.verification_inconclusive);
    assert_eq!(outcome.stuck_mask, 0x0001);
    assert_eq!(state.release_obligation_mask(), 0x0001);
}

#[cfg(feature = "test-support")]
#[test]
fn foreground_change_after_cleanup_send_keeps_verified_release_unavailable() {
    struct ForegroundReset;

    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            crate::focus::set_foreground_window_for_test(None);
        }
    }

    let _foreground_lock = crate::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    crate::focus::set_foreground_window_for_test(Some(42));

    let send_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let send_count_for_emitter = Arc::clone(&send_count);
    let mut state = TrackedKeyState::with_emitter(move |codes, _| {
        send_count_for_emitter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let result = test_send_result(codes.len() as u8, codes.len() as u8, 0);
        crate::focus::set_foreground_window_for_test(None);
        result
    });
    state.set_probe(|_, _| {
        if crate::focus::foreground_window_matches(42) {
            InstrumentPhysicalState::AllUp
        } else {
            InstrumentPhysicalState::Inconclusive
        }
    });
    state.active_mask = 1;

    let outcome = state.release_scope(ReleaseScope::Tracked, 42);
    assert!(!outcome.released_successfully);
    assert!(outcome.verification_inconclusive);
    assert_eq!(outcome.stuck_mask, 1);
    assert_eq!(outcome.attempts, 1);
    assert_eq!(state.release_obligation_mask(), 1);
    assert_eq!(send_count.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn tracked_partial_down_keeps_uncertain_keys_until_verified_cleanup() {
    let calls = Arc::new(Mutex::new(0usize));
    let calls_for_emitter = Arc::clone(&calls);
    let mut state = TrackedKeyState::with_packet_emitter(move |packet| {
        *calls_for_emitter.lock().expect("partial call count") += 1;
        SendTransactionOutcome {
            status: SendTransactionStatus::PartialProgress,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: 1,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::PartialProgress { inserted_count: 1 },
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: Some(QpcTicks::from_raw(10)),
                completed_ticks: Some(QpcTicks::from_raw(11)),
                timing_error: None,
            },
        }
    });
    state.set_emitter(|codes, key_up| {
        assert!(key_up, "cleanup uses only the tracked Up packet");
        test_send_result(codes.len() as u8, codes.len() as u8, 0)
    });

    let outcome = state.send_physical_packet(PhysicalPacket::new(0, 0b0011));
    assert_eq!(outcome.status, SendTransactionStatus::PartialProgress);
    assert_eq!(*calls.lock().expect("partial call count"), 1);
    let obligation = state.release_obligation_mask();
    assert_ne!(obligation, 0);
    assert_eq!(obligation & 0b11, 0b11);

    state.set_probe(|_, _| InstrumentPhysicalState::AllUp);
    let cleanup = state.release_scope(ReleaseScope::Tracked, 0);
    assert_eq!(cleanup.attempted_mask & obligation, obligation);
    assert!(cleanup.released_successfully);
    assert_eq!(state.release_obligation_mask(), 0);
}

#[test]
fn release_obligation_mask_unions_all_conservative_sources() {
    let mut state = TrackedKeyState::new();
    state.active_mask = 0x0001;
    state.possibly_active_mask = 0x0002;
    state.failed_release_mask = 0x0004;
    state.in_flight_mask = 0x0008;

    assert_eq!(state.release_obligation_mask(), 0x000f);
}

#[test]
fn cleanup_up_retries_only_needs_up_within_the_four_call_budget() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls_for_emitter = Arc::clone(&calls);
    let mut state = TrackedKeyState::with_emitter(move |codes, key_up| {
        assert!(key_up);
        calls_for_emitter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        test_send_result(codes.len() as u8, 0, 5)
    });
    state.active_mask = 1;
    state.set_probe(|_, _| InstrumentPhysicalState::Inconclusive);

    let outcome = state.release_scope(ReleaseScope::Tracked, 0);

    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
    assert_eq!(outcome.attempts, 4);
    assert!(!outcome.released_successfully);
    assert_eq!(outcome.stuck_mask, 1);
    assert_eq!(state.needs_up_mask(), 1);
    assert_eq!(state.needs_probe_mask, 0);
}

#[test]
fn cleanup_full_up_and_all_up_probe_clear_the_tracked_obligation() {
    let mut state = TrackedKeyState::with_emitter(|codes, _| {
        let len = codes.len() as u8;
        test_send_result(len, len, 0)
    });
    state.active_mask = 1;
    state.set_probe(|_, _| InstrumentPhysicalState::AllUp);

    let outcome = state.release_scope(ReleaseScope::Tracked, 0);

    assert!(outcome.released_successfully);
    assert_eq!(outcome.attempts, 1);
    assert_eq!(state.release_obligation_mask(), 0);
}

#[test]
fn cleanup_fsm_executes_tracked_then_verifies_physical_all_up() {
    let mut state =
        TrackedKeyState::with_emitter(|c, _| test_send_result(c.len() as u8, c.len() as u8, 0));
    state.custom_probe = Some(Box::new(|_, _| InstrumentPhysicalState::AllUp));
    state.active_mask = 0x0001;
    let outcome = state.release_scope(ReleaseScope::Tracked, 0);
    assert!(outcome.released_successfully);
    assert_eq!(state.active_mask, 0);
    assert_eq!(state.failed_release_mask, 0);
    assert_eq!(state.release_obligation_mask(), 0);
}

#[test]
fn cleanup_fsm_idempotent_on_repeated_calls() {
    let mut state =
        TrackedKeyState::with_emitter(|c, _| test_send_result(c.len() as u8, c.len() as u8, 0));
    state.custom_probe = Some(Box::new(|_, _| InstrumentPhysicalState::AllUp));
    state.active_mask = 0x0001;
    let outcome1 = state.release_scope(ReleaseScope::Tracked, 0);
    let outcome2 = state.release_scope(ReleaseScope::Tracked, 0);
    assert!(outcome1.released_successfully);
    assert!(outcome2.released_successfully);
    assert_eq!(state.active_mask, 0);
}

#[test]
fn cleanup_transport_anomaly_and_partial_transport_fail_closed_even_if_probe_is_all_up() {
    let mut state = TrackedKeyState::with_emitter(|codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: 0,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 5,
        timing_error: None,
    });
    // Physical all-up evidence cannot compensate for partial transport.
    state.custom_probe = Some(Box::new(|_, _| InstrumentPhysicalState::AllUp));
    state.active_mask = 0x0003;

    let outcome = state.release_all(0);
    assert!(!outcome.released_successfully);
    assert!(!outcome.verification_inconclusive);
    assert_eq!(outcome.stuck_keys(), vec![0x15, 0x16]);
    assert!(outcome.transport_anomaly);
    assert_eq!(state.release_obligation_mask(), 0x0003);
}

#[test]
fn terminal_up_timing_failure_keeps_timestamps_unavailable_but_confirms_full_transport() {
    let packet = super::super::outcome::PhysicalPacket::new(0b0011, 0);
    let outcome = super::super::up_transaction::terminal_up_outcome(
        packet,
        super::super::outcome::TerminalUpTransportResult {
            requested: 2,
            inserted: 2,
            started_ticks: None,
            completed_ticks: Some(QpcTicks::from_raw(17)),
            win32_error: 0,
            timing_error: Some(QpcError::CounterUnavailable),
        },
    );

    assert_eq!(outcome.status, SendTransactionStatus::Complete);
    assert_eq!(outcome.evidence.requested_mask, 0b0011);
    assert_eq!(outcome.evidence.confirmed_mask, 0b0011);
    assert_eq!(outcome.evidence.started_ticks, None);
    assert_eq!(
        outcome.evidence.completed_ticks,
        Some(QpcTicks::from_raw(17))
    );
    assert_eq!(
        outcome.evidence.timing_error,
        Some(QpcError::CounterUnavailable)
    );
}

#[test]
fn terminal_up_after_send_qpc_failure_keeps_completion_unavailable() {
    let packet = super::super::outcome::PhysicalPacket::new(0b0001, 0);
    let outcome = super::super::up_transaction::terminal_up_outcome(
        packet,
        super::super::outcome::TerminalUpTransportResult {
            requested: 1,
            inserted: 1,
            started_ticks: Some(QpcTicks::from_raw(23)),
            completed_ticks: None,
            win32_error: 0,
            timing_error: Some(QpcError::CounterUnavailable),
        },
    );

    assert_eq!(outcome.status, SendTransactionStatus::Complete);
    assert_eq!(outcome.evidence.started_ticks, Some(QpcTicks::from_raw(23)));
    assert_eq!(outcome.evidence.completed_ticks, None);
    assert_eq!(outcome.evidence.confirmed_mask, 1);
}

#[test]
fn terminal_up_zero_partial_and_win32_failure_never_confirm_transport() {
    let packet = super::super::outcome::PhysicalPacket::new(0b0011, 0);
    for (inserted, win32_error, expected_status) in [
        (0, 5, SendTransactionStatus::ZeroProgress),
        (1, 5, SendTransactionStatus::IntegrityLost),
        (2, 5, SendTransactionStatus::IntegrityLost),
    ] {
        let outcome = super::super::up_transaction::terminal_up_outcome(
            packet,
            super::super::outcome::TerminalUpTransportResult {
                requested: 2,
                inserted,
                started_ticks: Some(QpcTicks::from_raw(30)),
                completed_ticks: Some(QpcTicks::from_raw(31)),
                win32_error,
                timing_error: None,
            },
        );
        assert_eq!(outcome.status, expected_status);
        assert_eq!(outcome.evidence.first_inserted, inserted);
        assert_eq!(outcome.evidence.last_win32_error, Some(5));
        assert_eq!(outcome.evidence.confirmed_mask, 0);
    }
}

#[test]
fn terminal_up_qpc_pre_and_post_failures_do_not_reclassify_transport() {
    let packet = super::super::outcome::PhysicalPacket::new(0b0011, 0);
    for (started_ticks, completed_ticks) in [
        (None, Some(QpcTicks::from_raw(31))),
        (Some(QpcTicks::from_raw(30)), None),
        (None, None),
    ] {
        for (inserted, win32_error, expected_status) in [
            (0, 5, SendTransactionStatus::ZeroProgress),
            (1, 5, SendTransactionStatus::IntegrityLost),
            (2, 0, SendTransactionStatus::Complete),
        ] {
            let outcome = super::super::up_transaction::terminal_up_outcome(
                packet,
                super::super::outcome::TerminalUpTransportResult {
                    requested: 2,
                    inserted,
                    started_ticks,
                    completed_ticks,
                    win32_error,
                    timing_error: Some(QpcError::CounterUnavailable),
                },
            );

            assert_eq!(outcome.status, expected_status);
            assert_eq!(outcome.evidence.attempts, 1);
            assert_eq!(outcome.evidence.first_inserted, inserted);
            assert_eq!(
                outcome.evidence.last_win32_error,
                (win32_error != 0).then_some(win32_error)
            );
            assert_eq!(outcome.evidence.started_ticks, started_ticks);
            assert_eq!(outcome.evidence.completed_ticks, completed_ticks);
            assert_eq!(
                outcome.evidence.timing_error,
                Some(QpcError::CounterUnavailable)
            );
        }
    }
}

#[test]
fn terminal_zero_insert_with_all_up_probe_keeps_physical_verdict_and_obligation_separate() {
    let mut state = TrackedKeyState::new();
    state.active_mask = 1;
    state.set_terminal_up_emitter_for_test(|packet| {
        super::super::outcome::TerminalUpTransportResult {
            requested: packet.event_count(),
            inserted: 0,
            started_ticks: None,
            completed_ticks: None,
            win32_error: 5,
            timing_error: Some(QpcError::CounterUnavailable),
        }
    });
    state.set_probe(|_, _| InstrumentPhysicalState::AllUp);

    let outcome = state.release_terminal_tracked_scope_for_test(77);
    assert!(outcome.release_outcome.transport_anomaly);
    assert!(!outcome.release_outcome.released_successfully);
    assert!(!outcome.release_outcome.verification_inconclusive);
    assert_eq!(outcome.release_outcome.stuck_mask, 1);
    assert_eq!(
        outcome.probe_verdict,
        super::super::outcome::TerminalProbeVerdict::NotRequired
    );
    assert_eq!(outcome.send_attempts.len(), 4);
    assert!(outcome.send_attempts.iter().all(|attempt| {
        attempt.raw_calls == 1
            && attempt.requested_events == 1
            && attempt.inserted_events == 0
            && attempt.win32_error == Some(5)
            && attempt.started_ticks.is_none()
            && attempt.completed_ticks.is_none()
            && attempt.qpc_error == Some(QpcError::CounterUnavailable)
    }));
    assert_eq!(state.release_obligation_mask(), 1);
}

#[test]
fn terminal_partial_up_reports_only_the_physical_held_mask() {
    let mut state = TrackedKeyState::new();
    state.active_mask = 0b0001;
    state.set_terminal_up_emitter_for_test(|packet| {
        super::super::outcome::TerminalUpTransportResult {
            requested: packet.event_count(),
            inserted: packet.event_count(),
            started_ticks: Some(QpcTicks::from_raw(10)),
            completed_ticks: Some(QpcTicks::from_raw(11)),
            win32_error: 0,
            timing_error: None,
        }
    });
    state.set_probe(|_, _| InstrumentPhysicalState::Held(vec![0x15].into()));

    let outcome = state.release_terminal_tracked_scope_for_test(77);

    assert_eq!(
        outcome.probe_verdict,
        super::super::outcome::TerminalProbeVerdict::Held(0b0001)
    );
    assert_eq!(outcome.release_outcome.stuck_mask, 0b0001);
    assert!(!outcome.release_outcome.released_successfully);
    assert!(!outcome.release_outcome.verification_inconclusive);
    assert_eq!(state.release_obligation_mask(), 0b0001);
    assert_eq!(state.needs_probe_mask, 0b0001);
    assert_eq!(outcome.release_outcome.attempts, 1);
}

#[test]
fn terminal_up_cleanup_retains_inconclusive_obligations_and_is_idempotent_after_probe() {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let sent_for_emitter = Arc::clone(&sent);
    let mut state = TrackedKeyState::new();
    state.active_mask = 0b0001;
    state.possibly_active_mask = 0b0010;
    state.failed_release_mask = 0b0100;
    state.in_flight_mask = 0b1000;
    state.set_terminal_up_emitter_for_test(move |packet| {
        sent_for_emitter
            .lock()
            .expect("terminal packet lock")
            .push(packet);
        super::super::outcome::TerminalUpTransportResult {
            requested: packet.event_count(),
            inserted: packet.event_count(),
            started_ticks: None,
            completed_ticks: Some(QpcTicks::from_raw(41)),
            win32_error: 0,
            timing_error: Some(QpcError::CounterUnavailable),
        }
    });
    state.set_probe(|_, _| InstrumentPhysicalState::Inconclusive);

    let inconclusive = state.release_terminal_tracked_scope_for_test(77);
    assert!(!inconclusive.release_outcome.released_successfully);
    assert!(inconclusive.release_outcome.verification_inconclusive);
    assert_eq!(inconclusive.release_outcome.attempted_mask, 0b1111);
    assert_eq!(inconclusive.release_outcome.attempts, 1);
    assert_eq!(inconclusive.send_attempts.len(), 1);
    assert!(inconclusive.send_attempts.iter().all(|attempt| {
        attempt.raw_calls == 1
            && attempt.requested_events == 4
            && attempt.inserted_events == 4
            && attempt.win32_error.is_none()
            && attempt.started_ticks.is_none()
            && attempt.completed_ticks == Some(QpcTicks::from_raw(41))
            && attempt.qpc_error == Some(QpcError::CounterUnavailable)
    }));
    assert_eq!(
        inconclusive.probe_verdict,
        super::super::outcome::TerminalProbeVerdict::Inconclusive(0b1111)
    );
    assert_eq!(state.release_obligation_mask(), 0b1111);
    assert_eq!(
        state.timing_error, None,
        "terminal QPC errors stay separate"
    );
    assert!(
        sent.lock()
            .expect("terminal packet lock")
            .iter()
            .all(|packet| *packet == super::super::outcome::PhysicalPacket::new(0b1111, 0))
    );

    state.set_probe(|_, _| InstrumentPhysicalState::AllUp);
    let verified = state.release_terminal_tracked_scope_for_test(77);
    assert!(verified.release_outcome.released_successfully);
    assert!(!verified.release_outcome.transport_anomaly);
    assert_eq!(verified.release_outcome.attempts, 0);
    assert_eq!(
        verified.probe_verdict,
        super::super::outcome::TerminalProbeVerdict::AllUp
    );
    assert_eq!(state.release_obligation_mask(), 0);
    let attempts_after_verified = sent.lock().expect("terminal packet lock").len();
    let repeated = state.release_terminal_tracked_scope_for_test(77);
    assert!(repeated.release_outcome.released_successfully);
    assert_eq!(repeated.release_outcome.attempts, 0);
    assert!(repeated.send_attempts.is_empty());
    assert_eq!(
        repeated.probe_verdict,
        super::super::outcome::TerminalProbeVerdict::NotRequired
    );
    assert_eq!(
        sent.lock().expect("terminal packet lock").len(),
        attempts_after_verified
    );
}

#[test]
fn terminal_up_emitter_unwind_keeps_session_obligation_for_finalizer_retry() {
    let mut state = TrackedKeyState::new();
    state.active_mask = 0b0100;
    state.set_terminal_up_emitter_for_test(|_| panic!("injected terminal transport unwind"));

    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        state.release_terminal_tracked_scope_for_test(99)
    }));
    assert!(unwind.is_err());
    assert_eq!(state.release_attempts_total, 1);
    assert_eq!(state.release_obligation_mask(), 0b0100);
}

#[test]
fn resumable_release_remains_qpc_gated_while_terminal_release_uses_bounded_fallback() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let terminal_calls = Arc::new(AtomicUsize::new(0));
    let terminal_calls_for_emitter = Arc::clone(&terminal_calls);
    let mut state = TrackedKeyState::new();
    state.active_mask = 1;
    state.set_terminal_up_emitter_for_test(move |packet| {
        terminal_calls_for_emitter.fetch_add(1, Ordering::SeqCst);
        super::super::outcome::TerminalUpTransportResult {
            requested: packet.event_count(),
            inserted: packet.event_count(),
            started_ticks: None,
            completed_ticks: None,
            win32_error: 0,
            timing_error: Some(QpcError::CounterUnavailable),
        }
    });
    state.set_probe(|_, _| InstrumentPhysicalState::AllUp);

    let resumable = state.release_scope(ReleaseScope::Tracked, 70);
    assert!(!resumable.released_successfully);
    assert!(!resumable.verification_inconclusive);
    assert_eq!(terminal_calls.load(Ordering::SeqCst), 0);
    assert_eq!(state.release_obligation_mask(), 1);

    let terminal = state.release_terminal_tracked_scope_for_test(70);
    assert!(terminal.release_outcome.released_successfully);
    assert_eq!(terminal_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.release_obligation_mask(), 0);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MockWindowObservation {
    accepted_count: u8,
    foreground_at_send: Option<isize>,
}

type MockWindowObservations = Arc<Mutex<Vec<MockWindowObservation>>>;

fn mock_window_cleanup_state(
    target_hwnd: isize,
) -> (
    TrackedKeyState,
    Arc<std::sync::atomic::AtomicIsize>,
    Arc<std::sync::atomic::AtomicBool>,
    Arc<std::sync::atomic::AtomicBool>,
    MockWindowObservations,
) {
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

    let foreground = Arc::new(AtomicIsize::new(target_hwnd));
    let identity_valid = Arc::new(AtomicBool::new(true));
    let probe_verified = Arc::new(AtomicBool::new(false));
    let observations = Arc::new(Mutex::new(Vec::new()));
    let foreground_for_send = Arc::clone(&foreground);
    let identity_for_send = Arc::clone(&identity_valid);
    let observations_for_send = Arc::clone(&observations);
    let clock = crate::clock::QpcClock::initialize().expect("mock transport QPC clock");
    let mut backend = TrackedKeyState::with_emitter(move |scan_codes, key_up| {
        assert!(key_up, "window cleanup sends Up only");
        let foreground_at_send = foreground_for_send.load(Ordering::Acquire);
        let receiver_window = identity_for_send
            .load(Ordering::Acquire)
            .then_some(foreground_at_send);
        observations_for_send
            .lock()
            .expect("mock receiver observation lock")
            .push(MockWindowObservation {
                accepted_count: scan_codes.len() as u8,
                foreground_at_send: receiver_window,
            });
        let started_ticks = clock.now().expect("mock transport start QPC");
        let completed_ticks = clock.now().expect("mock transport completion QPC");
        PlatformSendResult {
            requested: scan_codes.len() as u8,
            inserted: scan_codes.len() as u8,
            started_ticks,
            completed_ticks: Some(completed_ticks),
            win32_error: 0,
            timing_error: None,
        }
    });
    let foreground_for_probe = Arc::clone(&foreground);
    let identity_for_probe = Arc::clone(&identity_valid);
    let probe_for_probe = Arc::clone(&probe_verified);
    backend.set_probe(move |unresolved, confirmed| {
        if identity_for_probe.load(Ordering::Acquire)
            && foreground_for_probe.load(Ordering::Acquire) == target_hwnd
            && probe_for_probe.load(Ordering::Acquire)
            && confirmed & unresolved == unresolved
        {
            InstrumentPhysicalState::AllUp
        } else {
            InstrumentPhysicalState::Inconclusive
        }
    });
    (
        backend,
        foreground,
        identity_valid,
        probe_verified,
        observations,
    )
}

#[test]
fn mock_alt_tab_resume_separates_transport_receiver_and_release_probe() {
    use std::sync::atomic::Ordering;

    const TARGET: isize = 41;
    const OTHER: isize = 42;
    let (mut backend, foreground, _, probe_verified, observations) =
        mock_window_cleanup_state(TARGET);
    backend.active_mask = 1;

    foreground.store(OTHER, Ordering::Release);
    let interrupted = backend.release_scope(ReleaseScope::Tracked, TARGET);
    assert!(!interrupted.transport_anomaly);
    assert!(interrupted.verification_inconclusive);
    assert_eq!(interrupted.attempts, 1);
    assert_eq!(backend.release_obligation_mask(), 1);
    assert_eq!(
        observations
            .lock()
            .expect("mock receiver observation lock")
            .as_slice(),
        &[MockWindowObservation {
            accepted_count: 1,
            foreground_at_send: Some(OTHER),
        }],
        "the mock records foreground-at-send separately from physical verification"
    );

    foreground.store(TARGET, Ordering::Release);
    probe_verified.store(true, Ordering::Release);
    let resumed = backend.release_scope(ReleaseScope::Tracked, TARGET);
    assert!(resumed.released_successfully);
    assert_eq!(backend.release_obligation_mask(), 0);
    assert_eq!(
        observations
            .lock()
            .expect("mock receiver observation lock")
            .last()
            .map(|observation| observation.foreground_at_send),
        Some(Some(OTHER)),
        "a completed Up is verified after focus returns without replaying it"
    );
}

#[test]
fn mock_stop_with_other_foreground_does_not_claim_target_release() {
    use std::sync::atomic::Ordering;

    const TARGET: isize = 51;
    const OTHER: isize = 52;
    let (mut backend, foreground, _, _, observations) = mock_window_cleanup_state(TARGET);
    backend.active_mask = 1;
    foreground.store(OTHER, Ordering::Release);

    let terminal = backend.release_terminal_tracked_scope_for_test(TARGET);
    assert!(!terminal.release_outcome.transport_anomaly);
    assert!(terminal.release_outcome.verification_inconclusive);
    assert_eq!(terminal.release_outcome.attempts, 1);
    assert_eq!(backend.release_obligation_mask(), 1);
    assert_eq!(
        observations
            .lock()
            .expect("mock receiver observation lock")
            .as_slice(),
        &[MockWindowObservation {
            accepted_count: 1,
            foreground_at_send: Some(OTHER),
        }]
    );
}

#[test]
fn mock_invalidated_owner_identity_keeps_release_uncertain_after_accepted_up() {
    use std::sync::atomic::Ordering;

    const TARGET: isize = 61;
    let (mut backend, _, identity_valid, _, observations) = mock_window_cleanup_state(TARGET);
    backend.active_mask = 1;
    identity_valid.store(false, Ordering::Release);

    let invalidated = backend.release_terminal_tracked_scope_for_test(TARGET);
    assert!(invalidated.release_outcome.verification_inconclusive);
    assert_eq!(invalidated.release_outcome.attempts, 1);
    assert_eq!(backend.release_obligation_mask(), 1);
    assert_eq!(
        observations
            .lock()
            .expect("mock receiver observation lock")
            .as_slice(),
        &[MockWindowObservation {
            accepted_count: 1,
            foreground_at_send: None,
        }],
        "invalid identity makes receiver attribution unavailable"
    );
}

#[test]
fn guarded_terminal_cleanup_revalidates_each_retry_and_keeps_prior_raw_evidence() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TARGET: isize = 71;
    let send_calls = Arc::new(AtomicUsize::new(0));
    let sends = Arc::clone(&send_calls);
    let mut backend = TrackedKeyState::new();
    backend.active_mask = 1;
    backend.set_release_owner_proof_for_test(
        1,
        crate::input::ReleaseOwnerProof::new(TARGET, 3, None, None).expect("test release origin"),
    );
    backend.set_terminal_up_emitter_for_test(move |packet| {
        sends.fetch_add(1, Ordering::AcqRel);
        super::super::outcome::TerminalUpTransportResult {
            requested: packet.event_count(),
            inserted: 0,
            started_ticks: None,
            completed_ticks: None,
            win32_error: 5,
            timing_error: Some(QpcError::CounterUnavailable),
        }
    });
    backend.set_probe(|_, _| InstrumentPhysicalState::Inconclusive);

    let mut admissions = 0;
    let outcome =
        backend.release_terminal_tracked_scope_with_attempt_admission(TARGET, |mask, proof| {
            admissions += 1;
            assert_eq!(mask, 1);
            assert_eq!(proof.target_hwnd(), TARGET);
            if admissions == 1 {
                Ok(())
            } else {
                Err("owner invalidated during cleanup retry".into())
            }
        });

    assert_eq!(admissions, 2, "the retry must revalidate its origin");
    assert_eq!(
        send_calls.load(Ordering::Acquire),
        1,
        "no second Up after invalidation"
    );
    assert_eq!(outcome.release_outcome.attempts, 1);
    assert_eq!(outcome.release_outcome.attempted_mask, 1);
    assert_eq!(outcome.release_outcome.stuck_mask, 1);
    assert_eq!(backend.release_obligation_mask(), 1);
    assert_eq!(
        outcome.withheld_reason.as_deref(),
        Some("owner invalidated during cleanup retry")
    );
    assert_eq!(outcome.send_attempts.len(), 1);
    assert_eq!(outcome.send_attempts[0].raw_calls, 1);
    assert_eq!(outcome.send_attempts[0].inserted_events, 0);
    assert_eq!(outcome.send_attempts[0].win32_error, Some(5));
    assert_eq!(
        outcome.send_attempts[0].qpc_error,
        Some(QpcError::CounterUnavailable)
    );
    assert_eq!(outcome.probe_verdict, TerminalProbeVerdict::NotRequired);
}

#[test]
fn guarded_terminal_cleanup_allows_healthy_retry_and_zero_obligation_is_noop() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TARGET: isize = 81;
    let send_calls = Arc::new(AtomicUsize::new(0));
    let sends = Arc::clone(&send_calls);
    let probes = Arc::new(AtomicUsize::new(0));
    let probe_count = Arc::clone(&probes);
    let mut backend = TrackedKeyState::new();
    backend.active_mask = 1;
    backend.set_release_owner_proof_for_test(
        1,
        crate::input::ReleaseOwnerProof::new(TARGET, 4, None, None).expect("test release origin"),
    );
    backend.set_emitter(move |scan_codes, key_up| {
        assert!(key_up);
        sends.fetch_add(1, Ordering::AcqRel);
        PlatformSendResult {
            requested: scan_codes.len() as u8,
            inserted: scan_codes.len() as u8,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            win32_error: 0,
            timing_error: None,
        }
    });
    backend.set_probe(move |_, _| {
        if probe_count.fetch_add(1, Ordering::AcqRel) == 0 {
            InstrumentPhysicalState::Held(vec![0x15].into())
        } else {
            InstrumentPhysicalState::AllUp
        }
    });

    let mut admissions = 0;
    let released = backend.release_terminal_tracked_scope_with_attempt_admission(TARGET, |_, _| {
        admissions += 1;
        Ok(())
    });
    assert!(released.release_outcome.released_successfully);
    assert_eq!(admissions, 1);
    assert_eq!(released.release_outcome.attempts, 1);
    assert_eq!(released.send_attempts.len(), 1);
    assert_eq!(send_calls.load(Ordering::Acquire), 1);

    let mut empty = TrackedKeyState::new();
    let empty_outcome = empty
        .release_terminal_tracked_scope_with_attempt_admission(TARGET, |_, _| {
            panic!("zero-obligation cleanup must not request owner proof")
        });
    assert_eq!(empty_outcome.release_outcome.attempted_mask, 0);
    assert_eq!(empty_outcome.release_outcome.attempts, 0);
    assert!(empty_outcome.send_attempts.is_empty());
    assert_eq!(
        empty_outcome.probe_verdict,
        TerminalProbeVerdict::NotRequired
    );
    assert_eq!(empty.release_obligation_mask(), 0);
}

#[test]
fn guarded_terminal_cleanup_retains_partial_and_uncertain_up_evidence_when_retry_is_withheld() {
    const TARGET: isize = 91;
    let mut backend = TrackedKeyState::new();
    backend.active_mask = 0b11;
    backend.set_release_owner_proof_for_test(
        0b11,
        crate::input::ReleaseOwnerProof::new(TARGET, 5, None, None).expect("test release origin"),
    );
    backend.set_terminal_up_emitter_for_test(|packet| {
        super::super::outcome::TerminalUpTransportResult {
            requested: packet.event_count(),
            inserted: 1,
            started_ticks: Some(QpcTicks::from_raw(10)),
            completed_ticks: Some(QpcTicks::from_raw(11)),
            win32_error: 5,
            timing_error: Some(QpcError::CounterUnavailable),
        }
    });
    backend.set_probe(|_, _| InstrumentPhysicalState::Held(vec![0x15].into()));

    let mut admissions = 0;
    let outcome = backend.release_terminal_tracked_scope_with_attempt_admission(TARGET, |_, _| {
        admissions += 1;
        if admissions == 1 {
            Ok(())
        } else {
            Err("origin proof unavailable".into())
        }
    });

    assert_eq!(outcome.release_outcome.attempts, 1);
    assert_eq!(outcome.release_outcome.attempted_mask, 0b11);
    assert_eq!(outcome.release_outcome.stuck_mask, 0b11);
    assert_eq!(backend.release_obligation_mask(), 0b11);
    assert_eq!(outcome.send_attempts.len(), 1);
    assert_eq!(outcome.send_attempts[0].raw_calls, 1);
    assert_eq!(outcome.send_attempts[0].inserted_events, 1);
    assert_eq!(
        outcome.send_attempts[0].qpc_error,
        Some(QpcError::CounterUnavailable)
    );
    assert_eq!(outcome.probe_verdict, TerminalProbeVerdict::NotRequired);
    assert_eq!(
        outcome.withheld_reason.as_deref(),
        Some("origin proof unavailable")
    );
}

#[test]
fn stop_upgrades_a_pause_drain_without_resetting_its_bounded_sender() {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TARGET: isize = 101;
    let resumable_calls = Arc::new(AtomicUsize::new(0));
    let terminal_calls = Arc::new(AtomicUsize::new(0));
    let resumable_calls_for_emitter = Arc::clone(&resumable_calls);
    let terminal_calls_for_emitter = Arc::clone(&terminal_calls);
    let mut backend = TrackedKeyState::new();
    backend.active_mask = 1;
    backend.set_release_owner_proof_for_test(
        1,
        crate::input::ReleaseOwnerProof::new(TARGET, 6, None, None).expect("test release origin"),
    );
    backend.set_emitter(move |codes, key_up| {
        assert!(key_up);
        resumable_calls_for_emitter.fetch_add(1, Ordering::AcqRel);
        test_send_result(codes.len() as u8, 0, 5)
    });
    backend.set_terminal_up_emitter_for_test(move |packet| {
        terminal_calls_for_emitter.fetch_add(1, Ordering::AcqRel);
        super::super::outcome::TerminalUpTransportResult {
            requested: packet.event_count(),
            inserted: packet.event_count(),
            started_ticks: Some(QpcTicks::ZERO),
            completed_ticks: Some(QpcTicks::ZERO),
            win32_error: 0,
            timing_error: None,
        }
    });
    backend.set_probe(|_, _| InstrumentPhysicalState::AllUp);

    let drain_sample = Cell::new(0usize);
    let outcome = backend.release_tracked_with_attempt_admission_and_drain_control(
        TARGET,
        |_, proof| {
            assert_eq!(proof.target_hwnd(), TARGET);
            Ok(())
        },
        || {
            let sample = drain_sample.get();
            drain_sample.set(sample + 1);
            Ok(crate::input::CleanupDrainDecision {
                terminal_requested: sample >= 2,
                origin_foreground: true,
            })
        },
    );

    assert!(outcome.release_outcome.released_successfully);
    assert_eq!(outcome.release_outcome.attempts, 2);
    assert_eq!(resumable_calls.load(Ordering::Acquire), 1);
    assert_eq!(terminal_calls.load(Ordering::Acquire), 1);
    assert_eq!(outcome.send_attempts.len(), 2);
    assert_eq!(outcome.send_attempts[0].raw_calls, 1);
    assert_eq!(outcome.send_attempts[1].raw_calls, 1);
    assert_eq!(backend.release_obligation_mask(), 0);
}

#[test]
fn terminal_background_drain_sends_one_best_effort_up_and_keeps_probe_residual() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    const TARGET: isize = 111;
    let send_calls = Arc::new(AtomicUsize::new(0));
    let probe_calls = Arc::new(AtomicUsize::new(0));
    let sends = Arc::clone(&send_calls);
    let probes = Arc::clone(&probe_calls);
    let mut backend = TrackedKeyState::new();
    backend.active_mask = 1;
    backend.set_release_owner_proof_for_test(
        1,
        crate::input::ReleaseOwnerProof::new(TARGET, 7, None, None).expect("test release origin"),
    );
    backend.set_terminal_up_emitter_for_test(move |packet| {
        sends.fetch_add(1, Ordering::AcqRel);
        super::super::outcome::TerminalUpTransportResult {
            requested: packet.event_count(),
            inserted: packet.event_count(),
            started_ticks: None,
            completed_ticks: None,
            win32_error: 0,
            timing_error: None,
        }
    });
    backend.set_probe(move |_, _| {
        probes.fetch_add(1, Ordering::AcqRel);
        InstrumentPhysicalState::AllUp
    });

    let mut drain_samples = 0;
    let outcome = backend.release_terminal_tracked_scope_with_attempt_admission_and_drain_control(
        TARGET,
        |_, proof| {
            assert_eq!(proof.target_hwnd(), TARGET);
            Ok(())
        },
        || {
            drain_samples += 1;
            Ok(crate::input::CleanupDrainDecision {
                terminal_requested: true,
                origin_foreground: false,
            })
        },
    );

    assert_eq!(drain_samples, 2, "drain rechecks immediately before send");
    assert_eq!(send_calls.load(Ordering::Acquire), 1);
    assert_eq!(probe_calls.load(Ordering::Acquire), 0);
    assert_eq!(outcome.release_outcome.attempts, 1);
    assert!(!outcome.release_outcome.released_successfully);
    assert!(outcome.release_outcome.verification_inconclusive);
    assert_eq!(outcome.needs_up_mask, 0);
    assert_eq!(outcome.needs_probe_mask, 1);
    assert_eq!(backend.release_obligation_mask(), 1);
    assert_eq!(outcome.send_attempts.len(), 1);
    assert_eq!(outcome.send_attempts[0].raw_calls, 1);
}

#[test]
fn origin_drift_at_the_final_send_boundary_withholds_the_up() {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const TARGET: isize = 121;
    let origin_current = Arc::new(AtomicBool::new(true));
    let send_calls = Arc::new(AtomicUsize::new(0));
    let origin_for_admission = Arc::clone(&origin_current);
    let sends = Arc::clone(&send_calls);
    let mut backend = TrackedKeyState::new();
    backend.active_mask = 1;
    backend.set_release_owner_proof_for_test(
        1,
        crate::input::ReleaseOwnerProof::new(TARGET, 8, None, None).expect("test release origin"),
    );
    backend.set_terminal_up_emitter_for_test(move |packet| {
        sends.fetch_add(1, Ordering::AcqRel);
        super::super::outcome::TerminalUpTransportResult {
            requested: packet.event_count(),
            inserted: packet.event_count(),
            started_ticks: None,
            completed_ticks: None,
            win32_error: 0,
            timing_error: None,
        }
    });

    let drain_sample = Cell::new(0usize);
    let admission_calls = Cell::new(0usize);
    let outcome = backend.release_tracked_with_attempt_admission_and_drain_control(
        TARGET,
        |_, proof| {
            admission_calls.set(admission_calls.get() + 1);
            assert!(origin_for_admission.load(Ordering::Acquire));
            assert_eq!(proof.target_hwnd(), TARGET);
            Ok(())
        },
        || {
            let sample = drain_sample.get();
            drain_sample.set(sample + 1);
            if sample == 0 {
                Ok(crate::input::CleanupDrainDecision::default())
            } else {
                origin_current.store(false, Ordering::Release);
                Err("origin drift at final send boundary".into())
            }
        },
    );

    assert_eq!(drain_sample.get(), 2);
    assert_eq!(admission_calls.get(), 1);
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
    assert_eq!(outcome.release_outcome.attempts, 0);
    assert_eq!(outcome.needs_up_mask, 1);
    assert_eq!(outcome.needs_probe_mask, 0);
    assert_eq!(backend.release_obligation_mask(), 1);
    assert_eq!(
        outcome.withheld_reason.as_deref(),
        Some("origin drift at final send boundary")
    );
}

#[test]
fn cleanup_up_uses_the_session_layout_profile_and_zero_mask_emits_nothing() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mut spec = InstrumentKeyProfileSpec::canonical();
    spec.keys[0] = PhysicalKey {
        scan_code: 0x39,
        extended: false,
    };
    spec.keys[1] = PhysicalKey {
        scan_code: 0x03,
        extended: false,
    };
    let profile = MaterializedInstrumentKeyProfile::from_validated(
        InstrumentKeyProfile::try_from_spec(spec).expect("valid swapped layout"),
    );
    let captured_codes = Arc::new(Mutex::new(Vec::new()));
    let captures = Arc::clone(&captured_codes);
    let mut backend = TrackedKeyState::with_profile(profile);
    backend.active_mask = 0b0011;
    backend.set_emitter(move |codes, key_up| {
        assert!(key_up);
        captures
            .lock()
            .expect("profile cleanup capture lock")
            .extend_from_slice(codes);
        test_send_result(codes.len() as u8, codes.len() as u8, 0)
    });
    backend.set_probe(|_, _| InstrumentPhysicalState::AllUp);
    let outcome = backend.release_scope(ReleaseScope::Tracked, 0);

    assert!(outcome.released_successfully);
    assert_eq!(
        captured_codes
            .lock()
            .expect("profile cleanup capture lock")
            .as_slice(),
        &[0x39, 0x03]
    );

    let zero_mask_send_calls = Arc::new(AtomicUsize::new(0));
    let sends = Arc::clone(&zero_mask_send_calls);
    let mut empty = TrackedKeyState::new();
    empty.set_terminal_up_emitter_for_test(move |_| {
        sends.fetch_add(1, Ordering::AcqRel);
        unreachable!("an empty journal cannot authorize an Up")
    });
    let zero = empty.release_terminal_tracked_scope_for_test(0);
    assert_eq!(zero.release_outcome.attempts, 0);
    assert_eq!(zero.release_outcome.attempted_mask, 0);
    assert!(zero.send_attempts.is_empty());
    assert_eq!(zero_mask_send_calls.load(Ordering::Acquire), 0);
}
