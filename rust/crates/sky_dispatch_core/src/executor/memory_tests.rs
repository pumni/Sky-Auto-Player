use super::*;
use crate::model::OnsetChordInput;
use crate::plan::build_onset_plan;
use crate::time::{DurationTicks, QpcTicks, TimelineTicks};

#[test]
fn executor_new_with_cancellation_and_memory_metrics() {
    let allowed = vec![0x15];
    let chords = vec![OnsetChordInput {
        source_action_index: 0,
        scheduled_us: 10_000,
        scan_codes: smallvec::smallvec![0x15],
    }];
    let plan = build_onset_plan(&chords, &allowed, 5_000).expect("plan");

    // 1. Cancellation aborts creation
    let mut check_count = 0;
    let res = DispatchExecutor::new_with_cancellation(
        &plan,
        DurationTicks::from_raw(5_000),
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
    assert_eq!(res.unwrap_err(), "aborted");

    // 2. Successful creation and memory metrics reporting
    let exec = DispatchExecutor::new_with_cancellation(
        &plan,
        DurationTicks::from_raw(5_000),
        |us| Ok::<TimelineTicks, String>(TimelineTicks::from_raw(us)),
        || Ok(()),
    )
    .expect("executor creation");

    let mem = exec.memory_metrics();
    assert_eq!(mem.chord_count, 1);
    assert!(mem.chord_capacity >= 1);
    assert_eq!(
        mem.chord_heap_bytes,
        mem.chord_capacity * std::mem::size_of::<PreparedChord>()
    );
    assert_eq!(mem.struct_bytes, std::mem::size_of::<DispatchExecutor>());
    assert_eq!(mem.total_bytes, mem.chord_heap_bytes + mem.struct_bytes);
}

#[test]
fn onset_executor_tracks_one_five_and_fifteen_key_chords() {
    for key_count in [1usize, 5, crate::model::MAX_KEYS] {
        let allowed: Vec<u16> = (0..key_count).map(|index| 0x15 + index as u16).collect();
        let chords = vec![OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 10_000,
            scan_codes: smallvec::SmallVec::from_slice(&allowed),
        }];
        let plan = build_onset_plan(&chords, &allowed, 1_000).expect("plan");
        let executor = DispatchExecutor::new(&plan, DurationTicks::from_raw(1_000), |us| {
            Ok::<TimelineTicks, String>(TimelineTicks::from_raw(us))
        })
        .expect("executor");
        let chord = executor.current_chord().expect("prepared onset chord");
        assert_eq!(chord.chord_index, 0);
        assert_eq!(chord.source_action_index, 0);
        assert_eq!(chord.onset_us, 10_000);
        assert_eq!(chord.key_count(), key_count);
        assert_eq!(chord.key_mask.count_ones() as usize, key_count);

        let memory = executor
            .try_memory_metrics()
            .expect("executor memory metrics");
        assert_eq!(memory.chord_count, 1);
        assert_eq!(
            memory.chord_heap_bytes,
            memory.chord_capacity * std::mem::size_of::<PreparedChord>()
        );
        eprintln!(
            "dispatch_memory workload=full_chord keys={key_count} executor_struct_bytes={} chord_count={} chord_capacity={} chord_capacity_bytes={} executor_total_bytes={} memory_basis=actual_vector_capacities",
            memory.struct_bytes,
            memory.chord_count,
            memory.chord_capacity,
            memory.chord_heap_bytes,
            memory.total_bytes,
        );
    }
}

#[test]
fn up_slot_timings_derives_nominal_up_and_preserves_completion_floor() {
    let allowed = vec![0x15, 0x16];
    let chords = vec![OnsetChordInput {
        source_action_index: 10,
        scheduled_us: 10_000,
        scan_codes: smallvec::smallvec![0x15, 0x16],
    }];
    let plan = build_onset_plan(&chords, &allowed, 8_000).expect("plan");
    let mut exec = DispatchExecutor::new(&plan, DurationTicks::from_raw(8_000), |us| {
        Ok::<TimelineTicks, String>(TimelineTicks::from_raw(us))
    })
    .expect("executor");

    assert!(exec.up_slot_timings(0b11).is_empty());

    // Complete Down late at 12_000 ticks (authored onset was 10_000).
    // Completion-anchored release floor = 12_000 + 8_000 = 20_000.
    // Diagnostic nominal Up = 10_000 + 8_000 = 18_000.
    exec.on_send_completed(0, 0b11, TimelineTicks::from_raw(12_000))
        .expect("down send completed");

    let timings = exec.up_slot_timings(0b11);
    assert_eq!(timings.len(), 2);
    for (slot, scan_code) in [(0u8, 0x15u16), (1u8, 0x16u16)] {
        let timing = timings.iter().find(|t| t.slot == slot).expect("slot");
        assert_eq!(timing.scan_code, scan_code);
        assert_eq!(timing.source_action_index, 10);
        assert_eq!(
            timing.nominal_up_offset_ticks,
            TimelineTicks::from_raw(18_000)
        );
        assert_eq!(timing.release_not_before_qpc, QpcTicks::from_raw(20_000));
    }

    let epoch = QpcTicks::from_raw(1_000);
    let summary = exec
        .up_slot_timing_summary(0b11, epoch)
        .expect("summary arithmetic")
        .expect("active Up slots");
    assert_eq!(summary.slot_count, 2);
    assert_eq!(summary.first_source_action_index, 10);
    assert_eq!(summary.authored_target_qpc, QpcTicks::from_raw(19_000));
    assert_eq!(
        summary.musical_up_not_before_qpc,
        QpcTicks::from_raw(20_000)
    );
    assert_eq!(summary.hold_floor_mask, 0b11);
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
        let mut inputs = Vec::with_capacity(chord_count);
        for i in 0..chord_count {
            let t = (i as u64 + 1) * 30_000;
            inputs.push(OnsetChordInput {
                source_action_index: i as u32,
                scheduled_us: t,
                scan_codes: smallvec::smallvec![0x15],
            });
        }
        let plan = build_onset_plan(&inputs, &allowed, 8_000).expect("plan");
        assert_eq!(plan.chords.len(), chord_count);
        let plan_memory = plan.memory_metrics().expect("plan memory metrics");
        let compile_memory = plan_memory.compile_memory;
        let executor = DispatchExecutor::new(&plan, DurationTicks::from_raw(8_000), |us| {
            Ok::<TimelineTicks, String>(TimelineTicks::from_raw(us))
        })
        .expect("production executor construction");
        let executor_memory = executor
            .try_memory_metrics()
            .expect("executor memory metrics");
        assert_eq!(executor.down_chord_count(), chord_count);
        let plan_heap_bytes = plan_memory.heap_payload_bytes;
        let live_scheduler_owned_bytes = plan_heap_bytes
            .checked_add(executor_memory.total_bytes)
            .expect("plan plus executor capacity");
        assert!(live_scheduler_owned_bytes <= crate::model::MAX_RETAINED_DISPATCH_BYTES);
        assert!(executor_memory.total_bytes <= MAX_EXECUTOR_BYTES);

        drop(executor);
        let prepared = PreparedDispatchSchedule::prepare(
            &plan,
            std::num::NonZeroU64::new(1_000_000).expect("QPC frequency"),
            compile_memory.caller_retained_capacity_bytes,
        )
        .expect("prepared dispatch plan");
        let prepared_memory = prepared.memory_metrics();
        let prepared_startup_peak_capacity_bound = compile_memory
            .phase_peak_capacity_bytes
            .max(prepared_memory.overlapping_peak_capacity_bytes);
        assert!(
            prepared_memory.overlapping_peak_capacity_bytes
                <= crate::model::MAX_RETAINED_DISPATCH_BYTES
        );
        assert!(prepared_startup_peak_capacity_bound <= crate::model::MAX_RETAINED_DISPATCH_BYTES);
        eprintln!(
            "prepared_dispatch_memory workload=one_key chords={chord_count} compiler_phase_peak_capacity_bytes={} prepared_plan_capacity_bytes={} prepare_overlapping_peak_capacity_bytes={} prepared_startup_peak_capacity_bound_bytes={prepared_startup_peak_capacity_bound} policy_min_hold_us=8000 qpc_frequency_hz=1000000",
            compile_memory.phase_peak_capacity_bytes,
            prepared_memory.plan_capacity_bytes,
            prepared_memory.overlapping_peak_capacity_bytes,
        );

        let per_chord_bytes = live_scheduler_owned_bytes / chord_count;
        assert!(
            per_chord_bytes < 256,
            "per chord bytes too large: {per_chord_bytes}"
        );
    }
}

#[test]
fn executor_storage_admission_rejects_overflow_and_budget_excess() {
    let chord_size = std::mem::size_of::<PreparedChord>();
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
fn executor_reservation_reports_real_try_reserve_failure() {
    let mut chords = Vec::new();
    let error = reserve_executor_chords(&mut chords, usize::MAX)
        .expect_err("unrepresentable Vec capacity must be denied by try_reserve_exact");
    assert!(error.contains("executor chord capacity allocation failed"));
    assert!(chords.is_empty());
}

#[test]
fn executor_cancellation_interrupts_large_materialization() {
    let mut inputs = Vec::new();
    for index in 0..9_000u32 {
        inputs.push(OnsetChordInput {
            source_action_index: index,
            scheduled_us: u64::from(index) * 20_000,
            scan_codes: smallvec::smallvec![0x15],
        });
    }
    let plan = build_onset_plan(&inputs, &[0x15], 8_000).expect("plan");
    let mut checks = 0;
    let mut converted_timestamps = 0;
    let result = DispatchExecutor::new_with_cancellation(
        &plan,
        DurationTicks::from_raw(8_000),
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
