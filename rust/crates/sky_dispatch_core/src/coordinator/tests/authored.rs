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
        .pop_next_due_authored(0, 0)
        .expect("first action is due");
    assert_eq!(first.scheduled_us, 0);
    assert_eq!(coordinator.next_authored_us(0), Some(1_000));
    assert!(coordinator.pop_next_due_authored(0, 0).is_none());

    let (second, _) = coordinator
        .pop_next_due_authored(1_000, 0)
        .expect("second action keeps its authored ordering");
    assert_eq!(second.scheduled_us, 1_000);
}

#[test]
fn production_ticks_preserve_authored_deadlines_without_dispatch_lead() {
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
    assert_eq!(
        second.effective_scheduled_ticks,
        TimelineTicks::from_raw(100)
    );
}

#[test]
fn production_ticks_do_not_advance_authored_deadlines() {
    for scheduled in [500, 499, 501] {
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
            Some(TimelineTicks::from_raw(scheduled))
        );
        let prepared = coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(scheduled), lead)
            .unwrap()
            .expect("deadline is due");
        assert_eq!(prepared.effective_scheduled_ticks.as_u64(), scheduled);
        assert_eq!(prepared.effective_scheduled_ticks.as_u64(), scheduled);
    }
}

#[test]
fn current_authored_packet_can_be_prepared_before_its_deadline() {
    let schedule = compile_runtime_intents(
        &[KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 10_000,
            scan_codes: vec![0x15].into(),
            reason: "future".into(),
        }],
        &[0x15],
    )
    .expect("valid future schedule");
    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, 0, 0, crate::time::TimelineTicks::from_raw);

    let prepared = coordinator
        .prepare_current_authored_packet()
        .expect("current packet preparation")
        .expect("future packet exists");
    assert_eq!(prepared.effective_scheduled_ticks.as_u64(), 10_000);
    assert!(
        coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(9_999), DurationTicks::ZERO)
            .expect("due check")
            .is_none()
    );
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
fn mixed_packet_waits_until_release_not_before_without_shifting_following_action() {
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
fn authored_packet_lifecycle_has_no_pending_release_state() {
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
    .expect("valid authored lifecycle schedule");
    let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 20, 0, TimelineTicks::from_raw);

    let down = coordinator
        .prepare_next_due_authored(TimelineTicks::ZERO, DurationTicks::ZERO)
        .expect("prepare down")
        .expect("down is due");
    coordinator
        .commit_packet_success(down, TimelineTicks::ZERO, TimelineTicks::from_raw(10))
        .expect("commit down");
    let up = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(100), DurationTicks::ZERO)
        .expect("prepare up")
        .expect("up is due");
    coordinator
        .commit_packet_success(
            up,
            TimelineTicks::from_raw(100),
            TimelineTicks::from_raw(101),
        )
        .expect("commit up");

    let counts = coordinator.generation_status_counts();
    assert_eq!(counts.get("released"), Some(&1));
    assert!(!counts.contains_key("release_pending"));
    assert_eq!(coordinator.active_mask, 0);
    assert!(coordinator.is_finished());
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
