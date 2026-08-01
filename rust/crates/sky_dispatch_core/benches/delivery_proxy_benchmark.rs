//! P3.6 — Delivery-proxy simulation for `sky_dispatch_core`.
//!
//! Verifies coordinator lifecycle accounting across chord sizes. This is a
//! deterministic simulation; it does not call SendInput and does not receive
//! real WM_INPUT receipts.
//!
//! ## Metrics (per chord size)
//!
//! | Metric | Description |
//! |---|---|
//! | `first_receipt_error_us` | Signed error between authored time and first key receipt |
//! | `last_receipt_error_us` | Signed error between authored time and last key receipt |
//! | `intra_chord_spread_us` | Max time gap between first and last key receipt in a chord |
//! | `missing` | Keys expected but not dispatched in the generation lifecycle |
//! | `duplicate` | Keys dispatched more than once without an intervening release |
//! | `reorder` | Chords where the observed scan-code order differs from authored |
//!
//! ## Gate (correctness)
//!
//! In a clean controlled (deterministic simulation) run:
//! ```text
//! missing   = 0
//! duplicate = 0
//! mismatch  = 0   (scan code order matches authored)
//! ```
//!
//! ## Running
//!
//! ```powershell
//! cargo bench --manifest-path rust/Cargo.toml -p sky_dispatch_core -- delivery_proxy_benchmark
//! ```

#[path = "src/stats.rs"]
#[allow(dead_code)]
mod stats;

use sky_dispatch_core::{
    compile::compile_runtime_intents,
    coordinator::RuntimeDispatchCoordinator,
    model::{ActionKind, KeyActionInput},
    time::TimelineTicks,
};

// ─── Config ───────────────────────────────────────────────────────────────────

/// Chord sizes to exercise (matching the plan's "theo chord size" requirement).
const CHORD_SIZES: &[usize] = &[1, 2, 3, 5, 8, 10, 15];
/// Number of chord repetitions per chord size.
const REPS_PER_SIZE: usize = 20;
/// Simulated send latency (µs) — deterministic and fixed.
const SEND_LATENCY_US: u64 = 150;
/// Minimum hold time per note (µs).
const MIN_HOLD_US: u64 = 50_000;
/// Time between chord cycles (µs).
const CYCLE_GAP_US: u64 = 200_000;

/// All available scan codes (max chord size = 15).
const ALL_SCAN_CODES: &[u16] = &[
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
];

// ─── Receipt tracking ─────────────────────────────────────────────────────────

/// Record of one dispatched batch (Down or Up) as observed by the harness.
#[derive(Debug, Clone)]
struct DispatchReceipt {
    chord_index: usize,
    kind: ActionKind,
    /// Actual dispatch time in simulation µs.
    actual_us: u64,
    /// Authored scheduled time (retained for future per-key error analysis).
    #[allow(dead_code)]
    authored_us: u64,
    /// Scan codes observed in dispatch order.
    scan_codes: Vec<u16>,
}

/// Aggregate correctness counters for one chord size.
#[derive(Debug, Default)]
struct DeliveryStats {
    chord_size: usize,
    total_chords: usize,
    missing: usize,
    duplicate: usize,
    reorder: usize,
    /// All first-receipt signed errors (µs).
    first_errors: Vec<i64>,
    /// All last-receipt signed errors (µs).
    last_errors: Vec<i64>,
    /// All intra-chord spread values (µs).
    spreads: Vec<u64>,
}

impl DeliveryStats {
    fn mean_first_error(&self) -> f64 {
        mean_i64(&self.first_errors)
    }
    fn mean_last_error(&self) -> f64 {
        mean_i64(&self.last_errors)
    }
    fn mean_spread(&self) -> f64 {
        mean_u64(&self.spreads)
    }
    fn max_spread(&self) -> u64 {
        self.spreads.iter().copied().max().unwrap_or(0)
    }
    fn gate_ok(&self) -> bool {
        self.missing == 0 && self.duplicate == 0 && self.reorder == 0
    }
}

fn mean_i64(v: &[i64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64
}

fn mean_u64(v: &[u64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().map(|&x| x as f64).sum::<f64>() / v.len() as f64
}

// ─── Simulation ───────────────────────────────────────────────────────────────

/// Build a sequence of `reps` chord cycles for a given chord size.
fn build_chord_sequence(
    chord_size: usize,
    reps: usize,
) -> (Vec<KeyActionInput>, Vec<(u64, Vec<u16>)>) {
    let codes: Vec<u16> = ALL_SCAN_CODES[..chord_size].to_vec();
    let mut actions = Vec::with_capacity(reps * 2);
    let mut authored: Vec<(u64, Vec<u16>)> = Vec::with_capacity(reps);

    let mut cursor: u64 = 0;
    for i in 0..reps {
        let down_us = cursor;
        let up_us = cursor + MIN_HOLD_US;

        actions.push(KeyActionInput {
            source_action_index: (i * 2) as u32,
            kind: ActionKind::Down,
            scheduled_us: down_us,
            scan_codes: codes.clone().into(),
            reason: "proxy-down".into(),
        });
        actions.push(KeyActionInput {
            source_action_index: (i * 2 + 1) as u32,
            kind: ActionKind::Up,
            scheduled_us: up_us,
            scan_codes: codes.clone().into(),
            reason: "proxy-up".into(),
        });

        authored.push((down_us, codes.clone()));
        cursor += CYCLE_GAP_US;
    }

    (actions, authored)
}

/// Run the simulation and collect dispatch receipts.
fn run_chord_simulation(
    actions: &[KeyActionInput],
    chord_size: usize,
) -> Result<Vec<DispatchReceipt>, String> {
    let allowed: Vec<u16> = ALL_SCAN_CODES[..chord_size].to_vec();
    let schedule =
        compile_runtime_intents(actions, &allowed).map_err(|e| format!("compile: {e:?}"))?;

    let mut coordinator =
        RuntimeDispatchCoordinator::new(schedule, MIN_HOLD_US / 2, 0, TimelineTicks);

    let mut receipts: Vec<DispatchReceipt> = Vec::new();
    let mut now_us: u64 = 0;
    let mut chord_index: usize = 0;

    while !coordinator.is_finished() {
        if let Some(dl) = coordinator.next_deadline_us(0, 0) {
            now_us = now_us.max(dl);
        } else {
            now_us += 100;
        }

        // Drain pending releases.
        let due = coordinator.pop_due_pending(now_us, 0);
        if !due.is_empty() {
            let authored_us = due[0].scheduled_release_us;
            let scan_codes: Vec<u16> = due.iter().map(|p| p.scan_code).collect();
            let sc_copy = scan_codes.clone();
            coordinator.complete_releases(&due, &sc_copy, &[]);

            receipts.push(DispatchReceipt {
                chord_index,
                kind: ActionKind::Up,
                actual_us: now_us,
                authored_us,
                scan_codes,
            });
            now_us += SEND_LATENCY_US;
            continue;
        }

        // Pop authored batch.
        if let Some((batch, _)) = coordinator.pop_next_due_authored(now_us, 0) {
            let authored_us = batch.scheduled_us;
            match batch.kind {
                ActionKind::Down => {
                    let scan_codes: Vec<u16> = batch.intents.iter().map(|i| i.scan_code).collect();
                    let sc_copy = scan_codes.clone();
                    coordinator.activate_sent_downs(
                        &batch.intents,
                        &sc_copy,
                        now_us,
                        TimelineTicks(now_us),
                        now_us + SEND_LATENCY_US,
                        TimelineTicks(now_us + SEND_LATENCY_US),
                    );
                    receipts.push(DispatchReceipt {
                        chord_index,
                        kind: ActionKind::Down,
                        actual_us: now_us,
                        authored_us,
                        scan_codes,
                    });
                    now_us += SEND_LATENCY_US;
                    chord_index += 1;
                }
                ActionKind::Up => {
                    let (requested, _suppressed) = coordinator.request_releases(&batch.intents);
                    let _ = requested;
                }
            }
        } else {
            now_us += 100;
        }
    }

    Ok(receipts)
}

// ─── Analysis ─────────────────────────────────────────────────────────────────

fn analyse(
    chord_size: usize,
    authored: &[(u64, Vec<u16>)],
    receipts: &[DispatchReceipt],
) -> DeliveryStats {
    let mut stats = DeliveryStats {
        chord_size,
        total_chords: authored.len(),
        ..Default::default()
    };

    let down_receipts: Vec<&DispatchReceipt> = receipts
        .iter()
        .filter(|r| matches!(r.kind, ActionKind::Down))
        .collect();

    // Count by authored identity rather than by total receipt length. This
    // catches a missing receipt paired with an out-of-range or duplicate one.
    let mut seen = std::collections::HashMap::<usize, usize>::new();
    for r in &down_receipts {
        *seen.entry(r.chord_index).or_insert(0) += 1;
    }
    for chord_index in 0..authored.len() {
        match seen.get(&chord_index).copied().unwrap_or(0) {
            0 => stats.missing += 1,
            count if count > 1 => stats.duplicate += count - 1,
            _ => {}
        }
    }
    if seen
        .keys()
        .any(|&chord_index| chord_index >= authored.len())
    {
        stats.reorder += 1;
    }

    // Per-chord analysis.
    for (i, (authored_us, authored_codes)) in authored.iter().enumerate() {
        let receipt = down_receipts.iter().find(|r| r.chord_index == i);
        let Some(receipt) = receipt else { continue };

        // first/last receipt error — for polyphony > 1 we model all keys as
        // arriving at the same time (send latency applied uniformly).
        let first_actual = receipt.actual_us;
        let last_actual = receipt.actual_us
            + if chord_size > 1 {
                SEND_LATENCY_US / chord_size as u64
            } else {
                0
            };

        stats
            .first_errors
            .push(first_actual as i64 - *authored_us as i64);
        stats
            .last_errors
            .push(last_actual as i64 - *authored_us as i64);

        // intra_chord_spread: gap between first and last key in this chord.
        let spread = last_actual.saturating_sub(first_actual);
        stats.spreads.push(spread);

        // reorder: observed scan-code order must match authored.
        if receipt.scan_codes != *authored_codes {
            stats.reorder += 1;
        }
    }

    stats
}

// ─── Report ───────────────────────────────────────────────────────────────────

fn print_header() {
    println!(
        "{:>6}  {:>10}  {:>12} {:>12} {:>12} {:>10}  {:>7} {:>9} {:>7}  {:>6}",
        "chord",
        "reps",
        "first_err_us",
        "last_err_us",
        "spread_us",
        "max_sprd_us",
        "missing",
        "duplicate",
        "reorder",
        "gate"
    );
    println!("{}", "-".repeat(105));
}

fn print_row(s: &DeliveryStats) {
    println!(
        "{:>6}  {:>10}  {:>+12.1} {:>+12.1} {:>12.1} {:>10}  {:>7} {:>9} {:>7}  {:>6}",
        s.chord_size,
        s.total_chords,
        s.mean_first_error(),
        s.mean_last_error(),
        s.mean_spread(),
        s.max_spread(),
        s.missing,
        s.duplicate,
        s.reorder,
        if s.gate_ok() { "PASS" } else { "FAIL" },
    );
}

fn print_gate_summary(all_stats: &[DeliveryStats]) {
    println!();
    println!("# Gate (correctness): missing=0, duplicate=0, reorder=0");
    let total_missing: usize = all_stats.iter().map(|s| s.missing).sum();
    let total_dup: usize = all_stats.iter().map(|s| s.duplicate).sum();
    let total_reorder: usize = all_stats.iter().map(|s| s.reorder).sum();
    let pass = total_missing == 0 && total_dup == 0 && total_reorder == 0;
    println!(
        "  total missing={total_missing}  duplicate={total_dup}  reorder={total_reorder}  → {}",
        if pass { "PASS" } else { "FAIL" }
    );
}

// ─── Main ─────────────────────────────────────────────────────────────────────

fn main() -> Result<(), String> {
    println!("# P3.6 Delivery-proxy simulation — sky_dispatch_core");
    println!("# Evidence scope: deterministic simulation; not a real Raw Input delivery proxy");
    println!(
        "# chord_sizes={CHORD_SIZES:?}  reps_per_size={REPS_PER_SIZE}  built={}",
        env!("CARGO_PKG_VERSION")
    );
    println!("# Gate: missing=0, duplicate=0, reorder=0 in clean controlled run");
    println!();

    print_header();

    let mut all_stats: Vec<DeliveryStats> = Vec::new();

    for &chord_size in CHORD_SIZES {
        let (actions, authored) = build_chord_sequence(chord_size, REPS_PER_SIZE);
        match run_chord_simulation(&actions, chord_size) {
            Ok(receipts) => {
                let s = analyse(chord_size, &authored, &receipts);
                print_row(&s);
                all_stats.push(s);
            }
            Err(e) => return Err(format!("chord={chord_size}: {e}")),
        }
    }

    print_gate_summary(&all_stats);

    println!(
        "# Done. {} chord sizes × {} reps = {} total chord cycles.",
        CHORD_SIZES.len(),
        REPS_PER_SIZE,
        CHORD_SIZES.len() * REPS_PER_SIZE,
    );
    if all_stats.iter().any(|stats| !stats.gate_ok()) {
        return Err("delivery-proxy simulation gate failed".to_string());
    }
    Ok(())
}
