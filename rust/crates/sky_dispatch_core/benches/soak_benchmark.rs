//! Authored-packet soak simulation for `sky_dispatch_core`.
//!
//! This benchmark exercises the production coordinator lifecycle:
//! authored packet -> final admission -> one completion -> next authored
//! packet.  It intentionally contains no pending-release or retry model.

use std::env;
use std::time::Instant;

use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::testing::simulate_schedule;

const DEFAULT_NOTES: usize = 800;
const NOTE_INTERVAL_US: u64 = 300_000;
const NOTE_HOLD_US: u64 = 80_000;
const SEND_LATENCY_US: u64 = 200;
const SCAN_CODES: &[u16] = &[
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
];
const POLYPHONY: &[usize] = &[1, 3, 6, 10, 15];

fn build_actions(notes: usize) -> Vec<KeyActionInput> {
    let mut actions = Vec::with_capacity(notes.saturating_mul(8));
    let mut source_action_index = 0;

    for note in 0..notes {
        let scheduled_us = note as u64 * NOTE_INTERVAL_US;
        let polyphony = POLYPHONY[note % POLYPHONY.len()];
        let scan_codes: Vec<u16> = SCAN_CODES.iter().copied().take(polyphony).collect();
        actions.push(KeyActionInput {
            source_action_index,
            kind: ActionKind::Down,
            scheduled_us,
            scan_codes: scan_codes.clone().into(),
            reason: "soak down".into(),
        });
        source_action_index += 1;
        actions.push(KeyActionInput {
            source_action_index,
            kind: ActionKind::Up,
            scheduled_us: scheduled_us + NOTE_HOLD_US,
            scan_codes: scan_codes.into(),
            reason: "soak up".into(),
        });
        source_action_index += 1;
    }

    actions
}

fn main() {
    let notes = env::var("SOAK_NOTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_NOTES);
    let actions = build_actions(notes);
    let started = Instant::now();
    let result = simulate_schedule(&actions, SCAN_CODES, NOTE_HOLD_US, SEND_LATENCY_US)
        .expect("authored soak simulation must complete");
    let elapsed_us = started.elapsed().as_micros();
    let expected_released = (0..notes)
        .map(|note| POLYPHONY[note % POLYPHONY.len()])
        .sum::<usize>() as u64;

    assert!(result.is_finished, "authored lifecycle did not finish");
    assert_eq!(result.status_counts.get("active"), Some(&0));
    assert_eq!(
        result.status_counts.get("released"),
        Some(&expected_released)
    );
    assert!(
        !result.status_counts.contains_key("release_pending"),
        "legacy pending-release state must not exist"
    );

    let report = serde_json::json!({
        "benchmark": "authored_packet_soak",
        "lifecycle": "authored_packet_to_one_sendinput_completion",
        "notes": notes,
        "events": result.events.len(),
        "status_counts": result.status_counts,
        "elapsed_us": elapsed_us,
        "send_latency_us": SEND_LATENCY_US,
        "min_hold_us": NOTE_HOLD_US,
        "production_dispatch": false,
    });
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
