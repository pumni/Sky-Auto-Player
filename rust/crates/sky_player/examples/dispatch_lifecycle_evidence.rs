#![forbid(unsafe_code)]

use serde_json::{Value, json};
use sky_dispatch_core::executor::DispatchDecision;
use sky_dispatch_core::time::QpcTicks;
use sky_dispatch_win32::clock::{QpcClock, qpc_frequency_checked};
use sky_dispatch_win32::input::PhysicalPacket;
use sky_player::engine::StagedDispatchOutcome;
use sky_player::engine::dispatch_primitives::{
    CleanupCapture, PacketCapture, ProductionDispatchTestHarness,
};
use std::time::Duration;

const FRAME_US: u64 = 16_667;
const MARGIN_US: u64 = 500;
const HOLD_US: u64 = FRAME_US + MARGIN_US;

fn accounting(harness: &ProductionDispatchTestHarness, phase: &str) -> Value {
    let value = harness.generation_accounting_for_test();
    json!({
        "phase": phase,
        "generation_accounting_owner": "live_scheduler",
        "total": value.total,
        "scheduled": value.scheduled,
        "activated": value.activated,
        "released": value.released,
        "cancelled": value.cancelled,
        "active": value.active,
    })
}

fn packet_capture_evidence(capture: &PacketCapture) -> Result<Vec<Value>, String> {
    let captured = capture
        .lock()
        .map_err(|_| "mock transport capture poisoned".to_string())?;
    let mut packets = Vec::with_capacity(captured.len());
    for (packet, evidence) in captured.iter() {
        let requested_mask = packet.up_mask | packet.down_mask;
        if evidence.requested_mask != requested_mask
            || evidence.confirmed_mask != requested_mask
            || evidence.first_inserted != packet.event_count()
            || evidence.attempts != 1
            || evidence.started_ticks.is_none()
            || evidence.completed_ticks.is_none()
            || evidence.completed_ticks < evidence.started_ticks
        {
            return Err("mock packet capture did not confirm one complete dispatched send".into());
        }
        packets.push(json!({
            "up_mask": packet.up_mask,
            "down_mask": packet.down_mask,
            "requested_mask": evidence.requested_mask,
            "confirmed_mask": evidence.confirmed_mask,
            "first_inserted": evidence.first_inserted,
            "attempts": evidence.attempts,
            "started_qpc_ticks": evidence.started_ticks.map(QpcTicks::as_u64),
            "completed_qpc_ticks": evidence.completed_ticks.map(QpcTicks::as_u64),
            "worker_outcome": "dispatched",
        }));
    }
    Ok(packets)
}

fn cleanup_capture_records(capture: &CleanupCapture) -> Result<Vec<(Vec<u16>, bool)>, String> {
    capture
        .lock()
        .map(|records| records.clone())
        .map_err(|_| "mock cleanup capture poisoned".to_string())
}

fn cleanup_capture_evidence(records: &[(Vec<u16>, bool)]) -> Value {
    let up_attempts = records.iter().filter(|(_, key_up)| *key_up).count();
    let down_attempts = records.len() - up_attempts;
    let up_events = records
        .iter()
        .filter(|(_, key_up)| *key_up)
        .map(|(scan_codes, _)| scan_codes.len())
        .sum::<usize>();
    let down_events = records
        .iter()
        .filter(|(_, key_up)| !*key_up)
        .map(|(scan_codes, _)| scan_codes.len())
        .sum::<usize>();
    let packets = records
        .iter()
        .map(|(scan_codes, key_up)| {
            json!({
                "scan_codes": scan_codes,
                "key_up": key_up,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "attempts": records.len(),
        "up_attempts": up_attempts,
        "down_attempts": down_attempts,
        "up_events": up_events,
        "down_events": down_events,
        "packets": packets,
    })
}

fn wait_until(clock: QpcClock, deadline: QpcTicks) -> Result<(), String> {
    loop {
        let now = clock
            .now()
            .map_err(|error| format!("QPC read while waiting: {error:?}"))?;
        if now >= deadline {
            return Ok(());
        }
        let remaining = deadline
            .checked_duration_since(now)
            .map_err(|error| format!("QPC wait ordering: {error}"))?;
        let sleep_us = clock
            .duration_to_us(remaining)
            .map_err(|error| format!("QPC wait conversion: {error:?}"))?
            .clamp(1, 10_000);
        std::thread::sleep(Duration::from_micros(sleep_us));
    }
}

fn dispatch_next(
    harness: &mut ProductionDispatchTestHarness,
    clock: QpcClock,
    epoch: QpcTicks,
) -> Result<Option<PhysicalPacket>, String> {
    loop {
        let now = clock
            .now()
            .map_err(|error| format!("QPC read before scheduler evaluation: {error:?}"))?;
        match harness.evaluate_live_scheduler_for_test(now, epoch)? {
            DispatchDecision::Finished => return Ok(None),
            DispatchDecision::Wait { deadline_qpc } => wait_until(clock, deadline_qpc)?,
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
                ..
            } => {
                let down_mask = down_chord.as_ref().map_or(0, |chord| chord.key_mask);
                let candidate = harness
                    .stage_candidate_with_scheduler_window_for_test(
                        up_mask,
                        down_chord,
                        timing_window,
                    )?
                    .ok_or("live scheduler Send decision did not stage a candidate")?;
                let sender_start = clock
                    .now()
                    .map_err(|error| format!("QPC read before mock transport: {error:?}"))?;
                harness.set_mock_sender_start_qpc_for_test(sender_start);
                if !matches!(
                    harness.dispatch_staged_candidate_for_test(candidate, false),
                    StagedDispatchOutcome::Dispatched
                ) {
                    return Err("production worker staging/send path did not dispatch".into());
                }
                return Ok(Some(PhysicalPacket::new(up_mask, down_mask)));
            }
        }
    }
}

fn new_epoch(
    clock: QpcClock,
    harness: &mut ProductionDispatchTestHarness,
) -> Result<QpcTicks, String> {
    let now = clock
        .now()
        .map_err(|error| format!("QPC read for playback epoch: {error:?}"))?;
    let pre_roll = clock
        .duration_from_us(1_000_000)
        .map_err(|error| format!("QPC pre-roll conversion: {error:?}"))?;
    let epoch = QpcTicks::from_raw(now.as_u64().saturating_sub(pre_roll.as_u64()));
    harness.reanchor_playback_clock_for_test(epoch)?;
    Ok(epoch)
}

fn run_no_pause_control(clock: QpcClock, frequency_hz: u64) -> Result<Value, String> {
    let mut harness =
        ProductionDispatchTestHarness::new_lifecycle_sequence_for_test(frequency_hz, false)?;
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0, 0, 0]);
    harness.attach_executor_for_test()?;
    let epoch = new_epoch(clock, &mut harness)?;
    let mut stages = vec![accounting(&harness, "before_first_down")];
    let expected_packets = [(0, 1), (1, 0), (0, 1), (1, 0)];

    for (index, expected) in expected_packets.into_iter().enumerate() {
        let packet = dispatch_next(&mut harness, clock, epoch)?
            .ok_or_else(|| format!("scheduler finished before packet {index}"))?;
        let observed = (packet.up_mask, packet.down_mask);
        if observed != expected {
            return Err(format!(
                "live scheduler packet {index} was {observed:?}, expected {expected:?}"
            ));
        }
        stages.push(accounting(
            &harness,
            match index {
                0 => "after_first_down",
                1 => "after_first_up",
                2 => "after_second_down",
                _ => "after_final_up",
            },
        ));
    }

    let packet_evidence = packet_capture_evidence(&packets)?;
    let attempts = packet_evidence.len();
    let accounting = harness.generation_accounting_for_test();
    let final_obligation = harness.release_obligation_mask_for_test();
    let passed = attempts == 4
        && accounting.activated == 2
        && accounting.released == 2
        && accounting.cancelled == 0
        && accounting.active == 0
        && final_obligation == 0;
    if !passed {
        return Err("no-pause production lifecycle accounting failed".into());
    }

    Ok(json!({
        "pause_requested": false,
        "sender_attempts": attempts,
        "musical_sender_attempts": attempts,
        "cleanup_sender_attempts": 0,
        "total_mock_send_attempts": attempts,
        "packets": packet_evidence,
        "accounting_stages": stages,
        "final_release_obligation_mask": final_obligation,
        "generation_accounting_owner": "live_scheduler",
        "passed": passed,
    }))
}

fn run_pause_resume(clock: QpcClock, frequency_hz: u64) -> Result<Value, String> {
    use sky_dispatch_core::clock::PauseReason;

    let mut harness =
        ProductionDispatchTestHarness::new_lifecycle_sequence_for_test(frequency_hz, false)?;
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    let cleanup_packets = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test()?;
    let initial_epoch = new_epoch(clock, &mut harness)?;
    let mut stages = vec![accounting(&harness, "before_first_down")];
    let first = dispatch_next(&mut harness, clock, initial_epoch)?
        .ok_or("scheduler finished before Pause control Down")?;
    if (first.up_mask, first.down_mask) != (0, 1) {
        return Err("Pause control did not begin with the live scheduler Down".into());
    }
    stages.push(accounting(&harness, "after_initial_down"));

    harness.set_pause_requested_for_test(true);
    let pause_at = clock
        .now()
        .map_err(|error| format!("QPC read before Pause commit: {error:?}"))?;
    harness
        .commit_pause_for_test(PauseReason::Manual, pause_at)
        .map_err(|error| format!("Pause control commit failed: {error}"))?;
    harness.apply_scheduler_resumable_lifecycle_transition_for_test()?;
    let pause_cleanup_records = cleanup_capture_records(&cleanup_packets)?;
    if pause_cleanup_records.is_empty()
        || harness.release_obligation_mask_for_test() != 0
        || !harness.is_paused_for_test()
    {
        return Err("Pause did not finish verified cleanup while remaining paused".into());
    }
    let attempts_while_paused = packet_capture_evidence(&packets)?.len().saturating_sub(1);
    if attempts_while_paused != 0 {
        return Err("a musical packet was sent before Resume committed".into());
    }
    stages.push(accounting(&harness, "after_pause_cleanup"));

    let resume_commit_qpc = clock
        .now()
        .map_err(|error| format!("QPC read at Resume commit: {error:?}"))?;
    harness.set_pause_requested_for_test(false);
    let resumed_epoch = harness
        .commit_resume_for_test(PauseReason::Manual, resume_commit_qpc)
        .map_err(|error| format!("Resume control commit failed: {error}"))?;
    if harness.is_paused_for_test() {
        return Err("Resume control did not close the manual pause".into());
    }
    let (up_mask, down_chord, timing_window) =
        match harness.evaluate_live_scheduler_for_test(resume_commit_qpc, resumed_epoch)? {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
                ..
            } => (up_mask, down_chord, timing_window),
            decision => {
                return Err(format!(
                    "scheduler did not admit its authored Down after Resume: {decision:?}"
                ));
            }
        };
    if up_mask != 0 || timing_window.packet_not_before_qpc > resume_commit_qpc {
        return Err(
            "fresh Resume admission added a physical deadline after the authored target was due"
                .into(),
        );
    }
    let candidate = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)?
        .ok_or("resumed Down did not stage after committed Resume")?;
    let staged_window = harness.staged_candidate_timing_window_for_test(&candidate);
    if staged_window.authored_target_qpc > resume_commit_qpc
        || staged_window.packet_not_before_qpc > resume_commit_qpc
    {
        return Err("staging changed the fresh authored Resume admission window".into());
    }
    let sender_start = clock
        .now()
        .map_err(|error| format!("QPC read before resumed mock send: {error:?}"))?;
    harness.set_mock_sender_start_qpc_for_test(sender_start);
    if sender_start < resume_commit_qpc
        || !matches!(
            harness.dispatch_staged_candidate_for_test(candidate, false),
            StagedDispatchOutcome::Dispatched
        )
    {
        return Err("resumed production packet did not dispatch after Resume committed".into());
    }
    stages.push(accounting(&harness, "after_resumed_down"));

    harness.apply_scheduler_resumable_lifecycle_transition_for_test()?;
    stages.push(accounting(&harness, "after_final_cleanup"));
    let packet_evidence = packet_capture_evidence(&packets)?;
    let attempts = packet_evidence.len();
    let initial_down_completion = packet_evidence
        .first()
        .and_then(|packet| packet["completed_qpc_ticks"].as_u64())
        .ok_or("initial Down capture has no completion QPC")?;
    let resumed_packet = packet_evidence
        .get(1)
        .ok_or("resumed Down capture is missing")?;
    let resumed_sender_start = resumed_packet["started_qpc_ticks"]
        .as_u64()
        .ok_or("resumed Down capture has no start QPC")?;
    if initial_down_completion > pause_at.as_u64()
        || resumed_sender_start != sender_start.as_u64()
        || resumed_sender_start < resume_commit_qpc.as_u64()
    {
        return Err(
            "captured send chronology does not follow Pause cleanup and Resume commit".into(),
        );
    }
    let all_cleanup_records = cleanup_capture_records(&cleanup_packets)?;
    if all_cleanup_records.len() <= pause_cleanup_records.len() {
        return Err("final cleanup capture is missing after the resumed Down".into());
    }
    let final_cleanup_records = &all_cleanup_records[pause_cleanup_records.len()..];
    let cleanup_attempts = all_cleanup_records.len();
    let final_obligation = harness.release_obligation_mask_for_test();
    if attempts != 2 || final_obligation != 0 {
        return Err("pause/resume sender count or final release obligation failed".into());
    }

    Ok(json!({
        "pause_requested": true,
        "sender_attempts": attempts,
        "musical_sender_attempts": attempts,
        "cleanup_sender_attempts": cleanup_attempts,
        "total_mock_send_attempts": attempts + cleanup_attempts,
        "packets": packet_evidence,
        "cleanup_captures": {
            "pause": cleanup_capture_evidence(&pause_cleanup_records),
            "final": cleanup_capture_evidence(final_cleanup_records),
            "total_attempts": cleanup_attempts,
        },
        "generation_accounting_owner": "live_scheduler",
        "accounting_stages": stages,
        "verified_cleanup_before_resume": true,
        "send_attempts_while_paused": attempts_while_paused,
        "resume_control_committed": true,
        "resume_commit_qpc_ticks": resume_commit_qpc.as_u64(),
        "resumed_epoch_qpc_ticks": resumed_epoch.as_u64(),
        "resumed_authored_target_qpc_ticks": staged_window.authored_target_qpc.as_u64(),
        "candidate_packet_not_before_qpc_ticks": staged_window.packet_not_before_qpc.as_u64(),
        "resumed_sender_start_qpc_ticks": resumed_sender_start,
        "final_release_obligation_mask": final_obligation,
        "passed": true,
    }))
}

fn source_identity_args() -> Result<(String, String), String> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--source-sha") {
        return Err("expected --source-sha <sha> --source-tree <tree>".into());
    }
    let sha = args.next().ok_or("missing source SHA")?;
    if args.next().as_deref() != Some("--source-tree") {
        return Err("expected --source-tree after source SHA".into());
    }
    let tree = args.next().ok_or("missing source tree SHA")?;
    if args.next().is_some() {
        return Err("unexpected arguments".into());
    }
    for (name, value) in [("source SHA", &sha), ("source tree SHA", &tree)] {
        if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{name} must be a full 40-character Git object ID"));
        }
    }
    Ok((sha, tree))
}

fn run() -> Result<Value, String> {
    let (source_sha, source_tree_sha) = source_identity_args()?;
    let frequency_hz =
        qpc_frequency_checked().map_err(|error| format!("QPC frequency unavailable: {error:?}"))?;
    let clock = QpcClock::from_frequency_hz(
        std::num::NonZeroU64::new(frequency_hz).ok_or("zero QPC frequency")?,
    );
    let normal = run_no_pause_control(clock, frequency_hz)?;
    let paused = run_pause_resume(clock, frequency_hz)?;
    let report = json!({
        "schema_version": 4,
        "report_kind": "dispatch_lifecycle",
        "result": "PASS",
        "provenance": {
            "source_sha": source_sha,
            "source_tree_sha": source_tree_sha,
            "execution": "offline_test_support",
            "transport_kind": "deterministic_mock",
            "native_sendinput_calls": 0,
        },
        "timing": {
            "fps": 60,
            "frame_us": FRAME_US,
            "hold_frames": 1,
            "timing_margin_us": MARGIN_US,
            "min_hold_us": HOLD_US,
            "qpc_frequency_hz": frequency_hz,
        },
        "live_scheduler": {
            "generation_accounting_owner": "live_scheduler",
            "no_pause_control": normal,
            "pause_cleanup_resume": paused,
        },
    });
    Ok(report)
}

fn main() {
    match run() {
        Ok(report) => println!("{report}"),
        Err(error) => {
            eprintln!("dispatch lifecycle evidence failed: {error}");
            std::process::exit(1);
        }
    }
}
