//! Bounded adaptive wake-guard controller according to Issue #646 §5.
//!
//! Adapts wake guard based on real kernel timer return behavior without changing
//! authored musical targets or adding a second scheduler.

use crate::clock::{DurationTicks, QpcClock, QpcTicks};

pub const MIN_GUARD_US: u64 = 250;
pub const MAX_GUARD_US: u64 = 1_500;
pub const SAFETY_PAD_US: u64 = 100;
pub const LEARNING_WINDOW_SAMPLES: u32 = 64;
pub const LEARNING_WINDOW_MIN_DURATION_US: u64 = 1_000_000;
pub const MAX_DOWNWARD_STEP_US: u64 = 25;
pub const DOWNWARD_HEADROOM_US: u64 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WaitGuardMode {
    #[default]
    Frozen,
    Adaptive,
}

impl WaitGuardMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "frozen" => Ok(Self::Frozen),
            "adaptive" => Ok(Self::Adaptive),
            _ => Err(format!(
                "unknown wait guard mode '{value}', expected 'frozen' or 'adaptive'"
            )),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Frozen => "frozen",
            Self::Adaptive => "adaptive",
        }
    }
}

/// A verified real kernel timer return sample for physical work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernelTimerWake {
    pub timer_return_qpc: QpcTicks,
    pub timer_arm_target_qpc: QpcTicks,
}

impl KernelTimerWake {
    #[inline]
    pub const fn new(timer_return_qpc: QpcTicks, timer_arm_target_qpc: QpcTicks) -> Self {
        Self {
            timer_return_qpc,
            timer_arm_target_qpc,
        }
    }
}

/// Small adaptive wake-guard controller.
///
/// Implements Issue #646 §5 fixed parameters:
/// - Guard 250–1.500 µs, safety pad 100 µs.
/// - Learning window 64 valid timer wakes and minimum 1 second.
/// - Immediate upward update when overshoot requires.
/// - Downward decrease at most 25 µs per valid window.
/// - Only learns from real kernel timer wakes for physical work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdaptiveWaitController {
    mode: WaitGuardMode,
    baseline_guard_us: u64,
    current_guard_us: u64,
    window_wake_count: u32,
    window_peak_overshoot_us: u64,
    window_start_qpc: QpcTicks,
    total_up_steps: u64,
    total_down_steps: u64,
    total_valid_wakes: u64,
}

impl AdaptiveWaitController {
    pub fn new(mode: WaitGuardMode, baseline_guard_us: u64, start_qpc: QpcTicks) -> Self {
        let clamped_baseline = baseline_guard_us.clamp(MIN_GUARD_US, MAX_GUARD_US);
        Self {
            mode,
            baseline_guard_us: clamped_baseline,
            current_guard_us: clamped_baseline,
            window_wake_count: 0,
            window_peak_overshoot_us: 0,
            window_start_qpc: start_qpc,
            total_up_steps: 0,
            total_down_steps: 0,
            total_valid_wakes: 0,
        }
    }

    #[inline]
    pub const fn mode(&self) -> WaitGuardMode {
        self.mode
    }

    #[inline]
    pub const fn current_guard_us(&self) -> u64 {
        self.current_guard_us
    }

    #[inline]
    pub const fn baseline_guard_us(&self) -> u64 {
        self.baseline_guard_us
    }

    #[inline]
    pub const fn total_up_steps(&self) -> u64 {
        self.total_up_steps
    }

    #[inline]
    pub const fn total_down_steps(&self) -> u64 {
        self.total_down_steps
    }

    #[inline]
    pub const fn total_valid_wakes(&self) -> u64 {
        self.total_valid_wakes
    }

    #[inline]
    pub const fn window_wake_count(&self) -> u32 {
        self.window_wake_count
    }

    #[inline]
    pub const fn window_peak_overshoot_us(&self) -> u64 {
        self.window_peak_overshoot_us
    }

    /// Reset learning window on resume without erasing the startup Frozen baseline.
    pub fn on_resume(&mut self, resume_qpc: QpcTicks) {
        self.window_wake_count = 0;
        self.window_peak_overshoot_us = 0;
        self.window_start_qpc = resume_qpc;
    }

    /// Record a valid kernel timer wake for physical work and evaluate controller adjustments.
    /// Returns true if `current_guard_us` changed.
    pub fn record_timer_wake(&mut self, wake: KernelTimerWake, clock: QpcClock) -> bool {
        self.total_valid_wakes = self.total_valid_wakes.saturating_add(1);

        let overshoot_us = if wake.timer_return_qpc > wake.timer_arm_target_qpc {
            let delta_ticks = wake
                .timer_return_qpc
                .checked_duration_since(wake.timer_arm_target_qpc)
                .unwrap_or(DurationTicks::ZERO);
            clock.duration_to_us(delta_ticks).unwrap_or(0)
        } else {
            0
        };

        if self.mode != WaitGuardMode::Adaptive {
            return false;
        }

        let mut guard_changed = false;

        // Immediate upward update if overshoot requires:
        // clamp(overshoot + 100us)
        let candidate_guard = overshoot_us
            .saturating_add(SAFETY_PAD_US)
            .clamp(MIN_GUARD_US, MAX_GUARD_US);
        if candidate_guard > self.current_guard_us {
            self.current_guard_us = candidate_guard;
            self.total_up_steps = self.total_up_steps.saturating_add(1);
            guard_changed = true;
        }

        self.window_peak_overshoot_us = self.window_peak_overshoot_us.max(overshoot_us);
        self.window_wake_count = self.window_wake_count.saturating_add(1);

        // Learning window: 64 valid timer wakes and minimum 1 second.
        let elapsed_window_us = if wake.timer_return_qpc >= self.window_start_qpc {
            let delta = wake
                .timer_return_qpc
                .checked_duration_since(self.window_start_qpc)
                .unwrap_or(DurationTicks::ZERO);
            clock.duration_to_us(delta).unwrap_or(0)
        } else {
            0
        };

        if self.window_wake_count >= LEARNING_WINDOW_SAMPLES
            && elapsed_window_us >= LEARNING_WINDOW_MIN_DURATION_US
        {
            // Downward adjustment: decrease at most 25 µs per valid window
            // only if window_peak + pad <= guard - 100 µs
            let peak_with_pad = self.window_peak_overshoot_us.saturating_add(SAFETY_PAD_US);
            if peak_with_pad.saturating_add(DOWNWARD_HEADROOM_US) <= self.current_guard_us {
                let headroom = self.current_guard_us.saturating_sub(peak_with_pad);
                let step = headroom.min(MAX_DOWNWARD_STEP_US);
                let new_guard = self.current_guard_us.saturating_sub(step).max(MIN_GUARD_US);
                if new_guard != self.current_guard_us {
                    self.current_guard_us = new_guard;
                    self.total_down_steps = self.total_down_steps.saturating_add(1);
                    guard_changed = true;
                }
            }

            self.window_wake_count = 0;
            self.window_peak_overshoot_us = 0;
            self.window_start_qpc = wake.timer_return_qpc;
        }

        guard_changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_mode_never_changes_guard() {
        let clock = QpcClock::initialize().expect("clock");
        let start_ticks = clock.now().expect("now");
        let mut controller = AdaptiveWaitController::new(WaitGuardMode::Frozen, 1_000, start_ticks);

        // Huge overshoot should not change Frozen guard
        let arm_target = start_ticks;
        let return_ticks = arm_target
            .checked_add_duration(clock.duration_from_us(5_000).expect("ticks"))
            .expect("add");
        let changed =
            controller.record_timer_wake(KernelTimerWake::new(return_ticks, arm_target), clock);

        assert!(!changed);
        assert_eq!(controller.current_guard_us(), 1_000);
        assert_eq!(controller.baseline_guard_us(), 1_000);
        assert_eq!(controller.total_up_steps(), 0);
    }

    #[test]
    fn adaptive_mode_increases_guard_immediately_on_overshoot() {
        let clock = QpcClock::initialize().expect("clock");
        let start_ticks = clock.now().expect("now");
        let mut controller = AdaptiveWaitController::new(WaitGuardMode::Adaptive, 500, start_ticks);

        // Overshoot of 600 µs: candidate = 600 + 100 = 700 µs > 500 µs
        let arm_target = start_ticks;
        let return_ticks = arm_target
            .checked_add_duration(clock.duration_from_us(600).expect("ticks"))
            .expect("add");
        let changed =
            controller.record_timer_wake(KernelTimerWake::new(return_ticks, arm_target), clock);

        assert!(changed);
        assert_eq!(controller.current_guard_us(), 700);
        assert_eq!(controller.total_up_steps(), 1);
    }

    #[test]
    fn adaptive_mode_clamps_at_max_guard() {
        let clock = QpcClock::initialize().expect("clock");
        let start_ticks = clock.now().expect("now");
        let mut controller = AdaptiveWaitController::new(WaitGuardMode::Adaptive, 500, start_ticks);

        // Overshoot of 2_000 µs: candidate = 2_000 + 100 = 2_100 -> clamped to 1_500 µs
        let arm_target = start_ticks;
        let return_ticks = arm_target
            .checked_add_duration(clock.duration_from_us(2_000).expect("ticks"))
            .expect("add");
        let changed =
            controller.record_timer_wake(KernelTimerWake::new(return_ticks, arm_target), clock);

        assert!(changed);
        assert_eq!(controller.current_guard_us(), 1_500);
        assert_eq!(controller.total_up_steps(), 1);
    }

    #[test]
    fn adaptive_mode_early_wake_has_zero_overshoot_no_increase() {
        let clock = QpcClock::initialize().expect("clock");
        let start_ticks = clock.now().expect("now");
        let mut controller = AdaptiveWaitController::new(WaitGuardMode::Adaptive, 500, start_ticks);

        // Timer woke early: return < target
        let arm_target = start_ticks
            .checked_add_duration(clock.duration_from_us(100).expect("ticks"))
            .expect("add");
        let return_ticks = start_ticks;
        let changed =
            controller.record_timer_wake(KernelTimerWake::new(return_ticks, arm_target), clock);

        assert!(!changed);
        assert_eq!(controller.current_guard_us(), 500);
        assert_eq!(controller.total_up_steps(), 0);
    }

    #[test]
    fn adaptive_mode_downward_step_requires_64_samples_and_1_second() {
        let clock = QpcClock::initialize().expect("clock");
        let start_ticks = clock.now().expect("now");
        let mut controller =
            AdaptiveWaitController::new(WaitGuardMode::Adaptive, 1_000, start_ticks);

        // 63 samples over 1.5 seconds: should NOT step down
        let mut current_time = start_ticks;
        let step_time = clock.duration_from_us(25_000).expect("ticks"); // 25ms each
        for _ in 0..63 {
            current_time = current_time.checked_add_duration(step_time).expect("add");
            // small overshoot of 10 µs: peak+pad = 110 µs <= 900 µs
            let arm_target = current_time;
            let return_ticks = arm_target
                .checked_add_duration(clock.duration_from_us(10).expect("ticks"))
                .expect("add");
            let changed =
                controller.record_timer_wake(KernelTimerWake::new(return_ticks, arm_target), clock);
            assert!(!changed);
        }
        assert_eq!(controller.current_guard_us(), 1_000);
        assert_eq!(controller.total_down_steps(), 0);

        // 64th sample: 64 samples AND > 1 second elapsed!
        current_time = current_time.checked_add_duration(step_time).expect("add");
        let arm_target = current_time;
        let return_ticks = arm_target
            .checked_add_duration(clock.duration_from_us(10).expect("ticks"))
            .expect("add");
        let changed =
            controller.record_timer_wake(KernelTimerWake::new(return_ticks, arm_target), clock);

        assert!(changed);
        // Decrease is max 25 µs: 1_000 - 25 = 975 µs!
        assert_eq!(controller.current_guard_us(), 975);
        assert_eq!(controller.total_down_steps(), 1);
        assert_eq!(controller.window_wake_count(), 0);
    }

    #[test]
    fn adaptive_mode_downward_step_blocked_when_headroom_is_less_than_100us() {
        let clock = QpcClock::initialize().expect("clock");
        let start_ticks = clock.now().expect("now");
        let mut controller = AdaptiveWaitController::new(WaitGuardMode::Adaptive, 400, start_ticks);

        // 64 samples over 1.5 seconds, but one sample has overshoot 250 µs
        // peak + pad = 250 + 100 = 350 µs.
        // 350 + 100 = 450 > current_guard (400 µs), so headroom < 100 µs!
        let mut current_time = start_ticks;
        let step_time = clock.duration_from_us(25_000).expect("ticks");
        for i in 0..64 {
            current_time = current_time.checked_add_duration(step_time).expect("add");
            let overshoot = if i == 10 { 250 } else { 10 };
            let arm_target = current_time;
            let return_ticks = arm_target
                .checked_add_duration(clock.duration_from_us(overshoot).expect("ticks"))
                .expect("add");
            controller.record_timer_wake(KernelTimerWake::new(return_ticks, arm_target), clock);
        }

        // Window completed and reset, but guard did NOT decrease because headroom was insufficient
        assert_eq!(controller.current_guard_us(), 400);
        assert_eq!(controller.total_down_steps(), 0);
        assert_eq!(controller.window_wake_count(), 0);
    }

    #[test]
    fn resume_resets_learning_window_without_erasing_baseline() {
        let clock = QpcClock::initialize().expect("clock");
        let start_ticks = clock.now().expect("now");
        let mut controller = AdaptiveWaitController::new(WaitGuardMode::Adaptive, 600, start_ticks);

        // Feed some samples
        let arm_target = start_ticks;
        let return_ticks = arm_target
            .checked_add_duration(clock.duration_from_us(50).expect("ticks"))
            .expect("add");
        controller.record_timer_wake(KernelTimerWake::new(return_ticks, arm_target), clock);
        assert_eq!(controller.window_wake_count(), 1);
        assert_eq!(controller.window_peak_overshoot_us(), 50);

        // Resume after pause or cleanup
        let resume_ticks = return_ticks
            .checked_add_duration(clock.duration_from_us(100_000).expect("ticks"))
            .expect("add");
        controller.on_resume(resume_ticks);

        assert_eq!(controller.window_wake_count(), 0);
        assert_eq!(controller.window_peak_overshoot_us(), 0);
        assert_eq!(controller.baseline_guard_us(), 600);
        assert_eq!(controller.current_guard_us(), 600);
    }
}
