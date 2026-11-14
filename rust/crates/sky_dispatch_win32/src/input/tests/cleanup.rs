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
    assert_eq!(outcome.attempts, 4);
    assert_eq!(state.release_obligation_mask(), 1);
    assert_eq!(send_count.load(std::sync::atomic::Ordering::SeqCst), 4);
}

#[test]
fn custom_full_instrument_cleanup_preserves_all_fifteen_uncertain_slots_until_verified() {
    let mut spec = InstrumentKeyProfileSpec::canonical();
    for (slot, scan_code) in super::super::profile::SUPPORTED_NON_EXTENDED_SCAN_CODES
        .iter()
        .copied()
        .take(15)
        .enumerate()
    {
        spec.keys[slot] = PhysicalKey {
            scan_code,
            extended: false,
        };
    }
    let profile = MaterializedInstrumentKeyProfile::from_validated(
        InstrumentKeyProfile::try_from_spec(spec).expect("custom instrument profile"),
    );
    let verified = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let verified_for_probe = Arc::clone(&verified);
    let mut state = TrackedKeyState::with_profile(profile);
    state.set_emitter(|codes, _| test_send_result(codes.len() as u8, codes.len() as u8, 0));
    state.set_probe(move |_, _| {
        if verified_for_probe.load(std::sync::atomic::Ordering::Acquire) {
            InstrumentPhysicalState::AllUp
        } else {
            InstrumentPhysicalState::Inconclusive
        }
    });

    let first = state.release_all_full_instrument(0);
    assert!(!first.released_successfully);
    assert!(first.verification_inconclusive);
    assert_eq!(first.attempted_mask, 0x7fff);
    assert_eq!(first.stuck_mask, 0x7fff);
    assert_eq!(first.attempts, 4);
    assert_eq!(state.release_obligation_mask(), 0x7fff);
    assert_eq!(state.failed_release_mask, 0x7fff);
    assert_eq!(state.release_attempts_total, 4);
    assert_eq!(
        first.stuck_keys_with_profile(state.instrument_key_profile()),
        state
            .instrument_key_profile()
            .scan_codes_from_mask(0x7fff)
            .into_vec()
    );

    verified.store(true, std::sync::atomic::Ordering::Release);
    let second = state.release_all_full_instrument(0);
    assert!(second.released_successfully);
    assert!(!second.verification_inconclusive);
    assert_eq!(second.attempted_mask, 0x7fff);
    assert_eq!(second.stuck_mask, 0);
    assert_eq!(second.attempts, 1);
    assert_eq!(state.release_obligation_mask(), 0);
    assert_eq!(state.release_attempts_total, 5);
}

#[test]
fn partial_inserted_without_rollback_is_integrity_lost() {
    let outcome = emit_down_with(&[0x15, 0x16], |codes, key_up| {
        test_send_result(codes.len() as u8, if key_up { 2 } else { 1 }, 0)
    });
    assert_eq!(outcome.status, SendTransactionStatus::IntegrityLost);
    assert!(!outcome.is_success());
    assert_eq!(outcome.evidence.first_inserted, 1);
    assert_eq!(outcome.evidence.confirmed_mask, 0);
}

#[test]
fn tracked_partial_down_keeps_uncertain_keys_until_verified_cleanup() {
    let calls = Arc::new(Mutex::new(0usize));
    let calls_for_emitter = Arc::clone(&calls);
    let mut state = TrackedKeyState::with_emitter(move |codes, key_up| {
        *calls_for_emitter.lock().expect("partial call count") += 1;
        if key_up {
            test_send_result(codes.len() as u8, codes.len() as u8, 0)
        } else {
            test_send_result(codes.len() as u8, 1, 5)
        }
    });

    let outcome = state.key_down(&[0x15, 0x16]);
    assert_eq!(outcome.status, SendTransactionStatus::IntegrityLost);
    assert_eq!(*calls.lock().expect("partial call count"), 1);
    let obligation = state.release_obligation_mask();
    assert_ne!(obligation, 0);
    assert_eq!(obligation & 0b11, 0b11);

    state.custom_probe = Some(Box::new(|_, _| InstrumentPhysicalState::AllUp));
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
fn full_inserted_with_win32_error_is_integrity_lost() {
    let outcome = emit_down_with(&[0x15, 0x16], |codes, _| {
        let len = codes.len() as u8;
        test_send_result(len, len, 5)
    });
    assert_eq!(outcome.status, SendTransactionStatus::IntegrityLost);
    assert!(!outcome.is_success());
    assert_eq!(outcome.evidence.first_win32_error, Some(5));
    assert_eq!(outcome.evidence.confirmed_mask, 0);
}

#[test]
fn missing_completion_qpc_after_send_is_clock_failure_after_send() {
    let outcome = emit_down_with(&[0x15, 0x16], |codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: codes.len() as u8,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: None,
        win32_error: 0,
        timing_error: None,
    });
    assert_eq!(outcome.status, SendTransactionStatus::ClockFailureAfterSend);
    assert!(!outcome.is_success());
    assert_eq!(outcome.evidence.confirmed_mask, 0);
}

#[test]
fn zero_progress_retry_only_runs_when_first_inserted_is_zero() {
    let mut call_count = 0;
    let outcome = emit_down_with(&[0x15, 0x16], |codes, _| {
        call_count += 1;
        let len = codes.len() as u8;
        if call_count == 1 {
            test_send_result(len, 1, 0)
        } else {
            test_send_result(len, 2, 0)
        }
    });
    assert_eq!(call_count, 2);
    assert_eq!(outcome.status, SendTransactionStatus::IntegrityLost);
    assert_eq!(outcome.evidence.zero_progress_retries, 0);
}

#[test]
fn up_send_failure_leaves_requested_release_unconfirmed() {
    let mut state = TrackedKeyState::with_emitter(|codes, key_up| {
        let len = codes.len() as u8;
        if key_up {
            test_send_result(len, 0, 5)
        } else {
            test_send_result(len, len, 0)
        }
    });

    let down_outcome = state.key_down(&[0x15]);
    assert_eq!(down_outcome.status, SendTransactionStatus::Complete);
    assert_ne!(state.active_mask & (1 << 0), 0);

    let up_outcome = state.key_up(&[0x15]);
    assert_ne!(up_outcome.status, SendTransactionStatus::Complete);
    assert_ne!(state.release_obligation_mask() & (1 << 0), 0);
}

#[test]
fn up_send_success_clears_requested_release() {
    let mut state = TrackedKeyState::with_emitter(|codes, _| {
        let len = codes.len() as u8;
        test_send_result(len, len, 0)
    });
    let down_outcome = state.key_down(&[0x15]);
    assert_eq!(down_outcome.status, SendTransactionStatus::Complete);
    let up_outcome = state.key_up(&[0x15]);
    assert_eq!(up_outcome.status, SendTransactionStatus::Complete);
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
fn full_cleanup_emits_custom_profile_keys_from_logical_mask() {
    let mut spec = InstrumentKeyProfileSpec::canonical();
    for (slot, scan_code) in super::super::profile::SUPPORTED_NON_EXTENDED_SCAN_CODES
        .iter()
        .copied()
        .take(15)
        .enumerate()
    {
        spec.keys[slot] = PhysicalKey {
            scan_code,
            extended: false,
        };
    }
    let profile = MaterializedInstrumentKeyProfile::from_validated(
        InstrumentKeyProfile::try_from_spec(spec).expect("custom profile"),
    );
    let emitted = Arc::new(Mutex::new(Vec::<(Vec<u16>, bool)>::new()));
    let emitted_copy = Arc::clone(&emitted);
    let mut state = TrackedKeyState::with_profile(profile);
    state.set_emitter(move |codes, key_up| {
        emitted_copy
            .lock()
            .expect("emitted lock")
            .push((codes.to_vec(), key_up));
        test_send_result(codes.len() as u8, codes.len() as u8, 0)
    });
    state.set_probe(|_, _| InstrumentPhysicalState::AllUp);

    let outcome = state.release_all_full_instrument(0);
    assert!(outcome.released_successfully);
    let calls = emitted.lock().expect("emitted lock");
    assert_eq!(calls.len(), 1);
    assert!(calls[0].1);
    assert_eq!(
        calls[0].0,
        vec![
            0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x10, 0x11,
            0x12
        ]
    );
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
    assert!(outcome.verification_inconclusive);
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

    assert_eq!(outcome.status, SendTransactionStatus::ClockFailureAfterSend);
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

    assert_eq!(outcome.status, SendTransactionStatus::ClockFailureAfterSend);
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

    let inconclusive = state.release_terminal_tracked_scope(77);
    assert!(!inconclusive.released_successfully);
    assert!(inconclusive.verification_inconclusive);
    assert_eq!(inconclusive.attempted_mask, 0b1111);
    assert_eq!(inconclusive.attempts, 4);
    assert_eq!(state.release_obligation_mask(), 0b1111);
    assert_eq!(state.timing_error, Some(QpcError::CounterUnavailable));
    assert!(
        sent.lock()
            .expect("terminal packet lock")
            .iter()
            .all(|packet| *packet == super::super::outcome::PhysicalPacket::new(0b1111, 0))
    );

    state.set_probe(|_, _| InstrumentPhysicalState::AllUp);
    let verified = state.release_terminal_tracked_scope(77);
    assert!(verified.released_successfully);
    assert_eq!(state.release_obligation_mask(), 0);
    let attempts_after_verified = sent.lock().expect("terminal packet lock").len();
    let repeated = state.release_terminal_tracked_scope(77);
    assert!(repeated.released_successfully);
    assert_eq!(repeated.attempts, 0);
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
        state.release_terminal_tracked_scope(99)
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
    assert!(resumable.verification_inconclusive);
    assert_eq!(terminal_calls.load(Ordering::SeqCst), 0);
    assert_eq!(state.release_obligation_mask(), 1);

    let terminal = state.release_terminal_tracked_scope(70);
    assert!(terminal.released_successfully);
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
    assert_eq!(interrupted.attempts, 4);
    assert_eq!(backend.release_obligation_mask(), 1);
    assert_eq!(
        observations
            .lock()
            .expect("mock receiver observation lock")
            .as_slice(),
        &[MockWindowObservation {
            accepted_count: 1,
            foreground_at_send: Some(OTHER),
        }; 4],
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
        Some(Some(TARGET))
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

    let terminal = backend.release_terminal_tracked_scope(TARGET);
    assert!(!terminal.transport_anomaly);
    assert!(terminal.verification_inconclusive);
    assert_eq!(terminal.attempts, 4);
    assert_eq!(backend.release_obligation_mask(), 1);
    assert_eq!(
        observations
            .lock()
            .expect("mock receiver observation lock")
            .as_slice(),
        &[MockWindowObservation {
            accepted_count: 1,
            foreground_at_send: Some(OTHER),
        }; 4]
    );
}

#[test]
fn mock_invalidated_owner_identity_keeps_release_uncertain_after_accepted_up() {
    use std::sync::atomic::Ordering;

    const TARGET: isize = 61;
    let (mut backend, _, identity_valid, _, observations) = mock_window_cleanup_state(TARGET);
    backend.active_mask = 1;
    identity_valid.store(false, Ordering::Release);

    let invalidated = backend.release_terminal_tracked_scope(TARGET);
    assert!(invalidated.verification_inconclusive);
    assert_eq!(invalidated.attempts, 4);
    assert_eq!(backend.release_obligation_mask(), 1);
    assert_eq!(
        observations
            .lock()
            .expect("mock receiver observation lock")
            .as_slice(),
        &[MockWindowObservation {
            accepted_count: 1,
            foreground_at_send: None,
        }; 4],
        "invalid identity makes receiver attribution unavailable"
    );
}
