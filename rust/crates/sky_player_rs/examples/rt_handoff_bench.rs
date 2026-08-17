//! Small RT handoff benchmark used for before/after comparison.
//!
//! This deliberately has no Criterion dependency.  The plan is built before
//! the real QPC wait, then the deterministic test emitter exercises admission,
//! packet construction, coordinator commit, and the fixed observation enqueue.
//! It is compiled only with `test-support`; production binaries do not contain
//! this harness.

#![cfg(feature = "test-support")]

use serde_json::json;
use sky_dispatch_core::time::TimelineTicks;
use sky_dispatch_win32::clock::{QpcClock, qpc_frequency_checked};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::wait::{HybridWaiter, WaitResult, WakeErrorStats};
use sky_player_rs::engine::dispatch_primitives::{
    DispatchObservation, DispatchPath, DispatchStep, OBSERVATION_QUEUE_CAPACITY,
    PendingObservationQueue, ProductionDispatchTestHarness,
};
use std::hint::black_box;
use std::time::Instant;

const DEFAULT_ITERATIONS: usize = 2_000;
const DUE_US: u64 = 10_000;
const WAKE_PROBE_SAMPLES: usize = 32;

fn due_us() -> u64 {
    std::env::var("RT_HANDOFF_BENCH_DUE_US")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &u64| (1..=60_000).contains(value))
        .unwrap_or(DUE_US)
}

fn iterations() -> usize {
    std::env::var("RT_HANDOFF_BENCH_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &usize| (1..=100_000).contains(value))
        .unwrap_or(DEFAULT_ITERATIONS)
}

fn rust_version() -> String {
    std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unavailable".to_string())
}

#[derive(Clone, Copy)]
struct WaitMode {
    name: &'static str,
    waitable_timer_enabled: bool,
    event_wait_enabled: bool,
    adaptive_spin_enabled: bool,
    effective_spin_threshold_us: u64,
    startup_wake_error: WakeErrorStats,
}

#[derive(Default)]
struct Samples {
    wake_to_admission_us: Vec<u64>,
    final_spin_us: Vec<u64>,
    dispatch_start_error_us: Vec<i64>,
    pre_call_to_completion_us: Vec<u64>,
    target_to_completion_us: Vec<i64>,
    completion_error_us: Vec<i64>,
    physical_dispatches: usize,
    early_dispatch_count: usize,
}

fn quantile<T: Copy + Ord>(values: &mut [T], numerator: usize, denominator: usize) -> Option<T> {
    if values.is_empty() || denominator == 0 {
        return None;
    }
    values.sort_unstable();
    let index = ((values.len() - 1) * numerator / denominator).min(values.len() - 1);
    values.get(index).copied()
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn observer_ab_iterations() -> usize {
    iterations().max(10_000)
}

fn representative_observation() -> DispatchObservation {
    let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 0);
    let plan = harness.plan_current_dispatch_projected();
    let step = harness.dispatch_authored_with_plan(&plan);
    assert!(
        matches!(step, DispatchStep::Dispatched),
        "representative observation dispatch failed: {step:?}"
    );
    harness
        .pop_observation()
        .expect("representative dispatch must enqueue one observation")
}

fn nanos_summary(mut values: Vec<u64>) -> serde_json::Value {
    json!({
        "p50": quantile(&mut values, 50, 100),
        "p95": quantile(&mut values, 95, 100),
        "p99": quantile(&mut values, 99, 100),
        "p99_9": quantile(&mut values, 999, 1000),
        "max": values.iter().copied().max(),
        "samples": values.len(),
    })
}

/// Paired producer-only A/B measurement. It deliberately avoids a production
/// bypass flag: the baseline does no queue work, the available queue measures
/// the healthy push+len path, and the saturated queue models a stalled
/// consumer. Observation construction/copying and queue draining are outside
/// the timed regions.
fn observation_enqueue_ab() -> serde_json::Value {
    let template = representative_observation();
    let available = PendingObservationQueue::default();
    let saturated = PendingObservationQueue::default();
    let mut available_dropped = 0;
    let mut available_high_watermark = 0;
    let mut saturated_dropped = 0;
    let mut saturated_high_watermark = 0;
    for _ in 0..OBSERVATION_QUEUE_CAPACITY {
        saturated.push(
            template,
            &mut saturated_dropped,
            &mut saturated_high_watermark,
        );
    }

    let sample_count = observer_ab_iterations();
    let mut bypass_ns = Vec::with_capacity(sample_count);
    let mut available_ns = Vec::with_capacity(sample_count);
    let mut saturated_ns = Vec::with_capacity(sample_count);
    for _ in 0..sample_count {
        let bypass_observation = template;
        let started = Instant::now();
        black_box(&bypass_observation);
        bypass_ns.push(elapsed_ns(started));

        let available_observation = template;
        let started = Instant::now();
        available.push(
            available_observation,
            &mut available_dropped,
            &mut available_high_watermark,
        );
        available_ns.push(elapsed_ns(started));
        black_box(available.pop_front()).expect("available queue sample must be present");

        let saturated_observation = template;
        let started = Instant::now();
        saturated.push(
            saturated_observation,
            &mut saturated_dropped,
            &mut saturated_high_watermark,
        );
        saturated_ns.push(elapsed_ns(started));
    }

    assert_eq!(available_dropped, 0);
    assert_eq!(saturated_dropped, sample_count as u64);
    // The precision producer intentionally does not call ArrayQueue::len()
    // to maintain a watermark.  Keep the variables in the report for schema
    // compatibility, but assert the Round 2 no-len contract instead.
    assert_eq!(available_high_watermark, 0);
    assert_eq!(saturated_high_watermark, 0);
    json!({
        "scope": "producer primitive only; observation construction, copying, and consumer drain excluded",
        "clock": "std::time::Instant",
        "high_watermark_tracking": "disabled_in_precision_path",
        "iterations": sample_count,
        "baseline_bypass_ns": nanos_summary(bypass_ns),
        "available_queue_push_ns": nanos_summary(available_ns),
        "saturated_queue_push_ns": nanos_summary(saturated_ns),
        "available_queue_dropped": available_dropped,
        "available_queue_high_watermark": available_high_watermark,
        "saturated_queue_dropped": saturated_dropped,
        "saturated_queue_high_watermark": saturated_high_watermark,
    })
}

fn signed_qpc_us(
    clock: QpcClock,
    end: sky_dispatch_win32::clock::QpcTicks,
    start: sky_dispatch_win32::clock::QpcTicks,
) -> i64 {
    let (negative, ticks) = if end >= start {
        (
            false,
            end.checked_duration_since(start).expect("QPC ordering"),
        )
    } else {
        (
            true,
            start.checked_duration_since(end).expect("QPC ordering"),
        )
    };
    let value = clock.duration_to_us(ticks).expect("QPC conversion") as i64;
    if negative { -value } else { value }
}

fn signed_timeline_us(clock: QpcClock, end: TimelineTicks, start: TimelineTicks) -> i64 {
    let (negative, ticks) = if end >= start {
        (
            false,
            end.checked_duration_since(start)
                .expect("timeline ordering"),
        )
    } else {
        (
            true,
            start
                .checked_duration_since(end)
                .expect("timeline ordering"),
        )
    };
    let value = clock.duration_to_us(ticks).expect("timeline conversion") as i64;
    if negative { -value } else { value }
}

fn add_observation(samples: &mut Samples, observation: DispatchObservation, wait: WaitResult) {
    let qpc_clock = QpcClock::initialize().expect("QPC");
    match observation {
        DispatchObservation::Down(value) => {
            let wake_to_proof_us = value.wake_qpc.and_then(|wake| {
                value
                    .final_proof_qpc
                    .checked_duration_since(wake)
                    .ok()
                    .and_then(|ticks| qpc_clock.duration_to_us(ticks).ok())
            }).unwrap_or_else(|| {
                panic!(
                    "missing Down wake sample: wake={:?}, final_proof={:?}, sendinput_completion={:?}",
                    value.wake_qpc,
                    value.final_proof_qpc,
                    value.sendinput_completion_qpc
                )
            });
            samples.wake_to_admission_us.push(wake_to_proof_us);
            samples.final_spin_us.push(
                qpc_clock
                    .duration_to_us(wait.spin_ticks)
                    .expect("spin duration"),
            );
            samples.target_to_completion_us.push(signed_qpc_us(
                qpc_clock,
                value.sendinput_completion_qpc,
                value.physical_target_qpc,
            ));
            if value.pre_call_qpc < value.physical_target_qpc {
                samples.early_dispatch_count += 1;
            }
            samples.dispatch_start_error_us.push(signed_qpc_us(
                qpc_clock,
                value.pre_call_qpc,
                value.physical_target_qpc,
            ));
            samples.pre_call_to_completion_us.push(
                qpc_clock
                    .duration_to_us(
                        value
                            .sendinput_completion_qpc
                            .checked_duration_since(value.pre_call_qpc)
                            .expect("QPC ordering"),
                    )
                    .expect("duration"),
            );
            let completed_effective_ticks = value
                .sendinput_completion_qpc
                .checked_duration_since(value.epoch_qpc)
                .map(|ticks| TimelineTicks::from_raw(ticks.as_u64()))
                .unwrap_or(TimelineTicks::ZERO);
            samples.completion_error_us.push(signed_timeline_us(
                qpc_clock,
                completed_effective_ticks,
                value.trace.effective_deadline_ticks,
            ));
        }
        DispatchObservation::Up(value) => {
            let wake_to_proof_us = value.wake_qpc.and_then(|wake| {
                value
                    .final_proof_qpc
                    .checked_duration_since(wake)
                    .ok()
                    .and_then(|ticks| qpc_clock.duration_to_us(ticks).ok())
            }).unwrap_or_else(|| {
                panic!(
                    "missing Up wake sample: wake={:?}, final_proof={:?}, sendinput_completion={:?}",
                    value.wake_qpc,
                    value.final_proof_qpc,
                    value.sendinput_completion_qpc
                )
            });
            samples.wake_to_admission_us.push(wake_to_proof_us);
            samples.final_spin_us.push(
                qpc_clock
                    .duration_to_us(wait.spin_ticks)
                    .expect("spin duration"),
            );
            samples.target_to_completion_us.push(signed_qpc_us(
                qpc_clock,
                value.sendinput_completion_qpc,
                value.physical_target_qpc,
            ));
            if value.pre_call_qpc < value.physical_target_qpc {
                samples.early_dispatch_count += 1;
            }
            samples.dispatch_start_error_us.push(signed_qpc_us(
                qpc_clock,
                value.pre_call_qpc,
                value.physical_target_qpc,
            ));
            samples.pre_call_to_completion_us.push(
                qpc_clock
                    .duration_to_us(value.pre_call_to_completion_ticks)
                    .expect("duration"),
            );
            samples.completion_error_us.push(signed_timeline_us(
                qpc_clock,
                value.completed_effective_ticks,
                value.trace.effective_deadline_ticks,
            ));
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

fn run_down(key_count: usize, mode: WaitMode) -> Result<Samples, String> {
    let mut samples = Samples::default();
    for _ in 0..iterations() {
        let mut harness =
            ProductionDispatchTestHarness::new_down_chord_with_gap(key_count, due_us());
        harness.configure_wait_policy(
            mode.waitable_timer_enabled,
            mode.event_wait_enabled,
            mode.effective_spin_threshold_us,
        )?;
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
        let wait = harness.last_wait_result().expect("benchmark wait result");
        while let Some(observation) = harness.pop_observation() {
            add_observation(&mut samples, observation, wait);
        }
    }
    Ok(samples)
}

fn run_up(key_count: usize, mode: WaitMode) -> Result<Samples, String> {
    let mut samples = Samples::default();
    for _ in 0..iterations() {
        let mut harness =
            ProductionDispatchTestHarness::new_uponly_release_chord_with_gap(key_count, due_us());
        harness.configure_wait_policy(
            mode.waitable_timer_enabled,
            mode.event_wait_enabled,
            mode.effective_spin_threshold_us,
        )?;
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
        let wait = harness.last_wait_result().expect("benchmark wait result");
        while let Some(observation) = harness.pop_observation() {
            add_observation(&mut samples, observation, wait);
        }
    }
    Ok(samples)
}

fn run_mixed(event_count: usize, mode: WaitMode) -> Result<Samples, String> {
    let mut samples = Samples::default();
    for _ in 0..iterations() {
        let mut harness =
            ProductionDispatchTestHarness::new_mixed_events_with_gap(event_count, due_us());
        harness.configure_wait_policy(
            mode.waitable_timer_enabled,
            mode.event_wait_enabled,
            mode.effective_spin_threshold_us,
        )?;
        while harness.pop_observation().is_some() {}
        let plan = plan_projected(&mut harness);
        let step = harness.wait_and_dispatch_current_plan(&plan)?;
        assert!(
            matches!(step, DispatchStep::Dispatched),
            "mixed step: {step:?}"
        );
        samples.physical_dispatches += 1;
        let wait = harness.last_wait_result().expect("benchmark wait result");
        while let Some(observation) = harness.pop_observation() {
            add_observation(&mut samples, observation, wait);
        }
    }
    Ok(samples)
}

fn summarize(mut samples: Samples) -> serde_json::Value {
    assert_eq!(
        samples.wake_to_admission_us.len(),
        samples.physical_dispatches,
        "every successful physical dispatch must have one raw wake-to-send sample"
    );
    assert_eq!(
        samples.wake_to_admission_us.len(),
        iterations(),
        "scenario did not produce the requested number of timing samples"
    );
    assert_eq!(
        samples.dispatch_start_error_us.len(),
        samples.physical_dispatches,
        "every physical dispatch must have one start-error sample"
    );
    json!({
        "controller": "dispatch_start_error",
            "wake_to_final_proof_us": {
            "p50": quantile(&mut samples.wake_to_admission_us, 50, 100),
            "p95": quantile(&mut samples.wake_to_admission_us, 95, 100),
            "p99": quantile(&mut samples.wake_to_admission_us, 99, 100),
            "p99_9": quantile(&mut samples.wake_to_admission_us, 999, 1000),
            "max": samples.wake_to_admission_us.iter().copied().max(),
            "samples": samples.wake_to_admission_us.len(),
        },
        "final_spin_us": {
            "p50": quantile(&mut samples.final_spin_us, 50, 100),
            "p95": quantile(&mut samples.final_spin_us, 95, 100),
            "p99": quantile(&mut samples.final_spin_us, 99, 100),
            "p99_9": quantile(&mut samples.final_spin_us, 999, 1000),
            "max": samples.final_spin_us.iter().copied().max(),
            "samples": samples.final_spin_us.len(),
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
        "pre_call_to_completion_us": {
            "min": samples.pre_call_to_completion_us.iter().copied().min(),
            "p50": quantile(&mut samples.pre_call_to_completion_us, 50, 100),
            "p95": quantile(&mut samples.pre_call_to_completion_us, 95, 100),
            "p99": quantile(&mut samples.pre_call_to_completion_us, 99, 100),
            "p99_9": quantile(&mut samples.pre_call_to_completion_us, 999, 1000),
            "max": samples.pre_call_to_completion_us.iter().copied().max(),
            "samples": samples.pre_call_to_completion_us.len(),
        },
        "target_to_completion_us": {
            "p50": quantile(&mut samples.target_to_completion_us, 50, 100),
            "p95": quantile(&mut samples.target_to_completion_us, 95, 100),
            "p99": quantile(&mut samples.target_to_completion_us, 99, 100),
            "p99_9": quantile(&mut samples.target_to_completion_us, 999, 1000),
            "max": samples.target_to_completion_us.iter().copied().max(),
            "samples": samples.target_to_completion_us.len(),
        },
        "physical_dispatches": samples.physical_dispatches,
        "early_dispatch_count": samples.early_dispatch_count,
        "observation_queue": "bounded_nonblocking_on",
    })
}

fn build_wait_mode(
    name: &'static str,
    waitable_timer_enabled: bool,
    event_wait_enabled: bool,
    adaptive_spin_enabled: bool,
) -> WaitMode {
    let qpc_clock = QpcClock::initialize().expect("QPC");
    let waiter = HybridWaiter::with_options(waitable_timer_enabled, event_wait_enabled);
    let interrupt = OwnedEvent::new_auto_reset().expect("benchmark interrupt event");
    let startup_wake_error = waiter
        .probe_wake_error_stats(qpc_clock, &interrupt, WAKE_PROBE_SAMPLES)
        .unwrap_or_else(|| panic!("{name}: startup wake probe failed; refusing mock fallback"));
    let effective_spin_threshold_us = if adaptive_spin_enabled {
        sky_player_rs::engine::dispatch_primitives::legacy_adaptive_spin_threshold_us(
            startup_wake_error.p95_us,
        )
    } else if event_wait_enabled {
        sky_player_rs::engine::dispatch_primitives::LEGACY_ADAPTIVE_SPIN_FLOOR_US
    } else {
        0
    };
    WaitMode {
        name,
        waitable_timer_enabled,
        event_wait_enabled,
        adaptive_spin_enabled,
        effective_spin_threshold_us,
        startup_wake_error,
    }
}

fn build_fixed_wait_mode(name: &'static str, spin_threshold_us: u64) -> WaitMode {
    let qpc_clock = QpcClock::initialize().expect("QPC");
    let waiter = HybridWaiter::with_options(true, true);
    let interrupt = OwnedEvent::new_auto_reset().expect("benchmark interrupt event");
    let startup_wake_error = waiter
        .probe_wake_error_stats(qpc_clock, &interrupt, WAKE_PROBE_SAMPLES)
        .unwrap_or_else(|| panic!("{name}: startup wake probe failed; refusing mock fallback"));
    WaitMode {
        name,
        waitable_timer_enabled: true,
        event_wait_enabled: true,
        adaptive_spin_enabled: false,
        effective_spin_threshold_us: spin_threshold_us,
        startup_wake_error,
    }
}

fn wake_error_json(stats: WakeErrorStats) -> serde_json::Value {
    json!({
        "p50_us": stats.p50_us,
        "p95_us": stats.p95_us,
        "p99_us": stats.p99_us,
        "max_us": stats.max_us,
        "robust_us": stats.robust_us,
    })
}

fn main() {
    let started = Instant::now();
    let qpc_frequency = qpc_frequency_checked().expect("QPC frequency");
    let modes = [
        build_wait_mode("production_adaptive_spin", true, true, true),
        build_fixed_wait_mode("fixed_spin_250us", 250),
        build_fixed_wait_mode("fixed_spin_400us", 400),
        build_fixed_wait_mode("fixed_spin_700us", 700),
        build_fixed_wait_mode("fixed_spin_1000us", 1_000),
    ];
    let mut mode_reports = serde_json::Map::new();
    for mode in modes {
        let mut scenarios = serde_json::Map::new();
        scenarios.insert(
            "down_only_1".to_string(),
            summarize(run_down(1, mode).unwrap_or_else(|error| panic!("{error}"))),
        );
        scenarios.insert(
            "down_only_5".to_string(),
            summarize(run_down(5, mode).unwrap_or_else(|error| panic!("{error}"))),
        );
        scenarios.insert(
            "down_only_15".to_string(),
            summarize(run_down(15, mode).unwrap_or_else(|error| panic!("{error}"))),
        );
        scenarios.insert(
            "up_only_1".to_string(),
            summarize(run_up(1, mode).unwrap_or_else(|error| panic!("{error}"))),
        );
        for key_count in [5, 15] {
            scenarios.insert(
                format!("up_only_{key_count}"),
                summarize(run_up(key_count, mode).unwrap_or_else(|error| panic!("{error}"))),
            );
        }
        for event_count in [2, 10, 30] {
            scenarios.insert(
                format!("mixed_{event_count}"),
                summarize(run_mixed(event_count, mode).unwrap_or_else(|error| panic!("{error}"))),
            );
        }
        mode_reports.insert(
            mode.name.to_string(),
            json!({
                "waitable_timer_enabled": mode.waitable_timer_enabled,
                "event_wait_enabled": mode.event_wait_enabled,
                "adaptive_spin_enabled": mode.adaptive_spin_enabled,
                "spin_floor_us": sky_player_rs::engine::dispatch_primitives::LEGACY_ADAPTIVE_SPIN_FLOOR_US,
                "effective_spin_threshold_us": mode.effective_spin_threshold_us,
                "mmcss_mode": "off_test_guard",
                "priority_mode": "off_test_guard",
                "startup_kernel_timer_wake_error_us": wake_error_json(mode.startup_wake_error),
                "iterations": iterations(),
                "scenarios": scenarios,
            }),
        );
    }
    let observation_enqueue_ab = observation_enqueue_ab();
    let output = serde_json::to_string_pretty(&json!({
        "benchmark": "rt_handoff_bench",
        "production_timing_policy": true,
        "evidence_scope": "Rust handoff timing with deterministic mock transport; not Raw Input or game-observed latency",
        "rust_version": rust_version(),
        "qpc_frequency": qpc_frequency,
        "iterations": iterations(),
        "deadline_us": due_us(),
        "transport": "deterministic_mock",
        "observation_enqueue_ab": observation_enqueue_ab,
        "modes": mode_reports,
        "elapsed_ms": started.elapsed().as_millis(),
    }))
    .expect("serialize benchmark output");
    if let Some(path) = std::env::args_os().nth(1) {
        std::fs::write(path, &output).expect("write benchmark output");
    }
    println!("{output}");
}
