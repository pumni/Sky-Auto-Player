//! Pure Rust playback engine.
//!
//! This crate contains the authoritative scheduler/session implementation.
//! Native delivery uses this crate directly; the engine itself remains
//! independent of application UI stacks.

/// Narrow, typed native surface for delivery adapters.
///
/// Keep the small set of plan-building, timing, and schedule types
/// needed by the desktop adapter behind one typed facade.
pub mod adapter_support {
    pub use sky_dispatch_core::executor::{
        DispatchDecision, DispatchExecutor, PreparedChord, PreparedDispatchMemoryMetrics,
        PreparedDispatchSchedule,
    };
    pub use sky_dispatch_core::model::{
        CompileMemoryMetrics, MAX_KEYS, OnsetChord, OnsetChordInput, OnsetPlan,
        OnsetPlanMemoryMetrics,
    };
    pub use sky_dispatch_core::plan::{MAX_CHORDS, PlanError};
    pub use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
    pub use sky_dispatch_win32::clock::QpcError;
    pub use sky_dispatch_win32::clock::QpcTicks;
    pub use sky_dispatch_win32::input::PHYSICAL_INSTRUMENT_SCAN_CODES;
    pub use sky_dispatch_win32::mmcss::PriorityMode;

    pub const SCHEMA_VERSION: u32 = sky_dispatch_core::SCHEMA_VERSION;
    pub const MAX_RETAINED_DISPATCH_BYTES: usize =
        sky_dispatch_core::model::MAX_RETAINED_DISPATCH_BYTES;

    pub fn build_onset_plan(
        chords: &[OnsetChordInput],
        allowed_scan_codes: &[u16],
        hold_us: u64,
    ) -> Result<OnsetPlan, PlanError> {
        sky_dispatch_core::plan::build_onset_plan(chords, allowed_scan_codes, hold_us)
    }

    pub fn build_onset_plan_with_cancellation(
        chords: &[OnsetChordInput],
        allowed_scan_codes: &[u16],
        hold_us: u64,
        cancelled: impl FnMut() -> bool,
    ) -> Result<OnsetPlan, PlanError> {
        sky_dispatch_core::plan::build_onset_plan_with_cancellation(
            chords,
            allowed_scan_codes,
            hold_us,
            cancelled,
        )
    }

    pub fn build_onset_plan_with_memory_accounting(
        chords: &[OnsetChordInput],
        allowed_scan_codes: &[u16],
        hold_us: u64,
        input_vector_capacity: usize,
        caller_retained_capacity_bytes: usize,
        cancelled: impl FnMut() -> bool,
    ) -> Result<OnsetPlan, PlanError> {
        sky_dispatch_core::plan::build_onset_plan_with_memory_metrics(
            chords,
            allowed_scan_codes,
            hold_us,
            input_vector_capacity,
            caller_retained_capacity_bytes,
            cancelled,
        )
    }

    pub fn qpc_frequency_hz() -> Result<u64, QpcError> {
        sky_dispatch_win32::clock::qpc_frequency_checked()
    }

    pub fn win32_backend_available() -> bool {
        sky_dispatch_win32::win32_available()
    }
}

pub mod engine;
