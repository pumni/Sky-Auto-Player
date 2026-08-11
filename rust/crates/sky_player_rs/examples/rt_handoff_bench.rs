//! Small RT handoff benchmark used for before/after comparison.
//!
//! This deliberately has no Criterion dependency.  The plan is built before
//! the real QPC wait, then the deterministic test emitter exercises admission,
//! packet construction, coordinator commit, and the fixed observation enqueue.
//! It is compiled only with `test-support`; production binaries do not contain
//! this harness.

#![cfg(feature = "test-support")]

use serde_json::json;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::QpcClock;
use sky_player_rs::engine::dispatch_primitives::{
    DispatchObservation, DispatchPath, DispatchStep, ProductionDispatchTestHarness,
};
use std::time::Instant;

const ITERATIONS: usize = 2_000;
const DUE_US: u64 = 10_000;

#[derive(Default)]
struct Samples {
    wake_to_send_start_us: Vec<u64>,
    dispatch_start_error_us: Vec<i64>,
    send_duration_us: Vec<u64>,
    completion_error_us: Vec<i64>,
    physical_dispatches: usize,
}

fn quantile<T: Copy + Ord>(values: &mut [T], numerator: usize, denominator: usize) -> Option<T> {
    if values.is_empty() || denominator == 0 {
        return None;
    }
    values.sort_unstable();
    let index = ((values.len() - 1) * numerator / denominator).min(values.len() - 1);
    values.get(index).copied()
}

fn add_observation(samples: &mut Samples, observation: DispatchObservation) {
    let qpc_clock = QpcClock::initialize().expect("QPC");
    let signed_ticks_to_us = |value: i64| {
        qpc_clock
            .duration_to_us(DurationTicks::from_raw(value.unsigned_abs()))
            .expect("timing conversion") as i64
            * value.signum()
    };
    match observation {
        DispatchObservation::Down(value) => {
            let wake_to_send_start_us = value.wake_qpc.and_then(|wake| {
                value
                    .sender_started_qpc
                    .checked_duration_since(wake)
                    .ok()
                    .and_then(|ticks| qpc_clock.duration_to_us(ticks).ok())
            }).unwrap_or_else(|| {
                panic!(
                    "missing Down wake sample: wake={:?}, sender_started={:?}, sender_completed={:?}",
                    value.trace.wake_ticks,
                    value.trace.sender_started_ticks,
                    value.trace.sender_completed_ticks
                )
            });
            samples.wake_to_send_start_us.push(wake_to_send_start_us);
            samples
                .dispatch_start_error_us
                .push(signed_ticks_to_us(value.trace.dispatch_start_error_ticks));
            samples.send_duration_us.push(
                qpc_clock
                    .duration_to_us(value.sender_duration_ticks)
                    .expect("duration"),
            );
            samples
                .completion_error_us
                .push(signed_ticks_to_us(value.trace.completion_error_ticks));
        }
        DispatchObservation::Up(value) => {
            let wake_to_send_start_us = value.wake_qpc.and_then(|wake| {
                value
                    .sender_started_qpc
                    .checked_duration_since(wake)
                    .ok()
                    .and_then(|ticks| qpc_clock.duration_to_us(ticks).ok())
            }).unwrap_or_else(|| {
                panic!(
                    "missing Up wake sample: wake={:?}, sender_started={:?}, sender_completed={:?}",
                    value.trace.wake_ticks,
                    value.trace.sender_started_ticks,
                    value.trace.sender_completed_ticks
                )
            });
            samples.wake_to_send_start_us.push(wake_to_send_start_us);
            samples
                .dispatch_start_error_us
                .push(signed_ticks_to_us(value.trace.dispatch_start_error_ticks));
            samples.send_duration_us.push(
                qpc_clock
                    .duration_to_us(value.sender_duration_ticks)
                    .expect("duration"),
            );
            samples
                .completion_error_us
                .push(signed_ticks_to_us(value.trace.completion_error_ticks));
        }
        DispatchObservation::Wait(wait) => {
            panic!("benchmark handoff queued an unexpected wait observation: {wait:?}")
        }
        DispatchObservation::StaleMetadata(value) => {
            panic!("benchmark handoff queued unexpected stale metadata: {value:?}")
        }
        DispatchObservation::BlockedUnfocused(value) => {
            panic!("benchmark handoff queued unexpected blocked observation: {value:?}")
        }
    }
}

fn plan_projected(
    harness: &mut ProductionDispatchTestHarness,
) -> sky_player_rs::engine::dispatch_primitives::NextDispatchPlan {
    harness.plan_current_dispatch_projected()
}

fn run_down(key_count: usize) -> Result<Samples, String> {
    let mut samples = Samples::default();
    for _ in 0..ITERATIONS {
        let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(key_count, DUE_US);
        let plan = plan_projected(&mut harness);
        let step = harness.wait_and_dispatch_current_plan(&plan)?;
        assert!(
            matches!(step, DispatchStep::Dispatched),
            "down step: {step:?}"
        );
        assert_eq!(
            harness.pending_observation_count(),
            1,
            "down dispatch did not enqueue one raw observation"
        );
        samples.physical_dispatches += 1;
        while let Some(observation) = harness.pop_observation() {
            add_observation(&mut samples, observation);
        }
    }
    Ok(samples)
}

fn run_up(key_count: usize) -> Result<Samples, String> {
    let mut samples = Samples::default();
    for _ in 0..ITERATIONS {
        let mut harness =
            ProductionDispatchTestHarness::new_uponly_release_chord_with_gap(key_count, DUE_US);
        while harness.pop_observation().is_some() {}
        assert_eq!(
            harness.current_authored_path(),
            Some(DispatchPath::UpOnly {
                up_count: key_count
            }),
            "authored benchmark setup did not leave the requested physical UpOnly packet"
        );
        let plan = plan_projected(&mut harness);
        let step = harness.wait_and_dispatch_current_plan(&plan)?;
        assert!(
            matches!(step, DispatchStep::Dispatched),
            "up step: {step:?}"
        );
        samples.physical_dispatches += 1;
        while let Some(observation) = harness.pop_observation() {
            add_observation(&mut samples, observation);
        }
    }
    Ok(samples)
}

fn run_mixed(event_count: usize) -> Result<Samples, String> {
    let mut samples = Samples::default();
    for _ in 0..ITERATIONS {
        let mut harness =
            ProductionDispatchTestHarness::new_mixed_events_with_gap(event_count, DUE_US);
        while harness.pop_observation().is_some() {}
        let plan = plan_projected(&mut harness);
        let step = harness.wait_and_dispatch_current_plan(&plan)?;
        assert!(
            matches!(step, DispatchStep::Dispatched),
            "mixed step: {step:?}"
        );
        samples.physical_dispatches += 1;
        while let Some(observation) = harness.pop_observation() {
            add_observation(&mut samples, observation);
        }
    }
    Ok(samples)
}

fn summarize(mut samples: Samples) -> serde_json::Value {
    assert_eq!(
        samples.wake_to_send_start_us.len(),
        samples.physical_dispatches,
        "every successful physical dispatch must have one raw wake-to-send sample"
    );
    assert_eq!(
        samples.wake_to_send_start_us.len(),
        ITERATIONS,
        "scenario did not produce the requested number of timing samples"
    );
    assert_eq!(
        samples.dispatch_start_error_us.len(),
        samples.physical_dispatches,
        "every physical dispatch must have one start-error sample"
    );
    json!({
        "controller": "dispatch_start_error",
        "deadline_wake_to_send_start_us": {
            "p50": quantile(&mut samples.wake_to_send_start_us, 50, 100),
            "p95": quantile(&mut samples.wake_to_send_start_us, 95, 100),
            "p99": quantile(&mut samples.wake_to_send_start_us, 99, 100),
            "max": samples.wake_to_send_start_us.iter().copied().max(),
            "samples": samples.wake_to_send_start_us.len(),
        },
        "dispatch_start_error_us": {
            "min": samples.dispatch_start_error_us.iter().copied().min(),
            "p50": quantile(&mut samples.dispatch_start_error_us, 50, 100),
            "p95": quantile(&mut samples.dispatch_start_error_us, 95, 100),
            "p99": quantile(&mut samples.dispatch_start_error_us, 99, 100),
            "p99_9": quantile(&mut samples.dispatch_start_error_us, 999, 1000),
            "max": samples.dispatch_start_error_us.iter().copied().max(),
            "samples": samples.dispatch_start_error_us.len(),
        },
        "completion_error_us_diagnostic": {
            "p01": quantile(&mut samples.completion_error_us, 1, 100),
            "p50": quantile(&mut samples.completion_error_us, 50, 100),
            "p95": quantile(&mut samples.completion_error_us, 95, 100),
            "p99": quantile(&mut samples.completion_error_us, 99, 100),
            "max_abs": samples.completion_error_us.iter().map(|value| value.unsigned_abs()).max(),
            "samples": samples.completion_error_us.len(),
        },
        "send_duration_us": {
            "min": samples.send_duration_us.iter().copied().min(),
            "p50": quantile(&mut samples.send_duration_us, 50, 100),
            "p95": quantile(&mut samples.send_duration_us, 95, 100),
            "p99": quantile(&mut samples.send_duration_us, 99, 100),
            "p99_9": quantile(&mut samples.send_duration_us, 999, 1000),
            "max": samples.send_duration_us.iter().copied().max(),
            "samples": samples.send_duration_us.len(),
        },
        "physical_dispatches": samples.physical_dispatches,
    })
}

fn main() {
    let started = Instant::now();
    let mut scenarios = serde_json::Map::new();
    {
        scenarios.insert(
            "down_only_1".to_string(),
            summarize(run_down(1).unwrap_or_else(|error| panic!("{error}"))),
        );
        scenarios.insert(
            "down_only_5".to_string(),
            summarize(run_down(5).unwrap_or_else(|error| panic!("{error}"))),
        );
        scenarios.insert(
            "down_only_15".to_string(),
            summarize(run_down(15).unwrap_or_else(|error| panic!("{error}"))),
        );
        scenarios.insert(
            "up_only_1".to_string(),
            summarize(run_up(1).unwrap_or_else(|error| panic!("{error}"))),
        );
        for key_count in [5, 15] {
            scenarios.insert(
                format!("up_only_{key_count}"),
                summarize(run_up(key_count).unwrap_or_else(|error| panic!("{error}"))),
            );
        }
        for event_count in [2, 10, 30] {
            scenarios.insert(
                format!("mixed_{event_count}"),
                summarize(run_mixed(event_count).unwrap_or_else(|error| panic!("{error}"))),
            );
        }
    }
    let output = serde_json::to_string_pretty(&json!({
        "benchmark": "rt_handoff_bench",
        "iterations": ITERATIONS,
        "deadline_us": DUE_US,
        "transport": "deterministic_mock",
        "scenarios": scenarios,
        "elapsed_ms": started.elapsed().as_millis(),
    }))
    .expect("serialize benchmark output");
    if let Some(path) = std::env::args_os().nth(1) {
        std::fs::write(path, &output).expect("write benchmark output");
    }
    println!("{output}");
}
