use super::*;

use super::physical::{
    InstrumentPhysicalState, ReconciledRelease, instrument_physical_state_for_mask,
    keyboard_context_for_target, map_instrument_virtual_keys, mask_for_scan_codes,
    reconcile_release_observation,
};
use super::scan_code::{
    FULL_INSTRUMENT_MASK, PHYSICAL_INSTRUMENT_SCAN_CODES, key_mask, scan_codes_from_mask,
    valid_instrument_scan_code,
};
use super::tracked::TEST_RELEASE_SLEEP_COUNT;
use crate::clock::QpcTicks;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

mod cleanup;

fn scripted_result(requested: usize, inserted: u8, _completed_us: u64) -> PlatformSendResult {
    PlatformSendResult {
        requested: requested as u8,
        inserted,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 0,
        timing_error: None,
    }
}

#[test]
fn down_retry_matrix_is_exact_and_clamped() {
    for (script, expected_sent, expected_dropped, expected_calls, expected_success) in [
        (vec![3], 3, 0, 1, true),
        // A partial first insertion is rolled back; the remainder is
        // never emitted as a second note-on chord.
        (vec![2, 1], 0, 3, 2, false),
        (vec![0, 0], 0, 3, 2, false),
        (vec![1, 1], 0, 3, 2, false),
        (vec![99], 3, 0, 1, true),
    ] {
        let mut returns = VecDeque::from(script);
        let mut calls = 0;
        let emitted = emit_down_with(&[0x15, 0x16, 0x17], |codes, _| {
            calls += 1;
            scripted_result(codes.len(), returns.pop_front().unwrap_or(0), calls)
        });
        assert_eq!(emitted.sent_scan_codes().len(), expected_sent);
        assert_eq!(calls, expected_calls);
        assert_eq!(emitted.is_success(), expected_success);
        let dropped = (emitted.evidence.requested_mask.count_ones()
            - emitted.evidence.confirmed_mask.count_ones()) as usize;
        assert_eq!(dropped, expected_dropped);
    }
}

#[test]
fn partial_note_on_marks_integrity_loss_and_rolls_back_uncertain_chord() {
    let mut calls = 0;
    let emitted = emit_down_with(&[0x15, 0x16, 0x17], |codes, key_up| {
        calls += 1;
        assert_eq!(key_up, calls == 2);
        scripted_result(codes.len(), 2, calls)
    });

    assert!(!emitted.is_success());
    assert!(matches!(
        emitted.status,
        SendTransactionStatus::IntegrityLost
    ));
    assert_eq!(emitted.evidence.first_inserted, 2);
    assert_eq!(emitted.sent_scan_codes().len(), 0);
    assert_eq!(emitted.evidence.attempts, 2);
}

#[test]
fn partial_note_on_rolls_back_the_uncertain_whole_chord() {
    let mut calls = Vec::new();
    let emitted = emit_down_with(&[0x15, 0x16, 0x17], |codes, key_up| {
        calls.push((codes.to_vec(), key_up));
        scripted_result(
            codes.len(),
            if key_up { codes.len() as u8 } else { 1 },
            calls.len() as u64,
        )
    });

    assert!(!emitted.is_success());
    assert!(matches!(
        emitted.status,
        SendTransactionStatus::IntegrityLost
    ));
    assert_eq!(
        calls,
        vec![
            (vec![0x15, 0x16, 0x17], false),
            (vec![0x15, 0x16, 0x17], true)
        ]
    );
}

#[test]
fn partial_note_on_after_zero_retry_rolls_back_the_uncertain_whole_chord() {
    let mut calls = Vec::new();
    let emitted = emit_down_with(&[0x15, 0x16, 0x17], |codes, key_up| {
        calls.push((codes.to_vec(), key_up));
        let inserted = match calls.len() {
            1 => 0,
            2 if !key_up => 1,
            _ if key_up => 2,
            _ => 0,
        };
        scripted_result(codes.len(), inserted, calls.len() as u64)
    });

    assert!(!emitted.is_success());
    assert!(matches!(
        emitted.status,
        SendTransactionStatus::IntegrityLost
    ));
    assert_eq!(
        calls,
        vec![
            (vec![0x15, 0x16, 0x17], false),
            (vec![0x15, 0x16, 0x17], false),
            (vec![0x15, 0x16, 0x17], true),
        ]
    );
    assert_eq!(emitted.evidence.first_inserted, 0);
    assert_eq!(emitted.evidence.attempts, 3);
}

#[test]
fn zero_progress_can_retry_whole_chord_without_splitting() {
    let mut calls = 0;
    let emitted = emit_down_with(&[0x15, 0x16, 0x17], |codes, key_up| {
        calls += 1;
        assert!(!key_up);
        scripted_result(codes.len(), if calls == 1 { 0 } else { 3 }, calls)
    });

    assert!(emitted.is_success());
    assert_eq!(emitted.sent_scan_codes().len(), 3);
    assert_eq!(emitted.evidence.attempts, 2);
    assert_eq!(emitted.evidence.zero_progress_retries, 1);
    assert!(!matches!(
        emitted.status,
        SendTransactionStatus::IntegrityLost
    ));
}

#[test]
fn complete_rollback_reports_no_residue_after_zero_retry() {
    let mut calls = 0;
    let emitted = emit_down_with(&[0x15, 0x16, 0x17], |codes, key_up| {
        calls += 1;
        let inserted = match calls {
            1 => 0,
            2 if !key_up => 1,
            3 if key_up => codes.len() as u8,
            _ => 0,
        };
        scripted_result(codes.len(), inserted, calls)
    });

    assert!(!emitted.is_success());
    assert_eq!(emitted.evidence.confirmed_mask, 0);
}

#[test]
fn up_retry_matrix_is_immediate_and_bounded() {
    for (script, expected_sent, expected_calls, expected_success) in [
        (vec![3], 3, 1, true),
        (vec![1, 3], 3, 2, true),
        (vec![0, 0], 0, 2, false),
        (vec![99], 3, 1, true),
    ] {
        let mut returns = VecDeque::from(script);
        let mut calls = 0;
        let emitted = emit_up_with_immediate(&[0x15, 0x16, 0x17], |codes, _| {
            calls += 1;
            scripted_result(codes.len(), returns.pop_front().unwrap_or(0), calls)
        });
        assert_eq!(emitted.sent_scan_codes().len(), expected_sent);
        assert_eq!(calls, expected_calls);
        assert_eq!(emitted.is_success(), expected_success);
    }
}

#[test]
fn partial_note_off_retries_the_entire_requested_set() {
    let mut calls = Vec::new();
    let emitted = emit_up_with_immediate(&[0x15, 0x16, 0x17], |codes, key_up| {
        assert!(key_up);
        calls.push(codes.to_vec());
        scripted_result(
            codes.len(),
            if calls.len() == 1 { 1 } else { 3 },
            calls.len() as u64,
        )
    });

    assert!(emitted.is_success());
    assert_eq!(calls, vec![vec![0x15, 0x16, 0x17], vec![0x15, 0x16, 0x17]]);
}

#[test]
fn structured_win32_errors_survive_down_retry() {
    let mut calls = 0;
    let emitted = emit_down_with(&[0x15, 0x16], |codes, _| {
        calls += 1;
        PlatformSendResult {
            requested: codes.len() as u8,
            inserted: 1,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            win32_error: if calls == 1 { 5 } else { 0 },
            timing_error: None,
        }
    });

    assert!(!emitted.is_success());
    assert!(matches!(
        emitted.status,
        SendTransactionStatus::IntegrityLost
    ));
    assert_eq!(emitted.evidence.first_win32_error, Some(5));
    assert_eq!(emitted.evidence.last_win32_error, Some(5));
    assert_eq!(emitted.evidence.attempts, 2);
    assert_eq!(emitted.evidence.zero_progress_retries, 0);
}

#[test]
fn structured_win32_errors_survive_up_zero_progress_retries() {
    let mut calls = 0;
    let emitted = emit_up_with_immediate(&[0x15], |codes, _| {
        calls += 1;
        PlatformSendResult {
            requested: codes.len() as u8,
            inserted: 0,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            win32_error: if calls == 1 { 5 } else { 1460 },
            timing_error: None,
        }
    });

    assert!(!emitted.is_success());
    assert_eq!(emitted.evidence.first_win32_error, Some(5));
    assert_eq!(emitted.evidence.last_win32_error, Some(1460));
    assert_eq!(emitted.evidence.attempts, 2);
    assert_eq!(emitted.evidence.zero_progress_retries, 1);
}

#[test]
fn zero_progress_note_on_counts_rejection_without_counting_a_split() {
    let mut state = TrackedKeyState::with_emitter(|codes, key_up| {
        assert!(!key_up);
        PlatformSendResult {
            requested: codes.len() as u8,
            inserted: 0,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            win32_error: 5,
            timing_error: None,
        }
    });

    let result = state.key_down(&[0x15, 0x16]);
    assert_eq!(result.status, SendTransactionStatus::ZeroProgress);
    assert_eq!(state.chords_rejected, 1);
    assert_eq!(state.authored_keys_rejected, 2);
    assert_eq!(state.sendinput_partial_events, 0);
    assert_eq!(state.sendinput_zero_progress_failures, 1);
    assert_eq!(state.chord_split_events, 0);
}

#[test]
fn tracked_note_on_uses_one_send_attempt_without_retry() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_emitter = Arc::clone(&calls);
    let mut state = TrackedKeyState::with_emitter(move |codes, key_up| {
        assert!(!key_up);
        calls_for_emitter.fetch_add(1, Ordering::SeqCst);
        test_send_result(codes.len() as u8, 0, 1460)
    });

    let outcome = state.key_down(&[0x15, 0x16]);

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(outcome.status, SendTransactionStatus::ZeroProgress);
    assert_eq!(outcome.evidence.attempts, 1);
    assert_eq!(outcome.evidence.zero_progress_retries, 0);
    assert_eq!(state.sendinput_zero_progress_failures, 1);
    assert_eq!(state.chord_split_events, 0);
}

#[test]
fn tracked_note_off_uses_one_send_attempt_without_retry() {
    let up_calls = Arc::new(AtomicUsize::new(0));
    let up_calls_for_emitter = Arc::clone(&up_calls);
    let mut state = TrackedKeyState::with_emitter(move |codes, key_up| {
        if key_up {
            up_calls_for_emitter.fetch_add(1, Ordering::SeqCst);
            test_send_result(codes.len() as u8, 0, 1460)
        } else {
            test_send_result(codes.len() as u8, codes.len() as u8, 0)
        }
    });
    assert_eq!(
        state.key_down(&[0x15, 0x16]).status,
        SendTransactionStatus::Complete
    );

    let outcome = state.key_up(&[0x15, 0x16]);

    assert_eq!(up_calls.load(Ordering::SeqCst), 1);
    assert_eq!(outcome.status, SendTransactionStatus::ZeroProgress);
    assert_eq!(outcome.evidence.attempts, 1);
    assert_eq!(outcome.evidence.zero_progress_retries, 0);
    assert_ne!(state.failed_release_mask & 0b11, 0);
}

#[test]
fn instrument_scan_codes_are_the_physical_allowlist() {
    assert_eq!(
        PHYSICAL_INSTRUMENT_SCAN_CODES,
        [
            0x15, 0x16, 0x17, 0x18, 0x19, 0x23, 0x24, 0x25, 0x26, 0x27, 0x31, 0x32, 0x33, 0x34,
            0x35,
        ]
    );
    assert_eq!(FULL_INSTRUMENT_MASK, 0x7fff);
    assert_eq!(
        mask_for_scan_codes(&PHYSICAL_INSTRUMENT_SCAN_CODES),
        Some(FULL_INSTRUMENT_MASK)
    );
    assert_eq!(
        PHYSICAL_INSTRUMENT_SCAN_CODES
            .iter()
            .filter_map(|&scan_code| key_mask(scan_code))
            .fold(0, |mask, bit| mask | bit),
        FULL_INSTRUMENT_MASK
    );
    assert!(
        PHYSICAL_INSTRUMENT_SCAN_CODES
            .iter()
            .all(|&scan_code| valid_instrument_scan_code(scan_code))
    );
    assert!(!valid_instrument_scan_code(0x14));
    assert!(!valid_instrument_scan_code(0x36));
    assert!(!valid_instrument_scan_code(0xffff));
}

#[test]
fn physical_verification_masks_are_bounded_and_subset_specific() {
    assert_eq!(mask_for_scan_codes(&[]), Some(0));
    assert_eq!(mask_for_scan_codes(&[0x15]), Some(1));
    assert_eq!(
        mask_for_scan_codes(&[0x15, 0x35]),
        Some((1 << 0) | (1 << 14))
    );
    assert_eq!(mask_for_scan_codes(&[0xffff]), None);
    assert_eq!(mask_for_scan_codes(&[0x15, 0xffff]), None);
    assert_eq!(
        instrument_physical_state_for_mask(0, 0),
        InstrumentPhysicalState::AllUp
    );
    assert_eq!(
        instrument_physical_state_for_mask(0, FULL_INSTRUMENT_MASK | (1 << 15)),
        InstrumentPhysicalState::Inconclusive
    );
}

#[cfg(windows)]
#[test]
fn current_layout_maps_all_instrument_scan_codes() {
    let context = keyboard_context_for_target(0).expect("current keyboard layout");
    let virtual_keys =
        map_instrument_virtual_keys(&context, FULL_INSTRUMENT_MASK).expect("instrument mappings");
    assert!(virtual_keys.iter().all(|&virtual_key| virtual_key != 0));
}

#[test]
fn zero_target_is_inconclusive_for_physical_preflight() {
    assert_eq!(
        TrackedKeyState::new().ensure_instrument_keys_physically_up(0),
        Err(PhysicalKeyPreflightError::VerificationInconclusive)
    );
}

#[test]
fn raw_sender_rejects_unknown_scan_code_before_backend_call() {
    let result = send_input_raw(&[0xffff], false);
    assert_eq!(result.requested, 1);
    assert_eq!(result.inserted, 0);
    assert_eq!(result.win32_error, 87);
    assert_eq!(result.timing_error, None);

    let oversized = send_input_raw(&[0x15; 16], false);
    assert_eq!(oversized.inserted, 0);
    assert_eq!(oversized.win32_error, 87);
}

#[test]
fn full_instrument_release_reports_unreleased_keys() {
    let mut state = TrackedKeyState::with_emitter(|codes, key_up| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: if key_up { 0 } else { codes.len() as u8 },
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 5,
        timing_error: None,
    });
    let outcome = state.release_all_full_instrument(0);
    assert!(!outcome.released_successfully);
    assert_eq!(outcome.attempted(), PHYSICAL_INSTRUMENT_SCAN_CODES);
    assert_eq!(outcome.stuck_keys(), PHYSICAL_INSTRUMENT_SCAN_CODES);
    assert_eq!(state.failed_release_mask.count_ones(), 15);
}

#[test]
fn partial_transport_plus_physical_all_up_is_verified_success() {
    let reconciled = reconcile_release_observation(0x0003, 0x0001, InstrumentPhysicalState::AllUp);
    assert_eq!(reconciled, ReconciledRelease::VerifiedAllUp);
}

#[test]
fn zero_progress_plus_physical_all_up_is_verified_success() {
    let reconciled = reconcile_release_observation(0x0003, 0x0000, InstrumentPhysicalState::AllUp);
    assert_eq!(reconciled, ReconciledRelease::VerifiedAllUp);
}

#[test]
fn physical_held_reports_only_held_subset() {
    let reconciled = reconcile_release_observation(
        0x0007,
        0x0001,
        InstrumentPhysicalState::Held(smallvec::smallvec![0x16]),
    );
    assert_eq!(reconciled, ReconciledRelease::Held(0x0002));
}

#[test]
fn physical_inconclusive_preserves_only_unconfirmed_subset() {
    let reconciled =
        reconcile_release_observation(0x0007, 0x0001, InstrumentPhysicalState::Inconclusive);
    assert_eq!(reconciled, ReconciledRelease::Inconclusive(0x0006));
}

#[test]
fn verified_all_up_clears_all_tracking_masks() {
    let mut state = TrackedKeyState::with_emitter(|codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: codes.len() as u8,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 0,
        timing_error: None,
    });
    state.custom_probe = Some(Box::new(|_, _| InstrumentPhysicalState::AllUp));
    state.active_mask = 0x0001;
    state.possibly_active_mask = 0x0002;
    state.failed_release_mask = 0x0004;

    let outcome = state.release_all(0);
    assert!(outcome.released_successfully);
    assert!(outcome.stuck_keys().is_empty());
    assert!(!outcome.verification_inconclusive);
    assert_eq!(state.active_mask, 0);
    assert_eq!(state.possibly_active_mask, 0);
    assert_eq!(state.failed_release_mask, 0);
    assert!(state.last_error.is_none());
}

#[test]
fn full_instrument_all_up_clears_transport_derived_stuck_keys() {
    let mut state = TrackedKeyState::with_emitter(|codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: codes.len() as u8,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 0,
        timing_error: None,
    });
    state.custom_probe = Some(Box::new(|_, _| InstrumentPhysicalState::AllUp));
    state.failed_release_mask = FULL_INSTRUMENT_MASK;

    let outcome = state.release_all_full_instrument(0);
    assert!(outcome.released_successfully);
    assert!(outcome.stuck_keys().is_empty());
    assert!(!outcome.verification_inconclusive);
    assert_eq!(state.failed_release_mask, 0);
}

#[test]
fn custom_emitter_still_uses_transport_evidence() {
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
fn tracked_cleanup_does_not_call_full_cleanup() {
    let emitted_batches = Arc::new(Mutex::new(Vec::new()));
    let batches_clone = emitted_batches.clone();
    let mut state = TrackedKeyState::with_emitter(move |codes, _| {
        batches_clone.lock().unwrap().push(codes.to_vec());
        PlatformSendResult {
            requested: codes.len() as u8,
            inserted: codes.len() as u8,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            win32_error: 0,
            timing_error: None,
        }
    });
    state.custom_probe = Some(Box::new(|_, _| InstrumentPhysicalState::AllUp));
    state.active_mask = 0x0001;

    let outcome = state.release_all(0);
    assert!(outcome.released_successfully);
    let batches = emitted_batches.lock().unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0], vec![0x15]);
}

#[test]
fn full_cleanup_does_not_run_tracked_cleanup_first() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let count_clone = call_count.clone();
    let mut state = TrackedKeyState::with_emitter(move |codes, _| {
        count_clone.fetch_add(1, Ordering::Relaxed);
        PlatformSendResult {
            requested: codes.len() as u8,
            inserted: codes.len() as u8,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            win32_error: 0,
            timing_error: None,
        }
    });
    state.custom_probe = Some(Box::new(|_, _| InstrumentPhysicalState::AllUp));
    state.active_mask = 0x0001;

    let outcome = state.release_all_full_instrument(0);
    assert!(outcome.released_successfully);
    assert_eq!(call_count.load(Ordering::Relaxed), 1);
}

#[test]
fn clean_first_attempt_has_no_sleep() {
    TEST_RELEASE_SLEEP_COUNT.store(0, Ordering::Relaxed);

    let mut state = TrackedKeyState::with_emitter(|codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: codes.len() as u8,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 0,
        timing_error: None,
    });
    state.custom_probe = Some(Box::new(|_, _| InstrumentPhysicalState::AllUp));
    state.active_mask = 0x0001;

    let outcome = state.release_all(0);
    assert!(outcome.released_successfully);
    assert_eq!(TEST_RELEASE_SLEEP_COUNT.load(Ordering::Relaxed), 0);
}

#[test]
fn retry_sends_only_unresolved_mask() {
    let emitted_batches = Arc::new(Mutex::new(Vec::new()));
    let batches_clone = emitted_batches.clone();
    let mut state = TrackedKeyState::with_emitter(move |codes, _| {
        let mut guard = batches_clone.lock().unwrap();
        guard.push(codes.to_vec());
        let inserted = if guard.len() == 1 {
            0
        } else {
            codes.len() as u8
        };
        PlatformSendResult {
            requested: codes.len() as u8,
            inserted,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            win32_error: 0,
            timing_error: None,
        }
    });
    // The probe mirrors transport progress: held until fully confirmed. First
    // attempt confirms nothing, so the FSM retries the full set.
    state.custom_probe = Some(Box::new(|mask, confirmed| {
        if confirmed == mask {
            InstrumentPhysicalState::AllUp
        } else {
            InstrumentPhysicalState::Held(scan_codes_from_mask(mask & !confirmed))
        }
    }));
    state.active_mask = 0x0003;

    let outcome = state.release_all(0);
    assert!(outcome.released_successfully);
    let batches = emitted_batches.lock().unwrap();
    assert!(batches.len() >= 2);
    assert_eq!(batches[0], vec![0x15, 0x16]);
}

#[test]
fn verified_keys_are_not_retried() {
    let emitted_batches = Arc::new(Mutex::new(Vec::new()));
    let batches_clone = emitted_batches.clone();
    let mut state = TrackedKeyState::with_emitter(move |codes, _| {
        let mut guard = batches_clone.lock().unwrap();
        guard.push(codes.to_vec());
        PlatformSendResult {
            requested: codes.len() as u8,
            inserted: codes.len() as u8,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            win32_error: 0,
            timing_error: None,
        }
    });
    state.custom_probe = Some(Box::new(|_, _| InstrumentPhysicalState::AllUp));
    state.active_mask = 0x0003;

    let outcome = state.release_all(0);
    assert!(outcome.released_successfully);
    let batches = emitted_batches.lock().unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0], vec![0x15, 0x16]);
}

#[test]
fn cleanup_send_count_is_bounded() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let count_clone = call_count.clone();
    let mut state = TrackedKeyState::with_emitter(move |codes, _| {
        count_clone.fetch_add(1, Ordering::Relaxed);
        PlatformSendResult {
            requested: codes.len() as u8,
            inserted: 0,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            win32_error: 5,
            timing_error: None,
        }
    });
    state.active_mask = 0x0001;

    let _outcome = state.release_all(0);
    assert_eq!(call_count.load(Ordering::Relaxed), 4);
}

#[test]
fn cleanup_probe_count_is_bounded() {
    let call_count = Arc::new(AtomicUsize::new(0));
    let count_clone = call_count.clone();
    let mut state = TrackedKeyState::with_emitter(move |codes, _| {
        count_clone.fetch_add(1, Ordering::Relaxed);
        PlatformSendResult {
            requested: codes.len() as u8,
            inserted: 0,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            win32_error: 5,
            timing_error: None,
        }
    });
    state.active_mask = 0x0001;

    let _outcome = state.release_all(0);
    assert!(call_count.load(Ordering::Relaxed) <= 4);
}

#[test]
fn cleanup_sleep_count_is_bounded() {
    TEST_RELEASE_SLEEP_COUNT.store(0, Ordering::Relaxed);

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
    assert_eq!(TEST_RELEASE_SLEEP_COUNT.load(Ordering::Relaxed), 3);
}

#[test]
fn final_inconclusive_result_fails_closed() {
    let mut state = TrackedKeyState::with_emitter(|codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: 0,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 5,
        timing_error: None,
    });
    state.active_mask = 0x0003;

    let outcome = state.release_all(0);
    assert!(!outcome.released_successfully);
    assert!(outcome.verification_inconclusive);
    assert_eq!(outcome.stuck_keys(), vec![0x15, 0x16]);
    assert_eq!(state.failed_release_mask, 0x0003);
}

#[test]
fn final_held_result_reports_exact_subset() {
    let reconciled = reconcile_release_observation(
        0x0007,
        0x0001,
        InstrumentPhysicalState::Held(smallvec::smallvec![0x16]),
    );
    assert_eq!(reconciled, ReconciledRelease::Held(0x0002));
}

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
    assert!(outcome.stuck_keys().is_empty());
    assert_eq!(outcome.attempts, 1);
    // Transport-confirmed keys are still cleared from tracking state; only the
    // physical-verification dimension is allowed to remain inconclusive.
    assert_eq!(state.active_mask, 0);
    assert_eq!(state.possibly_active_mask, 0);
    assert_eq!(state.failed_release_mask, 0);
    assert!(!outcome.transport_anomaly);
}

#[test]
fn cleanup_clears_resolved_keys_before_final_outcome() {
    let mut state = TrackedKeyState::with_emitter(|codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: 0,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 5,
        timing_error: None,
    });
    // Physical probe reports only 0x16 (B) held each attempt, so key 0x15 (A)
    // is resolved away on the first transition even though the release as a
    // whole cannot be proven complete.
    state.custom_probe = Some(Box::new(|_, mask| {
        let _ = mask;
        InstrumentPhysicalState::Held(smallvec::smallvec![0x16])
    }));
    state.active_mask = 0x0003;
    state.possibly_active_mask = 0x0003;

    let _outcome = state.release_all(0);
    // Key A (0x15) is resolved away on the first transition; key B (0x16)
    // remains physically held and is tracked as stuck fail-closed.
    assert_eq!(state.active_mask & 0x0001, 0);
    assert_eq!(state.possibly_active_mask & 0x0001, 0);
    assert_eq!(state.failed_release_mask, 0x0002);
}

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
    // Transport confirmed every key, but without a probe the physical verdict
    // is Inconclusive: no key is left stuck, yet all-up is not provable.
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
fn cleanup_transport_anomaly_never_forces_verification_inconclusive() {
    // Even when the transport path saw an anomaly, a conclusive Held verdict
    // from the probe must NOT be reported as inconclusive: the two dimensions
    // are independent and must never be OR-ed together.
    let mut state = TrackedKeyState::with_emitter(|codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: 0,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 5,
        timing_error: None,
    });
    state.custom_probe = Some(Box::new(|_, _| {
        InstrumentPhysicalState::Held(smallvec::smallvec![0x15])
    }));
    state.active_mask = 0x0001;

    let outcome = state.release_all(0);
    assert!(outcome.transport_anomaly);
    assert!(!outcome.released_successfully);
    assert!(
        !outcome.verification_inconclusive,
        "Held is conclusive, not inconclusive"
    );
    assert_eq!(outcome.stuck_keys(), vec![0x15]);
}

#[test]
fn cleanup_transport_anomaly_is_aggregated_independently_of_physical_all_up() {
    let mut state = TrackedKeyState::with_emitter(|codes, _| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: 0,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 5,
        timing_error: None,
    });
    // Physical probe sees all keys up despite the transport failure. The two
    // dimensions are independent: released_successfully=true and
    // verification_inconclusive=false, but transport_anomaly=true.
    state.custom_probe = Some(Box::new(|_, _| InstrumentPhysicalState::AllUp));
    state.active_mask = 0x0003;

    let outcome = state.release_all(0);
    assert!(outcome.released_successfully);
    assert!(!outcome.verification_inconclusive);
    assert!(outcome.stuck_keys().is_empty());
    assert_eq!(outcome.attempts, 1);
    assert!(outcome.transport_anomaly);
}

#[test]
fn test_send_transaction_status_exhaustive_match() {
    let statuses = [
        SendTransactionStatus::Complete,
        SendTransactionStatus::ZeroProgress,
        SendTransactionStatus::PartialProgress,
        SendTransactionStatus::IntegrityLost,
        SendTransactionStatus::ClockFailureBeforeSend,
        SendTransactionStatus::ClockFailureAfterSend,
    ];
    for status in statuses {
        match status {
            SendTransactionStatus::Complete => {}
            SendTransactionStatus::ZeroProgress => {}
            SendTransactionStatus::PartialProgress => {}
            SendTransactionStatus::IntegrityLost => {}
            SendTransactionStatus::ClockFailureBeforeSend => {}
            SendTransactionStatus::ClockFailureAfterSend => {}
        }
    }
}

fn test_send_result(requested: u8, inserted: u8, err: u32) -> PlatformSendResult {
    PlatformSendResult {
        requested,
        inserted,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: err,
        timing_error: None,
    }
}
