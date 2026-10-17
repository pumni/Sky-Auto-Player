use super::telemetry::TelemetryMode;
use sky_dispatch_core::model::RuntimeSchedule;
use sky_dispatch_win32::input::{InstrumentKeyProfileSpec, MaterializedInstrumentKeyProfile};
use sky_dispatch_win32::mmcss::PriorityMode;

pub(crate) const STARTUP_READINESS_RESERVE_US: u64 = 2_000;
pub(crate) const DEFAULT_SPIN_THRESHOLD_US: u64 = 1_000;
pub(crate) const MIN_CALIBRATED_SPIN_US: u64 = 250;
pub(crate) const CALIBRATION_SAFETY_MARGIN_US: u64 = 50;
pub(crate) const CALIBRATION_SAMPLES: usize = 6;
pub(crate) const CALIBRATION_MAX_STARTUP_BUDGET_US: u64 = 20_000;
pub(crate) const MIN_PRODUCTION_PREROLL_US: u64 = 50_000;
/// Maximum interval for the non-realtime supervisor to prove it is still
/// alive. The direct desktop adapter consumes this same qualified safety
/// boundary.
pub const DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US: u64 = 3_000_000;

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
use std::sync::Arc;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering};

#[cfg(any(test, feature = "test-support"))]
use super::FaultInjectionScript;
#[cfg(any(test, feature = "test-support"))]
use super::SystemPowerState;

#[cfg(any(test, feature = "test-support"))]
pub type RestoreRaceHook =
    Arc<dyn Fn(&AtomicBool, &super::dispatch_primitives::SessionTarget) + Send + Sync>;

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
    pub schedule: RuntimeSchedule,
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
    #[cfg(any(test, feature = "test-support"))]
    pub restore_race_hook: Option<RestoreRaceHook>,
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
    pub(crate) stale_packet_committed: AtomicU64,
    pub(crate) first_physical_send_started: AtomicU64,
    pub(crate) boot_delay_us: AtomicU64,
    release_floor_control_enabled: AtomicBool,
    completed_up_count: AtomicU64,
    released_up_count: AtomicU64,
    resume_cleanup_qpc_ticks: AtomicU64,
    pause_ack_qpc_ticks: AtomicU64,
    pause_ack_pending: AtomicBool,
    down_authored_target_qpc_ticks: AtomicU64,
    down_not_before_qpc_ticks: AtomicU64,
    down_sender_start_qpc_ticks: AtomicU64,
    down_sender_attempt_count: AtomicU64,
    watchdog_hold: AtomicBool,
    watchdog_waiting: AtomicBool,
    startup_checkpoint: AtomicU8,
    startup_checkpoint_occurrence: AtomicUsize,
    startup_checkpoint_reached: AtomicBool,
    startup_checkpoint_released: AtomicBool,
    startup_checkpoint_lock: std::sync::Mutex<()>,
    startup_checkpoint_cv: std::sync::Condvar,
    panic_startup_checkpoint: AtomicU8,
    fail_worker_spawn: AtomicBool,
    fail_watchdog_spawn: AtomicBool,
    fail_observer_spawn: AtomicBool,
    fail_startup_epoch_conversion: AtomicBool,
    scheduling_threads:
        std::sync::Mutex<(Option<std::thread::ThreadId>, Option<std::thread::ThreadId>)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum StartupCheckpoint {
    StreamBuild = 1,
    CoordinatorReset = 2,
    BeforeReady = 3,
    ReadyToArmed = 4,
    EpochPrepared = 5,
    ArmedPublished = 6,
    InitialWait = 7,
    FirstDownAdmission = 8,
    StreamMaterialization = 9,
}

#[cfg(any(test, feature = "test-support"))]
impl StartupOrderingHook {
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

    pub(crate) fn mark_stale_packet_committed(&self) {
        self.stale_packet_committed
            .store(self.next_sequence(), Ordering::SeqCst);
    }

    pub(crate) fn mark_first_physical_send_started(&self) {
        if self.first_physical_send_started.load(Ordering::SeqCst) == 0 {
            self.first_physical_send_started
                .store(self.next_sequence(), Ordering::SeqCst);
        }
    }

    #[allow(dead_code)]
    pub(crate) fn set_boot_delay_us(&self, delay_us: u64) {
        self.boot_delay_us.store(delay_us, Ordering::SeqCst);
    }

    /// Enable the feature-gated release-barrier lifecycle fixture. The worker pauses after
    /// each successful Up until the acceptance controller requests pause and
    /// releases that exact completion boundary.
    pub fn enable_release_floor_control(&self, initial_worker_delay_us: u64) {
        self.boot_delay_us
            .store(initial_worker_delay_us, Ordering::SeqCst);
        self.release_floor_control_enabled
            .store(true, Ordering::Release);
    }

    pub fn completed_up_count(&self) -> u64 {
        self.completed_up_count.load(Ordering::Acquire)
    }

    pub fn release_completed_up_through(&self, count: u64) {
        self.released_up_count.store(count, Ordering::Release);
    }

    pub fn resume_cleanup_qpc_ticks(&self) -> u64 {
        self.resume_cleanup_qpc_ticks.load(Ordering::Acquire)
    }

    pub fn clear_pause_ack_qpc(&self) {
        self.pause_ack_qpc_ticks.store(0, Ordering::Release);
        self.pause_ack_pending.store(true, Ordering::Release);
    }

    pub fn pause_ack_qpc_ticks(&self) -> u64 {
        self.pause_ack_qpc_ticks.load(Ordering::Acquire)
    }

    pub fn last_down_window(&self) -> (u64, u64) {
        (
            self.down_authored_target_qpc_ticks.load(Ordering::Acquire),
            self.down_not_before_qpc_ticks.load(Ordering::Acquire),
        )
    }

    pub fn last_down_sender_start_qpc_ticks(&self) -> u64 {
        self.down_sender_start_qpc_ticks.load(Ordering::Acquire)
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

    pub(crate) fn record_resume_cleanup_qpc(&self, qpc_ticks: u64) {
        if self.release_floor_control_enabled.load(Ordering::Acquire) {
            self.resume_cleanup_qpc_ticks
                .store(qpc_ticks, Ordering::Release);
        }
    }

    pub(crate) fn record_pause_ack_qpc(&self, qpc_ticks: u64) {
        if self.release_floor_control_enabled.load(Ordering::Acquire) {
            self.pause_ack_qpc_ticks.store(qpc_ticks, Ordering::Release);
            self.pause_ack_pending.store(false, Ordering::Release);
        }
    }

    pub(crate) fn pause_ack_pending(&self) -> bool {
        self.release_floor_control_enabled.load(Ordering::Acquire)
            && self.pause_ack_pending.load(Ordering::Acquire)
    }

    pub(crate) fn record_down_window(&self, authored_target: u64, not_before: u64) {
        self.down_authored_target_qpc_ticks
            .store(authored_target, Ordering::Release);
        self.down_not_before_qpc_ticks
            .store(not_before, Ordering::Release);
    }

    pub(crate) fn record_down_sender_attempt(&self, started_qpc_ticks: Option<u64>) {
        self.down_sender_attempt_count
            .fetch_add(1, Ordering::AcqRel);
        self.down_sender_start_qpc_ticks
            .store(started_qpc_ticks.unwrap_or(0), Ordering::Release);
    }

    pub(crate) fn wait_after_successful_up(&self) {
        if !self.release_floor_control_enabled.load(Ordering::Acquire) {
            return;
        }
        let count = self.completed_up_count.fetch_add(1, Ordering::AcqRel) + 1;
        while self.released_up_count.load(Ordering::Acquire) < count {
            std::thread::sleep(std::time::Duration::from_micros(50));
        }
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
    pub(super) prepared_packet_ambiguity_mask: Option<u16>,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) preflight_user_held_mask: Option<u16>,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) modifier_key_state_query_for_test: Option<Arc<dyn Fn(i32) -> i16 + Send + Sync>>,
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
                min_release_gap_us: 16_667,
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
    pub min_release_gap_us: u64,
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
    /// Test-only early handoff margin. This seam keeps mock-session tests
    /// independent from host timer overshoot without changing authored
    /// targets or the production wait policy.
    #[cfg(any(test, feature = "test-support"))]
    pub test_spin_threshold_us: Option<u64>,
    #[cfg(any(test, feature = "test-support"))]
    pub test_wait_policy: TestWaitPolicy,
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
        WaitOptions,
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
    fn production_calibrated_test_policy_has_no_legacy_spin_override() {
        let options = WaitOptions {
            enable_waitable_timer: true,
            enable_event_wait: true,
            supervisor_lease_timeout_us: 0,
            test_spin_threshold_us: None,
            test_wait_policy: TestWaitPolicy::ProductionCalibrated,
        };

        assert_eq!(options.test_wait_policy.label(), "production_calibrated");
        assert!(options.production_wait_policy(false));
        assert_eq!(options.test_spin_threshold_us, None);
    }
}
