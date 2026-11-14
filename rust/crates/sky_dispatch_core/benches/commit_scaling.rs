//! Scaling evidence for the successful production executor commit.
//!
//! Schedule compilation and executor construction happen before the timer.
//! The measured commit activates one fixed chord while the compiled schedule
//! grows, verifying the live executor commit remains bounded by the 15 slots.

use serde_json::json;
use sky_dispatch_core::compile::compile_runtime_intents;
use sky_dispatch_core::executor::{DispatchDecision, DispatchExecutor};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use std::env;
use std::hint::black_box;
use std::time::Instant;

const GENERATION_COUNTS: &[usize] = &[100, 1_000, 10_000];
const SCAN_CODES: &[u16] = &[
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
];
const REPRESENTATION_INNER_ITERATIONS: usize = 1_000;

fn sample_count() -> usize {
    env::var("COMMIT_SCALING_SAMPLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &usize| (8..=512).contains(value))
        .unwrap_or(64)
}

fn build_actions(generation_count: usize, polyphony: usize) -> Vec<KeyActionInput> {
    assert!(polyphony <= generation_count);
    let mut actions = Vec::with_capacity(generation_count.saturating_mul(2));
    let first_scans = SCAN_CODES[..polyphony].to_vec();
    actions.push(KeyActionInput {
        source_action_index: 0,
        kind: ActionKind::Down,
        scheduled_us: 0,
        scan_codes: first_scans.clone().into(),
        reason: "scaling-down".into(),
    });
    actions.push(KeyActionInput {
        source_action_index: 1,
        kind: ActionKind::Up,
        scheduled_us: 1,
        scan_codes: first_scans.into(),
        reason: "scaling-up".into(),
    });
    for generation in polyphony..generation_count {
        let source = (generation.saturating_mul(2)) as u32;
        let scheduled_us = (generation.saturating_mul(2)) as u64;
        actions.push(KeyActionInput {
            source_action_index: source,
            kind: ActionKind::Down,
            scheduled_us,
            scan_codes: vec![SCAN_CODES[0]].into(),
            reason: "scaling-down".into(),
        });
        actions.push(KeyActionInput {
            source_action_index: source.saturating_add(1),
            kind: ActionKind::Up,
            scheduled_us: scheduled_us.saturating_add(1),
            scan_codes: vec![SCAN_CODES[0]].into(),
            reason: "scaling-up".into(),
        });
    }
    actions
}

fn quantile(values: &mut [u128], numerator: usize, denominator: usize) -> u128 {
    values.sort_unstable();
    let index = ((values.len() - 1) * numerator / denominator).min(values.len() - 1);
    values[index]
}

fn measure(generation_count: usize, polyphony: usize, samples: usize) -> serde_json::Value {
    let actions = build_actions(generation_count, polyphony);
    let schedule = compile_runtime_intents(&actions, SCAN_CODES).expect("valid scaling schedule");
    let mut timings = Vec::with_capacity(samples);

    for _ in 0..samples {
        let mut executor = DispatchExecutor::new(
            &schedule,
            DurationTicks::ZERO,
            DurationTicks::ZERO,
            |microseconds| Ok::<_, String>(TimelineTicks::from_raw(microseconds)),
        )
        .expect("production executor construction");
        let down_mask = match executor
            .evaluate_at(QpcTicks::ZERO, QpcTicks::ZERO)
            .expect("first production dispatch decision")
        {
            DispatchDecision::Send {
                up_mask: 0,
                down_chord: Some(chord),
                ..
            } => chord.down_mask,
            decision => panic!("expected first chord Down, got {decision:?}"),
        };
        let started = Instant::now();
        executor
            .on_send_completed_at(0, down_mask, QpcTicks::ZERO)
            .expect("successful packet commit");
        black_box((executor.chord_cursor(), executor.active_mask()));
        timings.push(started.elapsed().as_nanos());
    }

    json!({
        "generations": generation_count,
        "current_packet_polyphony": polyphony,
        "samples": samples,
        "commit_ns": {
            "p50": quantile(&mut timings.clone(), 50, 100),
            "p95": quantile(&mut timings.clone(), 95, 100),
            "max": timings.iter().copied().max().unwrap_or_default(),
        },
    })
}

fn measure_scan_code_representation(samples: usize) -> serde_json::Value {
    let actions = build_actions(100, 15);
    let schedule =
        compile_runtime_intents(&actions, SCAN_CODES).expect("valid representation schedule");
    let executor = DispatchExecutor::new(
        &schedule,
        DurationTicks::ZERO,
        DurationTicks::ZERO,
        |microseconds| Ok::<_, String>(TimelineTicks::from_raw(microseconds)),
    )
    .expect("production executor construction");
    let chord = executor
        .current_chord()
        .expect("first production Down chord");

    let mut frozen_timings = Vec::with_capacity(samples);
    let mut lookup_timings = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        let mut checksum = 0u16;
        for _ in 0..REPRESENTATION_INNER_ITERATIONS {
            for down in chord.intents {
                checksum = checksum.wrapping_add(black_box(down.scan_code));
            }
        }
        black_box(checksum);
        frozen_timings.push(started.elapsed().as_nanos());

        let started = Instant::now();
        let mut checksum = 0u16;
        for _ in 0..REPRESENTATION_INNER_ITERATIONS {
            for down in chord.intents {
                let slot = black_box(down.key_slot);
                let scan_code = executor
                    .key_registry()
                    .scan_code_for(slot)
                    .expect("prepared slot belongs to registry");
                checksum = checksum.wrapping_add(black_box(scan_code));
            }
        }
        black_box(checksum);
        lookup_timings.push(started.elapsed().as_nanos());
    }

    json!({
        "samples": samples,
        "inner_iterations": REPRESENTATION_INNER_ITERATIONS,
        "polyphony": 15,
        "frozen_scan_code_ns": {
            "p50": quantile(&mut frozen_timings.clone(), 50, 100),
            "p95": quantile(&mut frozen_timings.clone(), 95, 100),
            "max": frozen_timings.iter().copied().max().unwrap_or_default(),
        },
        "registry_lookup_ns": {
            "p50": quantile(&mut lookup_timings.clone(), 50, 100),
            "p95": quantile(&mut lookup_timings.clone(), 95, 100),
            "max": lookup_timings.iter().copied().max().unwrap_or_default(),
        },
    })
}

fn main() {
    let samples = sample_count();
    let cases = GENERATION_COUNTS
        .iter()
        .flat_map(|&generation_count| {
            [1usize, 15usize]
                .into_iter()
                .map(move |polyphony| measure(generation_count, polyphony, samples))
        })
        .collect::<Vec<_>>();
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "benchmark": "dispatch_executor_commit_scaling",
            "scheduler_owner": "DispatchExecutor",
            "samples_per_case": samples,
            "cases": cases,
            "scan_code_representation": measure_scan_code_representation(samples),
        }))
        .expect("serialize benchmark report")
    );
}
