use super::*;
use crate::model::OnsetChordInput;
use crate::plan::build_onset_plan;
use crate::time::{DurationTicks, QpcTicks, TimelineTicks};

fn make_executor(chords: &[(u32, u64, &[u16])], allowed: &[u16], hold_us: u64) -> DispatchExecutor {
    let inputs: Vec<OnsetChordInput> = chords
        .iter()
        .map(|&(idx, us, scans)| OnsetChordInput {
            source_action_index: idx,
            scheduled_us: us,
            scan_codes: scans.iter().copied().collect(),
        })
        .collect();
    let plan = build_onset_plan(&inputs, allowed, hold_us).expect("plan");
    let qpc_1mhz = std::num::NonZeroU64::new(1_000_000).expect("1 MHz");
    let prepared = PreparedDispatchSchedule::prepare(&plan, qpc_1mhz, 0).expect("prepared");
    DispatchExecutor::from_prepared_schedule(prepared)
}

#[test]
fn empty_plan_finishes_immediately_with_zero_work() {
    let exec = make_executor(&[], &[0x15], 16_667);
    assert!(!exec.has_physical_work());
    assert_eq!(exec.evaluate(TimelineTicks::ZERO), DispatchAction::Finished);
    assert!(exec.is_finished(TimelineTicks::ZERO));
    exec.check_invariants().unwrap();
}

#[test]
fn fifteen_key_chord_activates_and_releases_all_slots() {
    let allowed: Vec<u16> = (0..MAX_KEYS).map(|i| 0x15 + i as u16).collect();
    let mut exec = make_executor(&[(0, 500, &allowed)], &allowed, 16_667);
    assert_eq!(
        exec.evaluate(TimelineTicks::from_raw(0)),
        DispatchAction::Wait {
            deadline_ticks: TimelineTicks::from_raw(500),
        }
    );
    let DispatchAction::Send {
        up_mask: 0,
        down_chord: Some(chord),
    } = exec.evaluate(TimelineTicks::from_raw(500))
    else {
        panic!("expected 15-key Down");
    };
    assert_eq!(chord.key_mask, ALL_KEYS_MASK);
    exec.on_send_completed(0, ALL_KEYS_MASK, TimelineTicks::from_raw(520))
        .unwrap();
    assert_eq!(exec.active_mask(), ALL_KEYS_MASK);
    assert_eq!(exec.accounting().active, 15);

    let release_at = TimelineTicks::from_raw(520 + 16_667);
    assert_eq!(
        exec.evaluate(TimelineTicks::from_raw(520 + 16_666)),
        DispatchAction::Wait {
            deadline_ticks: release_at,
        }
    );
    assert_eq!(
        exec.evaluate(release_at),
        DispatchAction::Send {
            up_mask: ALL_KEYS_MASK,
            down_chord: None,
        }
    );
    exec.on_send_completed(ALL_KEYS_MASK, 0, release_at)
        .unwrap();
    assert_eq!(exec.evaluate(release_at), DispatchAction::Finished);
    assert_eq!(exec.accounting().released, 15);
    exec.check_invariants().unwrap();
}

#[test]
fn verified_cleanup_and_attempted_down_consumption_preserve_accounting() {
    let mut exec = make_executor(
        &[(0, 0, &[0x15]), (1, 20_000, &[0x16]), (2, 40_000, &[0x15])],
        &[0x15, 0x16],
        16_667,
    );
    exec.on_send_completed(0, 1, TimelineTicks::ZERO).unwrap();
    assert_eq!(exec.accounting().active, 1);

    let cancelled = exec.on_verified_cleanup().unwrap();
    assert_eq!(cancelled, 1);
    assert_eq!(exec.active_mask(), 0);
    assert_eq!(exec.accounting().cancelled, 1);

    exec.consume_attempted_down_after_verified_cleanup(1, 2)
        .unwrap();
    assert_eq!(exec.chord_cursor(), 2);
    assert_eq!(exec.accounting().cancelled, 2);
    exec.check_invariants().unwrap();

    let terminal = exec
        .cancel_remaining_generations_for_terminal_shutdown()
        .unwrap();
    assert_eq!(terminal, 1);
    assert_eq!(exec.accounting().cancelled, 3);
    assert_eq!(exec.accounting().scheduled, 0);
    exec.check_invariants().unwrap();
}

#[test]
fn proven_raw_zero_down_consumes_cursor_without_disturbing_active_slots() {
    let mut exec = make_executor(
        &[(0, 0, &[0x15]), (1, 5_000, &[0x16])],
        &[0x15, 0x16],
        16_667,
    );
    exec.on_send_completed(0, 1, TimelineTicks::from_raw(10))
        .unwrap();
    exec.consume_attempted_down_after_proven_raw_zero(1, 2)
        .unwrap();
    assert_eq!(exec.chord_cursor(), 2);
    assert_eq!(exec.active_mask(), 1);
    assert_eq!(
        exec.slot_release_not_before_qpc(0),
        Some(QpcTicks::from_raw(16_677))
    );
    assert_eq!(exec.accounting().cancelled, 1);
    assert_eq!(exec.accounting().active, 1);
    exec.check_invariants().unwrap();
}

#[test]
fn independent_up_is_never_delayed_to_batch_with_future_down() {
    let mut exec = make_executor(
        &[(0, 0, &[0x15]), (1, 17_000, &[0x16])],
        &[0x15, 0x16],
        16_667,
    );
    exec.on_send_completed(0, 1, TimelineTicks::ZERO).unwrap();
    // At 16_667, key 0's Up is due while key 1's Down is not due until 17_000.
    // Must emit Up immediately (D5: never delay an eligible Up to batch with future Down).
    assert_eq!(
        exec.evaluate(TimelineTicks::from_raw(16_667)),
        DispatchAction::Send {
            up_mask: 1,
            down_chord: None,
        }
    );
}

#[test]
fn diagnostic_nominal_up_preserves_exact_ceil_rounding_at_nonintegral_qpc_and_late_completion() {
    // At f = 3_580_000 Hz, onset_us = 2, hold_us = 2:
    //   ceil(2 * 3.58) = 8, ceil(2 * 3.58) = 8 -> physical lower bound 8 + 8 = 16
    //   exact diagnostic nominal ceil((2 + 2) * 3.58) = ceil(14.32) = 15 (differs by 1 tick).
    let freq = NonZeroU64::new(3_580_000).expect("3.58 MHz");
    let plan = build_onset_plan(
        &[OnsetChordInput {
            source_action_index: 7,
            scheduled_us: 2,
            scan_codes: smallvec::smallvec![0x15],
        }],
        &[0x15],
        2,
    )
    .expect("plan");
    let prepared = PreparedDispatchSchedule::prepare(&plan, freq, 0).expect("prepared");
    assert_eq!(prepared.hold_ticks(), DurationTicks::from_raw(8));
    let mut exec = DispatchExecutor::from_prepared_schedule(prepared);
    let epoch = QpcTicks::from_raw(1_000);

    // 1. Timely completion right at authored onset (epoch + 8):
    //    physical release_not_before = epoch + 8 + 8 = epoch + 16
    //    diagnostic nominal_up_offset_ticks = ceil((2 + 2) * 3.58) = 15.
    exec.on_send_completed_at(0, 1, QpcTicks::from_raw(1_008))
        .expect("timely Down commit");
    let timings = exec.up_slot_timings(1);
    assert_eq!(timings.len(), 1);
    assert_eq!(
        timings[0].nominal_up_offset_ticks,
        TimelineTicks::from_raw(15)
    );
    assert_eq!(timings[0].release_not_before_qpc, QpcTicks::from_raw(1_016));
    let DispatchDecision::Send {
        up_mask: 1,
        up_summary: Some(summary),
        ..
    } = exec
        .evaluate_at(QpcTicks::from_raw(1_016), epoch)
        .expect("evaluate summary")
    else {
        panic!("expected due Up send with summary");
    };
    assert_eq!(summary.authored_target_qpc, QpcTicks::from_raw(1_015));
    assert_eq!(summary.musical_up_not_before_qpc, QpcTicks::from_raw(1_016));
    assert_eq!(summary.hold_floor_mask, 1);

    // 2. Late completion at epoch + 100:
    //    physical release_not_before = epoch + 100 + 8 = epoch + 108
    //    diagnostic nominal_up_offset_ticks remains exact ceil((2 + 2) * 3.58) = 15.
    let prepared_late = PreparedDispatchSchedule::prepare(&plan, freq, 0).expect("prepared");
    let mut exec_late = DispatchExecutor::from_prepared_schedule(prepared_late);
    exec_late
        .on_send_completed_at(0, 1, QpcTicks::from_raw(1_100))
        .expect("late Down commit");
    let late_timings = exec_late.up_slot_timings(1);
    assert_eq!(
        late_timings[0].nominal_up_offset_ticks,
        TimelineTicks::from_raw(15)
    );
    assert_eq!(
        late_timings[0].release_not_before_qpc,
        QpcTicks::from_raw(1_108)
    );
}

#[test]
fn fifteen_slots_distinct_deadlines_release_in_order_without_starvation() {
    let allowed: Vec<u16> = (0..MAX_KEYS).map(|i| 0x15 + i as u16).collect();
    let chords: Vec<(u32, u64, &[u16])> = (0..MAX_KEYS)
        .map(|i| (i as u32, (i as u64) * 1_000, &allowed[i..=i]))
        .collect();
    let mut exec = make_executor(&chords, &allowed, 20_000);
    let epoch = QpcTicks::from_raw(100_000);

    // Activate all 15 slots at staggered completion times (each 1_000 ticks apart).
    for i in 0..MAX_KEYS {
        let now = QpcTicks::from_raw(100_000 + (i as u64) * 1_000);
        let DispatchDecision::Send {
            up_mask: 0,
            down_chord: Some(chord),
            ..
        } = exec.evaluate_at(now, epoch).expect("eval")
        else {
            panic!("expected Down for slot {i}");
        };
        assert_eq!(chord.key_mask, 1u16 << i);
        exec.on_send_completed_at(0, chord.key_mask, now)
            .expect("commit Down");
    }
    assert_eq!(exec.active_mask(), ALL_KEYS_MASK);

    // Release all 15 slots in strict deadline order without starvation.
    for i in 0..MAX_KEYS {
        let deadline = QpcTicks::from_raw(100_000 + (i as u64) * 1_000 + 20_000);
        assert_eq!(
            exec.evaluate_at(QpcTicks::from_raw(deadline.as_u64() - 1), epoch)
                .expect("wait"),
            DispatchDecision::Wait {
                deadline_qpc: deadline,
            }
        );
        let DispatchDecision::Send {
            up_mask,
            down_chord: None,
            ..
        } = exec.evaluate_at(deadline, epoch).expect("release")
        else {
            panic!("expected Up for slot {i}");
        };
        assert_eq!(up_mask, 1u16 << i);
        exec.on_send_completed_at(up_mask, 0, deadline)
            .expect("commit Up");
    }
    assert_eq!(exec.active_mask(), 0);
    exec.check_invariants().unwrap();
}

#[test]
fn attempted_down_consumption_rejects_mismatched_source_or_mask() {
    let mut exec = make_executor(&[(10, 0, &[0x15])], &[0x15, 0x16], 16_667);
    assert!(
        exec.consume_attempted_down_after_proven_raw_zero(99, 1)
            .is_err()
    );
    assert!(
        exec.consume_attempted_down_after_proven_raw_zero(10, 0b10)
            .is_err()
    );
    assert!(
        exec.consume_attempted_down_after_verified_cleanup(99, 1)
            .is_err()
    );
    assert!(
        exec.consume_attempted_down_after_verified_cleanup(10, 0b10)
            .is_err()
    );
    // State remains completely unmutated.
    assert_eq!(exec.chord_cursor(), 0);
    assert_eq!(exec.accounting().cancelled, 0);
    exec.check_invariants().unwrap();
}

#[test]
fn cleanup_overflow_preflights_without_mutation() {
    let mut exec = make_executor(&[(0, 0, &[0x15, 0x16])], &[0x15, 0x16], 16_667);
    exec.on_send_completed(0, 0b11, TimelineTicks::ZERO)
        .unwrap();
    // Force cancelled counter near u64::MAX to verify overflow preflight atomicity.
    exec.counters.cancelled = u64::MAX - 1;
    let err = exec
        .on_verified_cleanup()
        .expect_err("cancelled counter overflow must fail closed");
    assert!(err.contains("overflow"), "{err}");
    // Active mask, slots, and counters must remain untouched.
    assert_eq!(exec.active_mask(), 0b11);
    assert_eq!(exec.counters.cancelled, u64::MAX - 1);
}

#[test]
fn end_state_waits_for_trailing_schedule_end_offset_with_zero_active_keys() {
    let plan = build_onset_plan(&[], &[0x15], 16_667)
        .expect("empty plan")
        .with_trailing_end_offset_us(50_000)
        .expect("trailing end");
    let qpc_1mhz = NonZeroU64::new(1_000_000).expect("1 MHz");
    let prepared = PreparedDispatchSchedule::prepare(&plan, qpc_1mhz, 0).expect("prepared");
    let exec = DispatchExecutor::from_prepared_schedule(prepared);
    let epoch = QpcTicks::from_raw(10_000);
    assert_eq!(
        exec.evaluate_at(QpcTicks::from_raw(59_999), epoch).unwrap(),
        DispatchDecision::Wait {
            deadline_qpc: QpcTicks::from_raw(60_000),
        }
    );
    assert_eq!(
        exec.evaluate_at(QpcTicks::from_raw(60_000), epoch).unwrap(),
        DispatchDecision::Finished
    );
}

#[test]
fn forced_conversion_failure_fails_executor_construction() {
    let plan = build_onset_plan(
        &[OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 2_000_000,
            scan_codes: smallvec::smallvec![0x15],
        }],
        &[0x15],
        16_667,
    )
    .expect("plan");
    let overflowing_freq = NonZeroU64::new(u64::MAX).expect("max frequency");
    let err = PreparedDispatchSchedule::prepare(&plan, overflowing_freq, 0)
        .expect_err("conversion overflow must reject prepared schedule construction");
    assert!(err.contains("overflow"), "{err}");
}

#[test]
fn prepared_nominal_up_exact_rounding_at_10mhz_and_3579545hz_and_compact_layout() {
    // PreparedChord keeps 32-byte layout (nominal_up_offset_ticks replaces onset_us).
    assert_eq!(std::mem::size_of::<PreparedChord>(), 32);

    // 1. 10 MHz (exact integral 10 ticks/us): ceil((onset + H) * 10) == onset_ticks + hold_ticks.
    let freq_10m = NonZeroU64::new(10_000_000).expect("10 MHz");
    let plan_10m = build_onset_plan(
        &[OnsetChordInput {
            source_action_index: 1,
            scheduled_us: 12_345,
            scan_codes: smallvec::smallvec![0x15],
        }],
        &[0x15],
        17_167,
    )
    .expect("10 MHz plan");
    let prep_10m = PreparedDispatchSchedule::prepare(&plan_10m, freq_10m, 0).expect("10 MHz prep");
    let exec_10m = DispatchExecutor::from_prepared_schedule(prep_10m);
    let c10 = exec_10m.current_chord().expect("chord");
    assert_eq!(c10.onset_ticks, TimelineTicks::from_raw(123_450));
    assert_eq!(
        c10.nominal_up_offset_ticks,
        TimelineTicks::from_raw(295_120)
    );

    // 2. 3,579,545 Hz (NTSC colorburst non-integral QPC):
    //    onset_us = 100, hold_us = 17_167:
    //    ceil(100 * 3579545 / 1e6) = ceil(357.9545) = 358
    //    ceil(17167 * 3579545 / 1e6) = ceil(61450.049015) = 61451
    //    onset_ticks + hold_ticks = 358 + 61451 = 61809
    //    ceil((100 + 17167) * 3579545 / 1e6) = ceil(61808.003515) = 61809 (same here);
    //    Now test onset_us = 1, hold_us = 1:
    //    ceil(1 * 3.579545) = 4; 4 + 4 = 8;
    //    ceil(2 * 3.579545) = ceil(7.15909) = 8 (same);
    //    Now test onset_us = 2, hold_us = 2:
    //    ceil(2 * 3.579545) = ceil(7.15909) = 8; 8 + 8 = 16;
    //    ceil(4 * 3.579545) = ceil(14.31818) = 15 (differs by 1 tick!).
    let freq_ntsc = NonZeroU64::new(3_579_545).expect("3.579545 MHz");
    let plan_ntsc = build_onset_plan(
        &[OnsetChordInput {
            source_action_index: 2,
            scheduled_us: 2,
            scan_codes: smallvec::smallvec![0x15],
        }],
        &[0x15],
        2,
    )
    .expect("NTSC plan");
    let prep_ntsc = PreparedDispatchSchedule::prepare(&plan_ntsc, freq_ntsc, 0).expect("NTSC prep");
    assert_eq!(prep_ntsc.hold_ticks(), DurationTicks::from_raw(8));
    let exec_ntsc = DispatchExecutor::from_prepared_schedule(prep_ntsc);
    let cntsc = exec_ntsc.current_chord().expect("chord");
    assert_eq!(cntsc.onset_ticks, TimelineTicks::from_raw(8));
    assert_eq!(cntsc.nominal_up_offset_ticks, TimelineTicks::from_raw(15));
    assert_ne!(
        cntsc.nominal_up_offset_ticks.as_u64(),
        cntsc.onset_ticks.as_u64() + exec_ntsc.hold_ticks().as_u64(),
        "exact ceil((onset+H)*f/1e6) must stay distinct from ceil(onset*f/1e6) + ceil(H*f/1e6)"
    );
}

#[test]
fn epoch_overflow_during_send_timing_projection_fails_closed() {
    let freq = NonZeroU64::new(1_000_000).expect("1 MHz");
    let plan = build_onset_plan(
        &[OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
        }],
        &[0x15],
        10_000,
    )
    .expect("plan");
    let prep = PreparedDispatchSchedule::prepare(&plan, freq, 0).expect("prep");
    let mut exec = DispatchExecutor::from_prepared_schedule(prep);
    let near_max_epoch = QpcTicks::from_raw(u64::MAX - 5_000);
    // Activate at near_max_epoch - 20_000 so completion + H fits u64,
    // while epoch + nominal_up_offset_ticks (near_max_epoch + 10_000) overflows u64.
    exec.on_send_completed_at(0, 1, QpcTicks::from_raw(u64::MAX - 20_000))
        .expect("Down commit fits u64");
    let err = exec
        .evaluate_at(QpcTicks::from_raw(u64::MAX - 5_000), near_max_epoch)
        .expect_err("epoch + nominal_up_offset_ticks overflow must fail closed");
    assert!(matches!(err, TimeArithmeticError::Overflow));
}

#[test]
fn compact_active_slot_origin_resets_cleanly_on_cleanup_and_terminal_shutdown() {
    let mut exec = make_executor(
        &[(10, 0, &[0x15, 0x16]), (20, 50_000, &[0x15])],
        &[0x15, 0x16],
        10_000,
    );
    let epoch = QpcTicks::from_raw(1_000);
    exec.on_send_completed_at(0, 0b11, epoch)
        .expect("commit Down");
    assert_eq!(exec.up_source_action_index(0b11), Some(10));
    assert!(exec.active_generation_for_slot(0).is_some());
    assert!(exec.active_generation_for_slot(1).is_some());

    assert_eq!(exec.on_verified_cleanup().expect("cleanup"), 2);
    assert_eq!(exec.active_mask(), 0);
    assert_eq!(exec.up_source_action_index(0b11), None);
    assert_eq!(exec.active_generation_for_slot(0), None);
    exec.check_invariants().unwrap();

    // Terminal shutdown cancels remaining scheduled chord cleanly.
    assert_eq!(
        exec.cancel_remaining_generations_for_terminal_shutdown()
            .expect("terminal cancel"),
        1
    );
    exec.check_invariants().unwrap();
}
