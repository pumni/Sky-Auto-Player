use super::*;
use crate::time::TimelineTicks;

#[test]
fn test_compile_basic_pairing() {
    let allowed = vec![1, 2, 3];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 1000,
            scan_codes: smallvec::smallvec![1, 2],
            reason: "chord".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 2000,
            scan_codes: smallvec::smallvec![1],
            reason: "release".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: 2100,
            scan_codes: smallvec::smallvec![2],
            reason: "release".into(),
        },
    ];

    let sched = compile_runtime_intents(&actions, &allowed).unwrap();
    assert_eq!(sched.generation_count, 2);
    assert_eq!(sched.batches.len(), 3);

    // Down batch has gen 0 and gen 1
    let down = sched.materialize_batch(0, 0);
    assert_eq!(down.intents[0].generation_id, Some(0));
    assert_eq!(down.intents[1].generation_id, Some(1));

    // Up 1 matches gen 0
    let up_one = sched.materialize_batch(1, 0);
    assert_eq!(up_one.intents[0].generation_id, Some(0));
    // Up 2 matches gen 1
    let up_two = sched.materialize_batch(2, 0);
    assert_eq!(up_two.intents[0].generation_id, Some(1));
}

#[test]
fn test_unmatched_up_suppressed() {
    let allowed = vec![1];
    let actions = vec![KeyActionInput {
        source_action_index: 0,
        kind: ActionKind::Up,
        scheduled_us: 1000,
        scan_codes: smallvec::smallvec![1],
        reason: "stale".into(),
    }];

    let sched = compile_runtime_intents(&actions, &allowed).unwrap();
    assert_eq!(sched.generation_count, 0);
    assert_eq!(sched.materialize_batch(0, 0).intents[0].generation_id, None);
}

#[test]
fn rejects_unclosed_generation_at_eof() {
    let error = compile_runtime_intents(
        &[KeyActionInput {
            source_action_index: 7,
            kind: ActionKind::Down,
            scheduled_us: 1234,
            scan_codes: smallvec::smallvec![1],
            reason: "open".into(),
        }],
        &[1],
    )
    .expect_err("an authored Down must have a matching Up");

    assert_eq!(
        error,
        CompileError::UnclosedGeneration {
            scan_code: 1,
            down_source_action_index: 7,
            down_scheduled_us: 1234,
        }
    );
}

#[test]
fn rejects_one_unclosed_generation_in_a_chord() {
    let error = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![1, 2],
                reason: "chord".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 200,
                scan_codes: smallvec::smallvec![1],
                reason: "partial release".into(),
            },
        ],
        &[1, 2],
    )
    .expect_err("every chord key must have an authored release");

    assert_eq!(
        error,
        CompileError::UnclosedGeneration {
            scan_code: 2,
            down_source_action_index: 0,
            down_scheduled_us: 100,
        }
    );
}

#[test]
fn accepts_exact_pair_and_complete_same_key_retrigger_pairs() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![1],
                reason: "first down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 200,
                scan_codes: smallvec::smallvec![1],
                reason: "first up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 300,
                scan_codes: smallvec::smallvec![1],
                reason: "second down".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 400,
                scan_codes: smallvec::smallvec![1],
                reason: "second up".into(),
            },
        ],
        &[1],
    )
    .expect("complete authored generations are valid");

    assert_eq!(schedule.generation_count, 2);
}

#[test]
fn multiple_same_timestamp_down_batches_are_rejected_as_non_atomic() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![1],
            reason: "left".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![1],
            reason: "release".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![2],
            reason: "right".into(),
        },
    ];
    assert!(matches!(
        compile_runtime_intents(&actions, &[1, 2]),
        Err(CompileError::SameTimestampDownBatch { scheduled_us: 100 })
    ));
}

#[test]
fn mixed_timestamp_packet_canonicalizes_disjoint_up_before_down() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![1],
                reason: "first".into(),
            },
            // Authored order is deliberately Down-before-Up at this
            // timestamp. The packet must still release generation 0
            // before activating the unrelated key's generation 1.
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 200,
                scan_codes: smallvec::smallvec![2],
                reason: "new key".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 200,
                scan_codes: smallvec::smallvec![1],
                reason: "release".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 300,
                scan_codes: smallvec::smallvec![2],
                reason: "release".into(),
            },
        ],
        &[1, 2],
    )
    .unwrap();

    assert_eq!(schedule.packets.len(), 3);
    assert_eq!(schedule.packets[0].packet_id, 0);
    assert_eq!(schedule.packets[1].packet_id, 1);
    let packet = schedule.view_packet_ticks(1, TimelineTicks::ZERO).unwrap();
    assert_eq!(packet.up_mask(), 0b01);
    assert_eq!(packet.down_mask(), 0b10);
    assert_eq!(packet.up_mask() & packet.down_mask(), 0);
    assert_eq!(packet.header.down_source_action_index, Some(1));
    assert_eq!(packet.up_intents[0].generation_id(), 0);
    assert_eq!(packet.down_intents[0].generation_id(), 1);
    assert_eq!(schedule.batches[1].kind, ActionKind::Down);
    assert_eq!(schedule.batches[2].kind, ActionKind::Up);
}

#[test]
fn matched_same_timestamp_up_down_is_rejected_with_typed_error() {
    let error = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![1],
                reason: "first".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 200,
                scan_codes: smallvec::smallvec![1],
                reason: "retrigger".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 200,
                scan_codes: smallvec::smallvec![1],
                reason: "release".into(),
            },
        ],
        &[1],
    )
    .expect_err("matched same-timestamp retrigger must be rejected");

    assert_eq!(
        error,
        CompileError::MatchedSameTimestampUpDown {
            scan_code: 1,
            scheduled_us: 200,
            up_source_action_index: 2,
            down_source_action_index: 1,
        }
    );
}

#[test]
fn duplicate_same_timestamp_up_is_rejected() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Up,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![1],
            reason: "stale one".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![1],
            reason: "stale two".into(),
        },
    ];
    assert!(matches!(
        compile_runtime_intents(&actions, &[1]),
        Err(CompileError::DuplicateSameTimestampUp {
            scan_code: 1,
            scheduled_us: 100
        })
    ));
}

#[test]
fn stale_up_is_not_included_in_packet_physical_mask() {
    let schedule = compile_runtime_intents(
        &[KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Up,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![1],
            reason: "stale".into(),
        }],
        &[1],
    )
    .unwrap();

    let packet = schedule.view_packet_ticks(0, TimelineTicks::ZERO).unwrap();
    assert_eq!(packet.up_mask(), 0);
    assert_eq!(packet.up_intents.len(), 1);
    assert_eq!(packet.up_intents[0].generation_id(), NO_GENERATION_ID);
}

#[test]
fn stale_same_timestamp_up_does_not_reject_new_down() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![1],
                reason: "stale release".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![1],
                reason: "new press".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 200,
                scan_codes: smallvec::smallvec![1],
                reason: "release".into(),
            },
        ],
        &[1],
    )
    .expect("stale logical Up must not conflict with a new Down");

    let packet = schedule.view_packet_ticks(0, TimelineTicks::ZERO).unwrap();
    assert_eq!(packet.up_mask(), 0);
    assert_eq!(packet.down_mask(), 1);
    assert_eq!(packet.up_mask() & packet.down_mask(), 0);
}

#[test]
fn successful_compilation_keeps_physical_packet_masks_disjoint() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![1],
                reason: "down one".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![1],
                reason: "up one".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![2],
                reason: "down two".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 200,
                scan_codes: smallvec::smallvec![2],
                reason: "up two".into(),
            },
        ],
        &[1, 2],
    )
    .expect("disjoint mixed packets must compile");

    assert!(
        schedule
            .packets
            .iter()
            .all(|packet| packet.up_mask & packet.down_mask == 0)
    );
}

#[test]
fn multi_up_only_packet_has_up_only_kind() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![1, 2],
                reason: "down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![1],
                reason: "up one".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![2],
                reason: "up two".into(),
            },
        ],
        &[1, 2],
    )
    .unwrap();
    let packet = schedule.packets[1];
    assert_eq!(packet.up_mask, 0b11);
    assert_eq!(packet.down_mask, 0);
    assert_eq!(
        crate::coordinator::physical_packet_kind(packet.up_mask, packet.down_mask),
        Ok(PhysicalPacketKind::UpOnly)
    );
}

#[test]
fn empty_packet_is_rejected_as_invariant_error() {
    assert!(matches!(
        crate::coordinator::physical_packet_kind(0, 0),
        Err(crate::coordinator::CoordinatorError::Invariant(_))
    ));
}

#[test]
fn test_rejects_non_monotonic_and_untrusted_actions() {
    let allowed = vec![1];
    let invalid = vec![
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Down,
            scheduled_us: 2,
            scan_codes: smallvec::smallvec![1],
            reason: "first".into(),
        },
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Up,
            scheduled_us: 1,
            scan_codes: smallvec::smallvec![1],
            reason: "second".into(),
        },
    ];
    assert!(matches!(
        compile_runtime_intents(&invalid, &allowed),
        Err(CompileError::NonMonotonicSourceIndex { .. })
    ));

    let outside_allowlist = vec![KeyActionInput {
        source_action_index: 0,
        kind: ActionKind::Down,
        scheduled_us: 0,
        scan_codes: smallvec::smallvec![2],
        reason: "invalid".into(),
    }];
    assert!(matches!(
        compile_runtime_intents(&outside_allowlist, &allowed),
        Err(CompileError::ScanCodeNotAllowed { scan_code: 2, .. })
    ));
}

#[test]
fn schedule_uses_a_flat_intent_arena() {
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 10,
                scan_codes: smallvec::smallvec![1],
                reason: "single".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 20,
                scan_codes: smallvec::smallvec![2, 3],
                reason: "chord".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 30,
                scan_codes: smallvec::smallvec![1],
                reason: "release".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 40,
                scan_codes: smallvec::smallvec![2, 3],
                reason: "release".into(),
            },
        ],
        &[1, 2, 3],
    )
    .unwrap();
    assert_eq!(schedule.intents.len(), 6);
    assert_eq!(schedule.batches[0].intent_len, 1);
    assert_eq!(schedule.batches[1].intent_len, 2);
    assert!(std::mem::size_of::<CompiledBatch>() <= 32);
    assert_eq!(std::mem::size_of::<CompactIntent>(), 8);
}

#[test]
fn test_reject_overlapping_same_key_down() {
    let allowed = vec![1];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 1000,
            scan_codes: smallvec::smallvec![1],
            reason: "first down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Down,
            scheduled_us: 2000,
            scan_codes: smallvec::smallvec![1],
            reason: "overlapping down".into(),
        },
    ];
    let err = compile_runtime_intents(&actions, &allowed).unwrap_err();
    assert!(matches!(
        err,
        CompileError::OverlappingSameKeyDown {
            scan_code: 1,
            first_down_action_index: 0,
            second_down_action_index: 1,
            ..
        }
    ));
}

#[test]
fn test_allow_down_down_different_keys() {
    let allowed = vec![1, 2];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 1000,
            scan_codes: smallvec::smallvec![1],
            reason: "down 1".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Down,
            scheduled_us: 2000,
            scan_codes: smallvec::smallvec![2],
            reason: "down 2".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: 3000,
            scan_codes: smallvec::smallvec![1, 2],
            reason: "release".into(),
        },
    ];
    let sched = compile_runtime_intents(&actions, &allowed).unwrap();
    assert_eq!(sched.generation_count, 2);
}

#[test]
fn test_reject_chord_overlapping_active_key() {
    let allowed = vec![1, 2, 3];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 1000,
            scan_codes: smallvec::smallvec![1, 2],
            reason: "chord 1".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 1500,
            scan_codes: smallvec::smallvec![1],
            reason: "release 1".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 2000,
            scan_codes: smallvec::smallvec![2, 3],
            reason: "chord 2".into(),
        },
    ];
    let err = compile_runtime_intents(&actions, &allowed).unwrap_err();
    assert!(matches!(
        err,
        CompileError::OverlappingSameKeyDown { scan_code: 2, .. }
    ));
}

#[test]
fn test_reused_key_after_up_allowed() {
    let allowed = vec![1];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 1000,
            scan_codes: smallvec::smallvec![1],
            reason: "first down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 1500,
            scan_codes: smallvec::smallvec![1],
            reason: "release".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 2000,
            scan_codes: smallvec::smallvec![1],
            reason: "second down".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 3000,
            scan_codes: smallvec::smallvec![1],
            reason: "second up".into(),
        },
    ];
    let sched = compile_runtime_intents(&actions, &allowed).unwrap();
    assert_eq!(sched.generation_count, 2);
}
