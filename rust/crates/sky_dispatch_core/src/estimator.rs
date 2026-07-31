//! Conservative adaptive SendInput completion-latency estimator.
//!
//! The estimator deliberately models the upper tail of the observed path
//! rather than its mean.  Dispatch lead is a safety margin, so a p95 estimate
//! is a better fit than an EMA-only centre estimate.  The state format is
//! versioned; version 2 caches are accepted and migrated into a conservative
//! synthetic rolling window on import.

use crate::model::ActionKind;
use serde::{Deserialize, Serialize};

const SEED_SAMPLES: usize = 5;
const ROLLING_WINDOW: usize = 32;
const MAX_RESIDUAL_US: i64 = 500;
const MAX_SAMPLE_US: u64 = 60_000_000;
const BASE_COLD_PRIOR_US: u64 = 100;
const PER_KEY_COLD_PRIOR_US: u64 = 40;
const EARLY_CORRECTION_DECAY: f64 = 0.25;
pub const ESTIMATOR_STATE_VERSION: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LatencyClass {
    Hot,
    Cold,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EstimatorStateJson {
    pub version: u32,
    #[serde(default)]
    pub saved_at: String,
    pub max_poly: usize,
    // Legacy summary fields remain in the on-disk format so version 2 caches
    // can be inspected and migrated without a separate decoder.
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
    /// Version 3 rolling samples, indexed by polyphony bucket.
    #[serde(default)]
    pub samples_down: Vec<Vec<u64>>,
    #[serde(default)]
    pub samples_up: Vec<Vec<u64>>,
    #[serde(default)]
    pub samples_down_total: Vec<u64>,
    #[serde(default)]
    pub samples_up_total: Vec<u64>,
    /// Version 4 residual completion bias for note-off dispatches.
    #[serde(default)]
    pub ema_residual_up: f64,
    #[serde(default)]
    pub warm_residual_up: bool,
    #[serde(default)]
    pub count_residual_up: u64,
    #[serde(default)]
    pub sum_residual_up: i64,
}

#[derive(Debug, Clone)]
struct RollingSamples {
    values: [u64; ROLLING_WINDOW],
    len: u8,
    cursor: u8,
    cached_p95: Option<u64>,
}

impl Default for RollingSamples {
    fn default() -> Self {
        Self {
            values: [0; ROLLING_WINDOW],
            len: 0,
            cursor: 0,
            cached_p95: None,
        }
    }
}

impl RollingSamples {
    fn push(&mut self, value: u64) {
        if usize::from(self.len) < ROLLING_WINDOW {
            self.values[usize::from(self.len)] = value;
            self.len += 1;
            self.cursor = self.len % ROLLING_WINDOW as u8;
        } else {
            self.values[usize::from(self.cursor)] = value;
            self.cursor = (self.cursor + 1) % ROLLING_WINDOW as u8;
        }
        self.refresh_p95();
    }

    fn is_warm(&self) -> bool {
        usize::from(self.len) >= SEED_SAMPLES
    }

    fn p95(&self) -> Option<u64> {
        self.cached_p95
    }

    fn max(&self) -> Option<u64> {
        let len = usize::from(self.len);
        (len > 0).then(|| self.values[..len].iter().copied().max().unwrap_or(0))
    }

    fn refresh_p95(&mut self) {
        if !self.is_warm() {
            self.cached_p95 = None;
            return;
        }
        let len = usize::from(self.len);
        let mut sorted = self.values;
        sorted[..len].sort_unstable();
        let rank = (len * 95).saturating_add(99) / 100;
        self.cached_p95 = Some(sorted[rank.saturating_sub(1).min(len - 1)]);
    }

    fn to_vec(&self) -> Vec<u64> {
        let len = usize::from(self.len);
        if len < ROLLING_WINDOW {
            return self.values[..len].to_vec();
        }
        (0..ROLLING_WINDOW)
            .map(|offset| self.values[(usize::from(self.cursor) + offset) % ROLLING_WINDOW])
            .collect()
    }

    fn from_values(values: &[u64]) -> Result<Self, String> {
        if values.len() > ROLLING_WINDOW {
            return Err(format!(
                "estimator rolling window must contain at most {ROLLING_WINDOW} samples"
            ));
        }
        let mut result = Self::default();
        for &value in values {
            if value > MAX_SAMPLE_US {
                return Err("estimator sample is outside the accepted range".to_string());
            }
            result.push(value);
        }
        Ok(result)
    }

    fn from_legacy(value: f64, warm: bool, count: u64) -> Result<Self, String> {
        if !value.is_finite() || value < 0.0 || value > MAX_SAMPLE_US as f64 {
            return Err("legacy estimator value is outside the accepted range".to_string());
        }
        let mut result = Self::default();
        if warm && count >= SEED_SAMPLES as u64 {
            let rounded = round_half_to_even(value).max(0) as u64;
            for _ in 0..ROLLING_WINDOW.min(count as usize) {
                result.push(rounded);
            }
        }
        Ok(result)
    }
}

#[derive(Debug, Clone)]
pub struct SendLatencyEstimator {
    pub max_poly: usize,
    alpha: f64,
    max_lead_us: u64,
    count_down: Vec<u64>,
    sum_down: Vec<u64>,
    down_windows: Vec<RollingSamples>,
    down_total_window: RollingSamples,
    count_down_total: u64,
    sum_down_total: u64,
    count_up: Vec<u64>,
    sum_up: Vec<u64>,
    up_windows: Vec<RollingSamples>,
    up_total_window: RollingSamples,
    count_up_total: u64,
    sum_up_total: u64,
    count_residual: u64,
    sum_residual: i64,
    ema_residual: f64,
    warm_residual: bool,
    count_residual_up: u64,
    sum_residual_up: i64,
    ema_residual_up: f64,
    warm_residual_up: bool,
    cold_down_windows: Vec<RollingSamples>,
    cold_up_windows: Vec<RollingSamples>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeadEstimate {
    pub applied_us: u64,
    pub uncapped_us: u64,
    pub saturated: bool,
}

impl SendLatencyEstimator {
    pub fn new(alpha: f64, max_lead_us: u64, max_poly: usize) -> Self {
        let size = max_poly + 1;
        Self {
            max_poly,
            alpha,
            max_lead_us,
            count_down: vec![0; size],
            sum_down: vec![0; size],
            down_windows: vec![RollingSamples::default(); size],
            down_total_window: RollingSamples::default(),
            count_down_total: 0,
            sum_down_total: 0,
            count_up: vec![0; size],
            sum_up: vec![0; size],
            up_windows: vec![RollingSamples::default(); size],
            up_total_window: RollingSamples::default(),
            count_up_total: 0,
            sum_up_total: 0,
            count_residual: 0,
            sum_residual: 0,
            ema_residual: 0.0,
            warm_residual: false,
            count_residual_up: 0,
            sum_residual_up: 0,
            ema_residual_up: 0.0,
            warm_residual_up: false,
            cold_down_windows: vec![RollingSamples::default(); size],
            cold_up_windows: vec![RollingSamples::default(); size],
        }
    }

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
                self.count_down[n] = self.count_down[n].saturating_add(1);
                self.sum_down[n] = self.sum_down[n].saturating_add(duration_us);
                self.down_windows[n].push(duration_us);
                if latency_class == LatencyClass::Cold {
                    self.cold_down_windows[n].push(duration_us);
                }
                self.count_down_total = self.count_down_total.saturating_add(1);
                self.sum_down_total = self.sum_down_total.saturating_add(duration_us);
                self.down_total_window.push(duration_us);
            }
            ActionKind::Up => {
                self.count_up[n] = self.count_up[n].saturating_add(1);
                self.sum_up[n] = self.sum_up[n].saturating_add(duration_us);
                self.up_windows[n].push(duration_us);
                if latency_class == LatencyClass::Cold {
                    self.cold_up_windows[n].push(duration_us);
                }
                self.count_up_total = self.count_up_total.saturating_add(1);
                self.sum_up_total = self.sum_up_total.saturating_add(duration_us);
                self.up_total_window.push(duration_us);
            }
        }
    }

    pub fn update_completion_error(&mut self, kind: ActionKind, error_us: i64) {
        let sample = error_us.clamp(-MAX_RESIDUAL_US, MAX_RESIDUAL_US * 2);
        let (count, sum, ema, warm) = match kind {
            ActionKind::Down => (
                &mut self.count_residual,
                &mut self.sum_residual,
                &mut self.ema_residual,
                &mut self.warm_residual,
            ),
            ActionKind::Up => (
                &mut self.count_residual_up,
                &mut self.sum_residual_up,
                &mut self.ema_residual_up,
                &mut self.warm_residual_up,
            ),
        };
        *count = count.saturating_add(1);
        *sum = sum.saturating_add(sample);
        if *warm {
            *ema = self.alpha * sample as f64 + (1.0 - self.alpha) * *ema;
        } else if *count >= SEED_SAMPLES as u64 {
            *ema = *sum as f64 / *count as f64;
            *warm = true;
        }
    }

    pub fn residual_bias_us(&self) -> u64 {
        self.residual_adjustment_us().max(0) as u64
    }

    /// Signed correction for the completion residual.
    ///
    /// Late completion raises lead at full strength; early completion only
    /// reduces it at a quarter rate so a short-lived early sample cannot
    /// immediately erase a safety margin.
    pub fn residual_adjustment_us(&self) -> i64 {
        self.residual_adjustment_us_for(ActionKind::Down)
    }

    pub fn residual_adjustment_us_for(&self, kind: ActionKind) -> i64 {
        let (warm, ema) = match kind {
            ActionKind::Down => (self.warm_residual, self.ema_residual),
            ActionKind::Up => (self.warm_residual_up, self.ema_residual_up),
        };
        if !warm {
            return 0;
        }
        let rounded = round_half_to_even(ema);
        if rounded >= 0 {
            rounded.min(MAX_RESIDUAL_US)
        } else {
            round_half_to_even(rounded as f64 * EARLY_CORRECTION_DECAY)
        }
    }

    fn cold_prior_us(n: usize) -> u64 {
        BASE_COLD_PRIOR_US
            .saturating_add(PER_KEY_COLD_PRIOR_US.saturating_mul(n.saturating_sub(1) as u64))
    }

    fn raw_estimate_us(
        &self,
        kind: ActionKind,
        n: usize,
        latency_class: LatencyClass,
        strict_upper_tail: bool,
    ) -> u64 {
        let (windows, total_window) = match kind {
            ActionKind::Down => (&self.down_windows, &self.down_total_window),
            ActionKind::Up => (&self.up_windows, &self.up_total_window),
        };
        let cold_local = match kind {
            ActionKind::Down => self.cold_down_windows[n].p95(),
            ActionKind::Up => self.cold_up_windows[n].p95(),
        };
        let quantile = |window: &RollingSamples| {
            if strict_upper_tail {
                window.max()
            } else {
                window.p95()
            }
        };
        let local = quantile(&windows[n])
            .into_iter()
            .chain(
                (latency_class == LatencyClass::Cold)
                    .then_some(cold_local)
                    .flatten(),
            )
            .max();
        let global = quantile(total_window);
        let lower_bucket = (1..=n)
            .rev()
            .find_map(|bucket| quantile(&windows[bucket]).map(|estimate| (bucket, estimate)))
            .map(|(bucket, estimate)| {
                estimate.saturating_add(
                    PER_KEY_COLD_PRIOR_US.saturating_mul(n.saturating_sub(bucket) as u64),
                )
            });
        // Keep the global tail as a guard in strict mode even after a sparse
        // local bucket has one or two samples. A newly seeded polyphony
        // bucket must not erase a recent strict-mode outlier observed
        // elsewhere on the input path. Normal mode retains the local-first
        // behavior so a calibrated bucket is not overridden by an unrelated
        // global sample.
        let local_or_global = if strict_upper_tail {
            local.into_iter().chain(global).max()
        } else {
            local.or(global)
        };
        local_or_global
            .into_iter()
            .chain(lower_bucket)
            .chain(std::iter::once(Self::cold_prior_us(n)))
            .max()
            .unwrap_or_else(|| Self::cold_prior_us(n))
    }

    fn base_lead_us(
        &self,
        kind: ActionKind,
        n: usize,
        latency_class: LatencyClass,
        strict_upper_tail: bool,
    ) -> u64 {
        let raw = self.raw_estimate_us(kind, n, latency_class, strict_upper_tail) as i64;
        let residual = self.residual_adjustment_us_for(kind);
        raw.saturating_add(residual).max(0) as u64
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
        // The envelope is intentional: independently observed buckets must
        // never make a larger chord receive a smaller lead than a smaller
        // chord.  This also makes cold-start behaviour conservative when a
        // new polyphony bucket has not yet accumulated five samples.
        let monotonic = (1..=n)
            .map(|bucket| self.base_lead_us(kind, bucket, latency_class, strict_upper_tail))
            .max()
            .unwrap_or(0);
        LeadEstimate {
            applied_us: monotonic.min(self.max_lead_us),
            uncapped_us: monotonic,
            saturated: monotonic > self.max_lead_us,
        }
    }

    pub fn get_lead_us(&self, kind: ActionKind, n_keys: usize) -> u64 {
        self.estimate_lead(kind, n_keys).applied_us
    }

    /// Returns true when the uncapped monotonic lead would exceed the
    /// configured cap.  This is telemetry information, not a second dispatch
    /// decision, so callers can distinguish a healthy capped value from a
    /// model that is no longer able to compensate the observed path.
    pub fn lead_saturated(&self, kind: ActionKind, n_keys: usize) -> bool {
        self.estimate_lead(kind, n_keys).saturated
    }

    pub fn export_state(&self) -> EstimatorStateJson {
        EstimatorStateJson {
            version: ESTIMATOR_STATE_VERSION,
            saved_at: String::new(),
            max_poly: self.max_poly,
            ema_down: self
                .down_windows
                .iter()
                .map(|window| window.p95().unwrap_or(0) as f64)
                .collect(),
            warm_down: self
                .down_windows
                .iter()
                .map(RollingSamples::is_warm)
                .collect(),
            count_down: self.count_down.clone(),
            sum_down: self.sum_down.clone(),
            ema_down_total: self.down_total_window.p95().unwrap_or(0) as f64,
            warm_down_total: self.down_total_window.is_warm(),
            count_down_total: self.count_down_total,
            sum_down_total: self.sum_down_total,
            ema_up: self.up_total_window.p95().unwrap_or(0) as f64,
            warm_up: self.up_total_window.is_warm(),
            count_up: self.count_up_total,
            sum_up: self.sum_up_total,
            ema_residual: self.ema_residual,
            warm_residual: self.warm_residual,
            count_residual: self.count_residual,
            sum_residual: self.sum_residual,
            samples_down: self
                .down_windows
                .iter()
                .map(RollingSamples::to_vec)
                .collect(),
            samples_up: self.up_windows.iter().map(RollingSamples::to_vec).collect(),
            samples_down_total: self.down_total_window.to_vec(),
            samples_up_total: self.up_total_window.to_vec(),
            ema_residual_up: self.ema_residual_up,
            warm_residual_up: self.warm_residual_up,
            count_residual_up: self.count_residual_up,
            sum_residual_up: self.sum_residual_up,
        }
    }

    pub fn import_state(&mut self, json_str: &str) -> Result<(), String> {
        let state: EstimatorStateJson =
            serde_json::from_str(json_str).map_err(|e| format!("invalid estimator json: {e}"))?;
        if !matches!(state.version, 2 | 3 | ESTIMATOR_STATE_VERSION) {
            return Err(format!("unsupported estimator version: {}", state.version));
        }
        if !(1..=32).contains(&state.max_poly) {
            return Err("max_poly must be in 1..=32".to_string());
        }
        let expected_len = state.max_poly + 1;
        if state.ema_down.len() != expected_len
            || state.warm_down.len() != expected_len
            || state.count_down.len() != expected_len
            || state.sum_down.len() != expected_len
        {
            return Err("estimator bucket arrays do not match max_poly".to_string());
        }
        let valid_legacy =
            |value: f64| value.is_finite() && value >= 0.0 && value <= MAX_SAMPLE_US as f64;
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
            return Err("estimator residual value is outside the accepted range".to_string());
        }

        let mut imported_down = Vec::with_capacity(expected_len);
        let mut imported_up = Vec::with_capacity(expected_len);
        let imported_down_total;
        let imported_up_total;
        if state.version >= 3 {
            if state.samples_down.len() != expected_len || state.samples_up.len() != expected_len {
                return Err("estimator rolling bucket arrays do not match max_poly".to_string());
            }
            for values in &state.samples_down {
                imported_down.push(RollingSamples::from_values(values)?);
            }
            for values in &state.samples_up {
                imported_up.push(RollingSamples::from_values(values)?);
            }
            imported_down_total = RollingSamples::from_values(&state.samples_down_total)?;
            imported_up_total = RollingSamples::from_values(&state.samples_up_total)?;
        } else {
            for index in 0..expected_len {
                imported_down.push(RollingSamples::from_legacy(
                    state.ema_down[index],
                    state.warm_down[index],
                    state.count_down[index],
                )?);
                imported_up.push(RollingSamples::default());
            }
            imported_down_total = RollingSamples::from_legacy(
                state.ema_down_total,
                state.warm_down_total,
                state.count_down_total,
            )?;
            imported_up_total =
                RollingSamples::from_legacy(state.ema_up, state.warm_up, state.count_up)?;
            if state.warm_up && state.count_up >= SEED_SAMPLES as u64 {
                for window in &mut imported_up {
                    // Version 2 had one global Up estimate.  Give every
                    // bucket the same conservative seed, then let new
                    // observations specialize it.
                    *window = imported_up_total.clone();
                }
            }
        }

        // Validate first, then apply atomically. Preserve the constructed
        // polyphony when importing a lower-polyphony cache.
        let target_poly = self.max_poly.max(state.max_poly);
        imported_down.resize(target_poly + 1, RollingSamples::default());
        imported_up.resize(target_poly + 1, RollingSamples::default());
        let mut counts_down = state.count_down;
        counts_down.resize(target_poly + 1, 0);
        let mut sums_down = state.sum_down;
        sums_down.resize(target_poly + 1, 0);
        let mut counts_up = vec![0; target_poly + 1];
        let mut sums_up = vec![0; target_poly + 1];
        if state.version >= 3 {
            // Version 3 has bucket counts only for Down for compatibility;
            // bucket Up counts are represented by the rolling windows.
            for (index, window) in imported_up.iter().enumerate() {
                counts_up[index] = window.len as u64;
                sums_up[index] = window.values[..usize::from(window.len)].iter().sum();
            }
        } else {
            counts_up[0] = state.count_up;
            sums_up[0] = state.sum_up;
        }

        self.max_poly = target_poly;
        self.count_down = counts_down;
        self.sum_down = sums_down;
        self.down_windows = imported_down;
        self.down_total_window = imported_down_total;
        self.count_down_total = state.count_down_total;
        self.sum_down_total = state.sum_down_total;
        self.count_up = counts_up;
        self.sum_up = sums_up;
        self.up_windows = imported_up;
        self.up_total_window = imported_up_total;
        self.count_up_total = state.count_up;
        self.sum_up_total = state.sum_up;
        self.ema_residual = state.ema_residual;
        self.warm_residual = state.warm_residual;
        self.count_residual = state.count_residual;
        self.sum_residual = state.sum_residual;
        self.ema_residual_up = state.ema_residual_up;
        self.warm_residual_up = state.warm_residual_up;
        self.count_residual_up = state.count_residual_up;
        self.sum_residual_up = state.sum_residual_up;
        self.cold_down_windows = vec![RollingSamples::default(); target_poly + 1];
        self.cold_up_windows = vec![RollingSamples::default(); target_poly + 1];
        Ok(())
    }
}

impl Default for SendLatencyEstimator {
    fn default() -> Self {
        Self::new(0.2, 2_000, 15)
    }
}

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
    fn rolling_p95_is_conservative_and_bounded() {
        let mut samples = RollingSamples::default();
        for value in 100..132 {
            samples.push(value);
        }
        assert_eq!(samples.p95(), Some(130));
        samples.push(2_000);
        assert_eq!(usize::from(samples.len), ROLLING_WINDOW);
        assert_eq!(samples.p95(), Some(131));
    }

    #[test]
    fn strict_upper_tail_keeps_a_single_recent_outlier_visible() {
        let mut estimator = SendLatencyEstimator::new(0.2, 4_000, 1);
        for _ in 0..32 {
            estimator.update(ActionKind::Down, 100, 1);
        }
        estimator.update(ActionKind::Down, 2_000, 1);

        assert_eq!(
            estimator
                .estimate_lead_with_class(ActionKind::Down, 1, LatencyClass::Hot)
                .applied_us,
            100
        );
        assert_eq!(
            estimator
                .estimate_lead_with_class_and_policy(ActionKind::Down, 1, LatencyClass::Hot, true,)
                .applied_us,
            2_000
        );
    }

    #[test]
    fn strict_sparse_bucket_keeps_global_upper_tail_guard() {
        let mut estimator = SendLatencyEstimator::new(0.2, 4_000, 8);
        for _ in 0..32 {
            estimator.update(ActionKind::Down, 1_500, 8);
        }
        for _ in 0..5 {
            estimator.update(ActionKind::Down, 300, 1);
        }

        assert_eq!(
            estimator
                .estimate_lead_with_class_and_policy(ActionKind::Down, 1, LatencyClass::Hot, true)
                .applied_us,
            1_500
        );
    }

    #[test]
    fn cold_start_is_conservative_for_large_chords() {
        let estimator = SendLatencyEstimator::new(0.2, 2_000, 15);
        assert_eq!(estimator.get_lead_us(ActionKind::Down, 1), 100);
        assert_eq!(estimator.get_lead_us(ActionKind::Down, 15), 660);
        assert!(
            estimator.get_lead_us(ActionKind::Down, 15)
                >= estimator.get_lead_us(ActionKind::Down, 1)
        );
    }

    #[test]
    fn both_directions_use_polyphony_buckets_and_monotonic_envelope() {
        let mut estimator = SendLatencyEstimator::new(0.2, 4_000, 6);
        for _ in 0..5 {
            estimator.update(ActionKind::Down, 100, 1);
            estimator.update(ActionKind::Down, 700, 3);
            estimator.update(ActionKind::Up, 120, 1);
            estimator.update(ActionKind::Up, 800, 3);
        }
        assert_eq!(estimator.get_lead_us(ActionKind::Down, 1), 100);
        assert_eq!(estimator.get_lead_us(ActionKind::Down, 3), 700);
        assert_eq!(estimator.get_lead_us(ActionKind::Up, 1), 120);
        assert_eq!(estimator.get_lead_us(ActionKind::Up, 3), 800);
    }

    #[test]
    fn v3_state_round_trip_preserves_rolling_samples() {
        let mut source = SendLatencyEstimator::new(0.2, 2_000, 2);
        for value in [100, 110, 120, 130, 140] {
            source.update(ActionKind::Down, value, 2);
            source.update(ActionKind::Up, value + 10, 2);
        }
        let json = serde_json::to_string(&source.export_state()).unwrap();
        let mut restored = SendLatencyEstimator::new(0.2, 2_000, 6);
        restored.import_state(&json).unwrap();
        assert_eq!(restored.export_state().version, ESTIMATOR_STATE_VERSION);
        assert_eq!(restored.get_lead_us(ActionKind::Down, 2), 140);
        assert_eq!(restored.get_lead_us(ActionKind::Up, 2), 150);
        assert_eq!(restored.max_poly, 6);
    }

    #[test]
    fn wrapped_state_round_trip_preserves_ring_order_and_future_updates() {
        let mut source = SendLatencyEstimator::new(0.2, 4_000, 2);
        for value in 100..139 {
            source.update(ActionKind::Down, value, 2);
        }
        let json = serde_json::to_string(&source.export_state()).unwrap();
        let mut restored = SendLatencyEstimator::new(0.2, 4_000, 2);
        restored.import_state(&json).unwrap();

        source.update(ActionKind::Down, 2_000, 2);
        restored.update(ActionKind::Down, 2_000, 2);
        assert_eq!(
            source.get_lead_us(ActionKind::Down, 2),
            restored.get_lead_us(ActionKind::Down, 2)
        );
        assert_eq!(
            source.export_state().samples_down,
            restored.export_state().samples_down
        );
    }

    #[test]
    fn up_residual_is_learned_separately_from_down_residual() {
        let mut estimator = SendLatencyEstimator::new(0.2, 4_000, 2);
        for _ in 0..5 {
            estimator.update(ActionKind::Up, 100, 1);
            estimator.update_completion_error(ActionKind::Up, 300);
        }
        assert_eq!(estimator.residual_adjustment_us_for(ActionKind::Up), 300);
        assert_eq!(estimator.residual_adjustment_us_for(ActionKind::Down), 0);
        assert_eq!(estimator.get_lead_us(ActionKind::Up, 1), 400);
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
        let mut estimator = SendLatencyEstimator::new(0.2, 2_000, 6);
        estimator.import_state(legacy).unwrap();
        assert!(estimator.get_lead_us(ActionKind::Down, 2) >= 200);
        assert!(estimator.get_lead_us(ActionKind::Up, 1) >= 120);
    }

    #[test]
    fn invalid_state_does_not_mutate_estimator() {
        let mut estimator = SendLatencyEstimator::new(0.2, 2_000, 6);
        for _ in 0..5 {
            estimator.update(ActionKind::Down, 100, 1);
        }
        let lead_before = estimator.get_lead_us(ActionKind::Down, 1);
        let invalid = r#"{
            "version": 3, "saved_at": "", "max_poly": 2,
            "ema_down": [0.0], "warm_down": [false],
            "count_down": [0], "sum_down": [0],
            "ema_down_total": 0.0, "warm_down_total": false,
            "count_down_total": 0, "sum_down_total": 0,
            "ema_up": 0.0, "warm_up": false, "count_up": 0, "sum_up": 0,
            "ema_residual": 0.0, "warm_residual": false,
            "count_residual": 0, "sum_residual": 0,
            "samples_down": [], "samples_up": [],
            "samples_down_total": [], "samples_up_total": []
        }"#;
        assert!(estimator.import_state(invalid).is_err());
        assert_eq!(estimator.get_lead_us(ActionKind::Down, 1), lead_before);
        assert_eq!(estimator.max_poly, 6);
    }

    #[test]
    fn early_residual_reduces_lead_more_slowly_than_late_residual_increases_it() {
        let mut estimator = SendLatencyEstimator::new(0.2, 2_000, 6);
        for _ in 0..5 {
            estimator.update(ActionKind::Down, 800, 1);
            estimator.update_completion_error(ActionKind::Down, -400);
        }
        assert_eq!(estimator.residual_adjustment_us(), -100);
        assert_eq!(estimator.get_lead_us(ActionKind::Down, 1), 700);

        for _ in 0..5 {
            estimator.update_completion_error(ActionKind::Down, 400);
        }
        assert!(estimator.residual_adjustment_us() > -100);
    }
}
