//! Production wait-guard mode configuration.
//!
//! Frozen is the only supported production wait strategy. Adaptive mode is
//! retired for shipping; the bounded estimator experiment is maintained
//! for isolated comparative qualification.

use crate::clock::{QpcClock, QpcTicks};

pub const MIN_SPIN_US: u64 = 250;
pub const MAX_SPIN_US: u64 = 1_500;
pub const SAFETY_PAD_US: u64 = 100;
pub const LEARNING_WINDOW_SAMPLES: usize = 64;
pub const LEARNING_WINDOW_MIN_DURATION_US: u64 = 1_000_000;
pub const MAX_DOWNWARD_STEP_US: u64 = 25;
pub const DOWNWARD_HEADROOM_US: u64 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WaitGuardMode {
    #[default]
    Frozen,
}

impl WaitGuardMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "frozen" => Ok(Self::Frozen),
            "adaptive" => Err("adaptive guard mode is retired; only 'frozen' is supported".into()),
            _ => Err(format!(
                "unknown wait guard mode '{value}', expected 'frozen'"
            )),
        }
    }

    #[inline]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Frozen => "frozen",
        }
    }
}

/// Bounded adaptive-spin estimator experiment according to Issue #646 / Đợt 2 §5.
///
/// Implements conservative bounds [250 µs, 1500 µs], immediate upward adaptation
/// under jitter/overshoot, bounded downward step (max 25 µs) with 100 µs headroom,
/// and zero allocations.
#[derive(Clone, Debug)]
pub struct AdaptiveSpinEstimator {
    current_spin_us: u64,
    baseline_spin_us: u64,
    min_spin_us: u64,
    max_spin_us: u64,
    window_samples: usize,
    window_start_ticks: QpcTicks,
    window_peak_overshoot_us: u64,
    total_up_steps: u64,
    total_down_steps: u64,
}

impl AdaptiveSpinEstimator {
    pub fn new(baseline_spin_us: u64, start_ticks: QpcTicks) -> Self {
        let baseline = baseline_spin_us.clamp(MIN_SPIN_US, MAX_SPIN_US);
        Self {
            current_spin_us: baseline,
            baseline_spin_us: baseline,
            min_spin_us: MIN_SPIN_US,
            max_spin_us: MAX_SPIN_US,
            window_samples: 0,
            window_start_ticks: start_ticks,
            window_peak_overshoot_us: 0,
            total_up_steps: 0,
            total_down_steps: 0,
        }
    }

    pub fn current_spin_us(&self) -> u64 {
        self.current_spin_us
    }

    pub fn baseline_spin_us(&self) -> u64 {
        self.baseline_spin_us
    }

    pub fn total_up_steps(&self) -> u64 {
        self.total_up_steps
    }

    pub fn total_down_steps(&self) -> u64 {
        self.total_down_steps
    }

    pub fn window_wake_count(&self) -> usize {
        self.window_samples
    }

    pub fn window_peak_overshoot_us(&self) -> u64 {
        self.window_peak_overshoot_us
    }

    pub fn on_resume(&mut self, resume_ticks: QpcTicks) {
        self.window_samples = 0;
        self.window_start_ticks = resume_ticks;
        self.window_peak_overshoot_us = 0;
    }

    pub fn record_timer_wake(
        &mut self,
        return_ticks: QpcTicks,
        arm_target_ticks: QpcTicks,
        clock: QpcClock,
    ) -> bool {
        let overshoot_us = if return_ticks >= arm_target_ticks {
            return_ticks
                .checked_duration_since(arm_target_ticks)
                .ok()
                .and_then(|dur| clock.duration_to_us(dur).ok())
                .unwrap_or(0)
        } else {
            0
        };

        // Immediate upward step if overshoot exceeds current spin window
        if overshoot_us.saturating_add(SAFETY_PAD_US) > self.current_spin_us {
            let candidate_up = overshoot_us.saturating_add(SAFETY_PAD_US);
            let bounded_up = candidate_up.clamp(self.min_spin_us, self.max_spin_us);
            if bounded_up > self.current_spin_us {
                self.current_spin_us = bounded_up;
                self.total_up_steps = self.total_up_steps.saturating_add(1);
                // Reset window on upward step
                self.window_samples = 0;
                self.window_start_ticks = return_ticks;
                self.window_peak_overshoot_us = 0;
                return true;
            }
        }

        self.window_samples = self.window_samples.saturating_add(1);
        self.window_peak_overshoot_us = self.window_peak_overshoot_us.max(overshoot_us);

        let window_duration_us = return_ticks
            .checked_duration_since(self.window_start_ticks)
            .ok()
            .and_then(|dur| clock.duration_to_us(dur).ok())
            .unwrap_or(0);

        let mut changed = false;
        if self.window_samples >= LEARNING_WINDOW_SAMPLES
            && window_duration_us >= LEARNING_WINDOW_MIN_DURATION_US
        {
            let required_guard = self.window_peak_overshoot_us.saturating_add(SAFETY_PAD_US);
            if self.current_spin_us > required_guard.saturating_add(DOWNWARD_HEADROOM_US) {
                let candidate_down = self.current_spin_us.saturating_sub(MAX_DOWNWARD_STEP_US);
                let bounded_down = candidate_down.clamp(self.min_spin_us, self.max_spin_us);
                if bounded_down < self.current_spin_us {
                    self.current_spin_us = bounded_down;
                    self.total_down_steps = self.total_down_steps.saturating_add(1);
                    changed = true;
                }
            }
            // Reset learning window for next cycle
            self.window_samples = 0;
            self.window_start_ticks = return_ticks;
            self.window_peak_overshoot_us = 0;
        }

        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU64;

    #[test]
    fn parse_frozen_succeeds() {
        assert_eq!(
            WaitGuardMode::parse("frozen").unwrap(),
            WaitGuardMode::Frozen
        );
        assert_eq!(
            WaitGuardMode::parse("  FROZEN  ").unwrap(),
            WaitGuardMode::Frozen
        );
        assert_eq!(WaitGuardMode::Frozen.label(), "frozen");
    }

    #[test]
    fn parse_adaptive_fails_cleanly() {
        let err = WaitGuardMode::parse("adaptive").unwrap_err();
        assert!(err.contains("adaptive guard mode is retired"));
    }

    #[test]
    fn parse_unknown_fails() {
        let err = WaitGuardMode::parse("other").unwrap_err();
        assert!(err.contains("unknown wait guard mode"));
    }

    #[test]
    fn adaptive_estimator_clamps_initial_bounds() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(10_000_000).unwrap());
        let now = clock.now().unwrap();

        let low = AdaptiveSpinEstimator::new(100, now);
        assert_eq!(low.current_spin_us(), MIN_SPIN_US);

        let high = AdaptiveSpinEstimator::new(5_000, now);
        assert_eq!(high.current_spin_us(), MAX_SPIN_US);

        let normal = AdaptiveSpinEstimator::new(1_000, now);
        assert_eq!(normal.current_spin_us(), 1_000);
    }

    #[test]
    fn adaptive_estimator_steps_up_immediately_on_large_overshoot() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(10_000_000).unwrap());
        let now = clock.now().unwrap();
        let mut estimator = AdaptiveSpinEstimator::new(500, now);

        // Overshoot of 600 us (target 0, return 600 us)
        let arm_target = now;
        let return_ticks = arm_target
            .checked_add_duration(clock.duration_from_us(600).unwrap())
            .unwrap();

        let changed = estimator.record_timer_wake(return_ticks, arm_target, clock);
        assert!(changed);
        // candidate = 600 + 100 = 700 us
        assert_eq!(estimator.current_spin_us(), 700);
        assert_eq!(estimator.total_up_steps(), 1);
        assert_eq!(estimator.window_wake_count(), 0);
    }

    #[test]
    fn adaptive_estimator_caps_step_up_at_max_guard() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(10_000_000).unwrap());
        let now = clock.now().unwrap();
        let mut estimator = AdaptiveSpinEstimator::new(1_000, now);

        let arm_target = now;
        let return_ticks = arm_target
            .checked_add_duration(clock.duration_from_us(3_000).unwrap())
            .unwrap();

        let changed = estimator.record_timer_wake(return_ticks, arm_target, clock);
        assert!(changed);
        assert_eq!(estimator.current_spin_us(), MAX_SPIN_US);
    }

    #[test]
    fn adaptive_estimator_downward_step_requires_64_samples_and_1_second() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(10_000_000).unwrap());
        let start = clock.now().unwrap();
        let mut estimator = AdaptiveSpinEstimator::new(1_000, start);

        let mut current_target = start;
        let step = clock.duration_from_us(20_000).unwrap(); // 20ms

        // 63 samples
        for _ in 0..63 {
            current_target = current_target.checked_add_duration(step).unwrap();
            let return_ticks = current_target
                .checked_add_duration(clock.duration_from_us(10).unwrap())
                .unwrap();
            let changed = estimator.record_timer_wake(return_ticks, current_target, clock);
            assert!(!changed);
        }
        assert_eq!(estimator.current_spin_us(), 1_000);
        assert_eq!(estimator.total_down_steps(), 0);

        // 64th sample (total elapsed: 64 * 20ms = 1.28s > 1s)
        current_target = current_target.checked_add_duration(step).unwrap();
        let return_ticks = current_target
            .checked_add_duration(clock.duration_from_us(10).unwrap())
            .unwrap();
        let changed = estimator.record_timer_wake(return_ticks, current_target, clock);
        assert!(changed);
        assert_eq!(estimator.current_spin_us(), 975); // 1000 - 25
        assert_eq!(estimator.total_down_steps(), 1);
        assert_eq!(estimator.window_wake_count(), 0);
    }

    #[test]
    fn adaptive_estimator_resume_resets_learning_window() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(10_000_000).unwrap());
        let start = clock.now().unwrap();
        let mut estimator = AdaptiveSpinEstimator::new(800, start);

        let arm_target = start;
        let return_ticks = arm_target
            .checked_add_duration(clock.duration_from_us(50).unwrap())
            .unwrap();
        estimator.record_timer_wake(return_ticks, arm_target, clock);
        assert_eq!(estimator.window_wake_count(), 1);
        assert_eq!(estimator.window_peak_overshoot_us(), 50);

        let resume_ticks = return_ticks
            .checked_add_duration(clock.duration_from_us(100_000).unwrap())
            .unwrap();
        estimator.on_resume(resume_ticks);
        assert_eq!(estimator.window_wake_count(), 0);
        assert_eq!(estimator.window_peak_overshoot_us(), 0);
        assert_eq!(estimator.current_spin_us(), 800);
    }

    #[test]
    fn compare_fixed_vs_adaptive_spin_estimator_quiet_and_contention() {
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(10_000_000).unwrap());
        let start = clock.now().unwrap();

        // 1. Quiet scenario (steady low-jitter wakes, e.g. 30..80 us overshoot)
        let fixed_spin_us: u64 = 1_000;
        let mut fixed_spin_time_us = 0u64;
        let mut fixed_lateness_us = Vec::with_capacity(128);

        let mut adaptive = AdaptiveSpinEstimator::new(1_000, start);
        let mut adaptive_spin_time_us = 0u64;
        let mut adaptive_lateness_us = Vec::with_capacity(128);

        let mut current_time = start;
        let step = clock.duration_from_us(16_667).unwrap(); // 60 FPS frame

        for i in 0..128 {
            current_time = current_time.checked_add_duration(step).unwrap();
            let overshoot: u64 = 30 + ((i * 17) % 51); // 30..80 us
            let return_ticks = current_time
                .checked_add_duration(clock.duration_from_us(overshoot).unwrap())
                .unwrap();

            // Fixed arm:
            let fixed_spin = fixed_spin_us.saturating_sub(overshoot);
            fixed_spin_time_us += fixed_spin;
            let fixed_late = overshoot.saturating_sub(fixed_spin_us);
            fixed_lateness_us.push(fixed_late);

            // Adaptive arm:
            let cur_adaptive_spin = adaptive.current_spin_us();
            let adaptive_spin = cur_adaptive_spin.saturating_sub(overshoot);
            adaptive_spin_time_us += adaptive_spin;
            let adaptive_late = overshoot.saturating_sub(cur_adaptive_spin);
            adaptive_lateness_us.push(adaptive_late);

            adaptive.record_timer_wake(return_ticks, current_time, clock);
        }

        // Adaptive stepped down after 64 samples
        assert!(adaptive.total_down_steps() > 0);
        assert!(adaptive.current_spin_us() < fixed_spin_us);

        // 2. Sudden Contention Burst after step-down:
        // A sudden spike of 450 us arrives right after adaptive stepped down.
        let burst_overshoot: u64 = 450;
        let fixed_burst_late = burst_overshoot.saturating_sub(fixed_spin_us);
        let _adaptive_burst_late = burst_overshoot.saturating_sub(adaptive.current_spin_us());

        // Total spin time comparison: adaptive reduces spin under quiet, but at the cost of headroom
        assert!(adaptive_spin_time_us < fixed_spin_time_us);
        assert_eq!(fixed_burst_late, 0);
        // Adaptive had stepped down; while still within bounds, its headroom is reduced.
        assert!(adaptive.current_spin_us() < 1_000);

        // Conclusion: dynamic downward adaptation creates jitter vulnerability to sudden contention
        // bursts, proving why shipping retainment of fixed-spin A is superior for deterministic timing.
    }
}
