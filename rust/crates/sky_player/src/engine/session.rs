use super::config::{AdmittedNativeSessionOptions, DispatchProfile, NativeSessionOptions};
use super::shared::{
    SessionCommands, SessionLifecycle, SessionPublication, SessionShared, SessionTarget,
    SystemPowerEndpoint,
};
use super::worker::Worker;
use super::*;
use crate::engine::config::{MIN_PRODUCTION_PREROLL_US, TimingOptions, validate_timing_constants};
use crate::engine::{EnginePollSnapshot, EnginePollStatus};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex as StdMutex};
use std::time::Duration;

fn publish_focus_hint(focus_active: &AtomicBool, interrupt: &OwnedEvent, active: bool) {
    if focus_active.swap(active, Ordering::AcqRel) != active {
        let _ = interrupt.signal();
    }
}

fn timeline_rebase_reason(_code: u8) -> Option<String> {
    // Compatibility field only: authored dispatch no longer rebases its
    // timeline and therefore never emits a reason.
    None
}

fn last_missed_down_reason(valid: bool, reason_code: u8) -> Option<String> {
    if !valid {
        return None;
    }
    match reason_code {
        1 => Some("down_unobserved_backlog".to_string()),
        _ => None,
    }
}

fn signal_supervisor_expiry(shared: &SessionShared) {
    shared
        .commands
        .supervisor_expired
        .store(true, Ordering::Release);
    shared
        .commands
        .panic_requested
        .store(true, Ordering::Release);
    let _ = shared.commands.interrupt.signal();
}

fn supervisor_watchdog_loop(
    shared: Arc<SessionShared>,
    qpc_clock: QpcClock,
    lease_timeout_ticks: DurationTicks,
) {
    let (done_lock, done_cv) = &shared.lifecycle.completed;
    let Ok(mut done) = done_lock.lock() else {
        signal_supervisor_expiry(&shared);
        return;
    };

    while !*done {
        let now_ticks = match qpc_clock.now() {
            Ok(ticks) => ticks,
            Err(_) => {
                signal_supervisor_expiry(&shared);
                return;
            }
        };
        match super::worker::supervisor_lease_expired(
            now_ticks,
            lease_timeout_ticks,
            &shared.publication.supervisor_heartbeat_ticks,
        ) {
            Ok(true) | Err(_) => {
                signal_supervisor_expiry(&shared);
                return;
            }
            Ok(false) => {}
        }

        let heartbeat = shared
            .publication
            .supervisor_heartbeat_ticks
            .load(Ordering::Acquire);
        let remaining_ticks = if heartbeat == 0 || heartbeat >= now_ticks.as_u64() {
            lease_timeout_ticks.as_u64()
        } else {
            lease_timeout_ticks
                .as_u64()
                .saturating_sub(now_ticks.as_u64().saturating_sub(heartbeat))
        };
        let remaining_us = qpc_clock
            .duration_to_us(DurationTicks::from_raw(remaining_ticks))
            .unwrap_or(1)
            .max(1);
        let wait_result = done_cv.wait_timeout(done, Duration::from_micros(remaining_us));
        match wait_result {
            Ok((next_done, _)) => done = next_done,
            Err(_) => {
                signal_supervisor_expiry(&shared);
                return;
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn validate_native_schedule_timing(
    schedule: &sky_dispatch_core::model::RuntimeSchedule,
    effective_min_hold_us: u64,
    qpc_clock: QpcClock,
) -> Result<(), String> {
    validate_native_schedule_timing_with_release_gap(schedule, effective_min_hold_us, 0, qpc_clock)
}

pub(crate) fn validate_native_schedule_timing_with_release_gap(
    schedule: &sky_dispatch_core::model::RuntimeSchedule,
    effective_min_hold_us: u64,
    min_release_gap_us: u64,
    qpc_clock: QpcClock,
) -> Result<(), String> {
    sky_dispatch_core::validation::validate_min_hold_and_release_gap_feasibility(
        schedule,
        effective_min_hold_us,
        min_release_gap_us,
    )
    .map_err(|error| format!("native schedule admission failed: {error}"))?;
    let effective_min_hold_ticks = qpc_clock
        .duration_from_us(effective_min_hold_us)
        .map_err(|error| format!("native hold-floor conversion failed: {error:?}"))?;
    let min_release_gap_ticks = qpc_clock
        .duration_from_us(min_release_gap_us)
        .map_err(|error| format!("native release-gap conversion failed: {error:?}"))?;
    sky_dispatch_core::validation::validate_min_hold_and_release_gap_feasibility_ticks(
        schedule,
        effective_min_hold_us,
        effective_min_hold_ticks,
        min_release_gap_us,
        min_release_gap_ticks,
        |microseconds| qpc_clock.timeline_from_us(microseconds).ok(),
    )
    .map_err(|error| format!("native tick-domain admission failed: {error}"))
}

pub(crate) fn validate_native_timing_contract(timing: &TimingOptions) -> Result<(), String> {
    let expected_min_hold_us = timing
        .frame_base_hold_us
        .checked_add(timing.timing_margin_us)
        .ok_or_else(|| {
            "native timing contract overflow: frame_base_hold_us + timing_margin_us".to_string()
        })?;
    if timing.min_hold_us != expected_min_hold_us {
        return Err(format!(
            "native timing contract mismatch: min_hold_us is {}, expected frame_base_hold_us + timing_margin_us = {expected_min_hold_us}",
            timing.min_hold_us
        ));
    }

    let expected_min_release_gap_us = timing
        .frame_us
        .checked_add(timing.timing_margin_us)
        .ok_or_else(|| {
            "native timing contract overflow: frame_us + timing_margin_us".to_string()
        })?;
    if timing.min_release_gap_us != expected_min_release_gap_us {
        return Err(format!(
            "native timing contract mismatch: min_release_gap_us is {}, expected frame_us + timing_margin_us = {expected_min_release_gap_us}",
            timing.min_release_gap_us
        ));
    }

    Ok(())
}

pub struct NativeDispatchSession {
    config: Mutex<Option<AdmittedNativeSessionOptions>>,
    profile: DispatchProfile,
    generation_count: u64,
    shared: Arc<SessionShared>,
    thread_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    watchdog_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemPowerSnapshot {
    pub suspended: bool,
    pub down_blocked: bool,
    pub suspend_notifications: u64,
    pub resume_notifications: u64,
    pub duplicate_notifications: u64,
}

impl NativeDispatchSession {
    pub fn new(options: NativeSessionOptions) -> Result<Self, String> {
        let endpoint = SystemPowerEndpoint::new()?;
        Self::new_with_power_endpoint(options, endpoint)
    }

    pub fn new_with_power_endpoint(
        mut options: NativeSessionOptions,
        power_endpoint: Arc<SystemPowerEndpoint>,
    ) -> Result<Self, String> {
        power_endpoint.reset_for_new_session();
        validate_timing_constants()?;
        validate_native_timing_contract(&options.timing)?;
        // This is the authoritative native admission boundary.  Python calls
        // the same core validator before crossing into Rust, but direct native
        // callers must not be able to construct a session that can only fail
        // after the worker has started.
        let qpc_clock = QpcClock::initialize()
            .map_err(|error| format!("QPC admission failed before session creation: {error:?}"))?;
        let min_release_gap_us = options.timing.min_release_gap_us;
        validate_native_schedule_timing_with_release_gap(
            &options.schedule,
            options.timing.min_hold_us,
            min_release_gap_us,
            qpc_clock,
        )?;
        if !cfg!(windows) && matches!(&options.backend, BackendConfig::Production) {
            return Err("production native dispatch is supported only on Windows".to_string());
        }
        let profile_spec = options
            .instrument_key_profile
            .take()
            .unwrap_or_else(sky_dispatch_win32::input::InstrumentKeyProfileSpec::canonical);
        let instrument_key_profile =
            sky_dispatch_win32::input::InstrumentKeyProfile::try_from_spec(profile_spec)
                .map(sky_dispatch_win32::input::MaterializedInstrumentKeyProfile::from_validated)
                .map_err(|error| format!("native instrument profile admission failed: {error}"))?;
        let initial_heartbeat_ticks = qpc_clock
            .now()
            .map_err(|error| format!("QPC admission failed before session creation: {error:?}"))?;
        let interrupt = power_endpoint.interrupt();
        let total_us = options
            .schedule
            .batches
            .last()
            .map_or(0, |batch| batch.scheduled_us);
        let generation_count = options.schedule.generation_count;
        let metrics = Arc::new(SharedMetrics::default());
        *metrics.requested_priority_mode.lock() = "unknown".to_string();
        *metrics.requested_wait_policy.lock() = "unknown".to_string();
        *metrics.effective_wait_policy.lock() = "unknown".to_string();
        let mut initial_metrics = metrics.snapshot.load();
        initial_metrics.total_us = total_us;
        let _ = metrics.snapshot.try_publish(&initial_metrics);
        let shared = Arc::new(SessionShared {
            commands: SessionCommands {
                interrupt,
                system_power: power_endpoint.state(),
                desired_pause: AtomicBool::new(false),
                quit_requested: AtomicBool::new(false),
                skip_requested: AtomicBool::new(false),
                panic_requested: AtomicBool::new(false),
                supervisor_expired: AtomicBool::new(false),
                // Supervisor-published focus state is the precision-path hint;
                // final admission revalidates only the target/focus atomics.
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
                progress_clock: super::shared::SharedProgressClock::default(),
                telemetry_output: Mutex::new(None),
                priority_acquired: Mutex::new("pending".to_string()),
                supervisor_heartbeat_ticks: AtomicU64::new(initial_heartbeat_ticks.as_u64()),
                startup_requested_ticks: AtomicU64::new(0),
                epoch_qpc: AtomicU64::new(0),
                pre_roll_us: AtomicU64::new(0),
                armed: AtomicBool::new(false),
                startup_ready_ticks: AtomicU64::new(0),
                startup_latency_us: AtomicU64::new(0),
                startup_ready: AtomicBool::new(false),
            },
        });
        let admitted_options = AdmittedNativeSessionOptions {
            options,
            instrument_key_profile,
        };
        Ok(Self {
            profile: admitted_options.options.profile,
            config: Mutex::new(Some(admitted_options)),
            generation_count,
            shared,
            thread_handle: Mutex::new(None),
            watchdog_handle: Mutex::new(None),
        })
    }

    fn live_projection(&self) -> ((u64, bool), u64) {
        let metrics_paused = self
            .shared
            .publication
            .metrics
            .is_paused
            .load(Ordering::Relaxed);
        let anchor = self.shared.publication.progress_clock.load();
        let exposed_paused = anchor
            .map(|value| value.paused && !value.frozen)
            .unwrap_or(metrics_paused);
        let Ok(qpc_clock) = QpcClock::initialize() else {
            return ((0, exposed_paused), 0);
        };
        let Ok(now_qpc) = qpc_clock.now() else {
            return ((0, exposed_paused), 0);
        };
        let elapsed_us = anchor
            .map(|value| value.elapsed_us(now_qpc, qpc_clock))
            .unwrap_or_default();
        let pre_roll_remaining_us = self.pre_roll_remaining_at(now_qpc, qpc_clock);
        ((elapsed_us, exposed_paused), pre_roll_remaining_us)
    }

    pub fn arm(&self, requested_pre_roll_us: u64) -> Result<(), String> {
        validate_timing_constants()?;
        let qpc_clock = QpcClock::initialize()
            .map_err(|error| format!("QPC arm admission failed: {error:?}"))?;
        let effective_pre_roll_us = requested_pre_roll_us.max(MIN_PRODUCTION_PREROLL_US);
        let arm_qpc = qpc_clock
            .now()
            .map_err(|error| format!("QPC arm sample failed: {error:?}"))?;
        let epoch_qpc = arm_qpc
            .checked_add_duration(
                qpc_clock
                    .duration_from_us(effective_pre_roll_us)
                    .map_err(|error| format!("pre-roll conversion failed: {error:?}"))?,
            )
            .map_err(|error| format!("pre-roll epoch arithmetic failed: {error}"))?;
        let lease_timeout_us = self
            .config
            .lock()
            .as_ref()
            .map(|config| config.options.wait.supervisor_lease_timeout_us)
            .ok_or_else(|| "session configuration is no longer available".to_string())?;
        let lease_timeout_ticks = qpc_clock
            .duration_from_us(lease_timeout_us)
            .map_err(|error| format!("lease timeout conversion failed: {error:?}"))?;

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
        let Some(config) = self.config.lock().take() else {
            self.shared
                .lifecycle
                .lifecycle
                .store(LIFECYCLE_POISONED, Ordering::Release);
            let (done_lock, done_cv) = &self.shared.lifecycle.completed;
            if let Ok(mut done) = done_lock.lock() {
                *done = true;
                done_cv.notify_all();
            }
            return Err("session configuration is no longer available".to_string());
        };

        #[cfg(any(test, feature = "test-support"))]
        let timer_lifecycle_context = config.options.timer_lifecycle_context.clone();

        self.shared
            .publication
            .supervisor_heartbeat_ticks
            .store(arm_qpc.as_u64(), Ordering::Release);

        let watchdog_handle = if lease_timeout_ticks != DurationTicks::ZERO {
            let watchdog_shared = Arc::clone(&self.shared);
            match std::thread::Builder::new()
                .name("sky-supervisor-watchdog".to_string())
                .spawn(move || {
                    supervisor_watchdog_loop(watchdog_shared, qpc_clock, lease_timeout_ticks)
                }) {
                Ok(handle) => Some(handle),
                Err(error) => {
                    self.shared
                        .lifecycle
                        .lifecycle
                        .store(LIFECYCLE_POISONED, Ordering::Release);
                    let (done_lock, done_cv) = &self.shared.lifecycle.completed;
                    if let Ok(mut done) = done_lock.lock() {
                        *done = true;
                        done_cv.notify_all();
                    }
                    return Err(format!("failed to spawn supervisor watchdog: {error}"));
                }
            }
        } else {
            None
        };

        let shared = Arc::clone(&self.shared);
        self.shared
            .publication
            .startup_requested_ticks
            .store(arm_qpc.as_u64(), Ordering::Release);
        self.shared
            .publication
            .epoch_qpc
            .store(epoch_qpc.as_u64(), Ordering::Release);
        self.shared
            .publication
            .pre_roll_us
            .store(effective_pre_roll_us, Ordering::Release);
        self.shared.publication.armed.store(true, Ordering::Release);

        let spawn_result = std::thread::Builder::new()
            .name("sky-native-dispatch".to_string())
            .spawn(move || {
                #[cfg(any(test, feature = "test-support"))]
                let worker_result = if let Some(context) = timer_lifecycle_context {
                    sky_dispatch_win32::timer::test_support::with_context(&context, || {
                        catch_unwind(AssertUnwindSafe(|| {
                            Worker::new(config, shared.as_ref(), epoch_qpc).run()
                        }))
                    })
                } else {
                    catch_unwind(AssertUnwindSafe(|| {
                        Worker::new(config, shared.as_ref(), epoch_qpc).run()
                    }))
                };
                #[cfg(not(any(test, feature = "test-support")))]
                let worker_result = catch_unwind(AssertUnwindSafe(|| {
                    Worker::new(config, shared.as_ref(), epoch_qpc).run()
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
                *self.watchdog_handle.lock() = watchdog_handle;
                Ok(())
            }
            Err(error) => {
                self.shared
                    .lifecycle
                    .lifecycle
                    .store(LIFECYCLE_POISONED, Ordering::Release);
                let (done_lock, done_cv) = &self.shared.lifecycle.completed;
                if let Ok(mut done) = done_lock.lock() {
                    *done = true;
                    done_cv.notify_all();
                }
                if let Some(handle) = watchdog_handle {
                    let _ = handle.join();
                }
                Err(format!("failed to spawn native dispatch worker: {error}"))
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn start(&self) -> Result<(), String> {
        self.arm(0)
    }

    #[cfg(test)]
    pub(crate) fn epoch_qpc_for_test(&self) -> QpcTicks {
        QpcTicks::from_raw(self.shared.publication.epoch_qpc.load(Ordering::Acquire))
    }

    #[cfg(test)]
    pub(crate) fn pre_roll_us_for_test(&self) -> u64 {
        self.shared.publication.pre_roll_us.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn supervisor_heartbeat_qpc_for_test(&self) -> QpcTicks {
        QpcTicks::from_raw(
            self.shared
                .publication
                .supervisor_heartbeat_ticks
                .load(Ordering::Acquire),
        )
    }

    #[cfg(test)]
    pub(crate) fn spawn_handles_present_for_test(&self) -> (bool, bool) {
        (
            self.thread_handle.lock().is_some(),
            self.watchdog_handle.lock().is_some(),
        )
    }

    fn pre_roll_remaining_at(&self, now: QpcTicks, clock: QpcClock) -> u64 {
        let epoch_raw = self.shared.publication.epoch_qpc.load(Ordering::Acquire);
        if epoch_raw == 0 {
            return 0;
        }
        let epoch = QpcTicks::from_raw(epoch_raw);
        let Ok(remaining) = epoch.checked_duration_since(now) else {
            return 0;
        };
        clock.duration_to_us(remaining).unwrap_or_default()
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

    pub fn set_live_diagnostics_enabled(&self, enabled: bool) {
        let metrics = &self.shared.publication.metrics;
        if enabled {
            metrics.last_publish_us.store(0, Ordering::Relaxed);
        }
        metrics
            .live_diagnostics_enabled
            .store(enabled, Ordering::Release);
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

    /// Publish a transition-only supervisor focus hint. This wakes the worker
    /// for prompt coarse-gate handling, but never authorizes physical input.
    pub fn set_focus_hint(&self, active: bool) {
        publish_focus_hint(
            &self.shared.commands.focus_active,
            &self.shared.commands.interrupt,
            active,
        );
    }

    /// Record a Windows suspend/resume transition without formatting, logging,
    /// locking, or publishing from the OS callback.
    pub fn notify_system_power(&self, suspended: bool) -> bool {
        self.shared
            .commands
            .system_power
            .notify(suspended, &self.shared.commands.interrupt)
    }

    pub fn notify_system_power_at(
        &self,
        suspended: bool,
        suspend_boundary_qpc: Option<QpcTicks>,
    ) -> bool {
        self.shared.commands.system_power.notify_at(
            suspended,
            suspend_boundary_qpc,
            &self.shared.commands.interrupt,
        )
    }

    /// Disable callback routing after the worker has completed cleanup. The
    /// process-level endpoint can then be reused by a later session without
    /// allowing a stale callback to wake an inactive worker.
    pub fn deactivate_system_power(&self) {
        self.shared.commands.system_power.deactivate();
    }

    pub fn system_power_snapshot(&self) -> SystemPowerSnapshot {
        let (
            suspended,
            down_blocked,
            suspend_notifications,
            resume_notifications,
            duplicate_notifications,
        ) = self.shared.commands.system_power.snapshot();
        SystemPowerSnapshot {
            suspended,
            down_blocked,
            suspend_notifications,
            resume_notifications,
            duplicate_notifications,
        }
    }

    fn poll_status(
        &self,
        lifecycle: u8,
        paused: bool,
        pre_roll_remaining_us: u64,
    ) -> EnginePollStatus {
        match lifecycle {
            LIFECYCLE_NEW => EnginePollStatus::Ready,
            LIFECYCLE_RUNNING
                if self.shared.publication.armed.load(Ordering::Acquire)
                    && pre_roll_remaining_us > 0 =>
            {
                EnginePollStatus::Preroll
            }
            LIFECYCLE_RUNNING if paused => EnginePollStatus::Paused,
            LIFECYCLE_RUNNING => EnginePollStatus::Playing,
            LIFECYCLE_FINISHED => match self
                .shared
                .lifecycle
                .terminal_outcome
                .load(Ordering::Acquire)
            {
                OUTCOME_ERROR => EnginePollStatus::Error,
                OUTCOME_QUIT => EnginePollStatus::Quit,
                OUTCOME_SKIPPED => EnginePollStatus::Skipped,
                _ => EnginePollStatus::Finished,
            },
            LIFECYCLE_POISONED
                if self
                    .shared
                    .publication
                    .metrics
                    .panicked
                    .load(Ordering::Acquire) =>
            {
                EnginePollStatus::Panicked
            }
            LIFECYCLE_POISONED => EnginePollStatus::Poisoned,
            _ => EnginePollStatus::Invalid,
        }
    }

    /// Return only lifecycle/progress state needed by the supervisor loop.
    ///
    /// In particular, this function must not load `SharedMetrics::snapshot`:
    /// diagnostic counters and recent latency samples belong to the slower
    /// HUD snapshot and terminal report paths.
    pub fn poll_state(&self) -> EnginePollSnapshot {
        let lifecycle = self.shared.lifecycle.lifecycle.load(Ordering::Acquire);
        let ((elapsed_us, paused), pre_roll_remaining_us) = self.live_projection();
        EnginePollSnapshot {
            elapsed_us,
            pre_roll_remaining_us,
            is_finished: matches!(lifecycle, LIFECYCLE_FINISHED | LIFECYCLE_POISONED),
            is_paused: paused,
            status: self.poll_status(lifecycle, paused, pre_roll_remaining_us),
        }
    }

    pub fn snapshot_lite(&self) -> EngineProgressSnapshot {
        let lifecycle = self.shared.lifecycle.lifecycle.load(Ordering::Acquire);
        let ((elapsed_us, paused), pre_roll_remaining_us) = self.live_projection();
        let status = self
            .poll_status(lifecycle, paused, pre_roll_remaining_us)
            .as_str();
        let local = self.shared.publication.metrics.snapshot.load();
        EngineProgressSnapshot {
            elapsed_us,
            total_us: local.total_us,
            pre_roll_remaining_us,
            max_lateness_us: local.max_lateness_us,
            late_2ms: local.late_2ms,
            late_5ms: local.late_5ms,
            late_10ms: local.late_10ms,
            max_sendinput_pre_call_lateness_us: local.max_sendinput_pre_call_lateness_us,
            pre_call_lt_250us: local.pre_call_lt_250us,
            pre_call_250_500us: local.pre_call_250_500us,
            pre_call_500_750us: local.pre_call_500_750us,
            pre_call_750_1000us: local.pre_call_750_1000us,
            pre_call_1000_1500us: local.pre_call_1000_1500us,
            pre_call_1500_2000us: local.pre_call_1500_2000us,
            pre_call_ge_2000us: local.pre_call_ge_2000us,
            pre_call_late_2ms: local.pre_call_late_2ms,
            pre_call_late_5ms: local.pre_call_late_5ms,
            pre_call_late_10ms: local.pre_call_late_10ms,
            release_max_us: local.release_max_us,
            release_late_2ms: local.release_late_2ms,
            recent_latencies_us: local.recent_latencies.to_vec(),
            recent_latency_samples_available: self.profile.observer_enabled(),
            is_running: lifecycle == LIFECYCLE_RUNNING,
            is_finished: matches!(lifecycle, LIFECYCLE_FINISHED | LIFECYCLE_POISONED),
            is_paused: paused,
            status: status.to_string(),
            has_terminal_error: self
                .shared
                .publication
                .metrics
                .terminal_error
                .lock()
                .is_some(),
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
            missed_down_boundaries: local.missed_down_boundaries,
            missed_down_keys: local.missed_down_keys,
            missed_unobserved_backlog_boundaries: local.missed_unobserved_backlog_boundaries,
            missed_physical_window_boundaries: local.missed_physical_window_boundaries,
            final_gate_control_rejections: local.final_gate_control_rejections,
            final_gate_target_changes: local.final_gate_target_changes,
            final_gate_focus_losses: local.final_gate_focus_losses,
            final_gate_lease_expirations: local.final_gate_lease_expirations,
            final_sender_window_expirations: local.final_sender_window_expirations,
            release_floor_infeasible_boundaries: local.release_floor_infeasible_boundaries,
            hold_floor_delay_boundaries: local.hold_floor_delay_boundaries,
            max_hold_floor_delay_us: local.max_hold_floor_delay_us,
            release_floor_delay_boundaries: local.release_floor_delay_boundaries,
            max_release_floor_delay_us: local.max_release_floor_delay_us,
            max_missed_lateness_ticks: local.max_missed_lateness_ticks,
            keys_inserted_before_failure: local.keys_inserted_before_failure,
            keys_rolled_back: local.keys_rolled_back,
            rollback_residue_keys: local.rollback_residue_keys,
            production_forensics_available: local.production_forensics_available,
            production_forensics_version: local.production_forensics_version,
            production_hold_pair_samples: local.production_hold_pair_samples,
            production_min_hold_start_after_down_completion_ticks: local
                .production_min_hold_start_after_down_completion_ticks,
            production_hold_floor_violation_count: local.production_hold_floor_violation_count,
            production_release_floor_samples: local.production_release_floor_samples,
            production_min_down_start_after_up_completion_ticks: local
                .production_min_down_start_after_up_completion_ticks,
            production_release_floor_violation_count: local
                .production_release_floor_violation_count,
            production_same_key_overlap_forensics_count: local
                .production_same_key_overlap_forensics_count,
            production_anchor_overwrite_count: local.production_anchor_overwrite_count,
            production_unmatched_up_count: local.production_unmatched_up_count,
            production_anomaly_ring_overwrite_count: local.production_anomaly_ring_overwrite_count,
            production_forensics_anomaly_count: local.production_forensics_anomaly_count,
            input_path_degraded: local.input_path_degraded,
            sendinput_path_degraded: local.sendinput_path_degraded,
            core_post_send_degraded: local.core_post_send_degraded,
            post_send_metrics_available: local.post_send_metrics_available,
            observer_degraded: local.observer_degraded,
            wait_path_degraded: local.wait_path_degraded,
            sendinput_warn_threshold_us: local.sendinput_warn_threshold_us,
            core_post_send_warn_threshold_us: local.core_post_send_warn_threshold_us,
            observer_warn_threshold_us: local.observer_warn_threshold_us,
            wait_warn_threshold_us: local.wait_warn_threshold_us,
            sendinput_degraded_samples: local.sendinput_degraded_samples,
            core_post_send_degraded_samples: local.core_post_send_degraded_samples,
            observer_degraded_samples: local.observer_degraded_samples,
            wait_degraded_samples: local.wait_degraded_samples,
            wait_backend_failures: local.wait_backend_failures,
            wait_clock_failures: local.wait_clock_failures,
            wait_interrupted_count: local.wait_interrupted_count,
            sendinput_window_bad_count: local.sendinput_window_bad_count,
            core_post_send_window_bad_count: local.core_post_send_window_bad_count,
            observer_window_bad_count: local.observer_window_bad_count,
            wait_window_bad_count: local.wait_window_bad_count,
            sendinput_window_sample_count: local.sendinput_window_sample_count,
            core_post_send_window_sample_count: local.core_post_send_window_sample_count,
            observer_window_sample_count: local.observer_window_sample_count,
            wait_window_sample_count: local.wait_window_sample_count,
            timeline_rebase_count: local.timeline_rebase_count,
            timeline_rebase_total_us: local.timeline_rebase_total_us,
            timeline_rebase_max_us: local.timeline_rebase_max_us,
            core_post_send_max_us: local.core_post_send_max_us,
            wake_to_send_max_us: local.wake_to_send_max_us,
            observer_duration_max_us: local.observer_duration_max_us,
            observer_dropped_samples: local.observer_dropped_samples,
            observer_queue_high_watermark: local.observer_queue_high_watermark,
            dispatch_occupancy_max_us: local.dispatch_occupancy_max_us,
            recovered_zero_progress_but_late: local.recovered_zero_progress_but_late,
            recovered_zero_progress_retries: local.recovered_zero_progress_retries,
            recovered_partial_up_retries: local.recovered_partial_up_retries,
        }
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        let lifecycle = self.shared.lifecycle.lifecycle.load(Ordering::Acquire);
        let ((elapsed_us, paused), pre_roll_remaining_us) = self.live_projection();
        let status = self
            .poll_status(lifecycle, paused, pre_roll_remaining_us)
            .as_str();
        let local = self.shared.publication.metrics.snapshot.load();
        let startup_ready = self
            .shared
            .publication
            .startup_ready
            .load(Ordering::Acquire);
        EngineSnapshot {
            elapsed_us,
            total_us: local.total_us,
            pre_roll_remaining_us,
            lateness_us: local.lateness_us,
            max_lateness_us: local.max_lateness_us,
            late_2ms: local.late_2ms,
            late_5ms: local.late_5ms,
            late_10ms: local.late_10ms,
            max_sendinput_pre_call_lateness_us: local.max_sendinput_pre_call_lateness_us,
            pre_call_lt_250us: local.pre_call_lt_250us,
            pre_call_250_500us: local.pre_call_250_500us,
            pre_call_500_750us: local.pre_call_500_750us,
            pre_call_750_1000us: local.pre_call_750_1000us,
            pre_call_1000_1500us: local.pre_call_1000_1500us,
            pre_call_1500_2000us: local.pre_call_1500_2000us,
            pre_call_ge_2000us: local.pre_call_ge_2000us,
            pre_call_late_2ms: local.pre_call_late_2ms,
            pre_call_late_5ms: local.pre_call_late_5ms,
            pre_call_late_10ms: local.pre_call_late_10ms,
            release_max_us: local.release_max_us,
            release_late_2ms: local.release_late_2ms,
            recent_latencies_us: local.recent_latencies.to_vec(),
            recent_latency_samples_available: self.profile.observer_enabled(),
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
            missed_down_boundaries: local.missed_down_boundaries,
            missed_down_keys: local.missed_down_keys,
            missed_unobserved_backlog_boundaries: local.missed_unobserved_backlog_boundaries,
            missed_physical_window_boundaries: local.missed_physical_window_boundaries,
            final_gate_control_rejections: local.final_gate_control_rejections,
            final_gate_target_changes: local.final_gate_target_changes,
            final_gate_focus_losses: local.final_gate_focus_losses,
            final_gate_lease_expirations: local.final_gate_lease_expirations,
            final_sender_window_expirations: local.final_sender_window_expirations,
            release_floor_infeasible_boundaries: local.release_floor_infeasible_boundaries,
            hold_floor_delay_boundaries: local.hold_floor_delay_boundaries,
            max_hold_floor_delay_us: local.max_hold_floor_delay_us,
            last_hold_floor_delay_mask: local.last_hold_floor_delay_mask,
            last_hold_floor_authored_target_qpc_ticks: local
                .last_hold_floor_authored_target_qpc_ticks,
            last_hold_floor_not_before_qpc_ticks: local.last_hold_floor_not_before_qpc_ticks,
            release_floor_delay_boundaries: local.release_floor_delay_boundaries,
            max_release_floor_delay_us: local.max_release_floor_delay_us,
            last_release_floor_delay_mask: local.last_release_floor_delay_mask,
            last_release_floor_authored_target_qpc_ticks: local
                .last_release_floor_authored_target_qpc_ticks,
            last_release_floor_not_before_qpc_ticks: local.last_release_floor_not_before_qpc_ticks,
            max_missed_lateness_ticks: local.max_missed_lateness_ticks,
            keys_inserted_before_failure: local.keys_inserted_before_failure,
            keys_rolled_back: local.keys_rolled_back,
            rollback_residue_keys: local.rollback_residue_keys,
            production_forensics_available: local.production_forensics_available,
            production_forensics_version: local.production_forensics_version,
            production_hold_pair_samples: local.production_hold_pair_samples,
            production_min_hold_start_after_down_completion_ticks: local
                .production_min_hold_start_after_down_completion_ticks,
            production_hold_floor_violation_count: local.production_hold_floor_violation_count,
            production_release_floor_samples: local.production_release_floor_samples,
            production_min_down_start_after_up_completion_ticks: local
                .production_min_down_start_after_up_completion_ticks,
            production_release_floor_violation_count: local
                .production_release_floor_violation_count,
            production_hold_floor_ticks: local.production_hold_floor_ticks,
            production_release_floor_ticks: local.production_release_floor_ticks,
            production_same_key_overlap_forensics_count: local
                .production_same_key_overlap_forensics_count,
            production_anchor_overwrite_count: local.production_anchor_overwrite_count,
            production_unmatched_up_count: local.production_unmatched_up_count,
            production_anomaly_ring_overwrite_count: local.production_anomaly_ring_overwrite_count,
            production_forensics_anomaly_count: local.production_forensics_anomaly_count,
            production_structural_anomaly_count: local.production_structural_anomaly_count,
            production_timing_diagnostic_count: local.production_timing_diagnostic_count,
            last_missed_down_reason: last_missed_down_reason(
                local.last_missed_down_valid,
                local.last_missed_down_reason_code,
            ),
            last_missed_down_source_action_index: local
                .last_missed_down_valid
                .then_some(local.last_missed_down_source_action_index),
            last_missed_down_mask: local.last_missed_down_mask,
            last_missed_down_lateness_ticks: local.last_missed_down_lateness_ticks,
            chord_integrity_lost: local.chord_integrity_lost,
            lead_saturation_count_down: local.lead_saturation_count_down.to_vec(),
            lead_saturation_count_up: local.lead_saturation_count_up.to_vec(),
            positive_residual_at_cap: local.positive_residual_at_cap,
            recovered_zero_progress_but_late: local.recovered_zero_progress_but_late,
            recovered_zero_progress_retries: local.recovered_zero_progress_retries,
            recovered_partial_up_retries: local.recovered_partial_up_retries,
            outcome: self.terminal_outcome().map(str::to_string),
            requested_rt_priority_mode: self
                .shared
                .publication
                .metrics
                .requested_priority_mode
                .lock()
                .clone(),
            rt_priority_acquired: self.shared.publication.priority_acquired.lock().clone(),
            requested_wait_policy: self
                .shared
                .publication
                .metrics
                .requested_wait_policy
                .lock()
                .clone(),
            effective_wait_policy: self
                .shared
                .publication
                .metrics
                .effective_wait_policy
                .lock()
                .clone(),
            startup_calibration_executed: local.startup_calibration_executed,
            startup_calibration_sample_count: local.startup_calibration_sample_count,
            startup_wake_error_p50_us: local.wake_error_p50_us,
            startup_wake_error_p95_us: local.wake_error_p95_us,
            startup_wake_error_p99_us: local.wake_error_p99_us,
            startup_wake_error_max_us: local.wake_error_max_us,
            startup_wake_error_robust_us: local.startup_wake_error_robust_us,
            effective_spin_threshold_us: local.effective_spin_threshold_us,
            spin_threshold_source: crate::engine::config::SpinThresholdSource::from_raw(
                local.spin_threshold_source,
            )
            .label()
            .to_string(),
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
            core_post_send_degraded: local.core_post_send_degraded,
            post_send_metrics_available: local.post_send_metrics_available,
            observer_degraded: local.observer_degraded,
            wait_path_degraded: local.wait_path_degraded,
            sendinput_warn_threshold_us: local.sendinput_warn_threshold_us,
            core_post_send_warn_threshold_us: local.core_post_send_warn_threshold_us,
            observer_warn_threshold_us: local.observer_warn_threshold_us,
            wait_warn_threshold_us: local.wait_warn_threshold_us,
            sendinput_degraded_samples: local.sendinput_degraded_samples,
            core_post_send_degraded_samples: local.core_post_send_degraded_samples,
            observer_degraded_samples: local.observer_degraded_samples,
            wait_degraded_samples: local.wait_degraded_samples,
            wait_backend_failures: local.wait_backend_failures,
            wait_clock_failures: local.wait_clock_failures,
            wait_interrupted_count: local.wait_interrupted_count,
            sendinput_window_bad_count: local.sendinput_window_bad_count,
            core_post_send_window_bad_count: local.core_post_send_window_bad_count,
            observer_window_bad_count: local.observer_window_bad_count,
            wait_window_bad_count: local.wait_window_bad_count,
            sendinput_window_sample_count: local.sendinput_window_sample_count,
            core_post_send_window_sample_count: local.core_post_send_window_sample_count,
            observer_window_sample_count: local.observer_window_sample_count,
            wait_window_sample_count: local.wait_window_sample_count,
            timeline_rebase_count: local.timeline_rebase_count,
            timeline_rebase_total_us: local.timeline_rebase_total_us,
            timeline_rebase_max_us: local.timeline_rebase_max_us,
            timeline_rebase_last_reason: timeline_rebase_reason(local.timeline_rebase_last_reason),
            dispatch_occupancy_max_us: local.dispatch_occupancy_max_us,
            send_down_degraded_samples: local.send_down_degraded_samples,
            send_up_degraded_samples: local.send_up_degraded_samples,
            send_mixed_degraded_samples: local.send_mixed_degraded_samples,
            send_down_warn_threshold_us: local.send_down_warn_threshold_us,
            send_up_warn_threshold_us: local.send_up_warn_threshold_us,
            send_mixed_warn_threshold_us: local.send_mixed_warn_threshold_us,
            wait_target_error_us: local.wait_target_error_us,
            idle_wake_count: local.idle_wake_count,
            wait_planned_gap_max_us: local.wait_planned_gap_max_us,
            wait_planned_gap_hot_count: local.wait_planned_gap_hot_count,
            wait_planned_gap_cold_count: local.wait_planned_gap_cold_count,
            wait_planned_gap_hot_lateness_max_us: local.wait_planned_gap_hot_lateness_max_us,
            wait_planned_gap_cold_lateness_max_us: local.wait_planned_gap_cold_lateness_max_us,
            physical_target_to_wake_max_us: local.physical_target_to_wake_max_us,
            core_post_send_max_us: local.core_post_send_max_us,
            wake_to_final_policy_max_us: local.wake_to_final_policy_max_us,
            final_policy_to_pre_call_max_us: local.final_policy_to_pre_call_max_us,
            sendinput_duration_max_us: local.sendinput_duration_max_us,
            wake_to_send_max_us: local.wake_to_send_max_us,
            observer_duration_max_us: local.observer_duration_max_us,
            observer_dropped_samples: local.observer_dropped_samples,
            observer_queue_high_watermark: local.observer_queue_high_watermark,
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
            startup_ready,
            startup_latency_us: startup_ready.then(|| {
                self.shared
                    .publication
                    .startup_latency_us
                    .load(Ordering::Relaxed)
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn startup_ticks(&self) -> (u64, u64) {
        (
            self.shared
                .publication
                .startup_requested_ticks
                .load(Ordering::Acquire),
            self.shared
                .publication
                .startup_ready_ticks
                .load(Ordering::Relaxed),
        )
    }

    #[cfg(test)]
    pub(super) fn shared_for_test(&self) -> &SessionShared {
        &self.shared
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
        if let Some(handle) = self.watchdog_handle.lock().take() {
            handle
                .join()
                .map_err(|_| "supervisor watchdog panicked".to_string())?;
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
}

#[cfg(test)]
mod tests {
    use super::publish_focus_hint;
    use sky_dispatch_win32::event::OwnedEvent;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn focus_hint_signals_only_on_state_transition() {
        let focus_active = AtomicBool::new(true);
        let interrupt = OwnedEvent::new_auto_reset().expect("interrupt event");

        publish_focus_hint(&focus_active, &interrupt, true);
        assert_eq!(interrupt.signal_generation(), 0);
        assert!(!interrupt.try_take());

        publish_focus_hint(&focus_active, &interrupt, false);
        assert_eq!(interrupt.signal_generation(), 1);
        assert!(interrupt.try_take());

        publish_focus_hint(&focus_active, &interrupt, false);
        assert_eq!(interrupt.signal_generation(), 1);
        assert!(!interrupt.try_take());
    }

    #[test]
    fn supervisor_watchdog_is_session_owned_control_plane_only() {
        let source = include_str!("session.rs").replace("\r\n", "\n");
        let watchdog = source
            .split("fn supervisor_watchdog_loop")
            .nth(1)
            .expect("session watchdog implementation");
        assert!(watchdog.contains("wait_timeout"));
        assert!(watchdog.contains("supervisor_expired"));
        assert!(watchdog.contains("panic_requested"));
        assert!(watchdog.contains("interrupt.signal"));
        assert!(source.contains("sky-supervisor-watchdog"));
        assert!(source.contains("watchdog_handle"));

        let heartbeat_store = source
            .find(".supervisor_heartbeat_ticks\n            .store(arm_qpc.as_u64()")
            .expect("arm heartbeat publication");
        let watchdog_spawn = source
            .find(".name(\"sky-supervisor-watchdog\".to_string())")
            .expect("watchdog spawn");
        assert!(
            heartbeat_store < watchdog_spawn,
            "watchdog must not observe the construction-time heartbeat"
        );
    }
}
