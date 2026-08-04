//! Windows SendInput API wrappers, input packet prewarming, and tracked key backend.

mod outcome;
mod physical;
mod raw;
mod scan_code;
mod tracked;

pub use outcome::{
    DownSendOutcome, EmitResult, InputSendResult, PhysicalKeyPreflightError, PlatformSendResult,
    ReleaseAllOutcome,
};
pub use physical::is_scan_code_physically_down;
pub use raw::{emit_down, emit_down_with, send_input_raw, send_input_raw_with_clock};
pub use scan_code::{FULL_INSTRUMENT_MASK, PHYSICAL_INSTRUMENT_SCAN_CODES, SKY_PLAYER_SIGNATURE};
#[cfg(any(test, feature = "test-support"))]
pub use tracked::CustomEmitterFn;
pub use tracked::{TrackedKeyState, emit_up, emit_up_with};

pub(crate) use physical::*;
pub(crate) use raw::*;
pub(crate) use scan_code::*;
#[cfg(test)]
pub(crate) use tracked::emit_up_with_immediate;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::QpcTicks;
    use std::collections::VecDeque;

    fn scripted_result(requested: usize, inserted: u32, completed_us: u64) -> PlatformSendResult {
        PlatformSendResult {
            requested: requested as u32,
            inserted,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            completed_us,
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
            (vec![2, 1], 0, 1, 2, false),
            (vec![0, 0], 0, 3, 2, false),
            (vec![1, 1], 0, 2, 2, false),
            (vec![99], 3, 0, 1, true),
        ] {
            let mut returns = VecDeque::from(script);
            let mut calls = 0;
            let emitted = emit_down_with(&[2, 3, 4], |codes, _| {
                calls += 1;
                scripted_result(codes.len(), returns.pop_front().unwrap_or(0), calls)
            });
            assert_eq!(emitted.sent.len(), expected_sent);
            assert_eq!(calls, expected_calls);
            assert_eq!(emitted.success, expected_success);
            assert_eq!(emitted.keys_dropped, expected_dropped);
        }
    }

    #[test]
    fn partial_note_on_marks_integrity_loss_and_rolls_back_uncertain_chord() {
        let mut calls = 0;
        let emitted = emit_down_with(&[2, 3, 4], |codes, key_up| {
            calls += 1;
            assert_eq!(key_up, calls == 2);
            scripted_result(codes.len(), 2, calls)
        });

        assert!(!emitted.success);
        assert!(emitted.partial_progress);
        assert!(emitted.chord_integrity_lost);
        assert_eq!(emitted.first_inserted, 2);
        assert_eq!(emitted.sent.len(), 0);
        assert_eq!(emitted.send_attempts, 2);
    }

    #[test]
    fn partial_note_on_rolls_back_the_uncertain_whole_chord() {
        let mut calls = Vec::new();
        let emitted = emit_down_with(&[2, 3, 4], |codes, key_up| {
            calls.push((codes.to_vec(), key_up));
            scripted_result(
                codes.len(),
                if key_up { codes.len() as u32 } else { 1 },
                calls.len() as u64,
            )
        });

        assert!(!emitted.success);
        assert!(emitted.chord_integrity_lost);
        assert_eq!(calls, vec![(vec![2, 3, 4], false), (vec![2, 3, 4], true)]);
    }

    #[test]
    fn partial_note_on_after_zero_retry_rolls_back_the_uncertain_whole_chord() {
        let mut calls = Vec::new();
        let emitted = emit_down_with(&[2, 3, 4], |codes, key_up| {
            calls.push((codes.to_vec(), key_up));
            let inserted = match calls.len() {
                1 => 0,
                2 if !key_up => 1,
                _ if key_up => 2,
                _ => 0,
            };
            scripted_result(codes.len(), inserted, calls.len() as u64)
        });

        assert!(!emitted.success);
        assert!(emitted.chord_integrity_lost);
        assert_eq!(
            calls,
            vec![
                (vec![2, 3, 4], false),
                (vec![2, 3, 4], false),
                (vec![2, 3, 4], true),
            ]
        );
        assert_eq!(emitted.keys_inserted_before_failure, 1);
        assert_eq!(emitted.keys_rolled_back, 2);
        assert_eq!(emitted.rollback_residue_keys, 1);
    }

    #[test]
    fn zero_progress_can_retry_whole_chord_without_splitting() {
        let mut calls = 0;
        let emitted = emit_down_with(&[2, 3, 4], |codes, key_up| {
            calls += 1;
            assert!(!key_up);
            scripted_result(codes.len(), if calls == 1 { 0 } else { 3 }, calls)
        });

        assert!(emitted.success);
        assert_eq!(emitted.sent.len(), 3);
        assert_eq!(emitted.send_attempts, 2);
        assert_eq!(emitted.zero_progress_retries, 1);
        assert!(emitted.retried_after_zero_progress);
        assert!(!emitted.chord_integrity_lost);
    }

    #[test]
    fn complete_rollback_reports_no_residue_after_zero_retry() {
        let mut calls = 0;
        let emitted = emit_down_with(&[2, 3, 4], |codes, key_up| {
            calls += 1;
            let inserted = match calls {
                1 => 0,
                2 if !key_up => 1,
                3 if key_up => codes.len() as u32,
                _ => 0,
            };
            scripted_result(codes.len(), inserted, calls)
        });

        assert!(!emitted.success);
        assert_eq!(emitted.rollback_residue_keys, 0);
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
            let emitted = emit_up_with_immediate(&[2, 3, 4], |codes, _| {
                calls += 1;
                scripted_result(codes.len(), returns.pop_front().unwrap_or(0), calls)
            });
            assert_eq!(emitted.sent.len(), expected_sent);
            assert_eq!(calls, expected_calls);
            assert_eq!(emitted.success, expected_success);
        }
    }

    #[test]
    fn partial_note_off_retries_the_entire_requested_set() {
        let mut calls = Vec::new();
        let emitted = emit_up_with_immediate(&[2, 3, 4], |codes, key_up| {
            assert!(key_up);
            calls.push(codes.to_vec());
            scripted_result(
                codes.len(),
                if calls.len() == 1 { 1 } else { 3 },
                calls.len() as u64,
            )
        });

        assert!(emitted.success);
        assert_eq!(calls, vec![vec![2, 3, 4], vec![2, 3, 4]]);
    }

    #[test]
    fn structured_win32_errors_survive_down_retry() {
        let mut calls = 0;
        let emitted = emit_down_with(&[2, 3], |codes, _| {
            calls += 1;
            PlatformSendResult {
                requested: codes.len() as u32,
                inserted: 1,
                started_ticks: QpcTicks::ZERO,
                completed_ticks: Some(QpcTicks::ZERO),
                completed_us: calls,
                win32_error: if calls == 1 { 5 } else { 0 },
                timing_error: None,
            }
        });

        assert!(!emitted.success);
        assert!(emitted.chord_integrity_lost);
        assert_eq!(emitted.first_win32_error, Some(5));
        assert_eq!(emitted.last_win32_error, Some(5));
        assert_eq!(emitted.send_attempts, 2);
        assert_eq!(emitted.zero_progress_retries, 0);
    }

    #[test]
    fn structured_win32_errors_survive_up_zero_progress_retries() {
        let mut calls = 0;
        let emitted = emit_up_with_immediate(&[2], |codes, _| {
            calls += 1;
            PlatformSendResult {
                requested: codes.len() as u32,
                inserted: 0,
                started_ticks: QpcTicks::ZERO,
                completed_ticks: Some(QpcTicks::ZERO),
                completed_us: calls,
                win32_error: if calls == 1 { 5 } else { 1460 },
                timing_error: None,
            }
        });

        assert!(!emitted.success);
        assert_eq!(emitted.first_win32_error, Some(5));
        assert_eq!(emitted.last_win32_error, Some(1460));
        assert_eq!(emitted.send_attempts, 2);
        assert_eq!(emitted.zero_progress_retries, 1);
        assert!(!emitted.partial_progress);
    }

    #[test]
    fn zero_progress_note_on_counts_rejection_without_counting_a_split() {
        let mut state = TrackedKeyState::with_emitter(|codes, key_up| {
            assert!(!key_up);
            PlatformSendResult {
                requested: codes.len() as u32,
                inserted: 0,
                started_ticks: QpcTicks::ZERO,
                completed_ticks: Some(QpcTicks::ZERO),
                completed_us: 1,
                win32_error: 5,
                timing_error: None,
            }
        });

        let result = state.key_down(&[2, 3]);
        assert!(matches!(result, DownSendOutcome::ZeroProgress { .. }));
        assert_eq!(state.chords_rejected, 1);
        assert_eq!(state.authored_keys_rejected, 2);
        assert_eq!(state.sendinput_partial_events, 0);
        assert_eq!(state.sendinput_zero_progress_failures, 1);
        assert_eq!(state.chord_split_events, 0);
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
        let virtual_keys = map_instrument_virtual_keys(&context, FULL_INSTRUMENT_MASK)
            .expect("instrument mappings");
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
            requested: codes.len() as u32,
            inserted: if key_up { 0 } else { codes.len() as u32 },
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            completed_us: 10,
            win32_error: 5,
            timing_error: None,
        });
        let outcome = state.release_all_full_instrument(0);
        assert!(!outcome.released_successfully);
        assert_eq!(outcome.attempted, PHYSICAL_INSTRUMENT_SCAN_CODES);
        assert_eq!(outcome.stuck_keys, PHYSICAL_INSTRUMENT_SCAN_CODES);
        assert_eq!(state.failed_release_mask.count_ones(), 15);
    }
}
