//! Scaling evidence for the successful coordinator commit.
//!
//! Each sample measures only the bounded successful commit.  Schedule
//! construction, coordinator allocation, packet preparation, and intent
//! copying happen before the timer so the result answers the specific
//! question: does a fixed packet get slower as the unrelated generation
//! ledger grows?

use serde_json::json;
use sky_dispatch_core::compile::compile_runtime_intents;
use sky_dispatch_core::coordinator::RuntimeDispatchCoordinator;
use sky_dispatch_core::model::{ActionKind, CompactIntent, KeyActionInput, MAX_KEYS};
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use smallvec::SmallVec;
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
        let coordinator = RuntimeDispatchCoordinator::try_new_ticks(
            schedule.clone(),
            0,
            DurationTicks::ZERO,
            |microseconds| Ok(TimelineTicks::from_raw(microseconds)),
        )
        .expect("coordinator construction");
        let mut coordinator = coordinator;
        let prepared = coordinator
            .prepare_current_authored_packet()
            .expect("current packet preparation")
            .expect("first packet");
        let packet = coordinator
            .schedule
            .view_packet_ticks(prepared.packet_index, prepared.effective_scheduled_ticks)
            .expect("first packet view");
        let up_intents = packet
            .up_intents
            .iter()
            .copied()
            .collect::<SmallVec<[CompactIntent; MAX_KEYS]>>();
        let down_intents = packet
            .down_intents
            .iter()
            .copied()
            .collect::<SmallVec<[CompactIntent; MAX_KEYS]>>();
        let down_source_action_index = packet.header.down_source_action_index;
        let started = Instant::now();
        coordinator
            .commit_prepared_packet_success_parts(
                prepared,
                &up_intents,
                &down_intents,
                down_source_action_index,
                TimelineTicks::ZERO,
                TimelineTicks::ZERO,
            )
            .expect("successful packet commit");
        black_box((coordinator.cursor, coordinator.active_mask));
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
            "benchmark": "coordinator_commit_scaling",
            "samples_per_case": samples,
            "cases": cases,
        }))
        .expect("serialize benchmark report")
    );
}
