//! Bounded 15-slot musical dispatch executor.
//!
//! Manages exact musical generation ownership, independent release eligibility
//! per slot, atomic Down-chord transport gating, Up-before-Down combining,
//! and physical H/G timing floors.

use smallvec::SmallVec;

use crate::coordinator::{GenerationAccounting, GenerationCounters};
use crate::model::{GenerationId, KeyRegistry, KeySlot, MAX_KEYS, RuntimeSchedule};
use crate::time::{DurationTicks, TimelineTicks};

/// Per-slot musical ownership state for the 15 instrument keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MusicalSlot {
    /// Active musical generation in this slot, if any.
    pub active_generation: Option<GenerationId>,
    /// Exact authored Up offset for this generation.
    pub authored_up_offset_ticks: TimelineTicks,
    /// Physical hold floor: successful Down completion + H.
    pub release_not_before_ticks: TimelineTicks,
    /// Physical repress floor: previous Up completion + T.
    pub next_down_not_before_ticks: TimelineTicks,
    /// Exact authored source action index of the paired Up.
    pub paired_up_source_action_index: u32,
}

impl Default for MusicalSlot {
    fn default() -> Self {
        Self {
            active_generation: None,
            authored_up_offset_ticks: TimelineTicks::ZERO,
            release_not_before_ticks: TimelineTicks::ZERO,
            next_down_not_before_ticks: TimelineTicks::ZERO,
            paired_up_source_action_index: 0,
        }
    }
}

/// A Down intent with timeline tick offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownIntentTicks {
    pub generation_id: GenerationId,
    pub key_slot: KeySlot,
    pub scan_code: u16,
    pub source_action_index: u32,
    pub reason_id: crate::model::ReasonId,
    pub paired_up_offset_ticks: TimelineTicks,
    pub paired_up_offset_us: u64,
    pub paired_up_source_action_index: u32,
}

/// An immutable Down chord with timeline tick offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownChordTicks {
    pub chord_index: usize,
    pub scheduled_ticks: TimelineTicks,
    pub scheduled_us: u64,
    pub down_mask: u16,
    pub source_action_index: u32,
    pub reason_id: crate::model::ReasonId,
    pub intents: SmallVec<[DownIntentTicks; MAX_KEYS]>,
}

/// The next action decided by the 15-slot executor at time `now_ticks`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchAction<'a> {
    /// Ready physical packet to stage and send.
    Send {
        up_mask: u16,
        down_chord: Option<&'a DownChordTicks>,
    },
    /// No physical work ready; wait until the specified deadline.
    Wait { deadline_ticks: TimelineTicks },
    /// All Down chords sent, all slots released, and timeline end offset reached.
    Finished,
}

/// Bounded 15-slot musical executor.
#[derive(Debug, Clone)]
pub struct DispatchExecutor {
    down_chords: Vec<DownChordTicks>,
    schedule_end_offset_ticks: TimelineTicks,
    schedule_end_offset_us: u64,
    key_registry: KeyRegistry,
    chord_cursor: usize,
    slots: [MusicalSlot; MAX_KEYS],
    resume_barrier_ticks: TimelineTicks,
    min_hold_ticks: DurationTicks,
    frame_ticks: DurationTicks,
    counters: GenerationCounters,
    total_generations: u64,
    activated_generations: u64,
}

impl DispatchExecutor {
    /// Construct a new 15-slot executor from a validated `RuntimeSchedule`.
    pub fn new<F, E>(
        schedule: &RuntimeSchedule,
        min_hold_ticks: DurationTicks,
        frame_ticks: DurationTicks,
        mut us_to_ticks: F,
    ) -> Result<Self, String>
    where
        F: FnMut(u64) -> Result<TimelineTicks, E>,
        E: std::fmt::Display,
    {
        let mut down_chords = Vec::with_capacity(schedule.down_chords.len());
        for chord in &schedule.down_chords {
            let scheduled_ticks = us_to_ticks(chord.scheduled_us).map_err(|e| {
                format!(
                    "scheduled timestamp conversion failed for chord at {}us: {e}",
                    chord.scheduled_us
                )
            })?;
            let mut intents = SmallVec::new();
            for intent in &chord.intents {
                let paired_up_offset_ticks =
                    us_to_ticks(intent.paired_up_offset_us).map_err(|e| {
                        format!(
                            "paired Up offset timestamp conversion failed at {}us: {e}",
                            intent.paired_up_offset_us
                        )
                    })?;
                intents.push(DownIntentTicks {
                    generation_id: intent.generation_id,
                    key_slot: intent.key_slot,
                    scan_code: intent.scan_code,
                    source_action_index: intent.source_action_index,
                    reason_id: intent.reason_id,
                    paired_up_offset_ticks,
                    paired_up_offset_us: intent.paired_up_offset_us,
                    paired_up_source_action_index: intent.paired_up_source_action_index,
                });
            }
            down_chords.push(DownChordTicks {
                chord_index: chord.chord_index,
                scheduled_ticks,
                scheduled_us: chord.scheduled_us,
                down_mask: chord.down_mask,
                source_action_index: chord.source_action_index,
                reason_id: chord.reason_id,
                intents,
            });
        }

        let schedule_end_offset_ticks =
            us_to_ticks(schedule.schedule_end_offset_us).map_err(|e| {
                format!(
                    "schedule end offset timestamp conversion failed at {}us: {e}",
                    schedule.schedule_end_offset_us
                )
            })?;

        Ok(Self {
            down_chords,
            schedule_end_offset_ticks,
            schedule_end_offset_us: schedule.schedule_end_offset_us,
            key_registry: schedule.key_registry.clone(),
            chord_cursor: 0,
            slots: [MusicalSlot::default(); MAX_KEYS],
            resume_barrier_ticks: TimelineTicks::ZERO,
            min_hold_ticks,
            frame_ticks,
            counters: GenerationCounters::default(),
            total_generations: schedule.generation_count,
            activated_generations: 0,
        })
    }

    #[inline]
    pub fn chord_cursor(&self) -> usize {
        self.chord_cursor
    }

    #[inline]
    pub fn down_chord_count(&self) -> usize {
        self.down_chords.len()
    }

    #[inline]
    pub fn total_chords(&self) -> usize {
        self.down_chords.len()
    }

    #[inline]
    pub fn remaining_chords(&self) -> usize {
        self.down_chords.len().saturating_sub(self.chord_cursor)
    }

    #[inline]
    pub fn slots(&self) -> &[MusicalSlot; MAX_KEYS] {
        &self.slots
    }

    #[inline]
    pub fn key_registry(&self) -> &KeyRegistry {
        &self.key_registry
    }

    #[inline]
    pub fn active_mask(&self) -> u16 {
        let mut mask = 0u16;
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.active_generation.is_some() {
                mask |= 1 << i;
            }
        }
        mask
    }

    /// Whether there is any pending or active physical work (chords remaining or active key slots held).
    /// When false, any subsequent wait is purely the metadata-only tail.
    #[inline]
    pub fn has_physical_work(&self) -> bool {
        self.remaining_chords() > 0 || self.active_mask() != 0
    }

    #[inline]
    pub fn schedule_end_offset_ticks(&self) -> TimelineTicks {
        self.schedule_end_offset_ticks
    }

    #[inline]
    pub fn schedule_end_offset_us(&self) -> u64 {
        self.schedule_end_offset_us
    }

    #[inline]
    pub fn current_chord(&self) -> Option<&DownChordTicks> {
        self.down_chords.get(self.chord_cursor)
    }

    #[inline]
    pub fn resume_barrier_ticks(&self) -> TimelineTicks {
        self.resume_barrier_ticks
    }

    /// Evaluate the current state at `now_ticks` and decide the next dispatch action.
    pub fn evaluate(&self, now_ticks: TimelineTicks) -> DispatchAction<'_> {
        let mut up_ready_mask = 0u16;
        let mut earliest_active_up_due: Option<TimelineTicks> = None;

        for (slot_idx, slot) in self.slots.iter().enumerate() {
            if slot.active_generation.is_some() {
                let due = slot
                    .authored_up_offset_ticks
                    .max(slot.release_not_before_ticks);
                if now_ticks >= due {
                    up_ready_mask |= 1 << slot_idx;
                } else {
                    earliest_active_up_due = Some(match earliest_active_up_due {
                        Some(earliest) => earliest.min(due),
                        None => due,
                    });
                }
            }
        }

        let chord_opt = self.down_chords.get(self.chord_cursor);
        let mut chord_eligible = false;
        let mut chord_earliest_due = TimelineTicks::ZERO;

        if let Some(chord) = chord_opt {
            chord_eligible = true;
            chord_earliest_due = chord.scheduled_ticks;
            if now_ticks < chord_earliest_due {
                chord_eligible = false;
            }

            for intent in &chord.intents {
                let k = intent.key_slot as usize;
                let slot = &self.slots[k];
                if slot.active_generation.is_some() {
                    chord_eligible = false;
                    let key_release_due = slot
                        .authored_up_offset_ticks
                        .max(slot.release_not_before_ticks);
                    let key_next_down_min =
                        key_release_due.saturating_add_duration(self.frame_ticks);
                    chord_earliest_due = chord_earliest_due.max(key_next_down_min);
                } else if now_ticks < slot.next_down_not_before_ticks {
                    chord_eligible = false;
                    chord_earliest_due = chord_earliest_due.max(slot.next_down_not_before_ticks);
                }
            }
        }

        match (up_ready_mask != 0, chord_eligible) {
            (true, true) => DispatchAction::Send {
                up_mask: up_ready_mask,
                down_chord: chord_opt,
            },
            (true, false) => DispatchAction::Send {
                up_mask: up_ready_mask,
                down_chord: None,
            },
            (false, true) => DispatchAction::Send {
                up_mask: 0,
                down_chord: chord_opt,
            },
            (false, false) => {
                let mut earliest_wake: Option<TimelineTicks> = earliest_active_up_due;
                if chord_opt.is_some() {
                    earliest_wake = Some(match earliest_wake {
                        Some(earliest) => earliest.min(chord_earliest_due),
                        None => chord_earliest_due,
                    });
                }

                if let Some(wake) = earliest_wake {
                    DispatchAction::Wait {
                        deadline_ticks: wake,
                    }
                } else if now_ticks < self.schedule_end_offset_ticks {
                    DispatchAction::Wait {
                        deadline_ticks: self.schedule_end_offset_ticks,
                    }
                } else {
                    DispatchAction::Finished
                }
            }
        }
    }

    /// Commit the result of a successful or partial SendInput transaction.
    pub fn on_send_completed(
        &mut self,
        confirmed_up_mask: u16,
        confirmed_down_mask: u16,
        completion_ticks: TimelineTicks,
    ) -> Result<(), String> {
        let mut mask = confirmed_up_mask;
        while mask != 0 {
            let slot_idx = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            let slot = &mut self.slots[slot_idx];
            if slot.active_generation.take().is_some() {
                slot.next_down_not_before_ticks =
                    completion_ticks.saturating_add_duration(self.frame_ticks);
                self.counters.released = self
                    .counters
                    .released
                    .checked_add(1)
                    .ok_or("counter overflow")?;
            } else {
                return Err(format!("confirmed Up on inactive slot {slot_idx}"));
            }
        }

        if confirmed_down_mask != 0 {
            if self.chord_cursor >= self.down_chords.len() {
                return Err("confirmed Down chord when chord cursor is exhausted".to_string());
            }
            let chord = &self.down_chords[self.chord_cursor];
            if chord.down_mask != confirmed_down_mask {
                return Err(format!(
                    "confirmed Down mask 0x{:04x} does not match chord mask 0x{:04x}",
                    confirmed_down_mask, chord.down_mask
                ));
            }
            for intent in &chord.intents {
                let k = intent.key_slot as usize;
                let slot = &mut self.slots[k];
                if slot.active_generation.is_some() {
                    return Err(format!("attempted to activate already active slot {k}"));
                }
                slot.active_generation = Some(intent.generation_id);
                slot.authored_up_offset_ticks = intent.paired_up_offset_ticks;
                slot.paired_up_source_action_index = intent.paired_up_source_action_index;
                slot.release_not_before_ticks =
                    completion_ticks.saturating_add_duration(self.min_hold_ticks);
                self.activated_generations = self
                    .activated_generations
                    .checked_add(1)
                    .ok_or("counter overflow")?;
            }
            self.chord_cursor += 1;
        }

        Ok(())
    }

    /// Check that cancelling all currently active generations would not overflow
    /// the cancelled counter.
    pub fn check_cleanup_overflow(&self) -> Result<(), String> {
        let active_count = self
            .slots
            .iter()
            .filter(|s| s.active_generation.is_some())
            .count() as u64;
        self.counters
            .cancelled
            .checked_add(active_count)
            .ok_or_else(|| "cancelled counter overflow during resumable cleanup".to_string())?;
        Ok(())
    }

    /// Apply verified resumable cleanup at timestamp `completion_ticks`.
    ///
    /// Clears active musical owners in all 15 slots without physical replay,
    /// marks cancelled generations in accounting, and sets the session-wide
    /// resume barrier to `completion_ticks + T`.
    pub fn on_verified_cleanup(&mut self, completion_ticks: TimelineTicks) -> Result<u64, String> {
        let active_count = self
            .slots
            .iter()
            .filter(|s| s.active_generation.is_some())
            .count() as u64;
        self.counters.cancelled = self
            .counters
            .cancelled
            .checked_add(active_count)
            .ok_or("cancelled counter overflow")?;
        for slot in &mut self.slots {
            slot.active_generation = None;
        }
        let barrier = completion_ticks.saturating_add_duration(self.frame_ticks);
        self.resume_barrier_ticks = self.resume_barrier_ticks.max(barrier);
        Ok(active_count)
    }

    /// True when all chords have been dispatched, all slots are empty, and
    /// the musical timeline has reached `schedule_end_offset_ticks`.
    #[inline]
    pub fn is_finished(&self, now_ticks: TimelineTicks) -> bool {
        self.chord_cursor >= self.down_chords.len()
            && self.slots.iter().all(|s| s.active_generation.is_none())
            && now_ticks >= self.schedule_end_offset_ticks
    }

    /// Snapshot current generation accounting.
    pub fn accounting(&self) -> GenerationAccounting {
        let active_count = self
            .slots
            .iter()
            .filter(|s| s.active_generation.is_some())
            .count() as u64;
        let scheduled = self
            .total_generations
            .saturating_sub(self.activated_generations);
        GenerationAccounting {
            total: self.total_generations,
            activated: self.activated_generations,
            scheduled,
            active: active_count,
            released: self.counters.released,
            dropped_conflict: self.counters.dropped_conflict,
            dropped_backend: self.counters.dropped_backend,
            dropped_expired: self.counters.dropped_expired,
            cancelled: self.counters.cancelled,
        }
    }

    /// Return the authored source action index of the first released key in `up_mask`.
    pub fn up_source_action_index(&self, up_mask: u16) -> Option<u32> {
        for slot in 0..MAX_KEYS {
            if up_mask & (1 << slot) != 0 {
                return Some(self.slots[slot].paired_up_source_action_index);
            }
        }
        None
    }
}

#[cfg(test)]
mod scale_tests;
#[cfg(test)]
mod tests;
