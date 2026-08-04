use super::telemetry::TelemetryMode;
use sky_dispatch_core::model::RuntimeSchedule;
use sky_dispatch_win32::mmcss::PriorityMode;

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

pub(super) struct WorkerConfig {
    pub(super) schedule: RuntimeSchedule,
    pub(super) min_hold_us: u64,
    pub(super) max_lead_us: u64,
    pub(super) dispatch_lead_us: u64,
    pub(super) allowed_count: usize,
    pub(super) backend: BackendConfig,
    pub(super) require_focus: bool,
    pub(super) focus_restore_grace_us: u64,
    pub(super) spin_threshold_us: u64,
    pub(super) core_warmup_budget_us: u64,
    pub(super) telemetry_mode: TelemetryMode,
    pub(super) telemetry_capacity: usize,
    pub(super) priority_mode: PriorityMode,
    pub(super) enable_waitable_timer: bool,
    pub(super) enable_event_wait: bool,
    pub(super) enable_adaptive_spin: bool,
    pub(super) spin_floor_us: u64,
    pub(super) estimator_state_json: Option<String>,
    pub(super) enable_adaptive_lead: bool,
    pub(super) input_path_warn_us: u64,
    pub(super) strict_timing: bool,
    pub(super) strict_down_completion_late_us: u64,
    pub(super) strict_up_completion_late_us: u64,
    pub(super) supervisor_lease_timeout_us: u64,
}
