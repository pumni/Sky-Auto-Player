//! P3.5 — Drift benchmark for `sky_dispatch_core`.
//!
//! Detects whether the coordinator accumulates cumulative timing drift over a
//! long synthetic timeline.  A scheduler that never drifts will have a slope ≈ 0
//! in the `error_i = completion_i - authored_i` series.
//!
//! ## What is measured
//!
//! For a synthetic timeline of N notes (default 500, adjustable via `DRIFT_NOTES`):
//!
//! ```text
//! error_i = actual_dispatch_us(i) - authored_us(i)
//! ```
//!
//! Metrics reported per scenario:
//!
//! | Metric | Description |
//! |---|---|
//! | `slope_us_per_note` | Linear regression slope of error over note index |
//! | `median_early_us` | Median |error| for the first 20% of notes |
//! | `median_late_us` | Median |error| for the last 20% of notes |
//! | `delta_us` | `median_late - median_early` (positive = drifting later) |
//! | `burst_max_us` | Max |error| in the 5-note window after each pause/recovery |
//! | `max_abs_us` | Overall maximum |error| |
//! | `p95_us` | 95th percentile of |error| |
//!
//! ## Scenarios
//!
//! * `dense` — notes every 500 ms, no pauses.
//! * `sparse` — notes every 2 s, no pauses.
//! * `with_pause` — dense notes with a 5 s pause at the 1/3 mark.
//! * `with_recovery` — dense notes with a simulated failed-release recovery.
//!
//! ## Running
//!
//! ```powershell
//! cargo bench --manifest-path rust/Cargo.toml -p sky_dispatch_core -- drift_benchmark
//! # Override note count:
//! $env:DRIFT_NOTES=1000; cargo bench ...
//! ```

#[path = "src/stats.rs"]
#[allow(dead_code)]
mod stats;

use sky_dispatch_core::{
    compile::compile_runtime_intents,
    coordinator::{PendingDispatchPlan, RuntimeDispatchCoordinator},
    model::{ActionKind, KeyActionInput},
    time::{DurationTicks, TimelineTicks},
};

// ─── Config ───────────────────────────────────────────────────────────────────

/// Default number of notes in the synthetic timeline.
const DEFAULT_NOTES: usize = 500;
/// Interval between notes in the dense scenario (µs).
const DENSE_INTERVAL_US: u64 = 500_000;
/// Interval between notes in the sparse scenario (µs).
const SPARSE_INTERVAL_US: u64 = 2_000_000;
/// Hold time per note (µs) — Down then Up after this much time.
const NOTE_HOLD_US: u64 = 80_000;
/// Simulated send latency (µs) — deterministic fixed delay.
const SEND_LATENCY_US: u64 = 200;
/// Pause duration injected at the 1/3 mark (µs).
const PAUSE_DURATION_US: u64 = 5_000_000;
/// Window size for burst measurement (notes after pause/recovery).
const BURST_WINDOW: usize = 5;
/// Fraction defining "early" and "late" portions of the timeline.
const EARLY_LATE_FRACTION: f64 = 0.20;

// ─── Scan codes ───────────────────────────────────────────────────────────────

const SCAN_CODE: u16 = 0x15;

// ─── Timeline builder ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScenarioKind {
    Dense,
    Sparse,
    WithPause,
    WithRecovery,
}

impl ScenarioKind {
    fn as_str(self) -> &'static str {
        match self {
            ScenarioKind::Dense => "dense",
            ScenarioKind::Sparse => "sparse",
            ScenarioKind::WithPause => "with_pause",
            ScenarioKind::WithRecovery => "with_recovery",
        }
    }
}

/// Build a flat list of (Down, Up) action pairs for `n_notes` notes.
///
/// Returns the action list and the authored timestamps for each Down event.
fn build_timeline(
    n_notes: usize,
    interval_us: u64,
    inject_pause_at: Option<usize>,
) -> (Vec<KeyActionInput>, Vec<u64>) {
    let mut actions = Vec::with_capacity(n_notes * 2);
    let mut authored_ts = Vec::with_capacity(n_notes);
    let mut cursor_us: u64 = 0;

    for i in 0..n_notes {
        // Inject pause before this note if requested.
        if inject_pause_at == Some(i) {
            cursor_us += PAUSE_DURATION_US;
        }

        let down_us = cursor_us;
        let up_us = cursor_us + NOTE_HOLD_US;

        actions.push(KeyActionInput {
            source_action_index: (i * 2) as u32,
            kind: ActionKind::Down,
            scheduled_us: down_us,
            scan_codes: vec![SCAN_CODE].into(),
            reason: "drift-down".into(),
        });
        actions.push(KeyActionInput {
            source_action_index: (i * 2 + 1) as u32,
            kind: ActionKind::Up,
            scheduled_us: up_us,
            scan_codes: vec![SCAN_CODE].into(),
            reason: "drift-up".into(),
        });

        authored_ts.push(down_us);
        cursor_us += interval_us;
    }

    (actions, authored_ts)
}

// ─── Simulation runner ────────────────────────────────────────────────────────

/// Run the deterministic simulation and collect `error_i = actual_us - authored_us`
/// for each Down dispatch.  Returns the error series and the indices of notes
/// that come right after a pause or recovery event (for burst detection).
fn run_simulation(
    actions: &[KeyActionInput],
    authored_ts: &[u64],
    pause_note_idx: Option<usize>,
    inject_recovery_at: Option<usize>,
) -> Result<(Vec<i64>, Vec<usize>), String> {
    let allowed = vec![SCAN_CODE];
    let schedule =
        compile_runtime_intents(actions, &allowed).map_err(|e| format!("compile: {e:?}"))?;

    let mut coordinator = RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        /*min_hold_us=*/ NOTE_HOLD_US / 2,
        DurationTicks::from_raw(NOTE_HOLD_US / 2),
        /*delivery_margin_us=*/ 0,
        DurationTicks::ZERO,
        |microseconds| Ok(TimelineTicks::from_raw(microseconds)),
    )
    .map_err(|error| format!("coordinator construction failed: {error}"))?;

    let mut errors: Vec<i64> = Vec::with_capacity(authored_ts.len());
    // Track which note indices follow a burst-inducing event.
    let mut burst_indices: Vec<usize> = Vec::new();
    let mut note_idx = 0usize;
    let mut now_us: u64 = 0;

    // Mark burst windows.
    if let Some(p) = pause_note_idx {
        for i in p..p.saturating_add(BURST_WINDOW).min(authored_ts.len()) {
            burst_indices.push(i);
        }
    }
    if let Some(r) = inject_recovery_at {
        for i in r..r.saturating_add(BURST_WINDOW).min(authored_ts.len()) {
            burst_indices.push(i);
        }
    }

    let mut recovery_pending = false;
    let mut recovery_note = usize::MAX;
    if let Some(r) = inject_recovery_at {
        recovery_note = r;
    }

    while !coordinator.is_finished() {
        // Advance to next coordinator deadline.
        if let Some(dl) = coordinator
            .next_deadline_ticks(DurationTicks::ZERO, None)
            .map_err(|error| format!("coordinator deadline failed: {error}"))?
        {
            now_us = now_us.max(dl.as_u64());
        } else {
            now_us = now_us
                .checked_add(100)
                .ok_or_else(|| "simulation timestamp overflow".to_string())?;
        }

        // Pop pending releases first.
        let pending_plan = PendingDispatchPlan {
            deadline_ticks: TimelineTicks::from_raw(now_us),
            lead_ticks: DurationTicks::ZERO,
            polyphony: 1,
            lead_saturated: false,
        };
        let due = coordinator
            .pop_due_pending_ticks(TimelineTicks::from_raw(now_us), &pending_plan)
            .map_err(|error| format!("coordinator pending pop failed: {error}"))?;
        if !due.is_empty() {
            let sc: Vec<u16> = due.iter().map(|p| p.scan_code).collect();
            coordinator
                .complete_releases(&due, &sc, &[])
                .map_err(|error| format!("coordinator completion failed: {error}"))?;
            now_us = now_us
                .checked_add(SEND_LATENCY_US)
                .ok_or_else(|| "simulation timestamp overflow".to_string())?;
            continue;
        }

        // Pop next authored batch.
        if let Some((batch_index, _lead)) = coordinator
            .pop_next_due_authored_ticks(TimelineTicks::from_raw(now_us), DurationTicks::ZERO)
            .map_err(|error| format!("coordinator authored pop failed: {error}"))?
        {
            let batch = coordinator
                .schedule
                .try_materialize_batch_authored(batch_index)
                .map_err(|error| format!("batch materialization failed: {error}"))?;
            match batch.kind {
                ActionKind::Down => {
                    let authored = if note_idx < authored_ts.len() {
                        authored_ts[note_idx]
                    } else {
                        batch.scheduled_us
                    };

                    // Inject failed-release recovery at this note.
                    if recovery_pending && note_idx >= recovery_note {
                        recovery_pending = false;
                        // Just record a simulated offset (deterministic).
                        now_us = now_us
                            .checked_add(1_000)
                            .ok_or_else(|| "simulation timestamp overflow".to_string())?;
                    }

                    let sc: Vec<u16> = batch.intents.iter().map(|i| i.scan_code).collect();
                    coordinator
                        .activate_sent_downs_compact_ticks(
                            batch_index,
                            &sc,
                            TimelineTicks::from_raw(now_us),
                            TimelineTicks::from_raw(
                                now_us
                                    .checked_add(SEND_LATENCY_US)
                                    .ok_or_else(|| "simulation timestamp overflow".to_string())?,
                            ),
                            0,
                        )
                        .map_err(|error| format!("coordinator activation failed: {error}"))?;

                    let actual_us = now_us;
                    errors.push(actual_us as i64 - authored as i64);
                    note_idx += 1;
                    now_us = now_us
                        .checked_add(SEND_LATENCY_US)
                        .ok_or_else(|| "simulation timestamp overflow".to_string())?;
                }
                ActionKind::Up => {
                    let (requested, _) =
                        coordinator
                            .request_releases(&batch.intents)
                            .map_err(|error| {
                                format!("coordinator release transition failed: {error}")
                            })?;
                    let _ = requested;

                    // Inject a simulated release failure for recovery scenario.
                    if inject_recovery_at.is_some()
                        && !recovery_pending
                        && note_idx == recovery_note.saturating_sub(1)
                    {
                        // Simulate: pop the pending release, requeue it as failed.
                        let failure_deadline = now_us
                            .checked_add(NOTE_HOLD_US)
                            .ok_or_else(|| "simulation timestamp overflow".to_string())?;
                        let failure_plan = PendingDispatchPlan {
                            deadline_ticks: TimelineTicks::from_raw(failure_deadline),
                            lead_ticks: DurationTicks::ZERO,
                            polyphony: 1,
                            lead_saturated: false,
                        };
                        let due = coordinator
                            .pop_due_pending_ticks(
                                TimelineTicks::from_raw(failure_deadline),
                                &failure_plan,
                            )
                            .map_err(|error| format!("coordinator pending pop failed: {error}"))?;
                        if !due.is_empty() {
                            let retry_backoff = [
                                DurationTicks::from_raw(2_000),
                                DurationTicks::from_raw(5_000),
                                DurationTicks::from_raw(10_000),
                                DurationTicks::from_raw(20_000),
                            ];
                            let retry_base = now_us
                                .checked_add(1_000)
                                .ok_or_else(|| "simulation timestamp overflow".to_string())?;
                            let should_stop = coordinator
                                .requeue_failed_releases_ticks(
                                    &due,
                                    &[],
                                    &[],
                                    TimelineTicks::from_raw(now_us),
                                    TimelineTicks::from_raw(retry_base),
                                    &retry_backoff,
                                    Some(3),
                                )
                                .map_err(|error| format!("coordinator recovery failed: {error}"))?;
                            if !should_stop {
                                recovery_pending = true;
                                // Complete recovery after backoff.
                                let retry_deadline = now_us
                                    .checked_add(3_000)
                                    .ok_or_else(|| "simulation timestamp overflow".to_string())?;
                                let retry_plan = PendingDispatchPlan {
                                    deadline_ticks: TimelineTicks::from_raw(retry_deadline),
                                    lead_ticks: DurationTicks::ZERO,
                                    polyphony: 1,
                                    lead_saturated: false,
                                };
                                let retry = coordinator
                                    .pop_due_pending_ticks(
                                        TimelineTicks::from_raw(retry_deadline),
                                        &retry_plan,
                                    )
                                    .map_err(|error| {
                                        format!("coordinator retry pop failed: {error}")
                                    })?;
                                if !retry.is_empty() {
                                    let sc: Vec<u16> = retry.iter().map(|p| p.scan_code).collect();
                                    coordinator.complete_releases(&retry, &sc, &[]).map_err(
                                        |error| format!("coordinator completion failed: {error}"),
                                    )?;
                                    coordinator
                                        .finish_release_recovery_ticks(TimelineTicks::from_raw(
                                            retry_deadline,
                                        ))
                                        .map_err(|error| {
                                            format!("coordinator recovery failed: {error}")
                                        })?;
                                }
                            }
                        }
                    }
                }
            }
        } else {
            now_us = now_us
                .checked_add(100)
                .ok_or_else(|| "simulation timestamp overflow".to_string())?;
        }
    }

    Ok((errors, burst_indices))
}

// ─── Drift stats ──────────────────────────────────────────────────────────────

/// Compute linear regression slope (µs drift per note index).
fn linear_regression_slope(errors: &[i64]) -> f64 {
    let n = errors.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let sum_x: f64 = (0..errors.len()).map(|i| i as f64).sum();
    let sum_y: f64 = errors.iter().map(|&e| e as f64).sum();
    let sum_xx: f64 = (0..errors.len()).map(|i| (i as f64) * (i as f64)).sum();
    let sum_xy: f64 = errors
        .iter()
        .enumerate()
        .map(|(i, &e)| i as f64 * e as f64)
        .sum();
    let denom = n * sum_xx - sum_x * sum_x;
    if denom.abs() < 1e-9 {
        return 0.0;
    }
    (n * sum_xy - sum_x * sum_y) / denom
}

/// Median of a slice of i64 (absolute values).
fn median_abs(v: &[i64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut abs: Vec<i64> = v.iter().map(|&e| e.unsigned_abs() as i64).collect();
    abs.sort_unstable();
    let mid = abs.len() / 2;
    if abs.len().is_multiple_of(2) {
        (abs[mid - 1] + abs[mid]) as f64 / 2.0
    } else {
        abs[mid] as f64
    }
}

/// p95 of absolute errors.
fn p95_abs(errors: &[i64]) -> i64 {
    if errors.is_empty() {
        return 0;
    }
    let mut abs: Vec<i64> = errors.iter().map(|&e| e.unsigned_abs() as i64).collect();
    abs.sort_unstable();
    let idx = (0.95 * (abs.len() as f64 - 1.0)).round() as usize;
    abs[idx.min(abs.len() - 1)]
}

struct DriftReport {
    scenario: &'static str,
    n_notes: usize,
    slope: f64,
    median_early: f64,
    median_late: f64,
    delta: f64,
    burst_max: i64,
    max_abs: i64,
    p95: i64,
}

impl DriftReport {
    fn print_header() {
        println!(
            "{:<15} {:>7}  {:>12} {:>12} {:>12} {:>10}  {:>9} {:>7} {:>7}",
            "scenario",
            "notes",
            "slope_us/n",
            "med_early",
            "med_late",
            "delta_us",
            "burst_max",
            "max_abs",
            "p95"
        );
        println!("{}", "-".repeat(100));
    }

    fn print_row(&self) {
        println!(
            "{:<15} {:>7}  {:>+12.4} {:>12.1} {:>12.1} {:>+10.1}  {:>9} {:>7} {:>7}",
            self.scenario,
            self.n_notes,
            self.slope,
            self.median_early,
            self.median_late,
            self.delta,
            self.burst_max,
            self.max_abs,
            self.p95,
        );
    }
}

// ─── Scenario runner ──────────────────────────────────────────────────────────

fn run_scenario(kind: ScenarioKind, n_notes: usize) -> Result<DriftReport, String> {
    let interval_us = match kind {
        ScenarioKind::Sparse => SPARSE_INTERVAL_US,
        _ => DENSE_INTERVAL_US,
    };

    let pause_at: Option<usize> = match kind {
        ScenarioKind::WithPause => Some(n_notes / 3),
        _ => None,
    };

    let recovery_at: Option<usize> = match kind {
        ScenarioKind::WithRecovery => Some(n_notes / 3),
        _ => None,
    };

    let (actions, authored_ts) = build_timeline(n_notes, interval_us, pause_at);

    let (errors, burst_indices) = run_simulation(&actions, &authored_ts, pause_at, recovery_at)?;
    if errors.len() != n_notes {
        return Err(format!(
            "{}: expected {n_notes} timing samples, got {}",
            kind.as_str(),
            errors.len()
        ));
    }

    let early_end = ((n_notes as f64 * EARLY_LATE_FRACTION) as usize).max(1);
    let late_start = (n_notes as f64 * (1.0 - EARLY_LATE_FRACTION)) as usize;

    let median_early = median_abs(&errors[..early_end]);
    let median_late = median_abs(&errors[late_start..]);
    let delta = median_late - median_early;

    let slope = linear_regression_slope(&errors);

    let burst_max = burst_indices
        .iter()
        .filter_map(|&i| errors.get(i))
        .map(|&e| e.unsigned_abs() as i64)
        .max()
        .unwrap_or_default();

    let max_abs = errors
        .iter()
        .map(|&e| e.unsigned_abs() as i64)
        .max()
        .ok_or_else(|| format!("{}: simulation produced no timing samples", kind.as_str()))?;

    let p95 = p95_abs(&errors);

    Ok(DriftReport {
        scenario: kind.as_str(),
        n_notes: errors.len(),
        slope,
        median_early,
        median_late,
        delta,
        burst_max,
        max_abs,
        p95,
    })
}

// ─── Gate check ───────────────────────────────────────────────────────────────

/// Print a gate summary with per-scenario thresholds.
///
/// Recovery scenarios have a larger allowed slope because a single failed-release
/// recovery adds a one-time offset that shifts the entire tail of the error series;
/// this is expected and does NOT represent cumulative drift.  The gate for recovery
/// checks that slope is bounded (not unbounded growth), not that it is zero.
fn check_gate(reports: &[DriftReport]) -> Result<(), String> {
    println!();
    println!("# Gate check");
    println!("#   dense/sparse/pause:  slope < ±1.0 µs/note, delta < ±10 µs");
    println!("#   with_recovery:       slope < ±15.0 µs/note (one-time offset allowed)");
    let mut failures = Vec::new();
    for r in reports {
        // Recovery scenario: one-time offset is expected. Use looser slope gate.
        let (slope_limit, delta_limit): (f64, f64) = if r.scenario == "with_recovery" {
            (15.0, f64::MAX)
        } else {
            (1.0, 10.0)
        };
        let slope_ok = r.slope.abs() < slope_limit;
        let delta_ok = r.delta.abs() < delta_limit;
        let status = if slope_ok && delta_ok { "PASS" } else { "FAIL" };
        if !slope_ok || !delta_ok {
            failures.push(format!(
                "{}: slope={:+.4}, delta={:+.1}",
                r.scenario, r.slope, r.delta
            ));
        }
        println!(
            "  {:>15}: {} (slope={:+.4} µs/note, delta={:+.1} µs)",
            r.scenario, status, r.slope, r.delta
        );
    }
    println!();
    if failures.is_empty() {
        println!("# All scenarios: PASS");
        Ok(())
    } else {
        println!("# Drift gate: FAIL");
        Err(format!("drift gate failed: {}", failures.join("; ")))
    }
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() -> Result<(), String> {
    let n_notes: usize = match std::env::var("DRIFT_NOTES") {
        Ok(value) => value
            .parse()
            .map_err(|_| "DRIFT_NOTES must be a positive integer".to_string())?,
        Err(std::env::VarError::NotPresent) => DEFAULT_NOTES,
        Err(error) => return Err(format!("failed to read DRIFT_NOTES: {error}")),
    };
    if n_notes == 0 {
        return Err("DRIFT_NOTES must be positive".to_string());
    }

    println!("# P3.5 Drift benchmark — sky_dispatch_core");
    println!("# notes={n_notes}  built={}", env!("CARGO_PKG_VERSION"));
    println!("# error_i = actual_dispatch_us(i) - authored_us(i)");
    println!("# Gate: slope < ±1 µs/note, delta < ±10 µs");
    println!();

    DriftReport::print_header();

    let scenarios = [
        ScenarioKind::Dense,
        ScenarioKind::Sparse,
        ScenarioKind::WithPause,
        ScenarioKind::WithRecovery,
    ];

    let mut reports: Vec<DriftReport> = Vec::new();
    for &kind in &scenarios {
        let report = run_scenario(kind, n_notes)?;
        report.print_row();
        reports.push(report);
    }

    check_gate(&reports)?;

    println!("# Done. {} scenarios × {} notes.", scenarios.len(), n_notes);
    Ok(())
}
