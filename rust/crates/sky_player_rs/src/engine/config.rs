use super::telemetry::TelemetryMode;
use sky_dispatch_core::model::RuntimeSchedule;
use sky_dispatch_win32::mmcss::PriorityMode;

pub(crate) const DEFAULT_ADMISSION_GUARD_US: u64 = 2_000;
pub(crate) const DEFAULT_SPIN_THRESHOLD_US: u64 = 700;
pub(crate) const MIN_PRODUCTION_PREROLL_US: u64 = 50_000;

pub(crate) fn validate_timing_constants() -> Result<(), String> {
    if DEFAULT_ADMISSION_GUARD_US <= DEFAULT_SPIN_THRESHOLD_US {
        return Err("admission guard must be greater than spin threshold".to_string());
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

    pub(crate) fn strict_timing(self) -> bool {
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

pub(crate) enum BackendConfig {
    Production,
    #[cfg(any(test, feature = "test-support"))]
    Mock {
        latency_base_us: u64,
        latency_per_key_us: u64,
        fault_script: FaultInjectionScript,
    },
}

pub(crate) struct NativeSessionOptions {
    pub(crate) schedule: RuntimeSchedule,
    pub(crate) backend: BackendConfig,
    pub(crate) profile: DispatchProfile,
    pub(crate) timing: TimingOptions,
    pub(crate) focus: FocusOptions,
    pub(crate) wait: WaitOptions,
    pub(crate) telemetry: TelemetryOptions,
    pub(crate) priority: PriorityOptions,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) startup_ordering_hook: Option<Arc<StartupOrderingHook>>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) restore_race_hook: Option<RestoreRaceHook>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
pub(crate) struct StartupOrderingHook {
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

pub(crate) struct TimingOptions {
    pub(crate) game_fps: u16,
    pub(crate) min_hold_us: u64,
    pub(crate) down_late_grace_us: u64,
    pub(crate) strict_timing: bool,
    pub(crate) strict_down_completion_late_us: u64,
    pub(crate) strict_up_completion_late_us: u64,
    pub(crate) input_path_warn_us: u64,
}

pub(crate) struct FocusOptions {
    pub(crate) require_focus: bool,
    pub(crate) focus_restore_grace_us: u64,
}

pub(crate) struct WaitOptions {
    pub(crate) enable_waitable_timer: bool,
    pub(crate) enable_event_wait: bool,
    pub(crate) supervisor_lease_timeout_us: u64,
    /// Test-only early handoff margin. Production always uses the fixed
    /// `DEFAULT_SPIN_THRESHOLD_US` value; this seam keeps mock-session tests
    /// independent from host timer overshoot without changing authored
    /// targets or the production wait policy.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) test_spin_threshold_us: Option<u64>,
}

pub(crate) struct TelemetryOptions {
    pub(crate) mode: TelemetryMode,
    pub(crate) capacity: usize,
}

pub(crate) struct PriorityOptions {
    pub(crate) mode: PriorityMode,
}

#[cfg(test)]
mod tests {
    use super::DispatchProfile;

    #[test]
    fn production_profile_has_no_deferred_observer() {
        assert!(!DispatchProfile::Production.observer_enabled());
        assert!(DispatchProfile::StrictTimingDiagnostic.observer_enabled());
    }
}
