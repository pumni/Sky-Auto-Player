//! Allocation-free adaptive SendInput completion-cost estimator.
//!
//! The controller learns sender-side completion cost for each physical packet
//! path and event cardinality. It does not model game observation, wake
//! reserves, runtime Hot/Cold classes, or completion-error correction.

mod state;
mod window;

#[cfg(test)]
mod tests;

pub use state::{EstimatorStateJson, SampleWindowJson};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use window::{ROLLING_WINDOW_CAPACITY, RollingWindow};

pub const MAX_SAMPLE_US: u64 = 60_000_000;
pub const ESTIMATOR_STATE_VERSION: u32 = 12;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EstimatorConfigError {
    #[error("max_lead_us must be at most MAX_SAMPLE_US")]
    InvalidMaxLead,
    #[error("max_events must be in 1..=32")]
    InvalidMaxEvents,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EstimatorStateError {
    #[error("event_count {0} is outside the configured range")]
    InvalidEventCount(usize),
    #[error("estimator arithmetic overflow while updating {0}")]
    ArithmeticOverflow(&'static str),
    #[error("invalid estimator JSON: {0}")]
    InvalidJson(String),
    #[error("unsupported estimator version {0}; timing cache must be regenerated")]
    UnsupportedVersion(u32),
    #[error("state max_events {0} is outside 1..=32")]
    InvalidStateMaxEvents(usize),
    #[error("state max_events {state} does not match configured max_events {configured}")]
    MaxEventsMismatch { state: usize, configured: usize },
    #[error("{0} state vector length does not equal max_events + 1")]
    InvalidVectorLength(&'static str),
    #[error("persisted sample {0} exceeds MAX_SAMPLE_US")]
    PersistedSampleTooLarge(u64),
    #[error("persisted bucket contains {0} samples; maximum is 32")]
    TooManySamples(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendPath {
    DownOnly,
    UpOnly,
    Mixed,
}

impl SendPath {
    const fn index(self) -> usize {
        match self {
            Self::DownOnly => 0,
            Self::UpOnly => 1,
            Self::Mixed => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeadEstimate {
    pub applied_us: u64,
    pub uncapped_us: u64,
    pub saturated: bool,
}

#[derive(Debug, Clone)]
pub struct DispatchCostEstimator {
    max_lead_us: u64,
    max_events: usize,
    windows: [Vec<RollingWindow>; 3],
    normal_cache: [Vec<LeadEstimate>; 3],
    strict_cache: [Vec<LeadEstimate>; 3],
}

impl DispatchCostEstimator {
    pub fn try_new(max_lead_us: u64, max_events: usize) -> Result<Self, EstimatorConfigError> {
        if max_lead_us > MAX_SAMPLE_US {
            return Err(EstimatorConfigError::InvalidMaxLead);
        }
        if !(1..=ROLLING_WINDOW_CAPACITY).contains(&max_events) {
            return Err(EstimatorConfigError::InvalidMaxEvents);
        }
        let window = || vec![RollingWindow::default(); max_events + 1];
        let cache = || vec![LeadEstimate::default(); max_events + 1];
        let mut estimator = Self {
            max_lead_us,
            max_events,
            windows: [window(), window(), window()],
            normal_cache: [cache(), cache(), cache()],
            strict_cache: [cache(), cache(), cache()],
        };
        estimator.refresh_all();
        Ok(estimator)
    }

    #[cfg(test)]
    pub(super) fn new(max_lead_us: u64, max_events: usize) -> Self {
        Self::try_new(max_lead_us, max_events).expect("valid estimator configuration")
    }

    pub fn update(
        &mut self,
        path: SendPath,
        event_count: usize,
        dispatch_cost_us: u64,
    ) -> Result<(), EstimatorStateError> {
        if !(1..=self.max_events).contains(&event_count) {
            return Err(EstimatorStateError::InvalidEventCount(event_count));
        }
        self.windows[path.index()][event_count].push(dispatch_cost_us);
        self.refresh_path(path);
        Ok(())
    }

    pub fn estimate_lead(
        &self,
        path: SendPath,
        event_count: usize,
        strict_upper_tail: bool,
    ) -> LeadEstimate {
        let count = event_count.clamp(1, self.max_events);
        let cache = if strict_upper_tail {
            &self.strict_cache
        } else {
            &self.normal_cache
        };
        cache[path.index()][count]
    }

    pub fn export_state(&self) -> EstimatorStateJson {
        let export_path = |windows: &[RollingWindow]| {
            windows
                .iter()
                .map(|window| state::SampleWindowJson {
                    samples: window.ordered_samples(),
                })
                .collect()
        };
        EstimatorStateJson {
            version: ESTIMATOR_STATE_VERSION,
            max_events: self.max_events,
            down: export_path(&self.windows[SendPath::DownOnly.index()]),
            up: export_path(&self.windows[SendPath::UpOnly.index()]),
            mixed: export_path(&self.windows[SendPath::Mixed.index()]),
        }
    }

    pub fn import_state(&mut self, json_str: &str) -> Result<(), EstimatorStateError> {
        let state: EstimatorStateJson = serde_json::from_str(json_str)
            .map_err(|error| EstimatorStateError::InvalidJson(error.to_string()))?;
        if state.version != ESTIMATOR_STATE_VERSION {
            return Err(EstimatorStateError::UnsupportedVersion(state.version));
        }
        if !(1..=ROLLING_WINDOW_CAPACITY).contains(&state.max_events) {
            return Err(EstimatorStateError::InvalidStateMaxEvents(state.max_events));
        }
        if state.max_events != self.max_events {
            return Err(EstimatorStateError::MaxEventsMismatch {
                state: state.max_events,
                configured: self.max_events,
            });
        }
        let down = Self::import_path("down", &state.down, self.max_events)?;
        let up = Self::import_path("up", &state.up, self.max_events)?;
        let mixed = Self::import_path("mixed", &state.mixed, self.max_events)?;
        self.windows = [down, up, mixed];
        self.refresh_all();
        Ok(())
    }

    fn import_path(
        name: &'static str,
        buckets: &[state::SampleWindowJson],
        max_events: usize,
    ) -> Result<Vec<RollingWindow>, EstimatorStateError> {
        if buckets.len() != max_events + 1 {
            return Err(EstimatorStateError::InvalidVectorLength(name));
        }
        buckets
            .iter()
            .map(|bucket| RollingWindow::from_samples(&bucket.samples))
            .collect()
    }

    fn refresh_all(&mut self) {
        self.refresh_path(SendPath::DownOnly);
        self.refresh_path(SendPath::UpOnly);
        self.refresh_path(SendPath::Mixed);
    }

    fn refresh_path(&mut self, path: SendPath) {
        let mut normal_raw = [0u64; ROLLING_WINDOW_CAPACITY + 1];
        let mut strict_raw = [0u64; ROLLING_WINDOW_CAPACITY + 1];
        let windows = &self.windows[path.index()];
        for event_count in 1..=self.max_events {
            normal_raw[event_count] =
                Self::raw_estimate(windows, event_count, false, self.max_events);
            strict_raw[event_count] =
                Self::raw_estimate(windows, event_count, true, self.max_events);
        }
        let mut normal_prefix = 0;
        let mut strict_prefix = 0;
        for event_count in 1..=self.max_events {
            normal_prefix = normal_prefix.max(normal_raw[event_count]);
            strict_prefix = strict_prefix.max(strict_raw[event_count]);
            self.normal_cache[path.index()][event_count] = self.apply_limit(normal_prefix);
            self.strict_cache[path.index()][event_count] = self.apply_limit(strict_prefix);
        }
        self.normal_cache[path.index()][0] = LeadEstimate::default();
        self.strict_cache[path.index()][0] = LeadEstimate::default();
    }

    fn raw_estimate(
        windows: &[RollingWindow],
        event_count: usize,
        strict_upper_tail: bool,
        max_events: usize,
    ) -> u64 {
        if windows[event_count].is_seeded() {
            return if strict_upper_tail {
                windows[event_count].max().unwrap_or(0)
            } else {
                windows[event_count].p95().unwrap_or(0)
            };
        }
        for lower in (1..event_count).rev() {
            if windows[lower].is_seeded() {
                return if strict_upper_tail {
                    windows[lower].max().unwrap_or(0)
                } else {
                    windows[lower].p95().unwrap_or(0)
                };
            }
        }
        for window in windows.iter().take(max_events + 1).skip(event_count + 1) {
            if window.is_seeded() {
                return if strict_upper_tail {
                    window.max().unwrap_or(0)
                } else {
                    window.p95().unwrap_or(0)
                };
            }
        }
        0
    }

    fn apply_limit(&self, uncapped_us: u64) -> LeadEstimate {
        LeadEstimate {
            applied_us: uncapped_us.min(self.max_lead_us),
            uncapped_us,
            saturated: uncapped_us > self.max_lead_us,
        }
    }
}

impl Default for DispatchCostEstimator {
    fn default() -> Self {
        Self::try_new(2_000, 30).expect("default estimator configuration is valid")
    }
}
