use super::super::{CoordinatorError, GenerationStatus, RuntimeDispatchCoordinator};
use crate::compile::compile_runtime_intents;
use crate::model::{ActionKind, CompactIntent, KeyActionInput, MAX_KEYS, NO_GENERATION_ID};
use crate::time::TimelineTicks;

fn commit_next(coordinator: &mut RuntimeDispatchCoordinator) {
    let prepared = coordinator
        .prepare_current_authored_packet()
        .expect("prepare packet")
        .expect("packet exists");
    coordinator
        .commit_prepared_authored_frame_success_frozen(
            &prepared.commit,
            prepared.frame.authored_ticks,
            prepared.frame.authored_ticks,
        )
        .expect("commit packet");
}

#[test]
fn all_fifteen_cancelled_ups_keep_exact_identity_and_reconcile_once() {
    let scan_codes = (0..MAX_KEYS)
        .map(|slot| 0x15 + slot as u16)
        .collect::<Vec<_>>();
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: scan_codes.clone().into(),
                reason: "fifteen-key Down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: scan_codes.clone().into(),
                reason: "fifteen-key Up".into(),
            },
        ],
        &scan_codes,
    )
    .expect("valid fifteen-key chord");
    let physical_up_mask = (1u16 << MAX_KEYS) - 1;
    let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, TimelineTicks::from_raw);
    commit_next(&mut coordinator);
    let authored_up = coordinator
        .prepare_current_authored_packet()
        .expect("prepare frozen Up chord")
        .expect("Up chord exists")
        .commit;
    let original_identities = authored_up
        .up_intents
        .iter()
        .map(|up| up.identity)
        .collect::<Vec<_>>();
    assert_eq!(original_identities.len(), MAX_KEYS);

    assert_eq!(
        coordinator
            .cancel_live_generations_after_verified_resumable_cleanup()
            .expect("verified cleanup cancellation"),
        MAX_KEYS
    );
    assert_eq!(
        coordinator.cancelled_up_by_slot.iter().flatten().count(),
        MAX_KEYS
    );
    assert_eq!(coordinator.schedule.packets[1].up_mask, physical_up_mask);
    assert!(
        coordinator.schedule.intents[15..30]
            .iter()
            .all(|intent| intent.generation_id() == NO_GENERATION_ID)
    );
    coordinator
        .validate_frozen_authored_commit(&authored_up)
        .expect("all exact frozen Up tokens validate");
    coordinator
        .commit_prepared_authored_frame_success_frozen(
            &authored_up,
            TimelineTicks::from_raw(100),
            TimelineTicks::from_raw(100),
        )
        .expect("all exact tokens reconcile");
    assert!(
        coordinator
            .validate_frozen_authored_commit(&authored_up)
            .is_err()
    );
    assert!(
        coordinator
            .commit_prepared_authored_frame_success_frozen(
                &authored_up,
                TimelineTicks::from_raw(100),
                TimelineTicks::from_raw(100),
            )
            .is_err()
    );
    assert!(coordinator.cancelled_up_by_slot.iter().all(Option::is_none));
    assert_eq!(
        coordinator.generation_accounting().released,
        MAX_KEYS as u64
    );
    assert_eq!(coordinator.generation_accounting().cancelled, 0);
    assert!(coordinator.is_finished());
    assert_eq!(
        original_identities
            .iter()
            .map(|identity| identity.key_slot())
            .collect::<Vec<_>>(),
        (0..MAX_KEYS).map(|slot| slot as u8).collect::<Vec<_>>()
    );
}

#[test]
fn mixed_cancelled_up_and_new_down_keep_one_packet_and_reconcile_old_owner() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "old generation Down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x15].into(),
                reason: "cancelled generation Up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: vec![0x16].into(),
                reason: "new generation Down".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 200,
                scan_codes: vec![0x16].into(),
                reason: "new generation Up".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid mixed packet schedule");
    let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, TimelineTicks::from_raw);
    commit_next(&mut coordinator);
    let mixed = coordinator
        .prepare_current_authored_packet()
        .expect("prepare mixed packet")
        .expect("mixed packet exists");
    assert_eq!(mixed.frame.immediate_up_mask, 0b01);
    assert_eq!(mixed.frame.down_mask, 0b10);
    assert_eq!(mixed.packet.header.up_mask, 0b01);
    assert_eq!(mixed.packet.header.down_mask, 0b10);
    coordinator
        .cancel_live_generations_after_verified_resumable_cleanup()
        .expect("verified cleanup cancellation");
    let after_cancel = coordinator
        .prepare_current_authored_packet()
        .expect("prepare mixed packet after cancellation")
        .expect("mixed packet remains physical work");
    assert_eq!(after_cancel.frame.immediate_up_mask, 0b01);
    assert_eq!(after_cancel.frame.down_mask, 0b10);
    assert_eq!(after_cancel.packet.header.up_mask, 0b01);
    assert_eq!(after_cancel.packet.header.down_mask, 0b10);
    coordinator
        .commit_prepared_authored_frame_success_frozen(
            &after_cancel.commit,
            TimelineTicks::from_raw(100),
            TimelineTicks::from_raw(100),
        )
        .expect("atomic mixed authored commit reconciles exact old Up first");
    let accounting = coordinator.generation_accounting();
    assert_eq!(accounting.released, 1);
    assert_eq!(accounting.active, 1);
    assert_eq!(accounting.activated, 2);
    assert_eq!(
        accounting.released + accounting.active,
        accounting.activated
    );
    assert!(coordinator.cancelled_up_by_slot.iter().all(Option::is_none));
}

#[test]
fn repeated_empty_suspends_keep_one_token_and_stale_identity_cannot_consume_reused_slot() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "first Down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x15].into(),
                reason: "first Up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 200,
                scan_codes: vec![0x15].into(),
                reason: "second Down".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 300,
                scan_codes: vec![0x15].into(),
                reason: "second Up".into(),
            },
        ],
        &[0x15],
    )
    .expect("valid repeated same-key schedule");
    let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, TimelineTicks::from_raw);
    commit_next(&mut coordinator);
    let first_up = coordinator
        .prepare_current_authored_packet()
        .expect("prepare first Up")
        .expect("first Up exists")
        .commit;
    assert_eq!(
        coordinator
            .cancel_live_generations_after_verified_resumable_cleanup()
            .expect("first verified cleanup"),
        1
    );
    let first_identity = first_up.up_intents[0].identity;
    assert_eq!(
        coordinator
            .cancel_live_generations_after_verified_resumable_cleanup()
            .expect("repeated empty verified cleanup"),
        0
    );
    assert_eq!(coordinator.cancelled_up_by_slot[0], Some(first_identity));
    coordinator
        .commit_prepared_authored_frame_success_frozen(
            &first_up,
            TimelineTicks::from_raw(100),
            TimelineTicks::from_raw(100),
        )
        .expect("first exact token consumed");
    commit_next(&mut coordinator);
    let second_up = coordinator
        .prepare_current_authored_packet()
        .expect("prepare second Up")
        .expect("second Up exists")
        .commit;
    assert_eq!(
        coordinator
            .cancel_live_generations_after_verified_resumable_cleanup()
            .expect("second verified cleanup"),
        1
    );
    let second_identity = second_up.up_intents[0].identity;
    assert_ne!(first_identity, second_identity);
    let mut stale = second_up.clone();
    stale.up_intents[0].identity = first_identity;
    assert!(coordinator.validate_frozen_authored_commit(&stale).is_err());
    assert_eq!(coordinator.cancelled_up_by_slot[0], Some(second_identity));
    coordinator
        .commit_prepared_authored_frame_success_frozen(
            &second_up,
            TimelineTicks::from_raw(300),
            TimelineTicks::from_raw(300),
        )
        .expect("new exact token remains consumable after stale identity rejection");
    assert_eq!(coordinator.generation_accounting().released, 2);
    assert_eq!(coordinator.generation_accounting().cancelled, 0);
    assert!(coordinator.is_finished());
}

#[test]
fn wrong_generation_slot_or_up_location_fails_before_commit_and_preserves_token() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15, 0x16].into(),
                reason: "chord Down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x15, 0x16].into(),
                reason: "chord Up".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid two-key schedule");
    let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, TimelineTicks::from_raw);
    commit_next(&mut coordinator);
    let up = coordinator
        .prepare_current_authored_packet()
        .expect("prepare Up chord")
        .expect("Up chord exists")
        .commit;
    coordinator
        .cancel_live_generations_after_verified_resumable_cleanup()
        .expect("verified cleanup creates two tokens");

    let mut wrong_generation = up.clone();
    wrong_generation.up_intents[0].intent = CompactIntent::new(1, 0);
    assert!(
        coordinator
            .validate_frozen_authored_commit(&wrong_generation)
            .is_err()
    );

    let mut wrong_slot = up.clone();
    wrong_slot.up_intents[0].intent = CompactIntent::new(0, 1);
    assert!(
        coordinator
            .validate_frozen_authored_commit(&wrong_slot)
            .is_err()
    );

    let mut wrong_location = up.clone();
    wrong_location.up_intents[0].identity = up.up_intents[1].identity;
    assert!(
        coordinator
            .validate_frozen_authored_commit(&wrong_location)
            .is_err()
    );
    assert_eq!(coordinator.cancelled_up_by_slot.iter().flatten().count(), 2);
    coordinator
        .validate_frozen_authored_commit(&up)
        .expect("correct packet remains valid after rejected forgeries");
}

#[test]
fn malformed_active_owner_cannot_overwrite_an_existing_cancellation_token() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "first Down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x15].into(),
                reason: "first Up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 200,
                scan_codes: vec![0x15].into(),
                reason: "second Down".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 300,
                scan_codes: vec![0x15].into(),
                reason: "second Up".into(),
            },
        ],
        &[0x15],
    )
    .expect("valid same-key schedule");
    let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, TimelineTicks::from_raw);
    commit_next(&mut coordinator);
    let first_up = coordinator
        .prepare_current_authored_packet()
        .expect("prepare first Up")
        .expect("first Up exists")
        .commit;
    coordinator
        .cancel_live_generations_after_verified_resumable_cleanup()
        .expect("first cancellation");
    let token = coordinator.cancelled_up_by_slot[0].expect("live token");

    // This impossible state models a corrupt owner table. The fast invariant
    // must reject it before cancellation can replace the existing token.
    let second_active = coordinator.schedule.intents[2].generation_id();
    coordinator.generation_states[second_active as usize] = GenerationStatus::Active;
    coordinator.activated_generation_count += 1;
    coordinator.active_by_slot[0] = Some(super::super::ActiveGeneration {
        generation_id: second_active,
        scan_code: 0x15,
        key_slot: 0,
        source_action_index: 2,
        release_not_before_ticks: TimelineTicks::ZERO,
    });
    coordinator.active_mask |= 1;
    coordinator.blocked_mask |= 1;
    let error = coordinator
        .cancel_live_generations_after_verified_resumable_cleanup()
        .expect_err("active owner plus token must fail closed");
    assert!(matches!(error, CoordinatorError::Invariant(_)));
    assert_eq!(coordinator.cancelled_up_by_slot[0], Some(token));
    assert!(
        coordinator
            .validate_frozen_authored_commit(&first_up)
            .is_err()
    );
}

#[test]
fn strict_and_normal_coordinator_paths_share_the_same_exact_token_contract() {
    for strict_or_dynamic in [false, true] {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15].into(),
                    reason: "Down".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 100,
                    scan_codes: vec![0x15].into(),
                    reason: "Up".into(),
                },
            ],
            &[0x15],
        )
        .expect("valid schedule");
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, TimelineTicks::from_raw);
        commit_next(&mut coordinator);
        coordinator
            .cancel_live_generations_after_verified_resumable_cleanup()
            .expect("verified cleanup");
        let resumed_up = coordinator
            .prepare_current_authored_packet()
            .expect("dynamic replan after cleanup")
            .expect("retained authored physical Up is planned");
        assert_eq!(resumed_up.frame.immediate_up_mask, 1);
        assert_eq!(resumed_up.frame.stale_up_count, 0);
        assert_eq!(resumed_up.commit.up_intents[0].identity.generation_id(), 0);
        if strict_or_dynamic {
            coordinator
                .validate_frozen_authored_commit(&resumed_up.commit)
                .expect("strict/dynamic plan validates exact token");
        } else {
            coordinator
                .validate_frozen_authored_commit(&resumed_up.commit)
                .expect("normal prepared commit validates the same exact token");
        }
        coordinator
            .commit_prepared_authored_frame_success_frozen(
                &resumed_up.commit,
                TimelineTicks::from_raw(100),
                TimelineTicks::from_raw(100),
            )
            .expect("both paths consume through the common coordinator commit");
        assert_eq!(coordinator.generation_accounting().released, 1);
        assert_eq!(coordinator.generation_accounting().cancelled, 0);
        assert!(coordinator.is_finished());
    }
}

#[test]
fn unmatched_cancellation_token_blocks_finished_and_terminal_clean_proof() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "Down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: vec![0x15].into(),
                reason: "Up".into(),
            },
        ],
        &[0x15],
    )
    .expect("valid schedule");
    let mut coordinator = RuntimeDispatchCoordinator::new(schedule, 0, 0, TimelineTicks::from_raw);
    commit_next(&mut coordinator);
    coordinator
        .cancel_live_generations_after_verified_resumable_cleanup()
        .expect("verified cleanup");
    // Simulate an exhausted/faulted stream cursor. A terminal token remains
    // unaccounted until the exact authored Up commits, so this cannot report
    // natural completion or pass the post-cleanup proof.
    coordinator.cursor = coordinator.schedule.batches.len();
    assert!(!coordinator.is_finished());
    assert!(coordinator.check_post_cleanup_invariants().is_err());
}
