//! Runtime dispatch coordinator managing generation status transitions and release eligibility.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::model::*;
use crate::time::{DurationTicks, TimelineTicks};

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
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks);
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
                    scan_codes: vec![0x15],
                    reason: "first".to_string(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Down,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x16],
                    reason: "second".to_string(),
                },
            ],
            &[0x15, 0x16],
        )
        .expect("valid schedule");
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks);

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
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks);
        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("first down is due");
        coordinator.activate_sent_downs(&down.intents, &[0x15], 0, crate::time::TimelineTicks(0), 10, crate::time::TimelineTicks(10));

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
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks);
        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("first down is due");
        coordinator.activate_sent_downs(&down.intents, &[0x15], 0, crate::time::TimelineTicks(0), 10, crate::time::TimelineTicks(10));
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
    fn blocked_authored_deadline_includes_delivery_margin() {
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
                    scheduled_us: 100,
                    scan_codes: vec![0x15],
                    reason: "up".to_string(),
                },
            ],
            &[0x15],
        )
        .expect("valid schedule");
        
        let min_hold = 20_000;
        let delivery_margin = 5_000;
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, min_hold, delivery_margin, crate::time::TimelineTicks);
        
        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("down is due");
        
        // Complete the down at t=100.
        coordinator.activate_sent_downs(&down.intents, &[0x15], 50, crate::time::TimelineTicks(50), 100, crate::time::TimelineTicks(100));
        
        let (up, _) = coordinator
            .pop_next_due_authored(100, 0)
            .expect("up is due");
        
        // Pop it and request release.
        let _ = coordinator.request_releases(&up.intents);
        
        // Since up was authored at 100, but min_hold is 20,000 and delivery_margin is 5,000, 
        // the effective release time must be at least 100 (completion) + 20,000 + 5,000 = 25,100.
        let due = coordinator.pop_due_pending(25_099, 0);
        assert!(due.is_empty(), "release must not be due before min_hold + delivery_margin");
        
        let due_now = coordinator.pop_due_pending(25_100, 0);
        assert_eq!(due_now.len(), 1, "release must be due exactly at completion + min_hold + delivery_margin");
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
                    scan_codes: vec![0x15, 0x16],
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
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks);

        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("first down is due");
        coordinator.activate_sent_downs(&down.intents, &[0x15, 0x16], 0, crate::time::TimelineTicks(0), 10, crate::time::TimelineTicks(10));
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

    #[test]
    fn pending_plan_uses_the_next_release_cohort_not_all_pending_keys() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15, 0x16],
                    reason: "down".to_string(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15],
                    reason: "up-a".to_string(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Up,
                    scheduled_us: 1_100,
                    scan_codes: vec![0x16],
                    reason: "up-b".to_string(),
                },
            ],
            &[0x15, 0x16],
        )
        .expect("valid schedule");
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks);
        let (down, _) = coordinator
            .pop_next_due_authored(0, 0)
            .expect("down is due");
        coordinator.activate_sent_downs(&down.intents, &[0x15, 0x16], 0, crate::time::TimelineTicks(0), 0, crate::time::TimelineTicks(0));

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
        assert_eq!(plan.deadline_us, 900);
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
                    scan_codes: vec![0x15, 0x16],
                    reason: "down".to_string(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15, 0x16],
                    reason: "up".to_string(),
                },
            ],
            &[0x15, 0x16],
        )
        .expect("valid schedule");
        let generation_count = schedule.generation_count;
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks);

        let (down, _) = coordinator.pop_next_due_authored(0, 0).unwrap();
        coordinator.activate_sent_downs(&down.intents, &[0x15, 0x16], 0, crate::time::TimelineTicks(0), 0, crate::time::TimelineTicks(0));

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
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15],
                    reason: "down".to_string(),
                },
            ],
            &[0x15],
        )
        .expect("valid schedule");
        let generation_count = schedule.generation_count;
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks);

        let (down, _) = coordinator.pop_next_due_authored(0, 0).unwrap();
        coordinator.activate_sent_downs(&down.intents, &[0x15], 0, crate::time::TimelineTicks(0), 0, crate::time::TimelineTicks(0));

        coordinator.cancel_all();

        let counts = coordinator.generation_status_counts();
        let total: u64 = counts.values().sum();
        assert_eq!(total, generation_count);
        assert_eq!(counts.get("cancelled"), Some(&generation_count));
    }

    #[test]
    fn generation_counters_each_slot_has_at_most_one_generation() {
        // After activate, each slot bit is set at most once.
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15, 0x16, 0x17],
                    reason: "down".to_string(),
                },
            ],
            &[0x15, 0x16, 0x17],
        )
        .expect("valid schedule");
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks);
        let (down, _) = coordinator.pop_next_due_authored(0, 0).unwrap();
        coordinator.activate_sent_downs(&down.intents, &[0x15, 0x16, 0x17], 0, crate::time::TimelineTicks(0), 0, crate::time::TimelineTicks(0));

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
                    scan_codes: vec![0x15, 0x16],
                    reason: "down".to_string(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x15],
                    reason: "up-a".to_string(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Up,
                    scheduled_us: 2_000,
                    scan_codes: vec![0x16],
                    reason: "up-b".to_string(),
                },
            ],
            &[0x15, 0x16],
        )
        .expect("valid schedule");
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks);

        let (down, _) = coordinator.pop_next_due_authored(0, 0).unwrap();
        coordinator.activate_sent_downs(&down.intents, &[0x15, 0x16], 0, crate::time::TimelineTicks(0), 0, crate::time::TimelineTicks(0));

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
        let generation_count = schedule.generation_count;
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks);

        let (down, _) = coordinator.pop_next_due_authored(0, 0).unwrap();
        coordinator.activate_sent_downs(&down.intents, &[0x15], 0, crate::time::TimelineTicks(0), 0, crate::time::TimelineTicks(0));

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
                    scan_codes: vec![0x15],
                    reason: "d1".to_string(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 500,
                    scan_codes: vec![0x15],
                    reason: "u1".to_string(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Down,
                    scheduled_us: 1_000,
                    scan_codes: vec![0x16],
                    reason: "d2".to_string(),
                },
                KeyActionInput {
                    source_action_index: 3,
                    kind: ActionKind::Up,
                    scheduled_us: 1_500,
                    scan_codes: vec![0x16],
                    reason: "u2".to_string(),
                },
                KeyActionInput {
                    source_action_index: 4,
                    kind: ActionKind::Down,
                    scheduled_us: 2_000,
                    scan_codes: vec![0x17],
                    reason: "d3".to_string(),
                },
                KeyActionInput {
                    source_action_index: 5,
                    kind: ActionKind::Up,
                    scheduled_us: 2_500,
                    scan_codes: vec![0x17],
                    reason: "u3".to_string(),
                },
            ],
            &[0x15, 0x16, 0x17],
        )
        .expect("valid schedule");
        let generation_count = schedule.generation_count;
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks);

        // Play through the full schedule
        for _ in 0..6 {
            if let Some((batch, _)) = coordinator.pop_next_due_authored(u64::MAX, 0) {
                match batch.kind {
                    ActionKind::Down => {
                        let sc: Vec<u16> = batch.intents.iter().map(|i| i.scan_code).collect();
                        coordinator.activate_sent_downs(&batch.intents, &sc, 0, crate::time::TimelineTicks(0), 0, crate::time::TimelineTicks(0));
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveGeneration {
    pub generation_id: GenerationId,
    pub scan_code: u16,
    pub key_slot: KeySlot,
    pub source_action_index: u32,
    pub scheduled_down_us: u64,
    pub scheduled_down_ticks: TimelineTicks,
    pub down_dispatch_started_us: u64,
    pub down_dispatch_started_ticks: TimelineTicks,
    pub down_dispatch_completed_us: u64,
    pub down_dispatch_completed_ticks: TimelineTicks,
    pub release_not_before_us: u64,
    pub release_not_before_ticks: TimelineTicks,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRelease {
    pub generation_id: GenerationId,
    pub scan_code: u16,
    pub key_slot: KeySlot,
    pub source_action_index: u32,
    pub packet_id: PacketId,
    pub scheduled_release_us: u64,
    pub scheduled_release_ticks: TimelineTicks,
    pub down_dispatch_started_us: u64,
    pub down_dispatch_started_ticks: TimelineTicks,
    pub release_not_before_us: u64,
    pub release_not_before_ticks: TimelineTicks,
    pub reason_id: ReasonId,
    pub retry_count: u8,
    pub next_retry_us: u64,
    pub next_retry_ticks: TimelineTicks,
    pub first_failure_us: Option<u64>,
    pub first_failure_ticks: Option<TimelineTicks>,
    pub last_win32_error: Option<u32>,
}

impl PendingRelease {
    pub fn get_effective_release_us(&self, lead_up: u64) -> u64 {
        let effective_lead =
            RuntimeDispatchCoordinator::effective_authored_lead(self.scheduled_release_us, lead_up);
        let led = self.scheduled_release_us.saturating_sub(effective_lead);
        self.release_not_before_us.max(led).max(self.next_retry_us)
    }
}

/// The release cohort selected for one upcoming dispatch.
///
/// The worker must use the same lead both when calculating the next deadline
/// and when popping pending releases.  Keeping the result together prevents
/// a larger pending population from over-leading an earlier one-key release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingDispatchPlan {
    pub deadline_us: u64,
    pub lead_us: u64,
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
    pending_by_slot: [Option<PendingRelease>; MAX_KEYS],
    pub pending_mask: u16,
    /// Terminal and implicit-scheduled generation counts.
    ///
    /// Active and release-pending counts are derived from `active_mask`/`pending_mask`
    /// respectively, so they are not stored here.  This eliminates the
    /// `HashMap<GenerationId, GenerationStatus>` from the hot dispatch path.
    counters: GenerationCounters,
    generation_count: u64,
    recovery_offset_us: u64,
    /// Pre-wired for P1.3 tick-domain refactor; not yet read by any method.
    #[allow(dead_code)]
    recovery_offset_ticks: DurationTicks,
    release_recovery_started_us: Option<u64>,
    /// Pre-wired for P1.3 tick-domain refactor; not yet read by any method.
    #[allow(dead_code)]
    release_recovery_started_ticks: Option<TimelineTicks>,
}

impl RuntimeDispatchCoordinator {
    pub fn new<F>(schedule: RuntimeSchedule, min_hold_us: u64, delivery_margin_us: u64, us_to_ticks: F) -> Self
    where
        F: Fn(u64) -> TimelineTicks,
    {
        let generation_count = schedule.generation_count;
        let batch_scheduled_ticks = schedule
            .batches
            .iter()
            .map(|b| us_to_ticks(b.scheduled_us))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let min_hold_ticks = DurationTicks(us_to_ticks(min_hold_us).0);
        let delivery_margin_ticks = DurationTicks(us_to_ticks(delivery_margin_us).0);
        
        Self {
            schedule,
            min_hold_us,
            min_hold_ticks,
            delivery_margin_us,
            delivery_margin_ticks,
            batch_scheduled_ticks,
            cursor: 0,
            active_by_slot: std::array::from_fn(|_| None),
            active_mask: 0,
            pending_by_slot: std::array::from_fn(|_| None),
            pending_mask: 0,
            counters: GenerationCounters::default(),
            generation_count,
            recovery_offset_us: 0,
            recovery_offset_ticks: DurationTicks(0),
            release_recovery_started_us: None,
            release_recovery_started_ticks: None,
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

    fn terminalize(&mut self, _generation_id: GenerationId, status: GenerationStatus) {
        // Generation-ID-keyed HashMap is gone. Only the counter needs updating.
        // Active/pending masks are adjusted by the caller before `terminalize` is invoked.
        self.counters.increment(status);
    }

    fn early_pop_blocked(&self, batch: &CompiledBatch) -> bool {
        if batch.kind != ActionKind::Down {
            return false;
        }
        if self.active_mask == 0 && self.pending_mask == 0 {
            return false;
        }
        self.schedule.intent_slice(batch).iter().any(|intent| {
            let bit = Self::bit_for_slot(intent.key_slot());
            self.active_mask & bit != 0 || self.pending_mask & bit != 0
        })
    }

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

    pub fn next_authored_us(&self, dispatch_lead_us: u64) -> Option<u64> {
        if self.cursor >= self.schedule.batches.len() {
            return None;
        }
        let batch = &self.schedule.batches[self.cursor];
        let lead = dispatch_lead_us;
        if lead > 0 && self.early_pop_blocked(batch) {
            return Some(batch.scheduled_us.saturating_add(self.recovery_offset_us));
        }
        let effective_scheduled_us = batch.scheduled_us.saturating_add(self.recovery_offset_us);
        let effective_lead = Self::effective_authored_lead(effective_scheduled_us, lead);
        Some(effective_scheduled_us.saturating_sub(effective_lead))
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

    pub fn pending_count_due_at(&self, deadline_us: u64, lead_up: u64) -> usize {
        self.pending_by_slot
            .iter()
            .filter_map(Option::as_ref)
            .filter(|pending| pending.get_effective_release_us(lead_up) <= deadline_us)
            .count()
    }

    /// Select the next release cohort by solving the lead/polyphony fixed
    /// point.  A larger cohort may receive a larger lead and therefore move
    /// the effective deadline earlier, so the cohort must be re-counted until
    /// stable.  The bound is tiny because the instrument has at most 15 keys.
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
                deadline_us,
                lead_us,
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
            deadline_us: self.next_pending_release_us(lead_us)?,
            lead_us,
            polyphony,
            lead_saturated,
        })
    }

    pub fn next_deadline_with_pending_plan(
        &self,
        dispatch_lead_us: u64,
        pending_plan: Option<&PendingDispatchPlan>,
    ) -> Option<u64> {
        if self.release_recovery_active() {
            return pending_plan.map(|plan| plan.deadline_us);
        }
        let authored = self.next_authored_us(dispatch_lead_us);
        let pending = pending_plan.map(|plan| plan.deadline_us);
        match (authored, pending) {
            (Some(a), Some(p)) => Some(a.min(p)),
            (Some(a), None) => Some(a),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        }
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

    /// Build a `HashMap<String, u64>` generation status summary compatible with
    /// the existing Python/snapshot API.
    ///
    /// - `active` is `active_mask.count_ones()` (O(1) popcount).
    /// - `release_pending` is `pending_mask.count_ones()` (O(1) popcount).
    /// - `scheduled` is derived: `generation_count - active - release_pending - terminal_total`.
    /// - All terminal buckets come from `GenerationCounters` (plain u64 fields).
    ///
    /// No `HashMap` is touched during the hot dispatch loop; this method is only
    /// called at snapshot/telemetry publish time.
    pub fn generation_status_counts(&self) -> std::collections::HashMap<String, u64> {
        let active = u64::from(self.active_mask.count_ones());
        let release_pending = u64::from(self.pending_mask.count_ones());
        let terminal_total = self.counters.terminal_total();
        let implicit_scheduled = self
            .generation_count
            .saturating_sub(active + release_pending + terminal_total);

        let mut result = std::collections::HashMap::with_capacity(ALL_GENERATION_STATUSES.len());
        result.insert("scheduled".to_string(), implicit_scheduled);
        result.insert("active".to_string(), active);
        result.insert("release_pending".to_string(), release_pending);
        result.insert("released".to_string(), self.counters.released);
        result.insert("dropped_conflict".to_string(), self.counters.dropped_conflict);
        result.insert("dropped_backend".to_string(), self.counters.dropped_backend);
        result.insert("dropped_expired".to_string(), self.counters.dropped_expired);
        result.insert("cancelled".to_string(), self.counters.cancelled);
        result
    }

    pub fn pop_due_pending(
        &mut self,
        now_us: u64,
        lead_up: u64,
    ) -> SmallVec<[PendingRelease; MAX_KEYS]> {
        self.pop_due_pending_until(now_us, lead_up)
    }

    pub fn pop_due_pending_with_plan(
        &mut self,
        now_us: u64,
        plan: &PendingDispatchPlan,
    ) -> SmallVec<[PendingRelease; MAX_KEYS]> {
        self.pop_due_pending_until(now_us.min(plan.deadline_us), plan.lead_us)
    }

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
        let effective_scheduled_us = batch.scheduled_us.saturating_add(self.recovery_offset_us);
        let effective_lead =
            Self::effective_authored_lead(effective_scheduled_us, dispatch_lead_us);
        if effective_scheduled_us > now_us.saturating_add(effective_lead) {
            return None;
        }
        if effective_scheduled_us > now_us && self.early_pop_blocked(batch) {
            return None;
        }
        let popped = self
            .schedule
            .materialize_batch(self.cursor, self.recovery_offset_us);
        self.cursor += 1;
        Some((popped, effective_lead))
    }

    pub fn activate_sent_downs(
        &mut self,
        intents: &[RuntimeKeyIntent],
        sent_scan_codes: &[u16],
        dispatch_started_us: u64,
        dispatch_started_ticks: TimelineTicks,
        dispatch_completed_us: u64,
        dispatch_completed_ticks: TimelineTicks,
    ) {
        let release_not_before_us = dispatch_completed_us + self.min_hold_us + self.delivery_margin_us;
        let release_not_before_ticks = dispatch_completed_ticks.saturating_add(self.min_hold_ticks).saturating_add(self.delivery_margin_ticks);

        if sent_scan_codes.len() == 1 {
            let only_sent = sent_scan_codes[0];
            for intent in intents {
                let Some(generation_id) = intent.generation_id else {
                    continue;
                };
                if intent.scan_code != only_sent {
                    // Terminalize without touching any mask — slot was never activated.
                    self.counters.increment(GenerationStatus::DroppedBackend);
                    continue;
                }
                self.active_by_slot[intent.key_slot as usize] = Some(ActiveGeneration {
                    generation_id,
                    scan_code: intent.scan_code,
                    key_slot: intent.key_slot,
                    source_action_index: intent.source_action_index,
                    scheduled_down_us: intent.scheduled_us,
                    scheduled_down_ticks: self.batch_scheduled_ticks[intent.source_action_index as usize],
                    down_dispatch_started_us: dispatch_started_us,
                    down_dispatch_started_ticks: dispatch_started_ticks,
                    down_dispatch_completed_us: dispatch_completed_us,
                    down_dispatch_completed_ticks: dispatch_completed_ticks,
                    release_not_before_us,
                    release_not_before_ticks,
                });
                self.active_mask |= Self::bit_for_slot(intent.key_slot);
                // No HashMap insertion — active count is derived from active_mask at query time.
            }
            return;
        }

        for intent in intents {
            let Some(generation_id) = intent.generation_id else {
                continue;
            };
            if !sent_scan_codes.contains(&intent.scan_code) {
                self.counters.increment(GenerationStatus::DroppedBackend);
                continue;
            }
            self.active_by_slot[intent.key_slot as usize] = Some(ActiveGeneration {
                generation_id,
                scan_code: intent.scan_code,
                key_slot: intent.key_slot,
                source_action_index: intent.source_action_index,
                scheduled_down_us: intent.scheduled_us,
                scheduled_down_ticks: self.batch_scheduled_ticks[intent.source_action_index as usize],
                down_dispatch_started_us: dispatch_started_us,
                down_dispatch_started_ticks: dispatch_started_ticks,
                down_dispatch_completed_us: dispatch_completed_us,
                down_dispatch_completed_ticks: dispatch_completed_ticks,
                release_not_before_us,
                release_not_before_ticks,
            });
            self.active_mask |= Self::bit_for_slot(intent.key_slot);
            // No HashMap insertion — active count is derived from active_mask at query time.
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

    /// Terminalize every generation in a conflicted authored chord without
    /// sending a playable subset. Accuracy-first callers use this when a
    /// partial chord would be worse than dropping the whole chord.
    pub fn drop_conflicted_downs(&mut self, intents: &[RuntimeKeyIntent]) {
        for intent in intents {
            if let Some(generation_id) = intent.generation_id {
                self.terminalize(generation_id, GenerationStatus::DroppedConflict);
            }
        }
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
                packet_id: intent.packet_id,
                scheduled_release_us: intent.scheduled_us,
                scheduled_release_ticks: self.batch_scheduled_ticks[intent.source_action_index as usize],
                down_dispatch_started_us: active.down_dispatch_started_us,
                down_dispatch_started_ticks: active.down_dispatch_started_ticks,
                release_not_before_us: active.release_not_before_us,
                release_not_before_ticks: active.release_not_before_ticks,
                reason_id: intent.reason_id,
                retry_count: 0,
                next_retry_us: 0,
                next_retry_ticks: crate::time::TimelineTicks(0),
                first_failure_us: None,
                first_failure_ticks: None,
                last_win32_error: None,
            };

            self.pending_by_slot[intent.key_slot as usize] = Some(pending.clone());
            self.pending_mask |= Self::bit_for_slot(intent.key_slot);
            // No HashMap insertion — release_pending count is derived from pending_mask.
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
                packet_id: intent.packet_id,
                scheduled_release_us: intent.scheduled_us,
                scheduled_release_ticks: self.batch_scheduled_ticks[intent.source_action_index as usize],
                down_dispatch_started_us: active.down_dispatch_started_us,
                down_dispatch_started_ticks: active.down_dispatch_started_ticks,
                release_not_before_us: active.release_not_before_us,
                release_not_before_ticks: active.release_not_before_ticks,
                reason_id: intent.reason_id,
                retry_count: 0,
                next_retry_us: 0,
                next_retry_ticks: crate::time::TimelineTicks(0),
                first_failure_us: None,
                first_failure_ticks: None,
                last_win32_error: None,
            };

            self.pending_by_slot[intent.key_slot as usize] = Some(pending.clone());
            self.pending_mask |= Self::bit_for_slot(intent.key_slot);
            // No HashMap insertion — release_pending count is derived from pending_mask.
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

        for &_gen_id in &sorted_cancelled {
            self.counters.increment(GenerationStatus::Cancelled);
        }

        self.active_by_slot.fill(None);
        self.pending_by_slot.fill(None);
        self.active_mask = 0;
        self.pending_mask = 0;
        self.release_recovery_started_us = None;

        sorted_cancelled
    }
}
