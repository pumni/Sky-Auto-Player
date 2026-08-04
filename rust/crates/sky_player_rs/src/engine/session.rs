use super::shared::{
    SessionCommands, SessionLifecycle, SessionPublication, SessionShared, SessionTarget,
};
use super::worker::Worker;
use super::*;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex as StdMutex};
use std::time::Duration;

pub struct NativeDispatchSession {
    config: Mutex<Option<WorkerConfig>>,
    generation_count: u64,
    shared: Arc<SessionShared>,
    thread_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl NativeDispatchSession {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        schedule: RuntimeSchedule,
        min_hold_us: u64,
        max_lead_us: u64,
        dispatch_lead_us: u64,
        allowed_scan_codes: Vec<u16>,
        backend: BackendConfig,
        require_focus: bool,
        focus_restore_grace_us: u64,
        spin_threshold_us: u64,
        core_warmup_budget_us: u64,
        telemetry_mode: TelemetryMode,
        telemetry_capacity: usize,
        priority_mode: PriorityMode,
        enable_waitable_timer: bool,
        enable_event_wait: bool,
        enable_adaptive_spin: bool,
        spin_floor_us: u64,
        estimator_state_json: Option<String>,
        enable_adaptive_lead: bool,
        input_path_warn_us: u64,
        strict_timing: bool,
        strict_down_completion_late_us: u64,
        strict_up_completion_late_us: u64,
        supervisor_lease_timeout_us: u64,
    ) -> Result<Self, String> {
        if !cfg!(windows) && matches!(&backend, BackendConfig::Production) {
            return Err("production native dispatch is supported only on Windows".to_string());
        }
        let initial_heartbeat_ticks = sky_dispatch_win32::clock::qpc_now_ticks_checked()
            .map_err(|error| format!("QPC admission failed before session creation: {error:?}"))?;
        let interrupt = OwnedEvent::new_auto_reset()
            .ok_or_else(|| "failed to create command event".to_string())?;
        let total_us = schedule
            .batches
            .last()
            .map_or(0, |batch| batch.scheduled_us);
        let generation_count = schedule.generation_count;
        let metrics = SharedMetrics::default();
        metrics.snapshot.lock().total_us = total_us;
        let shared = Arc::new(SessionShared {
            commands: SessionCommands {
                interrupt,
                desired_pause: AtomicBool::new(false),
                quit_requested: AtomicBool::new(false),
                skip_requested: AtomicBool::new(false),
                panic_requested: AtomicBool::new(false),
                // Foreground ownership is derived from target_hwnd inside the
                // worker. Python no longer publishes a second focus boolean.
                focus_active: AtomicBool::new(true),
                #[cfg(any(test, feature = "test-support"))]
                command_timing: CommandTimingState::default(),
            },
            target: SessionTarget {
                target_hwnd: AtomicIsize::new(0),
                target_generation: AtomicU64::new(0),
            },
            lifecycle: SessionLifecycle {
                lifecycle: AtomicU8::new(LIFECYCLE_NEW),
                terminal_outcome: AtomicU8::new(OUTCOME_NONE),
                completed: (StdMutex::new(false), Condvar::new()),
            },
            publication: SessionPublication {
                metrics,
                telemetry_output: Mutex::new(None),
                priority_acquired: Mutex::new("pending".to_string()),
                estimator_output: Mutex::new(None),
                supervisor_heartbeat_ticks: AtomicU64::new(initial_heartbeat_ticks.as_u64()),
            },
        });
        Ok(Self {
            config: Mutex::new(Some(WorkerConfig {
                schedule,
                min_hold_us,
                max_lead_us,
                dispatch_lead_us,
                allowed_count: allowed_scan_codes.len(),
                backend,
                require_focus,
                focus_restore_grace_us,
                spin_threshold_us,
                core_warmup_budget_us,
                telemetry_mode,
                telemetry_capacity,
                priority_mode,
                enable_waitable_timer,
                enable_event_wait,
                enable_adaptive_spin,
                spin_floor_us,
                estimator_state_json,
                enable_adaptive_lead,
                input_path_warn_us,
                strict_timing,
                strict_down_completion_late_us,
                strict_up_completion_late_us,
                supervisor_lease_timeout_us,
            })),
            generation_count,
            shared,
            thread_handle: Mutex::new(None),
        })
    }

    pub fn start(&self) -> Result<(), String> {
        self.shared
            .lifecycle
            .lifecycle
            .compare_exchange(
                LIFECYCLE_NEW,
                LIFECYCLE_RUNNING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|state| format!("session cannot start from lifecycle state {state}"))?;
        let heartbeat_ticks = match sky_dispatch_win32::clock::qpc_now_ticks_checked() {
            Ok(value) => value,
            Err(error) => {
                self.shared
                    .lifecycle
                    .lifecycle
                    .store(LIFECYCLE_POISONED, Ordering::Release);
                return Err(format!(
                    "QPC admission failed before worker start: {error:?}"
                ));
            }
        };
        let Some(config) = self.config.lock().take() else {
            self.shared
                .lifecycle
                .lifecycle
                .store(LIFECYCLE_POISONED, Ordering::Release);
            return Err("session configuration is no longer available".to_string());
        };

        let shared = Arc::clone(&self.shared);
        self.shared
            .publication
            .supervisor_heartbeat_ticks
            .store(heartbeat_ticks.as_u64(), Ordering::Release);

        let spawn_result = std::thread::Builder::new()
            .name("sky-native-dispatch".to_string())
            .spawn(move || {
                let worker_result = catch_unwind(AssertUnwindSafe(|| {
                    Worker::new(config, shared.as_ref()).run()
                }));
                let (worker_outcome, panic_message) = match worker_result {
                    Ok(outcome) => (outcome, None),
                    Err(payload) => {
                        let message = payload
                            .downcast_ref::<&str>()
                            .map(|value| (*value).to_string())
                            .or_else(|| payload.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "native worker panicked".to_string());
                        (OUTCOME_ERROR, Some(message))
                    }
                };
                let panicked = panic_message.is_some();
                if let Some(message) = panic_message {
                    shared
                        .publication
                        .metrics
                        .terminal_error
                        .lock()
                        .replace(message);
                }
                shared
                    .lifecycle
                    .terminal_outcome
                    .store(worker_outcome, Ordering::Release);
                shared
                    .publication
                    .metrics
                    .panicked
                    .store(panicked, Ordering::Release);
                if panicked {
                    shared
                        .lifecycle
                        .lifecycle
                        .store(LIFECYCLE_POISONED, Ordering::Release);
                } else {
                    let _ = shared.lifecycle.lifecycle.compare_exchange(
                        LIFECYCLE_RUNNING,
                        LIFECYCLE_FINISHED,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                }
                let (done_lock, done_cv) = &shared.lifecycle.completed;
                if let Ok(mut done) = done_lock.lock() {
                    *done = true;
                    done_cv.notify_all();
                }
            });

        match spawn_result {
            Ok(handle) => {
                *self.thread_handle.lock() = Some(handle);
                Ok(())
            }
            Err(error) => {
                self.shared
                    .lifecycle
                    .lifecycle
                    .store(LIFECYCLE_POISONED, Ordering::Release);
                Err(format!("failed to spawn native dispatch worker: {error}"))
            }
        }
    }

    fn signal_worker(&self) -> Result<(), String> {
        if !matches!(
            self.shared.lifecycle.lifecycle.load(Ordering::Acquire),
            LIFECYCLE_RUNNING | LIFECYCLE_POISONED
        ) {
            return Err("session commands require a running worker".to_string());
        }
        let _ = self.shared.commands.interrupt.signal();
        Ok(())
    }

    pub fn pause(&self) -> Result<(), String> {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.pause_with_timing_token().map(|_| ())
        }
        #[cfg(not(any(test, feature = "test-support")))]
        {
            if self.shared.lifecycle.lifecycle.load(Ordering::Acquire) != LIFECYCLE_RUNNING {
                return Err("session commands require a running worker".to_string());
            }
            self.shared
                .commands
                .desired_pause
                .store(true, Ordering::Release);
            let _ = self.shared.commands.interrupt.signal();
            Ok(())
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn pause_with_timing_token(&self) -> Result<u64, String> {
        if self.shared.lifecycle.lifecycle.load(Ordering::Acquire) != LIFECYCLE_RUNNING {
            return Err("session commands require a running worker".to_string());
        }
        let request_ticks = sky_dispatch_win32::clock::qpc_now_ticks_checked()
            .map_err(|error| format!("QPC pause request failed: {error:?}"))?;
        let generation = self
            .shared
            .commands
            .command_timing
            .request_pause(request_ticks)
            .map_err(|error| error.to_string())?;
        self.shared
            .commands
            .desired_pause
            .store(true, Ordering::Release);
        let _ = self.shared.commands.interrupt.signal();
        Ok(generation)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn pause_timing_result(
        &self,
        generation: u64,
    ) -> Result<Option<CommandTimingResult>, String> {
        if generation == 0 {
            return Err("pause timing generation must be non-zero".to_string());
        }
        let qpc_clock = QpcClock::initialize()
            .map_err(|error| format!("QPC pause timing conversion failed: {error:?}"))?;
        match self
            .shared
            .commands
            .command_timing
            .result(generation, qpc_clock)
            .map_err(|error| error.to_string())?
        {
            PauseTimingLookup::Pending => Ok(None),
            PauseTimingLookup::Complete(result) => Ok(Some(result)),
            PauseTimingLookup::Cancelled => Err(format!(
                "pause timing generation {generation} was cancelled"
            )),
            PauseTimingLookup::UnknownGeneration => {
                Err(format!("unknown pause timing generation {generation}"))
            }
        }
    }

    pub fn resume(&self) -> Result<(), String> {
        if self.shared.lifecycle.lifecycle.load(Ordering::Acquire) != LIFECYCLE_RUNNING {
            return Err("session commands require a running worker".to_string());
        }
        #[cfg(any(test, feature = "test-support"))]
        self.shared.commands.command_timing.cancel_pause_request();
        self.shared
            .commands
            .desired_pause
            .store(false, Ordering::Release);
        let _ = self.shared.commands.interrupt.signal();
        Ok(())
    }

    pub fn skip(&self) -> Result<(), String> {
        if !matches!(
            self.shared.lifecycle.lifecycle.load(Ordering::Acquire),
            LIFECYCLE_RUNNING | LIFECYCLE_POISONED
        ) {
            return Err("session commands require a running worker".to_string());
        }
        self.shared
            .commands
            .skip_requested
            .store(true, Ordering::Release);
        self.signal_worker()
    }

    pub fn quit(&self) -> Result<(), String> {
        if !matches!(
            self.shared.lifecycle.lifecycle.load(Ordering::Acquire),
            LIFECYCLE_RUNNING | LIFECYCLE_POISONED
        ) {
            return Err("session commands require a running worker".to_string());
        }
        self.shared
            .commands
            .quit_requested
            .store(true, Ordering::Release);
        self.signal_worker()
    }

    pub fn panic_release(&self) -> Result<(), String> {
        if !matches!(
            self.shared.lifecycle.lifecycle.load(Ordering::Acquire),
            LIFECYCLE_RUNNING | LIFECYCLE_POISONED
        ) {
            return Err("session commands require a running worker".to_string());
        }
        self.shared
            .commands
            .panic_requested
            .store(true, Ordering::Release);
        self.signal_worker()
    }

    pub fn heartbeat(&self) -> Result<(), String> {
        if self.shared.lifecycle.lifecycle.load(Ordering::Acquire) == LIFECYCLE_RUNNING {
            let now = sky_dispatch_win32::clock::qpc_now_ticks_checked()
                .map_err(|error| format!("QPC heartbeat failed: {error:?}"))?;
            self.shared
                .publication
                .supervisor_heartbeat_ticks
                .store(now.as_u64(), Ordering::Release);
        }
        Ok(())
    }

    pub fn set_target_hwnd(&self, hwnd: isize) {
        if self.shared.target.target_hwnd.swap(hwnd, Ordering::AcqRel) != hwnd {
            self.shared
                .target
                .target_generation
                .fetch_add(1, Ordering::AcqRel);
            let _ = self.shared.commands.interrupt.signal();
        }
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        let lifecycle = self.shared.lifecycle.lifecycle.load(Ordering::Acquire);
        let paused = self
            .shared
            .publication
            .metrics
            .is_paused
            .load(Ordering::Relaxed);
        let outcome = self
            .shared
            .lifecycle
            .terminal_outcome
            .load(Ordering::Acquire);
        let status = match lifecycle {
            LIFECYCLE_NEW => "ready",
            LIFECYCLE_RUNNING if paused => "paused",
            LIFECYCLE_RUNNING => "playing",
            LIFECYCLE_FINISHED => match outcome {
                OUTCOME_ERROR => "error",
                OUTCOME_QUIT => "quit",
                OUTCOME_SKIPPED => "skipped",
                _ => "finished",
            },
            LIFECYCLE_POISONED
                if self
                    .shared
                    .publication
                    .metrics
                    .panicked
                    .load(Ordering::Acquire) =>
            {
                "panicked"
            }
            LIFECYCLE_POISONED => "poisoned",
            _ => "invalid",
        };
        let local = self.shared.publication.metrics.snapshot.lock().clone();
        EngineSnapshot {
            elapsed_us: local.elapsed_us,
            total_us: local.total_us,
            lateness_us: local.lateness_us,
            max_lateness_us: local.max_lateness_us,
            late_2ms: local.late_2ms,
            late_5ms: local.late_5ms,
            late_10ms: local.late_10ms,
            release_max_us: local.release_max_us,
            release_late_2ms: local.release_late_2ms,
            recent_latencies_us: local.recent_latencies.to_vec(),
            is_running: lifecycle == LIFECYCLE_RUNNING,
            is_finished: matches!(lifecycle, LIFECYCLE_FINISHED | LIFECYCLE_POISONED),
            is_paused: paused,
            status: status.to_string(),
            active_count: local.active_count as usize,
            possibly_active_count: local.possibly_active_count as usize,
            failed_release_count: local.failed_release_count as usize,
            last_error: self.shared.publication.metrics.last_error.lock().clone(),
            keys_dropped: local.keys_dropped,
            chord_split_events: local.chord_split_events,
            sendinput_partial_events: local.sendinput_partial_events,
            sendinput_zero_progress_failures: local.sendinput_zero_progress_failures,
            chords_rejected: local.chords_rejected,
            authored_conflict_events: local.authored_conflict_events,
            authored_chords_rejected: local.authored_chords_rejected,
            authored_keys_rejected: local.authored_keys_rejected,
            keys_inserted_before_failure: local.keys_inserted_before_failure,
            keys_rolled_back: local.keys_rolled_back,
            rollback_residue_keys: local.rollback_residue_keys,
            lead_saturation_count_down: local.lead_saturation_count_down.to_vec(),
            lead_saturation_count_up: local.lead_saturation_count_up.to_vec(),
            positive_residual_at_cap: local.positive_residual_at_cap,
            recovered_zero_progress_but_late: local.recovered_zero_progress_but_late,
            outcome: self.terminal_outcome().map(str::to_string),
            rt_priority_acquired: self.shared.publication.priority_acquired.lock().clone(),
            effective_spin_threshold_us: local.effective_spin_threshold_us,
            wake_error_p50_us: local.wake_error_p50_us,
            wake_error_p95_us: local.wake_error_p95_us,
            wake_error_p99_us: local.wake_error_p99_us,
            wake_error_max_us: local.wake_error_max_us,
            spin_time_us: local.spin_time_us,
            playback_wall_time_us: local.playback_wall_time_us,
            spin_duty_cycle_ppm: local.spin_duty_cycle_ppm,
            worker_cpu_time_us: local.worker_cpu_time_us,
            process_cpu_time_us: local.process_cpu_time_us,
            wait_strategy_acquired: self
                .shared
                .publication
                .metrics
                .wait_strategy_acquired
                .lock()
                .clone(),
            power_throttling_disabled: local.power_throttling_disabled,
            input_path_degraded: local.input_path_degraded,
            sendinput_path_degraded: local.sendinput_path_degraded,
            bookkeeping_degraded: local.bookkeeping_degraded,
            wait_path_degraded: local.wait_path_degraded,
            wait_target_error_us: local.wait_target_error_us,
            idle_wake_count: local.idle_wake_count,
            terminal_error: self
                .shared
                .publication
                .metrics
                .terminal_error
                .lock()
                .clone(),
            secondary_errors: self
                .shared
                .publication
                .metrics
                .secondary_errors
                .lock()
                .clone(),
            generation_count: self.generation_count,
            generation_status_counts: self
                .shared
                .publication
                .metrics
                .generation_status_counts
                .lock()
                .clone(),
            abort_counts_by_reason: self
                .shared
                .publication
                .metrics
                .abort_counts_by_reason
                .lock()
                .clone(),
            release_outcome: self
                .shared
                .publication
                .metrics
                .terminal_release_outcome
                .lock()
                .clone(),
        }
    }

    pub fn join(&self, timeout: Duration) -> Result<bool, String> {
        if self.shared.lifecycle.lifecycle.load(Ordering::Acquire) == LIFECYCLE_NEW {
            return Err("session has not been started".to_string());
        }
        let (done_lock, done_cv) = &self.shared.lifecycle.completed;
        let done = done_lock
            .lock()
            .map_err(|_| "session completion lock was poisoned".to_string())?;
        let (done, _) = done_cv
            .wait_timeout_while(done, timeout, |done| !*done)
            .map_err(|_| "session completion wait was poisoned".to_string())?;
        if !*done {
            return Ok(false);
        }
        drop(done);
        if let Some(handle) = self.thread_handle.lock().take() {
            handle
                .join()
                .map_err(|_| "native dispatch worker panicked".to_string())?;
        }
        Ok(true)
    }

    pub fn take_telemetry_json(&self) -> Result<String, String> {
        let (done_lock, _) = &self.shared.lifecycle.completed;
        let done = done_lock
            .lock()
            .map_err(|_| "session completion lock was poisoned".to_string())?;
        if !*done {
            return Err("telemetry is available only after worker termination".to_string());
        }
        drop(done);
        let output = self
            .shared
            .publication
            .telemetry_output
            .lock()
            .take()
            .ok_or_else(|| "telemetry has already been taken".to_string())?;
        serde_json::to_string(&output)
            .map_err(|error| format!("failed to serialize native telemetry: {error}"))
    }

    pub fn terminal_outcome(&self) -> Option<&'static str> {
        match self
            .shared
            .lifecycle
            .terminal_outcome
            .load(Ordering::Acquire)
        {
            OUTCOME_NONE => None,
            OUTCOME_FINISHED => Some("finished"),
            OUTCOME_QUIT => Some("quit"),
            OUTCOME_SKIPPED => Some("skipped"),
            OUTCOME_ERROR => Some("error"),
            _ => Some("error"),
        }
    }

    pub fn estimator_state_json(&self) -> Result<String, String> {
        let (done_lock, _) = &self.shared.lifecycle.completed;
        let done = done_lock
            .lock()
            .map_err(|_| "session completion lock was poisoned".to_string())?;
        if !*done {
            return Err("estimator state is available only after worker termination".to_string());
        }
        drop(done);
        self.shared
            .publication
            .estimator_output
            .lock()
            .clone()
            .ok_or_else(|| "native estimator state is unavailable".to_string())
    }
}
