#![forbid(unsafe_code)]

use serde_json::{Value, json};
use sky_dispatch_core::executor::DispatchDecision;
use sky_dispatch_core::time::QpcTicks;
use sky_dispatch_win32::clock::{QpcClock, qpc_frequency_checked};
use sky_dispatch_win32::input::PhysicalPacket;
use sky_player::engine::StagedDispatchOutcome;
use sky_player::engine::dispatch_primitives::ProductionDispatchTestHarness;
use std::time::Duration;

const FRAME_US: u64 = 16_667;
const MARGIN_US: u64 = 500;
const GAP_US: u64 = FRAME_US + MARGIN_US;

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
            } => {
                let down_chord: Option<sky_dispatch_core::executor::DownChordMetadata> =
                    down_chord.map(Into::into);
                let down_mask = down_chord.as_ref().map_or(0, |chord| chord.down_mask);
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
    let mut observed_packets = Vec::with_capacity(expected_packets.len());

    for (index, expected) in expected_packets.into_iter().enumerate() {
        let packet = dispatch_next(&mut harness, clock, epoch)?
            .ok_or_else(|| format!("scheduler finished before packet {index}"))?;
        let observed = (packet.up_mask, packet.down_mask);
        if observed != expected {
            return Err(format!(
                "live scheduler packet {index} was {observed:?}, expected {expected:?}"
            ));
        }
        observed_packets.push(json!({"up_mask": packet.up_mask, "down_mask": packet.down_mask}));
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

    let captured = packets
        .lock()
        .map_err(|_| "mock transport capture poisoned".to_string())?;
    let attempts = captured.len();
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
        "packets": observed_packets,
        "accounting_stages": stages,
        "final_release_obligation_mask": final_obligation,
        "generation_accounting_owner": "live_scheduler",
        "passed": passed,
    }))
}

fn run_pause_resume(clock: QpcClock, frequency_hz: u64) -> Result<Value, String> {
    let mut harness =
        ProductionDispatchTestHarness::new_lifecycle_sequence_for_test(frequency_hz, false)?;
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    let (_, cleanup_completion) =
        harness.configure_mock_cleanup_emitter_with_completion_qpc_for_test();
    harness.attach_executor_for_test()?;
    let epoch = new_epoch(clock, &mut harness)?;
    let first = dispatch_next(&mut harness, clock, epoch)?
        .ok_or("scheduler finished before pause control Down")?;
    if (first.up_mask, first.down_mask) != (0, 1) {
        return Err("pause control did not begin with the live scheduler Down".into());
    }
    let mut stages = vec![accounting(&harness, "after_initial_down")];

    harness.set_pause_requested_for_test(true);
    harness.apply_scheduler_resumable_lifecycle_transition_with_post_cleanup_qpc_for_test(
        || {
            Ok(QpcTicks::from_raw(
                cleanup_completion.load(std::sync::atomic::Ordering::Acquire),
            ))
        },
    )?;
    let cleanup_qpc =
        QpcTicks::from_raw(cleanup_completion.load(std::sync::atomic::Ordering::Acquire));
    let barrier = harness
        .resume_barrier_qpc_for_test()
        .ok_or("verified cleanup did not establish a resume barrier")?;
    let timing = harness.dispatch_timing_config_for_test()?;
    let gap_ticks = harness
        .qpc_duration_from_us_for_test(timing.min_release_gap_us)?
        .as_u64();
    if timing.min_release_gap_us != GAP_US
        || barrier.as_u64()
            != cleanup_qpc
                .as_u64()
                .checked_add(gap_ticks)
                .ok_or("barrier overflow")?
        || harness.release_obligation_mask_for_test() != 0
    {
        return Err("pause cleanup did not establish verified completion + G".into());
    }
    stages.push(accounting(&harness, "after_pause_cleanup"));

    let g_minus_one = QpcTicks::from_raw(barrier.as_u64() - 1);
    let g_minus_one_decision = harness.evaluate_live_scheduler_for_test(g_minus_one, epoch)?;
    let g_minus_one_waits = matches!(
        g_minus_one_decision,
        DispatchDecision::Wait { deadline_qpc } if deadline_qpc == barrier
    );
    if !g_minus_one_waits {
        return Err("live scheduler did not wait at G-1".into());
    }
    harness.set_pause_requested_for_test(false);
    let (up_mask, down_chord, timing_window) =
        match harness.evaluate_live_scheduler_for_test(barrier, epoch)? {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
            } => (up_mask, down_chord.map(Into::into), timing_window),
            decision => {
                return Err(format!(
                    "live scheduler did not allow exact G: {decision:?}"
                ));
            }
        };
    if up_mask != 0 || timing_window.packet_not_before_qpc != barrier {
        return Err("exact-G scheduler decision did not preserve the barrier window".into());
    }
    let candidate = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)?
        .ok_or("exact-G Down did not stage")?;
    let staged_window = harness.staged_candidate_timing_window_for_test(&candidate);
    if staged_window.packet_not_before_qpc != barrier {
        return Err("production staging changed cleanup completion + G".into());
    }
    wait_until(clock, barrier)?;
    let sender_start = clock
        .now()
        .map_err(|error| format!("QPC read before resumed mock send: {error:?}"))?;
    harness.set_mock_sender_start_qpc_for_test(sender_start);
    if !matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Dispatched
    ) {
        return Err("exact-G resumed production packet did not dispatch".into());
    }
    stages.push(accounting(&harness, "after_resumed_down"));

    harness.apply_scheduler_resumable_lifecycle_transition_with_post_cleanup_qpc_for_test(
        || {
            Ok(QpcTicks::from_raw(
                cleanup_completion.load(std::sync::atomic::Ordering::Acquire),
            ))
        },
    )?;
    stages.push(accounting(&harness, "after_final_cleanup"));
    let attempts = packets
        .lock()
        .map_err(|_| "pause/resume packet capture poisoned".to_string())?
        .len();
    let final_obligation = harness.release_obligation_mask_for_test();
    if attempts != 2 || final_obligation != 0 {
        return Err("pause/resume sender count or final release obligation failed".into());
    }

    Ok(json!({
        "pause_requested": true,
        "sender_attempts": attempts,
        "generation_accounting_owner": "live_scheduler",
        "accounting_stages": stages,
        "cleanup_completion_qpc_ticks": cleanup_qpc.as_u64(),
        "min_release_gap_us": timing.min_release_gap_us,
        "min_release_gap_ticks": gap_ticks,
        "resume_barrier_qpc_ticks": barrier.as_u64(),
        "candidate_packet_not_before_qpc_ticks": staged_window.packet_not_before_qpc.as_u64(),
        "g_minus_one_decision": "wait",
        "exact_g_decision": "send",
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
        "schema_version": 2,
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
            "min_hold_us": GAP_US,
            "min_release_gap_us": GAP_US,
            "qpc_frequency_hz": frequency_hz,
            "min_release_gap_ticks": clock.duration_from_us(GAP_US)
                .map_err(|error| format!("G conversion: {error:?}"))?.as_u64(),
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
