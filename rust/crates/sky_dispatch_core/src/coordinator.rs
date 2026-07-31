//! Runtime dispatch coordinator managing generation status transitions and release eligibility.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::collections::HashMap;

use crate::model::*;

pub const MAX_RELEASE_RETRIES: u8 = 8;
const RELEASE_RETRY_BACKOFF_US: [u64; 4] = [2_000, 5_000, 10_000, 20_000];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationStatus {
    Scheduled,
    Active,
    ReleasePending,
    Released,
    DroppedConflict,
    DroppedBackend,
    DroppedExpired,
    Cancelled,
}

impl GenerationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Active => "active",
            Self::ReleasePending => "release_pending",
            Self::Released => "released",
            Self::DroppedConflict => "dropped_conflict",
            Self::DroppedBackend => "dropped_backend",
            Self::DroppedExpired => "dropped_expired",
            Self::Cancelled => "cancelled",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GenerationStatus, RuntimeDispatchCoordinator};
    use crate::compile::compile_runtime_intents;
    use crate::model::{ActionKind, KeyActionInput};

    #[test]
    fn final_focus_drop_is_terminal_and_cannot_replay_authored_batch() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15],
                    reason: "down".to_string(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15],
                    reason: "up".to_string(),
                },
            ],
            &[0x15],
        )
        .expect("valid schedule");
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0);
        let (batch, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("down batch is due");

        coordinator.drop_expired_downs(&batch.intents);

        assert!(coordinator.pop_next_due_authored(0, 0).is_none());
        assert_eq!(
            coordinator
                .generation_status_counts()
                .get(GenerationStatus::DroppedExpired.as_str()),
            Some(&1)
        );
    }

    #[test]
    fn failed_release_is_requeued_and_unblocks_later_same_key_down() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15],
                    reason: "down-1".to_string(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15],
                    reason: "up-1".to_string(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Down,
                    scheduled_us: 4_000,
                    scan_codes: vec![0x15],
                    reason: "down-2".to_string(),
                },
            ],
            &[0x15],
        )
        .expect("valid schedule");
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0);
        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("first down is due");
        coordinator.activate_sent_downs(&down.intents, &[0x15], 0, 10);

        let (up, _) = coordinator
            .pop_next_due_authored(1_000, 0)
            .expect("up is due");
        let (_, suppressed) = coordinator.request_releases(&up.intents);
        assert!(suppressed.is_empty());

        let due = coordinator.pop_due_pending(1_000, 0);
        assert_eq!(due.len(), 1);
        assert!(!coordinator.requeue_failed_releases(&due, &[], &[], 1_000, 1_000, Some(5),));
        assert_eq!(coordinator.next_pending_release_us(0), Some(3_000));
        assert!(!coordinator.is_finished());

        let retry = coordinator.pop_due_pending(3_000, 0);
        assert_eq!(retry.len(), 1);
        coordinator.complete_releases(&retry, &[0x15], &[]);
        assert_eq!(coordinator.finish_release_recovery(3_000), Some(2_000));
        assert_eq!(coordinator.schedule.batches[2].scheduled_us, 4_000);

        let (next_down, _) = coordinator
            .pop_next_due_authored(6_000, 0)
            .expect("same-key down remains schedulable after recovery");
        let (playable, conflicts) = coordinator.split_down_intents(&next_down.intents);
        assert_eq!(playable.len(), 1);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn same_key_down_waits_for_recovery_and_timeline_does_not_catch_up() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15],
                    reason: "down-1".to_string(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15],
                    reason: "up-1".to_string(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Down,
                    scheduled_us: 2_000,
                    scan_codes: vec![0x15],
                    reason: "down-2".to_string(),
                },
            ],
            &[0x15],
        )
        .expect("valid schedule");
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0);
        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("first down is due");
        coordinator.activate_sent_downs(&down.intents, &[0x15], 0, 10);
        let (up, _) = coordinator
            .pop_next_due_authored(1_000, 0)
            .expect("up is due");
        let _ = coordinator.request_releases(&up.intents);
        let due = coordinator.pop_due_pending(1_000, 0);
        assert!(!coordinator.requeue_failed_releases(&due, &[], &[], 1_000, 1_000, Some(5),));

        assert!(coordinator.pop_next_due_authored(2_000, 0).is_none());
        assert_eq!(coordinator.next_deadline_us(0, 0), Some(3_000));

        let retry = coordinator.pop_due_pending(3_000, 0);
        coordinator.complete_releases(&retry, &[0x15], &[]);
        assert_eq!(coordinator.finish_release_recovery(3_000), Some(2_000));
        assert_eq!(coordinator.schedule.batches[2].scheduled_us, 2_000);
        assert!(coordinator.pop_next_due_authored(3_000, 0).is_none());
        let (next_down, _) = coordinator
            .pop_next_due_authored(4_000, 0)
            .expect("timeline-shifted same-key down is due");
        assert_eq!(next_down.scheduled_us, 4_000);
        let (playable, conflicts) = coordinator.split_down_intents(&next_down.intents);
        assert_eq!(playable.len(), 1);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn blocked_authored_deadline_includes_recovery_offset_when_lead_is_enabled() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15, 0x16],
                    reason: "down-1".to_string(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15],
                    reason: "up-1".to_string(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Down,
                    scheduled_us: 2_000,
                    scan_codes: vec![0x15, 0x16],
                    reason: "down-2".to_string(),
                },
            ],
            &[0x15, 0x16],
        )
        .expect("valid schedule");
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0);

        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("first down is due");
        coordinator.activate_sent_downs(&down.intents, &[0x15, 0x16], 0, 10);
        let (up, _) = coordinator
            .pop_next_due_authored(1_000, 0)
            .expect("release is due");
        let _ = coordinator.request_releases(&up.intents);
        let due = coordinator.pop_due_pending(1_000, 0);
        assert!(!coordinator.requeue_failed_releases(&due, &[], &[], 1_000, 1_500, Some(5),));
        assert_eq!(coordinator.next_pending_release_us(0), Some(3_500));

        let retry = coordinator.pop_due_pending(3_500, 0);
        coordinator.complete_releases(&retry, &[0x15], &[]);
        assert_eq!(coordinator.finish_release_recovery(3_500), Some(2_500));

        // Key 0x16 remains active, so the next chord cannot be early-popped.
        // Its blocked deadline must still use the effective timeline offset.
        assert_eq!(coordinator.next_authored_us(100), Some(4_500));
    }
}

pub const ALL_GENERATION_STATUSES: [GenerationStatus; 8] = [
    GenerationStatus::Scheduled,
    GenerationStatus::Active,
    GenerationStatus::ReleasePending,
    GenerationStatus::Released,
    GenerationStatus::DroppedConflict,
    GenerationStatus::DroppedBackend,
    GenerationStatus::DroppedExpired,
    GenerationStatus::Cancelled,
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveGeneration {
    pub generation_id: GenerationId,
    pub scan_code: u16,
    pub key_slot: KeySlot,
    pub source_action_index: u32,
    pub scheduled_down_us: u64,
    pub down_dispatch_started_us: u64,
    pub down_dispatch_completed_us: u64,
    pub release_not_before_us: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRelease {
    pub generation_id: GenerationId,
    pub scan_code: u16,
    pub key_slot: KeySlot,
    pub source_action_index: u32,
    pub scheduled_release_us: u64,
    pub down_dispatch_started_us: u64,
    pub release_not_before_us: u64,
    pub reason_id: ReasonId,
    pub retry_count: u8,
    pub next_retry_us: u64,
    pub first_failure_us: Option<u64>,
    pub last_win32_error: Option<u32>,
}

impl PendingRelease {
    pub fn get_effective_release_us(&self, lead_up: u64) -> u64 {
        let led = self.scheduled_release_us.saturating_sub(lead_up);
        self.release_not_before_us.max(led).max(self.next_retry_us)
    }
}

#[derive(Debug)]
pub struct RuntimeDispatchCoordinator {
    pub schedule: RuntimeSchedule,
    pub min_hold_us: u64,
    pub cursor: usize,
    active_by_slot: [Option<ActiveGeneration>; MAX_KEYS],
    active_mask: u16,
    pending_by_slot: [Option<PendingRelease>; MAX_KEYS],
    pending_mask: u16,
    pub status_by_generation: HashMap<GenerationId, GenerationStatus>,
    terminal_counts: HashMap<GenerationStatus, u64>,
    generation_count: u64,
    recovery_offset_us: u64,
    release_recovery_started_us: Option<u64>,
}

impl RuntimeDispatchCoordinator {
    pub fn new(schedule: RuntimeSchedule, min_hold_us: u64) -> Self {
        let generation_count = schedule.generation_count;
        Self {
            schedule,
            min_hold_us,
            cursor: 0,
            active_by_slot: std::array::from_fn(|_| None),
            active_mask: 0,
            pending_by_slot: std::array::from_fn(|_| None),
            pending_mask: 0,
            status_by_generation: HashMap::with_capacity(MAX_KEYS),
            terminal_counts: HashMap::with_capacity(ALL_GENERATION_STATUSES.len()),
            generation_count,
            recovery_offset_us: 0,
            release_recovery_started_us: None,
        }
    }

    fn bit_for_slot(slot: KeySlot) -> u16 {
        1u16 << slot
    }

    fn active_for_slot(&self, slot: KeySlot) -> Option<&ActiveGeneration> {
        self.active_by_slot
            .get(slot as usize)
            .and_then(Option::as_ref)
    }

    pub fn recovery_offset_us(&self) -> u64 {
        self.recovery_offset_us
    }

    pub fn effective_total_us(&self) -> u64 {
        self.schedule.batches.last().map_or(0, |batch| {
            batch.scheduled_us.saturating_add(self.recovery_offset_us)
        })
    }

    fn terminalize(&mut self, generation_id: GenerationId, status: GenerationStatus) {
        self.status_by_generation.remove(&generation_id);
        *self.terminal_counts.entry(status).or_insert(0) += 1;
    }

    fn early_pop_blocked(&self, batch: &CompiledBatch) -> bool {
        if batch.kind != ActionKind::Down {
            return false;
        }
        if self.active_mask == 0 && self.pending_mask == 0 {
            return false;
        }
        self.schedule.intent_slice(batch).iter().any(|intent| {
            let bit = Self::bit_for_slot(intent.key_slot);
            self.active_mask & bit != 0 || self.pending_mask & bit != 0
        })
    }

    pub fn next_authored_us(&self, dispatch_lead_us: u64) -> Option<u64> {
        if self.cursor >= self.schedule.batches.len() {
            return None;
        }
        let batch = &self.schedule.batches[self.cursor];
        let lead = dispatch_lead_us;
        if lead > 0 && self.early_pop_blocked(batch) {
            return Some(batch.scheduled_us.saturating_add(self.recovery_offset_us));
        }
        Some(
            batch
                .scheduled_us
                .saturating_add(self.recovery_offset_us)
                .saturating_sub(lead),
        )
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

    pub fn next_pending_release_us(&self, lead_up: u64) -> Option<u64> {
        if self.pending_mask == 0 {
            return None;
        }
        self.pending_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .map(|pending| pending.get_effective_release_us(lead_up))
            .min()
    }

    pub fn next_deadline_us(&self, dispatch_lead_us: u64, lead_up: u64) -> Option<u64> {
        if self.release_recovery_active() {
            return self.next_pending_release_us(lead_up);
        }
        let authored = self.next_authored_us(dispatch_lead_us);
        let pending = self.next_pending_release_us(lead_up);
        match (authored, pending) {
            (Some(a), Some(p)) => Some(a.min(p)),
            (Some(a), None) => Some(a),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        }
    }

    pub fn is_finished(&self) -> bool {
        // An authored down may legitimately have no matching up in the input
        // timeline.  The worker's terminal cleanup owns that case, so do not
        // wait forever on an active generation that has no pending release.
        // Failed pending releases are kept alive by `requeue_failed_releases`
        // until they succeed or recovery aborts the session.
        self.cursor >= self.schedule.batches.len() && self.pending_mask == 0
    }

    pub fn generation_status_counts(&self) -> HashMap<String, u64> {
        let mut counts: HashMap<GenerationStatus, u64> = self.terminal_counts.clone();
        let mut nonterminal: u64 = 0;
        for &status in self.status_by_generation.values() {
            *counts.entry(status).or_insert(0) += 1;
            nonterminal += 1;
        }
        let terminal_total: u64 = self.terminal_counts.values().sum();
        let implicit_scheduled = self
            .generation_count
            .saturating_sub(terminal_total + nonterminal);
        if implicit_scheduled > 0 {
            *counts.entry(GenerationStatus::Scheduled).or_insert(0) += implicit_scheduled;
        }
        let mut result = HashMap::new();
        for status in &ALL_GENERATION_STATUSES {
            result.insert(
                status.as_str().to_string(),
                *counts.get(status).unwrap_or(&0),
            );
        }
        result
    }

    pub fn pop_due_pending(
        &mut self,
        now_us: u64,
        lead_up: u64,
    ) -> SmallVec<[PendingRelease; MAX_KEYS]> {
        if self.pending_mask == 0 {
            return SmallVec::new();
        }

        let mut due: SmallVec<[PendingRelease; MAX_KEYS]> = self
            .pending_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .filter(|pending| pending.get_effective_release_us(lead_up) <= now_us)
            .cloned()
            .collect();

        if due.is_empty() {
            return SmallVec::new();
        }

        due.sort_by_key(|p| {
            (
                p.get_effective_release_us(lead_up),
                p.source_action_index,
                p.scan_code,
            )
        });

        for p in &due {
            let slot = p.key_slot as usize;
            self.pending_by_slot[slot] = None;
            self.pending_mask &= !Self::bit_for_slot(p.key_slot);
        }

        due
    }

    pub fn pop_next_due_authored(
        &mut self,
        now_us: u64,
        dispatch_lead_us: u64,
    ) -> Option<(RuntimeBatch, u64)> {
        if self.cursor >= self.schedule.batches.len() {
            return None;
        }
        // A failed release recovery is a contiguous pause interval for the
        // authored timeline. Do not pop overdue batches and turn recovery
        // into a same-key conflict or a catch-up burst.
        if self.release_recovery_active() {
            return None;
        }
        let batch = &self.schedule.batches[self.cursor];
        let lead = dispatch_lead_us;
        let effective_scheduled_us = batch.scheduled_us.saturating_add(self.recovery_offset_us);
        if effective_scheduled_us > now_us.saturating_add(lead) {
            return None;
        }
        if effective_scheduled_us > now_us && self.early_pop_blocked(batch) {
            return None;
        }
        let popped = self
            .schedule
            .materialize_batch(self.cursor, self.recovery_offset_us);
        self.cursor += 1;
        Some((popped, lead))
    }

    pub fn activate_sent_downs(
        &mut self,
        intents: &[RuntimeKeyIntent],
        sent_scan_codes: &[u16],
        dispatch_started_us: u64,
        dispatch_completed_us: u64,
    ) {
        let release_not_before_us = dispatch_completed_us + self.min_hold_us;

        if sent_scan_codes.len() == 1 {
            let only_sent = sent_scan_codes[0];
            for intent in intents {
                let Some(generation_id) = intent.generation_id else {
                    continue;
                };
                if intent.scan_code != only_sent {
                    self.terminalize(generation_id, GenerationStatus::DroppedBackend);
                    continue;
                }
                self.active_by_slot[intent.key_slot as usize] = Some(ActiveGeneration {
                    generation_id,
                    scan_code: intent.scan_code,
                    key_slot: intent.key_slot,
                    source_action_index: intent.source_action_index,
                    scheduled_down_us: intent.scheduled_us,
                    down_dispatch_started_us: dispatch_started_us,
                    down_dispatch_completed_us: dispatch_completed_us,
                    release_not_before_us,
                });
                self.active_mask |= Self::bit_for_slot(intent.key_slot);
                self.status_by_generation
                    .insert(generation_id, GenerationStatus::Active);
            }
            return;
        }

        for intent in intents {
            let Some(generation_id) = intent.generation_id else {
                continue;
            };
            if !sent_scan_codes.contains(&intent.scan_code) {
                self.terminalize(generation_id, GenerationStatus::DroppedBackend);
                continue;
            }
            self.active_by_slot[intent.key_slot as usize] = Some(ActiveGeneration {
                generation_id,
                scan_code: intent.scan_code,
                key_slot: intent.key_slot,
                source_action_index: intent.source_action_index,
                scheduled_down_us: intent.scheduled_us,
                down_dispatch_started_us: dispatch_started_us,
                down_dispatch_completed_us: dispatch_completed_us,
                release_not_before_us,
            });
            self.active_mask |= Self::bit_for_slot(intent.key_slot);
            self.status_by_generation
                .insert(generation_id, GenerationStatus::Active);
        }
    }

    pub fn split_down_intents(
        &mut self,
        intents: &[RuntimeKeyIntent],
    ) -> (
        SmallVec<[RuntimeKeyIntent; MAX_KEYS]>,
        SmallVec<[RuntimeKeyIntent; MAX_KEYS]>,
    ) {
        if self.active_mask == 0 {
            return (intents.iter().cloned().collect(), SmallVec::new());
        }
        let mut playable = SmallVec::new();
        let mut conflicts = SmallVec::new();

        for intent in intents {
            if self.active_mask & Self::bit_for_slot(intent.key_slot) != 0 {
                conflicts.push(intent.clone());
                if let Some(gen_id) = intent.generation_id {
                    self.terminalize(gen_id, GenerationStatus::DroppedConflict);
                }
            } else {
                playable.push(intent.clone());
            }
        }
        (playable, conflicts)
    }

    pub fn drop_expired_downs(&mut self, intents: &[RuntimeKeyIntent]) {
        for intent in intents {
            if let Some(gen_id) = intent.generation_id {
                self.terminalize(gen_id, GenerationStatus::DroppedExpired);
            }
        }
    }

    pub fn request_releases(
        &mut self,
        intents: &[RuntimeKeyIntent],
    ) -> (
        SmallVec<[PendingRelease; MAX_KEYS]>,
        SmallVec<[RuntimeKeyIntent; MAX_KEYS]>,
    ) {
        if intents.len() == 1 {
            let intent = &intents[0];
            let Some(generation_id) = intent.generation_id else {
                return (SmallVec::new(), std::iter::once(intent.clone()).collect());
            };
            let active = self.active_for_slot(intent.key_slot);
            let Some(active) = active else {
                return (SmallVec::new(), std::iter::once(intent.clone()).collect());
            };
            if active.generation_id != generation_id {
                return (SmallVec::new(), std::iter::once(intent.clone()).collect());
            }

            let pending = PendingRelease {
                generation_id,
                scan_code: intent.scan_code,
                key_slot: intent.key_slot,
                source_action_index: intent.source_action_index,
                scheduled_release_us: intent.scheduled_us,
                down_dispatch_started_us: active.down_dispatch_started_us,
                release_not_before_us: active.release_not_before_us,
                reason_id: intent.reason_id,
                retry_count: 0,
                next_retry_us: 0,
                first_failure_us: None,
                last_win32_error: None,
            };

            self.pending_by_slot[intent.key_slot as usize] = Some(pending.clone());
            self.pending_mask |= Self::bit_for_slot(intent.key_slot);
            self.status_by_generation
                .insert(generation_id, GenerationStatus::ReleasePending);
            return (std::iter::once(pending).collect(), SmallVec::new());
        }

        let mut requested = SmallVec::new();
        let mut suppressed = SmallVec::new();

        for intent in intents {
            let Some(generation_id) = intent.generation_id else {
                suppressed.push(intent.clone());
                continue;
            };
            let active = self.active_for_slot(intent.key_slot);
            let Some(active) = active else {
                suppressed.push(intent.clone());
                continue;
            };
            if active.generation_id != generation_id {
                suppressed.push(intent.clone());
                continue;
            }

            let pending = PendingRelease {
                generation_id,
                scan_code: intent.scan_code,
                key_slot: intent.key_slot,
                source_action_index: intent.source_action_index,
                scheduled_release_us: intent.scheduled_us,
                down_dispatch_started_us: active.down_dispatch_started_us,
                release_not_before_us: active.release_not_before_us,
                reason_id: intent.reason_id,
                retry_count: 0,
                next_retry_us: 0,
                first_failure_us: None,
                last_win32_error: None,
            };

            self.pending_by_slot[intent.key_slot as usize] = Some(pending.clone());
            self.pending_mask |= Self::bit_for_slot(intent.key_slot);
            self.status_by_generation
                .insert(generation_id, GenerationStatus::ReleasePending);
            requested.push(pending);
        }

        (requested, suppressed)
    }

    pub fn complete_releases(
        &mut self,
        releases: &[PendingRelease],
        sent_scan_codes: &[u16],
        skipped_scan_codes: &[u16],
    ) {
        for pending in releases {
            let in_sent = sent_scan_codes.contains(&pending.scan_code);
            let in_skipped = skipped_scan_codes.contains(&pending.scan_code);
            if !in_sent && !in_skipped {
                continue;
            }
            if matches!(self.active_for_slot(pending.key_slot), Some(active) if active.generation_id == pending.generation_id)
            {
                self.active_by_slot[pending.key_slot as usize] = None;
                self.active_mask &= !Self::bit_for_slot(pending.key_slot);
            }
            let status = if in_sent {
                GenerationStatus::Released
            } else {
                GenerationStatus::DroppedBackend
            };
            self.terminalize(pending.generation_id, status);
        }
    }

    pub fn release_recovery_active(&self) -> bool {
        self.release_recovery_started_us.is_some()
    }

    /// End a recovery pause after the pending release set is empty.
    ///
    /// The authored schedule is immutable.  A single offset moves the
    /// effective playback timeline, so recovery remains O(1) regardless of
    /// how many batches are still queued.
    pub fn finish_release_recovery(&mut self, completed_us: u64) -> Option<u64> {
        if self.pending_mask != 0 {
            return None;
        }
        let started_us = self.release_recovery_started_us.take()?;
        let pause_us = completed_us.saturating_sub(started_us);
        self.recovery_offset_us = self.recovery_offset_us.saturating_add(pause_us);
        Some(pause_us)
    }

    /// Requeue release work that did not reach the operating-system input
    /// stream. The active generation remains owned by the coordinator while
    /// bounded retries are pending; callers must stop playback and perform
    /// full-instrument recovery when this returns `true`.
    ///
    /// `recovery_started_us` is sampled immediately before the failed backend
    /// call. `retry_base_us` is sampled from backend completion and is used
    /// only to schedule the next retry after the call has returned.
    pub fn requeue_failed_releases(
        &mut self,
        releases: &[PendingRelease],
        sent_scan_codes: &[u16],
        skipped_scan_codes: &[u16],
        recovery_started_us: u64,
        retry_base_us: u64,
        last_win32_error: Option<u32>,
    ) -> bool {
        let mut recovery_required = false;
        for pending in releases {
            if sent_scan_codes.contains(&pending.scan_code)
                || skipped_scan_codes.contains(&pending.scan_code)
            {
                continue;
            }
            if !matches!(
                self.active_for_slot(pending.key_slot),
                Some(active) if active.generation_id == pending.generation_id
            ) {
                continue;
            }

            let retry_count = pending.retry_count.saturating_add(1);
            if retry_count > MAX_RELEASE_RETRIES {
                recovery_required = true;
                continue;
            }
            let delay_index =
                usize::from(retry_count.saturating_sub(1)).min(RELEASE_RETRY_BACKOFF_US.len() - 1);
            let mut retry = pending.clone();
            self.release_recovery_started_us
                .get_or_insert(recovery_started_us);
            retry.retry_count = retry_count;
            retry.next_retry_us =
                retry_base_us.saturating_add(RELEASE_RETRY_BACKOFF_US[delay_index]);
            retry.first_failure_us = Some(pending.first_failure_us.unwrap_or(recovery_started_us));
            retry.last_win32_error = last_win32_error.or(pending.last_win32_error);
            let retry_slot = retry.key_slot;
            self.pending_mask |= Self::bit_for_slot(retry_slot);
            self.pending_by_slot[retry_slot as usize] = Some(retry);
        }
        recovery_required
    }

    pub fn cancel_all(&mut self) -> Vec<GenerationId> {
        let mut cancelled_ids: SmallVec<[GenerationId; MAX_KEYS * 2]> = self
            .active_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .map(|active| active.generation_id)
            .collect();
        for pending_id in self
            .pending_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .map(|pending| pending.generation_id)
        {
            if !cancelled_ids.contains(&pending_id) {
                cancelled_ids.push(pending_id);
            }
        }

        let mut sorted_cancelled: Vec<GenerationId> = cancelled_ids.into_vec();
        sorted_cancelled.sort_unstable();

        for &gen_id in &sorted_cancelled {
            self.terminalize(gen_id, GenerationStatus::Cancelled);
        }

        self.active_by_slot.fill(None);
        self.pending_by_slot.fill(None);
        self.active_mask = 0;
        self.pending_mask = 0;
        self.release_recovery_started_us = None;

        sorted_cancelled
    }
}
