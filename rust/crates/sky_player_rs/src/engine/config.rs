use super::telemetry::TelemetryMode;
use sky_dispatch_core::model::RuntimeSchedule;
use sky_dispatch_win32::mmcss::PriorityMode;

#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;
#[cfg(any(test, feature = "test-support"))]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(any(test, feature = "test-support"))]
use super::FaultInjectionScript;

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
    pub(crate) timing: TimingOptions,
    pub(crate) focus: FocusOptions,
    pub(crate) wait: WaitOptions,
    pub(crate) telemetry: TelemetryOptions,
    pub(crate) priority: PriorityOptions,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) startup_ordering_hook: Option<Arc<StartupOrderingHook>>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Default)]
pub(crate) struct StartupOrderingHook {
    sequence: AtomicU64,
    pub(crate) stale_packet_committed: AtomicU64,
    pub(crate) first_physical_send_started: AtomicU64,
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
}

pub(crate) struct WorkerConfig {
    pub(super) backend: BackendConfig,
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
            timing: TimingOptions {
                min_hold_us: 10_000,
                strict_timing: false,
                strict_down_completion_late_us: 2_000,
                strict_up_completion_late_us: 2_000,
                input_path_warn_us: 300,
                spin_threshold_us: 150,
                spin_floor_us: 700,
            },
            focus: FocusOptions {
                require_focus: false,
                focus_restore_grace_us: 100_000,
            },
            wait: WaitOptions {
                enable_waitable_timer: true,
                enable_event_wait: true,
                enable_adaptive_spin: false,
                supervisor_lease_timeout_us: 0,
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
    pub(crate) min_hold_us: u64,
    pub(crate) strict_timing: bool,
    pub(crate) strict_down_completion_late_us: u64,
    pub(crate) strict_up_completion_late_us: u64,
    pub(crate) input_path_warn_us: u64,
    pub(crate) spin_threshold_us: u64,
    pub(crate) spin_floor_us: u64,
}

pub(crate) struct FocusOptions {
    pub(crate) require_focus: bool,
    pub(crate) focus_restore_grace_us: u64,
}

pub(crate) struct WaitOptions {
    pub(crate) enable_waitable_timer: bool,
    pub(crate) enable_event_wait: bool,
    pub(crate) enable_adaptive_spin: bool,
    pub(crate) supervisor_lease_timeout_us: u64,
}

pub(crate) struct TelemetryOptions {
    pub(crate) mode: TelemetryMode,
    pub(crate) capacity: usize,
}

pub(crate) struct PriorityOptions {
    pub(crate) mode: PriorityMode,
}
