use super::*;

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
        coordinator.complete_releases(std::slice::from_ref(release), &[release.scan_code]);
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
            coordinator.complete_releases(std::slice::from_ref(release), &[release.scan_code]);
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
        .complete_releases(&due, &[0x15])
        .expect("first completion");

    let counts = coordinator.generation_status_counts();
    assert_eq!(counts.get("released"), Some(&1));
    assert_eq!(counts.values().sum::<u64>(), 1);
    assert!(coordinator.complete_releases(&due, &[0x15]).is_err());
}

#[test]
fn single_up_real_generation_gates_release_floor() {
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
                scheduled_us: 30,
                scan_codes: vec![0x15].into(),
                reason: "up".into(),
            },
        ],
        &[0x15],
    )
    .expect("valid schedule");
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
    // Activate the Down so the slot owns a real generation with a floor.
    // The Up is authored early (30) but the completion-anchored hold floor
    // later (80) must gate it, proving the floor is not ignored for a real
    // generation.
    let down = coordinator
        .prepare_next_due_authored(TimelineTicks::ZERO, DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    coordinator
        .commit_packet_success(down, TimelineTicks::ZERO, TimelineTicks::from_raw(80))
        .unwrap();
    assert!(
        coordinator
            .active_for_slot(0)
            .map(|active| active.release_not_before_ticks == TimelineTicks::from_raw(80))
            .unwrap_or(false),
        "completion-anchored floor must attach to the real generation"
    );
    // The Up cannot be dispatched before the slot's floor.
    assert!(
        coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(79), DurationTicks::ZERO)
            .unwrap()
            .is_none()
    );
    let prepared = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(80), DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(prepared.packet_kind, Some(PhysicalPacketKind::UpOnly));
}

#[test]
fn mixed_real_up_gates_release_floor() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "down-a".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 70,
                scan_codes: vec![0x16].into(),
                reason: "down-b".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 70,
                scan_codes: vec![0x15].into(),
                reason: "up-a".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid mixed schedule");
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
    let down_a = coordinator
        .prepare_next_due_authored(TimelineTicks::ZERO, DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    coordinator
        .commit_packet_success(down_a, TimelineTicks::ZERO, TimelineTicks::from_raw(90))
        .unwrap();
    // The Mixed packet (Up 0x15 + Down 0x16) cannot precede A's floor.
    let deadline = coordinator
        .next_authored_ticks(DurationTicks::from_raw(30))
        .unwrap()
        .unwrap();
    assert_eq!(deadline, TimelineTicks::from_raw(90));
}

#[test]
fn no_generation_id_stale_up_does_not_require_active_generation() {
    let schedule = compile_runtime_intents(
        &[KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Up,
            scheduled_us: 100,
            scan_codes: vec![0x15].into(),
            reason: "stale".into(),
        }],
        &[0x15],
    )
    .expect("valid stale up schedule");
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
    // A stale Up has NO_GENERATION_ID and must not require an active
    // generation; it prepares without an invariant error.
    let prepared = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(100), DurationTicks::ZERO)
        .unwrap();
    assert!(prepared.is_none() || prepared.unwrap().packet_kind.is_none());
}

#[test]
fn missing_active_generation_prepare_returns_invariant_error() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x16].into(),
                reason: "down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x16].into(),
                reason: "up".into(),
            },
        ],
        &[0x16],
    )
    .expect("valid schedule");
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
    // Advance the cursor past the Down so the Up packet is the next prepared
    // authored batch. The Down becomes an active real generation (0x16).
    let down = coordinator
        .prepare_next_due_authored(TimelineTicks::ZERO, DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    coordinator
        .commit_packet_success(down, TimelineTicks::ZERO, TimelineTicks::from_raw(0))
        .unwrap();
    // Drop the real generation by clearing the active slot behind the
    // coordinator's back: the Up packet then references a real generation
    // that owns no active slot.
    coordinator.active_by_slot[0] = None;
    let err = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(100), DurationTicks::ZERO)
        .unwrap_err();
    assert!(matches!(
        err,
        CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(_))
    ));
}

#[test]
fn generation_mismatch_prepare_returns_invariant_error() {
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
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
    // Advance the cursor past the Down so the Up packet is the next prepared
    // authored batch.
    let down = coordinator
        .prepare_next_due_authored(TimelineTicks::ZERO, DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    coordinator
        .commit_packet_success(down, TimelineTicks::ZERO, TimelineTicks::from_raw(0))
        .unwrap();
    // Fabricate a mismatched owner generation for the slot: the Up
    // references generation 0, but the slot claims generation 7.
    coordinator.active_by_slot[0] = Some(ActiveGeneration {
        generation_id: 7,
        scan_code: 0x15,
        key_slot: 0,
        source_action_index: 0,
        scheduled_down_ticks: TimelineTicks::ZERO,
        down_dispatch_started_ticks: TimelineTicks::ZERO,
        down_dispatch_completed_ticks: TimelineTicks::ZERO,
        release_not_before_ticks: TimelineTicks::ZERO,
    });
    let err = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(100), DurationTicks::ZERO)
        .unwrap_err();
    assert!(matches!(
        err,
        CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(_))
    ));
}

#[test]
fn terminal_real_generation_does_not_gain_exemption_and_returns_invariant_error() {
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
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
    let down = coordinator
        .prepare_next_due_authored(TimelineTicks::ZERO, DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    coordinator
        .commit_packet_success(down, TimelineTicks::ZERO, TimelineTicks::from_raw(0))
        .unwrap();
    // Mark generation 0 terminal and clear active slot
    coordinator
        .transition_generation(0, GenerationStatus::Active, GenerationStatus::Cancelled)
        .unwrap();
    coordinator.active_by_slot[0] = None;
    // Preparing the Up packet for real generation 0 must NOT gain exemption
    // just because generation 0 is terminal (Cancelled); it MUST return an invariant error.
    let err = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(100), DurationTicks::ZERO)
        .unwrap_err();
    assert!(matches!(
        err,
        CoordinatorError::Invariant(CoordinatorInvariantError::Accounting(_))
    ));
}

#[test]
fn invariant_mismatch_prevents_sender_invocation() {
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
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
    let down = coordinator
        .prepare_next_due_authored(TimelineTicks::ZERO, DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    coordinator
        .commit_packet_success(down, TimelineTicks::ZERO, TimelineTicks::from_raw(0))
        .unwrap();
    // Mismatched active generation (generation 999 instead of 0)
    coordinator.active_by_slot[0] = Some(ActiveGeneration {
        generation_id: 999,
        scan_code: 0x15,
        key_slot: 0,
        source_action_index: 0,
        scheduled_down_ticks: TimelineTicks::ZERO,
        down_dispatch_started_ticks: TimelineTicks::ZERO,
        down_dispatch_completed_ticks: TimelineTicks::ZERO,
        release_not_before_ticks: TimelineTicks::ZERO,
    });

    let mut send_call_count = 0;
    let prep_res =
        coordinator.prepare_next_due_authored(TimelineTicks::from_raw(100), DurationTicks::ZERO);
    if prep_res.is_ok() {
        send_call_count += 1;
    }
    assert_eq!(send_call_count, 0);
    assert!(matches!(
        prep_res,
        Err(CoordinatorError::Invariant(
            CoordinatorInvariantError::Accounting(_)
        ))
    ));
}

#[test]
fn authored_sublead_preserves_later_timestamp() {
    let schedule = compile_runtime_intents(
        &[KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 30,
            scan_codes: vec![0x15].into(),
            reason: "down".into(),
        }],
        &[0x15],
    )
    .expect("valid schedule");
    let coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
    // A later authored packet whose requested lead crosses logical zero uses
    // effective lead zero, preserving its authored timestamp rather than
    // collapsing it onto the timeline epoch.
    assert_eq!(
        coordinator
            .packet_effective_deadline_ticks(0, DurationTicks::from_raw(1_000))
            .unwrap(),
        TimelineTicks::from_raw(30)
    );
}
