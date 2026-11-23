use super::*;
use crate::compile::compile_runtime_intents;
use crate::model::{ActionKind, KeyActionInput};

const FREQUENCY_HZ: u64 = 3_579_545;

#[derive(Debug, Eq, PartialEq)]
struct ChordOracle {
    index: usize,
    scheduled_us: u64,
    scheduled_ticks: u64,
    source_action_index: u32,
    down_mask: u16,
    intent_start: usize,
    intent_count: usize,
    reason: String,
}

#[derive(Debug, Eq, PartialEq)]
struct IntentOracle {
    source_action_index: u32,
    generation_id: GenerationId,
    key_slot: KeySlot,
    scan_code: u16,
    reason: String,
    paired_up_source_action_index: u32,
    paired_up_offset_us: u64,
    paired_up_offset_ticks: u64,
}

fn qpc_ceil(microseconds: u64) -> TimelineTicks {
    let numerator = u128::from(microseconds) * u128::from(FREQUENCY_HZ) + 999_999;
    TimelineTicks::from_raw(u64::try_from(numerator / 1_000_000).expect("fixture conversion"))
}

fn fixture() -> RuntimeSchedule {
    let allowed = [0x15, 0x16, 0x17];
    let actions = [
        (
            10,
            ActionKind::Down,
            1_001,
            &[0x15, 0x16][..],
            "first chord",
        ),
        (11, ActionKind::Up, 1_301, &[0x15][..], "release A"),
        (12, ActionKind::Up, 1_701, &[0x16][..], "release B"),
        (13, ActionKind::Down, 1_801, &[0x15][..], "second A"),
        (14, ActionKind::Up, 2_501, &[0x15][..], "second A up"),
        (15, ActionKind::Down, 2_601, &[0x17][..], "third C"),
        (16, ActionKind::Up, 3_201, &[0x17][..], "third C up"),
    ]
    .into_iter()
    .map(
        |(source_action_index, kind, scheduled_us, scan_codes, reason)| KeyActionInput {
            source_action_index,
            kind,
            scheduled_us,
            scan_codes: smallvec::SmallVec::from_slice(scan_codes),
            reason: reason.into(),
        },
    )
    .collect::<Vec<_>>();
    compile_runtime_intents(&actions, &allowed).expect("valid oracle schedule")
}

fn one_key_repeat_schedule(chord_count: usize) -> RuntimeSchedule {
    let actions = (0..u32::try_from(chord_count).expect("test chord count"))
        .flat_map(|index| {
            let scheduled_us = u64::from(index) * 2;
            [
                KeyActionInput {
                    source_action_index: index * 2,
                    kind: ActionKind::Down,
                    scheduled_us,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "capacity".into(),
                },
                KeyActionInput {
                    source_action_index: index * 2 + 1,
                    kind: ActionKind::Up,
                    scheduled_us: scheduled_us + 1,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "capacity".into(),
                },
            ]
        })
        .collect::<Vec<_>>();
    compile_runtime_intents(&actions, &[0x15]).expect("valid one-key repeat schedule")
}

fn chord_oracle(schedule: &RuntimeSchedule, chords: &[DownChordHeaderTicks]) -> Vec<ChordOracle> {
    chords
        .iter()
        .map(|chord| ChordOracle {
            index: chord.chord_index,
            scheduled_us: chord.scheduled_us,
            scheduled_ticks: chord.scheduled_ticks.as_u64(),
            source_action_index: chord.source_action_index,
            down_mask: chord.down_mask,
            intent_start: chord.intent_start,
            intent_count: chord.intent_count,
            reason: schedule.reason_table[usize::from(chord.reason_id)].clone(),
        })
        .collect()
}

fn intent_oracle(schedule: &RuntimeSchedule, intents: &[DownIntentTicks]) -> Vec<IntentOracle> {
    intents
        .iter()
        .map(|intent| IntentOracle {
            source_action_index: intent.source_action_index,
            generation_id: intent.generation_id,
            key_slot: intent.key_slot,
            scan_code: intent.scan_code,
            reason: schedule.reason_table[usize::from(intent.reason_id)].clone(),
            paired_up_source_action_index: intent.paired_up_source_action_index,
            paired_up_offset_us: intent.paired_up_offset_us,
            paired_up_offset_ticks: intent.paired_up_offset_ticks.as_u64(),
        })
        .collect()
}

#[test]
fn canonical_generation_arena_arrays_and_qpc_rounding_match_the_reference() {
    let schedule = fixture();
    let frequency = NonZeroU64::new(FREQUENCY_HZ).expect("odd QPC frequency");
    let prepared = PreparedDispatchSchedule::prepare(&schedule, frequency, 101, 77, 0)
        .expect("prepared oracle schedule");
    let legacy = DispatchExecutor::new(
        &schedule,
        DurationTicks::from_raw(qpc_ceil(101).as_u64()),
        DurationTicks::from_raw(qpc_ceil(77).as_u64()),
        |us| Ok::<_, std::convert::Infallible>(qpc_ceil(us)),
    )
    .expect("legacy oracle schedule");

    let expected_chords = [
        ChordOracle {
            index: 0,
            scheduled_us: 1_001,
            scheduled_ticks: 3_584,
            source_action_index: 10,
            down_mask: 0b11,
            intent_start: 0,
            intent_count: 2,
            reason: "first chord".into(),
        },
        ChordOracle {
            index: 1,
            scheduled_us: 1_801,
            scheduled_ticks: 6_447,
            source_action_index: 13,
            down_mask: 0b01,
            intent_start: 2,
            intent_count: 1,
            reason: "second A".into(),
        },
        ChordOracle {
            index: 2,
            scheduled_us: 2_601,
            scheduled_ticks: 9_311,
            source_action_index: 15,
            down_mask: 0b100,
            intent_start: 3,
            intent_count: 1,
            reason: "third C".into(),
        },
    ];
    let expected_intents = [
        IntentOracle {
            source_action_index: 10,
            generation_id: 0,
            key_slot: 0,
            scan_code: 0x15,
            reason: "first chord".into(),
            paired_up_source_action_index: 11,
            paired_up_offset_us: 1_301,
            paired_up_offset_ticks: 4_657,
        },
        IntentOracle {
            source_action_index: 10,
            generation_id: 1,
            key_slot: 1,
            scan_code: 0x16,
            reason: "first chord".into(),
            paired_up_source_action_index: 12,
            paired_up_offset_us: 1_701,
            paired_up_offset_ticks: 6_089,
        },
        IntentOracle {
            source_action_index: 13,
            generation_id: 2,
            key_slot: 0,
            scan_code: 0x15,
            reason: "second A".into(),
            paired_up_source_action_index: 14,
            paired_up_offset_us: 2_501,
            paired_up_offset_ticks: 8_953,
        },
        IntentOracle {
            source_action_index: 15,
            generation_id: 3,
            key_slot: 2,
            scan_code: 0x17,
            reason: "third C".into(),
            paired_up_source_action_index: 16,
            paired_up_offset_us: 3_201,
            paired_up_offset_ticks: 11_459,
        },
    ];

    let prepared_chords = chord_oracle(&schedule, &prepared.down_chords);
    let prepared_intents = intent_oracle(&schedule, &prepared.down_intents);
    let legacy_chords = chord_oracle(&schedule, &legacy.down_chords);
    let legacy_intents = intent_oracle(&schedule, &legacy.down_intents);
    assert_eq!(prepared_chords, expected_chords);
    assert_eq!(prepared_intents, expected_intents);
    assert_eq!(legacy_chords, expected_chords);
    assert_eq!(legacy_intents, expected_intents);

    let memory = prepared.memory_metrics();
    assert_eq!(memory.chord_count, 3);
    assert_eq!(memory.chord_capacity, 3);
    assert_eq!(memory.intent_count, 4);
    assert_eq!(memory.intent_capacity, 4);
    assert_eq!(memory.unmatched_up_count, 0);
    assert_eq!(memory.unmatched_up_capacity, 0);
    assert_eq!(
        memory.plan_capacity_bytes,
        3 * std::mem::size_of::<DownChordHeaderTicks>()
            + 4 * std::mem::size_of::<DownIntentTicks>()
            + std::mem::size_of::<PreparedDispatchSchedule>()
    );
    if usize::BITS == 64 {
        assert_eq!(memory.plan_capacity_bytes, 568);
        assert_eq!(memory.overlapping_peak_capacity_bytes, 3_410);
    }
    assert_eq!(legacy.memory_metrics().chord_count, 3);
    assert_eq!(legacy.memory_metrics().intent_count, 4);
}

#[test]
fn empty_and_fresh_legacy_executors_keep_zeroed_arena_state() {
    let empty = compile_runtime_intents(&[], &[0x15]).expect("empty schedule");
    let frequency = NonZeroU64::new(FREQUENCY_HZ).expect("odd QPC frequency");
    let prepared = PreparedDispatchSchedule::prepare(&empty, frequency, 101, 77, 0)
        .expect("empty prepared schedule");
    assert!(prepared.down_chords.is_empty());
    assert!(prepared.down_intents.is_empty());
    let prepared_executor = DispatchExecutor::from_prepared_schedule(prepared);
    assert_eq!(prepared_executor.chord_cursor(), 0);
    assert_eq!(prepared_executor.active_mask(), 0);

    let legacy_first = DispatchExecutor::new(
        &empty,
        DurationTicks::from_raw(qpc_ceil(101).as_u64()),
        DurationTicks::from_raw(qpc_ceil(77).as_u64()),
        |us| Ok::<_, std::convert::Infallible>(qpc_ceil(us)),
    )
    .expect("first empty legacy executor");
    let legacy_second = DispatchExecutor::new(
        &empty,
        DurationTicks::from_raw(qpc_ceil(101).as_u64()),
        DurationTicks::from_raw(qpc_ceil(77).as_u64()),
        |us| Ok::<_, std::convert::Infallible>(qpc_ceil(us)),
    )
    .expect("fresh empty legacy executor");
    for executor in [&legacy_first, &legacy_second] {
        assert!(executor.down_chords.is_empty());
        assert!(executor.down_intents.is_empty());
        assert_eq!(executor.chord_cursor(), 0);
        assert_eq!(executor.active_mask(), 0);
        assert_eq!(executor.accounting().active, 0);
        assert_eq!(executor.accounting().scheduled, 0);
    }
    assert_eq!(
        legacy_first.memory_metrics(),
        legacy_second.memory_metrics()
    );
}

#[test]
fn prepared_retained_capacity_budget_keeps_exact_accept_and_reject_boundary() {
    let schedule = fixture();
    let frequency = NonZeroU64::new(FREQUENCY_HZ).expect("odd QPC frequency");
    let baseline =
        PreparedDispatchSchedule::prepare(&schedule, frequency, 101, 77, 0).expect("baseline plan");
    let schedule_memory = schedule
        .memory_metrics()
        .expect("schedule capacity metrics");
    let schedule_bytes = schedule_memory
        .heap_payload_bytes
        .checked_add(schedule_memory.struct_bytes)
        .expect("schedule capacity bytes");
    let combined_capacity = schedule_bytes
        .checked_add(baseline.memory_metrics().plan_capacity_bytes)
        .expect("overlapping capacity bytes");
    let exact_fit_caller = crate::model::MAX_RETAINED_DISPATCH_BYTES - combined_capacity;
    let exact_fit =
        PreparedDispatchSchedule::prepare(&schedule, frequency, 101, 77, exact_fit_caller)
            .expect("plan exactly at retained startup budget");
    assert_eq!(
        exact_fit.memory_metrics().overlapping_peak_capacity_bytes,
        crate::model::MAX_RETAINED_DISPATCH_BYTES
    );

    let over_budget =
        PreparedDispatchSchedule::prepare(&schedule, frequency, 101, 77, exact_fit_caller + 1)
            .expect_err("one byte over retained startup budget must fail");
    assert_eq!(
        over_budget,
        format!(
            "prepared playback capacity ({} bytes) exceeds the {} byte retained-dispatch budget",
            crate::model::MAX_RETAINED_DISPATCH_BYTES + 1,
            crate::model::MAX_RETAINED_DISPATCH_BYTES,
        )
    );
}

#[test]
fn prepared_schedule_validates_unmatched_up_timestamps() {
    let mut unmatched_up = compile_runtime_intents(
        &[KeyActionInput {
            source_action_index: 7,
            kind: ActionKind::Up,
            scheduled_us: 10,
            scan_codes: smallvec::smallvec![0x15],
            reason: "unmatched up".into(),
        }],
        &[0x15],
    )
    .expect("unmatched Up schedule");
    unmatched_up.unmatched_ups[0].scheduled_us = u64::MAX;
    let prepared_error = PreparedDispatchSchedule::prepare(
        &unmatched_up,
        NonZeroU64::new(FREQUENCY_HZ).expect("frequency"),
        101,
        77,
        0,
    )
    .expect_err("prepared unmatched-Up range must be checked");
    assert!(prepared_error.contains("supported timestamp range"));

    let schedule = fixture();
    let legacy_error = DispatchExecutor::new(
        &schedule,
        DurationTicks::from_raw(1),
        DurationTicks::from_raw(1),
        |microseconds| {
            if microseconds == 1_001 {
                Err("oracle conversion failure")
            } else {
                Ok(TimelineTicks::from_raw(microseconds))
            }
        },
    )
    .expect_err("legacy conversion error must retain caller text");
    assert_eq!(
        legacy_error,
        "scheduled timestamp conversion failed for chord at 1001us: oracle conversion failure"
    );

    let mut near_overflow = compile_runtime_intents(&[], &[0x15]).expect("empty schedule");
    near_overflow.schedule_end_offset_us = u64::MAX;
    let prepared_overflow = PreparedDispatchSchedule::prepare(
        &near_overflow,
        NonZeroU64::new(FREQUENCY_HZ).expect("frequency"),
        101,
        77,
        0,
    )
    .expect_err("prepared end conversion should detect overflow");
    assert!(prepared_overflow.contains("supported timestamp range"));
    let legacy_overflow = DispatchExecutor::new(
        &near_overflow,
        DurationTicks::from_raw(1),
        DurationTicks::from_raw(1),
        |microseconds| {
            if microseconds == u64::MAX {
                Err("near-overflow conversion")
            } else {
                Ok(TimelineTicks::from_raw(microseconds))
            }
        },
    )
    .expect_err("legacy end conversion should preserve adapter failure");
    assert_eq!(
        legacy_overflow,
        format!(
            "schedule end offset timestamp conversion failed at {}us: near-overflow conversion",
            u64::MAX
        )
    );
}

#[test]
fn prepared_builder_keeps_its_first_chord_checkpoint_boundary() {
    let schedule = one_key_repeat_schedule(5_000);
    assert_eq!(schedule.down_chords.len(), 5_000);

    let mut checkpoints = 0usize;
    let result = PreparedDispatchSchedule::prepare_with_cancellation(
        &schedule,
        NonZeroU64::new(1_000_000).expect("QPC frequency"),
        0,
        0,
        0,
        || {
            checkpoints += 1;
            if checkpoints == 7 {
                Err("cancelled at prepared arena materialization".into())
            } else {
                Ok(())
            }
        },
    );
    let error = match result {
        Ok(_) => panic!("cancellation at first materialization checkpoint"),
        Err(error) => error,
    };
    assert_eq!(error, "cancelled at prepared arena materialization");
    assert_eq!(checkpoints, 7);
}

#[test]
fn prepared_retained_capacity_matrix_matches_reviewed_head() {
    let frequency = NonZeroU64::new(1_000_000).expect("QPC frequency");
    for chord_count in [0usize, 1, 100, 5_000] {
        let schedule = one_key_repeat_schedule(chord_count);
        let prepared = PreparedDispatchSchedule::prepare(&schedule, frequency, 0, 0, 0)
            .expect("prepared capacity schedule");
        let memory = prepared.memory_metrics();
        assert_eq!(memory.chord_count, chord_count);
        assert_eq!(memory.chord_capacity, chord_count);
        assert_eq!(memory.intent_count, chord_count);
        assert_eq!(memory.intent_capacity, chord_count);
        assert_eq!(memory.unmatched_up_count, 0);
        assert_eq!(memory.unmatched_up_capacity, 0);
        let expected_plan_bytes = chord_count * std::mem::size_of::<DownChordHeaderTicks>()
            + chord_count * std::mem::size_of::<DownIntentTicks>()
            + std::mem::size_of::<PreparedDispatchSchedule>();
        let schedule_metrics = schedule.memory_metrics().expect("schedule memory");
        let schedule_bytes = schedule_metrics.heap_payload_bytes + schedule_metrics.struct_bytes;
        let expected_peak_bytes = schedule_bytes
            .checked_add(expected_plan_bytes)
            .expect("schedule and prepared plan capacity")
            .max(schedule.compile_memory.phase_peak_capacity_bytes);
        assert_eq!(memory.plan_capacity_bytes, expected_plan_bytes);
        assert_eq!(memory.overlapping_peak_capacity_bytes, expected_peak_bytes);
    }
}
