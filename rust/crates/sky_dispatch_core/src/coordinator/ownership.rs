#[cfg(test)]
use super::RELEASE_RETRY_BACKOFF_US;
use super::{
    ActiveGeneration, CoordinatorError, CoordinatorInvariantError, GenerationStatus,
    MAX_RELEASE_RETRIES, PendingRelease, ReleaseRequestResult, RuntimeDispatchCoordinator,
    TimelineRebaseReason,
};
use crate::model::*;
use crate::time::{DurationTicks, TimelineTicks};
use smallvec::SmallVec;

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
    ///
    /// The authored timestamp and packet/batch ordering remain unchanged.
    /// Only the lifecycle identity and derived physical Up mask are changed;
    /// otherwise a later release-floor lookup would correctly reject a real
    /// Up that no longer has an active generation.  This is lifecycle
    /// reconciliation, not a timeline rebase.
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

    /// Activate sent downs from a batch index without allocating.
    ///
    /// `sent_scan_codes` is the slice returned by `SendInput` bookkeeping —
    /// only keys present in this slice are activated.  Keys in the batch
    /// that were not sent are terminated as `DroppedBackend`.
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
            let Some(sc) = self.schedule.key_registry.scan_code_for(slot) else {
                return Err(CoordinatorError::InvalidKeySlot { slot });
            };
            if !sent_scan_codes.contains(&sc) {
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
                scan_code: sc,
                key_slot: slot,
                source_action_index,
                scheduled_down_ticks: self
                    .batch_scheduled_ticks
                    .get(batch_index)
                    .copied()
                    .ok_or(CoordinatorError::InvalidBatchIndex { index: batch_index })?
                    .checked_add_duration(self.recovery_offset_ticks)?,
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

        if sent_scan_codes.len() == 1 {
            let only_sent = sent_scan_codes[0];
            for intent in intents {
                let Some(generation_id) = intent.generation_id else {
                    continue;
                };
                if intent.scan_code != only_sent {
                    // Terminalize without touching any mask — slot was never activated.
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
                self.active_by_slot[intent.key_slot as usize] = Some(ActiveGeneration {
                    generation_id,
                    scan_code: intent.scan_code,
                    key_slot: intent.key_slot,
                    source_action_index: intent.source_action_index,
                    scheduled_down_ticks: self
                        .batch_scheduled_ticks
                        .get(intent.compiled_batch_index)
                        .copied()
                        .ok_or(CoordinatorError::InvalidBatchIndex {
                            index: intent.compiled_batch_index,
                        })?
                        .checked_add_duration(self.recovery_offset_ticks)?,
                    down_dispatch_started_ticks: dispatch_started_ticks,
                    down_dispatch_completed_ticks: dispatch_completed_ticks,
                    release_not_before_ticks,
                });
                self.active_mask |= Self::bit_for_slot(intent.key_slot);
                self.blocked_mask |= Self::bit_for_slot(intent.key_slot);
                // No HashMap insertion — active count is derived from active_mask at query time.
            }
            return Ok(());
        }

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
            self.active_by_slot[intent.key_slot as usize] = Some(ActiveGeneration {
                generation_id,
                scan_code: intent.scan_code,
                key_slot: intent.key_slot,
                source_action_index: intent.source_action_index,
                scheduled_down_ticks: self
                    .batch_scheduled_ticks
                    .get(intent.compiled_batch_index)
                    .copied()
                    .ok_or(CoordinatorError::InvalidBatchIndex {
                        index: intent.compiled_batch_index,
                    })?
                    .checked_add_duration(self.recovery_offset_ticks)?,
                down_dispatch_started_ticks: dispatch_started_ticks,
                down_dispatch_completed_ticks: dispatch_completed_ticks,
                release_not_before_ticks,
            });
            self.active_mask |= Self::bit_for_slot(intent.key_slot);
            self.blocked_mask |= Self::bit_for_slot(intent.key_slot);
            // No HashMap insertion — active count is derived from active_mask at query time.
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

    pub fn request_releases(
        &mut self,
        intents: &[RuntimeKeyIntent],
    ) -> Result<ReleaseRequestResult, CoordinatorError> {
        if intents.len() == 1 {
            let intent = &intents[0];
            let Some(generation_id) = intent.generation_id else {
                return Ok((SmallVec::new(), std::iter::once(intent.clone()).collect()));
            };
            let active = self.active_for_slot(intent.key_slot).cloned();
            let Some(active) = active else {
                return Ok((SmallVec::new(), std::iter::once(intent.clone()).collect()));
            };
            if active.generation_id != generation_id {
                return Ok((SmallVec::new(), std::iter::once(intent.clone()).collect()));
            }
            self.transition_generation(
                generation_id,
                GenerationStatus::Active,
                GenerationStatus::ReleasePending,
            )?;

            let pending = PendingRelease {
                generation_id,
                scan_code: intent.scan_code,
                key_slot: intent.key_slot,
                source_action_index: intent.source_action_index,
                packet_id: intent.packet_id,
                scheduled_release_us: intent.scheduled_us,
                scheduled_release_ticks: self
                    .batch_scheduled_ticks
                    .get(intent.compiled_batch_index)
                    .copied()
                    .ok_or(CoordinatorError::InvalidBatchIndex {
                        index: intent.compiled_batch_index,
                    })?
                    .checked_add_duration(self.recovery_offset_ticks)?,
                down_dispatch_started_ticks: active.down_dispatch_started_ticks,
                release_not_before_ticks: active.release_not_before_ticks,
                reason_id: intent.reason_id,
                retry_count: 0,
                next_retry_ticks: crate::time::TimelineTicks::ZERO,
                first_failure_ticks: None,
                last_win32_error: None,
            };

            self.pending_by_slot[intent.key_slot as usize] = Some(pending.clone());
            self.pending_mask |= Self::bit_for_slot(intent.key_slot);
            self.active_mask &= !Self::bit_for_slot(intent.key_slot);
            // No HashMap insertion — release_pending count is derived from pending_mask.
            return Ok((std::iter::once(pending).collect(), SmallVec::new()));
        }

        let mut requested = SmallVec::new();
        let mut suppressed = SmallVec::new();

        for intent in intents {
            let Some(generation_id) = intent.generation_id else {
                suppressed.push(intent.clone());
                continue;
            };
            let active = self.active_for_slot(intent.key_slot).cloned();
            let Some(active) = active else {
                suppressed.push(intent.clone());
                continue;
            };
            if active.generation_id != generation_id {
                suppressed.push(intent.clone());
                continue;
            }
            self.transition_generation(
                generation_id,
                GenerationStatus::Active,
                GenerationStatus::ReleasePending,
            )?;

            let pending = PendingRelease {
                generation_id,
                scan_code: intent.scan_code,
                key_slot: intent.key_slot,
                source_action_index: intent.source_action_index,
                packet_id: intent.packet_id,
                scheduled_release_us: intent.scheduled_us,
                scheduled_release_ticks: self
                    .batch_scheduled_ticks
                    .get(intent.compiled_batch_index)
                    .copied()
                    .ok_or(CoordinatorError::InvalidBatchIndex {
                        index: intent.compiled_batch_index,
                    })?
                    .checked_add_duration(self.recovery_offset_ticks)?,
                down_dispatch_started_ticks: active.down_dispatch_started_ticks,
                release_not_before_ticks: active.release_not_before_ticks,
                reason_id: intent.reason_id,
                retry_count: 0,
                next_retry_ticks: crate::time::TimelineTicks::ZERO,
                first_failure_ticks: None,
                last_win32_error: None,
            };

            self.pending_by_slot[intent.key_slot as usize] = Some(pending.clone());
            self.pending_mask |= Self::bit_for_slot(intent.key_slot);
            self.active_mask &= !Self::bit_for_slot(intent.key_slot);
            // No HashMap insertion — release_pending count is derived from pending_mask.
            requested.push(pending);
        }

        Ok((requested, suppressed))
    }

    /// Complete exactly the pending releases whose key was confirmed by
    /// confirmed physical transport evidence (`confirmed_mask`).
    ///
    /// Only a confirmed bit transitions `ReleasePending -> Released`, clears
    /// the active/pending/blocked state. Any unconfirmed bit stays
    /// `ReleasePending` and remains pending/blocked; it is never coalesced to
    /// `DroppedBackend` merely to unblock the key.
    pub fn complete_releases_mask(
        &mut self,
        releases: &[PendingRelease],
        confirmed_mask: u16,
    ) -> Result<(), CoordinatorError> {
        for pending in releases {
            let bit = Self::bit_for_slot(pending.key_slot);
            if (confirmed_mask & bit) == 0 {
                continue;
            }
            self.transition_generation(
                pending.generation_id,
                GenerationStatus::ReleasePending,
                GenerationStatus::Released,
            )?;
            if matches!(self.active_for_slot(pending.key_slot), Some(active) if active.generation_id == pending.generation_id)
            {
                self.active_by_slot[pending.key_slot as usize] = None;
                self.active_mask &= !bit;
                self.blocked_mask &= !bit;
            }
            self.pending_mask &= !bit;
            self.pending_by_slot[pending.key_slot as usize] = None;
        }
        Ok(())
    }

    pub fn complete_releases(
        &mut self,
        releases: &[PendingRelease],
        sent_scan_codes: &[u16],
    ) -> Result<(), CoordinatorError> {
        let mut confirmed_mask = 0u16;
        for pending in releases {
            let bit = Self::bit_for_slot(pending.key_slot);
            if sent_scan_codes.contains(&pending.scan_code) {
                confirmed_mask |= bit;
            }
        }
        self.complete_releases_mask(releases, confirmed_mask)
    }

    pub fn release_recovery_active(&self) -> bool {
        self.release_recovery_started_ticks.is_some()
    }

    /// End a recovery pause after the pending release set is empty.
    ///
    /// The authored schedule is immutable.  A single offset moves the
    /// effective playback timeline, so recovery remains O(1) regardless of
    /// how many batches are still queued.
    #[cfg(test)]
    pub fn finish_release_recovery(
        &mut self,
        completed_us: u64,
    ) -> Result<Option<u64>, CoordinatorError> {
        self.finish_release_recovery_ticks(TimelineTicks::from_raw(completed_us))
            .map(|pause| pause.map(DurationTicks::as_u64))
    }

    pub fn finish_release_recovery_ticks(
        &mut self,
        completed: TimelineTicks,
    ) -> Result<Option<DurationTicks>, CoordinatorError> {
        if self.pending_mask != 0 {
            return Ok(None);
        }
        let Some(started) = self.release_recovery_started_ticks.take() else {
            return Ok(None);
        };
        let pause = completed.checked_duration_since(started)?;
        if pause != DurationTicks::ZERO {
            self.apply_timeline_rebase(pause, TimelineRebaseReason::ReleaseRecovery)?;
        }
        Ok(Some(pause))
    }

    /// Requeue release work that did not reach the operating-system input
    /// stream. The active generation remains owned by the coordinator while
    /// bounded retries are pending; callers must stop playback and perform
    /// full-instrument recovery when this returns `true`.
    ///
    /// `recovery_started_us` is sampled immediately before the failed backend
    /// call. `retry_base_us` is sampled from backend completion and is used
    /// only to schedule the next retry after the call has returned.
    #[cfg(test)]
    pub fn requeue_failed_releases(
        &mut self,
        releases: &[PendingRelease],
        sent_scan_codes: &[u16],
        recovery_started_us: u64,
        retry_base_us: u64,
        last_win32_error: Option<u32>,
    ) -> Result<bool, CoordinatorError> {
        let backoff = RELEASE_RETRY_BACKOFF_US.map(DurationTicks::from_raw);
        let recovery_required = self.requeue_failed_releases_ticks(
            releases,
            sent_scan_codes,
            TimelineTicks::from_raw(recovery_started_us),
            TimelineTicks::from_raw(retry_base_us),
            &backoff,
            last_win32_error,
        )?;
        Ok(recovery_required)
    }

    /// Requeue exactly the pending releases that were not confirmed by
    /// confirmed physical transport evidence.
    ///
    /// A confirmed bit is never retried. Every unconfirmed bit is retried
    /// (or triggers controlled full-instrument recovery once the retry budget
    /// is exhausted). Skipped transport is *not* treated as acknowledged
    /// success; an unconfirmed key stays owned by the coordinator until it is
    /// physically confirmed or recovery is forced.
    #[allow(clippy::too_many_arguments)]
    pub fn requeue_unconfirmed_releases_ticks(
        &mut self,
        releases: &[PendingRelease],
        confirmed_mask: u16,
        recovery_started: TimelineTicks,
        retry_base: TimelineTicks,
        retry_backoff: &[DurationTicks],
        last_win32_error: Option<u32>,
    ) -> Result<bool, CoordinatorError> {
        if retry_backoff.is_empty() {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "release retry backoff must not be empty".to_string(),
                ),
            ));
        }
        let mut recovery_required = false;
        for pending in releases {
            let bit = Self::bit_for_slot(pending.key_slot);
            if (confirmed_mask & bit) != 0 {
                continue;
            }
            let is_matching_gen = self.pending_by_slot[pending.key_slot as usize]
                .as_ref()
                .map_or_else(
                    || {
                        self.active_for_slot(pending.key_slot)
                            .is_some_and(|active| active.generation_id == pending.generation_id)
                    },
                    |p| p.generation_id == pending.generation_id,
                );
            if !is_matching_gen {
                continue;
            }
            let Some(retry_count) = pending.retry_count.checked_add(1) else {
                recovery_required = true;
                continue;
            };
            if retry_count > MAX_RELEASE_RETRIES {
                recovery_required = true;
                continue;
            }
            let delay_index = usize::from(retry_count - 1).min(retry_backoff.len() - 1);
            let next_retry = retry_base.checked_add_duration(retry_backoff[delay_index])?;
            let mut retry = pending.clone();
            self.release_recovery_started_ticks
                .get_or_insert(recovery_started);
            retry.retry_count = retry_count;
            retry.next_retry_ticks = next_retry;
            retry.first_failure_ticks =
                Some(pending.first_failure_ticks.unwrap_or(recovery_started));
            retry.last_win32_error = last_win32_error.or(pending.last_win32_error);
            let retry_slot = usize::from(retry.key_slot);
            if retry_slot >= MAX_KEYS {
                return Err(CoordinatorError::InvalidKeySlot {
                    slot: retry.key_slot,
                });
            }
            self.pending_mask |= bit;
            self.pending_by_slot[retry_slot] = Some(retry);
        }
        self.check_invariants()?;
        Ok(recovery_required)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn requeue_failed_releases_ticks(
        &mut self,
        releases: &[PendingRelease],
        sent_scan_codes: &[u16],
        recovery_started: TimelineTicks,
        retry_base: TimelineTicks,
        retry_backoff: &[DurationTicks],
        last_win32_error: Option<u32>,
    ) -> Result<bool, CoordinatorError> {
        let mut confirmed_mask = 0u16;
        for pending in releases {
            let bit = Self::bit_for_slot(pending.key_slot);
            if sent_scan_codes.contains(&pending.scan_code) {
                confirmed_mask |= bit;
            }
        }
        self.requeue_unconfirmed_releases_ticks(
            releases,
            confirmed_mask,
            recovery_started,
            retry_base,
            retry_backoff,
            last_win32_error,
        )
    }
}
