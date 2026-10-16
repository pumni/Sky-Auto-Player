//! Executable Coordinator R3 timing and lifecycle contracts.
//!
//! This example is compiled only with `test-support`. It calls the existing
//! prepared worker dispatch path and labels deterministic transport evidence
//! as mock data. It never sends real keyboard input.

use serde_json::{Value, json};
use sky_dispatch_core::compile::compile_runtime_intents;
use sky_dispatch_core::model::{
    ActionKind, CompactIntent, CompiledBatch, CompiledPacket, KeyActionInput, RuntimeSchedule,
};
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::{QpcClock, QpcTicks, qpc_frequency_checked};
use sky_dispatch_win32::input::{PHYSICAL_INSTRUMENT_SCAN_CODES, PhysicalPacket, SendEvidence};
use sky_player::engine::dispatch_primitives::{
    DispatchStep, PRODUCTION_MIN_PREROLL_US, PRODUCTION_STARTUP_READINESS_RESERVE_US,
    ProductionDispatchTestHarness, RtR3CleanupCapture,
};
use std::alloc::{GlobalAlloc, Layout, System};
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const HOLD_US: u64 = 17_167;
const FRAME_US: u64 = 16_667;
const MARGIN_US: u64 = 500;
const WARMUP_DEFAULT: usize = 1_000;
const MEASURED_DEFAULT: usize = 10_000;
const P4_STARTUP_SEED: u64 = 0x5234_5034;
const P4_STARTUP_HOLD_US: u64 = HOLD_US;
const P4_DIAGNOSTIC_GENERATION_COUNTS: [usize; 4] = [100, 1_000, 10_000, 100_000];

struct DiagnosticAllocator;

static TRACK_ALLOCATIONS: AtomicBool = AtomicBool::new(false);
static TRACKED_ALLOCATION_CALLS: AtomicU64 = AtomicU64::new(0);
static TRACKED_ALLOCATION_BYTES: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for DiagnosticAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if TRACK_ALLOCATIONS.load(Ordering::Relaxed) && !pointer.is_null() {
            TRACKED_ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            TRACKED_ALLOCATION_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if TRACK_ALLOCATIONS.load(Ordering::Relaxed) && !pointer.is_null() {
            TRACKED_ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            TRACKED_ALLOCATION_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let replacement = unsafe { System.realloc(pointer, layout, new_size) };
        if TRACK_ALLOCATIONS.load(Ordering::Relaxed) && !replacement.is_null() {
            TRACKED_ALLOCATION_CALLS.fetch_add(1, Ordering::Relaxed);
            TRACKED_ALLOCATION_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        replacement
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static DIAGNOSTIC_ALLOCATOR: DiagnosticAllocator = DiagnosticAllocator;

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
    generation_count: usize,
}

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--help") {
        println!(
            "Usage: rt_r3_probe --mode <contracts|precision|hol|startup|polling> --output <path> [options]\n\
             Diagnostics modes: hol; startup --generation-count <100|1000|10000|100000>; polling.\n\
             Polling includes the fixed 17167us short-hold reference.\n\
             Other options: --load-mode <quiet|cpu_contention>, --workload <all|id>, --warmup <n>, --measured <n>, --seed <u64>, --run-index <n>.\n\
             All diagnostics use test-support prepared paths or offline models; no real keyboard input is emitted."
        );
        return;
    }
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
            "hol" => {
                let hol = run_hol_diagnostics()?;
                (
                    "fixed-hol-scenarios".to_string(),
                    0,
                    0,
                    json!({"hol_scenarios": hol}),
                    json!({}),
                    json!({}),
                    false,
                )
            }
            "startup" => {
                let startup = run_startup_case(arguments.generation_count, arguments.seed)?;
                (
                    format!("startup-{}-generations", arguments.generation_count),
                    0,
                    0,
                    json!({"startup_case": startup}),
                    json!({}),
                    json!({}),
                    false,
                )
            }
            "polling" => {
                let polling = run_frame_polling_model()?;
                (
                    "fixed-frame-polling-matrix".to_string(),
                    0,
                    0,
                    json!({"frame_polling": polling}),
                    json!({}),
                    json!({}),
                    false,
                )
            }
            other => {
                return Err(format!(
                    "unsupported mode {other:?}; use contracts, precision, hol, startup, or polling"
                ));
            }
        };
    let finished_at_ms = unix_ms()?;
    let qpc_source = if arguments.mode == "precision" {
        "host QueryPerformanceCounter"
    } else if arguments.mode == "hol" {
        "synthetic QPC at 1 MHz through ProductionDispatchTestHarness prepared dispatch"
    } else if arguments.mode == "startup" {
        "synthetic QPC at 1 MHz through production coordinator/prepared-stream construction"
    } else if arguments.mode == "polling" {
        "integer rational timebase; no QPC or OS/game receiver sampling"
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
        "measurement_boundary": "Precision sample-start QPC is sampled immediately before prepared-dispatch entry; the authoritative sender pre-call is sampled before the mock emitter; mock completion is sampled inside the emitter and its separate delta includes test-seam/QPC overhead. P4 startup preparation timings and allocation counters are scoped to the named test-support operations; polling is an offline model.",
        "receipt_boundary": "deterministic mock transport for prepared dispatch; modeled polling only for receiver sampling; no game/audio receipt claim"
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
    let mut generation_count = 100usize;
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
            "--generation-count" => generation_count = parse_usize(&value, "generation-count")?,
            _ => return Err(format!("unknown argument {flag}")),
        }
    }
    if warmup > 100_000 || measured == 0 || measured > 100_000 || run_index == 0 {
        return Err("warmup, measured, or run-index is outside its supported range".into());
    }
    let mode = mode.ok_or_else(|| "--mode is required".to_string())?;
    if mode != "contracts"
        && mode != "precision"
        && mode != "hol"
        && mode != "startup"
        && mode != "polling"
    {
        return Err("--mode must be contracts, precision, hol, startup, or polling".into());
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
        generation_count,
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
    let malformed_duplicate_up_control = validate_duplicate_up_tail();
    let mut vector_a = Vec::new();
    for (frequency_hz, odd_vector) in [(1_000_000, false), (10_000_000, false), (10_000_003, true)]
    {
        vector_a.push(run_lifecycle_vector(frequency_hz, odd_vector)?);
    }
    let final_lease_age_proof = run_final_lease_age_proof_contract()?;
    let lifecycle_counterexample_reproduced = vector_a.len() == 3
        && vector_a
            .iter()
            .all(|case| case["normal_gap_contract_passed"].as_bool() == Some(false));
    let lifecycle_characterization_integrity_passed = vector_a.len() == 3
        && vector_a
            .iter()
            .all(|case| case["due_seam_integrity_passed"].as_bool() == Some(true));
    Ok(json!({
        "A_same_key_physical_floor_pause_resume": {
            "hypothesis_status": if lifecycle_counterexample_reproduced { "REPRODUCED" } else { "NO-REPRODUCTION" },
            "characterization_integrity_passed": lifecycle_characterization_integrity_passed,
            "normal_gap_contract_passed": vector_a.iter().all(|case| case["normal_gap_contract_passed"].as_bool() == Some(true)),
            "vector_cases": vector_a,
            "malformed_duplicate_up_control": malformed_duplicate_up_control,
            "path_evidence": "test-support due seam recomputes the current production physical timing window for the current prepared packet on every call, accepts wall QPC only, and enters the unchanged prepared suffix only when due; manual pause/resume uses the dispatch-loop verified resumable cleanup transition",
            "oracle": "no sender, cursor advance, or generation commit before the current packet_not_before_qpc; resumed Down must satisfy musical Up completion plus one frame and the absolute post-cleanup global Down barrier"
        },
        "F_final_lease_age_proof": final_lease_age_proof,
        "C_serial_HOL": run_hol_vector()?,
        "D_focus_loss_policy": run_focus_vectors()?,
        "E_empty_cleanup": run_empty_cleanup()?,
        "scope": "D4 lifecycle release-barrier qualification plus retained lease, HOL, focus, and empty-cleanup characterizations; no H/G, scheduler, or cleanup transport policy changed"
    }))
}

fn run_final_lease_age_proof_contract() -> Result<Value, String> {
    let timeout_ticks = duration_ticks(
        QpcClock::initialize().map_err(|error| format!("QPC: {error:?}"))?,
        3_000_000,
    )?;
    let overdue = run_final_lease_age_case("overdue_unlatched", 1)?;
    let unproven = run_final_lease_age_case("missing_progress_unproven", -2)?;
    let fresh = run_final_lease_age_case("fresh_progress", -1)?;
    let overdue_passed = overdue["dispatch_step"]
        == "TerminateStatic(\"supervisor_lease_expired\")"
        && overdue["elapsed_ticks"].as_u64() > overdue["timeout_ticks"].as_u64()
        && overdue["sender_attempts"].as_u64() == Some(0)
        && overdue["expired_latched_after"].as_bool() == Some(true)
        && overdue["cursor_unchanged"].as_bool() == Some(true)
        && overdue["generation_accounting_unchanged"].as_bool() == Some(true);
    let unproven_passed = unproven["dispatch_step"]
        == "TerminateStatic(\"supervisor_lease_unproven\")"
        && unproven["sender_attempts"].as_u64() == Some(0)
        && unproven["expired_latched_after"].as_bool() == Some(false)
        && unproven["cursor_unchanged"].as_bool() == Some(true)
        && unproven["generation_accounting_unchanged"].as_bool() == Some(true);
    let fresh_passed = fresh["dispatch_step"] == "Dispatched"
        && fresh["sender_attempts"].as_u64() == Some(1)
        && fresh["cursor_advanced_once"].as_bool() == Some(true);
    Ok(json!({
        "status": if overdue_passed && unproven_passed && fresh_passed { "PASS" } else { "FAIL" },
        "timeout_ticks": timeout_ticks.as_u64(),
        "overdue_unlatched": overdue,
        "missing_progress_unproven": unproven,
        "fresh_progress": fresh,
        "overdue_passed": overdue_passed,
        "unproven_passed": unproven_passed,
        "fresh_passed": fresh_passed,
        "oracle": "final eligible Down proves lease age from a fresh QPC sample; Expired and Unproven terminate before sender without cursor or generation advance; fresh progress remains eligible; no watchdog is started"
    }))
}

fn run_final_lease_age_case(name: &str, age_us: i64) -> Result<Value, String> {
    let clock = QpcClock::initialize().map_err(|error| format!("QPC: {error:?}"))?;
    let timeout = duration_ticks(clock, 3_000_000)?.as_u64();
    let mut harness =
        ProductionDispatchTestHarness::new_r3_prepared_down_or_uponly_for_test(1, 20_000)?;
    let packets = harness.configure_r3_mock_packet_sender_for_test(&[0]);
    harness.prepare_prepared_stream_for_test();
    harness.set_playback_epoch_qpc_for_test(QpcTicks::ZERO);
    harness.set_supervisor_lease_timeout_for_r3_probe(3_000_000)?;
    let sample = harness.qpc_now_for_test()?;
    let progress = if age_us == -1 {
        sample
    } else if age_us == -2 {
        QpcTicks::ZERO
    } else {
        let age = duration_ticks(clock, age_us as u64)?.as_u64();
        QpcTicks::from_raw(
            sample
                .as_u64()
                .checked_sub(timeout.saturating_add(age))
                .ok_or_else(|| format!("{name} QPC underflow"))?,
        )
    };
    harness.set_supervisor_heartbeat_for_test(progress);
    harness.set_r3_mock_sender_start_qpc_for_test(sample);
    let elapsed_ticks = sample
        .checked_duration_since(progress)
        .map_err(|error| format!("{name} fixture age order: {error:?}"))?;
    let cursor_before = harness.prepared_cursor_for_test();
    let accounting_before =
        generation_accounting_signature(harness.generation_accounting_for_test());
    let step =
        harness.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(sample);
    let sender_attempts = packets
        .lock()
        .map_err(|_| "lease-age packet capture lock poisoned".to_string())?
        .iter()
        .map(|(_, evidence)| u64::from(evidence.attempts))
        .sum::<u64>();
    let cursor_after = harness.prepared_cursor_for_test();
    let accounting_after =
        generation_accounting_signature(harness.generation_accounting_for_test());
    Ok(json!({
        "case": name,
        "progress_qpc_ticks": progress.as_u64(),
        "sample_qpc_ticks": sample.as_u64(),
        "elapsed_ticks": elapsed_ticks.as_u64(),
        "timeout_ticks": timeout,
        "dispatch_step": format!("{step:?}"),
        "sender_attempts": sender_attempts,
        "expired_latched_after": harness.supervisor_lease_expired_for_r3_probe(),
        "cursor_before": cursor_before,
        "cursor_after": cursor_after,
        "cursor_unchanged": cursor_after == cursor_before,
        "cursor_advanced_once": cursor_after == cursor_before + 1,
        "generation_accounting_before": accounting_before,
        "generation_accounting_after": accounting_after,
        "generation_accounting_unchanged": accounting_after == accounting_before
    }))
}

fn generation_accounting_signature(
    accounting: sky_dispatch_core::coordinator::GenerationAccounting,
) -> Value {
    json!({
        "total": accounting.total,
        "activated": accounting.activated,
        "scheduled": accounting.scheduled,
        "active": accounting.active,
        "released": accounting.released,
        "dropped_conflict": accounting.dropped_conflict,
        "dropped_backend": accounting.dropped_backend,
        "dropped_expired": accounting.dropped_expired,
        "cancelled": accounting.cancelled
    })
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
            json!({"accepted":false,"status":"REJECTED_BY_SCHEDULE_COMPILER","error":format!("{error:?}"),"disposition":"Separate malformed-input control only; the lifecycle baseline uses the valid four-event D0,U17167,D34334,U51501 sequence."})
        }
    }
}

struct LifecyclePrefix {
    harness: ProductionDispatchTestHarness,
    packets: Arc<Mutex<Vec<(PhysicalPacket, SendEvidence)>>>,
    first_step: String,
    up_step: String,
    first_observation: bool,
    up_observation: bool,
    down_completion: QpcTicks,
    up_completion: QpcTicks,
}

fn run_lifecycle_prefix(frequency_hz: u64, odd_vector: bool) -> Result<LifecyclePrefix, String> {
    let frequency = NonZeroU64::new(frequency_hz).ok_or("zero QPC frequency")?;
    let clock = QpcClock::from_frequency_hz(frequency);
    let mut harness = ProductionDispatchTestHarness::new_r3_lifecycle_sequence_for_test(
        frequency_hz,
        odd_vector,
    )?;
    let packets = harness.configure_r3_mock_packet_sender_for_test(&[0, 50, 0, 0]);
    harness.prepare_prepared_stream_for_test();
    let epoch = ticks_at_us(clock, 82_700)?;
    harness.reanchor_playback_clock_for_r3_probe(epoch)?;

    let down_at = ticks_at_us(clock, 100_000)?;
    let down_floor = harness.prepared_physical_floor_for_r3_probe()?;
    harness.set_r3_mock_sender_start_qpc_for_test(down_at);
    let first_step = harness
        .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_r3_probe(down_at)?
        .ok_or("first lifecycle Down was unexpectedly before its physical floor")?;
    let first_observation = harness.pop_r3_send_observation_for_test().is_some();
    let down_completion = evidence_completion(&packets, 0)?;

    let up_floor = harness.prepared_physical_floor_for_r3_probe()?;
    harness.set_r3_mock_sender_start_qpc_for_test(up_floor.packet_not_before_qpc);
    let up_step = harness
        .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_r3_probe(
            up_floor.packet_not_before_qpc,
        )?
        .ok_or("musical Up was unexpectedly before its physical floor")?;
    let up_observation = harness.pop_r3_send_observation_for_test().is_some();
    let up_completion = evidence_completion(&packets, 1)?;

    if down_floor.packet_not_before_qpc > down_at {
        return Err("initial lifecycle Down did not reach its due floor".to_string());
    }
    Ok(LifecyclePrefix {
        harness,
        packets,
        first_step: format!("{first_step:?}"),
        up_step: format!("{up_step:?}"),
        first_observation,
        up_observation,
        down_completion,
        up_completion,
    })
}

fn generation_accounting_json(harness: &ProductionDispatchTestHarness) -> Value {
    let accounting = harness.generation_accounting_for_test();
    json!({
        "activated": accounting.activated,
        "released": accounting.released,
        "cancelled": accounting.cancelled
    })
}

fn captured_packet_count(
    packets: &Arc<Mutex<Vec<(PhysicalPacket, SendEvidence)>>>,
) -> Result<usize, String> {
    packets
        .lock()
        .map(|capture| capture.len())
        .map_err(|_| "P0 packet capture lock poisoned".to_string())
}

fn run_lifecycle_vector(frequency_hz: u64, odd_vector: bool) -> Result<Value, String> {
    let frequency = NonZeroU64::new(frequency_hz).ok_or("zero QPC frequency")?;
    let clock = QpcClock::from_frequency_hz(frequency);
    let authored_offsets = if odd_vector {
        vec![0, 17_168, 34_336, 51_504]
    } else {
        vec![0, 17_167, 34_334, 51_501]
    };
    let frame_ticks = duration_ticks(clock, FRAME_US)?;

    // The negative control observes the same prepared next Down without a
    // pause transition and must hold at 118,300 us until its physical floor.
    let mut no_pause = run_lifecycle_prefix(frequency_hz, odd_vector)?;
    let no_pause_floor = no_pause.harness.prepared_physical_floor_for_r3_probe()?;
    let no_pause_at = ticks_at_us(clock, 118_300)?;
    let no_pause_cursor_before = no_pause.harness.prepared_cursor_for_test();
    let no_pause_attempts_before = captured_packet_count(&no_pause.packets)?;
    let no_pause_accounting_before = generation_accounting_json(&no_pause.harness);
    let no_pause_obligation_before = no_pause.harness.release_obligation_mask_for_test();
    no_pause
        .harness
        .set_r3_mock_sender_start_qpc_for_test(no_pause_at);
    let no_pause_step = no_pause
        .harness
        .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_r3_probe(no_pause_at)?;
    let no_pause_attempts_after = captured_packet_count(&no_pause.packets)?;
    let no_pause_cursor_after = no_pause.harness.prepared_cursor_for_test();
    let no_pause_accounting_after = generation_accounting_json(&no_pause.harness);
    let no_pause_obligation_after = no_pause.harness.release_obligation_mask_for_test();
    let no_pause_completion_floor = no_pause
        .up_completion
        .checked_add_duration(frame_ticks)
        .map_err(|error| format!("no-pause release floor overflow: {error}"))?;

    // Repeat the prefix and perform the real manual pause/resume transition.
    let mut lifecycle = run_lifecycle_prefix(frequency_hz, odd_vector)?;
    let before_cleanup_floor = lifecycle.harness.prepared_physical_floor_for_r3_probe()?;
    let before_cleanup = json!({
        "floor": tick_json(clock, before_cleanup_floor.packet_not_before_qpc)?,
        "cursor": lifecycle.harness.prepared_cursor_for_test(),
        "sender_attempts": captured_packet_count(&lifecycle.packets)?,
        "generation_accounting": generation_accounting_json(&lifecycle.harness),
        "release_obligation_mask": lifecycle.harness.release_obligation_mask_for_test()
    });
    let pause_at = ticks_at_us(clock, 117_300)?;
    let resume_at = ticks_at_us(clock, 118_300)?;
    lifecycle
        .harness
        .manual_pause_resume_for_r3_probe(pause_at, resume_at)?;
    let after_cleanup_floor = lifecycle.harness.prepared_physical_floor_for_r3_probe()?;
    let after_cleanup_attempts = captured_packet_count(&lifecycle.packets)?;
    let after_cleanup_cursor = lifecycle.harness.prepared_cursor_for_test();
    let after_cleanup_accounting = generation_accounting_json(&lifecycle.harness);
    let after_cleanup_obligation = lifecycle.harness.release_obligation_mask_for_test();
    let after_cleanup = json!({
        "floor": tick_json(clock, after_cleanup_floor.packet_not_before_qpc)?,
        "cursor": after_cleanup_cursor,
        "sender_attempts": after_cleanup_attempts,
        "generation_accounting": after_cleanup_accounting,
        "release_obligation_mask": after_cleanup_obligation
    });

    // The pre-cleanup window is retained only as a snapshot. The contract seam
    // recomputes current production authority after resume and accepts wall_now
    // alone; if resume is early, advance to that current floor and retry.
    let cursor_before_next_down = lifecycle.harness.prepared_cursor_for_test();
    let attempts_before_next_down = captured_packet_count(&lifecycle.packets)?;
    let accounting_before_next_down = generation_accounting_json(&lifecycle.harness);
    let obligation_before_next_down = lifecycle.harness.release_obligation_mask_for_test();
    let resume_gate_was_due = resume_at >= after_cleanup_floor.packet_not_before_qpc;
    lifecycle
        .harness
        .set_r3_mock_sender_start_qpc_for_test(resume_at);
    let resume_step = lifecycle
        .harness
        .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_r3_probe(resume_at)?;
    let resume_step_label = format!("{resume_step:?}");
    let cursor_after_resume_gate = lifecycle.harness.prepared_cursor_for_test();
    let attempts_after_resume_gate = captured_packet_count(&lifecycle.packets)?;
    let accounting_after_resume_gate = generation_accounting_json(&lifecycle.harness);
    let obligation_after_resume_gate = lifecycle.harness.release_obligation_mask_for_test();
    let resume_gate_integrity_passed = if resume_gate_was_due {
        matches!(resume_step.as_ref(), Some(DispatchStep::Dispatched))
            && attempts_after_resume_gate == attempts_before_next_down + 1
            && cursor_after_resume_gate == cursor_before_next_down + 1
    } else {
        resume_step.is_none()
            && attempts_after_resume_gate == attempts_before_next_down
            && cursor_after_resume_gate == cursor_before_next_down
            && accounting_after_resume_gate == accounting_before_next_down
            && obligation_after_resume_gate == obligation_before_next_down
    };
    if !resume_gate_integrity_passed {
        return Err(
            "resume due-seam result did not match the current production physical floor"
                .to_string(),
        );
    }
    let (next_down_step, next_down_pre_call) = match resume_step {
        Some(DispatchStep::Dispatched) => (DispatchStep::Dispatched, resume_at),
        Some(step) => {
            return Err(format!(
                "resumed next Down returned unexpected step {step:?}"
            ));
        }
        None => {
            let current_floor = lifecycle.harness.prepared_physical_floor_for_r3_probe()?;
            let retry_at = if resume_at >= current_floor.packet_not_before_qpc {
                resume_at
            } else {
                current_floor.packet_not_before_qpc
            };
            lifecycle
                .harness
                .set_r3_mock_sender_start_qpc_for_test(retry_at);
            let step = lifecycle
                .harness
                .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_r3_probe(retry_at)?
                .ok_or(
                    "resumed next Down remained before its recomputed production physical floor",
                )?;
            if !matches!(step, DispatchStep::Dispatched) {
                return Err(format!(
                    "resumed next Down returned unexpected step {step:?}"
                ));
            }
            (step, retry_at)
        }
    };
    let next_down_observation = lifecycle
        .harness
        .pop_r3_send_observation_for_test()
        .is_some();
    let actual_next_down = evidence_start(&lifecycle.packets, 2)?;
    let next_down_completion = evidence_completion(&lifecycle.packets, 2)?;
    let required_down_floor = lifecycle
        .up_completion
        .checked_add_duration(frame_ticks)
        .map_err(|error| format!("release floor overflow: {error}"))?;

    let final_up_floor = lifecycle.harness.prepared_physical_floor_for_r3_probe()?;
    lifecycle
        .harness
        .set_r3_mock_sender_start_qpc_for_test(final_up_floor.packet_not_before_qpc);
    let final_up_step = lifecycle
        .harness
        .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_r3_probe(
            final_up_floor.packet_not_before_qpc,
        )?
        .ok_or("final lifecycle Up was unexpectedly before its physical floor")?;
    let final_up_observation = lifecycle
        .harness
        .pop_r3_send_observation_for_test()
        .is_some();
    let final_up_actual = evidence_start(&lifecycle.packets, 3)?;
    let final_up_completion = evidence_completion(&lifecycle.packets, 3)?;
    let final_accounting = generation_accounting_json(&lifecycle.harness);
    let final_obligation = lifecycle.harness.release_obligation_mask_for_test();
    let config = lifecycle.harness.r3_timing_config_for_test()?;

    let no_pause_not_due = no_pause_step.is_none()
        && no_pause_floor.packet_not_before_qpc == no_pause_completion_floor
        && no_pause_at < no_pause_floor.packet_not_before_qpc
        && no_pause_cursor_before == no_pause_cursor_after
        && no_pause_attempts_before == no_pause_attempts_after
        && no_pause_accounting_before == no_pause_accounting_after
        && no_pause_obligation_before == no_pause_obligation_after;
    let resumed_not_due = if resume_gate_was_due {
        None
    } else {
        Some(
            resume_step.is_none()
                && cursor_before_next_down == cursor_after_resume_gate
                && attempts_before_next_down == attempts_after_resume_gate
                && accounting_before_next_down == accounting_after_resume_gate
                && obligation_before_next_down == obligation_after_resume_gate
                && attempts_after_resume_gate == after_cleanup_attempts
                && cursor_after_resume_gate == after_cleanup_cursor,
        )
    };
    let normal_gap_contract_passed = actual_next_down >= required_down_floor;
    let due_seam_integrity_passed = no_pause_not_due
        && resume_gate_integrity_passed
        && actual_next_down >= after_cleanup_floor.packet_not_before_qpc
        && config.native_admission_passed
        && config.frame_us == FRAME_US
        && config.effective_hold_us == HOLD_US
        && config.min_release_gap_us == HOLD_US
        && actual_next_down == next_down_pre_call
        && final_up_actual == final_up_floor.packet_not_before_qpc
        && next_down_completion <= final_up_actual
        && final_accounting["activated"].as_u64() == Some(2)
        && final_accounting["released"].as_u64() == Some(2)
        && final_obligation == 0;

    Ok(json!({
        "qpc_frequency_hz": frequency_hz,
        "conversion": {
            "authored_offsets_us": authored_offsets,
            "authored_offset_ticks": authored_offsets.iter().map(|us| duration_ticks(clock,*us).map(|ticks| ticks.as_u64())).collect::<Result<Vec<_>,_>>()?,
            "effective_hold_h_us": config.effective_hold_us,
            "effective_hold_h_ticks": config.effective_hold_ticks,
            "same_key_gap_g_us": config.frame_us,
            "same_key_gap_g_ticks": config.frame_ticks,
            "timing_margin_us": config.timing_margin_us,
            "timing_margin_ticks": config.timing_margin_ticks,
            "min_release_gap_us": config.min_release_gap_us,
            "min_release_gap_ticks": config.min_release_gap_ticks,
            "native_admission": if config.native_admission_passed { "PASS" } else { "FAIL" },
            "odd_frequency_feasible_extra_us": if odd_vector { 1 } else { 0 }
        },
        "events": {
            "down": event_json(clock, evidence_start(&lifecycle.packets,0)?, lifecycle.down_completion)?,
            "musical_up": event_json(clock, evidence_start(&lifecycle.packets,1)?, lifecycle.up_completion)?,
            "pause_qpc": tick_json(clock,pause_at)?,
            "resume_qpc": tick_json(clock,resume_at)?,
            "redown": event_json(clock,actual_next_down,next_down_completion)?,
            "final_up": event_json(clock,final_up_actual,final_up_completion)?
        },
        "steps": {
            "first_down": lifecycle.first_step,
            "musical_up": lifecycle.up_step,
            "dispatch_at_resume": resume_step_label,
            "redown": format!("{next_down_step:?}"),
            "final_up": format!("{final_up_step:?}")
        },
        "prepared_observations_available": [lifecycle.first_observation,lifecycle.up_observation,next_down_observation,final_up_observation],
        "no_pause_negative_control": {
            "qpc": tick_json(clock,no_pause_at)?,
            "floor": tick_json(clock,no_pause_floor.packet_not_before_qpc)?,
            "floor_equals_up_completion_plus_frame": no_pause_floor.packet_not_before_qpc == no_pause_completion_floor,
            "dispatch_step": format!("{no_pause_step:?}"),
            "sender_attempts_before": no_pause_attempts_before,
            "sender_attempts_after": no_pause_attempts_after,
            "cursor_before": no_pause_cursor_before,
            "cursor_after": no_pause_cursor_after,
            "generation_accounting_before": no_pause_accounting_before,
            "generation_accounting_after": no_pause_accounting_after,
            "release_obligation_before": no_pause_obligation_before,
            "release_obligation_after": no_pause_obligation_after,
            "not_due_without_send_or_advance": no_pause_not_due
        },
        "lifecycle_snapshots": {
            "before_cleanup": before_cleanup,
            "after_cleanup": after_cleanup,
            "before_next_down": {
                "pre_cleanup_floor_snapshot_only": tick_json(clock,before_cleanup_floor.packet_not_before_qpc)?,
                "post_cleanup_recomputed_floor": tick_json(clock,after_cleanup_floor.packet_not_before_qpc)?,
                "resume_qpc": tick_json(clock,resume_at)?,
                "earliest_dispatch_qpc": tick_json(clock, if resume_at >= after_cleanup_floor.packet_not_before_qpc { resume_at } else { after_cleanup_floor.packet_not_before_qpc })?,
                "dispatch_step_at_resume": resume_step_label,
                "current_floor_was_due_at_resume": resume_gate_was_due,
                "cursor_before": cursor_before_next_down,
                "cursor_after_gate": cursor_after_resume_gate,
                "sender_attempts_before": attempts_before_next_down,
                "sender_attempts_after_gate": attempts_after_resume_gate,
                "generation_accounting_before": accounting_before_next_down,
                "generation_accounting_after_gate": accounting_after_resume_gate,
                "release_obligation_before": obligation_before_next_down,
                "release_obligation_after_gate": obligation_after_resume_gate,
                "not_due_without_send_or_advance": resumed_not_due
            }
        },
        "cleanup_transport_sends": after_cleanup_attempts.saturating_sub(2),
        "required_next_down_not_before": tick_json(clock,required_down_floor)?,
        "production_down_floor": tick_json(clock,after_cleanup_floor.packet_not_before_qpc)?,
        "normal_gap_contract_passed": normal_gap_contract_passed,
        "below_completion_plus_frame": actual_next_down < required_down_floor,
        "final_generation_accounting": final_accounting,
        "final_release_obligation_mask": final_obligation,
        "due_seam_integrity_passed": due_seam_integrity_passed,
        "disposition": if normal_gap_contract_passed { "NOT_REPRODUCED" } else { "REPRODUCED" }
    }))
}

fn run_hol_vector() -> Result<Value, String> {
    let frequency_hz = 1_000_000;
    let clock = QpcClock::from_frequency_hz(NonZeroU64::new(frequency_hz).expect("frequency"));
    let mut harness = ProductionDispatchTestHarness::new_r3_hol_sequence_for_test(frequency_hz)?;
    let timing_config = harness.r3_timing_config_for_test()?;
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
        "timing_config": {
            "frame_gap_g_us": timing_config.frame_us,
            "frame_gap_g_ticks": timing_config.frame_ticks,
            "frame_base_hold_us": timing_config.frame_base_hold_us,
            "timing_margin_us": timing_config.timing_margin_us,
            "timing_margin_ticks": timing_config.timing_margin_ticks,
            "effective_hold_h_us": timing_config.effective_hold_us,
            "effective_hold_h_ticks": timing_config.effective_hold_ticks,
            "min_release_gap_us": timing_config.min_release_gap_us,
            "min_release_gap_ticks": timing_config.min_release_gap_ticks,
            "native_admission": if timing_config.native_admission_passed { "PASS" } else { "FAIL" },
            "tick_domain_admission": if timing_config.native_admission_passed { "PASS" } else { "FAIL" }
        },
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

fn make_diagnostic_action(
    source_action_index: u32,
    kind: ActionKind,
    scheduled_us: u64,
    scan_codes: &[u16],
    reason: &'static str,
) -> KeyActionInput {
    KeyActionInput {
        source_action_index,
        kind,
        scheduled_us,
        scan_codes: scan_codes.iter().copied().collect(),
        reason: reason.into(),
    }
}

fn qpc_absolute_us(clock: QpcClock, qpc: QpcTicks) -> Result<u64, String> {
    clock
        .duration_to_us(DurationTicks::from_raw(qpc.as_u64()))
        .map_err(|error| format!("diagnostic QPC to microseconds conversion: {error:?}"))
}

fn run_hol_case(
    case_id: &str,
    scenario: &str,
    actions: Vec<KeyActionInput>,
    sender_delays_us: &[u64],
    frame_us: u64,
    frame_base_hold_us: u64,
    timing_margin_us: u64,
    seed: u64,
) -> Result<Value, String> {
    let frequency_hz = 1_000_000;
    let clock = QpcClock::from_frequency_hz(NonZeroU64::new(frequency_hz).expect("frequency"));
    let mut harness = ProductionDispatchTestHarness::new_r3_diagnostics_sequence_for_test(
        &actions,
        frequency_hz,
        frame_us,
        frame_base_hold_us,
        timing_margin_us,
    )?;
    let captured = harness.configure_r3_mock_packet_sender_for_test(sender_delays_us);
    harness.prepare_prepared_stream_for_test();
    harness.set_playback_epoch_qpc_for_test(QpcTicks::ZERO);

    let expected_authored_targets: Vec<u64> = actions
        .iter()
        .map(|action| action.scheduled_us)
        .fold(Vec::new(), |mut values, scheduled_us| {
            if values.last().copied() != Some(scheduled_us) {
                values.push(scheduled_us);
            }
            values
        });
    let expected_generations = actions
        .iter()
        .filter(|action| action.kind == ActionKind::Down)
        .map(|action| action.scan_codes.len() as u64)
        .sum::<u64>();
    let mut rows = Vec::new();
    let mut previous_completion: Option<QpcTicks> = None;
    let mut previous_packet_mask = 0u16;
    let mut active_mask = 0u16;
    let mut down_replay = false;
    let mut transport_attempts = 0u64;

    for packet_index in 0..expected_authored_targets.len() {
        let floor = harness.prepared_physical_floor_for_r3_probe()?;
        let authored_target_qpc = floor.authored_target_qpc;
        let production_plan = harness.plan_current_dispatch();
        let ready_floor_qpc = harness
            .physical_wait_target_for_test(&production_plan)?
            .ok_or_else(|| {
                format!("{case_id} packet {packet_index} has no production wait target")
            })?;
        let serialized_qpc = previous_completion.unwrap_or(QpcTicks::ZERO);
        let actual_pre_qpc = ready_floor_qpc.max(serialized_qpc);
        harness.set_r3_mock_sender_start_qpc_for_test(actual_pre_qpc);
        let step = harness
            .dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(
                actual_pre_qpc,
            );
        if !matches!(step, DispatchStep::Dispatched) {
            return Err(format!(
                "{case_id} packet {packet_index} was not dispatched at the production wait target: {step:?}"
            ));
        }
        let capture = captured
            .lock()
            .map_err(|_| "HOL packet capture lock poisoned".to_string())?;
        let (packet, evidence) = capture
            .get(packet_index)
            .copied()
            .ok_or_else(|| format!("{case_id} packet {packet_index} had no sender evidence"))?;
        drop(capture);

        let actual_pre_qpc = evidence
            .started_ticks
            .ok_or_else(|| format!("{case_id} packet {packet_index} has no pre-call QPC"))?;
        let actual_complete_qpc = evidence
            .completed_ticks
            .ok_or_else(|| format!("{case_id} packet {packet_index} has no completion QPC"))?;
        let actual_pre_us = qpc_absolute_us(clock, actual_pre_qpc)?;
        let actual_complete_us = qpc_absolute_us(clock, actual_complete_qpc)?;
        let authored_target_us = qpc_absolute_us(clock, authored_target_qpc)?;
        let ready_floor_us = qpc_absolute_us(clock, ready_floor_qpc)?;
        let sender_duration_us = clock
            .duration_to_us(
                actual_complete_qpc
                    .checked_duration_since(actual_pre_qpc)
                    .map_err(|error| format!("{case_id} sender duration order: {error:?}"))?,
            )
            .map_err(|error| format!("{case_id} sender duration conversion: {error:?}"))?;
        let production_wait_target_below_guard_floor =
            ready_floor_qpc < floor.packet_not_before_qpc;
        let physical_floor_violation = production_wait_target_below_guard_floor
            || actual_pre_qpc < floor.packet_not_before_qpc;
        let earlier_packet_completed_after_ready =
            previous_completion.is_some_and(|completion| completion > ready_floor_qpc);
        let physical_block_mask = floor.hold_floor_mask | floor.release_floor_mask;
        let (blocking_packet, blocking_keymask, blocking_reason) =
            if earlier_packet_completed_after_ready {
                (
                    packet_index.checked_sub(1),
                    previous_packet_mask,
                    "serial_sender_completion",
                )
            } else if physical_block_mask != 0 {
                let capture = captured
                    .lock()
                    .map_err(|_| "HOL packet capture lock poisoned".to_string())?;
                let blocker = (0..packet_index).rev().find(|&prior_index| {
                    capture.get(prior_index).is_some_and(|(prior_packet, _)| {
                        (prior_packet.up_mask | prior_packet.down_mask) & physical_block_mask != 0
                    })
                });
                drop(capture);
                (blocker, physical_block_mask, "production_physical_floor")
            } else {
                (None, 0, "authored_target")
            };
        let delay_due_to_earlier_packet_us = if blocking_packet.is_some() {
            actual_pre_us.saturating_sub(authored_target_us)
        } else {
            0
        };
        let serial_delay_after_ready_us = actual_pre_us.saturating_sub(ready_floor_us);
        let packet_mask = packet.up_mask | packet.down_mask;
        if packet.down_mask & active_mask != 0 {
            down_replay = true;
        }
        active_mask = (active_mask & !packet.up_mask) | packet.down_mask;
        let attempts = u64::from(evidence.attempts);
        transport_attempts = transport_attempts.saturating_add(attempts);
        rows.push(json!({
            "case_id": case_id,
            "scenario": scenario,
            "seed": seed,
            "packet_index": packet_index,
            "authored_target_us": authored_target_us,
            "ready_floor_us": ready_floor_us,
            "actual_pre_us": actual_pre_us,
            "actual_complete_us": actual_complete_us,
            "configured_sender_delay_us": sender_delays_us.get(packet_index).copied().unwrap_or(0),
            "actual_sender_duration_us": sender_duration_us,
            "blocking_packet": blocking_packet,
            "blocking_keymask": format!("0x{blocking_keymask:04x}"),
            "blocking_reason": blocking_reason,
            "delay_due_to_earlier_packet_us": delay_due_to_earlier_packet_us,
            "physical_floor_delay_us": ready_floor_us.saturating_sub(authored_target_us),
            "serial_delay_after_ready_us": serial_delay_after_ready_us,
            "physical_floor_hold_keymask": format!("0x{:04x}", floor.hold_floor_mask),
            "physical_floor_release_keymask": format!("0x{:04x}", floor.release_floor_mask),
            "production_wait_target_below_guard_floor": production_wait_target_below_guard_floor,
            "physical_floor_violation": physical_floor_violation,
            "packet_up_keymask": format!("0x{:04x}", packet.up_mask),
            "packet_down_keymask": format!("0x{:04x}", packet.down_mask),
            "packet_keymask": format!("0x{packet_mask:04x}"),
            "packet_kind": if packet.up_mask != 0 && packet.down_mask != 0 { "mixed" } else if packet.up_mask != 0 { "up_only" } else { "down_only" },
            "packet_event_count": packet.event_count(),
            "transport_attempts": attempts,
            "down_replay_after_packet": down_replay,
            "accounting": generation_accounting_json(&harness),
            "prepared_cursor_after": harness.prepared_cursor_for_test()
        }));
        previous_completion = Some(actual_complete_qpc);
        previous_packet_mask = packet_mask;
    }

    let accounting = harness.generation_accounting_for_test();
    let release_obligation_mask = harness.release_obligation_mask_for_test();
    let authored_targets_unchanged = rows.len() == expected_authored_targets.len()
        && rows
            .iter()
            .zip(expected_authored_targets.iter())
            .all(|(row, expected)| row["authored_target_us"].as_u64() == Some(*expected));
    let down_packets = rows
        .iter()
        .filter(|row| row["packet_down_keymask"] != "0x0000")
        .count();
    let floor_violations = rows
        .iter()
        .filter(|row| row["physical_floor_violation"] == true)
        .count();
    let single_attempt_per_packet = rows
        .iter()
        .all(|row| row["transport_attempts"].as_u64() == Some(1));
    let disposition = if floor_violations != 0 || down_replay {
        "BLOCKED-DEFECT"
    } else {
        "CHARACTERIZATION_ONLY"
    };
    let summary = json!({
        "case_id": case_id,
        "scenario": scenario,
        "status": disposition,
        "seed": seed,
        "qpc_frequency_hz": frequency_hz,
        "frame_us": frame_us,
        "frame_base_hold_us": frame_base_hold_us,
        "timing_margin_us": timing_margin_us,
        "effective_hold_h_us": frame_base_hold_us.saturating_add(timing_margin_us),
        "authored_offsets_unchanged": authored_targets_unchanged,
        "physical_floor_violation_count": floor_violations,
        "down_replay": down_replay,
        "expected_down_generations": expected_generations,
        "actual_down_packets": down_packets,
        "generation_accounting": generation_accounting_json(&harness),
        "activated_generations": accounting.activated,
        "released_generations": accounting.released,
        "active_generations": accounting.active,
        "release_obligation_mask": release_obligation_mask,
        "total_transport_attempts": transport_attempts,
        "single_attempt_per_packet": single_attempt_per_packet,
        "configured_sender_delays_us": sender_delays_us,
        "rows": rows
    });
    if disposition == "BLOCKED-DEFECT" {
        return Err(format!(
            "BLOCKED-DEFECT: {case_id} observed {floor_violations} physical floor violation(s), down_replay={down_replay}"
        ));
    }
    if !authored_targets_unchanged
        || accounting.activated != expected_generations
        || accounting.active != 0
        || release_obligation_mask != 0
        || !single_attempt_per_packet
    {
        return Err(format!(
            "{case_id} failed a characterization accounting/oracle check: {summary}"
        ));
    }
    Ok(summary)
}

fn run_hol_diagnostics() -> Result<Value, String> {
    let a = 0x15;
    let b = 0x16;
    let c = 0x17;
    let d = 0x18;
    let base_cases = [
        (
            "hol-a-up-floor-delays-b",
            "D100000 completes110000; A Up authored117167 and is physically floored to127167; different-key B Down authored118000 follows serially.",
            vec![
                make_diagnostic_action(0, ActionKind::Down, 100_000, &[a], "p4-hol-a-down"),
                make_diagnostic_action(1, ActionKind::Up, 117_167, &[a], "p4-hol-a-up"),
                make_diagnostic_action(2, ActionKind::Down, 118_000, &[b], "p4-hol-b-down"),
                make_diagnostic_action(3, ActionKind::Up, 135_167, &[b], "p4-hol-b-up"),
            ],
            vec![10_000, 400, 200, 100],
            16_667,
            16_667,
            500,
        ),
        (
            "hol-mixed-up-a-down-b-one-syscall",
            "Up A and Down B share one authored timestamp; the maximum per-key physical floor delays the atomic mixed packet.",
            vec![
                make_diagnostic_action(0, ActionKind::Down, 100_000, &[a], "p4-mixed-a-down"),
                make_diagnostic_action(1, ActionKind::Up, 118_000, &[a], "p4-mixed-a-up"),
                make_diagnostic_action(2, ActionKind::Down, 118_000, &[b], "p4-mixed-b-down"),
                make_diagnostic_action(3, ActionKind::Up, 135_167, &[b], "p4-mixed-b-up"),
            ],
            vec![10_000, 600, 100],
            16_667,
            16_667,
            500,
        ),
        (
            "hol-overdue-down-burst-after-blocked-up",
            "Three unrelated Down packets become overdue behind one floor-blocked Up; output spacing is the configured sender duration, while authored targets remain fixed.",
            vec![
                make_diagnostic_action(0, ActionKind::Down, 100_000, &[a], "p4-burst-a-down"),
                make_diagnostic_action(1, ActionKind::Up, 117_167, &[a], "p4-burst-a-up"),
                make_diagnostic_action(2, ActionKind::Down, 118_000, &[b], "p4-burst-b-down"),
                make_diagnostic_action(3, ActionKind::Down, 119_000, &[c], "p4-burst-c-down"),
                make_diagnostic_action(4, ActionKind::Down, 120_000, &[d], "p4-burst-d-down"),
                make_diagnostic_action(5, ActionKind::Up, 135_167, &[b], "p4-burst-b-up"),
                make_diagnostic_action(6, ActionKind::Up, 136_167, &[c], "p4-burst-c-up"),
                make_diagnostic_action(7, ActionKind::Up, 137_167, &[d], "p4-burst-d-up"),
            ],
            vec![10_000, 3_000, 80, 90, 100, 100, 100, 100],
            16_667,
            16_667,
            500,
        ),
        (
            "hol-same-key-repeated-note-serialization",
            "The next same-key Down follows A Up and is checked against the production physical same-key frame floor and musical hold floor.",
            vec![
                make_diagnostic_action(0, ActionKind::Down, 100_000, &[a], "p4-repeat-a-down-1"),
                make_diagnostic_action(1, ActionKind::Up, 117_167, &[a], "p4-repeat-a-up-1"),
                make_diagnostic_action(2, ActionKind::Down, 135_000, &[a], "p4-repeat-a-down-2"),
                make_diagnostic_action(3, ActionKind::Up, 152_167, &[a], "p4-repeat-a-up-2"),
            ],
            vec![10_000, 50, 50, 50],
            16_667,
            16_667,
            500,
        ),
    ];
    let base_case_count = base_cases.len();
    let mut cases = Vec::new();
    let mut rows = Vec::new();
    for (case_id, scenario, actions, delays, frame, base_hold, margin) in base_cases {
        let result = run_hol_case(
            case_id,
            scenario,
            actions,
            &delays,
            frame,
            base_hold,
            margin,
            P4_STARTUP_SEED,
        )?;
        rows.extend(result["rows"].as_array().cloned().unwrap_or_default());
        cases.push(result);
    }

    let mut chord_case_count = 0usize;
    for fps in [30u64, 60, 120, 240] {
        let frame_us = 1_000_000u64.div_ceil(fps);
        for margin_us in [0u64, 500, 3_000] {
            for (hold_label, numerator, denominator) in
                [("1", 1u64, 1u64), ("1.25", 5, 4), ("1.5", 3, 2)]
            {
                let frame_base_hold_us = (frame_us * numerator).div_ceil(denominator);
                let effective_hold_us = frame_base_hold_us
                    .checked_add(margin_us)
                    .ok_or_else(|| "15-key chord effective hold overflow".to_string())?;
                let chord = PHYSICAL_INSTRUMENT_SCAN_CODES;
                let case_id = format!("hol-chord15-fps{fps}-margin{margin_us}-hold{hold_label}");
                let actions = vec![
                    make_diagnostic_action(0, ActionKind::Down, 100_000, &chord, "p4-chord15-down"),
                    make_diagnostic_action(
                        1,
                        ActionKind::Up,
                        100_000 + effective_hold_us,
                        &chord,
                        "p4-chord15-up",
                    ),
                ];
                let result = run_hol_case(
                    &case_id,
                    "Maximum 15-key chord with fixed frame rate, margin, and authored hold-frame multiplier.",
                    actions,
                    &[150, 150],
                    frame_us,
                    frame_base_hold_us,
                    margin_us,
                    P4_STARTUP_SEED,
                )?;
                rows.extend(result["rows"].as_array().cloned().unwrap_or_default());
                cases.push(result);
                chord_case_count += 1;
            }
        }
    }
    let packet_count = rows.len();
    Ok(json!({
        "status":"CHARACTERIZATION_ONLY",
        "scenario_id":"r3-p4-hol-fixed-v1",
        "dataset_seed":P4_STARTUP_SEED,
        "dispatch_path":"ProductionDispatchTestHarness -> prepared stream -> dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_r3_probe -> existing prepared dispatch/coordinator admission",
        "scheduler_duplicated":false,
        "sender":"deterministic mock with per-packet configured delays; no SendInput call",
        "base_scenario_count":base_case_count,
        "maximum_chord_case_count":chord_case_count,
        "physical_packet_row_count":packet_count,
        "physical_floor_violation_count":rows.iter().filter(|row| row["physical_floor_violation"] == true).count(),
        "down_replay_case_count":cases.iter().filter(|case| case["down_replay"] == true).count(),
        "known_disposition":"Head-of-line delay and overdue bursts are characterized limitations. No schedule reorder, catch-up, packet splitting, replay, or scheduler redesign is proposed.",
        "cases":cases,
        "rows":rows
    }))
}

fn measure_allocations_and_time<T>(operation: impl FnOnce() -> T) -> (T, u64, u64, u64) {
    TRACK_ALLOCATIONS.store(false, Ordering::SeqCst);
    TRACKED_ALLOCATION_CALLS.store(0, Ordering::SeqCst);
    TRACKED_ALLOCATION_BYTES.store(0, Ordering::SeqCst);
    TRACK_ALLOCATIONS.store(true, Ordering::SeqCst);
    let started = Instant::now();
    let value = operation();
    let elapsed_us = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
    TRACK_ALLOCATIONS.store(false, Ordering::SeqCst);
    let allocation_calls = TRACKED_ALLOCATION_CALLS.load(Ordering::SeqCst);
    let allocated_bytes = TRACKED_ALLOCATION_BYTES.load(Ordering::SeqCst);
    (value, elapsed_us, allocation_calls, allocated_bytes)
}

fn run_startup_case(generation_count: usize, seed: u64) -> Result<Value, String> {
    if !P4_DIAGNOSTIC_GENERATION_COUNTS.contains(&generation_count) {
        return Err(format!(
            "unsupported startup generation_count {generation_count}; fixed dataset is {P4_DIAGNOSTIC_GENERATION_COUNTS:?}"
        ));
    }
    if generation_count.saturating_mul(2) > sky_dispatch_core::compile::MAX_ACTIONS {
        return Err("startup dataset exceeds the existing authored action limit".into());
    }
    let mut actions = Vec::with_capacity(generation_count * 2);
    let first_slot = (seed % PHYSICAL_INSTRUMENT_SCAN_CODES.len() as u64) as usize;
    for generation in 0..generation_count {
        let slot = (first_slot + generation * 7) % PHYSICAL_INSTRUMENT_SCAN_CODES.len();
        let scan_code = PHYSICAL_INSTRUMENT_SCAN_CODES[slot];
        let down_us = (generation as u64)
            .checked_mul(P4_STARTUP_HOLD_US.saturating_mul(2))
            .ok_or_else(|| "startup fixture timestamp overflow".to_string())?;
        let up_us = down_us
            .checked_add(P4_STARTUP_HOLD_US)
            .ok_or_else(|| "startup fixture Up timestamp overflow".to_string())?;
        let down_index = u32::try_from(generation.saturating_mul(2))
            .map_err(|_| "startup source action index overflow".to_string())?;
        let up_index = down_index
            .checked_add(1)
            .ok_or_else(|| "startup Up source action index overflow".to_string())?;
        actions.push(make_diagnostic_action(
            down_index,
            ActionKind::Down,
            down_us,
            &[scan_code],
            "r3-p4-startup-down",
        ));
        actions.push(make_diagnostic_action(
            up_index,
            ActionKind::Up,
            up_us,
            &[scan_code],
            "r3-p4-startup-up",
        ));
    }

    let (compiled_result, compile_wall_us, compile_allocations, compile_allocated_bytes) =
        measure_allocations_and_time(|| {
            compile_runtime_intents(&actions, &PHYSICAL_INSTRUMENT_SCAN_CODES)
        });
    let schedule: RuntimeSchedule = compiled_result
        .map_err(|error| format!("fixed startup schedule compilation failed: {error:?}"))?;
    let compiled_counts = json!({
        "action_count":actions.len(),
        "generation_count":schedule.generation_count,
        "packet_count":schedule.packets.len(),
        "batch_count":schedule.batches.len(),
        "intent_count":schedule.intents.len(),
        "packet_type_size_bytes":std::mem::size_of::<CompiledPacket>(),
        "batch_type_size_bytes":std::mem::size_of::<CompiledBatch>(),
        "intent_type_size_bytes":std::mem::size_of::<CompactIntent>(),
        "runtime_schedule_type_size_bytes":std::mem::size_of::<RuntimeSchedule>(),
        "schedule_vector_payload_bytes":schedule.packets.len() * std::mem::size_of::<CompiledPacket>()
            + schedule.batches.len() * std::mem::size_of::<CompiledBatch>()
            + schedule.intents.len() * std::mem::size_of::<CompactIntent>(),
        "reason_text_bytes":schedule.reason_table.iter().map(String::len).sum::<usize>(),
        "input_fixture_allocated_before_measurement":true
    });

    let (coordinator_schedule, clone_wall_us, clone_allocations, clone_allocated_bytes) =
        measure_allocations_and_time(|| schedule.clone());
    let (builder_result, coordinator_wall_us, coordinator_allocations, coordinator_allocated_bytes) =
        measure_allocations_and_time(|| {
            ProductionDispatchTestHarness::new_r3_prepared_startup_builder_for_test(
                coordinator_schedule,
                1_000_000,
                P4_STARTUP_HOLD_US,
            )
        });
    let builder = builder_result?;
    let (
        coordinator_clone,
        coordinator_clone_wall_us,
        coordinator_clone_allocations,
        coordinator_clone_allocated_bytes,
    ) = measure_allocations_and_time(|| builder.clone_coordinator_for_test());
    drop(coordinator_clone);
    let (prepared_result, prepared_build_wall_us, prepared_allocations, prepared_allocated_bytes) =
        measure_allocations_and_time(|| builder.prepare());
    let prepared = prepared_result?;
    let available_startup_budget_us = PRODUCTION_MIN_PREROLL_US
        .checked_sub(PRODUCTION_STARTUP_READINESS_RESERVE_US)
        .ok_or_else(|| {
            "fixed startup readiness reserve exceeds the minimum pre-roll".to_string()
        })?;
    let measured_worker_path_us = coordinator_wall_us.saturating_add(prepared_build_wall_us);
    let would_admit_prepared_path = measured_worker_path_us <= available_startup_budget_us;
    let prepared_path_admission_outcome = if would_admit_prepared_path {
        "WOULD_ADMIT_WITHIN_SCOPED_MIN_PREROLL_BUDGET"
    } else {
        "WOULD_REJECT_SCOPED_MIN_PREROLL_BUDGET"
    };

    Ok(json!({
        "status":"CHARACTERIZATION_ONLY",
        "dataset_id":"r3-p4-startup-synthetic-v1",
        "seed":seed,
        "generation_count":generation_count,
        "action_count":actions.len(),
        "authored_schedule":"one Down/Up generation per row; H=17167us; start=i*34334us; Up=start+17167us; one of 15 canonical scan codes selected by (seed%15 + i*7)%15",
        "below_existing_action_limit":actions.len() < sky_dispatch_core::compile::MAX_ACTIONS,
        "action_limit":sky_dispatch_core::compile::MAX_ACTIONS,
        "schedule_compile_and_materialize":{
            "wall_us":compile_wall_us,
            "allocation_calls":compile_allocations,
            "requested_allocation_bytes":compile_allocated_bytes,
            "storage":compiled_counts
        },
        "coordinator_input_schedule_clone":{
            "wall_us":clone_wall_us,
            "allocation_calls":clone_allocations,
            "requested_allocation_bytes":clone_allocated_bytes,
            "meaning":"RuntimeSchedule clone used to retain the compiled fixture while transferring its owned copy into coordinator preparation; RuntimeDispatchCoordinator itself does not implement Clone"
        },
        "coordinator_initialization":{
            "wall_us":coordinator_wall_us,
            "allocation_calls":coordinator_allocations,
            "requested_allocation_bytes":coordinator_allocated_bytes
        },
        "coordinator_clone_for_characterization":{
            "wall_us":coordinator_clone_wall_us,
            "allocation_calls":coordinator_clone_allocations,
            "requested_allocation_bytes":coordinator_clone_allocated_bytes,
            "production_boot_uses_this_clone":false,
            "meaning":"feature=test-support deep clone of the coordinator and owned RuntimeSchedule; measured separately from actual boot construction and not included in startup budget admission comparison"
        },
        "prepared_stream_build_and_coordinator_reset":{
            "wall_us":prepared_build_wall_us,
            "allocation_calls":prepared_allocations,
            "requested_allocation_bytes":prepared_allocated_bytes,
            "includes":"production PreparedDispatchStream::build simulation, frozen entry materialization, and the production coordinator reconstruction returned by that builder"
        },
        "prepared_storage":{
            "prepared_entry_count":prepared.prepared_entry_count,
            "physical_entry_count":prepared.physical_entry_count,
            "metadata_entry_count":prepared.metadata_entry_count,
            "prepared_entry_type_size_bytes":prepared.prepared_entry_type_size_bytes,
            "prepared_entry_array_bytes":prepared.prepared_entry_array_bytes,
            "cancellation_ledger_bytes":prepared.cancellation_ledger_bytes,
            "rebuilt_coordinator_heap_payload_bytes":prepared.rebuilt_coordinator_heap_payload_bytes
        },
        "startup_budget":{
            "minimum_production_preroll_us":PRODUCTION_MIN_PREROLL_US,
            "readiness_reserve_us":PRODUCTION_STARTUP_READINESS_RESERVE_US,
            "captured_available_startup_budget_us":available_startup_budget_us,
            "measured_worker_path_us":measured_worker_path_us,
            "would_admit_prepared_path":would_admit_prepared_path,
            "prepared_path_admission_outcome":prepared_path_admission_outcome,
            "production_startup_attempted":false,
            "production_startup_rejection_observed":false,
            "comparison_scope":"coordinator initialization plus prepared-stream build/reset only; input generation and schedule compilation/materialization are separate because the worker receives a compiled RuntimeSchedule; excludes thread launch, target preflight, telemetry and other boot work"
        },
        "memory_boundary":{
            "allocation_counter":"probe-process global allocator counts successful alloc, alloc_zeroed, and realloc requested bytes only during the named operation; deallocation is not subtracted; it is not RSS",
            "structural_storage":"reported vector/entry payload estimates and type sizes exclude allocator metadata, nested control blocks and runtime/library memory",
            "process_rss":"captured separately by the PowerShell runner as process working-set samples"
        }
    }))
}

fn simulate_polling_case(
    fps: u64,
    margin_us: u64,
    hold_us_override: Option<u64>,
    phase_units: u64,
    phase_label: &str,
    stall_frames: u64,
    down_delivery_skew_us: u64,
    up_delivery_skew_us: u64,
    supplemental: bool,
) -> Value {
    // One frame is exactly 1,000,000 units where one unit is 1/fps microsecond.
    let frame_units = 1_000_000u64;
    let hold_units = hold_us_override
        .map(|hold_us| hold_us.saturating_mul(fps))
        .unwrap_or_else(|| frame_units.saturating_add(margin_us.saturating_mul(fps)));
    let down_delivery_units = down_delivery_skew_us.saturating_mul(fps);
    let up_delivery_units = hold_units.saturating_add(up_delivery_skew_us.saturating_mul(fps));
    let stall_until_units = stall_frames.saturating_mul(frame_units);
    let last_sample_units = up_delivery_units
        .max(stall_until_units)
        .saturating_add(frame_units.saturating_mul(2));
    let mut sample_units = phase_units;
    let mut poll_count = 0u32;
    let mut skipped_poll_count = 0u32;
    let mut down_state_observed = false;
    let mut separating_up_state_observed = false;
    while sample_units <= last_sample_units {
        if sample_units < stall_until_units {
            skipped_poll_count += 1;
        } else {
            poll_count += 1;
            // Fixed tie ordering: apply all deliveries at this timestamp before
            // sampling the receiver's current key state. Thus Down at a poll
            // boundary is visible; Up at the same boundary is already released.
            if sample_units >= down_delivery_units && sample_units < up_delivery_units {
                down_state_observed = true;
            } else if sample_units >= up_delivery_units {
                separating_up_state_observed = true;
            }
        }
        sample_units = sample_units.saturating_add(frame_units);
    }
    let hold_duration_units = up_delivery_units.saturating_sub(down_delivery_units);
    let hold_duration_whole_us = hold_duration_units / fps;
    let hold_duration_remainder = hold_duration_units % fps;
    json!({
        "case_id":format!("poll-fps{fps}-margin{margin_us}-phase{phase_label}-stall{stall_frames}-skew{down_delivery_skew_us}x{up_delivery_skew_us}"),
        "scenario":"frame_polling",
        "evidence_label":if supplemental { "MODELED_SHORT_HOLD_REFERENCE" } else { "MODELED_POLLING_ONLY" },
        "fps":fps,
        "frame_period_us_numerator":1_000_000,
        "frame_period_us_denominator":fps,
        "margin_us":margin_us,
        "hold_policy":"H=T+margin",
        "hold_us_override":hold_us_override,
        "hold_duration_us_floor":hold_duration_whole_us,
        "hold_duration_fraction_numerator":hold_duration_remainder,
        "hold_duration_fraction_denominator":fps,
        "poll_phase":phase_label,
        "poll_phase_timebase_units":phase_units,
        "receiver_stall_frames":stall_frames,
        "receiver_stall_us_floor":stall_until_units / fps,
        "down_delivery_skew_us":down_delivery_skew_us,
        "up_delivery_skew_us":up_delivery_skew_us,
        "poll_count":poll_count,
        "skipped_poll_count_during_stall":skipped_poll_count,
        "down_state_observed":down_state_observed,
        "separating_up_state_observed":separating_up_state_observed,
        "both_states_observed":down_state_observed && separating_up_state_observed,
        "ordering":"event deliveries at equal timestamps are applied before receiver sampling; stall interval is [0, stall_duration), so a poll exactly at stall end is eligible",
        "sky_consumption":"UNKNOWN"
    })
}

fn run_frame_polling_model() -> Result<Value, String> {
    let phases = [
        (0, "0"),
        (250_000, "T/4"),
        (500_000, "T/2"),
        (750_000, "3T/4"),
        (1_000_000, "T_exact_boundary"),
    ];
    let mut rows = Vec::new();
    for fps in [30u64, 60, 120, 240] {
        for margin_us in [0u64, 500, 3_000] {
            for (phase_units, phase_label) in phases {
                for stall_frames in [0u64, 1, 2] {
                    for delivery_skew_us in [0u64, 500, 2_000] {
                        for up_delivery_skew_us in [0u64, 500, 2_000] {
                            rows.push(simulate_polling_case(
                                fps,
                                margin_us,
                                None,
                                phase_units,
                                phase_label,
                                stall_frames,
                                delivery_skew_us,
                                up_delivery_skew_us,
                                false,
                            ));
                        }
                    }
                }
            }
        }
    }
    for (phase_units, phase_label) in [(0, "0"), (750_000, "3T/4"), (1_000_000, "T_exact_boundary")]
    {
        for stall_frames in [0, 1] {
            rows.push(simulate_polling_case(
                30,
                0,
                Some(HOLD_US),
                phase_units,
                phase_label,
                stall_frames,
                0,
                0,
                true,
            ));
        }
    }
    let expected_matrix_cases = 4 * 3 * 5 * 3 * 3 * 3;
    let short_hold_misses = rows
        .iter()
        .filter(|row| {
            row["evidence_label"] == "MODELED_SHORT_HOLD_REFERENCE"
                && row["down_state_observed"] == false
        })
        .count();
    if rows.len() != expected_matrix_cases + 6 || short_hold_misses == 0 {
        return Err(format!(
            "polling matrix has {} rows and {short_hold_misses} missed short-hold Downs; expected {} rows and visible misses",
            rows.len(),
            expected_matrix_cases + 6
        ));
    }
    Ok(json!({
        "status":"CHARACTERIZATION_ONLY",
        "scenario_id":"r3-p4-frame-polling-fixed-v1",
        "evidence_labels":{
            "modeled_polling":"This deterministic offline state-sampling simulation.",
            "actual_receive_only":"Separate published P1/P2 sink event evidence; not rerun in P4.",
            "sky_consumption":"UNKNOWN; not observed or inferred from DWM/monitor refresh rate."
        },
        "polling_model":"periodic state sampler at declared FPS, fixed phase and receiver stall; event delivery skew is independently applied to Down and Up",
        "matrix_case_count":expected_matrix_cases,
        "supplemental_17_167us_reference_case_count":6,
        "short_hold_missed_down_count":short_hold_misses,
        "assertion_boundary":"Missed Down/Up states are visible outcomes, not failures. No oracle assumes a 17167us hold is observed under 30FPS polling or a 30ms receiver stall.",
        "no_game_hook_or_input_injection":true,
        "rows":rows
    }))
}

#[cfg(test)]
mod p4_diagnostics_tests {
    use super::*;

    #[test]
    fn r3_p4_hol_rows_use_prepared_paths_and_attribute_sender_time() {
        let report = run_hol_diagnostics().expect("fixed HOL diagnostics");
        let cases = report["cases"].as_array().expect("case array");
        let first = cases
            .iter()
            .find(|case| case["case_id"] == "hol-a-up-floor-delays-b")
            .expect("first HOL case");
        assert_eq!(first["rows"][1]["ready_floor_us"], 127_167);
        assert_eq!(first["rows"][1]["actual_complete_us"], 127_567);
        assert_eq!(first["rows"][2]["actual_pre_us"], 127_567);
        assert_eq!(first["rows"][2]["blocking_packet"], 1);
        assert_eq!(first["rows"][2]["blocking_keymask"], "0x0001");
        assert_eq!(first["rows"][2]["delay_due_to_earlier_packet_us"], 9_567);

        let mixed = cases
            .iter()
            .find(|case| case["case_id"] == "hol-mixed-up-a-down-b-one-syscall")
            .expect("mixed HOL case");
        assert_eq!(mixed["rows"][1]["packet_kind"], "mixed");
        assert_eq!(mixed["rows"][1]["transport_attempts"], 1);
        assert_eq!(mixed["rows"][1]["packet_event_count"], 2);
        assert_eq!(report["physical_floor_violation_count"], 0);
        assert_eq!(report["down_replay_case_count"], 0);
    }

    #[test]
    fn r3_p4_short_hold_and_exact_boundary_can_miss_down_state() {
        let short = simulate_polling_case(30, 0, Some(HOLD_US), 750_000, "3T/4", 0, 0, 0, true);
        assert_eq!(short["down_state_observed"], false);
        assert_eq!(short["separating_up_state_observed"], true);

        let boundary =
            simulate_polling_case(30, 0, None, 1_000_000, "T_exact_boundary", 0, 0, 0, false);
        assert_eq!(boundary["down_state_observed"], false);
        assert_eq!(boundary["separating_up_state_observed"], true);
    }

    #[test]
    fn r3_p4_polling_matrix_keeps_missed_states_as_results() {
        let report = run_frame_polling_model().expect("fixed polling dataset");
        assert_eq!(report["matrix_case_count"], 1_620);
        assert_eq!(report["rows"].as_array().unwrap().len(), 1_626);
        assert_eq!(report["short_hold_missed_down_count"], 5);
        assert_eq!(report["rows"][0]["evidence_label"], "MODELED_POLLING_ONLY");
    }
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
    let mut sample_progress_publications = 0usize;
    let mut setup_progress_publications = 0usize;
    let mut first_sample_progress_qpc = None;
    let mut last_sample_progress_qpc = None;
    let mut config_evidence = None;
    let total = warmup_count.saturating_add(measured_count);
    let expected_setup_progress_publications = if matches!(
        workload.kind,
        WorkloadKind::Mixed(_, _) | WorkloadKind::UpOnly(_)
    ) {
        total
    } else {
        0
    };

    for sample_index in 0..total {
        let mut harness = make_precision_harness(workload)?;
        let _packets = harness.configure_r3_precision_mock_sender_for_test();
        harness.set_supervisor_lease_timeout_for_r3_probe(3_000_000)?;
        config_evidence.get_or_insert(harness.r3_timing_config_for_test()?);
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
            harness.set_supervisor_heartbeat_for_test(setup_at);
            setup_progress_publications += 1;
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
        let progress_qpc = harness.qpc_now_for_test()?;
        harness.set_supervisor_heartbeat_for_test(progress_qpc);
        sample_progress_publications += 1;
        first_sample_progress_qpc.get_or_insert(progress_qpc.as_u64());
        last_sample_progress_qpc = Some(progress_qpc.as_u64());
        let sample_start = harness.qpc_now_for_test()?;
        let step = harness.dispatch_prepared_current_at_synthetic_wall_qpc_for_test(sample_start);
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
        let start = sample_start;
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
    let config = config_evidence.ok_or("precision workload ran no samples")?;
    let lease_enabled = config.lease_enabled
        && config.lease_timeout_us == 3_000_000
        && config.lease_timeout_ticks > 0;
    let eligible = failed_samples == 0
        && admission_ticks.len() == measured_count
        && measured_sender_attempts == measured_count as u64
        && sample_progress_publications == total
        && lease_enabled
        && config.native_admission_passed
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
        "r3_timing_config": {
            "frame_gap_g_us":config.frame_us,
            "frame_gap_g_ticks":config.frame_ticks,
            "frame_base_hold_us":config.frame_base_hold_us,
            "timing_margin_us":config.timing_margin_us,
            "timing_margin_ticks":config.timing_margin_ticks,
            "effective_hold_h_us":config.effective_hold_us,
            "effective_hold_h_ticks":config.effective_hold_ticks,
            "min_release_gap_us":config.min_release_gap_us,
            "min_release_gap_ticks":config.min_release_gap_ticks,
            "native_admission":if config.native_admission_passed { "PASS" } else { "FAIL" },
            "tick_domain_admission":if config.native_admission_passed { "PASS" } else { "FAIL" }
        },
        "lease": {
            "enabled":config.lease_enabled,
            "timeout_us":config.lease_timeout_us,
            "timeout_ticks":config.lease_timeout_ticks,
            "progress_state":"fresh_progress_before_each_sample",
            "sample_progress_publication_count":sample_progress_publications,
            "expected_sample_progress_publications":total,
            "setup_progress_publication_count":setup_progress_publications,
            "setup_progress_published_before_setup_dispatch":setup_progress_publications == expected_setup_progress_publications,
            "first_sample_progress_qpc_ticks":first_sample_progress_qpc,
            "last_sample_progress_qpc_ticks":last_sample_progress_qpc,
            "watchdog_scheduler_started":false
        },
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
        "completion_model":"completion is sampled inside the deterministic mock emitter; its delta from the authoritative sender pre-call includes test-seam/QPC overhead and does not measure SendInput execution, device receipt, or game/audio receipt",
        "boundary_notes":"prepared-stream construction, optional setup dispatch, fresh lease progress publication, and sample-start setup occur outside each timed interval; sample-start QPC is sampled immediately before the measured prepared-dispatch entry; the backend samples authoritative pre-call before the mock emitter; completion is sampled inside the emitter; watchdog scheduler is not started; mock transport only"
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
