//! Executable P0 timing characterizations for Coordinator R3.
//!
//! This example is compiled only with `test-support`. It calls the existing
//! prepared worker dispatch path and labels deterministic transport evidence
//! as mock data. It never sends real keyboard input.

use serde_json::{Value, json};
use sky_dispatch_core::compile::compile_runtime_intents;
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::{QpcClock, QpcTicks, qpc_frequency_checked};
use sky_dispatch_win32::input::{PhysicalPacket, SendEvidence};
use sky_player::engine::dispatch_primitives::{
    DispatchStep, ProductionDispatchTestHarness, RtR3CleanupCapture,
};
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const HOLD_US: u64 = 17_167;
const FRAME_US: u64 = 16_667;
const MARGIN_US: u64 = 500;
const WARMUP_DEFAULT: usize = 1_000;
const MEASURED_DEFAULT: usize = 10_000;

#[derive(Clone, Copy)]
struct Workload {
    id: &'static str,
    kind: WorkloadKind,
}

#[derive(Clone, Copy)]
enum WorkloadKind {
    Down(usize),
    Mixed(usize, usize),
    UpOnly(usize),
}

const WORKLOADS: [Workload; 9] = [
    Workload {
        id: "down-1",
        kind: WorkloadKind::Down(1),
    },
    Workload {
        id: "down-5",
        kind: WorkloadKind::Down(5),
    },
    Workload {
        id: "down-15",
        kind: WorkloadKind::Down(15),
    },
    Workload {
        id: "mixed-1x1",
        kind: WorkloadKind::Mixed(1, 1),
    },
    Workload {
        id: "mixed-2x3",
        kind: WorkloadKind::Mixed(2, 3),
    },
    Workload {
        id: "mixed-7x8",
        kind: WorkloadKind::Mixed(7, 8),
    },
    Workload {
        id: "up-only-1",
        kind: WorkloadKind::UpOnly(1),
    },
    Workload {
        id: "up-only-5",
        kind: WorkloadKind::UpOnly(5),
    },
    Workload {
        id: "up-only-15",
        kind: WorkloadKind::UpOnly(15),
    },
];

struct Arguments {
    mode: String,
    load_mode: String,
    output: PathBuf,
    workload: String,
    warmup: usize,
    measured: usize,
    seed: u64,
    run_index: usize,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("rt_r3_probe: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let arguments = parse_args()?;
    let started_at_ms = unix_ms()?;
    let source_revision = git_revision()?;
    let runtime_revision =
        std::env::var("SKY_RT_R3_RUNTIME_REVISION").unwrap_or_else(|_| source_revision.clone());
    let executable_sha256 = executable_sha256()?;
    let (target, toolchain, os, cpu) = host_metadata();
    let frequency_hz =
        qpc_frequency_checked().map_err(|error| format!("QPC frequency: {error:?}"))?;
    let run_id = std::env::var("SKY_RT_R3_RUN_ID").unwrap_or_else(|_| generated_run_id());
    let contention_workers = std::env::var("SKY_RT_R3_CONTENTION_WORKERS")
        .ok()
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| "SKY_RT_R3_CONTENTION_WORKERS must be an integer".to_string())
        })
        .transpose()?
        .unwrap_or(0);
    if arguments.load_mode == "cpu_contention" && contention_workers != 2 {
        return Err(
            "cpu_contention measurements require exactly two runner-managed workers".into(),
        );
    }
    if arguments.load_mode == "quiet" && contention_workers != 0 {
        return Err("quiet measurements cannot declare contention workers".into());
    }

    let (workload_name, warmup_count, measured_count, vectors, distributions, counters, eligible) =
        match arguments.mode.as_str() {
            "contracts" => {
                let contract_report = contracts_report()?;
                (
                    "contracts".to_string(),
                    0,
                    0,
                    contract_report,
                    json!({}),
                    json!({}),
                    false,
                )
            }
            "precision" => {
                let selected = select_workloads(&arguments.workload)?;
                let mut reports = Vec::with_capacity(selected.len());
                let mut attempts = 0u64;
                let mut completed_samples = 0u64;
                let mut all_eligible =
                    arguments.measured == MEASURED_DEFAULT && arguments.warmup == WARMUP_DEFAULT;
                for workload in selected {
                    let result = precision_report(
                        workload,
                        arguments.warmup,
                        arguments.measured,
                        arguments.seed,
                        arguments.run_index,
                    )?;
                    attempts = attempts.saturating_add(
                        result["counters"]["measured_sender_attempts"]
                            .as_u64()
                            .unwrap_or(0),
                    );
                    completed_samples = completed_samples
                        .saturating_add(result["sample_count"].as_u64().unwrap_or(0));
                    all_eligible &= result["statistics_eligible"].as_bool() == Some(true);
                    reports.push(result);
                }
                (
                    arguments.workload.clone(),
                    arguments.warmup,
                    arguments.measured,
                    json!({"precision_runs": reports}),
                    json!({"scope": "per-workload", "sample_count": completed_samples}),
                    json!({
                        "measured_sender_attempts": attempts,
                        "expected_sender_attempts": (arguments.measured * select_workloads(&arguments.workload)?.len()) as u64,
                    }),
                    all_eligible,
                )
            }
            other => {
                return Err(format!(
                    "unsupported mode {other:?}; use contracts or precision"
                ));
            }
        };
    let finished_at_ms = unix_ms()?;
    let qpc_source = if arguments.mode == "precision" {
        "host QueryPerformanceCounter"
    } else {
        "synthetic QPC ticks per vector frequency; top-level host frequency is metadata only"
    };
    let report = json!({
        "schema_version": 1,
        "source_revision": source_revision,
        "runtime_revision": runtime_revision,
        "executable_sha256": executable_sha256,
        "toolchain": toolchain,
        "target": target,
        "os": os,
        "cpu": cpu,
        "power_state": std::env::var("SKY_RT_R3_POWER_STATE").unwrap_or_else(|_| "not-provided".to_string()),
        "focus_setup": "test-support focus_active=true with deterministic SessionTarget; no game process or foreground-window activation",
        "qpc_frequency_hz": frequency_hz,
        "profile": "release",
        "run_id": run_id,
        "mode": arguments.mode,
        "load_mode": arguments.load_mode,
        "contention_workers": contention_workers,
        "transport_kind": "deterministic-mock",
        "workload": workload_name,
        "seed": arguments.seed,
        "run_index": arguments.run_index,
        "warmup_count": warmup_count,
        "measured_count": measured_count,
        "timestamps": {"started_unix_ms": started_at_ms, "finished_unix_ms": finished_at_ms},
        "test_vectors": vectors,
        "hold_gap_accounting_obligation_counters": counters,
        "distributions": distributions,
        "statistics_eligible": eligible,
        "qpc_source": qpc_source,
        "measurement_boundary": "QPC sampled before prepared dispatch wrapper through the sender's authoritative pre-call QPC; sender completion recorded separately",
        "receipt_boundary": "deterministic mock transport; no Raw Input or game/audio receipt claim"
    });
    let encoded =
        serde_json::to_vec_pretty(&report).map_err(|error| format!("serialize JSON: {error}"))?;
    if let Some(parent) = arguments.output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create output folder: {error}"))?;
    }
    std::fs::write(&arguments.output, encoded)
        .map_err(|error| format!("write {}: {error}", arguments.output.display()))?;
    println!(
        "wrote schema=1 mode={} workload={} output={}",
        arguments.mode,
        arguments.workload,
        arguments.output.display()
    );
    Ok(())
}

fn parse_args() -> Result<Arguments, String> {
    let mut mode = None;
    let mut load_mode = "quiet".to_string();
    let mut output = None;
    let mut workload = "all".to_string();
    let mut warmup = WARMUP_DEFAULT;
    let mut measured = MEASURED_DEFAULT;
    let mut seed = 0x5233_u64;
    let mut run_index = 1usize;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--mode" => mode = Some(value),
            "--load-mode" => load_mode = value,
            "--output" => output = Some(PathBuf::from(value)),
            "--workload" => workload = value,
            "--warmup" => warmup = parse_usize(&value, "warmup")?,
            "--measured" => measured = parse_usize(&value, "measured")?,
            "--seed" => {
                seed = value
                    .parse()
                    .map_err(|_| "seed must be a u64".to_string())?
            }
            "--run-index" => run_index = parse_usize(&value, "run-index")?,
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    if warmup > 100_000 || measured == 0 || measured > 100_000 || run_index == 0 {
        return Err("warmup, measured, or run-index is outside its supported range".into());
    }
    let mode = mode.ok_or_else(|| "--mode is required".to_string())?;
    if mode != "contracts" && mode != "precision" {
        return Err("--mode must be contracts or precision".into());
    }
    if load_mode != "quiet" && load_mode != "cpu_contention" {
        return Err("--load-mode must be quiet or cpu_contention".into());
    }
    let output = output.ok_or_else(|| "--output is required".to_string())?;
    Ok(Arguments {
        mode,
        load_mode,
        output,
        workload,
        warmup,
        measured,
        seed,
        run_index,
    })
}

fn parse_usize(value: &str, label: &str) -> Result<usize, String> {
    value
        .parse()
        .map_err(|_| format!("{label} must be an unsigned integer"))
}

fn select_workloads(name: &str) -> Result<Vec<Workload>, String> {
    if name == "all" {
        return Ok(WORKLOADS.to_vec());
    }
    WORKLOADS
        .iter()
        .copied()
        .find(|workload| workload.id == name)
        .map(|workload| vec![workload])
        .ok_or_else(|| {
            format!("unknown workload {name:?}; use all or one of the declared workload IDs")
        })
}

fn contracts_report() -> Result<Value, String> {
    let duplicate_tail_validation = validate_duplicate_up_tail();
    let mut vector_a = Vec::new();
    for (frequency_hz, odd_vector) in [(1_000_000, false), (10_000_000, false), (10_000_003, true)]
    {
        vector_a.push(run_lifecycle_vector(frequency_hz, odd_vector)?);
    }
    let expired_without_watchdog = run_lease_case("expired-no-watchdog", 1_000_000, 4_000_001)?;
    let timeout_equality = run_lease_case("equality", 1_000_000, 4_000_000)?;
    let prior_fresh_progress = run_lease_case("fresh-progress", 3_999_999, 4_000_000)?;
    let lease_reproduced = expired_without_watchdog["dispatch_was_allowed"].as_bool() == Some(true)
        && expired_without_watchdog["musical_sender_attempts"]
            .as_u64()
            .unwrap_or(0)
            == 1;
    Ok(json!({
        "A_late_same_key_pause_resume": {
            "hypothesis_status": if vector_a.iter().any(|case| case["below_completion_plus_frame"] == true) { "REPRODUCED" } else { "NO-REPRODUCTION" },
            "vector_cases": vector_a,
            "exact_vector_duplicate_tail": duplicate_tail_validation,
            "path_evidence": "prepared normal dispatch followed by the dispatch-loop verified resumable cleanup transition and PlaybackClock manual pause/resume; no direct guard.reset call",
            "oracle": "next Down pre-call must be at least prior musical Up completion + one frame"
        },
        "B_enabled_lease_watchdog_delayed": {
            "hypothesis_status": if lease_reproduced { "REPRODUCED" } else { "NO-REPRODUCTION" },
            "expired_without_watchdog": expired_without_watchdog,
            "timeout_equality": timeout_equality,
            "prior_fresh_progress": prior_fresh_progress,
            "path_evidence": "prepared normal final Down admission; watchdog is not started by the test-support worker harness"
        },
        "C_serial_HOL": run_hol_vector()?,
        "D_focus_loss_policy": run_focus_vectors()?,
        "E_empty_cleanup": run_empty_cleanup()?,
        "scope": "characterization only; no production timing, lease, scheduler, or cleanup policy changed"
    }))
}

fn validate_duplicate_up_tail() -> Value {
    let actions = [
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: vec![0x15].into(),
            reason: "r3-p0-exact-vector-down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 17_167,
            scan_codes: vec![0x15].into(),
            reason: "r3-p0-exact-vector-up".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 34_334,
            scan_codes: vec![0x15].into(),
            reason: "r3-p0-exact-vector-redown".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 51_501,
            scan_codes: vec![0x15].into(),
            reason: "r3-p0-exact-vector-up-a".into(),
        },
        KeyActionInput {
            source_action_index: 4,
            kind: ActionKind::Up,
            scheduled_us: 51_501,
            scan_codes: vec![0x15].into(),
            reason: "r3-p0-exact-vector-up-b".into(),
        },
    ];
    match compile_runtime_intents(&actions, &[0x15]) {
        Ok(_) => json!({"accepted":true,"status":"COMPILED"}),
        Err(error) => {
            json!({"accepted":false,"status":"REJECTED_BY_SCHEDULE_COMPILER","error":format!("{error:?}"),"disposition":"The duplicate same-key same-timestamp Up tail is malformed; the executable lifecycle probe exercises the valid prefix through D0,U17167,D34334."})
        }
    }
}

fn run_lifecycle_vector(frequency_hz: u64, odd_vector: bool) -> Result<Value, String> {
    let frequency = NonZeroU64::new(frequency_hz).ok_or("zero QPC frequency")?;
    let clock = QpcClock::from_frequency_hz(frequency);
    let up_authored_us = if odd_vector { 17_168 } else { 17_167 };
    let mut harness = ProductionDispatchTestHarness::new_r3_lifecycle_sequence_for_test(
        frequency_hz,
        odd_vector,
    )?;
    let packets = harness.configure_r3_mock_packet_sender_for_test(&[0, 50, 0, 0]);
    harness.prepare_prepared_stream_for_test();

    let epoch = ticks_at_us(clock, 82_700)?;
    harness.reanchor_playback_clock_for_r3_probe(epoch)?;
    let down_at = ticks_at_us(clock, 100_000)?;
    harness.set_r3_mock_sender_start_qpc_for_test(down_at);
    let first_step =
        harness.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(down_at);
    let first_observation = harness.pop_r3_send_observation_for_test();

    let down_completion = evidence_completion(&packets, 0)?;
    let hold_ticks = duration_ticks(clock, HOLD_US)?;
    let frame_ticks = duration_ticks(clock, FRAME_US)?;
    let authored_up_target = epoch
        .checked_add_duration(duration_ticks(clock, up_authored_us)?)
        .map_err(|error| format!("authored Up target overflow: {error}"))?;
    let hold_floor = down_completion
        .checked_add_duration(hold_ticks)
        .map_err(|error| format!("hold floor overflow: {error}"))?;
    let up_at = authored_up_target.max(hold_floor);
    harness.set_r3_mock_sender_start_qpc_for_test(up_at);
    let up_step =
        harness.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(up_at);
    let up_observation = harness.pop_r3_send_observation_for_test();
    let up_completion = evidence_completion(&packets, 1)?;

    let pause_at = ticks_at_us(clock, 117_300)?;
    let resume_at = ticks_at_us(clock, 118_300)?;
    harness.set_r3_mock_sender_start_qpc_for_test(pause_at);
    harness.manual_pause_resume_for_r3_probe(pause_at, resume_at)?;
    let packet_count_after_empty_cleanup = packets
        .lock()
        .map_err(|_| "P0 packet capture lock poisoned".to_string())?
        .len();

    let next_down_at = resume_at;
    harness.set_r3_mock_sender_start_qpc_for_test(next_down_at);
    let next_down_step = harness
        .dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(next_down_at);
    let next_down_observation = harness.pop_r3_send_observation_for_test();
    let next_down_pre_call = evidence_start(&packets, 2)?;
    let required_down_floor = up_completion
        .checked_add_duration(frame_ticks)
        .map_err(|error| format!("release floor overflow: {error}"))?;
    let is_below = next_down_pre_call < required_down_floor;
    let authored_offsets = if odd_vector {
        vec![0, 17_168, 34_336, 51_504, 51_504]
    } else {
        vec![0, 17_167, 34_334, 51_501, 51_501]
    };
    Ok(json!({
        "qpc_frequency_hz": frequency_hz,
        "conversion": {
            "authored_offsets_us": authored_offsets,
            "authored_offset_ticks": authored_offsets.iter().map(|us| duration_ticks(clock,*us).map(|ticks| ticks.as_u64())).collect::<Result<Vec<_>,_>>()?,
            "hold_h_us": HOLD_US,
            "hold_h_ticks": hold_ticks.as_u64(),
            "same_key_gap_g_us": FRAME_US,
            "same_key_gap_g_ticks": frame_ticks.as_u64(),
            "odd_frequency_feasible_extra_us": if odd_vector { 1 } else { 0 }
        },
        "events": {
            "down": event_json(clock, evidence_start(&packets,0)?, down_completion)?,
            "musical_up": event_json(clock, evidence_start(&packets,1)?, up_completion)?,
            "pause_qpc": tick_json(clock,pause_at)?,
            "resume_qpc": tick_json(clock,resume_at)?,
            "next_down": event_json(clock,next_down_pre_call,evidence_completion(&packets,2)?)?
        },
        "steps": {
            "first_down": format!("{first_step:?}"),
            "musical_up": format!("{up_step:?}"),
            "next_down": format!("{next_down_step:?}")
        },
        "prepared_observations_available": [first_observation.is_some(),up_observation.is_some(),next_down_observation.is_some()],
        "empty_cleanup_packet_count": packet_count_after_empty_cleanup.saturating_sub(2),
        "required_next_down_not_before": tick_json(clock,required_down_floor)?,
        "below_completion_plus_frame": is_below,
        "disposition": if is_below { "REPRODUCED" } else { "NO-REPRODUCTION" }
    }))
}

fn run_lease_case(name: &str, last_progress_us: u64, now_us: u64) -> Result<Value, String> {
    let clock = QpcClock::initialize().map_err(|error| format!("QPC: {error:?}"))?;
    let mut harness =
        ProductionDispatchTestHarness::new_r3_prepared_down_or_uponly_for_test(1, 20_000)?;
    let packets = harness.configure_r3_mock_packet_sender_for_test(&[0]);
    harness.prepare_prepared_stream_for_test();
    harness.set_playback_epoch_qpc_for_test(QpcTicks::ZERO);
    harness.set_supervisor_lease_timeout_for_r3_probe(3_000_000)?;
    let last_progress = ticks_at_us(clock, last_progress_us)?;
    let now = ticks_at_us(clock, now_us)?;
    harness.set_supervisor_heartbeat_for_test(last_progress);
    harness.set_r3_mock_sender_start_qpc_for_test(now);
    let step =
        harness.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(now);
    let packet_count = packets
        .lock()
        .map_err(|_| "lease capture lock poisoned".to_string())?
        .len();
    let (attempts, packet) = if packet_count == 0 {
        (0, None)
    } else {
        let (attempts, packet) = packet_attempts(&packets, 0)?;
        (attempts, Some(packet_json(packet)))
    };
    Ok(json!({
        "case": name,
        "lease_timeout_us": 3_000_000,
        "lease_timeout_ticks": duration_ticks(clock, 3_000_000)?.as_u64(),
        "lease_enabled": true,
        "last_progress_us": last_progress_us,
        "now_us": now_us,
        "elapsed_us": now_us.saturating_sub(last_progress_us),
        "expired_latch_before": false,
        "dispatch_step": format!("{step:?}"),
        "musical_sender_attempts": attempts,
        "packet": packet,
        "dispatch_was_allowed": matches!(step, DispatchStep::Dispatched),
        "disposition": if matches!(step, DispatchStep::Dispatched) { "STALE_LEASE_ACCEPTED" } else { "BLOCKED" }
    }))
}

fn run_hol_vector() -> Result<Value, String> {
    let frequency_hz = 1_000_000;
    let clock = QpcClock::from_frequency_hz(NonZeroU64::new(frequency_hz).expect("frequency"));
    let mut harness = ProductionDispatchTestHarness::new_r3_hol_sequence_for_test(frequency_hz)?;
    let packets = harness.configure_r3_mock_packet_sender_for_test(&[10_000, 0, 0, 0]);
    harness.prepare_prepared_stream_for_test();
    harness.set_playback_epoch_qpc_for_test(QpcTicks::ZERO);

    harness.set_r3_mock_sender_start_qpc_for_test(ticks_at_us(clock, 100_000)?);
    let down = harness.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(
        ticks_at_us(clock, 100_000)?,
    );
    let down_completion = evidence_completion(&packets, 0)?;
    let a_up_at = down_completion
        .checked_add_duration(duration_ticks(clock, HOLD_US)?)
        .map_err(|error| format!("HOL Up floor overflow: {error}"))?;
    harness.set_r3_mock_sender_start_qpc_for_test(a_up_at);
    let a_up =
        harness.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(a_up_at);
    let b_target = ticks_at_us(clock, 118_000)?;
    let b_at = a_up_at;
    let cursor_before_b = harness.prepared_cursor_for_test();
    harness.set_r3_mock_sender_start_qpc_for_test(b_at);
    let b_down =
        harness.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(b_at);
    let cursor_after_b = harness.prepared_cursor_for_test();
    Ok(json!({
        "status": "CHARACTERIZATION_ONLY",
        "qpc_frequency_hz": frequency_hz,
        "authored_events_us": {"a_down":100_000,"a_up":117_167,"b_down":118_000,"b_up":135_167},
        "a_down_completion": tick_json(clock,down_completion)?,
        "a_up_target_us":117_167,
        "a_up_actual_pre_call": tick_json(clock,evidence_start(&packets,1)?)?,
        "b_target": tick_json(clock,b_target)?,
        "b_earliest_actual_pre_call": tick_json(clock,evidence_start(&packets,2)?)?,
        "b_lateness_us": clock.duration_to_us(b_at.checked_duration_since(b_target).map_err(|error| format!("HOL target order: {error}"))?).map_err(|error| format!("HOL lateness conversion: {error:?}"))?,
        "cursor_before_b": cursor_before_b,
        "cursor_after_b": cursor_after_b,
        "steps": {"a_down":format!("{down:?}"),"a_up":format!("{a_up:?}"),"b_down":format!("{b_down:?}")},
        "sender_attempts": [packet_attempts(&packets,0)?.0,packet_attempts(&packets,1)?.0,packet_attempts(&packets,2)?.0],
        "no_reorder_or_catch_up_claim": true
    }))
}

fn run_focus_vectors() -> Result<Value, String> {
    let mut restore =
        ProductionDispatchTestHarness::new_r3_prepared_down_or_uponly_for_test(1, 50_000)?;
    let restore_packets = restore.configure_r3_mock_packet_sender_for_test(&[]);
    let restore_cleanup = restore.configure_r3_mock_cleanup_emitter_for_test();
    restore.prepare_prepared_stream_for_test();
    let down_at = restore.qpc_now_for_test()?;
    restore.reanchor_playback_clock_for_r3_probe(down_at)?;
    restore.set_r3_mock_sender_start_qpc_for_test(down_at);
    let down_step =
        restore.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(down_at);
    let focus_lost_at = down_at
        .checked_add_duration(restore.qpc_duration_from_us_for_test(1_000)?)
        .map_err(|error| format!("focus-lost timestamp overflow: {error}"))?;
    restore.lose_focus_for_r3_probe(focus_lost_at)?;
    let packets_while_unfocused = restore_packets
        .lock()
        .map_err(|_| "focus capture lock poisoned".to_string())?
        .len();
    let cleanup_sends_while_unfocused = restore_cleanup
        .lock()
        .map_err(|_| "focus cleanup capture lock poisoned".to_string())?
        .len();
    let owned_mask_while_unfocused = restore.backend_active_mask();
    let focus_restored_at = focus_lost_at
        .checked_add_duration(restore.qpc_duration_from_us_for_test(10_000)?)
        .map_err(|error| format!("focus-restore timestamp overflow: {error}"))?;
    restore.set_r3_mock_sender_start_qpc_for_test(focus_restored_at);
    restore.restore_focus_for_r3_probe(focus_restored_at)?;
    let packets_after_restore = restore_packets
        .lock()
        .map_err(|_| "focus capture lock poisoned".to_string())?
        .len();
    let cleanup_after_restore = cleanup_sends_json(&restore_cleanup)?;
    let cleanup_sends_after_restore = cleanup_after_restore.len();
    let accounting_after_restore = restore.generation_accounting_for_test();
    let active_mask_after_restore = restore.backend_active_mask();
    let restored_case = json!({
        "down_step":format!("{down_step:?}"),
        "packets_while_unfocused":packets_while_unfocused,
        "cleanup_sends_while_unfocused":cleanup_sends_while_unfocused,
        "owned_mask_while_unfocused":owned_mask_while_unfocused,
        "packets_after_trustworthy_restore":packets_after_restore,
        "cleanup_sends_after_restore":cleanup_after_restore,
        "active_mask_after_restore":active_mask_after_restore,
        "release_obligation_after_restore":restore.release_obligation_mask_for_test(),
        "cancelled_generations":accounting_after_restore.cancelled,
        "release_on_restore":cleanup_sends_while_unfocused == 0 && cleanup_sends_after_restore > 0 && active_mask_after_restore == 0
    });

    let mut terminal =
        ProductionDispatchTestHarness::new_r3_prepared_down_or_uponly_for_test(1, 50_000)?;
    let terminal_packets = terminal.configure_r3_mock_packet_sender_for_test(&[]);
    let terminal_cleanup = terminal.configure_r3_mock_cleanup_emitter_for_test();
    terminal.prepare_prepared_stream_for_test();
    let terminal_down_at = terminal.qpc_now_for_test()?;
    terminal.reanchor_playback_clock_for_r3_probe(terminal_down_at)?;
    terminal.set_r3_mock_sender_start_qpc_for_test(terminal_down_at);
    let terminal_down = terminal
        .dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(
            terminal_down_at,
        );
    terminal.lose_focus_for_r3_probe(terminal.qpc_now_for_test()?)?;
    let packets_before_terminal_cleanup = terminal_packets
        .lock()
        .map_err(|_| "terminal capture lock poisoned".to_string())?
        .len();
    let cleanup_sends_before_terminal_cleanup = terminal_cleanup
        .lock()
        .map_err(|_| "terminal cleanup capture lock poisoned".to_string())?
        .len();
    terminal.set_r3_mock_sender_start_qpc_for_test(terminal.qpc_now_for_test()?);
    let finalized = terminal.finalize_for_r3_probe();
    let packets_after_terminal_cleanup = terminal_packets
        .lock()
        .map_err(|_| "terminal capture lock poisoned".to_string())?
        .len();
    let cleanup_sends_after_terminal_cleanup = cleanup_sends_json(&terminal_cleanup)?;
    let cleanup_attempted =
        cleanup_sends_after_terminal_cleanup.len() > cleanup_sends_before_terminal_cleanup;
    Ok(json!({
        "status":"CHARACTERIZATION_ONLY",
        "path_evidence":"test-support enter_focus_pause; no cleanup while unfocused; modeled trustworthy restore enters the dispatch loop's shared verified cleanup and prepared reconciliation path. The focus-grace scheduler itself is not exercised by this deterministic probe. Terminal behavior invokes the existing test-support finalizer.",
        "restore_path":restored_case,
        "terminal_cleanup_while_unfocused":{
            "down_step":format!("{terminal_down:?}"),
            "packets_before_cleanup":packets_before_terminal_cleanup,
            "packets_after_cleanup":packets_after_terminal_cleanup,
            "cleanup_sends_before_cleanup":cleanup_sends_before_terminal_cleanup,
            "cleanup_sends_after_cleanup":cleanup_sends_after_terminal_cleanup,
            "finalize":{
                "outcome_code":finalized.outcome_code,
                "attempted_mask":finalized.attempted_mask,
                "release_obligation_mask":finalized.release_obligation_mask,
                "active_mask":finalized.active_mask,
                "possibly_active_mask":finalized.possibly_active_mask,
                "failed_release_mask":finalized.failed_release_mask,
                "in_flight_mask":finalized.in_flight_mask,
                "activated":finalized.activated,
                "released":finalized.released,
                "cancelled":finalized.cancelled,
                "active_generations":finalized.active_generations
            },
            "ownership_scoped_terminal_release":cleanup_attempted
        }
    }))
}

fn run_empty_cleanup() -> Result<Value, String> {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let packets = harness.configure_r3_mock_packet_sender_for_test(&[]);
    let outcome = harness.release_all_for_r3_probe();
    let transport_calls = packets
        .lock()
        .map_err(|_| "empty cleanup capture lock poisoned".to_string())?
        .len();
    Ok(json!({
        "status":"CHARACTERIZATION_ONLY",
        "requested_mask":0,
        "attempted_mask":outcome.attempted_mask,
        "released_successfully":outcome.released_successfully,
        "transport_anomaly":outcome.transport_anomaly,
        "keyboard_transport_calls":transport_calls,
        "oracle_passed":outcome.attempted_mask == 0 && transport_calls == 0
    }))
}

fn precision_report(
    workload: Workload,
    warmup_count: usize,
    measured_count: usize,
    seed: u64,
    run_index: usize,
) -> Result<Value, String> {
    let mut admission_ticks = Vec::with_capacity(measured_count);
    let mut completion_ticks = Vec::with_capacity(measured_count);
    let mut admission_us = Vec::with_capacity(measured_count);
    let mut completion_us = Vec::with_capacity(measured_count);
    let mut measured_sender_attempts = 0u64;
    let mut setup_sender_attempts = 0u64;
    let mut packet_totals = [0u64; 2];
    let mut activated = 0u64;
    let mut released = 0u64;
    let mut cancelled = 0u64;
    let mut final_release_obligation_mask = 0u16;
    let mut failed_samples = 0usize;
    let total = warmup_count.saturating_add(measured_count);

    for sample_index in 0..total {
        let mut harness = make_precision_harness(workload)?;
        let _packets = harness.configure_r3_mock_packet_sender_for_test(&[]);
        harness.prepare_prepared_stream_for_test();
        let needs_active_keys = matches!(
            workload.kind,
            WorkloadKind::Mixed(_, _) | WorkloadKind::UpOnly(_)
        );
        if needs_active_keys {
            let now = harness.qpc_now_for_test()?;
            let old_by = harness.qpc_duration_from_us_for_test(100_000)?;
            let setup_at = QpcTicks::from_raw(
                now.as_u64()
                    .checked_sub(old_by.as_u64())
                    .ok_or_else(|| "precision setup QPC underflow".to_string())?,
            );
            harness.set_playback_epoch_qpc_for_test(setup_at);
            harness.set_r3_mock_sender_start_qpc_for_test(setup_at);
            let setup_step = harness
                .dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(
                    setup_at,
                );
            if !matches!(setup_step, DispatchStep::Dispatched) {
                failed_samples += 1;
                continue;
            }
            setup_sender_attempts += 1;
            while harness.pop_observation().is_some() {}
        }

        let now = harness.qpc_now_for_test()?;
        let offsets = harness.prepared_frame_offsets_for_test();
        let cursor = harness.prepared_cursor_for_test();
        let offset = *offsets.get(cursor).ok_or_else(|| {
            format!(
                "{} prepared stream has no frame at cursor {cursor}",
                workload.id
            )
        })?;
        let epoch = QpcTicks::from_raw(
            now.as_u64()
                .checked_sub(offset)
                .ok_or_else(|| "precision frame epoch underflow".to_string())?,
        );
        harness.set_playback_epoch_qpc_for_test(epoch);
        harness.set_r3_mock_sender_start_qpc_for_test(now);
        let step = harness.dispatch_prepared_current_at_synthetic_wall_qpc_for_test(now);
        if !matches!(step, DispatchStep::Dispatched) {
            failed_samples += 1;
            continue;
        }
        let Some((packet, pre_call, completed, attempts)) =
            harness.pop_r3_send_observation_for_test()
        else {
            failed_samples += 1;
            continue;
        };
        if attempts != 1 || !packet_matches_workload(packet, workload.kind) {
            failed_samples += 1;
            continue;
        }
        let start = now;
        let admission = pre_call
            .checked_duration_since(start)
            .map_err(|error| format!("precision pre-call precedes sample start: {error}"))?;
        let completion = completed
            .checked_duration_since(pre_call)
            .map_err(|error| format!("precision completion precedes pre-call: {error}"))?;
        if sample_index >= warmup_count {
            let clock = QpcClock::initialize().map_err(|error| format!("QPC: {error:?}"))?;
            admission_ticks.push(admission.as_u64());
            completion_ticks.push(completion.as_u64());
            admission_us.push(
                clock
                    .duration_to_us(admission)
                    .map_err(|error| format!("admission conversion: {error:?}"))?,
            );
            completion_us.push(
                clock
                    .duration_to_us(completion)
                    .map_err(|error| format!("completion conversion: {error:?}"))?,
            );
            measured_sender_attempts += u64::from(attempts);
            packet_totals[0] += u64::from(packet.up_mask.count_ones());
            packet_totals[1] += u64::from(packet.down_mask.count_ones());
            let accounting = harness.generation_accounting_for_test();
            activated = activated.saturating_add(accounting.activated);
            released = released.saturating_add(accounting.released);
            cancelled = cancelled.saturating_add(accounting.cancelled);
            final_release_obligation_mask = harness.release_obligation_mask_for_test();
        }
    }
    let eligible = failed_samples == 0
        && admission_ticks.len() == measured_count
        && measured_sender_attempts == measured_count as u64
        && measured_count == MEASURED_DEFAULT
        && warmup_count == WARMUP_DEFAULT;
    Ok(json!({
        "workload":workload.id,
        "transport_kind":"deterministic-mock",
        "seed":seed,
        "run_index":run_index,
        "warmup_count":warmup_count,
        "measured_count":measured_count,
        "sample_count":admission_ticks.len(),
        "failed_sample_count":failed_samples,
        "packet_shape":workload_shape(workload.kind),
        "counters":{
            "measured_sender_attempts":measured_sender_attempts,
            "expected_measured_sender_attempts":measured_count,
            "setup_sender_attempts":setup_sender_attempts,
            "up_events":packet_totals[0],
            "down_events":packet_totals[1],
            "activated_generations":activated,
            "released_generations":released,
            "cancelled_generations":cancelled,
            "final_release_obligation_mask":final_release_obligation_mask,
            "frame_gap_g_us":FRAME_US,
            "effective_hold_h_us":HOLD_US,
            "timing_margin_us":MARGIN_US
        },
        "distributions":{
            "admission_to_pre_call":distribution(&admission_ticks,&admission_us),
            "pre_call_to_sender_completion":distribution(&completion_ticks,&completion_us)
        },
        "raw_samples":{
            "admission_to_pre_call_ticks":admission_ticks,
            "admission_to_pre_call_us":admission_us,
            "pre_call_to_sender_completion_ticks":completion_ticks,
            "pre_call_to_sender_completion_us":completion_us
        },
        "statistics_eligible":eligible,
        "boundary_notes":"setup and prepared-stream construction occur outside each timed sample; one measured production prepared dispatch and one sender attempt per accepted sample; mock transport only"
    }))
}

fn make_precision_harness(workload: Workload) -> Result<ProductionDispatchTestHarness, String> {
    match workload.kind {
        WorkloadKind::Down(keys) | WorkloadKind::UpOnly(keys) => {
            ProductionDispatchTestHarness::new_r3_prepared_down_or_uponly_for_test(keys, 20_000)
        }
        WorkloadKind::Mixed(up, down) => {
            ProductionDispatchTestHarness::new_r3_prepared_mixed_for_test(up, down, 20_000)
        }
    }
}

fn workload_shape(kind: WorkloadKind) -> Value {
    match kind {
        WorkloadKind::Down(keys) => json!({"kind":"Down","down_keys":keys,"up_keys":0}),
        WorkloadKind::Mixed(up, down) => {
            json!({"kind":"Mixed","up_keys":up,"down_keys":down,"total_changed_keys":up+down})
        }
        WorkloadKind::UpOnly(keys) => json!({"kind":"UpOnly","up_keys":keys,"down_keys":0}),
    }
}

fn packet_matches_workload(packet: PhysicalPacket, kind: WorkloadKind) -> bool {
    match kind {
        WorkloadKind::Down(keys) => {
            packet.up_mask == 0 && packet.down_mask.count_ones() as usize == keys
        }
        WorkloadKind::Mixed(up, down) => {
            packet.up_mask.count_ones() as usize == up
                && packet.down_mask.count_ones() as usize == down
        }
        WorkloadKind::UpOnly(keys) => {
            packet.down_mask == 0 && packet.up_mask.count_ones() as usize == keys
        }
    }
}

fn distribution(ticks: &[u64], micros: &[u64]) -> Value {
    let mut sorted_ticks = ticks.to_vec();
    sorted_ticks.sort_unstable();
    let mut sorted_us = micros.to_vec();
    sorted_us.sort_unstable();
    json!({
        "sample_count":ticks.len(),
        "ticks":{
            "p50":nearest_rank(&sorted_ticks,50,100),
            "p95":nearest_rank(&sorted_ticks,95,100),
            "p99":nearest_rank(&sorted_ticks,99,100),
            "max":sorted_ticks.last().copied()
        },
        "microseconds":{
            "p50":nearest_rank(&sorted_us,50,100),
            "p95":nearest_rank(&sorted_us,95,100),
            "p99":nearest_rank(&sorted_us,99,100),
            "max":sorted_us.last().copied()
        }
    })
}

fn nearest_rank(values: &[u64], numerator: usize, denominator: usize) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    let rank = values
        .len()
        .saturating_mul(numerator)
        .div_ceil(denominator)
        .max(1);
    values.get(rank.saturating_sub(1)).copied()
}

fn packet_attempts(
    packets: &Arc<Mutex<Vec<(PhysicalPacket, SendEvidence)>>>,
    index: usize,
) -> Result<(u8, PhysicalPacket), String> {
    let packets = packets
        .lock()
        .map_err(|_| "P0 packet capture lock poisoned".to_string())?;
    let (packet, evidence) = packets
        .get(index)
        .ok_or_else(|| format!("expected packet capture {index}, got {}", packets.len()))?;
    Ok((evidence.attempts, *packet))
}

fn evidence_start(
    packets: &Arc<Mutex<Vec<(PhysicalPacket, SendEvidence)>>>,
    index: usize,
) -> Result<QpcTicks, String> {
    let packets = packets
        .lock()
        .map_err(|_| "P0 packet capture lock poisoned".to_string())?;
    packets
        .get(index)
        .and_then(|(_, evidence)| evidence.started_ticks)
        .ok_or_else(|| format!("missing sender start evidence {index}"))
}

fn evidence_completion(
    packets: &Arc<Mutex<Vec<(PhysicalPacket, SendEvidence)>>>,
    index: usize,
) -> Result<QpcTicks, String> {
    let packets = packets
        .lock()
        .map_err(|_| "P0 packet capture lock poisoned".to_string())?;
    packets
        .get(index)
        .and_then(|(_, evidence)| evidence.completed_ticks)
        .ok_or_else(|| format!("missing sender completion evidence {index}"))
}

fn packet_json(packet: PhysicalPacket) -> Value {
    json!({"up_mask":packet.up_mask,"down_mask":packet.down_mask,"event_count":packet.event_count()})
}

fn cleanup_sends_json(captured: &RtR3CleanupCapture) -> Result<Vec<Value>, String> {
    captured
        .lock()
        .map_err(|_| "P0 cleanup capture lock poisoned".to_string())
        .map(|sends| {
            sends
                .iter()
                .map(|(scan_codes, key_up)| json!({"scan_codes":scan_codes,"key_up":key_up}))
                .collect()
        })
}

fn event_json(clock: QpcClock, start: QpcTicks, completion: QpcTicks) -> Result<Value, String> {
    Ok(json!({"pre_call":tick_json(clock,start)?,"completion":tick_json(clock,completion)?}))
}

fn tick_json(clock: QpcClock, ticks: QpcTicks) -> Result<Value, String> {
    let us = clock
        .duration_to_us(DurationTicks::from_raw(ticks.as_u64()))
        .map_err(|error| format!("QPC tick conversion: {error:?}"))?;
    Ok(json!({"qpc_ticks":ticks.as_u64(),"microseconds_from_zero":us}))
}

fn ticks_at_us(clock: QpcClock, us: u64) -> Result<QpcTicks, String> {
    Ok(QpcTicks::from_raw(duration_ticks(clock, us)?.as_u64()))
}

fn duration_ticks(clock: QpcClock, us: u64) -> Result<DurationTicks, String> {
    clock
        .duration_from_us(us)
        .map_err(|error| format!("checked QPC conversion for {us}us: {error:?}"))
}

fn host_metadata() -> (String, Value, String, String) {
    let target = std::env::var("SKY_RT_R3_TARGET").unwrap_or_else(|_| rustc_host());
    let toolchain = json!({
        "rustc":std::env::var("SKY_RT_R3_RUSTC_VERSION").unwrap_or_else(|_|command_version("rustc","--version")),
        "cargo":std::env::var("SKY_RT_R3_CARGO_VERSION").unwrap_or_else(|_|command_version("cargo","--version")),
        "bun":std::env::var("SKY_RT_R3_BUN_VERSION").unwrap_or_else(|_|command_version("bun","--version")),
        "pwsh":std::env::var("SKY_RT_R3_PWSH_VERSION").unwrap_or_else(|_|command_version("pwsh","--version")),
        "git":std::env::var("SKY_RT_R3_GIT_VERSION").unwrap_or_else(|_|command_version("git","--version"))
    });
    let os = std::env::var("SKY_RT_R3_OS").unwrap_or_else(|_| {
        format!(
            "{} {}",
            std::env::consts::OS,
            std::env::var("OS").unwrap_or_default()
        )
    });
    let cpu = std::env::var("SKY_RT_R3_CPU").unwrap_or_else(|_| {
        std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unknown".to_string())
    });
    (target, toolchain, os, cpu)
}

fn command_version(program: &str, arg: &str) -> String {
    Command::new(program)
        .arg(arg)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unavailable".to_string())
}

fn rustc_host() -> String {
    let output = Command::new("rustc").arg("-vV").output();
    output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8(output.stdout)
                .ok()?
                .lines()
                .find_map(|line| line.strip_prefix("host: ").map(str::to_string))
        })
        .unwrap_or_else(|| format!("{}-unknown-unknown", std::env::consts::ARCH))
}

fn executable_sha256() -> Result<String, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("current executable path: {error}"))?;
    #[cfg(windows)]
    let output = Command::new("certutil.exe")
        .arg("-hashfile")
        .arg(&executable)
        .arg("SHA256")
        .output();
    #[cfg(not(windows))]
    let output = Command::new("sha256sum").arg(&executable).output();
    let output = output.map_err(|error| format!("start executable SHA-256 tool: {error}"))?;
    if !output.status.success() {
        return Err("executable SHA-256 tool failed".into());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.split_whitespace()
        .map(|token| token.trim().to_ascii_lowercase())
        .find(|token| token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| "could not parse executable SHA-256".to_string())
}

fn git_revision() -> Result<String, String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| format!("run git rev-parse HEAD: {error}"))?;
    if !output.status.success() {
        return Err("git rev-parse HEAD failed".into());
    }
    let revision = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if revision.len() != 40 || !revision.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("git HEAD is not a full 40-hex commit".into());
    }
    Ok(revision.to_ascii_lowercase())
}

fn generated_run_id() -> String {
    format!("p0-{}-{}", unix_ms().unwrap_or(0), std::process::id())
}

fn unix_ms() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|error| format!("system clock precedes Unix epoch: {error}"))
}
