//! Adaptive send-latency lead estimator port with Python-compatible bankers rounding.

use crate::model::ActionKind;
use serde::{Deserialize, Serialize};

const SEED_SAMPLES: u64 = 5;
const MAX_RESIDUAL_US: i64 = 500;

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
    pub ema_down: Vec<f64>,
    pub warm_down: Vec<bool>,
    pub count_down: Vec<u64>,
    pub sum_down: Vec<u64>,
    pub ema_down_total: f64,
    pub warm_down_total: bool,
    pub count_down_total: u64,
    pub sum_down_total: u64,
    pub ema_up: f64,
    pub warm_up: bool,
    pub count_up: u64,
    pub sum_up: u64,
    pub ema_residual: f64,
    pub warm_residual: bool,
    pub count_residual: u64,
    pub sum_residual: i64,
}

#[derive(Debug, Clone)]
pub struct SendLatencyEstimator {
    pub max_poly: usize,
    alpha: f64,
    max_lead_us: u64,
    count_down: Vec<u64>,
    sum_down: Vec<u64>,
    ema_down: Vec<f64>,
    warm_down: Vec<bool>,
    count_down_total: u64,
    sum_down_total: u64,
    ema_down_total: f64,
    count_up: u64,
    sum_up: u64,
    ema_up: f64,
    count_residual: u64,
    sum_residual: i64,
    ema_residual: f64,
    warm_residual: bool,
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
            ema_down: vec![0.0; size],
            warm_down: vec![false; size],
            count_down_total: 0,
            sum_down_total: 0,
            ema_down_total: 0.0,
            count_up: 0,
            sum_up: 0,
            ema_up: 0.0,
            count_residual: 0,
            sum_residual: 0,
            ema_residual: 0.0,
            warm_residual: false,
        }
    }

    pub fn update(&mut self, kind: ActionKind, duration_us: u64, n_keys: usize) {
        let duration_f = duration_us as f64;
        match kind {
            ActionKind::Down => {
                let n = 1.max(self.max_poly.min(n_keys));
                self.count_down[n] += 1;
                if self.warm_down[n] {
                    self.ema_down[n] =
                        self.alpha * duration_f + (1.0 - self.alpha) * self.ema_down[n];
                } else {
                    self.sum_down[n] += duration_us;
                    if self.count_down[n] >= SEED_SAMPLES {
                        self.ema_down[n] = self.sum_down[n] as f64 / self.count_down[n] as f64;
                        self.warm_down[n] = true;
                    }
                }

                self.count_down_total += 1;
                if self.count_down_total <= SEED_SAMPLES {
                    self.sum_down_total += duration_us;
                    if self.count_down_total == SEED_SAMPLES {
                        self.ema_down_total = self.sum_down_total as f64 / SEED_SAMPLES as f64;
                    }
                } else {
                    self.ema_down_total =
                        self.alpha * duration_f + (1.0 - self.alpha) * self.ema_down_total;
                }
            }
            ActionKind::Up => {
                self.count_up += 1;
                if self.count_up <= SEED_SAMPLES {
                    self.sum_up += duration_us;
                    if self.count_up == SEED_SAMPLES {
                        self.ema_up = self.sum_up as f64 / SEED_SAMPLES as f64;
                    }
                } else {
                    self.ema_up = self.alpha * duration_f + (1.0 - self.alpha) * self.ema_up;
                }
            }
        }
    }

    pub fn update_completion_error(&mut self, kind: ActionKind, error_us: i64) {
        if kind != ActionKind::Down {
            return;
        }
        let sample = (-MAX_RESIDUAL_US).max((MAX_RESIDUAL_US * 2).min(error_us));
        self.count_residual += 1;
        if self.warm_residual {
            self.ema_residual =
                self.alpha * (sample as f64) + (1.0 - self.alpha) * self.ema_residual;
        } else {
            self.sum_residual += sample;
            if self.count_residual >= SEED_SAMPLES {
                self.ema_residual = self.sum_residual as f64 / self.count_residual as f64;
                self.warm_residual = true;
            }
        }
    }

    pub fn residual_bias_us(&self) -> u64 {
        if !self.warm_residual {
            return 0;
        }
        let rounded = round_half_to_even(self.ema_residual);
        0.max(MAX_RESIDUAL_US.min(rounded)) as u64
    }

    pub fn get_lead_us(&self, kind: ActionKind, n_keys: usize) -> u64 {
        let residual = self.residual_bias_us() as i64;
        let max_lead = self.max_lead_us as i64;
        match kind {
            ActionKind::Down => {
                let n = 1.max(self.max_poly.min(n_keys));
                if self.warm_down[n] {
                    let rounded = round_half_to_even(self.ema_down[n]);
                    return (rounded + residual).clamp(0, max_lead) as u64;
                }
                for b in (1..=n).rev() {
                    if self.warm_down[b] {
                        let rounded = round_half_to_even(self.ema_down[b]);
                        return (rounded + residual).clamp(0, max_lead) as u64;
                    }
                }
                if self.count_down_total >= SEED_SAMPLES {
                    let rounded = round_half_to_even(self.ema_down_total);
                    return (rounded + residual).clamp(0, max_lead) as u64;
                }
                0
            }
            ActionKind::Up => {
                if self.count_up < SEED_SAMPLES {
                    return 0;
                }
                let rounded = round_half_to_even(self.ema_up);
                rounded.clamp(0, max_lead) as u64
            }
        }
    }

    pub fn export_state(&self) -> EstimatorStateJson {
        EstimatorStateJson {
            version: 2,
            saved_at: String::new(),
            max_poly: self.max_poly,
            ema_down: self.ema_down.clone(),
            warm_down: self.warm_down.clone(),
            count_down: self.count_down.clone(),
            sum_down: self.sum_down.clone(),
            ema_down_total: self.ema_down_total,
            warm_down_total: self.count_down_total >= SEED_SAMPLES,
            count_down_total: self.count_down_total,
            sum_down_total: self.sum_down_total,
            ema_up: self.ema_up,
            warm_up: self.count_up >= SEED_SAMPLES,
            count_up: self.count_up,
            sum_up: self.sum_up,
            ema_residual: self.ema_residual,
            warm_residual: self.warm_residual,
            count_residual: self.count_residual,
            sum_residual: self.sum_residual,
        }
    }

    pub fn import_state(&mut self, json_str: &str) -> Result<(), String> {
        let state: EstimatorStateJson =
            serde_json::from_str(json_str).map_err(|e| format!("invalid estimator json: {e}"))?;
        if state.version != 2 {
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
        let valid_lead = |value: f64| {
            value.is_finite() && value >= 0.0 && value <= self.max_lead_us as f64
        };
        if !state.ema_down.iter().copied().all(valid_lead)
            || !valid_lead(state.ema_down_total)
            || !valid_lead(state.ema_up)
        {
            return Err("estimator lead values are outside the accepted range".to_string());
        }
        if !state.ema_residual.is_finite()
            || state.ema_residual < -(MAX_RESIDUAL_US as f64)
            || state.ema_residual > (MAX_RESIDUAL_US * 2) as f64
        {
            return Err("estimator residual value is outside the accepted range".to_string());
        }

        // Validate first, then apply atomically. Preserve the constructed polyphony
        // when importing a cache produced by a lower-polyphony song.
        let target_poly = self.max_poly.max(state.max_poly);
        let pad = target_poly - state.max_poly;
        self.max_poly = target_poly;
        self.ema_down = state.ema_down;
        self.ema_down.extend(std::iter::repeat_n(0.0, pad));
        self.warm_down = state.warm_down;
        self.warm_down
            .extend(std::iter::repeat_n(false, pad));
        self.count_down = state.count_down;
        self.count_down.extend(std::iter::repeat_n(0, pad));
        self.sum_down = state.sum_down;
        self.sum_down.extend(std::iter::repeat_n(0, pad));
        self.ema_down_total = state.ema_down_total;
        self.count_down_total = state.count_down_total;
        self.sum_down_total = state.sum_down_total;
        self.ema_up = state.ema_up;
        self.count_up = state.count_up;
        self.sum_up = state.sum_up;
        self.ema_residual = state.ema_residual;
        self.warm_residual = state.warm_residual;
        self.count_residual = state.count_residual;
        self.sum_residual = state.sum_residual;
        Ok(())
    }
}

impl Default for SendLatencyEstimator {
    fn default() -> Self {
        Self::new(0.2, 2_000, 6)
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
    fn test_estimator_seeding_and_lead() {
        let mut est = SendLatencyEstimator::new(0.2, 2000, 6);
        assert_eq!(est.get_lead_us(ActionKind::Down, 1), 0);

        // Update 5 samples of 1000 us for polyphony 1
        for _ in 0..5 {
            est.update(ActionKind::Down, 1000, 1);
        }

        assert_eq!(est.get_lead_us(ActionKind::Down, 1), 1000);
        // Fallback for unseeded polyphony 2 -> nearest seeded <= 2 is 1 -> returns 1000 us
        assert_eq!(est.get_lead_us(ActionKind::Down, 2), 1000);
    }

    #[test]
    fn test_estimator_state_round_trip_preserves_negative_residual_sum() {
        let mut source = SendLatencyEstimator::new(0.2, 2000, 2);
        for _ in 0..5 {
            source.update_completion_error(ActionKind::Down, -100);
        }
        let json = serde_json::to_string(&source.export_state()).unwrap();

        let mut restored = SendLatencyEstimator::new(0.2, 2000, 6);
        restored.import_state(&json).unwrap();

        assert_eq!(restored.sum_residual, -500);
        assert_eq!(restored.max_poly, 6);
        assert_eq!(restored.ema_down.len(), 7);
    }

    #[test]
    fn test_estimator_import_rejects_invalid_buckets_without_mutation() {
        let mut estimator = SendLatencyEstimator::new(0.2, 2000, 6);
        for _ in 0..5 {
            estimator.update(ActionKind::Down, 100, 1);
        }
        let lead_before = estimator.get_lead_us(ActionKind::Down, 1);
        let invalid = r#"{
            "version": 2,
            "saved_at": "",
            "max_poly": 2,
            "ema_down": [0.0],
            "warm_down": [false],
            "count_down": [0],
            "sum_down": [0],
            "ema_down_total": 0.0,
            "warm_down_total": false,
            "count_down_total": 0,
            "sum_down_total": 0,
            "ema_up": 0.0,
            "warm_up": false,
            "count_up": 0,
            "sum_up": 0,
            "ema_residual": 0.0,
            "warm_residual": false,
            "count_residual": 0,
            "sum_residual": 0
        }"#;

        assert!(estimator.import_state(invalid).is_err());
        assert_eq!(estimator.get_lead_us(ActionKind::Down, 1), lead_before);
        assert_eq!(estimator.max_poly, 6);
    }
}
