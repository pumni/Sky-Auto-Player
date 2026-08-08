//! Windows SendInput API wrappers, input packet prewarming, and tracked key backend.

mod down_transaction;
mod outcome;
mod packet;
mod physical;
mod raw;
mod scan_code;
mod tracked;
mod up_transaction;

pub use down_transaction::{emit_down, emit_down_with};
pub use outcome::{
    PacketRetryReason, PhysicalKeyPreflightError, PhysicalPacket, PlatformSendResult,
    ReleaseAllOutcome, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
};

pub use packet::{MAX_PACKET_EVENTS, send_physical_packet_with_clock};
pub use physical::is_scan_code_physically_down;
pub use raw::{send_input_raw, send_input_raw_with_clock};
pub use scan_code::{
    FULL_INSTRUMENT_MASK, PHYSICAL_INSTRUMENT_SCAN_CODES, SKY_PLAYER_SIGNATURE,
    scan_codes_from_mask,
};
pub use tracked::{ReleaseScope, TrackedKeyState};

#[cfg(any(test, feature = "test-support"))]
pub use physical::InstrumentPhysicalState;
#[cfg(any(test, feature = "test-support"))]
pub use tracked::{CustomEmitterFn, CustomPacketEmitterFn, CustomProbeFn};
pub use up_transaction::{emit_up, emit_up_with};

#[cfg(test)]
pub(crate) use up_transaction::emit_up_with_immediate;

#[cfg(test)]
mod tests;
