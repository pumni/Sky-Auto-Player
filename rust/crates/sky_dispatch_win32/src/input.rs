//! Windows SendInput API wrappers, input packet prewarming, and tracked key backend.

mod outcome;
mod physical;
mod raw;
mod scan_code;
mod tracked;

pub use outcome::{
    DownSendOutcome, EmitResult, InputSendResult, PhysicalKeyPreflightError, PlatformSendResult,
    ReleaseAllOutcome,
};
pub use physical::is_scan_code_physically_down;
pub use raw::{emit_down, emit_down_with, send_input_raw, send_input_raw_with_clock};
pub use scan_code::{FULL_INSTRUMENT_MASK, PHYSICAL_INSTRUMENT_SCAN_CODES, SKY_PLAYER_SIGNATURE};
#[cfg(any(test, feature = "test-support"))]
pub use tracked::CustomEmitterFn;
pub use tracked::{TrackedKeyState, emit_up, emit_up_with};

#[cfg(test)]
pub(crate) use tracked::emit_up_with_immediate;

#[cfg(test)]
mod tests;
