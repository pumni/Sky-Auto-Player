use super::{
    ActiveGeneration, AuthoredUpIdentity, CoordinatorError, CoordinatorInvariantError,
    GenerationStatus, PreparedAuthoredCommit, RuntimeDispatchCoordinator,
};
use crate::model::*;
use crate::time::TimelineTicks;

impl RuntimeDispatchCoordinator {
    /// Apply one checked lifecycle transition. Generation IDs are only used
    /// to address the compiler-owned contiguous ledger, never as a batch
    /// timestamp/index identity.
    pub(super) fn transition_generation(
        &mut self,
        generation_id: GenerationId,
        expected: GenerationStatus,
        next: GenerationStatus,
    ) -> Result<(), CoordinatorError> {
        let Some(actual) = self.generation_states.get(generation_id as usize).copied() else {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::UnknownGeneration {
                    generation_id,
                    generation_count: self.generation_count,
                },
            ));
        };
        if actual != expected {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::UnexpectedTransition {
                    generation_id,
                    expected,
                    actual,
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
        if matches!(
            (expected, next),
            (GenerationStatus::Scheduled, GenerationStatus::Active)
        ) {
            self.activated_generation_count = self
                .activated_generation_count
                .checked_add(1)
                .ok_or_else(|| {
                    CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                        "activated generation counter overflow".to_string(),
                    ))
                })?;
        }
        if next.is_terminal() {
            self.counters.increment(next)?;
        }
        self.generation_states[generation_id as usize] = next;
        Ok(())
    }

    /// Reconcile one generation whose physical ownership was released by a
    /// resumable suspension before its immutable authored Up was dispatched.
    ///
    /// This is deliberately separate from [`transition_generation`]: a
    /// cancellation is not generally releasable, and arbitrary
    /// `Cancelled -> Released` transitions must remain impossible.
    pub(super) fn reconcile_resumable_cancelled_up(
        &mut self,
        identity: AuthoredUpIdentity,
    ) -> Result<(), CoordinatorError> {
        let generation_id = identity.generation_id();
        let slot = identity.key_slot();
        if usize::from(slot) >= MAX_KEYS
            || self.cancelled_up_by_slot[usize::from(slot)] != Some(identity)
        {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "authored Up does not own the exact live resumable cancellation token".into(),
                ),
            ));
        }
        let bit = Self::bit_for_slot(slot);
        if self.active_by_slot[usize::from(slot)].is_some()
            || self.active_mask & bit != 0
            || self.blocked_mask & bit != 0
            || self.pending_release_by_slot[usize::from(slot)].is_some()
        {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "resumable cancellation token conflicts with live coordinator ownership".into(),
                ),
            ));
        }
        self.validate_invalidated_up_identity(identity)
            .map_err(CoordinatorError::Invariant)?;
        let Some(actual) = self.generation_states.get(generation_id as usize).copied() else {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::UnknownGeneration {
                    generation_id,
                    generation_count: self.generation_count,
                },
            ));
        };
        if actual != GenerationStatus::Cancelled {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::UnexpectedTransition {
                    generation_id,
                    expected: GenerationStatus::Cancelled,
                    actual,
                    next: GenerationStatus::Released,
                },
            ));
        }
        if self.counters.cancelled == 0 {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "cancelled generation counter underflow during suspension reconciliation"
                        .into(),
                ),
            ));
        }
        let released = self.counters.released.checked_add(1).ok_or_else(|| {
            CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                "released generation counter overflow during suspension reconciliation".into(),
            ))
        })?;
        self.counters.cancelled -= 1;
        self.counters.released = released;
        self.generation_states[generation_id as usize] = GenerationStatus::Released;
        self.cancelled_up_by_slot[usize::from(slot)] = None;
        Ok(())
    }

    /// Validate all frozen Up identities against compiler locations and any
    /// live resumable cancellation token before a physical packet is sent.
    /// The operation is bounded by the current packet's at-most-fifteen Ups.
    pub fn validate_frozen_authored_commit(
        &self,
        commit: &PreparedAuthoredCommit,
    ) -> Result<(), CoordinatorError> {
        if commit.frame.packet_index >= self.schedule.packets.len() {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "frozen authored commit references an unknown packet".into(),
                ),
            ));
        }
        for up in &commit.up_intents {
            let identity = up.identity;
            if usize::from(identity.key_slot()) >= MAX_KEYS
                || identity.generation_id() != up.intent.generation_id()
                || identity.key_slot() != up.intent.key_slot()
                || identity.packet_index() != commit.frame.packet_index
            {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "frozen authored Up identity does not match its intent or packet".into(),
                    ),
                ));
            }
            let Some(mapped) = self
                .up_intent_locations
                .get(identity.generation_id() as usize)
                .copied()
            else {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::UnknownGeneration {
                        generation_id: identity.generation_id(),
                        generation_count: self.generation_count,
                    },
                ));
            };
            if mapped != Some((identity.packet_index(), identity.intent_index())) {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "frozen authored Up identity differs from compiler-owned location".into(),
                    ),
                ));
            }
            let Some(state) = self
                .generation_states
                .get(identity.generation_id() as usize)
                .copied()
            else {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::UnknownGeneration {
                        generation_id: identity.generation_id(),
                        generation_count: self.generation_count,
                    },
                ));
            };
            let compact = self
                .schedule
                .intents
                .get(identity.intent_index())
                .copied()
                .ok_or_else(|| {
                    CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                        "frozen authored Up intent location is outside the schedule".into(),
                    ))
                })?;
            match state {
                GenerationStatus::Active
                    if compact.generation_id() == identity.generation_id()
                        && compact.key_slot() == identity.key_slot()
                        && self.active_by_slot[usize::from(identity.key_slot())]
                            .as_ref()
                            .is_some_and(|active| {
                                active.generation_id == identity.generation_id()
                                    && active.key_slot == identity.key_slot()
                            })
                        && self.cancelled_up_by_slot[usize::from(identity.key_slot())]
                            .is_none() =>
                {
                    // Ordinary live-owner Up.
                }
                GenerationStatus::Cancelled
                    if self.cancelled_up_by_slot[usize::from(identity.key_slot())]
                        == Some(identity) =>
                {
                    let bit = Self::bit_for_slot(identity.key_slot());
                    if self.active_by_slot[usize::from(identity.key_slot())].is_some()
                        || self.active_mask & bit != 0
                        || self.blocked_mask & bit != 0
                    {
                        return Err(CoordinatorError::Invariant(
                            CoordinatorInvariantError::Accounting(
                                "cancelled-Up token conflicts with a live key owner".into(),
                            ),
                        ));
                    }
                    self.validate_invalidated_up_identity(identity)
                        .map_err(CoordinatorError::Invariant)?;
                }
                GenerationStatus::DroppedExpired if self.frozen_up_is_invalidated(identity) => {
                    // The immutable prepared stream may retain a Down whose
                    // authored Up was invalidated after it was dropped.
                }
                _ => {
                    return Err(CoordinatorError::Invariant(
                        CoordinatorInvariantError::Accounting(format!(
                            "frozen authored Up for generation {} has no matching active owner or cancellation proof",
                            identity.generation_id()
                        )),
                    ));
                }
            }
        }
        for down in &commit.down_intents {
            let slot = down.intent.key_slot();
            if usize::from(slot) >= MAX_KEYS
                || self.cancelled_up_by_slot[usize::from(slot)].is_some()
            {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(format!(
                        "frozen authored Down cannot pass a live cancelled-Up token for slot {slot}"
                    )),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn authored_up_identity_for_live_generation(
        &self,
        generation_id: GenerationId,
        key_slot: KeySlot,
    ) -> Result<AuthoredUpIdentity, CoordinatorError> {
        let Some((packet_index, intent_index)) = self
            .up_intent_locations
            .get(generation_id as usize)
            .copied()
            .flatten()
        else {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(format!(
                    "active generation {generation_id} has no authored Up location"
                )),
            ));
        };
        let compact = self
            .schedule
            .intents
            .get(intent_index)
            .copied()
            .ok_or_else(|| {
                CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                    "active generation authored Up location is outside the schedule".into(),
                ))
            })?;
        if compact.generation_id() != generation_id || compact.key_slot() != key_slot {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "active generation authored Up location does not match its owner".into(),
                ),
            ));
        }
        let packet = self.schedule.packets.get(packet_index).ok_or_else(|| {
            CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                "active generation authored Up packet is outside the schedule".into(),
            ))
        })?;
        let start = packet.up_intent_start as usize;
        let end = start
            .checked_add(usize::from(packet.up_intent_len))
            .ok_or_else(|| {
                CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                    "active generation authored Up range overflowed".into(),
                ))
            })?;
        if intent_index < start
            || intent_index >= end
            || packet.up_mask & Self::bit_for_slot(key_slot) == 0
        {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "active generation authored Up is not in its compiler packet".into(),
                ),
            ));
        }
        Ok(AuthoredUpIdentity {
            generation_id,
            key_slot,
            packet_index,
            intent_index,
        })
    }

    pub(super) fn invalidate_authored_up_for_resumable_cancellation(
        &mut self,
        identity: AuthoredUpIdentity,
    ) -> Result<(), CoordinatorError> {
        let generation_id = identity.generation_id();
        let Some(compact) = self.schedule.intents.get(identity.intent_index()).copied() else {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "resumable cancellation Up location is outside the schedule".into(),
                ),
            ));
        };
        if compact.generation_id() != generation_id || compact.key_slot() != identity.key_slot() {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "resumable cancellation Up identity changed before invalidation".into(),
                ),
            ));
        }
        let packet = self
            .schedule
            .packets
            .get(identity.packet_index())
            .ok_or_else(|| {
                CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                    "resumable cancellation Up packet is outside the schedule".into(),
                ))
            })?;
        let start = packet.up_intent_start as usize;
        let end = start
            .checked_add(usize::from(packet.up_intent_len))
            .ok_or_else(|| {
                CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                    "resumable cancellation Up range overflowed".into(),
                ))
            })?;
        if identity.intent_index() < start
            || identity.intent_index() >= end
            || packet.up_mask & Self::bit_for_slot(identity.key_slot()) == 0
        {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "resumable cancellation Up is absent from its physical authored packet".into(),
                ),
            ));
        }
        // Retain the authored physical Up. The earlier safety cleanup and the
        // later immutable authored packet are distinct observable events.
        self.schedule.intents[identity.intent_index()] =
            CompactIntent::new(NO_GENERATION_ID, identity.key_slot());
        Ok(())
    }

    pub(super) fn validate_invalidated_up_identity(
        &self,
        identity: AuthoredUpIdentity,
    ) -> Result<(), CoordinatorInvariantError> {
        let Some(mapped) = self
            .up_intent_locations
            .get(identity.generation_id() as usize)
            .copied()
        else {
            return Err(CoordinatorInvariantError::UnknownGeneration {
                generation_id: identity.generation_id(),
                generation_count: self.generation_count,
            });
        };
        if mapped != Some((identity.packet_index(), identity.intent_index())) {
            return Err(CoordinatorInvariantError::Accounting(
                "cancelled-Up token differs from compiler-owned location".into(),
            ));
        }
        let Some(compact) = self.schedule.intents.get(identity.intent_index()) else {
            return Err(CoordinatorInvariantError::Accounting(
                "cancelled-Up token location is outside the schedule".into(),
            ));
        };
        let Some(packet) = self.schedule.packets.get(identity.packet_index()) else {
            return Err(CoordinatorInvariantError::Accounting(
                "cancelled-Up token packet is outside the schedule".into(),
            ));
        };
        let start = packet.up_intent_start as usize;
        let end = start
            .checked_add(usize::from(packet.up_intent_len))
            .ok_or_else(|| {
                CoordinatorInvariantError::Accounting("cancelled-Up token range overflowed".into())
            })?;
        if compact.generation_id() != NO_GENERATION_ID
            || compact.key_slot() != identity.key_slot()
            || identity.intent_index() < start
            || identity.intent_index() >= end
            || packet.up_mask & Self::bit_for_slot(identity.key_slot()) == 0
        {
            return Err(CoordinatorInvariantError::Accounting(
                "cancelled-Up token does not identify a retained invalidated authored Up".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn validate_dropped_expired_up_identity(
        &self,
        identity: AuthoredUpIdentity,
    ) -> Result<(), CoordinatorInvariantError> {
        let Some(mapped) = self
            .up_intent_locations
            .get(identity.generation_id() as usize)
            .copied()
        else {
            return Err(CoordinatorInvariantError::UnknownGeneration {
                generation_id: identity.generation_id(),
                generation_count: self.generation_count,
            });
        };
        if mapped != Some((identity.packet_index(), identity.intent_index())) {
            return Err(CoordinatorInvariantError::Accounting(
                "dropped-Up identity differs from compiler-owned location".into(),
            ));
        }
        let Some(compact) = self.schedule.intents.get(identity.intent_index()) else {
            return Err(CoordinatorInvariantError::Accounting(
                "dropped-Up location is outside the schedule".into(),
            ));
        };
        let Some(packet) = self.schedule.packets.get(identity.packet_index()) else {
            return Err(CoordinatorInvariantError::Accounting(
                "dropped-Up packet is outside the schedule".into(),
            ));
        };
        if compact.generation_id() != NO_GENERATION_ID
            || compact.key_slot() != identity.key_slot()
            || packet.up_mask & Self::bit_for_slot(identity.key_slot()) != 0
        {
            return Err(CoordinatorInvariantError::Accounting(
                "dropped-Up identity is not invalidated from its physical packet".into(),
            ));
        }
        Ok(())
    }

    fn frozen_up_is_invalidated(&self, identity: AuthoredUpIdentity) -> bool {
        self.validate_dropped_expired_up_identity(identity).is_ok()
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
        _dispatch_started: TimelineTicks,
        _dispatch_completed: TimelineTicks,
        excluded_mask: u16,
    ) -> Result<(), CoordinatorError> {
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
        let release_not_before_ticks =
            scheduled_down_ticks.checked_add_duration(self.min_hold_ticks)?;
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
        _dispatch_started_ticks: TimelineTicks,
        _dispatch_completed_ticks: TimelineTicks,
    ) -> Result<(), CoordinatorError> {
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
            let release_not_before_ticks =
                scheduled_down_ticks.checked_add_duration(self.min_hold_ticks)?;
            self.active_by_slot[intent.key_slot as usize] = Some(ActiveGeneration {
                generation_id,
                scan_code: intent.scan_code,
                key_slot: intent.key_slot,
                source_action_index: intent.source_action_index,
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
