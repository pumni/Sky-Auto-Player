use super::{
    ActiveGeneration, CoordinatorError, CoordinatorInvariantError, GenerationStatus,
    PendingRelease, PreparedAuthoredCommit, PreparedAuthoredFrame, PreparedBatch,
    PreparedDeferredReleaseIntent, PreparedDownIntent, PreparedStalePacket,
    RuntimeDispatchCoordinator, physical_packet_kind,
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
    /// Completion-anchored releases are represented by the separate pending
    /// release table and never rewrite this authored timestamp.
    pub(crate) fn next_authored_ticks_uncompensated(
        &self,
    ) -> Result<Option<TimelineTicks>, CoordinatorError> {
        if self.schedule.batches.get(self.cursor).is_none() {
            return Ok(None);
        }
        Ok(Some(self.effective_batch_scheduled_ticks(self.cursor)?))
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

    /// Return the next authored boundary. Pending release boundaries are
    /// queried independently by the planner and selected per key.
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

    fn packet_intent_source_action_index(
        &self,
        packet: &CompiledPacket,
        intent_index: usize,
        kind: ActionKind,
    ) -> Result<u32, CoordinatorError> {
        let first_batch = usize::try_from(packet.first_batch_index).map_err(|_| {
            CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                "packet first batch index does not fit in usize".into(),
            ))
        })?;
        let batch_count = usize::from(packet.batch_count);
        let last_batch = first_batch
            .checked_add(batch_count)
            .ok_or(CoordinatorError::Time(
                crate::time::TimeArithmeticError::Overflow,
            ))?;
        for batch in self
            .schedule
            .batches
            .get(first_batch..last_batch)
            .ok_or(CoordinatorError::InvalidBatchIndex { index: first_batch })?
        {
            if batch.kind != kind {
                continue;
            }
            let start = usize::try_from(batch.intent_start).map_err(|_| {
                CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                    "intent start does not fit in usize".into(),
                ))
            })?;
            let end =
                start
                    .checked_add(usize::from(batch.intent_len))
                    .ok_or(CoordinatorError::Time(
                        crate::time::TimeArithmeticError::Overflow,
                    ))?;
            if (start..end).contains(&intent_index) {
                return Ok(batch.source_action_index);
            }
        }
        Err(CoordinatorError::Invariant(
            CoordinatorInvariantError::Accounting(
                "packet intent is not owned by its expected action kind".into(),
            ),
        ))
    }

    /// Classify the current immutable authored frame without mutating state.
    ///
    /// Release floors are evaluated per generation/key.  An unrelated
    /// deferred Up therefore does not change the authored target of a Down
    /// chord.  A deferred release required by that chord is structurally
    /// impossible and fails closed before any physical send.
    pub fn prepare_current_authored_frame(
        &self,
    ) -> Result<Option<PreparedAuthoredFrame>, CoordinatorError> {
        let Some(batch) = self.schedule.batches.get(self.cursor) else {
            return Ok(None);
        };
        let packet_index = usize::try_from(batch.packet_id).map_err(|_| {
            CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                "packet id does not fit in usize".into(),
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
        if packet.first_batch_index as usize != self.cursor {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "packet first batch does not match coordinator cursor".into(),
                ),
            ));
        }
        let authored_ticks = self.effective_batch_scheduled_ticks(self.cursor)?;
        let up_start = usize::try_from(packet.up_intent_start).map_err(|_| {
            CoordinatorError::Schedule(RuntimeScheduleError::InvalidPacketIntentRange {
                index: packet_index,
            })
        })?;
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

        let mut immediate_up_mask = 0u16;
        let mut deferred_up_mask = 0u16;
        let mut stale_up_count = 0u8;
        for (offset, compact) in up_intents.iter().copied().enumerate() {
            let generation_id = compact.generation_id();
            if generation_id == NO_GENERATION_ID {
                stale_up_count = stale_up_count.saturating_add(1);
                continue;
            }
            let slot = compact.key_slot();
            let bit = Self::bit_for_slot(slot);
            let Some(active) = self.active_for_slot(slot) else {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "authored Up has no active generation".into(),
                    ),
                ));
            };
            if active.generation_id != generation_id {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "authored Up generation does not own its key slot".into(),
                    ),
                ));
            }
            if active.release_not_before_ticks <= authored_ticks {
                immediate_up_mask |= bit;
            } else {
                deferred_up_mask |= bit;
            }
            if up_start.checked_add(offset).is_none() {
                return Err(CoordinatorError::Time(
                    crate::time::TimeArithmeticError::Overflow,
                ));
            }
        }

        let pending_blocked_mask = self
            .pending_release_by_slot
            .iter()
            .enumerate()
            .filter_map(|(slot, pending)| {
                pending.filter(|release| {
                    release.due_ticks > authored_ticks
                        && packet.down_mask & Self::bit_for_slot(slot as KeySlot) != 0
                })
            })
            .fold(0u16, |mask, pending| {
                mask | Self::bit_for_slot(pending.key_slot)
            });
        let blocked_mask = (deferred_up_mask | pending_blocked_mask) & packet.down_mask;
        if blocked_mask != 0 {
            let latest_required_release_ticks = self
                .pending_release_by_slot
                .iter()
                .flatten()
                .filter(|pending| blocked_mask & Self::bit_for_slot(pending.key_slot) != 0)
                .map(|pending| pending.due_ticks)
                .chain(up_intents.iter().filter_map(|compact| {
                    let slot = compact.key_slot();
                    if blocked_mask & Self::bit_for_slot(slot) == 0 {
                        return None;
                    }
                    self.active_for_slot(slot)
                        .map(|active| active.release_not_before_ticks)
                }))
                .max()
                .unwrap_or(authored_ticks);
            return Err(CoordinatorError::PhysicalDeadlineInfeasible {
                authored_ticks,
                blocked_mask,
                latest_required_release_ticks,
            });
        }

        Ok(Some(PreparedAuthoredFrame {
            first_batch_index: self.cursor,
            packet_index,
            packet_batch_count: usize::from(packet.batch_count),
            authored_ticks,
            immediate_up_mask,
            deferred_up_mask,
            down_mask: packet.down_mask,
            stale_up_count,
        }))
    }

    #[cfg(any(test, feature = "test-support"))]
    fn register_deferred_releases(
        &mut self,
        prepared: PreparedAuthoredFrame,
    ) -> Result<(), CoordinatorError> {
        if prepared.first_batch_index != self.cursor {
            return Err(CoordinatorError::PreparedBatchMismatch {
                prepared: prepared.first_batch_index,
                cursor: self.cursor,
            });
        }
        let packet = *self.schedule.packets.get(prepared.packet_index).ok_or(
            CoordinatorError::InvalidBatchIndex {
                index: prepared.packet_index,
            },
        )?;
        let up_start = usize::try_from(packet.up_intent_start).map_err(|_| {
            CoordinatorError::Schedule(RuntimeScheduleError::InvalidPacketIntentRange {
                index: prepared.packet_index,
            })
        })?;
        let up_end = up_start
            .checked_add(usize::from(packet.up_intent_len))
            .ok_or(CoordinatorError::Schedule(
                RuntimeScheduleError::InvalidPacketIntentRange {
                    index: prepared.packet_index,
                },
            ))?;
        let up_intents = self
            .schedule
            .intents
            .get(up_start..up_end)
            .ok_or(CoordinatorError::Schedule(
                RuntimeScheduleError::InvalidPacketIntentRange {
                    index: prepared.packet_index,
                },
            ))?
            .iter()
            .copied()
            .collect::<SmallVec<[_; MAX_KEYS]>>();

        for (offset, compact) in up_intents.into_iter().enumerate() {
            let generation_id = compact.generation_id();
            let slot = compact.key_slot();
            let bit = Self::bit_for_slot(slot);
            if generation_id == NO_GENERATION_ID || prepared.deferred_up_mask & bit == 0 {
                continue;
            }
            let active = self
                .active_for_slot(slot)
                .cloned()
                .ok_or(CoordinatorError::PendingReleaseOwnershipMismatch { slot })?;
            if active.generation_id != generation_id {
                return Err(CoordinatorError::PendingReleaseOwnershipMismatch { slot });
            }
            if self.pending_release_by_slot[usize::from(slot)].is_some() {
                return Err(CoordinatorError::PendingReleaseAlreadyRegistered { slot });
            }
            let source_action_index = self.packet_intent_source_action_index(
                &packet,
                up_start.checked_add(offset).ok_or(CoordinatorError::Time(
                    crate::time::TimeArithmeticError::Overflow,
                ))?,
                ActionKind::Up,
            )?;
            self.pending_release_by_slot[usize::from(slot)] = Some(PendingRelease {
                generation_id,
                key_slot: slot,
                authored_release_ticks: prepared.authored_ticks,
                due_ticks: active.release_not_before_ticks.max(prepared.authored_ticks),
                source_action_index,
            });
            self.pending_release_mask |= bit;
        }
        Ok(())
    }

    /// Consume an authored frame that has no immediate physical work.
    /// Deferred releases are registered independently so later authored frames
    /// are not held behind this metadata boundary.
    #[cfg(any(test, feature = "test-support"))]
    pub fn commit_authored_frame_metadata(
        &mut self,
        prepared: PreparedAuthoredFrame,
    ) -> Result<(), CoordinatorError> {
        if prepared.immediate_up_mask != 0 || prepared.down_mask != 0 {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "metadata commit contains physical authored work".into(),
                ),
            ));
        }
        let current = self.prepare_current_authored_frame()?.ok_or(
            CoordinatorError::PreparedBatchMismatch {
                prepared: prepared.first_batch_index,
                cursor: self.cursor,
            },
        )?;
        if current != prepared {
            return Err(CoordinatorError::PreparedBatchMismatch {
                prepared: prepared.first_batch_index,
                cursor: self.cursor,
            });
        }
        self.register_deferred_releases(prepared)?;
        self.cursor =
            self.cursor
                .checked_add(prepared.packet_batch_count)
                .ok_or(CoordinatorError::Time(
                    crate::time::TimeArithmeticError::Overflow,
                ))?;
        self.validate_local_slot_masks()?;
        Ok(())
    }

    /// Consume a metadata-only authored frame from frozen commit evidence.
    ///
    /// The planner builds this token before the timed wait.  The deadline
    /// path only registers the already-identified deferred releases and
    /// advances the cursor; it does not rediscover packet ranges or intents.
    pub fn commit_prepared_authored_frame_metadata_frozen(
        &mut self,
        commit: &PreparedAuthoredCommit,
    ) -> Result<(), CoordinatorError> {
        let prepared = commit.frame;
        if prepared.immediate_up_mask != 0
            || prepared.down_mask != 0
            || !commit.immediate_up_intents.is_empty()
            || !commit.down_intents.is_empty()
        {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "frozen metadata commit contains physical authored work".into(),
                ),
            ));
        }
        self.register_deferred_releases_frozen(prepared, &commit.deferred_up_intents)?;
        self.cursor =
            self.cursor
                .checked_add(prepared.packet_batch_count)
                .ok_or(CoordinatorError::Time(
                    crate::time::TimeArithmeticError::Overflow,
                ))?;
        self.validate_local_slot_masks()?;
        Ok(())
    }

    pub fn earliest_pending_release_ticks(&self) -> Option<TimelineTicks> {
        self.pending_release_by_slot
            .iter()
            .flatten()
            .map(|pending| pending.due_ticks)
            .min()
    }

    pub fn pending_release_mask_due_at(&self, target: TimelineTicks) -> u16 {
        self.pending_release_by_slot
            .iter()
            .flatten()
            .filter(|pending| pending.due_ticks == target)
            .fold(0u16, |mask, pending| {
                mask | Self::bit_for_slot(pending.key_slot)
            })
    }

    pub fn pending_release_source_action_index(&self, release_mask: u16) -> Option<u32> {
        self.pending_release_by_slot
            .iter()
            .flatten()
            .filter(|pending| release_mask & Self::bit_for_slot(pending.key_slot) != 0)
            .map(|pending| pending.source_action_index)
            .min()
    }

    /// Commit pending releases at one exact due boundary.  The caller may
    /// coalesce this mask with an authored Down transaction, but the logical
    /// Up transition must happen before the new Down transition.
    pub fn commit_pending_release_success(
        &mut self,
        release_mask: u16,
        started: TimelineTicks,
    ) -> Result<(), CoordinatorError> {
        let mut mask = release_mask;
        while mask != 0 {
            let slot = mask.trailing_zeros() as KeySlot;
            mask &= mask - 1;
            let bit = Self::bit_for_slot(slot);
            let pending = self
                .pending_release_for_slot(slot)
                .ok_or(CoordinatorError::PendingReleaseOwnershipMismatch { slot })?;
            let active = self
                .active_for_slot(slot)
                .ok_or(CoordinatorError::PendingReleaseOwnershipMismatch { slot })?;
            if active.generation_id != pending.generation_id || started < pending.due_ticks {
                return Err(CoordinatorError::PendingReleaseOwnershipMismatch { slot });
            }
            self.transition_generation(
                pending.generation_id,
                GenerationStatus::Active,
                GenerationStatus::Released,
            )?;
            self.active_by_slot[usize::from(slot)] = None;
            self.active_mask &= !bit;
            self.blocked_mask &= !bit;
            self.pending_release_by_slot[usize::from(slot)] = None;
            self.pending_release_mask &= !bit;
        }
        self.validate_local_slot_masks()?;
        Ok(())
    }

    /// Freeze all coordinator evidence required by the authored commit.  This
    /// is intentionally called before the timed wait; the completion path can
    /// then apply only the bounded token and never traverse schedule ranges.
    pub fn prepare_authored_commit(
        &self,
        prepared: PreparedAuthoredFrame,
    ) -> Result<PreparedAuthoredCommit, CoordinatorError> {
        let packet = *self.schedule.packets.get(prepared.packet_index).ok_or(
            CoordinatorError::InvalidBatchIndex {
                index: prepared.packet_index,
            },
        )?;
        let view = self
            .schedule
            .view_packet_ticks(prepared.packet_index, prepared.authored_ticks)?;
        let mut immediate_up_intents = SmallVec::new();
        let mut deferred_up_intents = SmallVec::new();
        let up_start = usize::try_from(packet.up_intent_start).map_err(|_| {
            CoordinatorError::Schedule(RuntimeScheduleError::InvalidPacketIntentRange {
                index: prepared.packet_index,
            })
        })?;
        for (offset, intent) in view.up_intents.iter().copied().enumerate() {
            if intent.generation_id() == NO_GENERATION_ID {
                continue;
            }
            let bit = Self::bit_for_slot(intent.key_slot());
            if prepared.immediate_up_mask & bit != 0 {
                immediate_up_intents.push(intent);
            } else if prepared.deferred_up_mask & bit != 0 {
                let source_action_index = self.packet_intent_source_action_index(
                    &packet,
                    up_start.checked_add(offset).ok_or(CoordinatorError::Time(
                        crate::time::TimeArithmeticError::Overflow,
                    ))?,
                    ActionKind::Up,
                )?;
                deferred_up_intents.push(PreparedDeferredReleaseIntent {
                    intent,
                    source_action_index,
                });
            }
        }
        let mut down_intents = SmallVec::new();
        for intent in view.down_intents.iter().copied() {
            let scan_code = self
                .schedule
                .key_registry
                .scan_code_for(intent.key_slot())
                .ok_or(CoordinatorError::InvalidKeySlot {
                    slot: intent.key_slot(),
                })?;
            down_intents.push(PreparedDownIntent { intent, scan_code });
        }
        Ok(PreparedAuthoredCommit {
            frame: prepared,
            immediate_up_intents,
            deferred_up_intents,
            down_intents,
            down_source_action_index: packet.down_source_action_index,
        })
    }

    fn register_deferred_releases_frozen(
        &mut self,
        prepared: PreparedAuthoredFrame,
        deferred_up_intents: &[PreparedDeferredReleaseIntent],
    ) -> Result<(), CoordinatorError> {
        if prepared.first_batch_index != self.cursor {
            return Err(CoordinatorError::PreparedBatchMismatch {
                prepared: prepared.first_batch_index,
                cursor: self.cursor,
            });
        }
        for release in deferred_up_intents {
            let generation_id = release.intent.generation_id();
            let slot = release.intent.key_slot();
            let bit = Self::bit_for_slot(slot);
            if prepared.deferred_up_mask & bit == 0 || generation_id == NO_GENERATION_ID {
                continue;
            }
            let active = self
                .active_for_slot(slot)
                .cloned()
                .ok_or(CoordinatorError::PendingReleaseOwnershipMismatch { slot })?;
            if active.generation_id != generation_id {
                return Err(CoordinatorError::PendingReleaseOwnershipMismatch { slot });
            }
            if self.pending_release_by_slot[usize::from(slot)].is_some() {
                return Err(CoordinatorError::PendingReleaseAlreadyRegistered { slot });
            }
            self.pending_release_by_slot[usize::from(slot)] = Some(PendingRelease {
                generation_id,
                key_slot: slot,
                authored_release_ticks: prepared.authored_ticks,
                due_ticks: active.release_not_before_ticks.max(prepared.authored_ticks),
                source_action_index: release.source_action_index,
            });
            self.pending_release_mask |= bit;
        }
        Ok(())
    }

    /// Apply a frozen authored commit after a successful physical send.
    /// There is no schedule or packet traversal on this path.
    pub fn commit_prepared_authored_frame_success_frozen(
        &mut self,
        commit: &PreparedAuthoredCommit,
        started: TimelineTicks,
        completed: TimelineTicks,
    ) -> Result<(), CoordinatorError> {
        let prepared = commit.frame;
        if prepared.first_batch_index != self.cursor {
            return Err(CoordinatorError::PreparedBatchMismatch {
                prepared: prepared.first_batch_index,
                cursor: self.cursor,
            });
        }
        let release_not_before_ticks = completed.checked_add_duration(self.min_hold_ticks)?;

        for compact in &commit.immediate_up_intents {
            let generation_id = compact.generation_id();
            let slot = compact.key_slot();
            let Some(active) = self.active_for_slot(slot).cloned() else {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "authored immediate Up has no active generation".into(),
                    ),
                ));
            };
            if active.generation_id != generation_id || started < active.release_not_before_ticks {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "authored immediate Up violates generation ownership or hold floor".into(),
                    ),
                ));
            }
            self.transition_generation(
                generation_id,
                GenerationStatus::Active,
                GenerationStatus::Released,
            )?;
            let bit = Self::bit_for_slot(slot);
            self.active_by_slot[usize::from(slot)] = None;
            self.active_mask &= !bit;
            self.blocked_mask &= !bit;
        }

        self.register_deferred_releases_frozen(prepared, &commit.deferred_up_intents)?;

        for down in &commit.down_intents {
            let generation_id = down.intent.generation_id();
            if generation_id == NO_GENERATION_ID {
                continue;
            }
            let slot = down.intent.key_slot();
            let bit = Self::bit_for_slot(slot);
            if self.active_by_slot[usize::from(slot)].is_some()
                || self.active_mask & bit != 0
                || self.blocked_mask & bit != 0
            {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "authored Down would overwrite an active or blocked key slot".into(),
                    ),
                ));
            }
            self.transition_generation(
                generation_id,
                GenerationStatus::Scheduled,
                GenerationStatus::Active,
            )?;
            self.active_by_slot[usize::from(slot)] = Some(ActiveGeneration {
                generation_id,
                scan_code: down.scan_code,
                key_slot: slot,
                source_action_index: commit.down_source_action_index.unwrap_or(0),
                scheduled_down_ticks: prepared.authored_ticks,
                down_dispatch_started_ticks: started,
                down_dispatch_completed_ticks: completed,
                release_not_before_ticks,
            });
            self.active_mask |= bit;
            self.blocked_mask |= bit;
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

    /// Commit one classified authored frame after its selected physical
    /// packet completed successfully.  Immediate releases are sent now;
    /// unrelated deferred releases are registered in the fixed pending table
    /// and never hold the frame's Down chord hostage.
    #[cfg(any(test, feature = "test-support"))]
    pub fn commit_prepared_authored_frame_success(
        &mut self,
        prepared: PreparedAuthoredFrame,
        immediate_up_intents: &[CompactIntent],
        down_intents: &[CompactIntent],
        down_source_action_index: Option<u32>,
        started: TimelineTicks,
        completed: TimelineTicks,
    ) -> Result<(), CoordinatorError> {
        if prepared.first_batch_index != self.cursor {
            return Err(CoordinatorError::PreparedBatchMismatch {
                prepared: prepared.first_batch_index,
                cursor: self.cursor,
            });
        }
        let current = self.prepare_current_authored_frame()?.ok_or(
            CoordinatorError::PreparedBatchMismatch {
                prepared: prepared.first_batch_index,
                cursor: self.cursor,
            },
        )?;
        if current != prepared {
            return Err(CoordinatorError::PreparedBatchMismatch {
                prepared: prepared.first_batch_index,
                cursor: self.cursor,
            });
        }
        let packet = *self.schedule.packets.get(prepared.packet_index).ok_or(
            CoordinatorError::InvalidBatchIndex {
                index: prepared.packet_index,
            },
        )?;
        if packet.first_batch_index as usize != prepared.first_batch_index
            || usize::from(packet.batch_count) != prepared.packet_batch_count
            || packet.down_source_action_index != down_source_action_index
        {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "prepared authored frame metadata changed before commit".into(),
                ),
            ));
        }
        let up_start = usize::try_from(packet.up_intent_start).map_err(|_| {
            CoordinatorError::Schedule(RuntimeScheduleError::InvalidPacketIntentRange {
                index: prepared.packet_index,
            })
        })?;
        let up_end = up_start
            .checked_add(usize::from(packet.up_intent_len))
            .ok_or(CoordinatorError::Schedule(
                RuntimeScheduleError::InvalidPacketIntentRange {
                    index: prepared.packet_index,
                },
            ))?;
        let down_start = usize::try_from(packet.down_intent_start).map_err(|_| {
            CoordinatorError::Schedule(RuntimeScheduleError::InvalidPacketIntentRange {
                index: prepared.packet_index,
            })
        })?;
        let down_end = down_start
            .checked_add(usize::from(packet.down_intent_len))
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
        let expected_up: SmallVec<[_; MAX_KEYS]> = current_up
            .iter()
            .copied()
            .filter(|intent| {
                let generation_id = intent.generation_id();
                generation_id != NO_GENERATION_ID
                    && prepared.immediate_up_mask & Self::bit_for_slot(intent.key_slot()) != 0
            })
            .collect();
        if expected_up.as_slice() != immediate_up_intents
            || current_down != down_intents
            || prepared.down_mask != packet.down_mask
        {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "prepared authored frame intents changed before commit".into(),
                ),
            ));
        }
        let release_not_before_ticks = completed.checked_add_duration(self.min_hold_ticks)?;

        for compact in immediate_up_intents.iter().copied() {
            let generation_id = compact.generation_id();
            let slot = compact.key_slot();
            let Some(active) = self.active_for_slot(slot).cloned() else {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "authored immediate Up has no active generation".into(),
                    ),
                ));
            };
            if active.generation_id != generation_id || started < active.release_not_before_ticks {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "authored immediate Up violates generation ownership or hold floor".into(),
                    ),
                ));
            }
            self.transition_generation(
                generation_id,
                GenerationStatus::Active,
                GenerationStatus::Released,
            )?;
            let bit = Self::bit_for_slot(slot);
            self.active_by_slot[usize::from(slot)] = None;
            self.active_mask &= !bit;
            self.blocked_mask &= !bit;
        }

        self.register_deferred_releases(prepared)?;

        for compact in down_intents.iter().copied() {
            let generation_id = compact.generation_id();
            if generation_id == NO_GENERATION_ID {
                continue;
            }
            let slot = compact.key_slot();
            let bit = Self::bit_for_slot(slot);
            if self.active_by_slot[usize::from(slot)].is_some()
                || self.active_mask & bit != 0
                || self.blocked_mask & bit != 0
            {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "authored Down would overwrite an active or blocked key slot".into(),
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
                scheduled_down_ticks: prepared.authored_ticks,
                down_dispatch_started_ticks: started,
                down_dispatch_completed_ticks: completed,
                release_not_before_ticks,
            });
            self.active_mask |= bit;
            self.blocked_mask |= bit;
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

    /// Prepare the current authored batch without consulting the current
    /// clock.  The authored timestamp is immutable; per-key completion floors
    /// are represented by pending releases in the new dispatch path.
    pub fn prepare_current_authored_batch(
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
        let effective_scheduled_ticks = self.effective_batch_scheduled_ticks(index)?;
        let packet_kind = physical_packet_kind(packet.up_mask, packet.down_mask)?;
        Ok(Some(PreparedBatch {
            index,
            effective_scheduled_ticks,
            packet_index,
            packet_batch_count: usize::from(packet.batch_count),
            packet_kind,
        }))
    }

    #[cfg(any(test, feature = "test-support"))]
    fn prepare_next_due_authored_uncompensated(
        &mut self,
        now: TimelineTicks,
    ) -> Result<Option<PreparedBatch>, CoordinatorError> {
        let Some(prepared) = self.prepare_current_authored_batch()? else {
            return Ok(None);
        };
        if prepared.effective_scheduled_ticks > now {
            return Ok(None);
        }
        Ok(Some(prepared))
    }

    #[cfg(test)]
    pub fn prepare_next_due_authored(
        &mut self,
        now: TimelineTicks,
        _dispatch_lead: DurationTicks,
    ) -> Result<Option<PreparedBatch>, CoordinatorError> {
        self.prepare_next_due_authored_uncompensated(now)
    }

    #[cfg(all(not(test), feature = "test-support"))]
    pub fn prepare_next_due_authored(
        &mut self,
        now: TimelineTicks,
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
    #[cfg(any(test, feature = "test-support"))]
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
    #[cfg(any(test, feature = "test-support"))]
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
        if self.effective_batch_scheduled_ticks(prepared.index)?
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
    #[inline]
    fn validate_local_slot_masks(&self) -> Result<(), CoordinatorError> {
        #[cfg(any(test, debug_assertions))]
        {
            self.validate_local_slot_masks_full()
        }
        #[cfg(not(any(test, debug_assertions)))]
        {
            Ok(())
        }
    }

    #[cfg(any(test, debug_assertions))]
    fn validate_local_slot_masks_full(&self) -> Result<(), CoordinatorError> {
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
            match self.pending_release_by_slot[slot] {
                Some(pending) => {
                    if pending.key_slot != slot as KeySlot || self.pending_release_mask & bit == 0 {
                        return Err(CoordinatorError::Invariant(
                            CoordinatorInvariantError::Accounting(
                                "pending slot and ownership mask disagree".to_string(),
                            ),
                        ));
                    }
                    let Some(active) = self.active_by_slot[slot].as_ref() else {
                        return Err(CoordinatorError::Invariant(
                            CoordinatorInvariantError::Accounting(
                                "pending slot has no active owner".to_string(),
                            ),
                        ));
                    };
                    if active.generation_id != pending.generation_id
                        || self.active_mask & bit == 0
                        || self.blocked_mask & bit == 0
                    {
                        return Err(CoordinatorError::Invariant(
                            CoordinatorInvariantError::Accounting(
                                "pending slot does not match active ownership".to_string(),
                            ),
                        ));
                    }
                }
                None if self.pending_release_mask & bit != 0 => {
                    return Err(CoordinatorError::Invariant(
                        CoordinatorInvariantError::Accounting(
                            "pending mask has no pending slot owner".to_string(),
                        ),
                    ));
                }
                None => {}
            }
        }
        if self
            .pending_release_by_slot
            .iter()
            .enumerate()
            .any(|(slot, pending)| {
                pending.is_some()
                    && self.pending_release_mask & Self::bit_for_slot(slot as KeySlot) == 0
            })
        {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "pending slot exists outside pending mask".to_string(),
                ),
            ));
        }
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
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

    #[cfg(any(test, feature = "test-support"))]
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
