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
use sky_dispatch_win32::clock::{QpcClock, QpcTicks, qpc_frequency_checked};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::input::{
    PhysicalPacket, PreparedPhysicalPacket, SendTransactionOutcome, SendTransactionStatus,
};
use sky_dispatch_win32::wait::{HybridWaiter, WakeErrorStats};
use sky_player::engine::dispatch_primitives::{
    DispatchObservation, DispatchPath, DispatchStep, NextDispatchPlan, OBSERVATION_QUEUE_CAPACITY,
    PendingObservationQueue, PrecisionHandoffEvidence, PreparationCounts,
    ProductionDispatchTestHarness,
};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::Instant;

const DEFAULT_ITERATIONS: usize = 10_000;
const DUE_US: u64 = 10_000;
const SYNTHETIC_TRANSPORT_COMPLETION_US: u64 = 8;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BenchmarkMode {
    RealWait,
    PhaseASyntheticTargetPlusOneTick,
    PhaseASenderOnly,
    PhaseAProductionBoundary,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BenchmarkScope {
    Full,
    RealWaitCore,
    PhaseASenderOnly,
    PhaseAProductionMatrix,
}

impl BenchmarkScope {
    fn from_env() -> Result<Self, String> {
        match std::env::var("RT_HANDOFF_BENCH_SCOPE").as_deref() {
            Ok("full") | Err(std::env::VarError::NotPresent) => Ok(Self::Full),
            Ok("real_wait_core") => Ok(Self::RealWaitCore),
            Ok("phase_a_sender_only") => Ok(Self::PhaseASenderOnly),
            Ok("phase_a_production_matrix") => Ok(Self::PhaseAProductionMatrix),
            Ok(value) => Err(format!(
                "RT_HANDOFF_BENCH_SCOPE must be full, real_wait_core, phase_a_sender_only, or phase_a_production_matrix, got {value:?}"
            )),
            Err(error) => Err(format!("RT_HANDOFF_BENCH_SCOPE is invalid: {error}")),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::RealWaitCore => "real_wait_core",
            Self::PhaseASenderOnly => "phase_a_sender_only",
            Self::PhaseAProductionMatrix => "phase_a_production_matrix",
        }
    }
}

impl BenchmarkMode {
    fn from_env() -> Result<Self, String> {
        match std::env::var("RT_HANDOFF_BENCH_MODE").as_deref() {
            Ok("real_wait") | Err(std::env::VarError::NotPresent) => Ok(Self::RealWait),
            Ok("phase_a_synthetic_target_plus_one_tick") => {
                Ok(Self::PhaseASyntheticTargetPlusOneTick)
            }
            Ok("phase_a_sender_only") => Ok(Self::PhaseASenderOnly),
            Ok("phase_a_production_boundary") => Ok(Self::PhaseAProductionBoundary),
            Ok(value) => Err(format!(
                "RT_HANDOFF_BENCH_MODE must be real_wait, phase_a_synthetic_target_plus_one_tick, phase_a_sender_only, or phase_a_production_boundary, got {value:?}"
            )),
            Err(error) => Err(format!("RT_HANDOFF_BENCH_MODE is invalid: {error}")),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::RealWait => "real_wait",
            Self::PhaseASyntheticTargetPlusOneTick => "phase_a_synthetic_target_plus_one_tick",
            Self::PhaseASenderOnly => "phase_a_sender_only",
            Self::PhaseAProductionBoundary => "phase_a_production_boundary",
        }
    }

    const fn uses_real_waiter(self) -> bool {
        matches!(self, Self::RealWait)
    }
}

#[derive(Default)]
struct Samples {
    plan_build_us: Vec<u64>,
    packet_header_reads_per_plan: Vec<u64>,
    expected_up_intents_per_plan: Vec<u64>,
    expected_down_intents_per_plan: Vec<u64>,
    up_intent_visits_per_plan: Vec<u64>,
    down_intent_visits_per_plan: Vec<u64>,
    secondary_batch_visits_per_plan: Vec<u64>,
    secondary_batch_visit_bounds_per_plan: Vec<u64>,
    intent_visits_per_plan: Vec<u64>,
    registry_lookups_per_plan: Vec<u64>,
    view_packet_calls_per_plan: Vec<u64>,
    commit_freeze_calls_per_plan: Vec<u64>,
    admission_wake_to_precision_wake_us: Vec<i64>,
    target_crossing_error_us: Vec<i64>,
    target_crossing_to_final_policy_us: Vec<i64>,
    final_policy_to_true_pre_call_us: Vec<i64>,
    dispatch_start_error_us: Vec<i64>,
    pre_call_to_completion_us: Vec<u64>,
    completion_to_rt_ready_us: Vec<i64>,
    target_to_completion_us: Vec<i64>,
    completion_error_us: Vec<i64>,
    physical_dispatches: usize,
    wait_count: usize,
    overdue_dispatch_count: usize,
    early_dispatch_count: usize,
    non_dispatches: usize,
    deadline_missed_count: usize,
    failure_reasons: BTreeMap<String, usize>,
    observation_count: usize,
    observation_gaps: usize,
    spin_time_us: Vec<u64>,
    wall_time_us: Vec<u64>,
}

impl Samples {
    fn record_failure(&mut self, reason: impl Into<String>) {
        self.non_dispatches += 1;
        *self.failure_reasons.entry(reason.into()).or_default() += 1;
    }

    fn record_observation_failure(&mut self, reason: impl Into<String>) {
        *self.failure_reasons.entry(reason.into()).or_default() += 1;
    }

    fn record_step_failure(&mut self, step: &DispatchStep) {
        let reason = match step {
            DispatchStep::TerminateStatic(reason)
                if matches!(
                    *reason,
                    "down_hard_late_abort" | "down_deadline_missed_before_send"
                ) =>
            {
                self.deadline_missed_count += 1;
                *reason
            }
            DispatchStep::TerminateStatic(reason) => *reason,
            other => return self.record_failure(format!("unexpected_step:{other:?}")),
        };
        self.record_failure(reason);
    }
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

fn cpu_duty_percent(
    cpu_started_us: u64,
    cpu_finished_us: u64,
    wall_started: Instant,
) -> Option<f64> {
    let cpu_elapsed_us = cpu_finished_us.checked_sub(cpu_started_us)?;
    let wall_elapsed_us = u64::try_from(wall_started.elapsed().as_micros()).ok()?;
    if cpu_finished_us == 0 || wall_elapsed_us == 0 {
        return None;
    }
    Some(cpu_elapsed_us as f64 * 100.0 / wall_elapsed_us as f64)
}

fn observer_ab_iterations() -> usize {
    iterations().max(10_000)
}

fn representative_observation() -> DispatchObservation {
    let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 0);
    let mut plan = NextDispatchPlan::default();
    harness.plan_current_dispatch_projected_into(&mut plan);
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

fn unsigned_summary(mut values: Vec<u64>) -> serde_json::Value {
    let min = values.iter().copied().min();
    let max = values.iter().copied().max();
    let p50 = quantile(&mut values, 50, 100);
    let p95 = quantile(&mut values, 95, 100);
    let p99 = quantile(&mut values, 99, 100);
    let p99_9 = quantile(&mut values, 999, 1000);
    json!({
        "min": min,
        "p50": p50,
        "p95": p95,
        "p99": p99,
        "p99_9": p99_9,
        "max": max,
        "samples": values.len(),
    })
}

fn signed_summary(mut values: Vec<i64>) -> serde_json::Value {
    let min = values.iter().copied().min();
    let max = values.iter().copied().max();
    let p50 = quantile(&mut values, 50, 100);
    let p95 = quantile(&mut values, 95, 100);
    let p99 = quantile(&mut values, 99, 100);
    let p99_9 = quantile(&mut values, 999, 1000);
    json!({
        "min": min,
        "p50": p50,
        "p95": p95,
        "p99": p99,
        "p99_9": p99_9,
        "max": max,
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

#[allow(clippy::too_many_arguments)]
fn record_precision_handoff(
    samples: &mut Samples,
    handoff: Option<PrecisionHandoffEvidence>,
    physical_target_qpc: QpcTicks,
    pre_call_qpc: QpcTicks,
    sendinput_completion_qpc: QpcTicks,
    dispatch_ready_qpc: Option<QpcTicks>,
    qpc_clock: QpcClock,
) {
    let Some(handoff) = handoff else {
        return;
    };
    if let Some(admission_wake_qpc) = handoff.admission_wake_qpc {
        samples
            .admission_wake_to_precision_wake_us
            .push(signed_qpc_us(
                qpc_clock,
                handoff.target_crossing_qpc,
                admission_wake_qpc,
            ));
    }
    samples.target_crossing_error_us.push(signed_qpc_us(
        qpc_clock,
        handoff.target_crossing_qpc,
        physical_target_qpc,
    ));
    samples
        .target_crossing_to_final_policy_us
        .push(signed_qpc_us(
            qpc_clock,
            handoff.final_policy_qpc,
            handoff.target_crossing_qpc,
        ));
    samples.final_policy_to_true_pre_call_us.push(signed_qpc_us(
        qpc_clock,
        pre_call_qpc,
        handoff.final_policy_qpc,
    ));
    if let Some(dispatch_ready_qpc) = dispatch_ready_qpc {
        samples.completion_to_rt_ready_us.push(signed_qpc_us(
            qpc_clock,
            dispatch_ready_qpc,
            sendinput_completion_qpc,
        ));
    }
}

fn add_observation(samples: &mut Samples, observation: DispatchObservation) {
    let qpc_clock = QpcClock::initialize().expect("QPC");
    match observation {
        DispatchObservation::Down(value) => {
            record_precision_handoff(
                samples,
                value.precision_handoff,
                value.physical_target_qpc,
                value.pre_call_qpc,
                value.sendinput_completion_qpc,
                value.dispatch_ready_qpc,
                qpc_clock,
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
            record_precision_handoff(
                samples,
                value.precision_handoff,
                value.physical_target_qpc,
                value.pre_call_qpc,
                value.sendinput_completion_qpc,
                value.dispatch_ready_qpc,
                qpc_clock,
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
            let _ = wait;
            samples.record_observation_failure("unexpected_wait_observation");
        }
        DispatchObservation::StaleMetadata(value) => {
            let _ = value;
            samples.record_observation_failure("unexpected_stale_metadata_observation");
        }
        DispatchObservation::BlockedUnfocused(value) => {
            let _ = value;
            samples.record_observation_failure("unexpected_blocked_focus_observation");
        }
        DispatchObservation::Lifecycle(value) => {
            let _ = value;
            samples.record_observation_failure("unexpected_lifecycle_observation");
        }
    }
}

fn plan_projected(harness: &mut ProductionDispatchTestHarness, plan: &mut NextDispatchPlan) {
    harness.plan_current_dispatch_projected_into(plan);
}

fn record_preparation_sample(samples: &mut Samples, counts: PreparationCounts, elapsed_ns: u64) {
    assert_eq!(
        counts.packet_header_reads, 1,
        "authored preparation must acquire one packet header"
    );
    assert_eq!(
        counts.view_packet_calls, 1,
        "authored preparation must build one packet view"
    );
    assert_eq!(
        counts.up_intent_visits, counts.expected_up_intents,
        "Up operation visits must match the packet cardinality captured at acquisition"
    );
    assert_eq!(
        counts.down_intent_visits, counts.expected_down_intents,
        "Down operation visits must match the packet cardinality captured at acquisition"
    );
    assert!(
        counts.secondary_batch_visits <= counts.secondary_batch_visit_bound,
        "deferred source resolution exceeded its bounded batch range"
    );
    assert_eq!(
        counts.commit_freeze_calls, 1,
        "authored preparation must freeze one commit token"
    );
    assert_eq!(
        counts.registry_lookups,
        counts
            .up_intent_visits
            .saturating_add(counts.down_intent_visits),
        "registry lookup evidence must match actual intent operations"
    );
    samples.plan_build_us.push(elapsed_ns / 1_000);
    samples
        .packet_header_reads_per_plan
        .push(counts.packet_header_reads);
    samples
        .expected_up_intents_per_plan
        .push(counts.expected_up_intents);
    samples
        .expected_down_intents_per_plan
        .push(counts.expected_down_intents);
    samples
        .up_intent_visits_per_plan
        .push(counts.up_intent_visits);
    samples
        .down_intent_visits_per_plan
        .push(counts.down_intent_visits);
    samples
        .secondary_batch_visits_per_plan
        .push(counts.secondary_batch_visits);
    samples
        .secondary_batch_visit_bounds_per_plan
        .push(counts.secondary_batch_visit_bound);
    samples.intent_visits_per_plan.push(
        counts
            .up_intent_visits
            .saturating_add(counts.down_intent_visits),
    );
    samples
        .registry_lookups_per_plan
        .push(counts.registry_lookups);
    samples
        .view_packet_calls_per_plan
        .push(counts.view_packet_calls);
    samples
        .commit_freeze_calls_per_plan
        .push(counts.commit_freeze_calls);
}

fn wait_and_dispatch_or_record(
    harness: &mut ProductionDispatchTestHarness,
    plan: &sky_player::engine::dispatch_primitives::NextDispatchPlan,
    benchmark_mode: BenchmarkMode,
    samples: &mut Samples,
) -> Result<Option<()>, String> {
    let step = match benchmark_mode {
        BenchmarkMode::RealWait => {
            let step = match harness.wait_and_dispatch_current_plan(plan) {
                Ok(step) => step,
                Err(error) => {
                    samples.record_failure(format!("wait_error:{error}"));
                    return Ok(None);
                }
            };
            if harness.last_wait_result().is_some() {
                samples.wait_count += 1;
            } else {
                // A target can become due while the benchmark is still
                // preparing the frozen plan.  That is a legitimate
                // production overdue path, but it is not a real waiter
                // sample and must make waiter qualification ineligible.
                samples.overdue_dispatch_count += 1;
            }
            step
        }
        BenchmarkMode::PhaseASyntheticTargetPlusOneTick => {
            if harness.physical_target_qpc_for_test(plan).is_none() {
                return Err("synthetic benchmark plan has no physical target".to_string());
            }
            harness.dispatch_at_phase_a_benchmark_boundary_for_test(
                plan,
                SYNTHETIC_TRANSPORT_COMPLETION_US,
            )
        }
        BenchmarkMode::PhaseASenderOnly => {
            return Err("phase_a_sender_only does not use coordinator dispatch".to_string());
        }
        BenchmarkMode::PhaseAProductionBoundary => {
            if harness.physical_target_qpc_for_test(plan).is_none() {
                return Err("production-boundary benchmark plan has no physical target".to_string());
            }
            harness.dispatch_at_phase_a_production_boundary_for_test(plan)
        }
    };
    if matches!(step, DispatchStep::Dispatched) {
        Ok(Some(()))
    } else {
        samples.record_step_failure(&step);
        Ok(None)
    }
}

fn drain_observations(harness: &mut ProductionDispatchTestHarness, samples: &mut Samples) {
    let mut physical_count = 0;
    while let Some(observation) = harness.pop_observation() {
        match observation {
            DispatchObservation::Down(_) | DispatchObservation::Up(_) => {
                physical_count += 1;
                samples.observation_count += 1;
                add_observation(samples, observation);
            }
            // A terminal cleanup/reset lifecycle can be emitted after a
            // mixed packet. It is not a physical timing sample; accepting it
            // here keeps the real-wait benchmark focused on the one physical
            // observation promised by the iteration while still detecting a
            // missing or duplicated Down/Up below.
            DispatchObservation::Lifecycle(_) => {}
            other => {
                samples.observation_count += 1;
                add_observation(samples, other);
            }
        }
    }
    if physical_count != 1 {
        samples.observation_gaps += 1;
    }
}

fn record_wait_metrics(
    samples: &mut Samples,
    harness: &ProductionDispatchTestHarness,
    benchmark_mode: BenchmarkMode,
) -> Result<(), String> {
    if matches!(benchmark_mode, BenchmarkMode::RealWait) {
        samples.spin_time_us.push(harness.last_wait_spin_us()?);
    }
    Ok(())
}

fn spin_duty_cycle_ppm(spin_time_us: &[u64], wall_time_us: &[u64]) -> u64 {
    let spin_total = spin_time_us
        .iter()
        .copied()
        .fold(0_u64, u64::saturating_add);
    let wall_total = wall_time_us
        .iter()
        .copied()
        .fold(0_u64, u64::saturating_add);
    if wall_total == 0 {
        0
    } else {
        spin_total
            .saturating_mul(1_000_000)
            .checked_div(wall_total)
            .unwrap_or(0)
    }
}

fn run_down(
    key_count: usize,
    mode: WaitMode,
    benchmark_mode: BenchmarkMode,
) -> Result<Samples, String> {
    let mut samples = Samples::default();
    for _ in 0..iterations() {
        let iteration_started = Instant::now();
        let mut harness =
            ProductionDispatchTestHarness::new_down_chord_with_gap(key_count, due_us());
        harness.enable_dispatch_ready_timing_for_benchmark();
        let alignment_margin_us =
            if matches!(benchmark_mode, BenchmarkMode::PhaseAProductionBoundary) {
                0
            } else {
                due_us()
            };
        harness.align_next_plan_to_benchmark_margin_for_test(alignment_margin_us);
        harness.configure_production_wait_policy(mode.effective_spin_threshold_us)?;
        harness.reset_preparation_counts_for_test();
        let mut plan = NextDispatchPlan::default();
        let plan_started = Instant::now();
        plan_projected(&mut harness, &mut plan);
        record_preparation_sample(
            &mut samples,
            harness.preparation_counts(),
            elapsed_ns(plan_started),
        );
        if wait_and_dispatch_or_record(&mut harness, &plan, benchmark_mode, &mut samples)?.is_none()
        {
            samples
                .wall_time_us
                .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
            continue;
        }
        samples.physical_dispatches += 1;
        record_wait_metrics(&mut samples, &harness, benchmark_mode)?;
        drain_observations(&mut harness, &mut samples);
        samples
            .wall_time_us
            .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
    }
    Ok(samples)
}

fn run_up(
    key_count: usize,
    mode: WaitMode,
    benchmark_mode: BenchmarkMode,
) -> Result<Samples, String> {
    let mut samples = Samples::default();
    for _ in 0..iterations() {
        let iteration_started = Instant::now();
        let mut harness = match ProductionDispatchTestHarness::try_new_uponly_release_chord_with_gap(
            key_count,
            due_us(),
        ) {
            Ok(harness) => harness,
            Err(error) => {
                samples.record_failure(format!("setup_error:{error}"));
                samples.wall_time_us.push(
                    u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX),
                );
                continue;
            }
        };
        harness.enable_dispatch_ready_timing_for_benchmark();
        harness.configure_production_wait_policy(mode.effective_spin_threshold_us)?;
        while harness.pop_observation().is_some() {}
        if matches!(benchmark_mode, BenchmarkMode::PhaseAProductionBoundary) {
            harness.align_next_plan_to_benchmark_margin_for_test(0);
        }
        assert_eq!(
            harness.current_authored_path(),
            Some(DispatchPath::UpOnly {
                up_count: key_count
            }),
            "authored benchmark setup did not leave the requested physical UpOnly packet"
        );
        harness.reset_preparation_counts_for_test();
        let mut plan = NextDispatchPlan::default();
        let plan_started = Instant::now();
        plan_projected(&mut harness, &mut plan);
        record_preparation_sample(
            &mut samples,
            harness.preparation_counts(),
            elapsed_ns(plan_started),
        );
        if wait_and_dispatch_or_record(&mut harness, &plan, benchmark_mode, &mut samples)?.is_none()
        {
            samples
                .wall_time_us
                .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
            continue;
        }
        samples.physical_dispatches += 1;
        record_wait_metrics(&mut samples, &harness, benchmark_mode)?;
        drain_observations(&mut harness, &mut samples);
        samples
            .wall_time_us
            .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
    }
    Ok(samples)
}

fn run_mixed(
    event_count: usize,
    mode: WaitMode,
    benchmark_mode: BenchmarkMode,
) -> Result<Samples, String> {
    let mut samples = Samples::default();
    for _ in 0..iterations() {
        let iteration_started = Instant::now();
        let mut harness = match ProductionDispatchTestHarness::try_new_mixed_events_with_gap(
            event_count,
            due_us(),
        ) {
            Ok(harness) => harness,
            Err(error) => {
                samples.record_failure(format!("setup_error:{error}"));
                samples.wall_time_us.push(
                    u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX),
                );
                continue;
            }
        };
        harness.enable_dispatch_ready_timing_for_benchmark();
        harness.configure_production_wait_policy(mode.effective_spin_threshold_us)?;
        while harness.pop_observation().is_some() {}
        if matches!(benchmark_mode, BenchmarkMode::PhaseAProductionBoundary) {
            harness.align_next_plan_to_benchmark_margin_for_test(0);
        }
        harness.reset_preparation_counts_for_test();
        let mut plan = NextDispatchPlan::default();
        let plan_started = Instant::now();
        plan_projected(&mut harness, &mut plan);
        record_preparation_sample(
            &mut samples,
            harness.preparation_counts(),
            elapsed_ns(plan_started),
        );
        if wait_and_dispatch_or_record(&mut harness, &plan, benchmark_mode, &mut samples)?.is_none()
        {
            samples
                .wall_time_us
                .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
            continue;
        }
        samples.physical_dispatches += 1;
        record_wait_metrics(&mut samples, &harness, benchmark_mode)?;
        drain_observations(&mut harness, &mut samples);
        samples
            .wall_time_us
            .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
    }
    Ok(samples)
}

fn add_sender_only_sample(
    samples: &mut Samples,
    target: QpcTicks,
    outcome: SendTransactionOutcome,
    clock: QpcClock,
) {
    if !matches!(outcome.status, SendTransactionStatus::Complete) {
        samples.record_failure(format!("sender_status:{:?}", outcome.status));
        return;
    }
    let Some(started) = outcome.evidence.started_ticks else {
        samples.record_failure("sender_missing_started_ticks");
        return;
    };
    let Some(completed) = outcome.evidence.completed_ticks else {
        samples.record_failure("sender_missing_completed_ticks");
        return;
    };
    samples.physical_dispatches += 1;
    samples.observation_count += 1;
    samples
        .dispatch_start_error_us
        .push(signed_qpc_us(clock, started, target));
    samples.pre_call_to_completion_us.push(
        completed
            .checked_duration_since(started)
            .ok()
            .and_then(|ticks| clock.duration_to_us(ticks).ok())
            .unwrap_or(0),
    );
    samples
        .target_to_completion_us
        .push(signed_qpc_us(clock, completed, target));
    if started < target {
        samples.early_dispatch_count += 1;
    }
}

fn run_phase_a_sender_only(packet: PhysicalPacket) -> Result<Samples, String> {
    let mut samples = Samples::default();
    let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(1, due_us());
    let prepared = PreparedPhysicalPacket::try_new(packet).expect("prepared sender-only packet");
    let clock = QpcClock::initialize().expect("QPC");
    for _ in 0..iterations() {
        let (target, outcome) = harness.send_prepared_phase_a_packet_for_test(&prepared);
        add_sender_only_sample(&mut samples, target, outcome, clock);
    }
    Ok(samples)
}

fn phase_a_sender_only_report() -> serde_json::Value {
    let scenarios = serde_json::json!({
        "down_only_15": summarize(run_phase_a_sender_only(PhysicalPacket::new(0, 0x7fff)).unwrap_or_else(|error| panic!("{error}"))),
        "up_only_15": summarize(run_phase_a_sender_only(PhysicalPacket::new(0x7fff, 0)).unwrap_or_else(|error| panic!("{error}"))),
        "mixed_2": summarize(run_phase_a_sender_only(PhysicalPacket::new(0b01, 0b10)).unwrap_or_else(|error| panic!("{error}"))),
    });
    serde_json::json!({
        "scope": "sender-only; prepared packet and tracked-state reconciliation retained; waiter/coordinator excluded",
        "waitable_timer_enabled": false,
        "event_wait_enabled": false,
        "adaptive_spin_enabled": false,
        "effective_spin_threshold_us": 0,
        "synthetic_boundary": "QPC target sampled immediately before sender call",
        "scenarios": scenarios,
        "iterations": iterations(),
    })
}

fn phase_a_production_matrix_report() -> serde_json::Value {
    let mode = build_wait_mode("production_boundary", true, true, true);
    let benchmark_mode = BenchmarkMode::PhaseAProductionBoundary;
    let mode_started = Instant::now();
    let cpu_started_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    let mut scenarios = serde_json::Map::new();
    for key_count in [1, 5, 15] {
        scenarios.insert(
            format!("down_only_{key_count}"),
            summarize(
                run_down(key_count, mode, benchmark_mode).unwrap_or_else(|error| panic!("{error}")),
            ),
        );
    }
    for key_count in [1, 5, 15] {
        scenarios.insert(
            format!("up_only_{key_count}"),
            summarize(
                run_up(key_count, mode, benchmark_mode).unwrap_or_else(|error| panic!("{error}")),
            ),
        );
    }
    for event_count in [2, 10, 14] {
        scenarios.insert(
            format!("mixed_{event_count}"),
            summarize(
                run_mixed(event_count, mode, benchmark_mode)
                    .unwrap_or_else(|error| panic!("{error}")),
            ),
        );
    }
    let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    serde_json::json!({
        "scope": "Phase-A acceptance production dispatch/admission/commit path with a deterministic direct crossing and mock transport; waiter scheduling excluded",
        "waitable_timer_enabled": mode.waitable_timer_enabled,
        "event_wait_enabled": mode.event_wait_enabled,
        "adaptive_spin_enabled": mode.adaptive_spin_enabled,
        "spin_floor_us": sky_player::engine::dispatch_primitives::PRODUCTION_MIN_SPIN_THRESHOLD_US,
        "calibration_samples": sky_player::engine::dispatch_primitives::PRODUCTION_CALIBRATION_SAMPLES,
        "calibration_budget_us": sky_player::engine::dispatch_primitives::PRODUCTION_CALIBRATION_BUDGET_US,
        "startup_readiness_reserve_us": sky_player::engine::dispatch_primitives::PRODUCTION_STARTUP_READINESS_RESERVE_US,
        "startup_kernel_timer_wake_error_us": wake_error_json(mode.startup_wake_error),
        "effective_spin_threshold_us": mode.effective_spin_threshold_us,
        "sender_start_timestamp_source": "mock transport QPC sampled at its immediate callback boundary; production native sender samples inside the SendInput envelope",
        "transport": "deterministic packet emitter with immediate QPC start and completion samples",
        "process_cpu_time_us": cpu_finished_us.saturating_sub(cpu_started_us),
        "process_cpu_duty_percent": cpu_duty_percent(cpu_started_us, cpu_finished_us, mode_started),
        "scenarios": scenarios,
        "iterations": iterations(),
    })
}

fn summarize(mut samples: Samples) -> serde_json::Value {
    assert!(
        samples.dispatch_start_error_us.len() <= samples.observation_count,
        "timing samples cannot exceed collected observations"
    );
    assert_eq!(
        samples.physical_dispatches + samples.non_dispatches,
        iterations(),
        "scenario did not account for every benchmark attempt"
    );
    let mut acceptance_failure_reasons = Vec::new();
    if samples.deadline_missed_count != 0 {
        acceptance_failure_reasons.push("deadline_missed_before_send");
    }
    if samples.non_dispatches != 0 {
        acceptance_failure_reasons.push("non_dispatches");
    }
    if samples.overdue_dispatch_count != 0 {
        acceptance_failure_reasons.push("overdue_dispatches");
    }
    if samples.early_dispatch_count != 0 {
        acceptance_failure_reasons.push("early_dispatch");
    }
    if !samples.failure_reasons.is_empty() {
        acceptance_failure_reasons.push("failure_reasons");
    }
    if samples.observation_gaps != 0 {
        acceptance_failure_reasons.push("observation_gaps");
    }
    if samples.dispatch_start_error_us.len() != samples.observation_count {
        acceptance_failure_reasons.push("missing_start_error_samples");
    }
    let acceptance_clean = acceptance_failure_reasons.is_empty();
    let statistics_eligible = acceptance_clean && iterations() >= 10_000;
    let sender_cutoff_exercised = true;
    let total_spin_time_us = samples.spin_time_us.iter().copied().sum::<u64>();
    let total_wall_time_us = samples.wall_time_us.iter().copied().sum::<u64>();
    let spin_duty = spin_duty_cycle_ppm(&samples.spin_time_us, &samples.wall_time_us);
    json!({
        "acceptance_clean": acceptance_clean,
        "acceptance_failure_reasons": acceptance_failure_reasons,
        "statistics_eligible": statistics_eligible,
        "qualification_dimensions": {
            "waiter_timing_clean": acceptance_clean,
            "dispatch_path_clean": acceptance_clean,
            "sender_cutoff_clean": false,
            "sender_cutoff_exercised": sender_cutoff_exercised,
            "sender_cutoff_note": "This benchmark uses the test-support sender seam; it does not qualify the production SendInput cutoff. Deterministic cutoff truth-table coverage is reported by the Rust sender tests.",
            "statistics_eligible": statistics_eligible,
        },
        "controller": "dispatch_start_error",
        "preparation": {
            "plan_build_us": unsigned_summary(samples.plan_build_us),
            "packet_header_reads_per_plan": unsigned_summary(samples.packet_header_reads_per_plan),
            "expected_up_intents_per_plan": unsigned_summary(
                samples.expected_up_intents_per_plan,
            ),
            "expected_down_intents_per_plan": unsigned_summary(
                samples.expected_down_intents_per_plan,
            ),
            "up_intent_visits_per_plan": unsigned_summary(samples.up_intent_visits_per_plan),
            "down_intent_visits_per_plan": unsigned_summary(samples.down_intent_visits_per_plan),
            "secondary_batch_visits_per_plan": unsigned_summary(
                samples.secondary_batch_visits_per_plan,
            ),
            "secondary_batch_visit_bounds_per_plan": unsigned_summary(
                samples.secondary_batch_visit_bounds_per_plan,
            ),
            "intent_visits_per_plan": unsigned_summary(samples.intent_visits_per_plan),
            "registry_lookups_per_plan": unsigned_summary(samples.registry_lookups_per_plan),
            "view_packet_calls_per_plan": unsigned_summary(samples.view_packet_calls_per_plan),
            "commit_freeze_calls_per_plan": unsigned_summary(samples.commit_freeze_calls_per_plan),
        },
        "admission_wake_to_precision_wake_us": signed_summary(
            samples.admission_wake_to_precision_wake_us,
        ),
        "target_crossing_error_us": signed_summary(samples.target_crossing_error_us),
        "target_crossing_to_final_policy_us": signed_summary(
            samples.target_crossing_to_final_policy_us,
        ),
        "final_policy_to_true_pre_call_us": signed_summary(samples.final_policy_to_true_pre_call_us),
        "dispatch_start_error_us": signed_summary(samples.dispatch_start_error_us),
        "completion_error_us_diagnostic": {
            "p01": quantile(&mut samples.completion_error_us, 1, 100),
            "p50": quantile(&mut samples.completion_error_us, 50, 100),
            "p95": quantile(&mut samples.completion_error_us, 95, 100),
            "p99": quantile(&mut samples.completion_error_us, 99, 100),
            "max_abs": samples.completion_error_us.iter().map(|value| value.unsigned_abs()).max(),
            "samples": samples.completion_error_us.len(),
        },
        "pre_call_to_completion_us": unsigned_summary(samples.pre_call_to_completion_us),
        "completion_to_rt_ready_us": signed_summary(samples.completion_to_rt_ready_us),
        "target_to_completion_us": signed_summary(samples.target_to_completion_us),
        "physical_dispatches": samples.physical_dispatches,
        "wait_count": samples.wait_count,
        "overdue_dispatch_count": samples.overdue_dispatch_count,
        "early_dispatch_count": samples.early_dispatch_count,
        "non_dispatches": samples.non_dispatches,
        "deadline_missed_before_send_count": samples.deadline_missed_count,
        "failure_reasons": samples.failure_reasons,
        "observation_count": samples.observation_count,
        "observation_gaps": samples.observation_gaps,
        "observation_queue": "bounded_nonblocking_on",
        "spin_time_us": unsigned_summary(samples.spin_time_us),
        "wall_time_us": unsigned_summary(samples.wall_time_us),
        "total_spin_time_us": total_spin_time_us,
        "total_wall_time_us": total_wall_time_us,
        "spin_duty_cycle_ppm": spin_duty,
    })
}

fn all_scenarios_clean(mode_reports: &serde_json::Map<String, serde_json::Value>) -> bool {
    mode_reports.values().all(|mode| {
        mode.get("scenarios")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|scenarios| {
                scenarios.values().all(|scenario| {
                    scenario
                        .get("acceptance_clean")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true)
                })
            })
    })
}

fn build_wait_mode(
    name: &'static str,
    waitable_timer_enabled: bool,
    event_wait_enabled: bool,
    adaptive_spin_enabled: bool,
) -> WaitMode {
    let qpc_clock = QpcClock::initialize().expect("QPC");
    let waiter = if waitable_timer_enabled && event_wait_enabled {
        HybridWaiter::production()
    } else {
        HybridWaiter::with_options(waitable_timer_enabled, event_wait_enabled)
    };
    let interrupt = OwnedEvent::new_auto_reset().expect("benchmark interrupt event");
    let startup_wake_error = waiter
        .probe_wake_error_stats(
            qpc_clock,
            &interrupt,
            sky_player::engine::dispatch_primitives::PRODUCTION_CALIBRATION_SAMPLES,
        )
        .unwrap_or_else(|| panic!("{name}: startup wake probe failed; refusing mock fallback"));
    let effective_spin_threshold_us = if adaptive_spin_enabled {
        sky_player::engine::dispatch_primitives::calibrated_spin_threshold_us(startup_wake_error)
    } else if event_wait_enabled {
        sky_player::engine::dispatch_primitives::LEGACY_ADAPTIVE_SPIN_FLOOR_US
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
    let waiter = HybridWaiter::production();
    let interrupt = OwnedEvent::new_auto_reset().expect("benchmark interrupt event");
    let startup_wake_error = waiter
        .probe_wake_error_stats(
            qpc_clock,
            &interrupt,
            sky_player::engine::dispatch_primitives::PRODUCTION_CALIBRATION_SAMPLES,
        )
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
    let benchmark_mode = BenchmarkMode::from_env().unwrap_or_else(|error| panic!("{error}"));
    let benchmark_scope = BenchmarkScope::from_env().unwrap_or_else(|error| panic!("{error}"));
    if matches!(benchmark_scope, BenchmarkScope::PhaseASenderOnly)
        && !matches!(benchmark_mode, BenchmarkMode::PhaseASenderOnly)
    {
        panic!("phase_a_sender_only requires phase_a_sender_only benchmark mode");
    }
    if matches!(benchmark_scope, BenchmarkScope::PhaseAProductionMatrix)
        && !matches!(benchmark_mode, BenchmarkMode::PhaseAProductionBoundary)
    {
        panic!("phase_a_production_matrix requires phase_a_production_boundary benchmark mode");
    }
    if matches!(benchmark_scope, BenchmarkScope::RealWaitCore)
        && !matches!(benchmark_mode, BenchmarkMode::RealWait)
    {
        panic!("real_wait_core requires real_wait benchmark mode");
    }
    let qpc_frequency = qpc_frequency_checked().expect("QPC frequency");
    let mut mode_reports = serde_json::Map::new();
    if matches!(
        benchmark_scope,
        BenchmarkScope::Full | BenchmarkScope::RealWaitCore
    ) {
        let modes = if matches!(benchmark_scope, BenchmarkScope::Full) {
            vec![
                build_wait_mode("production_adaptive_spin", true, true, true),
                build_fixed_wait_mode("fixed_spin_250us", 250),
                build_fixed_wait_mode("fixed_spin_400us", 400),
                build_fixed_wait_mode("fixed_spin_700us", 700),
                build_fixed_wait_mode("fixed_spin_1000us", 1_000),
                build_fixed_wait_mode("fixed_spin_1500us", 1_500),
            ]
        } else {
            vec![
                build_wait_mode("production_adaptive_spin", true, true, true),
                build_fixed_wait_mode("fixed_spin_400us", 400),
                build_fixed_wait_mode("fixed_spin_700us", 700),
                build_fixed_wait_mode("fixed_spin_1000us", 1_000),
            ]
        };
        for mode in modes {
            let mode_started = Instant::now();
            let cpu_started_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
            let mut scenarios = serde_json::Map::new();
            scenarios.insert(
                "down_only_1".to_string(),
                summarize(
                    run_down(1, mode, benchmark_mode).unwrap_or_else(|error| panic!("{error}")),
                ),
            );
            scenarios.insert(
                "down_only_5".to_string(),
                summarize(
                    run_down(5, mode, benchmark_mode).unwrap_or_else(|error| panic!("{error}")),
                ),
            );
            scenarios.insert(
                "down_only_15".to_string(),
                summarize(
                    run_down(15, mode, benchmark_mode).unwrap_or_else(|error| panic!("{error}")),
                ),
            );
            scenarios.insert(
                "up_only_1".to_string(),
                summarize(
                    run_up(1, mode, benchmark_mode).unwrap_or_else(|error| panic!("{error}")),
                ),
            );
            for key_count in [5, 15] {
                scenarios.insert(
                    format!("up_only_{key_count}"),
                    summarize(
                        run_up(key_count, mode, benchmark_mode)
                            .unwrap_or_else(|error| panic!("{error}")),
                    ),
                );
            }
            if matches!(benchmark_scope, BenchmarkScope::Full) {
                for event_count in [2, 10, 14] {
                    scenarios.insert(
                        format!("mixed_{event_count}"),
                        summarize(
                            run_mixed(event_count, mode, benchmark_mode)
                                .unwrap_or_else(|error| panic!("{error}")),
                        ),
                    );
                }
            }
            let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
            mode_reports.insert(
                mode.name.to_string(),
                json!({
                    "waitable_timer_enabled": mode.waitable_timer_enabled,
                    "event_wait_enabled": mode.event_wait_enabled,
                    "adaptive_spin_enabled": mode.adaptive_spin_enabled,
                    "spin_floor_us": sky_player::engine::dispatch_primitives::PRODUCTION_MIN_SPIN_THRESHOLD_US,
                    "calibration_samples": sky_player::engine::dispatch_primitives::PRODUCTION_CALIBRATION_SAMPLES,
                    "calibration_budget_us": sky_player::engine::dispatch_primitives::PRODUCTION_CALIBRATION_BUDGET_US,
                    "startup_readiness_reserve_us": sky_player::engine::dispatch_primitives::PRODUCTION_STARTUP_READINESS_RESERVE_US,
                    "effective_spin_threshold_us": mode.effective_spin_threshold_us,
                    "requested_wait_policy": "production_calibrated",
                    "effective_wait_policy": "production_calibrated",
                    "waiter_constructor": "HybridWaiter::production",
                    "mmcss_mode": "off_test_guard",
                    "priority_mode": "off_test_guard",
                    "startup_kernel_timer_wake_error_us": wake_error_json(mode.startup_wake_error),
                    "process_cpu_time_us": cpu_finished_us.saturating_sub(cpu_started_us),
                    "process_cpu_duty_percent": cpu_duty_percent(cpu_started_us, cpu_finished_us, mode_started),
                    "iterations": iterations(),
                    "scenarios": scenarios,
                }),
            );
        }
    } else if matches!(benchmark_scope, BenchmarkScope::PhaseAProductionMatrix) {
        mode_reports.insert(
            "phase_a_production_boundary".to_string(),
            phase_a_production_matrix_report(),
        );
    } else {
        mode_reports.insert(
            "phase_a_sender_only".to_string(),
            phase_a_sender_only_report(),
        );
    }
    let observation_enqueue_ab = observation_enqueue_ab();
    let acceptance_clean = all_scenarios_clean(&mode_reports);
    let statistics_eligible = acceptance_clean && iterations() >= 10_000;
    let sender_cutoff_exercised = matches!(
        benchmark_mode,
        BenchmarkMode::RealWait
            | BenchmarkMode::PhaseASyntheticTargetPlusOneTick
            | BenchmarkMode::PhaseAProductionBoundary
    );
    let output = serde_json::to_string_pretty(&json!({
        "benchmark": "rt_handoff_bench",
        "acceptance_clean": acceptance_clean,
        "statistics_eligible": statistics_eligible,
        "qualification_dimensions": {
            "waiter_timing_clean": benchmark_mode.uses_real_waiter().then_some(acceptance_clean),
            "dispatch_path_clean": acceptance_clean,
            "sender_cutoff_clean": false,
            "sender_cutoff_exercised": sender_cutoff_exercised,
            "sender_cutoff_qualification": "separate_native_sender_acceptance",
            "sender_cutoff_note": "This benchmark does not independently qualify the production SendInput cutoff; deterministic cutoff truth-table coverage is reported by the Rust sender tests and native acceptance seam.",
            "statistics_eligible": statistics_eligible,
        },
        "requested_wait_policy": "production_calibrated",
        "effective_wait_policy": "production_calibrated",
        "waiter_constructor": if benchmark_mode.uses_real_waiter() {
            "HybridWaiter::production"
        } else {
            "test_support_direct_boundary"
        },
        "priority_mode": "off_test_guard",
        "mmcss_mode": "off_test_guard",
        "production_timing_policy": true,
        "benchmark_scope": benchmark_scope.name(),
        "benchmark_mode": benchmark_mode.name(),
        "waiter_metrics_valid": benchmark_mode.uses_real_waiter(),
        "synthetic_transport_completion_us": matches!(
            benchmark_mode,
            BenchmarkMode::PhaseASyntheticTargetPlusOneTick
        )
        .then_some(SYNTHETIC_TRANSPORT_COMPLETION_US),
        "evidence_scope": match (benchmark_scope, benchmark_mode) {
            (BenchmarkScope::Full | BenchmarkScope::RealWaitCore, BenchmarkMode::RealWait) => "Rust handoff timing with deterministic mock transport and real HybridWaiter; test-support sender cutoff seam is exercised but production SendInput cutoff qualification is separate; not Raw Input or game-observed latency",
            (BenchmarkScope::Full | BenchmarkScope::RealWaitCore, _) => "Phase-A coordinator A/B with deterministic mock transport and a frozen target plus one synthetic QPC tick; waiter scheduling is intentionally excluded; not Raw Input or game-observed latency",
            (BenchmarkScope::PhaseASenderOnly, BenchmarkMode::PhaseASenderOnly) => "Phase-A sender-only A/B with prepared packets and tracked-state reconciliation; target is sampled immediately before the sender call; waiter/coordinator scheduling is intentionally excluded; not Raw Input or game-observed latency",
            (BenchmarkScope::PhaseASenderOnly, _) => "invalid benchmark scope/mode combination",
            (BenchmarkScope::PhaseAProductionMatrix, BenchmarkMode::PhaseAProductionBoundary) => "Phase-A acceptance A/B through the full coordinator dispatch/admission/commit path; a test-only direct boundary supplies the frozen crossing QPC and the mock transport records an immediate sender-boundary QPC; waiter scheduling and the real SendInput syscall are excluded; not Raw Input or game-observed latency",
            (BenchmarkScope::PhaseAProductionMatrix, _) => "invalid benchmark scope/mode combination",
        },
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
