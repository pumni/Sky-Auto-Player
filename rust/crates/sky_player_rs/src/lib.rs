#[cfg(feature = "calibration")]
pub mod calibration;
pub mod engine;
mod python;

use pyo3::prelude::*;

#[pyo3::pymodule(gil_used = false)]
fn sky_player_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    python::register(m)
}
