//! Small RT handoff benchmark used for before/after comparison.
//!
//! This deliberately has no Criterion dependency.  The plan is built before
//! the real QPC wait, then the deterministic test emitter exercises admission,
//! packet construction, coordinator commit, and the fixed observation enqueue.
//! It is compiled only with `test-support`; production binaries do not contain
//! this harness.

#![cfg(feature = "test-support")]

use serde_json::json;
use sky_dispatch_core::estimator::LatencyClass;
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::wait::{HybridWaiter, WaitOutcome};
use sky_player_rs::engine::dispatch_primitives::{
    DispatchObservation, DispatchStep, ProductionDispatchTestHarness,
};
use std::time::Instant;

const ITERATIONS: usize = 32;
const WAIT_US: u64 = 250;
const SPIN_US: u64 = 150;
const DUE_US: u64 = 10_000;

#[derive(Default)]
struct Samples {
    wake_to_send_start_us: Vec<u64>,
    sendinput_duration_us: Vec<u64>,
    completion_error_us: Vec<i64>,
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
    match observation {
        DispatchObservation::Down(value) => {
            if let Some(wake_to_send_start_us) = value.wake_to_send_start_us {
                samples.wake_to_send_start_us.push(wake_to_send_start_us);
            }
            samples.sendinput_duration_us.push(value.sender_duration_us);
            samples.completion_error_us.push(value.completion_error_us);
        }
        DispatchObservation::Up(value) => {
            if let Some(wake_to_send_start_us) = value.wake_to_send_start_us {
                samples.wake_to_send_start_us.push(wake_to_send_start_us);
            }
            samples.sendinput_duration_us.push(value.sender_duration_us);
            samples
                .completion_error_us
                .push(value.up_completion_error_us);
        }
        DispatchObservation::Wait(_) => {}
    }
}

fn wait_for_deadline(
    clock: QpcClock,
    waiter: &HybridWaiter,
) -> Option<sky_dispatch_win32::wait::WaitResult> {
    let interrupt = OwnedEvent::new_auto_reset()?;
    let now = clock.now().ok()?;
    let wait_ticks = clock.duration_from_us(WAIT_US).ok()?;
    let spin_ticks = clock.duration_from_us(SPIN_US).ok()?;
    let target = now.checked_add_duration(wait_ticks).ok()?;
    let result = waiter.wait_until_ticks_with_metrics_typed(clock, target, spin_ticks, &interrupt);
    (result.outcome == WaitOutcome::Deadline).then_some(result)
}

fn qpc_delta_us(
    clock: QpcClock,
    start: sky_dispatch_win32::clock::QpcTicks,
    end: sky_dispatch_win32::clock::QpcTicks,
) -> Option<u64> {
    let ticks = end.checked_duration_since(start).ok()?;
    clock.duration_to_us(ticks).ok()
}

fn run_down(key_count: usize, latency_class: LatencyClass) -> Samples {
    let mut samples = Samples::default();
    let clock = QpcClock::initialize().expect("QPC clock");
    let waiter = HybridWaiter::new();
    for _ in 0..ITERATIONS {
        let mut harness = ProductionDispatchTestHarness::new_down_chord(key_count);
        let plan = harness.plan_current_dispatch_class(latency_class);
        let Some(wait_result) = wait_for_deadline(clock, &waiter) else {
            continue;
        };
        harness.advance_playback_time_us(DUE_US);
        let send_start = clock.now().expect("send-start QPC");
        let step = harness.dispatch_authored_with_plan(&plan);
        assert!(
            matches!(step, DispatchStep::Dispatched),
            "down step: {step:?}"
        );
        if let Some(wake_qpc) = wait_result.wake_qpc
            && let Some(delta) = qpc_delta_us(clock, wake_qpc, send_start)
        {
            samples.wake_to_send_start_us.push(delta);
        }
        while let Some(observation) = harness.pop_observation() {
            add_observation(&mut samples, observation);
        }
    }
    samples
}

fn run_up(latency_class: LatencyClass) -> Samples {
    let mut samples = Samples::default();
    let clock = QpcClock::initialize().expect("QPC clock");
    let waiter = HybridWaiter::new();
    for _ in 0..ITERATIONS {
        let mut harness = ProductionDispatchTestHarness::new_uponly_release_with_gap(DUE_US);
        while harness.pop_observation().is_some() {}
        let plan = harness.plan_current_dispatch_class(latency_class);
        let Some(wait_result) = wait_for_deadline(clock, &waiter) else {
            continue;
        };
        harness.advance_playback_time_us(DUE_US);
        let send_start = clock.now().expect("send-start QPC");
        let step = harness.dispatch_authored_with_plan(&plan);
        assert!(
            matches!(step, DispatchStep::Dispatched),
            "up step: {step:?}"
        );
        if let Some(wake_qpc) = wait_result.wake_qpc
            && let Some(delta) = qpc_delta_us(clock, wake_qpc, send_start)
        {
            samples.wake_to_send_start_us.push(delta);
        }
        while let Some(observation) = harness.pop_observation() {
            add_observation(&mut samples, observation);
        }
    }
    samples
}

fn run_mixed(event_count: usize, latency_class: LatencyClass) -> Samples {
    let mut samples = Samples::default();
    let clock = QpcClock::initialize().expect("QPC clock");
    let waiter = HybridWaiter::new();
    for _ in 0..ITERATIONS {
        let mut harness = ProductionDispatchTestHarness::new_mixed_events(event_count);
        while harness.pop_observation().is_some() {}
        let plan = harness.plan_current_dispatch_class(latency_class);
        let Some(wait_result) = wait_for_deadline(clock, &waiter) else {
            continue;
        };
        harness.advance_playback_time_us(DUE_US);
        let send_start = clock.now().expect("send-start QPC");
        let step = harness.dispatch_authored_with_plan(&plan);
        assert!(
            matches!(step, DispatchStep::Dispatched),
            "mixed step: {step:?}"
        );
        if let Some(wake_qpc) = wait_result.wake_qpc
            && let Some(delta) = qpc_delta_us(clock, wake_qpc, send_start)
        {
            samples.wake_to_send_start_us.push(delta);
        }
        while let Some(observation) = harness.pop_observation() {
            add_observation(&mut samples, observation);
        }
    }
    samples
}

fn summarize(mut samples: Samples) -> serde_json::Value {
    json!({
        "deadline_wake_to_send_start_us": {
            "p50": quantile(&mut samples.wake_to_send_start_us, 50, 100),
            "p95": quantile(&mut samples.wake_to_send_start_us, 95, 100),
            "p99": quantile(&mut samples.wake_to_send_start_us, 99, 100),
            "max": samples.wake_to_send_start_us.iter().copied().max(),
            "samples": samples.wake_to_send_start_us.len(),
        },
        "completion_error_us": {
            "p50": quantile(&mut samples.completion_error_us, 50, 100),
            "p95": quantile(&mut samples.completion_error_us, 95, 100),
            "p99": quantile(&mut samples.completion_error_us, 99, 100),
            "max_abs": samples.completion_error_us.iter().map(|value| value.unsigned_abs()).max(),
            "samples": samples.completion_error_us.len(),
        },
        "sendinput_duration_us": {
            "p50": quantile(&mut samples.sendinput_duration_us, 50, 100),
            "p95": quantile(&mut samples.sendinput_duration_us, 95, 100),
            "p99": quantile(&mut samples.sendinput_duration_us, 99, 100),
            "max": samples.sendinput_duration_us.iter().copied().max(),
            "samples": samples.sendinput_duration_us.len(),
        },
    })
}

fn main() {
    let started = Instant::now();
    let mut scenarios = serde_json::Map::new();
    for latency_class in [LatencyClass::Hot, LatencyClass::Cold] {
        let class_name = match latency_class {
            LatencyClass::Hot => "hot",
            LatencyClass::Cold => "cold",
        };
        scenarios.insert(
            format!("down_only_1_{class_name}"),
            summarize(run_down(1, latency_class)),
        );
        scenarios.insert(
            format!("down_only_5_{class_name}"),
            summarize(run_down(5, latency_class)),
        );
        scenarios.insert(
            format!("down_only_15_{class_name}"),
            summarize(run_down(15, latency_class)),
        );
        scenarios.insert(
            format!("up_only_{class_name}"),
            summarize(run_up(latency_class)),
        );
        for event_count in [2, 10, 30] {
            scenarios.insert(
                format!("mixed_{event_count}_{class_name}"),
                summarize(run_mixed(event_count, latency_class)),
            );
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "benchmark": "rt_handoff_bench",
            "iterations": ITERATIONS,
            "wait_us": WAIT_US,
            "spin_us": SPIN_US,
            "transport": "deterministic_mock",
            "scenarios": scenarios,
            "elapsed_ms": started.elapsed().as_millis(),
        }))
        .expect("serialize benchmark output")
    );
}
