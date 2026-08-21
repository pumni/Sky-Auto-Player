//! Single-interval pause model for playback timing.

use crate::time::{DurationTicks, QpcTicks, TimeArithmeticError, TimelineTicks};

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    Manual = 1,
    Focus = 2,
}

impl PauseReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Focus => "focus",
        }
    }

    const fn bit(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PauseReasons(u8);

impl PauseReasons {
    const fn is_empty(self) -> bool {
        self.0 == 0
    }

    const fn contains(self, reason: PauseReason) -> bool {
        self.0 & reason.bit() != 0
    }

    fn insert(&mut self, reason: PauseReason) {
        self.0 |= reason.bit();
    }

    fn remove(&mut self, reason: PauseReason) {
        self.0 &= !reason.bit();
    }
}

#[derive(Debug, Clone)]
pub struct PlaybackClockState {
    pub start_perf: QpcTicks,
    pub pause_time: DurationTicks,
    pub pause_reasons: PauseReasons,
    pub pause_interval_started: Option<QpcTicks>,
    pub pause_open_reason: Option<PauseReason>,
    pub epoch: QpcTicks,
}

impl PlaybackClockState {
    pub fn new(
        start_perf: QpcTicks,
        pause_time: DurationTicks,
    ) -> Result<Self, TimeArithmeticError> {
        let epoch = start_perf.checked_add_duration(pause_time)?;
        Ok(Self {
            start_perf,
            pause_time,
            pause_reasons: PauseReasons::default(),
            pause_interval_started: None,
            pause_open_reason: None,
            epoch,
        })
    }

    pub fn is_paused(&self) -> bool {
        !self.pause_reasons.is_empty()
    }

    pub fn has_pause_reason(&self, reason: PauseReason) -> bool {
        self.pause_reasons.contains(reason)
    }

    pub fn enter_pause(
        &mut self,
        reason: PauseReason,
        now: QpcTicks,
    ) -> Result<bool, TimeArithmeticError> {
        if self.pause_reasons.contains(reason) {
            return Ok(false);
        }
        let was_empty = self.pause_reasons.is_empty();
        self.pause_reasons.insert(reason);
        if was_empty {
            self.pause_interval_started = Some(now);
            self.pause_open_reason = Some(reason);
        }
        Ok(was_empty)
    }

    pub fn exit_pause(
        &mut self,
        reason: PauseReason,
        now: QpcTicks,
    ) -> Result<Option<(DurationTicks, PauseReason)>, TimeArithmeticError> {
        if !self.pause_reasons.contains(reason) {
            return Ok(None);
        }
        self.pause_reasons.remove(reason);
        if !self.pause_reasons.is_empty() {
            return Ok(None);
        }
        let started = self
            .pause_interval_started
            .ok_or(TimeArithmeticError::NegativeOrder)?;
        let duration = now.checked_duration_since(started)?;
        let attribution = self.pause_open_reason.take().unwrap_or(reason);
        self.pause_interval_started = None;
        self.update_pause_time(duration)?;
        Ok(Some((duration, attribution)))
    }

    pub fn update_pause_time(
        &mut self,
        duration: DurationTicks,
    ) -> Result<(), TimeArithmeticError> {
        let pause_time = self.pause_time.checked_add(duration)?;
        let epoch = self.start_perf.checked_add_duration(pause_time)?;
        self.pause_time = pause_time;
        self.epoch = epoch;
        Ok(())
    }

    pub fn get_elapsed(&self, now: QpcTicks) -> Result<TimelineTicks, TimeArithmeticError> {
        if let Some(started) = self.pause_interval_started {
            Ok(TimelineTicks::from_raw(
                started.checked_duration_since(self.epoch)?.as_u64(),
            ))
        } else {
            Ok(TimelineTicks::from_raw(
                now.checked_duration_since(self.epoch)?.as_u64(),
            ))
        }
    }

    /// Return logical elapsed time for the one intentional startup interval
    /// where lead may place the first dispatch before the future epoch.
    /// Ordinary callers must use [`Self::get_elapsed`], which rejects
    /// timestamp underflow.
    pub fn get_elapsed_allow_pre_epoch(
        &self,
        now: QpcTicks,
        allow_pre_epoch: bool,
    ) -> Result<TimelineTicks, TimeArithmeticError> {
        if allow_pre_epoch && now < self.epoch {
            Ok(TimelineTicks::ZERO)
        } else {
            self.get_elapsed(now)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pause_single_interval_overlap() {
        let mut clock =
            PlaybackClockState::new(QpcTicks::from_raw(1000), DurationTicks::ZERO).unwrap();
        assert_eq!(
            clock.get_elapsed(QpcTicks::from_raw(1100)).unwrap(),
            TimelineTicks::from_raw(100)
        );

        // Enter manual pause at 1100
        assert!(
            clock
                .enter_pause(PauseReason::Manual, QpcTicks::from_raw(1100))
                .unwrap()
        );
        assert!(clock.is_paused());

        // Focus pause enters at 1200 while manual is active -> does not open new interval
        assert!(
            !clock
                .enter_pause(PauseReason::Focus, QpcTicks::from_raw(1200))
                .unwrap()
        );

        // Manual exits at 1300 -> interval still open by focus
        assert_eq!(
            clock
                .exit_pause(PauseReason::Manual, QpcTicks::from_raw(1300))
                .unwrap(),
            None
        );
        assert!(clock.is_paused());

        // Focus exits at 1500 -> interval closes, total duration = 1500 - 1100 = 400 us, attributed to manual
        let (duration, open_reason) = clock
            .exit_pause(PauseReason::Focus, QpcTicks::from_raw(1500))
            .unwrap()
            .unwrap();
        assert_eq!(duration, DurationTicks::from_raw(400));
        assert_eq!(open_reason, PauseReason::Manual);
        assert!(!clock.is_paused());

        // Elapsed at 1600 should be (1600 - (1000 + 400)) = 200 us
        assert_eq!(
            clock.get_elapsed(QpcTicks::from_raw(1600)).unwrap(),
            TimelineTicks::from_raw(200)
        );
    }

    #[test]
    fn pause_can_begin_before_a_future_physical_epoch() {
        let mut clock =
            PlaybackClockState::new(QpcTicks::from_raw(1_000), DurationTicks::ZERO).unwrap();
        assert!(
            clock
                .enter_pause(PauseReason::Focus, QpcTicks::from_raw(900))
                .unwrap()
        );
        assert_eq!(
            clock
                .exit_pause(PauseReason::Focus, QpcTicks::from_raw(1_100))
                .unwrap(),
            Some((DurationTicks::from_raw(200), PauseReason::Focus))
        );
        assert_eq!(clock.epoch, QpcTicks::from_raw(1_200));
    }
}
