pub mod engine;

use engine::NativeDispatchSession;
use parking_lot::Mutex;
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyDict, PyList, PyTuple};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_win32::input::{PHYSICAL_INSTRUMENT_SCAN_CODES, TrackedKeyState};
use std::collections::HashSet;
use std::sync::Arc;

fn strict_sequence<'py>(
    value: &Bound<'py, PyAny>,
    field: &str,
) -> PyResult<Vec<Bound<'py, PyAny>>> {
    if !value.is_instance_of::<PyList>() && !value.is_instance_of::<PyTuple>() {
        return Err(PyTypeError::new_err(format!(
            "{field} must be a list or tuple"
        )));
    }
    value.try_iter()?.collect()
}

fn strict_integer(value: &Bound<'_, PyAny>, field: &str) -> PyResult<i128> {
    if value.is_instance_of::<PyBool>() {
        return Err(PyTypeError::new_err(format!(
            "{field} must be an integer, not bool"
        )));
    }
    value
        .extract::<i128>()
        .map_err(|_| PyTypeError::new_err(format!("{field} must be an integer")))
}

fn strict_u32(value: &Bound<'_, PyAny>, field: &str) -> PyResult<u32> {
    let integer = strict_integer(value, field)?;
    u32::try_from(integer)
        .map_err(|_| PyValueError::new_err(format!("{field} must be in 0..=u32::MAX")))
}

fn strict_u64(value: &Bound<'_, PyAny>, field: &str) -> PyResult<u64> {
    let integer = strict_integer(value, field)?;
    u64::try_from(integer)
        .map_err(|_| PyValueError::new_err(format!("{field} must be in 0..=u64::MAX")))
}

fn strict_scan_codes(
    value: &Bound<'_, PyAny>,
    field: &str,
    allowed: Option<&HashSet<u16>>,
) -> PyResult<Vec<u16>> {
    let items = strict_sequence(value, field)?;
    if items.is_empty() || items.len() > sky_dispatch_core::model::MAX_KEYS {
        return Err(PyValueError::new_err(format!(
            "{field} must contain between 1 and {} scan codes",
            sky_dispatch_core::model::MAX_KEYS
        )));
    }

    let mut result = Vec::with_capacity(items.len());
    let mut seen = HashSet::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let item_field = format!("{field}[{index}]");
        let integer = strict_integer(item, &item_field)?;
        let scan_code = u16::try_from(integer)
            .map_err(|_| PyValueError::new_err(format!("{item_field} must be in 0..=u16::MAX")))?;
        if !seen.insert(scan_code) {
            return Err(PyValueError::new_err(format!(
                "{field} contains duplicate scan code {scan_code}"
            )));
        }
        if let Some(allowed) = allowed
            && !allowed.contains(&scan_code)
        {
            return Err(PyValueError::new_err(format!(
                "{item_field} scan code {scan_code} is outside the prepared allowlist"
            )));
        }
        result.push(scan_code);
    }
    Ok(result)
}

fn parse_allowed_scan_codes(value: &Bound<'_, PyAny>) -> PyResult<Vec<u16>> {
    let instrument: HashSet<u16> = PHYSICAL_INSTRUMENT_SCAN_CODES.iter().copied().collect();
    strict_scan_codes(value, "allowed_scan_codes", Some(&instrument))
}

fn parse_actions(
    value: &Bound<'_, PyAny>,
    allowed_scan_codes: &[u16],
) -> PyResult<Vec<KeyActionInput>> {
    let items = strict_sequence(value, "actions")?;
    if items.len() > sky_dispatch_core::compile::MAX_ACTIONS {
        return Err(PyValueError::new_err(format!(
            "actions exceeds the configured cap of {}",
            sky_dispatch_core::compile::MAX_ACTIONS
        )));
    }
    let allowed: HashSet<u16> = allowed_scan_codes.iter().copied().collect();
    let mut actions = Vec::with_capacity(items.len());

    for (position, item) in items.iter().enumerate() {
        let tuple = item.cast::<PyTuple>().map_err(|_| {
            PyTypeError::new_err(format!(
                "actions[{position}] must be a 5-item tuple \
                 (source_action_index, kind, at_us, scan_codes, reason)"
            ))
        })?;
        if tuple.len() != 5 {
            return Err(PyValueError::new_err(format!(
                "actions[{position}] must contain exactly 5 items"
            )));
        }

        let source_action_index = strict_u32(
            &tuple.get_item(0)?,
            &format!("actions[{position}].source_action_index"),
        )?;
        let kind_string = tuple
            .get_item(1)?
            .extract::<String>()
            .map_err(|_| PyTypeError::new_err(format!("actions[{position}].kind must be str")))?;
        let kind = match kind_string.as_str() {
            "down" => ActionKind::Down,
            "up" => ActionKind::Up,
            _ => {
                return Err(PyValueError::new_err(format!(
                    "actions[{position}].kind must be exactly 'down' or 'up'"
                )));
            }
        };
        let scheduled_us = strict_u64(&tuple.get_item(2)?, &format!("actions[{position}].at_us"))?;
        let scan_codes = strict_scan_codes(
            &tuple.get_item(3)?,
            &format!("actions[{position}].scan_codes"),
            Some(&allowed),
        )?;
        let reason = tuple
            .get_item(4)?
            .extract::<String>()
            .map_err(|_| PyTypeError::new_err(format!("actions[{position}].reason must be str")))?;
        if reason.len() > sky_dispatch_core::compile::MAX_REASON_BYTES {
            return Err(PyValueError::new_err(format!(
                "actions[{position}].reason exceeds {} UTF-8 bytes",
                sky_dispatch_core::compile::MAX_REASON_BYTES
            )));
        }

        actions.push(KeyActionInput {
            source_action_index,
            kind,
            scheduled_us,
            scan_codes,
            reason,
        });
    }
    Ok(actions)
}

#[pyclass(frozen)]
struct RustInputBackend {
    state: Arc<Mutex<TrackedKeyState>>,
}

#[pymethods]
impl RustInputBackend {
    #[new]
    #[pyo3(signature = (mock = false))]
    fn new(mock: bool) -> Self {
        let state = if mock {
            sky_dispatch_win32::input::TrackedKeyState::with_emitter(|scan_codes, _key_up| {
                sky_dispatch_win32::input::PlatformSendResult {
                    requested: scan_codes.len() as u32,
                    inserted: scan_codes.len() as u32,
                    completed_us: sky_dispatch_win32::clock::qpc_now_us(),
                    win32_error: 0,
                }
            })
        } else {
            sky_dispatch_win32::input::TrackedKeyState::new()
        };
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    fn key_down<'py>(
        &self,
        py: Python<'py>,
        scan_codes: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let allowed: HashSet<u16> = PHYSICAL_INSTRUMENT_SCAN_CODES.iter().copied().collect();
        let scan_codes = strict_scan_codes(scan_codes, "scan_codes", Some(&allowed))?;
        let res = self.state.lock().key_down(&scan_codes);
        let dict = PyDict::new(py);
        dict.set_item("sent", res.sent.to_vec())?;
        dict.set_item("skipped_duplicates", res.skipped_duplicates.to_vec())?;
        dict.set_item("success", res.success)?;
        dict.set_item("error", res.error)?;
        dict.set_item("send_completed_us", res.send_completed_us)?;
        Ok(dict)
    }

    fn key_up<'py>(
        &self,
        py: Python<'py>,
        scan_codes: &Bound<'_, PyAny>,
    ) -> PyResult<Bound<'py, PyDict>> {
        let allowed: HashSet<u16> = PHYSICAL_INSTRUMENT_SCAN_CODES.iter().copied().collect();
        let scan_codes = strict_scan_codes(scan_codes, "scan_codes", Some(&allowed))?;
        let res = self.state.lock().key_up(&scan_codes);
        let dict = PyDict::new(py);
        dict.set_item("sent", res.sent.to_vec())?;
        dict.set_item("skipped_duplicates", res.skipped_duplicates.to_vec())?;
        dict.set_item("success", res.success)?;
        dict.set_item("error", res.error)?;
        dict.set_item("send_completed_us", res.send_completed_us)?;
        Ok(dict)
    }

    fn release_all<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let outcome = self.state.lock().release_all();
        let dict = PyDict::new(py);
        dict.set_item("attempted", outcome.attempted)?;
        dict.set_item("released_successfully", outcome.released_successfully)?;
        dict.set_item("stuck_keys", outcome.stuck_keys)?;
        dict.set_item(
            "verification_inconclusive",
            outcome.verification_inconclusive,
        )?;
        Ok(dict)
    }

    fn release_all_full_instrument<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let outcome = self.state.lock().release_all_full_instrument();
        let dict = PyDict::new(py);
        dict.set_item("attempted", outcome.attempted)?;
        dict.set_item("released_successfully", outcome.released_successfully)?;
        dict.set_item("stuck_keys", outcome.stuck_keys)?;
        dict.set_item(
            "verification_inconclusive",
            outcome.verification_inconclusive,
        )?;
        Ok(dict)
    }

    fn get_health<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let state = self.state.lock();
        let dict = PyDict::new(py);
        dict.set_item("active_count", state.active_keys.len())?;
        dict.set_item("possibly_active_count", state.possibly_active_keys.len())?;
        dict.set_item("failed_release_count", state.failed_release_keys.len())?;
        dict.set_item("last_error", state.last_error.clone())?;
        dict.set_item("keys_dropped", state.keys_dropped)?;
        dict.set_item("chord_split_events", state.chord_split_events)?;
        Ok(dict)
    }
}

#[pyclass(frozen)]
struct NativeDispatchSessionPy {
    session: Arc<NativeDispatchSession>,
}

#[pymethods]
impl NativeDispatchSessionPy {
    #[new]
    #[pyo3(signature = (py_actions, allowed_scan_codes, min_hold_us = 50000, max_lead_us = 2000, mock_backend = true))]
    fn new(
        py_actions: &Bound<'_, PyAny>,
        allowed_scan_codes: &Bound<'_, PyAny>,
        min_hold_us: u64,
        max_lead_us: u64,
        mock_backend: bool,
    ) -> PyResult<Self> {
        let allowed_scan_codes = parse_allowed_scan_codes(allowed_scan_codes)?;
        let actions = parse_actions(py_actions, &allowed_scan_codes)?;
        let schedule =
            sky_dispatch_core::compile::compile_runtime_intents(&actions, &allowed_scan_codes)
                .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let session = NativeDispatchSession::new(
            schedule,
            min_hold_us,
            max_lead_us,
            allowed_scan_codes,
            mock_backend,
        );

        Ok(Self {
            session: Arc::new(session),
        })
    }

    fn start(&self) {
        self.session.start();
    }

    fn pause(&self) {
        self.session.pause();
    }

    fn resume(&self) {
        self.session.resume();
    }

    fn skip(&self) {
        self.session.skip();
    }

    fn quit(&self) {
        self.session.quit();
    }

    fn snapshot<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let snap = self.session.snapshot();
        let dict = PyDict::new(py);
        dict.set_item("elapsed_us", snap.elapsed_us)?;
        dict.set_item("total_us", snap.total_us)?;
        dict.set_item("lateness_us", snap.lateness_us)?;
        dict.set_item("max_lateness_us", snap.max_lateness_us)?;
        dict.set_item("late_2ms", snap.late_2ms)?;
        dict.set_item("late_5ms", snap.late_5ms)?;
        dict.set_item("late_10ms", snap.late_10ms)?;
        dict.set_item("is_running", snap.is_running)?;
        dict.set_item("is_finished", snap.is_finished)?;
        dict.set_item("is_paused", snap.is_paused)?;
        dict.set_item("status", snap.status)?;
        dict.set_item("active_count", snap.active_count)?;
        dict.set_item("keys_dropped", snap.keys_dropped)?;
        dict.set_item("chord_split_events", snap.chord_split_events)?;
        Ok(dict)
    }

    fn join(&self) {
        self.session.join();
    }
}

#[pyfunction]
fn build_info<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("version", env!("CARGO_PKG_VERSION"))?;
    dict.set_item("schema_version", sky_dispatch_core::SCHEMA_VERSION)?;
    dict.set_item("pyo3_version", "0.29.0")?;
    dict.set_item("free_threaded", true)?;
    dict.set_item("win32_backend", sky_dispatch_win32::win32_available())?;
    Ok(dict)
}

#[pyfunction]
fn simulate_schedule_rs(
    py_actions: &Bound<'_, PyAny>,
    allowed_scan_codes: &Bound<'_, PyAny>,
    min_hold_us: u64,
    send_latency_us: u64,
) -> PyResult<String> {
    let allowed_scan_codes = strict_scan_codes(allowed_scan_codes, "allowed_scan_codes", None)?;
    let actions = parse_actions(py_actions, &allowed_scan_codes)?;

    let result = sky_dispatch_core::testing::simulate_schedule(
        &actions,
        &allowed_scan_codes,
        min_hold_us,
        send_latency_us,
    )
    .map_err(|error| PyValueError::new_err(error.to_string()))?;

    let json = serde_json::to_string(&result)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

    Ok(json)
}

#[pyfunction]
fn sleep_until_rs(target_us: u64, spin_margin_us: u64) -> u64 {
    sky_dispatch_win32::sleeper::sleep_until_us(target_us, spin_margin_us)
}

#[pyfunction]
fn measure_spin_overhead_rs() -> u64 {
    sky_dispatch_win32::sleeper::measure_spin_overhead_us()
}

#[pyfunction]
fn qpc_now_rs() -> u64 {
    sky_dispatch_win32::clock::qpc_now_us()
}

/// Free-threaded PyO3 extension module for Sky Auto Player dispatch engine.
#[pyo3::pymodule(gil_used = false)]
fn sky_player_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<RustInputBackend>()?;
    m.add_class::<NativeDispatchSessionPy>()?;
    m.add_function(wrap_pyfunction!(build_info, m)?)?;
    m.add_function(wrap_pyfunction!(simulate_schedule_rs, m)?)?;
    m.add_function(wrap_pyfunction!(sleep_until_rs, m)?)?;
    m.add_function(wrap_pyfunction!(measure_spin_overhead_rs, m)?)?;
    m.add_function(wrap_pyfunction!(qpc_now_rs, m)?)?;
    Ok(())
}
