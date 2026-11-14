//! Runtime dispatch coordinator managing generation status transitions and release eligibility.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;

use crate::model::*;
use crate::time::{DurationTicks, TimelineTicks};

mod memory;
pub use memory::DispatchMemoryMetrics;
use memory::{check_retained_budget, checked_storage_bytes, checked_storage_sum};

static NEXT_COORDINATOR_ID: AtomicU64 = AtomicU64::new(1);

type SplitIntentResult = (
    SmallVec<[RuntimeKeyIntent; MAX_KEYS]>,
    SmallVec<[RuntimeKeyIntent; MAX_KEYS]>,
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationStatus {
    Scheduled,
    Active,
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
            Self::Released => "released",
            Self::DroppedConflict => "dropped_conflict",
            Self::DroppedBackend => "dropped_backend",
            Self::DroppedExpired => "dropped_expired",
            Self::Cancelled => "cancelled",
        }
    }

    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Released
                | Self::DroppedConflict
                | Self::DroppedBackend
                | Self::DroppedExpired
                | Self::Cancelled
        )
    }

    fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Scheduled, Self::Active)
                | (Self::Scheduled, Self::DroppedConflict)
                | (Self::Scheduled, Self::DroppedExpired)
                | (Self::Scheduled, Self::DroppedBackend)
                | (Self::Scheduled, Self::Cancelled)
                | (Self::Active, Self::Released)
                | (Self::Active, Self::DroppedBackend)
                | (Self::Active, Self::Cancelled)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CoordinatorInvariantError {
    #[error(
        "generation {generation_id} is outside the prepared ledger of {generation_count} generations"
    )]
    UnknownGeneration {
        generation_id: GenerationId,
        generation_count: u64,
    },
    #[error(
        "invalid generation transition for {generation_id}: expected {expected:?}, actual {actual:?}, next {next:?}"
    )]
    UnexpectedTransition {
        generation_id: GenerationId,
        expected: GenerationStatus,
        actual: GenerationStatus,
        next: GenerationStatus,
    },
    #[error("illegal generation transition for {generation_id}: {from:?} -> {to:?}")]
    IllegalTransition {
        generation_id: GenerationId,
        from: GenerationStatus,
        to: GenerationStatus,
    },
    #[error("generation accounting invariant failed: {0}")]
    Accounting(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CoordinatorError {
    #[error("coordinator invariant failure: {0}")]
    Invariant(#[from] CoordinatorInvariantError),
    #[error("coordinator time arithmetic failure: {0}")]
    Time(#[from] crate::time::TimeArithmeticError),
    #[error("invalid batch index {index}")]
    InvalidBatchIndex { index: usize },
    #[error("runtime schedule validation failed: {0}")]
    Schedule(#[from] RuntimeScheduleError),
    #[error("prepared batch index {prepared} does not match coordinator cursor {cursor}")]
    PreparedBatchMismatch { prepared: usize, cursor: usize },
    #[error("prepared authored proof does not match the current stream entry or coordinator")]
    PreparedAuthoredProofMismatch,
    #[error("invalid key slot {slot}")]
    InvalidKeySlot { slot: KeySlot },
    #[error("generation count does not fit in usize")]
    GenerationCountOverflow,
    #[error("coordinator allocation failed: {0}")]
    Allocation(String),
    #[error("coordinator preparation was cancelled")]
    PreparationCancelled,
    #[error("time conversion failed: {0}")]
    TimeConversion(String),
    #[error(
        "physical deadline infeasible at authored tick {authored_ticks:?}: blocked_mask=0x{blocked_mask:04x}, latest_required_release={latest_required_release_ticks:?}"
    )]
    PhysicalDeadlineInfeasible {
        authored_ticks: TimelineTicks,
        blocked_mask: u16,
        latest_required_release_ticks: TimelineTicks,
    },
    #[error("pending release already exists for key slot {slot}")]
    PendingReleaseAlreadyRegistered { slot: KeySlot },
    #[error("pending release does not match active generation for key slot {slot}")]
    PendingReleaseOwnershipMismatch { slot: KeySlot },
}

fn next_coordinator_id() -> Result<u64, CoordinatorError> {
    NEXT_COORDINATOR_ID
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| {
            CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                "coordinator identity space exhausted".into(),
            ))
        })
}

pub fn physical_packet_kind(
    up_mask: u16,
    down_mask: u16,
) -> Result<PhysicalPacketKind, CoordinatorError> {
    match (up_mask != 0, down_mask != 0) {
        (true, false) => Ok(PhysicalPacketKind::UpOnly),
        (false, true) => Ok(PhysicalPacketKind::DownOnly),
        (true, true) => Ok(PhysicalPacketKind::Mixed),
        (false, false) => Err(CoordinatorError::Invariant(
            CoordinatorInvariantError::Accounting("compiled physical packet is empty".into()),
        )),
    }
}

/// Counter-only generation summary.
///
/// Active counts are derived from `active_mask` at query time; this struct
/// tracks only terminal generations.
/// No `HashMap` is allocated; all fields are plain `u64`.
///
/// Invariant: `scheduled + active + released + dropped_conflict
///            + dropped_backend + dropped_expired + cancelled
///            == total (generation_count)`
///
/// "scheduled" is implicit: `generation_count - (active + terminal_total)`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenerationCounters {
    pub released: u64,
    pub dropped_conflict: u64,
    pub dropped_backend: u64,
    pub dropped_expired: u64,
    pub cancelled: u64,
}

impl GenerationCounters {
    /// Sum of all terminal buckets.
    fn terminal_total_checked(&self) -> Option<u64> {
        self.released
            .checked_add(self.dropped_conflict)?
            .checked_add(self.dropped_backend)?
            .checked_add(self.dropped_expired)?
            .checked_add(self.cancelled)
    }

    pub fn terminal_total(&self) -> u64 {
        self.terminal_total_checked()
            .expect("terminal generation counters must remain checked")
    }

    fn increment(&mut self, status: GenerationStatus) -> Result<(), CoordinatorInvariantError> {
        let counter = match status {
            GenerationStatus::Released => &mut self.released,
            GenerationStatus::DroppedConflict => &mut self.dropped_conflict,
            GenerationStatus::DroppedBackend => &mut self.dropped_backend,
            GenerationStatus::DroppedExpired => &mut self.dropped_expired,
            GenerationStatus::Cancelled => &mut self.cancelled,
            // Non-terminal states are not tracked here; they are derived from masks.
            GenerationStatus::Scheduled | GenerationStatus::Active => return Ok(()),
        };
        *counter = counter.checked_add(1).ok_or_else(|| {
            CoordinatorInvariantError::Accounting(format!("{status:?} generation counter overflow"))
        })?;
        Ok(())
    }
}

/// Allocation-free generation lifecycle accounting derived from the
/// compiler-owned generation ledger and coordinator masks/counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenerationAccounting {
    pub total: u64,
    pub activated: u64,
    pub scheduled: u64,
    pub active: u64,
    pub released: u64,
    pub dropped_conflict: u64,
    pub dropped_backend: u64,
    pub dropped_expired: u64,
    pub cancelled: u64,
}

pub const ALL_GENERATION_STATUSES: [GenerationStatus; 7] = [
    GenerationStatus::Scheduled,
    GenerationStatus::Active,
    GenerationStatus::Released,
    GenerationStatus::DroppedConflict,
    GenerationStatus::DroppedBackend,
    GenerationStatus::DroppedExpired,
    GenerationStatus::Cancelled,
];

mod authored;
mod cleanup;
mod conflict;
mod ownership;
mod stale;
mod timeline;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveGeneration {
    pub generation_id: GenerationId,
    pub scan_code: u16,
    pub key_slot: KeySlot,
    pub source_action_index: u32,
    pub release_not_before_ticks: TimelineTicks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedBatch {
    pub index: usize,
    pub effective_scheduled_ticks: TimelineTicks,
    /// Packet metadata is carried alongside the legacy batch preparation so
    /// the worker can atomically dispatch all authored actions at one
    /// timestamp without maintaining a second cursor.
    pub packet_index: usize,
    pub packet_batch_count: usize,
    /// Every prepared authored batch is physical work. Stale unmatched-Up
    /// metadata is represented only by [`PreparedStalePacket`] and cannot
    /// enter physical preparation.
    pub packet_kind: PhysicalPacketKind,
}

/// Authored-frame classification performed before timed waiting.
///
/// The compiler packet remains an immutable authored frame, but its Up
/// intents are classified per key against the pre-admitted authored hold
/// floor and recovery ownership. Preparation never mutates coordinator state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedAuthoredFrame {
    pub first_batch_index: usize,
    pub packet_index: usize,
    pub packet_batch_count: usize,
    pub authored_ticks: TimelineTicks,
    pub immediate_up_mask: u16,
    pub deferred_up_mask: u16,
    pub down_mask: u16,
    pub stale_up_count: u8,
}

/// Test-support evidence emitted by the coordinator's authored preparation
/// operation. These counters are populated at the operation sites themselves;
/// they are not inferred later from slice lengths.
#[cfg(feature = "test-support")]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AuthoredPreparationEvidence {
    pub packet_header_reads: u64,
    pub expected_up_intents: u64,
    pub expected_down_intents: u64,
    pub up_intent_visits: u64,
    pub down_intent_visits: u64,
    pub registry_lookups: u64,
    pub secondary_batch_visits: u64,
    pub secondary_batch_visit_bound: u64,
    pub view_packet_calls: u64,
    pub commit_freeze_calls: u64,
}

/// One allocation-free logical preparation product. The coordinator builds
/// the frame classification, validated packet view, frozen commit token, and
/// source metadata from one authoritative packet traversal before the timed
/// wait.
#[derive(Debug, Clone)]
pub struct PreparedAuthoredPacket<'a> {
    pub frame: PreparedAuthoredFrame,
    pub packet: PacketView<'a>,
    pub commit: PreparedAuthoredCommit,
    pub batch_source_action_index: u32,
    #[cfg(feature = "test-support")]
    pub preparation_evidence: AuthoredPreparationEvidence,
}

/// One prepared authored Up intent. Immediate and deferred releases share one
/// bounded vector; the frame masks classify each entry at commit time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedUpIntent {
    pub intent: CompactIntent,
    pub source_action_index: u32,
    /// Compiler-owned location identity for this authored Up. The schedule
    /// entry is invalidated in place when a live generation is cancelled, but
    /// this frozen identity remains stable in prepared streams.
    pub identity: AuthoredUpIdentity,
}

/// Exact authored identity for a paired Up. A generation ID alone is not
/// sufficient to reconcile a cancellation: the key slot and immutable
/// compiler location must also match.
///
/// The canonical CompactIntent encodes generation and slot in eight bytes.
/// Compiler packet and intent locations are u32-indexed, so storing them as
/// u32 preserves full identity while keeping the frozen token to 16 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AuthoredUpIdentity {
    generation_slot: CompactIntent,
    packet_index: u32,
    intent_index: u32,
}

impl AuthoredUpIdentity {
    pub(crate) fn try_new(
        generation_id: GenerationId,
        key_slot: KeySlot,
        packet_index: usize,
        intent_index: usize,
    ) -> Result<Self, CoordinatorError> {
        if generation_id == NO_GENERATION_ID
            || generation_id > MAX_COMPACT_GENERATION_ID
            || usize::from(key_slot) >= MAX_KEYS
        {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "frozen authored Up identity has an invalid generation or slot".into(),
                ),
            ));
        }
        let packet_index = u32::try_from(packet_index).map_err(|_| {
            CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                "frozen authored Up packet index exceeds compiler u32 range".into(),
            ))
        })?;
        let intent_index = u32::try_from(intent_index).map_err(|_| {
            CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                "frozen authored Up intent index exceeds compiler u32 range".into(),
            ))
        })?;
        Ok(Self {
            generation_slot: CompactIntent::new(generation_id, key_slot),
            packet_index,
            intent_index,
        })
    }

    pub fn generation_id(self) -> GenerationId {
        self.generation_slot.generation_id()
    }

    pub fn key_slot(self) -> KeySlot {
        self.generation_slot.key_slot()
    }

    pub fn packet_index(self) -> usize {
        self.packet_index as usize
    }

    pub fn intent_index(self) -> usize {
        self.intent_index as usize
    }
}

/// Frozen Down evidence retains the generation, slot and full scan code.
/// The compiler caps actions below u32::MAX, reserved here for NO_GENERATION_ID.
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedDownIntent {
    generation: u32,
    scan_code: u16,
    key_slot: KeySlot,
}

impl PreparedDownIntent {
    pub(crate) fn try_new(intent: CompactIntent, scan_code: u16) -> Result<Self, CoordinatorError> {
        let key_slot = intent.key_slot();
        if usize::from(key_slot) >= MAX_KEYS {
            return Err(CoordinatorError::InvalidKeySlot { slot: key_slot });
        }
        let generation = match intent.generation_id() {
            NO_GENERATION_ID => u32::MAX,
            generation_id => u32::try_from(generation_id)
                .ok()
                .filter(|&generation| generation != u32::MAX)
                .ok_or_else(|| {
                    CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                        "frozen authored Down generation exceeds compiler u32 range".into(),
                    ))
                })?,
        };
        Ok(Self {
            generation,
            scan_code,
            key_slot,
        })
    }

    #[inline]
    pub fn generation_id(&self) -> GenerationId {
        if self.generation == u32::MAX {
            NO_GENERATION_ID
        } else {
            u64::from(self.generation)
        }
    }

    #[inline]
    pub fn key_slot(&self) -> KeySlot {
        self.key_slot
    }

    #[inline]
    pub fn scan_code(&self) -> u16 {
        self.scan_code
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAuthoredCommit {
    pub frame: PreparedAuthoredFrame,
    pub up_intents: SmallVec<[PreparedUpIntent; MAX_KEYS]>,
    pub down_intents: SmallVec<[PreparedDownIntent; MAX_KEYS]>,
    pub down_source_action_index: Option<u32>,
}

/// One-use proof that a frozen authored commit matched the coordinator before
/// a prepared physical wait. The private references bind the proof to that
/// exact stream entry; coordinator identity, state revision, and cursor are
/// checked when the successful send commits it.
#[derive(Debug)]
pub struct PreparedAuthoredCommitProof<'a> {
    pub(super) commit: &'a PreparedAuthoredCommit,
    pub(super) coordinator_id: u64,
    pub(super) state_revision: u64,
    pub(super) cursor: usize,
}

/// One authored release that is waiting for its own physical due boundary.
/// The table is bounded by the fifteen physical key slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingRelease {
    pub generation_id: GenerationId,
    pub key_slot: KeySlot,
    pub authored_release_ticks: TimelineTicks,
    pub due_ticks: TimelineTicks,
    pub source_action_index: u32,
}

/// One compiler packet containing only unmatched Up metadata.
///
/// Stale packets are coordinator metadata, not physical work.  Keeping a
/// dedicated preparation type prevents the worker from manufacturing a
/// physical path or deadline for a packet that will never reach SendInput.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedStalePacket {
    pub first_batch_index: usize,
    pub packet_index: usize,
    pub packet_batch_count: usize,
    pub effective_scheduled_ticks: TimelineTicks,
    pub source_action_index: u32,
    pub suppressed_intent_count: usize,
}

#[derive(Debug)]
pub struct RuntimeDispatchCoordinator {
    coordinator_id: u64,
    state_revision: u64,
    pub schedule: RuntimeSchedule,
    pub min_hold_us: u64,
    pub min_hold_ticks: DurationTicks,
    pub batch_scheduled_ticks: Box<[TimelineTicks]>,
    pub cursor: usize,
    active_by_slot: [Option<ActiveGeneration>; MAX_KEYS],
    pub active_mask: u16,
    /// Physical-key ownership mask. An active key remains blocked until its
    /// authored Up is committed in the same canonical packet lifecycle.
    blocked_mask: u16,
    /// Terminal generation counts.
    ///
    /// Active count is derived from `active_mask`, so it is not stored here.
    /// This eliminates the `HashMap<GenerationId, GenerationStatus>` from the
    /// hot dispatch path.
    counters: GenerationCounters,
    activated_generation_count: u64,
    generation_states: Box<[GenerationStatus]>,
    generation_count: u64,
    up_intent_locations: Box<[Option<(usize, usize)>]>,
    /// At most one unmatched resumable cancellation can exist per physical
    /// instrument slot. Tokens are minted only by the worker's verified
    /// cleanup path and consumed by the exact frozen authored Up.
    cancelled_up_by_slot: [Option<AuthoredUpIdentity>; MAX_KEYS],
    pending_release_by_slot: [Option<PendingRelease>; MAX_KEYS],
    pending_release_mask: u16,
    pub executor: Option<crate::executor::DispatchExecutor>,
    startup_peak_extra_bytes: usize,
    #[cfg(any(test, feature = "test-support"))]
    operation_evidence: std::cell::Cell<CoordinatorOperationEvidence>,
}

#[cfg(feature = "test-support")]
impl Clone for RuntimeDispatchCoordinator {
    fn clone(&self) -> Self {
        Self {
            coordinator_id: next_coordinator_id().expect("coordinator identity space exhausted"),
            state_revision: self.state_revision,
            schedule: self.schedule.clone(),
            min_hold_us: self.min_hold_us,
            min_hold_ticks: self.min_hold_ticks,
            batch_scheduled_ticks: self.batch_scheduled_ticks.clone(),
            cursor: self.cursor,
            active_by_slot: self.active_by_slot.clone(),
            active_mask: self.active_mask,
            blocked_mask: self.blocked_mask,
            counters: self.counters,
            activated_generation_count: self.activated_generation_count,
            generation_states: self.generation_states.clone(),
            generation_count: self.generation_count,
            up_intent_locations: self.up_intent_locations.clone(),
            cancelled_up_by_slot: self.cancelled_up_by_slot,
            pending_release_by_slot: self.pending_release_by_slot,
            pending_release_mask: self.pending_release_mask,
            executor: self.executor.clone(),
            startup_peak_extra_bytes: self.startup_peak_extra_bytes,
            #[cfg(any(test, feature = "test-support"))]
            operation_evidence: std::cell::Cell::new(self.operation_evidence.get()),
        }
    }
}

/// Deterministic operation-site evidence for pause/resume complexity tests.
/// The fields count actual visits, not inferred slice lengths.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CoordinatorOperationEvidence {
    pub resumable_cancel_slot_visits: u64,
    pub local_invariant_slot_visits: u64,
    pub deep_audit_generation_visits: u64,
}

#[cfg(test)]
mod tests;

impl RuntimeDispatchCoordinator {
    #[cfg(test)]
    pub fn new<F>(
        schedule: RuntimeSchedule,
        min_hold_us: u64,
        _delivery_margin_us: u64,
        us_to_ticks: F,
    ) -> Self
    where
        F: Fn(u64) -> TimelineTicks,
    {
        Self::try_new_ticks(
            schedule,
            min_hold_us,
            DurationTicks::from_raw(us_to_ticks(min_hold_us).as_u64()),
            |microseconds| Ok(us_to_ticks(microseconds)),
        )
        .expect("legacy coordinator construction uses an infallible tick converter")
    }

    /// Construct the coordinator with all scheduling durations represented in
    /// the QPC tick domain.
    pub fn try_new_ticks<F>(
        schedule: RuntimeSchedule,
        min_hold_us: u64,
        min_hold_ticks: DurationTicks,
        us_to_ticks: F,
    ) -> Result<Self, CoordinatorError>
    where
        F: Fn(u64) -> Result<TimelineTicks, CoordinatorError>,
    {
        Self::try_new_ticks_with_checkpoint(
            schedule,
            min_hold_us,
            min_hold_ticks,
            us_to_ticks,
            || Ok(()),
        )
    }

    /// Construct the coordinator while periodically allowing a caller to
    /// cancel long schedule scans.  The callback is checked at bounded
    /// intervals during every allocation-populating traversal.
    pub fn try_new_ticks_with_checkpoint<F, C>(
        schedule: RuntimeSchedule,
        min_hold_us: u64,
        min_hold_ticks: DurationTicks,
        us_to_ticks: F,
        mut checkpoint: C,
    ) -> Result<Self, CoordinatorError>
    where
        F: Fn(u64) -> Result<TimelineTicks, CoordinatorError>,
        C: FnMut() -> Result<(), CoordinatorError>,
    {
        let generation_count = schedule.generation_count;
        let generation_count_usize = usize::try_from(generation_count)
            .map_err(|_| CoordinatorError::GenerationCountOverflow)?;
        let schedule_heap_bytes = schedule
            .memory_metrics()
            .map_err(CoordinatorError::Allocation)?
            .heap_payload_bytes;
        let projected_coordinator = checked_storage_sum([
            schedule.compile_memory.caller_retained_capacity_bytes,
            schedule_heap_bytes,
            std::mem::size_of::<Self>(),
            checked_storage_bytes(
                generation_count_usize,
                std::mem::size_of::<GenerationStatus>(),
            )?,
            checked_storage_bytes(schedule.batches.len(), std::mem::size_of::<TimelineTicks>())?,
            checked_storage_bytes(
                generation_count_usize,
                std::mem::size_of::<Option<(usize, usize)>>(),
            )?,
            checked_storage_bytes(
                schedule.down_chords.len(),
                std::mem::size_of::<crate::executor::DownChordTicks>(),
            )?,
        ])?;
        check_retained_budget(projected_coordinator)?;
        let mut generation_states = Vec::new();
        generation_states
            .try_reserve_exact(generation_count_usize)
            .map_err(|error| CoordinatorError::Allocation(error.to_string()))?;
        for index in 0..generation_count_usize {
            if index % 16_384 == 0 {
                checkpoint()?;
            }
            generation_states.push(GenerationStatus::Scheduled);
        }

        let mut batch_scheduled_ticks = Vec::new();
        batch_scheduled_ticks
            .try_reserve_exact(schedule.batches.len())
            .map_err(|error| CoordinatorError::Allocation(error.to_string()))?;
        for (index, batch) in schedule.batches.iter().enumerate() {
            if index % 128 == 0 {
                checkpoint()?;
            }
            batch_scheduled_ticks.push(us_to_ticks(batch.scheduled_us)?);
        }

        let mut up_intent_locations = Vec::new();
        up_intent_locations
            .try_reserve_exact(generation_count_usize)
            .map_err(|error| CoordinatorError::Allocation(error.to_string()))?;
        for index in 0..generation_count_usize {
            if index % 16_384 == 0 {
                checkpoint()?;
            }
            up_intent_locations.push(None);
        }
        let mut visited_intents = 0usize;
        for (packet_index, packet) in schedule.packets.iter().enumerate() {
            if packet_index % 128 == 0 {
                checkpoint()?;
            }
            let start = packet.up_intent_start as usize;
            let end = start.checked_add(usize::from(packet.up_intent_len)).ok_or(
                CoordinatorError::Schedule(RuntimeScheduleError::InvalidPacketIntentRange {
                    index: packet_index,
                }),
            )?;
            let intents = schedule
                .intents
                .get(start..end)
                .ok_or(CoordinatorError::Schedule(
                    RuntimeScheduleError::InvalidPacketIntentRange {
                        index: packet_index,
                    },
                ))?;
            for (offset, compact) in intents.iter().enumerate() {
                visited_intents = visited_intents.saturating_add(1);
                if visited_intents.is_multiple_of(256) {
                    checkpoint()?;
                }
                let generation_id = compact.generation_id();
                if generation_id == NO_GENERATION_ID {
                    continue;
                }
                let generation_index = usize::try_from(generation_id).map_err(|_| {
                    CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                        "generation id does not fit in usize".to_string(),
                    ))
                })?;
                let location = up_intent_locations.get_mut(generation_index).ok_or(
                    CoordinatorError::Invariant(CoordinatorInvariantError::UnknownGeneration {
                        generation_id,
                        generation_count,
                    }),
                )?;
                if location.is_some() {
                    return Err(CoordinatorError::Invariant(
                        CoordinatorInvariantError::Accounting(
                            "generation owns more than one authored Up intent".to_string(),
                        ),
                    ));
                }
                *location = Some((packet_index, start + offset));
            }
        }

        let coordinator_vector_peak_bytes = checked_storage_sum([
            checked_storage_bytes(
                batch_scheduled_ticks.capacity(),
                std::mem::size_of::<TimelineTicks>(),
            )?,
            checked_storage_bytes(
                generation_states.capacity(),
                std::mem::size_of::<GenerationStatus>(),
            )?,
            checked_storage_bytes(
                up_intent_locations.capacity(),
                std::mem::size_of::<Option<(usize, usize)>>(),
            )?,
        ])?;
        let startup_peak_preflight = checked_storage_sum([
            schedule.compile_memory.caller_retained_capacity_bytes,
            schedule_heap_bytes,
            std::mem::size_of::<Self>(),
            coordinator_vector_peak_bytes,
            checked_storage_bytes(
                schedule.down_chords.len(),
                std::mem::size_of::<crate::executor::DownChordTicks>(),
            )?,
        ])?;
        check_retained_budget(startup_peak_preflight)?;

        let coordinator_id = next_coordinator_id()?;
        Ok(Self {
            coordinator_id,
            state_revision: 0,
            schedule,
            min_hold_us,
            min_hold_ticks,
            batch_scheduled_ticks: batch_scheduled_ticks.into_boxed_slice(),
            cursor: 0,
            active_by_slot: std::array::from_fn(|_| None),
            active_mask: 0,
            blocked_mask: 0,
            counters: GenerationCounters::default(),
            activated_generation_count: 0,
            generation_states: generation_states.into_boxed_slice(),
            generation_count,
            up_intent_locations: up_intent_locations.into_boxed_slice(),
            cancelled_up_by_slot: [None; MAX_KEYS],
            pending_release_by_slot: [None; MAX_KEYS],
            pending_release_mask: 0,
            executor: None,
            startup_peak_extra_bytes: coordinator_vector_peak_bytes,
            #[cfg(any(test, feature = "test-support"))]
            operation_evidence: std::cell::Cell::new(CoordinatorOperationEvidence::default()),
        })
    }

    fn bit_for_slot(slot: KeySlot) -> u16 {
        1u16 << slot
    }

    #[inline]
    pub fn bit_for_slot_pub(slot: KeySlot) -> u16 {
        Self::bit_for_slot(slot)
    }

    pub fn active_for_slot(&self, slot: KeySlot) -> Option<&ActiveGeneration> {
        self.active_by_slot[usize::from(slot)].as_ref()
    }

    pub fn pending_release_for_slot(&self, slot: KeySlot) -> Option<PendingRelease> {
        self.pending_release_by_slot
            .get(usize::from(slot))
            .copied()
            .flatten()
    }

    pub fn pending_release_mask(&self) -> u16 {
        self.pending_release_mask
    }

    pub fn pending_release_count(&self) -> usize {
        self.pending_release_mask.count_ones() as usize
    }

    pub fn attach_executor<F, E>(
        &mut self,
        min_hold_ticks: DurationTicks,
        min_release_gap_ticks: DurationTicks,
        to_ticks: F,
    ) -> Result<(), String>
    where
        F: FnMut(u64) -> Result<TimelineTicks, E>,
        E: std::fmt::Display,
    {
        self.attach_executor_with_cancellation(
            min_hold_ticks,
            min_release_gap_ticks,
            to_ticks,
            || Ok(()),
        )
    }

    pub fn attach_executor_with_cancellation<F, E, C>(
        &mut self,
        min_hold_ticks: DurationTicks,
        min_release_gap_ticks: DurationTicks,
        to_ticks: F,
        checkpoint: C,
    ) -> Result<(), String>
    where
        F: FnMut(u64) -> Result<TimelineTicks, E>,
        E: std::fmt::Display,
        C: FnMut() -> Result<(), String>,
    {
        let projected_executor_bytes = self
            .schedule
            .down_chords
            .len()
            .checked_mul(std::mem::size_of::<crate::executor::DownChordTicks>())
            .ok_or_else(|| "executor storage size overflow".to_string())?;
        self.check_retained_with_executor_bytes(projected_executor_bytes)?;
        let executor = crate::executor::DispatchExecutor::new_with_cancellation(
            &self.schedule,
            min_hold_ticks,
            min_release_gap_ticks,
            to_ticks,
            checkpoint,
        )?;
        self.check_retained_with_executor_bytes(executor.try_memory_metrics()?.chord_heap_bytes)?;
        self.executor = Some(executor);
        Ok(())
    }
}
