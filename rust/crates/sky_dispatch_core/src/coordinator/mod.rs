//! Runtime dispatch coordinator managing generation status transitions and release eligibility.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use thiserror::Error;

use crate::model::*;
use crate::time::{DurationTicks, TimelineTicks};

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
    #[error("invalid key slot {slot}")]
    InvalidKeySlot { slot: KeySlot },
    #[error("generation count does not fit in usize")]
    GenerationCountOverflow,
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
    pub fn terminal_total(&self) -> u64 {
        self.released
            + self.dropped_conflict
            + self.dropped_backend
            + self.dropped_expired
            + self.cancelled
    }

    fn increment(&mut self, status: GenerationStatus) {
        match status {
            GenerationStatus::Released => self.released += 1,
            GenerationStatus::DroppedConflict => self.dropped_conflict += 1,
            GenerationStatus::DroppedBackend => self.dropped_backend += 1,
            GenerationStatus::DroppedExpired => self.dropped_expired += 1,
            GenerationStatus::Cancelled => self.cancelled += 1,
            // Non-terminal states are not tracked here; they are derived from masks.
            GenerationStatus::Scheduled | GenerationStatus::Active => {}
        }
    }
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
mod timeline;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveGeneration {
    pub generation_id: GenerationId,
    pub scan_code: u16,
    pub key_slot: KeySlot,
    pub source_action_index: u32,
    pub scheduled_down_ticks: TimelineTicks,
    pub down_dispatch_started_ticks: TimelineTicks,
    pub down_dispatch_completed_ticks: TimelineTicks,
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
/// intents are classified per key against completion-anchored release floors.
/// Preparation never mutates coordinator state.
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

/// One completion-anchored release that is waiting for its own physical due
/// boundary.  The table is bounded by the fifteen physical key slots.
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
    generation_states: Box<[GenerationStatus]>,
    generation_count: u64,
    up_intent_locations: Box<[Option<(usize, usize)>]>,
    pending_release_by_slot: [Option<PendingRelease>; MAX_KEYS],
    pending_release_mask: u16,
}

#[cfg(test)]
#[allow(unused_must_use)]
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
        let generation_count = schedule.generation_count;
        let generation_count_usize = usize::try_from(generation_count)
            .map_err(|_| CoordinatorError::GenerationCountOverflow)?;
        let generation_states =
            vec![GenerationStatus::Scheduled; generation_count_usize].into_boxed_slice();
        let batch_scheduled_ticks = schedule
            .batches
            .iter()
            .map(|b| us_to_ticks(b.scheduled_us))
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice();
        let mut up_intent_locations = vec![None; generation_count_usize];
        for (packet_index, packet) in schedule.packets.iter().enumerate() {
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

        Ok(Self {
            schedule,
            min_hold_us,
            min_hold_ticks,
            batch_scheduled_ticks,
            cursor: 0,
            active_by_slot: std::array::from_fn(|_| None),
            active_mask: 0,
            blocked_mask: 0,
            counters: GenerationCounters::default(),
            generation_states,
            generation_count,
            up_intent_locations: up_intent_locations.into_boxed_slice(),
            pending_release_by_slot: [None; MAX_KEYS],
            pending_release_mask: 0,
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
}
