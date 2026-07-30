//! Runtime dispatch coordinator managing generation status transitions and release eligibility.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::model::*;

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
}

impl PendingRelease {
    pub fn get_effective_release_us(&self, lead_up: u64) -> u64 {
        let led = self.scheduled_release_us.saturating_sub(lead_up);
        self.release_not_before_us.max(led)
    }
}

#[derive(Debug)]
pub struct RuntimeDispatchCoordinator {
    pub schedule: RuntimeSchedule,
    pub min_hold_us: u64,
    pub cursor: usize,
    pub active_by_scan_code: HashMap<u16, ActiveGeneration>,
    pub status_by_generation: HashMap<GenerationId, GenerationStatus>,
    terminal_counts: HashMap<GenerationStatus, u64>,
    generation_count: u64,
    pub pending_by_generation: HashMap<GenerationId, PendingRelease>,
    pub pending_scan_codes: HashSet<u16>,
}

impl RuntimeDispatchCoordinator {
    pub fn new(schedule: RuntimeSchedule, min_hold_us: u64) -> Self {
        let generation_count = schedule.generation_count;
        Self {
            schedule,
            min_hold_us,
            cursor: 0,
            active_by_scan_code: HashMap::new(),
            status_by_generation: HashMap::new(),
            terminal_counts: HashMap::new(),
            generation_count,
            pending_by_generation: HashMap::new(),
            pending_scan_codes: HashSet::new(),
        }
    }

    fn terminalize(&mut self, generation_id: GenerationId, status: GenerationStatus) {
        self.status_by_generation.remove(&generation_id);
        *self.terminal_counts.entry(status).or_insert(0) += 1;
    }

    fn early_pop_blocked(&self, batch: &RuntimeBatch) -> bool {
        if batch.kind != ActionKind::Down {
            return false;
        }
        if self.active_by_scan_code.is_empty() && self.pending_scan_codes.is_empty() {
            return false;
        }
        batch.intents.iter().any(|intent| {
            self.active_by_scan_code.contains_key(&intent.scan_code)
                || self.pending_scan_codes.contains(&intent.scan_code)
        })
    }

    pub fn next_authored_us(&self, dispatch_lead_us: u64) -> Option<u64> {
        if self.cursor >= self.schedule.batches.len() {
            return None;
        }
        let batch = &self.schedule.batches[self.cursor];
        let lead = dispatch_lead_us;
        if lead > 0 && self.early_pop_blocked(batch) {
            return Some(batch.scheduled_us);
        }
        Some(batch.scheduled_us.saturating_sub(lead))
    }

    pub fn next_pending_release_us(&self, lead_up: u64) -> Option<u64> {
        if self.pending_by_generation.is_empty() {
            return None;
        }
        self.pending_by_generation
            .values()
            .map(|pending| pending.get_effective_release_us(lead_up))
            .min()
    }

    pub fn next_deadline_us(&self, dispatch_lead_us: u64, lead_up: u64) -> Option<u64> {
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
        self.cursor >= self.schedule.batches.len() && self.pending_by_generation.is_empty()
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

    pub fn pop_due_pending(&mut self, now_us: u64, lead_up: u64) -> Vec<PendingRelease> {
        if self.pending_by_generation.is_empty() {
            return Vec::new();
        }

        if self.pending_by_generation.len() == 1 {
            let gen_id = *self.pending_by_generation.keys().next().unwrap();
            let pending = self.pending_by_generation.get(&gen_id).unwrap();
            if pending.get_effective_release_us(lead_up) > now_us {
                return Vec::new();
            }
            let pending = self.pending_by_generation.remove(&gen_id).unwrap();
            self.pending_scan_codes.remove(&pending.scan_code);
            return vec![pending];
        }

        let mut due: Vec<PendingRelease> = self
            .pending_by_generation
            .values()
            .filter(|p| p.get_effective_release_us(lead_up) <= now_us)
            .cloned()
            .collect();

        if due.is_empty() {
            return Vec::new();
        }

        due.sort_by_key(|p| {
            (
                p.get_effective_release_us(lead_up),
                p.source_action_index,
                p.scan_code,
            )
        });

        for p in &due {
            self.pending_by_generation.remove(&p.generation_id);
            self.pending_scan_codes.remove(&p.scan_code);
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
        let batch = &self.schedule.batches[self.cursor];
        let lead = dispatch_lead_us;
        if batch.scheduled_us > now_us + lead {
            return None;
        }
        if batch.scheduled_us > now_us && self.early_pop_blocked(batch) {
            return None;
        }
        let popped = self.schedule.batches[self.cursor].clone();
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
                self.active_by_scan_code.insert(
                    intent.scan_code,
                    ActiveGeneration {
                        generation_id,
                        scan_code: intent.scan_code,
                        key_slot: intent.key_slot,
                        source_action_index: intent.source_action_index,
                        scheduled_down_us: intent.scheduled_us,
                        down_dispatch_started_us: dispatch_started_us,
                        down_dispatch_completed_us: dispatch_completed_us,
                        release_not_before_us,
                    },
                );
                self.status_by_generation
                    .insert(generation_id, GenerationStatus::Active);
            }
            return;
        }

        let sent_set: HashSet<u16> = sent_scan_codes.iter().copied().collect();
        for intent in intents {
            let Some(generation_id) = intent.generation_id else {
                continue;
            };
            if !sent_set.contains(&intent.scan_code) {
                self.terminalize(generation_id, GenerationStatus::DroppedBackend);
                continue;
            }
            self.active_by_scan_code.insert(
                intent.scan_code,
                ActiveGeneration {
                    generation_id,
                    scan_code: intent.scan_code,
                    key_slot: intent.key_slot,
                    source_action_index: intent.source_action_index,
                    scheduled_down_us: intent.scheduled_us,
                    down_dispatch_started_us: dispatch_started_us,
                    down_dispatch_completed_us: dispatch_completed_us,
                    release_not_before_us,
                },
            );
            self.status_by_generation
                .insert(generation_id, GenerationStatus::Active);
        }
    }

    pub fn split_down_intents(
        &mut self,
        intents: &[RuntimeKeyIntent],
    ) -> (Vec<RuntimeKeyIntent>, Vec<RuntimeKeyIntent>) {
        if self.active_by_scan_code.is_empty() {
            return (intents.to_vec(), Vec::new());
        }
        let mut playable = Vec::new();
        let mut conflicts = Vec::new();

        for intent in intents {
            if self.active_by_scan_code.contains_key(&intent.scan_code) {
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
    ) -> (Vec<PendingRelease>, Vec<RuntimeKeyIntent>) {
        if intents.len() == 1 {
            let intent = &intents[0];
            let Some(generation_id) = intent.generation_id else {
                return (Vec::new(), vec![intent.clone()]);
            };
            let active = self.active_by_scan_code.get(&intent.scan_code);
            let Some(active) = active else {
                return (Vec::new(), vec![intent.clone()]);
            };
            if active.generation_id != generation_id {
                return (Vec::new(), vec![intent.clone()]);
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
            };

            self.pending_by_generation
                .insert(generation_id, pending.clone());
            self.pending_scan_codes.insert(intent.scan_code);
            self.status_by_generation
                .insert(generation_id, GenerationStatus::ReleasePending);
            return (vec![pending], Vec::new());
        }

        let mut requested = Vec::new();
        let mut suppressed = Vec::new();

        for intent in intents {
            let Some(generation_id) = intent.generation_id else {
                suppressed.push(intent.clone());
                continue;
            };
            let active = self.active_by_scan_code.get(&intent.scan_code);
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
            };

            self.pending_by_generation
                .insert(generation_id, pending.clone());
            self.pending_scan_codes.insert(intent.scan_code);
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
        let sent_set: HashSet<u16> = sent_scan_codes.iter().copied().collect();
        let skipped_set: HashSet<u16> = skipped_scan_codes.iter().copied().collect();

        for pending in releases {
            let in_sent = sent_set.contains(&pending.scan_code);
            let in_skipped = skipped_set.contains(&pending.scan_code);
            if !in_sent && !in_skipped {
                continue;
            }
            if matches!(self.active_by_scan_code.get(&pending.scan_code), Some(active) if active.generation_id == pending.generation_id)
            {
                self.active_by_scan_code.remove(&pending.scan_code);
            }
            let status = if in_sent {
                GenerationStatus::Released
            } else {
                GenerationStatus::DroppedBackend
            };
            self.terminalize(pending.generation_id, status);
        }
    }

    pub fn cancel_all(&mut self) -> Vec<GenerationId> {
        let mut cancelled_ids: HashSet<GenerationId> = self
            .active_by_scan_code
            .values()
            .map(|a| a.generation_id)
            .collect();
        for &pending_id in self.pending_by_generation.keys() {
            cancelled_ids.insert(pending_id);
        }

        let mut sorted_cancelled: Vec<GenerationId> = cancelled_ids.into_iter().collect();
        sorted_cancelled.sort_unstable();

        for &gen_id in &sorted_cancelled {
            self.terminalize(gen_id, GenerationStatus::Cancelled);
        }

        self.active_by_scan_code.clear();
        self.pending_by_generation.clear();
        self.pending_scan_codes.clear();

        sorted_cancelled
    }
}
