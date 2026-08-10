use super::{
    ActiveGeneration, CoordinatorError, CoordinatorInvariantError, GenerationStatus, PreparedBatch,
    ReleaseRequestResult, RuntimeDispatchCoordinator, physical_packet_kind,
};
use crate::model::*;
use crate::time::{DurationTicks, TimelineTicks};
use smallvec::SmallVec;

impl RuntimeDispatchCoordinator {
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
            return Some(
                batch
                    .scheduled_us
                    .saturating_add(self.recovery_offset_ticks.as_u64()),
            );
        }
        let effective_scheduled_us = batch
            .scheduled_us
            .saturating_add(self.recovery_offset_ticks.as_u64());
        let effective_lead = Self::effective_authored_lead(effective_scheduled_us, lead);
        Some(effective_scheduled_us.saturating_sub(effective_lead))
    }

    /// Return the next authored dispatch deadline in the playback tick domain.
    /// The authored schedule remains immutable; recovery is represented only
    /// by `recovery_offset_ticks`.
    pub fn next_authored_ticks(
        &self,
        dispatch_lead: DurationTicks,
    ) -> Result<Option<TimelineTicks>, CoordinatorError> {
        let Some(batch) = self.schedule.batches.get(self.cursor) else {
            return Ok(None);
        };
        let packet_index = usize::try_from(batch.packet_id).map_err(|_| {
            CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                "packet id does not fit in usize".to_string(),
            ))
        })?;
        let effective = self.packet_effective_deadline_ticks(
            packet_index,
            if self.early_pop_blocked(batch) {
                DurationTicks::ZERO
            } else {
                dispatch_lead
            },
        )?;
        Ok(Some(effective))
    }

    /// Return the earliest upcoming physical boundary without applying an
    /// adaptive dispatch lead.
    ///
    /// Planning uses this projection to classify the interval that will
    /// actually precede the next physical operation.  Release floors and
    /// recovery ownership remain part of the projection; only lead
    /// subtraction is omitted.
    pub fn next_uncompensated_deadline_ticks(
        &self,
    ) -> Result<Option<TimelineTicks>, CoordinatorError> {
        let authored = self.next_authored_ticks(DurationTicks::ZERO)?;
        let pending = self.next_pending_release_ticks(DurationTicks::ZERO)?;
        if self.release_recovery_started_ticks.is_some() {
            return Ok(pending);
        }
        Ok(match (authored, pending) {
            (Some(authored), Some(pending)) => Some(authored.min(pending)),
            (Some(authored), None) => Some(authored),
            (None, Some(pending)) => Some(pending),
            (None, None) => None,
        })
    }

    /// Polyphony of the next authored down batch, used to select its lead
    /// before the batch is popped from the schedule.
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
        dispatch_lead_us: u64,
    ) -> Option<(RuntimeBatch, u64)> {
        let (index, lead) = self
            .pop_next_due_authored_ticks(
                TimelineTicks::from_raw(now_us),
                DurationTicks::from_raw(dispatch_lead_us),
            )
            .ok()??;
        let popped = self
            .schedule
            .materialize_batch(index, self.recovery_offset_ticks.as_u64());
        Some((popped, lead.as_u64()))
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
        dispatch_lead_us: u64,
    ) -> Option<(usize, u64)> {
        self.pop_next_due_authored_ticks(
            TimelineTicks::from_raw(now_us),
            DurationTicks::from_raw(dispatch_lead_us),
        )
        .ok()?
        .map(|(index, lead)| (index, lead.as_u64()))
    }

    pub fn prepare_next_due_authored(
        &mut self,
        now: TimelineTicks,
        dispatch_lead: DurationTicks,
    ) -> Result<Option<PreparedBatch>, CoordinatorError> {
        if self.cursor >= self.schedule.batches.len()
            || self.release_recovery_started_ticks.is_some()
        {
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
            self.packet_effective_deadline_ticks(packet_index, DurationTicks::ZERO)?;
        let deadline = self.packet_effective_deadline_ticks(packet_index, dispatch_lead)?;
        let effective_lead = DurationTicks::from_raw(
            effective_scheduled_ticks
                .as_u64()
                .saturating_sub(deadline.as_u64()),
        );
        if deadline > now || (effective_scheduled_ticks > now && self.early_pop_blocked(&batch)) {
            return Ok(None);
        }
        let packet_kind = match physical_packet_kind(packet.up_mask, packet.down_mask) {
            Ok(kind) => Some(kind),
            Err(error)
                if packet.up_intent_len > 0
                    && packet.down_intent_len == 0
                    && self
                        .schedule
                        .intents
                        .get(
                            packet.up_intent_start as usize
                                ..(packet.up_intent_start as usize
                                    + usize::from(packet.up_intent_len)),
                        )
                        .is_some_and(|intents| {
                            intents
                                .iter()
                                .all(|intent| intent.generation_id() == NO_GENERATION_ID)
                        }) =>
            {
                let _ = error;
                None
            }
            Err(error) => return Err(error),
        };
        Ok(Some(PreparedBatch {
            index,
            effective_scheduled_ticks,
            effective_lead_ticks: effective_lead,
            packet_index,
            packet_batch_count: usize::from(packet.batch_count),
            packet_kind,
        }))
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
        if prepared.index != self.cursor {
            return Err(CoordinatorError::PreparedBatchMismatch {
                prepared: prepared.index,
                cursor: self.cursor,
            });
        }
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
        if prepared.packet_batch_count == 0 {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "compiled packet must contain at least one authored batch".to_string(),
                ),
            ));
        }
        let release_not_before_ticks = completed
            .checked_add_duration(self.min_hold_ticks)
            .and_then(|ticks| ticks.checked_add_duration(self.delivery_margin_ticks))?;

        // Apply releases first. Stale Up intents are present for authored
        // diagnostics but deliberately have NO_GENERATION_ID and no physical
        // event in the packet.
        for compact in up_intents {
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
            self.transition_generation(
                generation_id,
                GenerationStatus::Active,
                GenerationStatus::ReleasePending,
            )?;
            self.transition_generation(
                generation_id,
                GenerationStatus::ReleasePending,
                GenerationStatus::Released,
            )?;
            self.active_by_slot[usize::from(slot)] = None;
            self.active_mask &= !Self::bit_for_slot(slot);
            self.blocked_mask &= !Self::bit_for_slot(slot);
        }

        // Full SendInput success means every Down identity in the immutable
        // packet was inserted; no returned-count prefix is consulted.
        for compact in down_intents {
            let generation_id = compact.generation_id();
            if generation_id == NO_GENERATION_ID {
                continue;
            }
            let slot = compact.key_slot();
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
        self.check_invariants()?;
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

    pub fn commit_up_request(
        &mut self,
        prepared: PreparedBatch,
    ) -> Result<ReleaseRequestResult, CoordinatorError> {
        if prepared.index != self.cursor {
            return Err(CoordinatorError::PreparedBatchMismatch {
                prepared: prepared.index,
                cursor: self.cursor,
            });
        }
        let batch = self
            .schedule
            .view_batch_ticks(prepared.index, prepared.effective_scheduled_ticks)?
            .materialize();
        let result = self.request_releases(&batch.intents)?;
        self.cursor = self.cursor.checked_add(1).ok_or(CoordinatorError::Time(
            crate::time::TimeArithmeticError::Overflow,
        ))?;
        Ok(result)
    }

    pub fn pop_next_due_authored_ticks(
        &mut self,
        now: TimelineTicks,
        dispatch_lead: DurationTicks,
    ) -> Result<Option<(usize, DurationTicks)>, CoordinatorError> {
        let Some(prepared) = self.prepare_next_due_authored(now, dispatch_lead)? else {
            return Ok(None);
        };
        self.cursor = self.cursor.checked_add(1).ok_or(CoordinatorError::Time(
            crate::time::TimeArithmeticError::Overflow,
        ))?;
        Ok(Some((prepared.index, prepared.effective_lead_ticks)))
    }
}
