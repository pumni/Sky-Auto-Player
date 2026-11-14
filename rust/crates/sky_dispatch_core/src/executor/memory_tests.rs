use super::*;
use crate::compile::compile_runtime_intents;
use crate::model::{ActionKind, KeyActionInput};
use crate::time::{DurationTicks, QpcTicks, TimelineTicks};

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
    assert_eq!(mem.intent_count, 1);
    assert!(mem.intent_capacity >= 1);
    assert_eq!(
        mem.chord_header_heap_bytes,
        mem.chord_capacity * std::mem::size_of::<DownChordHeaderTicks>()
    );
    assert_eq!(
        mem.intent_heap_bytes,
        mem.intent_capacity * std::mem::size_of::<DownIntentTicks>()
    );
    assert_eq!(
        mem.chord_heap_bytes,
        mem.chord_header_heap_bytes + mem.intent_heap_bytes
    );
    assert_eq!(mem.struct_bytes, std::mem::size_of::<DispatchExecutor>());
    assert_eq!(mem.total_bytes, mem.chord_heap_bytes + mem.struct_bytes);
}

#[test]
fn compact_executor_arena_tracks_one_five_and_fifteen_key_chords() {
    for key_count in [1usize, 5, crate::model::MAX_KEYS] {
        let allowed: Vec<u16> = (0..key_count).map(|index| 0x15 + index as u16).collect();
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 10_000,
                scan_codes: smallvec::SmallVec::from_slice(&allowed),
                reason: "full chord Down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 30_000,
                scan_codes: smallvec::SmallVec::from_slice(&allowed),
                reason: "full chord Up".into(),
            },
        ];
        let schedule = compile_runtime_intents(&actions, &allowed).expect("schedule");
        let executor = DispatchExecutor::new(
            &schedule,
            DurationTicks::from_raw(1_000),
            DurationTicks::from_raw(16_667),
            |us| Ok::<TimelineTicks, String>(TimelineTicks::from_raw(us)),
        )
        .expect("compact executor");
        let chord = executor.current_chord().expect("compiled Down chord");
        assert_eq!(chord.chord_index, 0);
        assert_eq!(chord.source_action_index, 0);
        assert_eq!(chord.scheduled_us, 10_000);
        assert!(
            std::mem::size_of::<DownChordMetadata>() < std::mem::size_of::<DownChordTicks>(),
            "production staged provenance must not retain the owned chord fixture"
        );
        let actual_scan_codes: Vec<u16> = chord
            .intents
            .iter()
            .map(|intent| intent.scan_code)
            .collect();
        assert_eq!(
            actual_scan_codes, allowed,
            "intent order for {key_count} keys"
        );
        assert_eq!(chord.intents.len(), key_count);
        assert_eq!(chord.down_mask.count_ones() as usize, key_count);

        let memory = executor
            .try_memory_metrics()
            .expect("executor memory metrics");
        assert_eq!(memory.chord_count, 1);
        assert_eq!(memory.intent_count, key_count);
        assert_eq!(
            memory.chord_header_heap_bytes,
            memory.chord_capacity * std::mem::size_of::<DownChordHeaderTicks>()
        );
        assert_eq!(
            memory.intent_heap_bytes,
            memory.intent_capacity * std::mem::size_of::<DownIntentTicks>()
        );
        assert_eq!(
            memory.chord_heap_bytes,
            memory.chord_header_heap_bytes + memory.intent_heap_bytes
        );
        assert!(memory.chord_capacity >= memory.chord_count);
        assert!(memory.intent_capacity >= memory.intent_count);
        assert_eq!(
            memory.chord_heap_bytes,
            memory.chord_header_heap_bytes + memory.intent_heap_bytes
        );
        eprintln!(
            "dispatch_memory workload=full_chord keys={key_count} executor_struct_bytes={} compact_headers={} header_capacity_bytes={} intent_count={} intent_capacity={} intent_capacity_bytes={} executor_vector_capacity_bytes={} executor_total_bytes={} memory_basis=actual_vector_capacities",
            memory.struct_bytes,
            memory.chord_count,
            memory.chord_header_heap_bytes,
            memory.intent_count,
            memory.intent_capacity,
            memory.intent_heap_bytes,
            memory.chord_heap_bytes,
            memory.total_bytes,
        );
    }
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
    assert_eq!(timing.release_not_before_qpc, QpcTicks::from_raw(20_000));
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
        let sched = compile_runtime_intents(&actions, &allowed).expect("schedule");
        assert_eq!(sched.down_chords.len(), chord_count);
        let schedule_memory = sched.memory_metrics().expect("schedule memory metrics");
        let compile_memory = schedule_memory.compile_memory;
        let executor = DispatchExecutor::new(
            &sched,
            DurationTicks::from_raw(8_000),
            DurationTicks::from_raw(16_667),
            |us| Ok::<TimelineTicks, String>(TimelineTicks::from_raw(us)),
        )
        .expect("production executor construction");
        let executor_memory = executor
            .try_memory_metrics()
            .expect("executor memory metrics");
        assert_eq!(executor.down_chord_count(), chord_count);
        let runtime_schedule_capacity_bytes = schedule_memory.heap_payload_bytes;
        let live_scheduler_owned_bytes = runtime_schedule_capacity_bytes
            .checked_add(executor_memory.total_bytes)
            .expect("schedule plus executor capacity");
        let startup_peak_capacity_bound = compile_memory
            .phase_peak_capacity_bytes
            .max(live_scheduler_owned_bytes);
        assert!(live_scheduler_owned_bytes <= crate::model::MAX_RETAINED_DISPATCH_BYTES);
        assert!(executor_memory.total_bytes <= MAX_EXECUTOR_BYTES);
        eprintln!(
            "dispatch_memory workload=one_key chords={chord_count} memory_basis=calculated_size_and_capacity complete_application_process_peak=not_measured compiler_input_capacity_bytes={} compiler_temporary_capacity_bytes={} caller_retained_capacity_bytes={} runtime_schedule_heap_capacity_bytes={} executor_heap_capacity_bytes={} executor_struct_bytes={} executor_total_bytes={} schedule_and_executor_retained_capacity_bytes={live_scheduler_owned_bytes} compiler_phase_peak_capacity_bytes={} startup_peak_capacity_bound_bytes={startup_peak_capacity_bound} lifetime_model=compiled_schedule_and_executor_coexist_after_executor_construction;compiler_temporary_capacity_drops_after_compile;complete_worker_and_process_overhead_not_measured",
            compile_memory.input_capacity_bytes,
            compile_memory.temporary_capacity_bytes,
            compile_memory.caller_retained_capacity_bytes,
            runtime_schedule_capacity_bytes,
            executor_memory.chord_heap_bytes,
            executor_memory.struct_bytes,
            executor_memory.total_bytes,
            compile_memory.phase_peak_capacity_bytes,
        );
        assert!(
            live_scheduler_owned_bytes <= crate::model::MAX_RETAINED_DISPATCH_BYTES,
            "chord_count={chord_count} exceeded compiled schedule plus executor capacity budget: {live_scheduler_owned_bytes} > {}",
            crate::model::MAX_RETAINED_DISPATCH_BYTES
        );

        // Verify linearity: per-chord compiled schedule plus executor capacity is bounded.
        let per_chord_bytes = live_scheduler_owned_bytes / chord_count;
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
    assert_eq!(timing0.release_not_before_qpc, QpcTicks::from_raw(28_000));
    // Key 0 floor displacement = 28_000 - 25_000 = 3_000 ticks
    let floor_disp0 = timing0
        .release_not_before_qpc
        .as_u64()
        .saturating_sub(timing0.authored_up_offset_ticks.as_u64());
    assert_eq!(floor_disp0, 3_000);

    assert_eq!(timing1.scan_code, 0x16);
    assert_eq!(timing1.source_action_index, 12);
    assert_eq!(
        timing1.authored_up_offset_ticks,
        TimelineTicks::from_raw(40_000)
    );
    assert_eq!(timing1.release_not_before_qpc, QpcTicks::from_raw(28_000));
    // Key 1 floor displacement = 0 ticks (authored 40_000 > floor 28_000)
    let floor_disp1 = timing1
        .release_not_before_qpc
        .as_u64()
        .saturating_sub(timing1.authored_up_offset_ticks.as_u64());
    assert_eq!(floor_disp1, 0);
}
#[test]
fn executor_storage_admission_rejects_overflow_and_budget_excess() {
    let header_size = std::mem::size_of::<DownChordHeaderTicks>();
    let intent_size = std::mem::size_of::<DownIntentTicks>();
    let per_chord_size = header_size + intent_size;
    let largest_admitted = MAX_EXECUTOR_BYTES / per_chord_size;
    assert!(checked_executor_storage_bytes(largest_admitted, largest_admitted).is_ok());
    assert!(
        checked_executor_storage_bytes(largest_admitted + 1, largest_admitted + 1)
            .unwrap_err()
            .contains("exceeds budget")
    );
    assert!(
        checked_executor_storage_bytes(usize::MAX, 0)
            .unwrap_err()
            .contains("overflow")
    );
    assert!(
        checked_executor_storage_bytes(0, usize::MAX)
            .unwrap_err()
            .contains("overflow")
    );
}

#[test]
fn executor_reservation_reports_real_try_reserve_failure() {
    let mut chords = Vec::new();
    let error = reserve_executor_chords(&mut chords, usize::MAX)
        .expect_err("unrepresentable Vec capacity must be denied by try_reserve_exact");
    assert!(error.contains("executor chord capacity allocation failed"));
    assert!(chords.is_empty());

    let mut intents = Vec::new();
    let error = reserve_executor_intents(&mut intents, usize::MAX)
        .expect_err("unrepresentable intent arena capacity must be denied by try_reserve_exact");
    assert!(error.contains("executor intent-arena capacity allocation failed"));
    assert!(intents.is_empty());
}

#[test]
fn executor_cancellation_interrupts_large_materialization() {
    let mut actions = Vec::new();
    for index in 0..9_000u32 {
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
    let mut converted_timestamps = 0;
    let result = DispatchExecutor::new_with_cancellation(
        &schedule,
        DurationTicks::from_raw(8_000),
        DurationTicks::from_raw(16_667),
        |us| {
            converted_timestamps += 1;
            Ok::<TimelineTicks, String>(TimelineTicks::from_raw(us))
        },
        || {
            checks += 1;
            if checks == 4 {
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
    assert_eq!(checks, 4);
    assert_eq!(converted_timestamps, 2 * 4_096);
}
