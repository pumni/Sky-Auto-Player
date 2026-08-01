//! End-to-end real-time native dispatch session engine.

use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded};
use parking_lot::Mutex;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::coordinator::RuntimeDispatchCoordinator;
use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator};
use sky_dispatch_core::model::{ActionKind, RuntimeSchedule};
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::{
    QpcTicks, qpc_frequency_checked, qpc_now_ticks, qpc_now_ticks_checked, qpc_now_us,
    qpc_ticks_to_us, qpc_us_to_ticks,
};
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
const SEND_COLD_THRESHOLD_US: u64 = 20_000;
const CORE_WARMUP_SPIN_MAX_US: u64 = 500;
const INPUT_PATH_WINDOW_CAPACITY: usize = 64;
const STRICT_RETRY_LATE_THRESHOLD_US: u64 = 2_000;
const STRICT_SATURATION_ABORT_STREAK: u8 = 3;
const STARTUP_WAKE_GUARD_US: u64 = 1_000;

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
    Prefix {
        inserted: u8,
        latency_ticks: u64,
        win32_error: u32,
    },
    /// Emitter spin-stalls for `duration_ticks` QPC ticks without sending.
    Stall { duration_ticks: u64 },
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
                (
                    2,
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
    /// Up calls are those with odd call indices in alternating Down/Up schedules;
    /// for general use the caller should use a script with explicit indices.
    /// This variant matches all calls whose index >= 1 (approximation for simple
    /// single-chord schedules).
    pub fn persistent_release() -> Self {
        // Inject 128 Up failures — sufficient for any reasonable test schedule.
        let entries = (0..128)
            .filter(|i| i % 2 == 1)
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MockFailureMode {
    None,
    TransientRelease,
    PersistentRelease,
    ZeroProgressDownOnce,
}

impl From<MockFailureMode> for FaultInjectionScript {
    fn from(m: MockFailureMode) -> Self {
        match m {
            MockFailureMode::None => FaultInjectionScript::none(),
            MockFailureMode::TransientRelease => FaultInjectionScript::transient_release(),
            MockFailureMode::PersistentRelease => FaultInjectionScript::persistent_release(),
            MockFailureMode::ZeroProgressDownOnce => {
                FaultInjectionScript::zero_progress_down_once()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChordConflictPolicy {
    /// Preserve the legacy best-effort behavior and send only non-conflicting
    /// keys. This mode is intentionally not fidelity-first.
    DropConflictingKeys,
    /// Drop the complete authored chord when any key is already active.
    DropWholeChord,
    /// Drop the complete chord and terminate playback with a controlled error.
    AbortPlayback,
}

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
    pub generation_count: u64,
    pub generation_status_counts: HashMap<String, u64>,
    pub abort_counts_by_reason: HashMap<String, u64>,
    pub release_outcome: Option<ReleaseAllOutcome>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NativeTelemetryRecord {
    pub event_index: u32,
    pub dispatch_id: u64,
    pub kind: &'static str,
    pub scheduled_us: u64,
    pub actual_us: u64,
    pub dispatch_completed_us: u64,
    pub lateness_us: i64,
    pub visible_lateness_us: i64,
    pub send_duration_us: u64,
    pub send_duration_pure_us: u64,
    pub bookkeeping_us: u64,
    pub dispatch_lateness_us: i64,
    pub scan_codes: SmallVec<[u16; 15]>,
    pub sent_scan_codes: SmallVec<[u16; 15]>,
    pub skipped_scan_codes: SmallVec<[u16; 15]>,
    pub generation_ids: SmallVec<[u64; 15]>,
    pub runtime_outcome: &'static str,
    pub deferred_by_us: u64,
    pub pre_send_spin_us: u64,
    pub idle_gap_us: u64,
    #[serde(skip)]
    pub reason_id: u16,
    pub reason: Option<String>,
    pub applied_lead_us: u64,
    pub first_win32_error: Option<u32>,
    pub last_win32_error: Option<u32>,
    pub send_attempts: u8,
    pub zero_progress_retries: u8,
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
}

impl NativeTelemetrySummary {
    fn observe(&mut self, record: &NativeTelemetryRecord) {
        self.dispatch_count = self.dispatch_count.saturating_add(1);
        match record.kind {
            "down" => self.down_count = self.down_count.saturating_add(1),
            "up" => self.up_count = self.up_count.saturating_add(1),
            _ => {}
        }
        self.requested_key_count = self
            .requested_key_count
            .saturating_add(record.scan_codes.len() as u64);
        self.sent_key_count = self
            .sent_key_count
            .saturating_add(record.sent_scan_codes.len() as u64);
        self.skipped_key_count = self
            .skipped_key_count
            .saturating_add(record.skipped_scan_codes.len() as u64);
        self.max_lateness_us = self.max_lateness_us.max(record.visible_lateness_us);
        self.max_send_duration_us = self.max_send_duration_us.max(record.send_duration_us);
        let lateness_bucket = record
            .visible_lateness_us
            .max(0)
            .unsigned_abs()
            .saturating_div(50)
            .min(15) as usize;
        let send_bucket = record.send_duration_us.saturating_div(50).min(15) as usize;
        self.lateness_histogram_50us[lateness_bucket] =
            self.lateness_histogram_50us[lateness_bucket].saturating_add(1);
        self.send_duration_histogram_50us[send_bucket] =
            self.send_duration_histogram_50us[send_bucket].saturating_add(1);
    }
}

#[derive(Debug, Default, serde::Serialize)]
pub struct NativeTelemetryOutput {
    pub records: VecDeque<NativeTelemetryRecord>,
    pub summary: NativeTelemetrySummary,
    pub attempted: u64,
    pub accepted: u64,
    pub dropped: u64,
    pub truncated: bool,
    #[serde(skip)]
    reason_table: Vec<String>,
}

const MIXED_RELEASE_REASON: &str = "mixed_deferred_release";
const MIXED_RELEASE_REASON_ID: u16 = u16::MAX;

impl NativeTelemetryOutput {
    fn new(mode: TelemetryMode, capacity: usize, reason_table: Vec<String>) -> Self {
        Self {
            records: if matches!(mode, TelemetryMode::Ring | TelemetryMode::FullTrace) {
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
            reason_table,
        }
    }

    fn materialize_reasons(&mut self) -> Result<(), String> {
        for record in &mut self.records {
            if record.reason.is_none() {
                let reason = self
                    .reason_table
                    .get(record.reason_id as usize)
                    .map(String::as_str)
                    .or((record.reason_id == MIXED_RELEASE_REASON_ID)
                        .then_some(MIXED_RELEASE_REASON))
                    .ok_or_else(|| "native telemetry reason id is out of bounds".to_string())?;
                record.reason = Some(reason.to_string());
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryMode {
    Off,
    Summary,
    Ring,
    FullTrace,
}

struct TelemetryCollector {
    mode: TelemetryMode,
    capacity: usize,
    output: NativeTelemetryOutput,
    mixed_release_reason_id: u16,
    next_dispatch_id: u64,
    last_completed_us: Option<u64>,
}

impl TelemetryCollector {
    fn new(mode: TelemetryMode, capacity: usize, reason_table: Vec<String>) -> Self {
        Self {
            mode,
            capacity,
            output: NativeTelemetryOutput::new(mode, capacity, reason_table),
            mixed_release_reason_id: MIXED_RELEASE_REASON_ID,
            next_dispatch_id: 0,
            last_completed_us: None,
        }
    }

    fn push<F>(&mut self, build: F)
    where
        F: FnOnce() -> NativeTelemetryRecord,
    {
        self.output.attempted = self.output.attempted.saturating_add(1);
        if self.mode == TelemetryMode::Off {
            return;
        }

        let mut record = build();
        self.output.summary.observe(&record);
        record.dispatch_id = self.next_dispatch_id;
        record.idle_gap_us = self
            .last_completed_us
            .map_or(0, |previous| record.actual_us.saturating_sub(previous));
        self.last_completed_us = Some(record.dispatch_completed_us);

        match self.mode {
            TelemetryMode::Off => unreachable!(),
            TelemetryMode::Summary => {}
            TelemetryMode::Ring => {
                if self.output.records.len() == self.capacity {
                    self.output.records.pop_front();
                }
                self.output.records.push_back(record);
                self.output.accepted = self.output.accepted.saturating_add(1);
            }
            TelemetryMode::FullTrace => {
                if self.output.records.len() < self.capacity {
                    self.output.records.push_back(record);
                    self.output.accepted = self.output.accepted.saturating_add(1);
                } else {
                    self.output.dropped = self.output.dropped.saturating_add(1);
                    self.output.truncated = true;
                }
            }
        }

        self.next_dispatch_id = self.next_dispatch_id.saturating_add(1);
    }
}

#[derive(Clone, Copy, Debug)]
enum WorkerCommand {
    Skip,
    Quit,
    PanicRelease,
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
    if (force || now_us.saturating_sub(last) >= 20_000)
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
    late_pulse_drop_threshold_us: Option<u64>,
    chord_conflict_policy: ChordConflictPolicy,
    telemetry_mode: TelemetryMode,
    telemetry_capacity: usize,
    priority_mode: PriorityMode,
    enable_waitable_timer: bool,
    enable_event_wait: bool,
    enable_adaptive_spin: bool,
    enable_spin_reprobe: bool,
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
    command_tx: Sender<WorkerCommand>,
    command_rx: Receiver<WorkerCommand>,
    latency_tx: Sender<i64>,
    latency_rx: Receiver<i64>,
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
    supervisor_heartbeat_us: Arc<AtomicU64>,
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
        late_pulse_drop_threshold_us: Option<u64>,
        chord_conflict_policy: ChordConflictPolicy,
        telemetry_mode: TelemetryMode,
        telemetry_capacity: usize,
        priority_mode: PriorityMode,
        enable_waitable_timer: bool,
        enable_event_wait: bool,
        enable_adaptive_spin: bool,
        enable_spin_reprobe: bool,
        spin_floor_us: u64,
        estimator_state_json: Option<String>,
        enable_adaptive_lead: bool,
        input_path_warn_us: u64,
        strict_timing: bool,
        strict_down_completion_late_us: u64,
        strict_up_completion_late_us: u64,
        supervisor_lease_timeout_us: u64,
    ) -> Result<Self, String> {
        let interrupt = OwnedEvent::new_auto_reset()
            .ok_or_else(|| "failed to create command event".to_string())?;
        let total_us = schedule
            .batches
            .last()
            .map_or(0, |batch| batch.scheduled_us);
        let generation_count = schedule.generation_count;
        let (command_tx, command_rx) = bounded(32);
        let (latency_tx, latency_rx) = bounded(512);
        if let Some(raw) = &estimator_state_json {
            let mut validator =
                SendLatencyEstimator::new(0.2, max_lead_us, allowed_scan_codes.len());
            validator.import_state(raw)?;
        }
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
                late_pulse_drop_threshold_us,
                chord_conflict_policy,
                telemetry_mode,
                telemetry_capacity,
                priority_mode,
                enable_waitable_timer,
                enable_event_wait,
                enable_adaptive_spin,
                enable_spin_reprobe,
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
            command_tx,
            command_rx,
            latency_tx,
            latency_rx,
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
            supervisor_heartbeat_us: Arc::new(AtomicU64::new(qpc_now_us())),
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
        let Some(config) = self.config.lock().take() else {
            self.lifecycle.store(LIFECYCLE_POISONED, Ordering::Release);
            return Err("session configuration is no longer available".to_string());
        };

        let rx = self.command_rx.clone();
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
        let supervisor_heartbeat_us = Arc::clone(&self.supervisor_heartbeat_us);
        let latency_tx = self.latency_tx.clone();
        supervisor_heartbeat_us.store(qpc_now_us(), Ordering::Release);

        let spawn_result = std::thread::Builder::new()
            .name("sky-native-dispatch".to_string())
            .spawn(move || {
                let worker_result = catch_unwind(AssertUnwindSafe(|| {
                    run_worker(
                        config,
                        &rx,
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
                        &latency_tx,
                        &supervisor_heartbeat_us,
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

    fn send_command(&self, command: WorkerCommand) -> Result<(), String> {
        if !matches!(
            self.lifecycle.load(Ordering::Acquire),
            LIFECYCLE_RUNNING | LIFECYCLE_POISONED
        ) {
            return Err("session commands require a running worker".to_string());
        }
        let _ = self.command_tx.try_send(command);
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
        self.send_command(WorkerCommand::Skip)
    }

    pub fn quit(&self) -> Result<(), String> {
        if !matches!(
            self.lifecycle.load(Ordering::Acquire),
            LIFECYCLE_RUNNING | LIFECYCLE_POISONED
        ) {
            return Err("session commands require a running worker".to_string());
        }
        self.quit_requested.store(true, Ordering::Release);
        self.send_command(WorkerCommand::Quit)
    }

    pub fn panic_release(&self) -> Result<(), String> {
        if !matches!(
            self.lifecycle.load(Ordering::Acquire),
            LIFECYCLE_RUNNING | LIFECYCLE_POISONED
        ) {
            return Err("session commands require a running worker".to_string());
        }
        self.panic_requested.store(true, Ordering::Release);
        self.send_command(WorkerCommand::PanicRelease)
    }

    pub fn update_focus(&self, active: bool) {
        if self.focus_active.swap(active, Ordering::AcqRel) != active {
            let _ = self.interrupt.signal();
        }
    }

    pub fn heartbeat(&self) {
        if self.lifecycle.load(Ordering::Acquire) == LIFECYCLE_RUNNING {
            self.supervisor_heartbeat_us
                .store(qpc_now_us(), Ordering::Release);
        }
    }

    pub fn set_target_hwnd(&self, hwnd: isize) {
        if self.target_hwnd.swap(hwnd, Ordering::AcqRel) != hwnd {
            let _ = self.interrupt.signal();
        }
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        let lifecycle = self.lifecycle.load(Ordering::Acquire);
        let paused = self.metrics.is_paused.load(Ordering::Relaxed);
        let terminal_error = self.terminal_outcome.load(Ordering::Acquire) == OUTCOME_ERROR;
        let status = match lifecycle {
            LIFECYCLE_NEW => "ready",
            LIFECYCLE_RUNNING if paused => "paused",
            LIFECYCLE_RUNNING => "playing",
            LIFECYCLE_FINISHED if terminal_error => "error",
            LIFECYCLE_FINISHED => "finished",
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
            recent_latencies_us: self.latency_rx.try_iter().collect(),
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
        let mut output = self
            .telemetry_output
            .lock()
            .take()
            .ok_or_else(|| "telemetry has already been taken".to_string())?;
        output.materialize_reasons()?;
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

#[allow(clippy::too_many_arguments)]
fn run_worker(
    config: WorkerConfig,
    rx: &Receiver<WorkerCommand>,
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
    latency_tx: &Sender<i64>,
    supervisor_heartbeat_us: &AtomicU64,
) -> u8 {
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
                    PlatformSendResult {
                        requested: codes.len() as u32,
                        inserted: codes.len() as u32,
                        completed_us: qpc_now_us(),
                        win32_error: 0,
                    }
                }
                Some(InjectedSendOutcome::Full { latency_ticks }) => {
                    // Spin-wait for the specified QPC tick count.
                    let deadline = qpc_now_ticks().saturating_add(DurationTicks(*latency_ticks));
                    while qpc_now_ticks() < deadline {
                        std::hint::spin_loop();
                    }
                    PlatformSendResult {
                        requested: codes.len() as u32,
                        inserted: codes.len() as u32,
                        completed_us: qpc_now_us(),
                        win32_error: 0,
                    }
                }
                Some(InjectedSendOutcome::Zero {
                    latency_ticks,
                    win32_error,
                }) => {
                    let deadline = qpc_now_ticks().saturating_add(DurationTicks(*latency_ticks));
                    while qpc_now_ticks() < deadline {
                        std::hint::spin_loop();
                    }
                    PlatformSendResult {
                        requested: codes.len() as u32,
                        inserted: 0,
                        completed_us: qpc_now_us(),
                        win32_error: *win32_error,
                    }
                }
                Some(InjectedSendOutcome::Prefix {
                    inserted,
                    latency_ticks,
                    win32_error,
                }) => {
                    let deadline = qpc_now_ticks().saturating_add(DurationTicks(*latency_ticks));
                    while qpc_now_ticks() < deadline {
                        std::hint::spin_loop();
                    }
                    let inserted = (*inserted as u32).min(codes.len() as u32);
                    PlatformSendResult {
                        requested: codes.len() as u32,
                        inserted,
                        completed_us: qpc_now_us(),
                        win32_error: *win32_error,
                    }
                }
                Some(InjectedSendOutcome::Stall { duration_ticks }) => {
                    // Spin-stall: hold the emitter without sending any key.
                    // This simulates a scheduler stall or OS freeze without
                    // actually blocking the thread (consistent with RT discipline).
                    let deadline = qpc_now_ticks().saturating_add(DurationTicks(*duration_ticks));
                    while qpc_now_ticks() < deadline {
                        std::hint::spin_loop();
                    }
                    PlatformSendResult {
                        requested: codes.len() as u32,
                        inserted: 0,
                        completed_us: qpc_now_us(),
                        win32_error: 0,
                    }
                }
            }
        })
    } else {
        TrackedKeyState::new()
    };
    let mut local_metrics = WorkerMetricsLocal::default();
    let power_guard = PowerThrottlingGuard::disable_current_thread();
    local_metrics.power_throttling_disabled = power_guard.is_active();
    let priority_guard = MmcssGuard::acquire(config.priority_mode);
    *priority_acquired.lock() = priority_guard.acquired().to_string();
    let waiter = HybridWaiter::with_options(config.enable_waitable_timer, config.enable_event_wait);
    *metrics.wait_strategy_acquired.lock() = waiter.mode().to_string();
    let mut estimator = SendLatencyEstimator::new(0.2, config.max_lead_us, config.allowed_count);
    if let Some(raw) = &config.estimator_state_json {
        estimator
            .import_state(raw)
            .expect("estimator state was validated during prepare");
    }
    let telemetry_reason_table = config.schedule.reason_table.clone();
    let mut coordinator =
        RuntimeDispatchCoordinator::new(config.schedule, config.min_hold_us, 0, |us| {
            TimelineTicks(qpc_us_to_ticks(us))
        });
    local_metrics.total_us = coordinator.effective_total_us();
    let mut telemetry = TelemetryCollector::new(
        config.telemetry_mode,
        config.telemetry_capacity,
        telemetry_reason_table,
    );
    let mut abort_counts: HashMap<&'static str, u64> = HashMap::with_capacity(6);
    let mut effective_spin_threshold_us = config.spin_threshold_us;
    let _ = interrupt.try_take();
    if config.enable_adaptive_spin
        && let Some(stats) = waiter.probe_wake_error_stats(interrupt, 30)
    {
        publish_wake_error_stats(stats, &mut local_metrics);
        effective_spin_threshold_us = derive_spin_threshold_us(stats.p95_us, config.spin_floor_us);
    }
    local_metrics.effective_spin_threshold_us = effective_spin_threshold_us;
    let mut last_spin_probe_us = qpc_now_us();
    // Cold/hot classification must use physical QPC time.  The authored
    // playback clock deliberately freezes during pause/focus recovery, so a
    // logical gap cannot tell us whether the CPU/input path has gone cold.
    let mut last_send_qpc_us: Option<u64> = None;
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
    let startup_authored_us = coordinator
        .schedule
        .batches
        .first()
        .map(|batch| batch.scheduled_us);
    let startup_guard_us = STARTUP_WAKE_GUARD_US
        .saturating_add(effective_spin_threshold_us)
        .saturating_add(config.core_warmup_budget_us.min(CORE_WARMUP_SPIN_MAX_US));
    let startup_anchor_us = qpc_now_us()
        .saturating_add(startup_guard_us)
        .saturating_add(startup_lead_us);
    let mut clock_state = PlaybackClockState::new(
        QpcTicks(qpc_us_to_ticks(startup_anchor_us)),
        sky_dispatch_core::time::DurationTicks(0),
    );
    let mut startup_gate = startup_authored_us.map(|scheduled_us| (scheduled_us, startup_lead_us));
    let mut focus_restore_started_us: Option<u64> = None;
    let start_wall_time_us = qpc_now_us();
    let start_thread_cpu_us = current_thread_cpu_time_us();
    let start_process_cpu_us = current_process_cpu_time_us();
    let mut force_full_cleanup = false;
    let mut terminal_error: Option<String> = None;
    let mut last_published_error: Option<String> = None;

    let qpc_admission_error = qpc_frequency_checked()
        .err()
        .map(|error| format!("QPC frequency unavailable: {error:?}"))
        .or_else(|| {
            qpc_now_ticks_checked()
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

    let worker_result = catch_unwind(AssertUnwindSafe(|| {
        if terminal_error.is_some() {
            return;
        }
        while !coordinator.is_finished() {
            let loop_start_us = qpc_now_us();
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
            if supervisor_lease_expired(config.supervisor_lease_timeout_us, supervisor_heartbeat_us)
            {
                force_full_cleanup = true;
                terminal_error = Some("supervisor_lease_expired".to_string());
                break;
            }
            drain_commands(rx, quit_requested, skip_requested, panic_requested);
            if quit_requested.load(Ordering::Acquire) || skip_requested.load(Ordering::Acquire) {
                break;
            }
            if panic_requested.swap(false, Ordering::AcqRel) {
                let _ = backend.release_all_full_instrument();
                coordinator.cancel_all();
                *abort_counts.entry("panic").or_insert(0) += 1;
                publish_backend_metrics(
                    &backend,
                    &mut local_metrics,
                    metrics,
                    &mut last_published_error,
                );
                try_publish_metrics(&local_metrics, metrics, qpc_now_us(), true);
            }

            let mut now_us = qpc_now_us();
            let focus_ok = focus_matches(config.require_focus, focus_active, target_hwnd);
            let manual_pause = desired_pause.load(Ordering::Acquire);

            if !focus_ok {
                focus_restore_started_us = None;
                if !clock_state.has_pause_reason("focus") {
                    let _ = backend.release_all();
                    coordinator.cancel_all();
                    *abort_counts.entry("focus_lost").or_insert(0) += 1;
                    clock_state.enter_pause("focus", QpcTicks(qpc_us_to_ticks(now_us)));
                    publish_backend_metrics(
                        &backend,
                        &mut local_metrics,
                        metrics,
                        &mut last_published_error,
                    );
                    try_publish_metrics(&local_metrics, metrics, qpc_now_us(), true);
                }
            } else if clock_state.has_pause_reason("focus") {
                let restored_at = *focus_restore_started_us.get_or_insert(now_us);
                if now_us.saturating_sub(restored_at) >= config.focus_restore_grace_us {
                    // Second idempotent release happens while the restored
                    // target is foreground, before playback can resume.
                    let _ = backend.release_all();
                    coordinator.cancel_all();
                    // Cleanup can include bounded backend retries. Re-sample
                    // QPC after it completes so that the cleanup interval is
                    // included in the focus pause rather than lost from the
                    // playback clock.
                    let resumed_us = qpc_now_us();
                    let _ = clock_state.exit_pause("focus", QpcTicks(qpc_us_to_ticks(resumed_us)));
                    focus_restore_started_us = None;
                    publish_backend_metrics(
                        &backend,
                        &mut local_metrics,
                        metrics,
                        &mut last_published_error,
                    );
                    try_publish_metrics(&local_metrics, metrics, qpc_now_us(), true);
                }
            }

            if manual_pause && !clock_state.has_pause_reason("manual") {
                if !clock_state.is_paused() {
                    let _ = backend.release_all();
                    coordinator.cancel_all();
                    *abort_counts.entry("manual_pause").or_insert(0) += 1;
                    publish_backend_metrics(
                        &backend,
                        &mut local_metrics,
                        metrics,
                        &mut last_published_error,
                    );
                    try_publish_metrics(&local_metrics, metrics, qpc_now_us(), true);
                }
                clock_state.enter_pause("manual", QpcTicks(qpc_us_to_ticks(now_us)));
            } else if !manual_pause && clock_state.has_pause_reason("manual") {
                let _ = clock_state.exit_pause("manual", QpcTicks(qpc_us_to_ticks(now_us)));
            }

            let paused = clock_state.is_paused();
            metrics.is_paused.store(paused, Ordering::Relaxed);
            if paused {
                let pause_target_us = lease_bounded_us(
                    now_us.saturating_add(PAUSED_POLL_US),
                    config.supervisor_lease_timeout_us,
                    supervisor_heartbeat_us,
                );
                if let WaitOutcome::Failed(failure) =
                    waiter.wait_until_us(pause_target_us, 0, interrupt)
                {
                    local_metrics.wait_path_degraded = true;
                    if config.strict_timing {
                        force_full_cleanup = true;
                        terminal_error = Some(wait_failure_message(failure));
                        break;
                    }
                    std::thread::sleep(Duration::from_micros(500));
                }
                continue;
            }

            if let Some((startup_scheduled_us, startup_lead_us)) = startup_gate {
                let target_sample_ticks = qpc_now_ticks();
                let target_sample_qpc_us = qpc_ticks_to_us(target_sample_ticks);
                let target_qpc = anchored_dispatch_target_ticks(
                    target_sample_ticks,
                    target_sample_qpc_us,
                    qpc_ticks_to_us(clock_state.epoch),
                    startup_scheduled_us,
                    startup_lead_us,
                );
                if target_sample_ticks < target_qpc {
                    let bounded_target_qpc = lease_bounded_ticks(
                        target_qpc,
                        target_sample_ticks,
                        config.supervisor_lease_timeout_us,
                        supervisor_heartbeat_us,
                    );
                    let wait_result = waiter.wait_until_ticks_with_metrics(
                        bounded_target_qpc,
                        effective_spin_threshold_us,
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
                            if config.strict_timing {
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
                now_us = qpc_now_us();
            }

            let effective_now_ticks =
                TimelineTicks(clock_state.get_elapsed(QpcTicks(qpc_us_to_ticks(now_us))).0);
            let effective_now_us = qpc_ticks_to_us(QpcTicks(effective_now_ticks.0));
            local_metrics.elapsed_us = effective_now_us;
            let latency_class = classify_latency_class(last_send_qpc_us, now_us);

            let pending_plan = coordinator.plan_pending_dispatch(|polyphony| {
                if config.dispatch_lead_us > 0 {
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
                }
            });
            let lead_up = pending_plan.as_ref().map_or(0, |plan| plan.lead_us);
            let due_pending = pending_plan.as_ref().map_or_else(SmallVec::new, |plan| {
                coordinator.pop_due_pending_with_plan(effective_now_us, plan)
            });
            if !due_pending.is_empty() {
                let scan_codes: SmallVec<[u16; 15]> =
                    due_pending.iter().map(|p| p.scan_code).collect();
                let started_us = qpc_now_us();
                let actual_us = qpc_ticks_to_us(QpcTicks(
                    clock_state
                        .get_elapsed(QpcTicks(qpc_us_to_ticks(started_us)))
                        .0,
                ));
                let result = backend.key_up(&scan_codes);
                let completed_effective_ticks = TimelineTicks(
                    clock_state
                        .get_elapsed(QpcTicks(qpc_us_to_ticks(result.send_completed_us)))
                        .0,
                );
                let completed_effective = qpc_ticks_to_us(QpcTicks(completed_effective_ticks.0));
                last_send_qpc_us = Some(result.send_completed_us);
                let recovery_required = coordinator.requeue_failed_releases(
                    &due_pending,
                    &result.sent,
                    &result.skipped_duplicates,
                    actual_us,
                    completed_effective,
                    result.last_win32_error,
                );
                coordinator.complete_releases(
                    &due_pending,
                    &result.sent,
                    &result.skipped_duplicates,
                );
                // A successful retry closes the recovery pause. The
                // coordinator advances one immutable timeline offset, so
                // overdue work cannot burst immediately after recovery.
                if !recovery_required
                    && let Some(recovery_pause_us) =
                        coordinator.finish_release_recovery(completed_effective)
                {
                    local_metrics.total_us =
                        local_metrics.total_us.saturating_add(recovery_pause_us);
                }
                let bookkeeping_completed_us = qpc_now_us();
                let first = due_pending
                    .iter()
                    .min_by_key(|pending| {
                        (
                            pending.get_effective_release_us(lead_up),
                            pending.source_action_index,
                            pending.scan_code,
                        )
                    })
                    .expect("non-empty pending release batch");
                let scheduled_us = due_pending
                    .iter()
                    .map(|pending| pending.scheduled_release_us)
                    .min()
                    .unwrap_or(first.scheduled_release_us);
                let deferred_by_us = due_pending
                    .iter()
                    .map(|pending| {
                        pending
                            .release_not_before_us
                            .max(pending.next_retry_us)
                            .saturating_sub(pending.scheduled_release_us)
                    })
                    .max()
                    .unwrap_or(0);
                let mixed_source = due_pending.iter().any(|pending| {
                    pending.source_action_index != first.source_action_index
                        || pending.reason_id != first.reason_id
                });
                let up_completion_error_us = signed_delta(completed_effective, scheduled_us);
                let clean_up_sample = result.success
                    && result.sent.len() == scan_codes.len()
                    && result.skipped_duplicates.is_empty()
                    && result.send_attempts == 1
                    && deferred_by_us == 0
                    && !mixed_source;
                let strict_up_completion_late = config.strict_timing
                    && clean_up_sample
                    && up_completion_error_us
                        > config.strict_up_completion_late_us.min(i64::MAX as u64) as i64;
                let up_saturated_positive = pending_plan
                    .as_ref()
                    .is_some_and(|plan| plan.lead_saturated)
                    && up_completion_error_us > 0;
                up_saturation_positive_streak = if up_saturated_positive {
                    up_saturation_positive_streak.saturating_add(1)
                } else {
                    0
                };
                let saturation_abort = config.strict_timing
                    && up_saturation_positive_streak >= STRICT_SATURATION_ABORT_STREAK;
                if config.enable_adaptive_lead {
                    update_estimator_after_send_class(
                        &mut estimator,
                        ActionKind::Up,
                        result.send_completed_us.saturating_sub(started_us),
                        result.sent.len(),
                        scan_codes.len(),
                        lead_up,
                        up_completion_error_us,
                        clean_up_sample,
                        latency_class,
                    );
                }
                let reason_id = if mixed_source {
                    telemetry.mixed_release_reason_id
                } else {
                    first.reason_id
                };
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
                telemetry.push(|| NativeTelemetryRecord {
                    event_index: first.source_action_index,
                    dispatch_id: 0,
                    kind: "up",
                    scheduled_us,
                    actual_us,
                    dispatch_completed_us: completed_effective,
                    lateness_us: signed_delta(actual_us, scheduled_us),
                    visible_lateness_us: signed_delta(completed_effective, scheduled_us),
                    send_duration_us: bookkeeping_completed_us.saturating_sub(started_us),
                    send_duration_pure_us: result.send_completed_us.saturating_sub(started_us),
                    bookkeeping_us: bookkeeping_completed_us
                        .saturating_sub(result.send_completed_us),
                    dispatch_lateness_us: signed_delta(actual_us, scheduled_us)
                        .saturating_add(result.send_completed_us.saturating_sub(started_us) as i64),
                    scan_codes: scan_codes.clone(),
                    sent_scan_codes: result.sent.clone(),
                    skipped_scan_codes: result.skipped_duplicates.clone(),
                    generation_ids: due_pending
                        .iter()
                        .map(|pending| pending.generation_id)
                        .collect(),
                    runtime_outcome: release_outcome,
                    deferred_by_us,
                    pre_send_spin_us: pending_pre_send_spin_us,
                    idle_gap_us: 0,
                    reason_id,
                    reason: None,
                    applied_lead_us: lead_up,
                    first_win32_error: result.first_win32_error,
                    last_win32_error: result.last_win32_error,
                    send_attempts: result.send_attempts,
                    zero_progress_retries: result.zero_progress_retries,
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
                    latency_tx,
                );
                publish_backend_metrics(
                    &backend,
                    &mut local_metrics,
                    metrics,
                    &mut last_published_error,
                );
                try_publish_metrics(&local_metrics, metrics, qpc_now_us(), true);
                if recovery_required {
                    force_full_cleanup = true;
                    terminal_error = Some(format!(
                        "note-off recovery exhausted after {} retries{}",
                        sky_dispatch_core::coordinator::MAX_RELEASE_RETRIES,
                        result
                            .last_win32_error
                            .map_or(String::new(), |error| format!(" (Win32 error {error})"))
                    ));
                    let _ = backend.release_all_full_instrument();
                    coordinator.cancel_all();
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
            if let Some((batch_index, _lead)) =
                coordinator.pop_next_due_authored_index(effective_now_us, lead_down)
            {
                // --- Borrow scope: extract all scalar and stack data before any &mut call ---
                // `batch_view` borrows from `coordinator.schedule`. We must not call any
                // `&mut coordinator` method until this scope ends. Pull every field we need
                // into Copy / stack-owned values here.
                let batch_view = coordinator
                    .schedule
                    .view_batch(batch_index, coordinator.recovery_offset_us());
                let batch_kind = batch_view.kind();
                let batch_source_action_index = batch_view.source_action_index();
                let batch_scheduled_us = batch_view.scheduled_us;
                let batch_reason_id = batch_view.reason_id();
                let batch_intent_count = batch_view.intents.len();
                // Conflict check: O(N) bitwise, no allocation.
                let conflict_mask = coordinator.check_down_conflicts_compact(batch_view.intents);
                let has_conflicts = conflict_mask != 0;
                // Scan codes for SendInput: stack-only buffer.
                let scan_batch = batch_view.scan_code_batch_excluding_mask(conflict_mask);
                // Collect conflict scan codes into stack buffer for telemetry.
                let conflict_scan_batch = if has_conflicts {
                    batch_view.scan_code_batch_excluding_mask(!conflict_mask)
                } else {
                    sky_dispatch_core::model::ScanCodeBatch::new_empty()
                };
                // Materialise full generation ids for telemetry (only if enabled).
                // This allocation is guarded by the telemetry mode so production
                // fast path avoids SmallVec creation.
                let materialized_for_telemetry = if telemetry.mode != TelemetryMode::Off {
                    Some(batch_view.materialize())
                } else {
                    None
                };
                // --- End of borrow scope: all data is now in stack-local copies ---

                if batch_kind == ActionKind::Down {
                    // Repeat the foreground comparison at the final boundary
                    // immediately before SendInput. If focus changed after
                    // the outer-loop sample, terminalize this authored batch;
                    // it must not be replayed after the focus grace period.
                    if !focus_matches(config.require_focus, focus_active, target_hwnd) {
                        // Terminalize ALL intents (both conflicting and non-conflicting).
                        // We use the compact path: terminalize non-conflicting as DroppedExpired,
                        // conflicting were already accounted by check_down_conflicts_compact.
                        if !has_conflicts {
                            // Non-conflicting slots: terminate as expired.
                            // Fallback to the owned path for the cleanup branch — not hot.
                            let owned = coordinator.schedule.materialize_batch(
                                coordinator.cursor - 1,
                                coordinator.recovery_offset_us(),
                            );
                            coordinator.drop_expired_downs(&owned.intents);
                        } else {
                            let owned = coordinator.schedule.materialize_batch(
                                coordinator.cursor - 1,
                                coordinator.recovery_offset_us(),
                            );
                            coordinator.drop_expired_downs(&owned.intents);
                        }
                        let _ = backend.release_all();
                        coordinator.cancel_all();
                        clock_state.enter_pause("focus", QpcTicks(qpc_us_to_ticks(now_us)));
                        focus_restore_started_us = None;
                        telemetry.push(|| {
                            let scan_codes: SmallVec<[u16; 15]> =
                                scan_batch.as_slice().iter().copied().collect();
                            let generation_ids: SmallVec<[u64; 15]> = materialized_for_telemetry
                                .as_ref()
                                .map(|b| b.intents.iter().filter_map(|i| i.generation_id).collect())
                                .unwrap_or_default();
                            NativeTelemetryRecord {
                                event_index: batch_source_action_index,
                                dispatch_id: 0,
                                kind: "down",
                                scheduled_us: batch_scheduled_us,
                                actual_us: effective_now_us,
                                dispatch_completed_us: effective_now_us,
                                lateness_us: signed_delta(effective_now_us, batch_scheduled_us),
                                visible_lateness_us: signed_delta(
                                    effective_now_us,
                                    batch_scheduled_us,
                                ),
                                send_duration_us: 0,
                                send_duration_pure_us: 0,
                                bookkeeping_us: 0,
                                dispatch_lateness_us: signed_delta(
                                    effective_now_us,
                                    batch_scheduled_us,
                                ),
                                scan_codes,
                                sent_scan_codes: SmallVec::new(),
                                skipped_scan_codes: SmallVec::new(),
                                generation_ids,
                                runtime_outcome: "blocked_unfocused",
                                deferred_by_us: 0,
                                pre_send_spin_us: 0,
                                idle_gap_us: 0,
                                reason_id: batch_reason_id,
                                reason: None,
                                applied_lead_us: lead_down,
                                first_win32_error: None,
                                last_win32_error: None,
                                send_attempts: 0,
                                zero_progress_retries: 0,
                            }
                        });
                        publish_backend_metrics(
                            &backend,
                            &mut local_metrics,
                            metrics,
                            &mut last_published_error,
                        );
                        try_publish_metrics(&local_metrics, metrics, qpc_now_us(), true);
                        continue;
                    }
                    if config
                        .late_pulse_drop_threshold_us
                        .is_some_and(|threshold| {
                            threshold == 0
                                || (effective_now_us >= batch_scheduled_us
                                    && effective_now_us.saturating_sub(batch_scheduled_us)
                                        > threshold)
                        })
                    {
                        // Expired drop — materialize only for the coordinator call.
                        let owned = coordinator.schedule.materialize_batch(
                            coordinator.cursor - 1,
                            coordinator.recovery_offset_us(),
                        );
                        coordinator.drop_expired_downs(&owned.intents);
                        telemetry.push(|| {
                            let scan_codes: SmallVec<[u16; 15]> =
                                scan_batch.as_slice().iter().copied().collect();
                            let generation_ids: SmallVec<[u64; 15]> = materialized_for_telemetry
                                .as_ref()
                                .map(|b| b.intents.iter().filter_map(|i| i.generation_id).collect())
                                .unwrap_or_default();
                            NativeTelemetryRecord {
                                event_index: batch_source_action_index,
                                dispatch_id: 0,
                                kind: "down",
                                scheduled_us: batch_scheduled_us,
                                actual_us: effective_now_us,
                                dispatch_completed_us: effective_now_us,
                                lateness_us: signed_delta(effective_now_us, batch_scheduled_us),
                                visible_lateness_us: signed_delta(
                                    effective_now_us,
                                    batch_scheduled_us,
                                ),
                                send_duration_us: 0,
                                send_duration_pure_us: 0,
                                bookkeeping_us: 0,
                                dispatch_lateness_us: signed_delta(
                                    effective_now_us,
                                    batch_scheduled_us,
                                ),
                                scan_codes,
                                sent_scan_codes: SmallVec::new(),
                                skipped_scan_codes: SmallVec::new(),
                                generation_ids,
                                runtime_outcome: "dropped_expired",
                                deferred_by_us: 0,
                                pre_send_spin_us: 0,
                                idle_gap_us: 0,
                                reason_id: batch_reason_id,
                                reason: None,
                                applied_lead_us: lead_down,
                                first_win32_error: None,
                                last_win32_error: None,
                                send_attempts: 0,
                                zero_progress_retries: 0,
                            }
                        });
                        continue;
                    }

                    // Conflict handling. Conflict slots were checked with the compact mask;
                    // now update counters and decide policy.
                    let playable_count = scan_batch.len();
                    let conflict_count = conflict_scan_batch.len();
                    if has_conflicts {
                        local_metrics.authored_conflict_events =
                            local_metrics.authored_conflict_events.saturating_add(1);
                        local_metrics.authored_keys_rejected =
                            local_metrics.authored_keys_rejected.saturating_add(
                                if matches!(
                                    config.chord_conflict_policy,
                                    ChordConflictPolicy::DropWholeChord
                                        | ChordConflictPolicy::AbortPlayback
                                ) {
                                    batch_intent_count as u64
                                } else {
                                    conflict_count as u64
                                },
                            );
                        // Terminalize conflicted slots (counter update only, no mask change).
                        let owned = coordinator.schedule.materialize_batch(
                            coordinator.cursor - 1,
                            coordinator.recovery_offset_us(),
                        );
                        // Use the existing API which operates on RuntimeKeyIntent.
                        let conflicts_intents: SmallVec<[_; 15]> = owned
                            .intents
                            .iter()
                            .filter(|i| {
                                conflict_mask
                                    & sky_dispatch_core::coordinator::RuntimeDispatchCoordinator::bit_for_slot_pub(i.key_slot)
                                    != 0
                            })
                            .cloned()
                            .collect();
                        telemetry.push(|| {
                            let scan_codes: SmallVec<[u16; 15]> =
                                conflict_scan_batch.as_slice().iter().copied().collect();
                            let generation_ids: SmallVec<[u64; 15]> = conflicts_intents
                                .iter()
                                .filter_map(|i| i.generation_id)
                                .collect();
                            NativeTelemetryRecord {
                                event_index: batch_source_action_index,
                                dispatch_id: 0,
                                kind: "down",
                                scheduled_us: batch_scheduled_us,
                                actual_us: effective_now_us,
                                dispatch_completed_us: effective_now_us,
                                lateness_us: signed_delta(effective_now_us, batch_scheduled_us),
                                visible_lateness_us: signed_delta(
                                    effective_now_us,
                                    batch_scheduled_us,
                                ),
                                send_duration_us: 0,
                                send_duration_pure_us: 0,
                                bookkeeping_us: 0,
                                dispatch_lateness_us: signed_delta(
                                    effective_now_us,
                                    batch_scheduled_us,
                                ),
                                scan_codes,
                                sent_scan_codes: SmallVec::new(),
                                skipped_scan_codes: SmallVec::new(),
                                generation_ids,
                                runtime_outcome: "dropped_conflict",
                                deferred_by_us: 0,
                                pre_send_spin_us: 0,
                                idle_gap_us: 0,
                                reason_id: batch_reason_id,
                                reason: None,
                                applied_lead_us: lead_down,
                                first_win32_error: None,
                                last_win32_error: None,
                                send_attempts: 0,
                                zero_progress_retries: 0,
                            }
                        });
                    }
                    let send_playable = !has_conflicts
                        || matches!(
                            config.chord_conflict_policy,
                            ChordConflictPolicy::DropConflictingKeys
                        );
                    if has_conflicts && !send_playable {
                        local_metrics.authored_chords_rejected =
                            local_metrics.authored_chords_rejected.saturating_add(1);
                        // Terminalize playable (non-conflict) intents too since whole chord drops.
                        let owned = coordinator.schedule.materialize_batch(
                            coordinator.cursor - 1,
                            coordinator.recovery_offset_us(),
                        );
                        let playable_intents: SmallVec<[_; 15]> = owned
                            .intents
                            .iter()
                            .filter(|i| {
                                conflict_mask
                                    & sky_dispatch_core::coordinator::RuntimeDispatchCoordinator::bit_for_slot_pub(i.key_slot)
                                    == 0
                            })
                            .cloned()
                            .collect();
                        coordinator.drop_conflicted_downs(&playable_intents);
                        if matches!(
                            config.chord_conflict_policy,
                            ChordConflictPolicy::AbortPlayback
                        ) {
                            force_full_cleanup = true;
                            terminal_error = Some(format!(
                                "same-key conflict rejected authored chord at action {}",
                                batch_source_action_index
                            ));
                            publish_backend_metrics(
                                &backend,
                                &mut local_metrics,
                                metrics,
                                &mut last_published_error,
                            );
                            try_publish_metrics(&local_metrics, metrics, qpc_now_us(), true);
                            break;
                        }
                    }
                    if send_playable && !scan_batch.is_empty() {
                        let started_us = qpc_now_us();
                        let actual_us = qpc_ticks_to_us(QpcTicks(
                            clock_state
                                .get_elapsed(QpcTicks(qpc_us_to_ticks(started_us)))
                                .0,
                        ));
                        // SendInput uses the stack-only scan code buffer — no allocation.
                        let result = backend.key_down(scan_batch.as_slice());

                        let (
                            result_completed_us,
                            result_sent,
                            result_skipped_duplicates,
                            result_send_attempts,
                            result_zero_progress_retries,
                            result_retried_after_zero_progress,
                            result_chord_integrity_lost,
                            result_first_win32_error,
                            result_last_win32_error,
                            result_success,
                        ) = match &result {
                            sky_dispatch_win32::input::DownSendOutcome::Complete {
                                completed_us,
                                sent,
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                retried_after_zero_progress,
                            } => (
                                *completed_us,
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
                                completed_us,
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                first_error,
                                last_error,
                                ..
                            } => (
                                *completed_us,
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
                                completed_us,
                                sent,
                                skipped_duplicates,
                                send_attempts,
                                zero_progress_retries,
                                first_error,
                                last_error,
                                ..
                            } => (
                                *completed_us,
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

                        let completed_effective_ticks = TimelineTicks(
                            clock_state
                                .get_elapsed(QpcTicks(qpc_us_to_ticks(result_completed_us)))
                                .0,
                        );
                        let completed_effective =
                            qpc_ticks_to_us(QpcTicks(completed_effective_ticks.0));
                        last_send_qpc_us = Some(result_completed_us);
                        // Activate sent downs via the compact path.
                        coordinator.activate_sent_downs_compact(
                            coordinator.cursor - 1,
                            &result_sent,
                            actual_us,
                            effective_now_ticks,
                            completed_effective,
                            completed_effective_ticks,
                        );
                        let completion_error_us =
                            signed_delta(completed_effective, batch_scheduled_us);
                        let retry_late_threshold_us = config
                            .late_pulse_drop_threshold_us
                            .unwrap_or(STRICT_RETRY_LATE_THRESHOLD_US)
                            .min(i64::MAX as u64)
                            as i64;
                        let clean_down_sample = result_success
                            && result_sent.len() == playable_count
                            && result_skipped_duplicates.is_empty()
                            && result_send_attempts == 1
                            && !result_chord_integrity_lost;
                        let recovered_retry_late = result_retried_after_zero_progress
                            && completion_error_us > retry_late_threshold_us;
                        let retry_late_abort = config.strict_timing && recovered_retry_late;
                        let strict_down_completion_late = config.strict_timing
                            && clean_down_sample
                            && completion_error_us
                                > config.strict_down_completion_late_us.min(i64::MAX as u64) as i64;
                        if recovered_retry_late {
                            local_metrics.recovered_zero_progress_but_late = local_metrics
                                .recovered_zero_progress_but_late
                                .saturating_add(1);
                        }
                        down_saturation_positive_streak =
                            if lead_down_saturated && completion_error_us > 0 {
                                down_saturation_positive_streak.saturating_add(1)
                            } else {
                                0
                            };
                        let saturation_abort = config.strict_timing
                            && down_saturation_positive_streak >= STRICT_SATURATION_ABORT_STREAK;
                        if config.enable_adaptive_lead {
                            update_estimator_after_send_class(
                                &mut estimator,
                                ActionKind::Down,
                                result_completed_us.saturating_sub(started_us),
                                result_sent.len(),
                                playable_count,
                                lead_down,
                                completion_error_us,
                                clean_down_sample,
                                latency_class,
                            );
                        }
                        let bookkeeping_completed_us = qpc_now_us();
                        telemetry.push(|| {
                            // Scan codes: materialized view for telemetry includes full chord.
                            let scan_codes: SmallVec<[u16; 15]> =
                                scan_batch.as_slice().iter().copied().collect();
                            let generation_ids: SmallVec<[u64; 15]> = materialized_for_telemetry
                                .as_ref()
                                .map(|b| {
                                    b.intents
                                        .iter()
                                        .filter(|i| conflict_mask & (1u16 << i.key_slot) == 0)
                                        .filter_map(|i| i.generation_id)
                                        .collect()
                                })
                                .unwrap_or_default();
                            NativeTelemetryRecord {
                                event_index: batch_source_action_index,
                                dispatch_id: 0,
                                kind: "down",
                                scheduled_us: batch_scheduled_us,
                                actual_us,
                                dispatch_completed_us: completed_effective,
                                lateness_us: signed_delta(actual_us, batch_scheduled_us),
                                visible_lateness_us: signed_delta(
                                    completed_effective,
                                    batch_scheduled_us,
                                ),
                                send_duration_us: bookkeeping_completed_us
                                    .saturating_sub(started_us),
                                send_duration_pure_us: result_completed_us
                                    .saturating_sub(started_us),
                                bookkeeping_us: bookkeeping_completed_us
                                    .saturating_sub(result_completed_us),
                                dispatch_lateness_us: signed_delta(actual_us, batch_scheduled_us)
                                    .saturating_add(
                                        result_completed_us.saturating_sub(started_us) as i64
                                    ),
                                scan_codes,
                                sent_scan_codes: result_sent.clone(),
                                skipped_scan_codes: result_skipped_duplicates.clone(),
                                generation_ids,
                                runtime_outcome: if recovered_retry_late {
                                    "recovered_zero_progress_but_late"
                                } else if strict_down_completion_late {
                                    "strict_completion_slo_exceeded"
                                } else if result_chord_integrity_lost {
                                    "chord_integrity_lost"
                                } else if result_sent.len() == scan_batch.len() {
                                    "sent"
                                } else {
                                    "partial_note_on"
                                },
                                deferred_by_us: 0,
                                pre_send_spin_us: pending_pre_send_spin_us,
                                idle_gap_us: 0,
                                reason_id: batch_reason_id,
                                reason: None,
                                applied_lead_us: lead_down,
                                first_win32_error: result_first_win32_error,
                                last_win32_error: result_last_win32_error,
                                send_attempts: result_send_attempts,
                                zero_progress_retries: result_zero_progress_retries,
                            }
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
                            latency_tx,
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
                            try_publish_metrics(&local_metrics, metrics, qpc_now_us(), true);
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
                            try_publish_metrics(&local_metrics, metrics, qpc_now_us(), true);
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
                            try_publish_metrics(&local_metrics, metrics, qpc_now_us(), true);
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
                            try_publish_metrics(&local_metrics, metrics, qpc_now_us(), true);
                            break;
                        }
                    }
                } else {
                    // Up batch: must materialise to drive request_releases (needs generation IDs).
                    let owned = coordinator.schedule.materialize_batch(
                        coordinator.cursor - 1,
                        coordinator.recovery_offset_us(),
                    );
                    let (_, suppressed) = coordinator.request_releases(&owned.intents);
                    if !suppressed.is_empty() {
                        telemetry.push(|| NativeTelemetryRecord {
                            event_index: batch_source_action_index,
                            dispatch_id: 0,
                            kind: "up",
                            scheduled_us: batch_scheduled_us,
                            actual_us: effective_now_us,
                            dispatch_completed_us: effective_now_us,
                            lateness_us: signed_delta(effective_now_us, batch_scheduled_us),
                            visible_lateness_us: signed_delta(effective_now_us, batch_scheduled_us),
                            send_duration_us: 0,
                            send_duration_pure_us: 0,
                            bookkeeping_us: 0,
                            dispatch_lateness_us: signed_delta(
                                effective_now_us,
                                batch_scheduled_us,
                            ),
                            scan_codes: suppressed.iter().map(|intent| intent.scan_code).collect(),
                            sent_scan_codes: SmallVec::new(),
                            skipped_scan_codes: SmallVec::new(),
                            generation_ids: suppressed
                                .iter()
                                .filter_map(|intent| intent.generation_id)
                                .collect(),
                            runtime_outcome: "suppressed_stale_up",
                            deferred_by_us: 0,
                            pre_send_spin_us: 0,
                            idle_gap_us: 0,
                            reason_id: batch_reason_id,
                            reason: None,
                            applied_lead_us: lead_up,
                            first_win32_error: None,
                            last_win32_error: None,
                            send_attempts: 0,
                            zero_progress_retries: 0,
                        });
                    }
                }
                publish_backend_metrics(
                    &backend,
                    &mut local_metrics,
                    metrics,
                    &mut last_published_error,
                );
                try_publish_metrics(&local_metrics, metrics, qpc_now_us(), true);
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
            let pending_plan = coordinator.plan_pending_dispatch(|polyphony| {
                if config.dispatch_lead_us > 0 {
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
                }
            });
            if let Some(deadline_us) =
                coordinator.next_deadline_with_pending_plan(lead_down, pending_plan.as_ref())
            {
                // Take the QPC tick and its logical elapsed-time sample from
                // the same instant.  Reusing the older outer-loop elapsed
                // sample after doing bookkeeping shifts the absolute target
                // late by the whole A->B overhead interval.
                let target_sample_ticks = qpc_now_ticks();
                let target_sample_elapsed_us = qpc_ticks_to_us(QpcTicks(
                    clock_state
                        .get_elapsed(QpcTicks(qpc_us_to_ticks(qpc_ticks_to_us(
                            target_sample_ticks,
                        ))))
                        .0,
                ));
                if deadline_us > target_sample_elapsed_us {
                    let remaining_us = deadline_us - target_sample_elapsed_us;
                    if config.enable_adaptive_spin
                        && config.enable_spin_reprobe
                        && remaining_us >= 30_000
                        && now_us.saturating_sub(last_spin_probe_us) >= 30_000_000
                    {
                        if let Some(stats) = waiter.probe_wake_error_stats(interrupt, 8) {
                            publish_wake_error_stats(stats, &mut local_metrics);
                            let candidate =
                                derive_spin_threshold_us(stats.robust_us, config.spin_floor_us);
                            let adjusted =
                                adjust_spin_threshold(effective_spin_threshold_us, candidate);
                            if adjusted != effective_spin_threshold_us {
                                effective_spin_threshold_us = adjusted;
                                local_metrics.effective_spin_threshold_us = adjusted;
                            }
                            last_spin_probe_us = qpc_now_us();
                        }
                        continue;
                    }
                    let target_qpc = deadline_target_ticks_from_anchor(
                        target_sample_ticks,
                        qpc_ticks_to_us(target_sample_ticks),
                        qpc_ticks_to_us(clock_state.epoch),
                        deadline_us,
                    );
                    let cold_warmup_us = if last_send_qpc_us.is_none_or(|last| {
                        qpc_ticks_to_us(target_sample_ticks).saturating_sub(last)
                            > SEND_COLD_THRESHOLD_US
                    }) {
                        config.core_warmup_budget_us.min(CORE_WARMUP_SPIN_MAX_US)
                    } else {
                        0
                    };
                    let wait_result = waiter.wait_until_ticks_with_metrics(
                        lease_bounded_ticks(
                            target_qpc,
                            target_sample_ticks,
                            config.supervisor_lease_timeout_us,
                            supervisor_heartbeat_us,
                        ),
                        effective_spin_threshold_us.saturating_add(cold_warmup_us),
                        interrupt,
                    );
                    local_metrics.idle_wake_count = local_metrics.idle_wake_count.saturating_add(1);
                    local_metrics.spin_time_us = local_metrics
                        .spin_time_us
                        .saturating_add(wait_result.spin_us);
                    pending_pre_send_spin_us = wait_result.spin_us;
                    let wake_elapsed_us = qpc_ticks_to_us(QpcTicks(
                        clock_state
                            .get_elapsed(QpcTicks(qpc_us_to_ticks(qpc_now_us())))
                            .0,
                    ));
                    match wait_result.outcome {
                        WaitOutcome::Deadline => {
                            local_metrics.wait_target_error_us = local_metrics
                                .wait_target_error_us
                                .max(wake_elapsed_us.saturating_sub(deadline_us));
                        }
                        WaitOutcome::Failed(failure) => {
                            local_metrics.wait_path_degraded = true;
                            if config.strict_timing {
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
                        && wake_elapsed_us > deadline_us.saturating_add(config.input_path_warn_us)
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

    // This cleanup sits outside the contained loop so it also runs when an
    // unexpected panic crosses the orchestration/backend seam.
    let cleanup_result = catch_unwind(AssertUnwindSafe(|| {
        if worker_result.is_err() || force_full_cleanup {
            backend.release_all_full_instrument()
        } else {
            backend.release_all()
        }
    }));
    if let Ok(outcome) = &cleanup_result {
        *metrics.terminal_release_outcome.lock() = Some(outcome.clone());
    }
    if let Some(error) = terminal_error.as_ref() {
        *metrics.terminal_error.lock() = Some(error.clone());
    }
    coordinator.cancel_all();
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
    *metrics.generation_status_counts.lock() = coordinator.generation_status_counts();
    publish_backend_metrics(
        &backend,
        &mut local_metrics,
        metrics,
        &mut last_published_error,
    );

    let end_us = qpc_now_us();
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

fn supervisor_lease_expired(timeout_us: u64, heartbeat_us: &AtomicU64) -> bool {
    timeout_us > 0 && qpc_now_us().saturating_sub(heartbeat_us.load(Ordering::Acquire)) > timeout_us
}

fn lease_bounded_us(target_us: u64, timeout_us: u64, heartbeat_us: &AtomicU64) -> u64 {
    if timeout_us == 0 {
        return target_us;
    }
    let heartbeat = heartbeat_us.load(Ordering::Acquire);
    if heartbeat == 0 {
        return target_us;
    }
    target_us.min(heartbeat.saturating_add(timeout_us))
}

fn lease_bounded_ticks(
    target: QpcTicks,
    now_ticks: QpcTicks,
    timeout_us: u64,
    heartbeat_us: &AtomicU64,
) -> QpcTicks {
    if timeout_us == 0 {
        return target;
    }
    let heartbeat = heartbeat_us.load(Ordering::Acquire);
    if heartbeat == 0 {
        return target;
    }
    let lease_deadline_us = heartbeat.saturating_add(timeout_us);
    let now_us = qpc_ticks_to_us(now_ticks);
    if lease_deadline_us <= now_us {
        return now_ticks;
    }
    let lease_deadline = QpcTicks(
        now_ticks
            .0
            .saturating_add(qpc_us_to_ticks(lease_deadline_us - now_us)),
    );
    target.min(lease_deadline)
}

fn drain_commands(
    rx: &Receiver<WorkerCommand>,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
) {
    loop {
        match rx.try_recv() {
            Ok(WorkerCommand::Skip) => skip_requested.store(true, Ordering::Release),
            Ok(WorkerCommand::Quit) => quit_requested.store(true, Ordering::Release),
            Ok(WorkerCommand::PanicRelease) => panic_requested.store(true, Ordering::Release),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }
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
    latency_tx: &Sender<i64>,
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
    let _ = latency_tx.try_send(lateness_us);
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
    update_estimator_after_send_class(
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
) {
    if !clean_sample || sent_count == 0 {
        return;
    }
    estimator.update_with_class(kind, duration_us, authored_polyphony, latency_class);
    if applied_lead_us > 0 {
        estimator.update_completion_error_with_class(kind, completion_error_us, latency_class);
    }
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

fn classify_latency_class(last_send_qpc_us: Option<u64>, now_qpc_us: u64) -> LatencyClass {
    if last_send_qpc_us.is_none_or(|last| now_qpc_us.saturating_sub(last) > SEND_COLD_THRESHOLD_US)
    {
        LatencyClass::Cold
    } else {
        LatencyClass::Hot
    }
}

/// Map a non-negative logical deadline to an absolute QPC target using the
/// clock's physical anchor and one current sample.
fn deadline_target_ticks_from_anchor(
    now_ticks: QpcTicks,
    now_qpc_us: u64,
    anchor_us: u64,
    deadline_us: u64,
) -> QpcTicks {
    let target_us = anchor_us.saturating_add(deadline_us);
    if target_us <= now_qpc_us {
        return now_ticks;
    }
    QpcTicks(
        now_ticks
            .0
            .saturating_add(qpc_us_to_ticks(target_us.saturating_sub(now_qpc_us))),
    )
}

/// Map an authored timestamp minus lead, including the negative interval that
/// is intentionally needed for a first note at authored t=0.
fn anchored_dispatch_target_ticks(
    now_ticks: QpcTicks,
    now_qpc_us: u64,
    anchor_us: u64,
    scheduled_us: u64,
    lead_us: u64,
) -> QpcTicks {
    let target_us = anchor_us
        .saturating_add(scheduled_us)
        .saturating_sub(lead_us);
    if target_us <= now_qpc_us {
        return now_ticks;
    }
    QpcTicks(
        now_ticks
            .0
            .saturating_add(qpc_us_to_ticks(target_us.saturating_sub(now_qpc_us))),
    )
}

/// Legacy relative helper retained for unit-test compatibility.
#[cfg(test)]
fn deadline_target_ticks(now_ticks: QpcTicks, logical_now_us: u64, deadline_us: u64) -> QpcTicks {
    QpcTicks(
        now_ticks
            .0
            .saturating_add(qpc_us_to_ticks(deadline_us.saturating_sub(logical_now_us))),
    )
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
    }
}

fn derive_spin_threshold_us(wake_error_us: u64, spin_floor_us: u64) -> u64 {
    wake_error_us
        .saturating_add(200)
        .clamp(spin_floor_us, 3_000)
}

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
        INPUT_PATH_WINDOW_CAPACITY, WakeErrorStats, WorkerCommand, adjust_spin_threshold,
        anchored_dispatch_target_ticks, classify_latency_class, deadline_target_ticks,
        derive_spin_threshold_us, drain_commands, focus_gate_matches, record_input_path_health,
        release_runtime_outcome, update_estimator_after_send,
    };
    use crossbeam_channel::bounded;
    use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator};
    use sky_dispatch_core::model::ActionKind;
    use sky_dispatch_win32::clock::{QpcTicks, qpc_ticks_to_us, qpc_us_to_ticks};
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn deadline_mapper_uses_the_current_sample_without_overhead_drift() {
        let now_ticks = QpcTicks(10_000_000);
        let logical_now_us = qpc_ticks_to_us(now_ticks);
        let target = deadline_target_ticks(now_ticks, logical_now_us, logical_now_us + 1_000);
        assert_eq!(target.0 - now_ticks.0, qpc_us_to_ticks(1_000));
    }

    #[test]
    fn first_authored_zero_timestamp_can_use_a_future_physical_anchor() {
        let now_ticks = QpcTicks(10_000_000);
        let now_qpc_us = qpc_ticks_to_us(now_ticks);
        let startup_guard_us = 1_000;
        let lead_us = 500;
        let anchor_us = now_qpc_us
            .saturating_add(startup_guard_us)
            .saturating_add(lead_us);
        let target = anchored_dispatch_target_ticks(now_ticks, now_qpc_us, anchor_us, 0, lead_us);

        assert_eq!(target.0 - now_ticks.0, qpc_us_to_ticks(startup_guard_us));
    }

    #[test]
    fn cold_classification_uses_physical_gap_after_logical_pause() {
        assert_eq!(
            classify_latency_class(Some(100_000), 120_000),
            LatencyClass::Hot
        );
        assert_eq!(
            classify_latency_class(Some(100_000), 120_001 + super::SEND_COLD_THRESHOLD_US),
            LatencyClass::Cold
        );
        assert_eq!(classify_latency_class(None, 0), LatencyClass::Cold);
    }

    #[test]
    fn saturated_terminal_queue_cannot_overwrite_latest_pause_state() {
        let (tx, rx) = bounded(32);
        for _ in 0..32 {
            tx.send(WorkerCommand::Quit).unwrap();
        }
        let desired_pause = AtomicBool::new(true);

        // Resume is latest-wins state, not a queued command. A stale queued
        // terminal command must not be able to write it back.
        for index in 0..10_000 {
            desired_pause.store(index % 2 == 0, Ordering::Release);
        }
        let quit_requested = AtomicBool::new(false);
        let skip_requested = AtomicBool::new(false);
        let panic_requested = AtomicBool::new(false);
        drain_commands(&rx, &quit_requested, &skip_requested, &panic_requested);

        assert!(!desired_pause.load(Ordering::Acquire));
        assert!(quit_requested.load(Ordering::Acquire));
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
        let mut estimator = SendLatencyEstimator::new(0.2, 2_000, 6);

        update_estimator_after_send(&mut estimator, ActionKind::Down, 900, 0, 3, 500, 120, false);
        let state = estimator.export_state();
        assert_eq!(state.count_down[3], 0);
        assert_eq!(state.count_residual, 0);

        update_estimator_after_send(&mut estimator, ActionKind::Down, 900, 1, 3, 0, 120, false);
        let state = estimator.export_state();
        assert_eq!(state.count_down[3], 0);
        assert_eq!(state.count_residual, 0);

        update_estimator_after_send(&mut estimator, ActionKind::Down, 900, 1, 3, 500, 120, true);
        let state = estimator.export_state();
        assert_eq!(state.count_down[3], 1);
        assert_eq!(estimator.export_state().count_residual, 1);
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
}
