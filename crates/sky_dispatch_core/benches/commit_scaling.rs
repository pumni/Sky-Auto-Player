//! Scaling evidence for the successful production executor commit.
//!
//! Plan building and executor construction happen before the timer.
//! The measured commit activates one fixed chord while the onset plan
//! grows, verifying the live executor commit remains bounded by the 15 slots.

use serde_json::json;
use sky_dispatch_core::executor::{DispatchDecision, DispatchExecutor};
use sky_dispatch_core::model::OnsetChordInput;
use sky_dispatch_core::plan::build_onset_plan;
use sky_dispatch_core::time::QpcTicks;
use std::env;
use std::hint::black_box;
use std::time::Instant;

const GENERATION_COUNTS: &[usize] = &[100, 1_000, 10_000];
const SCAN_CODES: &[u16] = &[
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
];

fn sample_count() -> usize {
    env::var("COMMIT_SCALING_SAMPLES")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &usize| (8..=512).contains(value))
        .unwrap_or(64)
}

fn build_chords(generation_count: usize, polyphony: usize) -> Vec<OnsetChordInput> {
    assert!(polyphony <= generation_count);
    let mut chords = Vec::with_capacity(generation_count);
    chords.push(OnsetChordInput {
        source_action_index: 0,
        scheduled_us: 0,
        scan_codes: SCAN_CODES[..polyphony].iter().copied().collect(),
    });
    for (idx, generation) in (polyphony..generation_count).enumerate() {
        chords.push(OnsetChordInput {
            source_action_index: (idx + 1) as u32,
            scheduled_us: (generation.saturating_mul(2)) as u64,
            scan_codes: vec![SCAN_CODES[0]].into(),
        });
    }
    chords
}

fn quantile(values: &mut [u128], numerator: usize, denominator: usize) -> u128 {
    values.sort_unstable();
    let index = ((values.len() - 1) * numerator / denominator).min(values.len() - 1);
    values[index]
}

fn measure(generation_count: usize, polyphony: usize, samples: usize) -> serde_json::Value {
    let chords = build_chords(generation_count, polyphony);
    let plan = build_onset_plan(&chords, SCAN_CODES, 1).expect("valid scaling plan");
    let mut timings = Vec::with_capacity(samples);

    let qpc_1mhz = std::num::NonZeroU64::new(1_000_000).expect("1 MHz");
    for _ in 0..samples {
        let prepared =
            sky_dispatch_core::executor::PreparedDispatchSchedule::prepare(&plan, qpc_1mhz, 0)
                .expect("production prepared schedule");
        let mut executor = DispatchExecutor::from_prepared_schedule(prepared);
        let down_mask = match executor
            .evaluate_at(QpcTicks::ZERO, QpcTicks::ZERO)
            .expect("first production dispatch decision")
        {
            DispatchDecision::Send {
                up_mask: 0,
                down_chord: Some(chord),
                ..
            } => chord.key_mask,
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
        }))
        .expect("serialize benchmark report")
    );
}
