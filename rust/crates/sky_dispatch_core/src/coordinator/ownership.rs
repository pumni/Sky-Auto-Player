use super::{
    ActiveGeneration, CoordinatorError, CoordinatorInvariantError, GenerationStatus,
    RuntimeDispatchCoordinator,
};
use crate::model::*;
use crate::time::TimelineTicks;

impl RuntimeDispatchCoordinator {
    /// Apply one checked lifecycle transition. Generation IDs are only used
    /// to address the compiler-owned contiguous ledger, never as a batch
    /// timestamp/index identity.
    pub fn transition_generation(
        &mut self,
        generation_id: GenerationId,
        expected: GenerationStatus,
        next: GenerationStatus,
    ) -> Result<(), CoordinatorError> {
        let Some(state) = self.generation_states.get_mut(generation_id as usize) else {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::UnknownGeneration {
                    generation_id,
                    generation_count: self.generation_count,
                },
            ));
        };
        if *state != expected {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::UnexpectedTransition {
                    generation_id,
                    expected,
                    actual: *state,
                    next,
                },
            ));
        }
        if !expected.can_transition_to(next) {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::IllegalTransition {
                    generation_id,
                    from: expected,
                    to: next,
                },
            ));
        }
        *state = next;
        if next.is_terminal() {
            self.counters.increment(next);
        }
        Ok(())
    }

    pub(super) fn terminalize(
        &mut self,
        generation_id: GenerationId,
        status: GenerationStatus,
    ) -> Result<(), CoordinatorError> {
        self.transition_generation(generation_id, GenerationStatus::Scheduled, status)
    }

    /// Make the authored Up for a generation stale after its Down was
    /// terminalized without ever becoming physically active.
    pub(super) fn invalidate_up_for_generation(&mut self, generation_id: GenerationId) {
        if generation_id == NO_GENERATION_ID {
            return;
        }
        let Ok(generation_index) = usize::try_from(generation_id) else {
            return;
        };
        let Some(location) = self.up_intent_locations.get(generation_index) else {
            return;
        };
        let Some((packet_index, intent_index)) = *location else {
            return;
        };
        let Some(compact) = self.schedule.intents.get_mut(intent_index) else {
            return;
        };
        if compact.generation_id() == generation_id {
            let slot = compact.key_slot();
            *compact = CompactIntent::new(NO_GENERATION_ID, slot);
            self.schedule.packets[packet_index].up_mask &= !Self::bit_for_slot(slot);
        }
    }

    /// Activate the Down identities confirmed by one canonical packet.
    #[allow(clippy::too_many_arguments)]
    pub fn activate_sent_downs_compact_ticks(
        &mut self,
        batch_index: usize,
        sent_scan_codes: &[u16],
        dispatch_started: TimelineTicks,
        dispatch_completed: TimelineTicks,
        excluded_mask: u16,
    ) -> Result<(), CoordinatorError> {
        let release_not_before_ticks =
            dispatch_completed.checked_add_duration(self.min_hold_ticks)?;
        let batch = self
            .schedule
            .batches
            .get(batch_index)
            .ok_or(CoordinatorError::InvalidBatchIndex { index: batch_index })?;
        let source_action_index = batch.source_action_index;
        let start = batch.intent_start as usize;
        let end = start
            .checked_add(batch.intent_len as usize)
            .ok_or(CoordinatorError::Time(
                crate::time::TimeArithmeticError::Overflow,
            ))?;
        let scheduled_down_ticks = self
            .batch_scheduled_ticks
            .get(batch_index)
            .copied()
            .ok_or(CoordinatorError::InvalidBatchIndex { index: batch_index })?;
        for intent_index in start..end {
            let compact = *self
                .schedule
                .intents
                .get(intent_index)
                .ok_or(CoordinatorError::InvalidBatchIndex { index: batch_index })?;
            let generation_id = compact.generation_id();
            if generation_id == NO_GENERATION_ID {
                continue;
            }
            let slot = compact.key_slot();
            if excluded_mask & Self::bit_for_slot(slot) != 0 {
                continue;
            }
            let Some(scan_code) = self.schedule.key_registry.scan_code_for(slot) else {
                return Err(CoordinatorError::InvalidKeySlot { slot });
            };
            if !sent_scan_codes.contains(&scan_code) {
                self.transition_generation(
                    generation_id,
                    GenerationStatus::Scheduled,
                    GenerationStatus::DroppedBackend,
                )?;
                self.invalidate_up_for_generation(generation_id);
                continue;
            }
            self.transition_generation(
                generation_id,
                GenerationStatus::Scheduled,
                GenerationStatus::Active,
            )?;
            self.active_by_slot[usize::from(slot)] = Some(ActiveGeneration {
                generation_id,
                scan_code,
                key_slot: slot,
                source_action_index,
                scheduled_down_ticks,
                down_dispatch_started_ticks: dispatch_started,
                down_dispatch_completed_ticks: dispatch_completed,
                release_not_before_ticks,
            });
            self.active_mask |= Self::bit_for_slot(slot);
            self.blocked_mask |= Self::bit_for_slot(slot);
        }
        Ok(())
    }

    pub fn activate_sent_downs_ticks(
        &mut self,
        intents: &[RuntimeKeyIntent],
        sent_scan_codes: &[u16],
        dispatch_started_ticks: TimelineTicks,
        dispatch_completed_ticks: TimelineTicks,
    ) -> Result<(), CoordinatorError> {
        let release_not_before_ticks =
            dispatch_completed_ticks.checked_add_duration(self.min_hold_ticks)?;

        for intent in intents {
            let Some(generation_id) = intent.generation_id else {
                continue;
            };
            if !sent_scan_codes.contains(&intent.scan_code) {
                self.transition_generation(
                    generation_id,
                    GenerationStatus::Scheduled,
                    GenerationStatus::DroppedBackend,
                )?;
                self.invalidate_up_for_generation(generation_id);
                continue;
            }
            self.transition_generation(
                generation_id,
                GenerationStatus::Scheduled,
                GenerationStatus::Active,
            )?;
            let scheduled_down_ticks = self
                .batch_scheduled_ticks
                .get(intent.compiled_batch_index)
                .copied()
                .ok_or(CoordinatorError::InvalidBatchIndex {
                    index: intent.compiled_batch_index,
                })?;
            self.active_by_slot[intent.key_slot as usize] = Some(ActiveGeneration {
                generation_id,
                scan_code: intent.scan_code,
                key_slot: intent.key_slot,
                source_action_index: intent.source_action_index,
                scheduled_down_ticks,
                down_dispatch_started_ticks: dispatch_started_ticks,
                down_dispatch_completed_ticks: dispatch_completed_ticks,
                release_not_before_ticks,
            });
            self.active_mask |= Self::bit_for_slot(intent.key_slot);
            self.blocked_mask |= Self::bit_for_slot(intent.key_slot);
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn activate_sent_downs(
        &mut self,
        intents: &[RuntimeKeyIntent],
        sent_scan_codes: &[u16],
        _dispatch_started_us: u64,
        dispatch_started_ticks: TimelineTicks,
        _dispatch_completed_us: u64,
        dispatch_completed_ticks: TimelineTicks,
    ) -> Result<(), CoordinatorError> {
        self.activate_sent_downs_ticks(
            intents,
            sent_scan_codes,
            dispatch_started_ticks,
            dispatch_completed_ticks,
        )
    }
}
