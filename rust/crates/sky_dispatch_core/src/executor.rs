//! Bounded 15-slot completion-anchored pulse executor.
//!
//! Manages exact musical onset ordering, intact chord admission, independent
//! per-key completion-anchored release eligibility (`completion_qpc + H_ticks`),
//! Up-before-Down packet combining for disjoint masks, and checked generation
//! lifecycle accounting with zero heap allocation on healthy dispatch.

use smallvec::SmallVec;
use std::num::NonZeroU64;

use crate::accounting::{GenerationAccounting, GenerationCounters};
use crate::model::{
    ALL_KEYS_MASK, GenerationId, KeyRegistry, KeySlot, MAX_KEYS, MAX_RETAINED_DISPATCH_BYTES,
    OnsetPlan, generation_id_for,
};
use crate::time::{DurationTicks, QpcTicks, TimeArithmeticError, TimelineTicks};

mod memory;
pub use memory::MAX_EXECUTOR_BYTES;
pub(crate) use memory::checked_executor_storage_bytes;
use memory::reserve_executor_chords;

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

pub const EXECUTOR_BUILD_CHECKPOINT_INTERVAL: usize = 4096;

/// One immutable onset chord materialized in the QPC timeline domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedChord {
    pub chord_index: usize,
    pub onset_ticks: TimelineTicks,
    pub onset_us: u64,
    pub key_mask: u16,
    pub source_action_index: u32,
}

impl PreparedChord {
    #[inline]
    pub const fn key_count(self) -> usize {
        self.key_mask.count_ones() as usize
    }
}

/// Minimal per-slot diagnostic attribution retained while a musical key is active.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ActiveSlotOrigin {
    chord_index: usize,
    source_action_index: u32,
    down_onset_ticks: TimelineTicks,
}

/// Absolute-QPC decision made by the production scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchDecision {
    /// Ready physical packet to stage and send.
    Send {
        up_mask: u16,
        down_chord: Option<PreparedChord>,
        timing_window: DispatchTimingWindow,
    },
    /// No physical work ready; wait until the specified deadline.
    Wait { deadline_qpc: QpcTicks },
    /// All onset chords sent, all slots released, and timeline end offset reached.
    Finished,
}

/// Timeline-zero adapter retained for core unit fixtures that model an
/// epoch at QPC zero. Production dispatch uses [`DispatchDecision`].
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchAction {
    Send {
        up_mask: u16,
        down_chord: Option<PreparedChord>,
    },
    Wait {
        deadline_ticks: TimelineTicks,
    },
    Finished,
}

/// Scheduler-owned absolute QPC eligibility evidence for one physical packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchTimingWindow {
    /// Authored onset target (for Down) or diagnostic nominal release (`epoch + onset + H`, for Up).
    pub authored_target_qpc: QpcTicks,
    /// Authoritative completion-anchored release floor for due Ups (`max(completion + H)`).
    pub musical_up_not_before_qpc: QpcTicks,
    /// Authoritative Down eligibility floor (`epoch + onset`).
    pub down_not_before_qpc: QpcTicks,
    /// Combined packet eligibility floor (`max(musical_up_not_before_qpc, down_not_before_qpc)`).
    pub packet_not_before_qpc: QpcTicks,
    /// Bitmask of due Up slots whose completion-anchored floor exceeds their diagnostic nominal Up.
    pub hold_floor_mask: u16,
}

/// Measured memory footprint of the `DispatchExecutor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutorMemoryMetrics {
    pub chord_count: usize,
    pub chord_capacity: usize,
    pub chord_heap_bytes: usize,
    pub struct_bytes: usize,
    pub total_bytes: usize,
}

/// Capacity evidence for an immutable prepared dispatch plan and its
/// overlapping construction phase. Values describe allocated capacities,
/// not process RSS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedDispatchMemoryMetrics {
    pub chord_count: usize,
    pub chord_capacity: usize,
    pub generation_count: u64,
    pub plan_capacity_bytes: usize,
    pub overlapping_peak_capacity_bytes: usize,
}

/// Immutable, QPC-domain onset plan produced during Prepare.
/// Contains only the fixed chord array and metadata needed to initialize a
/// fresh session; cursor, active mask, deadlines, epoch, and transport state
/// are created separately by `DispatchExecutor::from_prepared_schedule`.
#[derive(Debug)]
pub struct PreparedDispatchSchedule {
    chords: Vec<PreparedChord>,
    schedule_end_offset_ticks: TimelineTicks,
    schedule_end_offset_us: u64,
    key_registry: KeyRegistry,
    total_generations: u64,
    qpc_frequency_hz: NonZeroU64,
    hold_us: u64,
    hold_ticks: DurationTicks,
    total_us: u64,
    has_down: bool,
    memory_metrics: PreparedDispatchMemoryMetrics,
}

/// Checked ceiling conversion from microseconds to QPC timeline ticks.
#[inline]
pub fn us_to_timeline_ticks_ceil(
    microseconds: u64,
    qpc_frequency_hz: NonZeroU64,
) -> Option<TimelineTicks> {
    let numerator = u128::from(microseconds)
        .checked_mul(u128::from(qpc_frequency_hz.get()))
        .and_then(|value| value.checked_add(999_999))?;
    u64::try_from(numerator / 1_000_000)
        .ok()
        .map(TimelineTicks::from_raw)
}

/// Checked ceiling conversion from microseconds to QPC duration ticks.
#[inline]
pub fn us_to_duration_ticks_ceil(
    microseconds: u64,
    qpc_frequency_hz: NonZeroU64,
) -> Option<DurationTicks> {
    us_to_timeline_ticks_ceil(microseconds, qpc_frequency_hz)
        .map(|ticks| DurationTicks::from_raw(ticks.as_u64()))
}

fn build_prepared_chords<C, F>(
    plan: &OnsetPlan,
    hold_ticks: DurationTicks,
    mut checkpoint: C,
    mut timestamp_ticks: F,
) -> Result<Vec<PreparedChord>, String>
where
    C: FnMut() -> Result<(), String>,
    F: FnMut(u64) -> Result<TimelineTicks, String>,
{
    let chord_count = plan.chords.len();
    checked_executor_storage_bytes(chord_count)?;
    checkpoint()?;

    let mut chords = Vec::new();
    reserve_executor_chords(&mut chords, chord_count)?;

    for (index, chord) in plan.chords.iter().enumerate() {
        if index.is_multiple_of(EXECUTOR_BUILD_CHECKPOINT_INTERVAL) {
            checkpoint()?;
        }
        if chord.key_mask == 0 || (chord.key_mask & !ALL_KEYS_MASK) != 0 {
            return Err(format!(
                "onset chord {} has invalid key mask {:#06x}",
                chord.chord_index, chord.key_mask
            ));
        }
        let onset_ticks = timestamp_ticks(chord.scheduled_us)?;
        // Prove onset_ticks + hold_ticks fits in TimelineTicks (checked arithmetic).
        onset_ticks
            .checked_add_duration(hold_ticks)
            .map_err(|error| format!("prepared hold-floor conversion overflow: {error}"))?;
        chords.push(PreparedChord {
            chord_index: chord.chord_index,
            onset_ticks,
            onset_us: chord.scheduled_us,
            key_mask: chord.key_mask,
            source_action_index: chord.source_action_index,
        });
    }
    Ok(chords)
}

impl PreparedDispatchSchedule {
    /// Materialize a validated `OnsetPlan` into the immutable QPC-relative
    /// representation used by a later session. Uses checked ceiling conversion
    /// identical to `QpcClock::timeline_from_us`.
    pub fn prepare(
        plan: &OnsetPlan,
        qpc_frequency_hz: NonZeroU64,
        caller_retained_capacity_bytes: usize,
    ) -> Result<Self, String> {
        Self::prepare_with_cancellation(
            plan,
            qpc_frequency_hz,
            caller_retained_capacity_bytes,
            || Ok(()),
        )
    }

    /// Cancellable form used by the desktop Prepare path and startup admission.
    pub fn prepare_with_cancellation(
        plan: &OnsetPlan,
        qpc_frequency_hz: NonZeroU64,
        caller_retained_capacity_bytes: usize,
        mut checkpoint: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        checkpoint()?;
        let hold_ticks = us_to_duration_ticks_ceil(plan.hold_us, qpc_frequency_hz)
            .ok_or_else(|| "prepared hold-floor conversion overflow".to_string())?;

        let chords = build_prepared_chords(plan, hold_ticks, &mut checkpoint, |microseconds| {
            us_to_timeline_ticks_ceil(microseconds, qpc_frequency_hz).ok_or_else(|| {
                format!("scheduled timestamp conversion failed for chord at {microseconds}us")
            })
        })?;
        checkpoint()?;

        let schedule_end_offset_ticks =
            us_to_timeline_ticks_ceil(plan.schedule_end_offset_us, qpc_frequency_hz)
                .ok_or_else(|| "schedule end offset timestamp conversion overflow".to_string())?;

        let plan_memory = plan.memory_metrics()?;
        let source_plan_bytes = plan_memory
            .heap_payload_bytes
            .checked_add(plan_memory.struct_bytes)
            .ok_or_else(|| "onset plan capacity overflow".to_string())?;
        let chord_bytes = chords
            .capacity()
            .checked_mul(std::mem::size_of::<PreparedChord>())
            .ok_or_else(|| "prepared chord capacity overflow".to_string())?;
        let plan_capacity_bytes = chord_bytes
            .checked_add(std::mem::size_of::<Self>())
            .ok_or_else(|| "prepared plan capacity overflow".to_string())?;
        let overlapping_peak_capacity_bytes = caller_retained_capacity_bytes
            .checked_add(source_plan_bytes)
            .and_then(|bytes| bytes.checked_add(plan_capacity_bytes))
            .ok_or_else(|| "prepared startup capacity overflow".to_string())?
            .max(plan.compile_memory.phase_peak_capacity_bytes);
        if overlapping_peak_capacity_bytes > MAX_RETAINED_DISPATCH_BYTES {
            return Err(format!(
                "prepared playback capacity ({overlapping_peak_capacity_bytes} bytes) exceeds the {} byte retained-dispatch budget",
                MAX_RETAINED_DISPATCH_BYTES
            ));
        }

        let memory_metrics = PreparedDispatchMemoryMetrics {
            chord_count: chords.len(),
            chord_capacity: chords.capacity(),
            generation_count: plan.generation_count,
            plan_capacity_bytes,
            overlapping_peak_capacity_bytes,
        };
        Ok(Self {
            has_down: !chords.is_empty(),
            chords,
            schedule_end_offset_ticks,
            schedule_end_offset_us: plan.schedule_end_offset_us,
            key_registry: plan.key_registry.clone(),
            total_generations: plan.generation_count,
            qpc_frequency_hz,
            hold_us: plan.hold_us,
            hold_ticks,
            total_us: plan.schedule_end_offset_us,
            memory_metrics,
        })
    }

    pub fn is_compatible(&self, qpc_frequency_hz: NonZeroU64, min_hold_us: u64) -> bool {
        self.qpc_frequency_hz == qpc_frequency_hz && self.hold_us == min_hold_us
    }

    pub fn qpc_frequency_hz(&self) -> NonZeroU64 {
        self.qpc_frequency_hz
    }

    pub fn min_hold_us(&self) -> u64 {
        self.hold_us
    }

    pub fn hold_ticks(&self) -> DurationTicks {
        self.hold_ticks
    }

    pub fn total_us(&self) -> u64 {
        self.total_us
    }

    pub fn generation_count(&self) -> u64 {
        self.total_generations
    }

    pub fn chord_count(&self) -> usize {
        self.chords.len()
    }

    pub fn has_down(&self) -> bool {
        self.has_down
    }

    pub fn memory_metrics(&self) -> PreparedDispatchMemoryMetrics {
        self.memory_metrics
    }

    pub fn schedule_end_offset_us(&self) -> u64 {
        self.schedule_end_offset_us
    }
}

/// Diagnostic timing and attribution for an active key slot being released.
/// `nominal_up_offset_ticks` is derived on demand from `down_onset_ticks + hold_ticks`
/// and is diagnostic only — it is never an execution gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UpSlotTiming {
    pub slot: u8,
    pub scan_code: u16,
    pub source_action_index: u32,
    pub nominal_up_offset_ticks: TimelineTicks,
    pub release_not_before_qpc: QpcTicks,
}

impl UpSlotTiming {
    pub const EMPTY: Self = Self {
        slot: 0xFF,
        scan_code: 0,
        source_action_index: 0,
        nominal_up_offset_ticks: TimelineTicks::ZERO,
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

/// Bounded 15-slot completion-anchored pulse executor.
#[derive(Debug, Clone)]
pub struct DispatchExecutor {
    chords: Vec<PreparedChord>,
    schedule_end_offset_ticks: TimelineTicks,
    schedule_end_offset_us: u64,
    key_registry: KeyRegistry,
    cursor: usize,
    active_mask: u16,
    release_not_before_qpc: [QpcTicks; MAX_KEYS],
    active_slot_origin: [ActiveSlotOrigin; MAX_KEYS],
    hold_ticks: DurationTicks,
    verified_cleanup_seen: bool,
    counters: GenerationCounters,
    total_generations: u64,
    activated_generations: u64,
}

impl DispatchExecutor {
    /// Bound the capacity-backed storage required to create an executor for a
    /// validated onset plan without allocating it.
    pub fn estimated_memory_bytes(plan: &OnsetPlan) -> Result<usize, String> {
        checked_executor_storage_bytes(plan.chords.len())?
            .checked_add(std::mem::size_of::<Self>())
            .ok_or_else(|| "executor total memory size overflow".to_string())
    }

    /// Create fresh mutable session state from a plan prepared before Arm.
    /// The chord vector is moved without rebuilding or reallocating.
    pub fn from_prepared_schedule(plan: PreparedDispatchSchedule) -> Self {
        let PreparedDispatchSchedule {
            chords,
            schedule_end_offset_ticks,
            schedule_end_offset_us,
            key_registry,
            total_generations,
            hold_ticks,
            ..
        } = plan;
        Self {
            chords,
            schedule_end_offset_ticks,
            schedule_end_offset_us,
            key_registry,
            cursor: 0,
            active_mask: 0,
            release_not_before_qpc: [QpcTicks::ZERO; MAX_KEYS],
            active_slot_origin: [ActiveSlotOrigin::default(); MAX_KEYS],
            hold_ticks,
            verified_cleanup_seen: false,
            counters: GenerationCounters::default(),
            total_generations,
            activated_generations: 0,
        }
    }

    /// Construct a new 15-slot executor directly from a validated `OnsetPlan`.
    pub fn new<F, E>(
        plan: &OnsetPlan,
        hold_ticks: DurationTicks,
        us_to_ticks: F,
    ) -> Result<Self, String>
    where
        F: FnMut(u64) -> Result<TimelineTicks, E>,
        E: std::fmt::Display,
    {
        Self::new_with_cancellation(plan, hold_ticks, us_to_ticks, || Ok(()))
    }

    /// Construct a new 15-slot executor with bounded memory admission and
    /// periodic cancellation checkpoints.
    pub fn new_with_cancellation<F, E, C>(
        plan: &OnsetPlan,
        hold_ticks: DurationTicks,
        mut us_to_ticks: F,
        mut checkpoint: C,
    ) -> Result<Self, String>
    where
        F: FnMut(u64) -> Result<TimelineTicks, E>,
        E: std::fmt::Display,
        C: FnMut() -> Result<(), String>,
    {
        let chords = build_prepared_chords(plan, hold_ticks, &mut checkpoint, |microseconds| {
            us_to_ticks(microseconds).map_err(|error| {
                format!(
                    "scheduled timestamp conversion failed for chord at {microseconds}us: {error}"
                )
            })
        })?;
        checked_executor_storage_bytes(chords.capacity())?;
        checkpoint()?;

        let schedule_end_offset_ticks =
            us_to_ticks(plan.schedule_end_offset_us).map_err(|error| {
                format!(
                    "schedule end offset timestamp conversion failed at {}us: {error}",
                    plan.schedule_end_offset_us
                )
            })?;

        Ok(Self {
            chords,
            schedule_end_offset_ticks,
            schedule_end_offset_us: plan.schedule_end_offset_us,
            key_registry: plan.key_registry.clone(),
            cursor: 0,
            active_mask: 0,
            release_not_before_qpc: [QpcTicks::ZERO; MAX_KEYS],
            active_slot_origin: [ActiveSlotOrigin::default(); MAX_KEYS],
            hold_ticks,
            verified_cleanup_seen: false,
            counters: GenerationCounters::default(),
            total_generations: plan.generation_count,
            activated_generations: 0,
        })
    }

    /// Return measured memory metrics for this executor instance.
    pub fn memory_metrics(&self) -> ExecutorMemoryMetrics {
        self.try_memory_metrics()
            .expect("allocated executor memory must fit its checked storage budget")
    }

    pub fn try_memory_metrics(&self) -> Result<ExecutorMemoryMetrics, String> {
        let chord_heap_bytes = checked_executor_storage_bytes(self.chords.capacity())?;
        let struct_bytes = std::mem::size_of::<Self>();
        let total_bytes = chord_heap_bytes
            .checked_add(struct_bytes)
            .ok_or_else(|| "executor total memory size overflow".to_string())?;
        Ok(ExecutorMemoryMetrics {
            chord_count: self.chords.len(),
            chord_capacity: self.chords.capacity(),
            chord_heap_bytes,
            struct_bytes,
            total_bytes,
        })
    }

    #[inline]
    fn nominal_up_offset_ticks(&self, slot: usize) -> Result<TimelineTicks, TimeArithmeticError> {
        self.active_slot_origin[slot]
            .down_onset_ticks
            .checked_add_duration(self.hold_ticks)
    }

    /// Extract per-slot diagnostic Up metadata for the keys active in `up_mask`.
    pub fn up_slot_timings(&self, up_mask: u16) -> SmallVec<[UpSlotTiming; MAX_KEYS]> {
        let mut timings = SmallVec::new();
        let mut remaining = up_mask & self.active_mask;
        while remaining != 0 {
            let slot_idx = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            let scan_code = self.key_registry.scan_code_for(slot_idx as u8).unwrap_or(0);
            let origin = self.active_slot_origin[slot_idx];
            let nominal_up_offset_ticks = origin
                .down_onset_ticks
                .saturating_add_duration(self.hold_ticks);
            timings.push(UpSlotTiming {
                slot: slot_idx as u8,
                scan_code,
                source_action_index: origin.source_action_index,
                nominal_up_offset_ticks,
                release_not_before_qpc: self.release_not_before_qpc[slot_idx],
            });
        }
        timings
    }

    /// Summarize Up timing without constructing per-slot observer records.
    pub fn up_slot_timing_summary(
        &self,
        up_mask: u16,
        epoch_qpc: QpcTicks,
    ) -> Result<Option<UpSlotTimingSummary>, TimeArithmeticError> {
        let mut remaining = up_mask & self.active_mask;
        let mut slot_count = 0u8;
        let mut first_source_action_index = None;
        let mut authored_target_qpc = None;
        let mut musical_up_not_before_qpc = None;
        let mut hold_floor_mask = 0u16;

        while remaining != 0 {
            let slot_idx = remaining.trailing_zeros() as usize;
            let slot_bit = 1u16 << slot_idx;
            remaining &= !slot_bit;

            let nominal_up_offset = self.nominal_up_offset_ticks(slot_idx)?;
            let nominal_up_qpc = epoch_qpc
                .checked_add_duration(DurationTicks::from_raw(nominal_up_offset.as_u64()))?;
            let release_not_before_qpc = self.release_not_before_qpc[slot_idx];
            slot_count += 1;
            first_source_action_index
                .get_or_insert(self.active_slot_origin[slot_idx].source_action_index);
            authored_target_qpc = Some(
                authored_target_qpc.map_or(nominal_up_qpc, |current: QpcTicks| {
                    current.min(nominal_up_qpc)
                }),
            );
            musical_up_not_before_qpc = Some(
                musical_up_not_before_qpc.map_or(release_not_before_qpc, |current: QpcTicks| {
                    current.max(release_not_before_qpc)
                }),
            );
            if release_not_before_qpc > nominal_up_qpc {
                hold_floor_mask |= slot_bit;
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

    /// Return the earliest completion-anchored release deadline among every
    /// active slot, including deadlines that are already due.
    pub fn earliest_active_up_due_qpc(&self) -> Option<QpcTicks> {
        let mut remaining = self.active_mask;
        let mut earliest: Option<QpcTicks> = None;
        while remaining != 0 {
            let slot_idx = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            let due = self.release_not_before_qpc[slot_idx];
            earliest = Some(earliest.map_or(due, |current| current.min(due)));
        }
        earliest
    }

    #[inline]
    pub fn chord_cursor(&self) -> usize {
        self.cursor
    }

    #[inline]
    pub fn down_chord_count(&self) -> usize {
        self.chords.len()
    }

    #[inline]
    pub fn total_chords(&self) -> usize {
        self.chords.len()
    }

    #[inline]
    pub fn remaining_chords(&self) -> usize {
        self.chords.len().saturating_sub(self.cursor)
    }

    #[inline]
    pub fn key_registry(&self) -> &KeyRegistry {
        &self.key_registry
    }

    #[inline]
    pub fn hold_ticks(&self) -> DurationTicks {
        self.hold_ticks
    }

    #[inline]
    pub fn active_mask(&self) -> u16 {
        self.active_mask
    }

    /// Return the completion-anchored release deadline for an active slot.
    #[inline]
    pub fn slot_release_not_before_qpc(&self, slot: KeySlot) -> Option<QpcTicks> {
        let idx = usize::from(slot);
        if idx >= MAX_KEYS || (self.active_mask & (1u16 << idx)) == 0 {
            return None;
        }
        Some(self.release_not_before_qpc[idx])
    }

    /// Return the derived stable generation identity for an active slot.
    #[inline]
    pub fn active_generation_for_slot(&self, slot: KeySlot) -> Option<GenerationId> {
        let idx = usize::from(slot);
        if idx >= MAX_KEYS || (self.active_mask & (1u16 << idx)) == 0 {
            return None;
        }
        generation_id_for(self.active_slot_origin[idx].chord_index, slot)
    }

    /// Whether there is any pending or active physical work (chords remaining
    /// or active key slots held).
    #[inline]
    pub fn has_physical_work(&self) -> bool {
        self.remaining_chords() > 0 || self.active_mask != 0
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
    pub fn current_chord(&self) -> Option<PreparedChord> {
        self.chords.get(self.cursor).copied()
    }

    /// Evaluate the current state at an absolute QPC time and decide the next action.
    ///
    /// - Musical Downs are eligible at `epoch_qpc + onset_ticks` only when ALL
    ///   slots in the next chord are currently idle (no early send, intact chord).
    /// - Active Ups are independently eligible at `release_not_before_qpc[slot]`
    ///   (`Down completion QPC + hold_ticks`), with no authored Up floor.
    /// - Disjoint due Ups and next Down combine into one packet (Up before Down).
    /// - If the next chord is blocked by active slot(s), its lower-bound wake is
    ///   `max(authored_down_qpc, latest blocking release)` to avoid spinning on
    ///   an already-due blocked Down.
    pub fn evaluate_at(
        &self,
        now_qpc: QpcTicks,
        epoch_qpc: QpcTicks,
    ) -> Result<DispatchDecision, TimeArithmeticError> {
        #[cfg(any(test, feature = "test-support"))]
        SCHEDULER_EVALUATION_COUNT.with(|count| count.set(count.get().saturating_add(1)));

        let mut up_ready_mask = 0u16;
        let mut earliest_future_up_due_qpc: Option<QpcTicks> = None;
        let mut up_packet_not_before_qpc: Option<QpcTicks> = None;
        let mut up_authored_target_qpc: Option<QpcTicks> = None;
        let mut up_hold_floor_mask = 0u16;

        let mut active = self.active_mask;
        while active != 0 {
            let slot_idx = active.trailing_zeros() as usize;
            let slot_bit = 1u16 << slot_idx;
            active &= !slot_bit;

            let due = self.release_not_before_qpc[slot_idx];
            if now_qpc >= due {
                up_ready_mask |= slot_bit;
                up_packet_not_before_qpc =
                    Some(up_packet_not_before_qpc.map_or(due, |value| value.max(due)));
                let nominal_up_offset = self.nominal_up_offset_ticks(slot_idx)?;
                let nominal_up_qpc = epoch_qpc
                    .checked_add_duration(DurationTicks::from_raw(nominal_up_offset.as_u64()))?;
                up_authored_target_qpc = Some(
                    up_authored_target_qpc
                        .map_or(nominal_up_qpc, |value| value.min(nominal_up_qpc)),
                );
                if due > nominal_up_qpc {
                    up_hold_floor_mask |= slot_bit;
                }
            } else {
                earliest_future_up_due_qpc =
                    Some(earliest_future_up_due_qpc.map_or(due, |value| value.min(due)));
            }
        }

        let chord_opt = self.current_chord();
        let mut chord_eligible = false;
        let mut chord_earliest_due_qpc = QpcTicks::ZERO;
        let mut chord_authored_target_qpc = None;

        if let Some(chord) = chord_opt {
            let authored_down_qpc = epoch_qpc
                .checked_add_duration(DurationTicks::from_raw(chord.onset_ticks.as_u64()))?;
            chord_authored_target_qpc = Some(authored_down_qpc);
            let mut blocking = chord.key_mask & self.active_mask;
            if blocking == 0 {
                chord_eligible = now_qpc >= authored_down_qpc;
                chord_earliest_due_qpc = authored_down_qpc;
            } else {
                chord_eligible = false;
                let mut bound = authored_down_qpc;
                while blocking != 0 {
                    let slot_idx = blocking.trailing_zeros() as usize;
                    blocking &= blocking - 1;
                    bound = bound.max(self.release_not_before_qpc[slot_idx]);
                }
                chord_earliest_due_qpc = bound;
            }
        }

        let up_ready_qpc = up_packet_not_before_qpc.filter(|_| up_ready_mask != 0);
        let up_authored_qpc = up_authored_target_qpc.filter(|_| up_ready_mask != 0);
        let down_ready_qpc = chord_eligible.then_some(chord_earliest_due_qpc);
        let down_authored_qpc = chord_authored_target_qpc.filter(|_| chord_eligible);

        if up_ready_mask != 0 || chord_eligible {
            let authored_target_qpc = match (up_authored_qpc, down_authored_qpc) {
                (Some(up), Some(down)) => up.min(down),
                (Some(up), None) => up,
                (None, Some(down)) => down,
                (None, None) => unreachable!("a send decision has an Up or Down direction"),
            };
            let musical_up_not_before_qpc = up_ready_qpc.unwrap_or(authored_target_qpc);
            let down_not_before_qpc = down_ready_qpc.unwrap_or(authored_target_qpc);
            return Ok(DispatchDecision::Send {
                up_mask: up_ready_mask,
                down_chord: chord_eligible.then_some(chord_opt).flatten(),
                timing_window: DispatchTimingWindow {
                    authored_target_qpc,
                    musical_up_not_before_qpc,
                    down_not_before_qpc,
                    packet_not_before_qpc: musical_up_not_before_qpc.max(down_not_before_qpc),
                    hold_floor_mask: up_hold_floor_mask,
                },
            });
        }

        let mut earliest_wake = earliest_future_up_due_qpc;
        if chord_opt.is_some() {
            earliest_wake = Some(earliest_wake.map_or(chord_earliest_due_qpc, |earliest| {
                earliest.min(chord_earliest_due_qpc)
            }));
        }

        if let Some(wake) = earliest_wake {
            Ok(DispatchDecision::Wait { deadline_qpc: wake })
        } else {
            let schedule_end_qpc = epoch_qpc.checked_add_duration(DurationTicks::from_raw(
                self.schedule_end_offset_ticks.as_u64(),
            ))?;
            if now_qpc < schedule_end_qpc {
                Ok(DispatchDecision::Wait {
                    deadline_qpc: schedule_end_qpc,
                })
            } else {
                Ok(DispatchDecision::Finished)
            }
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn evaluate(&self, now_ticks: TimelineTicks) -> DispatchAction {
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

    /// Commit the result of a complete healthy `SendInput` transaction.
    /// All arithmetic and structural checks run before any state mutation.
    pub fn on_send_completed_at(
        &mut self,
        confirmed_up_mask: u16,
        confirmed_down_mask: u16,
        completion_qpc: QpcTicks,
    ) -> Result<(), String> {
        if confirmed_up_mask & confirmed_down_mask != 0 {
            return Err("confirmed packet has overlapping Up and Down masks".to_string());
        }
        if (confirmed_up_mask & !ALL_KEYS_MASK) != 0 || (confirmed_down_mask & !ALL_KEYS_MASK) != 0
        {
            return Err("confirmed packet contains out-of-range key slot".to_string());
        }
        let inactive_ups = confirmed_up_mask & !self.active_mask;
        if inactive_ups != 0 {
            let slot_idx = inactive_ups.trailing_zeros() as usize;
            return Err(format!("confirmed Up on inactive slot {slot_idx}"));
        }

        let new_up_floor = if confirmed_down_mask != 0 {
            Some(
                completion_qpc
                    .checked_add_duration(self.hold_ticks)
                    .map_err(|error| format!("Down completion H floor overflow: {error}"))?,
            )
        } else {
            None
        };

        let chord = if confirmed_down_mask != 0 {
            let chord =
                self.chords.get(self.cursor).copied().ok_or_else(|| {
                    "confirmed Down chord when chord cursor is exhausted".to_string()
                })?;
            if chord.key_mask != confirmed_down_mask {
                return Err(format!(
                    "confirmed Down mask {confirmed_down_mask:#06x} does not match chord mask {:#06x}",
                    chord.key_mask
                ));
            }
            let already_active = self.active_mask & confirmed_down_mask;
            if already_active != 0 {
                let slot_idx = already_active.trailing_zeros() as usize;
                return Err(format!(
                    "attempted to activate already active slot {slot_idx}"
                ));
            }
            Some(chord)
        } else {
            None
        };

        let next_released = self
            .counters
            .released
            .checked_add(u64::from(confirmed_up_mask.count_ones()))
            .ok_or("released counter overflow")?;
        let activated_count = chord.map_or(0, |chord| u64::from(chord.key_mask.count_ones()));
        let next_activated = self
            .activated_generations
            .checked_add(activated_count)
            .ok_or("activated counter overflow")?;
        if next_activated > self.total_generations {
            return Err("activated generation count exceeds compiled total".to_string());
        }
        let next_cursor = self
            .cursor
            .checked_add(usize::from(chord.is_some()))
            .ok_or("Down chord cursor overflow")?;

        // Commit atomically after all checks succeeded.
        let mut up_slots = confirmed_up_mask;
        while up_slots != 0 {
            let slot_idx = up_slots.trailing_zeros() as usize;
            up_slots &= up_slots - 1;
            self.release_not_before_qpc[slot_idx] = QpcTicks::ZERO;
            self.active_slot_origin[slot_idx] = ActiveSlotOrigin::default();
        }
        self.active_mask &= !confirmed_up_mask;

        if let Some(chord) = chord {
            let release_deadline = new_up_floor.expect("Down commit computed release floor");
            let mut down_slots = chord.key_mask;
            while down_slots != 0 {
                let slot_idx = down_slots.trailing_zeros() as usize;
                down_slots &= down_slots - 1;
                self.release_not_before_qpc[slot_idx] = release_deadline;
                self.active_slot_origin[slot_idx] = ActiveSlotOrigin {
                    chord_index: chord.chord_index,
                    source_action_index: chord.source_action_index,
                    down_onset_ticks: chord.onset_ticks,
                };
            }
            self.active_mask |= chord.key_mask;
        }

        self.counters.released = next_released;
        self.activated_generations = next_activated;
        self.cursor = next_cursor;
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
        let active_count = u64::from(self.active_mask.count_ones());
        self.counters
            .cancelled
            .checked_add(active_count)
            .ok_or_else(|| "cancelled counter overflow during resumable cleanup".to_string())?;
        Ok(())
    }

    /// Apply verified resumable cleanup.
    ///
    /// Clears active musical owners in all 15 slots without physical replay and
    /// marks cancelled generations in accounting.
    pub fn on_verified_cleanup(&mut self) -> Result<u64, String> {
        let active_count = u64::from(self.active_mask.count_ones());
        let next_cancelled = self
            .counters
            .cancelled
            .checked_add(active_count)
            .ok_or("cancelled counter overflow")?;
        self.active_mask = 0;
        self.release_not_before_qpc.fill(QpcTicks::ZERO);
        self.active_slot_origin.fill(ActiveSlotOrigin::default());
        self.counters.cancelled = next_cancelled;
        self.verified_cleanup_seen = true;
        Ok(active_count)
    }

    /// Consume the current onset chord after an attempted packet was reconciled
    /// by verified safety cleanup. The attempted onset is cancelled once and
    /// never replayed.
    pub fn consume_attempted_down_after_verified_cleanup(
        &mut self,
        source_action_index: u32,
        down_mask: u16,
    ) -> Result<(), String> {
        if self.active_mask != 0 {
            return Err(
                "attempted Down cannot be consumed before active slots are cleaned up".into(),
            );
        }
        if !self.verified_cleanup_seen {
            return Err("attempted Down cannot be consumed without verified cleanup".into());
        }
        self.checked_attempted_down_cursor_transition(
            source_action_index,
            down_mask,
            "attempted Down",
        )
    }

    /// Consume a Down-only onset after one internally consistent raw-zero
    /// transport result. No completion QPC or cleanup barrier is created:
    /// unrelated active slots keep their existing H eligibility.
    pub fn consume_attempted_down_after_proven_raw_zero(
        &mut self,
        source_action_index: u32,
        down_mask: u16,
    ) -> Result<(), String> {
        if down_mask == 0 || self.active_mask & down_mask != 0 {
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
        if chord.source_action_index != source_action_index || chord.key_mask != down_mask {
            return Err(format!(
                "{context} identity does not match current chord: source_action={} down_mask={:#06x}, current_source_action={} current_down_mask={:#06x}",
                source_action_index, down_mask, chord.source_action_index, chord.key_mask,
            ));
        }
        let next_cancelled = self
            .counters
            .cancelled
            .checked_add(u64::from(chord.key_mask.count_ones()))
            .ok_or_else(|| format!("cancelled counter overflow while consuming {context}"))?;
        let next_cursor = self
            .cursor
            .checked_add(1)
            .ok_or_else(|| format!("Down chord cursor overflow while consuming {context}"))?;
        self.counters.cancelled = next_cancelled;
        self.cursor = next_cursor;
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
        self.active_mask = 0;
        self.release_not_before_qpc.fill(QpcTicks::ZERO);
        self.active_slot_origin.fill(ActiveSlotOrigin::default());
        Ok(cancelled_count)
    }

    /// Check the bounded scheduler state and generation lifecycle partition.
    pub fn check_invariants(&self) -> Result<(), String> {
        if self.cursor > self.chords.len() {
            return Err("scheduler chord cursor exceeds the prepared chord count".into());
        }
        if (self.active_mask & !ALL_KEYS_MASK) != 0 {
            return Err("scheduler active mask contains out-of-range key slot".into());
        }
        let mut active = self.active_mask;
        while active != 0 {
            let slot_idx = active.trailing_zeros() as usize;
            active &= active - 1;
            if self.active_slot_origin[slot_idx].chord_index >= self.chords.len() {
                return Err(format!(
                    "scheduler slot {slot_idx} references an out-of-range chord"
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

    /// True when all chords have been dispatched, all slots are empty, and
    /// the musical timeline has reached `schedule_end_offset_ticks`.
    #[inline]
    pub fn is_finished(&self, now_ticks: TimelineTicks) -> bool {
        self.cursor >= self.chords.len()
            && self.active_mask == 0
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
        Ok(self.cursor >= self.chords.len() && self.active_mask == 0 && now_qpc >= end_qpc)
    }

    /// Snapshot current generation accounting.
    pub fn accounting(&self) -> GenerationAccounting {
        let active_count = u64::from(self.active_mask.count_ones());
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
        let active_ups = up_mask & self.active_mask;
        if active_ups == 0 {
            return None;
        }
        let slot = active_ups.trailing_zeros() as usize;
        Some(self.active_slot_origin[slot].source_action_index)
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
