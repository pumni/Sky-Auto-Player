use super::{CoordinatorError, GenerationStatus, RuntimeDispatchCoordinator, SplitIntentResult};
use crate::model::*;
use smallvec::SmallVec;

impl RuntimeDispatchCoordinator {
    /// Check whether any intent in `compact_intents` conflicts with an
    /// already-active key slot.
    ///
    /// Returns a bitmask (`u16`) where each set bit corresponds to a key slot
    /// that is currently active.  A return value of `0` means no conflicts.
    ///
    /// This is the hot-path alternative to `split_down_intents` —
    /// it operates directly on the compact arena slice with one bitwise AND
    /// per intent and produces no allocation.
    pub fn check_down_conflicts_compact(&self, compact_intents: &[CompactIntent]) -> u16 {
        if self.blocked_mask == 0 {
            return 0;
        }
        let mut conflict_mask: u16 = 0;
        for compact in compact_intents {
            let bit = Self::bit_for_slot(compact.key_slot());
            if self.blocked_mask & bit != 0 {
                conflict_mask |= bit;
            }
        }
        conflict_mask
    }

    /// Check Down identities in a packet after accounting for the packet's
    /// canonical Up phase. A same-key retrigger is valid because its old
    /// generation is released by this very transaction before the new Down.
    pub fn check_packet_down_conflicts(&self, up_mask: u16, down_mask: u16) -> u16 {
        (self.blocked_mask & !up_mask) & down_mask
    }

    /// Terminalize the generations associated with every slot set in
    /// `conflict_mask` as `DroppedConflict`.
    ///
    /// Called after `check_down_conflicts_compact` returns a non-zero mask.
    /// Updating counters is the only side-effect; no mask bits are cleared
    /// (the slots were never activated for the conflicting batch).
    pub fn terminalize_conflicted_slots(
        &mut self,
        compact_intents: &[CompactIntent],
        conflict_mask: u16,
    ) -> Result<(), CoordinatorError> {
        if conflict_mask == 0 {
            return Ok(());
        }
        for compact in compact_intents {
            if conflict_mask & Self::bit_for_slot(compact.key_slot()) != 0 {
                // Only Down intents with a generation ID need terminalizing.
                if compact.generation_id() != NO_GENERATION_ID {
                    let generation_id = compact.generation_id();
                    self.terminalize(generation_id, GenerationStatus::DroppedConflict)?;
                    self.invalidate_up_for_generation(generation_id);
                }
            }
        }
        Ok(())
    }

    pub fn split_down_intents(
        &mut self,
        intents: &[RuntimeKeyIntent],
    ) -> Result<SplitIntentResult, CoordinatorError> {
        if self.blocked_mask == 0 {
            return Ok((intents.iter().cloned().collect(), SmallVec::new()));
        }
        let mut playable = SmallVec::new();
        let mut conflicts = SmallVec::new();

        for intent in intents {
            if self.blocked_mask & Self::bit_for_slot(intent.key_slot) != 0 {
                conflicts.push(intent.clone());
                if let Some(gen_id) = intent.generation_id {
                    self.terminalize(gen_id, GenerationStatus::DroppedConflict)?;
                    self.invalidate_up_for_generation(gen_id);
                }
            } else {
                playable.push(intent.clone());
            }
        }
        Ok((playable, conflicts))
    }

    /// Terminalize every generation in a conflicted authored chord without
    /// sending a playable subset. Accuracy-first callers use this when a
    /// partial chord would be worse than dropping the whole chord.
    pub fn drop_conflicted_downs(
        &mut self,
        intents: &[RuntimeKeyIntent],
    ) -> Result<(), CoordinatorError> {
        for intent in intents {
            if let Some(generation_id) = intent.generation_id {
                self.terminalize(generation_id, GenerationStatus::DroppedConflict)?;
                self.invalidate_up_for_generation(generation_id);
            }
        }
        Ok(())
    }

    pub fn drop_expired_downs(
        &mut self,
        intents: &[RuntimeKeyIntent],
    ) -> Result<(), CoordinatorError> {
        for intent in intents {
            if let Some(gen_id) = intent.generation_id {
                self.terminalize(gen_id, GenerationStatus::DroppedExpired)?;
                self.invalidate_up_for_generation(gen_id);
            }
        }
        Ok(())
    }
}
