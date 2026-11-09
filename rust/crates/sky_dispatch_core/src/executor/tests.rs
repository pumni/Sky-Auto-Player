use super::*;
use crate::compile::compile_runtime_intents;
use crate::model::{ActionKind, KeyActionInput};
use crate::time::{DurationTicks, TimelineTicks};

/// Helper to compile a schedule from simplified actions.
fn compile_actions(actions: &[KeyActionInput], allowed: &[u16]) -> RuntimeSchedule {
    compile_runtime_intents(actions, allowed).expect("valid test schedule")
}

#[test]
fn fixture_1_independent_release_a_floor_does_not_block_b_down() {
    // A-Up authored 20 ms, floor 25 ms; B-Down independent at 21 ms
    // H = 25 ms, T = 16 ms
    let allowed = vec![0x15, 0x16]; // 0x15 = Key A (slot 0), 0x16 = Key B (slot 1)
    let actions = vec![
        // Down A at 0 ms
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down-a".into(),
        },
        // Up A authored at 20 ms (20_000 us)
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 20_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up-a".into(),
        },
        // Down B at 21 ms (21_000 us)
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 21_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "down-b".into(),
        },
        // Up B at 40 ms (40_000 us)
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 40_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "up-b".into(),
        },
    ];

    let sched = compile_actions(&actions, &allowed);
    let min_hold = DurationTicks::from_raw(25_000); // H = 25 ms floor
    let frame_time = DurationTicks::from_raw(16_000); // T = 16 ms
    let mut executor =
        DispatchExecutor::new(&sched, min_hold, frame_time, TimelineTicks::from_raw).unwrap();

    // 1. At 0 ms: Down A is ready
    let step0 = executor.evaluate(TimelineTicks::from_raw(0));
    assert!(matches!(
        step0,
        DispatchAction::Send {
            up_mask: 0,
            down_chord: Some(chord)
        } if chord.down_mask == 0x01
    ));
    // Down A completed at 0 ms
    executor
        .on_send_completed(0, 0x01, TimelineTicks::from_raw(0))
        .unwrap();

    // Slot 0 (A) is now active. Its authored Up is 20_000, but floor is 0 + 25_000 = 25_000.
    // So A is due at 25_000 us.

    // 2. At 20 ms (20_000 us):
    // Authored Up is 20 ms, but floor is 25 ms.
    // Down B is scheduled at 21 ms.
    // Step should wait until 21 ms (when B becomes due)!
    let step20 = executor.evaluate(TimelineTicks::from_raw(20_000));
    assert_eq!(
        step20,
        DispatchAction::Wait {
            deadline_ticks: TimelineTicks::from_raw(21_000)
        }
    );

    // 3. At 21 ms (21_000 us):
    // A-Up is still waiting for floor (25 ms).
    // B-Down is independent and SCHEDULED AT 21 ms.
    // B-Down MUST BE ATTEMPTED AT 21 ms without being blocked by A!
    let step21 = executor.evaluate(TimelineTicks::from_raw(21_000));
    assert!(
        matches!(
            step21,
            DispatchAction::Send {
                up_mask: 0,
                down_chord: Some(chord)
            } if chord.down_mask == 0x02
        ),
        "B-Down must be dispatched at 21 ms without being blocked by A waiting for H floor"
    );
    // B-Down completes at 21 ms
    executor
        .on_send_completed(0, 0x02, TimelineTicks::from_raw(21_000))
        .unwrap();

    // 4. Between 21 ms and 25 ms:
    let step22 = executor.evaluate(TimelineTicks::from_raw(22_000));
    assert_eq!(
        step22,
        DispatchAction::Wait {
            deadline_ticks: TimelineTicks::from_raw(25_000)
        }
    );

    // 5. At 25 ms (25_000 us):
    // A-Up reaches its H floor of 25 ms!
    let step25 = executor.evaluate(TimelineTicks::from_raw(25_000));
    assert!(
        matches!(
            step25,
            DispatchAction::Send {
                up_mask: 0x01,
                down_chord: None
            }
        ),
        "A-Up must be dispatched at 25 ms floor"
    );
    executor
        .on_send_completed(0x01, 0, TimelineTicks::from_raw(25_000))
        .unwrap();
    assert_eq!(executor.active_mask(), 0x02); // Only B is active now
}

#[test]
fn fixture_2_same_key_down_waits_up_completion_plus_t() {
    // Same-key Down on A waits for Up completion + T
    let allowed = vec![0x15];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 10_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 20_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first up".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 28_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "second down".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 40_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "second up".into(),
        },
    ];

    let sched = compile_actions(&actions, &allowed);
    let min_hold = DurationTicks::from_raw(5_000); // H = 5 ms
    let frame_time = DurationTicks::from_raw(16_000); // T = 16 ms
    let mut executor =
        DispatchExecutor::new(&sched, min_hold, frame_time, TimelineTicks::from_raw).unwrap();

    // Down 1
    assert!(matches!(
        executor.evaluate(TimelineTicks::from_raw(10_000)),
        DispatchAction::Send {
            up_mask: 0,
            down_chord: Some(_)
        }
    ));
    executor
        .on_send_completed(0, 0x01, TimelineTicks::from_raw(10_000))
        .unwrap();

    // Up 1 at 20_000
    assert!(matches!(
        executor.evaluate(TimelineTicks::from_raw(20_000)),
        DispatchAction::Send {
            up_mask: 0x01,
            down_chord: None
        }
    ));
    // Up 1 completes at 20_000. Therefore next Down on A cannot happen before 20_000 + 16_000 = 36_000!
    executor
        .on_send_completed(0x01, 0, TimelineTicks::from_raw(20_000))
        .unwrap();

    // Second Down is authored at 28_000.
    // At 28_000:
    let step28 = executor.evaluate(TimelineTicks::from_raw(28_000));
    assert_eq!(
        step28,
        DispatchAction::Wait {
            deadline_ticks: TimelineTicks::from_raw(36_000)
        },
        "Same-key Down must wait until Up completion + T = 36 ms"
    );

    // At 35_999: still waiting
    let step35 = executor.evaluate(TimelineTicks::from_raw(35_999));
    assert!(matches!(step35, DispatchAction::Wait { .. }));

    // At 36_000: now T gap has elapsed, Down 2 is dispatched!
    let step36 = executor.evaluate(TimelineTicks::from_raw(36_000));
    assert!(
        matches!(
            step36,
            DispatchAction::Send {
                up_mask: 0,
                down_chord: Some(_)
            }
        ),
        "Down on A is eligible exactly at Up completion + T"
    );
}

#[test]
fn fixture_3_and_4_chord_blocked_by_member_does_not_split_while_unrelated_up_proceeds() {
    // Chord {B, C} at 21 ms blocked by C; B is NOT sent alone.
    // Meanwhile, unrelated Up A is due at 22 ms and MUST proceed while chord waits.
    let allowed = vec![0x15, 0x16, 0x17]; // 0x15 = A, 0x16 = B, 0x17 = C
    let actions = vec![
        // Chord {A, C} down at 0 ms
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15, 0x17],
            reason: "down-ac".into(),
        },
        // C up at 10 ms (so C next down floor = 10 ms + 16 ms = 26 ms)
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 10_000,
            scan_codes: smallvec::smallvec![0x17],
            reason: "up-c".into(),
        },
        // Chord {B, C} at 21 ms
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 21_000,
            scan_codes: smallvec::smallvec![0x16, 0x17],
            reason: "chord-bc".into(),
        },
        // Up A at 22 ms
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 22_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up-a".into(),
        },
        // Up B and Up C at 40 ms
        KeyActionInput {
            source_action_index: 4,
            kind: ActionKind::Up,
            scheduled_us: 40_000,
            scan_codes: smallvec::smallvec![0x16, 0x17],
            reason: "up-bc".into(),
        },
    ];

    let sched = compile_actions(&actions, &allowed);
    let min_hold = DurationTicks::from_raw(5_000); // H = 5 ms
    let frame_time = DurationTicks::from_raw(16_000); // T = 16 ms
    let mut executor =
        DispatchExecutor::new(&sched, min_hold, frame_time, TimelineTicks::from_raw).unwrap();

    // 0 ms: Down {A, C} (chord 0 has A, chord 1 has C? Wait, A and C are at same timestamp 0 so compiler combined them into chord {A, C})
    let step0 = executor.evaluate(TimelineTicks::from_raw(0));
    assert!(matches!(
        step0,
        DispatchAction::Send {
            up_mask: 0,
            down_chord: Some(chord)
        } if chord.down_mask == 0x05 // A (0x01) | C (0x04)
    ));
    executor
        .on_send_completed(0, 0x05, TimelineTicks::from_raw(0))
        .unwrap();

    // 10 ms: Up C is due
    let step10 = executor.evaluate(TimelineTicks::from_raw(10_000));
    assert!(matches!(
        step10,
        DispatchAction::Send {
            up_mask: 0x04, // Up C
            down_chord: None
        }
    ));
    // C Up completes at 10_000. So C's next Down floor is 10_000 + 16_000 = 26_000 us!
    executor
        .on_send_completed(0x04, 0, TimelineTicks::from_raw(10_000))
        .unwrap();

    // At 21 ms: Chord {B, C} is scheduled.
    // B is free, but C is BLOCKED by T floor (until 26 ms).
    // The chord must NOT split: B must NOT be dispatched alone!
    let step21 = executor.evaluate(TimelineTicks::from_raw(21_000));
    assert_eq!(
        step21,
        DispatchAction::Wait {
            deadline_ticks: TimelineTicks::from_raw(22_000) // Up A is due at 22 ms!
        },
        "Chord {{B, C}} must wait intact; B cannot split, next wake is Up A at 22 ms"
    );

    // At 22 ms: Up A is due!
    // Chord {B, C} is STILL BLOCKED by C (needs 26 ms).
    // Up A MUST proceed while chord {B, C} is waiting!
    let step22 = executor.evaluate(TimelineTicks::from_raw(22_000));
    assert!(
        matches!(
            step22,
            DispatchAction::Send {
                up_mask: 0x01, // Up A
                down_chord: None
            }
        ),
        "Up A must be dispatched at 22 ms even while chord {{B, C}} is waiting"
    );
    executor
        .on_send_completed(0x01, 0, TimelineTicks::from_raw(22_000))
        .unwrap();

    // At 26 ms: C's T floor has arrived!
    // Entire chord {B, C} is dispatched intact!
    let step26 = executor.evaluate(TimelineTicks::from_raw(26_000));
    assert!(
        matches!(
            step26,
            DispatchAction::Send {
                up_mask: 0,
                down_chord: Some(chord)
            } if chord.down_mask == 0x06 // B (0x02) | C (0x04)
        ),
        "Chord {{B, C}} must be dispatched intact as one atomic batch at 26 ms"
    );
    executor
        .on_send_completed(0, 0x06, TimelineTicks::from_raw(26_000))
        .unwrap();
}

#[test]
fn fixture_5_mixed_up_down_combined_when_coincident_split_when_divergent() {
    let allowed = vec![0x15, 0x16]; // A, B
    let actions = vec![
        // Down A at 0, Up A at 20 ms
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down-a".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 20_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up-a".into(),
        },
        // Down B at 20 ms (coincident with Up A!)
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 20_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "down-b".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 30_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "up-b".into(),
        },
    ];

    let sched = compile_actions(&actions, &allowed);
    let min_hold = DurationTicks::from_raw(10_000); // H = 10 ms
    let frame_time = DurationTicks::from_raw(16_000); // T = 16 ms
    let mut executor =
        DispatchExecutor::new(&sched, min_hold, frame_time, TimelineTicks::from_raw).unwrap();

    // 0 ms: Down A
    executor
        .on_send_completed(0, 0x01, TimelineTicks::from_raw(0))
        .unwrap();
    // A release floor is 0 + 10 ms = 10 ms. Authored Up is 20 ms.
    // So A Up is due at 20 ms.
    // Down B is scheduled at 20 ms.
    // At 20 ms: Both Up A and Down B are ready!
    let step20 = executor.evaluate(TimelineTicks::from_raw(20_000));
    assert!(
        matches!(
            step20,
            DispatchAction::Send {
                up_mask: 0x01,
                down_chord: Some(chord)
            } if chord.down_mask == 0x02
        ),
        "Coincident Up A and Down B must be combined into Up-before-Down batch"
    );
}

#[test]
fn fixture_6_down_delayed_past_authored_up_creates_valid_release() {
    // Down authored at 10 ms, Up authored at 20 ms.
    // Worker stall delays Down to 35 ms!
    let allowed = vec![0x15];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 10_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 20_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up".into(),
        },
    ];

    let sched = compile_actions(&actions, &allowed);
    let min_hold = DurationTicks::from_raw(10_000); // H = 10 ms
    let frame_time = DurationTicks::from_raw(16_000);
    let mut executor =
        DispatchExecutor::new(&sched, min_hold, frame_time, TimelineTicks::from_raw).unwrap();

    // Down completes late at 35 ms (past authored Up of 20 ms)
    executor
        .on_send_completed(0, 0x01, TimelineTicks::from_raw(35_000))
        .unwrap();

    // Release due MUST be max(20 ms, 35 ms + 10 ms) = 45 ms!
    let step35 = executor.evaluate(TimelineTicks::from_raw(35_000));
    assert_eq!(
        step35,
        DispatchAction::Wait {
            deadline_ticks: TimelineTicks::from_raw(45_000)
        },
        "Release obligation must be preserved with completion + H floor (45 ms)"
    );

    // At 45 ms: Up is dispatched!
    let step45 = executor.evaluate(TimelineTicks::from_raw(45_000));
    assert!(
        matches!(
            step45,
            DispatchAction::Send {
                up_mask: 0x01,
                down_chord: None
            }
        ),
        "Up must be dispatched at 45 ms and never lost"
    );
}

#[test]
fn fixture_7_15_slots_distinct_deadlines_no_starvation() {
    // 15 slots with distinct release deadlines
    let allowed: Vec<u16> = (0..MAX_KEYS).map(|i| 0x100 + i as u16).collect();
    let mut actions = Vec::new();
    // Chord of all 15 keys at 10 ms
    actions.push(KeyActionInput {
        source_action_index: 0,
        kind: ActionKind::Down,
        scheduled_us: 10_000,
        scan_codes: allowed.iter().copied().collect(),
        reason: "chord".into(),
    });
    // Staggered releases at 20 ms, 21 ms, 22 ms, ... 34 ms
    for (i, &sc) in allowed.iter().enumerate() {
        actions.push(KeyActionInput {
            source_action_index: 1 + i as u32,
            kind: ActionKind::Up,
            scheduled_us: 20_000 + (i as u64) * 1_000,
            scan_codes: smallvec::smallvec![sc],
            reason: format!("up-{sc}").into(),
        });
    }

    let sched = compile_actions(&actions, &allowed);
    let min_hold = DurationTicks::from_raw(5_000);
    let frame_time = DurationTicks::from_raw(16_000);
    let mut executor =
        DispatchExecutor::new(&sched, min_hold, frame_time, TimelineTicks::from_raw).unwrap();

    // Down chord of 15 keys completed at 10 ms
    executor
        .on_send_completed(0, 0x7FFF, TimelineTicks::from_raw(10_000))
        .unwrap();
    assert_eq!(executor.active_mask(), 0x7FFF);

    // Evaluate each millisecond from 20 ms to 34 ms
    for i in 0..15 {
        let now = 20_000 + (i as u64) * 1_000;
        let step = executor.evaluate(TimelineTicks::from_raw(now));
        let expected_mask = 1u16 << i;
        assert!(
            matches!(
                step,
                DispatchAction::Send {
                    up_mask,
                    down_chord: None
                } if up_mask == expected_mask
            ),
            "Slot {i} must be released exactly at its deadline {now} us without starvation"
        );
        executor
            .on_send_completed(expected_mask, 0, TimelineTicks::from_raw(now))
            .unwrap();
    }

    assert_eq!(executor.active_mask(), 0);
    assert_eq!(executor.accounting().released, 15);
}

#[test]
fn fixture_verified_cleanup_cancels_active_slots_without_replay_and_sets_resume_barrier() {
    let allowed = vec![0x15, 0x16];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 10_000,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "chord".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 30_000,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "up".into(),
        },
        // Second chord at 35 ms
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 35_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down-second".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 50_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up-second".into(),
        },
    ];

    let sched = compile_actions(&actions, &allowed);
    let min_hold = DurationTicks::from_raw(10_000);
    let frame_time = DurationTicks::from_raw(16_000); // T = 16 ms
    let mut executor =
        DispatchExecutor::new(&sched, min_hold, frame_time, TimelineTicks::from_raw).unwrap();

    // Down first chord at 10 ms
    executor
        .on_send_completed(0, 0x03, TimelineTicks::from_raw(10_000))
        .unwrap();
    assert_eq!(executor.active_mask(), 0x03);

    // During sustain (e.g. at 20 ms), focus is lost and verified cleanup occurs at Q = 25 ms
    let cancelled = executor
        .on_verified_cleanup(TimelineTicks::from_raw(25_000))
        .unwrap();
    assert_eq!(cancelled, 2);
    assert_eq!(executor.active_mask(), 0);
    assert_eq!(executor.accounting().cancelled, 2);

    // At 30 ms (the original authored Up time):
    // No physical Up should be replayed!
    // Resume barrier is Q + T = 25 ms + 16 ms = 41 ms.
    // Second Down (scheduled at 35 ms) MUST NOT DISPATCH before 41 ms!
    let step30 = executor.evaluate(TimelineTicks::from_raw(30_000));
    assert_eq!(
        step30,
        DispatchAction::Wait {
            deadline_ticks: TimelineTicks::from_raw(41_000)
        },
        "No stale Up replay; Down must wait until resume barrier Q + T = 41 ms"
    );

    // At 40 ms: still waiting for barrier
    let step40 = executor.evaluate(TimelineTicks::from_raw(40_000));
    assert!(matches!(step40, DispatchAction::Wait { .. }));

    // At 41 ms: barrier passed, second Down is dispatched!
    let step41 = executor.evaluate(TimelineTicks::from_raw(41_000));
    assert!(
        matches!(
            step41,
            DispatchAction::Send {
                up_mask: 0,
                down_chord: Some(chord)
            } if chord.down_mask == 0x01
        ),
        "Second Down dispatched after resume barrier"
    );
}
