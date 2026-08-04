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

    pub(crate) fn strict_timing(self) -> bool {
        matches!(self, Self::StrictTimingDiagnostic)
    }
}
pub(crate) struct WorkerConfig {
    pub(crate) schedule: sky_dispatch_core::model::RuntimeSchedule,
    pub(crate) min_hold_us: u64,
    pub(crate) max_lead_us: u64,
    pub(crate) dispatch_lead_us: u64,
    pub(crate) allowed_count: usize,
    pub(crate) mock_backend: bool,
    pub(crate) mock_latency_base_us: u64,
    pub(crate) mock_latency_per_key_us: u64,
    pub(crate) fault_script: crate::engine::test_support::fault_injection::FaultInjectionScript,
    pub(crate) require_focus: bool,
    pub(crate) focus_restore_grace_us: u64,
    pub(crate) spin_threshold_us: u64,
    pub(crate) core_warmup_budget_us: u64,
    pub(crate) telemetry_mode: TelemetryMode,
    pub(crate) telemetry_capacity: usize,
    pub(crate) priority_mode: sky_dispatch_win32::mmcss::PriorityMode,
    pub(crate) enable_waitable_timer: bool,
    pub(crate) enable_event_wait: bool,
    pub(crate) enable_adaptive_spin: bool,
    pub(crate) spin_floor_us: u64,
    pub(crate) estimator_state_json: Option<String>,
    pub(crate) enable_adaptive_lead: bool,
    pub(crate) input_path_warn_us: u64,
    pub(crate) strict_timing: bool,
    pub(crate) strict_down_completion_late_us: u64,
    pub(crate) strict_up_completion_late_us: u64,
    pub(crate) supervisor_lease_timeout_us: u64,
}
