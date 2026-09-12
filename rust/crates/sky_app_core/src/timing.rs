//! Materialized playback timing policy shared by planning and execution.
//!
//! User-owned Timing Margin is applied to authored hold and same-key release
//! timing. The fixed Down late cutoff is a separate dispatch policy.

use crate::song::SongError;

pub const DEFAULT_DOWN_LATE_GRACE_US: u64 = 500;
pub const DEFAULT_FOCUS_RESTORE_GRACE_US: u64 = 100_000;

#[derive(Debug, Clone, PartialEq)]
pub struct MaterializedTimingPolicy {
    pub fps: u16,
    pub frame_us: u64,
    pub hold_frames: f64,
    pub frame_base_hold_us: u64,
    pub timing_margin_us: u64,
    pub down_late_grace_us: u64,
    pub min_hold_us: u64,
    pub min_release_gap_us: u64,
    pub focus_restore_grace_us: u64,
}

impl MaterializedTimingPolicy {
    pub fn from_user_margin(
        fps: u16,
        hold_frames: f64,
        timing_margin_us: u64,
    ) -> Result<Self, SongError> {
        Self::from_user_margin_with_down_late_grace(
            fps,
            hold_frames,
            timing_margin_us,
            DEFAULT_DOWN_LATE_GRACE_US,
        )
    }

    /// Test seam proving that the sender cutoff does not affect authored timing.
    pub fn from_user_margin_with_down_late_grace(
        fps: u16,
        hold_frames: f64,
        timing_margin_us: u64,
        down_late_grace_us: u64,
    ) -> Result<Self, SongError> {
        if !hold_frames.is_finite() || !crate::settings::HOLD_FRAME_OPTIONS.contains(&hold_frames) {
            return Err(SongError::InvalidHold);
        }
        if !(crate::settings::MIN_TIMING_MARGIN_US..=crate::settings::MAX_TIMING_MARGIN_US)
            .contains(&timing_margin_us)
            || !timing_margin_us.is_multiple_of(crate::settings::TIMING_MARGIN_STEP_US)
        {
            return Err(SongError::InvalidTimingMargin);
        }

        let frame_us = crate::song::frame_us(fps)?;
        let frame_base_hold_us = (hold_frames * frame_us as f64).ceil() as u64;
        let min_hold_us = frame_base_hold_us
            .checked_add(timing_margin_us)
            .ok_or(SongError::TimingOverflow)?;
        let min_release_gap_us = frame_us
            .checked_add(timing_margin_us)
            .ok_or(SongError::TimingOverflow)?;

        Ok(Self {
            fps,
            frame_us,
            hold_frames,
            frame_base_hold_us,
            timing_margin_us,
            down_late_grace_us,
            min_hold_us,
            min_release_gap_us,
            focus_restore_grace_us: DEFAULT_FOCUS_RESTORE_GRACE_US,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::MaterializedTimingPolicy;

    #[test]
    fn materializes_symmetric_user_margin_for_supported_hold_profiles() {
        let expected = [
            (1.0, 16_667, 17_467, 17_467),
            (1.25, 20_834, 21_634, 17_467),
            (1.5, 25_001, 25_801, 17_467),
        ];
        for (hold_frames, base_hold_us, min_hold_us, release_gap_us) in expected {
            let policy = MaterializedTimingPolicy::from_user_margin(60, hold_frames, 800)
                .expect("valid timing policy");
            assert_eq!(policy.frame_us, 16_667);
            assert_eq!(policy.frame_base_hold_us, base_hold_us);
            assert_eq!(policy.timing_margin_us, 800);
            assert_eq!(policy.min_hold_us, min_hold_us);
            assert_eq!(policy.min_release_gap_us, release_gap_us);
            assert_eq!(policy.down_late_grace_us, 500);
        }
    }

    #[test]
    fn timing_margin_zero_is_valid_and_one_hundred_microseconds_is_symmetric() {
        let zero = MaterializedTimingPolicy::from_user_margin(60, 1.0, 0).expect("zero is valid");
        let eight_hundred =
            MaterializedTimingPolicy::from_user_margin(60, 1.0, 800).expect("valid margin");
        let nine_hundred =
            MaterializedTimingPolicy::from_user_margin(60, 1.0, 900).expect("valid margin");
        assert_eq!(
            (zero.min_hold_us, zero.min_release_gap_us),
            (16_667, 16_667)
        );
        assert_eq!(eight_hundred.min_hold_us, 17_467);
        assert_eq!(eight_hundred.min_release_gap_us, 17_467);
        assert_eq!(nine_hundred.min_hold_us - eight_hundred.min_hold_us, 100);
        assert_eq!(
            nine_hundred.min_release_gap_us - eight_hundred.min_release_gap_us,
            100
        );
        assert_eq!(
            (nine_hundred.min_hold_us + nine_hundred.min_release_gap_us)
                - (eight_hundred.min_hold_us + eight_hundred.min_release_gap_us),
            200
        );
    }

    #[test]
    fn down_grace_changes_only_the_dispatch_cutoff() {
        let mut baseline = None;
        for grace_us in [500, 750, 1_000] {
            let policy = MaterializedTimingPolicy::from_user_margin_with_down_late_grace(
                60, 1.0, 800, grace_us,
            )
            .expect("valid A/B policy");
            assert_eq!(policy.down_late_grace_us, grace_us);
            assert_eq!(policy.min_hold_us, 17_467);
            assert_eq!(policy.min_release_gap_us, 17_467);
            let authored = (
                policy.frame_base_hold_us,
                policy.min_hold_us,
                policy.min_release_gap_us,
            );
            if let Some(expected) = baseline {
                assert_eq!(authored, expected);
            } else {
                baseline = Some(authored);
            }
        }
    }

    #[test]
    fn materializes_all_supported_fps_with_checked_symmetric_equations() {
        for fps in crate::settings::VALID_FPS {
            let policy = MaterializedTimingPolicy::from_user_margin(fps, 1.25, 1_200)
                .expect("valid FPS policy");
            assert_eq!(policy.min_hold_us, policy.frame_base_hold_us + 1_200);
            assert_eq!(policy.min_release_gap_us, policy.frame_us + 1_200);
        }
    }

    #[test]
    fn materialization_rejects_out_of_range_or_non_step_margin() {
        for margin in [1, 799, 801, 3_001] {
            assert!(MaterializedTimingPolicy::from_user_margin(60, 1.0, margin).is_err());
        }
        assert_eq!(
            MaterializedTimingPolicy::from_user_margin(60, 1.0, 3_000)
                .expect("maximum is valid")
                .timing_margin_us,
            3_000
        );
    }
}
