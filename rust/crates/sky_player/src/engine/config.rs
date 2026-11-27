use super::telemetry::TelemetryMode;
use sky_dispatch_core::executor::PreparedDispatchSchedule;
use sky_dispatch_win32::input::{InstrumentKeyProfileSpec, MaterializedInstrumentKeyProfile};
use sky_dispatch_win32::mmcss::PriorityMode;

#[cfg(any(test, feature = "test-support"))]
mod startup_ordering;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use startup_ordering::InterruptedWaitEvidence;

mod effective_policy;
#[cfg(any(test, feature = "test-support"))]
pub mod hol;
pub(crate) use effective_policy::EffectiveRuntimePolicy;

pub(crate) const STARTUP_READINESS_RESERVE_US: u64 = 2_000;
pub(crate) const DEFAULT_SPIN_THRESHOLD_US: u64 = 1_000;
pub(crate) const MIN_CALIBRATED_SPIN_US: u64 = 250;
pub(crate) const CALIBRATION_SAFETY_MARGIN_US: u64 = 50;
pub(crate) const CALIBRATION_SAMPLES: usize = 6;
pub(crate) const CALIBRATION_MAX_STARTUP_BUDGET_US: u64 = 20_000;
pub(crate) const MIN_PRODUCTION_PREROLL_US: u64 = 50_000;

/// Experimental calibration placement exposed only by test-support builds.
/// Normal application builds use the internal shipping selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupCalibrationPlacement {
    BeforeSchedulingGuards,
    AfterSchedulingGuards,
    /// Test-only W2-E arm selector that bypasses the startup wake probe while
    /// keeping the production waiter, scheduling guards, preroll, and all
    /// lifecycle admission checkpoints intact.
    #[cfg(any(test, feature = "test-support"))]
    BypassedForFixedArm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum StartupCalibrationResult {
    NotRequested = 0,
    Completed = 1,
    BudgetExpired = 2,
    Interrupted = 3,
    Unavailable = 4,
}

impl StartupCalibrationResult {
    pub(crate) const fn from_raw(value: u8) -> Self {
        match value {
            1 => Self::Completed,
            2 => Self::BudgetExpired,
            3 => Self::Interrupted,
            4 => Self::Unavailable,
            _ => Self::NotRequested,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::NotRequested => "not_requested",
            Self::Completed => "completed",
            Self::BudgetExpired => "budget_expired",
            Self::Interrupted => "interrupted",
            Self::Unavailable => "unavailable",
        }
    }
}
/// Maximum interval for the non-realtime supervisor to prove it is still
/// alive. The direct desktop adapter consumes this same qualified safety
/// boundary.
pub const DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US: u64 = 3_000_000;
/// Maximum soft-wait duration when musical Down is blocked by physically held modifiers.
pub const DEFAULT_MODIFIER_HOLD_TIMEOUT_US: u64 = 2_000_000;

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestWaitPolicy {
    /// Historical test-support behavior. Existing tests use the wide spin
    /// window to avoid making host timer jitter part of their assertions.
    LegacyTestWideSpin,
    /// Test-only qualification policy that follows the shipping waiter and
    /// one-shot startup calibration without enabling production transport.
    ProductionCalibrated,
}

#[cfg(any(test, feature = "test-support"))]
impl TestWaitPolicy {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        match value {
            "legacy_test_wide_spin" => Ok(Self::LegacyTestWideSpin),
            "production_calibrated" => Ok(Self::ProductionCalibrated),
            _ => Err("wait_policy must be legacy_test_wide_spin or production_calibrated"),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::LegacyTestWideSpin => "legacy_test_wide_spin",
            Self::ProductionCalibrated => "production_calibrated",
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpinThresholdSource {
    Unknown = 0,
    ProductionStartupCalibration = 1,
    TestFixedOverride = 2,
    LegacyTestWideSpin = 3,
    ProductionFallback = 4,
}

impl SpinThresholdSource {
    pub(crate) const fn from_raw(value: u8) -> Self {
        match value {
            1 => Self::ProductionStartupCalibration,
            2 => Self::TestFixedOverride,
            3 => Self::LegacyTestWideSpin,
            4 => Self::ProductionFallback,
            _ => Self::Unknown,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::ProductionStartupCalibration => "production_startup_calibration",
            Self::TestFixedOverride => "test_fixed_override",
            Self::LegacyTestWideSpin => "legacy_test_wide_spin",
            Self::ProductionFallback => "production_startup_fallback",
        }
    }
}

pub(crate) fn validate_timing_constants() -> Result<(), String> {
    if STARTUP_READINESS_RESERVE_US <= DEFAULT_SPIN_THRESHOLD_US {
        return Err("startup readiness reserve must be greater than spin threshold".to_string());
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
use super::{FaultInjectionScript, SystemPowerState};
#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};

#[cfg(any(test, feature = "test-support"))]
pub(crate) type FocusPauseHook = Arc<dyn Fn() + Send + Sync>;

#[cfg(any(test, feature = "test-support"))]
pub(crate) type FinalGateRaceHook = Arc<
    dyn Fn(
            &AtomicBool,
            &super::shared::SessionTarget,
            &AtomicBool,
            &AtomicBool,
            &AtomicBool,
            &AtomicBool,
            &SystemPowerState,
        ) + Send
        + Sync,
>;

#[cfg(any(test, feature = "test-support"))]
pub(crate) type FinalGateBeforeSyscallHook = Arc<dyn Fn() + Send + Sync>;

/// Deliberate session profiles. The profile owns backend/policy selection so
/// callers do not compose a contradictory matrix of booleans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchProfile {
    Production,
    StrictTimingDiagnostic,
    #[cfg(any(test, feature = "test-support"))]
    MockTest,
}

impl DispatchProfile {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "production" => Ok(Self::Production),
            "strict_timing_diagnostic" => Ok(Self::StrictTimingDiagnostic),
            #[cfg(any(test, feature = "test-support"))]
            "mock_test" => Ok(Self::MockTest),
            _ => Err("profile must be 'production' or 'strict_timing_diagnostic'".to_string()),
        }
    }

    pub fn strict_timing(self) -> bool {
        matches!(self, Self::StrictTimingDiagnostic)
    }

    pub(crate) fn observer_enabled(self) -> bool {
        match self {
            Self::Production => false,
            Self::StrictTimingDiagnostic => true,
            #[cfg(any(test, feature = "test-support"))]
            Self::MockTest => true,
        }
    }
}

pub enum BackendConfig {
    Production,
    #[cfg(any(test, feature = "test-support"))]
    Mock {
        latency_base_us: u64,
        latency_per_key_us: u64,
        fault_script: FaultInjectionScript,
    },
}

pub struct NativeSessionOptions {
    /// Immutable QPC-domain plan prepared before native session admission.
    pub schedule: PreparedDispatchSchedule,
    pub backend: BackendConfig,
    pub profile: DispatchProfile,
    pub timing: TimingOptions,
    pub focus: FocusOptions,
    pub wait: WaitOptions,
    pub telemetry: TelemetryOptions,
    pub priority: PriorityOptions,
    /// Optional unvalidated Win32-boundary profile input. `None` selects the
    /// canonical 15-key profile during native session admission.
    pub instrument_key_profile: Option<InstrumentKeyProfileSpec>,
    #[cfg(any(test, feature = "test-support"))]
    pub startup_ordering_hook: Option<Arc<StartupOrderingHook>>,
    /// Paired benchmark selector; absent from production-facing options.
    #[cfg(any(test, feature = "test-support"))]
    pub startup_calibration_placement: StartupCalibrationPlacement,
    /// Enables existing QPC timing observation for qualification builds only.
    #[cfg(any(test, feature = "test-support"))]
    pub timing_observer_for_qualification: bool,
    #[cfg(any(test, feature = "test-support"))]
    pub focus_pause_hook: Option<FocusPauseHook>,
    #[cfg(any(test, feature = "test-support"))]
    pub timer_lifecycle_context:
        Option<sky_dispatch_win32::timer::test_support::TimerLifecycleContext>,
    #[cfg(any(test, feature = "test-support"))]
    pub prepared_packet_ambiguity_mask: Option<u16>,
    #[cfg(any(test, feature = "test-support"))]
    pub preflight_user_held_mask: Option<u16>,
    /// Deterministic final modifier-state source for native acceptance only.
    #[cfg(any(test, feature = "test-support"))]
    pub modifier_key_state_query_for_test: Option<Arc<dyn Fn(i32) -> i16 + Send + Sync>>,
}

pub(crate) struct AdmittedNativeSessionOptions {
    pub(crate) options: NativeSessionOptions,
    pub(crate) instrument_key_profile: MaterializedInstrumentKeyProfile,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
pub struct StartupOrderingHook {
    sequence: AtomicU64,
    cpu_baseline_trace: std::sync::Mutex<CpuBaselineOrderingTrace>,
    pub(crate) first_physical_send_started: AtomicU64,
    pub(crate) boot_delay_us: AtomicU64,
    down_authored_target_qpc_ticks: AtomicU64,
    down_not_before_qpc_ticks: AtomicU64,
    down_window_sequence: AtomicU64,
    down_sender_start_qpc_ticks: AtomicU64,
    down_sender_authored_target_qpc_ticks: AtomicU64,
    down_sender_packet_not_before_qpc_ticks: AtomicU64,
    pub(crate) up_only_sender_attempt_qpc_ticks: std::sync::Mutex<Option<(Option<u64>, u64, u64)>>,
    down_sender_wake_qpc_ticks: AtomicU64,
    down_sender_attempt_count: AtomicU64,
    interrupted_wait_count: AtomicU64,
    interrupted_wait_evidence: std::sync::Mutex<Option<InterruptedWaitEvidence>>,
    prepared_schedule_executor_consumption_count: AtomicU64,
    watchdog_hold: AtomicBool,
    watchdog_waiting: AtomicBool,
    startup_checkpoint: AtomicU8,
    startup_checkpoint_occurrence: AtomicUsize,
    startup_checkpoint_reached: AtomicBool,
    startup_checkpoint_released: AtomicBool,
    startup_checkpoint_lock: std::sync::Mutex<()>,
    startup_checkpoint_cv: std::sync::Condvar,
    #[cfg(test)]
    final_gate_focus_rejection_count: AtomicU64,
    #[cfg(test)]
    complete_startup_calibration_after_checkpoint: AtomicBool,
    panic_startup_checkpoint: AtomicU8,
    fail_worker_spawn: AtomicBool,
    fail_watchdog_spawn: AtomicBool,
    fail_observer_spawn: AtomicBool,
    fail_startup_epoch_conversion: AtomicBool,
    fail_guarded_calibration_origin: AtomicBool,
    pub(crate) hol: hol::HolStallController,
    scheduling_threads:
        std::sync::Mutex<(Option<std::thread::ThreadId>, Option<std::thread::ThreadId>)>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CpuBaselineOrderingTrace {
    next_sequence: u64,
    pub(crate) startup: Option<(u64, u64, u64)>,
    pub(crate) observer: Option<(u64, u64, u64)>,
    pub(crate) playback: Option<(u64, u64, u64)>,
    pub(crate) epoch_qpc_sample_completed: Option<u64>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy)]
enum CpuBaselineDestination {
    Startup,
    Observer,
    Playback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StartupCheckpoint {
    StreamBuild = 1,
    BeforeReady = 3,
    ReadyToArmed = 4,
    EpochPrepared = 5,
    #[cfg(any(test, feature = "test-support"))]
    ArmedPublished = 6,
    #[cfg(any(test, feature = "test-support"))]
    InitialWait = 7,
    #[cfg(any(test, feature = "test-support"))]
    FirstDownAdmission = 8,
    #[cfg(any(test, feature = "test-support"))]
    FirstDownCommitted = 10,
    #[cfg(any(test, feature = "test-support"))]
    TerminalCleanup = 11,
    #[cfg(any(test, feature = "test-support"))]
    WakeCalibration = 12,
    #[cfg(any(test, feature = "test-support"))]
    WorkerCompletionPublished = 13,
    #[cfg(any(test, feature = "test-support"))]
    PhysicalWait = 16,
    #[cfg(any(test, feature = "test-support"))]
    DispatchWait = 17,
}

#[cfg(any(test, feature = "test-support"))]
impl StartupOrderingHook {
    fn record_cpu_baseline_pair(&self, pair: (u64, u64), destination: CpuBaselineDestination) {
        if let Ok(mut trace) = self.cpu_baseline_trace.lock() {
            trace.next_sequence = trace.next_sequence.saturating_add(1);
            let sequence = trace.next_sequence;
            let event = Some((sequence, pair.0, pair.1));
            match destination {
                CpuBaselineDestination::Startup => trace.startup = event,
                CpuBaselineDestination::Observer => trace.observer = event,
                CpuBaselineDestination::Playback => trace.playback = event,
            }
        }
    }

    pub(crate) fn record_startup_cpu_baseline(&self, thread_cpu_us: u64, process_cpu_us: u64) {
        self.record_cpu_baseline_pair(
            (thread_cpu_us, process_cpu_us),
            CpuBaselineDestination::Startup,
        );
    }

    pub(crate) fn record_observer_cpu_baseline(&self, thread_cpu_us: u64, process_cpu_us: u64) {
        self.record_cpu_baseline_pair(
            (thread_cpu_us, process_cpu_us),
            CpuBaselineDestination::Observer,
        );
    }

    pub(crate) fn record_playback_cpu_baseline(&self, thread_cpu_us: u64, process_cpu_us: u64) {
        self.record_cpu_baseline_pair(
            (thread_cpu_us, process_cpu_us),
            CpuBaselineDestination::Playback,
        );
    }

    pub(crate) fn record_epoch_qpc_sample_completed(&self) {
        if let Ok(mut trace) = self.cpu_baseline_trace.lock() {
            trace.next_sequence = trace.next_sequence.saturating_add(1);
            let sequence = trace.next_sequence;
            trace.epoch_qpc_sample_completed = Some(sequence);
        }
    }

    #[cfg(all(test, feature = "test-support"))]
    pub(crate) fn cpu_baseline_trace_for_test(&self) -> CpuBaselineOrderingTrace {
        self.cpu_baseline_trace
            .lock()
            .map(|trace| *trace)
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(crate) fn complete_startup_calibration_after_checkpoint(&self) {
        self.complete_startup_calibration_after_checkpoint
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn take_startup_calibration_completion(&self) -> bool {
        self.complete_startup_calibration_after_checkpoint
            .swap(false, Ordering::AcqRel)
    }

    pub fn fail_next_worker_spawn(&self) {
        self.fail_worker_spawn.store(true, Ordering::Release);
    }

    pub(crate) fn take_worker_spawn_failure(&self) -> bool {
        self.fail_worker_spawn.swap(false, Ordering::AcqRel)
    }

    pub fn fail_next_watchdog_spawn(&self) {
        self.fail_watchdog_spawn.store(true, Ordering::Release);
    }

    pub(crate) fn take_watchdog_spawn_failure(&self) -> bool {
        self.fail_watchdog_spawn.swap(false, Ordering::AcqRel)
    }

    pub fn fail_next_observer_spawn(&self) {
        self.fail_observer_spawn.store(true, Ordering::Release);
    }

    pub(crate) fn take_observer_spawn_failure(&self) -> bool {
        self.fail_observer_spawn.swap(false, Ordering::AcqRel)
    }

    pub fn fail_next_startup_epoch_conversion(&self) {
        self.fail_startup_epoch_conversion
            .store(true, Ordering::Release);
    }

    pub(crate) fn take_startup_epoch_conversion_failure(&self) -> bool {
        self.fail_startup_epoch_conversion
            .swap(false, Ordering::AcqRel)
    }

    pub fn fail_next_guarded_calibration_origin(&self) {
        self.fail_guarded_calibration_origin
            .store(true, Ordering::Release);
    }

    pub(crate) fn take_guarded_calibration_origin_failure(&self) -> bool {
        self.fail_guarded_calibration_origin
            .swap(false, Ordering::AcqRel)
    }

    pub(crate) fn record_scheduling_guard_acquired(&self) {
        if let Ok(mut threads) = self.scheduling_threads.lock() {
            threads.0 = Some(std::thread::current().id());
        }
    }

    pub(crate) fn record_scheduling_guard_dropped(&self) {
        if let Ok(mut threads) = self.scheduling_threads.lock() {
            threads.1 = Some(std::thread::current().id());
        }
    }

    pub fn scheduling_guard_threads(
        &self,
    ) -> Option<(std::thread::ThreadId, std::thread::ThreadId)> {
        self.scheduling_threads
            .lock()
            .ok()
            .and_then(|threads| Some((threads.0?, threads.1?)))
    }

    pub fn scheduling_guard_acquired_thread(&self) -> Option<std::thread::ThreadId> {
        self.scheduling_threads
            .lock()
            .ok()
            .and_then(|threads| threads.0)
    }

    pub fn arm_startup_checkpoint(&self, checkpoint: StartupCheckpoint, occurrence: usize) {
        self.startup_checkpoint_reached
            .store(false, Ordering::Release);
        self.startup_checkpoint_released
            .store(false, Ordering::Release);
        self.startup_checkpoint_occurrence
            .store(occurrence.max(1), Ordering::Release);
        self.startup_checkpoint
            .store(checkpoint as u8, Ordering::Release);
    }

    pub fn panic_at_startup_checkpoint(&self, checkpoint: StartupCheckpoint) {
        self.panic_startup_checkpoint
            .store(checkpoint as u8, Ordering::Release);
    }

    pub fn wait_for_startup_checkpoint(&self, timeout: std::time::Duration) -> bool {
        let Ok(lock) = self.startup_checkpoint_lock.lock() else {
            return false;
        };
        let Ok((_guard, _result)) =
            self.startup_checkpoint_cv
                .wait_timeout_while(lock, timeout, |_| {
                    !self.startup_checkpoint_reached.load(Ordering::Acquire)
                })
        else {
            return false;
        };
        self.startup_checkpoint_reached.load(Ordering::Acquire)
    }

    pub fn release_startup_checkpoint(&self) {
        self.startup_checkpoint_released
            .store(true, Ordering::Release);
        self.startup_checkpoint_cv.notify_all();
    }

    #[cfg(test)]
    pub(crate) fn record_final_gate_focus_rejection_for_test(&self) {
        self.final_gate_focus_rejection_count
            .fetch_add(1, Ordering::AcqRel);
    }

    #[cfg(test)]
    pub(crate) fn final_gate_focus_rejection_count_for_test(&self) -> u64 {
        self.final_gate_focus_rejection_count
            .load(Ordering::Acquire)
    }

    pub(crate) fn visit_startup_checkpoint(&self, checkpoint: StartupCheckpoint) {
        if self
            .panic_startup_checkpoint
            .compare_exchange(checkpoint as u8, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            panic!("injected worker panic at startup checkpoint {checkpoint:?}");
        }
        if self.startup_checkpoint.load(Ordering::Acquire) != checkpoint as u8 {
            return;
        }
        let remaining = self.startup_checkpoint_occurrence.load(Ordering::Acquire);
        if remaining > 1 {
            self.startup_checkpoint_occurrence
                .fetch_sub(1, Ordering::AcqRel);
            return;
        }
        if self
            .startup_checkpoint
            .compare_exchange(checkpoint as u8, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        self.startup_checkpoint_reached
            .store(true, Ordering::Release);
        self.startup_checkpoint_cv.notify_all();
        let Ok(mut guard) = self.startup_checkpoint_lock.lock() else {
            return;
        };
        while !self.startup_checkpoint_released.load(Ordering::Acquire) {
            match self.startup_checkpoint_cv.wait(guard) {
                Ok(next) => guard = next,
                Err(_) => return,
            }
        }
    }

    fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub(crate) fn mark_first_physical_send_started(&self) {
        if self.first_physical_send_started.load(Ordering::SeqCst) == 0 {
            self.first_physical_send_started
                .store(self.next_sequence(), Ordering::SeqCst);
        }
    }

    #[cfg(test)]
    pub(crate) fn set_boot_delay_us(&self, delay_us: u64) {
        self.boot_delay_us.store(delay_us, Ordering::SeqCst);
    }

    pub fn last_down_sender_start_qpc_ticks(&self) -> u64 {
        self.down_sender_start_qpc_ticks.load(Ordering::Acquire)
    }

    #[cfg(all(test, feature = "test-support"))]
    pub(crate) fn last_down_sender_qpc_evidence(
        &self,
    ) -> (Option<u64>, Option<u64>, Option<u64>, Option<u64>) {
        let pre_call = self.down_sender_start_qpc_ticks.load(Ordering::Acquire);
        let authored_target = self
            .down_sender_authored_target_qpc_ticks
            .load(Ordering::Acquire);
        let packet_not_before = self
            .down_sender_packet_not_before_qpc_ticks
            .load(Ordering::Acquire);
        let wake = self.down_sender_wake_qpc_ticks.load(Ordering::Acquire);
        (
            (pre_call != 0).then_some(pre_call),
            (authored_target != 0).then_some(authored_target),
            (packet_not_before != 0).then_some(packet_not_before),
            (wake != 0).then_some(wake),
        )
    }

    pub fn down_sender_attempt_count(&self) -> u64 {
        self.down_sender_attempt_count.load(Ordering::Acquire)
    }

    /// Hold the real watchdog before its first age check for the delayed-watchdog
    /// acceptance case. The worker's final Down gate remains active.
    pub fn hold_watchdog(&self) {
        self.watchdog_hold.store(true, Ordering::Release);
    }

    pub fn watchdog_waiting(&self) -> bool {
        self.watchdog_waiting.load(Ordering::Acquire)
    }

    pub fn release_watchdog(&self) {
        self.watchdog_hold.store(false, Ordering::Release);
    }

    pub(crate) fn record_down_sender_attempt(
        &self,
        started_qpc_ticks: Option<u64>,
        authored_target_qpc_ticks: u64,
        packet_not_before_qpc_ticks: u64,
        wake_qpc_ticks: Option<u64>,
    ) {
        self.down_sender_attempt_count
            .fetch_add(1, Ordering::AcqRel);
        self.down_sender_start_qpc_ticks
            .store(started_qpc_ticks.unwrap_or(0), Ordering::Release);
        self.down_sender_authored_target_qpc_ticks
            .store(authored_target_qpc_ticks, Ordering::Release);
        self.down_sender_packet_not_before_qpc_ticks
            .store(packet_not_before_qpc_ticks, Ordering::Release);
        self.down_sender_wake_qpc_ticks
            .store(wake_qpc_ticks.unwrap_or(0), Ordering::Release);
    }

    pub(crate) fn hold_watchdog_before_check(&self) {
        if !self.watchdog_hold.load(Ordering::Acquire) {
            return;
        }
        self.watchdog_waiting.store(true, Ordering::Release);
    }

    pub fn watchdog_is_held(&self) -> bool {
        self.watchdog_hold.load(Ordering::Acquire)
    }

    pub fn enable_hol_stall(&self, duration_us: u64, target_mask: u16) {
        self.hol.enable_cycles(duration_us, target_mask, 1);
    }

    pub fn enable_hol_stall_cycles(&self, duration_us: u64, target_mask: u16, cycles: u32) {
        self.hol.enable_cycles(duration_us, target_mask, cycles);
    }

    pub fn hol_stall_evidence(&self) -> (u64, u64, u64, bool) {
        self.hol.evidence()
    }

    pub fn hol_stall_cycle_records(&self) -> Vec<hol::HolStallCycleRecord> {
        self.hol.cycle_records()
    }

    pub fn hol_clock_error(&self) -> bool {
        self.hol.clock_error()
    }

    pub(crate) fn maybe_inject_hol_stall(
        &self,
        down_mask: u16,
        source_action_index: u32,
        qpc_clock: sky_dispatch_win32::clock::QpcClock,
    ) {
        self.hol
            .maybe_inject_hol_stall(down_mask, source_action_index, qpc_clock);
    }
}

pub(crate) struct WorkerConfig {
    pub(super) backend: BackendConfig,
    pub(super) profile: DispatchProfile,
    pub(super) timing: TimingOptions,
    pub(super) focus: FocusOptions,
    pub(super) wait: WaitOptions,
    pub(super) telemetry: TelemetryOptions,
    pub(super) priority: PriorityOptions,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) startup_calibration_placement: StartupCalibrationPlacement,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) timing_observer_for_qualification: bool,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) prepared_packet_ambiguity_mask: Option<u16>,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) preflight_user_held_mask: Option<u16>,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) modifier_key_state_query_for_test: Option<Arc<dyn Fn(i32) -> i16 + Send + Sync>>,
}

impl WorkerConfig {
    #[cfg(test)]
    pub(crate) fn observer_enabled(&self) -> bool {
        let qualification_observer = self.timing_observer_for_qualification;
        effective_policy::observer_enabled_from_inputs(self.profile, qualification_observer)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            backend: BackendConfig::Production,
            profile: DispatchProfile::Production,
            timing: TimingOptions {
                game_fps: 60,
                min_hold_us: 10_000,
                frame_us: 16_667,
                frame_base_hold_us: 16_667,
                timing_margin_us: 0,
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
                enable_waitable_timer: true,
                enable_event_wait: true,
                supervisor_lease_timeout_us: 0,
                modifier_hold_timeout_us: DEFAULT_MODIFIER_HOLD_TIMEOUT_US,
                #[cfg(any(test, feature = "test-support"))]
                test_spin_threshold_us: None,
                #[cfg(any(test, feature = "test-support"))]
                test_wait_policy: TestWaitPolicy::LegacyTestWideSpin,
            },
            telemetry: TelemetryOptions {
                mode: TelemetryMode::Ring,
                capacity: 64,
            },
            priority: PriorityOptions {
                mode: PriorityMode::Off,
            },
            startup_calibration_placement: StartupCalibrationPlacement::BeforeSchedulingGuards,
            timing_observer_for_qualification: false,
            prepared_packet_ambiguity_mask: None,
            preflight_user_held_mask: None,
            modifier_key_state_query_for_test: None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TimingOptions {
    pub game_fps: u16,
    pub min_hold_us: u64,
    pub frame_us: u64,
    pub frame_base_hold_us: u64,
    pub timing_margin_us: u64,
    pub strict_timing: bool,
    pub strict_down_completion_late_us: u64,
    pub strict_up_completion_late_us: u64,
    pub input_path_warn_us: u64,
}

pub struct FocusOptions {
    pub require_focus: bool,
    pub focus_restore_grace_us: u64,
}

pub struct WaitOptions {
    pub enable_waitable_timer: bool,
    pub enable_event_wait: bool,
    pub supervisor_lease_timeout_us: u64,
    pub modifier_hold_timeout_us: u64,
    /// Test-only early handoff margin. This seam keeps mock-session tests
    /// independent from host timer overshoot without changing authored
    /// targets or the production wait policy.
    #[cfg(any(test, feature = "test-support"))]
    pub test_spin_threshold_us: Option<u64>,
    #[cfg(any(test, feature = "test-support"))]
    pub test_wait_policy: TestWaitPolicy,
}

impl Default for WaitOptions {
    fn default() -> Self {
        Self {
            enable_waitable_timer: true,
            enable_event_wait: true,
            supervisor_lease_timeout_us: DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US,
            modifier_hold_timeout_us: DEFAULT_MODIFIER_HOLD_TIMEOUT_US,
            #[cfg(any(test, feature = "test-support"))]
            test_spin_threshold_us: None,
            #[cfg(any(test, feature = "test-support"))]
            test_wait_policy: TestWaitPolicy::LegacyTestWideSpin,
        }
    }
}

impl WaitOptions {
    pub(crate) fn production_wait_policy(&self, backend_is_production: bool) -> bool {
        if backend_is_production {
            return true;
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            matches!(self.test_wait_policy, TestWaitPolicy::ProductionCalibrated)
        }
        #[cfg(not(any(test, feature = "test-support")))]
        {
            false
        }
    }

    pub(crate) fn requested_wait_policy_label(&self, backend_is_production: bool) -> &'static str {
        if backend_is_production {
            return "production_calibrated";
        }
        #[cfg(any(test, feature = "test-support"))]
        {
            self.test_wait_policy.label()
        }
        #[cfg(not(any(test, feature = "test-support")))]
        {
            "production_calibrated"
        }
    }
}

pub struct TelemetryOptions {
    pub mode: TelemetryMode,
    pub capacity: usize,
}

pub struct PriorityOptions {
    pub mode: PriorityMode,
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_SPIN_THRESHOLD_US, DispatchProfile, MIN_CALIBRATED_SPIN_US, TestWaitPolicy,
        WaitOptions, WorkerConfig,
    };

    #[test]
    fn production_wait_policy_has_bounded_spin_threshold() {
        assert_eq!(DEFAULT_SPIN_THRESHOLD_US, 1_000);
        assert_eq!(MIN_CALIBRATED_SPIN_US, 250);
        assert!(super::validate_timing_constants().is_ok());
    }

    #[test]
    fn production_profile_has_no_deferred_observer() {
        assert!(!DispatchProfile::Production.observer_enabled());
        assert!(DispatchProfile::StrictTimingDiagnostic.observer_enabled());
    }

    #[test]
    fn qualification_observer_preserves_production_dispatch_profile() {
        let mut config = WorkerConfig::default();
        assert_eq!(config.profile, DispatchProfile::Production);
        assert!(!config.observer_enabled());
        config.timing_observer_for_qualification = true;
        assert!(config.observer_enabled());
        assert_eq!(config.profile, DispatchProfile::Production);
    }

    #[test]
    fn production_calibrated_test_policy_has_no_legacy_spin_override() {
        let options = WaitOptions {
            enable_waitable_timer: true,
            enable_event_wait: true,
            supervisor_lease_timeout_us: 0,
            modifier_hold_timeout_us: super::DEFAULT_MODIFIER_HOLD_TIMEOUT_US,
            test_spin_threshold_us: None,
            test_wait_policy: TestWaitPolicy::ProductionCalibrated,
        };

        assert_eq!(options.test_wait_policy.label(), "production_calibrated");
        assert!(options.production_wait_policy(false));
        assert_eq!(options.test_spin_threshold_us, None);
    }
}
