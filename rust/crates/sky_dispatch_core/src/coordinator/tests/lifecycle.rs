use super::*;

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

    // Effective release floor is completion (100) + min_hold (20,000) + delivery_margin (5,000) = 25,100.
    assert!(
        coordinator
            .prepare_next_due_authored(
                crate::time::TimelineTicks::from_raw(25_099),
                crate::time::DurationTicks::ZERO,
            )
            .unwrap()
            .is_none(),
        "up packet must not be due before min_hold + delivery_margin"
    );

    let (up, _) = coordinator
        .pop_next_due_authored(25_100, 0)
        .expect("up is due at release floor");

    let _ = coordinator.request_releases(&up.intents);
    let due_now = coordinator.pop_due_pending(25_100, 0);
    assert_eq!(
        due_now.len(),
        1,
        "release must be due at completion + min_hold + delivery_margin"
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
    // Requeue only key 0x15; key 0x16 remains physically active and
    // continues to block the next authored chord.
    let _ = coordinator.request_releases(&up.intents[..1]);
    let due = coordinator.pop_due_pending(1_000, 0);
    assert!(
        !coordinator
            .requeue_failed_releases(&due, &[], 1_000, 1_500, Some(5))
            .expect("valid recovery")
    );
    assert_eq!(coordinator.next_pending_release_us(0), Some(3_500));

    let retry = coordinator.pop_due_pending(3_500, 0);
    coordinator.complete_releases(&retry, &[0x15]);
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
    // The due cohort is borrowed while transport is in flight.  Pending
    // ownership is cleared only by confirmed reconciliation.
    assert_eq!(coordinator.pending_mask.count_ones(), 2);
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
    coordinator.complete_releases(&due, &[0x15, 0x16]);

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
fn post_cleanup_invariant_rejects_nonterminal_state_until_cancelled() {
    let schedule = compile_runtime_intents(
        &[KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: vec![0x15].into(),
            reason: "future".into(),
        }],
        &[0x15],
    )
    .expect("valid schedule");
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);

    assert!(coordinator.check_invariants().is_ok());
    assert!(coordinator.check_post_cleanup_invariants().is_err());
    coordinator
        .cancel_all()
        .expect("terminal cancellation succeeds");
    assert!(coordinator.check_post_cleanup_invariants().is_ok());
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

    let stale_up = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(100), DurationTicks::ZERO)
        .expect("cancelled generation makes its future Up stale")
        .expect("stale authored Up remains in the immutable timeline");
    assert_eq!(stale_up.packet_kind, None);
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
    coordinator.complete_releases(&due, &[0x15]);

    let counts = coordinator.generation_status_counts();
    assert_eq!(counts.get("released"), Some(&1));
    assert_eq!(counts.get("active"), Some(&1)); // 0x16 still active
    assert_eq!(counts.get("release_pending"), Some(&0));

    // Release 0x16
    let (up_b, _) = coordinator.pop_next_due_authored(2_000, 0).unwrap();
    let _ = coordinator.request_releases(&up_b.intents);
    let due2 = coordinator.pop_due_pending(2_000, 0);
    coordinator.complete_releases(&due2, &[0x16]);

    let counts2 = coordinator.generation_status_counts();
    assert_eq!(counts2.get("released"), Some(&2));
    assert_eq!(counts2.get("active"), Some(&0));
}
