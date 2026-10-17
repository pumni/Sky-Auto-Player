use super::*;

#[test]
fn cleanup_inconclusive_with_transport_complete_fails_closed() {
    let mut state = TrackedKeyState::with_emitter(|codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: codes.len() as u8,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 0,
        timing_error: None,
    });
    // Physical probing is inconclusive even though transport confirmed every
    // key. It must never become a VerifiedAllUp by probing an empty set.
    state.custom_probe = Some(Box::new(|_, _| InstrumentPhysicalState::Inconclusive));
    state.active_mask = 0x0003;

    let outcome = state.release_all(0);
    assert!(!outcome.released_successfully);
    assert!(outcome.verification_inconclusive);
    assert_eq!(outcome.stuck_mask, 0x0003);
    assert_eq!(outcome.attempts, 4);
    // Successful insertion does not prove key-up when foreground verification
    // is inconclusive. Every conservative source retains the obligation.
    assert_eq!(state.active_mask, 0x0003);
    assert_eq!(state.possibly_active_mask, 0);
    assert_eq!(state.failed_release_mask, 0x0003);
    assert_eq!(state.release_obligation_mask(), 0x0003);
    assert!(!outcome.transport_anomaly);
}

#[test]
fn inconclusive_cleanup_retains_obligation_for_second_verified_invocation() {
    let send_count = Arc::new(AtomicUsize::new(0));
    let send_count_for_emitter = Arc::clone(&send_count);
    let mut state = TrackedKeyState::with_emitter(move |codes, _| {
        send_count_for_emitter.fetch_add(1, Ordering::SeqCst);
        test_send_result(codes.len() as u8, codes.len() as u8, 0)
    });
    state.set_probe(|_, _| InstrumentPhysicalState::Inconclusive);
    state.active_mask = 1;

    let first = state.release_scope(ReleaseScope::Tracked, 0);
    assert!(!first.released_successfully);
    assert!(first.verification_inconclusive);
    assert_eq!(first.stuck_mask, 1);
    assert_eq!(first.attempts, 4);
    assert_eq!(state.release_obligation_mask(), 1);
    assert_eq!(state.release_attempts_total, 4);

    state.set_probe(|_, _| InstrumentPhysicalState::AllUp);
    let second = state.release_scope(ReleaseScope::Tracked, 0);
    assert!(second.released_successfully);
    assert!(!second.verification_inconclusive);
    assert_eq!(second.attempted_mask, 1);
    assert_eq!(second.attempts, 1);
    assert_eq!(state.release_obligation_mask(), 0);
    assert_eq!(state.release_attempts_total, 5);
    assert_eq!(send_count.load(Ordering::SeqCst), 5);
}

#[test]
fn cleanup_transport_and_probe_evidence_matrix_is_conservative() {
    let requested = 0b111;
    let cases = [
        // AllUp only resolves keys whose Up insertion was complete.
        (
            "complete",
            0b111,
            InstrumentPhysicalState::AllUp,
            ReconciledRelease::VerifiedAllUp,
        ),
        (
            "partial",
            0b001,
            InstrumentPhysicalState::AllUp,
            ReconciledRelease::Inconclusive(0b110),
        ),
        (
            "zero",
            0b000,
            InstrumentPhysicalState::AllUp,
            ReconciledRelease::Inconclusive(0b111),
        ),
        // Held keys and every transport-uncertain key remain obligations.
        (
            "complete",
            0b111,
            InstrumentPhysicalState::Held(smallvec::smallvec![0x16]),
            ReconciledRelease::Held(0b010),
        ),
        (
            "partial",
            0b001,
            InstrumentPhysicalState::Held(smallvec::smallvec![0x17]),
            ReconciledRelease::Held(0b110),
        ),
        (
            "zero",
            0b000,
            InstrumentPhysicalState::Held(smallvec::smallvec![0x16]),
            ReconciledRelease::Held(0b111),
        ),
        (
            "complete",
            0b111,
            InstrumentPhysicalState::Held(smallvec::smallvec![]),
            ReconciledRelease::Inconclusive(0b111),
        ),
        // An inconclusive physical probe keeps the complete request even when
        // every Up was reported inserted.
        (
            "complete",
            0b111,
            InstrumentPhysicalState::Inconclusive,
            ReconciledRelease::Inconclusive(0b111),
        ),
        (
            "partial",
            0b001,
            InstrumentPhysicalState::Inconclusive,
            ReconciledRelease::Inconclusive(0b111),
        ),
        (
            "zero",
            0b000,
            InstrumentPhysicalState::Inconclusive,
            ReconciledRelease::Inconclusive(0b111),
        ),
    ];

    for (transport, confirmed, physical, expected) in cases {
        let actual = reconcile_release_observation(requested, confirmed, physical.clone());
        assert_eq!(
            actual, expected,
            "transport={transport}, confirmed={confirmed:#05b}, physical={physical:?}"
        );
    }
}

#[test]
fn partial_up_and_held_probe_keep_transport_uncertainty_across_retry_rounds() {
    let send_count = Arc::new(AtomicUsize::new(0));
    let send_count_for_emitter = Arc::clone(&send_count);
    let mut state = TrackedKeyState::with_emitter(move |codes, key_up| {
        assert!(key_up);
        if send_count_for_emitter.fetch_add(1, Ordering::SeqCst) == 0 {
            test_send_result(codes.len() as u8, 1, 5)
        } else {
            test_send_result(codes.len() as u8, codes.len() as u8, 0)
        }
    });
    let probe_count = Arc::new(AtomicUsize::new(0));
    let probe_count_for_closure = Arc::clone(&probe_count);
    let probe_inputs = Arc::new(Mutex::new(Vec::new()));
    let probe_inputs_for_closure = Arc::clone(&probe_inputs);
    state.set_probe(move |mask, confirmed| {
        probe_inputs_for_closure
            .lock()
            .expect("probe input capture")
            .push((mask, confirmed));
        if probe_count_for_closure.fetch_add(1, Ordering::SeqCst) == 0 {
            InstrumentPhysicalState::Held(smallvec::smallvec![0x17])
        } else {
            InstrumentPhysicalState::Inconclusive
        }
    });
    state.active_mask = 0b111;

    let outcome = state.release_scope(ReleaseScope::Tracked, 0);
    assert!(!outcome.released_successfully);
    assert!(outcome.transport_anomaly);
    assert!(outcome.verification_inconclusive);
    assert_eq!(outcome.stuck_mask, 0b110);
    assert_eq!(state.release_obligation_mask(), 0b110);
    assert_eq!(state.release_attempts_total, 4);
    let probes = probe_inputs.lock().expect("probe input capture");
    assert_eq!(probes[0], (0b111, 0b001));
    assert_eq!(
        probes[1].0, 0b110,
        "the unconfirmed key absent from the Held subset remains for retry"
    );
}

#[test]
fn cleanup_retains_transport_uncertain_keys_absent_from_held_subset() {
    let mut state = TrackedKeyState::with_emitter(|codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: 0,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 5,
        timing_error: None,
    });
    // The probe reports only 0x16 (B) held, but neither Up was confirmed by
    // transport. Absence from Held therefore cannot discharge either key.
    state.custom_probe = Some(Box::new(|_, mask| {
        let _ = mask;
        InstrumentPhysicalState::Held(smallvec::smallvec![0x16])
    }));
    state.active_mask = 0x0003;
    state.possibly_active_mask = 0x0003;

    let _outcome = state.release_all(0);
    assert_eq!(state.active_mask, 0x0003);
    assert_eq!(state.possibly_active_mask, 0x0003);
    assert_eq!(state.failed_release_mask, 0x0003);
    assert_eq!(state.release_obligation_mask(), 0x0003);
}
