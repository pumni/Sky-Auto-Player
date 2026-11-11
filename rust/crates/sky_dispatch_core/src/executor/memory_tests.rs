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

#[test]
fn executor_memory_scaling_100_1000_10000_and_budget_proof() {
    let allowed = vec![0x15];
    let selected = std::env::var("SKY_MEMORY_CHORD_COUNT")
        .ok()
        .map(|value| value.parse::<usize>().expect("valid memory workload"));
    assert!(selected.is_none_or(|count| [100, 1_000, 10_000, 100_000].contains(&count)));
    for &chord_count in &[100, 1_000, 10_000, 100_000] {
        if selected.is_some_and(|count| count != chord_count) {
            continue;
        }
        let mut actions = Vec::with_capacity(chord_count * 2);
        for i in 0..chord_count {
            let t = (i as u64 + 1) * 20_000;
            actions.push(KeyActionInput {
                source_action_index: (i * 2) as u32,
                kind: ActionKind::Down,
                scheduled_us: t,
                scan_codes: smallvec::smallvec![0x15],
                reason: "down".into(),
            });
            actions.push(KeyActionInput {
                source_action_index: (i * 2 + 1) as u32,
                kind: ActionKind::Up,
                scheduled_us: t + 10_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up".into(),
            });
        }
        let compiler_input_capacity_bytes = actions
            .capacity()
            .checked_mul(std::mem::size_of::<KeyActionInput>())
            .and_then(|bytes| {
                actions.iter().try_fold(bytes, |total, action| {
                    total.checked_add(action.reason.len())
                })
            })
            .expect("compiler input capacity accounting");
        let sched = compile_runtime_intents(&actions, &allowed).expect("schedule");
        assert_eq!(sched.down_chords.len(), chord_count);
        let schedule_memory = sched.memory_metrics().expect("schedule memory metrics");
        let schedule = sched;
        let mut coordinator = crate::coordinator::RuntimeDispatchCoordinator::new(
            schedule,
            0,
            0,
            TimelineTicks::from_raw,
        );
        coordinator
            .attach_executor_with_cancellation(
                DurationTicks::from_raw(8_000),
                DurationTicks::from_raw(16_667),
                |us| Ok::<TimelineTicks, String>(TimelineTicks::from_raw(us)),
                || Ok(()),
            )
            .expect("combined coordinator and executor admission");
        let mem = coordinator
            .memory_metrics()
            .expect("combined memory metrics");
        assert_eq!(
            coordinator.executor.as_ref().unwrap().down_chord_count(),
            chord_count
        );
        assert!(
            mem.executor_heap_bytes
                >= chord_count * std::mem::size_of::<crate::executor::DownChordTicks>()
        );
        assert!(mem.retained_capacity_bytes <= crate::model::MAX_RETAINED_DISPATCH_BYTES);
        assert!(mem.startup_peak_capacity_bytes <= crate::model::MAX_RETAINED_DISPATCH_BYTES);

        let reason_map_entry_bound = std::mem::size_of::<(String, crate::model::ReasonId)>() * 2
            + crate::compile::MAX_REASON_BYTES * 2;
        let compile_reason_map_bound = (u16::MAX as usize + 1) * reason_map_entry_bound;
        let compiler_phase_peak_upper_bound = compiler_input_capacity_bytes
            .checked_add(schedule_memory.heap_payload_bytes)
            .and_then(|bytes| bytes.checked_add(schedule_memory.struct_bytes))
            .and_then(|bytes| bytes.checked_add(compile_reason_map_bound))
            .expect("compiler phase upper bound");
        let overall_startup_peak_capacity_bound =
            compiler_phase_peak_upper_bound.max(mem.startup_peak_capacity_bytes);
        eprintln!(
            "dispatch_memory workload=one_key chords={chord_count} memory_basis=calculated_size_and_capacity windows_process_memory=not_measured compiler_input_capacity_bytes={compiler_input_capacity_bytes} schedule_heap_capacity_bytes={} coordinator_heap_capacity_bytes={} executor_heap_capacity_bytes={} coordinator_struct_bytes={} retained_capacity_bytes={} compiler_phase_peak_upper_bound_bytes={compiler_phase_peak_upper_bound} worker_startup_peak_capacity_bytes={} overall_startup_peak_capacity_bound_bytes={overall_startup_peak_capacity_bound}",
            mem.schedule_heap_bytes,
            mem.coordinator_heap_bytes,
            mem.executor_heap_bytes,
            mem.coordinator_struct_bytes,
            mem.retained_capacity_bytes,
            mem.startup_peak_capacity_bytes,
        );
        assert!(
            mem.retained_capacity_bytes <= MAX_EXECUTOR_BYTES,
            "chord_count={chord_count} exceeded combined retained dispatch budget: {} > {}",
            mem.retained_capacity_bytes,
            MAX_EXECUTOR_BYTES
        );

        // Verify linearity: per-chord combined footprint is strictly bounded (<2 KiB per chord)
        let per_chord_bytes = mem.retained_capacity_bytes / chord_count;
        assert!(
            per_chord_bytes < 2048,
            "per chord bytes too large: {per_chord_bytes}"
        );
    }
}

#[test]
fn up_slot_timings_multiple_slots_with_distinct_authored_and_floor() {
    let allowed = vec![0x15, 0x16];
    let actions = vec![
        KeyActionInput {
            source_action_index: 10,
            kind: ActionKind::Down,
            scheduled_us: 10_000,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "down".into(),
        },
        KeyActionInput {
            source_action_index: 11,
            kind: ActionKind::Up,
            scheduled_us: 25_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up0".into(),
        },
        KeyActionInput {
            source_action_index: 12,
            kind: ActionKind::Up,
            scheduled_us: 40_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "up1".into(),
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

    // Complete Down at 20_000 ticks.
    // Hold floor H = 8_000, so release floor is 20_000 + 8_000 = 28_000 for both keys.
    exec.on_send_completed(0, 0b11, TimelineTicks::from_raw(20_000))
        .expect("down send completed");

    // Query both active slots
    let timings = exec.up_slot_timings(0b11);
    assert_eq!(timings.len(), 2);

    let timing0 = timings.iter().find(|t| t.slot == 0).expect("slot 0");
    let timing1 = timings.iter().find(|t| t.slot == 1).expect("slot 1");

    assert_eq!(timing0.scan_code, 0x15);
    assert_eq!(timing0.source_action_index, 11);
    assert_eq!(
        timing0.authored_up_offset_ticks,
        TimelineTicks::from_raw(25_000)
    );
    assert_eq!(
        timing0.release_not_before_ticks,
        TimelineTicks::from_raw(28_000)
    );
    // Key 0 floor displacement = 28_000 - 25_000 = 3_000 ticks
    let floor_disp0 = timing0
        .release_not_before_ticks
        .as_u64()
        .saturating_sub(timing0.authored_up_offset_ticks.as_u64());
    assert_eq!(floor_disp0, 3_000);

    assert_eq!(timing1.scan_code, 0x16);
    assert_eq!(timing1.source_action_index, 12);
    assert_eq!(
        timing1.authored_up_offset_ticks,
        TimelineTicks::from_raw(40_000)
    );
    assert_eq!(
        timing1.release_not_before_ticks,
        TimelineTicks::from_raw(28_000)
    );
    // Key 1 floor displacement = 0 ticks (authored 40_000 > floor 28_000)
    let floor_disp1 = timing1
        .release_not_before_ticks
        .as_u64()
        .saturating_sub(timing1.authored_up_offset_ticks.as_u64());
    assert_eq!(floor_disp1, 0);
}
#[test]
fn executor_storage_admission_rejects_overflow_and_budget_excess() {
    let chord_size = std::mem::size_of::<DownChordTicks>();
    let largest_admitted = MAX_EXECUTOR_BYTES / chord_size;
    assert!(checked_executor_storage_bytes(largest_admitted).is_ok());
    assert!(
        checked_executor_storage_bytes(largest_admitted + 1)
            .unwrap_err()
            .contains("exceeds budget")
    );
    assert!(
        checked_executor_storage_bytes(usize::MAX)
            .unwrap_err()
            .contains("overflow")
    );
}

#[test]
fn executor_cancellation_interrupts_large_materialization() {
    let mut actions = Vec::new();
    for index in 0..1_000u32 {
        actions.push(KeyActionInput {
            source_action_index: index * 2,
            kind: ActionKind::Down,
            scheduled_us: u64::from(index) * 20_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down".into(),
        });
        actions.push(KeyActionInput {
            source_action_index: index * 2 + 1,
            kind: ActionKind::Up,
            scheduled_us: u64::from(index) * 20_000 + 10_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up".into(),
        });
    }
    let schedule = compile_runtime_intents(&actions, &[0x15]).expect("schedule");
    let mut checks = 0;
    let result = DispatchExecutor::new_with_cancellation(
        &schedule,
        DurationTicks::from_raw(8_000),
        DurationTicks::from_raw(16_667),
        |us| Ok::<TimelineTicks, String>(TimelineTicks::from_raw(us)),
        || {
            checks += 1;
            if checks == 2 {
                Err("cancelled during materialization".into())
            } else {
                Ok(())
            }
        },
    );
    assert!(
        result
            .unwrap_err()
            .contains("cancelled during materialization")
    );
    assert_eq!(checks, 2);
}
