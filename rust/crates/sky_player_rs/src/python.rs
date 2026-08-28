#[cfg(feature = "test-support")]
use super::engine::FaultInjectionScript;
use super::engine::{BackendConfig, DispatchProfile, NativeDispatchSession};

mod conversion;
mod session;
mod snapshot;
mod telemetry;
#[cfg(feature = "test-support")]
mod test_session;

use pyo3::Borrowed;
use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyDict, PyList, PyTuple};
use session::NativeDispatchSessionPy;
use sky_dispatch_core::model::{ActionKind, KeyActionInput, RuntimeSchedule};
use sky_dispatch_win32::input::PHYSICAL_INSTRUMENT_SCAN_CODES;
use sky_dispatch_win32::mmcss::PriorityMode;
use snapshot::{BackendHealthSnapshotPy, PollSnapshotPy, ProgressSnapshotPy};
use std::sync::Arc;
use telemetry::build_info;
#[cfg(feature = "test-support")]
use test_session::TestDispatchSessionPy;

#[pyfunction]
fn instrument_scan_codes() -> Vec<u16> {
    PHYSICAL_INSTRUMENT_SCAN_CODES.to_vec()
}

#[pyfunction]
fn host_timing_fingerprint_json() -> PyResult<String> {
    let fingerprint = sky_dispatch_win32::calibration::build_host_fingerprint()
        .map_err(|error| PyRuntimeError::new_err(error.to_string()))?;
    serde_json::to_string(&fingerprint).map_err(|error| {
        PyRuntimeError::new_err(format!("could not serialize host fingerprint: {error}"))
    })
}

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
#[derive(Clone)]
struct NativeSessionConfigPy {
    game_fps: u16,
    min_hold_us: u64,
    min_release_gap_us: u64,
    down_late_grace_us: u64,
    require_focus: bool,
    focus_restore_grace_us: u64,
    target_hwnd: isize,
    telemetry: bool,
    profile: DispatchProfile,
}

impl Default for NativeSessionConfigPy {
    fn default() -> Self {
        Self {
            game_fps: 60,
            min_hold_us: 50_000,
            min_release_gap_us: 1_000_000u64.div_ceil(60),
            down_late_grace_us: 500,
            require_focus: false,
            focus_restore_grace_us: 100_000,
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
        game_fps,
        min_hold_us = StrictU64(50000),
        down_late_grace_us = StrictU64(500),
        require_focus = false,
        focus_restore_grace_us = StrictU64(100000),
        target_hwnd = StrictU64(0),
        telemetry = false,
        profile = "production",
        min_release_gap_us = None
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        game_fps: StrictU64,
        min_hold_us: StrictU64,
        down_late_grace_us: StrictU64,
        require_focus: bool,
        focus_restore_grace_us: StrictU64,
        target_hwnd: StrictU64,
        telemetry: bool,
        profile: &str,
        min_release_gap_us: Option<StrictU64>,
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
        if down_late_grace_us.0 > min_hold_us.0 {
            return Err(PyValueError::new_err(
                "down_late_grace_us must not exceed min_hold_us",
            ));
        }
        if focus_restore_grace_us.0 > 60_000_000 {
            return Err(PyValueError::new_err(
                "focus_restore_grace_us must be at most 60000000",
            ));
        }
        let game_fps = u16::try_from(game_fps.0)
            .map_err(|_| PyValueError::new_err("game_fps must be an integer in 15..=240"))?;
        if !(15..=240).contains(&game_fps) {
            return Err(PyValueError::new_err("game_fps must be in 15..=240"));
        }
        let frame_us = 1_000_000u64.div_ceil(u64::from(game_fps));
        let min_release_gap_us = min_release_gap_us.map(|value| value.0).unwrap_or(frame_us);
        if min_release_gap_us < frame_us || min_release_gap_us > 60_000_000 {
            return Err(PyValueError::new_err(format!(
                "min_release_gap_us must be in {frame_us}..=60000000"
            )));
        }
        Ok(Self {
            game_fps,
            min_hold_us: min_hold_us.0,
            min_release_gap_us,
            down_late_grace_us: down_late_grace_us.0,
            require_focus,
            focus_restore_grace_us: focus_restore_grace_us.0,
            target_hwnd,
            telemetry,
            profile,
        })
    }

    #[getter]
    fn game_fps(&self) -> u16 {
        self.game_fps
    }

    #[getter]
    fn min_hold_us(&self) -> u64 {
        self.min_hold_us
    }

    #[getter]
    fn min_release_gap_us(&self) -> u64 {
        self.min_release_gap_us
    }

    #[getter]
    fn down_late_grace_us(&self) -> u64 {
        self.down_late_grace_us
    }

    #[getter]
    fn require_focus(&self) -> bool {
        self.require_focus
    }

    #[getter]
    fn focus_restore_grace_us(&self) -> u64 {
        self.focus_restore_grace_us
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
    m.add_class::<BackendHealthSnapshotPy>()?;
    m.add_class::<PollSnapshotPy>()?;
    m.add_class::<ProgressSnapshotPy>()?;
    #[cfg(feature = "test-support")]
    m.add_class::<TestDispatchSessionPy>()?;
    m.add_function(wrap_pyfunction!(build_info, m)?)?;
    m.add_function(wrap_pyfunction!(instrument_scan_codes, m)?)?;
    m.add_function(wrap_pyfunction!(host_timing_fingerprint_json, m)?)?;
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

#[cfg(test)]
mod tests {
    use super::NativeSessionConfigPy;

    #[test]
    fn production_session_config_defaults_to_five_hundred_us_grace() {
        let config = NativeSessionConfigPy::default();
        assert_eq!(config.down_late_grace_us, 500);
        assert_eq!(config.min_release_gap_us, 16_667);
    }
}
