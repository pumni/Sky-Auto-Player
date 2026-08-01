//! Single-interval pause model for playback timing.

use crate::time::{DurationTicks, QpcTicks, TimelineTicks};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct PlaybackClockState {
    pub start_perf: QpcTicks,
    pub pause_time: DurationTicks,
    pub pause_reasons: HashSet<String>,
    pub pause_interval_started: Option<QpcTicks>,
    pub pause_open_reason: Option<String>,
    pub epoch: QpcTicks,
}

impl PlaybackClockState {
    pub fn new(start_perf: QpcTicks, pause_time: DurationTicks) -> Self {
        let epoch = start_perf.saturating_add(pause_time);
        Self {
            start_perf,
            pause_time,
            pause_reasons: HashSet::new(),
            pause_interval_started: None,
            pause_open_reason: None,
            epoch,
        }
    }

    pub fn is_paused(&self) -> bool {
        !self.pause_reasons.is_empty()
    }

    pub fn has_pause_reason(&self, reason: &str) -> bool {
        self.pause_reasons.contains(reason)
    }

    pub fn enter_pause(&mut self, reason: &str, now: QpcTicks) -> bool {
        if self.pause_reasons.contains(reason) {
            return false;
        }
        let was_empty = self.pause_reasons.is_empty();
        self.pause_reasons.insert(reason.to_string());
        if was_empty {
            self.pause_interval_started = Some(now);
            self.pause_open_reason = Some(reason.to_string());
        }
        was_empty
    }

    pub fn exit_pause(&mut self, reason: &str, now: QpcTicks) -> Option<(DurationTicks, String)> {
        if !self.pause_reasons.contains(reason) {
            return None;
        }
        self.pause_reasons.remove(reason);
        if !self.pause_reasons.is_empty() {
            return None;
        }
        let started = self
            .pause_interval_started
            .expect("pause anchor must exist when exiting last reason");
        let duration = now.duration_since(started);
        let attribution = self
            .pause_open_reason
            .take()
            .unwrap_or_else(|| reason.to_string());
        self.pause_interval_started = None;
        self.update_pause_time(duration);
        Some((duration, attribution))
    }

    pub fn update_pause_time(&mut self, duration: DurationTicks) {
        self.pause_time = self.pause_time.saturating_add(duration);
        self.epoch = self.start_perf.saturating_add(self.pause_time);
    }

    pub fn rebase_epoch(&mut self, now: QpcTicks) -> DurationTicks {
        let old_start = self.start_perf;
        self.start_perf = now;
        self.epoch = self.start_perf.saturating_add(self.pause_time);
        now.duration_since(old_start)
    }

    pub fn get_elapsed(&self, now: QpcTicks) -> TimelineTicks {
        if let Some(started) = self.pause_interval_started {
            TimelineTicks(started.duration_since(self.epoch).0)
        } else {
            TimelineTicks(now.duration_since(self.epoch).0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pause_single_interval_overlap() {
        let mut clock = PlaybackClockState::new(QpcTicks(1000), DurationTicks(0));
        assert_eq!(clock.get_elapsed(QpcTicks(1100)), TimelineTicks(100));

        // Enter manual pause at 1100
        assert!(clock.enter_pause("manual", QpcTicks(1100)));
        assert!(clock.is_paused());

        // Focus pause enters at 1200 while manual is active -> does not open new interval
        assert!(!clock.enter_pause("focus", QpcTicks(1200)));

        // Manual exits at 1300 -> interval still open by focus
        assert_eq!(clock.exit_pause("manual", QpcTicks(1300)), None);
        assert!(clock.is_paused());

        // Focus exits at 1500 -> interval closes, total duration = 1500 - 1100 = 400 us, attributed to manual
        let (duration, open_reason) = clock.exit_pause("focus", QpcTicks(1500)).unwrap();
        assert_eq!(duration, DurationTicks(400));
        assert_eq!(open_reason, "manual");
        assert!(!clock.is_paused());

        // Elapsed at 1600 should be (1600 - (1000 + 400)) = 200 us
        assert_eq!(clock.get_elapsed(QpcTicks(1600)), TimelineTicks(200));
    }
}
