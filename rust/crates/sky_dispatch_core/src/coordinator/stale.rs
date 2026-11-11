//! Stale packet preparation and commit for unmatched Up metadata.

use super::{
    CoordinatorError, CoordinatorInvariantError, PreparedStalePacket, RuntimeDispatchCoordinator,
};
use crate::model::*;
use crate::time::TimelineTicks;

impl RuntimeDispatchCoordinator {
    /// Prepare the current compiler packet when it contains only unmatched Up
    /// metadata.  This is intentionally separate from physical batch
    /// preparation so stale metadata cannot acquire a DispatchPath or timing
    /// lead by accident.
    pub fn prepare_current_stale_packet(
        &self,
    ) -> Result<Option<PreparedStalePacket>, CoordinatorError> {
        let Some(batch) = self.schedule.batches.get(self.cursor) else {
            return Ok(None);
        };
        let packet_index = usize::try_from(batch.packet_id).map_err(|_| {
            CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                "packet id does not fit in usize".to_string(),
            ))
        })?;
        let packet = self
            .schedule
            .packets
            .get(packet_index)
            .ok_or(CoordinatorError::Schedule(
                RuntimeScheduleError::InvalidPacketIndex {
                    index: packet_index,
                },
            ))?;
        if packet.first_batch_index as usize != self.cursor {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "stale packet first batch does not match coordinator cursor".to_string(),
                ),
            ));
        }
        if packet.up_mask != 0 || packet.down_mask != 0 {
            return Ok(None);
        }
        if packet.up_intent_len == 0 || packet.down_intent_len != 0 {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "empty compiled packet is not a stale unmatched-Up packet".to_string(),
                ),
            ));
        }
        let packet_batch_count = usize::from(packet.batch_count);
        if packet_batch_count == 0 {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "stale packet must contain at least one authored batch".to_string(),
                ),
            ));
        }
        let batch_end =
            self.cursor
                .checked_add(packet_batch_count)
                .ok_or(CoordinatorError::Time(
                    crate::time::TimeArithmeticError::Overflow,
                ))?;
        let batch_range = self
            .schedule
            .batches
            .get(self.cursor..batch_end)
            .ok_or(CoordinatorError::InvalidBatchIndex { index: self.cursor })?;
        if batch_range.iter().any(|candidate| {
            candidate.packet_id != batch.packet_id
                || candidate.kind != ActionKind::Up
                || candidate.intent_len == 0
        }) {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "stale packet contains an invalid authored batch".to_string(),
                ),
            ));
        }
        let up_start = packet.up_intent_start as usize;
        let up_end = up_start
            .checked_add(usize::from(packet.up_intent_len))
            .ok_or(CoordinatorError::Schedule(
                RuntimeScheduleError::InvalidPacketIntentRange {
                    index: packet_index,
                },
            ))?;
        let up_intents =
            self.schedule
                .intents
                .get(up_start..up_end)
                .ok_or(CoordinatorError::Schedule(
                    RuntimeScheduleError::InvalidPacketIntentRange {
                        index: packet_index,
                    },
                ))?;
        if !up_intents
            .iter()
            .all(|intent| intent.generation_id() == NO_GENERATION_ID)
        {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "zero-mask authored packet contains an owned Up intent".to_string(),
                ),
            ));
        }
        let effective_scheduled_ticks = self.effective_batch_scheduled_ticks(self.cursor)?;
        Ok(Some(PreparedStalePacket {
            first_batch_index: self.cursor,
            packet_index,
            packet_batch_count,
            effective_scheduled_ticks,
            source_action_index: batch.source_action_index,
            suppressed_intent_count: up_intents.len(),
        }))
    }

    /// Prepare the next stale packet at or before `now_ticks`, advancing the
    /// cursor past physical packets that have already arrived so that midstream
    /// and trailing stale metadata remains reachable under Phase B execution.
    pub const MAX_STALE_PACKETS_SCANNED_PER_STEP: usize = 32;

    /// Prepare the current stale packet at `now_ticks`.
    ///
    /// The scan is bounded to at most [`Self::MAX_STALE_PACKETS_SCANNED_PER_STEP`]
    /// packets per invocation to maintain real-time responsiveness.
    pub fn prepare_current_stale_packet_at(
        &mut self,
        now_ticks: TimelineTicks,
    ) -> Result<Option<PreparedStalePacket>, CoordinatorError> {
        let mut scanned = 0usize;
        while self.cursor < self.schedule.batches.len()
            && scanned < Self::MAX_STALE_PACKETS_SCANNED_PER_STEP
        {
            scanned += 1;
            let batch = &self.schedule.batches[self.cursor];
            let packet_index = usize::try_from(batch.packet_id).map_err(|_| {
                CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                    "packet id does not fit in usize".to_string(),
                ))
            })?;
            let packet =
                self.schedule
                    .packets
                    .get(packet_index)
                    .ok_or(CoordinatorError::Schedule(
                        RuntimeScheduleError::InvalidPacketIndex {
                            index: packet_index,
                        },
                    ))?;
            if packet.first_batch_index as usize != self.cursor {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "stale packet first batch does not match coordinator cursor".to_string(),
                    ),
                ));
            }
            let packet_ticks =
                self.effective_batch_scheduled_ticks(packet.first_batch_index as usize)?;
            if packet.up_mask == 0 && packet.down_mask == 0 && packet.up_intent_len > 0 {
                if packet_ticks <= now_ticks {
                    return self.prepare_current_stale_packet();
                } else {
                    return Ok(None);
                }
            } else if packet_ticks <= now_ticks {
                self.cursor += usize::from(packet.batch_count);
                continue;
            } else {
                return Ok(None);
            }
        }
        Ok(None)
    }

    /// Return the scheduled offset in timeline ticks of the earliest remaining
    /// stale packet, if any.
    ///
    /// The scan is bounded to at most [`Self::MAX_STALE_PACKETS_SCANNED_PER_STEP`]
    /// packets to prevent unbounded scanning during high-frequency dispatch ticks.
    pub fn next_stale_packet_target_ticks(
        &self,
    ) -> Result<Option<TimelineTicks>, CoordinatorError> {
        let mut cursor = self.cursor;
        let mut scanned = 0usize;
        while cursor < self.schedule.batches.len()
            && scanned < Self::MAX_STALE_PACKETS_SCANNED_PER_STEP
        {
            scanned += 1;
            let batch = self
                .schedule
                .batches
                .get(cursor)
                .ok_or(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "stale lookahead cursor is outside the batch schedule".to_string(),
                    ),
                ))?;
            let packet_index = usize::try_from(batch.packet_id).map_err(|_| {
                CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                    "packet id does not fit in usize".to_string(),
                ))
            })?;
            let packet =
                self.schedule
                    .packets
                    .get(packet_index)
                    .ok_or(CoordinatorError::Schedule(
                        RuntimeScheduleError::InvalidPacketIndex {
                            index: packet_index,
                        },
                    ))?;
            let packet_first_batch_index =
                usize::try_from(packet.first_batch_index).map_err(|_| {
                    CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                        "packet first batch index does not fit in usize".to_string(),
                    ))
                })?;
            if packet_first_batch_index != cursor {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "stale packet first batch does not match coordinator lookahead cursor"
                            .to_string(),
                    ),
                ));
            }
            let packet_batch_count = usize::from(packet.batch_count);
            if packet_batch_count == 0 {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "stale lookahead packet must contain at least one authored batch"
                            .to_string(),
                    ),
                ));
            }
            if packet.up_mask == 0 && packet.down_mask == 0 {
                if packet.up_intent_len == 0 || packet.down_intent_len != 0 {
                    return Err(CoordinatorError::Invariant(
                        CoordinatorInvariantError::Accounting(
                            "empty compiled packet is not a stale unmatched-Up packet".to_string(),
                        ),
                    ));
                }
                return self.effective_batch_scheduled_ticks(cursor).map(Some);
            }
            let next_cursor =
                cursor
                    .checked_add(packet_batch_count)
                    .ok_or(CoordinatorError::Time(
                        crate::time::TimeArithmeticError::Overflow,
                    ))?;
            if next_cursor > self.schedule.batches.len() {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "stale lookahead packet extends beyond the batch schedule".to_string(),
                    ),
                ));
            }
            cursor = next_cursor;
        }
        Ok(None)
    }

    /// Atomically consume one prepared stale compiler packet.
    pub fn commit_stale_packet(
        &mut self,
        prepared: PreparedStalePacket,
    ) -> Result<(), CoordinatorError> {
        let current = self
            .prepare_current_stale_packet()?
            .ok_or(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "prepared stale packet is no longer current".to_string(),
                ),
            ))?;
        if current != prepared {
            return Err(CoordinatorError::PreparedBatchMismatch {
                prepared: prepared.first_batch_index,
                cursor: self.cursor,
            });
        }
        self.cursor =
            self.cursor
                .checked_add(prepared.packet_batch_count)
                .ok_or(CoordinatorError::Time(
                    crate::time::TimeArithmeticError::Overflow,
                ))?;
        self.validate_local_slot_masks()?;
        Ok(())
    }
}
