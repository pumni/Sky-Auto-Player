//! Materialized playback timing policy shared by planning and execution.
//!
//! The policy is an application value, not a cache loader.  An outer adapter
//! resolves the calibration evidence and constructs this value once; callers
//! then pass the same value through analysis, fingerprinting and the native
//! session boundary.

use crate::song::SongError;

pub const DEFAULT_DOWN_LATE_GRACE_US: u64 = 500;
pub const DEFAULT_TRANSPORT_MARGIN_US: u64 = 300;
pub const DEFAULT_FOCUS_RESTORE_GRACE_US: u64 = 100_000;

#[derive(Debug, Clone, PartialEq)]
pub struct MaterializedTimingPolicy {
    pub fps: u16,
    pub frame_us: u64,
    pub hold_frames: f64,
    pub frame_base_hold_us: u64,
    pub down_late_grace_us: u64,
    pub transport_margin_us: u64,
    pub transport_margin_source: String,
    pub min_hold_us: u64,
    pub min_release_gap_us: u64,
    pub focus_restore_grace_us: u64,
}

impl MaterializedTimingPolicy {
    pub fn from_calibration(
        fps: u16,
        hold_frames: f64,
        transport_margin_us: u64,
        transport_margin_source: impl Into<String>,
    ) -> Result<Self, SongError> {
        let frame_us = crate::song::frame_us(fps)?;
        if !hold_frames.is_finite() || !crate::settings::HOLD_FRAME_OPTIONS.contains(&hold_frames) {
            return Err(SongError::InvalidHold);
        }
        let frame_base_hold_us = (hold_frames * frame_us as f64).ceil() as u64;
        Ok(Self {
            fps,
            frame_us,
            hold_frames,
            frame_base_hold_us,
            down_late_grace_us: DEFAULT_DOWN_LATE_GRACE_US,
            transport_margin_us,
            transport_margin_source: transport_margin_source.into(),
            min_hold_us: frame_base_hold_us
                .saturating_add(DEFAULT_DOWN_LATE_GRACE_US)
                .saturating_add(transport_margin_us),
            min_release_gap_us: frame_us
                .saturating_add(DEFAULT_DOWN_LATE_GRACE_US)
                .saturating_add(transport_margin_us),
            focus_restore_grace_us: DEFAULT_FOCUS_RESTORE_GRACE_US,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::MaterializedTimingPolicy;

    #[test]
    fn calibration_margin_materializes_once_for_planning_and_execution() {
        let policy = MaterializedTimingPolicy::from_calibration(60, 1.0, 777, "device_cache")
            .expect("valid policy");
        assert_eq!(policy.frame_us, 16_667);
        assert_eq!(policy.frame_base_hold_us, 16_667);
        assert_eq!(policy.down_late_grace_us, 500);
        assert_eq!(policy.transport_margin_us, 777);
        assert_eq!(policy.min_hold_us, 17_944);
        assert_eq!(policy.min_release_gap_us, 17_944);
        assert_eq!(policy.transport_margin_source, "device_cache");

        let cloned = policy.clone();
        assert_eq!(cloned, policy);
    }
}
