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
    DispatchExecutor::new(&plan, DurationTicks::from_raw(hold_us), |us| {
        Ok::<TimelineTicks, std::convert::Infallible>(TimelineTicks::from_raw(us))
    })
    .expect("executor")
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
