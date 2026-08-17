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
    assert!(outcome.stuck_keys().is_empty());
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
    assert!(outcome.verification_inconclusive);
    assert_eq!(outcome.stuck_keys(), vec![0x15]);
    assert_eq!(state.failed_release_mask, 0x0001);
}

#[test]
fn test_send_transaction_status_exhaustive_match() {
    let statuses = [
        SendTransactionStatus::Complete,
        SendTransactionStatus::ZeroProgress,
        SendTransactionStatus::PartialProgress,
        SendTransactionStatus::IntegrityLost,
        SendTransactionStatus::DeadlineMissedBeforeSend,
        SendTransactionStatus::ClockFailureBeforeSend,
        SendTransactionStatus::ClockFailureAfterSend,
    ];
    for status in statuses {
        match status {
            SendTransactionStatus::Complete => {}
            SendTransactionStatus::ZeroProgress => {}
            SendTransactionStatus::PartialProgress => {}
            SendTransactionStatus::IntegrityLost => {}
            SendTransactionStatus::DeadlineMissedBeforeSend => {}
            SendTransactionStatus::ClockFailureBeforeSend => {}
            SendTransactionStatus::ClockFailureAfterSend => {}
        }
    }
}
