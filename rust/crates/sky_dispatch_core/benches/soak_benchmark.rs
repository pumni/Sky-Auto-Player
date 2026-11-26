//! Production `DispatchExecutor` soak simulation for `sky_dispatch_core`.
//!
//! It advances the production scheduler through its real `evaluate_at` and
//! send-completion APIs. Worker admission and Win32 delivery are not measured.

use std::env;
use std::time::Instant;

use sky_dispatch_core::model::OnsetChordInput;
use sky_dispatch_core::testing::simulate_schedule;

const DEFAULT_NOTES: usize = 800;
const NOTE_INTERVAL_US: u64 = 300_000;
const NOTE_HOLD_US: u64 = 80_000;
const SEND_LATENCY_US: u64 = 200;
const SCAN_CODES: &[u16] = &[
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
];
const POLYPHONY: &[usize] = &[1, 3, 6, 10, 15];

fn build_chords(notes: usize) -> Vec<OnsetChordInput> {
    let mut chords = Vec::with_capacity(notes);
    for note in 0..notes {
        let scheduled_us = note as u64 * NOTE_INTERVAL_US;
        let polyphony = POLYPHONY[note % POLYPHONY.len()];
        chords.push(OnsetChordInput {
            source_action_index: note as u32,
            scheduled_us,
            scan_codes: SCAN_CODES.iter().copied().take(polyphony).collect(),
        });
    }
    chords
}

fn main() {
    let notes = env::var("SOAK_NOTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_NOTES);
    let chords = build_chords(notes);
    let started = Instant::now();
    let result = simulate_schedule(&chords, SCAN_CODES, NOTE_HOLD_US, SEND_LATENCY_US)
        .expect("executor soak simulation must complete");
    let elapsed_us = started.elapsed().as_micros();
    let expected_released = (0..notes)
        .map(|note| POLYPHONY[note % POLYPHONY.len()])
        .sum::<usize>() as u64;

    assert!(result.is_finished, "executor lifecycle did not finish");
    assert_eq!(result.status_counts.get("active"), Some(&0));
    assert_eq!(
        result.status_counts.get("released"),
        Some(&expected_released)
    );

    let report = serde_json::json!({
        "benchmark": "dispatch_executor_soak",
        "scheduler_owner": "DispatchExecutor",
        "lifecycle": "executor_decision_to_mock_send_completion",
        "notes": notes,
        "events": result.events.len(),
        "status_counts": result.status_counts,
        "elapsed_us": elapsed_us,
        "send_latency_us": SEND_LATENCY_US,
        "min_hold_us": NOTE_HOLD_US,
        "worker_admission_and_win32_delivery": "not_measured",
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
