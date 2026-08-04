use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::coordinator::{CoordinatorError, RuntimeDispatchCoordinator};
use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator};
use sky_dispatch_core::model::{ActionKind, RuntimeSchedule};
use sky_dispatch_core::time::{
    DurationTicks, SEND_COLD_THRESHOLD_US, TimeArithmeticError, TimelineTicks,
};
use sky_dispatch_win32::clock::qpc_us_to_ticks;
use sky_dispatch_win32::clock::{QpcClock, QpcError, QpcTicks, qpc_frequency_checked};
use sky_dispatch_win32::cpu::{current_process_cpu_time_us, current_thread_cpu_time_us};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::input::{PlatformSendResult, ReleaseAllOutcome, TrackedKeyState};
use sky_dispatch_win32::mmcss::{MmcssGuard, PriorityMode};
use sky_dispatch_win32::power::PowerThrottlingGuard;
use sky_dispatch_win32::wait::{HybridWaiter, WaitFailure, WaitOutcome, WakeErrorStats};
use smallvec::SmallVec;
use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex as StdMutex};
use std::time::Duration;
const LIFECYCLE_NEW: u8 = 0;
const LIFECYCLE_RUNNING: u8 = 1;
const LIFECYCLE_FINISHED: u8 = 2;
const LIFECYCLE_POISONED: u8 = 3;
const OUTCOME_NONE: u8 = 0;
const OUTCOME_FINISHED: u8 = 1;
const OUTCOME_QUIT: u8 = 2;
const OUTCOME_SKIPPED: u8 = 3;
const OUTCOME_ERROR: u8 = 4;
const PAUSED_POLL_US: u64 = 2_000;
const CORE_WARMUP_SPIN_MAX_US: u64 = 500;
const CPU_METRICS_SAMPLE_INTERVAL_US: u64 = 100_000;
const INPUT_PATH_WINDOW_CAPACITY: usize = 64;
const STRICT_RETRY_LATE_THRESHOLD_US: u64 = 2_000;
const HARD_LATE_ABORT_THRESHOLD_US: u64 = 20_000;
const STRICT_SATURATION_ABORT_STREAK: u8 = 3;
const STARTUP_WAKE_GUARD_US: u64 = 1_000;
const RELEASE_RETRY_BACKOFF_US: [u64; 4] = [2_000, 5_000, 10_000, 20_000];
use crate::engine::config::{DispatchProfile, TelemetryMode, WorkerConfig};
use crate::engine::snapshot::EngineSnapshot;
use crate::engine::telemetry::collector::*;
use crate::engine::telemetry::metrics::*;
use crate::engine::telemetry::trace::*;
#[cfg(any(test, feature = "test-support"))]
use crate::engine::test_support::command_timing::*;
#[cfg(any(test, feature = "test-support"))]
use crate::engine::test_support::fault_injection::*;
#[cfg(any(test, feature = "test-support"))]
use crate::engine::test_support::mock_sender::*;
use crate::engine::worker::session::NativeDispatchSession;
use parking_lot::Mutex;

pub fn cpu_metrics_sample_due(now_us: u64, last_sample_us: u64, interval_us: u64) -> bool {
    now_us.saturating_sub(last_sample_us) >= interval_us
}

#[allow(clippy::too_many_arguments)]
pub fn run_worker(
    config: WorkerConfig,
    interrupt: &OwnedEvent,
    desired_pause: &AtomicBool,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    metrics: &SharedMetrics,
    telemetry_output: &Mutex<Option<NativeTelemetryOutput>>,
    priority_acquired: &Mutex<String>,
    estimator_output: &Mutex<Option<String>>,
    supervisor_heartbeat_ticks: &AtomicU64,
    #[cfg(any(test, feature = "test-support"))] command_timing: &CommandTimingState,
) -> u8 {
    #[cfg(any(test, feature = "test-support"))]
    let _command_timing_cleanup = CommandTimingCleanup(command_timing);
    let qpc_clock = match QpcClock::initialize() {
        Ok(clock) => clock,
        Err(error) => {
            *metrics.last_error.lock() = Some(format!("QPC admission failed: {error:?}"));
            return 1;
        }
    };
    let focus_loss_fault = config.fault_script.focus_loss_after_due_before_send;
    let wait_fault = config.fault_script.wait_failure;
    let mut backend = if config.mock_backend {
        let script = Arc::new(config.fault_script);
        let latency_base_us = config.mock_latency_base_us;
        let latency_per_key_us = config.mock_latency_per_key_us;
        // call_index counts every emitter invocation (Down + Up, in order).
        let call_index = Arc::new(AtomicU64::new(0));
        let script_emitter = Arc::clone(&script);
        let call_index_emitter = Arc::clone(&call_index);
        TrackedKeyState::with_emitter(move |codes, _key_up| {
            let idx = call_index_emitter.fetch_add(1, Ordering::Relaxed) as usize;

            // Base per-call latency (mirrors old mock_latency_base_us / per_key).
            let base_latency_us = latency_base_us
                .saturating_add(latency_per_key_us.saturating_mul(codes.len() as u64));
            let sender_started_ticks = match qpc_clock.now() {
                Ok(ticks) => ticks,
                Err(error) => {
                    return mock_platform_send_result_from_started_ticks(
                        qpc_clock,
                        Err(error),
                        codes.len() as u32,
                        0,
                        0,
                        0,
                    );
                }
            };
            if base_latency_us > 0 {
                // Keep the artificial sender work after the sender start
                // boundary so test-support timing matches the real seam.
                std::thread::sleep(Duration::from_micros(base_latency_us));
            }

            match script_emitter.resolve(idx) {
                None | Some(InjectedSendOutcome::Full { latency_ticks: 0 }) => {
                    // Fast path: full success, no extra latency.
                    mock_platform_send_result_from_started_ticks(
                        qpc_clock,
                        Ok(sender_started_ticks),
                        codes.len() as u32,
                        codes.len() as u32,
                        0,
                        0,
                    )
                }
                Some(InjectedSendOutcome::Full { latency_ticks }) => {
                    mock_platform_send_result_from_started_ticks(
                        qpc_clock,
                        Ok(sender_started_ticks),
                        codes.len() as u32,
                        codes.len() as u32,
                        0,
                        *latency_ticks,
                    )
                }
                Some(InjectedSendOutcome::Zero {
                    latency_ticks,
                    win32_error,
                }) => mock_platform_send_result_from_started_ticks(
                    qpc_clock,
                    Ok(sender_started_ticks),
                    codes.len() as u32,
                    0,
                    *win32_error,
                    *latency_ticks,
                ),
                Some(InjectedSendOutcome::Partial {
                    inserted,
                    latency_ticks,
                    win32_error,
                }) => {
                    let inserted = (*inserted as u32).min(codes.len() as u32);
                    mock_platform_send_result_from_started_ticks(
                        qpc_clock,
                        Ok(sender_started_ticks),
                        codes.len() as u32,
                        inserted,
                        *win32_error,
                        *latency_ticks,
                    )
                }
                Some(InjectedSendOutcome::Stall { duration_ticks }) => {
                    // Spin-stall: hold the emitter without sending any key.
                    // This simulates a scheduler stall or OS freeze without
                    // actually blocking the thread (consistent with RT discipline).
                    mock_platform_send_result_from_started_ticks(
                        qpc_clock,
                        Ok(sender_started_ticks),
                        codes.len() as u32,
                        0,
                        0,
                        *duration_ticks,
                    )
                }
                Some(InjectedSendOutcome::PanicAfterSend) => {
                    let _ = mock_platform_send_result_from_started_ticks(
                        qpc_clock,
                        Ok(sender_started_ticks),
                        codes.len() as u32,
                        codes.len() as u32,
                        0,
                        0,
                    );
                    panic!("fault injection: panic after send before commit");
                }
                Some(InjectedSendOutcome::QpcFailureAfterSend) => {
                    let mut result = mock_platform_send_result_from_started_ticks(
                        qpc_clock,
                        Ok(sender_started_ticks),
                        codes.len() as u32,
                        codes.len() as u32,
                        0,
                        0,
                    );
                    result.timing_error = Some(QpcError::CounterUnavailable);
                    result
                }
            }
        })
    } else {
        TrackedKeyState::with_qpc_clock(qpc_clock)
    };
    let admission_failure =
        |backend: &mut TrackedKeyState, metrics: &SharedMetrics, primary_error: String| {
            let verification_hwnd = target_hwnd.load(Ordering::Acquire);
            let cleanup = backend.release_all_full_instrument(verification_hwnd);
            let message = if release_state_verified(backend, &cleanup) {
                primary_error
            } else {
                format!(
                    "{primary_error}; admission cleanup failed: {}",
                    describe_release_outcome(&cleanup)
                )
            };
            *metrics.last_error.lock() = Some(message);
            1
        };
    let mut local_metrics = WorkerMetricsLocal::default();
    let mut force_full_cleanup = false;
    let mut terminal_error: Option<String> = None;
    let mut secondary_errors: Vec<String> = Vec::new();
    let mut last_published_error: Option<String> = None;
    let mut focus_loss_fault_injected = false;
    let power_guard = PowerThrottlingGuard::disable_current_thread();
    local_metrics.power_throttling_disabled = power_guard.is_active();
    let priority_guard = MmcssGuard::acquire(config.priority_mode);
    *priority_acquired.lock() = priority_guard.acquired().to_string();
    let waiter = HybridWaiter::with_options(config.enable_waitable_timer, config.enable_event_wait);
    *metrics.wait_strategy_acquired.lock() = waiter.mode().to_string();
    let mut estimator =
        match SendLatencyEstimator::try_new(0.2, config.max_lead_us, config.allowed_count) {
            Ok(estimator) => estimator,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("invalid estimator configuration: {error}"),
                );
            }
        };
    if let Some(raw) = &config.estimator_state_json {
        // Timing caches are disposable runtime evidence. Any schema or
        // provenance mismatch starts from the conservative prior; it must not
        // turn a playback session into a keyboard cleanup failure.
        let _ = estimator.import_state(raw);
    }
    let min_hold_ticks = match qpc_clock.duration_from_us(config.min_hold_us) {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("min-hold conversion failed: {error:?}"),
            );
        }
    };
    let hard_late_abort_threshold_ticks =
        match qpc_clock.duration_from_us(HARD_LATE_ABORT_THRESHOLD_US) {
            Ok(ticks) => ticks,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("hard late-abort threshold conversion failed: {error:?}"),
                );
            }
        };
    let retry_late_threshold_ticks =
        match qpc_clock.duration_from_us(STRICT_RETRY_LATE_THRESHOLD_US) {
            Ok(ticks) => ticks,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("retry-late threshold conversion failed: {error:?}"),
                );
            }
        };
    let strict_down_completion_late_ticks =
        match qpc_clock.duration_from_us(config.strict_down_completion_late_us) {
            Ok(ticks) => ticks,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("strict note-on threshold conversion failed: {error:?}"),
                );
            }
        };
    let strict_up_completion_late_ticks =
        match qpc_clock.duration_from_us(config.strict_up_completion_late_us) {
            Ok(ticks) => ticks,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("strict note-off threshold conversion failed: {error:?}"),
                );
            }
        };
    let focus_restore_grace_ticks = match qpc_clock.duration_from_us(config.focus_restore_grace_us)
    {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("focus grace conversion failed: {error:?}"),
            );
        }
    };
    let paused_poll_ticks = match qpc_clock.duration_from_us(PAUSED_POLL_US) {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("paused polling conversion failed: {error:?}"),
            );
        }
    };
    let cold_threshold_ticks = match qpc_clock.duration_from_us(SEND_COLD_THRESHOLD_US) {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("cold threshold conversion failed: {error:?}"),
            );
        }
    };
    let core_warmup_ticks = match qpc_clock
        .duration_from_us(config.core_warmup_budget_us.min(CORE_WARMUP_SPIN_MAX_US))
    {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("core warmup conversion failed: {error:?}"),
            );
        }
    };
    let lease_timeout_ticks = match qpc_clock.duration_from_us(config.supervisor_lease_timeout_us) {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("lease timeout conversion failed: {error:?}"),
            );
        }
    };
    let retry_backoff_ticks: [DurationTicks; RELEASE_RETRY_BACKOFF_US.len()] =
        match RELEASE_RETRY_BACKOFF_US
            .map(|delay| qpc_clock.duration_from_us(delay))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .and_then(|values| {
                values
                    .try_into()
                    .map_err(|_| sky_dispatch_win32::clock::TimeConversionError::Overflow)
            }) {
            Ok(backoff) => backoff,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("retry backoff conversion failed: {error:?}"),
                );
            }
        };
    let delivery_margin_ticks = DurationTicks::ZERO;
    let mut coordinator = match RuntimeDispatchCoordinator::try_new_ticks(
        config.schedule,
        config.min_hold_us,
        min_hold_ticks,
        0,
        delivery_margin_ticks,
        |us| {
            qpc_clock
                .timeline_from_us(us)
                .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
        },
    ) {
        Ok(coordinator) => coordinator,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("coordinator construction failed: {error}"),
            );
        }
    };
    local_metrics.total_us = match coordinator.effective_total_ticks().and_then(|ticks| {
        qpc_clock
            .duration_to_us(DurationTicks::from_raw(ticks.as_u64()))
            .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
    }) {
        Ok(total_us) => total_us,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("total timeline conversion failed: {error}"),
            );
        }
    };
    let mut telemetry = TelemetryCollector::new(config.telemetry_mode, config.telemetry_capacity);
    let mut abort_counts: HashMap<&'static str, u64> = HashMap::with_capacity(6);
    let mut effective_spin_threshold_us = config.spin_threshold_us;
    let _ = interrupt.try_take();
    if config.enable_adaptive_spin
        && let Some(stats) = waiter.probe_wake_error_stats(qpc_clock, interrupt, 10)
    {
        publish_wake_error_stats(stats, &mut local_metrics);
        effective_spin_threshold_us = derive_spin_threshold_us(stats.p95_us, config.spin_floor_us);
    }
    local_metrics.effective_spin_threshold_us = effective_spin_threshold_us;
    let initial_now_ticks = match qpc_clock.now() {
        Ok(now) => now,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("QPC admission failed: {error:?}"),
            );
        }
    };
    let initial_now_us =
        match qpc_clock.duration_to_us(DurationTicks::from_raw(initial_now_ticks.as_u64())) {
            Ok(now) => now,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("QPC admission conversion failed: {error:?}"),
                );
            }
        };
    let effective_spin_threshold_ticks =
        match qpc_clock.duration_from_us(effective_spin_threshold_us) {
            Ok(ticks) => ticks,
            Err(error) => {
                return admission_failure(
                    &mut backend,
                    metrics,
                    format!("spin threshold conversion failed: {error:?}"),
                );
            }
        };
    // Cold/hot classification must use physical QPC time.  The authored
    // playback clock deliberately freezes during pause/focus recovery, so a
    // logical gap cannot tell us whether the CPU/input path has gone cold.
    let mut last_send_qpc_ticks: Option<QpcTicks> = None;
    let mut pending_pre_send_spin_us = 0;
    let mut down_saturation_positive_streak: u8 = 0;
    let mut up_saturation_positive_streak: u8 = 0;
    let mut send_duration_window = VecDeque::with_capacity(INPUT_PATH_WINDOW_CAPACITY);
    let mut send_over_warn_count = 0usize;
    let mut input_path_warn_started_us = None;
    let mut send_pure_window = VecDeque::with_capacity(INPUT_PATH_WINDOW_CAPACITY);
    let mut send_pure_over_warn_count = 0usize;
    let mut send_pure_warn_started_us = None;
    let mut bookkeeping_window = VecDeque::with_capacity(INPUT_PATH_WINDOW_CAPACITY);
    let mut bookkeeping_over_warn_count = 0usize;
    let mut bookkeeping_warn_started_us = None;
    // Keep the logical authored timeline at zero while placing the physical
    // anchor in the future.  This gives a t=0 action a real opportunity to
    // dispatch early by its measured lead instead of being forced late by the
    // worker prologue.
    let startup_class = LatencyClass::Cold;
    let startup_lead_us = if config.dispatch_lead_us > 0 {
        config.dispatch_lead_us
    } else if config.enable_adaptive_lead {
        estimator
            .estimate_lead_with_class_and_policy(
                ActionKind::Down,
                coordinator.next_authored_polyphony(),
                startup_class,
                config.strict_timing,
            )
            .applied_us
    } else {
        0
    };
    let startup_lead_ticks = match qpc_clock.duration_from_us(startup_lead_us) {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("startup lead conversion failed: {error:?}"),
            );
        }
    };
    let startup_guard_ticks = (|| {
        let wake_guard = qpc_clock
            .duration_from_us(STARTUP_WAKE_GUARD_US)
            .map_err(|error| format!("{error:?}"))?;
        let with_spin = wake_guard
            .checked_add(effective_spin_threshold_ticks)
            .map_err(|error| error.to_string())?;
        with_spin
            .checked_add(core_warmup_ticks)
            .map_err(|error| error.to_string())
    })();
    let startup_guard_ticks = match startup_guard_ticks {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("startup guard conversion failed: {error}"),
            );
        }
    };
    let startup_anchor_ticks = match initial_now_ticks
        .checked_add_duration(startup_guard_ticks)
        .and_then(|ticks| ticks.checked_add_duration(startup_lead_ticks))
    {
        Ok(ticks) => ticks,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("startup anchor arithmetic failed: {error}"),
            );
        }
    };
    let mut clock_state = match PlaybackClockState::new(
        startup_anchor_ticks,
        sky_dispatch_core::time::DurationTicks::from_raw(0),
    ) {
        Ok(clock) => clock,
        Err(error) => {
            return admission_failure(
                &mut backend,
                metrics,
                format!("playback clock initialization failed: {error}"),
            );
        }
    };
    let mut startup_gate = coordinator
        .batch_scheduled_ticks
        .first()
        .copied()
        .map(|scheduled_ticks| (scheduled_ticks, startup_lead_ticks));
    let mut focus_restore_started_ticks: Option<QpcTicks> = None;
    let mut verified_target: Option<TargetStamp> = None;
    let start_wall_time_us = initial_now_us;
    let start_thread_cpu_us = current_thread_cpu_time_us();
    let start_process_cpu_us = current_process_cpu_time_us();
    let mut last_cpu_metrics_sample_us = start_wall_time_us;
    let qpc_admission_error = qpc_frequency_checked()
        .err()
        .map(|error| format!("QPC frequency unavailable: {error:?}"))
        .or_else(|| {
            qpc_clock
                .now()
                .err()
                .map(|error| format!("QPC counter unavailable: {error:?}"))
        })
        .or_else(|| {
            config
                .strict_timing
                .then(|| waiter.initial_failure().map(wait_failure_message))
                .flatten()
        });
    if let Some(error) = qpc_admission_error {
        force_full_cleanup = true;
        terminal_error = Some(error);
    }
    if wait_fault {
        force_full_cleanup = true;
        terminal_error = Some("wait failure injected".to_string());
    }

    // Every QPC query after admission is part of the worker's correctness
    // boundary. A failed query is terminal and must take the cleanup path;
    // it must never become timestamp zero or a best-effort continuation.
    macro_rules! qpc_us_or_terminal {
        () => {{
            match qpc_clock.now().and_then(|ticks| {
                qpc_clock
                    .duration_to_us(DurationTicks::from_raw(ticks.as_u64()))
                    .map_err(|_| QpcError::ConversionOverflow)
            }) {
                Ok(value) => value,
                Err(error) => {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("QPC runtime failure: {error:?}"));
                    break;
                }
            }
        }};
    }

    macro_rules! qpc_ticks_or_terminal {
        () => {{
            match qpc_clock.now() {
                Ok(value) => value,
                Err(error) => {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("QPC runtime failure: {error:?}"));
                    break;
                }
            }
        }};
    }

    macro_rules! qpc_ticks_to_us_or_terminal {
        ($ticks:expr) => {{
            match qpc_clock.duration_to_us(DurationTicks::from_raw($ticks.as_u64())) {
                Ok(value) => value,
                Err(error) => {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("QPC conversion failure: {error:?}"));
                    break;
                }
            }
        }};
    }

    let worker_result = catch_unwind(AssertUnwindSafe(|| {
        if terminal_error.is_some() {
            return;
        }
        let mut allow_pre_epoch_startup_dispatch = false;
        while !coordinator.is_finished() {
            let loop_start_ticks = qpc_ticks_or_terminal!();
            let loop_start_us = qpc_ticks_to_us_or_terminal!(loop_start_ticks);
            local_metrics.playback_wall_time_us = loop_start_us.saturating_sub(start_wall_time_us);
            if cpu_metrics_sample_due(
                loop_start_us,
                last_cpu_metrics_sample_us,
                CPU_METRICS_SAMPLE_INTERVAL_US,
            ) {
                local_metrics.worker_cpu_time_us =
                    current_thread_cpu_time_us().saturating_sub(start_thread_cpu_us);
                local_metrics.process_cpu_time_us =
                    current_process_cpu_time_us().saturating_sub(start_process_cpu_us);
                last_cpu_metrics_sample_us = loop_start_us;
            }
            if local_metrics.playback_wall_time_us > 0 {
                local_metrics.spin_duty_cycle_ppm = (local_metrics.spin_time_us as u128 * 1_000_000
                    / local_metrics.playback_wall_time_us as u128)
                    as u64;
            }
            try_publish_metrics(&local_metrics, metrics, loop_start_us, false);
            match supervisor_lease_expired(
                loop_start_ticks,
                lease_timeout_ticks,
                supervisor_heartbeat_ticks,
            ) {
                Ok(true) => {
                    force_full_cleanup = true;
                    terminal_error = Some("supervisor_lease_expired".to_string());
                    break;
                }
                Ok(false) => {}
                Err(error) => {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("QPC runtime failure: {error:?}"));
                    break;
                }
            }
            if quit_requested.load(Ordering::Acquire) || skip_requested.load(Ordering::Acquire) {
                break;
            }
            if panic_requested.swap(false, Ordering::AcqRel) {
                verified_target = None;
                let panic_release =
                    backend.release_all_full_instrument(target_hwnd.load(Ordering::Acquire));
                if !release_state_verified(&backend, &panic_release) {
                    record_termination_error(
                        &mut terminal_error,
                        &mut secondary_errors,
                        format!(
                            "panic cleanup release verification failed: {}",
                            describe_release_outcome(&panic_release)
                        ),
                    );
                }
                cancel_coordinator_or_terminal(
                    &mut coordinator,
                    &mut force_full_cleanup,
                    &mut terminal_error,
                    &mut secondary_errors,
                );
                *abort_counts.entry("panic").or_insert(0) += 1;
                publish_backend_metrics(
                    &backend,
                    &mut local_metrics,
                    metrics,
                    &mut last_published_error,
                );
                try_publish_metrics(&local_metrics, metrics, qpc_us_or_terminal!(), true);
                terminal_error = Some("panic_release_requested".to_string());
                break;
            }

            let mut now_ticks = qpc_ticks_or_terminal!();
            let focus_ok = focus_matches(config.require_focus, focus_active, target_hwnd);
            let manual_pause = desired_pause.load(Ordering::Acquire);
            #[cfg(any(test, feature = "test-support"))]
            if command_timing.needs_observation() {
                let observed_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("QPC pause observation failed: {error:?}"));
                        break;
                    }
                };
                command_timing.observe_pause(observed_ticks);
            }

            if !focus_ok {
                verified_target = None;
                focus_restore_started_ticks = None;
                if !clock_state.has_pause_reason("focus") {
                    verified_target = None;
                    if let Err(error) = suspend_live_input(
                        &mut backend,
                        &mut coordinator,
                        target_hwnd.load(Ordering::Acquire),
                    ) {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("focus suspension failed: {error}"));
                        break;
                    }
                    *abort_counts.entry("focus_lost").or_insert(0) += 1;
                    if let Err(error) = clock_state.enter_pause("focus", now_ticks) {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("playback clock failure: {error}"));
                        break;
                    }
                    publish_backend_metrics(
                        &backend,
                        &mut local_metrics,
                        metrics,
                        &mut last_published_error,
                    );
                    try_publish_metrics(&local_metrics, metrics, qpc_us_or_terminal!(), true);
                }
            } else if clock_state.has_pause_reason("focus") {
                let restored_at = *focus_restore_started_ticks.get_or_insert(now_ticks);
                let focus_grace_elapsed = match now_ticks.checked_duration_since(restored_at) {
                    Ok(elapsed) => elapsed,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("focus grace clock failure: {error}"));
                        break;
                    }
                };
                if focus_grace_elapsed >= focus_restore_grace_ticks {
                    // Second idempotent release happens while the restored
                    // target is foreground, before playback can resume.
                    let preflight_target = load_target_stamp(target_hwnd, target_generation);
                    verified_target = None;
                    if let Err(error) =
                        suspend_live_input(&mut backend, &mut coordinator, preflight_target.hwnd)
                    {
                        verified_target = None;
                        force_full_cleanup = true;
                        terminal_error = Some(format!("focus restoration failed: {error}"));
                        break;
                    }
                    if let Err(error) = ensure_preflight_for_target(
                        &backend,
                        preflight_target,
                        &mut verified_target,
                    ) {
                        verified_target = None;
                        force_full_cleanup = true;
                        terminal_error = Some(format!(
                            "instrument key preflight failed during focus restoration; release the 15 instrument keys before playback: {error}"
                        ));
                        break;
                    }
                    if !focus_matches_hwnd(
                        config.require_focus,
                        focus_active,
                        preflight_target.hwnd,
                    ) || !target_stamp_still_current(
                        target_hwnd,
                        target_generation,
                        preflight_target,
                    ) {
                        verified_target = None;
                        focus_restore_started_ticks = None;
                        continue;
                    }
                    // Cleanup can include bounded backend retries. Re-sample
                    // QPC after it completes so that the cleanup interval is
                    // included in the focus pause rather than lost from the
                    // playback clock.
                    let resumed_ticks = qpc_ticks_or_terminal!();
                    if let Err(error) = clock_state.exit_pause("focus", resumed_ticks) {
                        verified_target = None;
                        force_full_cleanup = true;
                        terminal_error = Some(format!("playback clock failure: {error}"));
                        break;
                    }
                    if desired_pause.load(Ordering::Acquire) {
                        // Focus restoration is not the final admission when
                        // manual pause is still active. Require a separate
                        // manual-resume preflight for that epoch.
                        verified_target = None;
                    }
                    focus_restore_started_ticks = None;
                    publish_backend_metrics(
                        &backend,
                        &mut local_metrics,
                        metrics,
                        &mut last_published_error,
                    );
                    try_publish_metrics(&local_metrics, metrics, qpc_us_or_terminal!(), true);
                }
            }

            if manual_pause && !clock_state.has_pause_reason("manual") {
                verified_target = None;
                if !clock_state.is_paused() {
                    if let Err(error) = suspend_live_input(
                        &mut backend,
                        &mut coordinator,
                        target_hwnd.load(Ordering::Acquire),
                    ) {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("manual pause suspension failed: {error}"));
                        break;
                    }
                    *abort_counts.entry("manual_pause").or_insert(0) += 1;
                    publish_backend_metrics(
                        &backend,
                        &mut local_metrics,
                        metrics,
                        &mut last_published_error,
                    );
                    try_publish_metrics(&local_metrics, metrics, qpc_us_or_terminal!(), true);
                }
                if let Err(error) = clock_state.enter_pause("manual", now_ticks) {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("playback clock failure: {error}"));
                    break;
                }
            } else if !manual_pause && clock_state.has_pause_reason("manual") {
                if !clock_state.has_pause_reason("focus") {
                    // Manual resume is a new admission boundary. Keep the
                    // authored clock paused while physical cleanup/preflight
                    // runs and sample QPC only after those operations finish.
                    let preflight_target = load_target_stamp(target_hwnd, target_generation);
                    if let Err(error) = ensure_preflight_for_target(
                        &backend,
                        preflight_target,
                        &mut verified_target,
                    ) {
                        verified_target = None;
                        force_full_cleanup = true;
                        terminal_error = Some(format!(
                            "instrument key preflight failed on manual resume; release the 15 instrument keys before playback: {error}"
                        ));
                        break;
                    }
                    if !focus_matches_hwnd(
                        config.require_focus,
                        focus_active,
                        preflight_target.hwnd,
                    ) || !target_stamp_still_current(
                        target_hwnd,
                        target_generation,
                        preflight_target,
                    ) {
                        verified_target = None;
                        continue;
                    }
                    let resumed_ticks = qpc_ticks_or_terminal!();
                    if let Err(error) = clock_state.exit_pause("manual", resumed_ticks) {
                        verified_target = None;
                        force_full_cleanup = true;
                        terminal_error = Some(format!("playback clock failure: {error}"));
                        break;
                    }
                } else {
                    // Keep the manual pause reason until focus restoration
                    // has completed; an old QPC sample must not admit a
                    // partially resumed session.
                    verified_target = None;
                }
            }

            #[cfg(any(test, feature = "test-support"))]
            if clock_state.has_pause_reason("manual") && command_timing.needs_acknowledgment() {
                let acknowledged_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error =
                            Some(format!("QPC pause acknowledgment failed: {error:?}"));
                        break;
                    }
                };
                command_timing.acknowledge_pause(acknowledged_ticks);
            }

            let paused = clock_state.is_paused();
            metrics.is_paused.store(paused, Ordering::Relaxed);
            if paused {
                let pause_target = match now_ticks.checked_add_duration(paused_poll_ticks) {
                    Ok(target) => target,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error =
                            Some(format!("pause deadline arithmetic failure: {error}"));
                        break;
                    }
                };
                let pause_target = match lease_bounded_ticks(
                    pause_target,
                    lease_timeout_ticks,
                    supervisor_heartbeat_ticks,
                ) {
                    Ok(target) => target,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("pause lease deadline failure: {error:?}"));
                        break;
                    }
                };
                if let WaitOutcome::Failed(failure) = waiter
                    .wait_until_ticks_with_metrics_typed(
                        qpc_clock,
                        pause_target,
                        DurationTicks::ZERO,
                        interrupt,
                    )
                    .outcome
                {
                    local_metrics.wait_path_degraded = true;
                    if config.strict_timing || matches!(failure, WaitFailure::Clock) {
                        force_full_cleanup = true;
                        terminal_error = Some(wait_failure_message(failure));
                        break;
                    }
                    std::thread::sleep(Duration::from_micros(500));
                }
                continue;
            }

            if let Some((startup_scheduled_ticks, startup_lead_ticks)) = startup_gate {
                let target_sample_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error =
                            Some(format!("QPC failure before startup wait: {error:?}"));
                        break;
                    }
                };
                let target_qpc = match anchored_dispatch_target_ticks_typed(
                    target_sample_ticks,
                    clock_state.epoch,
                    startup_scheduled_ticks,
                    startup_lead_ticks,
                ) {
                    Ok(target) => target,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("startup deadline failure: {error:?}"));
                        break;
                    }
                };
                if target_sample_ticks < target_qpc {
                    let bounded_target_qpc = match lease_bounded_ticks(
                        target_qpc,
                        lease_timeout_ticks,
                        supervisor_heartbeat_ticks,
                    ) {
                        Ok(target) => target,
                        Err(error) => {
                            force_full_cleanup = true;
                            terminal_error = Some(format!("lease deadline failure: {error:?}"));
                            break;
                        }
                    };
                    let wait_result = waiter.wait_until_ticks_with_metrics_typed(
                        qpc_clock,
                        bounded_target_qpc,
                        effective_spin_threshold_ticks,
                        interrupt,
                    );
                    local_metrics.idle_wake_count = local_metrics.idle_wake_count.saturating_add(1);
                    local_metrics.spin_time_us = local_metrics
                        .spin_time_us
                        .saturating_add(wait_result.spin_us);
                    match wait_result.outcome {
                        WaitOutcome::Interrupted => continue,
                        WaitOutcome::Deadline => continue,
                        WaitOutcome::Failed(failure) => {
                            local_metrics.wait_path_degraded = true;
                            if config.strict_timing || matches!(failure, WaitFailure::Clock) {
                                force_full_cleanup = true;
                                terminal_error = Some(wait_failure_message(failure));
                                break;
                            }
                            std::thread::sleep(Duration::from_micros(500));
                            continue;
                        }
                    }
                }
                startup_gate = None;
                // A first note at authored t=0 may be dispatched before the
                // future physical epoch by its lead. The typed timeline has
                // no negative value, so this one startup sample is defined as
                // logical zero; later underflow remains terminal.
                allow_pre_epoch_startup_dispatch = true;
                now_ticks = qpc_ticks_or_terminal!();
            }

            let effective_now_ticks =
                if allow_pre_epoch_startup_dispatch && now_ticks < clock_state.epoch {
                    TimelineTicks::ZERO
                } else {
                    allow_pre_epoch_startup_dispatch = false;
                    match clock_state
                        .get_elapsed_allow_pre_epoch(now_ticks, allow_pre_epoch_startup_dispatch)
                    {
                        Ok(ticks) => ticks,
                        Err(error) => {
                            force_full_cleanup = true;
                            terminal_error = Some(format!("playback clock failure: {error}"));
                            break;
                        }
                    }
                };
            let effective_now_us = qpc_ticks_to_us_or_terminal!(effective_now_ticks);
            local_metrics.elapsed_us = effective_now_us;
            let latency_class = match classify_latency_class(
                last_send_qpc_ticks,
                now_ticks,
                cold_threshold_ticks,
            ) {
                Ok(class) => class,
                Err(error) => {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("QPC ordering failure: {error}"));
                    break;
                }
            };

            let pending_plan = match coordinator.plan_pending_dispatch_ticks(|polyphony| {
                let (lead_us, saturated) = if config.dispatch_lead_us > 0 {
                    (config.dispatch_lead_us, false)
                } else if config.enable_adaptive_lead {
                    let estimate = estimator.estimate_lead_with_class_and_policy(
                        ActionKind::Up,
                        polyphony,
                        latency_class,
                        config.strict_timing,
                    );
                    (estimate.applied_us, estimate.saturated)
                } else {
                    (0, false)
                };
                qpc_clock
                    .duration_from_us(lead_us)
                    .map(|ticks| (ticks, saturated))
                    .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
            }) {
                Ok(plan) => plan,
                Err(error) => {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("coordinator planning failure: {error}"));
                    break;
                }
            };
            let lead_up_ticks = match pending_plan.as_ref() {
                Some(plan) => plan.lead_ticks,
                None => DurationTicks::ZERO,
            };
            let lead_up = match qpc_clock.duration_to_us(lead_up_ticks) {
                Ok(lead) => lead,
                Err(error) => {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("lead telemetry conversion failure: {error:?}"));
                    break;
                }
            };
            let due_pending = match pending_plan.as_ref() {
                Some(plan) => match coordinator.pop_due_pending_ticks(effective_now_ticks, plan) {
                    Ok(due) => due,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("coordinator pending-pop failure: {error}"));
                        break;
                    }
                },
                None => SmallVec::new(),
            };
            if !due_pending.is_empty() {
                let scan_codes: SmallVec<[u16; 15]> =
                    due_pending.iter().map(|p| p.scan_code).collect();
                let started_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("QPC failure before note-off: {error:?}"));
                        break;
                    }
                };
                let started_us = qpc_ticks_to_us_or_terminal!(started_ticks);
                let actual_ticks = match clock_state
                    .get_elapsed_allow_pre_epoch(started_ticks, allow_pre_epoch_startup_dispatch)
                {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("playback clock failure: {error}"));
                        break;
                    }
                };
                let result = backend.key_up(&scan_codes);
                if let Some(error) = backend.timing_error.take() {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("QPC failure after note-off: {error:?}"));
                    break;
                }
                let completed_qpc_ticks = match result.send_completed_ticks {
                    Some(ticks) => ticks,
                    None => {
                        force_full_cleanup = true;
                        terminal_error = Some(
                            "SendInput note-off completed without a QPC completion boundary"
                                .to_string(),
                        );
                        break;
                    }
                };
                let completed_effective_ticks = match clock_state.get_elapsed_allow_pre_epoch(
                    completed_qpc_ticks,
                    allow_pre_epoch_startup_dispatch,
                ) {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("playback clock failure: {error}"));
                        break;
                    }
                };
                let completed_effective = qpc_ticks_to_us_or_terminal!(completed_effective_ticks);
                last_send_qpc_ticks = Some(completed_qpc_ticks);
                let recovery_required = match coordinator.requeue_failed_releases_ticks(
                    &due_pending,
                    &result.sent,
                    &result.skipped_duplicates,
                    actual_ticks,
                    completed_effective_ticks,
                    &retry_backoff_ticks,
                    result.last_win32_error,
                ) {
                    Ok(required) => required,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("coordinator recovery failure: {error}"));
                        break;
                    }
                };
                if let Err(error) = coordinator.complete_releases(
                    &due_pending,
                    &result.sent,
                    &result.skipped_duplicates,
                ) {
                    force_full_cleanup = true;
                    terminal_error =
                        Some(format!("coordinator release completion failure: {error}"));
                    break;
                }
                // A successful retry closes the recovery pause. The
                // coordinator advances one immutable timeline offset, so
                // overdue work cannot burst immediately after recovery.
                if !recovery_required {
                    match coordinator.finish_release_recovery_ticks(completed_effective_ticks) {
                        Ok(Some(recovery_pause_ticks)) => {
                            let recovery_pause_us =
                                match qpc_clock.duration_to_us(recovery_pause_ticks) {
                                    Ok(value) => value,
                                    Err(error) => {
                                        force_full_cleanup = true;
                                        terminal_error = Some(format!(
                                            "recovery telemetry conversion failure: {error:?}"
                                        ));
                                        break;
                                    }
                                };
                            local_metrics.total_us =
                                local_metrics.total_us.saturating_add(recovery_pause_us);
                        }
                        Ok(None) => {}
                        Err(error) => {
                            force_full_cleanup = true;
                            terminal_error =
                                Some(format!("coordinator recovery completion failure: {error}"));
                            break;
                        }
                    }
                }
                let bookkeeping_completed_us = qpc_us_or_terminal!();
                let mut first_index: Option<usize> = None;
                let mut first_deadline: Option<TimelineTicks> = None;
                for (index, pending) in due_pending.iter().enumerate() {
                    let deadline = match pending.get_effective_release_ticks(lead_up_ticks) {
                        Ok(deadline) => deadline,
                        Err(error) => {
                            force_full_cleanup = true;
                            terminal_error =
                                Some(format!("pending release deadline failure: {error}"));
                            break;
                        }
                    };
                    let is_better = match first_index {
                        None => true,
                        Some(best_index) => match first_deadline {
                            Some(best) => {
                                (deadline, pending.source_action_index, pending.scan_code)
                                    < (
                                        best,
                                        due_pending[best_index].source_action_index,
                                        due_pending[best_index].scan_code,
                                    )
                            }
                            None => {
                                force_full_cleanup = true;
                                terminal_error = Some(
                                    "pending release first-deadline state is inconsistent"
                                        .to_string(),
                                );
                                false
                            }
                        },
                    };
                    if is_better {
                        first_index = Some(index);
                        first_deadline = Some(deadline);
                    }
                }
                if terminal_error.is_some() {
                    break;
                }
                let Some(first_index) = first_index else {
                    force_full_cleanup = true;
                    terminal_error =
                        Some("coordinator returned an empty pending release batch".to_string());
                    break;
                };
                let Some(effective_deadline_ticks) = first_deadline else {
                    force_full_cleanup = true;
                    terminal_error = Some("coordinator returned no release deadline".to_string());
                    break;
                };
                let first = &due_pending[first_index];
                let Some(scheduled_ticks) = due_pending
                    .iter()
                    .map(|pending| pending.scheduled_release_ticks)
                    .min()
                else {
                    force_full_cleanup = true;
                    terminal_error =
                        Some("pending release batch has no scheduled timestamp".to_string());
                    break;
                };
                let scheduled_us = match qpc_clock.timeline_to_us(scheduled_ticks) {
                    Ok(value) => value,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error = Some(format!(
                            "pending release telemetry conversion failure: {error:?}"
                        ));
                        break;
                    }
                };
                let mut deferred_by_us = 0u64;
                for pending in &due_pending {
                    let ready_ticks = pending
                        .release_not_before_ticks
                        .max(pending.next_retry_ticks);
                    let deferred_ticks =
                        match ready_ticks.checked_duration_since(pending.scheduled_release_ticks) {
                            Ok(value) => value,
                            Err(sky_dispatch_core::time::TimeArithmeticError::NegativeOrder) => {
                                DurationTicks::ZERO
                            }
                            Err(error) => {
                                force_full_cleanup = true;
                                terminal_error =
                                    Some(format!("pending deferral arithmetic failure: {error}"));
                                break;
                            }
                        };
                    let deferred_us = match qpc_clock.duration_to_us(deferred_ticks) {
                        Ok(value) => value,
                        Err(error) => {
                            force_full_cleanup = true;
                            terminal_error =
                                Some(format!("pending deferral conversion failure: {error:?}"));
                            break;
                        }
                    };
                    deferred_by_us = deferred_by_us.max(deferred_us);
                }
                if terminal_error.is_some() {
                    break;
                }
                let mixed_source = due_pending.iter().any(|pending| {
                    pending.source_action_index != first.source_action_index
                        || pending.reason_id != first.reason_id
                });
                let up_completion_lateness_ticks = completed_effective_ticks
                    .checked_duration_since(scheduled_ticks)
                    .ok();
                let up_completion_error_ticks = match signed_timeline_delta_ticks(
                    completed_effective_ticks,
                    effective_deadline_ticks,
                ) {
                    Ok(value) => value,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error =
                            Some(format!("note-off timing conversion failure: {error}"));
                        break;
                    }
                };
                let up_authored_completion_error_ticks =
                    match signed_timeline_delta_ticks(completed_effective_ticks, scheduled_ticks) {
                        Ok(value) => value,
                        Err(error) => {
                            force_full_cleanup = true;
                            terminal_error = Some(format!(
                                "note-off authored timing conversion failure: {error}"
                            ));
                            break;
                        }
                    };
                let up_completion_error_us =
                    match signed_ticks_to_us(qpc_clock, up_completion_error_ticks) {
                        Ok(value) => value,
                        Err(error) => {
                            force_full_cleanup = true;
                            terminal_error =
                                Some(format!("note-off timing conversion failure: {error}"));
                            break;
                        }
                    };
                let clean_up_sample = result.success
                    && result.sent.len() == scan_codes.len()
                    && result.skipped_duplicates.is_empty()
                    && result.send_attempts == 1
                    && deferred_by_us == 0
                    && !mixed_source;
                let strict_up_completion_late = config.strict_timing
                    && clean_up_sample
                    && up_completion_lateness_ticks
                        .is_some_and(|late| late > strict_up_completion_late_ticks);
                let up_saturated_positive = pending_plan
                    .as_ref()
                    .is_some_and(|plan| plan.lead_saturated)
                    && up_completion_lateness_ticks.is_some();
                up_saturation_positive_streak = if up_saturated_positive {
                    up_saturation_positive_streak.saturating_add(1)
                } else {
                    0
                };
                let saturation_abort = config.strict_timing
                    && up_saturation_positive_streak >= STRICT_SATURATION_ABORT_STREAK;
                if config.enable_adaptive_lead
                    && let Err(error) = update_estimator_after_send_class(
                        &mut estimator,
                        ActionKind::Up,
                        result.send_completed_us.saturating_sub(started_us),
                        result.sent.len(),
                        scan_codes.len(),
                        lead_up,
                        up_completion_error_us,
                        clean_up_sample,
                        latency_class,
                    )
                {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("estimator update failure: {error}"));
                    break;
                }
                let release_outcome = if strict_up_completion_late {
                    "strict_completion_slo_exceeded"
                } else {
                    release_runtime_outcome(
                        deferred_by_us,
                        result.sent.len(),
                        scan_codes.len(),
                        recovery_required,
                    )
                };
                let mut trace_flags = 0;
                if result.sent.len() == scan_codes.len() {
                    trace_flags |= TRACE_FLAG_SENT_FULL;
                }
                if release_outcome == "deferred_release" || release_outcome == "failed_note_off" {
                    trace_flags |= TRACE_FLAG_RECOVERY;
                }
                if deferred_by_us > 0 {
                    trace_flags |= TRACE_FLAG_DEFERRED;
                }
                if release_outcome != "sent" {
                    trace_flags |= TRACE_FLAG_ANOMALY;
                }
                if let Err(error) = telemetry.try_push(|| {
                    RtTraceRecord::dispatched(
                        TraceContext {
                            event_index: first.source_action_index,
                            kind: TRACE_KIND_UP,
                            outcome: trace_outcome_code(release_outcome),
                            polyphony: scan_codes.len(),
                            flags: trace_flags,
                            win32_error: result.last_win32_error.unwrap_or(0),
                        },
                        TraceTiming {
                            authored_ticks: scheduled_ticks,
                            effective_deadline_ticks,
                            wake_ticks: actual_ticks,
                            send_started_ticks: result.send_started_ticks,
                            send_completed_ticks: result.send_completed_ticks,
                            completion_error_ticks: up_completion_error_ticks,
                            authored_completion_error_ticks: up_authored_completion_error_ticks,
                            applied_lead_ticks: lead_up_ticks,
                        },
                        TraceDelivery {
                            requested: scan_codes.len(),
                            sent: result.sent.len(),
                            skipped: result.skipped_duplicates.len(),
                            send_attempts: usize::from(result.send_attempts),
                        },
                    )
                }) {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("native telemetry record overflow: {error}"));
                    break;
                }
                if config.enable_adaptive_lead
                    && pending_plan
                        .as_ref()
                        .is_some_and(|plan| plan.lead_saturated)
                {
                    record_lead_saturation(
                        &mut local_metrics.lead_saturation_count_up,
                        &mut local_metrics.positive_residual_at_cap,
                        scan_codes.len(),
                        signed_delta(completed_effective, scheduled_us),
                    );
                }
                pending_pre_send_spin_us = 0;
                record_input_path_health(
                    bookkeeping_completed_us.saturating_sub(started_us),
                    completed_effective,
                    config.input_path_warn_us,
                    &mut send_duration_window,
                    &mut send_over_warn_count,
                    &mut input_path_warn_started_us,
                    &mut local_metrics.input_path_degraded,
                );
                record_input_path_health(
                    result.send_completed_us.saturating_sub(started_us),
                    completed_effective,
                    config.input_path_warn_us,
                    &mut send_pure_window,
                    &mut send_pure_over_warn_count,
                    &mut send_pure_warn_started_us,
                    &mut local_metrics.sendinput_path_degraded,
                );
                record_input_path_health(
                    bookkeeping_completed_us.saturating_sub(result.send_completed_us),
                    completed_effective,
                    config.input_path_warn_us,
                    &mut bookkeeping_window,
                    &mut bookkeeping_over_warn_count,
                    &mut bookkeeping_warn_started_us,
                    &mut local_metrics.bookkeeping_degraded,
                );
                let deferred_release = deferred_by_us > 0;
                record_lateness(
                    signed_delta(completed_effective, scheduled_us),
                    true,
                    deferred_release,
                    &mut local_metrics,
                );
                publish_backend_metrics(
                    &backend,
                    &mut local_metrics,
                    metrics,
                    &mut last_published_error,
                );
                try_publish_metrics(
                    &local_metrics,
                    metrics,
                    qpc_us_or_terminal!(),
                    !clean_up_sample || recovery_required,
                );
                if recovery_required {
                    verified_target = None;
                    force_full_cleanup = true;
                    terminal_error = Some(format!(
                        "note-off recovery exhausted after {} retries{}",
                        sky_dispatch_core::coordinator::MAX_RELEASE_RETRIES,
                        result
                            .last_win32_error
                            .map_or(String::new(), |error| format!(" (Win32 error {error})"))
                    ));
                    let recovery_cleanup =
                        backend.release_all_full_instrument(target_hwnd.load(Ordering::Acquire));
                    if !release_state_verified(&backend, &recovery_cleanup) {
                        record_termination_error(
                            &mut terminal_error,
                            &mut secondary_errors,
                            format!(
                                "recovery cleanup release verification failed: {}",
                                describe_release_outcome(&recovery_cleanup)
                            ),
                        );
                    }
                    cancel_coordinator_or_terminal(
                        &mut coordinator,
                        &mut force_full_cleanup,
                        &mut terminal_error,
                        &mut secondary_errors,
                    );
                    break;
                }
                if strict_up_completion_late {
                    force_full_cleanup = true;
                    terminal_error = Some(format!(
                        "strict timing completion SLO exceeded for note-off at action {}: completion was {}us late",
                        first.source_action_index, up_completion_error_us
                    ));
                    break;
                }
                if saturation_abort {
                    force_full_cleanup = true;
                    terminal_error = Some(format!(
                        "strict timing SLO exceeded: note-off lead saturated with positive residual for {} consecutive dispatches",
                        STRICT_SATURATION_ABORT_STREAK
                    ));
                    break;
                }
                continue;
            }

            let next_down_polyphony = coordinator.next_authored_polyphony();
            let (lead_down, lead_down_saturated) = if config.dispatch_lead_us > 0 {
                (config.dispatch_lead_us, false)
            } else if config.enable_adaptive_lead {
                let estimate = estimator.estimate_lead_with_class_and_policy(
                    ActionKind::Down,
                    next_down_polyphony,
                    latency_class,
                    config.strict_timing,
                );
                (estimate.applied_us, estimate.saturated)
            } else {
                (0, false)
            };
            let lead_down_ticks = match qpc_clock.duration_from_us(lead_down) {
                Ok(ticks) => ticks,
                Err(error) => {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("down lead conversion failure: {error:?}"));
                    break;
                }
            };
            let prepared_batch = match coordinator
                .prepare_next_due_authored(effective_now_ticks, lead_down_ticks)
            {
                Ok(value) => value,
                Err(error) => {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("coordinator authored-prepare failure: {error}"));
                    break;
                }
            };
            if let Some(prepared_batch) = prepared_batch {
                let batch_index = prepared_batch.index;
                // --- Borrow scope: extract all scalar and stack data before any &mut call ---
                // `batch_view` borrows from `coordinator.schedule`. We must not call any
                // `&mut coordinator` method until this scope ends. Pull every field we need
                // into Copy / stack-owned values here.
                let batch_scheduled_ticks = prepared_batch.effective_scheduled_ticks;
                let batch_view = match coordinator
                    .schedule
                    .view_batch_ticks(batch_index, batch_scheduled_ticks)
                {
                    Ok(value) => value,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("runtime schedule view failure: {error}"));
                        break;
                    }
                };
                let batch_kind = batch_view.kind();
                let batch_source_action_index = batch_view.source_action_index();
                let batch_scheduled_us = match qpc_clock.timeline_to_us(batch_scheduled_ticks) {
                    Ok(value) => value,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error =
                            Some(format!("schedule telemetry conversion failure: {error:?}"));
                        break;
                    }
                };
                let authored_batch_scheduled_ticks = coordinator.batch_scheduled_ticks[batch_index];
                let batch_intent_count = batch_view.intents.len();
                // Conflict check: O(N) bitwise, no allocation.
                let conflict_mask = coordinator.check_down_conflicts_compact(batch_view.intents);
                let has_conflicts = conflict_mask != 0;
                // Scan codes for SendInput: stack-only buffer.
                let scan_batch = batch_view.scan_code_batch_excluding_mask(conflict_mask);
                // --- End of borrow scope: all data is now in stack-local copies ---

                let mut force_dispatch_publish = false;
                if batch_kind == ActionKind::Down {
                    // Repeat the foreground comparison at the final boundary
                    // immediately before SendInput. If focus changed after
                    // the outer-loop sample, terminalize this authored batch;
                    // it must not be replayed after the focus grace period.
                    if !focus_matches(config.require_focus, focus_active, target_hwnd) {
                        // The batch was only prepared. Leave the cursor and generation
                        // ledger untouched so the same authored chord can be prepared again
                        // after focus restoration.
                        if let Err(error) = suspend_live_input(
                            &mut backend,
                            &mut coordinator,
                            target_hwnd.load(Ordering::Acquire),
                        ) {
                            force_full_cleanup = true;
                            terminal_error = Some(format!("focus suspension failed: {error}"));
                            break;
                        }
                        if let Err(error) = clock_state.enter_pause("focus", now_ticks) {
                            force_full_cleanup = true;
                            terminal_error = Some(format!("playback clock failure: {error}"));
                            break;
                        }
                        focus_restore_started_ticks = None;
                        if let Err(error) = telemetry.try_push(|| {
                            RtTraceRecord::dispatched(
                                TraceContext {
                                    event_index: batch_source_action_index,
                                    kind: TRACE_KIND_DOWN,
                                    outcome: trace_outcome_code("blocked_unfocused"),
                                    polyphony: batch_intent_count,
                                    flags: TRACE_FLAG_ANOMALY,
                                    win32_error: 0,
                                },
                                TraceTiming {
                                    authored_ticks: authored_batch_scheduled_ticks,
                                    effective_deadline_ticks: batch_scheduled_ticks,
                                    wake_ticks: effective_now_ticks,
                                    send_started_ticks: None,
                                    send_completed_ticks: None,
                                    completion_error_ticks: 0,
                                    authored_completion_error_ticks: 0,
                                    applied_lead_ticks: lead_down_ticks,
                                },
                                TraceDelivery {
                                    requested: 0,
                                    sent: 0,
                                    skipped: 0,
                                    send_attempts: 0,
                                },
                            )
                        }) {
                            force_full_cleanup = true;
                            terminal_error =
                                Some(format!("native telemetry record overflow: {error}"));
                            break;
                        }
                        publish_backend_metrics(
                            &backend,
                            &mut local_metrics,
                            metrics,
                            &mut last_published_error,
                        );
                        try_publish_metrics(&local_metrics, metrics, qpc_us_or_terminal!(), true);
                        continue;
                    }
                    if focus_loss_fault && !focus_loss_fault_injected {
                        focus_loss_fault_injected = true;
                        force_full_cleanup = true;
                        terminal_error = Some(
                            "focus lost after due check before SendInput boundary".to_string(),
                        );
                        break;
                    }
                    let preflight_target = load_target_stamp(target_hwnd, target_generation);
                    if let Err(error) = ensure_preflight_for_target(
                        &backend,
                        preflight_target,
                        &mut verified_target,
                    ) {
                        verified_target = None;
                        force_full_cleanup = true;
                        terminal_error = Some(format!(
                            "instrument key preflight failed; release the 15 instrument keys before playback: {error}"
                        ));
                        break;
                    }
                    if !target_stamp_still_current(target_hwnd, target_generation, preflight_target)
                    {
                        verified_target = None;
                        continue;
                    }
                    if effective_now_ticks
                        .checked_duration_since(batch_scheduled_ticks)
                        .is_ok_and(|late| late > hard_late_abort_threshold_ticks)
                    {
                        force_full_cleanup = true;
                        terminal_error = Some(format!(
                            "authored Down exceeded hard lateness safety threshold of {}us",
                            HARD_LATE_ABORT_THRESHOLD_US
                        ));
                        break;
                    }
                    // A runtime conflict means coordinator/backend state no
                    // longer matches the compiled schedule. It is an
                    // invariant failure, never a user-selectable drop mode.
                    if has_conflicts {
                        local_metrics.authored_conflict_events =
                            local_metrics.authored_conflict_events.saturating_add(1);
                        local_metrics.authored_chords_rejected =
                            local_metrics.authored_chords_rejected.saturating_add(1);
                        local_metrics.authored_keys_rejected = local_metrics
                            .authored_keys_rejected
                            .saturating_add(batch_intent_count as u64);
                        force_full_cleanup = true;
                        terminal_error = Some(format!(
                            "unexpected blocked authored Down at action {}",
                            batch_source_action_index
                        ));
                        break;
                    }

                    if !scan_batch.is_empty() {
                        // Preflight can perform multiple Win32 calls. Keep
                        // the final admission bound to the exact stamp that
                        // was verified and let command races return to the
                        // worker control path without becoming send failures.
                        match final_down_admission(
                            preflight_target,
                            config.require_focus,
                            focus_active,
                            target_hwnd,
                            target_generation,
                            quit_requested,
                            skip_requested,
                            panic_requested,
                            desired_pause,
                        ) {
                            DownAdmission::Allowed => {}
                            DownAdmission::FocusLost => {
                                verified_target = None;
                                let focus_ticks = qpc_ticks_or_terminal!();
                                if let Err(error) = suspend_live_input(
                                    &mut backend,
                                    &mut coordinator,
                                    target_hwnd.load(Ordering::Acquire),
                                ) {
                                    force_full_cleanup = true;
                                    terminal_error =
                                        Some(format!("focus suspension failed: {error}"));
                                    break;
                                }
                                if let Err(error) = clock_state.enter_pause("focus", focus_ticks) {
                                    force_full_cleanup = true;
                                    terminal_error = Some(format!(
                                        "playback clock failure after final focus check: {error}"
                                    ));
                                    break;
                                }
                                focus_restore_started_ticks = None;
                                publish_backend_metrics(
                                    &backend,
                                    &mut local_metrics,
                                    metrics,
                                    &mut last_published_error,
                                );
                                try_publish_metrics(
                                    &local_metrics,
                                    metrics,
                                    qpc_us_or_terminal!(),
                                    true,
                                );
                                continue;
                            }
                            DownAdmission::TargetChanged
                            | DownAdmission::PauseRequested
                            | DownAdmission::QuitRequested
                            | DownAdmission::SkipRequested
                            | DownAdmission::PanicRequested => {
                                verified_target = None;
                                continue;
                            }
                        }
                        // SendInput uses the stack-only scan code buffer — no allocation.
                        let result = backend.key_down(scan_batch.as_slice());
                        if let Some(error) = backend.timing_error.take() {
                            force_full_cleanup = true;
                            terminal_error = Some(format!("QPC failure after note-on: {error:?}"));
                            break;
                        }

                        let (
                            result_started_ticks,
                            result_completed_us,
                            result_completed_ticks,
                            result_sent,
                            result_skipped_duplicates,
                            result_send_attempts,
                            _result_zero_progress_retries,
                            result_retried_after_zero_progress,
                            result_chord_integrity_lost,
                            _result_first_win32_error,
                            result_last_win32_error,
                            result_success,
                        ) = match result {
                            sky_dispatch_win32::input::DownSendOutcome::Complete {
                                started_ticks,
                                completed_us,
                                completed_ticks,
                                sent,
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                retried_after_zero_progress,
                                ..
                            } => (
                                started_ticks,
                                completed_us,
                                completed_ticks,
                                sent,
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                retried_after_zero_progress,
                                false,
                                None,
                                None,
                                true,
                            ),
                            sky_dispatch_win32::input::DownSendOutcome::ZeroProgress {
                                started_ticks,
                                completed_us,
                                completed_ticks,
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                first_error,
                                last_error,
                                ..
                            } => (
                                started_ticks,
                                completed_us,
                                completed_ticks,
                                smallvec::SmallVec::<[u16; 15]>::new(),
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                zero_progress_retries > 0,
                                false,
                                first_error,
                                last_error,
                                false,
                            ),
                            sky_dispatch_win32::input::DownSendOutcome::IntegrityLost {
                                started_ticks,
                                completed_us,
                                completed_ticks,
                                sent,
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                first_error,
                                last_error,
                                ..
                            } => (
                                started_ticks,
                                completed_us,
                                completed_ticks,
                                sent,
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                zero_progress_retries > 0,
                                true,
                                first_error,
                                last_error,
                                false,
                            ),
                        };

                        if !result_success {
                            force_full_cleanup = true;
                            terminal_error = Some(format!(
                                "authored Down send integrity failure at action {}",
                                batch_source_action_index
                            ));
                            break;
                        }

                        // The sender owns the exact syscall boundary. The
                        // admission QPC sample was intentionally removed so
                        // coordinator activation and sender-duration metrics
                        // cannot use a timestamp taken before final checks.
                        let sender_started_ticks = match result_started_ticks {
                            Some(ticks) => ticks,
                            None => {
                                force_full_cleanup = true;
                                terminal_error = Some(
                                    "SendInput note-on succeeded without a QPC start boundary"
                                        .to_string(),
                                );
                                break;
                            }
                        };
                        let completed_qpc_ticks = match result_completed_ticks {
                            Some(ticks) => ticks,
                            None => {
                                force_full_cleanup = true;
                                terminal_error = Some(
                                    "SendInput note-on completed without a QPC completion boundary"
                                        .to_string(),
                                );
                                break;
                            }
                        };
                        let sender_duration_ticks = match completed_qpc_ticks
                            .checked_duration_since(sender_started_ticks)
                        {
                            Ok(duration) => duration,
                            Err(error) => {
                                force_full_cleanup = true;
                                terminal_error =
                                    Some(format!("note-on QPC ordering failure: {error}"));
                                break;
                            }
                        };
                        let sender_duration_us =
                            match qpc_clock.duration_to_us(sender_duration_ticks) {
                                Ok(duration) => duration,
                                Err(error) => {
                                    force_full_cleanup = true;
                                    terminal_error = Some(format!(
                                        "note-on sender duration conversion failure: {error:?}"
                                    ));
                                    break;
                                }
                            };
                        let sender_started_effective_ticks = match clock_state
                            .get_elapsed_allow_pre_epoch(
                                sender_started_ticks,
                                allow_pre_epoch_startup_dispatch,
                            ) {
                            Ok(ticks) => ticks,
                            Err(error) => {
                                force_full_cleanup = true;
                                terminal_error = Some(format!("playback clock failure: {error}"));
                                break;
                            }
                        };
                        let completed_effective_ticks = match clock_state
                            .get_elapsed_allow_pre_epoch(
                                completed_qpc_ticks,
                                allow_pre_epoch_startup_dispatch,
                            ) {
                            Ok(ticks) => ticks,
                            Err(error) => {
                                force_full_cleanup = true;
                                terminal_error = Some(format!("playback clock failure: {error}"));
                                break;
                            }
                        };
                        let completed_effective =
                            qpc_ticks_to_us_or_terminal!(completed_effective_ticks);
                        last_send_qpc_ticks = Some(completed_qpc_ticks);
                        if let Err(error) = coordinator.commit_down_success(
                            prepared_batch,
                            &result_sent,
                            sender_started_effective_ticks,
                            completed_effective_ticks,
                        ) {
                            force_full_cleanup = true;
                            terminal_error =
                                Some(format!("coordinator activation failure: {error}"));
                            break;
                        }
                        let completion_lateness_ticks = completed_effective_ticks
                            .checked_duration_since(batch_scheduled_ticks)
                            .ok();
                        let completion_error_ticks_value = match signed_timeline_delta_ticks(
                            completed_effective_ticks,
                            batch_scheduled_ticks,
                        ) {
                            Ok(value) => value,
                            Err(error) => {
                                force_full_cleanup = true;
                                terminal_error =
                                    Some(format!("note-on timing conversion failure: {error}"));
                                break;
                            }
                        };
                        let authored_completion_error_ticks_value =
                            match signed_timeline_delta_ticks(
                                completed_effective_ticks,
                                authored_batch_scheduled_ticks,
                            ) {
                                Ok(value) => value,
                                Err(error) => {
                                    force_full_cleanup = true;
                                    terminal_error = Some(format!(
                                        "note-on authored timing conversion failure: {error}"
                                    ));
                                    break;
                                }
                            };
                        let completion_error_us =
                            match signed_ticks_to_us(qpc_clock, completion_error_ticks_value) {
                                Ok(value) => value,
                                Err(error) => {
                                    force_full_cleanup = true;
                                    terminal_error =
                                        Some(format!("note-on timing conversion failure: {error}"));
                                    break;
                                }
                            };
                        let clean_down_sample = result_success
                            && result_sent.len() == batch_intent_count
                            && result_skipped_duplicates.is_empty()
                            && result_send_attempts == 1
                            && !result_chord_integrity_lost;
                        let recovered_retry_late = result_retried_after_zero_progress
                            && completion_lateness_ticks
                                .is_some_and(|late| late > retry_late_threshold_ticks);
                        let retry_late_abort = config.strict_timing && recovered_retry_late;
                        let strict_down_completion_late = config.strict_timing
                            && clean_down_sample
                            && completion_lateness_ticks
                                .is_some_and(|late| late > strict_down_completion_late_ticks);
                        if recovered_retry_late {
                            local_metrics.recovered_zero_progress_but_late = local_metrics
                                .recovered_zero_progress_but_late
                                .saturating_add(1);
                        }
                        down_saturation_positive_streak =
                            if lead_down_saturated && completion_lateness_ticks.is_some() {
                                down_saturation_positive_streak.saturating_add(1)
                            } else {
                                0
                            };
                        let saturation_abort = config.strict_timing
                            && down_saturation_positive_streak >= STRICT_SATURATION_ABORT_STREAK;
                        if config.enable_adaptive_lead
                            && let Err(error) = update_estimator_after_send_class(
                                &mut estimator,
                                ActionKind::Down,
                                sender_duration_us,
                                result_sent.len(),
                                batch_intent_count,
                                lead_down,
                                completion_error_us,
                                clean_down_sample,
                                latency_class,
                            )
                        {
                            force_full_cleanup = true;
                            terminal_error = Some(format!("estimator update failure: {error}"));
                            break;
                        }
                        let bookkeeping_completed_us = qpc_us_or_terminal!();
                        let down_outcome = if recovered_retry_late {
                            "recovered_zero_progress_but_late"
                        } else if strict_down_completion_late {
                            "strict_completion_slo_exceeded"
                        } else if result_chord_integrity_lost {
                            "chord_integrity_lost"
                        } else if result_sent.len() == scan_batch.len() {
                            "sent"
                        } else {
                            "partial_note_on"
                        };
                        force_dispatch_publish = !result_success
                            || result_retried_after_zero_progress
                            || result_chord_integrity_lost;
                        let mut trace_flags = 0;
                        if result_sent.len() == scan_batch.len() {
                            trace_flags |= TRACE_FLAG_SENT_FULL;
                        }
                        if recovered_retry_late || result_chord_integrity_lost {
                            trace_flags |= TRACE_FLAG_RECOVERY;
                        }
                        if down_outcome != "sent" {
                            trace_flags |= TRACE_FLAG_ANOMALY;
                        }
                        if let Err(error) = telemetry.try_push(|| {
                            RtTraceRecord::dispatched(
                                TraceContext {
                                    event_index: batch_source_action_index,
                                    kind: TRACE_KIND_DOWN,
                                    outcome: trace_outcome_code(down_outcome),
                                    polyphony: batch_intent_count,
                                    flags: trace_flags,
                                    win32_error: result_last_win32_error.unwrap_or(0),
                                },
                                TraceTiming {
                                    authored_ticks: authored_batch_scheduled_ticks,
                                    effective_deadline_ticks: batch_scheduled_ticks,
                                    wake_ticks: effective_now_ticks,
                                    send_started_ticks: Some(sender_started_ticks),
                                    send_completed_ticks: result_completed_ticks,
                                    completion_error_ticks: completion_error_ticks_value,
                                    authored_completion_error_ticks:
                                        authored_completion_error_ticks_value,
                                    applied_lead_ticks: lead_down_ticks,
                                },
                                TraceDelivery {
                                    requested: batch_intent_count,
                                    sent: result_sent.len(),
                                    skipped: result_skipped_duplicates.len(),
                                    send_attempts: usize::from(result_send_attempts),
                                },
                            )
                        }) {
                            force_full_cleanup = true;
                            terminal_error =
                                Some(format!("native telemetry record overflow: {error}"));
                            break;
                        }
                        if config.enable_adaptive_lead && lead_down_saturated {
                            record_lead_saturation(
                                &mut local_metrics.lead_saturation_count_down,
                                &mut local_metrics.positive_residual_at_cap,
                                batch_intent_count,
                                signed_delta(completed_effective, batch_scheduled_us),
                            );
                        }
                        pending_pre_send_spin_us = 0;
                        let bookkeeping_after_send_us =
                            bookkeeping_completed_us.saturating_sub(result_completed_us);
                        record_input_path_health(
                            sender_duration_us.saturating_add(bookkeeping_after_send_us),
                            completed_effective,
                            config.input_path_warn_us,
                            &mut send_duration_window,
                            &mut send_over_warn_count,
                            &mut input_path_warn_started_us,
                            &mut local_metrics.input_path_degraded,
                        );
                        record_input_path_health(
                            sender_duration_us,
                            completed_effective,
                            config.input_path_warn_us,
                            &mut send_pure_window,
                            &mut send_pure_over_warn_count,
                            &mut send_pure_warn_started_us,
                            &mut local_metrics.sendinput_path_degraded,
                        );
                        record_input_path_health(
                            bookkeeping_after_send_us,
                            completed_effective,
                            config.input_path_warn_us,
                            &mut bookkeeping_window,
                            &mut bookkeeping_over_warn_count,
                            &mut bookkeeping_warn_started_us,
                            &mut local_metrics.bookkeeping_degraded,
                        );
                        record_lateness(
                            signed_delta(completed_effective, batch_scheduled_us),
                            false,
                            false,
                            &mut local_metrics,
                        );
                        if result_chord_integrity_lost {
                            verified_target = None;
                            force_full_cleanup = true;
                            terminal_error = Some(format!(
                                "SendInput split authored chord at action {}",
                                batch_source_action_index
                            ));
                            publish_backend_metrics(
                                &backend,
                                &mut local_metrics,
                                metrics,
                                &mut last_published_error,
                            );
                            try_publish_metrics(
                                &local_metrics,
                                metrics,
                                qpc_us_or_terminal!(),
                                true,
                            );
                            break;
                        }
                        if retry_late_abort {
                            force_full_cleanup = true;
                            terminal_error = Some(format!(
                                "strict timing rejected zero-progress retry at action {}: completion was {}us late",
                                batch_source_action_index, completion_error_us
                            ));
                            publish_backend_metrics(
                                &backend,
                                &mut local_metrics,
                                metrics,
                                &mut last_published_error,
                            );
                            try_publish_metrics(
                                &local_metrics,
                                metrics,
                                qpc_us_or_terminal!(),
                                true,
                            );
                            break;
                        }
                        if strict_down_completion_late {
                            force_full_cleanup = true;
                            terminal_error = Some(format!(
                                "strict timing completion SLO exceeded for note-on at action {}: completion was {}us late",
                                batch_source_action_index, completion_error_us
                            ));
                            publish_backend_metrics(
                                &backend,
                                &mut local_metrics,
                                metrics,
                                &mut last_published_error,
                            );
                            try_publish_metrics(
                                &local_metrics,
                                metrics,
                                qpc_us_or_terminal!(),
                                true,
                            );
                            break;
                        }
                        if saturation_abort {
                            force_full_cleanup = true;
                            terminal_error = Some(format!(
                                "strict timing SLO exceeded: note-on lead saturated with positive residual for {} consecutive dispatches",
                                STRICT_SATURATION_ABORT_STREAK
                            ));
                            publish_backend_metrics(
                                &backend,
                                &mut local_metrics,
                                metrics,
                                &mut last_published_error,
                            );
                            try_publish_metrics(
                                &local_metrics,
                                metrics,
                                qpc_us_or_terminal!(),
                                true,
                            );
                            break;
                        }
                    }
                } else {
                    let (_, suppressed) = match coordinator.commit_up_request(prepared_batch) {
                        Ok(value) => value,
                        Err(error) => {
                            force_full_cleanup = true;
                            terminal_error =
                                Some(format!("coordinator release request failure: {error}"));
                            break;
                        }
                    };
                    if !suppressed.is_empty() {
                        force_dispatch_publish = true;
                        if let Err(error) = telemetry.try_push(|| {
                            RtTraceRecord::dispatched(
                                TraceContext {
                                    event_index: batch_source_action_index,
                                    kind: TRACE_KIND_UP,
                                    outcome: trace_outcome_code("suppressed_stale_up"),
                                    polyphony: suppressed.len(),
                                    flags: TRACE_FLAG_ANOMALY,
                                    win32_error: 0,
                                },
                                TraceTiming {
                                    authored_ticks: authored_batch_scheduled_ticks,
                                    effective_deadline_ticks: batch_scheduled_ticks,
                                    wake_ticks: effective_now_ticks,
                                    send_started_ticks: None,
                                    send_completed_ticks: None,
                                    completion_error_ticks: 0,
                                    authored_completion_error_ticks: 0,
                                    applied_lead_ticks: lead_up_ticks,
                                },
                                TraceDelivery {
                                    requested: 0,
                                    sent: 0,
                                    skipped: 0,
                                    send_attempts: 0,
                                },
                            )
                        }) {
                            force_full_cleanup = true;
                            terminal_error =
                                Some(format!("native telemetry record overflow: {error}"));
                            break;
                        }
                    }
                }
                publish_backend_metrics(
                    &backend,
                    &mut local_metrics,
                    metrics,
                    &mut last_published_error,
                );
                try_publish_metrics(
                    &local_metrics,
                    metrics,
                    qpc_us_or_terminal!(),
                    force_dispatch_publish,
                );
                continue;
            }

            let next_down_polyphony = coordinator.next_authored_polyphony();
            let lead_down = if config.dispatch_lead_us > 0 {
                config.dispatch_lead_us
            } else if config.enable_adaptive_lead {
                estimator
                    .estimate_lead_with_class_and_policy(
                        ActionKind::Down,
                        next_down_polyphony,
                        latency_class,
                        config.strict_timing,
                    )
                    .applied_us
            } else {
                0
            };
            let lead_down_ticks = match qpc_clock.duration_from_us(lead_down) {
                Ok(ticks) => ticks,
                Err(error) => {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("down lead conversion failure: {error:?}"));
                    break;
                }
            };
            let pending_plan = match coordinator.plan_pending_dispatch_ticks(|polyphony| {
                let (lead_us, saturated) = if config.dispatch_lead_us > 0 {
                    (config.dispatch_lead_us, false)
                } else if config.enable_adaptive_lead {
                    let estimate = estimator.estimate_lead_with_class_and_policy(
                        ActionKind::Up,
                        polyphony,
                        latency_class,
                        config.strict_timing,
                    );
                    (estimate.applied_us, estimate.saturated)
                } else {
                    (0, false)
                };
                qpc_clock
                    .duration_from_us(lead_us)
                    .map(|ticks| (ticks, saturated))
                    .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
            }) {
                Ok(plan) => plan,
                Err(error) => {
                    force_full_cleanup = true;
                    terminal_error = Some(format!("coordinator planning failure: {error}"));
                    break;
                }
            };
            let deadline_ticks =
                match coordinator.next_deadline_ticks(lead_down_ticks, pending_plan.as_ref()) {
                    Ok(deadline) => deadline,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("coordinator deadline failure: {error}"));
                        break;
                    }
                };
            if let Some(deadline_ticks) = deadline_ticks {
                // Take the QPC tick and its logical elapsed-time sample from
                // the same instant.  Reusing the older outer-loop elapsed
                // sample after doing bookkeeping shifts the absolute target
                // late by the whole A->B overhead interval.
                let target_sample_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error =
                            Some(format!("QPC failure before dispatch wait: {error:?}"));
                        break;
                    }
                };
                let target_sample_elapsed_ticks = match clock_state.get_elapsed_allow_pre_epoch(
                    target_sample_ticks,
                    allow_pre_epoch_startup_dispatch,
                ) {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("playback clock failure: {error}"));
                        break;
                    }
                };
                if deadline_ticks > target_sample_elapsed_ticks {
                    let target_qpc = match clock_state
                        .epoch
                        .checked_add_duration(DurationTicks::from_raw(deadline_ticks.as_u64()))
                    {
                        Ok(target) => target,
                        Err(error) => {
                            force_full_cleanup = true;
                            terminal_error = Some(format!("deadline arithmetic failure: {error}"));
                            break;
                        }
                    };
                    let cold_warmup_ticks = match last_send_qpc_ticks {
                        None => core_warmup_ticks,
                        Some(last_send_ticks) => {
                            let gap = match target_sample_ticks
                                .checked_duration_since(last_send_ticks)
                            {
                                Ok(gap) => gap,
                                Err(error) => {
                                    force_full_cleanup = true;
                                    terminal_error =
                                        Some(format!("cold classification clock failure: {error}"));
                                    break;
                                }
                            };
                            if gap > cold_threshold_ticks {
                                core_warmup_ticks
                            } else {
                                DurationTicks::ZERO
                            }
                        }
                    };
                    let wait_spin_threshold_ticks =
                        match effective_spin_threshold_ticks.checked_add(cold_warmup_ticks) {
                            Ok(threshold) => threshold,
                            Err(error) => {
                                force_full_cleanup = true;
                                terminal_error =
                                    Some(format!("spin threshold arithmetic failure: {error}"));
                                break;
                            }
                        };
                    let wait_result = waiter.wait_until_ticks_with_metrics_typed(
                        qpc_clock,
                        match lease_bounded_ticks(
                            target_qpc,
                            lease_timeout_ticks,
                            supervisor_heartbeat_ticks,
                        ) {
                            Ok(target) => target,
                            Err(error) => {
                                force_full_cleanup = true;
                                terminal_error = Some(format!("lease deadline failure: {error:?}"));
                                break;
                            }
                        },
                        wait_spin_threshold_ticks,
                        interrupt,
                    );
                    local_metrics.idle_wake_count = local_metrics.idle_wake_count.saturating_add(1);
                    local_metrics.spin_time_us = local_metrics
                        .spin_time_us
                        .saturating_add(wait_result.spin_us);
                    pending_pre_send_spin_us = wait_result.spin_us;
                    let wake_qpc_ticks = match qpc_clock.now() {
                        Ok(ticks) => ticks,
                        Err(error) => {
                            force_full_cleanup = true;
                            terminal_error = Some(format!("QPC runtime failure: {error:?}"));
                            break;
                        }
                    };
                    let wake_elapsed_ticks = match clock_state.get_elapsed_allow_pre_epoch(
                        wake_qpc_ticks,
                        allow_pre_epoch_startup_dispatch,
                    ) {
                        Ok(ticks) => ticks,
                        Err(error) => {
                            force_full_cleanup = true;
                            terminal_error = Some(format!("playback clock failure: {error}"));
                            break;
                        }
                    };
                    let wake_error_ticks =
                        match wake_lateness_ticks(wake_elapsed_ticks, deadline_ticks) {
                            Ok(ticks) => ticks,
                            Err(error) => {
                                force_full_cleanup = true;
                                terminal_error =
                                    Some(format!("wait target arithmetic failure: {error}"));
                                break;
                            }
                        };
                    let wake_error_us = qpc_ticks_to_us_or_terminal!(wake_error_ticks);
                    match wait_result.outcome {
                        WaitOutcome::Deadline => {
                            local_metrics.wait_target_error_us =
                                local_metrics.wait_target_error_us.max(wake_error_us);
                        }
                        WaitOutcome::Failed(failure) => {
                            local_metrics.wait_path_degraded = true;
                            if config.strict_timing || matches!(failure, WaitFailure::Clock) {
                                force_full_cleanup = true;
                                terminal_error = Some(wait_failure_message(failure));
                                break;
                            }
                            std::thread::sleep(Duration::from_micros(500));
                            pending_pre_send_spin_us = 0;
                            continue;
                        }
                        WaitOutcome::Interrupted => {}
                    }
                    if config.input_path_warn_us > 0 && wake_error_us > config.input_path_warn_us {
                        local_metrics.wait_path_degraded = true;
                    }
                    if wait_result.outcome == WaitOutcome::Interrupted {
                        pending_pre_send_spin_us = 0;
                        continue;
                    }
                }
            } else {
                break;
            }
        }
    }));

    // Validate before either cleanup operation can erase the evidence of a
    // coordinator mismatch. The first failure remains primary; later cleanup
    // and accounting failures are retained as secondary diagnostics.
    if let Err(error) = coordinator.check_invariants() {
        force_full_cleanup = true;
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            format!("coordinator pre-cleanup invariant failure: {error}"),
        );
    }

    if worker_result.is_err() {
        force_full_cleanup = true;
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            "worker panicked before terminal cleanup".to_string(),
        );
    }

    // This cleanup sits outside the contained loop so it also runs when an
    // unexpected panic crosses the orchestration/backend seam.
    let cleanup_result = catch_unwind(AssertUnwindSafe(|| {
        let outcome = if worker_result.is_err() || force_full_cleanup {
            backend.release_all_full_instrument(target_hwnd.load(Ordering::Acquire))
        } else {
            backend.release_all(target_hwnd.load(Ordering::Acquire))
        };
        if release_state_verified(&backend, &outcome) {
            outcome
        } else {
            // A normal-path release that cannot be verified gets one bounded
            // full-instrument recovery attempt before the result is published.
            backend.release_all_full_instrument(target_hwnd.load(Ordering::Acquire))
        }
    }));
    if let Ok(outcome) = &cleanup_result {
        *metrics.terminal_release_outcome.lock() = Some(outcome.clone());
        if !release_state_verified(&backend, outcome) {
            record_termination_error(
                &mut terminal_error,
                &mut secondary_errors,
                format!(
                    "terminal release verification failed: {}",
                    describe_release_outcome(outcome)
                ),
            );
        }
    } else {
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            "terminal backend cleanup panicked".to_string(),
        );
    }

    if terminal_error.is_none()
        && !skip_requested.load(Ordering::Acquire)
        && !quit_requested.load(Ordering::Acquire)
        && !clean_completion_proven(&coordinator, &backend)
    {
        terminal_error = Some(
            "clean completion contract failed: authored generations or backend state were not fully released"
                .to_string(),
        );
    }

    if let Err(error) = coordinator.cancel_all() {
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            format!("coordinator cancellation failure: {error}"),
        );
    }
    if let Err(error) = coordinator.check_post_cleanup_invariants() {
        record_termination_error(
            &mut terminal_error,
            &mut secondary_errors,
            format!("coordinator post-cleanup invariant failure: {error}"),
        );
    }
    let end_qpc = qpc_clock.now().and_then(|ticks| {
        qpc_clock
            .duration_to_us(DurationTicks::from_raw(ticks.as_u64()))
            .map_err(|_| QpcError::ConversionOverflow)
    });
    let end_us = match end_qpc {
        Ok(value) => value,
        Err(error) => {
            record_termination_error(
                &mut terminal_error,
                &mut secondary_errors,
                format!("QPC runtime failure during termination: {error:?}"),
            );
            start_wall_time_us
        }
    };
    let terminal_abort_reason =
        if worker_result.is_err() || cleanup_result.is_err() || terminal_error.is_some() {
            "error"
        } else if skip_requested.load(Ordering::Acquire) {
            "skipped"
        } else if quit_requested.load(Ordering::Acquire) {
            "quit"
        } else {
            "finished"
        };
    *abort_counts.entry(terminal_abort_reason).or_insert(0) += 1;
    *metrics.abort_counts_by_reason.lock() = abort_counts
        .into_iter()
        .map(|(reason, count)| (reason.to_string(), count))
        .collect();
    *metrics.terminal_error.lock() = terminal_error.clone();
    *metrics.secondary_errors.lock() = secondary_errors;
    *metrics.generation_status_counts.lock() = coordinator.generation_status_counts();
    publish_backend_metrics(
        &backend,
        &mut local_metrics,
        metrics,
        &mut last_published_error,
    );

    local_metrics.playback_wall_time_us = end_us.saturating_sub(start_wall_time_us);
    local_metrics.worker_cpu_time_us =
        current_thread_cpu_time_us().saturating_sub(start_thread_cpu_us);
    local_metrics.process_cpu_time_us =
        current_process_cpu_time_us().saturating_sub(start_process_cpu_us);
    if local_metrics.playback_wall_time_us > 0 {
        local_metrics.spin_duty_cycle_ppm = (local_metrics.spin_time_us as u128 * 1_000_000
            / local_metrics.playback_wall_time_us as u128)
            as u64;
    }
    try_publish_metrics(&local_metrics, metrics, end_us, true);
    metrics.is_paused.store(false, Ordering::Relaxed);
    telemetry.output.qpc_frequency_hz = qpc_clock.frequency_hz().get();
    *telemetry_output.lock() = Some(telemetry.output);
    *estimator_output.lock() = serde_json::to_string(&estimator.export_state()).ok();
    match (worker_result, cleanup_result) {
        (Err(payload), _) | (Ok(_), Err(payload)) => resume_unwind(payload),
        (Ok(_), Ok(_)) => {}
    }
    if terminal_error.is_some() {
        OUTCOME_ERROR
    } else if skip_requested.load(Ordering::Acquire) {
        OUTCOME_SKIPPED
    } else if quit_requested.load(Ordering::Acquire) {
        OUTCOME_QUIT
    } else {
        OUTCOME_FINISHED
    }
}

pub fn lease_bounded_ticks(
    target: QpcTicks,
    timeout_ticks: DurationTicks,
    heartbeat_ticks: &AtomicU64,
) -> Result<QpcTicks, QpcError> {
    if timeout_ticks == DurationTicks::ZERO {
        return Ok(target);
    }
    let heartbeat = heartbeat_ticks.load(Ordering::Acquire);
    if heartbeat == 0 {
        return Ok(target);
    }
    let lease_deadline = QpcTicks::from_raw(heartbeat)
        .checked_add_duration(timeout_ticks)
        .map_err(|_| QpcError::DeadlineOverflow)?;
    Ok(target.min(lease_deadline))
}

pub fn supervisor_lease_expired(
    now_ticks: QpcTicks,
    timeout_ticks: DurationTicks,
    heartbeat_ticks: &AtomicU64,
) -> Result<bool, QpcError> {
    if timeout_ticks == DurationTicks::ZERO {
        return Ok(false);
    }
    let heartbeat = heartbeat_ticks.load(Ordering::Acquire);
    if heartbeat == 0 {
        return Ok(false);
    }
    // The supervisor may publish a heartbeat after the worker sampled `now`.
    // A heartbeat at or beyond that sample is fresh, not a QPC underflow or
    // counter-corruption signal.
    if heartbeat >= now_ticks.as_u64() {
        return Ok(false);
    }
    let elapsed = now_ticks
        .checked_duration_since(QpcTicks::from_raw(heartbeat))
        .map_err(|_| QpcError::CounterUnavailable)?;
    Ok(elapsed > timeout_ticks)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetStamp {
    pub hwnd: isize,
    pub generation: u64,
}

pub fn load_target_stamp(target_hwnd: &AtomicIsize, target_generation: &AtomicU64) -> TargetStamp {
    TargetStamp {
        hwnd: target_hwnd.load(Ordering::Acquire),
        generation: target_generation.load(Ordering::Acquire),
    }
}

pub fn focus_matches_hwnd(
    require_focus: bool,
    focus_active: &AtomicBool,
    expected_hwnd: isize,
) -> bool {
    if !require_focus {
        return true;
    }
    let validated_focus_active = focus_active.load(Ordering::Acquire);
    let foreground_matches =
        expected_hwnd == 0 || sky_dispatch_win32::focus::foreground_window_matches(expected_hwnd);
    focus_gate_matches(
        require_focus,
        validated_focus_active,
        expected_hwnd,
        foreground_matches,
    )
}

pub fn focus_matches(
    require_focus: bool,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
) -> bool {
    focus_matches_hwnd(
        require_focus,
        focus_active,
        target_hwnd.load(Ordering::Acquire),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DownAdmission {
    Allowed,
    TargetChanged,
    FocusLost,
    PauseRequested,
    QuitRequested,
    SkipRequested,
    PanicRequested,
}

#[allow(clippy::too_many_arguments)]
pub fn final_down_admission(
    expected: TargetStamp,
    require_focus: bool,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    desired_pause: &AtomicBool,
) -> DownAdmission {
    if !focus_matches_hwnd(require_focus, focus_active, expected.hwnd) {
        return DownAdmission::FocusLost;
    }
    if !target_stamp_still_current(target_hwnd, target_generation, expected) {
        return DownAdmission::TargetChanged;
    }
    if quit_requested.load(Ordering::Acquire) {
        return DownAdmission::QuitRequested;
    }
    if skip_requested.load(Ordering::Acquire) {
        return DownAdmission::SkipRequested;
    }
    if panic_requested.load(Ordering::Acquire) {
        return DownAdmission::PanicRequested;
    }
    if desired_pause.load(Ordering::Acquire) {
        return DownAdmission::PauseRequested;
    }
    DownAdmission::Allowed
}

pub fn ensure_preflight_for_target(
    backend: &TrackedKeyState,
    current: TargetStamp,
    verified_target: &mut Option<TargetStamp>,
) -> Result<(), sky_dispatch_win32::input::PhysicalKeyPreflightError> {
    if *verified_target == Some(current) {
        return Ok(());
    }
    *verified_target = None;
    backend.ensure_instrument_keys_physically_up(current.hwnd)?;
    *verified_target = Some(current);
    Ok(())
}

pub fn target_stamp_still_current(
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    expected: TargetStamp,
) -> bool {
    target_generation.load(Ordering::Acquire) == expected.generation
        && target_hwnd.load(Ordering::Acquire) == expected.hwnd
}

pub fn record_lateness(
    lateness_us: i64,
    is_release: bool,
    deferred_release: bool,
    local_metrics: &mut WorkerMetricsLocal,
) {
    if deferred_release {
        return;
    }
    let clamped = lateness_us.max(0) as u64;
    local_metrics.lateness_us = clamped;
    if is_release {
        local_metrics.release_max_us = local_metrics.release_max_us.max(clamped);
        if clamped > 2_000 {
            local_metrics.release_late_2ms = local_metrics.release_late_2ms.saturating_add(1);
        }
        return;
    }
    local_metrics.max_lateness_us = local_metrics.max_lateness_us.max(clamped);
    if clamped > 10_000 {
        local_metrics.late_10ms = local_metrics.late_10ms.saturating_add(1);
    }
    if clamped > 5_000 {
        local_metrics.late_5ms = local_metrics.late_5ms.saturating_add(1);
    }
    if clamped > 2_000 {
        local_metrics.late_2ms = local_metrics.late_2ms.saturating_add(1);
    }
    local_metrics.recent_latencies.push(lateness_us);
}

pub fn cancel_coordinator_or_terminal(
    coordinator: &mut RuntimeDispatchCoordinator,
    force_full_cleanup: &mut bool,
    terminal_error: &mut Option<String>,
    secondary_errors: &mut Vec<String>,
) {
    if let Err(error) = coordinator.cancel_all() {
        *force_full_cleanup = true;
        record_termination_error(
            terminal_error,
            secondary_errors,
            format!("coordinator cancellation failure: {error}"),
        );
    }
}

pub fn release_outcome_verified(outcome: &ReleaseAllOutcome) -> bool {
    outcome.released_successfully
        && outcome.stuck_keys.is_empty()
        && !outcome.verification_inconclusive
}

pub fn release_state_verified(backend: &TrackedKeyState, outcome: &ReleaseAllOutcome) -> bool {
    release_outcome_verified(outcome)
        && backend.active_mask == 0
        && backend.possibly_active_mask == 0
        && backend.failed_release_mask == 0
}

pub fn clean_completion_proven(
    coordinator: &RuntimeDispatchCoordinator,
    backend: &TrackedKeyState,
) -> bool {
    let counts = coordinator.generation_status_counts();
    let all_released = counts.get("released").copied().unwrap_or_default()
        == coordinator.schedule.generation_count
        && counts.values().sum::<u64>() == coordinator.schedule.generation_count;
    all_released
        && counts.get("scheduled").copied().unwrap_or_default() == 0
        && counts.get("active").copied().unwrap_or_default() == 0
        && counts.get("release_pending").copied().unwrap_or_default() == 0
        && counts.get("dropped_backend").copied().unwrap_or_default() == 0
        && counts.get("dropped_conflict").copied().unwrap_or_default() == 0
        && counts.get("dropped_expired").copied().unwrap_or_default() == 0
        && counts.get("cancelled").copied().unwrap_or_default() == 0
        && backend.active_mask == 0
        && backend.possibly_active_mask == 0
        && backend.failed_release_mask == 0
        && backend.keys_dropped == 0
        && backend.chord_split_events == 0
        && backend.sendinput_partial_events == 0
        && backend.sendinput_zero_progress_failures == 0
        && backend.authored_keys_rejected == 0
}

pub fn describe_release_outcome(outcome: &ReleaseAllOutcome) -> String {
    format!(
        "released_successfully={}, stuck_keys={:?}, verification_inconclusive={}",
        outcome.released_successfully, outcome.stuck_keys, outcome.verification_inconclusive
    )
}

pub fn record_termination_error(
    primary: &mut Option<String>,
    secondary: &mut Vec<String>,
    error: String,
) {
    if primary.is_none() {
        *primary = Some(error);
    } else if primary.as_deref() != Some(error.as_str()) && !secondary.contains(&error) {
        secondary.push(error);
    }
}

/// Release physical input before cancelling only generations that still own it.
///
/// A suspend is resumable: authored generations that have not reached the
/// backend remain Scheduled. The backend result is checked before coordinator
/// state is changed, so an inconclusive release cannot be mistaken for a clean
/// pause.
pub fn suspend_live_input(
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    target_hwnd: isize,
) -> Result<Vec<u64>, String> {
    let initial = backend.release_all(target_hwnd);
    let release = if release_state_verified(backend, &initial) {
        initial
    } else {
        let full = backend.release_all_full_instrument(target_hwnd);
        if !release_state_verified(backend, &full) {
            return Err(format!(
                "release verification failed (initial: {}; full: {})",
                describe_release_outcome(&initial),
                describe_release_outcome(&full),
            ));
        }
        full
    };

    debug_assert!(release_state_verified(backend, &release));
    let cancelled = coordinator
        .cancel_live_generations()
        .map_err(|error| format!("coordinator live cancellation failed: {error}"))?;
    coordinator
        .check_invariants()
        .map_err(|error| format!("coordinator invariant failure after suspension: {error}"))?;
    Ok(cancelled)
}

pub fn release_runtime_outcome(
    deferred_by_us: u64,
    sent_count: usize,
    requested_count: usize,
    _recovery_required: bool,
) -> &'static str {
    let deferred = deferred_by_us > 0;
    match (sent_count == requested_count, sent_count > 0, deferred) {
        (true, _, true) => "deferred_release",
        (true, _, false) => "sent",
        (false, true, true) => "deferred_partial_note_off",
        (false, true, false) => "partial_note_off",
        (false, false, true) => "deferred_failed_note_off",
        (false, false, false) => "failed_note_off",
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub fn update_estimator_after_send(
    estimator: &mut SendLatencyEstimator,
    kind: ActionKind,
    duration_us: u64,
    sent_count: usize,
    authored_polyphony: usize,
    applied_lead_us: u64,
    completion_error_us: i64,
    clean_sample: bool,
) {
    let _ = update_estimator_after_send_class(
        estimator,
        kind,
        duration_us,
        sent_count,
        authored_polyphony,
        applied_lead_us,
        completion_error_us,
        clean_sample,
        LatencyClass::Hot,
    );
}

#[allow(clippy::too_many_arguments)]
pub fn update_estimator_after_send_class(
    estimator: &mut SendLatencyEstimator,
    kind: ActionKind,
    duration_us: u64,
    sent_count: usize,
    authored_polyphony: usize,
    applied_lead_us: u64,
    completion_error_us: i64,
    clean_sample: bool,
    latency_class: LatencyClass,
) -> Result<(), sky_dispatch_core::estimator::EstimatorStateError> {
    if !clean_sample || sent_count == 0 {
        return Ok(());
    }
    estimator.update_observation(
        kind,
        latency_class,
        duration_us,
        authored_polyphony,
        (applied_lead_us > 0).then_some(completion_error_us),
    )
}

pub fn record_lead_saturation(
    counters: &mut [u64; 16],
    positive_residual_at_cap: &mut u64,
    polyphony: usize,
    completion_error_us: i64,
) {
    let bucket = polyphony.clamp(1, 15);
    counters[bucket] = counters[bucket].saturating_add(1);
    if completion_error_us > 0 {
        *positive_residual_at_cap = positive_residual_at_cap.saturating_add(1);
    }
}

pub fn signed_delta(lhs: u64, rhs: u64) -> i64 {
    let delta = lhs as i128 - rhs as i128;
    delta.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

pub fn signed_timeline_delta_ticks(
    completed: TimelineTicks,
    deadline: TimelineTicks,
) -> Result<i64, TimeArithmeticError> {
    let (negative, duration) = if completed >= deadline {
        (false, completed.checked_duration_since(deadline)?)
    } else {
        (true, deadline.checked_duration_since(completed)?)
    };
    let magnitude = duration.as_u64();
    if magnitude <= i64::MAX as u64 {
        let magnitude = i64::try_from(magnitude).map_err(|_| TimeArithmeticError::Overflow)?;
        return Ok(if negative { -magnitude } else { magnitude });
    }
    if negative && magnitude == (i64::MAX as u64) + 1 {
        return Ok(i64::MIN);
    }
    Err(TimeArithmeticError::Overflow)
}

pub fn wake_lateness_ticks(
    wake: TimelineTicks,
    deadline: TimelineTicks,
) -> Result<DurationTicks, TimeArithmeticError> {
    match wake.checked_duration_since(deadline) {
        Ok(duration) => Ok(duration),
        Err(TimeArithmeticError::NegativeOrder) => Ok(DurationTicks::ZERO),
        Err(error) => Err(error),
    }
}

pub fn signed_ticks_to_us(qpc_clock: QpcClock, delta_ticks: i64) -> Result<i64, String> {
    let magnitude = delta_ticks.unsigned_abs();
    let microseconds = qpc_clock
        .duration_to_us(DurationTicks::from_raw(magnitude))
        .map_err(|error| format!("{error:?}"))?;
    let signed = if delta_ticks < 0 {
        -i128::from(microseconds)
    } else {
        i128::from(microseconds)
    };
    i64::try_from(signed).map_err(|_| "signed timing delta exceeds i64 range".to_string())
}

/// Preserve the distinction between a logical operation and one SendInput
/// syscall. The operation spans the first call entry through the final call
/// return; a single-call duration is only valid for exactly one non-rollback
/// call.
#[cfg(test)]
pub fn exact_sender_durations(
    qpc_clock: QpcClock,
    started_ticks: Option<QpcTicks>,
    completed_ticks: Option<QpcTicks>,
    send_attempts: u8,
    rollback_call: bool,
) -> Result<(Option<u64>, Option<u64>), QpcError> {
    if send_attempts == 0 {
        return Ok((None, None));
    }
    let started = started_ticks.ok_or(QpcError::CounterUnavailable)?;
    let completed = completed_ticks.ok_or(QpcError::CounterUnavailable)?;
    let duration = completed
        .checked_duration_since(started)
        .map_err(|_| QpcError::CounterUnavailable)
        .and_then(|ticks| {
            qpc_clock
                .duration_to_us(ticks)
                .map_err(|_| QpcError::ConversionOverflow)
        })?;
    let single_call = (send_attempts == 1 && !rollback_call).then_some(duration);
    Ok((Some(duration), single_call))
}

pub fn classify_latency_class(
    last_send_qpc_ticks: Option<QpcTicks>,
    now_qpc_ticks: QpcTicks,
    cold_threshold_ticks: DurationTicks,
) -> Result<LatencyClass, TimeArithmeticError> {
    let Some(last) = last_send_qpc_ticks else {
        return Ok(LatencyClass::Cold);
    };
    let gap = now_qpc_ticks.checked_duration_since(last)?;
    Ok(if gap > cold_threshold_ticks {
        LatencyClass::Cold
    } else {
        LatencyClass::Hot
    })
}

pub fn anchored_dispatch_target_ticks_typed(
    now_ticks: QpcTicks,
    anchor_ticks: QpcTicks,
    scheduled_ticks: TimelineTicks,
    lead_ticks: DurationTicks,
) -> Result<QpcTicks, QpcError> {
    let authored_target = anchor_ticks
        .checked_add_duration(DurationTicks::from_raw(scheduled_ticks.as_u64()))
        .map_err(|_| QpcError::DeadlineOverflow)?;
    let target = authored_target
        .as_u64()
        .checked_sub(lead_ticks.as_u64())
        .map(QpcTicks::from_raw)
        .ok_or(QpcError::DeadlineOverflow)?;
    Ok(target.max(now_ticks))
}

/// Map an authored timestamp minus lead, including the negative interval that
/// is intentionally needed for a first note at authored t=0.
#[cfg(test)]
#[allow(clippy::manual_unwrap_or, clippy::manual_unwrap_or_default)]
pub fn anchored_dispatch_target_ticks(
    qpc_clock: QpcClock,
    now_ticks: QpcTicks,
    now_qpc_us: u64,
    anchor_us: u64,
    scheduled_us: u64,
    lead_us: u64,
) -> Result<QpcTicks, QpcError> {
    let target_us = match anchor_us
        .checked_add(scheduled_us)
        .ok_or(QpcError::DeadlineOverflow)?
        .checked_sub(lead_us)
    {
        Some(value) => value,
        None => 0,
    };
    if target_us <= now_qpc_us {
        return Ok(now_ticks);
    }
    let delta = qpc_clock
        .duration_from_us(
            target_us
                .checked_sub(now_qpc_us)
                .ok_or(QpcError::DeadlineOverflow)?,
        )
        .map_err(|_| QpcError::DeadlineOverflow)?;
    now_ticks
        .checked_add_duration(delta)
        .map_err(|_| QpcError::DeadlineOverflow)
}

/// Legacy relative helper retained for unit-test compatibility.
#[cfg(test)]
pub fn deadline_target_ticks(
    now_ticks: QpcTicks,
    logical_now_us: u64,
    deadline_us: u64,
) -> QpcTicks {
    QpcTicks::from_raw(now_ticks.as_u64().saturating_add(
        qpc_us_to_ticks(deadline_us.saturating_sub(logical_now_us)).expect("test QPC conversion"),
    ))
}

pub fn publish_wake_error_stats(stats: WakeErrorStats, local_metrics: &mut WorkerMetricsLocal) {
    local_metrics.wake_error_p50_us = stats.p50_us;
    local_metrics.wake_error_p95_us = stats.p95_us;
    local_metrics.wake_error_p99_us = stats.p99_us;
    local_metrics.wake_error_max_us = stats.max_us;
}

pub fn wait_failure_message(failure: WaitFailure) -> String {
    match failure {
        WaitFailure::TimerCreate { win32_error } => {
            format!("high-resolution waitable timer creation failed (Win32 error {win32_error})")
        }
        WaitFailure::TimerArm { win32_error } => {
            format!("high-resolution waitable timer arm failed (Win32 error {win32_error})")
        }
        WaitFailure::TimerWait { win32_error } => {
            format!("high-resolution waitable timer wait failed (Win32 error {win32_error})")
        }
        WaitFailure::MultiWait { win32_error } => {
            format!("interruptible wait failed (Win32 error {win32_error})")
        }
        WaitFailure::Clock => "QPC failed during real-time wait".to_string(),
    }
}

pub fn derive_spin_threshold_us(wake_error_us: u64, spin_floor_us: u64) -> u64 {
    wake_error_us
        .saturating_add(200)
        .clamp(spin_floor_us, 3_000)
}

#[cfg(test)]
pub fn adjust_spin_threshold(current_us: u64, candidate_us: u64) -> u64 {
    if candidate_us >= current_us {
        candidate_us
    } else {
        current_us.saturating_sub(current_us.saturating_sub(candidate_us).min(50))
    }
}

pub fn record_input_path_health(
    send_duration_us: u64,
    elapsed_us: u64,
    warn_us: u64,
    window: &mut VecDeque<u64>,
    over_warn_count: &mut usize,
    warn_started_us: &mut Option<u64>,
    degraded: &mut bool,
) {
    if warn_us == 0 {
        return;
    }
    if window.len() == INPUT_PATH_WINDOW_CAPACITY
        && let Some(value) = window.pop_front()
        && value > warn_us
    {
        *over_warn_count = over_warn_count.saturating_sub(1);
    }
    let value = send_duration_us;
    window.push_back(value);
    debug_assert!(window.len() <= INPUT_PATH_WINDOW_CAPACITY);
    if value > warn_us {
        *over_warn_count += 1;
    }

    let length = window.len();
    let required_warn_samples = (0.95_f64 * (length.saturating_sub(1) as f64)).round() as usize;
    if *over_warn_count
        <= length
            .saturating_sub(1)
            .saturating_sub(required_warn_samples)
    {
        *warn_started_us = None;
        return;
    }
    if warn_started_us.is_none() {
        *warn_started_us = Some(elapsed_us);
        return;
    }
    if elapsed_us.saturating_sub(warn_started_us.unwrap_or(elapsed_us)) >= 1_000_000 {
        *degraded = true;
    }
}

pub fn focus_gate_matches(
    require_focus: bool,
    validated_focus_active: bool,
    target_hwnd: isize,
    foreground_matches: bool,
) -> bool {
    if !require_focus {
        return true;
    }
    let hwnd_matches = target_hwnd != 0 && foreground_matches;
    validated_focus_active && hwnd_matches
}

pub fn publish_backend_metrics(
    backend: &TrackedKeyState,
    local_metrics: &mut WorkerMetricsLocal,
    shared_metrics: &SharedMetrics,
    last_published_error: &mut Option<String>,
) {
    local_metrics.active_count = backend.active_mask.count_ones() as u64;
    local_metrics.keys_dropped = backend.keys_dropped;
    local_metrics.possibly_active_count = backend.possibly_active_mask.count_ones() as u64;
    local_metrics.failed_release_count = backend.failed_release_mask.count_ones() as u64;
    // The healthy dispatch path never takes this lock. Error text is
    // published only when the backend error state changes, including the
    // transition back to None after a successful recovery.
    if last_published_error.as_ref() != backend.last_error.as_ref() {
        let mut published = shared_metrics.last_error.lock();
        *published = backend.last_error.clone();
        *last_published_error = backend.last_error.clone();
    }
    local_metrics.chord_split_events = backend.chord_split_events;
    local_metrics.sendinput_partial_events = backend.sendinput_partial_events;
    local_metrics.sendinput_zero_progress_failures = backend.sendinput_zero_progress_failures;
    local_metrics.chords_rejected = backend.chords_rejected;
    local_metrics.keys_inserted_before_failure = backend.keys_inserted_before_failure;
    local_metrics.keys_rolled_back = backend.keys_rolled_back;
    local_metrics.rollback_residue_keys = backend.rollback_residue_keys;
}
