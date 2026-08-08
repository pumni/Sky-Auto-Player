use super::*;

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
fn up_send_failure_leaves_pending_release_unacknowledged() {
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
    assert_ne!(
        (state.active_mask | state.failed_release_mask) & (1 << 0),
        0
    );
}

#[test]
fn up_send_success_clears_pending_release() {
    let mut state = TrackedKeyState::with_emitter(|codes, _| {
        let len = codes.len() as u8;
        test_send_result(len, len, 0)
    });
    let down_outcome = state.key_down(&[0x15]);
    assert_eq!(down_outcome.status, SendTransactionStatus::Complete);
    let up_outcome = state.key_up(&[0x15]);
    assert_eq!(up_outcome.status, SendTransactionStatus::Complete);
    assert_eq!(state.active_mask, 0);
    assert_eq!(state.failed_release_mask, 0);
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
