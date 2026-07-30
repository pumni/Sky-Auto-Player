//! Single-interval pause model for playback timing.

use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct PlaybackClockState {
    pub start_perf: u64,
    pub pause_time_us: u64,
    pub pause_reasons: HashSet<String>,
    pub pause_interval_started_us: Option<u64>,
    pub pause_open_reason: Option<String>,
    pub epoch_us: u64,
}

impl PlaybackClockState {
    pub fn new(start_perf: u64, pause_time_us: u64) -> Self {
        let epoch_us = start_perf + pause_time_us;
        Self {
            start_perf,
            pause_time_us,
            pause_reasons: HashSet::new(),
            pause_interval_started_us: None,
            pause_open_reason: None,
            epoch_us,
        }
    }

    pub fn is_paused(&self) -> bool {
        !self.pause_reasons.is_empty()
    }

    pub fn has_pause_reason(&self, reason: &str) -> bool {
        self.pause_reasons.contains(reason)
    }

    pub fn enter_pause(&mut self, reason: &str, now_us: u64) -> bool {
        if self.pause_reasons.contains(reason) {
            return false;
        }
        let was_empty = self.pause_reasons.is_empty();
        self.pause_reasons.insert(reason.to_string());
        if was_empty {
            self.pause_interval_started_us = Some(now_us);
            self.pause_open_reason = Some(reason.to_string());
        }
        was_empty
    }

    pub fn exit_pause(&mut self, reason: &str, now_us: u64) -> Option<(u64, String)> {
        if !self.pause_reasons.contains(reason) {
            return None;
        }
        self.pause_reasons.remove(reason);
        if !self.pause_reasons.is_empty() {
            return None;
        }
        let started_us = self
            .pause_interval_started_us
            .expect("pause anchor must exist when exiting last reason");
        let duration_us = now_us.saturating_sub(started_us);
        let attribution = self
            .pause_open_reason
            .take()
            .unwrap_or_else(|| reason.to_string());
        self.pause_interval_started_us = None;
        self.update_pause_time(duration_us);
        Some((duration_us, attribution))
    }

    pub fn update_pause_time(&mut self, duration_us: u64) {
        self.pause_time_us += duration_us;
        self.epoch_us = self.start_perf + self.pause_time_us;
    }

    pub fn rebase_epoch(&mut self, now_us: u64) -> u64 {
        let old_start = self.start_perf;
        self.start_perf = now_us;
        self.epoch_us = self.start_perf + self.pause_time_us;
        now_us.saturating_sub(old_start)
    }

    pub fn get_elapsed_us(&self, now_us: u64) -> u64 {
        if let Some(started_us) = self.pause_interval_started_us {
            started_us.saturating_sub(self.epoch_us)
        } else {
            now_us.saturating_sub(self.epoch_us)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pause_single_interval_overlap() {
        let mut clock = PlaybackClockState::new(1000, 0);
        assert_eq!(clock.get_elapsed_us(1100), 100);

        // Enter manual pause at 1100
        assert!(clock.enter_pause("manual", 1100));
        assert!(clock.is_paused());

        // Focus pause enters at 1200 while manual is active -> does not open new interval
        assert!(!clock.enter_pause("focus", 1200));

        // Manual exits at 1300 -> interval still open by focus
        assert_eq!(clock.exit_pause("manual", 1300), None);
        assert!(clock.is_paused());

        // Focus exits at 1500 -> interval closes, total duration = 1500 - 1100 = 400 us, attributed to manual
        let (duration, open_reason) = clock.exit_pause("focus", 1500).unwrap();
        assert_eq!(duration, 400);
        assert_eq!(open_reason, "manual");
        assert!(!clock.is_paused());

        // Elapsed at 1600 should be (1600 - (1000 + 400)) = 200 us
        assert_eq!(clock.get_elapsed_us(1600), 200);
    }
}
