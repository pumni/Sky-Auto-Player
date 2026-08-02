//! End-to-end real-time native dispatch session engine.

use parking_lot::Mutex;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::coordinator::{CoordinatorError, RuntimeDispatchCoordinator};
use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator};
use sky_dispatch_core::model::{ActionKind, RuntimeSchedule};
use sky_dispatch_core::time::{
    DurationTicks, SEND_COLD_THRESHOLD_US, TimeArithmeticError, TimelineTicks,
};
#[cfg(test)]
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
const OUTCOME_SHUTDOWN_TIMEOUT: u8 = 5;
const PAUSED_POLL_US: u64 = 2_000;
const CORE_WARMUP_SPIN_MAX_US: u64 = 500;
const INPUT_PATH_WINDOW_CAPACITY: usize = 64;
const STRICT_RETRY_LATE_THRESHOLD_US: u64 = 2_000;
const HARD_LATE_ABORT_THRESHOLD_US: u64 = 20_000;
const STRICT_SATURATION_ABORT_STREAK: u8 = 3;
const STARTUP_WAKE_GUARD_US: u64 = 1_000;
const RELEASE_RETRY_BACKOFF_US: [u64; 4] = [2_000, 5_000, 10_000, 20_000];

/// Outcome injected for a single mock-backend `SendInput` call (identified by
/// call index, zero-based, counting both Down and Up calls in order).
///
/// This is test-only infrastructure reachable only when `mock_backend=true`.
/// It never touches the real `SendInput` path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InjectedSendOutcome {
    /// All keys inserted; emitter waits `latency_ticks` QPC ticks (spin).
    Full { latency_ticks: u64 },
    /// Zero keys inserted (complete failure); optional spin.
    Zero {
        latency_ticks: u64,
        win32_error: u32,
    },
    /// Partial insertion: exactly `inserted` keys succeed.
    Partial {
        inserted: u8,
        latency_ticks: u64,
        win32_error: u32,
    },
    /// Emitter spin-stalls for `duration_ticks` QPC ticks without sending.
    Stall { duration_ticks: u64 },
    /// Return from the simulated send boundary, then panic before coordinator commit.
    PanicAfterSend,
    /// Return a complete send receipt but fail the post-send QPC boundary.
    QpcFailureAfterSend,
}

/// Script that maps call-index → `InjectedSendOutcome`.
///
/// Entries are matched by call index in O(n) over the script length (scripts
/// are short — a few entries at most). Calls whose index has no matching entry
/// behave as `InjectedSendOutcome::Full { latency_ticks: 0 }` (success, no latency).
#[derive(Clone, Debug, Default)]
pub struct FaultInjectionScript {
    /// `(call_index, outcome)` pairs, unsorted.
    pub entries: Vec<(usize, InjectedSendOutcome)>,
    /// Base latency applied to every call (on top of per-call outcome latency).
    pub base_latency_ticks: u64,
    /// Extra latency per key, in QPC ticks, applied to every call.
    pub per_key_latency_ticks: u64,
    pub focus_loss_after_due_before_send: bool,
    pub wait_failure: bool,
}

impl FaultInjectionScript {
    /// No failures, no latency.
    pub fn none() -> Self {
        Self::default()
    }

    /// Fail the first 3 Up calls (transient release failure).
    pub fn transient_release() -> Self {
        Self {
            entries: vec![
                (
                    1,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
                (
                    2,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
                (
                    3,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
            ],
            ..Default::default()
        }
    }

    /// All Up calls fail (persistent release failure).
    ///
    /// The first Down call is index 0.  Every subsequent emitter call is an Up
    /// call or an Up retry for the fault-injection schedules used by the
    /// worker tests, so all indices from 1 onward fail.  This deliberately
    /// avoids assuming that Up calls have odd indices: an immediate retry is
    /// another Up call and must remain failed in this mode.
    pub fn persistent_release() -> Self {
        // Inject 128 failures — sufficient for any reasonable test schedule.
        let entries = (1..128)
            .map(|i| {
                (
                    i,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                )
            })
            .collect();
        Self {
            entries,
            ..Default::default()
        }
    }

    /// The first Down call gets zero progress (ZeroProgressDownOnce).
    pub fn zero_progress_down_once() -> Self {
        Self {
            entries: vec![(
                0,
                InjectedSendOutcome::Zero {
                    latency_ticks: 0,
                    win32_error: 1460,
                },
            )],
            ..Default::default()
        }
    }

    /// Both immediate Down attempts make zero progress.
    pub fn persistent_zero_down() -> Self {
        Self {
            entries: vec![
                (
                    0,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
                (
                    1,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
            ],
            ..Default::default()
        }
    }

    /// The first Down attempt partially inserts a chord.
    pub fn partial_down_first_attempt() -> Self {
        Self {
            entries: vec![(
                0,
                InjectedSendOutcome::Partial {
                    inserted: 1,
                    latency_ticks: 0,
                    win32_error: 5,
                },
            )],
            ..Default::default()
        }
    }

    /// The first Down attempt is empty, then the immediate retry splits.
    pub fn partial_down_after_zero_retry() -> Self {
        Self {
            entries: vec![
                (
                    0,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
                (
                    1,
                    InjectedSendOutcome::Partial {
                        inserted: 1,
                        latency_ticks: 0,
                        win32_error: 5,
                    },
                ),
            ],
            ..Default::default()
        }
    }

    /// Every Up attempt makes zero progress.
    pub fn persistent_zero_up() -> Self {
        Self::persistent_release()
    }

    /// Panic after the simulated SendInput boundary and before coordinator commit.
    pub fn panic_after_send_before_commit() -> Self {
        Self {
            entries: vec![(0, InjectedSendOutcome::PanicAfterSend)],
            ..Default::default()
        }
    }

    pub fn focus_loss_after_due_before_send() -> Self {
        Self {
            focus_loss_after_due_before_send: true,
            ..Default::default()
        }
    }

    pub fn qpc_failure_after_send() -> Self {
        Self {
            entries: vec![(0, InjectedSendOutcome::QpcFailureAfterSend)],
            ..Default::default()
        }
    }

    pub fn wait_failure() -> Self {
        Self {
            wait_failure: true,
            ..Default::default()
        }
    }

    /// Resolve the outcome for `call_index`.  Returns `None` → Full success, no latency.
    pub fn resolve(&self, call_index: usize) -> Option<&InjectedSendOutcome> {
        self.entries
            .iter()
            .find(|(idx, _)| *idx == call_index)
            .map(|(_, o)| o)
    }
}

/// Kept for backward-compatibility with existing call sites that imported this name.
/// New code should use `FaultInjectionScript` directly.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct EngineSnapshot {
    pub elapsed_us: u64,
    pub total_us: u64,
    pub lateness_us: u64,
    pub max_lateness_us: u64,
    pub late_2ms: u64,
    pub late_5ms: u64,
    pub late_10ms: u64,
    pub release_max_us: u64,
    pub release_late_2ms: u64,
    pub recent_latencies_us: Vec<i64>,
    pub is_running: bool,
    pub is_finished: bool,
    pub is_paused: bool,
    pub status: String,
    pub active_count: usize,
    pub possibly_active_count: usize,
    pub failed_release_count: usize,
    pub last_error: Option<String>,
    pub keys_dropped: u64,
    pub chord_split_events: u64,
    pub sendinput_partial_events: u64,
    pub sendinput_zero_progress_failures: u64,
    pub chords_rejected: u64,
    pub authored_conflict_events: u64,
    pub authored_chords_rejected: u64,
    pub authored_keys_rejected: u64,
    pub keys_inserted_before_failure: u64,
    pub keys_rolled_back: u64,
    pub rollback_residue_keys: u64,
    pub lead_saturation_count_down: Vec<u64>,
    pub lead_saturation_count_up: Vec<u64>,
    pub positive_residual_at_cap: u64,
    pub recovered_zero_progress_but_late: u64,
    pub outcome: Option<String>,
    pub rt_priority_acquired: String,
    pub effective_spin_threshold_us: u64,
    pub wake_error_p50_us: u64,
    pub wake_error_p95_us: u64,
    pub wake_error_p99_us: u64,
    pub wake_error_max_us: u64,
    pub spin_time_us: u64,
    pub playback_wall_time_us: u64,
    pub spin_duty_cycle_ppm: u64,
    pub worker_cpu_time_us: u64,
    pub process_cpu_time_us: u64,
    pub wait_strategy_acquired: String,
    pub power_throttling_disabled: bool,
    pub input_path_degraded: bool,
    pub sendinput_path_degraded: bool,
    pub bookkeeping_degraded: bool,
    pub wait_path_degraded: bool,
    pub wait_target_error_us: u64,
    pub idle_wake_count: u64,
    pub terminal_error: Option<String>,
    pub secondary_errors: Vec<String>,
    pub generation_count: u64,
    pub generation_status_counts: HashMap<String, u64>,
    pub abort_counts_by_reason: HashMap<String, u64>,
    pub release_outcome: Option<ReleaseAllOutcome>,
}

/// Fixed-size record retained on the real-time worker path.
///
/// Human-readable outcome names, authored reasons, scan-code lists and
/// microsecond projections are deliberately materialized only after the
/// worker has stopped.  This is the only representation kept in the bounded
/// native ring.
#[repr(C)]
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct RtTraceRecord {
    pub event_index: u32,
    pub kind: u8,
    pub outcome: u8,
    pub polyphony: u8,
    pub flags: u8,
    pub authored_ticks: u64,
    pub effective_deadline_ticks: u64,
    pub wake_ticks: u64,
    pub send_started_ticks: u64,
    pub send_completed_ticks: u64,
    pub completion_error_ticks: i64,
    pub applied_lead_ticks: u32,
    pub win32_error: u32,
}

pub const NATIVE_TELEMETRY_SCHEMA_VERSION: u32 = 4;

const TRACE_KIND_DOWN: u8 = 0;
const TRACE_KIND_UP: u8 = 1;
const TRACE_FLAG_SENT_FULL: u8 = 1 << 0;
const TRACE_FLAG_RECOVERY: u8 = 1 << 1;
const TRACE_FLAG_DEFERRED: u8 = 1 << 2;
const TRACE_FLAG_ANOMALY: u8 = 1 << 3;

fn trace_outcome_code(outcome: &str) -> u8 {
    match outcome {
        "sent" => 0,
        "deferred_release" => 1,
        "failed_note_off" => 2,
        "blocked_unfocused" => 3,
        "suppressed_stale_up" => 4,
        "recovered_zero_progress_but_late" => 5,
        "strict_completion_slo_exceeded" => 6,
        "chord_integrity_lost" => 7,
        "aborted" => 8,
        _ => 255,
    }
}

#[derive(Debug, Default, serde::Serialize)]
pub struct NativeTelemetrySummary {
    pub dispatch_count: u64,
    pub down_count: u64,
    pub up_count: u64,
    pub requested_key_count: u64,
    pub sent_key_count: u64,
    pub skipped_key_count: u64,
    pub max_lateness_us: i64,
    pub max_send_duration_us: u64,
    pub lateness_histogram_50us: [u64; 16],
    pub send_duration_histogram_50us: [u64; 16],
    /// Values at or above the final finite histogram bucket. The last array
    /// slot is retained for compatibility; this counter makes the tail
    /// explicit instead of pretending it is a narrow 750–800 µs bucket.
    pub lateness_overflow_count: u64,
    pub send_duration_overflow_count: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TimingSemantics {
    pub evidence_kind: &'static str,
    pub scheduled_boundary: &'static str,
    pub wake_boundary: &'static str,
    pub sender_start_boundary: &'static str,
    pub sender_completion_boundary: &'static str,
    pub game_observed_available: bool,
}

impl Default for TimingSemantics {
    fn default() -> Self {
        Self {
            evidence_kind: "sender_completion",
            scheduled_boundary: "authored_timeline",
            wake_boundary: "worker_wake_before_sendinput",
            sender_start_boundary: "sendinput_call_entry",
            sender_completion_boundary: "sendinput_call_return",
            game_observed_available: false,
        }
    }
}

impl NativeTelemetrySummary {
    fn observe(&mut self, record: &RtTraceRecord) {
        self.dispatch_count = self.dispatch_count.saturating_add(1);
        match record.kind {
            TRACE_KIND_DOWN => self.down_count = self.down_count.saturating_add(1),
            TRACE_KIND_UP => self.up_count = self.up_count.saturating_add(1),
            _ => {}
        }
        self.requested_key_count = self
            .requested_key_count
            .saturating_add(u64::from(record.polyphony));
        if record.flags & TRACE_FLAG_SENT_FULL != 0 {
            self.sent_key_count = self
                .sent_key_count
                .saturating_add(u64::from(record.polyphony));
        }
        if record.flags & TRACE_FLAG_SENT_FULL == 0 {
            self.skipped_key_count = self
                .skipped_key_count
                .saturating_add(u64::from(record.polyphony));
        }
    }
}

#[derive(Debug, Default, serde::Serialize)]
pub struct NativeTelemetryOutput {
    pub schema_version: u32,
    pub qpc_frequency_hz: u64,
    pub records: VecDeque<RtTraceRecord>,
    pub summary: NativeTelemetrySummary,
    pub attempted: u64,
    pub accepted: u64,
    pub dropped: u64,
    pub truncated: bool,
    pub timing_semantics: TimingSemantics,
}

impl NativeTelemetryOutput {
    fn new(mode: TelemetryMode, capacity: usize) -> Self {
        Self {
            schema_version: NATIVE_TELEMETRY_SCHEMA_VERSION,
            qpc_frequency_hz: 0,
            records: if matches!(mode, TelemetryMode::Ring) {
                // Reserve the complete bounded buffer before the worker
                // epoch. Telemetry is an opt-in diagnostic mode; once it is
                // enabled, a later VecDeque growth/copy on the dispatch thread is
                // a worse failure mode than its predictable memory cost.
                VecDeque::with_capacity(capacity)
            } else {
                VecDeque::new()
            },
            summary: NativeTelemetrySummary::default(),
            attempted: 0,
            accepted: 0,
            dropped: 0,
            truncated: false,
            timing_semantics: TimingSemantics::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryMode {
    Off,
    Ring,
}

/// Deliberate session profiles. The profile owns backend/policy selection so
/// callers do not compose a contradictory matrix of booleans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchProfile {
    Production,
    StrictTimingDiagnostic,
    MockTest,
}

impl DispatchProfile {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "production" => Ok(Self::Production),
            "strict_timing_diagnostic" => Ok(Self::StrictTimingDiagnostic),
            "mock_test" => Ok(Self::MockTest),
            _ => Err(
                "profile must be 'production', 'strict_timing_diagnostic', or 'mock_test'"
                    .to_string(),
            ),
        }
    }

    pub(crate) fn uses_mock_backend(self) -> bool {
        matches!(self, Self::MockTest)
    }

    pub(crate) fn strict_timing(self) -> bool {
        matches!(self, Self::StrictTimingDiagnostic)
    }
}

struct TelemetryCollector {
    mode: TelemetryMode,
    capacity: usize,
    output: NativeTelemetryOutput,
}

impl TelemetryCollector {
    fn new(mode: TelemetryMode, capacity: usize) -> Self {
        Self {
            mode,
            capacity,
            output: NativeTelemetryOutput::new(mode, capacity),
        }
    }

    fn push<F>(&mut self, build: F)
    where
        F: FnOnce() -> RtTraceRecord,
    {
        self.output.attempted = self.output.attempted.saturating_add(1);
        if self.mode == TelemetryMode::Off {
            return;
        }

        if self.output.records.len() == self.capacity {
            self.output.dropped = self.output.dropped.saturating_add(1);
            self.output.truncated = true;
            return;
        }

        let record = build();
        self.output.summary.observe(&record);

        match self.mode {
            TelemetryMode::Off => unreachable!(),
            TelemetryMode::Ring => {
                self.output.records.push_back(record);
                self.output.accepted = self.output.accepted.saturating_add(1);
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct RecentLatencyRing {
    values: [i32; 32],
    next: u8,
    len: u8,
}

impl RecentLatencyRing {
    fn push(&mut self, value: i64) {
        self.values[usize::from(self.next)] =
            value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        self.next = (self.next + 1) % self.values.len() as u8;
        self.len = self.len.saturating_add(1).min(self.values.len() as u8);
    }

    fn to_vec(&self) -> Vec<i64> {
        let len = usize::from(self.len);
        let start = if self.len == self.values.len() as u8 {
            usize::from(self.next)
        } else {
            0
        };
        (0..len)
            .map(|offset| i64::from(self.values[(start + offset) % self.values.len()]))
            .collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkerMetricsLocal {
    pub elapsed_us: u64,
    pub total_us: u64,
    pub lateness_us: u64,
    pub max_lateness_us: u64,
    pub late_2ms: u64,
    pub late_5ms: u64,
    pub late_10ms: u64,
    pub release_max_us: u64,
    pub release_late_2ms: u64,
    pub active_count: u64,
    pub possibly_active_count: u64,
    pub failed_release_count: u64,
    pub keys_dropped: u64,
    pub chord_split_events: u64,
    pub sendinput_partial_events: u64,
    pub sendinput_zero_progress_failures: u64,
    pub chords_rejected: u64,
    pub authored_conflict_events: u64,
    pub authored_chords_rejected: u64,
    pub authored_keys_rejected: u64,
    pub keys_inserted_before_failure: u64,
    pub keys_rolled_back: u64,
    pub rollback_residue_keys: u64,
    pub lead_saturation_count_down: [u64; 16],
    pub lead_saturation_count_up: [u64; 16],
    pub positive_residual_at_cap: u64,
    pub recovered_zero_progress_but_late: u64,
    pub effective_spin_threshold_us: u64,
    pub wake_error_p50_us: u64,
    pub wake_error_p95_us: u64,
    pub wake_error_p99_us: u64,
    pub wake_error_max_us: u64,
    pub spin_time_us: u64,
    pub playback_wall_time_us: u64,
    pub spin_duty_cycle_ppm: u64,
    pub worker_cpu_time_us: u64,
    pub process_cpu_time_us: u64,
    pub power_throttling_disabled: bool,
    pub input_path_degraded: bool,
    pub sendinput_path_degraded: bool,
    pub bookkeeping_degraded: bool,
    pub wait_path_degraded: bool,
    pub wait_target_error_us: u64,
    pub idle_wake_count: u64,
    recent_latencies: RecentLatencyRing,
}

#[derive(Default)]
struct SharedMetrics {
    snapshot: parking_lot::Mutex<WorkerMetricsLocal>,
    last_publish_us: AtomicU64,
    is_paused: AtomicBool,
    panicked: AtomicBool,
    last_error: Mutex<Option<String>>,
    wait_strategy_acquired: Mutex<String>,
    terminal_error: Mutex<Option<String>>,
    secondary_errors: Mutex<Vec<String>>,
    generation_status_counts: Mutex<HashMap<String, u64>>,
    abort_counts_by_reason: Mutex<HashMap<String, u64>>,
    terminal_release_outcome: Mutex<Option<ReleaseAllOutcome>>,
}

fn try_publish_metrics(
    local: &WorkerMetricsLocal,
    shared: &SharedMetrics,
    now_us: u64,
    force: bool,
) {
    let last = shared.last_publish_us.load(Ordering::Relaxed);
    if (force || now_us.saturating_sub(last) >= 50_000)
        && let Some(mut guard) = shared.snapshot.try_lock()
    {
        *guard = local.clone();
        shared.last_publish_us.store(now_us, Ordering::Relaxed);
    }
}

struct WorkerConfig {
    schedule: RuntimeSchedule,
    min_hold_us: u64,
    max_lead_us: u64,
    dispatch_lead_us: u64,
    allowed_count: usize,
    mock_backend: bool,
    mock_latency_base_us: u64,
    mock_latency_per_key_us: u64,
    fault_script: FaultInjectionScript,
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
}

pub struct NativeDispatchSession {
    config: Mutex<Option<WorkerConfig>>,
    generation_count: u64,
    interrupt: Arc<OwnedEvent>,
    desired_pause: Arc<AtomicBool>,
    quit_requested: Arc<AtomicBool>,
    skip_requested: Arc<AtomicBool>,
    panic_requested: Arc<AtomicBool>,
    focus_active: Arc<AtomicBool>,
    target_hwnd: Arc<AtomicIsize>,
    lifecycle: Arc<AtomicU8>,
    terminal_outcome: Arc<AtomicU8>,
    metrics: Arc<SharedMetrics>,
    thread_handle: Mutex<Option<std::thread::JoinHandle<()>>>,
    completed: Arc<(StdMutex<bool>, Condvar)>,
    telemetry_output: Arc<Mutex<Option<NativeTelemetryOutput>>>,
    priority_acquired: Arc<Mutex<String>>,
    estimator_output: Arc<Mutex<Option<String>>>,
    supervisor_heartbeat_ticks: Arc<AtomicU64>,
}

impl NativeDispatchSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        schedule: RuntimeSchedule,
        min_hold_us: u64,
        max_lead_us: u64,
        dispatch_lead_us: u64,
        allowed_scan_codes: Vec<u16>,
        mock_backend: bool,
        mock_latency_base_us: u64,
        mock_latency_per_key_us: u64,
        fault_script: FaultInjectionScript,
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
        let initial_heartbeat_ticks = sky_dispatch_win32::clock::qpc_now_ticks_checked()
            .map_err(|error| format!("QPC admission failed before session creation: {error:?}"))?;
        let interrupt = OwnedEvent::new_auto_reset()
            .ok_or_else(|| "failed to create command event".to_string())?;
        let total_us = schedule
            .batches
            .last()
            .map_or(0, |batch| batch.scheduled_us);
        let generation_count = schedule.generation_count;
        let metrics = Arc::new(SharedMetrics::default());
        metrics.snapshot.lock().total_us = total_us;
        Ok(Self {
            config: Mutex::new(Some(WorkerConfig {
                schedule,
                min_hold_us,
                max_lead_us,
                dispatch_lead_us,
                allowed_count: allowed_scan_codes.len(),
                mock_backend,
                mock_latency_base_us,
                mock_latency_per_key_us,
                fault_script,
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
            interrupt: Arc::new(interrupt),
            desired_pause: Arc::new(AtomicBool::new(false)),
            quit_requested: Arc::new(AtomicBool::new(false)),
            skip_requested: Arc::new(AtomicBool::new(false)),
            panic_requested: Arc::new(AtomicBool::new(false)),
            focus_active: Arc::new(AtomicBool::new(!require_focus)),
            target_hwnd: Arc::new(AtomicIsize::new(0)),
            lifecycle: Arc::new(AtomicU8::new(LIFECYCLE_NEW)),
            terminal_outcome: Arc::new(AtomicU8::new(OUTCOME_NONE)),
            metrics,
            thread_handle: Mutex::new(None),
            completed: Arc::new((StdMutex::new(false), Condvar::new())),
            telemetry_output: Arc::new(Mutex::new(None)),
            priority_acquired: Arc::new(Mutex::new("pending".to_string())),
            estimator_output: Arc::new(Mutex::new(None)),
            supervisor_heartbeat_ticks: Arc::new(AtomicU64::new(initial_heartbeat_ticks.as_u64())),
        })
    }

    pub fn start(&self) -> Result<(), String> {
        self.lifecycle
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
                self.lifecycle.store(LIFECYCLE_POISONED, Ordering::Release);
                return Err(format!(
                    "QPC admission failed before worker start: {error:?}"
                ));
            }
        };
        let Some(config) = self.config.lock().take() else {
            self.lifecycle.store(LIFECYCLE_POISONED, Ordering::Release);
            return Err("session configuration is no longer available".to_string());
        };

        let interrupt = Arc::clone(&self.interrupt);
        let desired_pause = Arc::clone(&self.desired_pause);
        let quit_requested = Arc::clone(&self.quit_requested);
        let skip_requested = Arc::clone(&self.skip_requested);
        let panic_requested = Arc::clone(&self.panic_requested);
        let focus_active = Arc::clone(&self.focus_active);
        let target_hwnd = Arc::clone(&self.target_hwnd);
        let lifecycle = Arc::clone(&self.lifecycle);
        let terminal_outcome = Arc::clone(&self.terminal_outcome);
        let metrics = Arc::clone(&self.metrics);
        let completed = Arc::clone(&self.completed);
        let telemetry_output = Arc::clone(&self.telemetry_output);
        let priority_acquired = Arc::clone(&self.priority_acquired);
        let estimator_output = Arc::clone(&self.estimator_output);
        let supervisor_heartbeat_ticks = Arc::clone(&self.supervisor_heartbeat_ticks);
        supervisor_heartbeat_ticks.store(heartbeat_ticks.as_u64(), Ordering::Release);

        let spawn_result = std::thread::Builder::new()
            .name("sky-native-dispatch".to_string())
            .spawn(move || {
                let worker_result = catch_unwind(AssertUnwindSafe(|| {
                    run_worker(
                        config,
                        &interrupt,
                        &desired_pause,
                        &quit_requested,
                        &skip_requested,
                        &panic_requested,
                        &focus_active,
                        &target_hwnd,
                        &metrics,
                        &telemetry_output,
                        &priority_acquired,
                        &estimator_output,
                        &supervisor_heartbeat_ticks,
                    )
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
                    *metrics.terminal_error.lock() = Some(message);
                }
                if terminal_outcome.load(Ordering::Acquire) != OUTCOME_SHUTDOWN_TIMEOUT {
                    terminal_outcome.store(worker_outcome, Ordering::Release);
                }
                metrics.panicked.store(panicked, Ordering::Release);
                if panicked {
                    lifecycle.store(LIFECYCLE_POISONED, Ordering::Release);
                } else {
                    // A join timeout poisons the session permanently; a late
                    // worker exit must not make that session look healthy.
                    let _ = lifecycle.compare_exchange(
                        LIFECYCLE_RUNNING,
                        LIFECYCLE_FINISHED,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                }
                let (done_lock, done_cv) = &*completed;
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
                self.lifecycle.store(LIFECYCLE_POISONED, Ordering::Release);
                Err(format!("failed to spawn native dispatch worker: {error}"))
            }
        }
    }

    fn signal_worker(&self) -> Result<(), String> {
        if !matches!(
            self.lifecycle.load(Ordering::Acquire),
            LIFECYCLE_RUNNING | LIFECYCLE_POISONED
        ) {
            return Err("session commands require a running worker".to_string());
        }
        let _ = self.interrupt.signal();
        Ok(())
    }

    pub fn pause(&self) -> Result<(), String> {
        if self.lifecycle.load(Ordering::Acquire) != LIFECYCLE_RUNNING {
            return Err("session commands require a running worker".to_string());
        }
        self.desired_pause.store(true, Ordering::Release);
        let _ = self.interrupt.signal();
        Ok(())
    }

    pub fn resume(&self) -> Result<(), String> {
        if self.lifecycle.load(Ordering::Acquire) != LIFECYCLE_RUNNING {
            return Err("session commands require a running worker".to_string());
        }
        self.desired_pause.store(false, Ordering::Release);
        let _ = self.interrupt.signal();
        Ok(())
    }

    pub fn skip(&self) -> Result<(), String> {
        if !matches!(
            self.lifecycle.load(Ordering::Acquire),
            LIFECYCLE_RUNNING | LIFECYCLE_POISONED
        ) {
            return Err("session commands require a running worker".to_string());
        }
        self.skip_requested.store(true, Ordering::Release);
        self.signal_worker()
    }

    pub fn quit(&self) -> Result<(), String> {
        if !matches!(
            self.lifecycle.load(Ordering::Acquire),
            LIFECYCLE_RUNNING | LIFECYCLE_POISONED
        ) {
            return Err("session commands require a running worker".to_string());
        }
        self.quit_requested.store(true, Ordering::Release);
        self.signal_worker()
    }

    pub fn panic_release(&self) -> Result<(), String> {
        if !matches!(
            self.lifecycle.load(Ordering::Acquire),
            LIFECYCLE_RUNNING | LIFECYCLE_POISONED
        ) {
            return Err("session commands require a running worker".to_string());
        }
        self.panic_requested.store(true, Ordering::Release);
        self.signal_worker()
    }

    pub fn update_focus(&self, active: bool) {
        if self.focus_active.swap(active, Ordering::AcqRel) != active {
            let _ = self.interrupt.signal();
        }
    }

    pub fn heartbeat(&self) -> Result<(), String> {
        if self.lifecycle.load(Ordering::Acquire) == LIFECYCLE_RUNNING {
            let now = sky_dispatch_win32::clock::qpc_now_ticks_checked()
                .map_err(|error| format!("QPC heartbeat failed: {error:?}"))?;
            self.supervisor_heartbeat_ticks
                .store(now.as_u64(), Ordering::Release);
        }
        Ok(())
    }

    pub fn set_target_hwnd(&self, hwnd: isize) {
        if self.target_hwnd.swap(hwnd, Ordering::AcqRel) != hwnd {
            let _ = self.interrupt.signal();
        }
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        let lifecycle = self.lifecycle.load(Ordering::Acquire);
        let paused = self.metrics.is_paused.load(Ordering::Relaxed);
        let outcome = self.terminal_outcome.load(Ordering::Acquire);
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
            LIFECYCLE_POISONED if self.metrics.panicked.load(Ordering::Acquire) => "panicked",
            LIFECYCLE_POISONED => "poisoned",
            _ => "invalid",
        };
        let local = self.metrics.snapshot.lock().clone();
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
            last_error: self.metrics.last_error.lock().clone(),
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
            rt_priority_acquired: self.priority_acquired.lock().clone(),
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
            wait_strategy_acquired: self.metrics.wait_strategy_acquired.lock().clone(),
            power_throttling_disabled: local.power_throttling_disabled,
            input_path_degraded: local.input_path_degraded,
            sendinput_path_degraded: local.sendinput_path_degraded,
            bookkeeping_degraded: local.bookkeeping_degraded,
            wait_path_degraded: local.wait_path_degraded,
            wait_target_error_us: local.wait_target_error_us,
            idle_wake_count: local.idle_wake_count,
            terminal_error: self.metrics.terminal_error.lock().clone(),
            secondary_errors: self.metrics.secondary_errors.lock().clone(),
            generation_count: self.generation_count,
            generation_status_counts: self.metrics.generation_status_counts.lock().clone(),
            abort_counts_by_reason: self.metrics.abort_counts_by_reason.lock().clone(),
            release_outcome: self.metrics.terminal_release_outcome.lock().clone(),
        }
    }

    pub fn join(&self, timeout: Duration) -> Result<bool, String> {
        if self.lifecycle.load(Ordering::Acquire) == LIFECYCLE_NEW {
            return Err("session has not been started".to_string());
        }
        let (done_lock, done_cv) = &*self.completed;
        let done = done_lock
            .lock()
            .map_err(|_| "session completion lock was poisoned".to_string())?;
        let (done, _) = done_cv
            .wait_timeout_while(done, timeout, |done| !*done)
            .map_err(|_| "session completion wait was poisoned".to_string())?;
        if !*done {
            self.lifecycle.store(LIFECYCLE_POISONED, Ordering::Release);
            self.terminal_outcome
                .store(OUTCOME_SHUTDOWN_TIMEOUT, Ordering::Release);
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
        let (done_lock, _) = &*self.completed;
        let done = done_lock
            .lock()
            .map_err(|_| "session completion lock was poisoned".to_string())?;
        if !*done {
            return Err("telemetry is available only after worker termination".to_string());
        }
        drop(done);
        let output = self
            .telemetry_output
            .lock()
            .take()
            .ok_or_else(|| "telemetry has already been taken".to_string())?;
        serde_json::to_string(&output)
            .map_err(|error| format!("failed to serialize native telemetry: {error}"))
    }

    pub fn terminal_outcome(&self) -> Option<&'static str> {
        match self.terminal_outcome.load(Ordering::Acquire) {
            OUTCOME_NONE => None,
            OUTCOME_FINISHED => Some("finished"),
            OUTCOME_QUIT => Some("quit"),
            OUTCOME_SKIPPED => Some("skipped"),
            OUTCOME_ERROR => Some("error"),
            OUTCOME_SHUTDOWN_TIMEOUT => Some("shutdown_timeout"),
            _ => Some("error"),
        }
    }

    pub fn estimator_state_json(&self) -> Result<String, String> {
        let (done_lock, _) = &*self.completed;
        let done = done_lock
            .lock()
            .map_err(|_| "session completion lock was poisoned".to_string())?;
        if !*done {
            return Err("estimator state is available only after worker termination".to_string());
        }
        drop(done);
        self.estimator_output
            .lock()
            .clone()
            .ok_or_else(|| "native estimator state is unavailable".to_string())
    }
}

fn mock_platform_send_result(
    qpc_clock: QpcClock,
    requested: u32,
    inserted: u32,
    win32_error: u32,
    latency_ticks: u64,
) -> PlatformSendResult {
    let started_ticks = match qpc_clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return PlatformSendResult {
                requested,
                inserted: 0,
                started_ticks: QpcTicks::ZERO,
                completed_ticks: None,
                completed_us: 0,
                win32_error,
                timing_error: Some(error),
            };
        }
    };
    let deadline = match started_ticks.checked_add_duration(DurationTicks::from_raw(latency_ticks))
    {
        Ok(deadline) => deadline,
        Err(_) => {
            return PlatformSendResult {
                requested,
                inserted: 0,
                started_ticks,
                completed_ticks: None,
                completed_us: 0,
                win32_error,
                timing_error: Some(QpcError::DeadlineOverflow),
            };
        }
    };
    loop {
        match qpc_clock.now() {
            Ok(now) if now >= deadline => {
                let (completed_us, timing_error) =
                    match qpc_clock.duration_to_us(DurationTicks::from_raw(now.as_u64())) {
                        Ok(micros) => (micros, None),
                        Err(_) => (0, Some(QpcError::ConversionOverflow)),
                    };
                return PlatformSendResult {
                    requested,
                    inserted,
                    started_ticks,
                    completed_ticks: Some(now),
                    completed_us,
                    win32_error,
                    timing_error,
                };
            }
            Ok(_) => std::hint::spin_loop(),
            Err(error) => {
                return PlatformSendResult {
                    requested,
                    inserted: 0,
                    started_ticks,
                    completed_ticks: None,
                    completed_us: 0,
                    win32_error,
                    timing_error: Some(error),
                };
            }
        }
    }
}

fn admission_failure(
    backend: &mut TrackedKeyState,
    metrics: &SharedMetrics,
    primary_error: String,
) -> u8 {
    let cleanup = backend.release_all_full_instrument();
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
}

#[allow(clippy::too_many_arguments)]
fn run_worker(
    config: WorkerConfig,
    interrupt: &OwnedEvent,
    desired_pause: &AtomicBool,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    metrics: &SharedMetrics,
    telemetry_output: &Mutex<Option<NativeTelemetryOutput>>,
    priority_acquired: &Mutex<String>,
    estimator_output: &Mutex<Option<String>>,
    supervisor_heartbeat_ticks: &AtomicU64,
) -> u8 {
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
            if base_latency_us > 0 {
                // Legacy latency path: real sleep (matches old mock behaviour).
                std::thread::sleep(Duration::from_micros(base_latency_us));
            }

            match script_emitter.resolve(idx) {
                None | Some(InjectedSendOutcome::Full { latency_ticks: 0 }) => {
                    // Fast path: full success, no extra latency.
                    mock_platform_send_result(
                        qpc_clock,
                        codes.len() as u32,
                        codes.len() as u32,
                        0,
                        0,
                    )
                }
                Some(InjectedSendOutcome::Full { latency_ticks }) => mock_platform_send_result(
                    qpc_clock,
                    codes.len() as u32,
                    codes.len() as u32,
                    0,
                    *latency_ticks,
                ),
                Some(InjectedSendOutcome::Zero {
                    latency_ticks,
                    win32_error,
                }) => mock_platform_send_result(
                    qpc_clock,
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
                    mock_platform_send_result(
                        qpc_clock,
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
                    mock_platform_send_result(qpc_clock, codes.len() as u32, 0, 0, *duration_ticks)
                }
                Some(InjectedSendOutcome::PanicAfterSend) => {
                    let _ = mock_platform_send_result(
                        qpc_clock,
                        codes.len() as u32,
                        codes.len() as u32,
                        0,
                        0,
                    );
                    panic!("fault injection: panic after send before commit");
                }
                Some(InjectedSendOutcome::QpcFailureAfterSend) => {
                    let mut result = mock_platform_send_result(
                        qpc_clock,
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
    let mut physical_key_preflight_required = true;
    let start_wall_time_us = initial_now_us;
    let start_thread_cpu_us = current_thread_cpu_time_us();
    let start_process_cpu_us = current_process_cpu_time_us();
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
            local_metrics.worker_cpu_time_us =
                current_thread_cpu_time_us().saturating_sub(start_thread_cpu_us);
            local_metrics.process_cpu_time_us =
                current_process_cpu_time_us().saturating_sub(start_process_cpu_us);
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
                let panic_release = backend.release_all_full_instrument();
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

            if !focus_ok {
                focus_restore_started_ticks = None;
                if !clock_state.has_pause_reason("focus") {
                    if let Err(error) = suspend_live_input(&mut backend, &mut coordinator) {
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
                    if let Err(error) = suspend_live_input(&mut backend, &mut coordinator) {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("focus restoration failed: {error}"));
                        break;
                    }
                    // Cleanup can include bounded backend retries. Re-sample
                    // QPC after it completes so that the cleanup interval is
                    // included in the focus pause rather than lost from the
                    // playback clock.
                    let resumed_ticks = qpc_ticks_or_terminal!();
                    if let Err(error) = clock_state.exit_pause("focus", resumed_ticks) {
                        force_full_cleanup = true;
                        terminal_error = Some(format!("playback clock failure: {error}"));
                        break;
                    }
                    focus_restore_started_ticks = None;
                    physical_key_preflight_required = true;
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
                if !clock_state.is_paused() {
                    if let Err(error) = suspend_live_input(&mut backend, &mut coordinator) {
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
            } else if !manual_pause
                && clock_state.has_pause_reason("manual")
                && let Err(error) = clock_state.exit_pause("manual", now_ticks)
            {
                force_full_cleanup = true;
                terminal_error = Some(format!("playback clock failure: {error}"));
                break;
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
                let up_completion_error_ticks = match completion_error_ticks(
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
                telemetry.push(|| RtTraceRecord {
                    event_index: first.source_action_index,
                    kind: TRACE_KIND_UP,
                    outcome: trace_outcome_code(release_outcome),
                    polyphony: scan_codes.len() as u8,
                    flags: trace_flags,
                    authored_ticks: scheduled_ticks.as_u64(),
                    effective_deadline_ticks: effective_deadline_ticks.as_u64(),
                    wake_ticks: actual_ticks.as_u64(),
                    send_started_ticks: result.send_started_ticks.map_or(0, |ticks| ticks.as_u64()),
                    send_completed_ticks: result
                        .send_completed_ticks
                        .map_or(0, |ticks| ticks.as_u64()),
                    completion_error_ticks: up_completion_error_ticks,
                    applied_lead_ticks: lead_up_ticks.as_u64() as u32,
                    win32_error: result.last_win32_error.unwrap_or(0),
                });
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
                try_publish_metrics(&local_metrics, metrics, qpc_us_or_terminal!(), true);
                if recovery_required {
                    force_full_cleanup = true;
                    terminal_error = Some(format!(
                        "note-off recovery exhausted after {} retries{}",
                        sky_dispatch_core::coordinator::MAX_RELEASE_RETRIES,
                        result
                            .last_win32_error
                            .map_or(String::new(), |error| format!(" (Win32 error {error})"))
                    ));
                    let recovery_cleanup = backend.release_all_full_instrument();
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

                if batch_kind == ActionKind::Down {
                    // Repeat the foreground comparison at the final boundary
                    // immediately before SendInput. If focus changed after
                    // the outer-loop sample, terminalize this authored batch;
                    // it must not be replayed after the focus grace period.
                    if !focus_matches(config.require_focus, focus_active, target_hwnd) {
                        // The batch was only prepared. Leave the cursor and generation
                        // ledger untouched so the same authored chord can be prepared again
                        // after focus restoration.
                        if let Err(error) = suspend_live_input(&mut backend, &mut coordinator) {
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
                        telemetry.push(|| RtTraceRecord {
                            event_index: batch_source_action_index,
                            kind: TRACE_KIND_DOWN,
                            outcome: trace_outcome_code("blocked_unfocused"),
                            polyphony: scan_batch.len() as u8,
                            flags: TRACE_FLAG_ANOMALY,
                            authored_ticks: authored_batch_scheduled_ticks.as_u64(),
                            effective_deadline_ticks: batch_scheduled_ticks.as_u64(),
                            wake_ticks: effective_now_ticks.as_u64(),
                            send_started_ticks: 0,
                            send_completed_ticks: 0,
                            completion_error_ticks: 0,
                            applied_lead_ticks: lead_down_ticks.as_u64() as u32,
                            win32_error: 0,
                        });
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
                    if physical_key_preflight_required {
                        if let Err(error) = backend.ensure_instrument_keys_physically_up() {
                            force_full_cleanup = true;
                            terminal_error = Some(format!(
                                "instrument key preflight failed; release the 15 instrument keys before playback: {error}"
                            ));
                            break;
                        }
                        physical_key_preflight_required = false;
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
                        let started_ticks = match qpc_clock.now() {
                            Ok(ticks) => ticks,
                            Err(error) => {
                                force_full_cleanup = true;
                                terminal_error =
                                    Some(format!("QPC failure before note-on: {error:?}"));
                                break;
                            }
                        };
                        let started_us = qpc_ticks_to_us_or_terminal!(started_ticks);
                        let actual_ticks = match clock_state.get_elapsed_allow_pre_epoch(
                            started_ticks,
                            allow_pre_epoch_startup_dispatch,
                        ) {
                            Ok(ticks) => ticks,
                            Err(error) => {
                                force_full_cleanup = true;
                                terminal_error = Some(format!("playback clock failure: {error}"));
                                break;
                            }
                        };
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
                        ) = match &result {
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
                                *started_ticks,
                                *completed_us,
                                *completed_ticks,
                                sent.clone(),
                                skipped_duplicates.clone(),
                                *send_attempts,
                                *zero_progress_retries,
                                *retried_after_zero_progress,
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
                                *started_ticks,
                                *completed_us,
                                *completed_ticks,
                                smallvec::SmallVec::<[u16; 15]>::new(),
                                skipped_duplicates.clone(),
                                *send_attempts,
                                *zero_progress_retries,
                                *zero_progress_retries > 0,
                                false,
                                *first_error,
                                *last_error,
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
                                *started_ticks,
                                *completed_us,
                                *completed_ticks,
                                sent.clone(),
                                skipped_duplicates.clone(),
                                *send_attempts,
                                *zero_progress_retries,
                                *zero_progress_retries > 0,
                                true,
                                *first_error,
                                *last_error,
                                false,
                            ),
                        };

                        if matches!(
                            &result,
                            sky_dispatch_win32::input::DownSendOutcome::ZeroProgress { .. }
                                | sky_dispatch_win32::input::DownSendOutcome::IntegrityLost { .. }
                        ) {
                            force_full_cleanup = true;
                            terminal_error = Some(format!(
                                "authored Down send integrity failure at action {}",
                                batch_source_action_index
                            ));
                            break;
                        }

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
                            effective_now_ticks,
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
                        let completion_error_ticks_value = match completion_error_ticks(
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
                                result_completed_us.saturating_sub(started_us),
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
                        telemetry.push(|| RtTraceRecord {
                            event_index: batch_source_action_index,
                            kind: TRACE_KIND_DOWN,
                            outcome: trace_outcome_code(down_outcome),
                            polyphony: batch_intent_count as u8,
                            flags: trace_flags,
                            authored_ticks: authored_batch_scheduled_ticks.as_u64(),
                            effective_deadline_ticks: batch_scheduled_ticks.as_u64(),
                            wake_ticks: actual_ticks.as_u64(),
                            send_started_ticks: result_started_ticks
                                .map_or(0, |ticks| ticks.as_u64()),
                            send_completed_ticks: result_completed_ticks
                                .map_or(0, |ticks| ticks.as_u64()),
                            completion_error_ticks: completion_error_ticks_value,
                            applied_lead_ticks: lead_down_ticks.as_u64() as u32,
                            win32_error: result_last_win32_error.unwrap_or(0),
                        });
                        if config.enable_adaptive_lead && lead_down_saturated {
                            record_lead_saturation(
                                &mut local_metrics.lead_saturation_count_down,
                                &mut local_metrics.positive_residual_at_cap,
                                batch_intent_count,
                                signed_delta(completed_effective, batch_scheduled_us),
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
                            result_completed_us.saturating_sub(started_us),
                            completed_effective,
                            config.input_path_warn_us,
                            &mut send_pure_window,
                            &mut send_pure_over_warn_count,
                            &mut send_pure_warn_started_us,
                            &mut local_metrics.sendinput_path_degraded,
                        );
                        record_input_path_health(
                            bookkeeping_completed_us.saturating_sub(result_completed_us),
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
                        telemetry.push(|| RtTraceRecord {
                            event_index: batch_source_action_index,
                            kind: TRACE_KIND_UP,
                            outcome: trace_outcome_code("suppressed_stale_up"),
                            polyphony: suppressed.len() as u8,
                            flags: TRACE_FLAG_ANOMALY,
                            authored_ticks: authored_batch_scheduled_ticks.as_u64(),
                            effective_deadline_ticks: batch_scheduled_ticks.as_u64(),
                            wake_ticks: effective_now_ticks.as_u64(),
                            send_started_ticks: 0,
                            send_completed_ticks: 0,
                            completion_error_ticks: 0,
                            applied_lead_ticks: lead_up_ticks.as_u64() as u32,
                            win32_error: 0,
                        });
                    }
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
                let target_sample_elapsed_us =
                    qpc_ticks_to_us_or_terminal!(target_sample_elapsed_ticks);
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
                    let wake_elapsed_us = qpc_ticks_to_us_or_terminal!(wake_elapsed_ticks);
                    match wait_result.outcome {
                        WaitOutcome::Deadline => {
                            local_metrics.wait_target_error_us = local_metrics
                                .wait_target_error_us
                                .max(wake_elapsed_us.saturating_sub(target_sample_elapsed_us));
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
                    if config.input_path_warn_us > 0
                        && wake_elapsed_us
                            > target_sample_elapsed_us.saturating_add(config.input_path_warn_us)
                    {
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
            backend.release_all_full_instrument()
        } else {
            backend.release_all()
        };
        if release_state_verified(&backend, &outcome) {
            outcome
        } else {
            // A normal-path release that cannot be verified gets one bounded
            // full-instrument recovery attempt before the result is published.
            backend.release_all_full_instrument()
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

fn lease_bounded_ticks(
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

fn supervisor_lease_expired(
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

fn focus_matches(
    require_focus: bool,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
) -> bool {
    if !require_focus {
        return true;
    }
    let validated_focus_active = focus_active.load(Ordering::Acquire);
    let target = target_hwnd.load(Ordering::Acquire);
    let foreground_matches =
        target == 0 || sky_dispatch_win32::focus::foreground_window_matches(target);
    focus_gate_matches(
        require_focus,
        validated_focus_active,
        target,
        foreground_matches,
    )
}

fn record_lateness(
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

fn cancel_coordinator_or_terminal(
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

fn release_outcome_verified(outcome: &ReleaseAllOutcome) -> bool {
    outcome.released_successfully
        && outcome.stuck_keys.is_empty()
        && !outcome.verification_inconclusive
}

fn release_state_verified(backend: &TrackedKeyState, outcome: &ReleaseAllOutcome) -> bool {
    release_outcome_verified(outcome)
        && backend.active_mask == 0
        && backend.possibly_active_mask == 0
        && backend.failed_release_mask == 0
}

fn clean_completion_proven(
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

fn describe_release_outcome(outcome: &ReleaseAllOutcome) -> String {
    format!(
        "released_successfully={}, stuck_keys={:?}, verification_inconclusive={}",
        outcome.released_successfully, outcome.stuck_keys, outcome.verification_inconclusive
    )
}

fn record_termination_error(
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
fn suspend_live_input(
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
) -> Result<Vec<u64>, String> {
    let initial = backend.release_all();
    let release = if release_state_verified(backend, &initial) {
        initial
    } else {
        let full = backend.release_all_full_instrument();
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

fn release_runtime_outcome(
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
fn update_estimator_after_send(
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
fn update_estimator_after_send_class(
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
    estimator.update_with_class(kind, duration_us, authored_polyphony, latency_class)?;
    if applied_lead_us > 0 {
        estimator.update_completion_error_with_class(kind, completion_error_us, latency_class)?;
    }
    Ok(())
}

fn record_lead_saturation(
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

fn signed_delta(lhs: u64, rhs: u64) -> i64 {
    let delta = lhs as i128 - rhs as i128;
    delta.clamp(i64::MIN as i128, i64::MAX as i128) as i64
}

fn completion_error_ticks(
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

fn signed_ticks_to_us(qpc_clock: QpcClock, delta_ticks: i64) -> Result<i64, String> {
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
fn exact_sender_durations(
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

fn classify_latency_class(
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

fn anchored_dispatch_target_ticks_typed(
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
fn anchored_dispatch_target_ticks(
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
fn deadline_target_ticks(now_ticks: QpcTicks, logical_now_us: u64, deadline_us: u64) -> QpcTicks {
    QpcTicks::from_raw(now_ticks.as_u64().saturating_add(
        qpc_us_to_ticks(deadline_us.saturating_sub(logical_now_us)).expect("test QPC conversion"),
    ))
}

fn publish_wake_error_stats(stats: WakeErrorStats, local_metrics: &mut WorkerMetricsLocal) {
    local_metrics.wake_error_p50_us = stats.p50_us;
    local_metrics.wake_error_p95_us = stats.p95_us;
    local_metrics.wake_error_p99_us = stats.p99_us;
    local_metrics.wake_error_max_us = stats.max_us;
}

fn wait_failure_message(failure: WaitFailure) -> String {
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

fn derive_spin_threshold_us(wake_error_us: u64, spin_floor_us: u64) -> u64 {
    wake_error_us
        .saturating_add(200)
        .clamp(spin_floor_us, 3_000)
}

#[cfg(test)]
fn adjust_spin_threshold(current_us: u64, candidate_us: u64) -> u64 {
    if candidate_us >= current_us {
        candidate_us
    } else {
        current_us.saturating_sub(current_us.saturating_sub(candidate_us).min(50))
    }
}

fn record_input_path_health(
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

fn focus_gate_matches(
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

fn publish_backend_metrics(
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

#[cfg(test)]
mod tests {
    use super::{
        FaultInjectionScript, INPUT_PATH_WINDOW_CAPACITY, InjectedSendOutcome,
        NativeDispatchSession, TelemetryMode, WakeErrorStats, adjust_spin_threshold,
        anchored_dispatch_target_ticks, classify_latency_class, completion_error_ticks,
        deadline_target_ticks, derive_spin_threshold_us, exact_sender_durations,
        focus_gate_matches, record_input_path_health, record_termination_error,
        release_runtime_outcome, supervisor_lease_expired, update_estimator_after_send,
    };
    use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator};
    use sky_dispatch_core::model::{ActionKind, KeyActionInput};
    use sky_dispatch_core::time::TimelineTicks;
    use sky_dispatch_win32::clock::{
        DurationTicks, QpcClock, QpcTicks, qpc_frequency, qpc_ticks_to_us, qpc_us_to_ticks,
    };
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    #[test]
    fn supervisor_lease_treats_future_heartbeat_as_fresh() {
        let heartbeat = AtomicU64::new(1_001);
        assert_eq!(
            supervisor_lease_expired(
                QpcTicks::from_raw(1_000),
                DurationTicks::from_raw(100),
                &heartbeat,
            ),
            Ok(false)
        );
    }

    #[test]
    fn supervisor_lease_treats_equal_heartbeat_as_fresh() {
        let heartbeat = AtomicU64::new(1_000);
        assert_eq!(
            supervisor_lease_expired(
                QpcTicks::from_raw(1_000),
                DurationTicks::from_raw(100),
                &heartbeat,
            ),
            Ok(false)
        );
    }

    #[test]
    fn supervisor_lease_preserves_fresh_boundary_and_expiration() {
        let heartbeat = AtomicU64::new(1_000);
        let timeout = DurationTicks::from_raw(100);
        assert_eq!(
            supervisor_lease_expired(QpcTicks::from_raw(1_050), timeout, &heartbeat),
            Ok(false)
        );
        assert_eq!(
            supervisor_lease_expired(QpcTicks::from_raw(1_100), timeout, &heartbeat),
            Ok(false)
        );
        assert_eq!(
            supervisor_lease_expired(QpcTicks::from_raw(1_101), timeout, &heartbeat),
            Ok(true)
        );
    }

    #[test]
    fn supervisor_lease_disabled_is_never_expired() {
        let heartbeat = AtomicU64::new(1);
        assert_eq!(
            supervisor_lease_expired(QpcTicks::from_raw(2), DurationTicks::ZERO, &heartbeat),
            Ok(false)
        );
    }

    #[test]
    fn recent_latency_ring_is_bounded_and_keeps_latest_values() {
        let mut ring = super::RecentLatencyRing::default();
        for value in 0..40 {
            ring.push(value);
        }
        assert_eq!(ring.to_vec().len(), 32);
        assert_eq!(ring.to_vec().first(), Some(&8));
        assert_eq!(ring.to_vec().last(), Some(&39));
    }

    #[test]
    fn supervisor_lease_concurrent_publication_never_reports_clock_error() {
        let heartbeat = Arc::new(AtomicU64::new(1_000));
        let publisher_heartbeat = Arc::clone(&heartbeat);
        let publisher = std::thread::spawn(move || {
            for index in 0..10_000 {
                publisher_heartbeat.store(
                    if index % 2 == 0 { 1_000 } else { 1_001 },
                    Ordering::Release,
                );
            }
        });

        for _ in 0..10_000 {
            let result = supervisor_lease_expired(
                QpcTicks::from_raw(1_000),
                DurationTicks::from_raw(100),
                &heartbeat,
            );
            assert!(result.is_ok(), "concurrent heartbeat sample: {result:?}");
        }
        publisher.join().expect("heartbeat publisher must finish");
    }

    #[test]
    fn exact_sender_duration_distinguishes_single_call_from_operation() {
        let clock =
            QpcClock::from_frequency_hz(std::num::NonZeroU64::new(qpc_frequency()).unwrap());
        let started = QpcTicks::from_raw(1_000);
        let completed = started
            .checked_add_duration(DurationTicks::from_raw(qpc_us_to_ticks(20).unwrap()))
            .unwrap();

        assert_eq!(
            exact_sender_durations(clock, Some(started), Some(completed), 1, false).unwrap(),
            (Some(20), Some(20))
        );
        assert_eq!(
            exact_sender_durations(clock, Some(started), Some(completed), 2, false).unwrap(),
            (Some(20), None)
        );
        assert_eq!(
            exact_sender_durations(clock, Some(started), Some(completed), 2, true).unwrap(),
            (Some(20), None)
        );
        assert!(exact_sender_durations(clock, None, Some(completed), 1, false).is_err());
    }

    #[test]
    fn completion_error_ticks_preserves_signed_timeline_delta() {
        assert_eq!(
            completion_error_ticks(TimelineTicks::from_raw(120), TimelineTicks::from_raw(100)),
            Ok(20)
        );
        assert_eq!(
            completion_error_ticks(TimelineTicks::from_raw(100), TimelineTicks::from_raw(120)),
            Ok(-20)
        );
    }

    #[test]
    fn completion_error_ticks_rejects_unrepresentable_signed_delta() {
        assert_eq!(
            completion_error_ticks(
                TimelineTicks::from_raw(u64::MAX),
                TimelineTicks::from_raw(0),
            ),
            Err(sky_dispatch_core::time::TimeArithmeticError::Overflow)
        );
        assert_eq!(
            completion_error_ticks(
                TimelineTicks::from_raw(0),
                TimelineTicks::from_raw((i64::MAX as u64) + 1),
            ),
            Ok(i64::MIN)
        );
    }

    #[test]
    fn deadline_mapper_uses_the_current_sample_without_overhead_drift() {
        let now_ticks = QpcTicks::from_raw(10_000_000);
        let logical_now_us = qpc_ticks_to_us(now_ticks).unwrap();
        let target = deadline_target_ticks(now_ticks, logical_now_us, logical_now_us + 1_000);
        assert_eq!(
            target.as_u64() - now_ticks.as_u64(),
            qpc_us_to_ticks(1_000).unwrap()
        );
    }

    #[test]
    fn first_authored_zero_timestamp_can_use_a_future_physical_anchor() {
        let now_ticks = QpcTicks::from_raw(10_000_000);
        let now_qpc_us = qpc_ticks_to_us(now_ticks).unwrap();
        let startup_guard_us = 1_000;
        let lead_us = 500;
        let anchor_us = now_qpc_us
            .saturating_add(startup_guard_us)
            .saturating_add(lead_us);
        let clock =
            QpcClock::from_frequency_hz(std::num::NonZeroU64::new(qpc_frequency()).unwrap());
        let target =
            anchored_dispatch_target_ticks(clock, now_ticks, now_qpc_us, anchor_us, 0, lead_us)
                .unwrap();

        assert_eq!(
            target.as_u64() - now_ticks.as_u64(),
            qpc_us_to_ticks(startup_guard_us).unwrap()
        );
    }

    #[test]
    fn cold_classification_uses_physical_gap_after_logical_pause() {
        let last = QpcTicks::from_raw(qpc_us_to_ticks(100_000).unwrap());
        let threshold =
            DurationTicks::from_raw(qpc_us_to_ticks(super::SEND_COLD_THRESHOLD_US).unwrap());
        let hot_now = last
            .checked_add_duration(threshold)
            .expect("test timestamp");
        let cold_now = QpcTicks::from_raw(hot_now.as_u64() + 1);
        assert_eq!(
            classify_latency_class(Some(last), hot_now, threshold).unwrap(),
            LatencyClass::Hot
        );
        assert_eq!(
            classify_latency_class(Some(last), cold_now, threshold).unwrap(),
            LatencyClass::Cold
        );
        assert_eq!(
            classify_latency_class(None, QpcTicks::ZERO, threshold).unwrap(),
            LatencyClass::Cold
        );
        assert!(classify_latency_class(Some(cold_now), last, threshold).is_err());
    }

    #[test]
    fn focus_gate_requires_both_validation_and_foreground_match() {
        assert!(!focus_gate_matches(true, false, 123, true));
        assert!(!focus_gate_matches(true, true, 123, false));
        assert!(focus_gate_matches(true, true, 123, true));
        assert!(!focus_gate_matches(true, true, 0, false));
        assert!(focus_gate_matches(false, false, 123, false));
    }

    #[test]
    fn input_path_degraded_is_not_key_drop_state() {
        let mut window = VecDeque::with_capacity(64);
        let mut over_warn = 0;
        let mut started = None;
        let mut degraded = false;

        for elapsed_us in (0..=1_010_000).step_by(1_000) {
            record_input_path_health(
                400,
                elapsed_us,
                300,
                &mut window,
                &mut over_warn,
                &mut started,
                &mut degraded,
            );
        }

        assert!(degraded);
    }

    #[test]
    fn input_path_health_window_stays_bounded_and_tracks_latest_samples() {
        let mut window = VecDeque::with_capacity(INPUT_PATH_WINDOW_CAPACITY);
        let initial_capacity = window.capacity();
        let mut over_warn = 0;
        let mut started = None;
        let mut degraded = false;

        for _ in 0..10_000 {
            record_input_path_health(
                400,
                0,
                300,
                &mut window,
                &mut over_warn,
                &mut started,
                &mut degraded,
            );
        }

        assert_eq!(window.len(), INPUT_PATH_WINDOW_CAPACITY);
        assert_eq!(window.capacity(), initial_capacity);
        assert_eq!(over_warn, INPUT_PATH_WINDOW_CAPACITY);

        for _ in 0..INPUT_PATH_WINDOW_CAPACITY {
            record_input_path_health(
                100,
                0,
                300,
                &mut window,
                &mut over_warn,
                &mut started,
                &mut degraded,
            );
        }

        assert_eq!(window.len(), INPUT_PATH_WINDOW_CAPACITY);
        assert_eq!(over_warn, 0);
    }

    #[test]
    fn spin_threshold_rises_immediately_and_decays_with_hysteresis() {
        assert_eq!(adjust_spin_threshold(700, 1_000), 1_000);
        assert_eq!(adjust_spin_threshold(1_000, 700), 950);
        assert_eq!(adjust_spin_threshold(1_000, 100), 950);
    }

    #[test]
    fn single_outlier_in_periodic_reprobe_does_not_raise_threshold_to_cap() {
        let stats = WakeErrorStats {
            p50_us: 300,
            p95_us: 1_500,
            p99_us: 1_500,
            max_us: 1_500,
            robust_us: 300,
        };
        assert_eq!(derive_spin_threshold_us(stats.robust_us, 700), 700);
    }

    #[test]
    fn failed_send_does_not_seed_estimator_or_residual() {
        let mut estimator = SendLatencyEstimator::try_new(0.2, 2_000, 6).unwrap();

        update_estimator_after_send(&mut estimator, ActionKind::Down, 900, 0, 3, 500, 120, false);
        let state = estimator.export_state();
        assert_eq!(state.hist_down[3].hot_pairs, Vec::<[u64; 2]>::new());
        assert_eq!(state.residuals[0].count, 0);

        update_estimator_after_send(&mut estimator, ActionKind::Down, 900, 1, 3, 0, 120, false);
        let state = estimator.export_state();
        assert_eq!(state.hist_down[3].hot_pairs, Vec::<[u64; 2]>::new());
        assert_eq!(state.residuals[0].count, 0);

        update_estimator_after_send(&mut estimator, ActionKind::Down, 900, 1, 3, 500, 120, true);
        let state = estimator.export_state();
        assert_eq!(state.hist_down[3].hot_pairs, vec![[36, 1]]);
        assert_eq!(estimator.export_state().residuals[0].count, 1);
    }

    #[test]
    fn failed_release_outcome_and_completion_metrics_are_distinguishable() {
        assert_eq!(release_runtime_outcome(0, 1, 1, false), "sent");
        assert_eq!(
            release_runtime_outcome(100, 1, 2, false),
            "deferred_partial_note_off"
        );
        assert_eq!(release_runtime_outcome(0, 0, 1, true), "failed_note_off");
        assert_eq!(
            release_runtime_outcome(100, 0, 1, true),
            "deferred_failed_note_off"
        );
    }

    #[test]
    fn termination_error_aggregation_preserves_primary_and_secondary_errors() {
        let mut primary = None;
        let mut secondary = Vec::new();
        record_termination_error(
            &mut primary,
            &mut secondary,
            "coordinator pre-cleanup mismatch".to_string(),
        );
        record_termination_error(
            &mut primary,
            &mut secondary,
            "release verification failed".to_string(),
        );
        record_termination_error(
            &mut primary,
            &mut secondary,
            "release verification failed".to_string(),
        );

        assert_eq!(primary.as_deref(), Some("coordinator pre-cleanup mismatch"));
        assert_eq!(secondary, vec!["release verification failed"]);
    }

    #[test]
    fn persistent_zero_progress_down_aborts_before_the_next_authored_chord() {
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "down-1".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up-1".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 2_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "down-2".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 3_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "up-2".to_string().into(),
            },
        ];
        let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15, 0x16])
            .expect("valid fault-injection schedule");
        let script = FaultInjectionScript {
            entries: vec![
                (
                    0,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
                (
                    1,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
            ],
            ..FaultInjectionScript::default()
        };
        let session = NativeDispatchSession::new(
            schedule,
            0,
            2_000,
            0,
            vec![0x15, 0x16],
            true,
            0,
            0,
            script,
            false,
            100_000,
            150,
            0,
            TelemetryMode::Ring,
            64,
            sky_dispatch_win32::mmcss::PriorityMode::Off,
            true,
            true,
            false,
            700,
            None,
            false,
            300,
            false,
            2_000,
            2_000,
            0,
        )
        .expect("test session admission");

        session.start().expect("worker start");
        assert!(session.join(Duration::from_secs(5)).expect("worker join"));

        let snapshot = session.snapshot();
        assert_eq!(snapshot.status, "error", "zero progress must be terminal");
        let telemetry: serde_json::Value =
            serde_json::from_str(&session.take_telemetry_json().expect("telemetry"))
                .expect("valid telemetry JSON");
        let records = telemetry["records"].as_array().expect("records array");
        assert!(
            !records.iter().any(|record| {
                record["event_index"] == 2 && record["runtime_outcome"] == "sent"
            })
        );
    }
}
