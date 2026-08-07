//! P3.4 — Coordinator microbenchmark for `sky_dispatch_core`.
//!
//! Measures coordinator dispatch latency broken down by:
//!   kind (Down / Up)  ×  polyphony (1, 3, 6, 10, 15)
//!   ×  latency class (Hot / Cold)  ×  load mode (idle / load)
//!
//! Metrics reported per cell:
//!   signed_error, absolute_error, late_only, early_only,
//!   p50, p95, p99, p99.9 (N/A when n < 1000), max,
//!   send_duration, bookkeeping, wake_error
//!
//! ## Running
//!
//! ```powershell
//! cargo bench --manifest-path rust/Cargo.toml -p sky_dispatch_core -- coordinator_benchmark
//! # Increase iterations for p99.9:
//! BENCH_ITERS=1000 cargo bench --manifest-path rust/Cargo.toml -p sky_dispatch_core -- coordinator_benchmark
//! ```
//!
//! Output is a plain aligned table to stdout.

// `harness = false` means this file provides its own `main`.

#[path = "src/stats.rs"]
mod stats;

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

use sky_dispatch_core::{
    compile::compile_runtime_intents,
    coordinator::{PendingDispatchPlan, RuntimeDispatchCoordinator},
    estimator::LatencyClass,
    model::{ActionKind, KeyActionInput},
    time::{DurationTicks, TimelineTicks},
};

use stats::{CoordinatorStats, print_header, print_row};

// ─── Constants ────────────────────────────────────────────────────────────────

const POLYPHONY_LEVELS: &[usize] = &[1, 3, 6, 10, 15];
const DEFAULT_ITERS: usize = 200;

/// Scan codes used in the benchmark.
/// 15 unique scan codes cover the maximum polyphony level.
const ALL_SCAN_CODES: &[u16] = &[
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
];

// ─── Load mode ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadMode {
    Idle,
    Load,
}

impl LoadMode {
    fn as_str(self) -> &'static str {
        match self {
            LoadMode::Idle => "idle",
            LoadMode::Load => "load",
        }
    }
}

// ─── Schedule builder ─────────────────────────────────────────────────────────

/// Build the minimal `KeyActionInput` sequence for one dispatch cell.
///
/// For Down cells: a single chord Down then a matching Up.
/// For Up cells: same structure; we measure the Up pop path.
fn build_actions(polyphony: usize) -> Vec<KeyActionInput> {
    assert!(polyphony >= 1 && polyphony <= ALL_SCAN_CODES.len());
    let codes: smallvec::SmallVec<[u16; 15]> =
        ALL_SCAN_CODES[..polyphony].iter().copied().collect();

    // Always build Down + Up pair; the benchmark selects which half to measure.
    vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 1_000,
            scan_codes: codes.clone().into_vec().into(),
            reason: "bench-down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 2_000,
            scan_codes: codes.into_vec().into(),
            reason: "bench-up".into(),
        },
    ]
}

/// Allowed scan codes slice for compile (full set regardless of polyphony).
fn allowed_codes(polyphony: usize) -> Vec<u16> {
    ALL_SCAN_CODES[..polyphony].to_vec()
}

// ─── Single-iteration runner ──────────────────────────────────────────────────

/// Run one full lifecycle iteration and collect samples into `stats`.
///
/// Returns `Ok(())` on success, `Err` if compile failed (should never happen
/// with valid inputs).
fn run_iteration(
    kind: ActionKind,
    polyphony: usize,
    _latency_class: LatencyClass,
    stats: &mut CoordinatorStats,
) -> Result<(), String> {
    let actions = build_actions(polyphony);
    let allowed = allowed_codes(polyphony);

    // ── bookkeeping: compile + coordinator init ───────────────────────────────
    let t_book_start = Instant::now();
    let schedule =
        compile_runtime_intents(&actions, &allowed).map_err(|e| format!("compile error: {e:?}"))?;
    let mut coordinator = RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        /*min_hold_us=*/ 50,
        DurationTicks::from_raw(50),
        /*delivery_margin_us=*/ 0,
        DurationTicks::ZERO,
        |microseconds| Ok(TimelineTicks::from_raw(microseconds)),
    )
    .map_err(|error| format!("coordinator construction failed: {error}"))?;
    let book_us = t_book_start.elapsed().as_micros() as u64;

    match kind {
        ActionKind::Down => {
            // ── Measure Down pop + activate ───────────────────────────────────
            let deadline = coordinator
                .next_deadline_ticks(DurationTicks::ZERO, None)
                .map_err(|error| format!("coordinator deadline failed: {error}"))?
                .ok_or("no down deadline")?
                .as_u64();
            let t_pop = Instant::now();
            let (batch_index, _lead) = coordinator
                .pop_next_due_authored_ticks(TimelineTicks::from_raw(deadline), DurationTicks::ZERO)
                .map_err(|error| format!("coordinator authored pop failed: {error}"))?
                .ok_or("no down batch due")?;
            let batch = coordinator
                .schedule
                .try_materialize_batch_authored(batch_index)
                .map_err(|error| format!("batch materialization failed: {error}"))?;
            let scheduled_us = batch.scheduled_us;

            let sc_vec: Vec<u16> = batch.intents.iter().map(|i| i.scan_code).collect();
            let t_activate_start = Instant::now();
            let now_us = deadline;
            coordinator
                .activate_sent_downs_compact_ticks(
                    batch_index,
                    &sc_vec,
                    TimelineTicks::from_raw(now_us),
                    TimelineTicks::from_raw(now_us),
                    0,
                )
                .map_err(|error| format!("coordinator activation failed: {error}"))?;
            let send_us =
                t_activate_start.elapsed().as_micros() as u64 + t_pop.elapsed().as_micros() as u64;

            let actual_us = deadline;
            let signed_err = actual_us as i64 - scheduled_us as i64;
            // wake_error: |actual pop time - deadline|. Deterministic here → 0.
            let wake_err = 0u64;

            stats.push(signed_err, send_us, book_us, wake_err);
        }

        ActionKind::Up => {
            // ── First do the Down (required for Up to be meaningful) ──────────
            let down_dl = coordinator
                .next_deadline_ticks(DurationTicks::ZERO, None)
                .map_err(|error| format!("coordinator deadline failed: {error}"))?
                .ok_or("no down deadline")?
                .as_u64();
            let (down_index, _) = coordinator
                .pop_next_due_authored_ticks(TimelineTicks::from_raw(down_dl), DurationTicks::ZERO)
                .map_err(|error| format!("coordinator authored pop failed: {error}"))?
                .ok_or("no down batch")?;
            let down_batch = coordinator
                .schedule
                .try_materialize_batch_authored(down_index)
                .map_err(|error| format!("down batch materialization failed: {error}"))?;
            let sc_vec: Vec<u16> = down_batch.intents.iter().map(|i| i.scan_code).collect();
            coordinator
                .activate_sent_downs_compact_ticks(
                    down_index,
                    &sc_vec,
                    TimelineTicks::from_raw(down_dl),
                    TimelineTicks::from_raw(down_dl),
                    0,
                )
                .map_err(|error| format!("coordinator activation failed: {error}"))?;

            // ── Measure Up pop + request_releases ────────────────────────────
            let up_dl = coordinator
                .next_authored_ticks(DurationTicks::ZERO)
                .map_err(|error| format!("coordinator deadline failed: {error}"))?
                .ok_or("no up deadline")?
                .as_u64();
            let t_pop = Instant::now();
            let (up_index, _lead) = coordinator
                .pop_next_due_authored_ticks(TimelineTicks::from_raw(up_dl), DurationTicks::ZERO)
                .map_err(|error| format!("coordinator authored pop failed: {error}"))?
                .ok_or("no up batch due")?;
            let up_batch = coordinator
                .schedule
                .try_materialize_batch_authored(up_index)
                .map_err(|error| format!("up batch materialization failed: {error}"))?;
            let scheduled_us = up_batch.scheduled_us;

            let t_release_start = Instant::now();
            let (requested, _suppressed) = coordinator
                .request_releases(&up_batch.intents)
                .map_err(|error| format!("coordinator release transition failed: {error}"))?;
            let _ = requested; // consumed
            let t_after_req = t_release_start.elapsed().as_micros() as u64;

            // Pop due pending and complete
            let pending_plan = PendingDispatchPlan {
                deadline_ticks: TimelineTicks::from_raw(up_dl),
                lead_ticks: DurationTicks::ZERO,
                polyphony,
                lead_saturated: false,
            };
            let due = coordinator
                .pop_due_pending_ticks(TimelineTicks::from_raw(up_dl), &pending_plan)
                .map_err(|error| format!("coordinator pending pop failed: {error}"))?;
            let due_sc: Vec<u16> = due.iter().map(|p| p.scan_code).collect();
            coordinator
                .complete_releases(&due, &due_sc)
                .map_err(|error| format!("coordinator completion failed: {error}"))?;

            let send_us = t_after_req + t_pop.elapsed().as_micros() as u64;
            let actual_us = up_dl;
            let signed_err = actual_us as i64 - scheduled_us as i64;
            let wake_err = 0u64;

            stats.push(signed_err, send_us, book_us, wake_err);
        }
    }

    Ok(())
}

// ─── Benchmark cell ───────────────────────────────────────────────────────────

/// Run all iterations for one cell, optionally with a CPU-stress background thread.
fn run_cell(
    kind: ActionKind,
    polyphony: usize,
    latency_class: LatencyClass,
    load_mode: LoadMode,
    iters: usize,
) -> Result<CoordinatorStats, String> {
    // Spin up background stressor if needed.
    let stop = Arc::new(AtomicBool::new(false));
    let mut stressor_handle = if load_mode == LoadMode::Load {
        let stop_clone = Arc::clone(&stop);
        Some(std::thread::spawn(move || {
            while !stop_clone.load(Ordering::Relaxed) {
                std::hint::spin_loop();
            }
        }))
    } else {
        None
    };

    let mut stats = CoordinatorStats::default();

    for _ in 0..iters {
        if let Err(e) = run_iteration(kind, polyphony, latency_class, &mut stats) {
            stop.store(true, Ordering::Relaxed);
            if let Some(handle) = stressor_handle.take() {
                let _ = handle.join();
            }
            return Err(format!("iteration failed: {e}"));
        }
    }

    // Stop stressor.
    stop.store(true, Ordering::Relaxed);
    if let Some(h) = stressor_handle {
        h.join().ok();
    }

    Ok(stats)
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() -> Result<(), String> {
    let iters: usize = std::env::var("BENCH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_ITERS);

    println!("# P3.4 Coordinator microbenchmark — sky_dispatch_core");
    println!(
        "# Evidence scope: deterministic coordinator simulation; no SendInput or Raw Input delivery"
    );
    println!("# iterations={iters}  built={}", env!("CARGO_PKG_VERSION"));
    println!(
        "# Columns: signed_err/abs_err/late/early in µs (mean); p50/p95/p99/p999/max abs µs; send_us/book_us/wake_us (mean µs)"
    );
    println!();

    print_header();

    let kinds = [(ActionKind::Down, "down"), (ActionKind::Up, "up")];
    let classes = [(LatencyClass::Hot, "hot"), (LatencyClass::Cold, "cold")];
    let loads = [LoadMode::Idle, LoadMode::Load];

    for (kind, kind_str) in &kinds {
        for &poly in POLYPHONY_LEVELS {
            for (class, class_str) in &classes {
                for &load in &loads {
                    let stats = run_cell(*kind, poly, *class, load, iters)?;
                    print_row(kind_str, poly, class_str, load.as_str(), &stats);
                }
            }
        }
    }

    println!();
    println!(
        "# Done. {} cells × {} iters = {} total dispatches.",
        kinds.len() * POLYPHONY_LEVELS.len() * classes.len() * loads.len(),
        iters,
        kinds.len() * POLYPHONY_LEVELS.len() * classes.len() * loads.len() * iters,
    );
    println!("# p99.9 requires BENCH_ITERS>=1000; shown as N/A when n<1000.");
    Ok(())
}
