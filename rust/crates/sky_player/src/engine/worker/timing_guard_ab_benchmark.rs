use super::{super::config, select_spin_threshold_us};
use serde_json::{Value, json};
use sky_dispatch_win32::{
    clock::{QpcClock, QpcTicks},
    cpu::{current_process_cpu_time_us, current_thread_cpu_time_us},
    event::OwnedEvent,
    wait::{HybridWaiter, WaitOutcome, WakeErrorStats},
};
use std::{
    fs::File,
    io::{BufWriter, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const EXPERIMENTAL_FIXED_GUARD_US: u64 = 1_000;
const FRAME_US: u64 = 16_667;
const SAMPLES_PER_RUN: usize = 900;
const MEASURED_PAIRS: usize = 6;
const WARMUP_RUNS_PER_ARM_PER_CONDITION: usize = 1;
const CALIBRATION_TIMEOUT_US: u64 = 20_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GuardArm {
    Fixed1Ms,
    Calibrated,
}

impl GuardArm {
    const fn label(self) -> &'static str {
        match self {
            Self::Fixed1Ms => "experimental_fixed_1000us",
            Self::Calibrated => "current_startup_calibration",
        }
    }
}

struct RunResult {
    condition: &'static str,
    arm: GuardArm,
    pair: Option<usize>,
    pair_order: &'static str,
    warmup: bool,
    selected_guard_us: u64,
    calibration: Option<WakeErrorStats>,
    calibration_duration_us: u64,
    expected_samples: usize,
    missing_qpc_samples: usize,
    failed_waits: usize,
    interrupted_waits: usize,
    early_wakes: usize,
    wake_lateness_us: Vec<i64>,
    wait_return_observation_gap_us: Vec<i64>,
    spin_time_us: u64,
    elapsed_us: u64,
    worker_thread_cpu_us: u64,
    process_cpu_us: u64,
}

#[derive(Clone, Copy)]
struct RunOrder {
    pair: Option<usize>,
    pair_order: &'static str,
    warmup: bool,
}

struct ContentionLoad {
    stop: Arc<AtomicBool>,
    worker_cpu_us: Arc<AtomicU64>,
    iterations: Arc<AtomicU64>,
    workers: Vec<JoinHandle<()>>,
}

impl ContentionLoad {
    fn start(worker_count: usize) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_cpu_us = Arc::new(AtomicU64::new(0));
        let iterations = Arc::new(AtomicU64::new(0));
        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let thread_stop = Arc::clone(&stop);
            let thread_cpu = Arc::clone(&worker_cpu_us);
            let thread_iterations = Arc::clone(&iterations);
            workers.push(std::thread::spawn(move || {
                const ACTIVE_SLICE: Duration = Duration::from_millis(5);
                const IDLE_SLICE: Duration = Duration::from_millis(5);
                let cpu_start = current_thread_cpu_time_us();
                let mut last_cpu_report = cpu_start;
                let mut local_iterations = 0_u64;
                let mut last_iteration_report = 0_u64;
                let mut next_report = Instant::now() + Duration::from_millis(100);
                while !thread_stop.load(Ordering::Relaxed) {
                    let active_until = Instant::now() + ACTIVE_SLICE;
                    while Instant::now() < active_until && !thread_stop.load(Ordering::Relaxed) {
                        local_iterations = local_iterations.wrapping_add(1);
                        std::hint::black_box(local_iterations);
                    }
                    std::thread::sleep(IDLE_SLICE);
                    if Instant::now() >= next_report {
                        let cpu_now = current_thread_cpu_time_us();
                        thread_cpu
                            .fetch_add(cpu_now.saturating_sub(last_cpu_report), Ordering::Relaxed);
                        last_cpu_report = cpu_now;
                        thread_iterations.fetch_add(
                            local_iterations.wrapping_sub(last_iteration_report),
                            Ordering::Relaxed,
                        );
                        last_iteration_report = local_iterations;
                        next_report += Duration::from_millis(100);
                    }
                }
                let cpu_end = current_thread_cpu_time_us();
                thread_cpu.fetch_add(cpu_end.saturating_sub(last_cpu_report), Ordering::Relaxed);
                thread_iterations.fetch_add(
                    local_iterations.wrapping_sub(last_iteration_report),
                    Ordering::Relaxed,
                );
            }));
        }
        Self {
            stop,
            worker_cpu_us,
            iterations,
            workers,
        }
    }

    fn finish(self) -> (u64, u64) {
        self.stop.store(true, Ordering::Release);
        for worker in self.workers {
            worker.join().expect("contention worker joined");
        }
        (
            self.worker_cpu_us.load(Ordering::Acquire),
            self.iterations.load(Ordering::Acquire),
        )
    }
}

fn signed_duration_us(clock: QpcClock, value: QpcTicks, baseline: QpcTicks) -> Option<i64> {
    let (negative, duration) = if value >= baseline {
        (false, value.checked_duration_since(baseline).ok()?)
    } else {
        (true, baseline.checked_duration_since(value).ok()?)
    };
    let magnitude = i64::try_from(clock.duration_to_us(duration).ok()?).ok()?;
    Some(if negative { -magnitude } else { magnitude })
}

fn nearest_rank(values: &[i64], permille: usize) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = sorted.len().saturating_mul(permille).div_ceil(1_000).max(1);
    sorted.get(rank - 1).copied()
}

fn percentile_set(values: &[i64]) -> Value {
    json!({
        "sample_count": values.len(),
        "p50_us": nearest_rank(values, 500),
        "p95_us": nearest_rank(values, 950),
        "p99_us": nearest_rank(values, 990),
        "p99_9_us": nearest_rank(values, 999),
        "max_us": values.iter().copied().max(),
    })
}

#[test]
fn nearest_rank_percentiles_use_ceiling_rank_and_preserve_empty_as_null() {
    assert_eq!(nearest_rank(&[4, 1, 3, 2], 500), Some(2));
    assert_eq!(nearest_rank(&[4, 1, 3, 2], 950), Some(4));
    assert_eq!(nearest_rank(&[4, 1, 3, 2], 999), Some(4));
    assert_eq!(percentile_set(&[])["p99_9_us"], Value::Null);
}

fn calibration_json(stats: Option<WakeErrorStats>) -> Value {
    match stats {
        Some(stats) => json!({
            "available": true,
            "sample_count": config::CALIBRATION_SAMPLES,
            "p50_us": stats.p50_us,
            "p95_us": stats.p95_us,
            "p99_us": stats.p99_us,
            "max_us": stats.max_us,
            "robust_median_plus_6_mad_us": stats.robust_us,
        }),
        None => json!({
            "available": false,
            "sample_count": 0,
            "p50_us": null,
            "p95_us": null,
            "p99_us": null,
            "max_us": null,
            "robust_median_plus_6_mad_us": null,
        }),
    }
}

fn run_once(
    condition: &'static str,
    arm: GuardArm,
    order: RunOrder,
    clock: QpcClock,
    contention: Option<&ContentionLoad>,
    sample_writer: &mut BufWriter<File>,
    run_writer: &mut BufWriter<File>,
) -> RunResult {
    let RunOrder {
        pair,
        pair_order,
        warmup,
    } = order;
    let waiter = HybridWaiter::production();
    assert!(
        waiter.initial_failure().is_none(),
        "production waiter initialization"
    );
    let interrupt = OwnedEvent::new_auto_reset().expect("interrupt event");

    // Both arms pay and record the same six-sample startup probe. The fixed
    // arm discards the probe result; calibrated uses the shipping selector.
    let calibration_start = clock.now().expect("calibration start QPC");
    let calibration_deadline = calibration_start
        .checked_add_duration(
            clock
                .duration_from_us(CALIBRATION_TIMEOUT_US)
                .expect("calibration timeout conversion"),
        )
        .expect("calibration deadline");
    let calibration = waiter.probe_wake_error_stats_until(
        clock,
        &interrupt,
        config::CALIBRATION_SAMPLES,
        Some(calibration_deadline),
    );
    let calibration_end = clock.now().expect("calibration end QPC");
    let calibration_duration_us = clock
        .duration_to_us(
            calibration_end
                .checked_duration_since(calibration_start)
                .expect("ordered calibration QPC"),
        )
        .expect("calibration duration conversion");
    let selected_guard_us = match arm {
        GuardArm::Fixed1Ms => EXPERIMENTAL_FIXED_GUARD_US,
        GuardArm::Calibrated => select_spin_threshold_us(calibration),
    };
    let spin_threshold = clock
        .duration_from_us(selected_guard_us)
        .expect("spin guard conversion");
    let interrupt = OwnedEvent::new_auto_reset().expect("measurement interrupt event");

    let measurement_start = clock.now().expect("measurement start QPC");
    let worker_cpu_start = current_thread_cpu_time_us();
    let process_cpu_start = current_process_cpu_time_us();
    let contender_cpu_start = contention
        .map(|load| load.worker_cpu_us.load(Ordering::Acquire))
        .unwrap_or_default();

    let mut result = RunResult {
        condition,
        arm,
        pair,
        pair_order,
        warmup,
        selected_guard_us,
        calibration,
        calibration_duration_us,
        expected_samples: SAMPLES_PER_RUN,
        missing_qpc_samples: 0,
        failed_waits: 0,
        interrupted_waits: 0,
        early_wakes: 0,
        wake_lateness_us: Vec::with_capacity(SAMPLES_PER_RUN),
        wait_return_observation_gap_us: Vec::with_capacity(SAMPLES_PER_RUN),
        spin_time_us: 0,
        elapsed_us: 0,
        worker_thread_cpu_us: 0,
        process_cpu_us: 0,
    };

    for sample_index in 0..SAMPLES_PER_RUN {
        let frame_index = u64::try_from(sample_index + 1).expect("frame index");
        let offset_us = FRAME_US
            .checked_mul(frame_index)
            .expect("authored wait offset");
        let target = measurement_start
            .checked_add_duration(clock.duration_from_us(offset_us).expect("frame conversion"))
            .expect("wait target");
        let waited =
            waiter.wait_until_ticks_with_metrics_typed(clock, target, spin_threshold, &interrupt);
        result.spin_time_us = result.spin_time_us.saturating_add(
            clock
                .duration_to_us(waited.spin_ticks)
                .expect("spin duration conversion"),
        );
        let observed_after_return = clock.now().ok();
        let wake = waited.wake_qpc;
        let lateness = wake.and_then(|wake| signed_duration_us(clock, wake, target));
        let observation_gap = wake
            .zip(observed_after_return)
            .and_then(|(wake, observed)| signed_duration_us(clock, observed, wake));
        match waited.outcome {
            WaitOutcome::Deadline => {}
            WaitOutcome::Interrupted => result.interrupted_waits += 1,
            WaitOutcome::Failed(_) => result.failed_waits += 1,
        }
        if wake.is_none() || lateness.is_none() {
            result.missing_qpc_samples += 1;
        } else if lateness.is_some_and(|value| value < 0) {
            result.early_wakes += 1;
        }
        if let Some(lateness) = lateness {
            result.wake_lateness_us.push(lateness);
        }
        if let Some(gap) = observation_gap {
            result.wait_return_observation_gap_us.push(gap);
        }

        let outcome = match waited.outcome {
            WaitOutcome::Deadline => "deadline",
            WaitOutcome::Interrupted => "interrupted",
            WaitOutcome::Failed(_) => "failed",
        };
        let arm_label = arm.label();
        let pair_label = pair.map_or_else(|| "warmup".to_string(), |value| value.to_string());
        let warmup_label = u8::from(warmup);
        let target_qpc = target.as_u64();
        let wake_qpc = wake.map_or_else(String::new, |value| value.as_u64().to_string());
        let lateness = lateness.map_or_else(String::new, |value| value.to_string());
        let observation_gap = observation_gap.map_or_else(String::new, |value| value.to_string());
        let spin_us = clock.duration_to_us(waited.spin_ticks).unwrap_or_default();
        writeln!(
            sample_writer,
            "{condition},{arm_label},{pair_label},{pair_order},{warmup_label},{sample_index},{target_qpc},{wake_qpc},{outcome},{lateness},{observation_gap},{spin_us}"
        )
        .expect("write raw sample row");
    }

    let measurement_end = clock.now().expect("measurement end QPC");
    result.elapsed_us = clock
        .duration_to_us(
            measurement_end
                .checked_duration_since(measurement_start)
                .expect("ordered measurement QPC"),
        )
        .expect("measurement duration conversion");
    result.worker_thread_cpu_us = current_thread_cpu_time_us().saturating_sub(worker_cpu_start);
    result.process_cpu_us = current_process_cpu_time_us().saturating_sub(process_cpu_start);
    let contender_cpu_end = contention
        .map(|load| load.worker_cpu_us.load(Ordering::Acquire))
        .unwrap_or_default();

    let calibration_json = calibration_json(result.calibration);
    let arm_label = arm.label();
    let pair_label = pair.map_or_else(|| "warmup".to_string(), |value| value.to_string());
    let warmup_label = u8::from(warmup);
    let selected_guard_us = result.selected_guard_us;
    let calibration_p50_us = calibration_json["p50_us"]
        .as_u64()
        .map_or_else(String::new, |value| value.to_string());
    let calibration_p95_us = calibration_json["p95_us"]
        .as_u64()
        .map_or_else(String::new, |value| value.to_string());
    let calibration_p99_us = calibration_json["p99_us"]
        .as_u64()
        .map_or_else(String::new, |value| value.to_string());
    let calibration_max_us = calibration_json["max_us"]
        .as_u64()
        .map_or_else(String::new, |value| value.to_string());
    let calibration_robust_us = calibration_json["robust_median_plus_6_mad_us"]
        .as_u64()
        .map_or_else(String::new, |value| value.to_string());
    let calibration_duration_us = result.calibration_duration_us;
    let elapsed_us = result.elapsed_us;
    let worker_thread_cpu_us = result.worker_thread_cpu_us;
    let process_cpu_us = result.process_cpu_us;
    let contender_cpu_us = contender_cpu_end.saturating_sub(contender_cpu_start);
    let wait_failures = result.failed_waits + result.interrupted_waits;
    writeln!(
        run_writer,
        "{condition},{arm_label},{pair_label},{pair_order},{warmup_label},{selected_guard_us},{calibration_p50_us},{calibration_p95_us},{calibration_p99_us},{calibration_max_us},{calibration_robust_us},{calibration_duration_us},{elapsed_us},{worker_thread_cpu_us},{process_cpu_us},{contender_cpu_us},{wait_failures}"
    )
    .expect("write run summary row");

    result
}

fn aggregate(condition: &'static str, arm: GuardArm, runs: &[RunResult]) -> Value {
    let measured = runs
        .iter()
        .filter(|run| run.condition == condition && run.arm == arm && !run.warmup)
        .collect::<Vec<_>>();
    let expected_samples = measured
        .iter()
        .map(|run| run.expected_samples)
        .sum::<usize>();
    let mut wake_lateness = measured
        .iter()
        .flat_map(|run| run.wake_lateness_us.iter().copied())
        .collect::<Vec<_>>();
    let mut return_gap = measured
        .iter()
        .flat_map(|run| run.wait_return_observation_gap_us.iter().copied())
        .collect::<Vec<_>>();
    wake_lateness.sort_unstable();
    return_gap.sort_unstable();
    let spin_us = measured.iter().map(|run| run.spin_time_us).sum::<u64>();
    let wall_us = measured.iter().map(|run| run.elapsed_us).sum::<u64>();
    let calibration_runs = measured
        .iter()
        .filter_map(|run| run.calibration)
        .collect::<Vec<_>>();
    let calibration_p99 = calibration_runs
        .iter()
        .map(|stats| i64::try_from(stats.p99_us).unwrap_or(i64::MAX))
        .collect::<Vec<_>>();
    let calibration_durations = measured
        .iter()
        .map(|run| i64::try_from(run.calibration_duration_us).unwrap_or(i64::MAX))
        .collect::<Vec<_>>();
    let ab_order_runs = measured.iter().filter(|run| run.pair_order == "AB").count();
    let ba_order_runs = measured.iter().filter(|run| run.pair_order == "BA").count();
    let pair_indices = measured
        .iter()
        .filter_map(|run| run.pair)
        .collect::<std::collections::BTreeSet<_>>();
    let selected_guards = measured
        .iter()
        .map(|run| run.selected_guard_us)
        .collect::<Vec<_>>();
    let total_misses = wake_lateness.iter().filter(|value| **value > 0).count();
    let late_250 = wake_lateness.iter().filter(|value| **value >= 250).count();
    let late_1ms = wake_lateness
        .iter()
        .filter(|value| **value >= 1_000)
        .count();
    let late_2ms = wake_lateness
        .iter()
        .filter(|value| **value >= 2_000)
        .count();
    let failed = measured.iter().map(|run| run.failed_waits).sum::<usize>();
    let interrupted = measured
        .iter()
        .map(|run| run.interrupted_waits)
        .sum::<usize>();
    let missing = measured
        .iter()
        .map(|run| run.missing_qpc_samples)
        .sum::<usize>();
    let early = measured.iter().map(|run| run.early_wakes).sum::<usize>();
    let worker_cpu_us = measured
        .iter()
        .map(|run| run.worker_thread_cpu_us)
        .sum::<u64>();
    let process_cpu_us = measured.iter().map(|run| run.process_cpu_us).sum::<u64>();
    let ratio_ppm = if wall_us == 0 {
        None
    } else {
        Some((u128::from(spin_us) * 1_000_000 / u128::from(wall_us)) as u64)
    };
    json!({
        "condition": condition,
        "arm": arm.label(),
        "measured_runs": measured.len(),
        "ab_order_runs": ab_order_runs,
        "ba_order_runs": ba_order_runs,
        "unique_pair_count": pair_indices.len(),
        "samples_expected": expected_samples,
        "samples_valid": wake_lateness.len(),
        "missing_or_invalid_qpc_samples": missing,
        "failed_waits": failed,
        "interrupted_waits": interrupted,
        "early_wakes": early,
        "wake_deadline_lateness_us": percentile_set(&wake_lateness),
        "wait_return_to_observation_qpc_gap_us_not_sender_pre_call": percentile_set(&return_gap),
        "deadline_misses_after_waiter_return": total_misses,
        "wake_lateness_ge_250us": late_250,
        "wake_lateness_ge_1000us": late_1ms,
        "wake_lateness_ge_2000us": late_2ms,
        "startup_calibration_runs_with_stats": calibration_runs.len(),
        "startup_calibration_runs_unavailable": measured.len().saturating_sub(calibration_runs.len()),
        "startup_calibration_p99_us": percentile_set(&calibration_p99),
        "startup_calibration_duration_us": percentile_set(&calibration_durations),
        "selected_guard_us_min": selected_guards.iter().copied().min(),
        "selected_guard_us_max": selected_guards.iter().copied().max(),
        "selected_guard_us_by_run": selected_guards,
        "worker_thread_cpu_time_us": worker_cpu_us,
        "process_cpu_time_us": process_cpu_us,
        "wall_time_us": wall_us,
        "spin_time_us": spin_us,
        "spin_to_wall_ratio_ppm": ratio_ppm,
        "sendinput_pre_call_qpc": null,
        "sendinput_completion_qpc": null,
        "sendinput_envelope_percentiles": null,
        "deadline_block_replan_transport_anomaly_cleanup": null,
        "not_measured_reason": "waiter-only harness: no DispatchExecutor, owner/focus gate, sender, or cleanup path"
    })
}

#[test]
#[ignore = "explicit M4 Windows waiter-only A/B experiment; does not call SendInput"]
fn offline_waiter_guard_ab_benchmark() {
    assert_eq!(config::CALIBRATION_SAMPLES, 6);
    assert_eq!(
        config::DEFAULT_SPIN_THRESHOLD_US,
        EXPERIMENTAL_FIXED_GUARD_US
    );
    let output_dir = std::env::var_os("ISSUE_666_M4_OUTPUT_DIR")
        .expect("run via scripts/run_issue666_m4_guard_ab_offline.ps1");
    let output_dir = std::path::PathBuf::from(output_dir);
    let mut sample_writer = BufWriter::new(
        File::create(output_dir.join("wait-samples.csv")).expect("create raw sample artifact"),
    );
    let mut run_writer = BufWriter::new(
        File::create(output_dir.join("wait-runs.csv")).expect("create run summary artifact"),
    );
    writeln!(
        sample_writer,
        "condition,arm,pair,pair_order,warmup,sample_index,target_qpc,wake_qpc,outcome,wake_lateness_us,wait_return_observation_qpc_gap_us,spin_us"
    )
    .expect("write sample header");
    writeln!(
        run_writer,
        "condition,arm,pair,pair_order,warmup,selected_guard_us,calibration_p50_us,calibration_p95_us,calibration_p99_us,calibration_max_us,calibration_robust_us,calibration_duration_us,measurement_elapsed_us,worker_thread_cpu_us,process_cpu_us,contention_worker_cpu_us,wait_failures"
    )
    .expect("write run header");

    let clock = QpcClock::initialize().expect("QPC clock");
    let mut runs = Vec::with_capacity(28);
    for condition in ["quiet", "contention"] {
        let contention = (condition == "contention").then(|| ContentionLoad::start(2));
        if contention.is_some() {
            std::thread::sleep(Duration::from_millis(250));
        }

        for arm in [GuardArm::Fixed1Ms, GuardArm::Calibrated] {
            runs.push(run_once(
                condition,
                arm,
                RunOrder {
                    pair: None,
                    pair_order: "warmup",
                    warmup: true,
                },
                clock,
                contention.as_ref(),
                &mut sample_writer,
                &mut run_writer,
            ));
        }

        for pair in 0..MEASURED_PAIRS {
            let order = if pair % 2 == 0 {
                [GuardArm::Fixed1Ms, GuardArm::Calibrated]
            } else {
                [GuardArm::Calibrated, GuardArm::Fixed1Ms]
            };
            let order_label = if pair % 2 == 0 { "AB" } else { "BA" };
            for arm in order {
                runs.push(run_once(
                    condition,
                    arm,
                    RunOrder {
                        pair: Some(pair + 1),
                        pair_order: order_label,
                        warmup: false,
                    },
                    clock,
                    contention.as_ref(),
                    &mut sample_writer,
                    &mut run_writer,
                ));
            }
        }

        let (contention_cpu_us, contention_iterations) =
            contention.map_or((0, 0), ContentionLoad::finish);
        for arm in [GuardArm::Fixed1Ms, GuardArm::Calibrated] {
            let summary = aggregate(condition, arm, &runs);
            let expected_per_arm = MEASURED_PAIRS * SAMPLES_PER_RUN;
            assert_eq!(
                summary["samples_expected"].as_u64(),
                Some(expected_per_arm as u64),
                "six paired runs per arm for {condition}"
            );
            assert_eq!(
                summary["samples_valid"].as_u64(),
                Some(expected_per_arm as u64),
                "all waiter deadlines need a measured QPC for {condition}"
            );
            assert_eq!(summary["ab_order_runs"].as_u64(), Some(3));
            assert_eq!(summary["ba_order_runs"].as_u64(), Some(3));
            assert_eq!(summary["unique_pair_count"].as_u64(), Some(6));
            assert_eq!(summary["failed_waits"].as_u64(), Some(0));
            assert_eq!(summary["interrupted_waits"].as_u64(), Some(0));
            println!("M4_GUARD_AB_CELL {}", summary);
        }
        if condition == "contention" {
            println!(
                "M4_GUARD_AB_CONTENTION_LOAD {}",
                json!({
                    "worker_threads": 2,
                    "active_slice_ms": 5,
                    "idle_slice_ms": 5,
                    "priority": "normal/inherited",
                    "affinity": "unmodified",
                    "worker_cpu_time_us": contention_cpu_us,
                    "loop_iterations": contention_iterations,
                })
            );
        }
    }
    sample_writer.flush().expect("flush sample artifact");
    run_writer.flush().expect("flush run artifact");

    let qpc_frequency_hz = clock.frequency_hz().get();
    let summary = json!({
        "schema_version": 1,
        "evidence_kind": "offline_windows_waiter_ab_not_playback_qualification",
        "sendinput_called": false,
        "source": "same test-support binary and candidate checkout for both arms",
        "qpc_frequency_hz": qpc_frequency_hz,
        "percentile_method": "nearest rank: sorted[ceil(p*n)-1]",
        "warmups_per_arm_per_condition": WARMUP_RUNS_PER_ARM_PER_CONDITION,
        "measured_ab_ba_pairs_per_condition": MEASURED_PAIRS,
        "measured_runs_per_arm_per_condition": MEASURED_PAIRS,
        "samples_per_run": SAMPLES_PER_RUN,
        "samples_per_arm_per_condition": MEASURED_PAIRS * SAMPLES_PER_RUN,
        "schedule": {
            "workload_version": "m4-wait-cadence-60fps-v1",
            "deadline_count_per_run": SAMPLES_PER_RUN,
            "frame_us": FRAME_US,
            "first_deadline_offset_us": FRAME_US,
            "last_deadline_offset_us": FRAME_US * SAMPLES_PER_RUN as u64,
            "fps": 60,
            "h_g_margin_us": null,
            "note_and_packet_delivery": "not present in waiter-only workload",
        },
        "arms": [
            {"label": GuardArm::Fixed1Ms.label(), "threshold_us": EXPERIMENTAL_FIXED_GUARD_US,
             "production_default_changed": false},
            {"label": GuardArm::Calibrated.label(), "selection": "sky_player::engine::worker::timing::select_spin_threshold_us",
             "fallback_us": select_spin_threshold_us(None)},
        ],
        "cells": [
            aggregate("quiet", GuardArm::Fixed1Ms, &runs),
            aggregate("quiet", GuardArm::Calibrated, &runs),
            aggregate("contention", GuardArm::Fixed1Ms, &runs),
            aggregate("contention", GuardArm::Calibrated, &runs),
        ],
        "raw_artifacts": ["wait-samples.csv", "wait-runs.csv"],
        "outcome_counts_include_only_waiter_deadline_behavior": true,
    });
    println!("M4_GUARD_AB_RESULT_JSON={summary}");
}
