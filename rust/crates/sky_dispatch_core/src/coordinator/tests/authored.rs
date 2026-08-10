use super::*;

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
        .pop_next_due_authored(0, 500)
        .expect("first action is due");
    assert_eq!(first.scheduled_us, 0);
    assert_eq!(coordinator.next_authored_us(500), Some(500));
    assert!(coordinator.pop_next_due_authored(0, 500).is_none());

    let (second, _) = coordinator
        .pop_next_due_authored(500, 500)
        .expect("second action keeps its authored ordering");
    assert_eq!(second.scheduled_us, 1_000);
}

#[test]
fn production_ticks_preserve_later_sublead_deadline_and_applied_lead() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "startup".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: vec![0x16].into(),
                reason: "later sublead".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid schedule");
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
    let lead = DurationTicks::from_raw(500);

    let first = coordinator
        .prepare_next_due_authored(TimelineTicks::ZERO, lead)
        .unwrap()
        .expect("first startup packet is due at the logical epoch");
    assert_eq!(first.effective_scheduled_ticks, TimelineTicks::ZERO);
    assert_eq!(first.effective_lead_ticks, DurationTicks::ZERO);
    coordinator
        .commit_packet_success(first, TimelineTicks::ZERO, TimelineTicks::ZERO)
        .unwrap();

    assert_eq!(
        coordinator.next_authored_ticks(lead).unwrap(),
        Some(TimelineTicks::from_raw(100))
    );
    assert!(
        coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(99), lead)
            .unwrap()
            .is_none()
    );
    let second = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(100), lead)
        .unwrap()
        .expect("later sublead packet keeps its authored deadline");
    assert_eq!(second.effective_scheduled_ticks.as_u64(), 100);
    assert_eq!(second.effective_lead_ticks, DurationTicks::ZERO);
}

#[test]
fn production_ticks_apply_lead_only_at_or_after_authored_timestamp() {
    for (scheduled, expected_deadline, expected_lead) in
        [(500, 0, 500), (499, 499, 0), (501, 1, 500)]
    {
        let schedule = compile_runtime_intents(
            &[KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: scheduled,
                scan_codes: vec![0x15].into(),
                reason: "boundary".into(),
            }],
            &[0x15],
        )
        .expect("valid schedule");
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
        let lead = DurationTicks::from_raw(500);

        assert_eq!(
            coordinator.next_authored_ticks(lead).unwrap(),
            Some(TimelineTicks::from_raw(expected_deadline))
        );
        let prepared = coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(expected_deadline), lead)
            .unwrap()
            .expect("deadline is due");
        assert_eq!(prepared.effective_scheduled_ticks.as_u64(), scheduled);
        assert_eq!(prepared.effective_lead_ticks.as_u64(), expected_lead);
    }
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
            .requeue_failed_releases(&due, &[], 1_000, 1_000, Some(5))
            .expect("valid recovery")
    );
    assert_eq!(coordinator.next_pending_release_us(0), Some(3_000));
    assert!(!coordinator.is_finished());

    let retry = coordinator.pop_due_pending(3_000, 0);
    assert_eq!(retry.len(), 1);
    coordinator.complete_releases(&retry, &[0x15]);
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
fn down_commit_uses_pre_send_timestamp() {
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
    let prepared = coordinator
        .prepare_next_due_authored(
            crate::time::TimelineTicks::from_raw(100),
            crate::time::DurationTicks::ZERO,
        )
        .expect("prepare down")
        .expect("down is due");

    coordinator
        .commit_down_success(
            prepared,
            &[0x15],
            crate::time::TimelineTicks::from_raw(120),
            crate::time::TimelineTicks::from_raw(150),
        )
        .expect("commit down");

    let active = coordinator.active_for_slot(0).expect("active generation");
    assert_eq!(
        active.down_dispatch_started_ticks,
        crate::time::TimelineTicks::from_raw(120)
    );
    assert_eq!(
        active.down_dispatch_completed_ticks,
        crate::time::TimelineTicks::from_raw(150)
    );
}

#[test]
fn packet_commit_releases_before_retrigger_down_and_advances_once() {
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
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: vec![0x15].into(),
                reason: "retrigger release".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: vec![0x15, 0x16].into(),
                reason: "retrigger chord".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid packet schedule");
    let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, TimelineTicks::from_raw);

    let first = coordinator
        .prepare_next_due_authored(TimelineTicks::ZERO, DurationTicks::ZERO)
        .expect("prepare first packet")
        .expect("first packet is due");
    assert_eq!(first.packet_batch_count, 1);
    coordinator
        .commit_packet_success(
            first,
            TimelineTicks::from_raw(10),
            TimelineTicks::from_raw(20),
        )
        .expect("commit first packet");

    let retrigger = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(1_000), DurationTicks::ZERO)
        .expect("prepare retrigger packet")
        .expect("retrigger packet is due");
    assert_eq!(retrigger.packet_batch_count, 2);
    assert_eq!(retrigger.packet_kind, PhysicalPacketKind::Mixed);
    let packet = coordinator
        .schedule
        .view_packet_ticks(retrigger.packet_index, retrigger.effective_scheduled_ticks)
        .expect("packet view");
    assert_eq!(packet.up_mask(), 0b01);
    assert_eq!(packet.down_mask(), 0b11);
    coordinator
        .commit_packet_success(
            retrigger,
            TimelineTicks::from_raw(1_010),
            TimelineTicks::from_raw(1_020),
        )
        .expect("commit retrigger packet");

    assert_eq!(coordinator.cursor, 3);
    assert_eq!(coordinator.active_mask, 0b11);
    assert_eq!(coordinator.active_for_slot(0).unwrap().generation_id, 1);
    assert_eq!(coordinator.active_for_slot(1).unwrap().generation_id, 2);
    assert_eq!(
        coordinator
            .generation_status_counts()
            .get(GenerationStatus::Released.as_str()),
        Some(&1)
    );
}

#[test]
fn multi_up_only_packet_advances_all_batches_once() {
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
                scheduled_us: 100,
                scan_codes: vec![0x15].into(),
                reason: "up one".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x16].into(),
                reason: "up two".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .unwrap();
    let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, TimelineTicks::from_raw);
    let first = coordinator
        .prepare_next_due_authored(TimelineTicks::ZERO, DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    coordinator
        .commit_packet_success(first, TimelineTicks::ZERO, TimelineTicks::from_raw(10))
        .unwrap();
    let release = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(100), DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(release.packet_kind, PhysicalPacketKind::UpOnly);
    coordinator
        .commit_packet_success(
            release,
            TimelineTicks::from_raw(100),
            TimelineTicks::from_raw(110),
        )
        .unwrap();
    assert_eq!(coordinator.cursor, 3);
    assert_eq!(coordinator.active_mask, 0);
    assert_eq!(coordinator.pending_mask, 0);
    assert_eq!(
        coordinator
            .generation_status_counts()
            .get(GenerationStatus::Released.as_str()),
        Some(&2)
    );
}

#[test]
fn stale_multi_up_packet_suppression_advances_atomically() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Up,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "stale-a".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 0,
                scan_codes: vec![0x16].into(),
                reason: "stale-b".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: vec![0x15].into(),
                reason: "first-down".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid stale packet schedule");
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);

    let stale = coordinator
        .prepare_current_stale_packet()
        .expect("prepare stale packet")
        .expect("stale packet is current");
    assert_eq!(stale.packet_batch_count, 2);
    assert_eq!(stale.suppressed_intent_count, 2);
    coordinator
        .commit_stale_packet(stale)
        .expect("suppress stale packet");
    assert_eq!(coordinator.cursor, 2);

    let next = coordinator
        .prepare_next_due_authored(
            crate::time::TimelineTicks::from_raw(100),
            DurationTicks::ZERO,
        )
        .expect("prepare first physical packet")
        .expect("first physical packet is due");
    assert_eq!(next.packet_batch_count, 1);
    assert_eq!(next.packet_kind, PhysicalPacketKind::DownOnly);
}

#[test]
fn stale_and_physical_intents_share_one_concrete_packet_kind() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x15].into(),
                reason: "stale-a".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x16].into(),
                reason: "stale-b".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: vec![0x17].into(),
                reason: "physical-down".into(),
            },
        ],
        &[0x15, 0x16, 0x17],
    )
    .expect("valid mixed metadata/physical packet");
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
    let prepared = coordinator
        .prepare_next_due_authored(
            crate::time::TimelineTicks::from_raw(100),
            DurationTicks::ZERO,
        )
        .expect("prepare concrete physical packet")
        .expect("packet is due");
    assert_eq!(prepared.packet_batch_count, 3);
    assert_eq!(prepared.packet_kind, PhysicalPacketKind::DownOnly);
    let packet = coordinator
        .schedule
        .view_packet_ticks(prepared.packet_index, prepared.effective_scheduled_ticks)
        .expect("packet view");
    assert_eq!(packet.up_mask(), 0);
    assert_eq!(packet.down_mask(), 0b100);
    coordinator
        .commit_packet_success(
            prepared,
            crate::time::TimelineTicks::from_raw(100),
            crate::time::TimelineTicks::from_raw(110),
        )
        .expect("commit one physical packet");
    assert_eq!(coordinator.cursor, 3);
}

#[test]
fn owned_and_stale_up_with_down_is_mixed_with_physical_count_two() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "owned-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x15].into(),
                reason: "owned-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x16].into(),
                reason: "stale-up".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: vec![0x17].into(),
                reason: "retrigger-down".into(),
            },
        ],
        &[0x15, 0x16, 0x17],
    )
    .expect("valid owned/stale mixed packet");
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);
    let first = coordinator
        .prepare_next_due_authored(crate::time::TimelineTicks::ZERO, DurationTicks::ZERO)
        .expect("prepare first down")
        .expect("first down is due");
    coordinator
        .commit_packet_success(
            first,
            crate::time::TimelineTicks::ZERO,
            crate::time::TimelineTicks::from_raw(10),
        )
        .expect("commit first down");
    let mixed = coordinator
        .prepare_next_due_authored(
            crate::time::TimelineTicks::from_raw(100),
            DurationTicks::ZERO,
        )
        .expect("prepare mixed packet")
        .expect("mixed packet is due");
    assert_eq!(mixed.packet_batch_count, 3);
    assert_eq!(mixed.packet_kind, PhysicalPacketKind::Mixed);
    let packet = coordinator
        .schedule
        .view_packet_ticks(mixed.packet_index, mixed.effective_scheduled_ticks)
        .expect("mixed packet view");
    assert_eq!(packet.up_mask().count_ones(), 1);
    assert_eq!(packet.down_mask().count_ones(), 1);
    assert_eq!(
        packet.up_mask().count_ones() + packet.down_mask().count_ones(),
        2
    );
}

#[test]
fn mixed_packet_waits_until_release_not_before_and_rebases_following_action() {
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
                scheduled_us: 100,
                scan_codes: vec![0x16].into(),
                reason: "retrigger".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x15].into(),
                reason: "release".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Down,
                scheduled_us: 200,
                scan_codes: vec![0x17].into(),
                reason: "following".into(),
            },
        ],
        &[0x15, 0x16, 0x17],
    )
    .unwrap();
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 100, 0, TimelineTicks::from_raw);
    let first = coordinator
        .prepare_next_due_authored(TimelineTicks::ZERO, DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    coordinator
        .commit_packet_success(first, TimelineTicks::ZERO, TimelineTicks::from_raw(20))
        .unwrap();

    assert!(
        coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(100), DurationTicks::ZERO)
            .unwrap()
            .is_none()
    );
    let mixed = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(120), DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(mixed.packet_kind, PhysicalPacketKind::Mixed);
    coordinator
        .commit_packet_success(
            mixed,
            TimelineTicks::from_raw(120),
            TimelineTicks::from_raw(130),
        )
        .unwrap();
    assert_eq!(coordinator.recovery_offset_ticks().as_u64(), 0);
    assert_eq!(coordinator.timeline_rebase_count(), 0);
    let following = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(199), DurationTicks::ZERO)
        .unwrap();
    assert!(following.is_none());
    let following = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(200), DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(following.effective_scheduled_ticks.as_u64(), 200);
}

#[test]
fn up_only_release_floor_is_not_reduced_by_dispatch_lead() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15, 0x16].into(),
                reason: "first chord".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x15].into(),
                reason: "release one".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x16].into(),
                reason: "release two".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid up-only packet schedule");
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 100, 0, TimelineTicks::from_raw);
    let first = coordinator
        .prepare_next_due_authored(TimelineTicks::ZERO, DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    coordinator
        .commit_packet_success(first, TimelineTicks::ZERO, TimelineTicks::from_raw(20))
        .unwrap();

    let lead = DurationTicks::from_raw(50);
    assert_eq!(
        coordinator.next_authored_ticks(lead).unwrap(),
        Some(TimelineTicks::from_raw(120))
    );
    assert!(
        coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(119), lead)
            .unwrap()
            .is_none()
    );
    let prepared = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(120), lead)
        .unwrap()
        .unwrap();
    assert_eq!(prepared.packet_kind, PhysicalPacketKind::UpOnly);
}

#[test]
fn mixed_release_floor_is_not_reduced_by_dispatch_lead() {
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
                scheduled_us: 100,
                scan_codes: vec![0x16].into(),
                reason: "retrigger".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x15].into(),
                reason: "release".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid mixed packet schedule");
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 100, 0, TimelineTicks::from_raw);
    let first = coordinator
        .prepare_next_due_authored(TimelineTicks::ZERO, DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    coordinator
        .commit_packet_success(first, TimelineTicks::ZERO, TimelineTicks::from_raw(20))
        .unwrap();

    let lead = DurationTicks::from_raw(50);
    assert_eq!(
        coordinator.next_authored_ticks(lead).unwrap(),
        Some(TimelineTicks::from_raw(120))
    );
    assert!(
        coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(119), lead)
            .unwrap()
            .is_none()
    );
    let prepared = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(120), lead)
        .unwrap()
        .unwrap();
    assert_eq!(prepared.packet_kind, PhysicalPacketKind::Mixed);
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
            .requeue_failed_releases(&due, &[], 1_000, 1_000, Some(5))
            .expect("valid recovery")
    );

    assert!(coordinator.pop_next_due_authored(2_000, 0).is_none());
    assert_eq!(coordinator.next_deadline_us(0, 0), Some(3_000));

    let retry = coordinator.pop_due_pending(3_000, 0);
    coordinator.complete_releases(&retry, &[0x15]);
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
