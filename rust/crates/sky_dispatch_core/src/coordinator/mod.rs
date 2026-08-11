//! Runtime dispatch coordinator managing generation status transitions and release eligibility.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use thiserror::Error;

use crate::model::*;
use crate::time::{DurationTicks, TimelineTicks};

pub const MAX_RELEASE_RETRIES: u8 = 8;
#[cfg(test)]
const RELEASE_RETRY_BACKOFF_US: [u64; 4] = [2_000, 5_000, 10_000, 20_000];

type SplitIntentResult = (
    SmallVec<[RuntimeKeyIntent; MAX_KEYS]>,
    SmallVec<[RuntimeKeyIntent; MAX_KEYS]>,
);
type ReleaseRequestResult = (
    SmallVec<[PendingRelease; MAX_KEYS]>,
    SmallVec<[RuntimeKeyIntent; MAX_KEYS]>,
);

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
                | (Self::Active, Self::ReleasePending)
                | (Self::Active, Self::DroppedBackend)
                | (Self::Active, Self::Cancelled)
                | (Self::ReleasePending, Self::Released)
                | (Self::ReleasePending, Self::DroppedBackend)
                | (Self::ReleasePending, Self::Cancelled)
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
/// Active and release-pending counts are derived from `active_mask`/`pending_mask`
/// at query time; this struct tracks only terminal and implicit-scheduled generations.
/// No `HashMap` is allocated; all fields are plain `u64`.
///
/// Invariant: `scheduled + active + release_pending + released
///            + dropped_conflict + dropped_backend + dropped_expired + cancelled
///            == total (generation_count)`
///
/// "scheduled" is implicit: `generation_count - (active + release_pending + terminal_total)`.
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
            GenerationStatus::Scheduled
            | GenerationStatus::Active
            | GenerationStatus::ReleasePending => {}
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

mod authored;
mod cleanup;
mod conflict;
mod ownership;
mod pending;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRelease {
    pub generation_id: GenerationId,
    pub scan_code: u16,
    pub key_slot: KeySlot,
    pub source_action_index: u32,
    pub packet_id: PacketId,
    /// Authored microsecond metadata retained for telemetry serialization only.
    pub scheduled_release_us: u64,
    pub scheduled_release_ticks: TimelineTicks,
    pub down_dispatch_started_ticks: TimelineTicks,
    pub release_not_before_ticks: TimelineTicks,
    pub reason_id: ReasonId,
    pub retry_count: u8,
    pub next_retry_ticks: TimelineTicks,
    pub first_failure_ticks: Option<TimelineTicks>,
    pub last_win32_error: Option<u32>,
}

impl PendingRelease {
    #[allow(dead_code)]
    #[cfg(test)]
    pub fn get_effective_release_us(&self) -> u64 {
        self.release_not_before_ticks
            .as_u64()
            .max(self.scheduled_release_us)
            .max(self.next_retry_ticks.as_u64())
    }

    pub fn get_effective_release_ticks(&self) -> Result<TimelineTicks, CoordinatorError> {
        Ok(self
            .release_not_before_ticks
            .max(self.scheduled_release_ticks)
            .max(self.next_retry_ticks))
    }
}

/// The release cohort selected for one upcoming dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingDispatchPlan {
    pub deadline_ticks: TimelineTicks,
    pub polyphony: usize,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineRebaseReason {
    ReleaseRecovery,
}

impl TimelineRebaseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReleaseRecovery => "release_recovery",
        }
    }
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
    /// Physical-key ownership mask. A release-pending key remains blocked
    /// here until a verified Up completion, while logical accounting moves
    /// from `active_mask` to `pending_mask`.
    blocked_mask: u16,
    pending_by_slot: [Option<PendingRelease>; MAX_KEYS],
    pub pending_mask: u16,
    /// Terminal and implicit-scheduled generation counts.
    ///
    /// Active and release-pending counts are derived from `active_mask`/`pending_mask`
    /// respectively, so they are not stored here.  This eliminates the
    /// `HashMap<GenerationId, GenerationStatus>` from the hot dispatch path.
    counters: GenerationCounters,
    generation_states: Box<[GenerationStatus]>,
    generation_count: u64,
    recovery_offset_ticks: DurationTicks,
    up_intent_locations: Box<[Option<(usize, usize)>]>,
    timeline_rebase_count: u64,
    timeline_rebase_total_ticks: u64,
    timeline_rebase_max_ticks: u64,
    last_timeline_rebase_reason: Option<TimelineRebaseReason>,
    release_recovery_started_ticks: Option<TimelineTicks>,
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
            pending_by_slot: std::array::from_fn(|_| None),
            pending_mask: 0,
            counters: GenerationCounters::default(),
            generation_states,
            generation_count,
            recovery_offset_ticks: DurationTicks::ZERO,
            up_intent_locations: up_intent_locations.into_boxed_slice(),
            timeline_rebase_count: 0,
            timeline_rebase_total_ticks: 0,
            timeline_rebase_max_ticks: 0,
            last_timeline_rebase_reason: None,
            release_recovery_started_ticks: None,
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

    pub fn recovery_offset_ticks(&self) -> DurationTicks {
        self.recovery_offset_ticks
    }

    pub fn timeline_rebase_count(&self) -> u64 {
        self.timeline_rebase_count
    }

    pub fn timeline_rebase_total_ticks(&self) -> DurationTicks {
        DurationTicks::from_raw(self.timeline_rebase_total_ticks)
    }

    pub fn timeline_rebase_max_ticks(&self) -> DurationTicks {
        DurationTicks::from_raw(self.timeline_rebase_max_ticks)
    }

    pub fn last_timeline_rebase_reason(&self) -> Option<TimelineRebaseReason> {
        self.last_timeline_rebase_reason
    }
}
