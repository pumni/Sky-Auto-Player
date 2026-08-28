use super::conversion::{parse_schedule_with_allowlist, validate_schedule_timing};
use super::session::{EffectiveSessionConfig, NativeDispatchSessionPy};
use super::*;
use crate::engine::{
    FocusOptions, NativeSessionOptions, PriorityOptions, TelemetryOptions, TimingOptions,
    WaitOptions,
};

#[pyclass(name = "TestDispatchSession", frozen)]
pub(super) struct TestDispatchSessionPy {
    session: Arc<NativeDispatchSession>,
}

#[pymethods]
impl TestDispatchSessionPy {
    #[new]
    #[pyo3(signature = (
        py_actions,
        allowed_scan_codes,
        min_hold_us = StrictU64(100),
        game_fps = StrictU64(60),
        mock_latency_base_us = StrictU64(80),
        mock_latency_per_key_us = StrictU64(40),
        telemetry_capacity = StrictU64(1024),
        dispatch_lead_us = StrictU64(0),
        rt_priority_mode = "off",
        enable_waitable_timer = true,
        enable_event_wait = true,
        enable_adaptive_spin = true,
        enable_dispatch_cost_lead = true,
        fault_mode = "none",
        min_release_gap_us = None,
        down_late_grace_us = StrictU64(0)
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py_actions: &Bound<'_, PyAny>,
        allowed_scan_codes: &Bound<'_, PyAny>,
        min_hold_us: StrictU64,
        game_fps: StrictU64,
        mock_latency_base_us: StrictU64,
        mock_latency_per_key_us: StrictU64,
        telemetry_capacity: StrictU64,
        dispatch_lead_us: StrictU64,
        rt_priority_mode: &str,
        enable_waitable_timer: bool,
        enable_event_wait: bool,
        enable_adaptive_spin: bool,
        enable_dispatch_cost_lead: bool,
        fault_mode: &str,
        min_release_gap_us: Option<StrictU64>,
        down_late_grace_us: StrictU64,
    ) -> PyResult<Self> {
        let _ = (enable_adaptive_spin, enable_dispatch_cost_lead);
        let min_hold_us = min_hold_us.0;
        let down_late_grace_us = down_late_grace_us.0;
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
        let mock_latency_base_us = mock_latency_base_us.0;
        let mock_latency_per_key_us = mock_latency_per_key_us.0;
        let _dispatch_lead_us = dispatch_lead_us.0;
        let telemetry_capacity = usize::try_from(telemetry_capacity.0)
            .map_err(|_| PyValueError::new_err("telemetry_capacity is too large"))?;
        if min_hold_us > 60_000_000 {
            return Err(PyValueError::new_err(
                "min_hold_us must be at most 60000000",
            ));
        }
        if down_late_grace_us > min_hold_us {
            return Err(PyValueError::new_err(
                "down_late_grace_us must not exceed min_hold_us",
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
        let fault_script = match fault_mode {
            "none" => FaultInjectionScript::none(),
            "zero_progress" => FaultInjectionScript::zero_progress_down_once(),
            "zero_progress_failed" => FaultInjectionScript::persistent_zero_down(),
            "partial" => FaultInjectionScript::partial_down_first_attempt(),
            "partial_after_zero_retry" => FaultInjectionScript::partial_down_after_zero_retry(),
            _ => {
                return Err(PyValueError::new_err(
                    "fault_mode must be none, zero_progress, zero_progress_failed, partial, or partial_after_zero_retry",
                ));
            }
        };
        let effective_min_hold_us = min_hold_us;
        let (schedule, _allowed_scan_codes) =
            parse_schedule_with_allowlist(py_actions, allowed_scan_codes)?;
        validate_schedule_timing(&schedule, effective_min_hold_us, min_release_gap_us)?;
        let session = NativeDispatchSession::new(NativeSessionOptions {
            schedule,
            backend: BackendConfig::Mock {
                latency_base_us: mock_latency_base_us,
                latency_per_key_us: mock_latency_per_key_us,
                fault_script,
            },
            profile: DispatchProfile::MockTest,
            timing: TimingOptions {
                game_fps,
                min_hold_us: effective_min_hold_us,
                min_release_gap_us,
                down_late_grace_us,
                strict_timing: false,
                strict_down_completion_late_us: 2_000,
                strict_up_completion_late_us: 2_000,
                input_path_warn_us: 300,
            },
            focus: FocusOptions {
                require_focus: false,
                focus_restore_grace_us: 100_000,
            },
            wait: WaitOptions {
                enable_waitable_timer,
                enable_event_wait,
                supervisor_lease_timeout_us: 3_000_000,
                #[cfg(any(test, feature = "test-support"))]
                test_spin_threshold_us: Some(20_000),
            },
            telemetry: TelemetryOptions {
                mode: crate::engine::TelemetryMode::Ring,
                capacity: telemetry_capacity,
            },
            priority: PriorityOptions {
                mode: priority_mode,
            },
            #[cfg(any(test, feature = "test-support"))]
            startup_ordering_hook: None,
            #[cfg(any(test, feature = "test-support"))]
            restore_race_hook: None,
            #[cfg(any(test, feature = "test-support"))]
            timer_lifecycle_context: None,
        })
        .map_err(PyRuntimeError::new_err)?;
        Ok(Self {
            session: Arc::new(session),
        })
    }

    fn arm(&self, pre_roll_us: StrictU64) -> PyResult<()> {
        self.session
            .arm(pre_roll_us.0)
            .map_err(PyRuntimeError::new_err)
    }

    fn start(&self) -> PyResult<()> {
        self.arm(StrictU64(0))
    }

    /// Keep the test-support session's supervisor lease alive for long-running
    /// acceptance benchmarks; production publishes this through its adapter.
    fn heartbeat(&self) -> PyResult<()> {
        self.session.heartbeat().map_err(PyRuntimeError::new_err)
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
            effective_config: EffectiveSessionConfig::default(),
        }
        .snapshot(py)
    }

    fn poll_state(&self) -> PollSnapshotPy {
        PollSnapshotPy::from_snapshot(self.session.poll_state())
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
