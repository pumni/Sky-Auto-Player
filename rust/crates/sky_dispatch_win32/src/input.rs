//! Windows SendInput API wrappers, input packet prewarming, and tracked key backend.

mod down_transaction;
mod outcome;
mod packet;
mod physical;
mod raw;
mod scan_code;
mod tracked;
mod up_transaction;

#[cfg(test)]
pub(crate) use down_transaction::emit_down_with;
pub use down_transaction::{emit_down, emit_down_once, emit_down_once_with};
pub use outcome::{
    PacketPreparationError, PacketRetryReason, PhysicalKeyPreflightError, PhysicalPacket,
    PlatformSendResult, ReleaseAllOutcome, SendEvidence, SendTransactionOutcome,
    SendTransactionStatus,
};

pub(crate) use packet::PreparedTaggedCalibrationPacket;
pub use packet::{
    MAX_PACKET_EVENTS, PreparedPacketView, PreparedPhysicalPacket,
    send_physical_packet_once_with_clock, send_prepared_physical_packet_once,
    send_prepared_physical_packet_once_at_target_with_cutoff,
    send_prepared_physical_packet_once_with_cutoff, send_prepared_physical_packet_once_with_start,
    send_prepared_physical_packet_once_with_start_and_cutoff,
    send_prepared_physical_packet_view_once,
    send_prepared_physical_packet_view_once_at_target_with_cutoff,
    send_prepared_physical_packet_view_once_with_cutoff,
    send_prepared_physical_packet_view_once_with_start,
    send_prepared_physical_packet_view_once_with_start_and_cutoff,
};
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
