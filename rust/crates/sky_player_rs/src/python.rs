#[cfg(feature = "test-support")]
use super::engine::FaultInjectionScript;
use super::engine::{BackendConfig, DispatchProfile, NativeDispatchSession};

mod conversion;
mod session;
mod telemetry;

use pyo3::Borrowed;
use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyDict, PyList, PyTuple};
use session::NativeDispatchSessionPy;
#[cfg(feature = "test-support")]
use session::TestDispatchSessionPy;
use sky_dispatch_core::model::{ActionKind, KeyActionInput, RuntimeSchedule};
use sky_dispatch_win32::input::PHYSICAL_INSTRUMENT_SCAN_CODES;
use sky_dispatch_win32::mmcss::PriorityMode;
use std::sync::Arc;
use telemetry::build_info;

#[cfg(feature = "calibration")]
use super::calibration;

#[derive(Clone, Copy)]
struct StrictU64(u64);

impl<'a, 'py> FromPyObject<'a, 'py> for StrictU64 {
    type Error = PyErr;

    fn extract(obj: Borrowed<'a, 'py, PyAny>) -> Result<Self, Self::Error> {
        if obj.is_instance_of::<PyBool>() {
            return Err(PyTypeError::new_err("expected integer, not bool"));
        }
        let value = obj
            .extract::<i128>()
            .map_err(|_| PyTypeError::new_err("expected integer"))?;
        let value = u64::try_from(value)
            .map_err(|_| PyValueError::new_err("integer must be in 0..=u64::MAX"))?;
        Ok(Self(value))
    }
}

#[pyclass(name = "SessionConfig", frozen, from_py_object)]
#[derive(Clone, Copy)]
struct NativeSessionConfigPy {
    min_hold_us: u64,
    require_focus: bool,
    target_hwnd: isize,
    telemetry: bool,
    profile: DispatchProfile,
}

impl Default for NativeSessionConfigPy {
    fn default() -> Self {
        Self {
            min_hold_us: 50_000,
            require_focus: false,
            target_hwnd: 0,
            telemetry: false,
            profile: DispatchProfile::Production,
        }
    }
}

#[pymethods]
impl NativeSessionConfigPy {
    #[new]
    #[pyo3(signature = (
        min_hold_us = StrictU64(50000),
        require_focus = false,
        target_hwnd = StrictU64(0),
        telemetry = false,
        profile = "production"
    ))]
    fn new(
        min_hold_us: StrictU64,
        require_focus: bool,
        target_hwnd: StrictU64,
        telemetry: bool,
        profile: &str,
    ) -> PyResult<Self> {
        let target_hwnd = isize::try_from(target_hwnd.0)
            .map_err(|_| PyValueError::new_err("target_hwnd is outside the platform range"))?;
        let profile = DispatchProfile::parse(profile).map_err(PyValueError::new_err)?;
        #[cfg(any(test, feature = "test-support"))]
        if profile == DispatchProfile::MockTest {
            return Err(PyValueError::new_err(
                "mock_test is available only to Rust test support",
            ));
        }
        if min_hold_us.0 > 60_000_000 {
            return Err(PyValueError::new_err(
                "min_hold_us must be at most 60000000",
            ));
        }
        Ok(Self {
            min_hold_us: min_hold_us.0,
            require_focus,
            target_hwnd,
            telemetry,
            profile,
        })
    }

    #[getter]
    fn min_hold_us(&self) -> u64 {
        self.min_hold_us
    }

    #[getter]
    fn require_focus(&self) -> bool {
        self.require_focus
    }

    #[getter]
    fn target_hwnd(&self) -> isize {
        self.target_hwnd
    }

    #[getter]
    fn telemetry(&self) -> bool {
        self.telemetry
    }

    #[getter]
    fn profile(&self) -> &'static str {
        match self.profile {
            DispatchProfile::Production => "production",
            DispatchProfile::StrictTimingDiagnostic => "strict_timing_diagnostic",
            #[cfg(any(test, feature = "test-support"))]
            DispatchProfile::MockTest => "mock_test",
        }
    }
}

pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<NativeSessionConfigPy>()?;
    m.add_class::<NativeDispatchSessionPy>()?;
    #[cfg(feature = "test-support")]
    m.add_class::<TestDispatchSessionPy>()?;
    m.add_function(wrap_pyfunction!(build_info, m)?)?;
    #[cfg(feature = "calibration")]
    {
        m.add_function(wrap_pyfunction!(calibration::run_calibration_rs, m)?)?;
        m.add_function(wrap_pyfunction!(
            calibration::calibration_schema_version,
            m
        )?)?;
    }
    Ok(())
}
