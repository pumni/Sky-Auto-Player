#![allow(unused_must_use)]

use proptest::prelude::*;
use sky_dispatch_core::compile::compile_runtime_intents;
use sky_dispatch_core::coordinator::{PendingDispatchPlan, RuntimeDispatchCoordinator};
use sky_dispatch_core::model::{ActionKind, KeyActionInput, RuntimeBatch, RuntimeKeyIntent};
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};

fn test_coordinator(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    min_hold_us: u64,
    delivery_margin_us: u64,
    _us_to_ticks: fn(u64) -> TimelineTicks,
) -> RuntimeDispatchCoordinator {
    RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        min_hold_us,
        DurationTicks::from_raw(min_hold_us),
        delivery_margin_us,
        DurationTicks::from_raw(delivery_margin_us),
        |microseconds| Ok(TimelineTicks::from_raw(microseconds)),
    )
    .expect("test coordinator configuration is valid")
}

trait LegacyCoordinatorTestApi {
    fn next_pending_release_us(&self, lead_up: u64) -> Option<u64>;
    fn pending_count_due_at(&self, deadline_us: u64, lead_up: u64) -> usize;
    fn pop_due_pending(
        &mut self,
        now_us: u64,
        lead_up: u64,
    ) -> smallvec::SmallVec<[sky_dispatch_core::coordinator::PendingRelease; 15]>;
    fn pop_next_due_authored(
        &mut self,
        now_us: u64,
        dispatch_lead_us: u64,
    ) -> Option<(RuntimeBatch, u64)>;
    fn activate_sent_downs(
        &mut self,
        intents: &[RuntimeKeyIntent],
        sent_scan_codes: &[u16],
        _dispatch_started_us: u64,
        dispatch_started_ticks: TimelineTicks,
        _dispatch_completed_us: u64,
        dispatch_completed_ticks: TimelineTicks,
    ) -> Result<(), sky_dispatch_core::coordinator::CoordinatorError>;
}

impl LegacyCoordinatorTestApi for RuntimeDispatchCoordinator {
    fn next_pending_release_us(&self, lead_up: u64) -> Option<u64> {
        self.next_pending_release_ticks(DurationTicks::from_raw(lead_up))
            .expect("typed pending deadline")
            .map(TimelineTicks::as_u64)
    }

    fn pending_count_due_at(&self, deadline_us: u64, lead_up: u64) -> usize {
        self.pending_count_due_at_ticks(
            TimelineTicks::from_raw(deadline_us),
            DurationTicks::from_raw(lead_up),
        )
        .expect("typed pending count")
    }

    fn pop_due_pending(
        &mut self,
        now_us: u64,
        lead_up: u64,
    ) -> smallvec::SmallVec<[sky_dispatch_core::coordinator::PendingRelease; 15]> {
        let plan = PendingDispatchPlan {
            deadline_ticks: TimelineTicks::from_raw(now_us),
            lead_ticks: DurationTicks::from_raw(lead_up),
            polyphony: 1,
            lead_saturated: false,
        };
        self.pop_due_pending_ticks(TimelineTicks::from_raw(now_us), &plan)
            .expect("typed pending pop")
    }

    fn pop_next_due_authored(
        &mut self,
        now_us: u64,
        dispatch_lead_us: u64,
    ) -> Option<(RuntimeBatch, u64)> {
        let (index, lead) = self
            .pop_next_due_authored_ticks(
                TimelineTicks::from_raw(now_us),
                DurationTicks::from_raw(dispatch_lead_us),
            )
            .expect("typed authored pop")?;
        let batch = self
            .schedule
            .try_materialize_batch_authored(index)
            .expect("typed authored batch materialization");
        Some((batch, lead.as_u64()))
    }

    fn activate_sent_downs(
        &mut self,
        intents: &[RuntimeKeyIntent],
        sent_scan_codes: &[u16],
        _dispatch_started_us: u64,
        dispatch_started_ticks: TimelineTicks,
        _dispatch_completed_us: u64,
        dispatch_completed_ticks: TimelineTicks,
    ) -> Result<(), sky_dispatch_core::coordinator::CoordinatorError> {
        self.activate_sent_downs_ticks(
            intents,
            sent_scan_codes,
            dispatch_started_ticks,
            dispatch_completed_ticks,
        )
    }
}

proptest! {
    /// Source action IDs are metadata and may have arbitrary gaps. They must
    /// never be used as storage indexes for the prepared batch timeline.
    #[test]
    fn invariant_non_contiguous_source_ids_complete_lifecycle(
        gaps in prop::collection::vec(1_u32..=100_000, 4),
    ) {
        let mut source_ids = Vec::with_capacity(4);
        let mut source_id = 100_u32;
        for gap in gaps {
            source_ids.push(source_id);
            source_id = source_id.checked_add(gap).unwrap();
        }
        let actions = vec![
            KeyActionInput {
                source_action_index: source_ids[0],
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![2],
                reason: "gapped-down-a".to_string().into(),
            },
            KeyActionInput {
                source_action_index: source_ids[1],
                kind: ActionKind::Up,
                scheduled_us: 10_000,
                scan_codes: smallvec::smallvec![2],
                reason: "gapped-up-a".to_string().into(),
            },
            KeyActionInput {
                source_action_index: source_ids[2],
                kind: ActionKind::Down,
                scheduled_us: 20_000,
                scan_codes: smallvec::smallvec![3],
                reason: "gapped-down-b".to_string().into(),
            },
            KeyActionInput {
                source_action_index: source_ids[3],
                kind: ActionKind::Up,
                scheduled_us: 30_000,
                scan_codes: smallvec::smallvec![3],
                reason: "gapped-up-b".to_string().into(),
            },
        ];
        let schedule = compile_runtime_intents(&actions, &[2, 3]).unwrap();
        let generation_count = schedule.generation_count;
        let mut coordinator = test_coordinator(
            schedule,
            0,
            0,
            sky_dispatch_core::time::TimelineTicks::from_raw,
        );

        while !coordinator.is_finished() {
            if let Some((batch, _)) = coordinator.pop_next_due_authored(u64::MAX, 0) {
                match batch.kind {
                    ActionKind::Down => {
                        let (playable, conflicts) = coordinator
                            .split_down_intents(&batch.intents)
                            .expect("valid transition");
                        prop_assert!(conflicts.is_empty());
                        let sent: Vec<u16> = playable.iter().map(|intent| intent.scan_code).collect();
                        coordinator.activate_sent_downs(
                            &playable,
                            &sent,
                            batch.scheduled_us,
                            sky_dispatch_core::time::TimelineTicks::from_raw(batch.scheduled_us),
                            batch.scheduled_us,
                            sky_dispatch_core::time::TimelineTicks::from_raw(batch.scheduled_us),
                        );
                    }
                    ActionKind::Up => {
                        let (requested, suppressed) = coordinator
                            .request_releases(&batch.intents)
                            .expect("valid transition");
                        prop_assert!(suppressed.is_empty());
                        let due = coordinator.pop_due_pending(u64::MAX, 0);
                        prop_assert_eq!(due.len(), requested.len());
                        let sent: Vec<u16> = due.iter().map(|pending| pending.scan_code).collect();
                        coordinator.complete_releases(&due, &sent, &[]);
                    }
                }
            } else {
                prop_assert!(false, "gapped schedule stopped before terminal state");
            }
        }
        let counts = coordinator.generation_status_counts();
        prop_assert_eq!(counts.values().sum::<u64>(), generation_count);
        prop_assert_eq!(counts.get("released"), Some(&generation_count));
    }

    #[test]
    fn completion_anchor_and_generation_counts_hold_for_valid_chords(
        mut scan_codes in prop::collection::vec(2_u16..=6, 1..=5),
        authored_hold_us in 1_u64..20_000,
        min_hold_us in 1_u64..20_000,
        send_latency_us in 0_u64..2_000,
        release_lead_us in 0_u64..2_000,
    ) {
        scan_codes.sort_unstable();
        scan_codes.dedup();
        let down_scheduled_us = 1_000;
        let up_scheduled_us = down_scheduled_us + authored_hold_us;
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: down_scheduled_us,
                scan_codes: scan_codes.clone().into(),
                reason: "property-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: up_scheduled_us,
                scan_codes: scan_codes.clone().into(),
                reason: "property-up".to_string().into(),
            },
        ];
        let schedule = compile_runtime_intents(&actions, &scan_codes).unwrap();
        let generation_count = schedule.generation_count;
        let mut coordinator = test_coordinator(schedule, min_hold_us, 0, sky_dispatch_core::time::TimelineTicks::from_raw);

        let (down, _) = coordinator
            .pop_next_due_authored(down_scheduled_us, 0)
            .expect("down must be due");
        let (playable, conflicts) = coordinator
            .split_down_intents(&down.intents)
            .expect("valid transition");
        prop_assert!(conflicts.is_empty());
        let completed_us = down_scheduled_us + send_latency_us;
        coordinator.activate_sent_downs(
            &playable,
            &scan_codes,
            down_scheduled_us,
            sky_dispatch_core::time::TimelineTicks::from_raw(down_scheduled_us),
            completed_us,
            sky_dispatch_core::time::TimelineTicks::from_raw(completed_us),
        );

        let (up, _) = coordinator
            .pop_next_due_authored(up_scheduled_us, 0)
            .expect("up must be due");
        let (requested, suppressed) = coordinator
            .request_releases(&up.intents)
            .expect("valid transition");
        prop_assert!(suppressed.is_empty());
        let expected_due = up_scheduled_us
            .saturating_sub(release_lead_us)
            .max(completed_us + min_hold_us);
        prop_assert_eq!(
            coordinator.next_pending_release_us(release_lead_us),
            Some(expected_due)
        );
        if expected_due > 0 {
            prop_assert!(
                coordinator
                    .pop_due_pending(expected_due - 1, release_lead_us)
                    .is_empty()
            );
        }
        let due = coordinator.pop_due_pending(expected_due, release_lead_us);
        prop_assert_eq!(due.len(), requested.len());
        coordinator.complete_releases(&due, &scan_codes, &[]);

        let counts = coordinator.generation_status_counts();
        let count_total: u64 = counts.values().sum();
        prop_assert_eq!(count_total, generation_count);
        prop_assert_eq!(counts.get("released"), Some(&generation_count));
        prop_assert!(coordinator.is_finished());
    }
}

// ==========================================================================
// P3.2 Priority 1 — Fatal invariants
// ==========================================================================

proptest! {
    /// Invariant 1: No key active after terminal cleanup.
    /// Invariant 10: Sum of generation_status_counts == generation_count.
    #[test]
    fn invariant_no_active_keys_after_cleanup_and_count_sum(
        mut scan_codes in prop::collection::vec(2_u16..=6, 1..=4),
        min_hold_us in 0_u64..5_000,
        send_latency_us in 0_u64..1_000,
    ) {
        scan_codes.sort_unstable();
        scan_codes.dedup();
        if scan_codes.is_empty() {
            scan_codes.push(2);
        }
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: scan_codes.clone().into(),
                reason: "d".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 10_000,
                scan_codes: scan_codes.clone().into(),
                reason: "u".to_string().into(),
            },
        ];
        let schedule = compile_runtime_intents(&actions, &scan_codes).unwrap();
        let generation_count = schedule.generation_count;
        let mut coord = test_coordinator(
            schedule,
            min_hold_us,
            0,
            sky_dispatch_core::time::TimelineTicks::from_raw,
        );

        let (down, _) = coord.pop_next_due_authored(1_000, 0).unwrap();
        let (playable, _) = coord
            .split_down_intents(&down.intents)
            .expect("valid transition");
        let completed = 1_000 + send_latency_us;
        coord.activate_sent_downs(
            &playable,
            &scan_codes,
            1_000,
            sky_dispatch_core::time::TimelineTicks::from_raw(1_000),
            completed,
            sky_dispatch_core::time::TimelineTicks::from_raw(completed),
        );

        let (up, _) = coord.pop_next_due_authored(10_000, 0).unwrap();
        let (requested, _) = coord
            .request_releases(&up.intents)
            .expect("valid transition");
        let due_us = coord.next_pending_release_us(0).unwrap_or(10_000);
        let due = coord.pop_due_pending(due_us, 0);
        coord.complete_releases(&due, &scan_codes, &[]);

        // Invariant 1: is_finished implies all terminal.
        prop_assert!(coord.is_finished());

        // Invariant 10: sum of counts == generation_count.
        let counts = coord.generation_status_counts();
        let total: u64 = counts.values().sum();
        prop_assert_eq!(total, generation_count);
        let _ = requested;
    }
}

proptest! {
    /// Invariant 8 (positive): compile succeeds for non-overlapping schedule.
    #[test]
    fn invariant_compile_success_implies_no_same_key_overlap(
        mut codes in prop::collection::vec(2_u16..=8, 1..=3),
        gap_us in 1_u64..10_000,
    ) {
        codes.sort_unstable();
        codes.dedup();
        if codes.is_empty() {
            codes.push(2);
        }
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: codes.clone().into(),
                reason: "d1".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: gap_us,
                scan_codes: codes.clone().into(),
                reason: "u1".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: gap_us + 1,
                scan_codes: codes.clone().into(),
                reason: "d2".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: gap_us * 2 + 1,
                scan_codes: codes.clone().into(),
                reason: "u2".to_string().into(),
            },
        ];
        let result = compile_runtime_intents(&actions, &codes);
        prop_assert!(result.is_ok(), "non-overlapping schedule should compile: {:?}", result);
        let schedule = result.unwrap();
        prop_assert!(schedule.generation_count > 0);
    }

    /// Invariant 8 (negative): compiler rejects same-key overlap.
    #[test]
    fn invariant_compiler_rejects_same_key_overlap(
        code in 2_u16..=6,
    ) {
        use sky_dispatch_core::compile::CompileError;
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![code],
                reason: "d1".to_string().into(),
            },
            // Same key Down before Up — overlap:
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![code],
                reason: "d2-overlap".to_string().into(),
            },
        ];
        let result = compile_runtime_intents(&actions, &[code]);
        prop_assert!(
            matches!(result, Err(CompileError::OverlappingSameKeyDown { .. })),
            "expected OverlappingSameKeyDown, got {:?}", result
        );
    }
}

// ==========================================================================
// P3.2 Priority 2 — Timing invariants
// ==========================================================================

proptest! {
    /// Invariant 4: Release must not be scheduled before min-hold expires.
    #[test]
    fn invariant_release_not_before_min_hold(
        mut codes in prop::collection::vec(2_u16..=6, 1..=3),
        hold_us in 1_u64..50_000,
        min_hold_us in 1_u64..50_000,
        send_latency_us in 0_u64..5_000,
    ) {
        codes.sort_unstable();
        codes.dedup();
        if codes.is_empty() { codes.push(2); }
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: codes.clone().into(),
                reason: "d".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1_000 + hold_us,
                scan_codes: codes.clone().into(),
                reason: "u".to_string().into(),
            },
        ];
        let schedule = compile_runtime_intents(&actions, &codes).unwrap();
        let mut coord = test_coordinator(
            schedule,
            min_hold_us,
            0,
            sky_dispatch_core::time::TimelineTicks::from_raw,
        );

        let (down, _) = coord.pop_next_due_authored(1_000, 0).unwrap();
        let (playable, _) = coord
            .split_down_intents(&down.intents)
            .expect("valid transition");
        let completed_us = 1_000 + send_latency_us;
        coord.activate_sent_downs(
            &playable,
            &codes,
            1_000,
            sky_dispatch_core::time::TimelineTicks::from_raw(1_000),
            completed_us,
            sky_dispatch_core::time::TimelineTicks::from_raw(completed_us),
        );

        let (up, _) = coord.pop_next_due_authored(1_000 + hold_us, 0).unwrap();
        let _ = coord.request_releases(&up.intents);

        let min_allowed = completed_us + min_hold_us;
        if let Some(due) = coord.next_pending_release_us(0) {
            prop_assert!(
                due >= min_allowed,
                "release due={due} < min_allowed={min_allowed}"
            );
        }
    }

    /// Invariant 9: Canonical chord order is stable and sorted.
    #[test]
    fn invariant_canonical_chord_order_stable(
        mut codes in prop::collection::vec(2_u16..=15, 2..=5),
    ) {
        codes.sort_unstable();
        codes.dedup();
        if codes.len() < 2 { return Ok(()); }
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: codes.clone().into(),
                reason: "d".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: codes.clone().into(),
                reason: "u".to_string().into(),
            },
        ];
        let schedule = compile_runtime_intents(&actions, &codes).unwrap();
        let mut coord = test_coordinator(
            schedule,
            0,
            0,
            sky_dispatch_core::time::TimelineTicks::from_raw,
        );

        let (down, _) = coord.pop_next_due_authored(0, 0).unwrap();
        let (playable, _) = coord
            .split_down_intents(&down.intents)
            .expect("valid transition");
        let order: Vec<u16> = playable.iter().map(|i| i.scan_code).collect();

        let mut sorted = order.clone();
        sorted.sort_unstable();
        prop_assert_eq!(order, sorted, "chord scan_code order is not canonical");
    }
}

// ==========================================================================
// P3.2 Priority 3 — Structural invariants
// ==========================================================================

proptest! {
    /// Invariant 12: Corrupt estimator cache JSON must not panic.
    #[test]
    fn invariant_corrupt_estimator_does_not_panic(
        raw in ".*",
    ) {
        use sky_dispatch_core::estimator::SendLatencyEstimator;
        let mut estimator = SendLatencyEstimator::new(0.2, 2_000, 15);
        let _ = estimator.import_state(&raw);
    }

    /// Invariant 11: pop_due_pending never returns more entries than pending_count_due_at.
    #[test]
    fn invariant_pop_due_pending_bounded_by_pending_count(
        mut codes in prop::collection::vec(2_u16..=6, 1..=3),
        min_hold_us in 0_u64..5_000,
        send_latency_us in 0_u64..1_000,
    ) {
        codes.sort_unstable();
        codes.dedup();
        if codes.is_empty() { codes.push(2); }
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: codes.clone().into(),
                reason: "d".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 5_000,
                scan_codes: codes.clone().into(),
                reason: "u".to_string().into(),
            },
        ];
        let schedule = compile_runtime_intents(&actions, &codes).unwrap();
        let mut coord = test_coordinator(
            schedule,
            min_hold_us,
            0,
            sky_dispatch_core::time::TimelineTicks::from_raw,
        );

        let (down, _) = coord.pop_next_due_authored(0, 0).unwrap();
        let (playable, _) = coord
            .split_down_intents(&down.intents)
            .expect("valid transition");
        let completed = send_latency_us;
        coord.activate_sent_downs(
            &playable,
            &codes,
            0,
            sky_dispatch_core::time::TimelineTicks::from_raw(0),
            completed,
            sky_dispatch_core::time::TimelineTicks::from_raw(completed),
        );
        let (up, _) = coord.pop_next_due_authored(5_000, 0).unwrap();
        let _ = coord.request_releases(&up.intents);

        let due_us = coord.next_pending_release_us(0).unwrap_or(u64::MAX);
        let count_before = coord.pending_count_due_at(due_us, 0);
        let popped = coord.pop_due_pending(due_us, 0);

        prop_assert!(
            popped.len() <= count_before,
            "pop returned {} but count_before was {}", popped.len(), count_before
        );
    }
}
