use super::{EstimatorStateError, MAX_SAMPLE_US};

pub(super) const ROLLING_WINDOW_CAPACITY: usize = 32;
pub(super) const SEED_SAMPLES: usize = 5;
pub(super) const NORMAL_PERCENTILE: usize = 95;

#[derive(Debug, Clone, Copy)]
pub(super) struct RollingWindow {
    values: [u64; ROLLING_WINDOW_CAPACITY],
    len: usize,
    next: usize,
}

impl Default for RollingWindow {
    fn default() -> Self {
        Self {
            values: [0; ROLLING_WINDOW_CAPACITY],
            len: 0,
            next: 0,
        }
    }
}

impl RollingWindow {
    pub(super) fn push(&mut self, value_us: u64) {
        self.values[self.next] = value_us.min(MAX_SAMPLE_US);
        self.next = (self.next + 1) % ROLLING_WINDOW_CAPACITY;
        self.len = self.len.saturating_add(1).min(ROLLING_WINDOW_CAPACITY);
    }

    pub(super) fn is_seeded(&self) -> bool {
        self.len >= SEED_SAMPLES
    }

    pub(super) fn max(&self) -> Option<u64> {
        (self.len > 0).then(|| self.values[..self.len].iter().copied().max().unwrap_or(0))
    }

    pub(super) fn p95(&self) -> Option<u64> {
        if !self.is_seeded() {
            return None;
        }
        let mut sorted = [0u64; ROLLING_WINDOW_CAPACITY];
        sorted[..self.len].copy_from_slice(&self.values[..self.len]);
        sorted[..self.len].sort_unstable();
        let rank = (self.len * NORMAL_PERCENTILE).div_ceil(100);
        Some(sorted[rank.saturating_sub(1)])
    }

    pub(super) fn ordered_samples(&self) -> Vec<u64> {
        let mut samples = Vec::with_capacity(self.len);
        let oldest = if self.len == ROLLING_WINDOW_CAPACITY {
            self.next
        } else {
            0
        };
        for offset in 0..self.len {
            samples.push(self.values[(oldest + offset) % ROLLING_WINDOW_CAPACITY]);
        }
        samples
    }

    pub(super) fn from_samples(samples: &[u64]) -> Result<Self, EstimatorStateError> {
        if samples.len() > ROLLING_WINDOW_CAPACITY {
            return Err(EstimatorStateError::TooManySamples(samples.len()));
        }
        let mut window = Self::default();
        for &sample in samples {
            if sample > MAX_SAMPLE_US {
                return Err(EstimatorStateError::PersistedSampleTooLarge(sample));
            }
            window.push(sample);
        }
        Ok(window)
    }
}
