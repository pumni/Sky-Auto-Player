use super::conversion::*;
use super::snapshot::ProgressSnapshotPy;
use super::*;
use crate::engine::{
    FocusOptions, NativeSessionOptions, PriorityOptions, TelemetryOptions, TimingOptions,
    WaitOptions,
};

#[pyclass(name = "DispatchSession", frozen)]
pub(super) struct NativeDispatchSessionPy {
    session: Arc<NativeDispatchSession>,
    effective_config: EffectiveSessionConfig,
}

#[derive(Clone, Default)]
struct EffectiveSessionConfig {
    game_fps: u16,
    requested_min_hold_us: u64,
    effective_min_hold_us: u64,
    require_focus: bool,
    focus_restore_grace_us: u64,
    telemetry_mode: &'static str,
    profile: &'static str,
}

impl EffectiveSessionConfig {
    fn to_py_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("game_fps", self.game_fps)?;
        dict.set_item("requested_min_hold_us", self.requested_min_hold_us)?;
        dict.set_item("effective_min_hold_us", self.effective_min_hold_us)?;
        dict.set_item("require_focus", self.require_focus)?;
        dict.set_item("focus_restore_grace_us", self.focus_restore_grace_us)?;
        dict.set_item("telemetry_mode", self.telemetry_mode)?;
        dict.set_item("profile", self.profile)?;
        Ok(dict)
    }
}

#[pymethods]
impl NativeDispatchSessionPy {
    #[new]
    #[pyo3(signature = (py_actions, config = None))]
    fn new(py_actions: &Bound<'_, PyAny>, config: Option<NativeSessionConfigPy>) -> PyResult<Self> {
        let config = config.unwrap_or_default();
        let parsed_profile = config.profile;
        let game_fps = config.game_fps;
        let min_hold_us = config.min_hold_us;
        let frame_period_us = 1_000_000u64.div_ceil(u64::from(game_fps));
        let effective_min_hold_us = min_hold_us.max(frame_period_us.saturating_add(500));
        let require_focus = config.require_focus;
        let focus_restore_grace_us = config.focus_restore_grace_us;
        let parsed_telemetry_mode = if config.telemetry {
            crate::engine::TelemetryMode::Ring
        } else {
            crate::engine::TelemetryMode::Off
        };
        let telemetry_capacity = 1_024;
        let priority_mode = PriorityMode::Auto;
        let enable_waitable_timer = true;
        let enable_event_wait = true;
        // Compatibility input only: estimator state is intentionally ignored.
        let _deprecated_estimator_state_json = config.estimator_state_json.as_ref();
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
        let schedule = parse_schedule(py_actions)?;
        validate_schedule_timing(&schedule, effective_min_hold_us)?;
        let session = NativeDispatchSession::new(NativeSessionOptions {
            schedule,
            backend: BackendConfig::Production,
            timing: TimingOptions {
                min_hold_us: effective_min_hold_us,
                strict_timing,
                strict_down_completion_late_us,
                strict_up_completion_late_us,
                input_path_warn_us,
            },
            focus: FocusOptions {
                require_focus,
                focus_restore_grace_us,
            },
            wait: WaitOptions {
                enable_waitable_timer,
                enable_event_wait,
                supervisor_lease_timeout_us,
            },
            telemetry: TelemetryOptions {
                mode: parsed_telemetry_mode,
                capacity: telemetry_capacity,
            },
            priority: PriorityOptions {
                mode: priority_mode,
            },
            #[cfg(any(test, feature = "test-support"))]
            startup_ordering_hook: None,
            #[cfg(any(test, feature = "test-support"))]
            restore_race_hook: None,
        })
        .map_err(PyRuntimeError::new_err)?;
        session.set_target_hwnd(config.target_hwnd);

        Ok(Self {
            session: Arc::new(session),
            effective_config: EffectiveSessionConfig {
                game_fps,
                requested_min_hold_us: min_hold_us,
                effective_min_hold_us,
                require_focus,
                focus_restore_grace_us,
                telemetry_mode: if config.telemetry { "ring" } else { "off" },
                profile: match parsed_profile {
                    DispatchProfile::Production => "production",
                    DispatchProfile::StrictTimingDiagnostic => "strict_timing_diagnostic",
                    #[cfg(any(test, feature = "test-support"))]
                    DispatchProfile::MockTest => "mock_test",
                },
            },
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

    fn set_focus_hint(&self, active: bool) -> PyResult<()> {
        self.session.set_focus_hint(active);
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
        dict.set_item("chord_integrity_lost", snap.chord_integrity_lost)?;
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
        dict.set_item(
            "recovered_zero_progress_retries",
            snap.recovered_zero_progress_retries,
        )?;
        dict.set_item(
            "recovered_partial_up_retries",
            snap.recovered_partial_up_retries,
        )?;
        dict.set_item("outcome", snap.outcome)?;
        dict.set_item("startup_ready", snap.startup_ready)?;
        dict.set_item("startup_latency_us", snap.startup_latency_us)?;
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
        dict.set_item("core_post_send_degraded", snap.core_post_send_degraded)?;
        dict.set_item("observer_degraded", snap.observer_degraded)?;
        dict.set_item("wait_path_degraded", snap.wait_path_degraded)?;
        dict.set_item(
            "sendinput_warn_threshold_us",
            snap.sendinput_warn_threshold_us,
        )?;
        dict.set_item(
            "core_post_send_warn_threshold_us",
            snap.core_post_send_warn_threshold_us,
        )?;
        dict.set_item(
            "observer_warn_threshold_us",
            snap.observer_warn_threshold_us,
        )?;
        dict.set_item("wait_warn_threshold_us", snap.wait_warn_threshold_us)?;
        dict.set_item(
            "sendinput_degraded_samples",
            snap.sendinput_degraded_samples,
        )?;
        dict.set_item(
            "core_post_send_degraded_samples",
            snap.core_post_send_degraded_samples,
        )?;
        dict.set_item("observer_degraded_samples", snap.observer_degraded_samples)?;
        dict.set_item("wait_degraded_samples", snap.wait_degraded_samples)?;
        dict.set_item("wait_backend_failures", snap.wait_backend_failures)?;
        dict.set_item("wait_clock_failures", snap.wait_clock_failures)?;
        dict.set_item("wait_interrupted_count", snap.wait_interrupted_count)?;
        dict.set_item(
            "sendinput_window_bad_count",
            snap.sendinput_window_bad_count,
        )?;
        dict.set_item(
            "core_post_send_window_bad_count",
            snap.core_post_send_window_bad_count,
        )?;
        dict.set_item("observer_window_bad_count", snap.observer_window_bad_count)?;
        dict.set_item("wait_window_bad_count", snap.wait_window_bad_count)?;
        dict.set_item(
            "sendinput_window_sample_count",
            snap.sendinput_window_sample_count,
        )?;
        dict.set_item(
            "core_post_send_window_sample_count",
            snap.core_post_send_window_sample_count,
        )?;
        dict.set_item(
            "observer_window_sample_count",
            snap.observer_window_sample_count,
        )?;
        dict.set_item("wait_window_sample_count", snap.wait_window_sample_count)?;
        dict.set_item("timeline_rebase_count", snap.timeline_rebase_count)?;
        dict.set_item("timeline_rebase_total_us", snap.timeline_rebase_total_us)?;
        dict.set_item("timeline_rebase_max_us", snap.timeline_rebase_max_us)?;
        dict.set_item(
            "timeline_rebase_last_reason",
            snap.timeline_rebase_last_reason,
        )?;
        dict.set_item("core_post_send_max_us", snap.core_post_send_max_us)?;
        dict.set_item("wake_to_send_max_us", snap.wake_to_send_max_us)?;
        dict.set_item("observer_duration_max_us", snap.observer_duration_max_us)?;
        dict.set_item("observer_dropped_samples", snap.observer_dropped_samples)?;
        dict.set_item(
            "observer_queue_high_watermark",
            snap.observer_queue_high_watermark,
        )?;
        dict.set_item("dispatch_occupancy_max_us", snap.dispatch_occupancy_max_us)?;
        dict.set_item(
            "send_down_degraded_samples",
            snap.send_down_degraded_samples,
        )?;
        dict.set_item("send_up_degraded_samples", snap.send_up_degraded_samples)?;
        dict.set_item(
            "send_mixed_degraded_samples",
            snap.send_mixed_degraded_samples,
        )?;
        dict.set_item(
            "send_down_warn_threshold_us",
            snap.send_down_warn_threshold_us,
        )?;
        dict.set_item("send_up_warn_threshold_us", snap.send_up_warn_threshold_us)?;
        dict.set_item(
            "send_mixed_warn_threshold_us",
            snap.send_mixed_warn_threshold_us,
        )?;
        dict.set_item("wait_target_error_us", snap.wait_target_error_us)?;
        dict.set_item("idle_wake_count", snap.idle_wake_count)?;
        dict.set_item("terminal_error", snap.terminal_error)?;
        dict.set_item("secondary_errors", snap.secondary_errors)?;
        dict.set_item("generation_count", snap.generation_count)?;
        dict.set_item("generation_status_counts", snap.generation_status_counts)?;
        dict.set_item("abort_counts_by_reason", snap.abort_counts_by_reason)?;
        if let Some(outcome) = snap.release_outcome {
            let release = PyDict::new(py);
            release.set_item("attempted", outcome.attempted())?;
            release.set_item("released_successfully", outcome.released_successfully)?;
            release.set_item("stuck_keys", outcome.stuck_keys())?;
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

    fn snapshot_lite(&self) -> ProgressSnapshotPy {
        ProgressSnapshotPy::from_snapshot(&self.session.snapshot_lite())
    }

    fn session_report<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let report = PyDict::new(py);
        report.set_item("snapshot", self.snapshot(py)?)?;
        report.set_item("effective_config", self.effective_config.to_py_dict(py)?)?;
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
pub(super) struct TestDispatchSessionPy {
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
        fault_mode = "none"
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
    ) -> PyResult<Self> {
        let _ = (enable_adaptive_spin, enable_dispatch_cost_lead);
        let min_hold_us = min_hold_us.0;
        let game_fps = u16::try_from(game_fps.0)
            .map_err(|_| PyValueError::new_err("game_fps must be an integer in 15..=240"))?;
        if !(15..=240).contains(&game_fps) {
            return Err(PyValueError::new_err("game_fps must be in 15..=240"));
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
        let frame_period_us = 1_000_000u64.div_ceil(u64::from(game_fps));
        let effective_min_hold_us = min_hold_us.max(frame_period_us.saturating_add(500));
        let (schedule, _allowed_scan_codes) =
            parse_schedule_with_allowlist(py_actions, allowed_scan_codes)?;
        validate_schedule_timing(&schedule, effective_min_hold_us)?;
        let session = NativeDispatchSession::new(NativeSessionOptions {
            schedule,
            backend: BackendConfig::Mock {
                latency_base_us: mock_latency_base_us,
                latency_per_key_us: mock_latency_per_key_us,
                fault_script,
            },
            timing: TimingOptions {
                min_hold_us: effective_min_hold_us,
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
        })
        .map_err(PyRuntimeError::new_err)?;
        Ok(Self {
            session: Arc::new(session),
        })
    }

    fn start(&self) -> PyResult<()> {
        self.session.start().map_err(PyRuntimeError::new_err)
    }

    /// Keep the test-support session's supervisor lease alive for acceptance
    /// benchmarks that intentionally run longer than the three-second lease.
    /// Production callers already publish this heartbeat through the normal
    /// supervisor adapter; exposing it here keeps the test harness on the
    /// same lifecycle contract without adding a production backend surface.
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
