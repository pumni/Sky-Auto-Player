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
            && terminal_count == self.schedule.generation_count
    }

    /// Verify the compact masks and terminal ledger agree exactly.
    pub fn check_invariants(&self) -> Result<(), CoordinatorInvariantError> {
        if self.generation_states.len() as u64 != self.generation_count {
            return Err(CoordinatorInvariantError::Accounting(format!(
                "ledger length {} != generation count {}",
                self.generation_states.len(),
                self.generation_count
            )));
        }

        let scheduled = self
            .generation_states
            .iter()
            .filter(|state| **state == GenerationStatus::Scheduled)
            .count() as u64;
        let active = self
            .generation_states
            .iter()
            .filter(|state| **state == GenerationStatus::Active)
            .count() as u64;
        let terminal = self
            .generation_states
            .iter()
            .filter(|state| state.is_terminal())
            .count() as u64;
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
        for slot in 0..MAX_KEYS {
            let bit = Self::bit_for_slot(slot as KeySlot);
            let pending = self.pending_release_by_slot[slot];
            if pending.is_some() != (self.pending_release_mask & bit != 0) {
                return Err(CoordinatorInvariantError::Accounting(format!(
                    "pending slot {slot} and pending mask disagree"
                )));
            }
            let Some(pending) = pending else {
                continue;
            };
            if pending.key_slot != slot as KeySlot {
                return Err(CoordinatorInvariantError::Accounting(format!(
                    "pending slot {slot} stores key slot {}",
                    pending.key_slot
                )));
            }
            let Some(active) = self.active_by_slot[slot].as_ref() else {
                return Err(CoordinatorInvariantError::Accounting(format!(
                    "pending slot {slot} has no active owner"
                )));
            };
            if active.generation_id != pending.generation_id
                || self.active_mask & bit == 0
                || self.blocked_mask & bit == 0
            {
                return Err(CoordinatorInvariantError::Accounting(format!(
                    "pending slot {slot} does not match active ownership"
                )));
            }
            let Some(state) = self.generation_states.get(pending.generation_id as usize) else {
                return Err(CoordinatorInvariantError::UnknownGeneration {
                    generation_id: pending.generation_id,
                    generation_count: self.generation_count,
                });
            };
            if *state != GenerationStatus::Active {
                return Err(CoordinatorInvariantError::Accounting(format!(
                    "pending slot {slot} points to non-active generation {}",
                    pending.generation_id
                )));
            }
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

        self.check_invariants()?;

        Ok(sorted_cancelled)
    }

    /// Cancel only generations that currently own physical input state.
    ///
    /// Authored generations that have not been dispatched remain Scheduled,
    /// so a focus/manual suspension can resume the immutable authored cursor
    /// without ever attempting `Cancelled -> Active`.
    pub fn cancel_live_generations(&mut self) -> Result<Vec<GenerationId>, CoordinatorError> {
        self.check_invariants()?;
        let mut cancelled_ids: SmallVec<[GenerationId; MAX_KEYS * 2]> = self
            .generation_states
            .iter()
            .enumerate()
            .filter_map(|(index, state)| matches!(state, GenerationStatus::Active).then_some(index))
            .map(|index| {
                GenerationId::try_from(index).map_err(|_| {
                    CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                        "generation ledger index does not fit GenerationId".to_string(),
                    ))
                })
            })
            .collect::<Result<SmallVec<_>, _>>()?;
        cancelled_ids.sort_unstable();

        for generation_id in cancelled_ids.iter().copied() {
            let state = *self.generation_states.get(generation_id as usize).ok_or(
                CoordinatorError::Invariant(CoordinatorInvariantError::UnknownGeneration {
                    generation_id,
                    generation_count: self.generation_count,
                }),
            )?;
            self.transition_generation(generation_id, state, GenerationStatus::Cancelled)?;
            self.invalidate_up_for_generation(generation_id);
        }

        self.active_by_slot.fill(None);
        self.active_mask = 0;
        self.blocked_mask = 0;
        self.pending_release_by_slot.fill(None);
        self.pending_release_mask = 0;
        self.check_invariants()?;

        Ok(cancelled_ids.into_vec())
    }
}
