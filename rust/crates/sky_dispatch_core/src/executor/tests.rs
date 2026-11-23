use super::*;
use crate::compile::compile_runtime_intents;
use crate::model::{ActionKind, KeyActionInput};
use crate::time::{DurationTicks, TimelineTicks};

/// Helper to compile a schedule from simplified actions.
fn compile_actions(actions: &[KeyActionInput], allowed: &[u16]) -> RuntimeSchedule {
    compile_runtime_intents(actions, allowed).expect("valid test schedule")
}

fn test_ticks(us: u64) -> Result<TimelineTicks, std::convert::Infallible> {
    Ok(TimelineTicks::from_raw(us))
}

#[test]
fn prepared_plan_starts_fresh_and_matches_raw_executor_decisions() {
    let allowed = vec![0x15];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 5_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first up".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 7_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "second down".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 12_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "second up".into(),
        },
    ];
    let schedule = compile_actions(&actions, &allowed);
    let frequency = std::num::NonZeroU64::new(1_000_000).expect("QPC frequency");
    let prepared = PreparedDispatchSchedule::prepare(&schedule, frequency, 1_000, 0)
        .expect("physical schedule is admissible");
    assert!(prepared.is_compatible(frequency, 1_000));
    assert!(!prepared.is_compatible(
        std::num::NonZeroU64::new(2_000_000).expect("alternate QPC frequency"),
        1_000
    ));
    assert!(!prepared.is_compatible(frequency, 1_001));

    let chord_storage = prepared.down_chords.as_ptr();
    let intent_storage = prepared.down_intents.as_ptr();
    let mut prepared_executor = DispatchExecutor::from_prepared_schedule(prepared);
    let mut raw_executor =
        DispatchExecutor::new(&schedule, DurationTicks::from_raw(1_000), test_ticks)
            .expect("raw schedule executor");
    assert_eq!(prepared_executor.active_mask(), 0);
    assert_eq!(prepared_executor.down_chords.as_ptr(), chord_storage);
    assert_eq!(prepared_executor.down_intents.as_ptr(), intent_storage);
    assert_eq!(
        prepared_executor.total_chords(),
        raw_executor.total_chords()
    );
    for timestamp in [0, 5_000, 7_000, 12_000] {
        let now = crate::time::QpcTicks::from_raw(timestamp);
        let prepared_decision = prepared_executor
            .evaluate_at(now, crate::time::QpcTicks::ZERO)
            .expect("prepared decision");
        let raw_decision = raw_executor
            .evaluate_at(now, crate::time::QpcTicks::ZERO)
            .expect("raw decision");
        assert_eq!(prepared_decision, raw_decision);
        if let DispatchDecision::Send {
            up_mask,
            down_chord,
            ..
        } = prepared_decision
        {
            let down_mask = down_chord.map_or(0, |chord| chord.down_mask);
            prepared_executor
                .on_send_completed_at(up_mask, down_mask, now)
                .expect("prepared send completion");
            raw_executor
                .on_send_completed_at(up_mask, down_mask, now)
                .expect("raw send completion");
        }
    }
    assert_eq!(
        prepared_executor.evaluate_at(
            crate::time::QpcTicks::from_raw(12_000),
            crate::time::QpcTicks::ZERO,
        ),
        Ok(DispatchDecision::Finished)
    );
    assert_eq!(prepared_executor.active_mask(), 0);
}

#[test]
fn prepared_plan_rounds_qpc_offsets_up_and_checkpoints_validation() {
    const FREQUENCY_HZ: u64 = 3_579_545;
    fn ceil_qpc_ticks(microseconds: u64) -> u64 {
        let numerator = u128::from(microseconds) * u128::from(FREQUENCY_HZ) + 999_999;
        u64::try_from(numerator / 1_000_000).expect("fixture QPC conversion")
    }

    #[derive(Debug, Clone, Copy, Eq, PartialEq)]
    enum ObservedDecision {
        Send {
            up_mask: u16,
            down_mask: u16,
            timing_window: DispatchTimingWindow,
        },
        Wait {
            deadline_qpc: u64,
        },
        Finished,
    }

    fn observe(decision: DispatchDecision<'_>) -> ObservedDecision {
        match decision {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
            } => ObservedDecision::Send {
                up_mask,
                down_mask: down_chord.map_or(0, |chord| chord.down_mask),
                timing_window,
            },
            DispatchDecision::Wait { deadline_qpc } => ObservedDecision::Wait {
                deadline_qpc: deadline_qpc.as_u64(),
            },
            DispatchDecision::Finished => ObservedDecision::Finished,
        }
    }

    fn evaluate_decision_pair(
        prepared: &mut DispatchExecutor,
        raw: &mut DispatchExecutor,
        now_qpc: u64,
    ) -> ObservedDecision {
        let now = crate::time::QpcTicks::from_raw(now_qpc);
        let prepared_decision = observe(
            prepared
                .evaluate_at(now, crate::time::QpcTicks::ZERO)
                .expect("prepared executor decision"),
        );
        let raw_decision = observe(
            raw.evaluate_at(now, crate::time::QpcTicks::ZERO)
                .expect("raw executor decision"),
        );
        assert_eq!(prepared_decision, raw_decision, "at QPC {now_qpc}");
        prepared_decision
    }

    fn evaluate_pair(
        prepared: &mut DispatchExecutor,
        raw: &mut DispatchExecutor,
        now_qpc: u64,
        completion_qpc: Option<u64>,
    ) -> ObservedDecision {
        let prepared_decision = evaluate_decision_pair(prepared, raw, now_qpc);
        match &prepared_decision {
            ObservedDecision::Send {
                up_mask, down_mask, ..
            } => {
                let completed_at = crate::time::QpcTicks::from_raw(
                    completion_qpc.expect("Send decision has a completion timestamp"),
                );
                prepared
                    .on_send_completed_at(*up_mask, *down_mask, completed_at)
                    .expect("prepared executor completion");
                raw.on_send_completed_at(*up_mask, *down_mask, completed_at)
                    .expect("raw executor completion");
            }
            ObservedDecision::Wait { .. } | ObservedDecision::Finished => {
                assert!(
                    completion_qpc.is_none(),
                    "non-Send decision has no completion"
                );
            }
        }
        prepared_decision
    }

    let allowed = vec![0x15, 0x16, 0x17, 0x18];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "initial chord".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 50,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "mixed packet releases".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 50,
            scan_codes: smallvec::smallvec![0x17],
            reason: "mixed packet independent down".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Down,
            scheduled_us: 51,
            scan_codes: smallvec::smallvec![0x18],
            reason: "independent key down".into(),
        },
        KeyActionInput {
            source_action_index: 4,
            kind: ActionKind::Down,
            scheduled_us: 57,
            scan_codes: smallvec::smallvec![0x15],
            reason: "same key repress".into(),
        },
        KeyActionInput {
            source_action_index: 5,
            kind: ActionKind::Up,
            scheduled_us: 61,
            scan_codes: smallvec::smallvec![0x18],
            reason: "independent key release".into(),
        },
        KeyActionInput {
            source_action_index: 6,
            kind: ActionKind::Up,
            scheduled_us: 80,
            scan_codes: smallvec::smallvec![0x17],
            reason: "independent down release".into(),
        },
        KeyActionInput {
            source_action_index: 7,
            kind: ActionKind::Up,
            scheduled_us: 90,
            scan_codes: smallvec::smallvec![0x15],
            reason: "same key repress release".into(),
        },
    ];
    let schedule = compile_actions(&actions, &allowed);
    let frequency = std::num::NonZeroU64::new(FREQUENCY_HZ).expect("QPC frequency");
    let min_hold_ticks = ceil_qpc_ticks(10);
    let plan = PreparedDispatchSchedule::prepare(&schedule, frequency, 10, 0)
        .expect("QPC timestamp and floor conversions round up");
    assert_eq!(plan.min_hold_ticks.as_u64(), min_hold_ticks);
    assert_eq!(plan.min_hold_ticks.as_u64(), 36);
    assert_eq!(
        plan.down_intents[0]
            .paired_up_not_before_offset_ticks
            .as_u64(),
        179
    );
    assert_eq!(plan.down_chords[1].scheduled_ticks.as_u64(), 179);
    assert_eq!(plan.down_chords[2].scheduled_ticks.as_u64(), 183);
    assert_eq!(plan.down_chords[3].scheduled_ticks.as_u64(), 205);

    let mut prepared_executor = DispatchExecutor::from_prepared_schedule(plan);
    let mut raw_executor = DispatchExecutor::new(
        &schedule,
        DurationTicks::from_raw(min_hold_ticks),
        |microseconds| {
            Ok::<_, std::convert::Infallible>(TimelineTicks::from_raw(ceil_qpc_ticks(microseconds)))
        },
    )
    .expect("raw QPC-domain schedule executor");

    assert_eq!(
        evaluate_pair(&mut prepared_executor, &mut raw_executor, 0, Some(10)),
        ObservedDecision::Send {
            up_mask: 0,
            down_mask: 0b0011,
            timing_window: DispatchTimingWindow {
                authored_target_qpc: crate::time::QpcTicks::ZERO,
                musical_up_not_before_qpc: crate::time::QpcTicks::ZERO,
                down_not_before_qpc: crate::time::QpcTicks::ZERO,
                packet_not_before_qpc: crate::time::QpcTicks::ZERO,
                hold_floor_mask: 0,
            },
        }
    );
    assert_eq!(
        evaluate_pair(&mut prepared_executor, &mut raw_executor, 179, Some(200)),
        ObservedDecision::Send {
            up_mask: 0b0011,
            down_mask: 0b0100,
            timing_window: DispatchTimingWindow {
                authored_target_qpc: crate::time::QpcTicks::from_raw(179),
                musical_up_not_before_qpc: crate::time::QpcTicks::from_raw(179),
                down_not_before_qpc: crate::time::QpcTicks::from_raw(179),
                packet_not_before_qpc: crate::time::QpcTicks::from_raw(179),
                hold_floor_mask: 0,
            },
        }
    );
    assert_eq!(
        evaluate_pair(&mut prepared_executor, &mut raw_executor, 200, Some(205)),
        ObservedDecision::Send {
            up_mask: 0,
            down_mask: 0b1000,
            timing_window: DispatchTimingWindow {
                authored_target_qpc: crate::time::QpcTicks::from_raw(183),
                musical_up_not_before_qpc: crate::time::QpcTicks::from_raw(183),
                down_not_before_qpc: crate::time::QpcTicks::from_raw(183),
                packet_not_before_qpc: crate::time::QpcTicks::from_raw(183),
                hold_floor_mask: 0,
            },
        }
    );
    assert!(matches!(
        evaluate_pair(&mut prepared_executor, &mut raw_executor, 205, Some(230)),
        ObservedDecision::Send { down_mask: 0b0001, timing_window: DispatchTimingWindow { down_not_before_qpc, packet_not_before_qpc, .. }, .. }
            if down_not_before_qpc.as_u64() == 205 && packet_not_before_qpc.as_u64() == 205
    ));
    assert_eq!(
        evaluate_pair(&mut prepared_executor, &mut raw_executor, 240, None),
        ObservedDecision::Wait { deadline_qpc: 241 }
    );
    assert!(matches!(
        evaluate_decision_pair(&mut prepared_executor, &mut raw_executor, 241),
        ObservedDecision::Send { up_mask: 0b1000, timing_window: DispatchTimingWindow { musical_up_not_before_qpc, hold_floor_mask: 0b1000, .. }, .. }
            if musical_up_not_before_qpc.as_u64() == 241
    ));
    let prepared_past_h = evaluate_decision_pair(&mut prepared_executor, &mut raw_executor, 242);
    assert!(matches!(
        prepared_past_h,
        ObservedDecision::Send {
            up_mask: 0b1000,
            ..
        }
    ));
    assert!(matches!(
        evaluate_pair(&mut prepared_executor, &mut raw_executor, 241, Some(245)),
        ObservedDecision::Send { up_mask: 0b1000, timing_window: DispatchTimingWindow { musical_up_not_before_qpc, hold_floor_mask: 0b1000, .. }, .. }
            if musical_up_not_before_qpc.as_u64() == 241
    ));
    assert!(matches!(
        evaluate_pair(&mut prepared_executor, &mut raw_executor, 287, Some(290)),
        ObservedDecision::Send {
            up_mask: 0b0100,
            ..
        }
    ));
    assert!(matches!(
        evaluate_pair(&mut prepared_executor, &mut raw_executor, 323, Some(330)),
        ObservedDecision::Send {
            up_mask: 0b0001,
            ..
        }
    ));
    assert_eq!(
        evaluate_pair(&mut prepared_executor, &mut raw_executor, 330, None),
        ObservedDecision::Finished
    );
    assert_eq!(prepared_executor.active_mask(), 0);
    assert_eq!(raw_executor.active_mask(), 0);

    let many_actions = (0..5_000_u32)
        .flat_map(|index| {
            let at_us = u64::from(index) * 2;
            [
                KeyActionInput {
                    source_action_index: index * 2,
                    kind: ActionKind::Down,
                    scheduled_us: at_us,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "down".into(),
                },
                KeyActionInput {
                    source_action_index: index * 2 + 1,
                    kind: ActionKind::Up,
                    scheduled_us: at_us + 1,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "up".into(),
                },
            ]
        })
        .collect::<Vec<_>>();
    let large_schedule = compile_actions(&many_actions, &allowed);
    let mut checkpoints = 0;
    let error = PreparedDispatchSchedule::prepare_with_cancellation(
        &large_schedule,
        std::num::NonZeroU64::new(1_000_000).expect("QPC frequency"),
        0,
        0,
        || {
            checkpoints += 1;
            if checkpoints == 3 {
                Err("test cancellation".into())
            } else {
                Ok(())
            }
        },
    )
    .expect_err("validation checks cancellation while walking large schedules");
    assert!(error.contains("cancelled"));
    assert_eq!(checkpoints, 3);
}

#[test]
fn paired_up_qpc_normalization_adds_rounded_down_and_hold_ticks() {
    let schedule = compile_actions(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "Down at 1ms".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 18_167,
                scan_codes: smallvec::smallvec![0x15],
                reason: "authored Up at Down + H".into(),
            },
        ],
        &[0x15],
    );
    let prepared = PreparedDispatchSchedule::prepare(
        &schedule,
        NonZeroU64::new(3_579_545).expect("QPC frequency"),
        17_167,
        0,
    )
    .expect("authored H boundary is normalized into QPC ticks");
    assert_eq!(prepared.down_intents[0].paired_up_offset_us, 18_167);
    assert_eq!(
        prepared.down_intents[0].authored_up_offset_ticks.as_u64(),
        65_030
    );
    assert_eq!(
        prepared.down_intents[0]
            .paired_up_not_before_offset_ticks
            .as_u64(),
        65_031
    );

    let down_due = prepared.down_chords[0].scheduled_ticks;
    let mut executor = DispatchExecutor::from_prepared_schedule(prepared);
    let down = executor.evaluate(down_due);
    let DispatchAction::Send {
        up_mask: 0,
        down_chord: Some(chord),
    } = down
    else {
        panic!("initial Down is due");
    };
    executor
        .on_send_completed(0, chord.down_mask, down_due)
        .expect("commit Down");
    assert_eq!(
        executor.evaluate(TimelineTicks::from_raw(65_030)),
        DispatchAction::Wait {
            deadline_ticks: TimelineTicks::from_raw(65_031)
        }
    );
    assert!(matches!(
        executor.evaluate(TimelineTicks::from_raw(65_031)),
        DispatchAction::Send { up_mask: 1, .. }
    ));
}

#[test]
fn prepared_plan_retains_unmatched_up_metadata_without_a_down_stream() {
    let schedule = compile_actions(
        &[KeyActionInput {
            source_action_index: 7,
            kind: ActionKind::Up,
            scheduled_us: 10,
            scan_codes: smallvec::smallvec![0x15],
            reason: "unmatched up".into(),
        }],
        &[0x15],
    );
    let frequency = std::num::NonZeroU64::new(1_000_000).expect("QPC frequency");
    let prepared = PreparedDispatchSchedule::prepare(&schedule, frequency, 0, 0)
        .expect("unmatched Up is admitted as diagnostics metadata");
    assert!(!prepared.has_down());
    assert_eq!(prepared.authored_timestamp_group_count(), 1);
    assert_eq!(prepared.generation_count(), 0);
    assert_eq!(prepared.unmatched_up_metadata().len(), 1);
    assert_eq!(prepared.unmatched_up_metadata()[0].source_action_index, 7);
    assert_eq!(
        prepared.unmatched_up_metadata()[0].scheduled_ticks,
        TimelineTicks::from_raw(10)
    );
}

#[test]
fn fixture_1_independent_hold_floor_does_not_block_b_down() {
    // A-Up authored 20 ms, hold floor 25 ms; B-Down independent at 21 ms.
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
    let mut executor = DispatchExecutor::new(&sched, min_hold, test_ticks).unwrap();

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

    // Slot 0 (A) is now active. Its authored Up is 20_000, but its hold floor is 25_000.
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
fn fixture_2_same_key_down_waits_only_for_up_commit_and_authored_due_time() {
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
    let mut executor = DispatchExecutor::new(&sched, min_hold, test_ticks).unwrap();

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
    // Up 1 commits at 20_000. The next authored Down is already due, so a
    // fresh scheduler evaluation can admit it without a post-Up floor.
    executor
        .on_send_completed(0x01, 0, TimelineTicks::from_raw(20_000))
        .unwrap();

    // Second Down is authored at 28_000.
    let step28 = executor.evaluate(TimelineTicks::from_raw(28_000));
    assert!(
        matches!(
            step28,
            DispatchAction::Send {
                up_mask: 0,
                down_chord: Some(_)
            }
        ),
        "same-key Down is eligible at its authored onset after Up commit"
    );
}

#[test]
fn fixture_3_same_key_chord_reuses_slot_after_up_commit_without_gap_floor() {
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
        // C up at 10 ms
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
    let mut executor = DispatchExecutor::new(&sched, min_hold, test_ticks).unwrap();

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
    // C's Up is committed. A later Down can reuse the slot as soon as its
    // authored onset is due.
    executor
        .on_send_completed(0x04, 0, TimelineTicks::from_raw(10_000))
        .unwrap();

    // At 21 ms: Chord {B, C} is scheduled.
    let step21 = executor.evaluate(TimelineTicks::from_raw(21_000));
    assert!(matches!(
        step21,
        DispatchAction::Send {
            up_mask: 0,
            down_chord: Some(chord)
        } if chord.down_mask == 0x06
    ));
    executor
        .on_send_completed(0, 0x06, TimelineTicks::from_raw(21_000))
        .unwrap();

    // Up A remains independent and proceeds at its own authored deadline.
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
    let mut executor = DispatchExecutor::new(&sched, min_hold, test_ticks).unwrap();

    // 0 ms: Down A
    executor
        .on_send_completed(0, 0x01, TimelineTicks::from_raw(0))
        .unwrap();
    // A hold floor is 0 + 10 ms = 10 ms. Authored Up is 20 ms.
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
fn fixture_hol_stalled_down_does_not_block_independent_slot() {
    // Key A authored at 10 ms, Up at 20 ms.
    // Key B authored at 16 ms, Up at 30 ms.
    // Worker stall of 5,000 us delays Key A Down to 15 ms!
    let allowed = vec![0x15, 0x16];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 10_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down A".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Down,
            scheduled_us: 16_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "down B".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: 20_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up A".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 30_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "up B".into(),
        },
    ];

    let sched = compile_actions(&actions, &allowed);
    let min_hold = DurationTicks::from_raw(10_000); // H = 10 ms
    let mut executor = DispatchExecutor::new(&sched, min_hold, test_ticks).unwrap();

    // 10 ms: Key A is dispatched
    let step10 = executor.evaluate(TimelineTicks::from_raw(10_000));
    assert!(matches!(
        step10,
        DispatchAction::Send {
            up_mask: 0,
            down_chord: Some(chord),
        } if chord.down_mask == 0x01
    ));

    // A 5,000 us worker stall delays Down completion to 15,000 us
    executor
        .on_send_completed(0, 0x01, TimelineTicks::from_raw(15_000))
        .unwrap();

    // Key A's Up is now due at max(20 ms, 15 ms + 10 ms) = 25 ms.
    // At 16 ms: Key B's authored Down is due.
    // Key A's hold floor is 25 ms, but Key B uses an independent slot and is due at 16 ms.
    let step16 = executor.evaluate(TimelineTicks::from_raw(16_000));
    assert!(
        matches!(
            step16,
            DispatchAction::Send {
                up_mask: 0,
                down_chord: Some(chord),
            } if chord.down_mask == 0x02
        ),
        "Key B must be dispatched at 16 ms without HOL blocking behind Key A's hold floor"
    );

    // Key B completes on time at 16 ms
    executor
        .on_send_completed(0, 0x02, TimelineTicks::from_raw(16_000))
        .unwrap();

    // At 25 ms: Key A's release obligation is due and emitted
    let step25 = executor.evaluate(TimelineTicks::from_raw(25_000));
    assert_eq!(
        step25,
        DispatchAction::Send {
            up_mask: 0x01,
            down_chord: None,
        }
    );
    executor
        .on_send_completed(0x01, 0, TimelineTicks::from_raw(25_000))
        .unwrap();

    // At 30 ms: Key B's release obligation is due and emitted
    let step30 = executor.evaluate(TimelineTicks::from_raw(30_000));
    assert_eq!(
        step30,
        DispatchAction::Send {
            up_mask: 0x02,
            down_chord: None,
        }
    );
    executor
        .on_send_completed(0x02, 0, TimelineTicks::from_raw(30_000))
        .unwrap();

    // Episode ends cleanly
    let end = executor.evaluate(TimelineTicks::from_raw(30_001));
    assert_eq!(end, DispatchAction::Finished);
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
    let mut executor = DispatchExecutor::new(&sched, min_hold, test_ticks).unwrap();

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
    let mut executor = DispatchExecutor::new(&sched, min_hold, test_ticks).unwrap();

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
fn fixture_verified_cleanup_cancels_active_slots_without_replay_or_added_delay() {
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
    let mut executor = DispatchExecutor::new(&sched, min_hold, test_ticks).unwrap();

    // Down first chord at 10 ms
    executor
        .on_send_completed(0, 0x03, TimelineTicks::from_raw(10_000))
        .unwrap();
    assert_eq!(executor.active_mask(), 0x03);

    // During sustain, focus is lost and verified cleanup cancels the active generation.
    let cancelled = executor.on_verified_cleanup().unwrap();
    assert_eq!(cancelled, 2);
    assert_eq!(executor.active_mask(), 0);
    assert_eq!(executor.accounting().cancelled, 2);
    // At 30 ms (the original authored Up time), no physical Up is replayed.
    // The next Down remains on its authored 35 ms target.
    let step30 = executor.evaluate(TimelineTicks::from_raw(30_000));
    assert_eq!(
        step30,
        DispatchAction::Wait {
            deadline_ticks: TimelineTicks::from_raw(35_000)
        },
        "No stale Up replay or added wait; the next authored Down is the next physical deadline"
    );

    // The authored second Down at 35 ms remains held until the exact barrier.
    let step35 = executor.evaluate(TimelineTicks::from_raw(35_000));
    assert!(
        matches!(
            step35,
            DispatchAction::Send {
                up_mask: 0,
                down_chord: Some(chord)
            } if chord.down_mask == 0x01
        ),
        "Second Down dispatched at its authored target"
    );
}

#[test]
fn fixture_forced_conversion_failure_fails_construction() {
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
    let min_hold = DurationTicks::from_raw(10_000);
    let err = DispatchExecutor::new(&sched, min_hold, |us| {
        if us == 10_000 {
            Err("simulated QPC clock overflow")
        } else {
            Ok(TimelineTicks::from_raw(us))
        }
    })
    .expect_err("conversion failure must fail executor construction");
    assert!(err.contains("simulated QPC clock overflow"));
}

#[test]
fn fixture_end_state_waits_for_schedule_end_offset_even_with_zero_active_keys() {
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
    let mut sched = compile_actions(&actions, &allowed);
    // Explicitly extend schedule end offset to 60 ms (metadata-only tail)
    sched.schedule_end_offset_us = 60_000;

    let min_hold = DurationTicks::from_raw(10_000);
    let mut executor = DispatchExecutor::new(&sched, min_hold, test_ticks).unwrap();

    // 10 ms: Down
    let step = executor.evaluate(TimelineTicks::from_raw(10_000));
    assert!(matches!(
        step,
        DispatchAction::Send {
            down_chord: Some(_),
            ..
        }
    ));
    executor
        .on_send_completed(0, 0b1, TimelineTicks::from_raw(10_000))
        .unwrap();

    // 20 ms: Up
    let step = executor.evaluate(TimelineTicks::from_raw(20_000));
    assert!(matches!(
        step,
        DispatchAction::Send {
            up_mask: 0b1,
            down_chord: None
        }
    ));
    executor
        .on_send_completed(0b1, 0, TimelineTicks::from_raw(20_000))
        .unwrap();

    // At 25 ms: all chords dispatched, all slots empty, BUT schedule_end_offset (60 ms) not reached!
    assert_eq!(executor.active_mask(), 0);
    assert!(
        !executor.is_finished(TimelineTicks::from_raw(25_000)),
        "executor must NOT be finished before schedule_end_offset"
    );
    let step = executor.evaluate(TimelineTicks::from_raw(25_000));
    assert_eq!(
        step,
        DispatchAction::Wait {
            deadline_ticks: TimelineTicks::from_raw(60_000),
        },
        "executor must wait for metadata-only schedule end offset"
    );

    // At 60 ms: reached schedule_end_offset
    assert!(
        executor.is_finished(TimelineTicks::from_raw(60_000)),
        "executor must be finished once schedule_end_offset is reached"
    );
    let step = executor.evaluate(TimelineTicks::from_raw(60_000));
    assert_eq!(step, DispatchAction::Finished);
}
