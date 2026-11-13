//! Bounded 15-slot musical dispatch executor.
//!
//! Manages exact musical generation ownership, independent release eligibility
//! per slot, atomic Down-chord transport gating, Up-before-Down combining,
//! and physical H/G timing floors.

use smallvec::SmallVec;

use crate::coordinator::{GenerationAccounting, GenerationCounters};
use crate::model::{GenerationId, KeyRegistry, KeySlot, MAX_KEYS, RuntimeSchedule};
use crate::time::{DurationTicks, QpcTicks, TimeArithmeticError, TimelineTicks};
mod memory;
pub use memory::MAX_EXECUTOR_BYTES;
use memory::{checked_executor_storage_bytes, reserve_executor_chords};

/// Per-slot musical ownership state for the 15 instrument keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MusicalSlot {
    /// Active musical generation in this slot, if any.
    pub active_generation: Option<GenerationId>,
    /// Exact authored Up offset for this generation.
    pub authored_up_offset_ticks: TimelineTicks,
    /// Physical hold floor: successful Down completion QPC + H.
    pub release_not_before_qpc: QpcTicks,
    /// Physical repress floor: previous Up completion QPC + T.
    pub next_down_not_before_qpc: QpcTicks,
    /// Exact authored source action index of the paired Up.
    pub paired_up_source_action_index: u32,
}

impl Default for MusicalSlot {
    fn default() -> Self {
        Self {
            active_generation: None,
            authored_up_offset_ticks: TimelineTicks::ZERO,
            release_not_before_qpc: QpcTicks::ZERO,
            next_down_not_before_qpc: QpcTicks::ZERO,
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

/// Absolute-QPC decision made by the production scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchDecision<'a> {
    /// Ready physical packet to stage and send.
    Send {
        up_mask: u16,
        down_chord: Option<&'a DownChordTicks>,
        timing_window: DispatchTimingWindow,
    },
    /// No physical work ready; wait until the specified deadline.
    Wait { deadline_qpc: QpcTicks },
    /// All Down chords sent, all slots released, and timeline end offset reached.
    Finished,
}

/// Timeline-zero adapter retained for core unit fixtures that model an
/// epoch at QPC zero. Production dispatch uses [`DispatchDecision`].
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchAction<'a> {
    Send {
        up_mask: u16,
        down_chord: Option<&'a DownChordTicks>,
    },
    Wait {
        deadline_ticks: TimelineTicks,
    },
    Finished,
}

/// Scheduler-owned absolute QPC eligibility evidence for one physical packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchTimingWindow {
    pub authored_target_qpc: QpcTicks,
    pub musical_up_not_before_qpc: QpcTicks,
    pub down_not_before_qpc: QpcTicks,
    pub packet_not_before_qpc: QpcTicks,
    pub hold_floor_mask: u16,
    pub release_floor_mask: u16,
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
    resume_barrier_qpc: Option<QpcTicks>,
    min_hold_ticks: DurationTicks,
    frame_ticks: DurationTicks,
    counters: GenerationCounters,
    total_generations: u64,
    activated_generations: u64,
}

pub const EXECUTOR_BUILD_CHECKPOINT_INTERVAL: usize = 4096;

/// Measured memory footprint of the `DispatchExecutor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutorMemoryMetrics {
    pub chord_count: usize,
    pub chord_capacity: usize,
    pub chord_heap_bytes: usize,
    pub struct_bytes: usize,
    pub total_bytes: usize,
}

/// Timing and authored metadata for an active key slot being released.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UpSlotTiming {
    pub slot: u8,
    pub scan_code: u16,
    pub source_action_index: u32,
    pub authored_up_offset_ticks: TimelineTicks,
    pub release_not_before_qpc: QpcTicks,
}

impl UpSlotTiming {
    pub const EMPTY: Self = Self {
        slot: 0xFF,
        scan_code: 0,
        source_action_index: 0,
        authored_up_offset_ticks: TimelineTicks::ZERO,
        release_not_before_qpc: QpcTicks::ZERO,
    };
}

impl DispatchExecutor {
    /// Construct a new 15-slot executor from a validated `RuntimeSchedule`.
    pub fn new<F, E>(
        schedule: &RuntimeSchedule,
        min_hold_ticks: DurationTicks,
        frame_ticks: DurationTicks,
        us_to_ticks: F,
    ) -> Result<Self, String>
    where
        F: FnMut(u64) -> Result<TimelineTicks, E>,
        E: std::fmt::Display,
    {
        Self::new_with_cancellation(
            schedule,
            min_hold_ticks,
            frame_ticks,
            us_to_ticks,
            || Ok(()),
        )
    }

    /// Construct a new 15-slot executor with bounded memory admission and periodic cancellation checkpoints.
    pub fn new_with_cancellation<F, E, C>(
        schedule: &RuntimeSchedule,
        min_hold_ticks: DurationTicks,
        frame_ticks: DurationTicks,
        mut us_to_ticks: F,
        mut checkpoint: C,
    ) -> Result<Self, String>
    where
        F: FnMut(u64) -> Result<TimelineTicks, E>,
        E: std::fmt::Display,
        C: FnMut() -> Result<(), String>,
    {
        let chord_count = schedule.down_chords.len();
        checked_executor_storage_bytes(chord_count)?;

        checkpoint()?;

        let mut down_chords = Vec::new();
        reserve_executor_chords(&mut down_chords, chord_count)?;

        for (idx, chord) in schedule.down_chords.iter().enumerate() {
            if idx != 0 && idx % EXECUTOR_BUILD_CHECKPOINT_INTERVAL == 0 {
                checkpoint()?;
            }
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

        checkpoint()?;

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
            resume_barrier_qpc: None,
            min_hold_ticks,
            frame_ticks,
            counters: GenerationCounters::default(),
            total_generations: schedule.generation_count,
            activated_generations: 0,
        })
    }

    /// Return measured memory metrics for this executor instance.
    pub fn memory_metrics(&self) -> ExecutorMemoryMetrics {
        self.try_memory_metrics()
            .expect("allocated executor memory must fit its checked storage budget")
    }

    pub fn try_memory_metrics(&self) -> Result<ExecutorMemoryMetrics, String> {
        let chord_heap_bytes = checked_executor_storage_bytes(self.down_chords.capacity())?;
        let struct_bytes = std::mem::size_of::<Self>();
        let total_bytes = chord_heap_bytes
            .checked_add(struct_bytes)
            .ok_or_else(|| "executor total memory size overflow".to_string())?;
        Ok(ExecutorMemoryMetrics {
            chord_count: self.down_chords.len(),
            chord_capacity: self.down_chords.capacity(),
            chord_heap_bytes,
            struct_bytes,
            total_bytes,
        })
    }

    /// Extract per-slot authored Up metadata for the keys active in `up_mask`.
    pub fn up_slot_timings(&self, up_mask: u16) -> SmallVec<[UpSlotTiming; MAX_KEYS]> {
        let mut timings = SmallVec::new();
        for (i, slot) in self.slots.iter().enumerate() {
            if (up_mask & (1 << i)) != 0 && slot.active_generation.is_some() {
                let scan_code = self.key_registry.scan_code_for(i as u8).unwrap_or(0);
                timings.push(UpSlotTiming {
                    slot: i as u8,
                    scan_code,
                    source_action_index: slot.paired_up_source_action_index,
                    authored_up_offset_ticks: slot.authored_up_offset_ticks,
                    release_not_before_qpc: slot.release_not_before_qpc,
                });
            }
        }
        timings
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
    pub fn resume_barrier_qpc(&self) -> Option<QpcTicks> {
        self.resume_barrier_qpc
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn resume_barrier_ticks(&self) -> TimelineTicks {
        TimelineTicks::from_raw(self.resume_barrier_qpc.unwrap_or(QpcTicks::ZERO).as_u64())
    }

    /// Evaluate the current state at an absolute QPC time and decide the next action.
    /// Authored targets are offsets from the current playback epoch; physical
    /// H/G and resume floors remain absolute across pause-driven epoch changes.
    pub fn evaluate_at(
        &self,
        now_qpc: QpcTicks,
        epoch_qpc: QpcTicks,
    ) -> Result<DispatchDecision<'_>, TimeArithmeticError> {
        let mut up_ready_mask = 0u16;
        let mut earliest_active_up_due_qpc: Option<QpcTicks> = None;
        let mut up_packet_not_before_qpc: Option<QpcTicks> = None;
        let mut up_authored_target_qpc: Option<QpcTicks> = None;
        let mut up_hold_floor_mask = 0u16;

        for (slot_idx, slot) in self.slots.iter().enumerate() {
            if slot.active_generation.is_some() {
                let authored_up_qpc = epoch_qpc.checked_add_duration(DurationTicks::from_raw(
                    slot.authored_up_offset_ticks.as_u64(),
                ))?;
                let due = authored_up_qpc.max(slot.release_not_before_qpc);
                if now_qpc >= due {
                    up_ready_mask |= 1 << slot_idx;
                    up_packet_not_before_qpc =
                        Some(up_packet_not_before_qpc.map_or(due, |value| value.max(due)));
                    up_authored_target_qpc = Some(
                        up_authored_target_qpc
                            .map_or(authored_up_qpc, |value| value.min(authored_up_qpc)),
                    );
                    if slot.release_not_before_qpc > authored_up_qpc {
                        up_hold_floor_mask |= 1 << slot_idx;
                    }
                } else {
                    // A due Up is sent immediately. Until then, only the
                    // earliest active release can wake the worker.
                    earliest_active_up_due_qpc =
                        Some(earliest_active_up_due_qpc.map_or(due, |value| value.min(due)));
                }
            }
        }

        let chord_opt = self.down_chords.get(self.chord_cursor);
        let mut chord_eligible = false;
        let mut chord_earliest_due_qpc = QpcTicks::ZERO;
        let mut chord_authored_target_qpc = None;
        let mut down_release_floor_mask = 0u16;

        if let Some(chord) = chord_opt {
            let authored_down_qpc = epoch_qpc
                .checked_add_duration(DurationTicks::from_raw(chord.scheduled_ticks.as_u64()))?;
            chord_authored_target_qpc = Some(authored_down_qpc);
            chord_eligible = true;
            chord_earliest_due_qpc = authored_down_qpc;
            if now_qpc < authored_down_qpc {
                chord_eligible = false;
            }
            if let Some(resume_barrier) = self.resume_barrier_qpc
                && resume_barrier > authored_down_qpc
            {
                chord_earliest_due_qpc = chord_earliest_due_qpc.max(resume_barrier);
                down_release_floor_mask |= chord.down_mask;
                if now_qpc < resume_barrier {
                    chord_eligible = false;
                }
            }

            for intent in &chord.intents {
                let k = intent.key_slot as usize;
                let slot = &self.slots[k];
                if slot.active_generation.is_some() {
                    chord_eligible = false;
                    let authored_up_qpc = epoch_qpc.checked_add_duration(
                        DurationTicks::from_raw(slot.authored_up_offset_ticks.as_u64()),
                    )?;
                    let key_release_due = authored_up_qpc.max(slot.release_not_before_qpc);
                    let key_next_down_min =
                        key_release_due.checked_add_duration(self.frame_ticks)?;
                    chord_earliest_due_qpc = chord_earliest_due_qpc.max(key_next_down_min);
                    if key_next_down_min > authored_down_qpc {
                        down_release_floor_mask |= 1 << k;
                    }
                } else {
                    chord_earliest_due_qpc =
                        chord_earliest_due_qpc.max(slot.next_down_not_before_qpc);
                    if slot.next_down_not_before_qpc > authored_down_qpc {
                        down_release_floor_mask |= 1 << k;
                    }
                    if now_qpc < slot.next_down_not_before_qpc {
                        chord_eligible = false;
                    }
                }
            }
        }

        let up_ready_qpc = up_packet_not_before_qpc.filter(|_| up_ready_mask != 0);
        let up_authored_qpc = up_authored_target_qpc.filter(|_| up_ready_mask != 0);
        let down_ready_qpc = chord_eligible.then_some(chord_earliest_due_qpc);
        let down_authored_qpc = chord_authored_target_qpc.filter(|_| chord_eligible);
        match (up_ready_mask != 0, chord_eligible) {
            (true, true) | (true, false) | (false, true) => {
                let authored_target_qpc = match (up_authored_qpc, down_authored_qpc) {
                    (Some(up), Some(down)) => up.min(down),
                    (Some(up), None) => up,
                    (None, Some(down)) => down,
                    (None, None) => unreachable!("a send action has a selected direction"),
                };
                let musical_up_not_before_qpc = up_ready_qpc.unwrap_or(authored_target_qpc);
                let down_not_before_qpc = down_ready_qpc.unwrap_or(authored_target_qpc);
                Ok(DispatchDecision::Send {
                    up_mask: if up_ready_mask != 0 { up_ready_mask } else { 0 },
                    down_chord: if chord_eligible { chord_opt } else { None },
                    timing_window: DispatchTimingWindow {
                        authored_target_qpc,
                        musical_up_not_before_qpc,
                        down_not_before_qpc,
                        packet_not_before_qpc: musical_up_not_before_qpc.max(down_not_before_qpc),
                        hold_floor_mask: up_hold_floor_mask,
                        release_floor_mask: if chord_eligible {
                            down_release_floor_mask
                        } else {
                            0
                        },
                    },
                })
            }
            (false, false) => {
                let mut earliest_wake = if up_ready_mask == 0 {
                    earliest_active_up_due_qpc
                } else {
                    None
                };
                if chord_opt.is_some() {
                    earliest_wake =
                        Some(earliest_wake.map_or(chord_earliest_due_qpc, |earliest| {
                            earliest.min(chord_earliest_due_qpc)
                        }));
                }

                if let Some(wake) = earliest_wake {
                    Ok(DispatchDecision::Wait { deadline_qpc: wake })
                } else {
                    let schedule_end_qpc = epoch_qpc.checked_add_duration(
                        DurationTicks::from_raw(self.schedule_end_offset_ticks.as_u64()),
                    )?;
                    if now_qpc < schedule_end_qpc {
                        Ok(DispatchDecision::Wait {
                            deadline_qpc: schedule_end_qpc,
                        })
                    } else {
                        Ok(DispatchDecision::Finished)
                    }
                }
            }
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn evaluate(&self, now_ticks: TimelineTicks) -> DispatchAction<'_> {
        match self
            .evaluate_at(QpcTicks::from_raw(now_ticks.as_u64()), QpcTicks::ZERO)
            .expect("timeline-zero test scheduler timestamps must fit QPC")
        {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                ..
            } => DispatchAction::Send {
                up_mask,
                down_chord,
            },
            DispatchDecision::Wait { deadline_qpc } => DispatchAction::Wait {
                deadline_ticks: TimelineTicks::from_raw(deadline_qpc.as_u64()),
            },
            DispatchDecision::Finished => DispatchAction::Finished,
        }
    }

    /// Commit the result of a successful or partial SendInput transaction.
    pub fn on_send_completed_at(
        &mut self,
        confirmed_up_mask: u16,
        confirmed_down_mask: u16,
        completion_qpc: QpcTicks,
    ) -> Result<(), String> {
        if confirmed_up_mask & confirmed_down_mask != 0 {
            return Err("confirmed packet has overlapping Up and Down masks".to_string());
        }
        let new_down_floor = if confirmed_up_mask != 0 {
            Some(
                completion_qpc
                    .checked_add_duration(self.frame_ticks)
                    .map_err(|error| format!("Up completion G floor overflow: {error}"))?,
            )
        } else {
            None
        };
        let new_up_floor = if confirmed_down_mask != 0 {
            Some(
                completion_qpc
                    .checked_add_duration(self.min_hold_ticks)
                    .map_err(|error| format!("Down completion H floor overflow: {error}"))?,
            )
        } else {
            None
        };

        let mut up_slots = confirmed_up_mask;
        while up_slots != 0 {
            let slot_idx = up_slots.trailing_zeros() as usize;
            up_slots &= up_slots - 1;
            if self.slots[slot_idx].active_generation.is_none() {
                return Err(format!("confirmed Up on inactive slot {slot_idx}"));
            }
        }

        let chord = if confirmed_down_mask != 0 {
            let chord = self
                .down_chords
                .get(self.chord_cursor)
                .ok_or_else(|| "confirmed Down chord when chord cursor is exhausted".to_string())?;
            if chord.down_mask != confirmed_down_mask {
                return Err(format!(
                    "confirmed Down mask 0x{:04x} does not match chord mask 0x{:04x}",
                    confirmed_down_mask, chord.down_mask
                ));
            }
            for intent in &chord.intents {
                if self.slots[intent.key_slot as usize]
                    .active_generation
                    .is_some()
                {
                    return Err(format!(
                        "attempted to activate already active slot {}",
                        intent.key_slot
                    ));
                }
            }
            Some(chord)
        } else {
            None
        };
        if chord.is_some() && confirmed_down_mask & confirmed_up_mask != 0 {
            return Err("confirmed packet releases and represses the same slot".to_string());
        }
        let next_released = self
            .counters
            .released
            .checked_add(u64::from(confirmed_up_mask.count_ones()))
            .ok_or("released counter overflow")?;
        let activated_count = chord.map_or(0, |chord| chord.intents.len() as u64);
        let next_activated = self
            .activated_generations
            .checked_add(activated_count)
            .ok_or("activated counter overflow")?;
        if next_activated > self.total_generations {
            return Err("activated generation count exceeds compiled total".to_string());
        }
        let next_cursor = self
            .chord_cursor
            .checked_add(if chord.is_some() { 1 } else { 0 })
            .ok_or("Down chord cursor overflow")?;

        let mut up_slots = confirmed_up_mask;
        while up_slots != 0 {
            let slot_idx = up_slots.trailing_zeros() as usize;
            up_slots &= up_slots - 1;
            let slot = &mut self.slots[slot_idx];
            slot.active_generation = None;
            slot.release_not_before_qpc = QpcTicks::ZERO;
            slot.next_down_not_before_qpc = new_down_floor.unwrap_or(QpcTicks::ZERO);
        }
        if let Some(chord) = chord {
            for intent in &chord.intents {
                let slot = &mut self.slots[intent.key_slot as usize];
                slot.active_generation = Some(intent.generation_id);
                slot.authored_up_offset_ticks = intent.paired_up_offset_ticks;
                slot.paired_up_source_action_index = intent.paired_up_source_action_index;
                slot.release_not_before_qpc = new_up_floor.unwrap_or(QpcTicks::ZERO);
                slot.next_down_not_before_qpc = QpcTicks::ZERO;
            }
        }
        self.counters.released = next_released;
        self.activated_generations = next_activated;
        self.chord_cursor = next_cursor;

        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn on_send_completed(
        &mut self,
        confirmed_up_mask: u16,
        confirmed_down_mask: u16,
        completion_ticks: TimelineTicks,
    ) -> Result<(), String> {
        self.on_send_completed_at(
            confirmed_up_mask,
            confirmed_down_mask,
            QpcTicks::from_raw(completion_ticks.as_u64()),
        )
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
    pub fn on_verified_cleanup_at(&mut self, completion_qpc: QpcTicks) -> Result<u64, String> {
        let active_count = self
            .slots
            .iter()
            .filter(|s| s.active_generation.is_some())
            .count() as u64;
        let next_cancelled = self
            .counters
            .cancelled
            .checked_add(active_count)
            .ok_or("cancelled counter overflow")?;
        let barrier = completion_qpc
            .checked_add_duration(self.frame_ticks)
            .map_err(|error| format!("resume barrier overflow: {error}"))?;
        for slot in &mut self.slots {
            slot.active_generation = None;
            slot.release_not_before_qpc = QpcTicks::ZERO;
            slot.next_down_not_before_qpc = QpcTicks::ZERO;
        }
        self.counters.cancelled = next_cancelled;
        self.resume_barrier_qpc = Some(
            self.resume_barrier_qpc
                .map_or(barrier, |current| current.max(barrier)),
        );
        Ok(active_count)
    }

    /// Cancel every remaining generation after verified terminal cleanup.
    /// Scheduled generations are included because the session will not resume.
    pub fn on_verified_terminal_cleanup(&mut self) -> Result<u64, String> {
        let accounting = self.accounting();
        let cancelled_count = accounting
            .active
            .checked_add(accounting.scheduled)
            .ok_or("terminal cancellation count overflow")?;
        let next_cancelled = self
            .counters
            .cancelled
            .checked_add(cancelled_count)
            .ok_or("terminal cancelled counter overflow")?;
        self.counters.cancelled = next_cancelled;
        for slot in &mut self.slots {
            slot.active_generation = None;
            slot.release_not_before_qpc = QpcTicks::ZERO;
            slot.next_down_not_before_qpc = QpcTicks::ZERO;
        }
        Ok(cancelled_count)
    }

    /// Check the bounded scheduler state and generation lifecycle partition.
    pub fn check_invariants(&self) -> Result<(), String> {
        if self.chord_cursor > self.down_chords.len() {
            return Err("scheduler chord cursor exceeds the compiled chord count".into());
        }
        for (slot_index, slot) in self.slots.iter().enumerate() {
            if slot
                .active_generation
                .is_some_and(|generation| generation >= self.total_generations)
            {
                return Err(format!(
                    "scheduler slot {slot_index} owns an unknown generation"
                ));
            }
            if let Some(generation) = slot.active_generation
                && self.slots[..slot_index]
                    .iter()
                    .any(|other| other.active_generation == Some(generation))
            {
                return Err(format!(
                    "scheduler generation {generation} owns multiple slots"
                ));
            }
        }
        let accounting = self.accounting();
        let accounted = accounting
            .scheduled
            .checked_add(accounting.active)
            .and_then(|count| count.checked_add(accounting.released))
            .and_then(|count| count.checked_add(accounting.dropped_conflict))
            .and_then(|count| count.checked_add(accounting.dropped_backend))
            .and_then(|count| count.checked_add(accounting.dropped_expired))
            .and_then(|count| count.checked_add(accounting.cancelled))
            .ok_or("scheduler lifecycle accounting overflow")?;
        if accounted != accounting.total {
            return Err(format!(
                "scheduler lifecycle accounting {accounted} does not match total {}",
                accounting.total
            ));
        }
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn on_verified_cleanup(&mut self, completion_ticks: TimelineTicks) -> Result<u64, String> {
        self.on_verified_cleanup_at(QpcTicks::from_raw(completion_ticks.as_u64()))
    }

    /// True when all chords have been dispatched, all slots are empty, and
    /// the musical timeline has reached `schedule_end_offset_ticks`.
    #[inline]
    pub fn is_finished(&self, now_ticks: TimelineTicks) -> bool {
        self.chord_cursor >= self.down_chords.len()
            && self.slots.iter().all(|s| s.active_generation.is_none())
            && now_ticks >= self.schedule_end_offset_ticks
    }

    pub fn is_finished_at(
        &self,
        now_qpc: QpcTicks,
        epoch_qpc: QpcTicks,
    ) -> Result<bool, TimeArithmeticError> {
        let end_qpc = epoch_qpc.checked_add_duration(DurationTicks::from_raw(
            self.schedule_end_offset_ticks.as_u64(),
        ))?;
        Ok(self.chord_cursor >= self.down_chords.len()
            && self
                .slots
                .iter()
                .all(|slot| slot.active_generation.is_none())
            && now_qpc >= end_qpc)
    }

    /// Snapshot current generation accounting.
    pub fn accounting(&self) -> GenerationAccounting {
        let active_count = self
            .slots
            .iter()
            .filter(|s| s.active_generation.is_some())
            .count() as u64;
        let terminal = self.counters.terminal_total();
        let scheduled = self
            .total_generations
            .checked_sub(active_count)
            .and_then(|count| count.checked_sub(terminal))
            .expect("scheduler lifecycle counters must remain valid");
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

    /// Return compact generation status counts for terminal telemetry.
    pub fn generation_status_counts(&self) -> std::collections::HashMap<String, u64> {
        let accounting = self.accounting();
        [
            ("scheduled", accounting.scheduled),
            ("active", accounting.active),
            ("released", accounting.released),
            ("dropped_conflict", accounting.dropped_conflict),
            ("dropped_backend", accounting.dropped_backend),
            ("dropped_expired", accounting.dropped_expired),
            ("cancelled", accounting.cancelled),
        ]
        .into_iter()
        .map(|(status, count)| (status.to_string(), count))
        .collect()
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
mod memory_tests;

#[cfg(test)]
mod absolute_tests;

#[cfg(test)]
#[path = "executor/metadata_tests.rs"]
mod metadata_tests;
#[cfg(test)]
mod scale_tests;
#[cfg(test)]
mod tests;
