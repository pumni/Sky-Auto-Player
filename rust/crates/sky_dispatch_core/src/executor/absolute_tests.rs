use super::*;
use crate::model::OnsetChordInput;
use crate::plan::build_onset_plan;
use crate::time::QpcTicks;

fn build_executor(
    chords: &[(u32, u64, &[u16])],
    allowed: &[u16],
    hold_us: u64,
) -> DispatchExecutor {
    let inputs: Vec<OnsetChordInput> = chords
        .iter()
        .map(|&(idx, us, scans)| OnsetChordInput {
            source_action_index: idx,
            scheduled_us: us,
            scan_codes: scans.iter().copied().collect(),
        })
        .collect();
    let plan = build_onset_plan(&inputs, allowed, hold_us).expect("valid onset plan");
    let qpc_1mhz = std::num::NonZeroU64::new(1_000_000).expect("1 MHz");
    let prepared = PreparedDispatchSchedule::prepare(&plan, qpc_1mhz, 0).expect("prepared");
    DispatchExecutor::from_prepared_schedule(prepared)
}

#[test]
fn absolute_qpc_no_early_down_and_completion_anchored_release() {
    let mut exec = build_executor(&[(0, 1_000, &[0x15])], &[0x15], 16_667);
    let epoch = QpcTicks::from_raw(5_000_000);

    // Before first onset: Wait until epoch + 1_000 (no early send, D3).
    assert_eq!(
        exec.evaluate_at(QpcTicks::from_raw(5_000_999), epoch)
            .unwrap(),
        DispatchDecision::Wait {
            deadline_qpc: QpcTicks::from_raw(5_001_000),
        }
    );

    // At onset: Down eligible.
    let decision = exec
        .evaluate_at(QpcTicks::from_raw(5_001_000), epoch)
        .unwrap();
    let DispatchDecision::Send {
        up_mask: 0,
        down_chord: Some(chord),
        timing_window,
    } = decision
    else {
        panic!("expected Down send, got {decision:?}");
    };
    assert_eq!(chord.key_mask, 1);
    assert_eq!(
        timing_window.authored_target_qpc,
        QpcTicks::from_raw(5_001_000)
    );
    assert_eq!(
        timing_window.down_not_before_qpc,
        QpcTicks::from_raw(5_001_000)
    );

    // Late Down completion at epoch + 1_500 -> release floor = epoch + 1_500 + 16_667 = 5_018_167.
    exec.on_send_completed_at(0, 1, QpcTicks::from_raw(5_001_500))
        .unwrap();
    assert_eq!(exec.active_mask(), 1);
    assert_eq!(
        exec.slot_release_not_before_qpc(0),
        Some(QpcTicks::from_raw(5_018_167))
    );

    // At diagnostic nominal Up (epoch + 17_667 = 5_017_667): still Wait until 5_018_167!
    assert_eq!(
        exec.evaluate_at(QpcTicks::from_raw(5_017_667), epoch)
            .unwrap(),
        DispatchDecision::Wait {
            deadline_qpc: QpcTicks::from_raw(5_018_167),
        }
    );

    // At 5_018_167: Up is eligible and marks hold_floor_mask bit 0.
    let up_decision = exec
        .evaluate_at(QpcTicks::from_raw(5_018_167), epoch)
        .unwrap();
    let DispatchDecision::Send {
        up_mask: 1,
        down_chord: None,
        timing_window,
    } = up_decision
    else {
        panic!("expected Up send, got {up_decision:?}");
    };
    assert_eq!(
        timing_window.musical_up_not_before_qpc,
        QpcTicks::from_raw(5_018_167)
    );
    assert_eq!(timing_window.hold_floor_mask, 1);

    exec.on_send_completed_at(1, 0, QpcTicks::from_raw(5_018_170))
        .unwrap();
    assert_eq!(exec.active_mask(), 0);
    assert_eq!(
        exec.evaluate_at(QpcTicks::from_raw(5_018_170), epoch)
            .unwrap(),
        DispatchDecision::Finished
    );
}

#[test]
fn blocked_same_key_repeat_waits_for_blocking_release_without_spinning() {
    let h = 16_667u64;
    let mut exec = build_executor(&[(0, 0, &[0x15]), (1, h, &[0x15])], &[0x15], h);
    let epoch = QpcTicks::from_raw(10_000);

    // First Down completes late at epoch + 2_000 -> release floor = epoch + 2_000 + h.
    exec.on_send_completed_at(0, 1, QpcTicks::from_raw(12_000))
        .unwrap();

    // At epoch + h (second chord's authored onset): slot 0 is still active until 12_000 + h.
    // Executor MUST NOT return Wait(epoch + h) (which would spin); it must return Wait(12_000 + h)!
    let at_authored = QpcTicks::from_raw(10_000 + h);
    assert_eq!(
        exec.evaluate_at(at_authored, epoch).unwrap(),
        DispatchDecision::Wait {
            deadline_qpc: QpcTicks::from_raw(12_000 + h),
        }
    );

    // At 12_000 + h: Up only (no Up+Down of same slot in one packet, D5).
    let up_only = exec
        .evaluate_at(QpcTicks::from_raw(12_000 + h), epoch)
        .unwrap();
    assert!(matches!(
        up_only,
        DispatchDecision::Send {
            up_mask: 1,
            down_chord: None,
            ..
        }
    ));

    // After Up commits, re-Down is immediately eligible with no G!
    exec.on_send_completed_at(1, 0, QpcTicks::from_raw(12_000 + h + 5))
        .unwrap();
    let re_down = exec
        .evaluate_at(QpcTicks::from_raw(12_000 + h + 5), epoch)
        .unwrap();
    assert!(matches!(
        re_down,
        DispatchDecision::Send {
            up_mask: 0,
            down_chord: Some(chord),
            ..
        } if chord.chord_index == 1 && chord.key_mask == 1
    ));
}

#[test]
fn disjoint_due_up_and_next_down_combine_in_one_packet() {
    let h = 16_667u64;
    let mut exec = build_executor(&[(0, 0, &[0x15]), (1, h, &[0x16])], &[0x15, 0x16], h);
    let epoch = QpcTicks::from_raw(1_000);

    // Key 0 completes on time at epoch -> release floor = epoch + h.
    exec.on_send_completed_at(0, 1, epoch).unwrap();

    // At epoch + h: Up(slot 0) and Down(slot 1) are disjoint and both due -> combined!
    let decision = exec
        .evaluate_at(QpcTicks::from_raw(1_000 + h), epoch)
        .unwrap();
    assert!(matches!(
        decision,
        DispatchDecision::Send {
            up_mask: 1,
            down_chord: Some(chord),
            ..
        } if chord.chord_index == 1 && chord.key_mask == 2
    ));
}

#[test]
fn arithmetic_overflow_fails_closed_before_mutation() {
    let mut exec = build_executor(&[(0, 100, &[0x15])], &[0x15], 16_667);
    assert!(
        exec.evaluate_at(QpcTicks::from_raw(u64::MAX), QpcTicks::from_raw(u64::MAX))
            .is_err()
    );

    let err = exec
        .on_send_completed_at(0, 1, QpcTicks::from_raw(u64::MAX - 10))
        .unwrap_err();
    assert!(err.contains("overflow"));
    // State must remain unmutated on error.
    assert_eq!(exec.chord_cursor(), 0);
    assert_eq!(exec.active_mask(), 0);
    assert_eq!(exec.accounting().activated, 0);
}
