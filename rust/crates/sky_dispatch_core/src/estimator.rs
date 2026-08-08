//! Conservative adaptive SendInput completion-latency estimator.
//!
//! ## P1.5 architecture
//!
//! Lead is broken into named components so each latency type is tracked
//! independently and telemetry can attribute where each microsecond of safety
//! margin comes from:
//!
//! ```text
//! lead = syscall_us + delivery_proxy_us + wake_reserve_us
//!        + cold_reserve_us + residual_bias_us
//! ```
//!
//! Each component is estimated by a dedicated sub-model:
//!
//! | Component | Model |
//! |---|---|
//! | `syscall_us` | Bounded histogram (fast model) + slow tail reserve |
//! | `delivery_proxy_us` | Static calibrated prior (updated by calibration harness) |
//! | `wake_reserve_us` | Constant scheduler guard |
//! | `cold_reserve_us` | Polyphony-linear cold prior |
//! | `residual_bias_us` | EMA of completion-error residual across six `(SendPath × LatencyClass)` channels |
//!
//! The histogram model is updated outside the precision decision and publishes
//! a fixed lookup table. Dispatch queries are O(1): no allocation, sort,
//! histogram scan, or clone occurs in the decision path.
//!
//! The slow tail reserve is a long-decay exponential envelope that prevents
//! the estimator from forgetting rare catastrophic tails too quickly.
//!
//! ## State version
//!
//! Version 9 publishes six timing channels: DownOnly Hot/Cold, UpOnly
//! Hot/Cold, and Mixed Hot/Cold. Timing cache state is ephemeral diagnostic
//! data, so only the current schema is accepted; older or newer versions are
//! discarded and the conservative prior is used.

use serde::{Deserialize, Serialize};
use thiserror::Error;

// ─── Constants ────────────────────────────────────────────────────────────────

/// Minimum samples before a bucket switches from prior to observed estimate.
const SEED_SAMPLES: usize = 5;
/// Histogram bucket width in microseconds (25 µs resolution).
const BUCKET_WIDTH_US: u64 = 25;
/// Number of histogram buckets (covers 0–6 375 µs; overflow lands in the last
/// bucket).
const BUCKET_COUNT: usize = 256;
/// Maximum value covered by the main histogram (non-overflow).
const HIST_MAX_US: u64 = BUCKET_WIDTH_US * (BUCKET_COUNT as u64 - 1);

/// Slow tail reserve decay coefficient for a 256-clean-sample half-life.
///
/// `0.5.powf(1.0 / 256.0)` is written as a literal so the hot update path
/// remains a single multiply without recomputing the coefficient.
const TAIL_RESERVE_DECAY: f64 = 0.99729605608547;
/// Hard lower bound on the slow tail reserve once it has been seeded with an
/// outlier.  Prevents the reserve from decaying to zero even with no new data.
const TAIL_RESERVE_FLOOR_US: u64 = 25;

/// Residual clamp: late by at most 1 000 µs, early correction at 0.25×.
const MAX_RESIDUAL_US: i64 = 1_000;
const EARLY_CORRECTION_DECAY: f64 = 0.25;

/// Samples that exceed this value are clamped before storage to prevent a
/// single catastrophic observation from ruining the model.
pub const MAX_SAMPLE_US: u64 = 60_000_000;

/// Conservative cold-start prior when no samples are available.
const BASE_COLD_PRIOR_US: u64 = 100;
/// Per-additional-key increment for the cold-start prior.
const PER_KEY_COLD_PRIOR_US: u64 = 40;

/// Scheduler wake-jitter reserve baked into every lead estimate.
/// This is separate from the syscall estimate and is not learned.
const WAKE_RESERVE_US: u64 = 50;

/// Current on-disk state format version.
pub const ESTIMATOR_STATE_VERSION: u32 = 9;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EstimatorConfigError {
    #[error("alpha must be finite and in (0, 1]")]
    InvalidAlpha,
    #[error("max_lead_us must be at most MAX_SAMPLE_US")]
    InvalidMaxLead,
    #[error("max_poly must be in 1..=32")]
    InvalidPolyphony,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EstimatorStateError {
    #[error("polyphony bucket {0} is outside the configured range")]
    InvalidPolyphony(usize),
    #[error("delivery proxy exceeds MAX_SAMPLE_US")]
    InvalidDeliveryProxy,

    #[error("estimator arithmetic overflow while updating {0}")]
    ArithmeticOverflow(&'static str),
}

// ─── Public types ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LatencyClass {
    Hot,
    Cold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendPath {
    DownOnly,
    UpOnly,
    Mixed,
}

pub fn channel_index(path: SendPath, class: LatencyClass) -> usize {
    match (path, class) {
        (SendPath::DownOnly, LatencyClass::Hot) => 0,
        (SendPath::DownOnly, LatencyClass::Cold) => 1,
        (SendPath::UpOnly, LatencyClass::Hot) => 2,
        (SendPath::UpOnly, LatencyClass::Cold) => 3,
        (SendPath::Mixed, LatencyClass::Hot) => 4,
        (SendPath::Mixed, LatencyClass::Cold) => 5,
    }
}

/// Named breakdown of the lead estimate so telemetry and tests can verify
/// each component independently.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct LeadComponents {
    /// SendInput syscall completion contribution (fast model + slow tail).
    pub syscall_us: u64,
    /// Delivery proxy contribution (calibrated prior or zero when uncalibrated).
    pub delivery_proxy_us: u64,
    /// Scheduler wake-jitter guard (constant).
    pub wake_reserve_us: u64,
    /// Cold-start polyphony reserve (only nonzero when latency class is Cold).
    pub cold_reserve_us: u64,
    /// Signed residual bias (completion error EMA).
    pub residual_bias_us: i64,
}

impl LeadComponents {
    /// Compute the uncapped lead from all components.
    #[allow(clippy::implicit_saturating_sub)]
    pub fn total_uncapped(&self) -> u64 {
        let positive = u128::from(self.syscall_us)
            + u128::from(self.delivery_proxy_us)
            + u128::from(self.wake_reserve_us)
            + u128::from(self.cold_reserve_us);
        let adjusted = if self.residual_bias_us < 0 {
            let magnitude = self.residual_bias_us.unsigned_abs() as u128;
            if magnitude > positive {
                0
            } else {
                positive - magnitude
            }
        } else {
            positive + self.residual_bias_us as u128
        };
        adjusted.min(u128::from(u64::MAX)) as u64
    }
}

/// Confidence level of the lead estimate for the requested polyphony bucket.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LeadConfidence {
    /// No samples observed; lead is the cold-start prior only.
    #[default]
    PriorOnly,
    /// Fewer than `SEED_SAMPLES` per bucket; still warming up.
    Warming,
    /// Histogram has enough samples; estimate is data-driven.
    Learned,
    /// Uncapped lead exceeds the configured maximum.
    Saturated,
}

/// Full lead estimate returned to callers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeadEstimate {
    pub applied_us: u64,
    pub uncapped_us: u64,
    pub saturated: bool,
    pub components: LeadComponents,
    pub confidence: LeadConfidence,
}

// ─── Histogram (fast model) ───────────────────────────────────────────────────

/// Bounded fixed-width histogram with O(1) update and O(BUCKET_COUNT) query.
///
/// Values beyond `HIST_MAX_US` accumulate in the final overflow bucket.
/// Overflow is tracked separately so `p_quantile` can detect when the tail
/// is dominated by overflow and conservatively use the slow tail reserve.
#[derive(Debug, Clone)]
struct Histogram {
    buckets: [u32; BUCKET_COUNT],
    total: u64,
    /// Maximum value seen (not clamped to HIST_MAX_US), for the slow-tail reserve.
    max_seen_us: u64,
    /// Samples in overflow bucket (index BUCKET_COUNT-1 or above).
    overflow_count: u32,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            buckets: [0u32; BUCKET_COUNT],
            total: 0,
            max_seen_us: 0,
            overflow_count: 0,
        }
    }
}

impl Histogram {
    fn checked_bucket_for(value_us: u64) -> usize {
        let clamped = value_us.min(MAX_SAMPLE_US);
        ((clamped / BUCKET_WIDTH_US) as usize).min(BUCKET_COUNT - 1)
    }

    fn ensure_push(&self, value_us: u64) -> Result<(), EstimatorStateError> {
        let bucket = Self::checked_bucket_for(value_us);
        self.buckets[bucket]
            .checked_add(1)
            .ok_or(EstimatorStateError::ArithmeticOverflow("histogram bucket"))?;
        self.total
            .checked_add(1)
            .ok_or(EstimatorStateError::ArithmeticOverflow("histogram total"))?;
        if bucket == BUCKET_COUNT - 1 {
            self.overflow_count
                .checked_add(1)
                .ok_or(EstimatorStateError::ArithmeticOverflow(
                    "histogram overflow count",
                ))?;
        }
        Ok(())
    }

    fn push(&mut self, value_us: u64) -> Result<(), EstimatorStateError> {
        let clamped = value_us.min(MAX_SAMPLE_US);
        let bucket = Self::checked_bucket_for(clamped);
        self.ensure_push(clamped)?;
        let next_bucket = self.buckets[bucket] + 1;
        let next_total = self.total + 1;
        let next_overflow = if bucket == BUCKET_COUNT - 1 {
            self.overflow_count + 1
        } else {
            self.overflow_count
        };
        self.buckets[bucket] = next_bucket;
        self.total = next_total;
        self.overflow_count = next_overflow;
        self.max_seen_us = self.max_seen_us.max(clamped);
        Ok(())
    }

    fn is_warm(&self) -> bool {
        self.total >= SEED_SAMPLES as u64
    }

    /// Estimated `p`-th quantile in microseconds (p in 0.0..=1.0).
    /// Returns `None` when the histogram has fewer than `SEED_SAMPLES`.
    fn p_quantile(&self, p: f64) -> Option<u64> {
        if !self.is_warm() {
            return None;
        }
        let target = ((self.total as f64) * p).ceil() as u64;
        let mut running = 0u64;
        for (i, &count) in self.buckets.iter().enumerate() {
            running += count as u64;
            if running >= target {
                // Return the upper bound of this bucket.
                return Some((i as u64 + 1) * BUCKET_WIDTH_US);
            }
        }
        // Entire mass is in overflow bucket (or rounding edge).
        Some(HIST_MAX_US + BUCKET_WIDTH_US)
    }

    fn p95(&self) -> Option<u64> {
        self.p_quantile(0.95)
    }

    fn max(&self) -> Option<u64> {
        (self.total > 0).then_some(self.max_seen_us)
    }

    /// Flatten the histogram into a compact `Vec<u64>` of (value, count) pairs
    /// for state export (non-zero buckets only).
    fn to_export_pairs(&self) -> Vec<[u64; 2]> {
        self.buckets
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0)
            .map(|(i, &c)| [i as u64, c as u64])
            .collect()
    }

    /// Reconstruct from exported (bucket_index, count) pairs.
    fn from_export_pairs(pairs: &[[u64; 2]]) -> Result<Self, String> {
        let mut hist = Self::default();
        for pair in pairs {
            let [idx, count] = *pair;
            if idx >= BUCKET_COUNT as u64 {
                return Err(format!("histogram bucket index {idx} out of range"));
            }
            if count == 0 {
                return Err("histogram bucket count must be non-zero".to_string());
            }
            if count > u32::MAX as u64 {
                return Err(format!("histogram bucket count {count} overflows u32"));
            }
            if hist.buckets[idx as usize] != 0 {
                return Err(format!("duplicate histogram bucket index {idx}"));
            }
            hist.buckets[idx as usize] = count as u32;
            hist.total = hist
                .total
                .checked_add(count)
                .ok_or_else(|| "histogram total overflows u64".to_string())?;
            let upper = (idx + 1) * BUCKET_WIDTH_US;
            if upper > hist.max_seen_us {
                hist.max_seen_us = upper;
            }
            if idx == BUCKET_COUNT as u64 - 1 {
                hist.overflow_count = hist
                    .overflow_count
                    .checked_add(count as u32)
                    .ok_or_else(|| "histogram overflow count overflows u32".to_string())?;
            }
        }
        Ok(hist)
    }

    fn merge_counts_from(&mut self, other: &Self) -> Result<(), String> {
        for (index, &count) in other.buckets.iter().enumerate() {
            self.buckets[index] = self.buckets[index]
                .checked_add(count)
                .ok_or_else(|| format!("histogram bucket {index} count overflows u32"))?;
        }
        self.total = self
            .total
            .checked_add(other.total)
            .ok_or_else(|| "histogram total overflows u64".to_string())?;
        self.overflow_count = self
            .overflow_count
            .checked_add(other.overflow_count)
            .ok_or_else(|| "histogram overflow count overflows u32".to_string())?;
        self.max_seen_us = self.max_seen_us.max(other.max_seen_us);
        Ok(())
    }
}

// ─── Slow tail reserve ────────────────────────────────────────────────────────

/// Long-decay exponential envelope that prevents forgetting rare outliers.
///
/// The reserve grows immediately when a new maximum exceeds the current value
/// and decays slowly thereafter.  This ensures that a single 3 ms outlier is
/// still visible after hundreds of subsequent 200 µs samples.
#[derive(Debug, Clone, Default)]
struct SlowTailReserve {
    value_us: u64,
}

impl SlowTailReserve {
    fn update(&mut self, observed_us: u64) {
        if observed_us >= self.value_us {
            self.value_us = observed_us;
        } else {
            // Exponential decay toward the observed value.
            let decayed = (self.value_us as f64 * TAIL_RESERVE_DECAY) as u64;
            self.value_us = decayed
                .max(observed_us)
                .max(TAIL_RESERVE_FLOOR_US.min(self.value_us));
        }
    }

    fn get(&self) -> u64 {
        self.value_us
    }
}

// ─── Residual EMA (6 independent channels) ───────────────────────────────────

/// EMA-based completion-error residual for one (kind × class) channel.
#[derive(Debug, Clone, Default)]
struct ResidualEma {
    count: u64,
    sum: i64,
    ema: f64,
    warm: bool,
}

impl ResidualEma {
    fn ensure_update(&self, sample: i64) -> Result<(), EstimatorStateError> {
        let clamped = sample.clamp(-MAX_RESIDUAL_US, MAX_RESIDUAL_US * 2);
        self.count
            .checked_add(1)
            .ok_or(EstimatorStateError::ArithmeticOverflow("residual count"))?;
        self.sum
            .checked_add(clamped)
            .ok_or(EstimatorStateError::ArithmeticOverflow("residual sum"))?;
        Ok(())
    }

    fn update(&mut self, alpha: f64, sample: i64) -> Result<(), EstimatorStateError> {
        let clamped = sample.clamp(-MAX_RESIDUAL_US, MAX_RESIDUAL_US * 2);
        let next_count = self
            .count
            .checked_add(1)
            .ok_or(EstimatorStateError::ArithmeticOverflow("residual count"))?;
        let next_sum = self
            .sum
            .checked_add(clamped)
            .ok_or(EstimatorStateError::ArithmeticOverflow("residual sum"))?;
        self.count = next_count;
        self.sum = next_sum;
        if self.warm {
            self.ema = alpha * clamped as f64 + (1.0 - alpha) * self.ema;
        } else if self.count >= SEED_SAMPLES as u64 {
            self.ema = self.sum as f64 / self.count as f64;
            self.warm = true;
        }
        Ok(())
    }

    fn adjustment_us(&self) -> i64 {
        if !self.warm {
            return 0;
        }
        let rounded = round_half_to_even(self.ema);
        if rounded >= 0 {
            rounded.min(MAX_RESIDUAL_US)
        } else {
            round_half_to_even(rounded as f64 * EARLY_CORRECTION_DECAY)
        }
    }
}

pub fn round_half_to_even(x: f64) -> i64 {
    let floor = x.floor();
    let diff = x - floor;
    if (diff - 0.5).abs() < 1e-9 {
        let i = floor as i64;
        if i % 2 == 0 { i } else { i + 1 }
    } else {
        x.round() as i64
    }
}

/// Collects all per-(polyphony, path, class) state for one path.
#[derive(Debug, Clone, Default)]
struct PathBuckets {
    hot: Vec<Histogram>,
    cold: Vec<Histogram>,
    /// Slow-tail envelopes remain class-specific. A cold outlier must not
    /// raise the strict hot estimate, including through the tail guard.
    tail_hot: Vec<SlowTailReserve>,
    tail_cold: Vec<SlowTailReserve>,
}

impl PathBuckets {
    fn new(size: usize) -> Self {
        Self {
            hot: vec![Histogram::default(); size],
            cold: vec![Histogram::default(); size],
            tail_hot: vec![SlowTailReserve::default(); size],
            tail_cold: vec![SlowTailReserve::default(); size],
        }
    }

    #[allow(dead_code)]
    fn resize(&mut self, new_size: usize) {
        self.hot.resize(new_size, Histogram::default());
        self.cold.resize(new_size, Histogram::default());
        self.tail_hot.resize(new_size, SlowTailReserve::default());
        self.tail_cold.resize(new_size, SlowTailReserve::default());
    }

    fn push(
        &mut self,
        n: usize,
        value_us: u64,
        class: LatencyClass,
    ) -> Result<(), EstimatorStateError> {
        match class {
            LatencyClass::Hot => {
                self.hot[n].push(value_us)?;
                self.tail_hot[n].update(value_us);
            }
            LatencyClass::Cold => {
                self.cold[n].push(value_us)?;
                self.tail_cold[n].update(value_us);
            }
        }
        Ok(())
    }

    fn ensure_push(
        &self,
        n: usize,
        value_us: u64,
        class: LatencyClass,
    ) -> Result<(), EstimatorStateError> {
        match class {
            LatencyClass::Hot => self.hot[n].ensure_push(value_us),
            LatencyClass::Cold => self.cold[n].ensure_push(value_us),
        }
    }

    /// Raw syscall estimate for bucket `n`.
    fn raw_estimate_us(
        &self,
        n: usize,
        class: LatencyClass,
        strict_upper_tail: bool,
    ) -> Option<u64> {
        let local = match class {
            LatencyClass::Hot => {
                if strict_upper_tail {
                    self.hot[n].max()
                } else {
                    self.hot[n].p95()
                }
            }
            LatencyClass::Cold => {
                if strict_upper_tail {
                    self.cold[n].max()
                } else {
                    self.cold[n].p95()
                }
            }
        };

        let tail_value = match class {
            LatencyClass::Hot => self.tail_hot[n].get(),
            LatencyClass::Cold => self.tail_cold[n].get(),
        };
        let tail = (tail_value > 0).then_some(tail_value);

        [local, tail].into_iter().flatten().max()
    }
}

#[derive(Debug, Clone, Default)]
struct PathTotals {
    hot: Histogram,
    cold: Histogram,
}

impl PathTotals {
    fn for_class(&self, class: LatencyClass) -> &Histogram {
        match class {
            LatencyClass::Hot => &self.hot,
            LatencyClass::Cold => &self.cold,
        }
    }

    fn for_class_mut(&mut self, class: LatencyClass) -> &mut Histogram {
        match class {
            LatencyClass::Hot => &mut self.hot,
            LatencyClass::Cold => &mut self.cold,
        }
    }

    fn ensure_push(&self, value_us: u64, class: LatencyClass) -> Result<(), EstimatorStateError> {
        self.for_class(class).ensure_push(value_us)
    }

    fn push(&mut self, value_us: u64, class: LatencyClass) -> Result<(), EstimatorStateError> {
        self.for_class_mut(class).push(value_us)
    }
}

// ─── On-disk format ──────────────────────────────────────────────────────────

/// Serialisable per-path histogram bucket with class-specific tails.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HistBucketJson {
    /// Non-zero (bucket_index, count) pairs for the hot histogram.
    #[serde(default)]
    pub hot_pairs: Vec<[u64; 2]>,
    /// Non-zero (bucket_index, count) pairs for the cold histogram.
    #[serde(default)]
    pub cold_pairs: Vec<[u64; 2]>,
    /// Hot-class slow tail reserve value.
    #[serde(default)]
    pub hot_tail_reserve_us: u64,
    /// Cold-class slow tail reserve value.
    #[serde(default)]
    pub cold_tail_reserve_us: u64,
}

/// Version-9 residual channel export.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ResidualChannelJson {
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub sum: i64,
    #[serde(default)]
    pub ema: f64,
    #[serde(default)]
    pub warm: bool,
}

/// Current on-disk format. Timing cache state is disposable diagnostic state;
/// older and newer schemas are rejected instead of being migrated in the
/// dispatch core.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstimatorStateJson {
    pub version: u32,
    pub max_poly: usize,

    /// Per-polyphony histogram state for Down direction.
    pub hist_down: Vec<HistBucketJson>,
    /// Per-polyphony histogram state for Up direction.
    pub hist_up: Vec<HistBucketJson>,
    /// Per-polyphony histogram state for Mixed direction.
    pub hist_mixed: Vec<HistBucketJson>,

    /// Six residual channels: down_hot, down_cold, up_hot, up_cold, mixed_hot, mixed_cold.
    pub residuals: Vec<ResidualChannelJson>,
    /// Delivery proxy channels ordered as Down/Hot, Down/Cold, Up/Hot, Up/Cold, Mixed/Hot, Mixed/Cold.
    pub delivery_proxy_channels: Vec<[u64; 6]>,
}

// ─── Main estimator ───────────────────────────────────────────────────────────

/// Conservative adaptive estimator with named lead components.
#[derive(Debug, Clone)]
pub struct SendLatencyEstimator {
    pub max_poly: usize,
    alpha: f64,
    max_lead_us: u64,

    down: PathBuckets,
    up: PathBuckets,
    mixed: PathBuckets,

    /// Global cross-bucket guards, kept independent for Hot and Cold.
    down_totals: PathTotals,
    up_totals: PathTotals,
    mixed_totals: PathTotals,

    /// Residual channels indexed by `channel_index(path, class)`.
    /// Order: [down_hot, down_cold, up_hot, up_cold, mixed_hot, mixed_cold].
    residuals: [ResidualEma; 6],

    /// Calibrated delivery proxy prior by path/class and polyphony.
    delivery_proxy_us: [Vec<u64>; 6],

    /// Cached normal/strict estimates indexed by polyphony, channel and
    /// policy. The cache is refreshed in place after a sample or calibration
    /// update; dispatch only indexes it.
    lead_cache: Vec<[[LeadEstimate; 2]; 6]>,
    #[cfg(test)]
    refresh_count: u64,
}

impl SendLatencyEstimator {
    pub fn try_new(
        alpha: f64,
        max_lead_us: u64,
        max_poly: usize,
    ) -> Result<Self, EstimatorConfigError> {
        if !alpha.is_finite() || !(0.0 < alpha && alpha <= 1.0) {
            return Err(EstimatorConfigError::InvalidAlpha);
        }
        if max_lead_us > MAX_SAMPLE_US {
            return Err(EstimatorConfigError::InvalidMaxLead);
        }
        if !(1..=32).contains(&max_poly) {
            return Err(EstimatorConfigError::InvalidPolyphony);
        }
        Ok(Self::new_unchecked(alpha, max_lead_us, max_poly))
    }

    fn new_unchecked(alpha: f64, max_lead_us: u64, max_poly: usize) -> Self {
        let size = max_poly + 1;
        let empty = LeadEstimate {
            applied_us: 0,
            uncapped_us: 0,
            saturated: false,
            components: LeadComponents::default(),
            confidence: LeadConfidence::PriorOnly,
        };
        let mut estimator = Self {
            max_poly,
            alpha,
            max_lead_us,
            down: PathBuckets::new(size),
            up: PathBuckets::new(size),
            mixed: PathBuckets::new(size),
            down_totals: PathTotals::default(),
            up_totals: PathTotals::default(),
            mixed_totals: PathTotals::default(),
            residuals: Default::default(),
            delivery_proxy_us: std::array::from_fn(|_| vec![0u64; size]),
            lead_cache: vec![[[empty; 2]; 6]; size],
            #[cfg(test)]
            refresh_count: 0,
        };
        estimator.refresh_lead_cache();
        estimator
    }

    fn refresh_lead_cache(&mut self) {
        for path in [SendPath::DownOnly, SendPath::UpOnly, SendPath::Mixed] {
            for class in [LatencyClass::Hot, LatencyClass::Cold] {
                self.refresh_channel(path, class);
            }
        }
    }

    fn refresh_channel(&mut self, path: SendPath, class: LatencyClass) {
        #[cfg(test)]
        {
            self.refresh_count = self.refresh_count.saturating_add(1);
        }
        let channel = channel_index(path, class);
        for strict_index in 0..2 {
            let strict = strict_index == 1;
            let mut best_components = LeadComponents::default();
            let mut best_uncapped = 0u64;
            let mut best_confidence = LeadConfidence::PriorOnly;
            for n in 1..=self.max_poly {
                let (components, confidence) = self.build_components(path, n, class, strict);
                let uncapped = components.total_uncapped();
                if uncapped >= best_uncapped {
                    best_uncapped = uncapped;
                    best_components = components;
                    best_confidence = confidence;
                }
                let saturated = best_uncapped > self.max_lead_us;
                self.lead_cache[n][channel][strict_index] = LeadEstimate {
                    applied_us: best_uncapped.min(self.max_lead_us),
                    uncapped_us: best_uncapped,
                    saturated,
                    components: best_components,
                    confidence: if saturated {
                        LeadConfidence::Saturated
                    } else {
                        best_confidence
                    },
                };
            }
        }
    }

    #[cfg(test)]
    fn new(alpha: f64, max_lead_us: u64, max_poly: usize) -> Self {
        Self::try_new(alpha, max_lead_us, max_poly)
            .expect("internal estimator defaults must be valid")
    }

    // ── Update API ────────────────────────────────────────────────────────────

    pub fn update(
        &mut self,
        path: SendPath,
        duration_us: u64,
        n_keys: usize,
    ) -> Result<(), EstimatorStateError> {
        self.update_with_class(path, duration_us, n_keys, LatencyClass::Hot)
    }

    pub fn update_with_class(
        &mut self,
        path: SendPath,
        duration_us: u64,
        n_keys: usize,
        latency_class: LatencyClass,
    ) -> Result<(), EstimatorStateError> {
        self.update_observation(path, latency_class, duration_us, n_keys, None)
    }

    pub fn update_observation(
        &mut self,
        path: SendPath,
        class: LatencyClass,
        duration_us: u64,
        polyphony: usize,
        completion_error_us: Option<i64>,
    ) -> Result<(), EstimatorStateError> {
        let duration_us = duration_us.min(MAX_SAMPLE_US);
        let n = 1.max(self.max_poly.min(polyphony));
        let channel = channel_index(path, class);
        match path {
            SendPath::DownOnly => {
                self.down.ensure_push(n, duration_us, class)?;
                self.down_totals.ensure_push(duration_us, class)?;
            }
            SendPath::UpOnly => {
                self.up.ensure_push(n, duration_us, class)?;
                self.up_totals.ensure_push(duration_us, class)?;
            }
            SendPath::Mixed => {
                self.mixed.ensure_push(n, duration_us, class)?;
                self.mixed_totals.ensure_push(duration_us, class)?;
            }
        }
        if let Some(error_us) = completion_error_us {
            self.residuals[channel].ensure_update(error_us)?;
        }
        match path {
            SendPath::DownOnly => {
                self.down.push(n, duration_us, class)?;
                self.down_totals.push(duration_us, class)?;
            }
            SendPath::UpOnly => {
                self.up.push(n, duration_us, class)?;
                self.up_totals.push(duration_us, class)?;
            }
            SendPath::Mixed => {
                self.mixed.push(n, duration_us, class)?;
                self.mixed_totals.push(duration_us, class)?;
            }
        }
        if let Some(error_us) = completion_error_us {
            let alpha = self.alpha;
            self.residuals[channel].update(alpha, error_us)?;
        }
        self.refresh_channel(path, class);
        Ok(())
    }

    pub fn update_completion_error(
        &mut self,
        path: SendPath,
        error_us: i64,
    ) -> Result<(), EstimatorStateError> {
        self.update_completion_error_with_class(path, error_us, LatencyClass::Hot)
    }

    pub fn update_completion_error_with_class(
        &mut self,
        path: SendPath,
        error_us: i64,
        class: LatencyClass,
    ) -> Result<(), EstimatorStateError> {
        let alpha = self.alpha;
        let channel = channel_index(path, class);
        self.residuals[channel].update(alpha, error_us)?;
        self.refresh_channel(path, class);
        Ok(())
    }

    pub fn set_delivery_proxy_us(
        &mut self,
        n_keys: usize,
        value_us: u64,
    ) -> Result<(), EstimatorStateError> {
        self.set_delivery_proxy_us_for(SendPath::DownOnly, LatencyClass::Hot, n_keys, value_us)
    }

    pub fn try_set_delivery_proxy_us_for(
        &mut self,
        path: SendPath,
        class: LatencyClass,
        n_keys: usize,
        value_us: u64,
    ) -> Result<(), EstimatorStateError> {
        if value_us > MAX_SAMPLE_US {
            return Err(EstimatorStateError::InvalidDeliveryProxy);
        }
        if n_keys == 0 || n_keys > self.max_poly {
            return Err(EstimatorStateError::InvalidPolyphony(n_keys));
        }
        let n = 1.max(self.max_poly.min(n_keys));
        let channel = channel_index(path, class);
        self.delivery_proxy_us[channel][n] = value_us;
        self.refresh_channel(path, class);
        Ok(())
    }

    pub fn set_delivery_proxy_us_for(
        &mut self,
        path: SendPath,
        class: LatencyClass,
        n_keys: usize,
        value_us: u64,
    ) -> Result<(), EstimatorStateError> {
        self.try_set_delivery_proxy_us_for(path, class, n_keys, value_us)
    }

    // ── Query API ─────────────────────────────────────────────────────────────

    pub fn residual_bias_us(&self) -> u64 {
        self.residual_adjustment_us().max(0) as u64
    }

    pub fn residual_adjustment_us(&self) -> i64 {
        self.residual_adjustment_us_for(SendPath::DownOnly)
    }

    pub fn residual_adjustment_us_for(&self, path: SendPath) -> i64 {
        self.residuals[channel_index(path, LatencyClass::Hot)].adjustment_us()
    }

    fn cold_prior_us(n: usize) -> u64 {
        BASE_COLD_PRIOR_US
            .saturating_add(PER_KEY_COLD_PRIOR_US.saturating_mul(n.saturating_sub(1) as u64))
    }

    fn syscall_estimate_us(
        &self,
        buckets: &PathBuckets,
        total: &Histogram,
        n: usize,
        class: LatencyClass,
        strict_upper_tail: bool,
    ) -> u64 {
        let local = buckets.raw_estimate_us(n, class, strict_upper_tail);

        let lower_bucket = (1..n).rev().find_map(|bucket| {
            buckets
                .raw_estimate_us(bucket, class, strict_upper_tail)
                .map(|est| {
                    est.saturating_add(
                        PER_KEY_COLD_PRIOR_US.saturating_mul(n.saturating_sub(bucket) as u64),
                    )
                })
        });

        let global = if strict_upper_tail {
            total.max()
        } else {
            total.p95()
        };
        let global_guard = if strict_upper_tail {
            local.into_iter().chain(global).max()
        } else {
            local.or(global)
        };

        global_guard
            .into_iter()
            .chain(lower_bucket)
            .max()
            .unwrap_or(0)
    }

    fn build_components(
        &self,
        path: SendPath,
        n: usize,
        class: LatencyClass,
        strict_upper_tail: bool,
    ) -> (LeadComponents, LeadConfidence) {
        let (buckets, total) = match path {
            SendPath::DownOnly => (&self.down, self.down_totals.for_class(class)),
            SendPath::UpOnly => (&self.up, self.up_totals.for_class(class)),
            SendPath::Mixed => (&self.mixed, self.mixed_totals.for_class(class)),
        };

        let syscall_us = self.syscall_estimate_us(buckets, total, n, class, strict_upper_tail);
        let channel = channel_index(path, class);
        let delivery_proxy_us = self.delivery_proxy_us[channel][n];
        let wake_reserve_us = WAKE_RESERVE_US;

        let cold_reserve_us =
            if class == LatencyClass::Cold && buckets.cold[n].total < SEED_SAMPLES as u64 {
                Self::cold_prior_us(n)
            } else {
                0
            };

        let residual_bias_us = self.residuals[channel].adjustment_us();

        let components = LeadComponents {
            syscall_us,
            delivery_proxy_us,
            wake_reserve_us,
            cold_reserve_us,
            residual_bias_us,
        };

        let local_total = match class {
            LatencyClass::Hot => buckets.hot[n].total,
            LatencyClass::Cold => buckets.cold[n].total,
        };
        let confidence = if local_total == 0 {
            LeadConfidence::PriorOnly
        } else if local_total < SEED_SAMPLES as u64 {
            LeadConfidence::Warming
        } else {
            LeadConfidence::Learned
        };

        (components, confidence)
    }

    pub fn estimate_lead(&self, path: SendPath, n_keys: usize) -> LeadEstimate {
        self.estimate_lead_with_class(path, n_keys, LatencyClass::Hot)
    }

    pub fn estimate_lead_with_class(
        &self,
        path: SendPath,
        n_keys: usize,
        latency_class: LatencyClass,
    ) -> LeadEstimate {
        self.estimate_lead_with_class_and_policy(path, n_keys, latency_class, false)
    }

    pub fn estimate_lead_with_class_and_policy(
        &self,
        path: SendPath,
        n_keys: usize,
        latency_class: LatencyClass,
        strict_upper_tail: bool,
    ) -> LeadEstimate {
        let n = 1.max(self.max_poly.min(n_keys));
        let channel = channel_index(path, latency_class);
        self.lead_cache[n][channel][usize::from(strict_upper_tail)]
    }

    pub fn get_lead_us(&self, path: SendPath, n_keys: usize) -> u64 {
        self.estimate_lead(path, n_keys).applied_us
    }

    pub fn lead_saturated(&self, path: SendPath, n_keys: usize) -> bool {
        self.estimate_lead(path, n_keys).saturated
    }

    // ── State persistence ─────────────────────────────────────────────────────

    pub fn export_state(&self) -> EstimatorStateJson {
        let export_buckets = |buckets: &PathBuckets| {
            buckets
                .hot
                .iter()
                .zip(&buckets.cold)
                .zip(&buckets.tail_hot)
                .zip(&buckets.tail_cold)
                .map(|(((hot, cold), hot_tail), cold_tail)| HistBucketJson {
                    hot_pairs: hot.to_export_pairs(),
                    cold_pairs: cold.to_export_pairs(),
                    hot_tail_reserve_us: hot_tail.get(),
                    cold_tail_reserve_us: cold_tail.get(),
                })
                .collect()
        };
        let hist_down = export_buckets(&self.down);
        let hist_up = export_buckets(&self.up);
        let hist_mixed = export_buckets(&self.mixed);

        let residuals = self
            .residuals
            .iter()
            .map(|r| ResidualChannelJson {
                count: r.count,
                sum: r.sum,
                ema: r.ema,
                warm: r.warm,
            })
            .collect();
        let delivery_proxy_channels: Vec<[u64; 6]> = (0..=self.max_poly)
            .map(|polyphony| {
                std::array::from_fn(|channel| self.delivery_proxy_us[channel][polyphony])
            })
            .collect();

        EstimatorStateJson {
            version: ESTIMATOR_STATE_VERSION,
            max_poly: self.max_poly,
            hist_down,
            hist_up,
            hist_mixed,
            residuals,
            delivery_proxy_channels,
        }
    }

    pub fn import_state(&mut self, json_str: &str) -> Result<(), String> {
        let state: EstimatorStateJson =
            serde_json::from_str(json_str).map_err(|e| format!("invalid estimator json: {e}"))?;

        if state.version != ESTIMATOR_STATE_VERSION {
            return Err(format!(
                "unsupported estimator version {}; timing cache must be regenerated",
                state.version
            ));
        }
        if !(1..=32).contains(&state.max_poly) {
            return Err("max_poly must be in 1..=32".to_string());
        }
        let expected_len = state.max_poly + 1;
        let target_poly = self.max_poly.max(state.max_poly);
        let target_len = target_poly + 1;

        let mut new_down = PathBuckets::new(target_len);
        let mut new_up = PathBuckets::new(target_len);
        let mut new_mixed = PathBuckets::new(target_len);
        let mut new_down_totals = PathTotals::default();
        let mut new_up_totals = PathTotals::default();
        let mut new_mixed_totals = PathTotals::default();

        if state.hist_down.len() != expected_len
            || state.hist_up.len() != expected_len
            || state.hist_mixed.len() != expected_len
        {
            return Err("hist_down/hist_up/hist_mixed length does not match max_poly".to_string());
        }
        for (i, bucket) in state.hist_down.iter().enumerate() {
            new_down.hot[i] = Histogram::from_export_pairs(&bucket.hot_pairs)?;
            new_down.cold[i] = Histogram::from_export_pairs(&bucket.cold_pairs)?;
            if bucket.hot_tail_reserve_us > MAX_SAMPLE_US
                || bucket.cold_tail_reserve_us > MAX_SAMPLE_US
            {
                return Err(format!("down tail reserve at {i} exceeds sample cap"));
            }
            new_down.tail_hot[i].value_us = bucket.hot_tail_reserve_us;
            new_down.tail_cold[i].value_us = bucket.cold_tail_reserve_us;
        }
        for (i, bucket) in state.hist_up.iter().enumerate() {
            new_up.hot[i] = Histogram::from_export_pairs(&bucket.hot_pairs)?;
            new_up.cold[i] = Histogram::from_export_pairs(&bucket.cold_pairs)?;
            if bucket.hot_tail_reserve_us > MAX_SAMPLE_US
                || bucket.cold_tail_reserve_us > MAX_SAMPLE_US
            {
                return Err(format!("up tail reserve at {i} exceeds sample cap"));
            }
            new_up.tail_hot[i].value_us = bucket.hot_tail_reserve_us;
            new_up.tail_cold[i].value_us = bucket.cold_tail_reserve_us;
        }
        for (i, bucket) in state.hist_mixed.iter().enumerate() {
            new_mixed.hot[i] = Histogram::from_export_pairs(&bucket.hot_pairs)?;
            new_mixed.cold[i] = Histogram::from_export_pairs(&bucket.cold_pairs)?;
            if bucket.hot_tail_reserve_us > MAX_SAMPLE_US
                || bucket.cold_tail_reserve_us > MAX_SAMPLE_US
            {
                return Err(format!("mixed tail reserve at {i} exceeds sample cap"));
            }
            new_mixed.tail_hot[i].value_us = bucket.hot_tail_reserve_us;
            new_mixed.tail_cold[i].value_us = bucket.cold_tail_reserve_us;
        }
        for h in &new_down.hot {
            new_down_totals.hot.merge_counts_from(h)?;
        }
        for h in &new_down.cold {
            new_down_totals.cold.merge_counts_from(h)?;
        }
        for h in &new_up.hot {
            new_up_totals.hot.merge_counts_from(h)?;
        }
        for h in &new_up.cold {
            new_up_totals.cold.merge_counts_from(h)?;
        }
        for h in &new_mixed.hot {
            new_mixed_totals.hot.merge_counts_from(h)?;
        }
        for h in &new_mixed.cold {
            new_mixed_totals.cold.merge_counts_from(h)?;
        }

        let mut new_residuals: [ResidualEma; 6] = Default::default();

        if state.residuals.len() != 6 {
            return Err("residuals must contain exactly six channels".to_string());
        }
        for (i, ch) in state.residuals.iter().enumerate() {
            if !ch.ema.is_finite()
                || ch.ema < -(MAX_RESIDUAL_US as f64)
                || ch.ema > (MAX_RESIDUAL_US * 2) as f64
            {
                return Err(format!("residual channel {i} ema is out of range"));
            }
            if (ch.warm && ch.count < SEED_SAMPLES as u64)
                || (!ch.warm && ch.count >= SEED_SAMPLES as u64)
            {
                return Err(format!("residual channel {i} has inconsistent warm state"));
            }
            if ch.sum.unsigned_abs()
                > ch.count
                    .checked_mul((MAX_RESIDUAL_US * 2) as u64)
                    .ok_or_else(|| format!("residual channel {i} sum overflows"))?
            {
                return Err(format!("residual channel {i} sum is out of range"));
            }
            new_residuals[i] = ResidualEma {
                count: ch.count,
                sum: ch.sum,
                ema: ch.ema,
                warm: ch.warm,
            };
        }

        let mut new_delivery_proxy: [Vec<u64>; 6] = std::array::from_fn(|_| vec![0u64; target_len]);
        if state.delivery_proxy_channels.len() != expected_len {
            return Err("delivery_proxy_channels length does not match max_poly".to_string());
        }
        for (polyphony, channels) in state.delivery_proxy_channels.iter().enumerate() {
            for (channel, &value) in channels.iter().enumerate() {
                if value > MAX_SAMPLE_US {
                    return Err(format!(
                        "delivery proxy channel {channel} at polyphony {polyphony} exceeds sample cap"
                    ));
                }
                new_delivery_proxy[channel][polyphony] = value;
            }
        }

        // ── Atomically apply all validated state ──────────────────────────────
        self.max_poly = target_poly;
        self.down = new_down;
        self.up = new_up;
        self.mixed = new_mixed;
        self.down_totals = new_down_totals;
        self.up_totals = new_up_totals;
        self.mixed_totals = new_mixed_totals;
        self.residuals = new_residuals;
        self.delivery_proxy_us = new_delivery_proxy;
        self.refresh_lead_cache();
        Ok(())
    }
}

impl Default for SendLatencyEstimator {
    fn default() -> Self {
        Self::new_unchecked(0.2, 2_000, 15)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(unused_must_use)]
mod tests {
    use super::*;

    #[test]
    fn test_bankers_rounding() {
        assert_eq!(round_half_to_even(0.5), 0);
        assert_eq!(round_half_to_even(1.5), 2);
        assert_eq!(round_half_to_even(2.5), 2);
        assert_eq!(round_half_to_even(3.5), 4);
        assert_eq!(round_half_to_even(0.4), 0);
        assert_eq!(round_half_to_even(0.6), 1);
        assert_eq!(round_half_to_even(-0.5), 0);
        assert_eq!(round_half_to_even(-1.5), -2);
        assert_eq!(round_half_to_even(-2.5), -2);
    }

    #[test]
    fn histogram_p95_is_conservative_and_bounded() {
        let mut hist = Histogram::default();
        for value in (100u64..=3100).step_by(100) {
            hist.push(value);
        }
        let p95 = hist.p95().expect("should be warm");
        let p50 = hist.p_quantile(0.5).unwrap();
        assert!(p95 > p50, "p95={p95} should exceed p50={p50}");
        assert!(
            p95 <= 3100 + BUCKET_WIDTH_US,
            "p95 should not exceed max+bucket"
        );
    }

    #[test]
    fn cold_start_is_conservative_for_large_chords() {
        let estimator = SendLatencyEstimator::new(0.2, 5_000, 15);
        let lead_1 = estimator.get_lead_us(SendPath::DownOnly, 1);
        let lead_15 = estimator.get_lead_us(SendPath::DownOnly, 15);
        assert!(
            lead_15 >= lead_1,
            "larger chord must have at least as much lead"
        );
        assert!(lead_1 > 0, "cold-start lead must be positive");
    }

    #[test]
    fn both_directions_use_polyphony_buckets_and_monotonic_envelope() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 6);
        for _ in 0..10 {
            estimator.update(SendPath::DownOnly, 200, 1);
            estimator.update(SendPath::DownOnly, 900, 3);
            estimator.update(SendPath::UpOnly, 150, 1);
            estimator.update(SendPath::UpOnly, 850, 3);
        }
        let down_1 = estimator.get_lead_us(SendPath::DownOnly, 1);
        let down_3 = estimator.get_lead_us(SendPath::DownOnly, 3);
        let up_1 = estimator.get_lead_us(SendPath::UpOnly, 1);
        let up_3 = estimator.get_lead_us(SendPath::UpOnly, 3);
        assert!(down_3 >= down_1, "down: 3-key must be >= 1-key");
        assert!(up_3 >= up_1, "up: 3-key must be >= 1-key");
        assert!(
            down_3 > cold_prior_us_helper(3) + WAKE_RESERVE_US - 50,
            "down_3={down_3} should be data-driven above cold prior"
        );
    }

    #[test]
    fn observation_update_matches_separate_histogram_and_residual_updates() {
        let mut combined = SendLatencyEstimator::new(0.2, 10_000, 4);
        let mut separate = combined.clone();
        for (path, class, duration, polyphony, residual) in [
            (SendPath::DownOnly, LatencyClass::Hot, 120, 1, Some(80)),
            (SendPath::DownOnly, LatencyClass::Cold, 2_400, 3, Some(-120)),
            (SendPath::UpOnly, LatencyClass::Hot, 260, 2, None),
            (SendPath::Mixed, LatencyClass::Hot, 350, 4, Some(40)),
        ] {
            combined
                .update_observation(path, class, duration, polyphony, residual)
                .unwrap();
            separate
                .update_with_class(path, duration, polyphony, class)
                .unwrap();
            if let Some(residual) = residual {
                separate
                    .update_completion_error_with_class(path, residual, class)
                    .unwrap();
            }
        }
        assert_eq!(
            serde_json::to_string(&combined.export_state()).unwrap(),
            serde_json::to_string(&separate.export_state()).unwrap()
        );
        for path in [SendPath::DownOnly, SendPath::UpOnly, SendPath::Mixed] {
            for class in [LatencyClass::Hot, LatencyClass::Cold] {
                for strict in [false, true] {
                    for polyphony in 1..=4 {
                        assert_eq!(
                            combined.estimate_lead_with_class_and_policy(
                                path, polyphony, class, strict
                            ),
                            separate.estimate_lead_with_class_and_policy(
                                path, polyphony, class, strict
                            )
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn observation_refreshes_the_cache_once() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 4);
        let initial_refreshes = estimator.refresh_count;
        estimator
            .update_observation(SendPath::DownOnly, LatencyClass::Hot, 100, 2, Some(50))
            .unwrap();
        assert_eq!(estimator.refresh_count, initial_refreshes + 1);
    }

    #[test]
    fn observation_refreshes_only_its_direction_and_class_channel() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 4);
        let up_cold_before = estimator
            .lead_cache
            .iter()
            .map(|poly_cache| poly_cache[channel_index(SendPath::UpOnly, LatencyClass::Cold)])
            .collect::<Vec<_>>();
        estimator
            .update_observation(SendPath::DownOnly, LatencyClass::Hot, 100, 2, Some(50))
            .unwrap();
        let up_cold_after = estimator
            .lead_cache
            .iter()
            .map(|poly_cache| poly_cache[channel_index(SendPath::UpOnly, LatencyClass::Cold)])
            .collect::<Vec<_>>();
        assert_eq!(up_cold_before, up_cold_after);
    }

    fn cold_prior_us_helper(n: usize) -> u64 {
        SendLatencyEstimator::cold_prior_us(n)
    }

    #[test]
    fn strict_upper_tail_keeps_a_single_recent_outlier_visible() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 1);
        for _ in 0..64 {
            estimator.update(SendPath::DownOnly, 100, 1);
        }
        estimator.update(SendPath::DownOnly, 3_000, 1);

        let normal = estimator
            .estimate_lead_with_class(SendPath::DownOnly, 1, LatencyClass::Hot)
            .applied_us;
        let strict = estimator
            .estimate_lead_with_class_and_policy(SendPath::DownOnly, 1, LatencyClass::Hot, true)
            .applied_us;
        assert!(strict >= normal, "strict={strict} normal={normal}");
        assert!(
            estimator.down.tail_hot[1].get() >= 3_000,
            "tail reserve must have captured the outlier"
        );
    }

    #[test]
    fn strict_sparse_bucket_keeps_global_upper_tail_guard() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 8);
        for _ in 0..32 {
            estimator.update(SendPath::DownOnly, 1_500, 8);
        }
        for _ in 0..5 {
            estimator.update(SendPath::DownOnly, 300, 1);
        }
        let strict_1 = estimator
            .estimate_lead_with_class_and_policy(SendPath::DownOnly, 1, LatencyClass::Hot, true)
            .applied_us;
        assert!(
            strict_1 >= 1_500,
            "strict 1-key lead={strict_1} should see global 1500µs tail"
        );
    }

    #[test]
    fn cold_outlier_does_not_raise_hot_global_guard() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 2);
        for _ in 0..8 {
            estimator.update_with_class(SendPath::DownOnly, 100, 1, LatencyClass::Hot);
        }
        let hot_before = estimator
            .estimate_lead_with_class_and_policy(SendPath::DownOnly, 1, LatencyClass::Hot, true)
            .applied_us;
        for _ in 0..8 {
            estimator.update_with_class(SendPath::DownOnly, 3_000, 2, LatencyClass::Cold);
        }
        let hot_after = estimator
            .estimate_lead_with_class_and_policy(SendPath::DownOnly, 1, LatencyClass::Hot, true)
            .applied_us;
        assert_eq!(hot_after, hot_before);
        assert!(
            estimator
                .estimate_lead_with_class_and_policy(
                    SendPath::DownOnly,
                    2,
                    LatencyClass::Cold,
                    true
                )
                .applied_us
                >= 3_000
        );
    }

    #[test]
    fn lead_components_are_named_and_non_zero() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 3);
        for _ in 0..10 {
            estimator.update(SendPath::DownOnly, 500, 2);
        }
        let est = estimator.estimate_lead(SendPath::DownOnly, 2);
        assert!(
            est.components.syscall_us > 0,
            "syscall component must be nonzero"
        );
        assert_eq!(est.components.wake_reserve_us, WAKE_RESERVE_US);
        assert_eq!(
            est.components.delivery_proxy_us, 0,
            "uncalibrated proxy should be 0"
        );
        assert_eq!(est.confidence, LeadConfidence::Learned);
    }

    #[test]
    fn confidence_progresses_from_prior_to_learned() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 1);
        assert_eq!(
            estimator.estimate_lead(SendPath::DownOnly, 1).confidence,
            LeadConfidence::PriorOnly
        );
        for i in 0..SEED_SAMPLES - 1 {
            estimator.update(SendPath::DownOnly, 100, 1);
            let conf = estimator.estimate_lead(SendPath::DownOnly, 1).confidence;
            assert_eq!(
                conf,
                LeadConfidence::Warming,
                "at sample {i} expected Warming"
            );
        }
        estimator.update(SendPath::DownOnly, 100, 1);
        assert_eq!(
            estimator.estimate_lead(SendPath::DownOnly, 1).confidence,
            LeadConfidence::Learned
        );
    }

    #[test]
    fn delivery_proxy_component_is_additive() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 2);
        for _ in 0..10 {
            estimator.update(SendPath::DownOnly, 200, 2);
        }
        let without_proxy = estimator.get_lead_us(SendPath::DownOnly, 2);
        estimator.set_delivery_proxy_us(2, 300).unwrap();
        let with_proxy = estimator.get_lead_us(SendPath::DownOnly, 2);
        assert_eq!(with_proxy, without_proxy + 300);
    }

    #[test]
    fn hot_and_cold_histograms_are_class_isolated() {
        let mut estimator = SendLatencyEstimator::new(0.2, 2_000, 3);
        for _ in 0..SEED_SAMPLES {
            estimator.update_with_class(SendPath::DownOnly, 400, 1, LatencyClass::Cold);
        }
        assert_eq!(estimator.down.hot[1].total, 0);
        assert_eq!(estimator.down.cold[1].total, SEED_SAMPLES as u64);

        for _ in 0..SEED_SAMPLES {
            estimator.update_with_class(SendPath::DownOnly, 100, 1, LatencyClass::Hot);
        }
        assert_eq!(estimator.down.hot[1].total, SEED_SAMPLES as u64);
        assert_eq!(estimator.down.cold[1].total, SEED_SAMPLES as u64);
    }

    #[test]
    fn delivery_proxy_is_independent_by_direction_and_class() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 3);
        estimator
            .set_delivery_proxy_us_for(SendPath::DownOnly, LatencyClass::Hot, 1, 100)
            .unwrap();
        estimator
            .set_delivery_proxy_us_for(SendPath::DownOnly, LatencyClass::Cold, 1, 200)
            .unwrap();
        estimator
            .set_delivery_proxy_us_for(SendPath::UpOnly, LatencyClass::Hot, 1, 300)
            .unwrap();
        estimator
            .set_delivery_proxy_us_for(SendPath::UpOnly, LatencyClass::Cold, 1, 400)
            .unwrap();
        estimator
            .set_delivery_proxy_us_for(SendPath::Mixed, LatencyClass::Hot, 1, 500)
            .unwrap();

        assert_eq!(
            estimator
                .estimate_lead_with_class(SendPath::DownOnly, 1, LatencyClass::Hot)
                .components
                .delivery_proxy_us,
            100
        );
        assert_eq!(
            estimator
                .estimate_lead_with_class(SendPath::DownOnly, 1, LatencyClass::Cold)
                .components
                .delivery_proxy_us,
            200
        );
        assert_eq!(
            estimator
                .estimate_lead_with_class(SendPath::UpOnly, 1, LatencyClass::Hot)
                .components
                .delivery_proxy_us,
            300
        );
        assert_eq!(
            estimator
                .estimate_lead_with_class(SendPath::UpOnly, 1, LatencyClass::Cold)
                .components
                .delivery_proxy_us,
            400
        );
        assert_eq!(
            estimator
                .estimate_lead_with_class(SendPath::Mixed, 1, LatencyClass::Hot)
                .components
                .delivery_proxy_us,
            500
        );
    }

    #[test]
    fn cold_prior_is_added_once_when_bucket_is_unwarmed() {
        let estimator = SendLatencyEstimator::new(0.2, 10_000, 3);
        let estimate =
            estimator.estimate_lead_with_class(SendPath::DownOnly, 1, LatencyClass::Cold);
        assert_eq!(estimate.components.syscall_us, 0);
        assert_eq!(estimate.components.cold_reserve_us, BASE_COLD_PRIOR_US);
        assert_eq!(
            estimate.components.total_uncapped(),
            BASE_COLD_PRIOR_US + WAKE_RESERVE_US
        );
    }

    #[test]
    fn slow_tail_reserve_decays_but_stays_above_floor() {
        let mut reserve = SlowTailReserve::default();
        reserve.update(5_000);
        for _ in 0..256 {
            reserve.update(100);
        }
        let value = reserve.get();
        assert!(value > 0, "slow tail reserve must not decay to zero");
        assert!(
            value < 5_000,
            "slow tail reserve should decay below the initial outlier"
        );
        assert!(
            value >= 2_000,
            "a 256-sample half-life must retain the outlier tail: {value}"
        );
    }

    #[test]
    fn up_residual_is_learned_separately_from_down_residual() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 2);
        for _ in 0..SEED_SAMPLES {
            estimator.update(SendPath::UpOnly, 100, 1);
            estimator.update_completion_error(SendPath::UpOnly, 300);
        }
        let adj_up = estimator.residual_adjustment_us_for(SendPath::UpOnly);
        let adj_down = estimator.residual_adjustment_us_for(SendPath::DownOnly);
        assert!(adj_up > 0, "up residual must be learned");
        assert_eq!(adj_down, 0, "down residual must remain zero");
    }

    #[test]
    fn cold_residual_is_learned_separately_from_hot_residual() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 2);
        for _ in 0..SEED_SAMPLES {
            estimator.update_with_class(SendPath::DownOnly, 100, 1, LatencyClass::Hot);
            estimator.update_completion_error_with_class(
                SendPath::DownOnly,
                300,
                LatencyClass::Hot,
            );

            estimator.update_with_class(SendPath::DownOnly, 100, 1, LatencyClass::Cold);
            estimator.update_completion_error_with_class(
                SendPath::DownOnly,
                -300,
                LatencyClass::Cold,
            );
        }

        let adj_hot = estimator.residual_adjustment_us_for(SendPath::DownOnly);
        let cold_idx = channel_index(SendPath::DownOnly, LatencyClass::Cold);
        let adj_cold = estimator.residuals[cold_idx].adjustment_us();

        assert!(adj_hot > 0, "hot residual must be positive learned");
        assert!(adj_cold < 0, "cold residual must be negative learned");
    }

    #[test]
    fn early_residual_reduces_lead_more_slowly_than_late_residual_increases_it() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 2);
        for _ in 0..SEED_SAMPLES {
            estimator.update(SendPath::DownOnly, 800, 1);
            estimator.update_completion_error(SendPath::DownOnly, -400);
        }
        let adj = estimator.residual_adjustment_us();
        assert!(adj < 0, "early residual should be negative");
        assert!(
            adj > -400,
            "dampened early residual must be less than raw -400"
        );

        for _ in 0..SEED_SAMPLES {
            estimator.update_completion_error(SendPath::DownOnly, 400);
        }
        let adj2 = estimator.residual_adjustment_us();
        assert!(
            adj2 > adj,
            "late samples must push residual back toward positive"
        );
    }

    #[test]
    fn current_state_round_trip_preserves_histogram() {
        let mut source = SendLatencyEstimator::new(0.2, 5_000, 2);
        for v in [100, 200, 300, 400, 500, 600, 700, 800] {
            source.update(SendPath::DownOnly, v, 2);
            source.update(SendPath::UpOnly, v + 50, 2);
            source.update(SendPath::Mixed, v + 100, 2);
        }
        let json = serde_json::to_string(&source.export_state()).unwrap();
        let mut restored = SendLatencyEstimator::new(0.2, 5_000, 4);
        restored.import_state(&json).unwrap();
        assert_eq!(restored.export_state().version, ESTIMATOR_STATE_VERSION);
        let src_lead = source.get_lead_us(SendPath::DownOnly, 2);
        let rst_lead = restored.get_lead_us(SendPath::DownOnly, 2);
        let diff = src_lead.abs_diff(rst_lead);
        assert!(
            diff <= BUCKET_WIDTH_US * 2,
            "round-trip lead should be within 2 bucket widths: src={src_lead} rst={rst_lead}"
        );
    }

    #[test]
    fn v8_state_rejected() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 2);
        let mut valid_v9 = estimator.export_state();
        valid_v9.version = 8;
        let invalid_v8 = serde_json::to_string(&valid_v9).unwrap();
        let err = estimator.import_state(&invalid_v8).unwrap_err();
        assert!(err.contains("unsupported estimator version 8"));
    }

    #[test]
    fn v9_empty_state_roundtrip() {
        let estimator = SendLatencyEstimator::new(0.2, 5_000, 2);
        let json = serde_json::to_string(&estimator.export_state()).unwrap();
        let mut restored = SendLatencyEstimator::new(0.2, 5_000, 2);
        restored.import_state(&json).unwrap();
        assert_eq!(restored.export_state().version, 9);
    }

    #[test]
    fn v9_populated_roundtrip() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 8);
        for _ in 0..10 {
            estimator.update(SendPath::DownOnly, 150, 1);
            estimator.update(SendPath::UpOnly, 200, 2);
            estimator.update(SendPath::Mixed, 350, 8);
        }
        let json = serde_json::to_string(&estimator.export_state()).unwrap();
        let mut restored = SendLatencyEstimator::new(0.2, 5_000, 8);
        restored.import_state(&json).unwrap();
        assert_eq!(
            restored.get_lead_us(SendPath::Mixed, 8),
            estimator.get_lead_us(SendPath::Mixed, 8)
        );
    }

    #[test]
    fn down_hot_isolated_from_down_cold() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 2);
        for _ in 0..10 {
            estimator.update_with_class(SendPath::DownOnly, 100, 1, LatencyClass::Hot);
        }
        assert_eq!(estimator.down.cold[1].total, 0);
    }

    #[test]
    fn up_hot_isolated_from_up_cold() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 2);
        for _ in 0..10 {
            estimator.update_with_class(SendPath::UpOnly, 100, 1, LatencyClass::Hot);
        }
        assert_eq!(estimator.up.cold[1].total, 0);
    }

    #[test]
    fn mixed_hot_isolated_from_mixed_cold() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 2);
        for _ in 0..10 {
            estimator.update_with_class(SendPath::Mixed, 100, 1, LatencyClass::Hot);
        }
        assert_eq!(estimator.mixed.cold[1].total, 0);
    }

    #[test]
    fn mixed_observations_do_not_train_down_or_up() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 4);
        for _ in 0..10 {
            estimator.update(SendPath::Mixed, 500, 4);
        }
        assert_eq!(estimator.down.hot[4].total, 0);
        assert_eq!(estimator.up.hot[4].total, 0);
        assert_eq!(estimator.mixed.hot[4].total, 10);
    }

    #[test]
    fn down_observations_do_not_train_mixed() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 4);
        for _ in 0..10 {
            estimator.update(SendPath::DownOnly, 500, 4);
        }
        assert_eq!(estimator.mixed.hot[4].total, 0);
    }

    #[test]
    fn mixed_event_count_8_trains_bucket_8() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 8);
        estimator.update(SendPath::Mixed, 400, 8).unwrap();
        assert_eq!(estimator.mixed.hot[8].total, 1);
    }

    #[test]
    fn five_clean_mixed_samples_transition_confidence_to_learned() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 8);
        assert_eq!(
            estimator.estimate_lead(SendPath::Mixed, 8).confidence,
            LeadConfidence::PriorOnly
        );
        for _ in 0..4 {
            estimator.update(SendPath::Mixed, 300, 8).unwrap();
        }
        assert_eq!(
            estimator.estimate_lead(SendPath::Mixed, 8).confidence,
            LeadConfidence::Warming
        );
        estimator.update(SendPath::Mixed, 300, 8).unwrap();
        assert_eq!(
            estimator.estimate_lead(SendPath::Mixed, 8).confidence,
            LeadConfidence::Learned
        );
    }

    #[test]
    fn query_path_performs_no_allocation() {
        let estimator = SendLatencyEstimator::new(0.2, 5_000, 8);
        let lead = estimator.estimate_lead_with_class_and_policy(
            SendPath::Mixed,
            8,
            LatencyClass::Hot,
            true,
        );
        assert!(lead.applied_us > 0);
    }
}
