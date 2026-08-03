#[cfg(feature = "calibration")]
pub mod calibration;
pub mod engine;

use engine::{DispatchProfile, FaultInjectionScript, NativeDispatchSession};
use pyo3::Borrowed;
use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyDict, PyList, PyTuple};
use sky_dispatch_core::model::{ActionKind, KeyActionInput, RuntimeSchedule};
use sky_dispatch_win32::input::PHYSICAL_INSTRUMENT_SCAN_CODES;
use sky_dispatch_win32::mmcss::PriorityMode;
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
            DispatchProfile::MockTest => "mock_test",
        }
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
    allowed: Option<&[u16]>,
) -> PyResult<smallvec::SmallVec<[u16; 4]>> {
    let items = strict_sequence(value, field)?;
    if items.is_empty() || items.len() > sky_dispatch_core::model::MAX_KEYS {
        return Err(PyValueError::new_err(format!(
            "{field} must contain between 1 and {} scan codes",
            sky_dispatch_core::model::MAX_KEYS
        )));
    }

    let mut result = smallvec::SmallVec::with_capacity(items.len());
    let mut seen = smallvec::SmallVec::<[u16; 15]>::new();
    for (index, item) in items.iter().enumerate() {
        let item_field = format!("{field}[{index}]");
        let integer = strict_integer(item, &item_field)?;
        let scan_code = u16::try_from(integer)
            .map_err(|_| PyValueError::new_err(format!("{item_field} must be in 0..=u16::MAX")))?;
        if seen.contains(&scan_code) {
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
        seen.push(scan_code);
        result.push(scan_code);
    }
    Ok(result)
}

fn parse_allowed_scan_codes(value: &Bound<'_, PyAny>) -> PyResult<Vec<u16>> {
    strict_scan_codes(
        value,
        "allowed_scan_codes",
        Some(&PHYSICAL_INSTRUMENT_SCAN_CODES),
    )
    .map(|v| v.into_vec())
}

fn parse_actions(
    value: &Bound<'_, PyAny>,
    allowed_scan_codes: &[u16],
) -> PyResult<Vec<KeyActionInput>> {
    let iter = value
        .try_iter()
        .map_err(|_| PyTypeError::new_err("actions must be an iterable"))?;

    let mut actions = Vec::new();
    let mut reason_interns = std::collections::HashMap::<String, Arc<str>>::new();

    for (position, item_res) in iter.enumerate() {
        if position >= sky_dispatch_core::compile::MAX_ACTIONS {
            return Err(PyValueError::new_err(format!(
                "actions exceeds the configured cap of {}",
                sky_dispatch_core::compile::MAX_ACTIONS
            )));
        }
        let item = item_res?;
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
            Some(allowed_scan_codes),
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

        let interned_reason = reason_interns
            .entry(reason.clone())
            .or_insert_with(|| Arc::from(reason))
            .clone();

        actions.push(KeyActionInput {
            source_action_index,
            kind,
            scheduled_us,
            scan_codes,
            reason: interned_reason,
        });
    }
    Ok(actions)
}

fn parse_schedule(
    py_actions: &Bound<'_, PyAny>,
    allowed_scan_codes: &Bound<'_, PyAny>,
) -> PyResult<(RuntimeSchedule, Vec<u16>)> {
    let allowed_scan_codes = parse_allowed_scan_codes(allowed_scan_codes)?;
    let actions = parse_actions(py_actions, &allowed_scan_codes)?;
    let schedule =
        sky_dispatch_core::compile::compile_runtime_intents(&actions, &allowed_scan_codes)
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
    Ok((schedule, allowed_scan_codes))
}

fn validate_schedule_timing(
    schedule: &RuntimeSchedule,
    min_hold_us: u64,
    max_lead_us: u64,
    dispatch_lead_us: u64,
) -> PyResult<()> {
    schedule
        .batches
        .last()
        .map_or(0, |batch| batch.scheduled_us)
        .checked_add(min_hold_us)
        .and_then(|value| value.checked_add(max_lead_us))
        .and_then(|value| value.checked_add(dispatch_lead_us))
        .ok_or_else(|| {
            PyValueError::new_err(
                "schedule and timing configuration exceed supported timestamp range",
            )
        })?;
    Ok(())
}

#[pyclass(name = "DispatchSession", frozen)]
struct NativeDispatchSessionPy {
    session: Arc<NativeDispatchSession>,
}

#[pymethods]
impl NativeDispatchSessionPy {
    #[new]
    #[pyo3(signature = (py_actions, allowed_scan_codes, config = None))]
    fn new(
        py_actions: &Bound<'_, PyAny>,
        allowed_scan_codes: &Bound<'_, PyAny>,
        config: Option<NativeSessionConfigPy>,
    ) -> PyResult<Self> {
        let config = config.unwrap_or_default();
        let parsed_profile = config.profile;
        let min_hold_us = config.min_hold_us;
        let max_lead_us = 2_000;
        let dispatch_lead_us = 0;
        let mock_backend = false;
        let mock_latency_base_us = 0;
        let mock_latency_per_key_us = 0;
        let fault_script = FaultInjectionScript::none();
        let require_focus = config.require_focus;
        let focus_restore_grace_us = 100_000;
        let spin_threshold_us = 150;
        let core_warmup_budget_us = 0;
        let parsed_telemetry_mode = if config.telemetry {
            crate::engine::TelemetryMode::Ring
        } else {
            crate::engine::TelemetryMode::Off
        };
        let telemetry_capacity = 1_024;
        let priority_mode = PriorityMode::Auto;
        let enable_waitable_timer = true;
        let enable_event_wait = true;
        let enable_adaptive_spin = true;
        let spin_floor_us = 700;
        let estimator_state_json = None;
        let enable_adaptive_lead = true;
        let input_path_warn_us = 300;
        let strict_timing = parsed_profile.strict_timing();
        let strict_down_completion_late_us = 2_000;
        let strict_up_completion_late_us = 2_000;
        let supervisor_lease_timeout_us = 3_000_000;
        if min_hold_us > 60_000_000 {
            return Err(PyValueError::new_err(
                "min_hold_us must be at most 60000000",
            ));
        }
        if max_lead_us > 10_000 {
            return Err(PyValueError::new_err("max_lead_us must be at most 10000"));
        }
        let (schedule, allowed_scan_codes) = parse_schedule(py_actions, allowed_scan_codes)?;
        validate_schedule_timing(&schedule, min_hold_us, max_lead_us, dispatch_lead_us)?;
        let session = NativeDispatchSession::new(
            schedule,
            min_hold_us,
            max_lead_us,
            dispatch_lead_us,
            allowed_scan_codes,
            mock_backend,
            mock_latency_base_us,
            mock_latency_per_key_us,
            fault_script,
            require_focus,
            focus_restore_grace_us,
            spin_threshold_us,
            core_warmup_budget_us,
            parsed_telemetry_mode,
            telemetry_capacity,
            priority_mode,
            enable_waitable_timer,
            enable_event_wait,
            enable_adaptive_spin,
            spin_floor_us,
            estimator_state_json.map(str::to_string),
            enable_adaptive_lead,
            input_path_warn_us,
            strict_timing,
            strict_down_completion_late_us,
            strict_up_completion_late_us,
            supervisor_lease_timeout_us,
        )
        .map_err(PyRuntimeError::new_err)?;
        session.set_target_hwnd(config.target_hwnd);

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

    #[cfg(feature = "test-support")]
    fn pause_with_timing_token(&self) -> PyResult<u64> {
        self.session
            .pause_with_timing_token()
            .map_err(PyRuntimeError::new_err)
    }

    #[cfg(feature = "test-support")]
    fn pause_timing_result<'py>(
        &self,
        py: Python<'py>,
        generation: StrictU64,
    ) -> PyResult<Option<Bound<'py, PyDict>>> {
        let Some(result) = self
            .session
            .pause_timing_result(generation.0)
            .map_err(PyRuntimeError::new_err)?
        else {
            return Ok(None);
        };
        let dict = PyDict::new(py);
        dict.set_item("generation", result.generation)?;
        dict.set_item("requested_ticks", result.requested_ticks.as_u64())?;
        dict.set_item("observed_ticks", result.observed_ticks.as_u64())?;
        dict.set_item("acknowledged_ticks", result.acknowledged_ticks.as_u64())?;
        dict.set_item("observation_latency_us", result.observation_latency_us)?;
        dict.set_item("completion_latency_us", result.completion_latency_us)?;
        dict.set_item("cleanup_cost_us", result.cleanup_cost_us)?;
        Ok(Some(dict))
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

    fn heartbeat(&self) -> PyResult<()> {
        self.session.heartbeat().map_err(PyRuntimeError::new_err)
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
        dict.set_item("sendinput_partial_events", snap.sendinput_partial_events)?;
        dict.set_item(
            "sendinput_zero_progress_failures",
            snap.sendinput_zero_progress_failures,
        )?;
        dict.set_item("chords_rejected", snap.chords_rejected)?;
        dict.set_item("authored_conflict_events", snap.authored_conflict_events)?;
        dict.set_item("authored_chords_rejected", snap.authored_chords_rejected)?;
        dict.set_item("authored_keys_rejected", snap.authored_keys_rejected)?;
        dict.set_item(
            "keys_inserted_before_failure",
            snap.keys_inserted_before_failure,
        )?;
        dict.set_item("keys_rolled_back", snap.keys_rolled_back)?;
        dict.set_item("rollback_residue_keys", snap.rollback_residue_keys)?;
        dict.set_item(
            "lead_saturation_count_down",
            snap.lead_saturation_count_down,
        )?;
        dict.set_item("lead_saturation_count_up", snap.lead_saturation_count_up)?;
        dict.set_item("positive_residual_at_cap", snap.positive_residual_at_cap)?;
        dict.set_item(
            "recovered_zero_progress_but_late",
            snap.recovered_zero_progress_but_late,
        )?;
        dict.set_item("outcome", snap.outcome)?;
        dict.set_item("rt_priority_acquired", snap.rt_priority_acquired)?;
        dict.set_item(
            "effective_spin_threshold_us",
            snap.effective_spin_threshold_us,
        )?;
        dict.set_item("wake_error_p50_us", snap.wake_error_p50_us)?;
        dict.set_item("wake_error_p95_us", snap.wake_error_p95_us)?;
        dict.set_item("wake_error_p99_us", snap.wake_error_p99_us)?;
        dict.set_item("wake_error_max_us", snap.wake_error_max_us)?;
        dict.set_item("spin_time_us", snap.spin_time_us)?;
        dict.set_item("playback_wall_time_us", snap.playback_wall_time_us)?;
        dict.set_item("spin_duty_cycle_ppm", snap.spin_duty_cycle_ppm)?;
        dict.set_item("worker_cpu_time_us", snap.worker_cpu_time_us)?;
        dict.set_item("process_cpu_time_us", snap.process_cpu_time_us)?;
        dict.set_item("wait_strategy_acquired", snap.wait_strategy_acquired)?;
        dict.set_item("power_throttling_disabled", snap.power_throttling_disabled)?;
        dict.set_item("input_path_degraded", snap.input_path_degraded)?;
        dict.set_item("sendinput_path_degraded", snap.sendinput_path_degraded)?;
        dict.set_item("bookkeeping_degraded", snap.bookkeeping_degraded)?;
        dict.set_item("wait_path_degraded", snap.wait_path_degraded)?;
        dict.set_item("wait_target_error_us", snap.wait_target_error_us)?;
        dict.set_item("idle_wake_count", snap.idle_wake_count)?;
        dict.set_item("terminal_error", snap.terminal_error)?;
        dict.set_item("secondary_errors", snap.secondary_errors)?;
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

    fn snapshot_lite<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let snap = self.session.snapshot();
        let dict = PyDict::new(py);
        dict.set_item("state", snap.status)?;
        dict.set_item("elapsed_us", snap.elapsed_us)?;
        dict.set_item("total_us", snap.total_us)?;
        dict.set_item("max_completion_error_us", snap.max_lateness_us)?;
        dict.set_item("active_keys", snap.active_count)?;
        dict.set_item(
            "health",
            if snap.terminal_error.is_some() || snap.failed_release_count > 0 {
                "error"
            } else if snap.input_path_degraded {
                "degraded"
            } else {
                "ok"
            },
        )?;
        dict.set_item("is_running", snap.is_running)?;
        dict.set_item("is_finished", snap.is_finished)?;
        dict.set_item("is_paused", snap.is_paused)?;
        dict.set_item("input_path_degraded", snap.input_path_degraded)?;
        Ok(dict)
    }

    fn session_report<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let report = PyDict::new(py);
        report.set_item("snapshot", self.snapshot(py)?)?;
        report.set_item("telemetry_json", self.take_telemetry_json(py)?)?;
        report.set_item("estimator_state_json", self.estimator_state_json()?)?;
        Ok(report)
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

#[cfg(feature = "test-support")]
#[pyclass(name = "TestDispatchSession", frozen)]
struct TestDispatchSessionPy {
    session: Arc<NativeDispatchSession>,
}

#[cfg(feature = "test-support")]
#[pymethods]
impl TestDispatchSessionPy {
    #[new]
    #[pyo3(signature = (
        py_actions,
        allowed_scan_codes,
        min_hold_us = StrictU64(100),
        mock_latency_base_us = StrictU64(80),
        mock_latency_per_key_us = StrictU64(40),
        telemetry_capacity = StrictU64(1024),
        rt_priority_mode = "off",
        enable_waitable_timer = true,
        enable_event_wait = true,
        enable_adaptive_spin = true,
        enable_adaptive_lead = true
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py_actions: &Bound<'_, PyAny>,
        allowed_scan_codes: &Bound<'_, PyAny>,
        min_hold_us: StrictU64,
        mock_latency_base_us: StrictU64,
        mock_latency_per_key_us: StrictU64,
        telemetry_capacity: StrictU64,
        rt_priority_mode: &str,
        enable_waitable_timer: bool,
        enable_event_wait: bool,
        enable_adaptive_spin: bool,
        enable_adaptive_lead: bool,
    ) -> PyResult<Self> {
        let min_hold_us = min_hold_us.0;
        let mock_latency_base_us = mock_latency_base_us.0;
        let mock_latency_per_key_us = mock_latency_per_key_us.0;
        let telemetry_capacity = usize::try_from(telemetry_capacity.0)
            .map_err(|_| PyValueError::new_err("telemetry_capacity is too large"))?;
        if min_hold_us > 60_000_000 {
            return Err(PyValueError::new_err(
                "min_hold_us must be at most 60000000",
            ));
        }
        if mock_latency_base_us > 1_000_000 || mock_latency_per_key_us > 1_000_000 {
            return Err(PyValueError::new_err(
                "mock latency values must be at most 1000000 microseconds",
            ));
        }
        if telemetry_capacity == 0 || telemetry_capacity > 4_096 {
            return Err(PyValueError::new_err(
                "telemetry_capacity must be between 1 and 4096",
            ));
        }
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
        let max_lead_us = 2_000;
        let dispatch_lead_us = 0;
        let (schedule, allowed_scan_codes) = parse_schedule(py_actions, allowed_scan_codes)?;
        validate_schedule_timing(&schedule, min_hold_us, max_lead_us, dispatch_lead_us)?;
        let session = NativeDispatchSession::new(
            schedule,
            min_hold_us,
            max_lead_us,
            dispatch_lead_us,
            allowed_scan_codes,
            true,
            mock_latency_base_us,
            mock_latency_per_key_us,
            FaultInjectionScript::none(),
            false,
            100_000,
            150,
            0,
            crate::engine::TelemetryMode::Ring,
            telemetry_capacity,
            priority_mode,
            enable_waitable_timer,
            enable_event_wait,
            enable_adaptive_spin,
            700,
            None,
            enable_adaptive_lead,
            300,
            false,
            2_000,
            2_000,
            3_000_000,
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

    fn pause_with_timing_token(&self) -> PyResult<u64> {
        self.session
            .pause_with_timing_token()
            .map_err(PyRuntimeError::new_err)
    }

    fn pause_timing_result<'py>(
        &self,
        py: Python<'py>,
        generation: StrictU64,
    ) -> PyResult<Option<Bound<'py, PyDict>>> {
        let Some(result) = self
            .session
            .pause_timing_result(generation.0)
            .map_err(PyRuntimeError::new_err)?
        else {
            return Ok(None);
        };
        let dict = PyDict::new(py);
        dict.set_item("generation", result.generation)?;
        dict.set_item("requested_ticks", result.requested_ticks.as_u64())?;
        dict.set_item("observed_ticks", result.observed_ticks.as_u64())?;
        dict.set_item("acknowledged_ticks", result.acknowledged_ticks.as_u64())?;
        dict.set_item("observation_latency_us", result.observation_latency_us)?;
        dict.set_item("completion_latency_us", result.completion_latency_us)?;
        dict.set_item("cleanup_cost_us", result.cleanup_cost_us)?;
        Ok(Some(dict))
    }

    fn quit(&self) -> PyResult<()> {
        self.session.quit().map_err(PyRuntimeError::new_err)
    }

    fn panic_release(&self) -> PyResult<()> {
        self.session
            .panic_release()
            .map_err(PyRuntimeError::new_err)
    }

    fn snapshot<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        NativeDispatchSessionPy {
            session: Arc::clone(&self.session),
        }
        .snapshot(py)
    }

    #[pyo3(signature = (timeout_ms = StrictU64(5000)))]
    fn join(&self, py: Python<'_>, timeout_ms: StrictU64) -> PyResult<bool> {
        if timeout_ms.0 == 0 || timeout_ms.0 > 60_000 {
            return Err(PyValueError::new_err(
                "timeout_ms must be between 1 and 60000",
            ));
        }
        py.detach(|| {
            self.session
                .join(std::time::Duration::from_millis(timeout_ms.0))
        })
        .map_err(PyRuntimeError::new_err)
    }

    fn take_telemetry_json(&self, py: Python<'_>) -> PyResult<String> {
        py.detach(|| self.session.take_telemetry_json())
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

/// Free-threaded PyO3 extension module for Sky Auto Player dispatch engine.
#[pyo3::pymodule(gil_used = false)]
fn sky_player_rs(m: &Bound<'_, PyModule>) -> PyResult<()> {
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
