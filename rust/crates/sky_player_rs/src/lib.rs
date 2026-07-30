pub mod engine;

use engine::NativeDispatchSession;
use parking_lot::Mutex;
use pyo3::Borrowed;
use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyDict, PyList, PyTuple};
use sky_dispatch_core::estimator::SendLatencyEstimator;
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_win32::input::{PHYSICAL_INSTRUMENT_SCAN_CODES, TrackedKeyState};
use sky_dispatch_win32::mmcss::PriorityMode;
use std::collections::HashSet;
use std::sync::Arc;

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

#[pyclass(name = "DispatchSession", frozen)]
struct NativeDispatchSessionPy {
    session: Arc<NativeDispatchSession>,
}

#[pymethods]
impl NativeDispatchSessionPy {
    #[new]
    #[allow(clippy::too_many_arguments)]
    #[pyo3(signature = (py_actions, allowed_scan_codes, min_hold_us = StrictU64(50000), max_lead_us = StrictU64(2000), dispatch_lead_us = StrictU64(0), mock_backend = true, require_focus = false, focus_restore_grace_us = StrictU64(100000), spin_threshold_us = StrictU64(150), core_warmup_budget_us = StrictU64(200), late_pulse_drop_threshold_us = None, same_key_conflict_policy = "degraded", telemetry_enabled = false, telemetry_capacity = StrictU64(200000), rt_priority_mode = "auto", enable_waitable_timer = true, enable_event_wait = true, enable_adaptive_spin = false, enable_spin_reprobe = false, spin_floor_us = StrictU64(700), estimator_state_json = None, enable_adaptive_lead = false, input_path_warn_us = StrictU64(300)))]
    fn new(
        py_actions: &Bound<'_, PyAny>,
        allowed_scan_codes: &Bound<'_, PyAny>,
        min_hold_us: StrictU64,
        max_lead_us: StrictU64,
        dispatch_lead_us: StrictU64,
        mock_backend: bool,
        require_focus: bool,
        focus_restore_grace_us: StrictU64,
        spin_threshold_us: StrictU64,
        core_warmup_budget_us: StrictU64,
        late_pulse_drop_threshold_us: Option<StrictU64>,
        same_key_conflict_policy: &str,
        telemetry_enabled: bool,
        telemetry_capacity: StrictU64,
        rt_priority_mode: &str,
        enable_waitable_timer: bool,
        enable_event_wait: bool,
        enable_adaptive_spin: bool,
        enable_spin_reprobe: bool,
        spin_floor_us: StrictU64,
        estimator_state_json: Option<&str>,
        enable_adaptive_lead: bool,
        input_path_warn_us: StrictU64,
    ) -> PyResult<Self> {
        let min_hold_us = min_hold_us.0;
        let max_lead_us = max_lead_us.0;
        let dispatch_lead_us = dispatch_lead_us.0;
        let focus_restore_grace_us = focus_restore_grace_us.0;
        let spin_threshold_us = spin_threshold_us.0;
        let core_warmup_budget_us = core_warmup_budget_us.0;
        let late_pulse_drop_threshold_us = late_pulse_drop_threshold_us.map(|value| value.0);
        let telemetry_capacity = usize::try_from(telemetry_capacity.0)
            .map_err(|_| PyValueError::new_err("telemetry_capacity is too large"))?;
        let spin_floor_us = spin_floor_us.0;
        let input_path_warn_us = input_path_warn_us.0;
        if min_hold_us > 60_000_000 {
            return Err(PyValueError::new_err(
                "min_hold_us must be at most 60000000",
            ));
        }
        if max_lead_us > 10_000 {
            return Err(PyValueError::new_err("max_lead_us must be at most 10000"));
        }
        if dispatch_lead_us > 10_000 {
            return Err(PyValueError::new_err(
                "dispatch_lead_us must be at most 10000",
            ));
        }
        if focus_restore_grace_us > 10_000_000 {
            return Err(PyValueError::new_err(
                "focus_restore_grace_us must be at most 10000000",
            ));
        }
        if spin_threshold_us > 10_000 {
            return Err(PyValueError::new_err(
                "spin_threshold_us must be at most 10000",
            ));
        }
        if core_warmup_budget_us > 500 {
            return Err(PyValueError::new_err(
                "core_warmup_budget_us must be at most 500",
            ));
        }
        if spin_floor_us > 3_000 {
            return Err(PyValueError::new_err("spin_floor_us must be at most 3000"));
        }
        if input_path_warn_us > 60_000_000 {
            return Err(PyValueError::new_err(
                "input_path_warn_us must be at most 60000000",
            ));
        }
        if estimator_state_json.is_some_and(|raw| raw.len() > 64 * 1024) {
            return Err(PyValueError::new_err(
                "estimator_state_json must be at most 65536 bytes",
            ));
        }
        if late_pulse_drop_threshold_us.is_some_and(|threshold| threshold > 60_000_000) {
            return Err(PyValueError::new_err(
                "late_pulse_drop_threshold_us must be at most 60000000",
            ));
        }
        if telemetry_capacity == 0 || telemetry_capacity > 200_000 {
            return Err(PyValueError::new_err(
                "telemetry_capacity must be between 1 and 200000",
            ));
        }
        let strict_same_key_conflicts = match same_key_conflict_policy {
            "degraded" => false,
            "strict" => true,
            _ => {
                return Err(PyValueError::new_err(
                    "same_key_conflict_policy must be 'degraded' or 'strict'",
                ));
            }
        };
        let priority_mode = match rt_priority_mode {
            "auto" => PriorityMode::Auto,
            "mmcss" => PriorityMode::Mmcss,
            "time_critical" => PriorityMode::TimeCritical,
            "highest" => PriorityMode::Highest,
            "off" => PriorityMode::Off,
            _ => {
                return Err(PyValueError::new_err(
                    "rt_priority_mode must be auto, mmcss, time_critical, highest, or off",
                ));
            }
        };
        let allowed_scan_codes = parse_allowed_scan_codes(allowed_scan_codes)?;
        if let Some(raw) = estimator_state_json {
            let mut validator =
                SendLatencyEstimator::new(0.2, max_lead_us, allowed_scan_codes.len());
            validator.import_state(raw).map_err(PyValueError::new_err)?;
        }
        let actions = parse_actions(py_actions, &allowed_scan_codes)?;
        let schedule =
            sky_dispatch_core::compile::compile_runtime_intents(&actions, &allowed_scan_codes)
                .map_err(|error| PyValueError::new_err(error.to_string()))?;
        let session = NativeDispatchSession::new(
            schedule,
            min_hold_us,
            max_lead_us,
            dispatch_lead_us,
            allowed_scan_codes,
            mock_backend,
            require_focus,
            focus_restore_grace_us,
            spin_threshold_us,
            core_warmup_budget_us,
            late_pulse_drop_threshold_us,
            strict_same_key_conflicts,
            telemetry_enabled,
            telemetry_capacity,
            priority_mode,
            enable_waitable_timer,
            enable_event_wait,
            enable_adaptive_spin,
            enable_spin_reprobe,
            spin_floor_us,
            estimator_state_json.map(str::to_string),
            enable_adaptive_lead,
            input_path_warn_us,
        )
        .map_err(PyRuntimeError::new_err)?;

        Ok(Self {
            session: Arc::new(session),
        })
    }

    fn start(&self) -> PyResult<()> {
        self.session.start().map_err(PyRuntimeError::new_err)
    }

    fn pause(&self) -> PyResult<()> {
        self.session.pause().map_err(PyRuntimeError::new_err)
    }

    fn resume(&self) -> PyResult<()> {
        self.session.resume().map_err(PyRuntimeError::new_err)
    }

    fn skip(&self) -> PyResult<()> {
        self.session.skip().map_err(PyRuntimeError::new_err)
    }

    fn quit(&self) -> PyResult<()> {
        self.session.quit().map_err(PyRuntimeError::new_err)
    }

    fn panic_release(&self) -> PyResult<()> {
        self.session
            .panic_release()
            .map_err(PyRuntimeError::new_err)
    }

    fn send_command(&self, command: &str) -> PyResult<bool> {
        match command {
            "pause" => self.session.pause(),
            "resume" => self.session.resume(),
            "skip" => self.session.skip(),
            "quit" => self.session.quit(),
            "panic" => self.session.panic_release(),
            _ => {
                return Err(PyValueError::new_err(
                    "command must be pause, resume, skip, quit, or panic",
                ));
            }
        }
        .map_err(PyRuntimeError::new_err)?;
        Ok(true)
    }

    #[pyo3(signature = (active, hwnd = None))]
    fn update_focus(&self, active: bool, hwnd: Option<StrictU64>) -> PyResult<()> {
        if let Some(hwnd) = hwnd {
            let hwnd = isize::try_from(hwnd.0)
                .map_err(|_| PyValueError::new_err("hwnd is outside the platform range"))?;
            self.session.set_target_hwnd(hwnd);
        }
        self.session.update_focus(active);
        Ok(())
    }

    fn set_target_hwnd(&self, hwnd: StrictU64) -> PyResult<()> {
        let hwnd = isize::try_from(hwnd.0)
            .map_err(|_| PyValueError::new_err("hwnd is outside the platform range"))?;
        self.session.set_target_hwnd(hwnd);
        Ok(())
    }

    fn snapshot<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let snap = self.session.snapshot();
        let dict = PyDict::new(py);
        dict.set_item("version", 1)?;
        dict.set_item("native_build_version", env!("CARGO_PKG_VERSION"))?;
        dict.set_item("native_build_commit", env!("SKY_NATIVE_BUILD_COMMIT"))?;
        dict.set_item("rustc_version", env!("SKY_RUSTC_VERSION"))?;
        dict.set_item("pyo3_version", "0.29.0")?;
        dict.set_item("native_abi", env!("SKY_NATIVE_ABI"))?;
        dict.set_item("schema_version", sky_dispatch_core::SCHEMA_VERSION)?;
        dict.set_item("elapsed_us", snap.elapsed_us)?;
        dict.set_item("total_us", snap.total_us)?;
        dict.set_item("lateness_us", snap.lateness_us)?;
        dict.set_item("max_lateness_us", snap.max_lateness_us)?;
        dict.set_item("late_2ms", snap.late_2ms)?;
        dict.set_item("late_5ms", snap.late_5ms)?;
        dict.set_item("late_10ms", snap.late_10ms)?;
        dict.set_item("release_max_us", snap.release_max_us)?;
        dict.set_item("release_late_2ms", snap.release_late_2ms)?;
        dict.set_item("recent_latencies_us", snap.recent_latencies_us)?;
        dict.set_item("is_running", snap.is_running)?;
        dict.set_item("is_finished", snap.is_finished)?;
        dict.set_item("is_paused", snap.is_paused)?;
        dict.set_item("status", snap.status)?;
        dict.set_item("active_count", snap.active_count)?;
        dict.set_item("possibly_active_count", snap.possibly_active_count)?;
        dict.set_item("failed_release_count", snap.failed_release_count)?;
        dict.set_item("last_error", snap.last_error)?;
        dict.set_item("keys_dropped", snap.keys_dropped)?;
        dict.set_item("chord_split_events", snap.chord_split_events)?;
        dict.set_item("outcome", snap.outcome)?;
        dict.set_item("rt_priority_acquired", snap.rt_priority_acquired)?;
        dict.set_item(
            "effective_spin_threshold_us",
            snap.effective_spin_threshold_us,
        )?;
        dict.set_item("wait_strategy_acquired", snap.wait_strategy_acquired)?;
        dict.set_item("power_throttling_disabled", snap.power_throttling_disabled)?;
        dict.set_item("input_path_degraded", snap.input_path_degraded)?;
        dict.set_item("idle_wake_count", snap.idle_wake_count)?;
        dict.set_item("terminal_error", snap.terminal_error)?;
        dict.set_item("generation_count", snap.generation_count)?;
        dict.set_item("generation_status_counts", snap.generation_status_counts)?;
        dict.set_item("abort_counts_by_reason", snap.abort_counts_by_reason)?;
        if let Some(outcome) = snap.release_outcome {
            let release = PyDict::new(py);
            release.set_item("attempted", outcome.attempted)?;
            release.set_item("released_successfully", outcome.released_successfully)?;
            release.set_item("stuck_keys", outcome.stuck_keys)?;
            release.set_item(
                "verification_inconclusive",
                outcome.verification_inconclusive,
            )?;
            dict.set_item("release_outcome", release)?;
        } else {
            dict.set_item("release_outcome", py.None())?;
        }
        Ok(dict)
    }

    fn try_result<'py>(&self, py: Python<'py>) -> PyResult<Option<Bound<'py, PyDict>>> {
        let Some(outcome) = self.session.terminal_outcome() else {
            return Ok(None);
        };
        let result = PyDict::new(py);
        result.set_item("outcome", outcome)?;
        result.set_item("snapshot", self.snapshot(py)?)?;
        Ok(Some(result))
    }

    #[pyo3(signature = (timeout_ms = StrictU64(5000)))]
    fn join(&self, py: Python<'_>, timeout_ms: StrictU64) -> PyResult<bool> {
        let timeout_ms = timeout_ms.0;
        if timeout_ms == 0 || timeout_ms > 60_000 {
            return Err(PyValueError::new_err(
                "timeout_ms must be between 1 and 60000",
            ));
        }
        py.detach(|| {
            self.session
                .join(std::time::Duration::from_millis(timeout_ms))
        })
        .map_err(PyRuntimeError::new_err)
    }

    fn take_telemetry_json(&self, py: Python<'_>) -> PyResult<String> {
        py.detach(|| self.session.take_telemetry_json())
            .map_err(PyRuntimeError::new_err)
    }

    fn estimator_state_json(&self) -> PyResult<String> {
        self.session
            .estimator_state_json()
            .map_err(PyRuntimeError::new_err)
    }
}

#[pyfunction]
fn build_info<'py>(py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("rust_core_enabled", true)?;
    dict.set_item("version", env!("CARGO_PKG_VERSION"))?;
    dict.set_item("rust_core_version", env!("CARGO_PKG_VERSION"))?;
    dict.set_item("rustc_version", env!("SKY_RUSTC_VERSION"))?;
    dict.set_item("schema_version", sky_dispatch_core::SCHEMA_VERSION)?;
    dict.set_item("native_schema_version", sky_dispatch_core::SCHEMA_VERSION)?;
    dict.set_item("pyo3_version", "0.29.0")?;
    dict.set_item("native_abi", env!("SKY_NATIVE_ABI"))?;
    dict.set_item("native_build_commit", env!("SKY_NATIVE_BUILD_COMMIT"))?;
    dict.set_item("free_threaded", true)?;
    dict.set_item("win32_backend", sky_dispatch_win32::win32_available())?;
    Ok(dict)
}

#[pyfunction]
fn simulate_schedule_rs(
    py_actions: &Bound<'_, PyAny>,
    allowed_scan_codes: &Bound<'_, PyAny>,
    min_hold_us: StrictU64,
    send_latency_us: StrictU64,
) -> PyResult<String> {
    let allowed_scan_codes = strict_scan_codes(allowed_scan_codes, "allowed_scan_codes", None)?;
    let actions = parse_actions(py_actions, &allowed_scan_codes)?;

    let result = sky_dispatch_core::testing::simulate_schedule(
        &actions,
        &allowed_scan_codes,
        min_hold_us.0,
        send_latency_us.0,
    )
    .map_err(|error| PyValueError::new_err(error.to_string()))?;

    let json = serde_json::to_string(&result)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(e.to_string()))?;

    Ok(json)
}

#[pyfunction]
fn sleep_until_rs(target_us: StrictU64, spin_margin_us: StrictU64) -> u64 {
    sky_dispatch_win32::sleeper::sleep_until_us(target_us.0, spin_margin_us.0)
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
    let dispatch_session = m.getattr("DispatchSession")?;
    m.add("NativeDispatchSessionPy", dispatch_session)?;
    m.add_function(wrap_pyfunction!(build_info, m)?)?;
    m.add_function(wrap_pyfunction!(simulate_schedule_rs, m)?)?;
    m.add_function(wrap_pyfunction!(sleep_until_rs, m)?)?;
    m.add_function(wrap_pyfunction!(measure_spin_overhead_rs, m)?)?;
    m.add_function(wrap_pyfunction!(qpc_now_rs, m)?)?;
    Ok(())
}
