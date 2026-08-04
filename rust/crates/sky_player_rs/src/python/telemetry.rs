use super::{Bound, PyDict, PyResult, PyRuntimeError, Python};
use pyo3::pyfunction;
use pyo3::types::PyDictMethods;

#[pyfunction]
pub(super) fn build_info<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("rust_core_enabled", true)?;
    dict.set_item("version", env!("CARGO_PKG_VERSION"))?;
    dict.set_item("rust_core_version", env!("CARGO_PKG_VERSION"))?;
    dict.set_item("rustc_version", env!("SKY_RUSTC_VERSION"))?;
    dict.set_item("schema_version", sky_dispatch_core::SCHEMA_VERSION)?;
    dict.set_item("native_schema_version", sky_dispatch_core::SCHEMA_VERSION)?;
    #[cfg(feature = "calibration")]
    dict.set_item(
        "calibration_schema_version",
        sky_dispatch_win32::calibration::CALIBRATION_SCHEMA_VERSION,
    )?;
    dict.set_item("pyo3_version", "0.29.0")?;
    dict.set_item("native_abi", env!("SKY_NATIVE_ABI"))?;
    dict.set_item(
        "qpc_frequency_hz",
        sky_dispatch_win32::clock::qpc_frequency_checked().map_err(|error| {
            PyRuntimeError::new_err(format!("QPC frequency unavailable: {error:?}"))
        })?,
    )?;
    dict.set_item("native_build_commit", env!("SKY_NATIVE_BUILD_COMMIT"))?;
    dict.set_item("free_threaded", true)?;
    dict.set_item("win32_backend", sky_dispatch_win32::win32_available())?;
    Ok(dict)
}
