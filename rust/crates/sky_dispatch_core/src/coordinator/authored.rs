use super::{
    ActiveGeneration, CoordinatorError, CoordinatorInvariantError, GenerationStatus, PreparedBatch,
    PreparedStalePacket, RuntimeDispatchCoordinator, physical_packet_kind,
};
use crate::model::*;
#[cfg(test)]
use crate::time::DurationTicks;
use crate::time::TimelineTicks;
use smallvec::SmallVec;

impl RuntimeDispatchCoordinator {
    #[cfg(test)]
    fn early_pop_blocked(&self, batch: &CompiledBatch) -> bool {
        if batch.kind != ActionKind::Down {
            return false;
        }
        if self.blocked_mask == 0 {
            return false;
        }
        self.schedule.intent_slice(batch).iter().any(|intent| {
            let bit = Self::bit_for_slot(intent.key_slot());
            self.blocked_mask & bit != 0
        })
    }

    #[cfg(test)]
    fn effective_authored_lead(scheduled_us: u64, requested_lead_us: u64) -> u64 {
        // The logical timeline is unsigned.  Applying a lead to an authored
        // timestamp smaller than that lead would saturate several distinct
        // deadlines to zero and could dispatch them as one burst.  The first
        // authored action is handled by the native worker's future physical
        // startup anchor; subsequent sub-lead actions stay ordered here.
        if scheduled_us >= requested_lead_us {
            requested_lead_us
        } else {
            0
        }
    }

    #[cfg(test)]
    pub fn next_authored_us(&self, dispatch_lead_us: u64) -> Option<u64> {
        if self.cursor >= self.schedule.batches.len() {
            return None;
        }
        let batch = &self.schedule.batches[self.cursor];
        let lead = dispatch_lead_us;
        if lead > 0 && self.early_pop_blocked(batch) {
            return Some(batch.scheduled_us);
        }
        let effective_scheduled_us = batch.scheduled_us;
        let effective_lead = Self::effective_authored_lead(effective_scheduled_us, lead);
        Some(effective_scheduled_us.saturating_sub(effective_lead))
    }

    /// Return the next authored dispatch deadline in the playback tick domain.
    /// The authored schedule remains immutable; physical release floors are
    /// applied only to the current canonical packet.
    pub(crate) fn next_authored_ticks_uncompensated(
        &self,
    ) -> Result<Option<TimelineTicks>, CoordinatorError> {
        let Some(batch) = self.schedule.batches.get(self.cursor) else {
            return Ok(None);
        };
        let packet_index = usize::try_from(batch.packet_id).map_err(|_| {
            CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                "packet id does not fit in usize".to_string(),
            ))
        })?;
        let effective = self.packet_effective_deadline_ticks_uncompensated(packet_index)?;
        Ok(Some(effective))
    }

    #[cfg(not(test))]
    pub fn next_authored_ticks(&self) -> Result<Option<TimelineTicks>, CoordinatorError> {
        self.next_authored_ticks_uncompensated()
    }

    #[cfg(test)]
    pub fn next_authored_ticks(
        &self,
        _dispatch_lead: DurationTicks,
    ) -> Result<Option<TimelineTicks>, CoordinatorError> {
        self.next_authored_ticks_uncompensated()
    }

    /// Return the earliest upcoming physical boundary from the authored and
    /// completion-floor projections.
    ///
    /// Planning uses this projection to classify the interval that will
    /// actually precede the next physical operation. Release floors are
    /// represented by authored batch deadlines. Production planning never
    /// subtracts a dispatch lead or scans recovery state.
    pub fn next_uncompensated_deadline_ticks(
        &self,
    ) -> Result<Option<TimelineTicks>, CoordinatorError> {
        let authored = self.next_authored_ticks_uncompensated()?;
        Ok(authored)
    }

    /// Polyphony of the next authored down batch, used to freeze its health
    /// budget before the batch is popped from the schedule.
    pub fn next_authored_polyphony(&self) -> usize {
        self.schedule
            .batches
            .get(self.cursor)
            .filter(|batch| batch.kind == ActionKind::Down)
            .map_or(1, |batch| batch.intent_len as usize)
    }

    pub fn next_authored_packet_masks(&self) -> Option<(u16, u16)> {
        let batch = self.schedule.batches.get(self.cursor)?;
        let packet = self.schedule.packets.get(batch.packet_id as usize)?;
        Some((packet.up_mask, packet.down_mask))
    }

    /// Return the first authored packet at or after the cursor that has a
    /// physical event, skipping compiler-preserved stale-Up metadata.
    pub fn next_physical_authored_packet(&self) -> Option<(TimelineTicks, u16, u16)> {
        for (batch_index, batch) in self.schedule.batches.iter().enumerate().skip(self.cursor) {
            let packet = self.schedule.packets.get(batch.packet_id as usize)?;
            if packet.up_mask == 0 && packet.down_mask == 0 {
                continue;
            }
            let scheduled_ticks = self.effective_batch_scheduled_ticks(batch_index).ok()?;
            return Some((scheduled_ticks, packet.up_mask, packet.down_mask));
        }
        None
    }

    #[cfg(test)]
    pub fn pop_next_due_authored(
        &mut self,
        now_us: u64,
        _dispatch_lead_us: u64,
    ) -> Option<(RuntimeBatch, u64)> {
        let index = self
            .pop_next_due_authored_ticks(TimelineTicks::from_raw(now_us))
            .ok()??;
        let popped = self.schedule.materialize_batch(index, 0);
        Some((popped, 0))
    }

    /// Variant of [`pop_next_due_authored`] that returns the batch index.
    ///
    /// This avoids returning a borrow tied to `&mut self`, allowing the caller
    /// to immutably borrow the schedule via `coordinator.schedule.view_batch(...)`
    /// and then call other `&self` methods like `check_down_conflicts_compact`.
    #[cfg(test)]
    pub fn pop_next_due_authored_index(
        &mut self,
        now_us: u64,
        _dispatch_lead_us: u64,
    ) -> Option<(usize, u64)> {
        self.pop_next_due_authored_ticks(TimelineTicks::from_raw(now_us))
            .ok()?
            .map(|index| (index, 0))
    }

    /// Prepare the current physical authored packet without consulting the
    /// current clock.  The worker uses this before entering its timed wait so
    /// packet identity, masks, and effective deadline are frozen together.
    pub fn prepare_current_authored_packet(
        &self,
    ) -> Result<Option<PreparedBatch>, CoordinatorError> {
        if self.cursor >= self.schedule.batches.len() {
            return Ok(None);
        }
        let index = self.cursor;
        let batch = *self
            .schedule
            .batches
            .get(index)
            .ok_or(CoordinatorError::InvalidBatchIndex { index })?;
        let packet_index = usize::try_from(batch.packet_id).map_err(|_| {
            CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                "packet id does not fit in usize".to_string(),
            ))
        })?;
        let packet = *self
            .schedule
            .packets
            .get(packet_index)
            .ok_or(CoordinatorError::Schedule(
                RuntimeScheduleError::InvalidPacketIndex {
                    index: packet_index,
                },
            ))?;
        if packet.first_batch_index as usize != index {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "packet first batch does not match coordinator cursor".to_string(),
                ),
            ));
        }
        let effective_scheduled_ticks =
            self.packet_effective_deadline_ticks_uncompensated(packet_index)?;
        let packet_kind = physical_packet_kind(packet.up_mask, packet.down_mask)?;
        Ok(Some(PreparedBatch {
            index,
            effective_scheduled_ticks,
            packet_index,
            packet_batch_count: usize::from(packet.batch_count),
            packet_kind,
        }))
    }

    fn prepare_next_due_authored_uncompensated(
        &mut self,
        now: TimelineTicks,
    ) -> Result<Option<PreparedBatch>, CoordinatorError> {
        let Some(prepared) = self.prepare_current_authored_packet()? else {
            return Ok(None);
        };
        if prepared.effective_scheduled_ticks > now {
            return Ok(None);
        }
        Ok(Some(prepared))
    }

    #[cfg(not(test))]
    pub fn prepare_next_due_authored(
        &mut self,
        now: TimelineTicks,
    ) -> Result<Option<PreparedBatch>, CoordinatorError> {
        self.prepare_next_due_authored_uncompensated(now)
    }

    #[cfg(test)]
    pub fn prepare_next_due_authored(
        &mut self,
        now: TimelineTicks,
        _dispatch_lead: DurationTicks,
    ) -> Result<Option<PreparedBatch>, CoordinatorError> {
        self.prepare_next_due_authored_uncompensated(now)
    }

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

    /// Commit one physical packet after the sender reported a complete
    /// transaction. The logical Up transition is applied before Down so a
    /// same-key retrigger replaces the previous generation atomically.
    pub fn commit_packet_success(
        &mut self,
        prepared: PreparedBatch,
        started: TimelineTicks,
        completed: TimelineTicks,
    ) -> Result<(), CoordinatorError> {
        let (up_intents, down_intents, down_source_action_index) = {
            let packet = self
                .schedule
                .view_packet_ticks(prepared.packet_index, prepared.effective_scheduled_ticks)?;
            if packet.header.first_batch_index as usize != prepared.index
                || usize::from(packet.header.batch_count) != prepared.packet_batch_count
            {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "prepared packet metadata changed before commit".to_string(),
                    ),
                ));
            }
            (
                packet
                    .up_intents
                    .iter()
                    .copied()
                    .collect::<SmallVec<[_; MAX_KEYS]>>(),
                packet
                    .down_intents
                    .iter()
                    .copied()
                    .collect::<SmallVec<[_; MAX_KEYS]>>(),
                packet.header.down_source_action_index,
            )
        };
        self.commit_prepared_packet_success_parts(
            prepared,
            &up_intents,
            &down_intents,
            down_source_action_index,
            started,
            completed,
        )
    }

    /// Commit a packet whose bounded logical contents were frozen before the
    /// physical deadline.  The healthy path validates only the current packet
    /// and transitions the identities it owns; it never recounts the complete
    /// generation ledger.
    pub fn commit_prepared_packet_success_parts(
        &mut self,
        prepared: PreparedBatch,
        up_intents: &[CompactIntent],
        down_intents: &[CompactIntent],
        down_source_action_index: Option<u32>,
        started: TimelineTicks,
        completed: TimelineTicks,
    ) -> Result<(), CoordinatorError> {
        if prepared.index != self.cursor {
            return Err(CoordinatorError::PreparedBatchMismatch {
                prepared: prepared.index,
                cursor: self.cursor,
            });
        }
        let packet =
            *self
                .schedule
                .packets
                .get(prepared.packet_index)
                .ok_or(CoordinatorError::Schedule(
                    RuntimeScheduleError::InvalidPacketIndex {
                        index: prepared.packet_index,
                    },
                ))?;
        if packet.first_batch_index as usize != prepared.index
            || usize::from(packet.batch_count) != prepared.packet_batch_count
            || packet.up_intent_len as usize != up_intents.len()
            || packet.down_intent_len as usize != down_intents.len()
            || packet.down_source_action_index != down_source_action_index
            || physical_packet_kind(packet.up_mask, packet.down_mask)? != prepared.packet_kind
        {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "prepared packet metadata changed before commit".to_string(),
                ),
            ));
        }
        let up_start = packet.up_intent_start as usize;
        let up_end = up_start
            .checked_add(up_intents.len())
            .ok_or(CoordinatorError::Schedule(
                RuntimeScheduleError::InvalidPacketIntentRange {
                    index: prepared.packet_index,
                },
            ))?;
        let down_start = packet.down_intent_start as usize;
        let down_end =
            down_start
                .checked_add(down_intents.len())
                .ok_or(CoordinatorError::Schedule(
                    RuntimeScheduleError::InvalidPacketIntentRange {
                        index: prepared.packet_index,
                    },
                ))?;
        let current_up =
            self.schedule
                .intents
                .get(up_start..up_end)
                .ok_or(CoordinatorError::Schedule(
                    RuntimeScheduleError::InvalidPacketIntentRange {
                        index: prepared.packet_index,
                    },
                ))?;
        let current_down =
            self.schedule
                .intents
                .get(down_start..down_end)
                .ok_or(CoordinatorError::Schedule(
                    RuntimeScheduleError::InvalidPacketIntentRange {
                        index: prepared.packet_index,
                    },
                ))?;
        if current_up != up_intents || current_down != down_intents {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "prepared packet intents changed before commit".to_string(),
                ),
            ));
        }
        if self.packet_effective_deadline_ticks_uncompensated(prepared.packet_index)?
            != prepared.effective_scheduled_ticks
        {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "prepared packet deadline changed before commit".to_string(),
                ),
            ));
        }
        if prepared.packet_batch_count == 0 {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "compiled packet must contain at least one authored batch".to_string(),
                ),
            ));
        }
        let release_not_before_ticks = completed.checked_add_duration(self.min_hold_ticks)?;

        // Apply releases first. Stale Up intents are present for authored
        // diagnostics but deliberately have NO_GENERATION_ID and no physical
        // event in the packet.
        for compact in up_intents.iter().copied() {
            let generation_id = compact.generation_id();
            if generation_id == NO_GENERATION_ID {
                continue;
            }
            let slot = compact.key_slot();
            let Some(active) = self.active_for_slot(slot).cloned() else {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "packet release has no active generation".to_string(),
                    ),
                ));
            };
            if active.generation_id != generation_id {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "packet release generation does not own its key slot".to_string(),
                    ),
                ));
            }
            if started < active.release_not_before_ticks {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "authored Up started before release_not_before".to_string(),
                    ),
                ));
            }
            self.transition_generation(
                generation_id,
                GenerationStatus::Active,
                GenerationStatus::Released,
            )?;
            self.active_by_slot[usize::from(slot)] = None;
            self.active_mask &= !Self::bit_for_slot(slot);
            self.blocked_mask &= !Self::bit_for_slot(slot);
        }

        // Full SendInput success means every Down identity in the immutable
        // packet was inserted; no returned-count prefix is consulted.
        for compact in down_intents.iter().copied() {
            let generation_id = compact.generation_id();
            if generation_id == NO_GENERATION_ID {
                continue;
            }
            let slot = compact.key_slot();
            let slot_bit = Self::bit_for_slot(slot);
            if self.active_by_slot[usize::from(slot)].is_some()
                || self.active_mask & slot_bit != 0
                || self.blocked_mask & slot_bit != 0
            {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "packet Down would overwrite an active or blocked key slot".to_string(),
                    ),
                ));
            }
            let scan_code = self
                .schedule
                .key_registry
                .scan_code_for(slot)
                .ok_or(CoordinatorError::InvalidKeySlot { slot })?;
            self.transition_generation(
                generation_id,
                GenerationStatus::Scheduled,
                GenerationStatus::Active,
            )?;
            self.active_by_slot[usize::from(slot)] = Some(ActiveGeneration {
                generation_id,
                scan_code,
                key_slot: slot,
                source_action_index: down_source_action_index.unwrap_or(0),
                scheduled_down_ticks: prepared.effective_scheduled_ticks,
                down_dispatch_started_ticks: started,
                down_dispatch_completed_ticks: completed,
                release_not_before_ticks,
            });
            self.active_mask |= Self::bit_for_slot(slot);
            self.blocked_mask |= Self::bit_for_slot(slot);
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

    /// Validate only the bounded slot/mask ownership maintained by local
    /// transitions.  The full generation-ledger verifier remains reserved for
    /// explicit cleanup and test validation, never for a stale or physical
    /// packet deadline path.
    fn validate_local_slot_masks(&self) -> Result<(), CoordinatorError> {
        for slot in 0..MAX_KEYS {
            let bit = Self::bit_for_slot(slot as KeySlot);
            match self.active_by_slot[slot].as_ref() {
                Some(active) => {
                    if active.key_slot != slot as KeySlot
                        || self.active_mask & bit == 0
                        || self.blocked_mask & bit == 0
                    {
                        return Err(CoordinatorError::Invariant(
                            CoordinatorInvariantError::Accounting(
                                "active slot and ownership masks disagree".to_string(),
                            ),
                        ));
                    }
                }
                None if self.active_mask & bit != 0 || self.blocked_mask & bit != 0 => {
                    return Err(CoordinatorError::Invariant(
                        CoordinatorInvariantError::Accounting(
                            "ownership mask has no active slot owner".to_string(),
                        ),
                    ));
                }
                None => {}
            }
        }
        Ok(())
    }

    pub fn commit_down_success(
        &mut self,
        prepared: PreparedBatch,
        sent_scan_codes: &[u16],
        started: TimelineTicks,
        completed: TimelineTicks,
    ) -> Result<(), CoordinatorError> {
        if prepared.index != self.cursor {
            return Err(CoordinatorError::PreparedBatchMismatch {
                prepared: prepared.index,
                cursor: self.cursor,
            });
        }
        self.activate_sent_downs_compact_ticks(
            prepared.index,
            sent_scan_codes,
            started,
            completed,
            0,
        )?;
        self.cursor = self.cursor.checked_add(1).ok_or(CoordinatorError::Time(
            crate::time::TimeArithmeticError::Overflow,
        ))?;
        Ok(())
    }

    pub fn pop_next_due_authored_ticks(
        &mut self,
        now: TimelineTicks,
    ) -> Result<Option<usize>, CoordinatorError> {
        let Some(prepared) = self.prepare_next_due_authored_uncompensated(now)? else {
            return Ok(None);
        };
        self.cursor = self.cursor.checked_add(1).ok_or(CoordinatorError::Time(
            crate::time::TimeArithmeticError::Overflow,
        ))?;
        Ok(Some(prepared.index))
    }
}
