//! Bounded 15-slot musical dispatch executor.
//!
//! Manages exact musical generation ownership, independent release eligibility
//! per slot, atomic Down-chord transport gating, Up-before-Down combining,
//! and physical H/G timing floors.

use smallvec::SmallVec;
use std::num::NonZeroU64;

use crate::accounting::{GenerationAccounting, GenerationCounters};
use crate::model::{GenerationId, KeyRegistry, KeySlot, MAX_KEYS, RuntimeSchedule};
use crate::time::{DurationTicks, QpcTicks, TimeArithmeticError, TimelineTicks};
mod memory;
pub use memory::MAX_EXECUTOR_BYTES;
pub(crate) use memory::checked_executor_storage_bytes;
use memory::{reserve_executor_chords, reserve_executor_intents};

#[cfg(any(test, feature = "test-support"))]
std::thread_local! {
    static SCHEDULER_EVALUATION_COUNT: std::cell::Cell<u64> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(any(test, feature = "test-support"))]
pub fn reset_scheduler_evaluation_count_for_test() {
    SCHEDULER_EVALUATION_COUNT.with(|count| count.set(0));
}

#[cfg(any(test, feature = "test-support"))]
pub fn scheduler_evaluation_count_for_test() -> u64 {
    SCHEDULER_EVALUATION_COUNT.with(std::cell::Cell::get)
}

/// Per-slot musical ownership state for the 15 instrument keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MusicalSlot {
    /// Active musical generation in this slot, if any.
    pub active_generation: Option<GenerationId>,
    /// Exact authored Up offset for this generation.
    pub authored_up_offset_ticks: TimelineTicks,
    /// Physical hold floor: successful Down completion QPC + H.
    pub release_not_before_qpc: QpcTicks,
    /// Physical repress floor: previous Up completion QPC + G.
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

/// An owned Down chord fixture. The live executor stores its chord header and
/// intents in separate contiguous vectors; this owned form remains useful to
/// compile/test-support paths that need a self-contained fixture.
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

/// Borrowed view over one compact live-executor chord and its contiguous intents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownChordView<'a> {
    pub chord_index: usize,
    pub scheduled_ticks: TimelineTicks,
    pub scheduled_us: u64,
    pub down_mask: u16,
    pub source_action_index: u32,
    pub reason_id: crate::model::ReasonId,
    pub intents: &'a [DownIntentTicks],
}

/// Small, copyable provenance retained by the worker while a packet is staged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownChordMetadata {
    pub chord_index: usize,
    pub scheduled_ticks: TimelineTicks,
    pub scheduled_us: u64,
    pub down_mask: u16,
    pub source_action_index: u32,
    pub reason_id: crate::model::ReasonId,
}

impl From<DownChordTicks> for DownChordMetadata {
    fn from(chord: DownChordTicks) -> Self {
        Self {
            chord_index: chord.chord_index,
            scheduled_ticks: chord.scheduled_ticks,
            scheduled_us: chord.scheduled_us,
            down_mask: chord.down_mask,
            source_action_index: chord.source_action_index,
            reason_id: chord.reason_id,
        }
    }
}

impl From<&DownChordTicks> for DownChordMetadata {
    fn from(chord: &DownChordTicks) -> Self {
        Self {
            chord_index: chord.chord_index,
            scheduled_ticks: chord.scheduled_ticks,
            scheduled_us: chord.scheduled_us,
            down_mask: chord.down_mask,
            source_action_index: chord.source_action_index,
            reason_id: chord.reason_id,
        }
    }
}

impl From<DownChordView<'_>> for DownChordMetadata {
    fn from(chord: DownChordView<'_>) -> Self {
        Self {
            chord_index: chord.chord_index,
            scheduled_ticks: chord.scheduled_ticks,
            scheduled_us: chord.scheduled_us,
            down_mask: chord.down_mask,
            source_action_index: chord.source_action_index,
            reason_id: chord.reason_id,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DownChordHeaderTicks {
    chord_index: usize,
    scheduled_ticks: TimelineTicks,
    scheduled_us: u64,
    down_mask: u16,
    source_action_index: u32,
    reason_id: crate::model::ReasonId,
    intent_start: usize,
    intent_count: usize,
}

/// Absolute-QPC decision made by the production scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchDecision<'a> {
    /// Ready physical packet to stage and send.
    Send {
        up_mask: u16,
        down_chord: Option<DownChordView<'a>>,
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
        down_chord: Option<DownChordView<'a>>,
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
    down_chords: Vec<DownChordHeaderTicks>,
    down_intents: Vec<DownIntentTicks>,
    schedule_end_offset_ticks: TimelineTicks,
    schedule_end_offset_us: u64,
    key_registry: KeyRegistry,
    chord_cursor: usize,
    slots: [MusicalSlot; MAX_KEYS],
    resume_barrier_qpc: Option<QpcTicks>,
    min_hold_ticks: DurationTicks,
    min_release_gap_ticks: DurationTicks,
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
    pub intent_count: usize,
    pub intent_capacity: usize,
    pub chord_header_heap_bytes: usize,
    pub intent_heap_bytes: usize,
    /// Total capacity bytes for both executor-owned vectors.
    pub chord_heap_bytes: usize,
    pub struct_bytes: usize,
    pub total_bytes: usize,
}

/// Authored metadata for an Up intent that has no paired Down generation.
/// This is prepared before Arm so startup does not walk every action merely
/// to produce the existing stale-metadata diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedUnmatchedUpMetadata {
    pub source_action_index: u32,
    pub scheduled_ticks: TimelineTicks,
    pub suppressed_intent_count: usize,
}

/// Capacity evidence for an immutable dispatch plan and its overlapping
/// construction phase. Values describe allocated capacities, not process RSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedDispatchMemoryMetrics {
    pub chord_count: usize,
    pub chord_capacity: usize,
    pub intent_count: usize,
    pub intent_capacity: usize,
    pub unmatched_up_count: usize,
    pub unmatched_up_capacity: usize,
    pub plan_capacity_bytes: usize,
    pub overlapping_peak_capacity_bytes: usize,
}

/// Immutable, QPC-domain dispatch materialization produced during Prepare.
/// It contains only the fixed dispatch representation and the metadata needed
/// to initialize a fresh session; cursor, slots, epoch, and transport state
/// are created separately by `DispatchExecutor::from_prepared_schedule`.
#[derive(Debug)]
pub struct PreparedDispatchSchedule {
    down_chords: Vec<DownChordHeaderTicks>,
    down_intents: Vec<DownIntentTicks>,
    schedule_end_offset_ticks: TimelineTicks,
    schedule_end_offset_us: u64,
    key_registry: KeyRegistry,
    total_generations: u64,
    qpc_frequency_hz: NonZeroU64,
    min_hold_us: u64,
    min_release_gap_us: u64,
    min_hold_ticks: DurationTicks,
    min_release_gap_ticks: DurationTicks,
    total_us: u64,
    packet_count: usize,
    has_down: bool,
    unmatched_up_metadata: Vec<PreparedUnmatchedUpMetadata>,
    memory_metrics: PreparedDispatchMemoryMetrics,
}

impl PreparedDispatchSchedule {
    /// Compile a `RuntimeSchedule` into the immutable QPC-relative physical
    /// representation used by a later session. The conversion rounds up in
    /// the same way as `QpcClock::timeline_from_us`.
    pub fn prepare(
        schedule: &RuntimeSchedule,
        qpc_frequency_hz: NonZeroU64,
        min_hold_us: u64,
        min_release_gap_us: u64,
        caller_retained_capacity_bytes: usize,
    ) -> Result<Self, String> {
        Self::prepare_with_cancellation(
            schedule,
            qpc_frequency_hz,
            min_hold_us,
            min_release_gap_us,
            caller_retained_capacity_bytes,
            || Ok(()),
        )
    }

    /// Cancellable form used by the desktop Prepare path and by raw-schedule
    /// admission. Checkpoints are bounded by the number of authored packets.
    pub fn prepare_with_cancellation(
        schedule: &RuntimeSchedule,
        qpc_frequency_hz: NonZeroU64,
        min_hold_us: u64,
        min_release_gap_us: u64,
        caller_retained_capacity_bytes: usize,
        mut checkpoint: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        checkpoint()?;
        let qpc_to_ticks = |microseconds| {
            let numerator = (microseconds as u128)
                .checked_mul(qpc_frequency_hz.get() as u128)
                .and_then(|value| value.checked_add(999_999))?;
            u64::try_from(numerator / 1_000_000)
                .ok()
                .map(TimelineTicks::from_raw)
        };
        let min_hold_ticks = qpc_to_ticks(min_hold_us)
            .map(|ticks| DurationTicks::from_raw(ticks.as_u64()))
            .ok_or_else(|| "prepared hold-floor conversion overflow".to_string())?;
        let min_release_gap_ticks = qpc_to_ticks(min_release_gap_us)
            .map(|ticks| DurationTicks::from_raw(ticks.as_u64()))
            .ok_or_else(|| "prepared release-gap conversion overflow".to_string())?;
        crate::validation::validate_min_hold_and_release_gap_feasibility_ticks_with_checkpoint(
            schedule,
            min_hold_us,
            min_hold_ticks,
            min_release_gap_us,
            min_release_gap_ticks,
            qpc_to_ticks,
            || checkpoint().map_err(|_| crate::validation::ScheduleTimingError::Cancelled),
        )
        .map_err(|error| format!("prepared native schedule admission failed: {error}"))?;
        checkpoint()?;

        let chord_count = schedule.down_chords.len();
        let mut intent_count = 0usize;
        for (index, chord) in schedule.down_chords.iter().enumerate() {
            if index % EXECUTOR_BUILD_CHECKPOINT_INTERVAL == 0 {
                checkpoint()?;
            }
            intent_count = intent_count
                .checked_add(chord.intents.len())
                .ok_or_else(|| "executor Down intent count overflow".to_string())?;
        }
        checked_executor_storage_bytes(chord_count, intent_count)?;

        let mut down_chords = Vec::new();
        reserve_executor_chords(&mut down_chords, chord_count)?;
        let mut down_intents = Vec::new();
        reserve_executor_intents(&mut down_intents, intent_count)?;
        for (index, chord) in schedule.down_chords.iter().enumerate() {
            if index % EXECUTOR_BUILD_CHECKPOINT_INTERVAL == 0 {
                checkpoint()?;
            }
            let scheduled_ticks = qpc_to_ticks(chord.scheduled_us).ok_or_else(|| {
                format!(
                    "scheduled timestamp conversion failed for chord at {}us",
                    chord.scheduled_us
                )
            })?;
            let intent_start = down_intents.len();
            for intent in &chord.intents {
                let paired_up_offset_ticks =
                    qpc_to_ticks(intent.paired_up_offset_us).ok_or_else(|| {
                        format!(
                            "paired Up offset timestamp conversion failed at {}us",
                            intent.paired_up_offset_us
                        )
                    })?;
                down_intents.push(DownIntentTicks {
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
            let intent_len = down_intents
                .len()
                .checked_sub(intent_start)
                .ok_or_else(|| "executor Down intent range underflow".to_string())?;
            down_chords.push(DownChordHeaderTicks {
                chord_index: chord.chord_index,
                scheduled_ticks,
                scheduled_us: chord.scheduled_us,
                down_mask: chord.down_mask,
                source_action_index: chord.source_action_index,
                reason_id: chord.reason_id,
                intent_start,
                intent_count: intent_len,
            });
        }
        if down_intents.len() != intent_count {
            return Err("executor Down intent count changed during preparation".to_string());
        }

        let mut unmatched_up_count = 0usize;
        for (index, batch) in schedule.batches.iter().enumerate() {
            if index % EXECUTOR_BUILD_CHECKPOINT_INTERVAL == 0 {
                checkpoint()?;
            }
            if batch.kind != crate::model::ActionKind::Up {
                continue;
            }
            let start = usize::try_from(batch.intent_start)
                .map_err(|_| "prepared unmatched-Up intent range overflow")?;
            let end = start
                .checked_add(usize::from(batch.intent_len))
                .ok_or("prepared unmatched-Up intent range overflow")?;
            let intents = schedule
                .intents
                .get(start..end)
                .ok_or("prepared unmatched-Up intent range is invalid")?;
            if intents
                .iter()
                .any(|intent| intent.generation_id() == crate::model::NO_GENERATION_ID)
            {
                unmatched_up_count = unmatched_up_count
                    .checked_add(1)
                    .ok_or_else(|| "prepared unmatched-Up count overflow".to_string())?;
            }
        }
        let mut unmatched_up_metadata = Vec::new();
        unmatched_up_metadata
            .try_reserve_exact(unmatched_up_count)
            .map_err(|error| format!("prepared unmatched-Up allocation failed: {error}"))?;
        for (index, batch) in schedule.batches.iter().enumerate() {
            if index % EXECUTOR_BUILD_CHECKPOINT_INTERVAL == 0 {
                checkpoint()?;
            }
            if batch.kind != crate::model::ActionKind::Up {
                continue;
            }
            let start = usize::try_from(batch.intent_start)
                .map_err(|_| "prepared unmatched-Up intent range overflow")?;
            let end = start
                .checked_add(usize::from(batch.intent_len))
                .ok_or("prepared unmatched-Up intent range overflow")?;
            let intents = schedule
                .intents
                .get(start..end)
                .ok_or("prepared unmatched-Up intent range is invalid")?;
            let suppressed_intent_count = intents
                .iter()
                .filter(|intent| intent.generation_id() == crate::model::NO_GENERATION_ID)
                .count();
            if suppressed_intent_count != 0 {
                unmatched_up_metadata.push(PreparedUnmatchedUpMetadata {
                    source_action_index: batch.source_action_index,
                    scheduled_ticks: qpc_to_ticks(batch.scheduled_us).ok_or_else(|| {
                        format!(
                            "unmatched Up timestamp conversion failed at {}us",
                            batch.scheduled_us
                        )
                    })?,
                    suppressed_intent_count,
                });
            }
        }
        checkpoint()?;

        let schedule_memory = schedule.memory_metrics_with_checkpoint(checkpoint)?;
        let schedule_bytes = schedule_memory
            .heap_payload_bytes
            .checked_add(schedule_memory.struct_bytes)
            .ok_or_else(|| "runtime schedule capacity overflow".to_string())?;
        let chord_bytes = down_chords
            .capacity()
            .checked_mul(std::mem::size_of::<DownChordHeaderTicks>())
            .ok_or_else(|| "prepared chord capacity overflow".to_string())?;
        let intent_bytes = down_intents
            .capacity()
            .checked_mul(std::mem::size_of::<DownIntentTicks>())
            .ok_or_else(|| "prepared intent capacity overflow".to_string())?;
        let unmatched_up_bytes = unmatched_up_metadata
            .capacity()
            .checked_mul(std::mem::size_of::<PreparedUnmatchedUpMetadata>())
            .ok_or_else(|| "prepared unmatched-Up capacity overflow".to_string())?;
        let plan_capacity_bytes = chord_bytes
            .checked_add(intent_bytes)
            .and_then(|bytes| bytes.checked_add(unmatched_up_bytes))
            .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Self>()))
            .ok_or_else(|| "prepared plan capacity overflow".to_string())?;
        let overlapping_peak_capacity_bytes = caller_retained_capacity_bytes
            .checked_add(schedule_bytes)
            .and_then(|bytes| bytes.checked_add(plan_capacity_bytes))
            .ok_or_else(|| "prepared startup capacity overflow".to_string())?
            .max(schedule.compile_memory.phase_peak_capacity_bytes);
        if overlapping_peak_capacity_bytes > crate::model::MAX_RETAINED_DISPATCH_BYTES {
            return Err(format!(
                "prepared playback capacity ({overlapping_peak_capacity_bytes} bytes) exceeds the {} byte retained-dispatch budget",
                crate::model::MAX_RETAINED_DISPATCH_BYTES
            ));
        }
        let total_us = schedule
            .batches
            .last()
            .map_or(0, |batch| batch.scheduled_us);
        let memory_metrics = PreparedDispatchMemoryMetrics {
            chord_count,
            chord_capacity: down_chords.capacity(),
            intent_count,
            intent_capacity: down_intents.capacity(),
            unmatched_up_count,
            unmatched_up_capacity: unmatched_up_metadata.capacity(),
            plan_capacity_bytes,
            overlapping_peak_capacity_bytes,
        };
        Ok(Self {
            down_chords,
            down_intents,
            schedule_end_offset_ticks: qpc_to_ticks(schedule.schedule_end_offset_us)
                .ok_or_else(|| "schedule end offset timestamp conversion overflow".to_string())?,
            schedule_end_offset_us: schedule.schedule_end_offset_us,
            key_registry: schedule.key_registry.clone(),
            total_generations: schedule.generation_count,
            qpc_frequency_hz,
            min_hold_us,
            min_release_gap_us,
            min_hold_ticks,
            min_release_gap_ticks,
            total_us,
            packet_count: schedule.packets.len(),
            has_down: !schedule.down_chords.is_empty(),
            unmatched_up_metadata,
            memory_metrics,
        })
    }

    pub fn is_compatible(
        &self,
        qpc_frequency_hz: NonZeroU64,
        min_hold_us: u64,
        min_release_gap_us: u64,
    ) -> bool {
        self.qpc_frequency_hz == qpc_frequency_hz
            && self.min_hold_us == min_hold_us
            && self.min_release_gap_us == min_release_gap_us
    }

    pub fn qpc_frequency_hz(&self) -> NonZeroU64 {
        self.qpc_frequency_hz
    }

    pub fn min_hold_us(&self) -> u64 {
        self.min_hold_us
    }

    pub fn min_release_gap_us(&self) -> u64 {
        self.min_release_gap_us
    }

    pub fn total_us(&self) -> u64 {
        self.total_us
    }

    pub fn generation_count(&self) -> u64 {
        self.total_generations
    }

    pub fn packet_count(&self) -> usize {
        self.packet_count
    }

    pub fn has_down(&self) -> bool {
        self.has_down
    }

    pub fn unmatched_up_metadata(&self) -> &[PreparedUnmatchedUpMetadata] {
        &self.unmatched_up_metadata
    }

    pub fn memory_metrics(&self) -> PreparedDispatchMemoryMetrics {
        self.memory_metrics
    }

    pub fn schedule_end_offset_us(&self) -> u64 {
        self.schedule_end_offset_us
    }
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

/// Compact timing data needed to stage an Up packet when per-slot observer
/// attribution is disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpSlotTimingSummary {
    pub slot_count: u8,
    pub first_source_action_index: u32,
    pub authored_target_qpc: QpcTicks,
    pub musical_up_not_before_qpc: QpcTicks,
    pub hold_floor_mask: u16,
}

impl DispatchExecutor {
    /// Create fresh mutable session state from a plan prepared before Arm.
    /// The authored arrays are moved without rebuilding or reallocating.
    pub fn from_prepared_schedule(plan: PreparedDispatchSchedule) -> Self {
        let PreparedDispatchSchedule {
            down_chords,
            down_intents,
            schedule_end_offset_ticks,
            schedule_end_offset_us,
            key_registry,
            total_generations,
            min_hold_ticks,
            min_release_gap_ticks,
            ..
        } = plan;
        Self {
            down_chords,
            down_intents,
            schedule_end_offset_ticks,
            schedule_end_offset_us,
            key_registry,
            chord_cursor: 0,
            slots: [MusicalSlot::default(); MAX_KEYS],
            resume_barrier_qpc: None,
            min_hold_ticks,
            min_release_gap_ticks,
            counters: GenerationCounters::default(),
            total_generations,
            activated_generations: 0,
        }
    }

    /// Construct a new 15-slot executor from a validated `RuntimeSchedule`.
    pub fn new<F, E>(
        schedule: &RuntimeSchedule,
        min_hold_ticks: DurationTicks,
        min_release_gap_ticks: DurationTicks,
        us_to_ticks: F,
    ) -> Result<Self, String>
    where
        F: FnMut(u64) -> Result<TimelineTicks, E>,
        E: std::fmt::Display,
    {
        Self::new_with_cancellation(
            schedule,
            min_hold_ticks,
            min_release_gap_ticks,
            us_to_ticks,
            || Ok(()),
        )
    }

    /// Construct a new 15-slot executor with bounded memory admission and periodic cancellation checkpoints.
    pub fn new_with_cancellation<F, E, C>(
        schedule: &RuntimeSchedule,
        min_hold_ticks: DurationTicks,
        min_release_gap_ticks: DurationTicks,
        mut us_to_ticks: F,
        mut checkpoint: C,
    ) -> Result<Self, String>
    where
        F: FnMut(u64) -> Result<TimelineTicks, E>,
        E: std::fmt::Display,
        C: FnMut() -> Result<(), String>,
    {
        let chord_count = schedule.down_chords.len();
        let mut intent_count = 0usize;
        for (idx, chord) in schedule.down_chords.iter().enumerate() {
            if idx != 0 && idx % EXECUTOR_BUILD_CHECKPOINT_INTERVAL == 0 {
                checkpoint()?;
            }
            intent_count = intent_count
                .checked_add(chord.intents.len())
                .ok_or_else(|| "executor Down intent count overflow".to_string())?;
        }
        checked_executor_storage_bytes(chord_count, intent_count)?;

        checkpoint()?;

        let mut down_chords = Vec::new();
        reserve_executor_chords(&mut down_chords, chord_count)?;
        let mut down_intents = Vec::new();
        reserve_executor_intents(&mut down_intents, intent_count)?;

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
            let intent_start = down_intents.len();
            for intent in &chord.intents {
                let paired_up_offset_ticks =
                    us_to_ticks(intent.paired_up_offset_us).map_err(|e| {
                        format!(
                            "paired Up offset timestamp conversion failed at {}us: {e}",
                            intent.paired_up_offset_us
                        )
                    })?;
                down_intents.push(DownIntentTicks {
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
            let intent_count = down_intents
                .len()
                .checked_sub(intent_start)
                .ok_or_else(|| "executor Down intent range underflow".to_string())?;
            let range_end = intent_start
                .checked_add(intent_count)
                .ok_or_else(|| "executor Down intent range overflow".to_string())?;
            if range_end != down_intents.len() || range_end > down_intents.capacity() {
                return Err("executor Down intent range exceeds reserved arena".to_string());
            }
            down_chords.push(DownChordHeaderTicks {
                chord_index: chord.chord_index,
                scheduled_ticks,
                scheduled_us: chord.scheduled_us,
                down_mask: chord.down_mask,
                source_action_index: chord.source_action_index,
                reason_id: chord.reason_id,
                intent_start,
                intent_count,
            });
        }

        if down_intents.len() != intent_count {
            return Err("executor Down intent count changed during compilation".to_string());
        }
        checked_executor_storage_bytes(down_chords.capacity(), down_intents.capacity())?;

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
            down_intents,
            schedule_end_offset_ticks,
            schedule_end_offset_us: schedule.schedule_end_offset_us,
            key_registry: schedule.key_registry.clone(),
            chord_cursor: 0,
            slots: [MusicalSlot::default(); MAX_KEYS],
            resume_barrier_qpc: None,
            min_hold_ticks,
            min_release_gap_ticks,
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
        let chord_header_heap_bytes = self
            .down_chords
            .capacity()
            .checked_mul(std::mem::size_of::<DownChordHeaderTicks>())
            .ok_or_else(|| "executor chord-header capacity size overflow".to_string())?;
        let intent_heap_bytes = self
            .down_intents
            .capacity()
            .checked_mul(std::mem::size_of::<DownIntentTicks>())
            .ok_or_else(|| "executor intent-arena capacity size overflow".to_string())?;
        let chord_heap_bytes = chord_header_heap_bytes
            .checked_add(intent_heap_bytes)
            .ok_or_else(|| "executor vector capacity size overflow".to_string())?;
        checked_executor_storage_bytes(self.down_chords.capacity(), self.down_intents.capacity())?;
        let struct_bytes = std::mem::size_of::<Self>();
        let total_bytes = chord_heap_bytes
            .checked_add(struct_bytes)
            .ok_or_else(|| "executor total memory size overflow".to_string())?;
        Ok(ExecutorMemoryMetrics {
            chord_count: self.down_chords.len(),
            chord_capacity: self.down_chords.capacity(),
            intent_count: self.down_intents.len(),
            intent_capacity: self.down_intents.capacity(),
            chord_header_heap_bytes,
            intent_heap_bytes,
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

    /// Summarize Up timing without constructing or copying per-slot observer
    /// records. The result preserves the Up-only packet window and source
    /// attribution needed by the worker when a mixed packet is replanned.
    pub fn up_slot_timing_summary(
        &self,
        up_mask: u16,
        epoch_qpc: QpcTicks,
    ) -> Result<Option<UpSlotTimingSummary>, TimeArithmeticError> {
        let mut remaining = up_mask;
        let mut slot_count = 0u8;
        let mut first_source_action_index = None;
        let mut authored_target_qpc = None;
        let mut musical_up_not_before_qpc = None;
        let mut hold_floor_mask = 0u16;

        while remaining != 0 {
            let slot_index = remaining.trailing_zeros() as usize;
            let slot_mask = 1u16 << slot_index;
            remaining &= !slot_mask;

            let Some(slot) = self.slots.get(slot_index) else {
                continue;
            };
            if slot.active_generation.is_none() {
                continue;
            }

            let authored_up_qpc = epoch_qpc.checked_add_duration(DurationTicks::from_raw(
                slot.authored_up_offset_ticks.as_u64(),
            ))?;
            let release_not_before_qpc = authored_up_qpc.max(slot.release_not_before_qpc);
            slot_count += 1;
            first_source_action_index.get_or_insert(slot.paired_up_source_action_index);
            authored_target_qpc = Some(
                authored_target_qpc.map_or(authored_up_qpc, |current: QpcTicks| {
                    current.min(authored_up_qpc)
                }),
            );
            musical_up_not_before_qpc = Some(
                musical_up_not_before_qpc.map_or(release_not_before_qpc, |current: QpcTicks| {
                    current.max(release_not_before_qpc)
                }),
            );
            if slot.release_not_before_qpc > authored_up_qpc {
                hold_floor_mask |= slot_mask;
            }
        }

        Ok(authored_target_qpc.zip(musical_up_not_before_qpc).map(
            |(authored_target_qpc, musical_up_not_before_qpc)| UpSlotTimingSummary {
                slot_count,
                first_source_action_index: first_source_action_index.unwrap_or_default(),
                authored_target_qpc,
                musical_up_not_before_qpc,
                hold_floor_mask,
            },
        ))
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
    pub fn current_chord(&self) -> Option<DownChordView<'_>> {
        self.down_chords
            .get(self.chord_cursor)
            .map(|chord| self.chord_view(chord))
    }

    fn chord_view(&self, chord: &DownChordHeaderTicks) -> DownChordView<'_> {
        let intent_end = chord
            .intent_start
            .checked_add(chord.intent_count)
            .expect("executor chord range was checked at construction");
        let intents = self
            .down_intents
            .get(chord.intent_start..intent_end)
            .expect("executor chord range is contained in its intent arena");
        DownChordView {
            chord_index: chord.chord_index,
            scheduled_ticks: chord.scheduled_ticks,
            scheduled_us: chord.scheduled_us,
            down_mask: chord.down_mask,
            source_action_index: chord.source_action_index,
            reason_id: chord.reason_id,
            intents,
        }
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
        #[cfg(any(test, feature = "test-support"))]
        SCHEDULER_EVALUATION_COUNT.with(|count| count.set(count.get().saturating_add(1)));
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

        let chord_opt = self.current_chord();
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

            for intent in chord.intents {
                let k = intent.key_slot as usize;
                let slot = &self.slots[k];
                if slot.active_generation.is_some() {
                    chord_eligible = false;
                    let authored_up_qpc = epoch_qpc.checked_add_duration(
                        DurationTicks::from_raw(slot.authored_up_offset_ticks.as_u64()),
                    )?;
                    let key_release_due = authored_up_qpc.max(slot.release_not_before_qpc);
                    let key_next_down_min =
                        key_release_due.checked_add_duration(self.min_release_gap_ticks)?;
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
                    .checked_add_duration(self.min_release_gap_ticks)
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
                .copied()
                .ok_or_else(|| "confirmed Down chord when chord cursor is exhausted".to_string())?;
            if chord.down_mask != confirmed_down_mask {
                return Err(format!(
                    "confirmed Down mask 0x{:04x} does not match chord mask 0x{:04x}",
                    confirmed_down_mask, chord.down_mask
                ));
            }
            for intent in self.chord_view(&chord).intents {
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
        let activated_count = chord.map_or(0, |chord| chord.intent_count as u64);
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
            let intent_start = chord.intent_start;
            let intent_end = intent_start
                .checked_add(chord.intent_count)
                .ok_or_else(|| "executor Down intent range overflow at commit".to_string())?;
            let chord_intents = self
                .down_intents
                .get(intent_start..intent_end)
                .ok_or_else(|| "executor Down intent range invalid at commit".to_string())?;
            for intent in chord_intents {
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
    /// resume barrier to `completion_ticks + G`.
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
            .checked_add_duration(self.min_release_gap_ticks)
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

    /// Consume the current authored Down chord after an attempted packet was
    /// reconciled by verified safety cleanup. The attempted onset is cancelled
    /// exactly once; the completion timestamp belongs to cleanup and is already
    /// represented by the resume barrier installed by `on_verified_cleanup_at`.
    /// No sender completion is inferred for the attempted musical packet.
    pub fn consume_attempted_down_after_verified_cleanup(
        &mut self,
        source_action_index: u32,
        down_mask: u16,
    ) -> Result<(), String> {
        if self.active_mask() != 0 {
            return Err(
                "attempted Down cannot be consumed before active slots are cleaned up".into(),
            );
        }
        if self.resume_barrier_qpc.is_none() {
            return Err(
                "attempted Down cannot be consumed without a verified cleanup barrier".into(),
            );
        }
        self.checked_attempted_down_cursor_transition(
            source_action_index,
            down_mask,
            "attempted Down",
        )
    }

    /// Consume a Down-only onset after one internally consistent raw-zero
    /// transport result. No completion QPC or cleanup barrier is created:
    /// the packet was attempted once, while unrelated active slots and their
    /// existing H/G floors remain live.
    pub fn consume_attempted_down_after_proven_raw_zero(
        &mut self,
        source_action_index: u32,
        down_mask: u16,
    ) -> Result<(), String> {
        if down_mask == 0 || self.active_mask() & down_mask != 0 {
            return Err("proven-zero Down overlaps an active musical slot".into());
        }
        self.checked_attempted_down_cursor_transition(
            source_action_index,
            down_mask,
            "proven-zero Down",
        )
    }

    fn checked_attempted_down_cursor_transition(
        &mut self,
        source_action_index: u32,
        down_mask: u16,
        context: &'static str,
    ) -> Result<(), String> {
        let chord = self
            .current_chord()
            .ok_or_else(|| format!("{context} chord cursor is exhausted"))?;
        if chord.source_action_index != source_action_index || chord.down_mask != down_mask {
            return Err(format!(
                "{context} identity does not match current chord: source_action={} down_mask={:#06x}, current_source_action={} current_down_mask={:#06x}",
                source_action_index, down_mask, chord.source_action_index, chord.down_mask,
            ));
        }
        let next_cancelled = self
            .counters
            .cancelled
            .checked_add(chord.intents.len() as u64)
            .ok_or_else(|| format!("cancelled counter overflow while consuming {context}"))?;
        let next_cursor = self
            .chord_cursor
            .checked_add(1)
            .ok_or_else(|| format!("Down chord cursor overflow while consuming {context}"))?;
        self.counters.cancelled = next_cancelled;
        self.chord_cursor = next_cursor;
        Ok(())
    }

    /// Cancel every remaining generation after the terminal cleanup attempt.
    /// Scheduled generations are included because terminal shutdown cannot resume.
    pub fn cancel_remaining_generations_for_terminal_shutdown(&mut self) -> Result<u64, String> {
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
mod scale_tests;
#[cfg(test)]
mod tests;
