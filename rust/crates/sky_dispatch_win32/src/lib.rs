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
    use input::PlatformSendResult;

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
            completed_us: clock::qpc_now_us(),
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

        let outcome = state.release_all();
        assert!(outcome.released_successfully);
        assert_eq!(state.active_mask, 0);
    }

    #[test]
    fn test_hybrid_sleeper() {
        let now = clock::qpc_now_us();
        let target = now + 1_000; // 1 ms in future
        let overshoot = sleeper::sleep_until_us(target, 200).expect("QPC");
        let end_time = clock::qpc_now_us();
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
