use super::telemetry::TelemetryMode;
use sky_dispatch_core::model::RuntimeSchedule;
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
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

#[cfg(any(test, feature = "test-support"))]
use super::FaultInjectionScript;

#[cfg(any(test, feature = "test-support"))]
pub(crate) type RestoreRaceHook = Arc<dyn Fn(&AtomicBool, &AtomicIsize, &AtomicU64) + Send + Sync>;

#[cfg(any(test, feature = "test-support"))]
pub(crate) type FinalGateRaceHook = Arc<
    dyn Fn(
            &AtomicBool,
            &AtomicIsize,
            &AtomicU64,
            &AtomicBool,
            &AtomicBool,
            &AtomicBool,
            &AtomicBool,
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
    #[cfg(any(test, feature = "test-support"))]
    pub startup_ordering_hook: Option<Arc<StartupOrderingHook>>,
    #[cfg(any(test, feature = "test-support"))]
    pub restore_race_hook: Option<RestoreRaceHook>,
    #[cfg(any(test, feature = "test-support"))]
    pub timer_lifecycle_context:
        Option<sky_dispatch_win32::timer::test_support::TimerLifecycleContext>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
pub struct StartupOrderingHook {
    sequence: AtomicU64,
    pub(crate) stale_packet_committed: AtomicU64,
    pub(crate) first_physical_send_started: AtomicU64,
    pub(crate) boot_delay_us: AtomicU64,
}

#[cfg(any(test, feature = "test-support"))]
impl StartupOrderingHook {
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
}

pub(crate) struct WorkerConfig {
    pub(super) backend: BackendConfig,
    pub(super) profile: DispatchProfile,
    pub(super) timing: TimingOptions,
    pub(super) focus: FocusOptions,
    pub(super) wait: WaitOptions,
    pub(super) telemetry: TelemetryOptions,
    pub(super) priority: PriorityOptions,
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
                // Production's public default is 500 us. Test-support
                // sessions supply their own explicit timing policy.
                down_late_grace_us: 500,
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
        }
    }
}

pub struct TimingOptions {
    pub game_fps: u16,
    pub min_hold_us: u64,
    pub min_release_gap_us: u64,
    pub down_late_grace_us: u64,
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
