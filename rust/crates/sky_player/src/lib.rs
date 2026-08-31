//! Pure Rust playback engine.
//!
//! This crate contains the authoritative scheduler/session implementation.
//! Delivery adapters such as the temporary `sky_player_rs` wheel depend on
//! it, while the engine itself remains independent of Python and UI stacks.

/// Narrow, typed native surface for delivery adapters.
///
/// The Python wheel is a temporary adapter, not a second owner of dispatch
/// policy.  Keep the small set of compilation, timing, calibration, and
/// schedule types it needs behind this facade so the adapter does not depend
/// directly on either dispatch crate.
pub mod adapter_support {
    pub use sky_dispatch_core::compile::{CompileError, MAX_ACTIONS, MAX_REASON_BYTES};
    pub use sky_dispatch_core::model::{ActionKind, KeyActionInput, MAX_KEYS, RuntimeSchedule};
    pub use sky_dispatch_core::validation::ScheduleTimingError;
    pub use sky_dispatch_win32::calibration::{
        CALIBRATION_SCHEMA_VERSION, CalibrationConfig, CalibrationError, HostFingerprint,
        run_calibration_json,
    };
    pub use sky_dispatch_win32::clock::QpcError;
    pub use sky_dispatch_win32::input::PHYSICAL_INSTRUMENT_SCAN_CODES;
    pub use sky_dispatch_win32::mmcss::PriorityMode;

    pub const SCHEMA_VERSION: u32 = sky_dispatch_core::SCHEMA_VERSION;

    pub fn compile_runtime_intents(
        actions: &[KeyActionInput],
        allowed_scan_codes: &[u16],
    ) -> Result<RuntimeSchedule, CompileError> {
        sky_dispatch_core::compile::compile_runtime_intents(actions, allowed_scan_codes)
    }

    pub fn validate_min_hold_and_release_gap_feasibility(
        schedule: &RuntimeSchedule,
        effective_min_hold_us: u64,
        min_release_gap_us: u64,
    ) -> Result<(), ScheduleTimingError> {
        sky_dispatch_core::validation::validate_min_hold_and_release_gap_feasibility(
            schedule,
            effective_min_hold_us,
            min_release_gap_us,
        )
    }

    pub fn build_host_fingerprint() -> Result<HostFingerprint, CalibrationError> {
        sky_dispatch_win32::calibration::build_host_fingerprint()
    }

    pub fn qpc_frequency_hz() -> Result<u64, QpcError> {
        sky_dispatch_win32::clock::qpc_frequency_checked()
    }

    pub fn win32_backend_available() -> bool {
        sky_dispatch_win32::win32_available()
    }
}

pub mod engine;
