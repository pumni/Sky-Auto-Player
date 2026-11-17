#[cfg(any(test, feature = "test-support"))]
use super::super::modifier_guard::observe_modifier_keys_with;
use super::super::modifier_guard::{ModifierKeyObservation, observe_current_modifier_keys};
#[cfg(any(test, feature = "test-support"))]
use super::super::outcome::{
    PacketRetryReason, PhysicalPacket, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
};
#[cfg(any(test, feature = "test-support"))]
use super::super::physical::InstrumentPhysicalState;
use super::super::profile::MaterializedInstrumentKeyProfile;
use super::{RELEASE_OWNER_SLOT_COUNT, TrackedKeyState};
use crate::clock::QpcClock;
use crate::input::ReleaseOwnerProof;
use std::fmt;

impl fmt::Debug for TrackedKeyState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrackedKeyState")
            .field("active_mask", &self.active_mask)
            .field("possibly_active_mask", &self.possibly_active_mask)
            .field("failed_release_mask", &self.failed_release_mask)
            .field("in_flight_mask", &self.in_flight_mask)
            .field("release_attempts_total", &self.release_attempts_total)
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

    /// Return every key mask that still carries a release obligation.
    pub fn release_obligation_mask(&self) -> u16 {
        self.active_mask
            | self.possibly_active_mask
            | self.failed_release_mask
            | self.in_flight_mask
    }

    /// Return the one frozen owner proof for a tracked cleanup mask. A mixed,
    /// missing, or conflicted origin fails closed rather than borrowing the
    /// currently published target's identity.
    pub fn release_owner_proof_for_mask(
        &self,
        mask: u16,
    ) -> Result<Option<ReleaseOwnerProof>, String> {
        if mask == 0 {
            return Ok(None);
        }
        if mask & !((1_u16 << RELEASE_OWNER_SLOT_COUNT) - 1) != 0 {
            return Err("tracked release mask contains a non-instrument slot".into());
        }
        let mut remaining = mask;
        let mut proof: Option<ReleaseOwnerProof> = None;
        while remaining != 0 {
            let slot = remaining.trailing_zeros() as usize;
            let bit = 1_u16 << slot;
            remaining &= !bit;
            #[cfg(any(test, feature = "test-support"))]
            super::TEST_RELEASE_OWNER_LEDGER_VISITS.increment();
            if self.release_owner_ledger.conflicted_mask & bit != 0 {
                return Err(format!(
                    "tracked release owner is ambiguous for slot {slot}"
                ));
            }
            let Some(slot_proof) = self.release_owner_ledger.owner_by_slot[slot].as_ref() else {
                return Err(format!(
                    "tracked release owner proof is missing for slot {slot}"
                ));
            };
            if let Some(existing) = proof.as_ref() {
                if !existing.same_owner(slot_proof) {
                    return Err("tracked release mask spans different owner generations".into());
                }
            } else {
                proof = Some(slot_proof.clone());
            }
        }
        Ok(proof)
    }

    /// Clear only the conservative journal residue created by one proven
    /// raw-zero Down-only attempt. The caller must classify the sender result
    /// before using this; unrelated active or uncertain slots are preserved.
    pub fn clear_proven_raw_zero_down_attempt(&mut self, down_mask: u16) -> Result<(), String> {
        if down_mask == 0
            || self.in_flight_mask != down_mask
            || self.possibly_active_mask & down_mask != down_mask
            || self.active_mask & down_mask != 0
            || self.failed_release_mask & down_mask != 0
        {
            return Err(format!(
                "proven-zero Down journal is inconsistent: down={down_mask:#06x}, active={:#06x}, possible={:#06x}, failed={:#06x}, in_flight={:#06x}",
                self.active_mask,
                self.possibly_active_mask,
                self.failed_release_mask,
                self.in_flight_mask,
            ));
        }
        self.possibly_active_mask &= !down_mask;
        self.in_flight_mask &= !down_mask;
        self.clear_unobligated_release_owners(down_mask);
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn reset_owner_ledger_visits_for_test(&self) {
        super::TEST_RELEASE_OWNER_LEDGER_VISITS.store(0, std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn owner_ledger_visits_for_test(&self) -> u64 {
        super::TEST_RELEASE_OWNER_LEDGER_VISITS.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn forget_release_owner_proof_for_test(&mut self, mask: u16) {
        let mut remaining = mask;
        while remaining != 0 {
            let slot = remaining.trailing_zeros() as usize;
            let bit = 1_u16 << slot;
            remaining &= !bit;
            if slot < RELEASE_OWNER_SLOT_COUNT {
                self.release_owner_ledger.owner_by_slot[slot] = None;
                self.release_owner_ledger.conflicted_mask &= !bit;
            }
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn conflict_release_owner_proof_for_test(&mut self, mask: u16) {
        self.release_owner_ledger.conflicted_mask |= mask;
    }

    pub(super) fn remember_release_owner_for_down(
        &mut self,
        down_mask: u16,
        proof: Option<&ReleaseOwnerProof>,
    ) {
        let mut remaining = down_mask;
        let obligations = self.release_obligation_mask();
        while remaining != 0 {
            let slot = remaining.trailing_zeros() as usize;
            let bit = 1_u16 << slot;
            remaining &= !bit;
            #[cfg(any(test, feature = "test-support"))]
            super::TEST_RELEASE_OWNER_LEDGER_VISITS.increment();
            if slot >= RELEASE_OWNER_SLOT_COUNT {
                self.release_owner_ledger.conflicted_mask |= bit;
                continue;
            }
            if obligations & bit == 0 {
                self.release_owner_ledger.owner_by_slot[slot] = proof.cloned();
                self.release_owner_ledger.conflicted_mask &= !bit;
                continue;
            }
            let same_owner = self.release_owner_ledger.owner_by_slot[slot]
                .as_ref()
                .zip(proof)
                .is_some_and(|(current, next)| current.same_owner(next));
            if !same_owner {
                self.release_owner_ledger.conflicted_mask |= bit;
            }
        }
    }

    pub(super) fn clear_unobligated_release_owners(&mut self, mask: u16) {
        let obligations = self.release_obligation_mask();
        let mut remaining = mask & ((1_u16 << RELEASE_OWNER_SLOT_COUNT) - 1);
        while remaining != 0 {
            let slot = remaining.trailing_zeros() as usize;
            let bit = 1_u16 << slot;
            remaining &= !bit;
            #[cfg(any(test, feature = "test-support"))]
            super::TEST_RELEASE_OWNER_LEDGER_VISITS.increment();
            if obligations & bit == 0 {
                self.release_owner_ledger.owner_by_slot[slot] = None;
                self.release_owner_ledger.conflicted_mask &= !bit;
            }
        }
    }

    /// Observe only fixed system modifiers at the final musical Down boundary.
    #[inline]
    pub fn observe_modifiers_before_down(&self) -> ModifierKeyObservation {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(query) = self.modifier_key_state_query.as_ref() {
            return observe_modifier_keys_with(query.as_ref());
        }
        observe_current_modifier_keys()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn panic_if_prepared_send_at(&self, point: super::PreparedSendPanicPoint) {
        if self.prepared_send_panic_point == Some(point) {
            panic!("test hook: prepared sender panic at {point:?}");
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_prepared_send_panic_point_for_test(
        &mut self,
        point: Option<super::PreparedSendPanicPoint>,
    ) {
        self.prepared_send_panic_point = point;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_prepared_packet_ambiguity_mask(&mut self, mask: u16) {
        self.prepared_packet_ambiguity_mask = Some(mask);
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

    #[cfg(test)]
    pub(crate) fn set_terminal_up_emitter_for_test<F>(&mut self, emitter: F)
    where
        F: Fn(PhysicalPacket) -> super::super::outcome::TerminalUpTransportResult
            + Send
            + Sync
            + 'static,
    {
        self.custom_terminal_up_emitter = Some(Box::new(emitter));
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
    pub fn set_physical_study_hook<F>(&mut self, hook: F)
    where
        F: Fn(u16) + Send + Sync + 'static,
    {
        self.physical_study_hook = Some(Box::new(hook));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn use_actual_pre_call_clock_for_test(&mut self) {
        self.use_actual_pre_call_clock_for_test = true;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn physical_study_final_policy_qpc_for_test(&self) -> Option<crate::clock::QpcTicks> {
        if self.use_actual_pre_call_clock_for_test {
            self.qpc_clock.and_then(|clock| clock.now().ok())
        } else {
            None
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_preflight_physical_mask_probe<F>(&mut self, probe: F)
    where
        F: Fn() -> u16 + Send + Sync + 'static,
    {
        self.preflight_physical_mask_probe = Some(Box::new(probe));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_modifier_key_state_query_for_test<F>(&mut self, query: F)
    where
        F: Fn(i32) -> i16 + Send + Sync + 'static,
    {
        self.modifier_key_state_query = Some(Box::new(query));
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_force_preflight_user_held_mask(&mut self, mask: u16) {
        self.force_preflight_user_held_mask = Some(mask);
    }

    pub fn with_qpc_clock(clock: QpcClock) -> Self {
        Self {
            qpc_clock: Some(clock),
            ..Default::default()
        }
    }

    pub fn with_qpc_clock_and_profile(
        clock: QpcClock,
        instrument_key_profile: MaterializedInstrumentKeyProfile,
    ) -> Self {
        Self {
            instrument_key_profile,
            qpc_clock: Some(clock),
            ..Default::default()
        }
    }

    pub fn with_profile(instrument_key_profile: MaterializedInstrumentKeyProfile) -> Self {
        Self {
            instrument_key_profile,
            ..Default::default()
        }
    }

    pub fn instrument_key_profile(&self) -> &MaterializedInstrumentKeyProfile {
        &self.instrument_key_profile
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
