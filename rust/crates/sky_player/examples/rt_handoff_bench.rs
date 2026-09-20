//! Small RT handoff benchmark used for before/after comparison.
//!
//! This deliberately has no Criterion dependency.  The plan is built before
//! the real QPC wait, then the deterministic test emitter exercises admission,
//! packet construction, coordinator commit, and the fixed observation enqueue.
//! It is compiled only with `test-support`; production binaries do not contain
//! this harness.

#![cfg(feature = "test-support")]

use serde::Serialize;
use serde_json::json;
use sky_dispatch_core::time::{SEND_COLD_THRESHOLD_US, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcTicks, qpc_frequency_checked};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::input::{
    PhysicalPacket, PreparedPhysicalPacket, SendTransactionOutcome, SendTransactionStatus,
};
use sky_dispatch_win32::wait::{HybridWaiter, WaitOutcome, WakeErrorStats};
use sky_player::engine::dispatch_primitives::{
    DispatchObservation, DispatchPath, DispatchStep, DownMissKind, NextDispatchPlan,
    OBSERVATION_QUEUE_CAPACITY, PendingObservationQueue, PhysicalFloorEvidence,
    PrecisionHandoffEvidence, PreparationCounts, ProductionDispatchTestHarness,
};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::atomic::Ordering;
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

fn require_focus_for_benchmark() -> bool {
    matches!(
        std::env::var("RT_HANDOFF_BENCH_REQUIRE_FOCUS").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

fn configure_focus_for_benchmark(harness: &mut ProductionDispatchTestHarness) {
    if require_focus_for_benchmark() {
        harness.set_require_focus_for_benchmark(true);
    }
    sky_dispatch_win32::focus::reset_foreground_query_count();
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
    Baseline,
    RealWaitCore,
    PhaseASenderOnly,
    PhaseAProductionMatrix,
    PhaseASparseGap,
    PhaseBB0,
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
            "baseline" => Ok(Self::Baseline),
            "real_wait_core" => Ok(Self::RealWaitCore),
            "phase_a_sender_only" => Ok(Self::PhaseASenderOnly),
            "phase_a_production_matrix" => Ok(Self::PhaseAProductionMatrix),
            "phase_a_sparse_gap" => Ok(Self::PhaseASparseGap),
            "phase_b0" => Ok(Self::PhaseBB0),
            value => Err(format!(
                "scope must be full, baseline, real_wait_core, phase_a_sender_only, phase_a_production_matrix, phase_a_sparse_gap, or phase_b0, got {value:?}"
            )),
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Baseline => "baseline",
            Self::RealWaitCore => "real_wait_core",
            Self::PhaseASenderOnly => "phase_a_sender_only",
            Self::PhaseAProductionMatrix => "phase_a_production_matrix",
            Self::PhaseASparseGap => "phase_a_sparse_gap",
            Self::PhaseBB0 => "phase_b0",
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
    target_minus_wait_entry_us: Vec<i64>,
    alignment_to_wait_entry_us: Vec<i64>,
    prepared_stream_build_us: Vec<u64>,
    hot_wait_count: usize,
    cold_wait_count: usize,
    missed_pre_call_lateness_us: Vec<i64>,
    missed_excess_beyond_latest_start_us: Vec<i64>,
    missed_down_unobserved_backlog: usize,
    missed_down_physical_window_expired: usize,
    missed_down_final_sender_window_expired: usize,
    transport_anomaly_count: usize,
    foreground_query_count: Vec<u64>,
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
            target_minus_wait_entry_us,
            alignment_to_wait_entry_us,
            prepared_stream_build_us,
            missed_pre_call_lateness_us,
            missed_excess_beyond_latest_start_us,
            foreground_query_count,
            spin_time_us,
            wall_time_us,
        );
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
    Samples::default()
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

#[derive(Debug, Serialize)]
struct BaselineBoundaryEvidence {
    scenario: &'static str,
    source_action_index: u32,
    compiled_packet_index: Option<u64>,
    up_mask: u16,
    down_mask: u16,
    physical_target_qpc: u64,
    crossing_qpc: Option<u64>,
    wake_qpc: Option<u64>,
    pre_call_qpc: Option<u64>,
    completion_qpc: Option<u64>,
    pre_call_minus_target_us: Option<i64>,
    completion_minus_pre_call_us: Option<u64>,
    send_status: Option<&'static str>,
    dispatch_step: &'static str,
    observed_disposition: &'static str,
    delivery: &'static str,
    non_send_reason: Option<&'static str>,
    expected_non_send_reason: Option<&'static str>,
    sender_window_expired: bool,
}

#[derive(Default)]
struct BaselineEvidence {
    boundaries: Vec<BaselineBoundaryEvidence>,
    successful_sendinput_transactions: usize,
    non_send_or_drop_boundaries: usize,
    timeline_rebase_count: u64,
    transport_anomaly_count: u64,
}

fn baseline_status_name(status: SendTransactionStatus) -> &'static str {
    match status {
        SendTransactionStatus::Complete => "Complete",
        SendTransactionStatus::PreparationRejected => "PreparationRejected",
        SendTransactionStatus::ZeroProgress => "ZeroProgress",
        SendTransactionStatus::PartialProgress => "PartialProgress",
        SendTransactionStatus::IntegrityLost => "IntegrityLost",
        SendTransactionStatus::DownExpiredBeforeSend => "DownExpiredBeforeSend",
        SendTransactionStatus::ClockFailureBeforeSend => "ClockFailureBeforeSend",
        SendTransactionStatus::ClockFailureAfterSend => "ClockFailureAfterSend",
    }
}

fn baseline_dispatch_step_name(step: &DispatchStep) -> &'static str {
    match step {
        DispatchStep::NoWork => "NoWork",
        DispatchStep::Dispatched => "Dispatched",
        DispatchStep::Continue => "Continue",
        DispatchStep::Terminate(_) => "Terminate",
        DispatchStep::TerminateStatic(_) => "TerminateStatic",
    }
}

fn baseline_down_miss_name(kind: DownMissKind) -> &'static str {
    match kind {
        DownMissKind::UnobservedBacklog => "UnobservedBacklog",
        DownMissKind::PhysicalWindowExpired => "PhysicalWindowExpired",
        DownMissKind::DownExpiredBeforeSend => "DownExpiredBeforeSend",
    }
}

#[allow(clippy::type_complexity)]
fn baseline_observed_disposition(
    step: &DispatchStep,
    outcome: Option<SendTransactionOutcome>,
    observation: Option<DispatchObservation>,
    final_gate_control_rejections: u64,
    sender_window_expired: bool,
) -> (
    &'static str,
    Option<&'static str>,
    Option<&'static str>,
    bool,
) {
    let from_status = |status: SendTransactionStatus| {
        if matches!(status, SendTransactionStatus::Complete) {
            (
                "successful_send",
                None,
                Some(baseline_status_name(status)),
                true,
            )
        } else {
            (
                "sender_status",
                Some(baseline_status_name(status)),
                Some(baseline_status_name(status)),
                false,
            )
        }
    };
    if let Some(outcome) = outcome {
        return from_status(outcome.status);
    }
    if let Some(observation) = observation {
        match observation {
            DispatchObservation::Down(value) => return from_status(value.trace.result_status),
            DispatchObservation::Up(value) => return from_status(value.result_status),
            DispatchObservation::DownMiss(value) => {
                return (
                    "down_miss",
                    Some(baseline_down_miss_name(value.kind)),
                    None,
                    false,
                );
            }
            DispatchObservation::Wait(_)
            | DispatchObservation::StaleMetadata(_)
            | DispatchObservation::BlockedUnfocused(_) => {}
        }
    }
    if final_gate_control_rejections != 0 {
        return ("control_disposition", Some("control_rejected"), None, false);
    }
    if sender_window_expired {
        return (
            "sender_cutoff_without_observation",
            Some("sender_window_expiration_without_observation"),
            None,
            false,
        );
    }
    match step {
        DispatchStep::Continue => (
            "dispatch_control",
            Some("dispatch_continue_without_observation"),
            None,
            false,
        ),
        DispatchStep::NoWork => ("no_work", Some("no_work"), None, false),
        DispatchStep::Dispatched => (
            "dispatch_disposition",
            Some("dispatched_without_send_evidence"),
            None,
            false,
        ),
        DispatchStep::Terminate(_) | DispatchStep::TerminateStatic(_) => (
            "dispatch_disposition",
            Some("dispatch_terminated"),
            None,
            false,
        ),
    }
}

#[allow(clippy::type_complexity)]
fn baseline_observation_timing(
    observation: DispatchObservation,
) -> (
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Option<u64>,
    Option<i64>,
    Option<u64>,
) {
    let qpc_clock = QpcClock::initialize().expect("baseline QPC clock");
    match observation {
        DispatchObservation::Down(value) => {
            let crossing_qpc = value
                .precision_handoff
                .map(|handoff| handoff.target_crossing_qpc.as_u64());
            let wake_qpc = value
                .wake_qpc
                .or_else(|| {
                    value
                        .precision_handoff
                        .and_then(|handoff| handoff.admission_wake_qpc)
                })
                .map(|ticks| ticks.as_u64());
            let pre_call_qpc = Some(value.pre_call_qpc.as_u64());
            let completion_qpc = Some(value.sendinput_completion_qpc.as_u64());
            let completion_duration = qpc_clock
                .duration_to_us(
                    value
                        .sendinput_completion_qpc
                        .checked_duration_since(value.pre_call_qpc)
                        .expect("baseline completion ordering"),
                )
                .ok();
            (
                crossing_qpc,
                wake_qpc,
                pre_call_qpc,
                completion_qpc,
                Some(signed_qpc_us(
                    qpc_clock,
                    value.pre_call_qpc,
                    value.physical_target_qpc,
                )),
                completion_duration,
            )
        }
        DispatchObservation::Up(value) => {
            let crossing_qpc = value
                .precision_handoff
                .map(|handoff| handoff.target_crossing_qpc.as_u64());
            let wake_qpc = value
                .wake_qpc
                .or_else(|| {
                    value
                        .precision_handoff
                        .and_then(|handoff| handoff.admission_wake_qpc)
                })
                .map(|ticks| ticks.as_u64());
            let pre_call_qpc = Some(value.pre_call_qpc.as_u64());
            let completion_qpc = Some(value.sendinput_completion_qpc.as_u64());
            let completion_duration = qpc_clock
                .duration_to_us(value.pre_call_to_completion_ticks)
                .ok();
            (
                crossing_qpc,
                wake_qpc,
                pre_call_qpc,
                completion_qpc,
                Some(signed_qpc_us(
                    qpc_clock,
                    value.pre_call_qpc,
                    value.physical_target_qpc,
                )),
                completion_duration,
            )
        }
        DispatchObservation::DownMiss(value) => {
            let pre_call_qpc = matches!(value.kind, DownMissKind::DownExpiredBeforeSend)
                .then_some(value.observed_qpc.as_u64());
            let pre_call_minus_target_us = pre_call_qpc.map(|pre_call| {
                signed_qpc_us(
                    qpc_clock,
                    QpcTicks::from_raw(pre_call),
                    value.physical_authored_target_qpc(),
                )
            });
            (
                None,
                None,
                pre_call_qpc,
                None,
                pre_call_minus_target_us,
                None,
            )
        }
        DispatchObservation::Wait(_)
        | DispatchObservation::StaleMetadata(_)
        | DispatchObservation::BlockedUnfocused(_) => (None, None, None, None, None, None),
    }
}

#[allow(clippy::too_many_arguments)]
fn baseline_record_boundary(
    evidence: &mut BaselineEvidence,
    scenario: &'static str,
    harness: &mut ProductionDispatchTestHarness,
    plan: &NextDispatchPlan,
    crossing_lateness_us: u64,
    now_qpc: Option<QpcTicks>,
    completion_delay_us: u64,
    inject_zero_progress_drop: bool,
    expected_non_send_reason: Option<&'static str>,
) {
    let prepared = harness
        .prepared_boundary_evidence_for_test(plan)
        .expect("baseline case must prepare one physical boundary");
    let target = prepared.physical_target_qpc;
    let crossing = target
        .checked_add_duration(
            harness
                .qpc_duration_from_us_for_test(crossing_lateness_us)
                .expect("baseline crossing conversion"),
        )
        .expect("baseline crossing arithmetic");
    let prior_sender_expirations = harness.final_sender_window_expirations_for_test();
    let prior_control_rejections = harness.final_gate_control_rejections_for_test();
    let prior_timeline_rebases = harness.timeline_rebase_count_for_test();
    let prior_anomalies = harness.transport_anomaly_counts_for_test();
    let (step, outcome) = match now_qpc {
        Some(now_qpc) => harness.dispatch_at_baseline_boundary_at_qpc_for_test(
            plan,
            now_qpc,
            crossing,
            completion_delay_us,
            inject_zero_progress_drop,
        ),
        None => harness.dispatch_at_baseline_boundary_for_test(
            plan,
            crossing_lateness_us,
            completion_delay_us,
            inject_zero_progress_drop,
        ),
    };
    let observation = harness.pop_observation();
    let observation_timing = observation.map(baseline_observation_timing);
    let sender_expired =
        harness.final_sender_window_expirations_for_test() > prior_sender_expirations;
    let control_rejections = harness
        .final_gate_control_rejections_for_test()
        .saturating_sub(prior_control_rejections);
    let (observed_disposition, observed_reason, observed_send_status, observed_success) =
        baseline_observed_disposition(
            &step,
            outcome,
            observation,
            control_rejections,
            sender_expired,
        );
    let delivery = if observed_success {
        evidence.successful_sendinput_transactions =
            evidence.successful_sendinput_transactions.saturating_add(1);
        "successful_send"
    } else {
        evidence.non_send_or_drop_boundaries =
            evidence.non_send_or_drop_boundaries.saturating_add(1);
        "non_send"
    };
    let (
        observation_crossing,
        wake_qpc,
        observed_pre_call,
        observed_completion,
        observed_residual,
        observed_duration,
    ) = observation_timing.unwrap_or((None, None, None, None, None, None));
    let (outcome_pre_call, outcome_completion, outcome_residual, outcome_duration) = outcome
        .map(|value| {
            let pre_call = value.evidence.started_ticks.map(|ticks| ticks.as_u64());
            let completion = value.evidence.completed_ticks.map(|ticks| ticks.as_u64());
            let residual = pre_call.map(|ticks| {
                signed_qpc_us(
                    QpcClock::initialize().expect("baseline QPC clock"),
                    QpcTicks::from_raw(ticks),
                    target,
                )
            });
            let duration = value.evidence.duration_ticks().ok().and_then(|ticks| {
                QpcClock::initialize()
                    .ok()
                    .and_then(|clock| clock.duration_to_us(ticks).ok())
            });
            (pre_call, completion, residual, duration)
        })
        .unwrap_or((None, None, None, None));
    evidence.boundaries.push(BaselineBoundaryEvidence {
        scenario,
        source_action_index: prepared.source_action_index,
        compiled_packet_index: prepared.compiled_packet_index,
        up_mask: prepared.packet.up_mask,
        down_mask: prepared.packet.down_mask,
        physical_target_qpc: target.as_u64(),
        crossing_qpc: observation_crossing.or(Some(crossing.as_u64())),
        wake_qpc,
        pre_call_qpc: outcome_pre_call.or(observed_pre_call),
        completion_qpc: outcome_completion.or(observed_completion),
        pre_call_minus_target_us: outcome_residual.or(observed_residual),
        completion_minus_pre_call_us: outcome_duration.or(observed_duration),
        send_status: observed_send_status,
        dispatch_step: baseline_dispatch_step_name(&step),
        observed_disposition,
        delivery,
        non_send_reason: observed_reason,
        expected_non_send_reason,
        sender_window_expired: sender_expired,
    });
    evidence.timeline_rebase_count = evidence.timeline_rebase_count.saturating_add(
        harness
            .timeline_rebase_count_for_test()
            .saturating_sub(prior_timeline_rebases),
    );
    let anomalies = harness.transport_anomaly_counts_for_test();
    evidence.transport_anomaly_count = evidence.transport_anomaly_count.saturating_add(
        anomalies
            .0
            .saturating_add(anomalies.1)
            .saturating_add(anomalies.2)
            .saturating_sub(
                prior_anomalies
                    .0
                    .saturating_add(prior_anomalies.1)
                    .saturating_add(prior_anomalies.2),
            ),
    );
}

fn baseline_floor_json(
    prepared: sky_player::engine::dispatch_primitives::PreparedBoundaryEvidence,
    floor: PhysicalFloorEvidence,
) -> serde_json::Value {
    json!({
        "source_action_index": prepared.source_action_index,
        "compiled_packet_index": prepared.compiled_packet_index,
        "up_mask": prepared.packet.up_mask,
        "down_mask": prepared.packet.down_mask,
        "authored_ticks": prepared.authored_ticks.as_u64(),
        "effective_deadline_ticks": prepared.effective_deadline_ticks.as_u64(),
        "authored_target_qpc": floor.authored_target_qpc.as_u64(),
        "physical_target_qpc": prepared.physical_target_qpc.as_u64(),
        "musical_up_not_before_qpc": floor.musical_up_not_before_qpc.as_u64(),
        "down_not_before_qpc": floor.down_not_before_qpc.as_u64(),
        "packet_not_before_qpc": floor.packet_not_before_qpc.as_u64(),
        "latest_down_start_qpc": floor.latest_down_start_qpc.map(QpcTicks::as_u64),
        "hold_floor_mask": floor.hold_floor_mask,
        "release_floor_mask": floor.release_floor_mask,
        "down_feasible": floor.down_feasible,
        "packet_not_before_after_authored_qpc": floor
            .packet_not_before_qpc
            .as_u64()
            .saturating_sub(floor.authored_target_qpc.as_u64()),
    })
}

fn baseline_completion_floor_variant(
    evidence: &mut BaselineEvidence,
    scenario: &'static str,
    harness: &mut ProductionDispatchTestHarness,
    completion_delay_us: u64,
) -> serde_json::Value {
    let packet_n = harness.plan_current_dispatch();
    baseline_record_boundary(
        evidence,
        scenario,
        harness,
        &packet_n,
        0,
        None,
        completion_delay_us,
        false,
        None,
    );
    let packet_n_evidence = evidence
        .boundaries
        .last()
        .expect("completion-floor packet N evidence");
    let packet_n1 = harness.plan_current_dispatch();
    let prepared_n1 = harness
        .prepared_boundary_evidence_for_test(&packet_n1)
        .expect("completion-floor packet N+1 preparation");
    let floor_n1 = harness
        .physical_floor_evidence_for_test(&packet_n1)
        .expect("completion-floor packet N+1 window");
    let wait_target_n1 = harness
        .physical_wait_target_for_test(&packet_n1)
        .expect("completion-floor packet N+1 wait target")
        .expect("completion-floor packet N+1 wait target exists");
    let packets_n1 = harness.configure_packet_capture();
    let n1_step = harness.dispatch_at_qpc_for_test(&packet_n1, wait_target_n1);
    let n1_packet_count = packets_n1
        .lock()
        .expect("completion-floor packet capture")
        .len();
    let n1_send_eligible = matches!(n1_step, DispatchStep::Dispatched) && n1_packet_count == 1;
    json!({
        "variant": scenario,
        "completion_delay_us": completion_delay_us,
        "packet_n": {
            "source_action_index": packet_n_evidence.source_action_index,
            "physical_target_qpc": packet_n_evidence.physical_target_qpc,
            "completion_qpc": packet_n_evidence.completion_qpc,
            "send_status": packet_n_evidence.send_status,
            "delivery": packet_n_evidence.delivery,
        },
        "packet_n_plus_one": baseline_floor_json(prepared_n1, floor_n1),
        "packet_n_plus_one_wait_target_qpc": wait_target_n1.as_u64(),
        "packet_n_plus_one_send_step": format!("{n1_step:?}"),
        "packet_n_plus_one_send_eligibility": n1_send_eligible,
        "packet_n_plus_one_classification": if n1_send_eligible {
            "successful_send"
        } else {
            "non_send"
        },
        "packet_n_plus_one_packet_count": n1_packet_count,
        "physical_window_expired_boundaries": harness
            .missed_physical_window_boundaries_for_test(),
    })
}

fn baseline_completion_floor_report(evidence: &mut BaselineEvidence) -> serde_json::Value {
    let mut control = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 30_000);
    control.align_next_plan_to_future_for_test(100_000);
    let shared_epoch = control.playback_epoch_qpc_for_test();
    let mut delayed = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 30_000);
    delayed.set_playback_epoch_qpc_for_test(shared_epoch);

    let control_report = baseline_completion_floor_variant(
        evidence,
        "completion_floor_control",
        &mut control,
        BASELINE_COMPLETION_DELAY_US,
    );
    let delayed_report = baseline_completion_floor_variant(
        evidence,
        "completion_floor_delayed",
        &mut delayed,
        40_000,
    );
    let control_next = &control_report["packet_n_plus_one"];
    let delayed_next = &delayed_report["packet_n_plus_one"];
    let same_authored_schedule = control_next["authored_ticks"] == delayed_next["authored_ticks"]
        && control_next["effective_deadline_ticks"] == delayed_next["effective_deadline_ticks"];
    let same_physical_target =
        control_next["physical_target_qpc"] == delayed_next["physical_target_qpc"];
    let same_packet_not_before =
        control_next["packet_not_before_qpc"] == delayed_next["packet_not_before_qpc"];
    let same_wait_target = control_report["packet_n_plus_one_wait_target_qpc"]
        == delayed_report["packet_n_plus_one_wait_target_qpc"];
    let same_send_eligibility = control_report["packet_n_plus_one_send_eligibility"]
        == delayed_report["packet_n_plus_one_send_eligibility"];
    let same_classification = control_report["packet_n_plus_one_classification"]
        == delayed_report["packet_n_plus_one_classification"];
    let same_coordinator_authored_timestamps = control_next["authored_ticks"]
        == delayed_next["authored_ticks"]
        && control_next["effective_deadline_ticks"] == delayed_next["effective_deadline_ticks"];
    let normal_floor_masks_zero = [
        &control_next["hold_floor_mask"],
        &control_next["release_floor_mask"],
        &delayed_next["hold_floor_mask"],
        &delayed_next["release_floor_mask"],
    ]
    .into_iter()
    .all(|mask| mask == &json!(0));
    let completion_qpc_differs = control_report["packet_n"]["completion_qpc"]
        != delayed_report["packet_n"]["completion_qpc"];
    let control_is_not_pressured =
        control_next["packet_not_before_qpc"] == control_next["authored_target_qpc"];
    let delayed_does_not_move_later_scheduling = same_authored_schedule
        && same_physical_target
        && same_packet_not_before
        && same_wait_target
        && same_send_eligibility
        && same_classification
        && same_coordinator_authored_timestamps
        && normal_floor_masks_zero;
    json!({
        "same_authored_prepared_schedule": same_authored_schedule,
        "same_packet_n_plus_one_target": same_physical_target,
        "same_packet_n_plus_one_packet_not_before": same_packet_not_before,
        "same_packet_n_plus_one_wait_target": same_wait_target,
        "same_packet_n_plus_one_send_eligibility": same_send_eligibility,
        "same_packet_n_plus_one_classification": same_classification,
        "same_coordinator_authored_timestamps": same_coordinator_authored_timestamps,
        "normal_hold_release_floor_masks_zero": normal_floor_masks_zero,
        "completion_qpc_of_packet_n_differs": completion_qpc_differs,
        "control": control_report,
        "delayed_completion": delayed_report,
        "delayed_completion_does_not_move_later_scheduling": delayed_does_not_move_later_scheduling,
        "control_has_no_downstream_floor_pressure": control_is_not_pressured,
        "completion_feedback_physical_window_expired": control_report[
            "physical_window_expired_boundaries"
        ]
        .as_u64()
        .unwrap_or(0)
            != 0
            || delayed_report["physical_window_expired_boundaries"]
                .as_u64()
                .unwrap_or(0)
                != 0,
        "acceptance_clean": delayed_does_not_move_later_scheduling
            && completion_qpc_differs
            && control_is_not_pressured
            && control_report["physical_window_expired_boundaries"] == json!(0)
            && delayed_report["physical_window_expired_boundaries"] == json!(0),
        "policy_changed": true,
    })
}

fn baseline_prepared_stream_report() -> serde_json::Value {
    let harness = ProductionDispatchTestHarness::new_same_key_retrigger_with_gap_for_test(20_000);
    let (entry_count, physical_count) = harness.prepared_stream_counts_for_test();
    json!({
        "construction": "session-startup test-support preparation contract",
        "entry_count": entry_count,
        "physical_frame_count": physical_count,
        "expected_physical_frame_count": 4,
        "metadata_entries": entry_count.saturating_sub(physical_count),
        "offsets_immutable": true,
        "deferred_up_mask_zero": true,
        "authored_same_key_order": "Down(K) -> Up(K) -> Down(K) -> Up(K)",
        "acceptance_clean": physical_count == 4,
    })
}

#[allow(clippy::too_many_arguments)]
fn baseline_semantic_expectation(
    boundaries: &[BaselineBoundaryEvidence],
    scenario: &'static str,
    expected_count: usize,
    expected_sources: &[u32],
    expected_packets: &[(u16, u16)],
    expected_delivery: &[&'static str],
    expected_statuses: &[Option<&'static str>],
    expected_reasons: &[Option<&'static str>],
    expected_steps: &[&'static str],
) -> serde_json::Value {
    let observed: Vec<&BaselineBoundaryEvidence> = boundaries
        .iter()
        .filter(|boundary| boundary.scenario == scenario)
        .collect();
    let observed_json: Vec<serde_json::Value> = observed
        .iter()
        .map(|boundary| {
            json!({
                "source_action_index": boundary.source_action_index,
                "up_mask": boundary.up_mask,
                "down_mask": boundary.down_mask,
                "delivery": boundary.delivery,
                "send_status": boundary.send_status,
                "non_send_reason": boundary.non_send_reason,
                "dispatch_step": boundary.dispatch_step,
                "observed_disposition": boundary.observed_disposition,
                "expected_non_send_reason": boundary.expected_non_send_reason,
            })
        })
        .collect();
    let observed_matches = observed.len() == expected_count
        && observed
            .iter()
            .zip(expected_sources.iter())
            .all(|(boundary, expected)| boundary.source_action_index == *expected)
        && observed
            .iter()
            .zip(expected_packets.iter())
            .all(|(boundary, expected)| (boundary.up_mask, boundary.down_mask) == *expected)
        && observed
            .iter()
            .zip(expected_delivery.iter())
            .all(|(boundary, expected)| boundary.delivery == *expected)
        && observed
            .iter()
            .zip(expected_statuses.iter())
            .all(|(boundary, expected)| boundary.send_status == *expected)
        && observed
            .iter()
            .zip(expected_reasons.iter())
            .all(|(boundary, expected)| boundary.non_send_reason == *expected)
        && observed
            .iter()
            .zip(expected_steps.iter())
            .all(|(boundary, expected)| boundary.dispatch_step == *expected);
    json!({
        "scenario": scenario,
        "expected": {
            "count": expected_count,
            "source_action_indices": expected_sources,
            "packets": expected_packets,
            "delivery": expected_delivery,
            "send_status": expected_statuses,
            "non_send_reason": expected_reasons,
            "dispatch_step": expected_steps,
        },
        "observed": observed_json,
        "pass": observed_matches,
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
    samples
        .foreground_query_count
        .push(sky_dispatch_win32::focus::foreground_query_count());
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

fn record_prepared_benchmark_evidence(
    samples: &mut Samples,
    harness: &ProductionDispatchTestHarness,
) -> Result<(), String> {
    let Some((alignment_qpc, target_qpc, wait_entry_qpc)) =
        harness.prepared_benchmark_qpc_evidence_for_test()
    else {
        return Err("prepared benchmark timing evidence was not recorded".to_string());
    };
    let qpc_clock = QpcClock::initialize().map_err(|error| format!("QPC: {error:?}"))?;
    samples
        .target_minus_wait_entry_us
        .push(signed_qpc_us(qpc_clock, target_qpc, wait_entry_qpc));
    samples.alignment_to_wait_entry_us.push(signed_qpc_us(
        qpc_clock,
        wait_entry_qpc,
        alignment_qpc,
    ));
    let build_duration_us = harness
        .prepared_stream_build_duration_us_for_test()
        .ok_or_else(|| "prepared stream build duration was not recorded".to_string())?;
    samples.prepared_stream_build_us.push(build_duration_us);
    Ok(())
}

fn record_harness_metrics(
    samples: &mut Samples,
    harness: &mut ProductionDispatchTestHarness,
) -> Result<(), String> {
    let (partial, zero_progress, integrity_lost) = harness.transport_anomaly_counts_for_test();
    let anomalies = partial
        .saturating_add(zero_progress)
        .saturating_add(integrity_lost);
    samples.transport_anomaly_count = samples
        .transport_anomaly_count
        .saturating_add(usize::try_from(anomalies).unwrap_or(usize::MAX));
    Ok(())
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
    let iteration_started = Instant::now();
    let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(key_count, gap_us);
    configure_focus_for_benchmark(&mut harness);
    harness.enable_dispatch_ready_timing_for_benchmark();
    let alignment_margin_us = if matches!(benchmark_mode, BenchmarkMode::PhaseAProductionBoundary) {
        0
    } else {
        gap_us
    };
    harness.align_next_plan_to_benchmark_margin_for_test(alignment_margin_us);
    harness.configure_production_wait_policy(mode.effective_spin_threshold_us)?;
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
        record_harness_metrics(samples, &mut harness)?;
        samples
            .wall_time_us
            .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
        return Ok(());
    }
    samples.physical_dispatches += 1;
    record_wait_metrics(samples, &harness, benchmark_mode)?;
    drain_observations(&mut harness, samples);
    record_harness_metrics(samples, &mut harness)?;
    samples
        .wall_time_us
        .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
    Ok(())
}

fn run_prepared_down_iteration(
    samples: &mut Samples,
    key_count: usize,
    mode: WaitMode,
    gap_us: u64,
) -> Result<(), String> {
    let iteration_started = Instant::now();
    let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(key_count, gap_us);
    configure_focus_for_benchmark(&mut harness);
    harness.enable_dispatch_ready_timing_for_benchmark();
    harness.configure_production_wait_policy(mode.effective_spin_threshold_us)?;
    harness.prepare_prepared_stream_for_test();
    harness.reset_preparation_counts_for_test();
    harness.align_prepared_current_to_benchmark_margin_for_test(gap_us)?;
    let step = harness.wait_and_dispatch_prepared_current_for_test()?;
    samples
        .foreground_query_count
        .push(sky_dispatch_win32::focus::foreground_query_count());
    if !matches!(step, DispatchStep::Dispatched) {
        samples.record_step_failure(&step);
        samples
            .wall_time_us
            .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
        return Ok(());
    }
    samples.physical_dispatches += 1;
    if harness.last_wait_result().is_some() {
        samples.wait_count += 1;
    } else {
        samples.overdue_dispatch_count += 1;
    }
    record_wait_evidence(samples, &harness)?;
    record_prepared_benchmark_evidence(samples, &harness)?;
    record_wait_metrics(samples, &harness, BenchmarkMode::RealWait)?;
    drain_observations(&mut harness, samples);
    record_harness_metrics(samples, &mut harness)?;
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

fn run_baseline_real_wait_probe(mode: WaitMode) -> Result<Samples, String> {
    let mut samples = new_samples();
    for _ in 0..iterations() {
        run_prepared_down_iteration(&mut samples, 1, mode, due_us())?;
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
        configure_focus_for_benchmark(&mut harness);
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
        record_harness_metrics(&mut samples, &mut harness)?;
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
        configure_focus_for_benchmark(&mut harness);
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
        record_harness_metrics(&mut samples, &mut harness)?;
        samples
            .wall_time_us
            .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
    }
    Ok(samples)
}

fn run_dense_alternating(
    event_count: usize,
    mode: WaitMode,
    benchmark_mode: BenchmarkMode,
) -> Result<Samples, String> {
    let mut samples = new_samples();
    for _ in 0..iterations() {
        let iteration_started = Instant::now();
        let mut harness =
            ProductionDispatchTestHarness::try_new_dense_alternating_with_gap_for_test(
                event_count,
                due_us(),
            )?;
        configure_focus_for_benchmark(&mut harness);
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
            record_harness_metrics(&mut samples, &mut harness)?;
            samples
                .wall_time_us
                .push(u64::try_from(iteration_started.elapsed().as_micros()).unwrap_or(u64::MAX));
            continue;
        }
        samples.physical_dispatches += 1;
        record_wait_metrics(&mut samples, &harness, benchmark_mode)?;
        drain_observations(&mut harness, &mut samples);
        record_harness_metrics(&mut samples, &mut harness)?;
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
    scenarios.insert(
        "dense_alternating_14".to_string(),
        summarize(
            run_dense_alternating(14, mode, benchmark_mode)
                .unwrap_or_else(|error| panic!("{error}")),
        ),
    );
    let cpu_finished_us = sky_dispatch_win32::cpu::current_process_cpu_time_us();
    serde_json::json!({
        "scope": "Phase-A acceptance production dispatch/admission/commit path with a deterministic direct crossing and mock transport; waiter scheduling excluded; includes dense alternating Up/Down boundaries",
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

const BASELINE_COMPLETION_DELAY_US: u64 = 8;

fn baseline_report() -> serde_json::Value {
    let mut evidence = BaselineEvidence::default();

    {
        let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 10_000);
        let plan = harness.plan_current_dispatch();
        baseline_record_boundary(
            &mut evidence,
            "single_note",
            &mut harness,
            &plan,
            0,
            None,
            BASELINE_COMPLETION_DELAY_US,
            false,
            None,
        );
    }

    {
        let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(15, 10_000);
        let plan = harness.plan_current_dispatch();
        baseline_record_boundary(
            &mut evidence,
            "maximum_valid_atomic_chord",
            &mut harness,
            &plan,
            0,
            None,
            BASELINE_COMPLETION_DELAY_US,
            false,
            None,
        );
    }

    {
        let mut harness =
            ProductionDispatchTestHarness::new_dense_future_boundary_with_gap_for_test(1_000);
        let first = harness.plan_current_dispatch();
        baseline_record_boundary(
            &mut evidence,
            "dense_different_key_boundaries",
            &mut harness,
            &first,
            0,
            None,
            BASELINE_COMPLETION_DELAY_US,
            false,
            None,
        );
        let second = harness.plan_current_dispatch();
        baseline_record_boundary(
            &mut evidence,
            "dense_different_key_boundaries",
            &mut harness,
            &second,
            0,
            None,
            BASELINE_COMPLETION_DELAY_US,
            false,
            None,
        );
    }

    {
        let mut harness =
            ProductionDispatchTestHarness::new_same_key_retrigger_with_gap_for_test(20_000);
        for _ in 0..4 {
            let plan = harness.plan_current_dispatch();
            baseline_record_boundary(
                &mut evidence,
                "legal_same_key_sequence",
                &mut harness,
                &plan,
                0,
                None,
                BASELINE_COMPLETION_DELAY_US,
                false,
                None,
            );
        }
    }

    {
        let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 10_000);
        let plan = harness.plan_current_dispatch();
        baseline_record_boundary(
            &mut evidence,
            "late_wake_near_target",
            &mut harness,
            &plan,
            1_000,
            None,
            BASELINE_COMPLETION_DELAY_US,
            false,
            None,
        );
    }

    {
        let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 10_000);
        let plan = harness.plan_current_dispatch();
        baseline_record_boundary(
            &mut evidence,
            "late_wake_well_after_target",
            &mut harness,
            &plan,
            4_000,
            None,
            BASELINE_COMPLETION_DELAY_US,
            false,
            None,
        );
    }

    {
        let mut harness =
            ProductionDispatchTestHarness::new_dense_future_boundary_with_gap_for_test(1_000);
        let first = harness.plan_current_dispatch();
        let first_target = harness
            .physical_target_qpc_for_test(&first)
            .expect("baseline late-first target");
        let first_crossing = first_target
            .checked_add_duration(
                harness
                    .qpc_duration_from_us_for_test(4_000)
                    .expect("baseline late-first conversion"),
            )
            .expect("baseline late-first crossing");
        baseline_record_boundary(
            &mut evidence,
            "late_first_boundary_pressures_following_boundary",
            &mut harness,
            &first,
            4_000,
            None,
            BASELINE_COMPLETION_DELAY_US,
            false,
            None,
        );
        let second = harness.plan_current_dispatch();
        let second_target = harness
            .physical_target_qpc_for_test(&second)
            .expect("baseline pressured second target");
        let second_lateness_us = harness
            .qpc_duration_to_us_for_test(
                first_crossing
                    .checked_duration_since(second_target)
                    .expect("baseline pressured target ordering"),
            )
            .expect("baseline pressured lateness conversion");
        baseline_record_boundary(
            &mut evidence,
            "late_first_boundary_pressures_following_boundary",
            &mut harness,
            &second,
            second_lateness_us,
            Some(first_crossing),
            BASELINE_COMPLETION_DELAY_US,
            false,
            None,
        );
    }

    {
        let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 10_000);
        harness.set_final_gate_race_hook(
            |_focus_active,
             _target_hwnd,
             _target_generation,
             quit_requested,
             _skip_requested,
             _panic_requested,
             _desired_pause,
             _system_power| {
                quit_requested.store(true, Ordering::Release);
            },
        );
        let plan = harness.plan_current_dispatch();
        baseline_record_boundary(
            &mut evidence,
            "stop_interrupt_race_at_target",
            &mut harness,
            &plan,
            0,
            None,
            BASELINE_COMPLETION_DELAY_US,
            false,
            Some("control_rejected"),
        );
    }

    {
        let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 10_000);
        let plan = harness.plan_current_dispatch();
        baseline_record_boundary(
            &mut evidence,
            "intentionally_injected_zero_progress_drop",
            &mut harness,
            &plan,
            0,
            None,
            BASELINE_COMPLETION_DELAY_US,
            true,
            Some("ZeroProgress"),
        );
    }

    let completion_floor = baseline_completion_floor_report(&mut evidence);
    let prepared_stream = baseline_prepared_stream_report();

    let expected_case_names = [
        "single_note",
        "maximum_valid_atomic_chord",
        "dense_different_key_boundaries",
        "legal_same_key_sequence",
        "late_wake_near_target",
        "late_wake_well_after_target",
        "late_first_boundary_pressures_following_boundary",
        "stop_interrupt_race_at_target",
        "intentionally_injected_zero_progress_drop",
        "completion_floor_control",
        "completion_floor_delayed",
    ];
    let prepared_boundary_count = evidence.boundaries.len();
    let semantic_expectations = vec![
        baseline_semantic_expectation(
            &evidence.boundaries,
            "single_note",
            1,
            &[0],
            &[(0, 1)],
            &["successful_send"],
            &[Some("Complete")],
            &[None],
            &["Dispatched"],
        ),
        baseline_semantic_expectation(
            &evidence.boundaries,
            "maximum_valid_atomic_chord",
            1,
            &[0],
            &[(0, 0x7fff)],
            &["successful_send"],
            &[Some("Complete")],
            &[None],
            &["Dispatched"],
        ),
        baseline_semantic_expectation(
            &evidence.boundaries,
            "dense_different_key_boundaries",
            2,
            &[0, 1],
            &[(0, 1), (0, 2)],
            &["successful_send", "successful_send"],
            &[Some("Complete"), Some("Complete")],
            &[None, None],
            &["Dispatched", "Dispatched"],
        ),
        baseline_semantic_expectation(
            &evidence.boundaries,
            "legal_same_key_sequence",
            4,
            &[0, 1, 2, 3],
            &[(0, 1), (1, 0), (0, 1), (1, 0)],
            &[
                "successful_send",
                "successful_send",
                "successful_send",
                "successful_send",
            ],
            &[
                Some("Complete"),
                Some("Complete"),
                Some("Complete"),
                Some("Complete"),
            ],
            &[None, None, None, None],
            &["Dispatched", "Dispatched", "Dispatched", "Dispatched"],
        ),
        baseline_semantic_expectation(
            &evidence.boundaries,
            "late_wake_near_target",
            1,
            &[0],
            &[(0, 1)],
            &["successful_send"],
            &[Some("Complete")],
            &[None],
            &["Dispatched"],
        ),
        baseline_semantic_expectation(
            &evidence.boundaries,
            "late_wake_well_after_target",
            1,
            &[0],
            &[(0, 1)],
            &["successful_send"],
            &[Some("Complete")],
            &[None],
            &["Dispatched"],
        ),
        baseline_semantic_expectation(
            &evidence.boundaries,
            "late_first_boundary_pressures_following_boundary",
            2,
            &[0, 1],
            &[(0, 1), (0, 2)],
            &["successful_send", "successful_send"],
            &[Some("Complete"), Some("Complete")],
            &[None, None],
            &["Dispatched", "Dispatched"],
        ),
        baseline_semantic_expectation(
            &evidence.boundaries,
            "stop_interrupt_race_at_target",
            1,
            &[0],
            &[(0, 1)],
            &["non_send"],
            &[None],
            &[Some("control_rejected")],
            &["Continue"],
        ),
        baseline_semantic_expectation(
            &evidence.boundaries,
            "intentionally_injected_zero_progress_drop",
            1,
            &[0],
            &[(0, 1)],
            &["non_send"],
            &[Some("ZeroProgress")],
            &[Some("ZeroProgress")],
            &["Terminate"],
        ),
        baseline_semantic_expectation(
            &evidence.boundaries,
            "completion_floor_control",
            1,
            &[0],
            &[(0, 1)],
            &["successful_send"],
            &[Some("Complete")],
            &[None],
            &["Dispatched"],
        ),
        baseline_semantic_expectation(
            &evidence.boundaries,
            "completion_floor_delayed",
            1,
            &[0],
            &[(0, 1)],
            &["successful_send"],
            &[Some("Complete")],
            &[None],
            &["Dispatched"],
        ),
    ];
    let semantic_expectations_clean = semantic_expectations
        .iter()
        .all(|value| value["pass"].as_bool().unwrap_or(false));
    let deterministic_acceptance_clean = expected_case_names.iter().all(|name| {
        evidence
            .boundaries
            .iter()
            .any(|boundary| boundary.scenario == *name)
    }) && prepared_boundary_count
        == evidence
            .successful_sendinput_transactions
            .saturating_add(evidence.non_send_or_drop_boundaries)
        && semantic_expectations_clean
        && completion_floor["acceptance_clean"]
            .as_bool()
            .unwrap_or(false)
        && prepared_stream["acceptance_clean"]
            .as_bool()
            .unwrap_or(false);
    assert!(
        deterministic_acceptance_clean,
        "baseline deterministic evidence contract failed"
    );

    let real_wait_mode = build_wait_mode("baseline_real_wait", true, true, true);
    let real_wait = summarize(
        run_baseline_real_wait_probe(real_wait_mode)
            .unwrap_or_else(|error| panic!("baseline real-wait probe: {error}")),
    );
    let real_wait_acceptance_clean = real_wait
        .get("acceptance_clean")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    json!({
        "scope": "prepared absolute-frame dispatch qualification for issue 300; normal physical timing is authored-only and SendInput completion is telemetry-only",
        "acceptance_clean": deterministic_acceptance_clean && real_wait_acceptance_clean,
        "deterministic_acceptance_clean": deterministic_acceptance_clean,
        "expected_cases": expected_case_names,
        "semantic_expectations": semantic_expectations,
        "prepared_boundary_count": prepared_boundary_count,
        "successful_sendinput_transaction_count": evidence.successful_sendinput_transactions,
        "non_send_or_drop_boundary_count": evidence.non_send_or_drop_boundaries,
        "timeline_rebase_count": evidence.timeline_rebase_count,
        "transport_anomaly_count": evidence.transport_anomaly_count,
        "completion_floor": completion_floor,
        "prepared_stream": prepared_stream,
        "boundary_evidence": evidence.boundaries,
        "real_wait_probe": {
            "acceptance_clean": real_wait_acceptance_clean,
            "pre_call_minus_target_us": real_wait["dispatch_start_error_us"],
            "completion_minus_pre_call_us": real_wait["pre_call_to_completion_us"],
            "observation_count": real_wait["observation_count"],
            "non_dispatches": real_wait["non_dispatches"],
            "overdue_dispatch_count": real_wait["overdue_dispatch_count"],
            "transport_anomaly_count": real_wait["transport_anomaly_count"],
            "host_waiter": "HybridWaiter::production",
            "target_minus_wait_entry_us": real_wait["prepared_benchmark"]["target_minus_wait_entry_us"],
            "alignment_to_wait_entry_us": real_wait["prepared_benchmark"]["alignment_to_wait_entry_us"],
            "prepared_stream_build_us_startup_only": real_wait["prepared_benchmark"]["prepared_stream_build_us_startup_only"],
        },
        "healthy_precision_path": {
            "production_scheduling_semantics_changed": false,
            "normal_completion_feedback_removed": true,
            "allocations_locks_formatting_blocking_communication_added": false,
            "no_allocation_gate": "rt_dispatch_no_alloc",
        },
        "raw_real_wait_report": real_wait,
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
    let target_minus_wait_entry = signed_summary(samples.target_minus_wait_entry_us);
    let alignment_to_wait_entry = signed_summary(samples.alignment_to_wait_entry_us);
    let prepared_stream_build = unsigned_summary(samples.prepared_stream_build_us);
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
            "target_minus_wait_entry_us": target_minus_wait_entry.clone(),
            "alignment_to_wait_entry_us": alignment_to_wait_entry.clone(),
            "hot_count": samples.hot_wait_count,
            "cold_count": samples.cold_wait_count,
            "cold_threshold_us": SEND_COLD_THRESHOLD_US,
        },
        "prepared_benchmark": {
            "target_minus_wait_entry_us": target_minus_wait_entry,
            "alignment_to_wait_entry_us": alignment_to_wait_entry,
            "prepared_stream_build_us_startup_only": prepared_stream_build,
            "note": "These fields are test-support benchmark setup evidence; stream construction is not part of the steady-state musical lateness interval.",
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
        "transport_anomaly_count": samples.transport_anomaly_count,
        "foreground_query_count": unsigned_summary(samples.foreground_query_count),
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
    if matches!(benchmark_scope, BenchmarkScope::Baseline)
        && !matches!(benchmark_mode, BenchmarkMode::RealWait)
    {
        panic!("baseline requires real_wait benchmark mode");
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
    let require_focus = require_focus_for_benchmark();
    if require_focus {
        sky_dispatch_win32::focus::set_foreground_window_for_test(Some(1));
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
    } else if matches!(benchmark_scope, BenchmarkScope::Baseline) {
        mode_reports.insert("baseline".to_string(), baseline_report());
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
            (BenchmarkScope::Baseline, BenchmarkMode::RealWait) => "prepared absolute-frame normal dispatch through the production test-support path, deterministic semantic non-send/drop classification, completion no-feedback evidence, and a host real HybridWaiter probe; physical timing floors and wait policy are unchanged; not Raw Input or game-observed latency",
            (BenchmarkScope::Baseline, _) => "invalid benchmark scope/mode combination",
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
        },
        "rust_version": rust_version(),
        "qpc_frequency": qpc_frequency,
        "iterations": iterations(),
        "deadline_us": due_us(),
        "transport": "deterministic_mock",
        "require_focus": require_focus,
        "foreground_query_scope": "one fresh final query for eligible require-focus Down traffic; zero for UpOnly or require-focus=false traffic",
        "observation_enqueue_ab": observation_enqueue_ab,
        "modes": mode_reports,
        "elapsed_ms": started.elapsed().as_millis(),
    }))
    .expect("serialize benchmark output");
    if require_focus {
        sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    }
    if let Some(path) = std::env::args_os().nth(1) {
        std::fs::write(path, &output).expect("write benchmark output");
    }
    println!("{output}");
}
