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
    #[error("invalid key slot {slot}")]
    InvalidKeySlot { slot: KeySlot },
    #[error("generation count does not fit in usize")]
    GenerationCountOverflow,
    #[error("time conversion failed: {0}")]
    TimeConversion(String),
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

#[cfg(test)]
#[allow(unused_must_use)]
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
                    scan_codes: vec![0x15].into(),
                    reason: "down".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15].into(),
                    reason: "up".into(),
                },
            ],
            &[0x15],
        )
        .expect("valid schedule");
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
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
    fn sublead_authored_batches_keep_distinct_deadlines() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15].into(),
                    reason: "first".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Down,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x16].into(),
                    reason: "second".into(),
                },
            ],
            &[0x15, 0x16],
        )
        .expect("valid schedule");
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);

        let (first, _) = coordinator
            .pop_next_due_authored(0, 2_000)
            .expect("first action is due");
        assert_eq!(first.scheduled_us, 0);
        assert_eq!(coordinator.next_authored_us(2_000), Some(1_000));
        assert!(coordinator.pop_next_due_authored(0, 2_000).is_none());

        let (second, _) = coordinator
            .pop_next_due_authored(1_000, 2_000)
            .expect("second action keeps its authored ordering");
        assert_eq!(second.scheduled_us, 1_000);
    }

    #[test]
    fn failed_release_is_requeued_and_unblocks_later_same_key_down() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15].into(),
                    reason: "down-1".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15].into(),
                    reason: "up-1".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Down,
                    scheduled_us: 4_000,
                    scan_codes: vec![0x15].into(),
                    reason: "down-2".into(),
                },
            ],
            &[0x15],
        )
        .expect("valid schedule");
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("first down is due");
        coordinator.activate_sent_downs(
            &down.intents,
            &[0x15],
            0,
            crate::time::TimelineTicks::from_raw(0),
            10,
            crate::time::TimelineTicks::from_raw(10),
        );

        let (up, _) = coordinator
            .pop_next_due_authored(1_000, 0)
            .expect("up is due");
        let (_, suppressed) = coordinator
            .request_releases(&up.intents)
            .expect("valid release request");
        assert!(suppressed.is_empty());

        let due = coordinator.pop_due_pending(1_000, 0);
        assert_eq!(due.len(), 1);
        assert!(
            !coordinator
                .requeue_failed_releases(&due, &[], &[], 1_000, 1_000, Some(5))
                .expect("valid recovery")
        );
        assert_eq!(coordinator.next_pending_release_us(0), Some(3_000));
        assert!(!coordinator.is_finished());

        let retry = coordinator.pop_due_pending(3_000, 0);
        assert_eq!(retry.len(), 1);
        coordinator.complete_releases(&retry, &[0x15], &[]);
        assert_eq!(coordinator.finish_release_recovery(3_000), Ok(Some(2_000)));
        assert_eq!(coordinator.schedule.batches[2].scheduled_us, 4_000);

        let (next_down, _) = coordinator
            .pop_next_due_authored(6_000, 0)
            .expect("same-key down remains schedulable after recovery");
        let (playable, conflicts) = coordinator
            .split_down_intents(&next_down.intents)
            .expect("valid transition");
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
                    scan_codes: vec![0x15].into(),
                    reason: "down-1".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15].into(),
                    reason: "up-1".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Down,
                    scheduled_us: 2_000,
                    scan_codes: vec![0x15].into(),
                    reason: "down-2".into(),
                },
            ],
            &[0x15],
        )
        .expect("valid schedule");
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("first down is due");
        coordinator.activate_sent_downs(
            &down.intents,
            &[0x15],
            0,
            crate::time::TimelineTicks::from_raw(0),
            10,
            crate::time::TimelineTicks::from_raw(10),
        );
        let (up, _) = coordinator
            .pop_next_due_authored(1_000, 0)
            .expect("up is due");
        let _ = coordinator.request_releases(&up.intents);
        let due = coordinator.pop_due_pending(1_000, 0);
        assert!(
            !coordinator
                .requeue_failed_releases(&due, &[], &[], 1_000, 1_000, Some(5))
                .expect("valid recovery")
        );

        assert!(coordinator.pop_next_due_authored(2_000, 0).is_none());
        assert_eq!(coordinator.next_deadline_us(0, 0), Some(3_000));

        let retry = coordinator.pop_due_pending(3_000, 0);
        coordinator.complete_releases(&retry, &[0x15], &[]);
        assert_eq!(coordinator.finish_release_recovery(3_000), Ok(Some(2_000)));
        assert_eq!(coordinator.schedule.batches[2].scheduled_us, 2_000);
        assert!(coordinator.pop_next_due_authored(3_000, 0).is_none());
        let (next_down, _) = coordinator
            .pop_next_due_authored(4_000, 0)
            .expect("timeline-shifted same-key down is due");
        assert_eq!(next_down.scheduled_us, 4_000);
        let (playable, conflicts) = coordinator
            .split_down_intents(&next_down.intents)
            .expect("valid transition");
        assert_eq!(playable.len(), 1);
        assert!(conflicts.is_empty());
    }

    #[test]
    fn blocked_authored_deadline_includes_delivery_margin() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15].into(),
                    reason: "down".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 100,
                    scan_codes: vec![0x15].into(),
                    reason: "up".into(),
                },
            ],
            &[0x15],
        )
        .expect("valid schedule");

        let min_hold = 20_000;
        let delivery_margin = 5_000;
        let mut coordinator = RuntimeDispatchCoordinator::new(
            schedule,
            min_hold,
            delivery_margin,
            crate::time::TimelineTicks::from_raw,
        );

        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("down is due");

        // Complete the down at t=100.
        coordinator.activate_sent_downs(
            &down.intents,
            &[0x15],
            50,
            crate::time::TimelineTicks::from_raw(50),
            100,
            crate::time::TimelineTicks::from_raw(100),
        );

        let (up, _) = coordinator
            .pop_next_due_authored(100, 0)
            .expect("up is due");

        // Pop it and request release.
        let _ = coordinator.request_releases(&up.intents);

        // Since up was authored at 100, but min_hold is 20,000 and delivery_margin is 5,000,
        // the effective release time must be at least 100 (completion) + 20,000 + 5,000 = 25,100.
        let due = coordinator.pop_due_pending(25_099, 0);
        assert!(
            due.is_empty(),
            "release must not be due before min_hold + delivery_margin"
        );

        let due_now = coordinator.pop_due_pending(25_100, 0);
        assert_eq!(
            due_now.len(),
            1,
            "release must be due exactly at completion + min_hold + delivery_margin"
        );
    }

    #[test]
    fn blocked_authored_deadline_includes_recovery_offset_when_lead_is_enabled() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15, 0x16].into(),
                    reason: "down-1".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15, 0x16].into(),
                    reason: "up-1".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Down,
                    scheduled_us: 2_000,
                    scan_codes: vec![0x15, 0x16].into(),
                    reason: "down-2".into(),
                },
            ],
            &[0x15, 0x16],
        )
        .expect("valid schedule");
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);

        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("first down is due");
        coordinator.activate_sent_downs(
            &down.intents,
            &[0x15, 0x16],
            0,
            crate::time::TimelineTicks::from_raw(0),
            10,
            crate::time::TimelineTicks::from_raw(10),
        );
        let (up, _) = coordinator
            .pop_next_due_authored(1_000, 0)
            .expect("release is due");
        let _ = coordinator.request_releases(&up.intents);
        let due = coordinator.pop_due_pending(1_000, 0);
        assert!(
            !coordinator
                .requeue_failed_releases(&due, &[], &[], 1_000, 1_500, Some(5))
                .expect("valid recovery")
        );
        assert_eq!(coordinator.next_pending_release_us(0), Some(3_500));

        let retry = coordinator.pop_due_pending(3_500, 0);
        coordinator.complete_releases(&retry, &[0x15], &[]);
        assert_eq!(coordinator.finish_release_recovery(3_500), Ok(Some(2_500)));

        // Key 0x16 remains active, so the next chord cannot be early-popped.
        // Its blocked deadline must still use the effective timeline offset.
        assert_eq!(coordinator.next_authored_us(100), Some(4_500));
    }

    #[test]
    fn pending_plan_uses_the_next_release_cohort_not_all_pending_keys() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15, 0x16].into(),
                    reason: "down".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15].into(),
                    reason: "up-a".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Up,
                    scheduled_us: 1_100,
                    scan_codes: vec![0x16].into(),
                    reason: "up-b".into(),
                },
            ],
            &[0x15, 0x16],
        )
        .expect("valid schedule");
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("down is due");
        coordinator.activate_sent_downs(
            &down.intents,
            &[0x15, 0x16],
            0,
            crate::time::TimelineTicks::from_raw(0),
            0,
            crate::time::TimelineTicks::from_raw(0),
        );

        let (up_a, _) = coordinator
            .pop_next_due_authored(1_000, 0)
            .expect("first release is due");
        let _ = coordinator.request_releases(&up_a.intents);
        let (up_b, _) = coordinator
            .pop_next_due_authored(1_100, 0)
            .expect("second release is due");
        let _ = coordinator.request_releases(&up_b.intents);

        let plan = coordinator
            .plan_pending_dispatch(|polyphony| match polyphony {
                1 => (100, false),
                2 => (200, false),
                _ => (200, false),
            })
            .expect("pending plan");
        assert_eq!(plan.polyphony, 1);
        assert_eq!(plan.deadline_ticks.as_u64(), 900);
        assert_eq!(
            coordinator.next_deadline_with_pending_plan(0, Some(&plan)),
            Some(900)
        );

        let due = coordinator.pop_due_pending_with_plan(1_000, &plan);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].scan_code, 0x15);
        assert_eq!(coordinator.pending_mask.count_ones(), 1);
    }

    // --- P2.2 tests: GenerationCounters invariants ---

    #[test]
    fn generation_counters_total_equals_generation_count_after_full_lifecycle() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15, 0x16].into(),
                    reason: "down".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15, 0x16].into(),
                    reason: "up".into(),
                },
            ],
            &[0x15, 0x16],
        )
        .expect("valid schedule");
        let generation_count = schedule.generation_count;
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);

        let (down, _) = coordinator.pop_next_due_authored(0, 0).unwrap();
        coordinator.activate_sent_downs(
            &down.intents,
            &[0x15, 0x16],
            0,
            crate::time::TimelineTicks::from_raw(0),
            0,
            crate::time::TimelineTicks::from_raw(0),
        );

        let (up, _) = coordinator.pop_next_due_authored(1_000, 0).unwrap();
        let _ = coordinator.request_releases(&up.intents);
        let due = coordinator.pop_due_pending(1_000, 0);
        coordinator.complete_releases(&due, &[0x15, 0x16], &[]);

        let counts = coordinator.generation_status_counts();
        let total: u64 = counts.values().sum();
        assert_eq!(total, generation_count, "total must equal generation_count");
        assert_eq!(counts.get("released"), Some(&generation_count));
        assert_eq!(counts.get("active"), Some(&0));
        assert_eq!(counts.get("release_pending"), Some(&0));
    }

    #[test]
    fn generation_counters_not_negative_invariant() {
        // Counters are u64; they cannot go below zero by type.
        // This test verifies that after cancel_all the total still equals generation_count.
        let schedule = compile_runtime_intents(
            &[KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "down".into(),
            }],
            &[0x15],
        )
        .expect("valid schedule");
        let generation_count = schedule.generation_count;
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);

        let (down, _) = coordinator.pop_next_due_authored(0, 0).unwrap();
        coordinator.activate_sent_downs(
            &down.intents,
            &[0x15],
            0,
            crate::time::TimelineTicks::from_raw(0),
            0,
            crate::time::TimelineTicks::from_raw(0),
        );

        coordinator.cancel_all();

        let counts = coordinator.generation_status_counts();
        let total: u64 = counts.values().sum();
        assert_eq!(total, generation_count);
        assert_eq!(counts.get("cancelled"), Some(&generation_count));
    }

    #[test]
    fn cancel_live_generations_preserves_future_scheduled_generations() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15].into(),
                    reason: "live".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 100,
                    scan_codes: vec![0x15].into(),
                    reason: "live-up".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Down,
                    scheduled_us: 200,
                    scan_codes: vec![0x16].into(),
                    reason: "future".into(),
                },
                KeyActionInput {
                    source_action_index: 3,
                    kind: ActionKind::Up,
                    scheduled_us: 300,
                    scan_codes: vec![0x16].into(),
                    reason: "future-up".into(),
                },
            ],
            &[0x15, 0x16],
        )
        .expect("valid schedule");
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
        let (live_down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("live down is due");
        coordinator
            .activate_sent_downs(
                &live_down.intents,
                &[0x15],
                0,
                crate::time::TimelineTicks::from_raw(0),
                0,
                crate::time::TimelineTicks::from_raw(0),
            )
            .expect("live down activates");

        let cancelled = coordinator
            .cancel_live_generations()
            .expect("live cancellation succeeds");
        assert_eq!(cancelled, vec![0]);
        let counts = coordinator.generation_status_counts();
        assert_eq!(counts.get(GenerationStatus::Cancelled.as_str()), Some(&1));
        assert_eq!(counts.get(GenerationStatus::Scheduled.as_str()), Some(&1));
        assert_eq!(counts.get(GenerationStatus::Active.as_str()), Some(&0));
        assert!(coordinator.check_invariants().is_ok());
        assert!(coordinator.cancel_live_generations().unwrap().is_empty());

        let (stale_up, _) = coordinator
            .pop_next_due_authored(100, 0)
            .expect("cancelled generation's authored up remains in cursor order");
        assert_eq!(stale_up.intents[0].scan_code, 0x15);
        let (future_down, _) = coordinator
            .pop_next_due_authored(200, 0)
            .expect("future down remains schedulable");
        assert_eq!(future_down.intents[0].scan_code, 0x16);
    }

    #[test]
    fn cancel_live_generations_cancels_release_pending_but_not_future() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15].into(),
                    reason: "live".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 100,
                    scan_codes: vec![0x15].into(),
                    reason: "live-up".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Down,
                    scheduled_us: 200,
                    scan_codes: vec![0x16].into(),
                    reason: "future".into(),
                },
            ],
            &[0x15, 0x16],
        )
        .expect("valid schedule");
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("down is due");
        coordinator
            .activate_sent_downs(
                &down.intents,
                &[0x15],
                0,
                crate::time::TimelineTicks::from_raw(0),
                0,
                crate::time::TimelineTicks::from_raw(0),
            )
            .expect("down activates");
        let (up, _) = coordinator
            .pop_next_due_authored(100, 0)
            .expect("up is due");
        coordinator
            .request_releases(&up.intents)
            .expect("release request succeeds");

        let cancelled = coordinator
            .cancel_live_generations()
            .expect("pending cancellation succeeds");
        assert_eq!(cancelled, vec![0]);
        let counts = coordinator.generation_status_counts();
        assert_eq!(counts.get(GenerationStatus::Cancelled.as_str()), Some(&1));
        assert_eq!(counts.get(GenerationStatus::Scheduled.as_str()), Some(&1));
        assert_eq!(
            counts.get(GenerationStatus::ReleasePending.as_str()),
            Some(&0)
        );
        assert!(coordinator.check_invariants().is_ok());
    }

    #[test]
    fn generation_counters_each_slot_has_at_most_one_generation() {
        // After activate, each slot bit is set at most once.
        let schedule = compile_runtime_intents(
            &[KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15, 0x16, 0x17].into(),
                reason: "down".into(),
            }],
            &[0x15, 0x16, 0x17],
        )
        .expect("valid schedule");
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
        let (down, _) = coordinator.pop_next_due_authored(0, 0).unwrap();
        coordinator.activate_sent_downs(
            &down.intents,
            &[0x15, 0x16, 0x17],
            0,
            crate::time::TimelineTicks::from_raw(0),
            0,
            crate::time::TimelineTicks::from_raw(0),
        );

        // active_mask must have exactly 3 bits set (one per slot)
        assert_eq!(coordinator.active_mask.count_ones(), 3);
        let counts = coordinator.generation_status_counts();
        assert_eq!(counts.get("active"), Some(&3));
    }

    #[test]
    fn release_correct_generation_counter() {
        // Verifies that releasing one generation does not affect counters of others.
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15, 0x16].into(),
                    reason: "down".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15].into(),
                    reason: "up-a".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Up,
                    scheduled_us: 2_000,
                    scan_codes: vec![0x16].into(),
                    reason: "up-b".into(),
                },
            ],
            &[0x15, 0x16],
        )
        .expect("valid schedule");
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);

        let (down, _) = coordinator.pop_next_due_authored(0, 0).unwrap();
        coordinator.activate_sent_downs(
            &down.intents,
            &[0x15, 0x16],
            0,
            crate::time::TimelineTicks::from_raw(0),
            0,
            crate::time::TimelineTicks::from_raw(0),
        );

        // Release only 0x15
        let (up_a, _) = coordinator.pop_next_due_authored(1_000, 0).unwrap();
        let _ = coordinator.request_releases(&up_a.intents);
        let due = coordinator.pop_due_pending(1_000, 0);
        coordinator.complete_releases(&due, &[0x15], &[]);

        let counts = coordinator.generation_status_counts();
        assert_eq!(counts.get("released"), Some(&1));
        assert_eq!(counts.get("active"), Some(&1)); // 0x16 still active
        assert_eq!(counts.get("release_pending"), Some(&0));

        // Release 0x16
        let (up_b, _) = coordinator.pop_next_due_authored(2_000, 0).unwrap();
        let _ = coordinator.request_releases(&up_b.intents);
        let due2 = coordinator.pop_due_pending(2_000, 0);
        coordinator.complete_releases(&due2, &[0x16], &[]);

        let counts2 = coordinator.generation_status_counts();
        assert_eq!(counts2.get("released"), Some(&2));
        assert_eq!(counts2.get("active"), Some(&0));
    }

    #[test]
    fn cancel_cleanup_correct_counter() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15].into(),
                    reason: "down".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15].into(),
                    reason: "up".into(),
                },
            ],
            &[0x15],
        )
        .expect("valid schedule");
        let generation_count = schedule.generation_count;
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);

        let (down, _) = coordinator.pop_next_due_authored(0, 0).unwrap();
        coordinator.activate_sent_downs(
            &down.intents,
            &[0x15],
            0,
            crate::time::TimelineTicks::from_raw(0),
            0,
            crate::time::TimelineTicks::from_raw(0),
        );

        // Cancel while active (before up)
        coordinator.cancel_all();

        let counts = coordinator.generation_status_counts();
        let total: u64 = counts.values().sum();
        // The Up intent was never processed: generation is cancelled while active
        // generation_count = 1 (one Down generation)
        assert_eq!(total, generation_count);
        assert_eq!(counts.get("cancelled"), Some(&1));
        assert_eq!(counts.get("active"), Some(&0));
    }

    #[test]
    fn differential_with_old_implementation_on_random_valid_schedule() {
        // Verify that generation_status_counts sums correctly for a 3-note sequence.
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15].into(),
                    reason: "d1".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 500,
                    scan_codes: vec![0x15].into(),
                    reason: "u1".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Down,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x16].into(),
                    reason: "d2".into(),
                },
                KeyActionInput {
                    source_action_index: 3,
                    kind: ActionKind::Up,
                    scheduled_us: 1_500,
                    scan_codes: vec![0x16].into(),
                    reason: "u2".into(),
                },
                KeyActionInput {
                    source_action_index: 4,
                    kind: ActionKind::Down,
                    scheduled_us: 2_000,
                    scan_codes: vec![0x17].into(),
                    reason: "d3".into(),
                },
                KeyActionInput {
                    source_action_index: 5,
                    kind: ActionKind::Up,
                    scheduled_us: 2_500,
                    scan_codes: vec![0x17].into(),
                    reason: "u3".into(),
                },
            ],
            &[0x15, 0x16, 0x17],
        )
        .expect("valid schedule");
        let generation_count = schedule.generation_count;
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);

        // Play through the full schedule
        for _ in 0..6 {
            if let Some((batch, _)) = coordinator.pop_next_due_authored(u64::MAX, 0) {
                match batch.kind {
                    ActionKind::Down => {
                        let sc: Vec<u16> = batch.intents.iter().map(|i| i.scan_code).collect();
                        coordinator.activate_sent_downs(
                            &batch.intents,
                            &sc,
                            0,
                            crate::time::TimelineTicks::from_raw(0),
                            0,
                            crate::time::TimelineTicks::from_raw(0),
                        );
                    }
                    ActionKind::Up => {
                        let _ = coordinator.request_releases(&batch.intents);
                    }
                }
            }
        }
        // Pop and complete all pending
        let due = coordinator.pop_due_pending(u64::MAX, 0);
        for release in &due {
            coordinator.complete_releases(std::slice::from_ref(release), &[release.scan_code], &[]);
        }

        let counts = coordinator.generation_status_counts();
        let total: u64 = counts.values().sum();
        assert_eq!(total, generation_count, "total must match generation_count");
    }

    #[test]
    fn non_contiguous_source_ids_do_not_index_batch_timestamps() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 100,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15].into(),
                    reason: "down-a".into(),
                },
                KeyActionInput {
                    source_action_index: 500,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15].into(),
                    reason: "up-a".into(),
                },
                KeyActionInput {
                    source_action_index: 9_000,
                    kind: ActionKind::Down,
                    scheduled_us: 2_000,
                    scan_codes: vec![0x16].into(),
                    reason: "down-b".into(),
                },
                KeyActionInput {
                    source_action_index: 12_000,
                    kind: ActionKind::Up,
                    scheduled_us: 3_000,
                    scan_codes: vec![0x16].into(),
                    reason: "up-b".into(),
                },
            ],
            &[0x15, 0x16],
        )
        .expect("valid non-contiguous source IDs");
        let generation_count = schedule.generation_count;
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);

        while let Some((batch, _)) = coordinator.pop_next_due_authored(u64::MAX, 0) {
            match batch.kind {
                ActionKind::Down => {
                    let sent: Vec<u16> = batch
                        .intents
                        .iter()
                        .map(|intent| intent.scan_code)
                        .collect();
                    coordinator.activate_sent_downs(
                        &batch.intents,
                        &sent,
                        0,
                        crate::time::TimelineTicks::from_raw(0),
                        0,
                        crate::time::TimelineTicks::from_raw(0),
                    );
                }
                ActionKind::Up => {
                    coordinator.request_releases(&batch.intents);
                }
            }
            let due = coordinator.pop_due_pending(u64::MAX, 0);
            for release in &due {
                coordinator.complete_releases(
                    std::slice::from_ref(release),
                    &[release.scan_code],
                    &[],
                );
            }
        }

        let counts = coordinator.generation_status_counts();
        assert_eq!(counts.get("released"), Some(&2));
        assert_eq!(counts.get("active"), Some(&0));
        assert_eq!(counts.get("release_pending"), Some(&0));
        assert_eq!(counts.values().sum::<u64>(), generation_count);
    }

    #[test]
    fn duplicate_release_completion_does_not_double_terminalize_generation() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 10,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15].into(),
                    reason: "down".into(),
                },
                KeyActionInput {
                    source_action_index: 20,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15].into(),
                    reason: "up".into(),
                },
            ],
            &[0x15],
        )
        .expect("valid schedule");
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
        let (down, _) = coordinator.pop_next_due_authored(0, 0).unwrap();
        coordinator.activate_sent_downs(
            &down.intents,
            &[0x15],
            0,
            crate::time::TimelineTicks::from_raw(0),
            0,
            crate::time::TimelineTicks::from_raw(0),
        );
        let (up, _) = coordinator.pop_next_due_authored(u64::MAX, 0).unwrap();
        coordinator.request_releases(&up.intents);
        let due = coordinator.pop_due_pending(u64::MAX, 0);
        coordinator
            .complete_releases(&due, &[0x15], &[])
            .expect("first completion");

        let counts = coordinator.generation_status_counts();
        assert_eq!(counts.get("released"), Some(&1));
        assert_eq!(counts.values().sum::<u64>(), 1);
        assert!(coordinator.complete_releases(&due, &[0x15], &[]).is_err());
    }
}

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
    pub fn get_effective_release_us(&self, lead_up: u64) -> u64 {
        let effective_lead = self.scheduled_release_us.saturating_sub(lead_up);
        self.release_not_before_ticks
            .as_u64()
            .max(effective_lead)
            .max(self.next_retry_ticks.as_u64())
    }

    pub fn get_effective_release_ticks(
        &self,
        lead_up: DurationTicks,
    ) -> Result<TimelineTicks, CoordinatorError> {
        let effective_lead =
            if self.scheduled_release_ticks >= TimelineTicks::from_raw(lead_up.as_u64()) {
                lead_up
            } else {
                DurationTicks::ZERO
            };
        let led = self
            .scheduled_release_ticks
            .checked_sub_duration(effective_lead)?;
        Ok(self
            .release_not_before_ticks
            .max(led)
            .max(self.next_retry_ticks))
    }
}

/// The release cohort selected for one upcoming dispatch.
///
/// The worker must use the same lead both when calculating the next deadline
/// and when popping pending releases.  Keeping the result together prevents
/// a larger pending population from over-leading an earlier one-key release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingDispatchPlan {
    pub deadline_ticks: TimelineTicks,
    pub lead_ticks: DurationTicks,
    /// Telemetry/API compatibility metadata. Scheduling uses the tick fields.
    pub polyphony: usize,
    pub lead_saturated: bool,
}

#[derive(Debug)]
pub struct RuntimeDispatchCoordinator {
    pub schedule: RuntimeSchedule,
    pub min_hold_us: u64,
    pub min_hold_ticks: DurationTicks,
    pub delivery_margin_us: u64,
    pub delivery_margin_ticks: DurationTicks,
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
    release_recovery_started_ticks: Option<TimelineTicks>,
}

impl RuntimeDispatchCoordinator {
    #[cfg(test)]
    pub fn new<F>(
        schedule: RuntimeSchedule,
        min_hold_us: u64,
        delivery_margin_us: u64,
        us_to_ticks: F,
    ) -> Self
    where
        F: Fn(u64) -> TimelineTicks,
    {
        Self::try_new_ticks(
            schedule,
            min_hold_us,
            DurationTicks::from_raw(us_to_ticks(min_hold_us).as_u64()),
            delivery_margin_us,
            DurationTicks::from_raw(us_to_ticks(delivery_margin_us).as_u64()),
            |microseconds| Ok(us_to_ticks(microseconds)),
        )
        .expect("legacy coordinator construction uses an infallible tick converter")
    }

    /// Construct the coordinator with all scheduling durations represented in
    /// the QPC tick domain. The microsecond arguments are retained only as
    /// immutable telemetry metadata; no deadline is derived from them.
    pub fn try_new_ticks<F>(
        schedule: RuntimeSchedule,
        min_hold_us: u64,
        min_hold_ticks: DurationTicks,
        delivery_margin_us: u64,
        delivery_margin_ticks: DurationTicks,
        us_to_ticks: F,
    ) -> Result<Self, CoordinatorError>
    where
        F: Fn(u64) -> Result<TimelineTicks, CoordinatorError>,
    {
        let generation_count = schedule.generation_count;
        let generation_states = vec![
            GenerationStatus::Scheduled;
            usize::try_from(generation_count)
                .map_err(|_| CoordinatorError::GenerationCountOverflow)?
        ]
        .into_boxed_slice();
        let batch_scheduled_ticks = schedule
            .batches
            .iter()
            .map(|b| us_to_ticks(b.scheduled_us))
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice();

        Ok(Self {
            schedule,
            min_hold_us,
            min_hold_ticks,
            delivery_margin_us,
            delivery_margin_ticks,
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

    fn active_for_slot(&self, slot: KeySlot) -> Option<&ActiveGeneration> {
        self.active_by_slot
            .get(slot as usize)
            .and_then(Option::as_ref)
    }

    pub fn recovery_offset_ticks(&self) -> DurationTicks {
        self.recovery_offset_ticks
    }

    pub fn effective_total_ticks(&self) -> Result<TimelineTicks, CoordinatorError> {
        self.batch_scheduled_ticks
            .last()
            .copied()
            .map_or(Ok(TimelineTicks::ZERO), |scheduled| {
                Ok(scheduled.checked_add_duration(self.recovery_offset_ticks)?)
            })
    }

    pub fn effective_batch_scheduled_ticks(
        &self,
        index: usize,
    ) -> Result<TimelineTicks, CoordinatorError> {
        self.batch_scheduled_ticks
            .get(index)
            .copied()
            .ok_or(CoordinatorError::InvalidBatchIndex { index })?
            .checked_add_duration(self.recovery_offset_ticks)
            .map_err(CoordinatorError::from)
    }

    /// Apply one checked lifecycle transition. Generation IDs are only used
    /// to address the compiler-owned contiguous ledger, never as a batch
    /// timestamp/index identity.
    pub fn transition_generation(
        &mut self,
        generation_id: GenerationId,
        expected: GenerationStatus,
        next: GenerationStatus,
    ) -> Result<(), CoordinatorError> {
        let Some(state) = self.generation_states.get_mut(generation_id as usize) else {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::UnknownGeneration {
                    generation_id,
                    generation_count: self.generation_count,
                },
            ));
        };
        if *state != expected {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::UnexpectedTransition {
                    generation_id,
                    expected,
                    actual: *state,
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
        *state = next;
        if next.is_terminal() {
            self.counters.increment(next);
        }
        Ok(())
    }

    fn terminalize(
        &mut self,
        generation_id: GenerationId,
        status: GenerationStatus,
    ) -> Result<(), CoordinatorError> {
        self.transition_generation(generation_id, GenerationStatus::Scheduled, status)
    }

    fn early_pop_blocked(&self, batch: &CompiledBatch) -> bool {
        if batch.kind != ActionKind::Down {
            return false;
        }
        if self.blocked_mask == 0 {
            return false;
        }
        self.schedule.intent_slice(batch).iter().any(|intent| {
            let bit = Self::bit_for_slot(intent.key_slot());
            self.blocked_mask & bit != 0
        })
    }

    #[cfg(test)]
    fn effective_authored_lead(scheduled_us: u64, requested_lead_us: u64) -> u64 {
        // The logical timeline is unsigned.  Applying a lead to an authored
        // timestamp smaller than that lead would saturate several distinct
        // deadlines to zero and could dispatch them as one burst.  The first
        // authored action is handled by the native worker's future physical
        // startup anchor; subsequent sub-lead actions stay ordered here.
        if scheduled_us >= requested_lead_us {
            requested_lead_us
        } else {
            0
        }
    }

    #[cfg(test)]
    pub fn next_authored_us(&self, dispatch_lead_us: u64) -> Option<u64> {
        if self.cursor >= self.schedule.batches.len() {
            return None;
        }
        let batch = &self.schedule.batches[self.cursor];
        let lead = dispatch_lead_us;
        if lead > 0 && self.early_pop_blocked(batch) {
            return Some(
                batch
                    .scheduled_us
                    .saturating_add(self.recovery_offset_ticks.as_u64()),
            );
        }
        let effective_scheduled_us = batch
            .scheduled_us
            .saturating_add(self.recovery_offset_ticks.as_u64());
        let effective_lead = Self::effective_authored_lead(effective_scheduled_us, lead);
        Some(effective_scheduled_us.saturating_sub(effective_lead))
    }

    /// Return the next authored dispatch deadline in the playback tick domain.
    /// The authored schedule remains immutable; recovery is represented only
    /// by `recovery_offset_ticks`.
    pub fn next_authored_ticks(
        &self,
        dispatch_lead: DurationTicks,
    ) -> Result<Option<TimelineTicks>, CoordinatorError> {
        let Some(batch) = self.schedule.batches.get(self.cursor) else {
            return Ok(None);
        };
        let authored = self
            .batch_scheduled_ticks
            .get(self.cursor)
            .copied()
            .ok_or(CoordinatorError::InvalidBatchIndex { index: self.cursor })?;
        let effective = authored.checked_add_duration(self.recovery_offset_ticks)?;
        if dispatch_lead == DurationTicks::ZERO || self.early_pop_blocked(batch) {
            return Ok(Some(effective));
        }
        let effective_lead = if effective >= TimelineTicks::from_raw(dispatch_lead.as_u64()) {
            dispatch_lead
        } else {
            DurationTicks::ZERO
        };
        Ok(Some(effective.checked_sub_duration(effective_lead)?))
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

    #[cfg(test)]
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

    pub fn next_pending_release_ticks(
        &self,
        lead_up: DurationTicks,
    ) -> Result<Option<TimelineTicks>, CoordinatorError> {
        self.pending_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .map(|pending| pending.get_effective_release_ticks(lead_up))
            .try_fold(None::<TimelineTicks>, |current, value| {
                let value = value?;
                Ok(Some(current.map_or(value, |best| best.min(value))))
            })
    }

    #[cfg(test)]
    pub fn pending_count_due_at(&self, deadline_us: u64, lead_up: u64) -> usize {
        self.pending_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .filter(|pending| pending.get_effective_release_us(lead_up) <= deadline_us)
            .count()
    }

    pub fn pending_count_due_at_ticks(
        &self,
        deadline: TimelineTicks,
        lead_up: DurationTicks,
    ) -> Result<usize, CoordinatorError> {
        self.pending_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .map(|pending| pending.get_effective_release_ticks(lead_up))
            .try_fold(0usize, |count, value| {
                Ok(count + usize::from(value? <= deadline))
            })
    }

    pub fn plan_pending_dispatch_ticks<F>(
        &self,
        lead_for_polyphony: F,
    ) -> Result<Option<PendingDispatchPlan>, CoordinatorError>
    where
        F: Fn(usize) -> Result<(DurationTicks, bool), CoordinatorError>,
    {
        if self.pending_mask == 0 {
            return Ok(None);
        }
        let mut polyphony = 1usize;
        for _ in 0..=MAX_KEYS {
            let (lead_ticks, lead_saturated) = lead_for_polyphony(polyphony)?;
            let deadline_ticks =
                self.next_pending_release_ticks(lead_ticks)?
                    .ok_or(CoordinatorError::Invariant(
                        CoordinatorInvariantError::Accounting(
                            "pending mask is set but no pending release exists".to_string(),
                        ),
                    ))?;
            let next_polyphony = self
                .pending_count_due_at_ticks(deadline_ticks, lead_ticks)?
                .clamp(1, MAX_KEYS);
            if next_polyphony == polyphony {
                return Ok(Some(PendingDispatchPlan {
                    deadline_ticks,
                    lead_ticks,
                    polyphony,
                    lead_saturated,
                }));
            }
            polyphony = next_polyphony;
        }
        let (lead_ticks, lead_saturated) = lead_for_polyphony(polyphony)?;
        let deadline_ticks =
            self.next_pending_release_ticks(lead_ticks)?
                .ok_or(CoordinatorError::Invariant(
                    CoordinatorInvariantError::Accounting(
                        "pending mask is set but no pending release exists".to_string(),
                    ),
                ))?;
        Ok(Some(PendingDispatchPlan {
            deadline_ticks,
            lead_ticks,
            polyphony,
            lead_saturated,
        }))
    }

    pub fn next_deadline_ticks(
        &self,
        dispatch_lead: DurationTicks,
        pending_plan: Option<&PendingDispatchPlan>,
    ) -> Result<Option<TimelineTicks>, CoordinatorError> {
        if self.release_recovery_started_ticks.is_some() {
            return Ok(pending_plan.map(|plan| plan.deadline_ticks));
        }
        let authored = self.next_authored_ticks(dispatch_lead)?;
        let pending = pending_plan.map(|plan| plan.deadline_ticks);
        Ok(match (authored, pending) {
            (Some(a), Some(p)) => Some(a.min(p)),
            (Some(a), None) => Some(a),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        })
    }

    /// Select the next release cohort by solving the lead/polyphony fixed
    /// point.  A larger cohort may receive a larger lead and therefore move
    /// the effective deadline earlier, so the cohort must be re-counted until
    /// stable.  The bound is tiny because the instrument has at most 15 keys.
    #[cfg(test)]
    pub fn plan_pending_dispatch<F>(&self, lead_for_polyphony: F) -> Option<PendingDispatchPlan>
    where
        F: Fn(usize) -> (u64, bool),
    {
        if self.pending_mask == 0 {
            return None;
        }

        let mut polyphony = 1usize;
        for _ in 0..=MAX_KEYS {
            let (lead_us, lead_saturated) = lead_for_polyphony(polyphony);
            let deadline_us = self.next_pending_release_us(lead_us)?;
            let next_polyphony = self.pending_count_due_at(deadline_us, lead_us).max(1);
            let plan = PendingDispatchPlan {
                deadline_ticks: TimelineTicks::from_raw(deadline_us),
                lead_ticks: DurationTicks::from_raw(lead_us),
                polyphony,
                lead_saturated,
            };
            if next_polyphony == polyphony {
                return Some(plan);
            }
            polyphony = next_polyphony.min(MAX_KEYS);
        }

        // The monotonic estimator should converge within MAX_KEYS steps.  If
        // a future custom estimator violates that assumption, return the last
        // bounded plan rather than looping on the real-time worker.
        let (lead_us, lead_saturated) = lead_for_polyphony(polyphony);
        Some(PendingDispatchPlan {
            deadline_ticks: TimelineTicks::from_raw(self.next_pending_release_us(lead_us)?),
            lead_ticks: DurationTicks::from_raw(lead_us),
            polyphony,
            lead_saturated,
        })
    }

    #[cfg(test)]
    pub fn next_deadline_with_pending_plan(
        &self,
        dispatch_lead_us: u64,
        pending_plan: Option<&PendingDispatchPlan>,
    ) -> Option<u64> {
        if self.release_recovery_active() {
            return pending_plan.map(|plan| plan.deadline_ticks.as_u64());
        }
        let authored = self.next_authored_us(dispatch_lead_us);
        let pending = pending_plan.map(|plan| plan.deadline_ticks.as_u64());
        match (authored, pending) {
            (Some(a), Some(p)) => Some(a.min(p)),
            (Some(a), None) => Some(a),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        }
    }

    #[cfg(test)]
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

    /// Verify the compact masks and terminal ledger agree exactly.
    pub fn check_invariants(&self) -> Result<(), CoordinatorInvariantError> {
        if self.generation_states.len() as u64 != self.generation_count {
            return Err(CoordinatorInvariantError::Accounting(format!(
                "ledger length {} != generation count {}",
                self.generation_states.len(),
                self.generation_count
            )));
        }

        let mut active = 0u64;
        let mut release_pending = 0u64;
        let mut terminal = 0u64;
        for state in &self.generation_states {
            match state {
                GenerationStatus::Active => active += 1,
                GenerationStatus::ReleasePending => release_pending += 1,
                GenerationStatus::Released
                | GenerationStatus::DroppedConflict
                | GenerationStatus::DroppedBackend
                | GenerationStatus::DroppedExpired
                | GenerationStatus::Cancelled => terminal += 1,
                GenerationStatus::Scheduled => {}
            }
        }
        if active != u64::from(self.active_mask.count_ones()) {
            return Err(CoordinatorInvariantError::Accounting(format!(
                "active ledger count {active} != active mask count {}",
                self.active_mask.count_ones()
            )));
        }
        if release_pending != u64::from(self.pending_mask.count_ones()) {
            return Err(CoordinatorInvariantError::Accounting(format!(
                "pending ledger count {release_pending} != pending mask count {}",
                self.pending_mask.count_ones()
            )));
        }
        if self.active_mask & self.pending_mask != 0 {
            return Err(CoordinatorInvariantError::Accounting(
                "active and pending masks overlap".to_string(),
            ));
        }
        if terminal != self.counters.terminal_total() {
            return Err(CoordinatorInvariantError::Accounting(format!(
                "terminal ledger count {terminal} != counters {}",
                self.counters.terminal_total()
            )));
        }
        Ok(())
    }

    /// Build a `HashMap<String, u64>` generation status summary compatible with
    /// the existing Python/snapshot API. Counts come directly from the checked
    /// generation ledger, so no subtraction or saturating arithmetic can hide
    /// an accounting mismatch.
    ///
    /// No `HashMap` is touched during the hot dispatch loop; this method is only
    /// called at snapshot/telemetry publish time.
    pub fn generation_status_counts(&self) -> std::collections::HashMap<String, u64> {
        let mut result = std::collections::HashMap::with_capacity(ALL_GENERATION_STATUSES.len());
        for status in ALL_GENERATION_STATUSES {
            result.insert(status.as_str().to_string(), 0);
        }
        for state in &self.generation_states {
            *result
                .get_mut(state.as_str())
                .expect("all generation states have a summary bucket") += 1;
        }
        result
    }

    #[cfg(test)]
    pub fn pop_due_pending(
        &mut self,
        now_us: u64,
        lead_up: u64,
    ) -> SmallVec<[PendingRelease; MAX_KEYS]> {
        self.pop_due_pending_until(now_us, lead_up)
    }

    #[cfg(test)]
    pub fn pop_due_pending_with_plan(
        &mut self,
        now_us: u64,
        plan: &PendingDispatchPlan,
    ) -> SmallVec<[PendingRelease; MAX_KEYS]> {
        self.pop_due_pending_until(
            now_us.min(plan.deadline_ticks.as_u64()),
            plan.lead_ticks.as_u64(),
        )
    }

    #[cfg(test)]
    fn pop_due_pending_until(
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

    pub fn pop_due_pending_ticks(
        &mut self,
        now: TimelineTicks,
        plan: &PendingDispatchPlan,
    ) -> Result<SmallVec<[PendingRelease; MAX_KEYS]>, CoordinatorError> {
        if self.pending_mask == 0 {
            return Ok(SmallVec::new());
        }
        let limit = now.min(plan.deadline_ticks);
        let mut due_with_deadline: SmallVec<[(PendingRelease, TimelineTicks); MAX_KEYS]> =
            SmallVec::new();
        for pending in self.pending_by_slot.iter().filter_map(Option::as_ref) {
            let deadline = pending.get_effective_release_ticks(plan.lead_ticks)?;
            if deadline <= limit {
                due_with_deadline.push((pending.clone(), deadline));
            }
        }
        due_with_deadline.sort_by_key(|(pending, deadline)| {
            (*deadline, pending.source_action_index, pending.scan_code)
        });
        let due: SmallVec<[PendingRelease; MAX_KEYS]> = due_with_deadline
            .iter()
            .map(|(pending, _)| pending.clone())
            .collect();
        for pending in &due {
            let slot = usize::from(pending.key_slot);
            let Some(current) = self.pending_by_slot.get(slot).and_then(Option::as_ref) else {
                return Err(CoordinatorError::InvalidKeySlot {
                    slot: pending.key_slot,
                });
            };
            if current.generation_id != pending.generation_id {
                return Err(CoordinatorError::Invariant(
                    CoordinatorInvariantError::UnexpectedTransition {
                        generation_id: pending.generation_id,
                        expected: GenerationStatus::ReleasePending,
                        actual: GenerationStatus::ReleasePending,
                        next: GenerationStatus::Released,
                    },
                ));
            }
            self.pending_by_slot[slot] = None;
            self.pending_mask &= !Self::bit_for_slot(pending.key_slot);
        }
        Ok(due)
    }

    #[cfg(test)]
    pub fn pop_next_due_authored(
        &mut self,
        now_us: u64,
        dispatch_lead_us: u64,
    ) -> Option<(RuntimeBatch, u64)> {
        let (index, lead) = self
            .pop_next_due_authored_ticks(
                TimelineTicks::from_raw(now_us),
                DurationTicks::from_raw(dispatch_lead_us),
            )
            .ok()??;
        let popped = self
            .schedule
            .materialize_batch(index, self.recovery_offset_ticks.as_u64());
        Some((popped, lead.as_u64()))
    }

    /// Variant of [`pop_next_due_authored`] that returns the batch index.
    ///
    /// This avoids returning a borrow tied to `&mut self`, allowing the caller
    /// to immutably borrow the schedule via `coordinator.schedule.view_batch(...)`
    /// and then call other `&self` methods like `check_down_conflicts_compact`.
    #[cfg(test)]
    pub fn pop_next_due_authored_index(
        &mut self,
        now_us: u64,
        dispatch_lead_us: u64,
    ) -> Option<(usize, u64)> {
        self.pop_next_due_authored_ticks(
            TimelineTicks::from_raw(now_us),
            DurationTicks::from_raw(dispatch_lead_us),
        )
        .ok()?
        .map(|(index, lead)| (index, lead.as_u64()))
    }

    pub fn pop_next_due_authored_ticks(
        &mut self,
        now: TimelineTicks,
        dispatch_lead: DurationTicks,
    ) -> Result<Option<(usize, DurationTicks)>, CoordinatorError> {
        if self.cursor >= self.schedule.batches.len()
            || self.release_recovery_started_ticks.is_some()
        {
            return Ok(None);
        }
        let index = self.cursor;
        let batch = self
            .schedule
            .batches
            .get(index)
            .ok_or(CoordinatorError::InvalidBatchIndex { index })?;
        let authored = self
            .batch_scheduled_ticks
            .get(index)
            .copied()
            .ok_or(CoordinatorError::InvalidBatchIndex { index })?
            .checked_add_duration(self.recovery_offset_ticks)?;
        let effective_lead = if authored >= TimelineTicks::from_raw(dispatch_lead.as_u64()) {
            dispatch_lead
        } else {
            DurationTicks::ZERO
        };
        let deadline = authored.checked_sub_duration(effective_lead)?;
        if deadline > now || (authored > now && self.early_pop_blocked(batch)) {
            return Ok(None);
        }
        self.cursor = self.cursor.checked_add(1).ok_or(CoordinatorError::Time(
            crate::time::TimeArithmeticError::Overflow,
        ))?;
        Ok(Some((index, effective_lead)))
    }

    /// Check whether any intent in `compact_intents` conflicts with an
    /// already-active key slot.
    ///
    /// Returns a bitmask (`u16`) where each set bit corresponds to a key slot
    /// that is currently active.  A return value of `0` means no conflicts.
    ///
    /// This is the hot-path alternative to [`split_down_intents`] —
    /// it operates directly on the compact arena slice with one bitwise AND
    /// per intent and produces no allocation.
    pub fn check_down_conflicts_compact(&self, compact_intents: &[CompactIntent]) -> u16 {
        if self.blocked_mask == 0 {
            return 0;
        }
        let mut conflict_mask: u16 = 0;
        for compact in compact_intents {
            let bit = Self::bit_for_slot(compact.key_slot());
            if self.blocked_mask & bit != 0 {
                conflict_mask |= bit;
            }
        }
        conflict_mask
    }

    /// Terminalize the generations associated with every slot set in
    /// `conflict_mask` as `DroppedConflict`.
    ///
    /// Called after [`check_down_conflicts_compact`] returns a non-zero mask.
    /// Updating counters is the only side-effect; no mask bits are cleared
    /// (the slots were never activated for the conflicting batch).
    pub fn terminalize_conflicted_slots(
        &mut self,
        compact_intents: &[CompactIntent],
        conflict_mask: u16,
    ) -> Result<(), CoordinatorError> {
        if conflict_mask == 0 {
            return Ok(());
        }
        for compact in compact_intents {
            if conflict_mask & Self::bit_for_slot(compact.key_slot()) != 0 {
                // Only Down intents with a generation ID need terminalizing.
                if compact.generation_id() != NO_GENERATION_ID {
                    self.terminalize(compact.generation_id(), GenerationStatus::DroppedConflict)?;
                }
            }
        }
        Ok(())
    }

    /// Activate sent downs from a batch index without allocating.
    ///
    /// `sent_scan_codes` is the slice returned by `SendInput` bookkeeping —
    /// only keys present in this slice are activated.  Keys in the batch
    /// that were not sent are terminated as `DroppedBackend`.
    #[allow(clippy::too_many_arguments)]
    pub fn activate_sent_downs_compact_ticks(
        &mut self,
        batch_index: usize,
        sent_scan_codes: &[u16],
        dispatch_started: TimelineTicks,
        dispatch_completed: TimelineTicks,
        excluded_mask: u16,
    ) -> Result<(), CoordinatorError> {
        let release_not_before_ticks = dispatch_completed
            .checked_add_duration(self.min_hold_ticks)
            .and_then(|ticks| ticks.checked_add_duration(self.delivery_margin_ticks))?;
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
            let Some(sc) = self.schedule.key_registry.scan_code_for(slot) else {
                return Err(CoordinatorError::InvalidKeySlot { slot });
            };
            if !sent_scan_codes.contains(&sc) {
                self.transition_generation(
                    generation_id,
                    GenerationStatus::Scheduled,
                    GenerationStatus::DroppedBackend,
                )?;
                continue;
            }
            self.transition_generation(
                generation_id,
                GenerationStatus::Scheduled,
                GenerationStatus::Active,
            )?;
            self.active_by_slot[usize::from(slot)] = Some(ActiveGeneration {
                generation_id,
                scan_code: sc,
                key_slot: slot,
                source_action_index,
                scheduled_down_ticks: self
                    .batch_scheduled_ticks
                    .get(batch_index)
                    .copied()
                    .ok_or(CoordinatorError::InvalidBatchIndex { index: batch_index })?
                    .checked_add_duration(self.recovery_offset_ticks)?,
                down_dispatch_started_ticks: dispatch_started,
                down_dispatch_completed_ticks: dispatch_completed,
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
        dispatch_started_ticks: TimelineTicks,
        dispatch_completed_ticks: TimelineTicks,
    ) -> Result<(), CoordinatorError> {
        let release_not_before_ticks = dispatch_completed_ticks
            .checked_add_duration(self.min_hold_ticks)
            .and_then(|ticks| ticks.checked_add_duration(self.delivery_margin_ticks))?;

        if sent_scan_codes.len() == 1 {
            let only_sent = sent_scan_codes[0];
            for intent in intents {
                let Some(generation_id) = intent.generation_id else {
                    continue;
                };
                if intent.scan_code != only_sent {
                    // Terminalize without touching any mask — slot was never activated.
                    self.transition_generation(
                        generation_id,
                        GenerationStatus::Scheduled,
                        GenerationStatus::DroppedBackend,
                    )?;
                    continue;
                }
                self.transition_generation(
                    generation_id,
                    GenerationStatus::Scheduled,
                    GenerationStatus::Active,
                )?;
                self.active_by_slot[intent.key_slot as usize] = Some(ActiveGeneration {
                    generation_id,
                    scan_code: intent.scan_code,
                    key_slot: intent.key_slot,
                    source_action_index: intent.source_action_index,
                    scheduled_down_ticks: self
                        .batch_scheduled_ticks
                        .get(intent.compiled_batch_index)
                        .copied()
                        .ok_or(CoordinatorError::InvalidBatchIndex {
                            index: intent.compiled_batch_index,
                        })?
                        .checked_add_duration(self.recovery_offset_ticks)?,
                    down_dispatch_started_ticks: dispatch_started_ticks,
                    down_dispatch_completed_ticks: dispatch_completed_ticks,
                    release_not_before_ticks,
                });
                self.active_mask |= Self::bit_for_slot(intent.key_slot);
                self.blocked_mask |= Self::bit_for_slot(intent.key_slot);
                // No HashMap insertion — active count is derived from active_mask at query time.
            }
            return Ok(());
        }

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
                continue;
            }
            self.transition_generation(
                generation_id,
                GenerationStatus::Scheduled,
                GenerationStatus::Active,
            )?;
            self.active_by_slot[intent.key_slot as usize] = Some(ActiveGeneration {
                generation_id,
                scan_code: intent.scan_code,
                key_slot: intent.key_slot,
                source_action_index: intent.source_action_index,
                scheduled_down_ticks: self
                    .batch_scheduled_ticks
                    .get(intent.compiled_batch_index)
                    .copied()
                    .ok_or(CoordinatorError::InvalidBatchIndex {
                        index: intent.compiled_batch_index,
                    })?
                    .checked_add_duration(self.recovery_offset_ticks)?,
                down_dispatch_started_ticks: dispatch_started_ticks,
                down_dispatch_completed_ticks: dispatch_completed_ticks,
                release_not_before_ticks,
            });
            self.active_mask |= Self::bit_for_slot(intent.key_slot);
            self.blocked_mask |= Self::bit_for_slot(intent.key_slot);
            // No HashMap insertion — active count is derived from active_mask at query time.
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

    pub fn split_down_intents(
        &mut self,
        intents: &[RuntimeKeyIntent],
    ) -> Result<SplitIntentResult, CoordinatorError> {
        if self.blocked_mask == 0 {
            return Ok((intents.iter().cloned().collect(), SmallVec::new()));
        }
        let mut playable = SmallVec::new();
        let mut conflicts = SmallVec::new();

        for intent in intents {
            if self.blocked_mask & Self::bit_for_slot(intent.key_slot) != 0 {
                conflicts.push(intent.clone());
                if let Some(gen_id) = intent.generation_id {
                    self.terminalize(gen_id, GenerationStatus::DroppedConflict)?;
                }
            } else {
                playable.push(intent.clone());
            }
        }
        Ok((playable, conflicts))
    }

    /// Terminalize every generation in a conflicted authored chord without
    /// sending a playable subset. Accuracy-first callers use this when a
    /// partial chord would be worse than dropping the whole chord.
    pub fn drop_conflicted_downs(
        &mut self,
        intents: &[RuntimeKeyIntent],
    ) -> Result<(), CoordinatorError> {
        for intent in intents {
            if let Some(generation_id) = intent.generation_id {
                self.terminalize(generation_id, GenerationStatus::DroppedConflict)?;
            }
        }
        Ok(())
    }

    pub fn drop_expired_downs(
        &mut self,
        intents: &[RuntimeKeyIntent],
    ) -> Result<(), CoordinatorError> {
        for intent in intents {
            if let Some(gen_id) = intent.generation_id {
                self.terminalize(gen_id, GenerationStatus::DroppedExpired)?;
            }
        }
        Ok(())
    }

    pub fn request_releases(
        &mut self,
        intents: &[RuntimeKeyIntent],
    ) -> Result<ReleaseRequestResult, CoordinatorError> {
        if intents.len() == 1 {
            let intent = &intents[0];
            let Some(generation_id) = intent.generation_id else {
                return Ok((SmallVec::new(), std::iter::once(intent.clone()).collect()));
            };
            let active = self.active_for_slot(intent.key_slot).cloned();
            let Some(active) = active else {
                return Ok((SmallVec::new(), std::iter::once(intent.clone()).collect()));
            };
            if active.generation_id != generation_id {
                return Ok((SmallVec::new(), std::iter::once(intent.clone()).collect()));
            }
            self.transition_generation(
                generation_id,
                GenerationStatus::Active,
                GenerationStatus::ReleasePending,
            )?;

            let pending = PendingRelease {
                generation_id,
                scan_code: intent.scan_code,
                key_slot: intent.key_slot,
                source_action_index: intent.source_action_index,
                packet_id: intent.packet_id,
                scheduled_release_us: intent.scheduled_us,
                scheduled_release_ticks: self
                    .batch_scheduled_ticks
                    .get(intent.compiled_batch_index)
                    .copied()
                    .ok_or(CoordinatorError::InvalidBatchIndex {
                        index: intent.compiled_batch_index,
                    })?
                    .checked_add_duration(self.recovery_offset_ticks)?,
                down_dispatch_started_ticks: active.down_dispatch_started_ticks,
                release_not_before_ticks: active.release_not_before_ticks,
                reason_id: intent.reason_id,
                retry_count: 0,
                next_retry_ticks: crate::time::TimelineTicks::ZERO,
                first_failure_ticks: None,
                last_win32_error: None,
            };

            self.pending_by_slot[intent.key_slot as usize] = Some(pending.clone());
            self.pending_mask |= Self::bit_for_slot(intent.key_slot);
            self.active_mask &= !Self::bit_for_slot(intent.key_slot);
            // No HashMap insertion — release_pending count is derived from pending_mask.
            return Ok((std::iter::once(pending).collect(), SmallVec::new()));
        }

        let mut requested = SmallVec::new();
        let mut suppressed = SmallVec::new();

        for intent in intents {
            let Some(generation_id) = intent.generation_id else {
                suppressed.push(intent.clone());
                continue;
            };
            let active = self.active_for_slot(intent.key_slot).cloned();
            let Some(active) = active else {
                suppressed.push(intent.clone());
                continue;
            };
            if active.generation_id != generation_id {
                suppressed.push(intent.clone());
                continue;
            }
            self.transition_generation(
                generation_id,
                GenerationStatus::Active,
                GenerationStatus::ReleasePending,
            )?;

            let pending = PendingRelease {
                generation_id,
                scan_code: intent.scan_code,
                key_slot: intent.key_slot,
                source_action_index: intent.source_action_index,
                packet_id: intent.packet_id,
                scheduled_release_us: intent.scheduled_us,
                scheduled_release_ticks: self
                    .batch_scheduled_ticks
                    .get(intent.compiled_batch_index)
                    .copied()
                    .ok_or(CoordinatorError::InvalidBatchIndex {
                        index: intent.compiled_batch_index,
                    })?
                    .checked_add_duration(self.recovery_offset_ticks)?,
                down_dispatch_started_ticks: active.down_dispatch_started_ticks,
                release_not_before_ticks: active.release_not_before_ticks,
                reason_id: intent.reason_id,
                retry_count: 0,
                next_retry_ticks: crate::time::TimelineTicks::ZERO,
                first_failure_ticks: None,
                last_win32_error: None,
            };

            self.pending_by_slot[intent.key_slot as usize] = Some(pending.clone());
            self.pending_mask |= Self::bit_for_slot(intent.key_slot);
            self.active_mask &= !Self::bit_for_slot(intent.key_slot);
            // No HashMap insertion — release_pending count is derived from pending_mask.
            requested.push(pending);
        }

        Ok((requested, suppressed))
    }

    pub fn complete_releases(
        &mut self,
        releases: &[PendingRelease],
        sent_scan_codes: &[u16],
        skipped_scan_codes: &[u16],
    ) -> Result<(), CoordinatorError> {
        for pending in releases {
            let in_sent = sent_scan_codes.contains(&pending.scan_code);
            let in_skipped = skipped_scan_codes.contains(&pending.scan_code);
            if !in_sent && !in_skipped {
                continue;
            }
            let status = if in_sent {
                GenerationStatus::Released
            } else {
                GenerationStatus::DroppedBackend
            };
            self.transition_generation(
                pending.generation_id,
                GenerationStatus::ReleasePending,
                status,
            )?;
            if matches!(self.active_for_slot(pending.key_slot), Some(active) if active.generation_id == pending.generation_id)
            {
                self.active_by_slot[pending.key_slot as usize] = None;
                self.active_mask &= !Self::bit_for_slot(pending.key_slot);
                self.blocked_mask &= !Self::bit_for_slot(pending.key_slot);
            }
            self.pending_mask &= !Self::bit_for_slot(pending.key_slot);
            self.pending_by_slot[pending.key_slot as usize] = None;
        }
        Ok(())
    }

    pub fn release_recovery_active(&self) -> bool {
        self.release_recovery_started_ticks.is_some()
    }

    /// End a recovery pause after the pending release set is empty.
    ///
    /// The authored schedule is immutable.  A single offset moves the
    /// effective playback timeline, so recovery remains O(1) regardless of
    /// how many batches are still queued.
    #[cfg(test)]
    pub fn finish_release_recovery(
        &mut self,
        completed_us: u64,
    ) -> Result<Option<u64>, CoordinatorError> {
        self.finish_release_recovery_ticks(TimelineTicks::from_raw(completed_us))
            .map(|pause| pause.map(DurationTicks::as_u64))
    }

    pub fn finish_release_recovery_ticks(
        &mut self,
        completed: TimelineTicks,
    ) -> Result<Option<DurationTicks>, CoordinatorError> {
        if self.pending_mask != 0 {
            return Ok(None);
        }
        let Some(started) = self.release_recovery_started_ticks.take() else {
            return Ok(None);
        };
        let pause = completed.checked_duration_since(started)?;
        self.recovery_offset_ticks = self.recovery_offset_ticks.checked_add(pause)?;
        Ok(Some(pause))
    }

    /// Requeue release work that did not reach the operating-system input
    /// stream. The active generation remains owned by the coordinator while
    /// bounded retries are pending; callers must stop playback and perform
    /// full-instrument recovery when this returns `true`.
    ///
    /// `recovery_started_us` is sampled immediately before the failed backend
    /// call. `retry_base_us` is sampled from backend completion and is used
    /// only to schedule the next retry after the call has returned.
    #[cfg(test)]
    pub fn requeue_failed_releases(
        &mut self,
        releases: &[PendingRelease],
        sent_scan_codes: &[u16],
        skipped_scan_codes: &[u16],
        recovery_started_us: u64,
        retry_base_us: u64,
        last_win32_error: Option<u32>,
    ) -> Result<bool, CoordinatorError> {
        let backoff = RELEASE_RETRY_BACKOFF_US.map(DurationTicks::from_raw);
        let recovery_required = self.requeue_failed_releases_ticks(
            releases,
            sent_scan_codes,
            skipped_scan_codes,
            TimelineTicks::from_raw(recovery_started_us),
            TimelineTicks::from_raw(retry_base_us),
            &backoff,
            last_win32_error,
        )?;
        Ok(recovery_required)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn requeue_failed_releases_ticks(
        &mut self,
        releases: &[PendingRelease],
        sent_scan_codes: &[u16],
        skipped_scan_codes: &[u16],
        recovery_started: TimelineTicks,
        retry_base: TimelineTicks,
        retry_backoff: &[DurationTicks],
        last_win32_error: Option<u32>,
    ) -> Result<bool, CoordinatorError> {
        if retry_backoff.is_empty() {
            return Err(CoordinatorError::Invariant(
                CoordinatorInvariantError::Accounting(
                    "release retry backoff must not be empty".to_string(),
                ),
            ));
        }
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
            let Some(retry_count) = pending.retry_count.checked_add(1) else {
                recovery_required = true;
                continue;
            };
            if retry_count > MAX_RELEASE_RETRIES {
                recovery_required = true;
                continue;
            }
            let delay_index = usize::from(retry_count - 1).min(retry_backoff.len() - 1);
            let next_retry = retry_base.checked_add_duration(retry_backoff[delay_index])?;
            let mut retry = pending.clone();
            self.release_recovery_started_ticks
                .get_or_insert(recovery_started);
            retry.retry_count = retry_count;
            retry.next_retry_ticks = next_retry;
            retry.first_failure_ticks =
                Some(pending.first_failure_ticks.unwrap_or(recovery_started));
            retry.last_win32_error = last_win32_error.or(pending.last_win32_error);
            let retry_slot = usize::from(retry.key_slot);
            if retry_slot >= MAX_KEYS {
                return Err(CoordinatorError::InvalidKeySlot {
                    slot: retry.key_slot,
                });
            }
            self.pending_mask |= Self::bit_for_slot(retry.key_slot);
            self.pending_by_slot[retry_slot] = Some(retry);
        }
        Ok(recovery_required)
    }

    pub fn cancel_all(&mut self) -> Result<Vec<GenerationId>, CoordinatorError> {
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

        for index in 0..self.generation_states.len() {
            let state = self.generation_states[index];
            if matches!(
                state,
                GenerationStatus::Scheduled
                    | GenerationStatus::Active
                    | GenerationStatus::ReleasePending
            ) {
                let generation_id = u64::try_from(index).map_err(|_| {
                    CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                        "generation ledger index does not fit GenerationId".to_string(),
                    ))
                })?;
                self.transition_generation(generation_id, state, GenerationStatus::Cancelled)?;
            }
        }

        self.active_by_slot.fill(None);
        self.pending_by_slot.fill(None);
        self.active_mask = 0;
        self.blocked_mask = 0;
        self.pending_mask = 0;
        self.release_recovery_started_ticks = None;

        Ok(sorted_cancelled)
    }

    /// Cancel only generations that currently own physical input state.
    ///
    /// Authored generations that have not been dispatched remain Scheduled,
    /// so a focus/manual suspension can resume the immutable authored cursor
    /// without ever attempting `Cancelled -> Active`.
    pub fn cancel_live_generations(&mut self) -> Result<Vec<GenerationId>, CoordinatorError> {
        let mut cancelled_ids: SmallVec<[GenerationId; MAX_KEYS * 2]> = self
            .generation_states
            .iter()
            .enumerate()
            .filter_map(|(index, state)| {
                matches!(
                    state,
                    GenerationStatus::Active | GenerationStatus::ReleasePending
                )
                .then_some(index)
            })
            .map(|index| {
                GenerationId::try_from(index).map_err(|_| {
                    CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(
                        "generation ledger index does not fit GenerationId".to_string(),
                    ))
                })
            })
            .collect::<Result<SmallVec<_>, _>>()?;
        cancelled_ids.sort_unstable();

        for generation_id in cancelled_ids.iter().copied() {
            let state = *self.generation_states.get(generation_id as usize).ok_or(
                CoordinatorError::Invariant(CoordinatorInvariantError::UnknownGeneration {
                    generation_id,
                    generation_count: self.generation_count,
                }),
            )?;
            self.transition_generation(generation_id, state, GenerationStatus::Cancelled)?;
        }

        self.active_by_slot.fill(None);
        self.pending_by_slot.fill(None);
        self.active_mask = 0;
        self.blocked_mask = 0;
        self.pending_mask = 0;
        self.release_recovery_started_ticks = None;
        self.check_invariants()?;

        Ok(cancelled_ids.into_vec())
    }
}
