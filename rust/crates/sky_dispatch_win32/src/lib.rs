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
    use input::{PacketClockFailurePhase, PhysicalSendOutcome, PlatformSendResult};

    #[test]
    fn test_win32_availability() {
        assert_eq!(win32_available(), cfg!(windows));
    }

    fn fake_success_emitter(scan_codes: &[u16], _key_up: bool) -> PlatformSendResult {
        PlatformSendResult {
            requested: scan_codes.len() as u32,
            inserted: scan_codes.len() as u32,
            started_ticks: clock::QpcTicks::ZERO,
            completed_ticks: Some(clock::QpcTicks::ZERO),
            completed_us: clock::qpc_now_us().expect("test QPC clock"),
            win32_error: 0,
            timing_error: None,
        }
    }

    #[test]
    fn test_tracked_key_state_lifecycle() {
        let mut state = input::TrackedKeyState::with_emitter(fake_success_emitter);
        assert_eq!(state.active_mask, 0);

        let res_down = state.key_down(&[0x15, 0x16]);
        if let input::DownSendOutcome::Complete { sent, .. } = res_down {
            assert_eq!(sent.as_slice(), &[0x15, 0x16]);
        } else {
            panic!("Expected Complete");
        }
        assert_eq!(state.active_mask.count_ones(), 2);

        let res_up = state.key_up(&[0x15]);
        assert!(res_up.success);
        assert_eq!(state.active_mask.count_ones(), 1);

        let outcome = state.release_all(0);
        assert!(outcome.released_successfully);
        assert_eq!(state.active_mask, 0);
    }

    #[test]
    fn mixed_physical_packet_partial_result_marks_entire_packet_uncertain() {
        let mut state =
            input::TrackedKeyState::with_packet_emitter(|packet| PhysicalSendOutcome::Partial {
                requested: packet.event_count(),
                inserted_count: 1,
                attempts: 1,
                retry_reason: input::PacketRetryReason::None,
                first_error: 5,
                last_error: 5,
                started_ticks: clock::QpcTicks::from_raw(10),
                completed_ticks: clock::QpcTicks::from_raw(20),
            });
        let outcome = state.key_down_physical_packet(input::PhysicalPacket::new(0b01, 0b11));
        assert!(matches!(
            outcome,
            input::DownSendOutcome::IntegrityLost {
                send_attempts: 1,
                ..
            }
        ));
        assert_eq!(state.active_mask, 0);
        assert_eq!(state.possibly_active_mask, 0b11);
    }

    #[test]
    fn packet_emitter_preserves_up_and_down_masks() {
        let mut state = input::TrackedKeyState::with_packet_emitter(|packet| {
            assert_eq!(packet.up_mask, 0b001);
            assert_eq!(packet.down_mask, 0b010);
            assert_eq!(packet.event_count(), 2);
            PhysicalSendOutcome::Complete {
                requested: 2,
                inserted: 2,
                attempts: 1,
                retry_reason: input::PacketRetryReason::None,
                started_ticks: clock::QpcTicks::from_raw(10),
                completed_ticks: clock::QpcTicks::from_raw(20),
            }
        });
        let outcome = state.key_down_physical_packet(input::PhysicalPacket::new(0b001, 0b010));
        assert!(matches!(outcome, input::DownSendOutcome::Complete { .. }));
        assert_eq!(state.active_mask, 0b010);
    }

    #[test]
    fn same_key_retrigger_packet_contains_two_physical_events() {
        let mut state = input::TrackedKeyState::with_packet_emitter(|packet| {
            assert_eq!(packet.up_mask, 0b001);
            assert_eq!(packet.down_mask, 0b001);
            assert_eq!(packet.event_count(), 2);
            PhysicalSendOutcome::Complete {
                requested: 2,
                inserted: 2,
                attempts: 1,
                retry_reason: input::PacketRetryReason::None,
                started_ticks: clock::QpcTicks::from_raw(10),
                completed_ticks: clock::QpcTicks::from_raw(20),
            }
        });
        let outcome = state.key_down_physical_packet(input::PhysicalPacket::new(0b001, 0b001));
        assert!(matches!(outcome, input::DownSendOutcome::Complete { .. }));
    }

    #[test]
    fn zero_progress_packet_does_not_increment_partial_counter() {
        let mut state = input::TrackedKeyState::with_packet_emitter(|packet| {
            PhysicalSendOutcome::ZeroProgress {
                requested: packet.event_count(),
                attempts: 2,
                retry_reason: input::PacketRetryReason::ZeroProgress,
                first_error: 1460,
                last_error: 1460,
                started_ticks: clock::QpcTicks::from_raw(10),
                completed_ticks: clock::QpcTicks::from_raw(20),
            }
        });
        let outcome = state.key_down_physical_packet(input::PhysicalPacket::new(0, 0b001));
        assert!(matches!(
            outcome,
            input::DownSendOutcome::ZeroProgress { .. }
        ));
        assert_eq!(state.sendinput_partial_events, 0);
        assert_eq!(state.sendinput_zero_progress_failures, 1);
        assert_eq!(state.chord_split_events, 0);
    }

    #[test]
    fn post_send_qpc_failure_does_not_commit_packet() {
        let mut state = input::TrackedKeyState::with_packet_emitter(|_packet| {
            PhysicalSendOutcome::ClockFailure {
                phase: PacketClockFailurePhase::AfterSend,
                send_was_called: true,
                inserted_count: Some(1),
                started_ticks: Some(clock::QpcTicks::from_raw(10)),
                error: clock::QpcError::CounterUnavailable,
            }
        });
        let outcome = state.key_down_physical_packet(input::PhysicalPacket::new(0, 0b001));
        assert!(matches!(
            outcome,
            input::DownSendOutcome::IntegrityLost {
                timing_error: Some(clock::QpcError::CounterUnavailable),
                ..
            }
        ));
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
        let guard = mmcss::MmcssGuard::join_pro_audio();
        if cfg!(windows) {
            // Guard creates without panicking
            drop(guard);
        }
    }

    #[test]
    fn test_measure_spin_overhead() {
        let overhead = sleeper::measure_spin_overhead_us().expect("QPC");
        assert!(overhead >= 1);
    }
}
