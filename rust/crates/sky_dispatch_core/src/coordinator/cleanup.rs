use super::{
    ALL_GENERATION_STATUSES, CoordinatorError, CoordinatorInvariantError, GenerationStatus,
    RuntimeDispatchCoordinator,
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
        // timeline.  The worker's terminal cleanup owns that case, so do not
        // wait forever on an active generation that has no pending release.
        // Failed pending releases are kept alive by `requeue_failed_releases`
        // until they succeed or recovery aborts the session.
        let terminal_count = self.counters.released
            + self.counters.dropped_conflict
            + self.counters.dropped_backend
            + self.counters.dropped_expired
            + self.counters.cancelled;
        self.cursor >= self.schedule.batches.len()
            && self.active_mask == 0
            && self.pending_mask == 0
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

        let mut active = 0u64;
        let mut release_pending = 0u64;
        let mut terminal = 0u64;
        for state in &self.generation_states {
            match state {
                GenerationStatus::Active => active += 1,
                GenerationStatus::ReleasePending => release_pending += 1,
                GenerationStatus::Released
                | GenerationStatus::DroppedConflict
                | GenerationStatus::DroppedBackend
                | GenerationStatus::DroppedExpired
                | GenerationStatus::Cancelled => terminal += 1,
                GenerationStatus::Scheduled => {}
            }
        }
        if active != u64::from(self.active_mask.count_ones()) {
            return Err(CoordinatorInvariantError::Accounting(format!(
                "active ledger count {active} != active mask count {}",
                self.active_mask.count_ones()
            )));
        }
        if release_pending != u64::from(self.pending_mask.count_ones()) {
            return Err(CoordinatorInvariantError::Accounting(format!(
                "pending ledger count {release_pending} != pending mask count {}",
                self.pending_mask.count_ones()
            )));
        }
        if self.active_mask & self.pending_mask != 0 {
            return Err(CoordinatorInvariantError::Accounting(
                "active and pending masks overlap".to_string(),
            ));
        }
        if terminal != self.counters.terminal_total() {
            return Err(CoordinatorInvariantError::Accounting(format!(
                "terminal ledger count {terminal} != counters {}",
                self.counters.terminal_total()
            )));
        }
        Ok(())
    }

    /// Verify the stronger state required after terminal backend cleanup.
    ///
    /// `check_invariants` proves that the ledger and masks agree; this method
    /// additionally proves that cleanup did not leave a live generation,
    /// pending slot, blocked slot, or authored cursor behind.
    pub fn check_post_cleanup_invariants(&self) -> Result<(), CoordinatorInvariantError> {
        self.check_invariants()?;
        if self.active_mask != 0 || self.pending_mask != 0 || self.blocked_mask != 0 {
            return Err(CoordinatorInvariantError::Accounting(
                "terminal cleanup left a live coordinator mask".to_string(),
            ));
        }
        if self.active_by_slot.iter().any(Option::is_some)
            || self.pending_by_slot.iter().any(Option::is_some)
        {
            return Err(CoordinatorInvariantError::Accounting(
                "terminal cleanup left a live coordinator slot".to_string(),
            ));
        }
        if self.release_recovery_started_ticks.is_some() {
            return Err(CoordinatorInvariantError::Accounting(
                "terminal cleanup left release recovery state".to_string(),
            ));
        }
        if self.generation_states.iter().any(|state| {
            matches!(
                state,
                GenerationStatus::Scheduled
                    | GenerationStatus::Active
                    | GenerationStatus::ReleasePending
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
        let mut cancelled_ids: SmallVec<[GenerationId; MAX_KEYS * 2]> = self
            .active_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .map(|active| active.generation_id)
            .collect();
        for pending_id in self
            .pending_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .map(|pending| pending.generation_id)
        {
            if !cancelled_ids.contains(&pending_id) {
                cancelled_ids.push(pending_id);
            }
        }

        let mut sorted_cancelled: Vec<GenerationId> = cancelled_ids.into_vec();
        sorted_cancelled.sort_unstable();

        for index in 0..self.generation_states.len() {
            let state = self.generation_states[index];
            if matches!(
                state,
                GenerationStatus::Scheduled
                    | GenerationStatus::Active
                    | GenerationStatus::ReleasePending
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
        self.pending_by_slot.fill(None);
        self.active_mask = 0;
        self.blocked_mask = 0;
        self.pending_mask = 0;
        self.release_recovery_started_ticks = None;

        self.check_invariants()?;

        Ok(sorted_cancelled)
    }

    /// Cancel only generations that currently own physical input state.
    ///
    /// Authored generations that have not been dispatched remain Scheduled,
    /// so a focus/manual suspension can resume the immutable authored cursor
    /// without ever attempting `Cancelled -> Active`.
    pub fn cancel_live_generations(&mut self) -> Result<Vec<GenerationId>, CoordinatorError> {
        let mut cancelled_ids: SmallVec<[GenerationId; MAX_KEYS * 2]> = self
            .generation_states
            .iter()
            .enumerate()
            .filter_map(|(index, state)| {
                matches!(
                    state,
                    GenerationStatus::Active | GenerationStatus::ReleasePending
                )
                .then_some(index)
            })
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
        self.pending_by_slot.fill(None);
        self.active_mask = 0;
        self.blocked_mask = 0;
        self.pending_mask = 0;
        self.release_recovery_started_ticks = None;
        self.check_invariants()?;

        Ok(cancelled_ids.into_vec())
    }
}
