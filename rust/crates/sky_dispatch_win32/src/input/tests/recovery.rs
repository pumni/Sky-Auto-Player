use super::*;

#[test]
fn custom_emitter_without_probe_never_synthesizes_all_up() {
    // A simulated transport emitter that fully confirms every key must NOT be
    // allowed to invent physical all-up evidence. Without an explicit probe the
    // cleanup FSM resolves Inconclusive and fails closed.
    let mut state = TrackedKeyState::with_emitter(|codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: codes.len() as u8,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 0,
        timing_error: None,
    });
    state.active_mask = 0x0003;

    let outcome = state.release_all(0);
    assert!(
        !outcome.released_successfully,
        "emitter alone must not confirm all-up"
    );
    assert!(outcome.verification_inconclusive);
    assert_eq!(outcome.stuck_mask, 0x0003);
    assert_eq!(state.release_obligation_mask(), 0x0003);
}

#[test]
fn custom_emitter_without_probe_never_synthesizes_held() {
    // A transport emitter reporting failure must not synthesize a Held verdict
    // either; without a probe the result is Inconclusive (fail-closed).
    let mut state = TrackedKeyState::with_emitter(|codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: 0,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 5,
        timing_error: None,
    });
    state.active_mask = 0x0001;

    let outcome = state.release_all(0);
    assert!(!outcome.released_successfully);
    assert!(!outcome.verification_inconclusive);
    assert_eq!(outcome.stuck_keys(), vec![0x15]);
    assert_eq!(state.failed_release_mask, 0x0001);
}

#[test]
fn release_owner_proof_retains_the_original_foreground_policy() {
    let authority = Arc::new(crate::focus::WindowIdentityAuthority::synthetic_for_test(
        77, 91,
    ));
    let focus_required = ReleaseOwnerProof::new_with_foreground_policy(
        77,
        3,
        Some(91),
        Some(Arc::clone(&authority)),
        true,
    )
    .expect("focus-required owner proof");
    let focus_free =
        ReleaseOwnerProof::new_with_foreground_policy(77, 3, Some(91), Some(authority), false)
            .expect("focus-free owner proof");

    assert!(focus_required.foreground_required());
    assert!(!focus_free.foreground_required());
    assert!(!focus_required.same_owner(&focus_free));
    assert!(focus_required.clone().same_owner(&focus_required));
}

#[test]
fn recovery_journal_reconstruction_preserves_only_the_supplied_residual() {
    let hwnd = 78;
    let owner_pid = 92;
    let authority = Arc::new(crate::focus::WindowIdentityAuthority::synthetic_for_test(
        hwnd, owner_pid,
    ));
    let proof = ReleaseOwnerProof::new_with_foreground_policy(
        hwnd,
        4,
        Some(owner_pid),
        Some(authority),
        true,
    )
    .expect("immutable recovery origin");
    let profile = InstrumentKeyProfileSpec::canonical().keys;
    let state = TrackedKeyState::from_terminal_release_residual(
        profile,
        proof.clone(),
        0x0005,
        0x0001,
        0x0004,
    )
    .expect("valid recovery journal");

    assert_eq!(state.release_obligation_mask(), 0x0005);
    assert_eq!(state.needs_up_mask(), 0x0001);
    assert_eq!(state.needs_probe_mask, 0x0004);
    assert!(
        state
            .session_release_owner_proof()
            .is_some_and(|actual| actual.same_owner(&proof))
    );
    assert!(
        TrackedKeyState::from_terminal_release_residual(
            profile,
            proof.clone(),
            0x0001,
            0x0001,
            0x0001,
        )
        .is_err()
    );
}

#[test]
fn test_send_transaction_status_exhaustive_match() {
    let statuses = [
        SendTransactionStatus::Complete,
        SendTransactionStatus::PreparationRejected,
        SendTransactionStatus::ZeroProgress,
        SendTransactionStatus::PartialProgress,
        SendTransactionStatus::IntegrityLost,
        SendTransactionStatus::ClockFailureBeforeSend,
        SendTransactionStatus::ClockFailureAfterSend,
    ];
    for status in statuses {
        match status {
            SendTransactionStatus::Complete => {}
            SendTransactionStatus::PreparationRejected => {}
            SendTransactionStatus::ZeroProgress => {}
            SendTransactionStatus::PartialProgress => {}
            SendTransactionStatus::IntegrityLost => {}
            SendTransactionStatus::ClockFailureBeforeSend => {}
            SendTransactionStatus::ClockFailureAfterSend => {}
        }
    }
}
