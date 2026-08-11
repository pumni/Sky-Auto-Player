use super::{
    CoordinatorError, CoordinatorInvariantError, GenerationStatus, PendingDispatchPlan,
    PendingRelease, RuntimeDispatchCoordinator,
};
use crate::model::*;
use crate::time::{DurationTicks, TimelineTicks};
use smallvec::SmallVec;

impl RuntimeDispatchCoordinator {
    #[cfg(test)]
    pub fn next_pending_release_us(&self, lead_up: u64) -> Option<u64> {
        if self.pending_mask == 0 {
            return None;
        }
        self.pending_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .map(|pending| pending.get_effective_release_us(lead_up))
            .min()
    }

    pub fn next_pending_release_ticks(
        &self,
        _lead_up: DurationTicks,
    ) -> Result<Option<TimelineTicks>, CoordinatorError> {
        self.pending_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .map(|pending| pending.get_effective_release_ticks(DurationTicks::ZERO))
            .try_fold(None::<TimelineTicks>, |current, value| {
                let value = value?;
                Ok(Some(current.map_or(value, |best| best.min(value))))
            })
    }

    #[cfg(test)]
    pub fn pending_count_due_at(&self, deadline_us: u64, lead_up: u64) -> usize {
        self.pending_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .filter(|pending| pending.get_effective_release_us(lead_up) <= deadline_us)
            .count()
    }

    pub fn pending_count_due_at_ticks(
        &self,
        deadline: TimelineTicks,
        _lead_up: DurationTicks,
    ) -> Result<usize, CoordinatorError> {
        self.pending_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .map(|pending| pending.get_effective_release_ticks(DurationTicks::ZERO))
            .try_fold(0usize, |count, value| {
                Ok(count + usize::from(value? <= deadline))
            })
    }

    pub fn plan_pending_dispatch_ticks(
        &self,
    ) -> Result<Option<PendingDispatchPlan>, CoordinatorError> {
        if self.pending_mask == 0 {
            return Ok(None);
        }
        let deadline_ticks = self
            .next_pending_release_ticks(DurationTicks::ZERO)?
            .ok_or_else(|| {
                CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                    "pending mask is set but no pending release exists".to_string(),
                ))
            })?;
        Ok(Some(PendingDispatchPlan {
            deadline_ticks,
            polyphony: self
                .pending_count_due_at_ticks(deadline_ticks, DurationTicks::ZERO)?
                .clamp(1, MAX_KEYS),
        }))
    }

    pub fn next_deadline_ticks(
        &self,
        _dispatch_lead: DurationTicks,
        pending_plan: Option<&PendingDispatchPlan>,
    ) -> Result<Option<TimelineTicks>, CoordinatorError> {
        if self.release_recovery_started_ticks.is_some() {
            return Ok(pending_plan.map(|plan| plan.deadline_ticks));
        }
        let authored = self.next_authored_ticks(DurationTicks::ZERO)?;
        let pending = pending_plan.map(|plan| plan.deadline_ticks);
        Ok(match (authored, pending) {
            (Some(a), Some(p)) => Some(a.min(p)),
            (Some(a), None) => Some(a),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        })
    }

    /// Select the next release cohort without applying a dispatch lead.
    ///
    /// The closure remains only as a source-compatible test seam for callers
    /// from the pre-refactor coordinator tests.  Its result is intentionally
    /// ignored: physical release deadlines are never advanced by estimation.
    #[cfg(test)]
    pub fn plan_pending_dispatch<F>(&self, _lead_for_polyphony: F) -> Option<PendingDispatchPlan>
    where
        F: Fn(usize) -> (u64, bool),
    {
        if self.pending_mask == 0 {
            return None;
        }
        let deadline_us = self.next_pending_release_us(0)?;
        Some(PendingDispatchPlan {
            deadline_ticks: TimelineTicks::from_raw(deadline_us),
            polyphony: self.pending_count_due_at(deadline_us, 0).clamp(1, MAX_KEYS),
        })
    }

    #[cfg(test)]
    pub fn next_deadline_with_pending_plan(
        &self,
        dispatch_lead_us: u64,
        pending_plan: Option<&PendingDispatchPlan>,
    ) -> Option<u64> {
        if self.release_recovery_active() {
            return pending_plan.map(|plan| plan.deadline_ticks.as_u64());
        }
        let authored = self.next_authored_us(dispatch_lead_us);
        let pending = pending_plan.map(|plan| plan.deadline_ticks.as_u64());
        match (authored, pending) {
            (Some(a), Some(p)) => Some(a.min(p)),
            (Some(a), None) => Some(a),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        }
    }

    #[cfg(test)]
    pub fn next_deadline_us(&self, dispatch_lead_us: u64, lead_up: u64) -> Option<u64> {
        if self.release_recovery_active() {
            return self.next_pending_release_us(lead_up);
        }
        let authored = self.next_authored_us(dispatch_lead_us);
        let pending = self.next_pending_release_us(lead_up);
        match (authored, pending) {
            (Some(a), Some(p)) => Some(a.min(p)),
            (Some(a), None) => Some(a),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        }
    }

    #[cfg(test)]
    pub fn pop_due_pending(
        &mut self,
        now_us: u64,
        lead_up: u64,
    ) -> SmallVec<[PendingRelease; MAX_KEYS]> {
        self.pop_due_pending_until(now_us, lead_up)
    }

    #[cfg(test)]
    pub fn pop_due_pending_with_plan(
        &mut self,
        now_us: u64,
        plan: &PendingDispatchPlan,
    ) -> SmallVec<[PendingRelease; MAX_KEYS]> {
        self.pop_due_pending_until(now_us.min(plan.deadline_ticks.as_u64()), 0)
    }

    #[cfg(test)]
    fn pop_due_pending_until(
        &mut self,
        now_us: u64,
        lead_up: u64,
    ) -> SmallVec<[PendingRelease; MAX_KEYS]> {
        if self.pending_mask == 0 {
            return SmallVec::new();
        }

        let mut due: SmallVec<[PendingRelease; MAX_KEYS]> = self
            .pending_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .filter(|pending| pending.get_effective_release_us(lead_up) <= now_us)
            .cloned()
            .collect();

        if due.is_empty() {
            return SmallVec::new();
        }

        due.sort_by_key(|p| {
            (
                p.get_effective_release_us(lead_up),
                p.source_action_index,
                p.scan_code,
            )
        });

        // Keep the coordinator-owned pending slots intact while the physical
        // sender is in flight.  A due batch is a borrow of release work, not a
        // logical ownership transfer; only confirmed reconciliation may clear
        // the generation and its pending mask.
        due
    }

    pub fn pop_due_pending_ticks(
        &mut self,
        now: TimelineTicks,
        plan: &PendingDispatchPlan,
    ) -> Result<SmallVec<[PendingRelease; MAX_KEYS]>, CoordinatorError> {
        if self.pending_mask == 0 {
            return Ok(SmallVec::new());
        }
        let limit = now.min(plan.deadline_ticks);
        let mut due_with_deadline: SmallVec<[(PendingRelease, TimelineTicks); MAX_KEYS]> =
            SmallVec::new();
        for pending in self.pending_by_slot.iter().filter_map(Option::as_ref) {
            let deadline = pending.get_effective_release_ticks(DurationTicks::ZERO)?;
            if deadline <= limit {
                due_with_deadline.push((pending.clone(), deadline));
            }
        }
        due_with_deadline.sort_by_key(|(pending, deadline)| {
            (*deadline, pending.source_action_index, pending.scan_code)
        });
        let due: SmallVec<[PendingRelease; MAX_KEYS]> = due_with_deadline
            .iter()
            .map(|(pending, _)| pending.clone())
            .collect();
        for pending in &due {
            let slot = usize::from(pending.key_slot);
            let Some(current) = self.pending_by_slot.get(slot).and_then(Option::as_ref) else {
                return Err(CoordinatorError::InvalidKeySlot {
                    slot: pending.key_slot,
                });
            };
            if current.generation_id != pending.generation_id {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::UnexpectedTransition {
                        generation_id: pending.generation_id,
                        expected: GenerationStatus::ReleasePending,
                        actual: GenerationStatus::ReleasePending,
                        next: GenerationStatus::Released,
                    },
                ));
            }
        }
        Ok(due)
    }
}
