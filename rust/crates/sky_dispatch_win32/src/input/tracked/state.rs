#[cfg(any(test, feature = "test-support"))]
use super::super::outcome::{
    PacketRetryReason, PhysicalPacket, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
};
#[cfg(any(test, feature = "test-support"))]
use super::super::physical::InstrumentPhysicalState;
use super::TrackedKeyState;
use crate::clock::QpcClock;
use std::fmt;

impl fmt::Debug for TrackedKeyState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrackedKeyState")
            .field("active_mask", &self.active_mask)
            .field("possibly_active_mask", &self.possibly_active_mask)
            .field("failed_release_mask", &self.failed_release_mask)
            .field("last_error", &self.last_error)
            .field("keys_dropped", &self.keys_dropped)
            .field("chord_split_events", &self.chord_split_events)
            .field("sendinput_partial_events", &self.sendinput_partial_events)
            .field(
                "sendinput_zero_progress_failures",
                &self.sendinput_zero_progress_failures,
            )
            .field("chords_rejected", &self.chords_rejected)
            .field("authored_keys_rejected", &self.authored_keys_rejected)
            .field(
                "keys_inserted_before_failure",
                &self.keys_inserted_before_failure,
            )
            .field("keys_rolled_back", &self.keys_rolled_back)
            .field("rollback_residue_keys", &self.rollback_residue_keys)
            .field("timing_error", &self.timing_error)
            .field("qpc_clock_configured", &self.qpc_clock.is_some())
            .finish()
    }
}

impl TrackedKeyState {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_emitter<F>(emitter: F) -> Self
    where
        F: Fn(&[u16], bool) -> super::super::outcome::PlatformSendResult + Send + Sync + 'static,
    {
        Self {
            custom_emitter: Some(Box::new(emitter)),
            ..Default::default()
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_packet_emitter<F>(emitter: F) -> Self
    where
        F: Fn(PhysicalPacket) -> SendTransactionOutcome + Send + Sync + 'static,
    {
        Self {
            custom_packet_emitter: Some(Box::new(emitter)),
            ..Default::default()
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_emitter<F>(&mut self, emitter: F)
    where
        F: Fn(&[u16], bool) -> super::super::outcome::PlatformSendResult + Send + Sync + 'static,
    {
        self.custom_emitter = Some(Box::new(emitter));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_packet_emitter<F>(&mut self, emitter: F)
    where
        F: Fn(PhysicalPacket) -> SendTransactionOutcome + Send + Sync + 'static,
    {
        self.custom_packet_emitter = Some(Box::new(emitter));
    }

    /// Install deterministic success test emitters for both single scan code and packet paths.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_test_emitters(&mut self) {
        let clock = self.qpc_clock;
        self.custom_emitter = Some(Box::new(move |scan_codes, _key_up| {
            super::super::outcome::PlatformSendResult {
                requested: scan_codes.len() as u8,
                inserted: scan_codes.len() as u8,
                started_ticks: clock
                    .and_then(|c| c.now().ok())
                    .unwrap_or(crate::clock::QpcTicks::ZERO),
                completed_ticks: clock.and_then(|c| c.now().ok()),
                win32_error: 0,
                timing_error: None,
            }
        }));
        self.custom_packet_emitter = Some(Box::new(move |packet| SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: packet.up_mask | packet.down_mask,
                skipped_mask: 0,
                first_inserted: packet.event_count(),
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: clock.and_then(|c| c.now().ok()),
                completed_ticks: clock.and_then(|c| c.now().ok()),
                timing_error: None,
            },
        }));
    }

    /// Install a deterministic physical probe for the cleanup FSM.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_probe<F>(&mut self, probe: F)
    where
        F: Fn(u16, u16) -> InstrumentPhysicalState + Send + Sync + 'static,
    {
        self.custom_probe = Some(Box::new(probe));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_full_instrument_release_counter(
        &mut self,
        counter: std::sync::Arc<std::sync::atomic::AtomicU64>,
    ) {
        self.full_instrument_release_counter = Some(counter);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_force_preflight_failure(
        &mut self,
        flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) {
        self.force_preflight_failure = Some(flag);
    }

    pub fn with_qpc_clock(clock: QpcClock) -> Self {
        Self {
            qpc_clock: Some(clock),
            ..Default::default()
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn uses_custom_emitter(&self) -> bool {
        self.custom_emitter.is_some() || self.custom_packet_emitter.is_some()
    }

    #[cfg(not(any(test, feature = "test-support")))]
    pub(super) fn uses_custom_emitter(&self) -> bool {
        false
    }
}
