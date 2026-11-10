use super::*;
use crate::compile::compile_runtime_intents;
use crate::model::{ActionKind, KeyActionInput};
use crate::time::{DurationTicks, TimelineTicks};

#[test]
fn executor_new_with_cancellation_and_memory_metrics() {
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
    let sched = compile_runtime_intents(&actions, &allowed).expect("schedule");

    // 1. Cancellation aborts creation
    let mut check_count = 0;
    let res = DispatchExecutor::new_with_cancellation(
        &sched,
        DurationTicks::from_raw(5_000),
        DurationTicks::from_raw(16_667),
        |us| Ok::<TimelineTicks, String>(TimelineTicks::from_raw(us)),
        || {
            check_count += 1;
            if check_count > 1 {
                Err("aborted".to_string())
            } else {
                Ok(())
            }
        },
    );
    assert!(res.is_err());
    assert_eq!(res.unwrap_err(), "aborted");

    // 2. Successful creation and memory metrics reporting
    let exec = DispatchExecutor::new_with_cancellation(
        &sched,
        DurationTicks::from_raw(5_000),
        DurationTicks::from_raw(16_667),
        |us| Ok::<TimelineTicks, String>(TimelineTicks::from_raw(us)),
        || Ok(()),
    )
    .expect("executor creation");

    let mem = exec.memory_metrics();
    assert_eq!(mem.chord_count, 1);
    assert!(mem.chord_capacity >= 1);
    assert_eq!(
        mem.chord_heap_bytes,
        mem.chord_capacity * std::mem::size_of::<crate::executor::DownChordTicks>()
    );
    assert_eq!(mem.struct_bytes, std::mem::size_of::<DispatchExecutor>());
    assert_eq!(mem.total_bytes, mem.chord_heap_bytes + mem.struct_bytes);
}

#[test]
fn up_slot_timings_preserves_exact_authored_and_floor() {
    let allowed = vec![0x15];
    let actions = vec![
        KeyActionInput {
            source_action_index: 10,
            kind: ActionKind::Down,
            scheduled_us: 10_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down".into(),
        },
        KeyActionInput {
            source_action_index: 11,
            kind: ActionKind::Up,
            scheduled_us: 25_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up".into(),
        },
    ];
    let sched = compile_runtime_intents(&actions, &allowed).expect("schedule");
    let mut exec = DispatchExecutor::new(
        &sched,
        DurationTicks::from_raw(8_000),
        DurationTicks::from_raw(16_667),
        |us| Ok::<TimelineTicks, String>(TimelineTicks::from_raw(us)),
    )
    .expect("executor");

    // Before Down completion: slot 0 is inactive
    let timings_before = exec.up_slot_timings(1);
    assert!(timings_before.is_empty());

    // Complete Down at 12_000 ticks
    exec.on_send_completed(0, 1, TimelineTicks::from_raw(12_000))
        .expect("down send completed");

    // After Down completion: slot 0 has exact authored up offset (25_000) and release floor (12_000 + 8_000 = 20_000)
    let timings_after = exec.up_slot_timings(1);
    assert_eq!(timings_after.len(), 1);
    let timing = timings_after[0];
    assert_eq!(timing.slot, 0);
    assert_eq!(timing.scan_code, 0x15);
    assert_eq!(timing.source_action_index, 11);
    assert_eq!(
        timing.authored_up_offset_ticks,
        TimelineTicks::from_raw(25_000)
    );
    assert_eq!(
        timing.release_not_before_ticks,
        TimelineTicks::from_raw(20_000)
    );
}
