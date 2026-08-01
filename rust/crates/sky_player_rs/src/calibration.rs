//! PyO3 wrapper exposing the native calibration harness to Python.
//!
//! This module is not part of the real-time dispatch path.  It is a separate
//! tool surface that blocks the calling thread while collecting Raw Input
//! delivery samples.  Python callers should invoke it from a worker thread or
//! subprocess to avoid blocking the UI.
//!
//! # Evidence scope
//!
//! All data produced here is **injected Raw Input delivery proxy** latency.
//! It is **not** Sky/game-observed latency.  The `evidence_kind` field in the
//! JSON output says so explicitly.

use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use sky_dispatch_win32::calibration::{CalibrationConfig, CalibrationError, run_calibration_json};

/// Run the native chord-aware calibration harness.
///
/// Returns a JSON string matching the output schema described in P1.4.
///
/// Parameters
/// ----------
/// polyphonies : list[int]
///     Chord sizes to probe. Each value must be between 1 and 15.
///     Default: [1, 2, 3, 5, 8, 15].
/// samples_per_bucket : int
///     Number of counted samples per (kind × polyphony × class) bucket.
///     Default: 500 (quick), 5000 (full).
/// warmup_samples : int
///     Warm-up injections before counting begins (marked Cold). Default: 20.
/// receipt_timeout_ms : int
///     Milliseconds to wait for Raw Input receipts per packet. Default: 200.
/// inter_sample_gap_us : int
///     Microseconds to sleep between samples. Default: 5000.
/// mode : str
///     "quick" (200 samples), "full" (5000 samples), or "custom" (use the
///     explicit parameters above). Default: "quick".
///
/// Raises
/// ------
/// RuntimeError
///     On platform error, window creation failure, or JSON serialisation error.
#[pyfunction]
#[pyo3(
    signature = (
        polyphonies = None,
        samples_per_bucket = None,
        warmup_samples = None,
        receipt_timeout_ms = None,
        inter_sample_gap_us = None,
        mode = None,
    )
)]
pub fn run_calibration_rs(
    py: Python<'_>,
    polyphonies: Option<Vec<u8>>,
    samples_per_bucket: Option<u32>,
    warmup_samples: Option<u32>,
    receipt_timeout_ms: Option<u32>,
    inter_sample_gap_us: Option<u64>,
    mode: Option<&str>,
) -> PyResult<String> {
    // Build base config from mode.
    let mut config = match mode.unwrap_or("quick") {
        "full" => CalibrationConfig::full(),
        "custom" => CalibrationConfig::default(),
        _ => CalibrationConfig::quick(), // "quick" and unknown → quick
    };

    if let Some(polys) = polyphonies {
        if polys.is_empty() {
            return Err(PyRuntimeError::new_err("polyphonies must not be empty"));
        }
        for &p in &polys {
            if p == 0 || p > 15 {
                return Err(PyRuntimeError::new_err(format!(
                    "polyphony {p} is out of range 1..=15"
                )));
            }
        }
        config.polyphonies = polys;
    }
    if let Some(s) = samples_per_bucket {
        if s == 0 {
            return Err(PyRuntimeError::new_err(
                "samples_per_bucket must be at least 1",
            ));
        }
        config.samples_per_bucket = s;
    }
    if let Some(w) = warmup_samples {
        config.warmup_samples = w;
    }
    if let Some(t) = receipt_timeout_ms {
        if t == 0 {
            return Err(PyRuntimeError::new_err(
                "receipt_timeout_ms must be at least 1",
            ));
        }
        config.receipt_timeout_ms = t;
    }
    if let Some(g) = inter_sample_gap_us {
        config.inter_sample_gap_us = g;
    }

    // Run the calibration on the calling thread. The Python GIL is released
    // for the entire blocking duration so other Python threads can progress.
    py.detach(|| run_calibration_json(&config))
        .map_err(|e| match e {
            CalibrationError::PlatformUnsupported => {
                PyRuntimeError::new_err("calibration is not supported on this platform")
            }
            other => PyRuntimeError::new_err(other.to_string()),
        })
}
