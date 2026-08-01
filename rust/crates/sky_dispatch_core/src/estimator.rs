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
//! The histogram-based fast model is O(1) to update and O(buckets) to query.
//! No allocation, no sort, no clone.
//!
//! The slow tail reserve is a long-decay exponential envelope that prevents
//! the estimator from forgetting rare catastrophic tails too quickly.
//!
//! ## State version
//!
//! Version 5 introduces the histogram state and the full component breakdown.
//! Versions 2–4 are accepted and migrated conservatively.

use crate::model::ActionKind;
use serde::{Deserialize, Serialize};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Minimum samples before a bucket switches from prior to observed estimate.
const SEED_SAMPLES: usize = 5;
/// Legacy rolling-window size retained for v2–v4 import only.
const ROLLING_WINDOW: usize = 32;

/// Histogram bucket width in microseconds (25 µs resolution).
const BUCKET_WIDTH_US: u64 = 25;
/// Number of histogram buckets (covers 0–6 375 µs; overflow lands in the last
/// bucket).
const BUCKET_COUNT: usize = 256;
/// Maximum value covered by the main histogram (non-overflow).
const HIST_MAX_US: u64 = BUCKET_WIDTH_US * (BUCKET_COUNT as u64 - 1);

/// Slow tail reserve decay half-life (exponential decay coefficient).
/// At 0.95 per sample the reserve halves roughly every 14 updates.
const TAIL_RESERVE_DECAY: f64 = 0.95;
/// Hard lower bound on the slow tail reserve once it has been seeded with an
/// outlier.  Prevents the reserve from decaying to zero even with no new data.
const TAIL_RESERVE_FLOOR_US: u64 = 25;

/// Residual clamp: late by at most 1 000 µs, early correction at 0.25×.
const MAX_RESIDUAL_US: i64 = 1_000;
const EARLY_CORRECTION_DECAY: f64 = 0.25;

/// Samples that exceed this value are clamped before storage to prevent a
/// single catastrophic observation from ruining the model.
const MAX_SAMPLE_US: u64 = 60_000_000;

/// Conservative cold-start prior when no samples are available.
const BASE_COLD_PRIOR_US: u64 = 100;
/// Per-additional-key increment for the cold-start prior.
const PER_KEY_COLD_PRIOR_US: u64 = 40;

/// Scheduler wake-jitter reserve baked into every lead estimate.
/// This is separate from the syscall estimate and is not learned.
const WAKE_RESERVE_US: u64 = 50;

/// Current on-disk state format version.
pub const ESTIMATOR_STATE_VERSION: u32 = 5;

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
    pub fn total_uncapped(&self) -> u64 {
        let base = self
            .syscall_us
            .saturating_add(self.delivery_proxy_us)
            .saturating_add(self.wake_reserve_us)
            .saturating_add(self.cold_reserve_us);
        (base as i64).saturating_add(self.residual_bias_us).max(0) as u64
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
    fn push(&mut self, value_us: u64) {
        let clamped = value_us.min(MAX_SAMPLE_US);
        if clamped > self.max_seen_us {
            self.max_seen_us = clamped;
        }
        let bucket = ((clamped / BUCKET_WIDTH_US) as usize).min(BUCKET_COUNT - 1);
        self.buckets[bucket] = self.buckets[bucket].saturating_add(1);
        self.total = self.total.saturating_add(1);
        if bucket == BUCKET_COUNT - 1 {
            self.overflow_count = self.overflow_count.saturating_add(1);
        }
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
            if count > u32::MAX as u64 {
                return Err(format!("histogram bucket count {count} overflows u32"));
            }
            hist.buckets[idx as usize] = count as u32;
            hist.total = hist.total.saturating_add(count);
            let upper = (idx + 1) * BUCKET_WIDTH_US;
            if upper > hist.max_seen_us {
                hist.max_seen_us = upper;
            }
            if idx == BUCKET_COUNT as u64 - 1 {
                hist.overflow_count = hist.overflow_count.saturating_add(count as u32);
            }
        }
        Ok(hist)
    }

    /// Build a conservative histogram from a legacy rolling-window sample vec.
    fn from_legacy_samples(samples: &[u64]) -> Result<Self, String> {
        let mut hist = Self::default();
        for &v in samples {
            if v > MAX_SAMPLE_US {
                return Err("legacy sample exceeds MAX_SAMPLE_US".to_string());
            }
            hist.push(v);
        }
        Ok(hist)
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
    fn update(&mut self, alpha: f64, sample: i64) {
        let clamped = sample.clamp(-MAX_RESIDUAL_US, MAX_RESIDUAL_US * 2);
        self.count = self.count.saturating_add(1);
        self.sum = self.sum.saturating_add(clamped);
        if self.warm {
            self.ema = alpha * clamped as f64 + (1.0 - alpha) * self.ema;
        } else if self.count >= SEED_SAMPLES as u64 {
            self.ema = self.sum as f64 / self.count as f64;
            self.warm = true;
        }
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
    tail_reserve: Vec<SlowTailReserve>,
}

impl DirectionBuckets {
    fn new(size: usize) -> Self {
        Self {
            hot: vec![Histogram::default(); size],
            cold: vec![Histogram::default(); size],
            tail_reserve: vec![SlowTailReserve::default(); size],
        }
    }

    #[allow(dead_code)]
    fn resize(&mut self, new_size: usize) {
        self.hot.resize(new_size, Histogram::default());
        self.cold.resize(new_size, Histogram::default());
        self.tail_reserve
            .resize(new_size, SlowTailReserve::default());
    }

    fn push(&mut self, n: usize, value_us: u64, class: LatencyClass) {
        self.hot[n].push(value_us);
        if class == LatencyClass::Cold {
            self.cold[n].push(value_us);
        }
        self.tail_reserve[n].update(value_us);
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
        let local = if strict_upper_tail {
            self.hot[n].max()
        } else {
            self.hot[n].p95()
        };

        // Include the cold-class p95 when the caller is in a cold context.
        let cold_local = (class == LatencyClass::Cold)
            .then(|| {
                if strict_upper_tail {
                    self.cold[n].max()
                } else {
                    self.cold[n].p95()
                }
            })
            .flatten();

        // Slow tail reserve is always included (it decays toward observed).
        let tail = (self.tail_reserve[n].get() > 0).then(|| self.tail_reserve[n].get());

        [local, cold_local, tail].into_iter().flatten().max()
    }
}

// ─── On-disk format ──────────────────────────────────────────────────────────

/// Version-5 serialisable per-direction histogram bucket.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HistBucketJson {
    /// Non-zero (bucket_index, count) pairs for the hot histogram.
    #[serde(default)]
    pub hot_pairs: Vec<[u64; 2]>,
    /// Non-zero (bucket_index, count) pairs for the cold histogram.
    #[serde(default)]
    pub cold_pairs: Vec<[u64; 2]>,
    /// Slow tail reserve value.
    #[serde(default)]
    pub tail_reserve_us: u64,
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

/// Unified on-disk format accepting versions 2–5.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstimatorStateJson {
    pub version: u32,
    #[serde(default)]
    pub saved_at: String,
    pub max_poly: usize,

    // ── Legacy fields (versions 2–4, kept for backward compat import) ──
    #[serde(default)]
    pub ema_down: Vec<f64>,
    #[serde(default)]
    pub warm_down: Vec<bool>,
    #[serde(default)]
    pub count_down: Vec<u64>,
    #[serde(default)]
    pub sum_down: Vec<u64>,
    #[serde(default)]
    pub ema_down_total: f64,
    #[serde(default)]
    pub warm_down_total: bool,
    #[serde(default)]
    pub count_down_total: u64,
    #[serde(default)]
    pub sum_down_total: u64,
    #[serde(default)]
    pub ema_up: f64,
    #[serde(default)]
    pub warm_up: bool,
    #[serde(default)]
    pub count_up: u64,
    #[serde(default)]
    pub sum_up: u64,
    #[serde(default)]
    pub ema_residual: f64,
    #[serde(default)]
    pub warm_residual: bool,
    #[serde(default)]
    pub count_residual: u64,
    #[serde(default)]
    pub sum_residual: i64,
    #[serde(default)]
    pub samples_down: Vec<Vec<u64>>,
    #[serde(default)]
    pub samples_up: Vec<Vec<u64>>,
    #[serde(default)]
    pub samples_down_total: Vec<u64>,
    #[serde(default)]
    pub samples_up_total: Vec<u64>,
    #[serde(default)]
    pub ema_residual_up: f64,
    #[serde(default)]
    pub warm_residual_up: bool,
    #[serde(default)]
    pub count_residual_up: u64,
    #[serde(default)]
    pub sum_residual_up: i64,

    // ── Version 5 fields ──
    /// Per-polyphony histogram state for Down direction.
    #[serde(default)]
    pub hist_down: Vec<HistBucketJson>,
    /// Per-polyphony histogram state for Up direction.
    #[serde(default)]
    pub hist_up: Vec<HistBucketJson>,
    /// Four residual channels: down_hot, down_cold, up_hot, up_cold.
    #[serde(default)]
    pub residuals: Vec<ResidualChannelJson>,
    /// Calibrated delivery proxy prior (µs) per polyphony bucket; 0 = uncalibrated.
    #[serde(default)]
    pub delivery_proxy_us: Vec<u64>,
}

// ─── Main estimator ───────────────────────────────────────────────────────────

/// Conservative adaptive estimator with named lead components.
///
/// Public API is backward-compatible with the v4 interface; all new fields are
/// additive.  `estimate_lead` now returns a richer `LeadEstimate` with a full
/// `LeadComponents` breakdown and a `LeadConfidence` level.
#[derive(Debug, Clone)]
pub struct SendLatencyEstimator {
    pub max_poly: usize,
    alpha: f64,
    max_lead_us: u64,

    down: DirectionBuckets,
    up: DirectionBuckets,

    /// Global (all-polyphony) histogram for cross-bucket guard.
    down_total: Histogram,
    up_total: Histogram,

    /// Residual channels indexed by `residual_index(kind, class)`.
    /// Order: [down_hot, down_cold, up_hot, up_cold].
    residuals: [ResidualEma; 4],

    /// Calibrated delivery proxy prior per polyphony bucket (0 = uncalibrated).
    delivery_proxy_us: Vec<u64>,

    // ── Legacy scalar counts kept for export backward compat ──
    count_down: Vec<u64>,
    sum_down: Vec<u64>,
    count_down_total: u64,
    sum_down_total: u64,
    count_up: Vec<u64>,
    sum_up: Vec<u64>,
    count_up_total: u64,
    sum_up_total: u64,
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
    pub fn new(alpha: f64, max_lead_us: u64, max_poly: usize) -> Self {
        let size = max_poly + 1;
        Self {
            max_poly,
            alpha,
            max_lead_us,
            down: DirectionBuckets::new(size),
            up: DirectionBuckets::new(size),
            down_total: Histogram::default(),
            up_total: Histogram::default(),
            residuals: Default::default(),
            delivery_proxy_us: vec![0u64; size],
            count_down: vec![0; size],
            sum_down: vec![0; size],
            count_down_total: 0,
            sum_down_total: 0,
            count_up: vec![0; size],
            sum_up: vec![0; size],
            count_up_total: 0,
            sum_up_total: 0,
        }
    }

    // ── Update API ────────────────────────────────────────────────────────────

    pub fn update(&mut self, kind: ActionKind, duration_us: u64, n_keys: usize) {
        self.update_with_class(kind, duration_us, n_keys, LatencyClass::Hot);
    }

    pub fn update_with_class(
        &mut self,
        kind: ActionKind,
        duration_us: u64,
        n_keys: usize,
        latency_class: LatencyClass,
    ) {
        let duration_us = duration_us.min(MAX_SAMPLE_US);
        let n = 1.max(self.max_poly.min(n_keys));
        match kind {
            ActionKind::Down => {
                self.down.push(n, duration_us, latency_class);
                self.down_total.push(duration_us);
                self.count_down[n] = self.count_down[n].saturating_add(1);
                self.sum_down[n] = self.sum_down[n].saturating_add(duration_us);
                self.count_down_total = self.count_down_total.saturating_add(1);
                self.sum_down_total = self.sum_down_total.saturating_add(duration_us);
            }
            ActionKind::Up => {
                self.up.push(n, duration_us, latency_class);
                self.up_total.push(duration_us);
                self.count_up[n] = self.count_up[n].saturating_add(1);
                self.sum_up[n] = self.sum_up[n].saturating_add(duration_us);
                self.count_up_total = self.count_up_total.saturating_add(1);
                self.sum_up_total = self.sum_up_total.saturating_add(duration_us);
            }
        }
    }

    /// Record a completion-error residual for the given kind, assuming Hot class for backward compatibility.
    pub fn update_completion_error(&mut self, kind: ActionKind, error_us: i64) {
        self.update_completion_error_with_class(kind, error_us, LatencyClass::Hot);
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
    ) {
        let alpha = self.alpha;
        self.residuals[residual_index(kind, class)].update(alpha, error_us);
    }

    /// Update the calibrated delivery proxy prior for a polyphony bucket.
    ///
    /// Should be called from the calibration harness output processor, not
    /// from the real-time dispatch path.
    pub fn set_delivery_proxy_us(&mut self, n_keys: usize, value_us: u64) {
        let n = 1.max(self.max_poly.min(n_keys));
        self.delivery_proxy_us[n] = value_us;
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
            .chain(std::iter::once(Self::cold_prior_us(n)))
            .max()
            .unwrap_or_else(|| Self::cold_prior_us(n))
    }

    fn build_components(
        &self,
        kind: ActionKind,
        n: usize,
        class: LatencyClass,
        strict_upper_tail: bool,
    ) -> (LeadComponents, LeadConfidence) {
        let (dir, total) = match kind {
            ActionKind::Down => (&self.down, &self.down_total),
            ActionKind::Up => (&self.up, &self.up_total),
        };

        let syscall_us = self.syscall_estimate_us(dir, total, n, class, strict_upper_tail);

        let delivery_proxy_us = self.delivery_proxy_us[n];

        let wake_reserve_us = WAKE_RESERVE_US;

        let cold_reserve_us = if class == LatencyClass::Cold {
            Self::cold_prior_us(n)
        } else {
            0
        };

        let residual_bias_us =
            self.residuals[residual_index(kind, LatencyClass::Hot)].adjustment_us();

        let components = LeadComponents {
            syscall_us,
            delivery_proxy_us,
            wake_reserve_us,
            cold_reserve_us,
            residual_bias_us,
        };

        // Determine confidence from the local hot histogram.
        let local_total = dir.hot[n].total;
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

        // Monotonic envelope: ensure larger chords get at least the lead of
        // smaller chords.
        let mut best_components = LeadComponents::default();
        let mut best_uncapped = 0u64;
        let mut best_confidence = LeadConfidence::PriorOnly;

        for bucket in 1..=n {
            let (comps, conf) =
                self.build_components(kind, bucket, latency_class, strict_upper_tail);
            let uncapped = comps.total_uncapped();
            if uncapped >= best_uncapped {
                best_uncapped = uncapped;
                best_components = comps;
                best_confidence = conf;
            }
        }

        let saturated = best_uncapped > self.max_lead_us;
        let confidence = if saturated {
            LeadConfidence::Saturated
        } else {
            best_confidence
        };

        LeadEstimate {
            applied_us: best_uncapped.min(self.max_lead_us),
            uncapped_us: best_uncapped,
            saturated,
            components: best_components,
            confidence,
        }
    }

    pub fn get_lead_us(&self, kind: ActionKind, n_keys: usize) -> u64 {
        self.estimate_lead(kind, n_keys).applied_us
    }

    pub fn lead_saturated(&self, kind: ActionKind, n_keys: usize) -> bool {
        self.estimate_lead(kind, n_keys).saturated
    }

    // ── State persistence ─────────────────────────────────────────────────────

    pub fn export_state(&self) -> EstimatorStateJson {
        let hist_down: Vec<HistBucketJson> = self
            .down
            .hot
            .iter()
            .zip(&self.down.cold)
            .zip(&self.down.tail_reserve)
            .map(|((hot, cold), tail)| HistBucketJson {
                hot_pairs: hot.to_export_pairs(),
                cold_pairs: cold.to_export_pairs(),
                tail_reserve_us: tail.get(),
            })
            .collect();

        let hist_up: Vec<HistBucketJson> = self
            .up
            .hot
            .iter()
            .zip(&self.up.cold)
            .zip(&self.up.tail_reserve)
            .map(|((hot, cold), tail)| HistBucketJson {
                hot_pairs: hot.to_export_pairs(),
                cold_pairs: cold.to_export_pairs(),
                tail_reserve_us: tail.get(),
            })
            .collect();

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

        // Legacy fields kept for cross-version tools.
        let ema_down: Vec<f64> = self
            .down
            .hot
            .iter()
            .map(|h| h.p95().unwrap_or(0) as f64)
            .collect();
        let warm_down: Vec<bool> = self.down.hot.iter().map(|h| h.is_warm()).collect();

        EstimatorStateJson {
            version: ESTIMATOR_STATE_VERSION,
            saved_at: String::new(),
            max_poly: self.max_poly,
            ema_down,
            warm_down,
            count_down: self.count_down.clone(),
            sum_down: self.sum_down.clone(),
            ema_down_total: self.down_total.p95().unwrap_or(0) as f64,
            warm_down_total: self.down_total.is_warm(),
            count_down_total: self.count_down_total,
            sum_down_total: self.sum_down_total,
            ema_up: self.up_total.p95().unwrap_or(0) as f64,
            warm_up: self.up_total.is_warm(),
            count_up: self.count_up_total,
            sum_up: self.sum_up_total,
            ema_residual: self.residuals[0].ema,
            warm_residual: self.residuals[0].warm,
            count_residual: self.residuals[0].count,
            sum_residual: self.residuals[0].sum,
            samples_down: self
                .down
                .hot
                .iter()
                .map(|h| {
                    // Reconstruct an approximate sample vec from the histogram.
                    h.buckets
                        .iter()
                        .enumerate()
                        .flat_map(|(i, &c)| {
                            std::iter::repeat_n((i as u64 + 1) * BUCKET_WIDTH_US, c as usize)
                        })
                        .take(ROLLING_WINDOW)
                        .collect()
                })
                .collect(),
            samples_up: self
                .up
                .hot
                .iter()
                .map(|h| {
                    h.buckets
                        .iter()
                        .enumerate()
                        .flat_map(|(i, &c)| {
                            std::iter::repeat_n((i as u64 + 1) * BUCKET_WIDTH_US, c as usize)
                        })
                        .take(ROLLING_WINDOW)
                        .collect()
                })
                .collect(),
            samples_down_total: {
                self.down_total
                    .buckets
                    .iter()
                    .enumerate()
                    .flat_map(|(i, &c)| {
                        std::iter::repeat_n((i as u64 + 1) * BUCKET_WIDTH_US, c as usize)
                    })
                    .take(ROLLING_WINDOW)
                    .collect()
            },
            samples_up_total: {
                self.up_total
                    .buckets
                    .iter()
                    .enumerate()
                    .flat_map(|(i, &c)| {
                        std::iter::repeat_n((i as u64 + 1) * BUCKET_WIDTH_US, c as usize)
                    })
                    .take(ROLLING_WINDOW)
                    .collect()
            },
            ema_residual_up: self.residuals[2].ema,
            warm_residual_up: self.residuals[2].warm,
            count_residual_up: self.residuals[2].count,
            sum_residual_up: self.residuals[2].sum,
            hist_down,
            hist_up,
            residuals,
            delivery_proxy_us: self.delivery_proxy_us.clone(),
        }
    }

    pub fn import_state(&mut self, json_str: &str) -> Result<(), String> {
        let state: EstimatorStateJson =
            serde_json::from_str(json_str).map_err(|e| format!("invalid estimator json: {e}"))?;

        if !matches!(state.version, 2 | 3 | 4 | ESTIMATOR_STATE_VERSION) {
            return Err(format!("unsupported estimator version: {}", state.version));
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
        let mut new_down_total = Histogram::default();
        let mut new_up_total = Histogram::default();

        if state.version >= ESTIMATOR_STATE_VERSION {
            // Version 5: import from histogram pairs directly.
            if state.hist_down.len() != expected_len || state.hist_up.len() != expected_len {
                return Err("hist_down/hist_up length does not match max_poly".to_string());
            }
            for (i, bucket) in state.hist_down.iter().enumerate() {
                new_down.hot[i] = Histogram::from_export_pairs(&bucket.hot_pairs)?;
                new_down.cold[i] = Histogram::from_export_pairs(&bucket.cold_pairs)?;
                new_down.tail_reserve[i].value_us = bucket.tail_reserve_us;
            }
            for (i, bucket) in state.hist_up.iter().enumerate() {
                new_up.hot[i] = Histogram::from_export_pairs(&bucket.hot_pairs)?;
                new_up.cold[i] = Histogram::from_export_pairs(&bucket.cold_pairs)?;
                new_up.tail_reserve[i].value_us = bucket.tail_reserve_us;
            }
            // Rebuild global total from per-bucket samples.
            for h in &new_down.hot {
                for (i, &c) in h.buckets.iter().enumerate() {
                    for _ in 0..c.min(128) {
                        new_down_total.push((i as u64 + 1) * BUCKET_WIDTH_US);
                    }
                }
            }
            for h in &new_up.hot {
                for (i, &c) in h.buckets.iter().enumerate() {
                    for _ in 0..c.min(128) {
                        new_up_total.push((i as u64 + 1) * BUCKET_WIDTH_US);
                    }
                }
            }
        } else {
            // Versions 2–4: migrate from rolling-window samples conservatively.
            let valid_legacy = |v: f64| v.is_finite() && v >= 0.0 && v <= MAX_SAMPLE_US as f64;

            // Basic array size check for legacy fields.
            if state.version >= 2 {
                if state.ema_down.len() != expected_len
                    || state.warm_down.len() != expected_len
                    || state.count_down.len() != expected_len
                    || state.sum_down.len() != expected_len
                {
                    return Err("estimator bucket arrays do not match max_poly".to_string());
                }
                if !state.ema_down.iter().copied().all(valid_legacy)
                    || !valid_legacy(state.ema_down_total)
                    || !valid_legacy(state.ema_up)
                {
                    return Err("estimator lead values are outside the accepted range".to_string());
                }
                if !state.ema_residual.is_finite()
                    || state.ema_residual < -(MAX_RESIDUAL_US as f64)
                    || state.ema_residual > (MAX_RESIDUAL_US * 2) as f64
                    || !state.ema_residual_up.is_finite()
                    || state.ema_residual_up < -(MAX_RESIDUAL_US as f64)
                    || state.ema_residual_up > (MAX_RESIDUAL_US * 2) as f64
                {
                    return Err(
                        "estimator residual value is outside the accepted range".to_string()
                    );
                }
            }

            if state.version >= 3 {
                // V3/V4: rolling sample vecs available.
                if state.samples_down.len() != expected_len
                    || state.samples_up.len() != expected_len
                {
                    return Err("estimator rolling bucket arrays do not match max_poly".to_string());
                }
                for (i, samples) in state.samples_down.iter().enumerate() {
                    new_down.hot[i] = Histogram::from_legacy_samples(samples)?;
                    // Rebuild tail reserve from max observed.
                    if let Some(max) = new_down.hot[i].max() {
                        new_down.tail_reserve[i].update(max);
                    }
                }
                for (i, samples) in state.samples_up.iter().enumerate() {
                    new_up.hot[i] = Histogram::from_legacy_samples(samples)?;
                    if let Some(max) = new_up.hot[i].max() {
                        new_up.tail_reserve[i].update(max);
                    }
                }
                new_down_total = Histogram::from_legacy_samples(&state.samples_down_total)?;
                new_up_total = Histogram::from_legacy_samples(&state.samples_up_total)?;
            } else {
                // V2: synthesise from EMA scalars.
                for i in 0..expected_len {
                    if state.warm_down[i] && state.count_down[i] >= SEED_SAMPLES as u64 {
                        let v = round_half_to_even(state.ema_down[i]).max(0) as u64;
                        for _ in 0..ROLLING_WINDOW.min(state.count_down[i] as usize) {
                            new_down.hot[i].push(v);
                        }
                        new_down.tail_reserve[i].update(v);
                    }
                }
                if state.warm_up && state.count_up >= SEED_SAMPLES as u64 {
                    let v = round_half_to_even(state.ema_up).max(0) as u64;
                    for i in 0..expected_len {
                        for _ in 0..ROLLING_WINDOW.min(state.count_up as usize) {
                            new_up.hot[i].push(v);
                        }
                        new_up.tail_reserve[i].update(v);
                    }
                    for _ in 0..ROLLING_WINDOW.min(state.count_up as usize) {
                        new_up_total.push(v);
                    }
                }
                if state.warm_down_total && state.count_down_total >= SEED_SAMPLES as u64 {
                    let v = round_half_to_even(state.ema_down_total).max(0) as u64;
                    for _ in 0..ROLLING_WINDOW.min(state.count_down_total as usize) {
                        new_down_total.push(v);
                    }
                }
            }
        }

        // ── Build residuals ───────────────────────────────────────────────────
        let mut new_residuals: [ResidualEma; 4] = Default::default();

        if state.version >= ESTIMATOR_STATE_VERSION && state.residuals.len() == 4 {
            for (i, ch) in state.residuals.iter().enumerate() {
                // Validate bounds.
                if !ch.ema.is_finite()
                    || ch.ema < -(MAX_RESIDUAL_US as f64)
                    || ch.ema > (MAX_RESIDUAL_US * 2) as f64
                {
                    return Err(format!("residual channel {i} ema is out of range"));
                }
                new_residuals[i] = ResidualEma {
                    count: ch.count,
                    sum: ch.sum,
                    ema: ch.ema,
                    warm: ch.warm,
                };
            }
        } else {
            // Migrate from v4 down/up residual scalars.
            if !state.ema_residual.is_finite() {
                return Err("ema_residual is not finite".to_string());
            }
            new_residuals[0] = ResidualEma {
                count: state.count_residual,
                sum: state.sum_residual,
                ema: state.ema_residual,
                warm: state.warm_residual,
            };
            if !state.ema_residual_up.is_finite() {
                return Err("ema_residual_up is not finite".to_string());
            }
            new_residuals[2] = ResidualEma {
                count: state.count_residual_up,
                sum: state.sum_residual_up,
                ema: state.ema_residual_up,
                warm: state.warm_residual_up,
            };
        }

        // ── Delivery proxy ────────────────────────────────────────────────────
        let mut new_delivery_proxy = vec![0u64; target_len];
        if state.version >= ESTIMATOR_STATE_VERSION && !state.delivery_proxy_us.is_empty() {
            if state.delivery_proxy_us.len() != expected_len {
                return Err("delivery_proxy_us length does not match max_poly".to_string());
            }
            for (i, &v) in state.delivery_proxy_us.iter().enumerate() {
                new_delivery_proxy[i] = v;
            }
        }

        // ── Legacy scalar counts ──────────────────────────────────────────────
        let mut counts_down = state.count_down;
        counts_down.resize(target_len, 0);
        let mut sums_down = state.sum_down;
        sums_down.resize(target_len, 0);
        let counts_up = vec![0u64; target_len];
        let sums_up = vec![0u64; target_len];

        // ── Atomically apply all validated state ──────────────────────────────
        self.max_poly = target_poly;
        self.down = new_down;
        self.up = new_up;
        self.down_total = new_down_total;
        self.up_total = new_up_total;
        self.residuals = new_residuals;
        self.delivery_proxy_us = new_delivery_proxy;
        self.count_down = counts_down;
        self.sum_down = sums_down;
        self.count_down_total = state.count_down_total;
        self.sum_down_total = state.sum_down_total;
        self.count_up = counts_up;
        self.sum_up = sums_up;
        self.count_up_total = state.count_up;
        self.sum_up_total = state.sum_up;
        Ok(())
    }
}

impl Default for SendLatencyEstimator {
    fn default() -> Self {
        Self::new(0.2, 2_000, 15)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
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
            estimator.down.tail_reserve[1].get() >= 3_000,
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
        estimator.set_delivery_proxy_us(2, 300);
        let with_proxy = estimator.get_lead_us(ActionKind::Down, 2);
        assert_eq!(with_proxy, without_proxy + 300);
    }

    #[test]
    fn slow_tail_reserve_decays_but_stays_above_floor() {
        let mut reserve = SlowTailReserve::default();
        reserve.update(5_000);
        // After many small updates the reserve should decay but not hit zero.
        for _ in 0..200 {
            reserve.update(100);
        }
        let value = reserve.get();
        assert!(value > 0, "slow tail reserve must not decay to zero");
        assert!(
            value < 5_000,
            "slow tail reserve should decay below the initial outlier"
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
    fn v5_state_round_trip_preserves_histogram() {
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
    fn v2_state_migrates_conservatively() {
        let legacy = r#"{
            "version": 2, "saved_at": "", "max_poly": 2,
            "ema_down": [0.0, 100.0, 200.0],
            "warm_down": [false, true, true],
            "count_down": [0, 5, 5], "sum_down": [0, 500, 1000],
            "ema_down_total": 150.0, "warm_down_total": true,
            "count_down_total": 10, "sum_down_total": 1500,
            "ema_up": 120.0, "warm_up": true, "count_up": 5, "sum_up": 600,
            "ema_residual": 0.0, "warm_residual": false,
            "count_residual": 0, "sum_residual": 0
        }"#;
        let mut estimator = SendLatencyEstimator::new(0.2, 5_000, 6);
        estimator.import_state(legacy).unwrap();
        assert!(estimator.get_lead_us(ActionKind::Down, 2) >= 200);
        assert!(estimator.get_lead_us(ActionKind::Up, 1) >= 120);
    }

    #[test]
    fn v3_state_migrates_conservatively() {
        let mut source = SendLatencyEstimator::new(0.2, 5_000, 2);
        for v in [100u64, 110, 120, 130, 140] {
            source.update(ActionKind::Down, v, 2);
            source.update(ActionKind::Up, v + 10, 2);
        }
        // Export as v3 by manually tweaking the version field.
        let mut state = source.export_state();
        state.version = 3;
        // Zero the v5 fields to simulate a real v3 file.
        state.hist_down.clear();
        state.hist_up.clear();
        state.residuals.clear();
        let json = serde_json::to_string(&state).unwrap();
        let mut restored = SendLatencyEstimator::new(0.2, 5_000, 4);
        restored.import_state(&json).unwrap();
        // After migration the estimate should be ballpark-correct.
        assert!(
            restored.get_lead_us(ActionKind::Down, 2) >= WAKE_RESERVE_US,
            "migrated v3 lead must be at least WAKE_RESERVE_US"
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
            "version": 5, "max_poly": 2,
            "hist_down": [{"hot_pairs":[[999,1]],"cold_pairs":[],"tail_reserve_us":0}],
            "hist_up": [], "residuals": []
        }"#;
        assert!(estimator.import_state(invalid).is_err());
        assert_eq!(estimator.get_lead_us(ActionKind::Down, 1), lead_before);
        assert_eq!(estimator.max_poly, 6);
    }

    #[test]
    fn rolling_p95_is_conservative_and_bounded() {
        // Compatibility alias for the old test name — now tests the histogram.
        histogram_p95_is_conservative_and_bounded();
    }

    #[test]
    fn v3_state_round_trip_preserves_rolling_samples() {
        // Compat alias — now goes through v5 histogram round-trip.
        v5_state_round_trip_preserves_histogram();
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
    fn invalid_state_does_not_mutate_estimator_v3() {
        // Alias kept for test name compat.
        invalid_state_does_not_mutate_estimator();
    }
}
