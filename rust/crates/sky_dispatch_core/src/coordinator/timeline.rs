use super::{CoordinatorError, CoordinatorInvariantError, RuntimeDispatchCoordinator};
use crate::model::*;
use crate::time::TimelineTicks;

impl RuntimeDispatchCoordinator {
    pub fn effective_total_ticks(&self) -> Result<TimelineTicks, CoordinatorError> {
        Ok(self
            .batch_scheduled_ticks
            .last()
            .copied()
            .unwrap_or(TimelineTicks::ZERO))
    }

    pub fn effective_batch_scheduled_ticks(
        &self,
        index: usize,
    ) -> Result<TimelineTicks, CoordinatorError> {
        self.batch_scheduled_ticks
            .get(index)
            .copied()
            .ok_or(CoordinatorError::InvalidBatchIndex { index })
    }

    /// Return the next dispatch deadline for one physical packet.
    ///
    /// A packet containing releases cannot be dispatched before the latest
    /// sender-side minimum-hold floor owned by its physical Up mask. Waiting
    /// and preparation both use this single calculation.
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

    pub(crate) fn packet_effective_deadline_ticks_uncompensated(
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
        let first_batch_index = packet.first_batch_index as usize;
        let authored = self
            .batch_scheduled_ticks
            .get(first_batch_index)
            .copied()
            .ok_or(CoordinatorError::InvalidBatchIndex {
                index: first_batch_index,
            })?;
        let release_not_before = self.packet_release_floor_ticks(packet_index)?;
        Ok(authored.max(release_not_before))
    }

    #[cfg(not(test))]
    pub fn packet_effective_deadline_ticks(
        &self,
        packet_index: usize,
    ) -> Result<TimelineTicks, CoordinatorError> {
        self.packet_effective_deadline_ticks_uncompensated(packet_index)
    }

    #[cfg(test)]
    pub fn packet_effective_deadline_ticks(
        &self,
        packet_index: usize,
        _dispatch_lead: crate::time::DurationTicks,
    ) -> Result<TimelineTicks, CoordinatorError> {
        self.packet_effective_deadline_ticks_uncompensated(packet_index)
    }
}
