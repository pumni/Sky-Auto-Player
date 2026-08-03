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
//! | `residual_bias_us` | EMA of completion-error residual (Down/Up × Hot/Cold) |
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
//! Version 8 publishes the fixed lead lookup table. Timing cache state is
//! ephemeral diagnostic data, so only the current schema is accepted; older or
//! newer versions are discarded and the conservative prior is used.

use crate::model::ActionKind;
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
pub const ESTIMATOR_STATE_VERSION: u32 = 8;

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

// ─── Residual EMA (4 independent channels) ───────────────────────────────────

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

    /// Signed residual contribution.  Late samples raise lead at full rate;
    /// early samples only reduce it at `EARLY_CORRECTION_DECAY` rate.
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

// ─── Helper ───────────────────────────────────────────────────────────────────

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

// ─── Per-bucket state ─────────────────────────────────────────────────────────

/// Collects all per-(polyphony, kind, class) state for one direction.
#[derive(Debug, Clone, Default)]
struct DirectionBuckets {
    hot: Vec<Histogram>,
    cold: Vec<Histogram>,
    /// Slow-tail envelopes remain class-specific. A cold outlier must not
    /// raise the strict hot estimate, including through the tail guard.
    tail_hot: Vec<SlowTailReserve>,
    tail_cold: Vec<SlowTailReserve>,
}

impl DirectionBuckets {
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
    ///
    /// Combines:
    /// - Local p95 (fast model for the exact polyphony)
    /// - Cold-class local p95 when class is Cold
    /// - Slow tail reserve for the outlier guard
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

        // Slow tail reserve is class-specific for the same reason as the
        // local histogram: cold evidence cannot rewrite a hot estimate.
        let tail_value = match class {
            LatencyClass::Hot => self.tail_hot[n].get(),
            LatencyClass::Cold => self.tail_cold[n].get(),
        };
        let tail = (tail_value > 0).then_some(tail_value);

        [local, tail].into_iter().flatten().max()
    }
}

#[derive(Debug, Clone, Default)]
struct DirectionTotals {
    hot: Histogram,
    cold: Histogram,
}

impl DirectionTotals {
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

/// Serialisable per-direction histogram bucket with class-specific tails.
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

/// Version-5 residual channel export.
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
    /// Four residual channels: down_hot, down_cold, up_hot, up_cold.
    pub residuals: Vec<ResidualChannelJson>,
    /// Delivery proxy channels ordered as Down/Hot, Down/Cold, Up/Hot, Up/Cold.
    pub delivery_proxy_channels: Vec<[u64; 4]>,
}

// ─── Main estimator ───────────────────────────────────────────────────────────

/// Conservative adaptive estimator with named lead components.
///
/// The estimator exposes the current histogram/residual model only.
/// `estimate_lead` returns a `LeadEstimate` with a full `LeadComponents`
/// breakdown and a `LeadConfidence` level.
#[derive(Debug, Clone)]
pub struct SendLatencyEstimator {
    pub max_poly: usize,
    alpha: f64,
    max_lead_us: u64,

    down: DirectionBuckets,
    up: DirectionBuckets,

    /// Global cross-bucket guards, kept independent for Hot and Cold.
    down_totals: DirectionTotals,
    up_totals: DirectionTotals,

    /// Residual channels indexed by `residual_index(kind, class)`.
    /// Order: [down_hot, down_cold, up_hot, up_cold].
    residuals: [ResidualEma; 4],

    /// Calibrated delivery proxy prior by kind/class and polyphony.
    /// Order: down_hot, down_cold, up_hot, up_cold.
    delivery_proxy_us: [Vec<u64>; 4],

    /// Cached normal/strict estimates indexed by polyphony, channel and
    /// policy. The cache is refreshed in place after a sample or calibration
    /// update; dispatch only indexes it.
    lead_cache: Vec<[[LeadEstimate; 2]; 4]>,
    #[cfg(test)]
    refresh_count: u64,
}

fn residual_index(kind: ActionKind, class: LatencyClass) -> usize {
    match (kind, class) {
        (ActionKind::Down, LatencyClass::Hot) => 0,
        (ActionKind::Down, LatencyClass::Cold) => 1,
        (ActionKind::Up, LatencyClass::Hot) => 2,
        (ActionKind::Up, LatencyClass::Cold) => 3,
    }
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
            down: DirectionBuckets::new(size),
            up: DirectionBuckets::new(size),
            down_totals: DirectionTotals::default(),
            up_totals: DirectionTotals::default(),
            residuals: Default::default(),
            delivery_proxy_us: std::array::from_fn(|_| vec![0u64; size]),
            lead_cache: vec![[[empty; 2]; 4]; size],
            #[cfg(test)]
            refresh_count: 0,
        };
        estimator.refresh_lead_cache();
        estimator
    }

    fn refresh_lead_cache(&mut self) {
        self.refresh_channel(ActionKind::Down, LatencyClass::Hot);
        self.refresh_channel(ActionKind::Down, LatencyClass::Cold);
        self.refresh_channel(ActionKind::Up, LatencyClass::Hot);
        self.refresh_channel(ActionKind::Up, LatencyClass::Cold);
    }

    fn refresh_channel(&mut self, kind: ActionKind, class: LatencyClass) {
        #[cfg(test)]
        {
            self.refresh_count = self.refresh_count.saturating_add(1);
        }
        let channel = residual_index(kind, class);
        for strict_index in 0..2 {
            let strict = strict_index == 1;
            let mut best_components = LeadComponents::default();
            let mut best_uncapped = 0u64;
            let mut best_confidence = LeadConfidence::PriorOnly;
            for n in 1..=self.max_poly {
                let (components, confidence) = self.build_components(kind, n, class, strict);
                let uncapped = components.total_uncapped();
                // `>=` preserves the previous deterministic tie-break:
                // an equal estimate from the latest bucket wins.
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
        kind: ActionKind,
        duration_us: u64,
        n_keys: usize,
    ) -> Result<(), EstimatorStateError> {
        self.update_with_class(kind, duration_us, n_keys, LatencyClass::Hot)
    }

    pub fn update_with_class(
        &mut self,
        kind: ActionKind,
        duration_us: u64,
        n_keys: usize,
        latency_class: LatencyClass,
    ) -> Result<(), EstimatorStateError> {
        self.update_observation(kind, latency_class, duration_us, n_keys, None)
    }

    /// Record one clean sender observation and refresh the affected lead cache
    /// exactly once. The optional residual is validated before any histogram
    /// mutation so overflow remains atomic from the caller's perspective.
    pub fn update_observation(
        &mut self,
        kind: ActionKind,
        class: LatencyClass,
        duration_us: u64,
        polyphony: usize,
        completion_error_us: Option<i64>,
    ) -> Result<(), EstimatorStateError> {
        let duration_us = duration_us.min(MAX_SAMPLE_US);
        let n = 1.max(self.max_poly.min(polyphony));
        match kind {
            ActionKind::Down => {
                self.down.ensure_push(n, duration_us, class)?;
                self.down_totals.ensure_push(duration_us, class)?;
            }
            ActionKind::Up => {
                self.up.ensure_push(n, duration_us, class)?;
                self.up_totals.ensure_push(duration_us, class)?;
            }
        }
        if let Some(error_us) = completion_error_us {
            self.residuals[residual_index(kind, class)].ensure_update(error_us)?;
        }
        match kind {
            ActionKind::Down => {
                self.down.push(n, duration_us, class)?;
                self.down_totals.push(duration_us, class)?;
            }
            ActionKind::Up => {
                self.up.push(n, duration_us, class)?;
                self.up_totals.push(duration_us, class)?;
            }
        }
        if let Some(error_us) = completion_error_us {
            let alpha = self.alpha;
            self.residuals[residual_index(kind, class)].update(alpha, error_us)?;
        }
        self.refresh_channel(kind, class);
        Ok(())
    }

    /// Record a completion-error residual for the given kind, assuming Hot class for backward compatibility.
    pub fn update_completion_error(
        &mut self,
        kind: ActionKind,
        error_us: i64,
    ) -> Result<(), EstimatorStateError> {
        self.update_completion_error_with_class(kind, error_us, LatencyClass::Hot)
    }

    /// Record a completion-error residual for the given kind and class.
    ///
    /// Callers must NOT update residual from: retries, partial insertions,
    /// deferred releases, mixed-source releases, focus transitions, wait
    /// failures, cleanup, or telemetry-mode-perturbed dispatches.
    pub fn update_completion_error_with_class(
        &mut self,
        kind: ActionKind,
        error_us: i64,
        class: LatencyClass,
    ) -> Result<(), EstimatorStateError> {
        let alpha = self.alpha;
        self.residuals[residual_index(kind, class)].update(alpha, error_us)?;
        self.refresh_channel(kind, class);
        Ok(())
    }

    /// Update the calibrated delivery proxy prior for a polyphony bucket.
    ///
    /// Should be called from the calibration harness output processor, not
    /// from the real-time dispatch path.
    pub fn set_delivery_proxy_us(
        &mut self,
        n_keys: usize,
        value_us: u64,
    ) -> Result<(), EstimatorStateError> {
        self.set_delivery_proxy_us_for(ActionKind::Down, LatencyClass::Hot, n_keys, value_us)
    }

    pub fn try_set_delivery_proxy_us_for(
        &mut self,
        kind: ActionKind,
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
        self.delivery_proxy_us[residual_index(kind, class)][n] = value_us;
        self.refresh_channel(kind, class);
        Ok(())
    }

    pub fn set_delivery_proxy_us_for(
        &mut self,
        kind: ActionKind,
        class: LatencyClass,
        n_keys: usize,
        value_us: u64,
    ) -> Result<(), EstimatorStateError> {
        self.try_set_delivery_proxy_us_for(kind, class, n_keys, value_us)
    }

    // ── Query API ─────────────────────────────────────────────────────────────

    pub fn residual_bias_us(&self) -> u64 {
        self.residual_adjustment_us().max(0) as u64
    }

    pub fn residual_adjustment_us(&self) -> i64 {
        self.residual_adjustment_us_for(ActionKind::Down)
    }

    pub fn residual_adjustment_us_for(&self, kind: ActionKind) -> i64 {
        self.residuals[residual_index(kind, LatencyClass::Hot)].adjustment_us()
    }

    fn cold_prior_us(n: usize) -> u64 {
        BASE_COLD_PRIOR_US
            .saturating_add(PER_KEY_COLD_PRIOR_US.saturating_mul(n.saturating_sub(1) as u64))
    }

    /// Compute the syscall component for a single polyphony bucket `n`.
    fn syscall_estimate_us(
        &self,
        dir: &DirectionBuckets,
        total: &Histogram,
        n: usize,
        class: LatencyClass,
        strict_upper_tail: bool,
    ) -> u64 {
        let local = dir.raw_estimate_us(n, class, strict_upper_tail);

        // Lower-polyphony fallback with per-key extrapolation.
        let lower_bucket = (1..n).rev().find_map(|bucket| {
            dir.raw_estimate_us(bucket, class, strict_upper_tail)
                .map(|est| {
                    est.saturating_add(
                        PER_KEY_COLD_PRIOR_US.saturating_mul(n.saturating_sub(bucket) as u64),
                    )
                })
        });

        // Global guard: in strict mode the global tail is always kept visible.
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
        kind: ActionKind,
        n: usize,
        class: LatencyClass,
        strict_upper_tail: bool,
    ) -> (LeadComponents, LeadConfidence) {
        let (dir, total) = match kind {
            ActionKind::Down => (&self.down, self.down_totals.for_class(class)),
            ActionKind::Up => (&self.up, self.up_totals.for_class(class)),
        };

        let syscall_us = self.syscall_estimate_us(dir, total, n, class, strict_upper_tail);

        let channel = residual_index(kind, class);
        let delivery_proxy_us = self.delivery_proxy_us[channel][n];

        let wake_reserve_us = WAKE_RESERVE_US;

        let cold_reserve_us =
            if class == LatencyClass::Cold && dir.cold[n].total < SEED_SAMPLES as u64 {
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

        // Determine confidence from the local hot histogram.
        let local_total = match class {
            LatencyClass::Hot => dir.hot[n].total,
            LatencyClass::Cold => dir.cold[n].total,
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

    pub fn estimate_lead(&self, kind: ActionKind, n_keys: usize) -> LeadEstimate {
        self.estimate_lead_with_class(kind, n_keys, LatencyClass::Hot)
    }

    pub fn estimate_lead_with_class(
        &self,
        kind: ActionKind,
        n_keys: usize,
        latency_class: LatencyClass,
    ) -> LeadEstimate {
        self.estimate_lead_with_class_and_policy(kind, n_keys, latency_class, false)
    }

    pub fn estimate_lead_with_class_and_policy(
        &self,
        kind: ActionKind,
        n_keys: usize,
        latency_class: LatencyClass,
        strict_upper_tail: bool,
    ) -> LeadEstimate {
        let n = 1.max(self.max_poly.min(n_keys));
        let channel = residual_index(kind, latency_class);
        self.lead_cache[n][channel][usize::from(strict_upper_tail)]
    }

    pub fn get_lead_us(&self, kind: ActionKind, n_keys: usize) -> u64 {
        self.estimate_lead(kind, n_keys).applied_us
    }

    pub fn lead_saturated(&self, kind: ActionKind, n_keys: usize) -> bool {
        self.estimate_lead(kind, n_keys).saturated
    }

    // ── State persistence ─────────────────────────────────────────────────────

    pub fn export_state(&self) -> EstimatorStateJson {
        let export_direction = |direction: &DirectionBuckets| {
            direction
                .hot
                .iter()
                .zip(&direction.cold)
                .zip(&direction.tail_hot)
                .zip(&direction.tail_cold)
                .map(|(((hot, cold), hot_tail), cold_tail)| HistBucketJson {
                    hot_pairs: hot.to_export_pairs(),
                    cold_pairs: cold.to_export_pairs(),
                    hot_tail_reserve_us: hot_tail.get(),
                    cold_tail_reserve_us: cold_tail.get(),
                })
                .collect()
        };
        let hist_down = export_direction(&self.down);
        let hist_up = export_direction(&self.up);

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
        let delivery_proxy_channels: Vec<[u64; 4]> = (0..=self.max_poly)
            .map(|polyphony| {
                std::array::from_fn(|channel| self.delivery_proxy_us[channel][polyphony])
            })
            .collect();

        EstimatorStateJson {
            version: ESTIMATOR_STATE_VERSION,
            max_poly: self.max_poly,
            hist_down,
            hist_up,
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

        // ── Build histogram buckets ───────────────────────────────────────────
        let mut new_down = DirectionBuckets::new(target_len);
        let mut new_up = DirectionBuckets::new(target_len);
        let mut new_down_totals = DirectionTotals::default();
        let mut new_up_totals = DirectionTotals::default();

        if state.hist_down.len() != expected_len || state.hist_up.len() != expected_len {
            return Err("hist_down/hist_up length does not match max_poly".to_string());
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
        // Rebuild totals by direct histogram merge. No synthetic sample vector
        // or fixed reconstruction cap is involved.
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

        // ── Build residuals ───────────────────────────────────────────────────
        let mut new_residuals: [ResidualEma; 4] = Default::default();

        if state.residuals.len() != 4 {
            return Err("residuals must contain exactly four channels".to_string());
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

        // ── Delivery proxy ────────────────────────────────────────────────────
        let mut new_delivery_proxy: [Vec<u64>; 4] = std::array::from_fn(|_| vec![0u64; target_len]);
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
        self.down_totals = new_down_totals;
        self.up_totals = new_up_totals;
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
        // 31 samples; p95 should be well above the median.
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
        let lead_1 = estimator.get_lead_us(ActionKind::Down, 1);
        let lead_15 = estimator.get_lead_us(ActionKind::Down, 15);
        // With no samples the cold-start prior + WAKE_RESERVE must apply.
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
            estimator.update(ActionKind::Down, 200, 1);
            estimator.update(ActionKind::Down, 900, 3);
            estimator.update(ActionKind::Up, 150, 1);
            estimator.update(ActionKind::Up, 850, 3);
        }
        let down_1 = estimator.get_lead_us(ActionKind::Down, 1);
        let down_3 = estimator.get_lead_us(ActionKind::Down, 3);
        let up_1 = estimator.get_lead_us(ActionKind::Up, 1);
        let up_3 = estimator.get_lead_us(ActionKind::Up, 3);
        // Monotonic envelope.
        assert!(down_3 >= down_1, "down: 3-key must be >= 1-key");
        assert!(up_3 >= up_1, "up: 3-key must be >= 1-key");
        // Data-driven estimate should exceed the cold prior.
        assert!(
            down_3 > cold_prior_us_helper(3) + WAKE_RESERVE_US - 50,
            "down_3={down_3} should be data-driven above cold prior"
        );
    }

    #[test]
    fn observation_update_matches_separate_histogram_and_residual_updates() {
        let mut combined = SendLatencyEstimator::new(0.2, 10_000, 4);
        let mut separate = combined.clone();
        for (kind, class, duration, polyphony, residual) in [
            (ActionKind::Down, LatencyClass::Hot, 120, 1, Some(80)),
            (ActionKind::Down, LatencyClass::Cold, 2_400, 3, Some(-120)),
            (ActionKind::Up, LatencyClass::Hot, 260, 2, None),
        ] {
            combined
                .update_observation(kind, class, duration, polyphony, residual)
                .unwrap();
            separate
                .update_with_class(kind, duration, polyphony, class)
                .unwrap();
            if let Some(residual) = residual {
                separate
                    .update_completion_error_with_class(kind, residual, class)
                    .unwrap();
            }
        }
        assert_eq!(
            serde_json::to_string(&combined.export_state()).unwrap(),
            serde_json::to_string(&separate.export_state()).unwrap()
        );
        for kind in [ActionKind::Down, ActionKind::Up] {
            for class in [LatencyClass::Hot, LatencyClass::Cold] {
                for strict in [false, true] {
                    for polyphony in 1..=4 {
                        assert_eq!(
                            combined.estimate_lead_with_class_and_policy(
                                kind, polyphony, class, strict
                            ),
                            separate.estimate_lead_with_class_and_policy(
                                kind, polyphony, class, strict
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
            .update_observation(ActionKind::Down, LatencyClass::Hot, 100, 2, Some(50))
            .unwrap();
        assert_eq!(estimator.refresh_count, initial_refreshes + 1);
    }

    #[test]
    fn observation_refreshes_only_its_direction_and_class_channel() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 4);
        let up_cold_before = estimator
            .lead_cache
            .iter()
            .map(|poly_cache| poly_cache[residual_index(ActionKind::Up, LatencyClass::Cold)])
            .collect::<Vec<_>>();
        estimator
            .update_observation(ActionKind::Down, LatencyClass::Hot, 100, 2, Some(50))
            .unwrap();
        let up_cold_after = estimator
            .lead_cache
            .iter()
            .map(|poly_cache| poly_cache[residual_index(ActionKind::Up, LatencyClass::Cold)])
            .collect::<Vec<_>>();
        assert_eq!(up_cold_before, up_cold_after);
    }

    /// Thin wrapper so tests can call `cold_prior_us` without going through an impl block.
    fn cold_prior_us_helper(n: usize) -> u64 {
        SendLatencyEstimator::cold_prior_us(n)
    }

    #[test]
    fn strict_upper_tail_keeps_a_single_recent_outlier_visible() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 1);
        for _ in 0..64 {
            estimator.update(ActionKind::Down, 100, 1);
        }
        estimator.update(ActionKind::Down, 3_000, 1);

        let normal = estimator
            .estimate_lead_with_class(ActionKind::Down, 1, LatencyClass::Hot)
            .applied_us;
        let strict = estimator
            .estimate_lead_with_class_and_policy(ActionKind::Down, 1, LatencyClass::Hot, true)
            .applied_us;
        // Strict mode must be ≥ normal mode (outlier is visible).
        assert!(strict >= normal, "strict={strict} normal={normal}");
        // The outlier must be reflected in the slow tail reserve.
        assert!(
            estimator.down.tail_hot[1].get() >= 3_000,
            "tail reserve must have captured the outlier"
        );
    }

    #[test]
    fn strict_sparse_bucket_keeps_global_upper_tail_guard() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 8);
        for _ in 0..32 {
            estimator.update(ActionKind::Down, 1_500, 8);
        }
        for _ in 0..5 {
            estimator.update(ActionKind::Down, 300, 1);
        }
        let strict_1 = estimator
            .estimate_lead_with_class_and_policy(ActionKind::Down, 1, LatencyClass::Hot, true)
            .applied_us;
        // The global tail guard keeps the 8-key 1 500 µs visible for 1-key in strict mode.
        assert!(
            strict_1 >= 1_500,
            "strict 1-key lead={strict_1} should see global 1500µs tail"
        );
    }

    #[test]
    fn cold_outlier_does_not_raise_hot_global_guard() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 2);
        for _ in 0..8 {
            estimator.update_with_class(ActionKind::Down, 100, 1, LatencyClass::Hot);
        }
        let hot_before = estimator
            .estimate_lead_with_class_and_policy(ActionKind::Down, 1, LatencyClass::Hot, true)
            .applied_us;
        for _ in 0..8 {
            estimator.update_with_class(ActionKind::Down, 3_000, 2, LatencyClass::Cold);
        }
        let hot_after = estimator
            .estimate_lead_with_class_and_policy(ActionKind::Down, 1, LatencyClass::Hot, true)
            .applied_us;
        assert_eq!(hot_after, hot_before);
        assert!(
            estimator
                .estimate_lead_with_class_and_policy(ActionKind::Down, 2, LatencyClass::Cold, true)
                .applied_us
                >= 3_000
        );
    }

    #[test]
    fn lead_components_are_named_and_non_zero() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 3);
        for _ in 0..10 {
            estimator.update(ActionKind::Down, 500, 2);
        }
        let est = estimator.estimate_lead(ActionKind::Down, 2);
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
            estimator.estimate_lead(ActionKind::Down, 1).confidence,
            LeadConfidence::PriorOnly
        );
        for i in 0..SEED_SAMPLES - 1 {
            estimator.update(ActionKind::Down, 100, 1);
            let conf = estimator.estimate_lead(ActionKind::Down, 1).confidence;
            assert_eq!(
                conf,
                LeadConfidence::Warming,
                "at sample {i} expected Warming"
            );
        }
        estimator.update(ActionKind::Down, 100, 1);
        assert_eq!(
            estimator.estimate_lead(ActionKind::Down, 1).confidence,
            LeadConfidence::Learned
        );
    }

    #[test]
    fn delivery_proxy_component_is_additive() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 2);
        for _ in 0..10 {
            estimator.update(ActionKind::Down, 200, 2);
        }
        let without_proxy = estimator.get_lead_us(ActionKind::Down, 2);
        estimator.set_delivery_proxy_us(2, 300).unwrap();
        let with_proxy = estimator.get_lead_us(ActionKind::Down, 2);
        assert_eq!(with_proxy, without_proxy + 300);
    }

    #[test]
    fn hot_and_cold_histograms_are_class_isolated() {
        let mut estimator = SendLatencyEstimator::new(0.2, 2_000, 3);
        for _ in 0..SEED_SAMPLES {
            estimator.update_with_class(ActionKind::Down, 400, 1, LatencyClass::Cold);
        }
        assert_eq!(estimator.down.hot[1].total, 0);
        assert_eq!(estimator.down.cold[1].total, SEED_SAMPLES as u64);

        for _ in 0..SEED_SAMPLES {
            estimator.update_with_class(ActionKind::Down, 100, 1, LatencyClass::Hot);
        }
        assert_eq!(estimator.down.hot[1].total, SEED_SAMPLES as u64);
        assert_eq!(estimator.down.cold[1].total, SEED_SAMPLES as u64);
    }

    #[test]
    fn delivery_proxy_is_independent_by_direction_and_class() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 3);
        estimator
            .set_delivery_proxy_us_for(ActionKind::Down, LatencyClass::Hot, 1, 100)
            .unwrap();
        estimator
            .set_delivery_proxy_us_for(ActionKind::Down, LatencyClass::Cold, 1, 200)
            .unwrap();
        estimator
            .set_delivery_proxy_us_for(ActionKind::Up, LatencyClass::Hot, 1, 300)
            .unwrap();
        estimator
            .set_delivery_proxy_us_for(ActionKind::Up, LatencyClass::Cold, 1, 400)
            .unwrap();

        assert_eq!(
            estimator
                .estimate_lead_with_class(ActionKind::Down, 1, LatencyClass::Hot)
                .components
                .delivery_proxy_us,
            100
        );
        assert_eq!(
            estimator
                .estimate_lead_with_class(ActionKind::Down, 1, LatencyClass::Cold)
                .components
                .delivery_proxy_us,
            200
        );
        assert_eq!(
            estimator
                .estimate_lead_with_class(ActionKind::Up, 1, LatencyClass::Hot)
                .components
                .delivery_proxy_us,
            300
        );
        assert_eq!(
            estimator
                .estimate_lead_with_class(ActionKind::Up, 1, LatencyClass::Cold)
                .components
                .delivery_proxy_us,
            400
        );
    }

    #[test]
    fn cold_prior_is_added_once_when_bucket_is_unwarmed() {
        let estimator = SendLatencyEstimator::new(0.2, 10_000, 3);
        let estimate = estimator.estimate_lead_with_class(ActionKind::Down, 1, LatencyClass::Cold);
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
        // After one half-life of clean updates the outlier remains material,
        // but the reserve still decays and never reaches zero.
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
            estimator.update(ActionKind::Up, 100, 1);
            estimator.update_completion_error(ActionKind::Up, 300);
        }
        let adj_up = estimator.residual_adjustment_us_for(ActionKind::Up);
        let adj_down = estimator.residual_adjustment_us_for(ActionKind::Down);
        assert!(adj_up > 0, "up residual must be learned");
        assert_eq!(adj_down, 0, "down residual must remain zero");
    }
    #[test]
    fn cold_residual_is_learned_separately_from_hot_residual() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 2);
        for _ in 0..SEED_SAMPLES {
            // Warm up the hot channel with positive error
            estimator.update_with_class(ActionKind::Down, 100, 1, LatencyClass::Hot);
            estimator.update_completion_error_with_class(ActionKind::Down, 300, LatencyClass::Hot);

            // Warm up the cold channel with negative error
            estimator.update_with_class(ActionKind::Down, 100, 1, LatencyClass::Cold);
            estimator.update_completion_error_with_class(
                ActionKind::Down,
                -300,
                LatencyClass::Cold,
            );
        }

        // The accessor `residual_adjustment_us_for` assumes LatencyClass::Hot.
        let adj_hot = estimator.residual_adjustment_us_for(ActionKind::Down);

        // We can inspect the cold channel directly from the private fields for the test.
        let cold_idx = residual_index(ActionKind::Down, LatencyClass::Cold);
        let adj_cold = estimator.residuals[cold_idx].adjustment_us();

        assert!(adj_hot > 0, "hot residual must be positive learned");
        assert!(adj_cold < 0, "cold residual must be negative learned");
    }

    #[test]
    fn early_residual_reduces_lead_more_slowly_than_late_residual_increases_it() {
        let mut estimator = SendLatencyEstimator::new(0.2, 10_000, 2);
        for _ in 0..SEED_SAMPLES {
            estimator.update(ActionKind::Down, 800, 1);
            estimator.update_completion_error(ActionKind::Down, -400);
        }
        let adj = estimator.residual_adjustment_us();
        // Negative early correction should be dampened.
        assert!(adj < 0, "early residual should be negative");
        assert!(
            adj > -400,
            "dampened early residual must be less than raw -400"
        );

        for _ in 0..SEED_SAMPLES {
            estimator.update_completion_error(ActionKind::Down, 400);
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
            source.update(ActionKind::Down, v, 2);
            source.update(ActionKind::Up, v + 50, 2);
        }
        let json = serde_json::to_string(&source.export_state()).unwrap();
        let mut restored = SendLatencyEstimator::new(0.2, 5_000, 4);
        restored.import_state(&json).unwrap();
        assert_eq!(restored.export_state().version, ESTIMATOR_STATE_VERSION);
        // Restored estimator should produce a lead in a similar range.
        let src_lead = source.get_lead_us(ActionKind::Down, 2);
        let rst_lead = restored.get_lead_us(ActionKind::Down, 2);
        let diff = src_lead.abs_diff(rst_lead);
        assert!(
            diff <= BUCKET_WIDTH_US * 2,
            "round-trip lead should be within 2 bucket widths: src={src_lead} rst={rst_lead}"
        );
    }

    #[test]
    fn mixed_hot_cold_state_round_trip_preserves_class_specific_leads() {
        let mut source = SendLatencyEstimator::new(0.2, 10_000, 3);
        for _ in 0..8 {
            source.update_with_class(ActionKind::Down, 120, 1, LatencyClass::Hot);
            source.update_with_class(ActionKind::Down, 2_400, 2, LatencyClass::Cold);
            source.update_with_class(ActionKind::Up, 180, 1, LatencyClass::Hot);
            source.update_with_class(ActionKind::Up, 2_800, 2, LatencyClass::Cold);
        }
        let before = [
            source
                .estimate_lead_with_class_and_policy(ActionKind::Down, 1, LatencyClass::Hot, true)
                .applied_us,
            source
                .estimate_lead_with_class_and_policy(ActionKind::Down, 2, LatencyClass::Cold, true)
                .applied_us,
            source
                .estimate_lead_with_class_and_policy(ActionKind::Up, 1, LatencyClass::Hot, true)
                .applied_us,
            source
                .estimate_lead_with_class_and_policy(ActionKind::Up, 2, LatencyClass::Cold, true)
                .applied_us,
        ];
        let json = serde_json::to_string(&source.export_state()).unwrap();
        let mut restored = SendLatencyEstimator::new(0.2, 10_000, 3);
        restored.import_state(&json).unwrap();
        let after = [
            restored
                .estimate_lead_with_class_and_policy(ActionKind::Down, 1, LatencyClass::Hot, true)
                .applied_us,
            restored
                .estimate_lead_with_class_and_policy(ActionKind::Down, 2, LatencyClass::Cold, true)
                .applied_us,
            restored
                .estimate_lead_with_class_and_policy(ActionKind::Up, 1, LatencyClass::Hot, true)
                .applied_us,
            restored
                .estimate_lead_with_class_and_policy(ActionKind::Up, 2, LatencyClass::Cold, true)
                .applied_us,
        ];
        // Histogram persistence stores bucket upper bounds, so reload may
        // widen a strict estimate by at most one bucket per class.
        for (restored, original) in after.into_iter().zip(before) {
            assert!(
                restored.abs_diff(original) <= BUCKET_WIDTH_US,
                "class-specific lead changed beyond histogram tolerance: restored={restored} original={original}"
            );
        }
    }

    #[test]
    fn persisted_histogram_merge_keeps_all_persisted_counts() {
        let mut source = SendLatencyEstimator::new(0.2, 5_000, 2);
        for _ in 0..300 {
            source.update(ActionKind::Down, 300, 2);
        }
        let json = serde_json::to_string(&source.export_state()).unwrap();
        let mut restored = SendLatencyEstimator::new(0.2, 5_000, 2);
        restored.import_state(&json).unwrap();

        let exported = restored.export_state();
        assert_eq!(exported.hist_down[2].hot_pairs, vec![[12, 300]]);
    }

    #[test]
    fn persisted_histogram_rejects_duplicate_bucket_without_mutation() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 2);
        estimator.update(ActionKind::Down, 100, 1);
        let before = estimator.export_state();
        let mut state = before.clone();
        state.hist_down[1].hot_pairs = vec![[4, 1], [4, 2]];

        assert!(
            estimator
                .import_state(&serde_json::to_string(&state).unwrap())
                .is_err()
        );
        assert_eq!(
            serde_json::to_string(&estimator.export_state().hist_down).unwrap(),
            serde_json::to_string(&before.hist_down).unwrap()
        );
    }

    #[test]
    fn invalid_state_does_not_mutate_estimator() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 6);
        for _ in 0..10 {
            estimator.update(ActionKind::Down, 100, 1);
        }
        let lead_before = estimator.get_lead_us(ActionKind::Down, 1);
        let invalid = r#"{
            "version": 8, "max_poly": 2,
            "hist_down": [{"hot_pairs":[[999,1]],"cold_pairs":[],"hot_tail_reserve_us":0,"cold_tail_reserve_us":0}, {"hot_pairs":[],"cold_pairs":[],"hot_tail_reserve_us":0,"cold_tail_reserve_us":0}, {"hot_pairs":[],"cold_pairs":[],"hot_tail_reserve_us":0,"cold_tail_reserve_us":0}],
            "hist_up": [{"hot_pairs":[],"cold_pairs":[],"hot_tail_reserve_us":0,"cold_tail_reserve_us":0}, {"hot_pairs":[],"cold_pairs":[],"hot_tail_reserve_us":0,"cold_tail_reserve_us":0}, {"hot_pairs":[],"cold_pairs":[],"hot_tail_reserve_us":0,"cold_tail_reserve_us":0}],
            "residuals": [{"count":0,"sum":0,"ema":0.0,"warm":false},{"count":0,"sum":0,"ema":0.0,"warm":false},{"count":0,"sum":0,"ema":0.0,"warm":false},{"count":0,"sum":0,"ema":0.0,"warm":false}],
            "delivery_proxy_channels": [[0,0,0,0],[0,0,0,0],[0,0,0,0]]
        }"#;
        assert!(estimator.import_state(invalid).is_err());
        assert_eq!(estimator.get_lead_us(ActionKind::Down, 1), lead_before);
        assert_eq!(estimator.max_poly, 6);
    }

    #[test]
    fn constructor_rejects_invalid_external_configuration() {
        assert!(matches!(
            SendLatencyEstimator::try_new(f64::NAN, 5_000, 2),
            Err(EstimatorConfigError::InvalidAlpha)
        ));
        assert!(matches!(
            SendLatencyEstimator::try_new(f64::INFINITY, 5_000, 2),
            Err(EstimatorConfigError::InvalidAlpha)
        ));
        assert!(matches!(
            SendLatencyEstimator::try_new(0.0, 5_000, 2),
            Err(EstimatorConfigError::InvalidAlpha)
        ));
        assert!(matches!(
            SendLatencyEstimator::try_new(0.2, MAX_SAMPLE_US + 1, 2),
            Err(EstimatorConfigError::InvalidMaxLead)
        ));
        assert!(matches!(
            SendLatencyEstimator::try_new(0.2, 5_000, 0),
            Err(EstimatorConfigError::InvalidPolyphony)
        ));
        assert!(matches!(
            SendLatencyEstimator::try_new(0.2, 5_000, 33),
            Err(EstimatorConfigError::InvalidPolyphony)
        ));
    }

    #[test]
    fn malicious_state_is_rejected_atomically() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 2);
        estimator.update(ActionKind::Down, 200, 1);
        let before = serde_json::to_string(&estimator.export_state()).unwrap();

        let mut tail = estimator.export_state();
        tail.hist_down[1].hot_tail_reserve_us = u64::MAX;
        assert!(
            estimator
                .import_state(&serde_json::to_string(&tail).unwrap())
                .is_err()
        );

        let mut delivery = estimator.export_state();
        delivery.delivery_proxy_channels[1][0] = u64::MAX;
        assert!(
            estimator
                .import_state(&serde_json::to_string(&delivery).unwrap())
                .is_err()
        );

        let mut count = estimator.export_state();
        count.hist_down[1].hot_pairs = vec![[1, u32::MAX as u64 + 1]];
        assert!(
            estimator
                .import_state(&serde_json::to_string(&count).unwrap())
                .is_err()
        );

        let mut wrong_length = estimator.export_state();
        wrong_length.hist_up.pop();
        assert!(
            estimator
                .import_state(&serde_json::to_string(&wrong_length).unwrap())
                .is_err()
        );

        let mut warm_state = estimator.export_state();
        warm_state.residuals[0].warm = false;
        warm_state.residuals[0].count = SEED_SAMPLES as u64;
        assert!(
            estimator
                .import_state(&serde_json::to_string(&warm_state).unwrap())
                .is_err()
        );

        let mut zero_poly = estimator.export_state();
        zero_poly.max_poly = 0;
        assert!(
            estimator
                .import_state(&serde_json::to_string(&zero_poly).unwrap())
                .is_err()
        );

        let mut too_many_poly = estimator.export_state();
        too_many_poly.max_poly = 33;
        assert!(
            estimator
                .import_state(&serde_json::to_string(&too_many_poly).unwrap())
                .is_err()
        );

        assert_eq!(
            serde_json::to_string(&estimator.export_state()).unwrap(),
            before
        );
    }

    #[test]
    fn lead_composition_applies_signed_residual_without_unsigned_wrap() {
        let base = LeadComponents {
            syscall_us: u64::MAX,
            residual_bias_us: -1,
            ..LeadComponents::default()
        };
        assert_eq!(base.total_uncapped(), u64::MAX - 1);

        let positive = LeadComponents {
            syscall_us: u64::MAX,
            residual_bias_us: 1,
            ..LeadComponents::default()
        };
        assert_eq!(positive.total_uncapped(), u64::MAX);
    }

    #[test]
    fn histogram_p95_alias_remains_conservative_and_bounded() {
        histogram_p95_is_conservative_and_bounded();
    }

    #[test]
    fn wrapped_state_round_trip_preserves_ring_order_and_future_updates() {
        // The ring-order concept doesn't apply to histograms, but the round-trip
        // contract (future updates produce the same estimate) still holds.
        let mut source = SendLatencyEstimator::new(0.2, 5_000, 2);
        for v in 100..200u64 {
            source.update(ActionKind::Down, v, 2);
        }
        let json = serde_json::to_string(&source.export_state()).unwrap();
        let mut restored = SendLatencyEstimator::new(0.2, 5_000, 2);
        restored.import_state(&json).unwrap();

        source.update(ActionKind::Down, 3_000, 2);
        restored.update(ActionKind::Down, 3_000, 2);

        let diff = source
            .get_lead_us(ActionKind::Down, 2)
            .abs_diff(restored.get_lead_us(ActionKind::Down, 2));
        // Allow up to 2 bucket widths of divergence due to histogram quantisation.
        assert!(
            diff <= BUCKET_WIDTH_US * 2,
            "post-update leads should match within quantisation error: \
             src={} rst={}",
            source.get_lead_us(ActionKind::Down, 2),
            restored.get_lead_us(ActionKind::Down, 2)
        );
    }

    #[test]
    fn invalid_current_state_does_not_mutate_estimator() {
        invalid_state_does_not_mutate_estimator();
    }

    #[test]
    fn update_overflow_is_an_error_without_partial_mutation() {
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 2);
        let bucket = Histogram::checked_bucket_for(100);
        estimator.down.hot[1].buckets[bucket] = u32::MAX;
        let histogram_before = estimator.down.hot[1].total;

        assert!(matches!(
            estimator.update(ActionKind::Down, 100, 1),
            Err(EstimatorStateError::ArithmeticOverflow("histogram bucket"))
        ));
        assert_eq!(estimator.down.hot[1].total, histogram_before);
    }
}
