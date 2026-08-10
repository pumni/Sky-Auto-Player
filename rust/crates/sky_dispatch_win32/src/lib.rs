//! Windows-specific SendInput, QPC clock, wait strategy, and real-time helpers.

#![deny(unsafe_op_in_unsafe_fn)]

pub mod calibration;
pub mod clock;
pub mod cpu;
pub mod event;
pub mod focus;
pub mod input;
pub mod mmcss;
pub mod power;
pub mod sleeper;
pub mod timer;
pub mod wait;

pub fn win32_available() -> bool {
    cfg!(windows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use input::{
        PacketRetryReason, PlatformSendResult, SendEvidence, SendTransactionOutcome,
        SendTransactionStatus,
    };

    #[test]
    fn test_win32_availability() {
        assert_eq!(win32_available(), cfg!(windows));
    }

    fn fake_success_emitter(scan_codes: &[u16], _key_up: bool) -> PlatformSendResult {
        PlatformSendResult {
            requested: scan_codes.len() as u8,
            inserted: scan_codes.len() as u8,
            started_ticks: clock::QpcTicks::ZERO,
            completed_ticks: Some(clock::QpcTicks::ZERO),
            win32_error: 0,
            timing_error: None,
        }
    }

    #[test]
    fn test_tracked_key_state_lifecycle() {
        let mut state = input::TrackedKeyState::with_emitter(fake_success_emitter);
        state.custom_probe = Some(Box::new(|_, _| input::InstrumentPhysicalState::AllUp));
        assert_eq!(state.active_mask, 0);

        let res_down = state.key_down(&[0x15, 0x16]);
        assert_eq!(res_down.status, SendTransactionStatus::Complete);
        assert_eq!(res_down.sent_scan_codes().as_slice(), &[0x15, 0x16]);
        assert_eq!(state.active_mask.count_ones(), 2);

        let res_up = state.key_up(&[0x15]);
        assert_eq!(res_up.status, SendTransactionStatus::Complete);
        assert_eq!(state.active_mask.count_ones(), 1);

        let outcome = state.release_all(0);
        assert!(outcome.released_successfully);
        assert_eq!(state.active_mask, 0);
    }

    #[test]
    fn mixed_physical_packet_partial_result_marks_entire_packet_uncertain() {
        let mut state =
            input::TrackedKeyState::with_packet_emitter(|packet| SendTransactionOutcome {
                status: SendTransactionStatus::IntegrityLost,
                evidence: SendEvidence {
                    requested_mask: packet.up_mask | packet.down_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: 1,
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: Some(5),
                    last_win32_error: Some(5),
                    started_ticks: Some(clock::QpcTicks::from_raw(10)),
                    completed_ticks: Some(clock::QpcTicks::from_raw(20)),
                    timing_error: None,
                },
            });
        let outcome = state.key_down_physical_packet(input::PhysicalPacket::new(0b01, 0b11));
        assert_eq!(outcome.status, SendTransactionStatus::IntegrityLost);
        assert_eq!(state.active_mask, 0);
        assert_eq!(state.possibly_active_mask, 0b11);
    }

    #[test]
    fn packet_emitter_preserves_up_and_down_masks() {
        let mut state = input::TrackedKeyState::with_packet_emitter(|packet| {
            assert_eq!(packet.up_mask, 0b001);
            assert_eq!(packet.down_mask, 0b010);
            assert_eq!(packet.event_count(), 2);
            SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask: 0b011,
                    confirmed_mask: 0b011,
                    skipped_mask: 0,
                    first_inserted: 2,
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(clock::QpcTicks::from_raw(10)),
                    completed_ticks: Some(clock::QpcTicks::from_raw(20)),
                    timing_error: None,
                },
            }
        });
        let outcome = state.key_down_physical_packet(input::PhysicalPacket::new(0b001, 0b010));
        assert_eq!(outcome.status, SendTransactionStatus::Complete);
        assert_eq!(state.active_mask, 0b010);
    }

    #[test]
    fn successful_up_preserves_unrelated_failed_release_error() {
        let mut state =
            input::TrackedKeyState::with_packet_emitter(|packet| SendTransactionOutcome {
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
                    started_ticks: Some(clock::QpcTicks::from_raw(10)),
                    completed_ticks: Some(clock::QpcTicks::from_raw(20)),
                    timing_error: None,
                },
            });
        state.failed_release_mask = 0b010;
        state.last_error = Some("unrelated failed release".to_string());

        let outcome = state.send_physical_packet(input::PhysicalPacket::new(0b001, 0));

        assert_eq!(outcome.status, SendTransactionStatus::Complete);
        assert_eq!(state.failed_release_mask, 0b010);
        assert_eq!(
            state.last_error.as_deref(),
            Some("unrelated failed release")
        );
    }

    #[test]
    fn zero_progress_packet_does_not_increment_partial_counter() {
        let mut state =
            input::TrackedKeyState::with_packet_emitter(|packet| SendTransactionOutcome {
                status: SendTransactionStatus::ZeroProgress,
                evidence: SendEvidence {
                    requested_mask: packet.up_mask | packet.down_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: 0,
                    attempts: 2,
                    zero_progress_retries: 1,
                    retry_reason: PacketRetryReason::ZeroProgress,
                    first_win32_error: Some(1460),
                    last_win32_error: Some(1460),
                    started_ticks: Some(clock::QpcTicks::from_raw(10)),
                    completed_ticks: Some(clock::QpcTicks::from_raw(20)),
                    timing_error: None,
                },
            });
        let outcome = state.key_down_physical_packet(input::PhysicalPacket::new(0, 0b001));
        assert_eq!(outcome.status, SendTransactionStatus::ZeroProgress);
        assert_eq!(state.sendinput_partial_events, 0);
        assert_eq!(state.sendinput_zero_progress_failures, 1);
        assert_eq!(state.chord_split_events, 0);
    }

    #[test]
    fn post_send_qpc_failure_does_not_commit_packet() {
        let mut state =
            input::TrackedKeyState::with_packet_emitter(|_packet| SendTransactionOutcome {
                status: SendTransactionStatus::ClockFailureAfterSend,
                evidence: SendEvidence {
                    requested_mask: 0b001,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: 1,
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(clock::QpcTicks::from_raw(10)),
                    completed_ticks: None,
                    timing_error: Some(clock::QpcError::CounterUnavailable),
                },
            });
        let outcome = state.key_down_physical_packet(input::PhysicalPacket::new(0, 0b001));
        assert_eq!(outcome.status, SendTransactionStatus::ClockFailureAfterSend);
        assert_eq!(state.active_mask, 0);
        assert_eq!(state.possibly_active_mask, 0b001);
        assert_eq!(state.sendinput_partial_events, 0);
    }

    #[test]
    fn test_hybrid_sleeper() {
        let now = clock::qpc_now_us().expect("test QPC clock");
        let target = now + 1_000; // 1 ms in future
        let overshoot = sleeper::sleep_until_us(target, 200).expect("QPC");
        let end_time = clock::qpc_now_us().expect("test QPC clock");
        assert!(end_time >= target);
        assert!((end_time - target).abs_diff(overshoot) <= 100);
    }

    #[test]
    fn test_mmcss_guard() {
        let guard = mmcss::MmcssGuard::join_games();
        if cfg!(windows) {
            drop(guard);
        }
    }

    #[test]
    fn test_measure_spin_overhead() {
        let overhead = sleeper::measure_spin_overhead_us().expect("QPC");
        assert!(overhead >= 1);
    }
}
