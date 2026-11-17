//! Windows SendInput API wrappers, input packet prewarming, and tracked key backend.

mod down_transaction;
mod modifier_guard;
mod outcome;
mod packet;
mod physical;
pub mod profile;
mod raw;
mod scan_code;
mod tracked;

#[cfg(feature = "test-support")]
pub mod physical_study;
mod up_transaction;

use std::sync::Arc;

#[cfg(any(test, feature = "test-support"))]
std::thread_local! {
    static RELEASE_OWNER_PROOF_ARC_CLONE_COUNT: std::cell::Cell<u64> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(any(test, feature = "test-support"))]
pub fn reset_release_owner_proof_arc_clone_count_for_test() {
    RELEASE_OWNER_PROOF_ARC_CLONE_COUNT.with(|count| count.set(0));
}

#[cfg(any(test, feature = "test-support"))]
pub fn release_owner_proof_arc_clone_count_for_test() -> u64 {
    RELEASE_OWNER_PROOF_ARC_CLONE_COUNT.with(std::cell::Cell::get)
}

/// Frozen target ownership evidence attached to a tracked release obligation.
/// Playback code keeps this value with the physical key journal so terminal
/// and resumable safety Up paths cannot borrow proof from a later target
/// generation. This is internal transport metadata, not a playback DTO.
pub struct ReleaseOwnerProof {
    target_hwnd: isize,
    target_generation: u64,
    owner_pid: Option<u32>,
    authority: Option<Arc<crate::focus::WindowIdentityAuthority>>,
}

impl Clone for ReleaseOwnerProof {
    fn clone(&self) -> Self {
        #[cfg(any(test, feature = "test-support"))]
        if self.authority.is_some() {
            RELEASE_OWNER_PROOF_ARC_CLONE_COUNT
                .with(|count| count.set(count.get().saturating_add(1)));
        }
        Self {
            target_hwnd: self.target_hwnd,
            target_generation: self.target_generation,
            owner_pid: self.owner_pid,
            authority: self.authority.as_ref().map(Arc::clone),
        }
    }
}

impl ReleaseOwnerProof {
    /// Build a fixed-size owner snapshot. A retained authority, when present,
    /// must describe the same HWND and PID recorded in this proof.
    pub fn new(
        target_hwnd: isize,
        target_generation: u64,
        owner_pid: Option<u32>,
        authority: Option<Arc<crate::focus::WindowIdentityAuthority>>,
    ) -> Option<Self> {
        if target_hwnd == 0 && (owner_pid.is_some() || authority.is_some()) {
            return None;
        }
        let authority_pid = authority.as_ref().map(|authority| {
            if authority.identity().hwnd == target_hwnd {
                authority.identity().owner_pid
            } else {
                0
            }
        });
        if authority_pid == Some(0)
            || authority_pid.is_some_and(|authority_pid| {
                owner_pid.is_some_and(|owner_pid| owner_pid != authority_pid)
            })
        {
            return None;
        }
        Some(Self {
            target_hwnd,
            target_generation,
            owner_pid: owner_pid.or(authority_pid),
            authority,
        })
    }

    pub fn target_hwnd(&self) -> isize {
        self.target_hwnd
    }

    pub fn target_generation(&self) -> u64 {
        self.target_generation
    }

    pub fn owner_pid(&self) -> Option<u32> {
        self.owner_pid
    }

    pub fn authority(&self) -> Option<&Arc<crate::focus::WindowIdentityAuthority>> {
        self.authority.as_ref()
    }

    /// Authority object identity matters even when HWND/PID values are equal:
    /// the process object is the proof against PID reuse and same-HWND ABA.
    pub fn same_owner(&self, other: &Self) -> bool {
        self.target_hwnd == other.target_hwnd
            && self.target_generation == other.target_generation
            && self.owner_pid == other.owner_pid
            && match (&self.authority, &other.authority) {
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                (None, None) => true,
                _ => false,
            }
    }
}

#[cfg(test)]
pub(crate) use down_transaction::emit_down_with;
pub use down_transaction::{emit_down, emit_down_once, emit_down_once_with};
pub use modifier_guard::{ModifierKeyObservation, ModifierMask};
pub use outcome::{
    PacketPreparationError, PacketRetryReason, PhysicalKeyPreflightError, PhysicalPacket,
    PlatformSendResult, ReleaseAllOutcome, SendEvidence, SendTransactionOutcome,
    SendTransactionStatus, TerminalProbeVerdict, TerminalReleaseEvidence,
    TerminalSendAttemptEvidence,
};

pub(crate) use packet::PreparedTaggedCalibrationPacket;
pub use packet::{
    MAX_PACKET_EVENTS, PreparedPacketView, PreparedPhysicalPacket,
    send_physical_packet_once_with_clock, send_prepared_physical_packet_once,
    send_prepared_physical_packet_once_at_target, send_prepared_physical_packet_once_with_start,
    send_prepared_physical_packet_view_once, send_prepared_physical_packet_view_once_at_target,
    send_prepared_physical_packet_view_once_with_start,
};
pub use physical::is_scan_code_physically_down;
pub use profile::{
    InstrumentKeyProfile, InstrumentKeyProfileError, InstrumentKeyProfileSpec,
    MaterializedInstrumentKeyProfile, PhysicalKey, SUPPORTED_NON_EXTENDED_SCAN_CODES,
};
pub use raw::{send_input_raw, send_input_raw_with_clock};
pub use scan_code::{
    FULL_INSTRUMENT_MASK, PHYSICAL_INSTRUMENT_SCAN_CODES, SKY_PLAYER_SIGNATURE,
    scan_codes_from_mask,
};
pub use tracked::{ReleaseScope, TrackedKeyState};

#[cfg(any(test, feature = "test-support"))]
pub use physical::InstrumentPhysicalState;
#[cfg(any(test, feature = "test-support"))]
pub use tracked::{CustomEmitterFn, CustomPacketEmitterFn, CustomProbeFn, PreparedSendPanicPoint};
pub use up_transaction::{emit_up, emit_up_with};

#[cfg(test)]
pub(crate) use up_transaction::emit_up_with_immediate;

#[cfg(test)]
mod tests;
