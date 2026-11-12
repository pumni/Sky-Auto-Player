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
