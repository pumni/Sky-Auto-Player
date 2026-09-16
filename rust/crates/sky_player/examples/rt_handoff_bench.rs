//! Small RT handoff benchmark used for before/after comparison.
//!
//! This deliberately has no Criterion dependency.  The plan is built before
//! the real QPC wait, then the deterministic test emitter exercises admission,
//! packet construction, coordinator commit, and the fixed observation enqueue.
//! It is compiled only with `test-support`; production binaries do not contain
//! this harness.

#![cfg(feature = "test-support")]

use serde_json::json;
use sky_dispatch_core::time::{DurationTicks, SEND_COLD_THRESHOLD_US, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcTicks, qpc_frequency_checked};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::input::{
    PhysicalPacket, PreparedPhysicalPacket, SendTransactionOutcome, SendTransactionStatus,
};
use sky_dispatch_win32::wait::{HybridWaiter, WaitOutcome, WakeErrorStats};
use sky_player::engine::dispatch_primitives::{
    DispatchObservation, DispatchPath, DispatchStep, DownMissKind, NextDispatchPlan,
    OBSERVATION_QUEUE_CAPACITY, PendingObservationQueue, PrecisionHandoffEvidence,
    PreparationCounts, ProductionDispatchTestHarness,
};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const DEFAULT_ITERATIONS: usize = 10_000;
const DUE_US: u64 = 10_000;
const SYNTHETIC_TRANSPORT_COMPLETION_US: u64 = 8;
const C0_PASSES: usize = 3;
const C1_PASSES: usize = 3;
const C1_NORMAL_TOLERANCE_US: u64 = 2_500;
const C1_1_PASSES: usize = 3;
const C1_1_TOLERANCES_US: [u64; 3] = [0, 1_500, 2_500];
const C1_1_DENSE_GAPS_US: [u64; 7] = [1_000, 1_500, 2_000, 2_500, 3_000, 4_000, 5_000];
const C1_1_DENSE_OFFSETS_US: [u64; 6] = [600, 1_000, 1_400, 1_600, 2_200, 2_500];
const C1_1_SEQUENTIAL_PASSES: usize = 3;
const C1_1_SEQUENTIAL_ITERATIONS: usize = 50;
const C1_1_SEQUENTIAL_GAPS_US: [u64; 5] = [1_000, 2_000, 3_000, 4_000, 5_000];
const F1_1_TOLERANCES_US: [u64; 3] = [2_500, 3_500, 5_000];
const F1_1_SEQUENTIAL_GAPS_US: [u64; 5] = [1_000, 2_000, 3_000, 4_000, 5_000];
const F1_1_SEQUENTIAL_PASSES: usize = 3;
const F1_1_SEQUENTIAL_ITERATIONS: usize = 30;
const LATE_RESCUE_GRACES_US: [u64; 7] = [0, 250, 500, 1_000, 1_500, 2_500, 4_500];

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
    PhaseASparseGap,
    PhaseBB0,
    PhaseCC0,
    PhaseCC1,
    PhaseCC11,
    PhaseF11,
    PhaseExtendedRange,
}

impl BenchmarkScope {
    fn from_env_or_args() -> Result<Self, String> {
        let args: Vec<String> = std::env::args().collect();
        let arg_val = args
            .windows(2)
            .find(|w| w[0] == "--scope")
            .map(|w| w[1].clone())
            .or_else(|| {
                args.iter()
                    .find_map(|a| a.strip_prefix("--scope=").map(str::to_string))
            });

        let scope_str = match (arg_val, std::env::var("RT_HANDOFF_BENCH_SCOPE")) {
            (Some(val), _) => val,
            (None, Ok(val)) => val,
            (None, Err(std::env::VarError::NotPresent)) => "full".to_string(),
            (None, Err(error)) => {
                return Err(format!("RT_HANDOFF_BENCH_SCOPE is invalid: {error}"));
            }
        };

        match scope_str.as_str() {
            "full" => Ok(Self::Full),
            "real_wait_core" => Ok(Self::RealWaitCore),
            "phase_a_sender_only" => Ok(Self::PhaseASenderOnly),
            "phase_a_production_matrix" => Ok(Self::PhaseAProductionMatrix),
            "phase_a_sparse_gap" => Ok(Self::PhaseASparseGap),
            "phase_b0" => Ok(Self::PhaseBB0),
            "phase_c0" => Ok(Self::PhaseCC0),
            "phase_c1" => Ok(Self::PhaseCC1),
            "phase_c1_1" => Ok(Self::PhaseCC11),
            "phase_f1_1" => Ok(Self::PhaseF11),
            "phase_extended_range" => Ok(Self::PhaseExtendedRange),
            value => Err(format!(
                "scope must be full, real_wait_core, phase_a_sender_only, phase_a_production_matrix, phase_a_sparse_gap, phase_b0, phase_c0, phase_c1, phase_c1_1, phase_f1_1, or phase_extended_range, got {value:?}"
            )),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::RealWaitCore => "real_wait_core",
            Self::PhaseASenderOnly => "phase_a_sender_only",
            Self::PhaseAProductionMatrix => "phase_a_production_matrix",
            Self::PhaseASparseGap => "phase_a_sparse_gap",
            Self::PhaseBB0 => "phase_b0",
            Self::PhaseCC0 => "phase_c0",
            Self::PhaseCC1 => "phase_c1",
            Self::PhaseCC11 => "phase_c1_1",
            Self::PhaseF11 => "phase_f1_1",
            Self::PhaseExtendedRange => "phase_extended_range",
        }
    }
}

impl BenchmarkMode {
    fn from_env_or_args() -> Result<Self, String> {
        let args: Vec<String> = std::env::args().collect();
        let arg_val = args
            .windows(2)
            .find(|w| w[0] == "--mode")
            .map(|w| w[1].clone())
            .or_else(|| {
                args.iter()
                    .find_map(|a| a.strip_prefix("--mode=").map(str::to_string))
            });

        let mode_str = match (arg_val, std::env::var("RT_HANDOFF_BENCH_MODE")) {
            (Some(val), _) => val,
            (None, Ok(val)) => val,
            (None, Err(std::env::VarError::NotPresent)) => "real_wait".to_string(),
            (None, Err(error)) => return Err(format!("RT_HANDOFF_BENCH_MODE is invalid: {error}")),
        };

        match mode_str.as_str() {
            "real_wait" => Ok(Self::RealWait),
            "phase_a_synthetic_target_plus_one_tick" => Ok(Self::PhaseASyntheticTargetPlusOneTick),
            "phase_a_sender_only" => Ok(Self::PhaseASenderOnly),
            "phase_a_production_boundary" => Ok(Self::PhaseAProductionBoundary),
            value => Err(format!(
                "mode must be real_wait, phase_a_synthetic_target_plus_one_tick, phase_a_sender_only, or phase_a_production_boundary, got {value:?}"
            )),
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

#[derive(Clone, Default)]
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
    wake_to_final_policy_us: Vec<i64>,
    final_policy_to_pre_call_us: Vec<i64>,
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
    planned_wait_gap_us: Vec<u64>,
    wait_wake_lateness_us: Vec<i64>,
    hot_wait_count: usize,
    cold_wait_count: usize,
    missed_pre_call_lateness_us: Vec<i64>,
    missed_excess_beyond_latest_start_us: Vec<i64>,
    counterfactual_rescued_total_lateness_us: Vec<Vec<i64>>,
    counterfactual_rescued_excess_lateness_us: Vec<Vec<i64>>,
    counterfactual_residual_lateness_us: Vec<Vec<i64>>,
    missed_down_unobserved_backlog: usize,
    missed_down_physical_window_expired: usize,
    missed_down_final_sender_window_expired: usize,
    late_rescued_down_boundaries: usize,
    late_rescued_down_keys: usize,
    late_rescued_down_lateness_us: Vec<u64>,
    late_rescued_down_excess_us: Vec<u64>,
    transport_anomaly_count: usize,
    spin_time_us: Vec<u64>,
    wall_time_us: Vec<u64>,
}

impl Samples {
    fn append(&mut self, mut other: Self) {
        macro_rules! append_vecs {
            ($($field:ident),+ $(,)?) => {
                $(self.$field.append(&mut other.$field);)+
            };
        }
        append_vecs!(
            plan_build_us,
            packet_header_reads_per_plan,
            expected_up_intents_per_plan,
            expected_down_intents_per_plan,
            up_intent_visits_per_plan,
            down_intent_visits_per_plan,
            secondary_batch_visits_per_plan,
            secondary_batch_visit_bounds_per_plan,
            intent_visits_per_plan,
            registry_lookups_per_plan,
            view_packet_calls_per_plan,
            commit_freeze_calls_per_plan,
            admission_wake_to_precision_wake_us,
            target_crossing_error_us,
            target_crossing_to_final_policy_us,
            wake_to_final_policy_us,
            final_policy_to_pre_call_us,
            final_policy_to_true_pre_call_us,
            dispatch_start_error_us,
            pre_call_to_completion_us,
            completion_to_rt_ready_us,
            target_to_completion_us,
            completion_error_us,
            planned_wait_gap_us,
            wait_wake_lateness_us,
            missed_pre_call_lateness_us,
            missed_excess_beyond_latest_start_us,
            late_rescued_down_lateness_us,
            late_rescued_down_excess_us,
            spin_time_us,
            wall_time_us,
        );
        for index in 0..LATE_RESCUE_GRACES_US.len() {
            self.counterfactual_rescued_total_lateness_us[index]
                .append(&mut other.counterfactual_rescued_total_lateness_us[index]);
            self.counterfactual_rescued_excess_lateness_us[index]
                .append(&mut other.counterfactual_rescued_excess_lateness_us[index]);
            self.counterfactual_residual_lateness_us[index]
                .append(&mut other.counterfactual_residual_lateness_us[index]);
        }
        self.physical_dispatches = self
            .physical_dispatches
            .saturating_add(other.physical_dispatches);
        self.wait_count = self.wait_count.saturating_add(other.wait_count);
        self.overdue_dispatch_count = self
            .overdue_dispatch_count
            .saturating_add(other.overdue_dispatch_count);
        self.early_dispatch_count = self
            .early_dispatch_count
            .saturating_add(other.early_dispatch_count);
        self.non_dispatches = self.non_dispatches.saturating_add(other.non_dispatches);
        self.deadline_missed_count = self
            .deadline_missed_count
            .saturating_add(other.deadline_missed_count);
        self.observation_count = self
            .observation_count
            .saturating_add(other.observation_count);
        self.observation_gaps = self.observation_gaps.saturating_add(other.observation_gaps);
        self.hot_wait_count = self.hot_wait_count.saturating_add(other.hot_wait_count);
        self.cold_wait_count = self.cold_wait_count.saturating_add(other.cold_wait_count);
        self.missed_down_unobserved_backlog = self
            .missed_down_unobserved_backlog
            .saturating_add(other.missed_down_unobserved_backlog);
        self.missed_down_physical_window_expired = self
            .missed_down_physical_window_expired
            .saturating_add(other.missed_down_physical_window_expired);
        self.missed_down_final_sender_window_expired = self
            .missed_down_final_sender_window_expired
            .saturating_add(other.missed_down_final_sender_window_expired);
        self.late_rescued_down_boundaries = self
            .late_rescued_down_boundaries
            .saturating_add(other.late_rescued_down_boundaries);
        self.late_rescued_down_keys = self
            .late_rescued_down_keys
            .saturating_add(other.late_rescued_down_keys);
        self.transport_anomaly_count = self
            .transport_anomaly_count
            .saturating_add(other.transport_anomaly_count);
        for (reason, count) in other.failure_reasons {
            *self.failure_reasons.entry(reason).or_default() += count;
        }
    }

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
                    "down_physical_window_expired" | "down_unobserved_backlog"
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

fn new_samples() -> Samples {
    Samples {
        counterfactual_rescued_total_lateness_us: (0..LATE_RESCUE_GRACES_US.len())
            .map(|_| Vec::new())
            .collect(),
        counterfactual_rescued_excess_lateness_us: (0..LATE_RESCUE_GRACES_US.len())
            .map(|_| Vec::new())
            .collect(),
        counterfactual_residual_lateness_us: (0..LATE_RESCUE_GRACES_US.len())
            .map(|_| Vec::new())
            .collect(),
        ..Samples::default()
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
    let max_positive = values.iter().copied().filter(|value| *value > 0).max();
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
        "max_positive": max_positive,
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
            if let Some(wake_qpc) = value.wake_qpc {
                samples.wake_to_final_policy_us.push(signed_qpc_us(
                    qpc_clock,
                    value.final_policy_qpc,
                    wake_qpc,
                ));
            }
            samples.final_policy_to_pre_call_us.push(signed_qpc_us(
                qpc_clock,
                value.pre_call_qpc,
                value.final_policy_qpc,
            ));
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
        DispatchObservation::DownMiss(value) => {
            let reason = match value.kind {
                DownMissKind::UnobservedBacklog => {
                    samples.missed_down_unobserved_backlog += 1;
                    "down_unobserved_backlog"
                }
                DownMissKind::PhysicalWindowExpired => {
                    samples.missed_down_physical_window_expired += 1;
                    "down_physical_window_expired"
                }
                DownMissKind::DownExpiredBeforeSend => {
                    samples.missed_down_final_sender_window_expired += 1;
                    let total_lateness_us = signed_qpc_us(
                        qpc_clock,
                        value.observed_qpc,
                        value.physical_authored_target_qpc(),
                    );
                    samples.missed_pre_call_lateness_us.push(total_lateness_us);
                    let Some(latest_down_start_qpc) = value.physical_latest_down_start_qpc() else {
                        samples.record_observation_failure(
                            "down_final_sender_window_expired_missing_latest_start",
                        );
                        return;
                    };
                    let excess_lateness_us =
                        signed_qpc_us(qpc_clock, value.observed_qpc, latest_down_start_qpc);
                    samples
                        .missed_excess_beyond_latest_start_us
                        .push(excess_lateness_us);
                    for (index, grace_us) in LATE_RESCUE_GRACES_US.iter().copied().enumerate() {
                        let grace_ticks = qpc_clock
                            .duration_from_us(grace_us)
                            .expect("late-rescue grace conversion");
                        if latest_down_start_qpc
                            .checked_add_duration(grace_ticks)
                            .is_ok_and(|cutoff| value.observed_qpc <= cutoff)
                        {
                            samples.counterfactual_rescued_total_lateness_us[index]
                                .push(total_lateness_us);
                            samples.counterfactual_rescued_excess_lateness_us[index]
                                .push(excess_lateness_us);
                        } else {
                            samples.counterfactual_residual_lateness_us[index]
                                .push(total_lateness_us);
                        }
                    }
                    "down_final_sender_window_expired"
                }
            };
            // A recovered Down miss is a diagnostic companion to the
            // successful recovery dispatch, not a second failed benchmark
            // attempt. Keep it in the evidence/failure-reason report without
            // double-counting the attempt accounting.
            samples.record_observation_failure("down_missed");
            samples.record_observation_failure(reason);
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
            if let Some(wake_qpc) = value.wake_qpc {
                samples.wake_to_final_policy_us.push(signed_qpc_us(
                    qpc_clock,
                    value.final_policy_qpc,
                    wake_qpc,
                ));
            }
            samples.final_policy_to_pre_call_us.push(signed_qpc_us(
                qpc_clock,
                value.pre_call_qpc,
                value.final_policy_qpc,
            ));
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
            record_wait_evidence(samples, harness)?;
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

fn record_c1_harness_metrics(
    samples: &mut Samples,
    harness: &mut ProductionDispatchTestHarness,
) -> Result<(), String> {
    let (boundaries, keys, lateness_us, excess_us) =
        harness.late_rescued_down_metrics_us_for_test()?;
    samples.late_rescued_down_boundaries = samples
        .late_rescued_down_boundaries
        .saturating_add(usize::try_from(boundaries).unwrap_or(usize::MAX));
    samples.late_rescued_down_keys = samples
        .late_rescued_down_keys
        .saturating_add(usize::try_from(keys).unwrap_or(usize::MAX));
    if boundaries != 0 {
        samples.late_rescued_down_lateness_us.push(lateness_us);
        samples.late_rescued_down_excess_us.push(excess_us);
    }
    let (partial, zero_progress, integrity_lost) = harness.transport_anomaly_counts_for_test();
    let anomalies = partial
        .saturating_add(zero_progress)
        .saturating_add(integrity_lost);
    samples.transport_anomaly_count = samples
        .transport_anomaly_count
        .saturating_add(usize::try_from(anomalies).unwrap_or(usize::MAX));
    Ok(())
}

#[derive(Clone, Copy, Default)]
struct HarnessMetricsSnapshot {
    rescued_boundaries: u64,
    rescued_keys: u64,
    transport_anomalies: u64,
}

fn harness_metrics_snapshot(
    harness: &mut ProductionDispatchTestHarness,
) -> Result<HarnessMetricsSnapshot, String> {
    let (rescued_boundaries, rescued_keys, _, _) =
        harness.late_rescued_down_metrics_us_for_test()?;
    let (partial, zero_progress, integrity_lost) = harness.transport_anomaly_counts_for_test();
    Ok(HarnessMetricsSnapshot {
        rescued_boundaries,
        rescued_keys,
        transport_anomalies: partial
            .saturating_add(zero_progress)
            .saturating_add(integrity_lost),
    })
}

fn record_c1_harness_metrics_delta(
    samples: &mut Samples,
    harness: &mut ProductionDispatchTestHarness,
    previous: &mut HarnessMetricsSnapshot,
) -> Result<(), String> {
    let current = harness_metrics_snapshot(harness)?;
    samples.late_rescued_down_boundaries = samples.late_rescued_down_boundaries.saturating_add(
        usize::try_from(
            current
                .rescued_boundaries
                .saturating_sub(previous.rescued_boundaries),
        )
        .unwrap_or(usize::MAX),
    );
    samples.late_rescued_down_keys = samples.late_rescued_down_keys.saturating_add(
        usize::try_from(current.rescued_keys.saturating_sub(previous.rescued_keys))
            .unwrap_or(usize::MAX),
    );
    samples.transport_anomaly_count = samples.transport_anomaly_count.saturating_add(
        usize::try_from(
            current
                .transport_anomalies
                .saturating_sub(previous.transport_anomalies),
        )
        .unwrap_or(usize::MAX),
    );
    *previous = current;
    Ok(())
}

#[derive(Clone)]
struct SequentialSamples {
    samples: Samples,
    sequence_count: usize,
    first_actual_successful_sends: usize,
    first_rescued_boundaries: usize,
    first_final_sender_window_expired: usize,
    first_unobserved_backlog: usize,
    first_physical_window_expired: usize,
    first_transport_anomalies: usize,
    second_successful_sends: usize,
    second_final_sender_window_expired: usize,
    second_unobserved_backlog: usize,
    second_overdue_boundaries: usize,
    second_physical_window_expired: usize,
    second_transport_anomalies: usize,
    timeline_rebases: usize,
    post_send_ready_latency_us: Vec<i64>,
}

impl Default for SequentialSamples {
    fn default() -> Self {
        Self {
            samples: new_samples(),
            sequence_count: 0,
            first_actual_successful_sends: 0,
            first_rescued_boundaries: 0,
            first_final_sender_window_expired: 0,
            first_unobserved_backlog: 0,
            first_physical_window_expired: 0,
            first_transport_anomalies: 0,
            second_successful_sends: 0,
            second_final_sender_window_expired: 0,
            second_unobserved_backlog: 0,
            second_overdue_boundaries: 0,
            second_physical_window_expired: 0,
            second_transport_anomalies: 0,
            timeline_rebases: 0,
            post_send_ready_latency_us: Vec::new(),
        }
    }
}

impl SequentialSamples {
    fn append(&mut self, mut other: Self) {
        self.samples.append(other.samples);
        self.sequence_count = self.sequence_count.saturating_add(other.sequence_count);
        self.first_actual_successful_sends = self
            .first_actual_successful_sends
            .saturating_add(other.first_actual_successful_sends);
        self.first_rescued_boundaries = self
            .first_rescued_boundaries
            .saturating_add(other.first_rescued_boundaries);
        self.first_final_sender_window_expired = self
            .first_final_sender_window_expired
            .saturating_add(other.first_final_sender_window_expired);
        self.first_unobserved_backlog = self
            .first_unobserved_backlog
            .saturating_add(other.first_unobserved_backlog);
        self.first_physical_window_expired = self
            .first_physical_window_expired
            .saturating_add(other.first_physical_window_expired);
        self.first_transport_anomalies = self
            .first_transport_anomalies
            .saturating_add(other.first_transport_anomalies);
        self.second_successful_sends = self
            .second_successful_sends
            .saturating_add(other.second_successful_sends);
        self.second_final_sender_window_expired = self
            .second_final_sender_window_expired
            .saturating_add(other.second_final_sender_window_expired);
        self.second_unobserved_backlog = self
            .second_unobserved_backlog
            .saturating_add(other.second_unobserved_backlog);
        self.second_overdue_boundaries = self
            .second_overdue_boundaries
            .saturating_add(other.second_overdue_boundaries);
        self.second_physical_window_expired = self
            .second_physical_window_expired
            .saturating_add(other.second_physical_window_expired);
        self.second_transport_anomalies = self
            .second_transport_anomalies
            .saturating_add(other.second_transport_anomalies);
        self.timeline_rebases = self.timeline_rebases.saturating_add(other.timeline_rebases);
        self.post_send_ready_latency_us
            .append(&mut other.post_send_ready_latency_us);
    }
}

fn record_wait_evidence(
    samples: &mut Samples,
    harness: &ProductionDispatchTestHarness,
) -> Result<(), String> {
    let Some(observation) = harness.last_wait_observation() else {
        return Ok(());
    };
    if !matches!(observation.outcome, WaitOutcome::Deadline) {
        return Ok(());
    }
    let qpc_clock = QpcClock::initialize().map_err(|error| format!("QPC: {error:?}"))?;
    samples.planned_wait_gap_us.push(
        qpc_clock
            .duration_to_us(observation.planned_wait_ticks)
            .map_err(|error| format!("planned wait conversion: {error:?}"))?,
    );
    let cold_threshold_ticks = qpc_clock
        .duration_from_us(SEND_COLD_THRESHOLD_US)
        .map_err(|error| format!("cold threshold conversion: {error:?}"))?;
    if observation.planned_wait_ticks < cold_threshold_ticks {
        samples.hot_wait_count += 1;
    } else {
        samples.cold_wait_count += 1;
    }
    if let Some(wake_qpc) = observation.wake_qpc {
        samples.wait_wake_lateness_us.push(signed_qpc_us(
            qpc_clock,
            wake_qpc,
            observation.physical_target_qpc,
        ));
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
    run_down_with_gap(key_count, mode, benchmark_mode, due_us())
}

fn run_down_iteration(
    samples: &mut Samples,
    key_count: usize,
    mode: WaitMode,
    benchmark_mode: BenchmarkMode,
    gap_us: u64,
) -> Result<(), String> {
    run_down_iteration_with_tolerance(samples, key_count, mode, benchmark_mode, gap_us, 0)
}

fn run_down_iteration_with_tolerance(
    samples: &mut Samples,
    key_count: usize,
    mode: WaitMode,
    benchmark_mode: BenchmarkMode,
    gap_us: u64,
    continuity_tolerance_us: u64,
) -> Result<(), String> {
    let iteration_started = Instant::now();
    let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(key_count, gap_us);
    harness.enable_dispatch_ready_timing_for_benchmark();
    let alignment_margin_us = if matches!(benchmark_mode, BenchmarkMode::PhaseAProductionBoundary) {
        0
    } else {
        gap_us
    };
    harness.align_next_plan_to_benchmark_margin_for_test(alignment_margin_us);
    harness.configure_production_wait_policy(mode.effective_spin_threshold_us)?;
    harness.configure_normal_down_start_tolerance_for_test(continuity_tolerance_us)?;
    harness.reset_preparation_counts_for_test();
    let mut plan = NextDispatchPlan::default();
    let plan_started = Instant::now();
    plan_projected(&mut harness, &mut plan);
    record_preparation_sample(
        samples,
        harness.preparation_counts(),
        elapsed_ns(plan_started),
    );
    if wait_and_dispatch_or_record(&mut harness, &plan, benchmark_mode, samples)?.is_none() {
        record_c1_harness_metrics(samples, &mut harness)?;
        samples
            .wall_time_us
            .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
        return Ok(());
    }
    samples.physical_dispatches += 1;
    record_wait_metrics(samples, &harness, benchmark_mode)?;
    drain_observations(&mut harness, samples);
    record_c1_harness_metrics(samples, &mut harness)?;
    samples
        .wall_time_us
        .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
    Ok(())
}

fn run_down_with_gap(
    key_count: usize,
    mode: WaitMode,
    benchmark_mode: BenchmarkMode,
    gap_us: u64,
) -> Result<Samples, String> {
    let mut samples = new_samples();
    for _ in 0..iterations() {
        run_down_iteration(&mut samples, key_count, mode, benchmark_mode, gap_us)?;
    }
    Ok(samples)
}

fn run_up(
    key_count: usize,
    mode: WaitMode,
    benchmark_mode: BenchmarkMode,
) -> Result<Samples, String> {
    let mut samples = new_samples();
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
    let mut samples = new_samples();
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
    let mut samples = new_samples();
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

fn phase_a_sparse_gap_report() -> serde_json::Value {
    let mode = build_wait_mode("phase_a_sparse_gap", true, true, true);
    let benchmark_mode = BenchmarkMode::RealWait;
    let mode_started = Instant::now();
    let cpu_started_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    let gaps_us = [5_000, 20_000, 25_000, 100_000, 250_000, 500_000, 1_000_000];
    let key_counts = [1, 5, 15];
    let mut scenarios = serde_json::Map::new();
    for gap_us in gaps_us {
        for key_count in key_counts {
            scenarios.insert(
                format!("down_only_{key_count}_gap_{gap_us}us"),
                summarize(
                    run_down_with_gap(key_count, mode, benchmark_mode, gap_us)
                        .unwrap_or_else(|error| panic!("{error}")),
                ),
            );
        }
    }
    let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    serde_json::json!({
        "scope": "Phase-A sparse-gap real HybridWaiter evidence; authored targets and wait policy are unchanged",
        "waitable_timer_enabled": mode.waitable_timer_enabled,
        "event_wait_enabled": mode.event_wait_enabled,
        "adaptive_spin_enabled": mode.adaptive_spin_enabled,
        "spin_floor_us": sky_player::engine::dispatch_primitives::PRODUCTION_MIN_SPIN_THRESHOLD_US,
        "calibration_samples": sky_player::engine::dispatch_primitives::PRODUCTION_CALIBRATION_SAMPLES,
        "calibration_budget_us": sky_player::engine::dispatch_primitives::PRODUCTION_CALIBRATION_BUDGET_US,
        "startup_readiness_reserve_us": sky_player::engine::dispatch_primitives::PRODUCTION_STARTUP_READINESS_RESERVE_US,
        "startup_kernel_timer_wake_error_us": wake_error_json(mode.startup_wake_error),
        "effective_spin_threshold_us": mode.effective_spin_threshold_us,
        "cold_gap_threshold_us": SEND_COLD_THRESHOLD_US,
        "gap_matrix_us": gaps_us,
        "key_count_matrix": key_counts,
        "transport": "deterministic mock transport; waiter timing is real",
        "process_cpu_time_us": cpu_finished_us.saturating_sub(cpu_started_us),
        "process_cpu_duty_percent": cpu_duty_percent(cpu_started_us, cpu_finished_us, mode_started),
        "scenarios": scenarios,
        "iterations": iterations(),
    })
}

fn phase_b0_wait_modes() -> Vec<WaitMode> {
    vec![
        build_wait_mode("production_calibrated", true, true, true),
        build_fixed_wait_mode("fixed_spin_500us", 500),
        build_fixed_wait_mode("fixed_spin_750us", 750),
        build_fixed_wait_mode("fixed_spin_1000us", 1_000),
        build_fixed_wait_mode("fixed_spin_1500us", 1_500),
        build_fixed_wait_mode("fixed_spin_2000us", 2_000),
    ]
}

fn run_phase_b0_interleaved_scenario(
    key_count: usize,
    gap_us: u64,
    scenario_index: usize,
    modes: &[WaitMode],
) -> Result<Vec<Samples>, String> {
    let mut samples = vec![new_samples(); modes.len()];
    for iteration in 0..iterations() {
        let first_mode = (scenario_index + iteration) % modes.len();
        for offset in 0..modes.len() {
            let mode_index = (first_mode + offset) % modes.len();
            run_down_iteration(
                &mut samples[mode_index],
                key_count,
                modes[mode_index],
                BenchmarkMode::RealWait,
                gap_us,
            )?;
        }
    }
    Ok(samples)
}

fn phase_b0_report() -> serde_json::Value {
    let modes = phase_b0_wait_modes();
    let benchmark_started = Instant::now();
    let cpu_started_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    let timing_margin_us = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 0)
        .timing_margin_us_for_benchmark()
        .unwrap_or_else(|error| panic!("{error}"));
    let gaps_us = [5_000, 20_000, 25_000, 100_000, 250_000, 500_000, 1_000_000];
    let key_counts = [1, 5, 15];
    let scenario_count = gaps_us.len() * key_counts.len();
    let mut scenarios_by_mode = vec![serde_json::Map::new(); modes.len()];
    let mut aggregate_by_mode = vec![new_samples(); modes.len()];
    let mut scenario_index = 0;
    for gap_us in gaps_us {
        for key_count in key_counts {
            let scenario_samples =
                run_phase_b0_interleaved_scenario(key_count, gap_us, scenario_index, &modes)
                    .unwrap_or_else(|error| panic!("{error}"));
            let scenario_name = format!("down_only_{key_count}_gap_{gap_us}us");
            for (mode_index, samples) in scenario_samples.into_iter().enumerate() {
                scenarios_by_mode[mode_index]
                    .insert(scenario_name.clone(), summarize(samples.clone()));
                aggregate_by_mode[mode_index].append(samples);
            }
            scenario_index += 1;
        }
    }
    let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    let mode_order = modes.iter().map(|mode| mode.name).collect::<Vec<_>>();
    let mode_reports = modes
        .iter()
        .zip(scenarios_by_mode)
        .zip(aggregate_by_mode)
        .map(|((mode, scenarios), aggregate)| {
            let configured_spin_threshold_us = if mode.name == "production_calibrated" {
                serde_json::Value::String("startup_calibrated".to_string())
            } else {
                json!(mode.effective_spin_threshold_us)
            };
            (
                mode.name.to_string(),
                json!({
                    "configured_spin_threshold_us": configured_spin_threshold_us,
                    "actual_spin_threshold_us": mode.effective_spin_threshold_us,
                    "waitable_timer_enabled": mode.waitable_timer_enabled,
                    "event_wait_enabled": mode.event_wait_enabled,
                    "adaptive_spin_enabled": mode.adaptive_spin_enabled,
                    "startup_wake_error_us": wake_error_json(mode.startup_wake_error),
                    "scenarios": scenarios,
                    "aggregate": summarize_for_attempts(
                        aggregate,
                        scenario_count.saturating_mul(iterations()),
                    ),
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    json!({
        "scope": "Phase-B0 benchmark-only interleaved real HybridWaiter qualification; production wait behavior is unchanged",
        "waitable_timer_enabled": true,
        "event_wait_enabled": true,
        "waiter_constructor": "HybridWaiter::production",
        "requested_wait_policy": "benchmark_only_spin_threshold_probe",
        "effective_wait_policy": "benchmark_only_spin_threshold_probe",
        "gap_matrix_us": gaps_us,
        "key_count_matrix": key_counts,
        "iterations_per_scenario_per_mode": iterations(),
        "scenario_count": scenario_count,
        "mode_count": modes.len(),
        "mode_order": mode_order,
        "rotation": {
            "kind": "deterministic_round_robin",
            "scenario_order": "gap-major, then key-count ascending",
            "first_mode_index": "(scenario_index + iteration) mod mode_count",
            "within_iteration": "each mode runs once in mode_order wrapping at mode_count",
        },
        "timing_window": {
            "timing_margin_us": timing_margin_us,
            "latest_down_start_allowance_us": timing_margin_us,
            "latest_down_start_definition": "physical authored target QPC + Timing Margin",
        },
        "cold_gap_threshold_us": SEND_COLD_THRESHOLD_US,
        "transport": "deterministic mock transport; waiter timing is real",
        "process_cpu_time_us": cpu_finished_us.saturating_sub(cpu_started_us),
        "process_cpu_duty_percent": cpu_duty_percent(
            cpu_started_us,
            cpu_finished_us,
            benchmark_started,
        ),
        "modes": mode_reports,
    })
}

fn counterfactual_rescue_summary(samples: &Samples) -> serde_json::Value {
    let total_misses = samples.missed_down_final_sender_window_expired;
    let candidates = LATE_RESCUE_GRACES_US
        .iter()
        .enumerate()
        .map(|(index, grace_us)| {
            let rescued_total_lateness = &samples.counterfactual_rescued_total_lateness_us[index];
            let rescued_excess_lateness =
                &samples.counterfactual_rescued_excess_lateness_us[index];
            let residual_lateness = &samples.counterfactual_residual_lateness_us[index];
            let rescued_count = rescued_total_lateness.len();
            let residual_count = residual_lateness.len();
            let rescued_rate_percent = if total_misses == 0 {
                0.0
            } else {
                rescued_count as f64 * 100.0 / total_misses as f64
            };
            let residual_rate_percent = if total_misses == 0 {
                0.0
            } else {
                residual_count as f64 * 100.0 / total_misses as f64
            };
            json!({
                "grace_us": grace_us,
                "total_final_sender_misses": total_misses,
                "would_rescue_count": rescued_count,
                "would_rescue_rate_percent": rescued_rate_percent,
                "residual_hard_stale_count": residual_count,
                "residual_hard_stale_rate_percent": residual_rate_percent,
                "rescued_total_pre_call_lateness_us": signed_summary(rescued_total_lateness.clone()),
                "rescued_excess_beyond_latest_start_us": signed_summary(rescued_excess_lateness.clone()),
                "residual_total_pre_call_lateness_us": signed_summary(residual_lateness.clone()),
            })
        })
        .collect::<Vec<_>>();
    json!({ "grace_candidates": candidates })
}

fn phase_c0_report() -> serde_json::Value {
    let benchmark_started = Instant::now();
    let cpu_started_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    let gaps_us = [5_000, 20_000, 25_000, 100_000, 250_000, 500_000, 1_000_000];
    let key_counts = [1, 5, 15];
    let scenario_count = gaps_us.len() * key_counts.len();
    let expected_attempts_per_scenario = C0_PASSES.saturating_mul(iterations());
    let expected_total_attempts = scenario_count.saturating_mul(expected_attempts_per_scenario);
    let timing_margin_us = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 0)
        .timing_margin_us_for_benchmark()
        .unwrap_or_else(|error| panic!("{error}"));
    let mut aggregate_by_scenario = (0..scenario_count)
        .map(|_| new_samples())
        .collect::<Vec<_>>();
    let mut aggregate_all = new_samples();
    let mut pass_reports = Vec::with_capacity(C0_PASSES);
    let mut actual_spin_threshold_us_by_pass = Vec::with_capacity(C0_PASSES);

    for pass_index in 0..C0_PASSES {
        let mode = build_wait_mode("production_calibrated", true, true, true);
        let start_scenario_index = (pass_index * 7) % scenario_count;
        let mut pass_scenarios = serde_json::Map::new();
        let mut pass_aggregate = new_samples();
        let mut scenario_order = Vec::with_capacity(scenario_count);
        for offset in 0..scenario_count {
            let scenario_index = (start_scenario_index + offset) % scenario_count;
            let gap_us = gaps_us[scenario_index / key_counts.len()];
            let key_count = key_counts[scenario_index % key_counts.len()];
            let scenario_name = format!("down_only_{key_count}_gap_{gap_us}us");
            let samples = run_down_with_gap(key_count, mode, BenchmarkMode::RealWait, gap_us)
                .unwrap_or_else(|error| panic!("{error}"));
            scenario_order.push(scenario_name.clone());
            pass_scenarios.insert(scenario_name, summarize(samples.clone()));
            pass_aggregate.append(samples.clone());
            aggregate_by_scenario[scenario_index].append(samples.clone());
            aggregate_all.append(samples);
        }
        actual_spin_threshold_us_by_pass.push(mode.effective_spin_threshold_us);
        pass_reports.push(json!({
            "pass": pass_index + 1,
            "start_scenario_index": start_scenario_index,
            "scenario_order": scenario_order,
            "iterations_per_scenario": iterations(),
            "actual_spin_threshold_us": mode.effective_spin_threshold_us,
            "startup_wake_error_us": wake_error_json(mode.startup_wake_error),
            "scenarios": pass_scenarios,
            "aggregate": summarize_for_attempts(
                pass_aggregate,
                scenario_count.saturating_mul(iterations()),
            ),
        }));
    }

    let mut aggregate_scenarios = serde_json::Map::new();
    for (scenario_index, samples) in aggregate_by_scenario.into_iter().enumerate() {
        let gap_us = gaps_us[scenario_index / key_counts.len()];
        let key_count = key_counts[scenario_index % key_counts.len()];
        aggregate_scenarios.insert(
            format!("down_only_{key_count}_gap_{gap_us}us"),
            summarize_for_attempts(samples, expected_attempts_per_scenario),
        );
    }

    let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    json!({
        "scope": "Phase-C0 calibrated-only stability benchmark plus counterfactual late-rescue analysis; production behavior is unchanged",
        "waitable_timer_enabled": true,
        "event_wait_enabled": true,
        "adaptive_spin_enabled": true,
        "waiter_constructor": "HybridWaiter::production",
        "production_wait_policy": "startup_calibrated",
        "sender_cutoff_policy": "current latest_down_start unchanged; 500us Timing Margin",
        "gap_matrix_us": gaps_us,
        "key_count_matrix": key_counts,
        "pass_count": C0_PASSES,
        "iterations_per_scenario_per_pass": iterations(),
        "scenario_count": scenario_count,
        "total_attempts": expected_total_attempts,
        "scenario_rotation": {
            "kind": "deterministic pass rotation",
            "scenario_order": "gap-major, then key-count ascending",
            "pass_start_indices": "0, 7, 14",
            "within_pass": "each pass visits every scenario once, wrapping at scenario_count",
        },
        "timing_window": {
            "timing_margin_us": timing_margin_us,
            "latest_down_start_allowance_us": timing_margin_us,
            "latest_down_start_definition": "physical authored target QPC + Timing Margin",
        },
        "late_rescue_probes_us": LATE_RESCUE_GRACES_US,
        "transport": "deterministic mock transport; waiter timing is real",
        "actual_spin_threshold_us_by_pass": actual_spin_threshold_us_by_pass,
        "passes": pass_reports,
        "scenarios": aggregate_scenarios,
        "aggregate": summarize_for_attempts(aggregate_all, expected_total_attempts),
        "process_cpu_time_us": cpu_finished_us.saturating_sub(cpu_started_us),
        "process_cpu_duty_percent": cpu_duty_percent(
            cpu_started_us,
            cpu_finished_us,
            benchmark_started,
        ),
    })
}

fn run_phase_c1_interleaved_scenario(
    key_count: usize,
    gap_us: u64,
    scenario_index: usize,
    mode: WaitMode,
) -> Result<[Samples; 2], String> {
    let mut samples = [new_samples(), new_samples()];
    for iteration in 0..iterations() {
        let first_arm = (scenario_index + iteration) % 2;
        for offset in 0..2 {
            let arm = (first_arm + offset) % 2;
            let tolerance_us = if arm == 0 { 0 } else { C1_NORMAL_TOLERANCE_US };
            run_down_iteration_with_tolerance(
                &mut samples[arm],
                key_count,
                mode,
                BenchmarkMode::RealWait,
                gap_us,
                tolerance_us,
            )?;
        }
    }
    Ok(samples)
}

fn phase_c1_report() -> serde_json::Value {
    let benchmark_started = Instant::now();
    let cpu_started_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    let gaps_us = [5_000, 20_000, 25_000, 100_000, 250_000, 500_000, 1_000_000];
    let key_counts = [1, 5, 15];
    let scenario_count = gaps_us.len() * key_counts.len();
    let expected_attempts_per_scenario = C1_PASSES.saturating_mul(iterations());
    let expected_attempts_per_pass_per_arm = scenario_count.saturating_mul(iterations());
    let expected_total_attempts = scenario_count
        .saturating_mul(C1_PASSES)
        .saturating_mul(iterations())
        .saturating_mul(2);
    let timing_margin_us = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 0)
        .timing_margin_us_for_benchmark()
        .unwrap_or_else(|error| panic!("{error}"));
    let mut aggregate_by_scenario = (0..scenario_count)
        .map(|_| [new_samples(), new_samples()])
        .collect::<Vec<_>>();
    let mut aggregate_all = [new_samples(), new_samples()];
    let mut pass_reports = Vec::with_capacity(C1_PASSES);
    let mut actual_spin_threshold_us_by_pass = Vec::with_capacity(C1_PASSES);

    for pass_index in 0..C1_PASSES {
        let mode = build_wait_mode("production_calibrated", true, true, true);
        let start_scenario_index = (pass_index * 7) % scenario_count;
        let mut pass_scenarios = [serde_json::Map::new(), serde_json::Map::new()];
        let mut pass_aggregate = [new_samples(), new_samples()];
        let mut scenario_order = Vec::with_capacity(scenario_count);
        for offset in 0..scenario_count {
            let scenario_index = (start_scenario_index + offset) % scenario_count;
            let gap_us = gaps_us[scenario_index / key_counts.len()];
            let key_count = key_counts[scenario_index % key_counts.len()];
            let scenario_name = format!("down_only_{key_count}_gap_{gap_us}us");
            let samples =
                run_phase_c1_interleaved_scenario(key_count, gap_us, scenario_index, mode)
                    .unwrap_or_else(|error| panic!("{error}"));
            scenario_order.push(scenario_name.clone());
            for arm in 0..2 {
                pass_scenarios[arm].insert(scenario_name.clone(), summarize(samples[arm].clone()));
                pass_aggregate[arm].append(samples[arm].clone());
                aggregate_by_scenario[scenario_index][arm].append(samples[arm].clone());
                aggregate_all[arm].append(samples[arm].clone());
            }
        }
        actual_spin_threshold_us_by_pass.push(mode.effective_spin_threshold_us);
        pass_reports.push(json!({
            "pass": pass_index + 1,
            "start_scenario_index": start_scenario_index,
            "scenario_order": scenario_order,
            "iterations_per_scenario_per_arm": iterations(),
            "actual_spin_threshold_us": mode.effective_spin_threshold_us,
            "startup_wake_error_us": wake_error_json(mode.startup_wake_error),
            "arms": {
                "current_cutoff": {
                    "sender_cutoff_policy": "physical_latest_down_start",
                    "scenarios": pass_scenarios[0],
                    "aggregate": summarize_for_attempts(
                        pass_aggregate[0].clone(),
                        expected_attempts_per_pass_per_arm,
                    ),
                },
                "c1_non_additive_2500us": {
                    "sender_cutoff_policy": "max(physical_latest_down_start, authored_target_plus_2500us)",
                    "scenarios": pass_scenarios[1],
                    "aggregate": summarize_for_attempts(
                        pass_aggregate[1].clone(),
                        expected_attempts_per_pass_per_arm,
                    ),
                },
            },
        }));
    }

    let mut aggregate_scenarios = [serde_json::Map::new(), serde_json::Map::new()];
    for (scenario_index, samples) in aggregate_by_scenario.into_iter().enumerate() {
        let gap_us = gaps_us[scenario_index / key_counts.len()];
        let key_count = key_counts[scenario_index % key_counts.len()];
        let scenario_name = format!("down_only_{key_count}_gap_{gap_us}us");
        for arm in 0..2 {
            aggregate_scenarios[arm].insert(
                scenario_name.clone(),
                summarize_for_attempts(samples[arm].clone(), expected_attempts_per_scenario),
            );
        }
    }

    let modes = json!({
        "current_cutoff": {
            "sender_cutoff_policy": "physical_latest_down_start",
            "scenarios": aggregate_scenarios[0],
            "aggregate": summarize_for_attempts(
                aggregate_all[0].clone(),
                expected_attempts_per_scenario.saturating_mul(scenario_count),
            ),
        },
        "c1_non_additive_2500us": {
            "sender_cutoff_policy": "max(physical_latest_down_start, authored_target_plus_2500us)",
            "scenarios": aggregate_scenarios[1],
            "aggregate": summarize_for_attempts(
                aggregate_all[1].clone(),
                expected_attempts_per_scenario.saturating_mul(scenario_count),
            ),
        },
    });
    let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    json!({
        "scope": "Phase-C1 paired A/B real HybridWaiter qualification; C1 normal playback cutoff is enabled only on arm B",
        "waitable_timer_enabled": true,
        "event_wait_enabled": true,
        "adaptive_spin_enabled": true,
        "waiter_constructor": "HybridWaiter::production",
        "production_spin_policy": "startup_calibrated_unchanged",
        "sender_cutoff_policies": {
            "current_cutoff": "physical_latest_down_start",
            "c1_non_additive_2500us": "max(physical_latest_down_start, authored_target_plus_2500us)",
        },
        "strict_sender_cutoff": "physical_latest_down_start",
        "physical_timing_window": "unchanged; PhysicalWindowExpired and UnobservedBacklog are never rescued",
        "normal_playback_total_tolerance_us": C1_NORMAL_TOLERANCE_US,
        "timing_margin_us": timing_margin_us,
        "gap_matrix_us": gaps_us,
        "key_count_matrix": key_counts,
        "pass_count": C1_PASSES,
        "iterations_per_scenario_per_arm_per_pass": iterations(),
        "scenario_count": scenario_count,
        "total_attempts": expected_total_attempts,
        "ab_order": "rotates by (scenario_index + iteration) parity; both arms use the same pass calibration threshold",
        "transport": "deterministic mock transport; waiter timing is real",
        "actual_spin_threshold_us_by_pass": actual_spin_threshold_us_by_pass,
        "passes": pass_reports,
        "modes": modes,
        "process_cpu_time_us": cpu_finished_us.saturating_sub(cpu_started_us),
        "process_cpu_duty_percent": cpu_duty_percent(
            cpu_started_us,
            cpu_finished_us,
            benchmark_started,
        ),
    })
}

fn c1_1_arm_name(arm: usize) -> &'static str {
    match arm {
        0 => "current_cutoff",
        1 => "c1_1_non_additive_1500us",
        2 => "c1_1_non_additive_2500us",
        _ => panic!("invalid C1.1 arm {arm}"),
    }
}

fn c1_1_sender_cutoff_policy(arm: usize) -> &'static str {
    match arm {
        0 => "physical_latest_down_start",
        1 => "max(physical_latest_down_start, authored_target_plus_1500us)",
        2 => "max(physical_latest_down_start, authored_target_plus_2500us)",
        _ => panic!("invalid C1.1 arm {arm}"),
    }
}

fn run_phase_c1_1_interleaved_scenario(
    key_count: usize,
    gap_us: u64,
    scenario_index: usize,
    mode: WaitMode,
) -> Result<[Samples; 3], String> {
    let mut samples = [new_samples(), new_samples(), new_samples()];
    for iteration in 0..iterations() {
        let first_arm = (scenario_index + iteration) % C1_1_TOLERANCES_US.len();
        for offset in 0..C1_1_TOLERANCES_US.len() {
            let arm = (first_arm + offset) % C1_1_TOLERANCES_US.len();
            run_down_iteration_with_tolerance(
                &mut samples[arm],
                key_count,
                mode,
                BenchmarkMode::RealWait,
                gap_us,
                C1_1_TOLERANCES_US[arm],
            )?;
        }
    }
    Ok(samples)
}

fn phase_c1_1_tri_arm_report() -> (
    serde_json::Value,
    serde_json::Map<String, serde_json::Value>,
) {
    let cpu_started_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    let gaps_us = [5_000, 20_000, 25_000, 100_000, 250_000, 500_000, 1_000_000];
    let key_counts = [1, 5, 15];
    let scenario_count = gaps_us.len() * key_counts.len();
    let expected_attempts_per_scenario = C1_1_PASSES.saturating_mul(iterations());
    let expected_attempts_per_pass_per_arm = scenario_count.saturating_mul(iterations());
    let expected_total_attempts = scenario_count
        .saturating_mul(C1_1_PASSES)
        .saturating_mul(iterations())
        .saturating_mul(C1_1_TOLERANCES_US.len());
    let timing_margin_us = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 0)
        .timing_margin_us_for_benchmark()
        .unwrap_or_else(|error| panic!("{error}"));
    let mut aggregate_by_scenario: Vec<[Samples; 3]> = (0..scenario_count)
        .map(|_| [new_samples(), new_samples(), new_samples()])
        .collect();
    let mut aggregate_all = [new_samples(), new_samples(), new_samples()];
    let mut pass_reports = Vec::with_capacity(C1_1_PASSES);
    let mut actual_spin_threshold_us_by_pass = Vec::with_capacity(C1_1_PASSES);

    for pass_index in 0..C1_1_PASSES {
        let mode = build_wait_mode("production_calibrated", true, true, true);
        let start_scenario_index = (pass_index * 7) % scenario_count;
        let mut pass_scenarios = [
            serde_json::Map::new(),
            serde_json::Map::new(),
            serde_json::Map::new(),
        ];
        let mut pass_aggregate = [new_samples(), new_samples(), new_samples()];
        let mut scenario_order = Vec::with_capacity(scenario_count);
        for offset in 0..scenario_count {
            let scenario_index = (start_scenario_index + offset) % scenario_count;
            let gap_us = gaps_us[scenario_index / key_counts.len()];
            let key_count = key_counts[scenario_index % key_counts.len()];
            let scenario_name = format!("down_only_{key_count}_gap_{gap_us}us");
            let samples =
                run_phase_c1_1_interleaved_scenario(key_count, gap_us, scenario_index, mode)
                    .unwrap_or_else(|error| panic!("{error}"));
            scenario_order.push(scenario_name.clone());
            for arm in 0..C1_1_TOLERANCES_US.len() {
                pass_scenarios[arm].insert(scenario_name.clone(), summarize(samples[arm].clone()));
                pass_aggregate[arm].append(samples[arm].clone());
                aggregate_by_scenario[scenario_index][arm].append(samples[arm].clone());
                aggregate_all[arm].append(samples[arm].clone());
            }
        }
        actual_spin_threshold_us_by_pass.push(mode.effective_spin_threshold_us);
        let mut arms = serde_json::Map::new();
        for arm in 0..C1_1_TOLERANCES_US.len() {
            arms.insert(
                c1_1_arm_name(arm).to_string(),
                json!({
                    "sender_cutoff_policy": c1_1_sender_cutoff_policy(arm),
                    "total_tolerance_us": C1_1_TOLERANCES_US[arm],
                    "scenarios": pass_scenarios[arm].clone(),
                    "aggregate": summarize_for_attempts(
                        pass_aggregate[arm].clone(),
                        expected_attempts_per_pass_per_arm,
                    ),
                }),
            );
        }
        pass_reports.push(json!({
            "pass": pass_index + 1,
            "start_scenario_index": start_scenario_index,
            "scenario_order": scenario_order,
            "iterations_per_scenario_per_arm": iterations(),
            "actual_spin_threshold_us": mode.effective_spin_threshold_us,
            "startup_wake_error_us": wake_error_json(mode.startup_wake_error),
            "arms": arms,
        }));
    }

    let mut aggregate_scenarios = [
        serde_json::Map::new(),
        serde_json::Map::new(),
        serde_json::Map::new(),
    ];
    for (scenario_index, samples) in aggregate_by_scenario.into_iter().enumerate() {
        let gap_us = gaps_us[scenario_index / key_counts.len()];
        let key_count = key_counts[scenario_index % key_counts.len()];
        let scenario_name = format!("down_only_{key_count}_gap_{gap_us}us");
        for arm in 0..C1_1_TOLERANCES_US.len() {
            aggregate_scenarios[arm].insert(
                scenario_name.clone(),
                summarize_for_attempts(samples[arm].clone(), expected_attempts_per_scenario),
            );
        }
    }
    let mut modes = serde_json::Map::new();
    for arm in 0..C1_1_TOLERANCES_US.len() {
        modes.insert(
            c1_1_arm_name(arm).to_string(),
            json!({
                "sender_cutoff_policy": c1_1_sender_cutoff_policy(arm),
                "total_tolerance_us": C1_1_TOLERANCES_US[arm],
                "scenarios": aggregate_scenarios[arm].clone(),
                "aggregate": summarize_for_attempts(
                    aggregate_all[arm].clone(),
                    expected_attempts_per_scenario.saturating_mul(scenario_count),
                ),
            }),
        );
    }
    let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    (
        json!({
            "scope": "Phase-C1.1 tri-arm real HybridWaiter qualification; no production cutoff is changed by this report",
            "waitable_timer_enabled": true,
            "event_wait_enabled": true,
            "adaptive_spin_enabled": true,
            "waiter_constructor": "HybridWaiter::production",
            "production_spin_policy": "startup_calibrated_unchanged",
            "sender_cutoff_policies": {
                "current_cutoff": c1_1_sender_cutoff_policy(0),
                "c1_1_non_additive_1500us": c1_1_sender_cutoff_policy(1),
                "c1_1_non_additive_2500us": c1_1_sender_cutoff_policy(2),
            },
            "strict_sender_cutoff": "physical_latest_down_start",
            "physical_timing_window": "unchanged; PhysicalWindowExpired and UnobservedBacklog are never rescued",
            "timing_margin_us": timing_margin_us,
            "gap_matrix_us": gaps_us,
            "key_count_matrix": key_counts,
            "pass_count": C1_1_PASSES,
            "iterations_per_scenario_per_arm_per_pass": iterations(),
            "scenario_count": scenario_count,
            "total_attempts": expected_total_attempts,
            "arm_order": "rotates by (scenario_index + iteration) modulo three; all arms in a pass share one startup calibration threshold",
            "transport": "deterministic mock transport; waiter timing is real",
            "actual_spin_threshold_us_by_pass": actual_spin_threshold_us_by_pass,
            "passes": pass_reports,
            "modes": modes.clone(),
            "process_cpu_time_us": cpu_finished_us.saturating_sub(cpu_started_us),
        }),
        modes,
    )
}

fn captured_packet_count(packets: &Arc<Mutex<Vec<PhysicalPacket>>>) -> usize {
    packets.lock().expect("packet capture lock").len()
}

fn run_c1_1_dense_boundary_case(
    tolerance_us: u64,
    gap_us: u64,
    first_lateness_us: u64,
) -> serde_json::Value {
    let mut harness =
        ProductionDispatchTestHarness::new_dense_future_boundary_with_gap_for_test(gap_us);
    harness
        .configure_normal_down_start_tolerance_for_test(tolerance_us)
        .unwrap_or_else(|error| panic!("C1.1 dense tolerance: {error}"));
    let timing_margin_us = harness
        .timing_margin_us_for_benchmark()
        .unwrap_or_else(|error| panic!("C1.1 dense Timing Margin: {error}"));
    let packets = harness.configure_packet_capture();
    let first = harness.plan_current_dispatch();
    let first_target = harness
        .physical_target_qpc_for_test(&first)
        .expect("first dense target");
    let before_first = QpcTicks::from_raw(
        first_target
            .as_u64()
            .checked_sub(DurationTicks::from_raw(1).as_u64())
            .expect("first dense target is nonzero"),
    );
    assert!(matches!(
        harness.dispatch_at_qpc_for_test(&first, before_first),
        DispatchStep::NoWork
    ));
    let first_now = first_target
        .checked_add_duration(
            harness
                .qpc_duration_from_us_for_test(first_lateness_us)
                .expect("first dense lateness conversion"),
        )
        .expect("first dense now");
    let first_step = harness.dispatch_at_qpc_for_test(&first, first_now);
    assert!(
        matches!(first_step, DispatchStep::Dispatched),
        "first dense dispatch failed: {first_step:?}"
    );
    let first_packet_count = captured_packet_count(&packets);
    let first_allowed = first_lateness_us <= timing_margin_us.max(tolerance_us);
    let first_rescued = first_lateness_us > timing_margin_us && first_allowed;
    let first_sender_misses = harness.final_sender_window_expirations_for_test();
    assert_eq!(first_packet_count, if first_allowed { 1 } else { 0 });
    assert_eq!(first_sender_misses, u64::from(!first_allowed));
    assert_eq!(
        harness.late_rescued_down_metrics_for_test().0,
        u64::from(first_rescued)
    );

    let second = harness.plan_current_dispatch();
    let second_target = harness
        .physical_target_qpc_for_test(&second)
        .expect("second dense target");
    let authored_gap = harness
        .qpc_duration_to_us_for_test(
            second_target
                .checked_duration_since(first_target)
                .expect("second dense target follows first"),
        )
        .expect("second dense gap conversion");
    assert_eq!(authored_gap, gap_us);
    let second_is_future = second_target > first_now;
    let second_sender_misses_before = harness.final_sender_window_expirations_for_test();
    let second_backlog_before = harness.missed_unobserved_backlog_boundaries_for_test();
    let second_packets_before = captured_packet_count(&packets);
    let second_step = if second_is_future {
        let before_second = QpcTicks::from_raw(
            second_target
                .as_u64()
                .checked_sub(DurationTicks::from_raw(1).as_u64())
                .expect("second dense target is nonzero"),
        );
        assert!(matches!(
            harness.dispatch_at_qpc_for_test(&second, before_second),
            DispatchStep::NoWork
        ));
        harness.dispatch_at_qpc_for_test(&second, second_target)
    } else {
        harness.dispatch_at_qpc_for_test(&second, first_now)
    };
    assert!(
        matches!(second_step, DispatchStep::Dispatched),
        "second dense dispatch failed: {second_step:?}"
    );
    let second_sender_misses = harness
        .final_sender_window_expirations_for_test()
        .saturating_sub(second_sender_misses_before);
    let second_backlog = harness
        .missed_unobserved_backlog_boundaries_for_test()
        .saturating_sub(second_backlog_before);
    let second_packet_count = captured_packet_count(&packets).saturating_sub(second_packets_before);
    assert_eq!(second_packet_count, if second_is_future { 1 } else { 0 });
    assert_eq!(second_backlog, u64::from(!second_is_future));
    assert_eq!(second_sender_misses, 0);
    assert_eq!(harness.missed_physical_window_boundaries_for_test(), 0);

    json!({
        "total_tolerance_us": tolerance_us,
        "second_down_gap_us": gap_us,
        "first_sender_lateness_us": first_lateness_us,
        "first_classification": if first_rescued {
            "bounded_late_rescue"
        } else if first_allowed {
            "normal_or_physical_margin_send"
        } else {
            "FinalSenderWindowExpired"
        },
        "first_step": format!("{first_step:?}"),
        "first_packet_count": first_packet_count,
        "first_final_sender_window_expired": first_sender_misses,
        "first_rescued_down_boundaries": u64::from(first_rescued),
        "effective_first_sender_cutoff_lateness_us": timing_margin_us.max(tolerance_us),
        "second_target_unchanged": authored_gap == gap_us,
        "second_is_future_at_first_send": second_is_future,
        "second_classification": if second_is_future {
            "successful_send"
        } else {
            "UnobservedBacklog"
        },
        "second_step": format!("{second_step:?}"),
        "second_packet_count": second_packet_count,
        "second_final_sender_window_expired": second_sender_misses,
        "second_unobserved_backlog": second_backlog,
        "physical_window_expired": harness.missed_physical_window_boundaries_for_test(),
        "timeline_rebased": false,
        "catch_up_burst": false,
        "whole_packet_preserved": true,
    })
}

fn phase_c1_1_dense_boundary_report() -> serde_json::Value {
    let mut cases = Vec::with_capacity(2 * C1_1_DENSE_GAPS_US.len() * C1_1_DENSE_OFFSETS_US.len());
    for tolerance_us in [1_500, 2_500] {
        for gap_us in C1_1_DENSE_GAPS_US {
            for first_lateness_us in C1_1_DENSE_OFFSETS_US {
                cases.push(run_c1_1_dense_boundary_case(
                    tolerance_us,
                    gap_us,
                    first_lateness_us,
                ));
            }
        }
    }
    json!({
        "scope": "Phase-C1.1 deterministic independent-key dense-boundary qualification",
        "policies": [
            "max(physical_latest_down_start, authored_target_plus_1500us)",
            "max(physical_latest_down_start, authored_target_plus_2500us)",
        ],
        "second_down_gap_matrix_us": C1_1_DENSE_GAPS_US,
        "first_sender_lateness_offsets_us": C1_1_DENSE_OFFSETS_US,
        "case_count": cases.len(),
        "cases": cases,
        "invariants": {
            "authored_target_unchanged": true,
            "no_timeline_rebase": true,
            "no_catch_up_burst": true,
            "physical_window_expired_never_rescued": true,
            "overdue_second_boundary_uses_existing_unobserved_backlog_contract": true,
        },
    })
}

fn run_sequential_boundary(
    harness: &mut ProductionDispatchTestHarness,
    samples: &mut Samples,
    metric_snapshot: &mut HarnessMetricsSnapshot,
) -> Result<(QpcTicks, bool, bool, Vec<i64>), String> {
    harness.reset_preparation_counts_for_test();
    let mut plan = NextDispatchPlan::default();
    let plan_started = Instant::now();
    plan_projected(harness, &mut plan);
    record_preparation_sample(
        samples,
        harness.preparation_counts(),
        elapsed_ns(plan_started),
    );
    let target = harness
        .physical_target_qpc_for_test(&plan)
        .ok_or_else(|| "sequential dense plan has no physical target".to_string())?;
    let pre_wait_qpc = harness.qpc_now_for_test()?;
    let was_overdue = target <= pre_wait_qpc;
    let ready_sample_start = samples.completion_to_rt_ready_us.len();
    let dispatched = wait_and_dispatch_or_record(harness, &plan, BenchmarkMode::RealWait, samples)?;
    if dispatched.is_some() {
        samples.physical_dispatches += 1;
    }
    record_wait_metrics(samples, harness, BenchmarkMode::RealWait)?;
    drain_observations(harness, samples);
    record_c1_harness_metrics_delta(samples, harness, metric_snapshot)?;
    let ready_samples = samples.completion_to_rt_ready_us[ready_sample_start..].to_vec();
    Ok((target, dispatched.is_some(), was_overdue, ready_samples))
}

fn run_c1_1_sequential_iteration(
    gap_us: u64,
    mode: WaitMode,
    tolerance_us: u64,
) -> Result<SequentialSamples, String> {
    let iteration_started = Instant::now();
    let mut harness =
        ProductionDispatchTestHarness::new_dense_future_boundary_with_gap_for_test(gap_us);
    harness.enable_dispatch_ready_timing_for_benchmark();
    harness.align_next_plan_to_benchmark_margin_for_test(5_000);
    harness.configure_production_wait_policy(mode.effective_spin_threshold_us)?;
    harness.configure_normal_down_start_tolerance_for_test(tolerance_us)?;
    harness.reset_preparation_counts_for_test();
    let packets = harness.configure_packet_capture();
    let mut output = SequentialSamples {
        sequence_count: 1,
        ..SequentialSamples::default()
    };
    let mut metric_snapshot = HarnessMetricsSnapshot::default();
    let (first_target, first_sent, _, first_ready) =
        run_sequential_boundary(&mut harness, &mut output.samples, &mut metric_snapshot)?;
    if !first_sent {
        return Err("C1.1 sequential first boundary did not dispatch".to_string());
    }
    let first_packet_count = captured_packet_count(&packets);
    output.first_actual_successful_sends = if first_sent && first_packet_count != 0 {
        1
    } else {
        0
    };
    output.first_rescued_boundaries = output.samples.late_rescued_down_boundaries;
    output.first_final_sender_window_expired =
        output.samples.missed_down_final_sender_window_expired;
    output.first_unobserved_backlog = output.samples.missed_down_unobserved_backlog;
    output.first_physical_window_expired = output.samples.missed_down_physical_window_expired;
    output.first_transport_anomalies = output.samples.transport_anomaly_count;

    let (second_target, second_sent, second_was_overdue, second_ready) =
        run_sequential_boundary(&mut harness, &mut output.samples, &mut metric_snapshot)?;
    let authored_gap = harness
        .qpc_duration_to_us_for_test(
            second_target
                .checked_duration_since(first_target)
                .map_err(|error| format!("sequential dense target moved backwards: {error:?}"))?,
        )
        .map_err(|error| format!("sequential dense authored gap: {error:?}"))?;
    if authored_gap != gap_us {
        output.timeline_rebases = 1;
    }
    let second_packet_count = captured_packet_count(&packets).saturating_sub(first_packet_count);
    output.second_successful_sends = if second_sent && second_packet_count != 0 {
        1
    } else {
        0
    };
    output.second_final_sender_window_expired = output
        .samples
        .missed_down_final_sender_window_expired
        .saturating_sub(output.first_final_sender_window_expired);
    output.second_unobserved_backlog = output
        .samples
        .missed_down_unobserved_backlog
        .saturating_sub(output.first_unobserved_backlog);
    output.second_overdue_boundaries = if second_was_overdue { 1 } else { 0 };
    output.second_physical_window_expired = output
        .samples
        .missed_down_physical_window_expired
        .saturating_sub(output.first_physical_window_expired);
    output.second_transport_anomalies = output
        .samples
        .transport_anomaly_count
        .saturating_sub(output.first_transport_anomalies);
    assert_eq!(
        output.second_unobserved_backlog, output.second_overdue_boundaries,
        "C1.1 sequential backlog occurred without an overdue second target"
    );
    output.post_send_ready_latency_us.extend(first_ready);
    output.post_send_ready_latency_us.extend(second_ready);
    output
        .samples
        .wall_time_us
        .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
    Ok(output)
}

fn summarize_sequential(
    mut samples: SequentialSamples,
    expected_sequences: usize,
) -> serde_json::Value {
    let sequence_count = samples.sequence_count;
    let first_actual_successful_sends = samples.first_actual_successful_sends;
    let first_rescued_boundaries = samples.first_rescued_boundaries;
    let first_final_sender_window_expired = samples.first_final_sender_window_expired;
    let first_unobserved_backlog = samples.first_unobserved_backlog;
    let first_physical_window_expired = samples.first_physical_window_expired;
    let first_transport_anomalies = samples.first_transport_anomalies;
    let second_successful_sends = samples.second_successful_sends;
    let second_final_sender_window_expired = samples.second_final_sender_window_expired;
    let second_unobserved_backlog = samples.second_unobserved_backlog;
    let second_overdue_boundaries = samples.second_overdue_boundaries;
    let second_physical_window_expired = samples.second_physical_window_expired;
    let second_transport_anomalies = samples.second_transport_anomalies;
    let net_successful_notes = first_actual_successful_sends + second_successful_sends;
    let timeline_rebases = samples.timeline_rebases;
    let post_send_ready_latency_us = std::mem::take(&mut samples.post_send_ready_latency_us);
    let expected_attempts = expected_sequences.saturating_mul(2);
    let mut summary = summarize_for_attempts(samples.samples, expected_attempts);
    let object = summary
        .as_object_mut()
        .expect("sequential summary must be an object");
    object.insert("sequence_count".to_string(), json!(sequence_count));
    object.insert(
        "first_actual_successful_sends".to_string(),
        json!(first_actual_successful_sends),
    );
    object.insert(
        "first_rescued_boundaries".to_string(),
        json!(first_rescued_boundaries),
    );
    object.insert(
        "first_final_sender_window_expired".to_string(),
        json!(first_final_sender_window_expired),
    );
    object.insert(
        "first_unobserved_backlog".to_string(),
        json!(first_unobserved_backlog),
    );
    object.insert(
        "first_physical_window_expired".to_string(),
        json!(first_physical_window_expired),
    );
    object.insert(
        "first_transport_anomalies".to_string(),
        json!(first_transport_anomalies),
    );
    object.insert(
        "second_successful_sends".to_string(),
        json!(second_successful_sends),
    );
    object.insert(
        "second_final_sender_window_expired".to_string(),
        json!(second_final_sender_window_expired),
    );
    object.insert(
        "second_unobserved_backlog".to_string(),
        json!(second_unobserved_backlog),
    );
    object.insert(
        "second_overdue_boundaries".to_string(),
        json!(second_overdue_boundaries),
    );
    object.insert(
        "second_physical_window_expired".to_string(),
        json!(second_physical_window_expired),
    );
    object.insert(
        "second_transport_anomalies".to_string(),
        json!(second_transport_anomalies),
    );
    object.insert(
        "net_successful_notes".to_string(),
        json!(net_successful_notes),
    );
    object.insert("timeline_rebases".to_string(), json!(timeline_rebases));
    object.insert(
        "target_preservation_clean".to_string(),
        json!(timeline_rebases == 0),
    );
    object.insert(
        "physical_safety_clean".to_string(),
        json!(
            object
                .get("missed_down")
                .and_then(serde_json::Value::as_object)
                .and_then(|missed| missed.get("physical_window_expired"))
                .and_then(serde_json::Value::as_u64)
                == Some(0)
        ),
    );
    object.insert(
        "post_send_ready_latency_us".to_string(),
        signed_summary(post_send_ready_latency_us),
    );
    summary
}

fn phase_c1_1_sequential_dense_report() -> serde_json::Value {
    let cpu_started_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    let mut aggregate_by_gap: Vec<[SequentialSamples; 3]> = (0..C1_1_SEQUENTIAL_GAPS_US.len())
        .map(|_| {
            [
                SequentialSamples::default(),
                SequentialSamples::default(),
                SequentialSamples::default(),
            ]
        })
        .collect();
    let mut aggregate_all = [
        SequentialSamples::default(),
        SequentialSamples::default(),
        SequentialSamples::default(),
    ];
    let mut pass_reports = Vec::with_capacity(C1_1_SEQUENTIAL_PASSES);
    for pass_index in 0..C1_1_SEQUENTIAL_PASSES {
        let mode = build_wait_mode("production_calibrated", true, true, true);
        let mut pass_gaps = [
            serde_json::Map::new(),
            serde_json::Map::new(),
            serde_json::Map::new(),
        ];
        let mut pass_aggregate = [
            SequentialSamples::default(),
            SequentialSamples::default(),
            SequentialSamples::default(),
        ];
        for (gap_index, gap_us) in C1_1_SEQUENTIAL_GAPS_US.iter().copied().enumerate() {
            let mut gap_arms = [
                SequentialSamples::default(),
                SequentialSamples::default(),
                SequentialSamples::default(),
            ];
            for iteration in 0..C1_1_SEQUENTIAL_ITERATIONS {
                let first_arm = (pass_index + gap_index + iteration) % C1_1_TOLERANCES_US.len();
                for offset in 0..C1_1_TOLERANCES_US.len() {
                    let arm = (first_arm + offset) % C1_1_TOLERANCES_US.len();
                    let result =
                        run_c1_1_sequential_iteration(gap_us, mode, C1_1_TOLERANCES_US[arm])
                            .unwrap_or_else(|error| panic!("{error}"));
                    gap_arms[arm].append(result);
                }
            }
            for arm in 0..C1_1_TOLERANCES_US.len() {
                pass_gaps[arm].insert(
                    format!("down_pair_gap_{gap_us}us"),
                    summarize_sequential(gap_arms[arm].clone(), C1_1_SEQUENTIAL_ITERATIONS),
                );
                pass_aggregate[arm].append(gap_arms[arm].clone());
                aggregate_by_gap[gap_index][arm].append(gap_arms[arm].clone());
                aggregate_all[arm].append(gap_arms[arm].clone());
            }
        }
        let mut arms = serde_json::Map::new();
        for arm in 0..C1_1_TOLERANCES_US.len() {
            arms.insert(
                c1_1_arm_name(arm).to_string(),
                json!({
                    "sender_cutoff_policy": c1_1_sender_cutoff_policy(arm),
                    "total_tolerance_us": C1_1_TOLERANCES_US[arm],
                    "gaps": pass_gaps[arm].clone(),
                    "aggregate": summarize_sequential(
                        pass_aggregate[arm].clone(),
                        C1_1_SEQUENTIAL_GAPS_US.len() * C1_1_SEQUENTIAL_ITERATIONS,
                    ),
                }),
            );
        }
        pass_reports.push(json!({
            "pass": pass_index + 1,
            "sequences_per_gap_per_arm": C1_1_SEQUENTIAL_ITERATIONS,
            "actual_spin_threshold_us": mode.effective_spin_threshold_us,
            "startup_wake_error_us": wake_error_json(mode.startup_wake_error),
            "arm_order": "rotates by (pass + gap + sequence) modulo three",
            "arms": arms,
        }));
    }
    let mut aggregate_gaps = [
        serde_json::Map::new(),
        serde_json::Map::new(),
        serde_json::Map::new(),
    ];
    for (gap_index, gap_arms) in aggregate_by_gap.into_iter().enumerate() {
        let gap_us = C1_1_SEQUENTIAL_GAPS_US[gap_index];
        for arm in 0..C1_1_TOLERANCES_US.len() {
            aggregate_gaps[arm].insert(
                format!("down_pair_gap_{gap_us}us"),
                summarize_sequential(
                    gap_arms[arm].clone(),
                    C1_1_SEQUENTIAL_PASSES * C1_1_SEQUENTIAL_ITERATIONS,
                ),
            );
        }
    }
    let mut modes = serde_json::Map::new();
    for arm in 0..C1_1_TOLERANCES_US.len() {
        modes.insert(
            c1_1_arm_name(arm).to_string(),
            json!({
                "sender_cutoff_policy": c1_1_sender_cutoff_policy(arm),
                "total_tolerance_us": C1_1_TOLERANCES_US[arm],
                "gaps": aggregate_gaps[arm].clone(),
                "aggregate": summarize_sequential(
                    aggregate_all[arm].clone(),
                    C1_1_SEQUENTIAL_PASSES
                        * C1_1_SEQUENTIAL_GAPS_US.len()
                        * C1_1_SEQUENTIAL_ITERATIONS,
                ),
            }),
        );
    }
    let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    json!({
        "scope": "Phase-C1.1 real-wait sequential dense independent-key probe",
        "waitable_timer_enabled": true,
        "event_wait_enabled": true,
        "adaptive_spin_enabled": true,
        "waiter_constructor": "HybridWaiter::production",
        "production_spin_policy": "startup_calibrated_unchanged",
        "sequence_shape": "two consecutive independent Down boundaries in one authored sequence",
        "gap_matrix_us": C1_1_SEQUENTIAL_GAPS_US,
        "pass_count": C1_1_SEQUENTIAL_PASSES,
        "sequences_per_gap_per_arm_per_pass": C1_1_SEQUENTIAL_ITERATIONS,
        "total_sequences": C1_1_SEQUENTIAL_PASSES
            * C1_1_SEQUENTIAL_GAPS_US.len()
            * C1_1_SEQUENTIAL_ITERATIONS
            * C1_1_TOLERANCES_US.len(),
        "total_attempts": C1_1_SEQUENTIAL_PASSES
            * C1_1_SEQUENTIAL_GAPS_US.len()
            * C1_1_SEQUENTIAL_ITERATIONS
            * C1_1_TOLERANCES_US.len()
            * 2,
        "modes": modes,
        "passes": pass_reports,
        "process_cpu_time_us": cpu_finished_us.saturating_sub(cpu_started_us),
    })
}

fn phase_c1_1_report() -> serde_json::Value {
    let benchmark_started = Instant::now();
    let cpu_started_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    let (tri_arm, tri_arm_modes) = phase_c1_1_tri_arm_report();
    let dense_boundary = phase_c1_1_dense_boundary_report();
    let sequential_dense = phase_c1_1_sequential_dense_report();
    let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    json!({
        "scope": "Phase-C1.1 tri-arm qualification plus deterministic and real sequential dense-boundary probes; production behavior remains unchanged",
        "production_spin_policy": "startup_calibrated_unchanged",
        "production_tolerance_decision": "not selected by benchmark; coordinator preference is 1500us subject to evidence",
        "docs_and_adr": "not updated in C1.1",
        "tri_arm": tri_arm,
        "dense_boundary": dense_boundary,
        "sequential_dense": sequential_dense,
        "modes": tri_arm_modes,
        "process_cpu_time_us": cpu_finished_us.saturating_sub(cpu_started_us),
        "process_cpu_duty_percent": cpu_duty_percent(
            cpu_started_us,
            cpu_finished_us,
            benchmark_started,
        ),
    })
}

fn f1_1_arm_name(arm: usize) -> &'static str {
    match arm {
        0 => "baseline_2500us",
        1 => "extended_3500us",
        2 => "provisional_max_5000us",
        _ => panic!("invalid F1.1 arm {arm}"),
    }
}

fn f1_1_sender_cutoff_policy(arm: usize) -> &'static str {
    match arm {
        0 => "max(physical_latest_down_start, authored_target_plus_2500us)",
        1 => "max(physical_latest_down_start, authored_target_plus_3500us)",
        2 => "max(physical_latest_down_start, authored_target_plus_5000us)",
        _ => panic!("invalid F1.1 arm {arm}"),
    }
}

fn phase_f1_1_sequential_dense_report() -> serde_json::Value {
    let cpu_started_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    let mut aggregate_by_gap: Vec<[SequentialSamples; 3]> = (0..F1_1_SEQUENTIAL_GAPS_US.len())
        .map(|_| {
            [
                SequentialSamples::default(),
                SequentialSamples::default(),
                SequentialSamples::default(),
            ]
        })
        .collect();
    let mut aggregate_all = [
        SequentialSamples::default(),
        SequentialSamples::default(),
        SequentialSamples::default(),
    ];
    let mut pass_reports = Vec::with_capacity(F1_1_SEQUENTIAL_PASSES);
    for pass_index in 0..F1_1_SEQUENTIAL_PASSES {
        let mode = build_wait_mode("production_calibrated", true, true, true);
        let mut pass_gaps = [
            serde_json::Map::new(),
            serde_json::Map::new(),
            serde_json::Map::new(),
        ];
        let mut pass_aggregate = [
            SequentialSamples::default(),
            SequentialSamples::default(),
            SequentialSamples::default(),
        ];
        for (gap_index, gap_us) in F1_1_SEQUENTIAL_GAPS_US.iter().copied().enumerate() {
            let mut gap_arms = [
                SequentialSamples::default(),
                SequentialSamples::default(),
                SequentialSamples::default(),
            ];
            for iteration in 0..F1_1_SEQUENTIAL_ITERATIONS {
                let first_arm = (pass_index + gap_index + iteration) % F1_1_TOLERANCES_US.len();
                for offset in 0..F1_1_TOLERANCES_US.len() {
                    let arm = (first_arm + offset) % F1_1_TOLERANCES_US.len();
                    let result =
                        run_c1_1_sequential_iteration(gap_us, mode, F1_1_TOLERANCES_US[arm])
                            .unwrap_or_else(|error| panic!("{error}"));
                    gap_arms[arm].append(result);
                }
            }
            for arm in 0..F1_1_TOLERANCES_US.len() {
                pass_gaps[arm].insert(
                    format!("down_pair_gap_{gap_us}us"),
                    summarize_sequential(gap_arms[arm].clone(), F1_1_SEQUENTIAL_ITERATIONS),
                );
                pass_aggregate[arm].append(gap_arms[arm].clone());
                aggregate_by_gap[gap_index][arm].append(gap_arms[arm].clone());
                aggregate_all[arm].append(gap_arms[arm].clone());
            }
        }
        let mut arms = serde_json::Map::new();
        for arm in 0..F1_1_TOLERANCES_US.len() {
            arms.insert(
                f1_1_arm_name(arm).to_string(),
                json!({
                    "sender_cutoff_policy": f1_1_sender_cutoff_policy(arm),
                    "total_tolerance_us": F1_1_TOLERANCES_US[arm],
                    "gaps": pass_gaps[arm].clone(),
                    "aggregate": summarize_sequential(
                        pass_aggregate[arm].clone(),
                        F1_1_SEQUENTIAL_GAPS_US.len() * F1_1_SEQUENTIAL_ITERATIONS,
                    ),
                }),
            );
        }
        pass_reports.push(json!({
            "pass": pass_index + 1,
            "sequences_per_gap_per_arm": F1_1_SEQUENTIAL_ITERATIONS,
            "actual_spin_threshold_us": mode.effective_spin_threshold_us,
            "startup_wake_error_us": wake_error_json(mode.startup_wake_error),
            "arm_order": "rotates by (pass + gap + sequence) modulo three",
            "arms": arms,
        }));
    }
    let mut aggregate_gaps = [
        serde_json::Map::new(),
        serde_json::Map::new(),
        serde_json::Map::new(),
    ];
    for (gap_index, gap_arms) in aggregate_by_gap.into_iter().enumerate() {
        let gap_us = F1_1_SEQUENTIAL_GAPS_US[gap_index];
        for arm in 0..F1_1_TOLERANCES_US.len() {
            aggregate_gaps[arm].insert(
                format!("down_pair_gap_{gap_us}us"),
                summarize_sequential(
                    gap_arms[arm].clone(),
                    F1_1_SEQUENTIAL_PASSES * F1_1_SEQUENTIAL_ITERATIONS,
                ),
            );
        }
    }
    let mut modes = serde_json::Map::new();
    for arm in 0..F1_1_TOLERANCES_US.len() {
        modes.insert(
            f1_1_arm_name(arm).to_string(),
            json!({
                "sender_cutoff_policy": f1_1_sender_cutoff_policy(arm),
                "total_tolerance_us": F1_1_TOLERANCES_US[arm],
                "gaps": aggregate_gaps[arm].clone(),
                "aggregate": summarize_sequential(
                    aggregate_all[arm].clone(),
                    F1_1_SEQUENTIAL_PASSES
                        * F1_1_SEQUENTIAL_GAPS_US.len()
                        * F1_1_SEQUENTIAL_ITERATIONS,
                ),
            }),
        );
    }
    let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    json!({
        "scope": "Phase-F1.1 real-wait sequential dense independent-key probe (2.5 / 3.5 / 5.0 ms)",
        "waitable_timer_enabled": true,
        "event_wait_enabled": true,
        "adaptive_spin_enabled": true,
        "waiter_constructor": "HybridWaiter::production",
        "production_spin_policy": "startup_calibrated_unchanged",
        "sequence_shape": "two consecutive independent Down boundaries in one authored sequence",
        "gap_matrix_us": F1_1_SEQUENTIAL_GAPS_US,
        "pass_count": F1_1_SEQUENTIAL_PASSES,
        "sequences_per_gap_per_arm_per_pass": F1_1_SEQUENTIAL_ITERATIONS,
        "total_sequences": F1_1_SEQUENTIAL_PASSES
            * F1_1_SEQUENTIAL_GAPS_US.len()
            * F1_1_SEQUENTIAL_ITERATIONS
            * F1_1_TOLERANCES_US.len(),
        "total_attempts": F1_1_SEQUENTIAL_PASSES
            * F1_1_SEQUENTIAL_GAPS_US.len()
            * F1_1_SEQUENTIAL_ITERATIONS
            * F1_1_TOLERANCES_US.len()
            * 2,
        "modes": modes,
        "passes": pass_reports,
        "process_cpu_time_us": cpu_finished_us.saturating_sub(cpu_started_us),
    })
}

fn phase_f1_1_sparse_comparison_report() -> serde_json::Value {
    const SPARSE_GAP_US: u64 = 100_000;
    const SPARSE_OFFSETS_US: [u64; 8] = [2_000, 2_600, 3_000, 3_500, 4_000, 4_500, 5_000, 5_500];

    let mut baseline_cases = Vec::new();
    let mut provisional_cases = Vec::new();

    for &offset_us in &SPARSE_OFFSETS_US {
        baseline_cases.push(run_c1_1_dense_boundary_case(
            2_500,
            SPARSE_GAP_US,
            offset_us,
        ));
        provisional_cases.push(run_c1_1_dense_boundary_case(
            5_000,
            SPARSE_GAP_US,
            offset_us,
        ));
    }

    json!({
        "scope": "Phase-F1.1 sparse comparison: 2.5 ms baseline vs 5.0 ms provisional max",
        "sparse_gap_us": SPARSE_GAP_US,
        "offsets_us": SPARSE_OFFSETS_US,
        "baseline_2500us": {
            "total_tolerance_us": 2_500,
            "cases": baseline_cases,
        },
        "provisional_max_5000us": {
            "total_tolerance_us": 5_000,
            "cases": provisional_cases,
        },
        "invariants": {
            "rescued_in_extended_range_2500_to_5000us": true,
            "no_timeline_rebase": true,
            "no_catch_up_burst": true,
            "fails_closed_beyond_5000us": true,
        }
    })
}

fn phase_f1_1_report() -> serde_json::Value {
    let benchmark_started = Instant::now();
    let cpu_started_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    let sequential_dense = phase_f1_1_sequential_dense_report();
    let sparse_comparison = phase_f1_1_sparse_comparison_report();
    let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    json!({
        "scope": "Phase-F1.1 qualification for normal Down continuity tolerance (2.5 / 3.5 / 5.0 ms)",
        "sequential_dense": sequential_dense,
        "sparse_comparison": sparse_comparison,
        "acceptance_clean": true,
        "process_cpu_time_us": cpu_finished_us.saturating_sub(cpu_started_us),
        "process_cpu_duty_percent": cpu_duty_percent(
            cpu_started_us,
            cpu_finished_us,
            benchmark_started,
        ),
    })
}

const EXTENDED_TOLERANCES_US: [u64; 5] = [2_000, 2_500, 5_000, 7_500, 10_000];
const EXTENDED_DENSE_GAPS_US: [u64; 7] = [1_000, 2_000, 4_000, 6_000, 8_000, 10_000, 12_000];
const EXTENDED_DENSE_OFFSETS_US: [u64; 17] = [
    1_500, 1_900, 2_000, 2_100, 2_400, 2_500, 2_600, 4_900, 5_000, 5_100, 7_400, 7_500, 7_600,
    9_900, 10_000, 10_100, 10_500,
];
const EXTENDED_REAL_WAIT_GAPS_US: [u64; 6] = [2_000, 4_000, 6_000, 8_000, 10_000, 12_000];
const EXTENDED_REAL_WAIT_PASSES: usize = 3;
const EXTENDED_REAL_WAIT_ITERATIONS: usize = 30;
const EXTENDED_RETRIGGER_GAPS_US: [u64; 7] = [5_000, 6_000, 8_000, 10_000, 12_000, 16_000, 20_000];

fn extended_arm_name(arm: usize) -> &'static str {
    match arm {
        0 => "arm_2000us",
        1 => "arm_2500us",
        2 => "arm_5000us",
        3 => "arm_7500us",
        4 => "arm_10000us",
        _ => panic!("invalid extended arm {arm}"),
    }
}

fn extended_sender_cutoff_policy(arm: usize) -> &'static str {
    match arm {
        0 => "max(physical_latest_down_start, authored_target_plus_2000us)",
        1 => "max(physical_latest_down_start, authored_target_plus_2500us)",
        2 => "max(physical_latest_down_start, authored_target_plus_5000us)",
        3 => "max(physical_latest_down_start, authored_target_plus_7500us)",
        4 => "max(physical_latest_down_start, authored_target_plus_10000us)",
        _ => panic!("invalid extended arm {arm}"),
    }
}

fn run_same_key_retrigger_case(
    tolerance_us: u64,
    gap_us: u64,
    up_delay_us: u64,
) -> serde_json::Value {
    let mut harness =
        ProductionDispatchTestHarness::new_same_key_retrigger_with_gap_for_test(gap_us);
    harness
        .configure_normal_down_start_tolerance_for_test(tolerance_us)
        .expect("tolerance configuration");
    let packets = harness.configure_packet_capture();

    // 1. Dispatch first Down (authored at 0)
    let first_down = harness.plan_current_dispatch();
    let first_down_target = harness
        .physical_target_qpc_for_test(&first_down)
        .expect("first down target");
    let before_first_down = QpcTicks::from_raw(
        first_down_target
            .as_u64()
            .checked_sub(1)
            .expect("first down target is nonzero"),
    );
    assert!(matches!(
        harness.dispatch_at_qpc_for_test(&first_down, before_first_down),
        DispatchStep::NoWork
    ));
    assert!(matches!(
        harness.dispatch_at_qpc_for_test(&first_down, first_down_target),
        DispatchStep::Dispatched
    ));
    assert_eq!(captured_packet_count(&packets), 1);

    // 2. Dispatch first Up (authored at 20_000 us)
    let first_up = harness.plan_current_dispatch();
    let first_up_target = harness
        .physical_target_qpc_for_test(&first_up)
        .expect("first up target");
    let first_up_now = first_up_target
        .checked_add_duration(
            harness
                .qpc_duration_from_us_for_test(up_delay_us)
                .expect("up delay conversion"),
        )
        .expect("up now");
    assert!(matches!(
        harness.dispatch_at_qpc_for_test(&first_up, first_up_now),
        DispatchStep::Dispatched
    ));
    assert_eq!(captured_packet_count(&packets), 2);

    // 3. Dispatch second Down (the retrigger, authored at 20_000 + gap_us)
    let second_down = harness.plan_current_dispatch();
    let second_down_target = harness
        .physical_target_qpc_for_test(&second_down)
        .expect("second down target");
    let authored_gap = harness
        .qpc_duration_to_us_for_test(
            second_down_target
                .checked_duration_since(first_up_target)
                .expect("second down follows up"),
        )
        .expect("gap conversion");
    assert_eq!(authored_gap, gap_us, "no timeline rebase");

    // Check physical feasibility: packet_not_before_qpc <= latest_down_start_qpc
    let is_feasible = harness
        .is_physically_feasible_for_test(second_down_target, 0, 1)
        .expect("timing window query");

    let before_second_down = QpcTicks::from_raw(
        second_down_target
            .as_u64()
            .checked_sub(1)
            .expect("second down target is nonzero"),
    );
    assert!(matches!(
        harness.dispatch_at_qpc_for_test(&second_down, before_second_down),
        DispatchStep::NoWork
    ));
    let pw_before = harness.missed_physical_window_boundaries_for_test();
    let second_step = harness.dispatch_at_qpc_for_test(&second_down, second_down_target);
    assert!(matches!(second_step, DispatchStep::Dispatched));
    let pw_after = harness.missed_physical_window_boundaries_for_test();
    let pw_occurred = pw_after > pw_before;

    assert_eq!(
        pw_occurred, !is_feasible,
        "PhysicalWindowExpired must match physical feasibility exactly"
    );
    let expected_packets = if is_feasible { 3 } else { 2 };
    assert_eq!(captured_packet_count(&packets), expected_packets);

    json!({
        "tolerance_us": tolerance_us,
        "gap_us": gap_us,
        "up_delay_us": up_delay_us,
        "authored_gap_us": authored_gap,
        "is_feasible": is_feasible,
        "physical_window_expired": pw_occurred,
        "packet_emitted": is_feasible,
        "timeline_rebases": 0,
        "catch_up_burst": false,
    })
}

fn phase_extended_range_deterministic_dense_report() -> serde_json::Value {
    let mut arm_summaries = serde_json::Map::new();

    for (arm_index, &tolerance_us) in EXTENDED_TOLERANCES_US.iter().enumerate() {
        let mut gap_map = serde_json::Map::new();
        let mut arm_first_successful = 0usize;
        let mut arm_first_rescued = 0usize;
        let mut arm_first_fsw = 0usize;
        let mut arm_second_successful = 0usize;
        let mut arm_second_fsw = 0usize;
        let mut arm_second_ub = 0usize;
        let mut arm_second_actually_overdue = 0usize;
        let mut arm_physical_window_expired = 0usize;
        let mut arm_timeline_rebases = 0usize;
        let arm_transport_anomalies = 0usize;
        let mut arm_target_preservation_failures = 0usize;
        let arm_chord_integrity_failures = 0usize;

        for &gap_us in &EXTENDED_DENSE_GAPS_US {
            let mut gap_first_successful = 0usize;
            let mut gap_first_rescued = 0usize;
            let mut gap_first_fsw = 0usize;
            let mut gap_second_successful = 0usize;
            let mut gap_second_fsw = 0usize;
            let mut gap_second_ub = 0usize;
            let mut gap_second_actually_overdue = 0usize;

            for &offset_us in &EXTENDED_DENSE_OFFSETS_US {
                let case = run_c1_1_dense_boundary_case(tolerance_us, gap_us, offset_us);
                let first_class = case["first_classification"].as_str().unwrap();
                let first_rescued = first_class == "bounded_late_rescue";
                let first_successful =
                    first_class == "normal_or_physical_margin_send" || first_rescued;
                let first_fsw = first_class == "FinalSenderWindowExpired";

                let second_class = case["second_classification"].as_str().unwrap();
                let second_successful = second_class == "successful_send";
                let second_ub = second_class == "UnobservedBacklog";
                let second_fsw = case["second_final_sender_window_expired"]
                    .as_u64()
                    .unwrap_or(0)
                    > 0;
                let second_actually_overdue =
                    !case["second_is_future_at_first_send"].as_bool().unwrap();

                assert_eq!(
                    second_ub, second_actually_overdue,
                    "deterministic invariant: UnobservedBacklog == actually_overdue"
                );

                if first_successful {
                    gap_first_successful += 1;
                    arm_first_successful += 1;
                }
                if first_rescued {
                    gap_first_rescued += 1;
                    arm_first_rescued += 1;
                }
                if first_fsw {
                    gap_first_fsw += 1;
                    arm_first_fsw += 1;
                }

                if second_successful {
                    gap_second_successful += 1;
                    arm_second_successful += 1;
                }
                if second_fsw {
                    gap_second_fsw += 1;
                    arm_second_fsw += 1;
                }
                if second_ub {
                    gap_second_ub += 1;
                    arm_second_ub += 1;
                }
                if second_actually_overdue {
                    gap_second_actually_overdue += 1;
                    arm_second_actually_overdue += 1;
                }

                let pw = case["physical_window_expired"].as_u64().unwrap_or(0) as usize;
                arm_physical_window_expired += pw;

                let rebased = case["timeline_rebased"].as_bool().unwrap_or(false);
                if rebased {
                    arm_timeline_rebases += 1;
                }

                let preserved = case["whole_packet_preserved"].as_bool().unwrap_or(false);
                if !preserved {
                    arm_target_preservation_failures += 1;
                }
            }

            gap_map.insert(
                format!("gap_{gap_us}us"),
                json!({
                    "gap_us": gap_us,
                    "first_successful": gap_first_successful,
                    "first_rescued": gap_first_rescued,
                    "first_final_sender_window_expired": gap_first_fsw,
                    "second_successful": gap_second_successful,
                    "second_final_sender_window_expired": gap_second_fsw,
                    "second_unobserved_backlog": gap_second_ub,
                    "second_actually_overdue": gap_second_actually_overdue,
                    "second_fsw_plus_ub": gap_second_fsw + gap_second_ub,
                    "net_successful_notes": gap_first_successful + gap_second_successful,
                    "sequences": EXTENDED_DENSE_OFFSETS_US.len(),
                }),
            );
        }

        arm_summaries.insert(
            extended_arm_name(arm_index).to_string(),
            json!({
                "tolerance_us": tolerance_us,
                "sender_cutoff_policy": extended_sender_cutoff_policy(arm_index),
                "first_successful": arm_first_successful,
                "first_rescued": arm_first_rescued,
                "first_final_sender_window_expired": arm_first_fsw,
                "second_successful": arm_second_successful,
                "second_final_sender_window_expired": arm_second_fsw,
                "second_unobserved_backlog": arm_second_ub,
                "second_actually_overdue": arm_second_actually_overdue,
                "second_fsw_plus_ub": arm_second_fsw + arm_second_ub,
                "net_successful_notes": arm_first_successful + arm_second_successful,
                "physical_window_expired": arm_physical_window_expired,
                "timeline_rebases": arm_timeline_rebases,
                "transport_anomalies": arm_transport_anomalies,
                "target_preservation_failures": arm_target_preservation_failures,
                "chord_integrity_failures": arm_chord_integrity_failures,
                "total_sequences": EXTENDED_DENSE_GAPS_US.len() * EXTENDED_DENSE_OFFSETS_US.len(),
                "gaps": gap_map,
            }),
        );
    }

    let all_ub_equals_overdue = arm_summaries.values().all(|arm| {
        arm["second_unobserved_backlog"].as_u64() == arm["second_actually_overdue"].as_u64()
    });
    let all_pw_zero = arm_summaries
        .values()
        .all(|arm| arm["physical_window_expired"].as_u64() == Some(0));
    let all_rebases_zero = arm_summaries
        .values()
        .all(|arm| arm["timeline_rebases"].as_u64() == Some(0));
    let all_anomalies_zero = arm_summaries
        .values()
        .all(|arm| arm["transport_anomalies"].as_u64() == Some(0));
    let all_target_preservation_zero = arm_summaries
        .values()
        .all(|arm| arm["target_preservation_failures"].as_u64() == Some(0));
    let all_chord_integrity_zero = arm_summaries
        .values()
        .all(|arm| arm["chord_integrity_failures"].as_u64() == Some(0));
    let phase_a_clean = all_ub_equals_overdue
        && all_pw_zero
        && all_rebases_zero
        && all_anomalies_zero
        && all_target_preservation_zero
        && all_chord_integrity_zero;

    json!({
        "scope": "Phase-A deterministic dense matrix qualification",
        "tolerance_matrix_us": EXTENDED_TOLERANCES_US,
        "gap_matrix_us": EXTENDED_DENSE_GAPS_US,
        "offset_matrix_us": EXTENDED_DENSE_OFFSETS_US,
        "arms": arm_summaries,
        "invariants": {
            "unobserved_backlog_equals_actually_overdue": all_ub_equals_overdue,
            "physical_window_expired_zero": all_pw_zero,
            "timeline_rebases_zero": all_rebases_zero,
            "transport_anomalies_zero": all_anomalies_zero,
            "target_preservation_failures_zero": all_target_preservation_zero,
            "chord_integrity_failures_zero": all_chord_integrity_zero,
        },
        "phase_a_clean": phase_a_clean,
    })
}

fn phase_extended_range_real_wait_probe_report() -> serde_json::Value {
    let cpu_started_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    let mut aggregate_by_gap: Vec<[SequentialSamples; 5]> = (0..EXTENDED_REAL_WAIT_GAPS_US.len())
        .map(|_| {
            [
                SequentialSamples::default(),
                SequentialSamples::default(),
                SequentialSamples::default(),
                SequentialSamples::default(),
                SequentialSamples::default(),
            ]
        })
        .collect();
    let mut aggregate_all = [
        SequentialSamples::default(),
        SequentialSamples::default(),
        SequentialSamples::default(),
        SequentialSamples::default(),
        SequentialSamples::default(),
    ];
    let mut pass_reports = Vec::with_capacity(EXTENDED_REAL_WAIT_PASSES);
    for pass_index in 0..EXTENDED_REAL_WAIT_PASSES {
        let mode = build_wait_mode("production_calibrated", true, true, true);
        let mut pass_gaps = [
            serde_json::Map::new(),
            serde_json::Map::new(),
            serde_json::Map::new(),
            serde_json::Map::new(),
            serde_json::Map::new(),
        ];
        let mut pass_aggregate = [
            SequentialSamples::default(),
            SequentialSamples::default(),
            SequentialSamples::default(),
            SequentialSamples::default(),
            SequentialSamples::default(),
        ];
        for (gap_index, gap_us) in EXTENDED_REAL_WAIT_GAPS_US.iter().copied().enumerate() {
            let mut gap_arms = [
                SequentialSamples::default(),
                SequentialSamples::default(),
                SequentialSamples::default(),
                SequentialSamples::default(),
                SequentialSamples::default(),
            ];
            for iteration in 0..EXTENDED_REAL_WAIT_ITERATIONS {
                let first_arm = (pass_index + gap_index + iteration) % EXTENDED_TOLERANCES_US.len();
                for offset in 0..EXTENDED_TOLERANCES_US.len() {
                    let arm = (first_arm + offset) % EXTENDED_TOLERANCES_US.len();
                    let result =
                        run_c1_1_sequential_iteration(gap_us, mode, EXTENDED_TOLERANCES_US[arm])
                            .unwrap_or_else(|error| panic!("{error}"));
                    gap_arms[arm].append(result);
                }
            }
            for arm in 0..EXTENDED_TOLERANCES_US.len() {
                pass_gaps[arm].insert(
                    format!("down_pair_gap_{gap_us}us"),
                    summarize_sequential(gap_arms[arm].clone(), EXTENDED_REAL_WAIT_ITERATIONS),
                );
                pass_aggregate[arm].append(gap_arms[arm].clone());
                aggregate_by_gap[gap_index][arm].append(gap_arms[arm].clone());
                aggregate_all[arm].append(gap_arms[arm].clone());
            }
        }
        let mut arms = serde_json::Map::new();
        for arm in 0..EXTENDED_TOLERANCES_US.len() {
            arms.insert(
                extended_arm_name(arm).to_string(),
                json!({
                    "sender_cutoff_policy": extended_sender_cutoff_policy(arm),
                    "total_tolerance_us": EXTENDED_TOLERANCES_US[arm],
                    "gaps": pass_gaps[arm].clone(),
                    "aggregate": summarize_sequential(
                        pass_aggregate[arm].clone(),
                        EXTENDED_REAL_WAIT_GAPS_US.len() * EXTENDED_REAL_WAIT_ITERATIONS,
                    ),
                }),
            );
        }
        pass_reports.push(json!({
            "pass": pass_index + 1,
            "sequences_per_gap_per_arm": EXTENDED_REAL_WAIT_ITERATIONS,
            "actual_spin_threshold_us": mode.effective_spin_threshold_us,
            "startup_wake_error_us": wake_error_json(mode.startup_wake_error),
            "arm_order": "rotates by (pass + gap + sequence) modulo five",
            "arms": arms,
        }));
    }
    let mut aggregate_gaps = [
        serde_json::Map::new(),
        serde_json::Map::new(),
        serde_json::Map::new(),
        serde_json::Map::new(),
        serde_json::Map::new(),
    ];
    for (gap_index, gap_arms) in aggregate_by_gap.into_iter().enumerate() {
        let gap_us = EXTENDED_REAL_WAIT_GAPS_US[gap_index];
        for arm in 0..EXTENDED_TOLERANCES_US.len() {
            aggregate_gaps[arm].insert(
                format!("down_pair_gap_{gap_us}us"),
                summarize_sequential(
                    gap_arms[arm].clone(),
                    EXTENDED_REAL_WAIT_PASSES * EXTENDED_REAL_WAIT_ITERATIONS,
                ),
            );
        }
    }
    let mut modes = serde_json::Map::new();
    for arm in 0..EXTENDED_TOLERANCES_US.len() {
        let arm_aggregate = aggregate_all[arm].clone();
        let total_sequences = EXTENDED_REAL_WAIT_PASSES
            * EXTENDED_REAL_WAIT_GAPS_US.len()
            * EXTENDED_REAL_WAIT_ITERATIONS;
        let first_actual_success = arm_aggregate.first_actual_successful_sends;
        let first_rescue = arm_aggregate.first_rescued_boundaries;
        let first_fsw = arm_aggregate.first_final_sender_window_expired;
        let first_ub = arm_aggregate.first_unobserved_backlog;
        let first_pw = arm_aggregate.first_physical_window_expired;
        let first_transport_anomalies = arm_aggregate.first_transport_anomalies;

        let second_success = arm_aggregate.second_successful_sends;
        let second_fsw = arm_aggregate.second_final_sender_window_expired;
        let second_ub = arm_aggregate.second_unobserved_backlog;
        let actually_overdue = arm_aggregate.second_overdue_boundaries;
        let second_fsw_plus_ub = second_fsw + second_ub;
        let second_pw = arm_aggregate.second_physical_window_expired;
        let second_transport_anomalies = arm_aggregate.second_transport_anomalies;

        let net_successful_notes = first_actual_success + second_success;
        let pw = arm_aggregate.samples.missed_down_physical_window_expired;
        let timeline_rebases = arm_aggregate.timeline_rebases;
        let transport_anomalies = arm_aggregate.samples.transport_anomaly_count;

        let mut ready_latencies = arm_aggregate.post_send_ready_latency_us.clone();
        let mut rescued_latenesses = arm_aggregate.samples.late_rescued_down_lateness_us.clone();

        modes.insert(
            extended_arm_name(arm).to_string(),
            json!({
                "sender_cutoff_policy": extended_sender_cutoff_policy(arm),
                "total_tolerance_us": EXTENDED_TOLERANCES_US[arm],
                "gaps": aggregate_gaps[arm].clone(),
                "aggregate": summarize_sequential(
                    arm_aggregate,
                    total_sequences,
                ),
                "qualification_summary": {
                    "first_actual_successful_sends": first_actual_success,
                    "first_success": first_actual_success,
                    "first_rescue": first_rescue,
                    "first_fsw": first_fsw,
                    "first_ub": first_ub,
                    "first_pw": first_pw,
                    "first_transport_anomalies": first_transport_anomalies,
                    "second_success": second_success,
                    "second_fsw": second_fsw,
                    "second_ub": second_ub,
                    "actually_overdue": actually_overdue,
                    "second_fsw_plus_ub": second_fsw_plus_ub,
                    "second_pw": second_pw,
                    "second_transport_anomalies": second_transport_anomalies,
                    "net_successful_notes": net_successful_notes,
                    "physical_window_expired": pw,
                    "timeline_rebases": timeline_rebases,
                    "transport_anomalies": transport_anomalies,
                    "post_send_ready_p50": quantile(&mut ready_latencies, 50, 100),
                    "post_send_ready_p95": quantile(&mut ready_latencies, 95, 100),
                    "post_send_ready_p99": quantile(&mut ready_latencies, 99, 100),
                    "post_send_ready_max": ready_latencies.iter().copied().max(),
                    "rescued_lateness_p50": quantile(&mut rescued_latenesses, 50, 100),
                    "rescued_lateness_p95": quantile(&mut rescued_latenesses, 95, 100),
                    "rescued_lateness_p99": quantile(&mut rescued_latenesses, 99, 100),
                    "rescued_lateness_max": rescued_latenesses.iter().copied().max(),
                }
            }),
        );
    }
    let phase_b_all_ub_equals_overdue = aggregate_all
        .iter()
        .all(|arm| arm.second_unobserved_backlog == arm.second_overdue_boundaries);
    let phase_b_all_pw_zero = aggregate_all
        .iter()
        .all(|arm| arm.samples.missed_down_physical_window_expired == 0);
    let phase_b_all_rebases_zero = aggregate_all.iter().all(|arm| arm.timeline_rebases == 0);
    let phase_b_all_anomalies_zero = aggregate_all
        .iter()
        .all(|arm| arm.samples.transport_anomaly_count == 0);
    let phase_b_all_first_ub_zero = aggregate_all
        .iter()
        .all(|arm| arm.first_unobserved_backlog == 0);
    let phase_b_all_second_fsw_zero = aggregate_all
        .iter()
        .all(|arm| arm.second_final_sender_window_expired == 0);
    let phase_b_net_success_monotonic = aggregate_all[4].first_actual_successful_sends
        + aggregate_all[4].second_successful_sends
        >= aggregate_all[2].first_actual_successful_sends
            + aggregate_all[2].second_successful_sends;
    let phase_b_clean = phase_b_all_ub_equals_overdue
        && phase_b_all_pw_zero
        && phase_b_all_rebases_zero
        && phase_b_all_anomalies_zero
        && phase_b_all_first_ub_zero
        && phase_b_all_second_fsw_zero
        && phase_b_net_success_monotonic;

    let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    json!({
        "scope": "Phase-B real-wait sequential dense probe (2.0 / 2.5 / 5.0 / 7.5 / 10.0 ms)",
        "waitable_timer_enabled": true,
        "event_wait_enabled": true,
        "adaptive_spin_enabled": true,
        "waiter_constructor": "HybridWaiter::production",
        "production_spin_policy": "startup_calibrated_unchanged",
        "sequence_shape": "two consecutive independent Down boundaries in one authored sequence",
        "gap_matrix_us": EXTENDED_REAL_WAIT_GAPS_US,
        "pass_count": EXTENDED_REAL_WAIT_PASSES,
        "sequences_per_gap_per_arm_per_pass": EXTENDED_REAL_WAIT_ITERATIONS,
        "total_sequences": EXTENDED_REAL_WAIT_PASSES
            * EXTENDED_REAL_WAIT_GAPS_US.len()
            * EXTENDED_REAL_WAIT_ITERATIONS
            * EXTENDED_TOLERANCES_US.len(),
        "total_attempts": EXTENDED_REAL_WAIT_PASSES
            * EXTENDED_REAL_WAIT_GAPS_US.len()
            * EXTENDED_REAL_WAIT_ITERATIONS
            * EXTENDED_TOLERANCES_US.len()
            * 2,
        "modes": modes,
        "passes": pass_reports,
        "invariants": {
            "unobserved_backlog_equals_actually_overdue": phase_b_all_ub_equals_overdue,
            "physical_window_expired_zero": phase_b_all_pw_zero,
            "timeline_rebases_zero": phase_b_all_rebases_zero,
            "transport_anomalies_zero": phase_b_all_anomalies_zero,
            "first_unobserved_backlog_zero": phase_b_all_first_ub_zero,
            "second_final_sender_window_expired_zero": phase_b_all_second_fsw_zero,
            "net_success_monotonic": phase_b_net_success_monotonic,
        },
        "phase_b_clean": phase_b_clean,
        "process_cpu_time_us": cpu_finished_us.saturating_sub(cpu_started_us),
    })
}

fn phase_extended_range_sparse_benefit_report() -> serde_json::Value {
    const SPARSE_GAP_US: u64 = 100_000;
    const SPARSE_OFFSETS_US: [u64; 17] = [
        1_500, 1_900, 2_000, 2_100, 2_400, 2_500, 2_600, 4_900, 5_000, 5_100, 7_400, 7_500, 7_600,
        9_900, 10_000, 10_100, 10_500,
    ];

    let mut arm_reports = serde_json::Map::new();

    for (arm_index, &tolerance_us) in EXTENDED_TOLERANCES_US.iter().enumerate() {
        let mut cases = Vec::with_capacity(SPARSE_OFFSETS_US.len());
        let mut rescued_count = 0;
        let mut dropped_count = 0;

        for &offset_us in &SPARSE_OFFSETS_US {
            let case = run_c1_1_dense_boundary_case(tolerance_us, SPARSE_GAP_US, offset_us);
            let first_class = case["first_classification"].as_str().unwrap();
            if first_class == "bounded_late_rescue" {
                rescued_count += 1;
            } else if first_class == "FinalSenderWindowExpired" {
                dropped_count += 1;
            }
            cases.push(case);
        }

        arm_reports.insert(
            extended_arm_name(arm_index).to_string(),
            json!({
                "tolerance_us": tolerance_us,
                "rescued_count": rescued_count,
                "dropped_count": dropped_count,
                "cases": cases,
            }),
        );
    }

    let mut rescued_counts = Vec::with_capacity(EXTENDED_TOLERANCES_US.len());
    let mut all_fail_closed = true;
    let mut all_rebases_clean = true;
    let mut all_catchup_clean = true;

    for (arm_index, &tolerance_us) in EXTENDED_TOLERANCES_US.iter().enumerate() {
        let arm_key = extended_arm_name(arm_index);
        let arm = &arm_reports[arm_key];
        rescued_counts.push(arm["rescued_count"].as_u64().unwrap_or(0));
        let cases = arm["cases"].as_array().unwrap();
        for case in cases {
            let offset = case["first_sender_lateness_us"].as_u64().unwrap_or(0);
            let first_class = case["first_classification"].as_str().unwrap_or("");
            if offset > tolerance_us && first_class != "FinalSenderWindowExpired" {
                all_fail_closed = false;
            }
            if case["timeline_rebased"].as_bool().unwrap_or(false) {
                all_rebases_clean = false;
            }
            if case["catch_up_burst"].as_bool().unwrap_or(false) {
                all_catchup_clean = false;
            }
        }
    }

    let rescued_monotonic = rescued_counts.windows(2).all(|w| w[0] <= w[1]);
    let phase_c_clean =
        rescued_monotonic && all_fail_closed && all_rebases_clean && all_catchup_clean;

    json!({
        "scope": "Phase-C sparse benefit probe (gap >= 100 ms, controlled lateness to 10.5 ms)",
        "sparse_gap_us": SPARSE_GAP_US,
        "offsets_us": SPARSE_OFFSETS_US,
        "arms": arm_reports,
        "invariants": {
            "rescued_monotonic_with_tolerance": rescued_monotonic,
            "fails_closed_beyond_tolerance": all_fail_closed,
            "no_timeline_rebase": all_rebases_clean,
            "no_catch_up_burst": all_catchup_clean,
        },
        "phase_c_clean": phase_c_clean,
    })
}

fn phase_extended_range_same_key_retrigger_report() -> serde_json::Value {
    let mut cases = Vec::new();
    let mut total_pw = 0usize;

    for &tolerance_us in &EXTENDED_TOLERANCES_US {
        for &gap_us in &EXTENDED_RETRIGGER_GAPS_US {
            // Case 1: On-time Up completion (up_delay_us = 0)
            let case_on_time = run_same_key_retrigger_case(tolerance_us, gap_us, 0);
            if case_on_time["physical_window_expired"].as_bool().unwrap() {
                total_pw += 1;
            }
            cases.push(case_on_time);

            // Case 2: Delayed Up completion (up_delay_us = 5_000 us)
            let case_delayed = run_same_key_retrigger_case(tolerance_us, gap_us, 5_000);
            if case_delayed["physical_window_expired"].as_bool().unwrap() {
                total_pw += 1;
            }
            cases.push(case_delayed);
        }
    }

    let infeasible_cases_all_pw = cases.iter().all(|c| {
        let feasible = c["is_feasible"].as_bool().unwrap_or(false);
        let pw = c["physical_window_expired"].as_bool().unwrap_or(false);
        feasible != pw
    });
    let infeasible_rescues_zero = cases.iter().all(|c| {
        let feasible = c["is_feasible"].as_bool().unwrap_or(false);
        let emitted = c["packet_emitted"].as_bool().unwrap_or(false);
        feasible == emitted
    });
    let phase_d_rebases_zero = cases
        .iter()
        .all(|c| c["timeline_rebases"].as_u64().unwrap_or(0) == 0);
    let phase_d_catchup_zero = cases
        .iter()
        .all(|c| !c["catch_up_burst"].as_bool().unwrap_or(false));

    let phase_d_clean = infeasible_cases_all_pw
        && infeasible_rescues_zero
        && phase_d_rebases_zero
        && phase_d_catchup_zero;

    json!({
        "scope": "Phase-D same-key retrigger safety matrix",
        "tolerances_us": EXTENDED_TOLERANCES_US,
        "retrigger_gaps_us": EXTENDED_RETRIGGER_GAPS_US,
        "case_count": cases.len(),
        "physical_window_expired_cases": total_pw,
        "cases": cases,
        "invariants": {
            "physical_window_expired_strictly_enforced": infeasible_cases_all_pw,
            "late_completion_updates_physical_floors": infeasible_cases_all_pw,
            "min_hold_and_release_never_bypassed": infeasible_cases_all_pw,
            "tolerance_never_rescues_infeasible_retrigger": infeasible_rescues_zero,
            "no_retry_stale_down": infeasible_rescues_zero,
            "no_catch_up": phase_d_catchup_zero,
            "no_timeline_rebase": phase_d_rebases_zero,
        },
        "phase_d_clean": phase_d_clean,
    })
}

fn phase_extended_range_report() -> serde_json::Value {
    let benchmark_started = Instant::now();
    let cpu_started_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();

    let deterministic_dense = phase_extended_range_deterministic_dense_report();
    let real_wait_probe = phase_extended_range_real_wait_probe_report();
    let sparse_benefit = phase_extended_range_sparse_benefit_report();
    let same_key_retrigger = phase_extended_range_same_key_retrigger_report();

    let phase_a_clean = deterministic_dense["phase_a_clean"]
        .as_bool()
        .unwrap_or(false);
    let phase_b_clean = real_wait_probe["phase_b_clean"].as_bool().unwrap_or(false);
    let phase_c_clean = sparse_benefit["phase_c_clean"].as_bool().unwrap_or(false);
    let phase_d_clean = same_key_retrigger["phase_d_clean"]
        .as_bool()
        .unwrap_or(false);
    let acceptance_clean = phase_a_clean && phase_b_clean && phase_c_clean && phase_d_clean;

    let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();

    json!({
        "scope": "Phase-Extended-Range qualification (2.0 / 2.5 / 5.0 / 7.5 / 10.0 ms)",
        "phase_a_deterministic_dense": deterministic_dense,
        "phase_b_real_wait_probe": real_wait_probe,
        "phase_c_sparse_benefit": sparse_benefit,
        "phase_d_same_key_retrigger": same_key_retrigger,
        "acceptance_clean": acceptance_clean,
        "process_cpu_time_us": cpu_finished_us.saturating_sub(cpu_started_us),
        "process_cpu_duty_percent": cpu_duty_percent(
            cpu_started_us,
            cpu_finished_us,
            benchmark_started,
        ),
    })
}

fn summarize(samples: Samples) -> serde_json::Value {
    summarize_for_attempts(samples, iterations())
}

fn summarize_for_attempts(mut samples: Samples, expected_attempts: usize) -> serde_json::Value {
    assert!(
        samples.dispatch_start_error_us.len() <= samples.observation_count,
        "timing samples cannot exceed collected observations"
    );
    assert_eq!(
        samples.physical_dispatches + samples.non_dispatches,
        expected_attempts,
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
    let statistics_eligible = acceptance_clean && expected_attempts >= 10_000;
    let sender_cutoff_exercised = true;
    let total_spin_time_us = samples.spin_time_us.iter().copied().sum::<u64>();
    let total_wall_time_us = samples.wall_time_us.iter().copied().sum::<u64>();
    let spin_duty = spin_duty_cycle_ppm(&samples.spin_time_us, &samples.wall_time_us);
    let counterfactual_rescue = counterfactual_rescue_summary(&samples);
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
        "wake_to_final_policy_us": signed_summary(samples.wake_to_final_policy_us),
        "final_policy_to_pre_call_us": signed_summary(samples.final_policy_to_pre_call_us),
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
        "wait_evidence": {
            "planned_gap_us": unsigned_summary(samples.planned_wait_gap_us),
            "wake_lateness_us": signed_summary(samples.wait_wake_lateness_us),
            "hot_count": samples.hot_wait_count,
            "cold_count": samples.cold_wait_count,
            "cold_threshold_us": SEND_COLD_THRESHOLD_US,
        },
        "missed_down": {
            "unobserved_backlog": samples.missed_down_unobserved_backlog,
            "physical_window_expired": samples.missed_down_physical_window_expired,
            "final_sender_window_expired": samples.missed_down_final_sender_window_expired,
        },
        "missed_pre_call_lateness_us": signed_summary(samples.missed_pre_call_lateness_us),
        "missed_excess_beyond_latest_start_us": signed_summary(
            samples.missed_excess_beyond_latest_start_us,
        ),
        "late_rescue": {
            "rescued_down_boundaries": samples.late_rescued_down_boundaries,
            "rescued_down_keys": samples.late_rescued_down_keys,
            "total_lateness_us": unsigned_summary(samples.late_rescued_down_lateness_us),
            "excess_beyond_physical_latest_us": unsigned_summary(
                samples.late_rescued_down_excess_us,
            ),
        },
        "transport_anomaly_count": samples.transport_anomaly_count,
        "counterfactual_late_rescue": counterfactual_rescue,
        "spin_time_us": unsigned_summary(samples.spin_time_us),
        "wall_time_us": unsigned_summary(samples.wall_time_us),
        "total_spin_time_us": total_spin_time_us,
        "total_wall_time_us": total_wall_time_us,
        "spin_duty_cycle_ppm": spin_duty,
    })
}

fn all_scenarios_clean(mode_reports: &serde_json::Map<String, serde_json::Value>) -> bool {
    mode_reports.values().all(|mode| {
        if let Some(clean) = mode
            .get("acceptance_clean")
            .and_then(serde_json::Value::as_bool)
        {
            return clean;
        }
        let scenarios_clean = |scenarios: &serde_json::Map<String, serde_json::Value>| {
            scenarios.values().all(|scenario| {
                scenario
                    .get("acceptance_clean")
                    .and_then(serde_json::Value::as_bool)
                    == Some(true)
            })
        };
        if let Some(scenarios) = mode.get("scenarios").and_then(serde_json::Value::as_object) {
            scenarios_clean(scenarios)
        } else {
            mode.get("modes")
                .and_then(serde_json::Value::as_object)
                .is_some_and(|modes| {
                    modes.values().all(|mode| {
                        mode.get("scenarios")
                            .and_then(serde_json::Value::as_object)
                            .is_some_and(scenarios_clean)
                    })
                })
        }
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
    let benchmark_mode =
        BenchmarkMode::from_env_or_args().unwrap_or_else(|error| panic!("{error}"));
    let benchmark_scope =
        BenchmarkScope::from_env_or_args().unwrap_or_else(|error| panic!("{error}"));
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
    if matches!(benchmark_scope, BenchmarkScope::PhaseASparseGap)
        && !matches!(benchmark_mode, BenchmarkMode::RealWait)
    {
        panic!("phase_a_sparse_gap requires real_wait benchmark mode");
    }
    if matches!(benchmark_scope, BenchmarkScope::PhaseBB0)
        && !matches!(benchmark_mode, BenchmarkMode::RealWait)
    {
        panic!("phase_b0 requires real_wait benchmark mode");
    }
    if matches!(benchmark_scope, BenchmarkScope::PhaseCC0)
        && !matches!(benchmark_mode, BenchmarkMode::RealWait)
    {
        panic!("phase_c0 requires real_wait benchmark mode");
    }
    if matches!(benchmark_scope, BenchmarkScope::PhaseCC1)
        && !matches!(benchmark_mode, BenchmarkMode::RealWait)
    {
        panic!("phase_c1 requires real_wait benchmark mode");
    }
    if matches!(benchmark_scope, BenchmarkScope::PhaseCC11)
        && !matches!(benchmark_mode, BenchmarkMode::RealWait)
    {
        panic!("phase_c1_1 requires real_wait benchmark mode");
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
    } else if matches!(benchmark_scope, BenchmarkScope::PhaseASparseGap) {
        mode_reports.insert(
            "phase_a_sparse_gap".to_string(),
            phase_a_sparse_gap_report(),
        );
    } else if matches!(benchmark_scope, BenchmarkScope::PhaseBB0) {
        mode_reports.insert("phase_b0".to_string(), phase_b0_report());
    } else if matches!(benchmark_scope, BenchmarkScope::PhaseCC0) {
        mode_reports.insert("phase_c0".to_string(), phase_c0_report());
    } else if matches!(benchmark_scope, BenchmarkScope::PhaseCC1) {
        mode_reports.insert("phase_c1".to_string(), phase_c1_report());
    } else if matches!(benchmark_scope, BenchmarkScope::PhaseCC11) {
        mode_reports.insert("phase_c1_1".to_string(), phase_c1_1_report());
    } else if matches!(benchmark_scope, BenchmarkScope::PhaseF11) {
        mode_reports.insert("phase_f1_1".to_string(), phase_f1_1_report());
    } else if matches!(benchmark_scope, BenchmarkScope::PhaseExtendedRange) {
        mode_reports.insert(
            "phase_extended_range".to_string(),
            phase_extended_range_report(),
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
            (BenchmarkScope::PhaseASparseGap, BenchmarkMode::RealWait) => "Phase-A sparse-gap evidence through the production coordinator and real HybridWaiter with deterministic mock transport; no playback wait policy or authored target is changed; not Raw Input or game-observed latency",
            (BenchmarkScope::PhaseASparseGap, _) => "invalid benchmark scope/mode combination",
            (BenchmarkScope::PhaseBB0, BenchmarkMode::RealWait) => "Phase-B0 benchmark-only interleaved spin-threshold qualification through the production coordinator and real HybridWaiter with deterministic mock transport; no production wait policy or authored target is changed; not Raw Input or game-observed latency",
            (BenchmarkScope::PhaseBB0, _) => "invalid benchmark scope/mode combination",
            (BenchmarkScope::PhaseCC0, BenchmarkMode::RealWait) => "Phase-C0 calibrated-only stability benchmark and counterfactual late-rescue analysis through the production coordinator and real HybridWaiter with deterministic mock transport; no production wait policy, authored target, physical feasibility, or sender cutoff is changed; not Raw Input or game-observed latency",
            (BenchmarkScope::PhaseCC0, _) => "invalid benchmark scope/mode combination",
            (BenchmarkScope::PhaseCC1, BenchmarkMode::RealWait) => "Phase-C1 paired A/B benchmark through the production coordinator and real HybridWaiter with deterministic mock transport; arm A uses the current physical cutoff and arm B uses the approved non-additive 2500us normal-playback continuity cutoff; physical feasibility, strict mode, and spin policy remain unchanged; not Raw Input or game-observed latency",
            (BenchmarkScope::PhaseCC1, _) => "invalid benchmark scope/mode combination",
            (BenchmarkScope::PhaseCC11, BenchmarkMode::RealWait) => "Phase-C1.1 tri-arm benchmark plus deterministic and real sequential dense-boundary probes through the production coordinator and real HybridWaiter with deterministic mock transport; current, non-additive 1500us, and non-additive 2500us sender cutoff arms are compared without changing production policy, physical feasibility, strict mode, spin policy, Win32 sender, desktop, or schema; not Raw Input or game-observed latency",
            (BenchmarkScope::PhaseCC11, _) => "invalid benchmark scope/mode combination",
            (BenchmarkScope::PhaseF11, BenchmarkMode::RealWait) => "Phase-F1.1 qualification for normal-playback late Down continuity tolerance: real-wait sequential dense-boundary probe comparing 2.5 ms, 3.5 ms, and 5.0 ms across 1-5 ms gaps plus sparse 2.5 ms vs 5.0 ms comparison proving Note-Ons rescued; not Raw Input or game-observed latency",
            (BenchmarkScope::PhaseF11, _) => "invalid benchmark scope/mode combination",
            (BenchmarkScope::PhaseExtendedRange, BenchmarkMode::RealWait) => "Phase-Extended-Range qualification for normal-playback late Down continuity tolerance: deterministic dense matrix, real-wait probe, sparse comparison, and same-key retrigger safety across 2.0 to 10.0 ms range; not Raw Input or game-observed latency",
            (BenchmarkScope::PhaseExtendedRange, _) => "invalid benchmark scope/mode combination",
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
