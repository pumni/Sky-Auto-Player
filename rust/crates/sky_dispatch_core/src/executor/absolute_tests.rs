use super::*;
use crate::compile::compile_runtime_intents;
use crate::model::{ActionKind, KeyActionInput, RuntimeSchedule};
use crate::time::{DurationTicks, TimelineTicks};

fn compile_actions(actions: &[KeyActionInput], allowed: &[u16]) -> RuntimeSchedule {
    compile_runtime_intents(actions, allowed).expect("valid scheduler fixture")
}

fn test_ticks(us: u64) -> Result<TimelineTicks, std::convert::Infallible> {
    Ok(TimelineTicks::from_raw(us))
}
#[test]
fn absolute_scheduler_keeps_an_independent_down_eligible_before_an_up_hold_floor() {
    let allowed = vec![0x15, 0x16];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "A down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 10,
            scan_codes: smallvec::smallvec![0x15],
            reason: "A up".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 12,
            scan_codes: smallvec::smallvec![0x16],
            reason: "B down".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 30,
            scan_codes: smallvec::smallvec![0x16],
            reason: "B up".into(),
        },
    ];
    let schedule = compile_actions(&actions, &allowed);
    let mut executor = DispatchExecutor::new(
        &schedule,
        DurationTicks::from_raw(16),
        DurationTicks::from_raw(1),
        test_ticks,
    )
    .expect("absolute scheduler");
    let epoch = QpcTicks::from_raw(1_000);

    let DispatchDecision::Send { down_chord, .. } = executor
        .evaluate_at(epoch, epoch)
        .expect("first Down decision")
    else {
        panic!("first Down should be ready at the epoch");
    };
    assert_eq!(down_chord.expect("A chord").down_mask, 1);
    executor
        .on_send_completed_at(0, 1, QpcTicks::from_raw(1_006))
        .expect("commit A Down at 6 ms");

    assert_eq!(
        executor
            .evaluate_at(QpcTicks::from_raw(1_010), epoch)
            .expect("next boundary"),
        DispatchDecision::Wait {
            deadline_qpc: QpcTicks::from_raw(1_012)
        }
    );
    let DispatchDecision::Send {
        up_mask,
        down_chord,
        timing_window,
    } = executor
        .evaluate_at(QpcTicks::from_raw(1_012), epoch)
        .expect("independent B Down")
    else {
        panic!("B Down must not wait for A's hold floor");
    };
    assert_eq!(up_mask, 0);
    assert_eq!(down_chord.expect("B chord").down_mask, 2);
    assert_eq!(timing_window.authored_target_qpc, QpcTicks::from_raw(1_012));
    executor
        .on_send_completed_at(0, 2, QpcTicks::from_raw(1_013))
        .expect("commit B Down");

    let DispatchDecision::Send { up_mask, .. } = executor
        .evaluate_at(QpcTicks::from_raw(1_022), epoch)
        .expect("A release floor")
    else {
        panic!("A Up should reach its completion-relative H floor");
    };
    assert_eq!(up_mask, 1);
}

#[test]
fn absolute_scheduler_coalesces_a_full_fifteen_input_packet() {
    let keys: Vec<u16> = (0x15..=0x23).collect();
    let first_seven = keys[..7].to_vec();
    let last_eight = keys[7..].to_vec();
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: first_seven.clone().into(),
            reason: "first chord".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 10,
            scan_codes: first_seven.into(),
            reason: "first release".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 10,
            scan_codes: last_eight.clone().into(),
            reason: "second chord".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 20,
            scan_codes: last_eight.into(),
            reason: "second release".into(),
        },
    ];
    let schedule = compile_actions(&actions, &keys);
    let mut executor = DispatchExecutor::new(
        &schedule,
        DurationTicks::ZERO,
        DurationTicks::from_raw(1),
        test_ticks,
    )
    .expect("absolute scheduler");
    let epoch = QpcTicks::from_raw(5_000);
    let DispatchDecision::Send { down_chord, .. } =
        executor.evaluate_at(epoch, epoch).expect("first chord")
    else {
        panic!("first chord should be ready");
    };
    let first_mask = down_chord.expect("first chord").down_mask;
    executor
        .on_send_completed_at(0, first_mask, epoch)
        .expect("commit first chord");

    let DispatchDecision::Send {
        up_mask,
        down_chord,
        ..
    } = executor
        .evaluate_at(QpcTicks::from_raw(5_010), epoch)
        .expect("mixed full packet")
    else {
        panic!("same-time independent Up and Down should coalesce");
    };
    let down_mask = down_chord.expect("second chord").down_mask;
    assert_eq!(up_mask.count_ones(), 7);
    assert_eq!(down_mask.count_ones(), 8);
    assert_eq!(up_mask & down_mask, 0);
    assert_eq!(up_mask.count_ones() + down_mask.count_ones(), 15);
    executor
        .on_send_completed_at(up_mask, down_mask, QpcTicks::from_raw(5_010))
        .expect("commit full packet");
}

#[test]
fn absolute_scheduler_uses_up_completion_for_repress_floor_and_resume_barrier() {
    let allowed = vec![0x15, 0x16];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first A down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 10,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first A up".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 12,
            scan_codes: smallvec::smallvec![0x15],
            reason: "second A down".into(),
        },
        KeyActionInput {
            source_action_index: 4,
            kind: ActionKind::Down,
            scheduled_us: 13,
            scan_codes: smallvec::smallvec![0x16],
            reason: "B down".into(),
        },
        KeyActionInput {
            source_action_index: 5,
            kind: ActionKind::Up,
            scheduled_us: 30,
            scan_codes: smallvec::smallvec![0x15],
            reason: "second A up".into(),
        },
        KeyActionInput {
            source_action_index: 6,
            kind: ActionKind::Up,
            scheduled_us: 40,
            scan_codes: smallvec::smallvec![0x16],
            reason: "B up".into(),
        },
    ];
    let schedule = compile_actions(&actions, &allowed);
    let mut executor = DispatchExecutor::new(
        &schedule,
        DurationTicks::ZERO,
        DurationTicks::from_raw(5),
        test_ticks,
    )
    .expect("absolute scheduler");
    let epoch = QpcTicks::from_raw(1_000);
    let DispatchDecision::Send { down_chord, .. } =
        executor.evaluate_at(epoch, epoch).expect("first Down")
    else {
        panic!("first Down should be ready");
    };
    let first_down = down_chord.expect("first chord").down_mask;
    assert_eq!(first_down, 1);
    executor
        .on_send_completed_at(0, first_down, QpcTicks::from_raw(1_006))
        .expect("first Down completion");

    let DispatchDecision::Send { up_mask, .. } = executor
        .evaluate_at(QpcTicks::from_raw(1_010), epoch)
        .expect("first Up")
    else {
        panic!("first Up should be due");
    };
    assert_eq!(up_mask, 1);
    executor
        .on_send_completed_at(up_mask, 0, QpcTicks::from_raw(1_025))
        .expect("slow Up completion");
    assert_eq!(
        executor
            .evaluate_at(QpcTicks::from_raw(1_029), epoch)
            .expect("repress floor"),
        DispatchDecision::Wait {
            deadline_qpc: QpcTicks::from_raw(1_030)
        }
    );
    let DispatchDecision::Send { down_chord, .. } = executor
        .evaluate_at(QpcTicks::from_raw(1_030), epoch)
        .expect("repress after slow Up")
    else {
        panic!("same key Down should follow Up completion plus G");
    };
    assert_eq!(down_chord.expect("second A chord").down_mask, 1);

    // A separate owner verifies the absolute post-cleanup barrier, including
    // the exact barrier QPC where the next chord becomes eligible.
    let mut resumed = DispatchExecutor::new(
        &schedule,
        DurationTicks::ZERO,
        DurationTicks::from_raw(5),
        test_ticks,
    )
    .expect("resume scheduler");
    let DispatchDecision::Send { down_chord, .. } =
        resumed.evaluate_at(epoch, epoch).expect("resume seed Down")
    else {
        panic!("resume seed Down should be ready");
    };
    resumed
        .on_send_completed_at(0, down_chord.expect("seed chord").down_mask, epoch)
        .expect("resume seed commit");
    resumed
        .on_verified_cleanup_at(QpcTicks::from_raw(2_000))
        .expect("verified cleanup");
    assert_eq!(
        resumed
            .evaluate_at(QpcTicks::from_raw(2_004), epoch)
            .expect("before resume barrier"),
        DispatchDecision::Wait {
            deadline_qpc: QpcTicks::from_raw(2_005)
        }
    );
    assert!(matches!(
        resumed
            .evaluate_at(QpcTicks::from_raw(2_005), epoch)
            .expect("resume barrier boundary"),
        DispatchDecision::Send { .. }
    ));
}

#[test]
fn absolute_scheduler_checked_overflow_does_not_mutate_slot_or_cursor() {
    let allowed = vec![0x15];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "overflow Down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 10,
            scan_codes: smallvec::smallvec![0x15],
            reason: "overflow Up".into(),
        },
    ];
    let schedule = compile_actions(&actions, &allowed);
    let mut executor = DispatchExecutor::new(
        &schedule,
        DurationTicks::from_raw(10),
        DurationTicks::from_raw(1),
        test_ticks,
    )
    .expect("absolute scheduler");
    let result = executor.on_send_completed_at(0, 1, QpcTicks::from_raw(u64::MAX - 5));
    assert!(result.is_err());
    assert_eq!(executor.active_mask(), 0);
    assert_eq!(executor.chord_cursor(), 0);
    assert_eq!(executor.accounting().activated, 0);
}

#[test]
fn absolute_scheduler_hol_compression_preserves_all_down_intents() {
    const HOLD_US: u64 = 17_167;
    const GAP_US: u64 = 16_667;
    const A_UP_US: u64 = 20_167;
    const AB_DOWN_US: u64 = 34_334;
    const C_DOWN_US: u64 = 35_000;
    const FINAL_UP_US: u64 = 60_000;

    let allowed = [0x15, 0x16, 0x17];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "A down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: A_UP_US,
            scan_codes: smallvec::smallvec![0x15],
            reason: "A up".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: AB_DOWN_US,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "A+B down".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Down,
            scheduled_us: C_DOWN_US,
            scan_codes: smallvec::smallvec![0x17],
            reason: "C down".into(),
        },
        KeyActionInput {
            source_action_index: 4,
            kind: ActionKind::Up,
            scheduled_us: FINAL_UP_US,
            scan_codes: smallvec::smallvec![0x15, 0x16, 0x17],
            reason: "final releases".into(),
        },
    ];
    let schedule = compile_actions(&actions, &allowed);

    assert_eq!(schedule.down_chords.len(), 3);
    assert_eq!(
        schedule
            .down_chords
            .iter()
            .map(|chord| chord.intents.len())
            .sum::<usize>(),
        4,
        "A, A+B, C contain four physical Down intents"
    );
    assert_eq!(schedule.generation_count, 4);
    assert_eq!(
        schedule
            .down_chords
            .iter()
            .map(|chord| chord.source_action_index)
            .collect::<Vec<_>>(),
        [0, 2, 3]
    );

    let run = |delayed_sender: bool| {
        let mut executor = DispatchExecutor::new(
            &schedule,
            DurationTicks::from_raw(HOLD_US),
            DurationTicks::from_raw(GAP_US),
            test_ticks,
        )
        .expect("absolute scheduler");
        let epoch = QpcTicks::ZERO;

        let (a_mask, a_intents) = match executor
            .evaluate_at(QpcTicks::ZERO, epoch)
            .expect("A Down at authored onset")
        {
            DispatchDecision::Send {
                up_mask: 0,
                down_chord: Some(chord),
                ..
            } => (chord.down_mask, chord.intents.len()),
            decision => panic!("expected first authored chord A, got {decision:?}"),
        };
        assert_eq!((a_mask, a_intents), (0b001, 1));
        let a_down_completion = if delayed_sender { 3_000 } else { 0 };
        executor
            .on_send_completed_at(0, a_mask, QpcTicks::from_raw(a_down_completion))
            .expect("commit A generation once");
        assert_eq!(executor.chord_cursor(), 1);
        assert_eq!(executor.accounting().activated, 1);

        let (a_up_mask, no_down) = match executor
            .evaluate_at(QpcTicks::from_raw(A_UP_US), epoch)
            .expect("A Up authored at 20,167 us")
        {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                ..
            } => (up_mask, down_chord.is_none()),
            decision => panic!("expected A Up at 20,167 us, got {decision:?}"),
        };
        assert_eq!(a_up_mask, 0b001);
        assert!(no_down);
        assert!(A_UP_US >= a_down_completion + HOLD_US);
        let a_up_completion = if delayed_sender { 20_267 } else { A_UP_US };
        executor
            .on_send_completed_at(a_up_mask, 0, QpcTicks::from_raw(a_up_completion))
            .expect("commit A release once");
        assert_eq!(
            executor.chord_cursor(),
            1,
            "Up does not advance Down cursor"
        );
        assert_eq!(executor.accounting().released, 1);

        let ab_eligible = if delayed_sender { 36_934 } else { 36_834 };
        assert_eq!(A_UP_US + GAP_US, 36_834);
        assert_eq!(a_up_completion + GAP_US, ab_eligible);
        assert_eq!(
            executor
                .evaluate_at(QpcTicks::from_raw(AB_DOWN_US), epoch)
                .expect("A+B must wait for authored onset and G floor"),
            DispatchDecision::Wait {
                deadline_qpc: QpcTicks::from_raw(ab_eligible)
            }
        );
        assert_eq!(
            executor
                .evaluate_at(QpcTicks::from_raw(ab_eligible - 1), epoch)
                .expect("A+B remains blocked before G"),
            DispatchDecision::Wait {
                deadline_qpc: QpcTicks::from_raw(ab_eligible)
            }
        );

        let (ab_mask, ab_intents) = match executor
            .evaluate_at(QpcTicks::from_raw(ab_eligible), epoch)
            .expect("A+B at its eligible boundary")
        {
            DispatchDecision::Send {
                up_mask: 0,
                down_chord: Some(chord),
                ..
            } => (chord.down_mask, chord.intents.len()),
            decision => panic!("expected second authored chord A+B, got {decision:?}"),
        };
        assert_eq!((ab_mask, ab_intents), (0b011, 2));
        assert_eq!(executor.chord_cursor(), 1);
        let ab_completion = if delayed_sender { 37_034 } else { ab_eligible };
        executor
            .on_send_completed_at(0, ab_mask, QpcTicks::from_raw(ab_completion))
            .expect("commit A+B generations once");
        assert_eq!(executor.chord_cursor(), 2);
        assert_eq!(executor.accounting().activated, 3);

        let (c_mask, c_intents) = match executor
            .evaluate_at(QpcTicks::from_raw(ab_completion), epoch)
            .expect("independent C follows the cursor immediately")
        {
            DispatchDecision::Send {
                up_mask: 0,
                down_chord: Some(chord),
                ..
            } => (chord.down_mask, chord.intents.len()),
            decision => panic!("expected third authored chord C, got {decision:?}"),
        };
        assert_eq!((c_mask, c_intents), (0b100, 1));
        assert_eq!(executor.chord_cursor(), 2);
        let c_start = ab_completion;
        let c_completion = c_start + 100;
        executor
            .on_send_completed_at(0, c_mask, QpcTicks::from_raw(c_completion))
            .expect("commit C generation once");
        assert_eq!(executor.chord_cursor(), 3);
        assert_eq!(executor.accounting().activated, 4);

        let (release_mask, no_down) = match executor
            .evaluate_at(QpcTicks::from_raw(FINAL_UP_US), epoch)
            .expect("all paired releases at final onset")
        {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                ..
            } => (up_mask, down_chord.is_none()),
            decision => panic!("expected final releases, got {decision:?}"),
        };
        assert_eq!(release_mask, 0b111);
        assert!(no_down);
        let release_completion = if delayed_sender {
            FINAL_UP_US + 100
        } else {
            FINAL_UP_US
        };
        executor
            .on_send_completed_at(release_mask, 0, QpcTicks::from_raw(release_completion))
            .expect("resolve all release obligations");

        let accounting = executor.accounting();
        assert_eq!(accounting.total, 4);
        assert_eq!(accounting.activated, 4);
        assert_eq!(accounting.released, 4);
        assert_eq!(accounting.active, 0);
        assert_eq!(accounting.scheduled, 0);
        assert_eq!(executor.active_mask(), 0);
        assert_eq!(executor.remaining_chords(), 0);
        assert!(
            executor
                .is_finished_at(QpcTicks::from_raw(release_completion), epoch)
                .expect("finish timestamp")
        );
        executor.check_invariants().expect("valid final lifecycle");

        (ab_eligible, ab_completion, c_start, c_completion)
    };

    let control = run(false);
    let delayed = run(true);
    assert_eq!(control, (36_834, 36_834, 36_834, 36_934));
    assert_eq!(delayed, (36_934, 37_034, 37_034, 37_134));
    assert_eq!(delayed.3 - delayed.1, 100);
    assert_eq!(
        (C_DOWN_US - AB_DOWN_US) - (delayed.2 - delayed.0),
        566,
        "delayed completion compresses the authored 666 us gap to 100 us"
    );
}
