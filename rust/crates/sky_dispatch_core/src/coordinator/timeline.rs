use super::{
    CoordinatorError, CoordinatorInvariantError, RuntimeDispatchCoordinator, TimelineRebaseReason,
};
use crate::model::*;
use crate::time::{DurationTicks, TimelineTicks};

impl RuntimeDispatchCoordinator {
    pub(super) fn apply_timeline_rebase(
        &mut self,
        delta: DurationTicks,
        reason: TimelineRebaseReason,
    ) -> Result<(), CoordinatorError> {
        if delta == DurationTicks::ZERO {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "timeline rebase delta must be non-zero".to_string(),
                ),
            ));
        }
        let next_offset = self.recovery_offset_ticks.checked_add(delta)?;
        let next_count =
            self.timeline_rebase_count
                .checked_add(1)
                .ok_or(CoordinatorError::Time(
                    crate::time::TimeArithmeticError::Overflow,
                ))?;
        let next_total = self
            .timeline_rebase_total_ticks
            .checked_add(delta.as_u64())
            .ok_or(CoordinatorError::Time(
                crate::time::TimeArithmeticError::Overflow,
            ))?;
        let next_max = self.timeline_rebase_max_ticks.max(delta.as_u64());
        self.recovery_offset_ticks = next_offset;
        self.timeline_rebase_count = next_count;
        self.timeline_rebase_total_ticks = next_total;
        self.timeline_rebase_max_ticks = next_max;
        self.last_timeline_rebase_reason = Some(reason);
        Ok(())
    }

    pub fn effective_total_ticks(&self) -> Result<TimelineTicks, CoordinatorError> {
        self.batch_scheduled_ticks
            .last()
            .copied()
            .map_or(Ok(TimelineTicks::ZERO), |scheduled| {
                Ok(scheduled.checked_add_duration(self.recovery_offset_ticks)?)
            })
    }

    pub fn effective_batch_scheduled_ticks(
        &self,
        index: usize,
    ) -> Result<TimelineTicks, CoordinatorError> {
        self.batch_scheduled_ticks
            .get(index)
            .copied()
            .ok_or(CoordinatorError::InvalidBatchIndex { index })?
            .checked_add_duration(self.recovery_offset_ticks)
            .map_err(CoordinatorError::from)
    }

    /// Return the next dispatch deadline for one physical packet.
    ///
    /// A packet containing releases cannot be dispatched before the latest
    /// sender-side minimum-hold floor owned by its physical Up mask.  Waiting
    /// and preparation both use this single calculation.
    /// Compute the sender-side minimum-hold floor owned by every physical Up
    /// intent with a real generation in the given packet.
    ///
    /// Fail-closed: a physical Up with a real generation must be backed by an
    /// active generation that owns its key slot, and that generation must match
    /// the authored generation. `NO_GENERATION_ID` (stale Up) does not require
    /// an active generation. Every real generation, including a generation
    /// whose lifecycle is terminal, must still be owned by the exact active
    /// slot or the deadline computation fails before `SendInput`.
    fn packet_release_floor_ticks(
        &self,
        packet_index: usize,
    ) -> Result<TimelineTicks, CoordinatorError> {
        let packet = *self
            .schedule
            .packets
            .get(packet_index)
            .ok_or(CoordinatorError::Schedule(
                RuntimeScheduleError::InvalidPacketIndex {
                    index: packet_index,
                },
            ))?;
        let up_start = packet.up_intent_start as usize;
        let up_end = up_start.checked_add(packet.up_intent_len as usize).ok_or(
            CoordinatorError::Schedule(RuntimeScheduleError::InvalidPacketIntentRange {
                index: packet_index,
            }),
        )?;
        let up_intents =
            self.schedule
                .intents
                .get(up_start..up_end)
                .ok_or(CoordinatorError::Schedule(
                    RuntimeScheduleError::InvalidPacketIntentRange {
                        index: packet_index,
                    },
                ))?;
        let mut release_not_before = TimelineTicks::ZERO;
        for compact in up_intents {
            let generation_id = compact.generation_id();
            if generation_id == NO_GENERATION_ID {
                continue;
            }
            let slot = compact.key_slot();
            let active = self.active_for_slot(slot).ok_or_else(|| {
                CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                    "physical up has no active generation".into(),
                ))
            })?;
            if active.generation_id != generation_id {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "packet release generation does not own its key slot".into(),
                    ),
                ));
            }
            release_not_before = release_not_before.max(active.release_not_before_ticks);
        }
        Ok(release_not_before)
    }

    pub fn packet_effective_deadline_ticks(
        &self,
        packet_index: usize,
        dispatch_lead: DurationTicks,
    ) -> Result<TimelineTicks, CoordinatorError> {
        let packet = *self
            .schedule
            .packets
            .get(packet_index)
            .ok_or(CoordinatorError::Schedule(
                RuntimeScheduleError::InvalidPacketIndex {
                    index: packet_index,
                },
            ))?;
        let first_batch_index = packet.first_batch_index as usize;
        let authored = self
            .batch_scheduled_ticks
            .get(first_batch_index)
            .copied()
            .ok_or(CoordinatorError::InvalidBatchIndex {
                index: first_batch_index,
            })?
            .checked_add_duration(self.recovery_offset_ticks)?;
        // Singular release-floor source of truth for the packet's physical Up.
        let release_not_before = self.packet_release_floor_ticks(packet_index)?;
        // Explicit pre-epoch/startup clamp: an authored timestamp before the
        // timeline epoch lowers the effective deadline to ZERO. The clamp is
        // intentionally explicit rather than a saturating subtraction, so a
        // real arithmetic order violation surfaces instead of being masked.
        let lead_deadline = match authored.checked_sub_duration(dispatch_lead) {
            Ok(deadline) => deadline,
            Err(crate::time::TimeArithmeticError::Underflow) => TimelineTicks::ZERO,
            Err(error) => return Err(error.into()),
        };
        Ok(lead_deadline.max(release_not_before))
    }
}
