use super::{
    ALL_GENERATION_STATUSES, CoordinatorError, CoordinatorInvariantError, GenerationAccounting,
    GenerationStatus, RuntimeDispatchCoordinator,
};
use crate::model::*;
use smallvec::SmallVec;

impl RuntimeDispatchCoordinator {
    pub fn is_finished(&self) -> bool {
        // This is a lifecycle predicate, not the success predicate. A
        // terminal backend/conflict/expired generation allows the worker to
        // stop, but the session must still reject OUTCOME_FINISHED unless
        // every generation is Released and every clean-completion counter is
        // zero.
        // An authored down may legitimately have no matching up in the input
        // timeline. The worker's terminal cleanup owns that case, so do not
        // wait forever on an active generation that has no authored release.
        let terminal_count = self.counters.released
            + self.counters.dropped_conflict
            + self.counters.dropped_backend
            + self.counters.dropped_expired
            + self.counters.cancelled;
        self.cursor >= self.schedule.batches.len()
            && self.active_mask == 0
            && self.pending_release_mask == 0
            && self.pending_release_by_slot.iter().all(Option::is_none)
            && self.cancelled_up_by_slot.iter().all(Option::is_none)
            && terminal_count == self.schedule.generation_count
    }

    /// Validate counters, masks, live owners, pending releases, and resumable
    /// cancellation tokens using bounded work over the fifteen instrument
    /// slots. This is the pause/resume critical-path proof; it deliberately
    /// does not scan the historical generation ledger.
    pub fn check_local_invariants(&self) -> Result<(), CoordinatorInvariantError> {
        if self.generation_states.len() as u64 != self.generation_count
            || self.generation_count != self.schedule.generation_count
        {
            return Err(CoordinatorInvariantError::Accounting(format!(
                "ledger length {} or schedule generation count {} != generation count {}",
                self.generation_states.len(),
                self.schedule.generation_count,
                self.generation_count
            )));
        }
        let terminal = self.counters.terminal_total_checked().ok_or_else(|| {
            CoordinatorInvariantError::Accounting("terminal counter overflow".to_string())
        })?;
        let active_count = u64::from(self.active_mask.count_ones());
        if self.activated_generation_count > self.generation_count {
            return Err(CoordinatorInvariantError::Accounting(
                "activated generation count exceeds total generation count".into(),
            ));
        }
        if self.counters.released > self.activated_generation_count {
            return Err(CoordinatorInvariantError::Accounting(
                "released generation count exceeds activated generation count".into(),
            ));
        }
        if active_count > self.activated_generation_count {
            return Err(CoordinatorInvariantError::Accounting(
                "active generation count exceeds activated generation count".into(),
            ));
        }
        let released_plus_active = self
            .counters
            .released
            .checked_add(active_count)
            .ok_or_else(|| {
                CoordinatorInvariantError::Accounting(
                    "released plus active generation count overflow".into(),
                )
            })?;
        if released_plus_active > self.activated_generation_count {
            return Err(CoordinatorInvariantError::Accounting(
                "released plus active generation count exceeds activated generation count".into(),
            ));
        }
        let classified = terminal.checked_add(active_count).ok_or_else(|| {
            CoordinatorInvariantError::Accounting(
                "active plus terminal generation count overflow".into(),
            )
        })?;
        if classified > self.generation_count {
            return Err(CoordinatorInvariantError::Accounting(
                "active plus terminal generation count exceeds total generation count".into(),
            ));
        }

        for slot in 0..MAX_KEYS {
            #[cfg(any(test, feature = "test-support"))]
            {
                let mut evidence = self.operation_evidence.get();
                evidence.local_invariant_slot_visits =
                    evidence.local_invariant_slot_visits.saturating_add(1);
                self.operation_evidence.set(evidence);
            }
            let key_slot = slot as KeySlot;
            let bit = Self::bit_for_slot(key_slot);
            let active = self.active_by_slot[slot].as_ref();
            let cancellation = self.cancelled_up_by_slot[slot];
            if active.is_some() && cancellation.is_some() {
                return Err(CoordinatorInvariantError::Accounting(format!(
                    "slot {slot} has both an active owner and a cancelled-Up token"
                )));
            }
            match active {
                Some(active) => {
                    if active.key_slot != key_slot
                        || self.active_mask & bit == 0
                        || self.blocked_mask & bit == 0
                        || self.pending_release_by_slot[slot].is_some_and(|pending| {
                            pending.generation_id != active.generation_id
                                || pending.key_slot != key_slot
                        })
                    {
                        return Err(CoordinatorInvariantError::Accounting(format!(
                            "active slot {slot} and ownership masks disagree"
                        )));
                    }
                    let Some(state) = self.generation_states.get(active.generation_id as usize)
                    else {
                        return Err(CoordinatorInvariantError::UnknownGeneration {
                            generation_id: active.generation_id,
                            generation_count: self.generation_count,
                        });
                    };
                    if *state != GenerationStatus::Active {
                        return Err(CoordinatorInvariantError::Accounting(format!(
                            "active slot {slot} points to non-active generation {}",
                            active.generation_id
                        )));
                    }
                }
                None if self.active_mask & bit != 0 || self.blocked_mask & bit != 0 => {
                    return Err(CoordinatorInvariantError::Accounting(format!(
                        "ownership mask for slot {slot} has no active slot owner"
                    )));
                }
                None => {}
            }

            let pending = self.pending_release_by_slot[slot];
            if pending.is_some() != (self.pending_release_mask & bit != 0) {
                return Err(CoordinatorInvariantError::Accounting(format!(
                    "pending slot {slot} and pending mask disagree"
                )));
            }
            if let Some(pending) = pending {
                if pending.key_slot != key_slot {
                    return Err(CoordinatorInvariantError::Accounting(format!(
                        "pending slot {slot} stores key slot {}",
                        pending.key_slot
                    )));
                }
                let Some(active) = active else {
                    return Err(CoordinatorInvariantError::Accounting(format!(
                        "pending slot {slot} has no active owner"
                    )));
                };
                if active.generation_id != pending.generation_id {
                    return Err(CoordinatorInvariantError::Accounting(format!(
                        "pending slot {slot} does not match active ownership"
                    )));
                }
            }

            if let Some(identity) = cancellation {
                if identity.key_slot() != key_slot
                    || pending.is_some()
                    || self.active_mask & bit != 0
                    || self.blocked_mask & bit != 0
                {
                    return Err(CoordinatorInvariantError::Accounting(format!(
                        "cancelled-Up token for slot {slot} conflicts with live ownership"
                    )));
                }
                self.validate_invalidated_up_identity(identity)?;
                let Some(state) = self
                    .generation_states
                    .get(identity.generation_id() as usize)
                else {
                    return Err(CoordinatorInvariantError::UnknownGeneration {
                        generation_id: identity.generation_id(),
                        generation_count: self.generation_count,
                    });
                };
                if *state != GenerationStatus::Cancelled {
                    return Err(CoordinatorInvariantError::Accounting(format!(
                        "cancelled-Up token for slot {slot} points to non-cancelled generation {}",
                        identity.generation_id()
                    )));
                }
            }
        }
        Ok(())
    }

    /// Verify the compact masks and terminal ledger agree exactly.
    pub fn check_invariants(&self) -> Result<(), CoordinatorInvariantError> {
        self.check_local_invariants()?;
        if self.generation_states.len() as u64 != self.generation_count {
            return Err(CoordinatorInvariantError::Accounting(format!(
                "ledger length {} != generation count {}",
                self.generation_states.len(),
                self.generation_count
            )));
        }

        let mut scheduled = 0u64;
        let mut active = 0u64;
        let mut terminal = 0u64;
        for state in &self.generation_states {
            #[cfg(any(test, feature = "test-support"))]
            {
                let mut evidence = self.operation_evidence.get();
                evidence.deep_audit_generation_visits =
                    evidence.deep_audit_generation_visits.saturating_add(1);
                self.operation_evidence.set(evidence);
            }
            match state {
                GenerationStatus::Scheduled => scheduled += 1,
                GenerationStatus::Active => active += 1,
                state => {
                    debug_assert!(state.is_terminal());
                    terminal += 1;
                }
            }
        }
        let counted_terminal = self.counters.terminal_total_checked().ok_or_else(|| {
            CoordinatorInvariantError::Accounting("terminal counter overflow".to_string())
        })?;
        if self.activated_generation_count > self.generation_count {
            return Err(CoordinatorInvariantError::Accounting(
                "activated generation count exceeds total generation count".to_string(),
            ));
        }
        if self.counters.released > self.activated_generation_count {
            return Err(CoordinatorInvariantError::Accounting(
                "released generation count exceeds activated generation count".to_string(),
            ));
        }
        if active > self.activated_generation_count {
            return Err(CoordinatorInvariantError::Accounting(
                "active generation count exceeds activated generation count".to_string(),
            ));
        }
        let released_plus_active = self.counters.released.checked_add(active).ok_or_else(|| {
            CoordinatorInvariantError::Accounting(
                "released plus active generation count overflow".to_string(),
            )
        })?;
        if released_plus_active > self.activated_generation_count {
            return Err(CoordinatorInvariantError::Accounting(
                "released plus active generation count exceeds activated generation count"
                    .to_string(),
            ));
        }
        if active != u64::from(self.active_mask.count_ones()) {
            return Err(CoordinatorInvariantError::Accounting(format!(
                "active ledger count {active} != active mask count {}",
                self.active_mask.count_ones()
            )));
        }
        if terminal != counted_terminal {
            return Err(CoordinatorInvariantError::Accounting(format!(
                "terminal ledger count {terminal} != counters {counted_terminal}"
            )));
        }
        let accounted = scheduled
            .checked_add(active)
            .and_then(|count| count.checked_add(terminal))
            .ok_or_else(|| {
                CoordinatorInvariantError::Accounting(
                    "scheduled, active, and terminal generation count overflow".to_string(),
                )
            })?;
        if accounted != self.generation_count {
            return Err(CoordinatorInvariantError::Accounting(format!(
                "scheduled + active + terminal {accounted} != generation count {}",
                self.generation_count
            )));
        }
        Ok(())
    }

    /// Return compact generation accounting without allocating or rebuilding
    /// a second lifecycle ledger.
    pub fn generation_accounting(&self) -> GenerationAccounting {
        let terminal = self
            .counters
            .terminal_total_checked()
            .expect("coordinator terminal counters must remain checked");
        let active = u64::from(self.active_mask.count_ones());
        let scheduled = self
            .generation_count
            .checked_sub(active)
            .and_then(|count| count.checked_sub(terminal))
            .expect("coordinator generation accounting must remain valid");
        GenerationAccounting {
            total: self.generation_count,
            activated: self.activated_generation_count,
            scheduled,
            active,
            released: self.counters.released,
            dropped_conflict: self.counters.dropped_conflict,
            dropped_backend: self.counters.dropped_backend,
            dropped_expired: self.counters.dropped_expired,
            cancelled: self.counters.cancelled,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn operation_evidence_for_test_support(&self) -> super::CoordinatorOperationEvidence {
        self.operation_evidence.get()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn reset_operation_evidence_for_test_support(&self) {
        self.operation_evidence
            .set(super::CoordinatorOperationEvidence::default());
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn cancelled_up_token_count_for_test_support(&self) -> usize {
        self.cancelled_up_by_slot
            .iter()
            .filter(|token| token.is_some())
            .count()
    }

    /// Verify the stronger state required after terminal backend cleanup.
    ///
    /// `check_invariants` proves that the ledger and masks agree; this method
    /// additionally proves that cleanup did not leave a live generation,
    /// blocked slot, or authored cursor behind.
    pub fn check_post_cleanup_invariants(&self) -> Result<(), CoordinatorInvariantError> {
        self.check_invariants()?;
        if self.active_mask != 0 || self.blocked_mask != 0 {
            return Err(CoordinatorInvariantError::Accounting(
                "terminal cleanup left a live coordinator mask".to_string(),
            ));
        }
        if self.active_by_slot.iter().any(Option::is_some) {
            return Err(CoordinatorInvariantError::Accounting(
                "terminal cleanup left a live coordinator slot".to_string(),
            ));
        }
        if self.pending_release_mask != 0
            || self.pending_release_by_slot.iter().any(Option::is_some)
        {
            return Err(CoordinatorInvariantError::Accounting(
                "terminal cleanup left a pending release".to_string(),
            ));
        }
        if self.cancelled_up_by_slot.iter().any(Option::is_some) {
            return Err(CoordinatorInvariantError::Accounting(
                "terminal cleanup left an unmatched cancelled-Up token".to_string(),
            ));
        }
        if self.generation_states.iter().any(|state| {
            matches!(
                state,
                GenerationStatus::Scheduled | GenerationStatus::Active
            )
        }) {
            return Err(CoordinatorInvariantError::Accounting(
                "terminal cleanup left a nonterminal generation".to_string(),
            ));
        }
        Ok(())
    }

    /// Build a `HashMap<String, u64>` generation status summary compatible with
    /// the existing Python/snapshot API. Counts come directly from the checked
    /// generation ledger, so no subtraction or saturating arithmetic can hide
    /// an accounting mismatch.
    ///
    /// No `HashMap` is touched during the hot dispatch loop; this method is only
    /// called at snapshot/telemetry publish time.
    pub fn generation_status_counts(&self) -> std::collections::HashMap<String, u64> {
        let mut result = std::collections::HashMap::with_capacity(ALL_GENERATION_STATUSES.len());
        for status in ALL_GENERATION_STATUSES {
            result.insert(status.as_str().to_string(), 0);
        }
        for state in &self.generation_states {
            *result
                .get_mut(state.as_str())
                .expect("all generation states have a summary bucket") += 1;
        }
        result
    }

    pub fn cancel_all(&mut self) -> Result<Vec<GenerationId>, CoordinatorError> {
        self.check_invariants()?;
        let cancelled_ids: SmallVec<[GenerationId; MAX_KEYS * 2]> = self
            .active_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .map(|active| active.generation_id)
            .collect();
        let mut sorted_cancelled: Vec<GenerationId> = cancelled_ids.into_vec();
        sorted_cancelled.sort_unstable();

        for index in 0..self.generation_states.len() {
            let state = self.generation_states[index];
            if matches!(
                state,
                GenerationStatus::Scheduled | GenerationStatus::Active
            ) {
                let generation_id = u64::try_from(index).map_err(|_| {
                    CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                        "generation ledger index does not fit GenerationId".to_string(),
                    ))
                })?;
                self.transition_generation(generation_id, state, GenerationStatus::Cancelled)?;
                self.invalidate_up_for_generation(generation_id);
            }
        }

        self.active_by_slot.fill(None);
        self.active_mask = 0;
        self.blocked_mask = 0;
        self.pending_release_by_slot.fill(None);
        self.pending_release_mask = 0;
        self.cancelled_up_by_slot.fill(None);

        self.check_invariants()?;

        Ok(sorted_cancelled)
    }

    /// Cancel only generations that currently own physical input state.
    ///
    /// The caller invokes this only after verified physical cleanup and a
    /// successfully established resume Down barrier. Live owners and their
    /// exact immutable authored Up locations are collected from the bounded
    /// fifteen-slot table; no historical generation scan or allocation is
    /// performed. Scheduled generations remain eligible for later playback.
    pub fn cancel_live_generations_after_verified_resumable_cleanup(
        &mut self,
    ) -> Result<usize, CoordinatorError> {
        self.check_local_invariants()
            .map_err(CoordinatorError::Invariant)?;
        let mut identities: [Option<super::AuthoredUpIdentity>; MAX_KEYS] = [None; MAX_KEYS];
        let mut live_count = 0usize;
        for (slot, identity_slot) in identities.iter_mut().enumerate() {
            #[cfg(any(test, feature = "test-support"))]
            {
                let mut evidence = self.operation_evidence.get();
                evidence.resumable_cancel_slot_visits =
                    evidence.resumable_cancel_slot_visits.saturating_add(1);
                self.operation_evidence.set(evidence);
            }
            let Some(active) = self.active_by_slot[slot].as_ref() else {
                continue;
            };
            let identity = self
                .authored_up_identity_for_live_generation(active.generation_id, active.key_slot)?;
            if self.cancelled_up_by_slot[slot].is_some() {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(format!(
                        "resumable cleanup would overwrite a live cancelled-Up token for slot {slot}"
                    )),
                ));
            }
            *identity_slot = Some(identity);
            live_count += 1;
        }
        let live_count_u64 = u64::try_from(live_count).map_err(|_| {
            CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                "active slot count does not fit generation counter".into(),
            ))
        })?;
        self.counters
            .cancelled
            .checked_add(live_count_u64)
            .ok_or_else(|| {
                CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                    "cancelled generation counter overflow during resumable cleanup".into(),
                ))
            })?;

        for (slot, identity) in identities.iter().copied().enumerate() {
            let Some(identity) = identity else {
                continue;
            };
            self.transition_generation(
                identity.generation_id(),
                GenerationStatus::Active,
                GenerationStatus::Cancelled,
            )?;
            self.invalidate_authored_up_for_resumable_cancellation(identity)?;
            self.cancelled_up_by_slot[slot] = Some(identity);
        }

        self.active_by_slot.fill(None);
        self.active_mask = 0;
        self.blocked_mask = 0;
        self.pending_release_by_slot.fill(None);
        self.pending_release_mask = 0;
        self.check_local_invariants()
            .map_err(CoordinatorError::Invariant)?;
        Ok(live_count)
    }
}
