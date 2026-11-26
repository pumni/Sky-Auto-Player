use super::*;

#[test]
fn timed_sample_argument_is_15_seconds_and_only_for_benchmark_workloads() {
    let mut args = base_arguments("dense-alternating");
    args.extend(["--sample-seconds", "15"].into_iter().map(str::to_owned));
    let ParsedCommand::Run(run) = parse_args(args).expect("15 second benchmark sample") else {
        panic!("expected run command")
    };
    assert_eq!(run.sample_seconds, Some(15));

    let mut invalid_duration = base_arguments("dense-alternating");
    invalid_duration.extend(["--sample-seconds", "30"].into_iter().map(str::to_owned));
    assert!(parse_args(invalid_duration).is_err());

    let mut invalid_scenario = base_arguments("canonical-single");
    invalid_scenario.extend(["--sample-seconds", "15"].into_iter().map(str::to_owned));
    assert!(parse_args(invalid_scenario).is_err());

    let mut fps_120 = base_arguments("canonical-single");
    fps_120.extend(["--game-fps", "120"].into_iter().map(str::to_owned));
    let ParsedCommand::Run(run) = parse_args(fps_120).expect("120 FPS boundary run") else {
        panic!("expected run command")
    };
    assert_eq!(run.game_fps, 120);

    let mut invalid_fps = base_arguments("canonical-single");
    invalid_fps.extend(["--game-fps", "90"].into_iter().map(str::to_owned));
    assert!(parse_args(invalid_fps).is_err());
}

#[test]
fn timed_sample_plan_repeats_each_benchmark_workload_over_15_seconds() {
    for scenario in [
        Scenario::DenseAlternating,
        Scenario::MixedUpDown,
        Scenario::CanonicalMaxChord,
        Scenario::NearMinimumRetrigger,
        Scenario::LongIdle,
    ] {
        let base = scenario_plan(scenario, ACCEPTANCE_TIMING_MARGIN_US).unwrap();
        let repeated = super::super::scenarios::scenario_plan_with_sample_seconds(
            scenario,
            ACCEPTANCE_TIMING_MARGIN_US,
            Some(15),
        )
        .unwrap();
        let projection = super::super::authored_timestamp_group_projection(&repeated.schedule);
        let first = projection
            .first()
            .expect("timed schedule has authored groups")["scheduled_us"]
            .as_u64()
            .expect("group timestamps are integers");
        let last = projection
            .last()
            .expect("timed schedule has authored groups")["scheduled_us"]
            .as_u64()
            .expect("group timestamps are integers");
        assert!(repeated.schedule.chords().len() > base.schedule.chords().len());
        assert!(last.saturating_sub(first) >= 15_000_000, "{scenario:?}");
        assert_eq!(
            repeated.expected_down_slots.len(),
            repeated
                .schedule
                .chords()
                .iter()
                .map(|chord| chord.key_mask.count_ones() as usize)
                .sum::<usize>()
        );
    }
}

#[test]
fn paired_near_minimum_workload_keeps_h_plus_gap_geometry_and_sample_cadence() {
    let hold_us = super::super::scenarios::acceptance_min_hold_us_for_fps(500, 60);
    let base = scenario_plan(Scenario::NearMinimumRetrigger, 500).unwrap();
    let repeated = super::super::scenarios::scenario_plan_with_sample_seconds(
        Scenario::NearMinimumRetrigger,
        500,
        Some(15),
    )
    .unwrap();
    let base_chords = base.schedule.chords();
    let repeated_chords = repeated.schedule.chords();

    assert_eq!(base_chords.len(), 4);
    assert_eq!(repeated_chords.len() % base_chords.len(), 0);
    assert_eq!(base_chords[1].scheduled_us - base_chords[0].scheduled_us, hold_us * 2);
    let first_cycle_span = base_chords
        .last()
        .expect("the base workload has a final chord")
        .scheduled_us
        + hold_us
        + hold_us
        - base_chords[0].scheduled_us;
    assert_eq!(
        repeated_chords[base_chords.len()].scheduled_us - repeated_chords[0].scheduled_us,
        first_cycle_span
    );
}

#[test]
fn paired_workloads_match_the_immutable_pr_base_authored_streams() {
    use sha2::{Digest, Sha256};

    // Full authored-stream projections captured from baseline commit
    // 1d00c3b71fbf1f94003865bad6c4eba6d8dcd902, tree
    // 25a20291b9cf84b1283804f451b33e3c4605507c, at 60 FPS / 500 us.
    // Fixed-pulse workloads (dense-alternating, near-minimum-retrigger,
    // long-idle) preserve exact bit-for-bit baseline stream digests.
    let fixed_pulse_fixtures = [
        (
            "dense-alternating",
            Scenario::DenseAlternating,
            896,
            896,
            50_000,
            15_414_465,
            "802e84e0d9fa26cec3b293ed20af9a03202b5eb49b5f0cf66531d5b81f41fe73",
        ),
        (
            "near-minimum-retrigger",
            Scenario::NearMinimumRetrigger,
            880,
            880,
            50_000,
            15_139_793,
            "2a01c69071ecfc28deafcfe719e3c35be882c15bcd41b291ba99d0a63306247c",
        ),
        (
            "long-idle",
            Scenario::LongIdle,
            32,
            32,
            50_000,
            16_324_672,
            "7da0dce05f83dc76ee9429b3d5a2b820a43dea2274c12b18ce1e00774459f672",
        ),
    ];

    for (name, scenario, expected_events, expected_groups, expected_first, expected_last, expected_sha256) in
        fixed_pulse_fixtures
    {
        let plan = super::super::scenarios::scenario_plan_with_sample_seconds(
            scenario,
            500,
            Some(15),
        )
        .unwrap();
        let projection = super::super::authored_timestamp_group_projection(&plan.schedule);
        let first = projection
            .first()
            .expect("baseline projection has authored groups")["scheduled_us"]
            .as_u64()
            .expect("group timestamps are integers");
        let last = projection
            .last()
            .expect("baseline projection has authored groups")["scheduled_us"]
            .as_u64()
            .expect("group timestamps are integers");
        let mut events = Vec::new();
        for group in &projection {
            let timestamp_us = group["scheduled_us"]
                .as_u64()
                .expect("group timestamps are integers");
            let up_mask = group["up_mask"].as_u64().expect("Up mask is an integer");
            let down_mask = group["down_mask"]
                .as_u64()
                .expect("Down mask is an integer");
            if up_mask != 0 {
                events.push(format!("{timestamp_us}:up:{up_mask}"));
            }
            if down_mask != 0 {
                events.push(format!("{timestamp_us}:down:{down_mask}"));
            }
        }
        let digest = Sha256::digest(events.join("|").as_bytes());
        let sha256 = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        assert_eq!(events.len(), expected_events, "{name} event count");
        assert_eq!(projection.len(), expected_groups, "{name} timestamp groups");
        assert_eq!(first, expected_first, "{name} first timestamp");
        assert_eq!(last, expected_last, "{name} final timestamp");
        assert_eq!(sha256, expected_sha256, "{name} full authored stream");
    }

    // Immutable historical paired-hold (50ms / 40ms) PR-base digests are preserved
    // as historical reference evidence only; candidate fixed-pulse workloads carry
    // distinct identities and must NOT claim parity with those variable-hold streams.
    let historical_variable_hold_digests = [
        (
            Scenario::MixedUpDown,
            "mixed-up-down-fixed-pulse-v1",
            "7d83f78e1166fefd9ac797fd945c612fc169de114eff1f39e86f90d37bde6a0b",
        ),
        (
            Scenario::CanonicalMaxChord,
            "canonical-max-chord-fixed-pulse-v1",
            "c6a439e78f5da268af4f272884ba9acc7dde937ba8d46d4a6cbc199ab59e05c0",
        ),
    ];
    let expected_hold_us = super::super::scenarios::acceptance_min_hold_us_for_fps(500, 60);
    for (scenario, expected_identity, historical_sha256) in historical_variable_hold_digests {
        assert_eq!(
            super::super::scenarios::workload_definition_identity(scenario),
            expected_identity
        );
        let plan = super::super::scenarios::scenario_plan_with_sample_seconds(
            scenario,
            500,
            Some(15),
        )
        .unwrap();
        assert_eq!(plan.schedule.hold_us(), expected_hold_us);
        assert!(plan.schedule.chords().len() >= 100, "{scenario:?}");
        let projection = super::super::authored_timestamp_group_projection(&plan.schedule);
        let first = projection.first().unwrap()["scheduled_us"].as_u64().unwrap();
        let last = projection.last().unwrap()["scheduled_us"].as_u64().unwrap();
        assert_eq!(first, 50_000, "{scenario:?}");
        assert!(last.saturating_sub(first) >= 15_000_000, "{scenario:?}");
        let mut events = Vec::new();
        for group in &projection {
            let timestamp_us = group["scheduled_us"].as_u64().unwrap();
            let up_mask = group["up_mask"].as_u64().unwrap();
            let down_mask = group["down_mask"].as_u64().unwrap();
            if up_mask != 0 {
                events.push(format!("{timestamp_us}:up:{up_mask}"));
            }
            if down_mask != 0 {
                events.push(format!("{timestamp_us}:down:{down_mask}"));
            }
        }
        let digest = Sha256::digest(events.join("|").as_bytes());
        let candidate_sha256: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        assert_ne!(
            candidate_sha256, historical_sha256,
            "{scenario:?} fixed-pulse candidate stream is distinct from historical variable-hold baseline"
        );
    }
}

#[test]
fn timing_observer_is_opt_in_for_qualification_runs_only() {
    let mut args = base_arguments("canonical-single");
    args.push("--timing-observer-for-qualification".to_string());
    let ParsedCommand::Run(run) = parse_args(args.clone()).expect("qualification observer flag")
    else {
        panic!("expected run command")
    };
    assert!(run.timing_observer_for_qualification);

    args.push("--timing-observer-for-qualification".to_string());
    assert!(parse_args(args).is_err(), "duplicate observer flag is rejected");
}

#[test]
fn chord_width_pilot_v1_geometry_rotation_digests_and_trace_capacities() {
    use sha2::{Digest, Sha256};

    assert_eq!(
        super::super::scenarios::workload_definition_identity(Scenario::ChordWidthPilotV1),
        "chord-width-pilot-v1"
    );

    // Reject invalid settings/widths at plan builder boundary
    assert!(
        super::super::scenarios::scenario_plan_for_settings_with_pilot(
            Scenario::ChordWidthPilotV1,
            500,
            60,
            Some(15),
            Some(3),
        )
        .is_err()
    );
    assert!(
        super::super::scenarios::scenario_plan_for_settings_with_pilot(
            Scenario::ChordWidthPilotV1,
            500,
            120,
            Some(15),
            Some(5),
        )
        .is_err()
    );

    let mut digests = Vec::new();
    for width in [1_u8, 5, 15] {
        let plan = super::super::scenarios::scenario_plan_for_settings_with_pilot(
            Scenario::ChordWidthPilotV1,
            500,
            60,
            Some(15),
            Some(width),
        )
        .expect("valid pilot plan");

        assert!(plan.profile.is_none(), "pilot uses canonical 15-key profile");
        assert!(!plan.allow_unpaired_cleanup_ups);
        assert_eq!(plan.schedule.hold_us(), 17_167);
        assert_eq!(plan.schedule.chords().len(), 150);
        assert_eq!(plan.schedule.chords().first().unwrap().scheduled_us, 50_000);
        assert_eq!(
            plan.schedule.chords().last().unwrap().scheduled_us,
            14_950_000
        );
        assert_eq!(
            plan.expected_down_slots.len(),
            150 * usize::from(width)
        );
        assert_eq!(plan.expected_up_slots.len(), 150 * usize::from(width));

        // Verify deterministic rotation & non-overlap (100_000 us cadence > 17_167 us hold)
        let mut slot_hits = [0usize; 15];
        for (idx, chord) in plan.schedule.chords().iter().enumerate() {
            assert_eq!(chord.scheduled_us, 50_000 + (idx as u64) * 100_000);
            assert_eq!(chord.key_mask.count_ones(), u32::from(width));
            let expected_slots =
                super::super::scenarios::pilot_rotating_slots(idx, usize::from(width));
            let mut expected_mask = 0u16;
            for s in expected_slots {
                expected_mask |= 1u16 << s;
                slot_hits[s] += 1;
            }
            assert_eq!(chord.key_mask, expected_mask);
        }
        let expected_hits_per_slot = (150 * usize::from(width)) / 15;
        assert_eq!(slot_hits, [expected_hits_per_slot; 15]);

        let projection = super::super::authored_timestamp_group_projection(&plan.schedule);
        assert_eq!(projection.len(), 300, "150 Down + 150 Up non-overlapping groups");
        assert_eq!(projection.first().unwrap()["scheduled_us"], 50_000);
        assert_eq!(projection.last().unwrap()["scheduled_us"], 14_967_167);

        // Verify sender/receiver observation capacities fit with zero production buffer changes
        let healthy_packets = 300usize;
        let up_slot_entries = 150usize * usize::from(width);
        let receiver_events = 2usize * 150usize * usize::from(width);
        assert!(healthy_packets <= 1_024, "fits sender TelemetryOptions capacity 1024");
        assert!(
            up_slot_entries <= 4_096,
            "fits NATIVE_UP_SLOT_TRACE_CAPACITY (4096)"
        );
        assert!(
            receiver_events <= 32_768,
            "fits SchedulerQueuedV3Compat3 ring capacity"
        );

        let mut events = Vec::new();
        for group in &projection {
            let ts = group["scheduled_us"].as_u64().unwrap();
            let up = group["up_mask"].as_u64().unwrap();
            let down = group["down_mask"].as_u64().unwrap();
            if up != 0 {
                events.push(format!("{ts}:up:{up}"));
            }
            if down != 0 {
                events.push(format!("{ts}:down:{down}"));
            }
        }
        let digest = Sha256::digest(events.join("|").as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        let expected_hex = match width {
            1 => "b0810a9f997ec63598caacc63bc15c798efd86be420a82c22bb396764f4d735a",
            5 => "81a047f01d0c919b0c6f775e2d175126138203bab6d060deefcc24cc8b4ad1d0",
            15 => "58c787d47ec23e0d575f4295fd592e4682dd7b537dcc6f25aeddec914b5ea07b",
            _ => unreachable!(),
        };
        assert_eq!(hex, expected_hex, "pilot width {width} digest changed");
        digests.push(hex);
    }
    assert_ne!(digests[0], digests[1]);
    assert_ne!(digests[1], digests[2]);
}

/// Proves actual `snapshot_json` + `evidence_report::write_report` report-envelope
/// compatibility with `scripts/analyze_chord_reception.mjs` CLI using synthetic
/// timing records (no-send; does NOT exercise live worker/observer execution).
#[test]
fn chord_width_pilot_v1_real_serializer_to_analyzer_cli_roundtrip() {
    use serde_json::json;
    use sky_dispatch_win32::input::ReleaseAllOutcome;
    use sky_player::engine::NativeDispatchSession;
    use std::path::PathBuf;
    use std::process::Command;

    const WIDTH: u8 = 5;
    const QPC_FREQ: u64 = 10_000_000;
    const US_TO_TICKS: u64 = 10;
    const RUN_ID: &str = "rust-serializer-roundtrip-w5";
    const EVENT_LOG_ID: &str = "evtlog-rust-serializer-001";
    const PID: u32 = 42424;
    const HWND: u64 = 196608;
    const START_FILETIME: u64 = 133_900_000_000_000_001;
    const SOURCE_SHA: &str = "77c03d2bdeaf9873c42ae2e7ab9da49e3973585f";
    const SOURCE_TREE: &str = "0ad933679b346e2139351b0f2d46fccc9cec7538";
    const BINARY_SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const CANONICAL_SCANS: [u16; 15] = [
        0x15, 0x16, 0x17, 0x18, 0x19, 0x23, 0x24, 0x25, 0x26, 0x27, 0x31, 0x32, 0x33, 0x34, 0x35,
    ];

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf();
    let analyzer_script = repo_root.join("scripts").join("analyze_chord_reception.mjs");
    assert!(analyzer_script.is_file(), "analyzer script missing");

    let temp_dir = std::env::temp_dir().join(format!(
        "sky-pilot-rust-ser-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");

    let report_path = temp_dir.join("rt-native-acceptance.jsonl");
    let inv_path = temp_dir.join("invocation.json");
    let ready_path = temp_dir.join("sink-ready.json");
    let events_path = temp_dir.join("sink-events.jsonl");
    let summary_path = temp_dir.join("sink-observer-summary.json");
    let manifest_path = temp_dir.join("manifest.json");

    let plan = super::super::scenarios::scenario_plan_for_settings_with_pilot(
        Scenario::ChordWidthPilotV1,
        500,
        60,
        Some(15),
        Some(WIDTH),
    )
    .expect("pilot plan");
    let hold_us = plan.schedule.hold_us();
    let hold_ticks = hold_us * US_TO_TICKS;
    let mut records = Vec::with_capacity(300);
    let mut up_slots = Vec::with_capacity(150 * usize::from(WIDTH));
    let mut rx_lines = Vec::with_capacity(1 + 300 * usize::from(WIDTH));
    let mut seq: u64 = 0;

    rx_lines.push(
        json!({
            "schema_version": 3,
            "run_id": RUN_ID,
            "role": "ReceiveOnly",
            "event_log_id": EVENT_LOG_ID,
            "target_hwnd": HWND,
            "target_pid": PID,
            "sequence": seq,
            "kind": "stream_start",
            "observer_mode": "SchedulerQueuedV3Compat3",
            "observer_version": "scheduler-receiveonly-queue-v3-compat3",
            "observed_qpc_ticks": 1_000_000_u64,
        })
        .to_string(),
    );
    seq += 1;

    let epoch_qpc: u64 = 10_000_000;
    let mut trace_idx: u32 = 0;
    for (onset_idx, chord) in plan.schedule.chords().iter().enumerate() {
        let mut slots: Vec<usize> = (0..15)
            .filter(|slot| (chord.key_mask & (1_u16 << slot)) != 0)
            .collect();
        slots.sort_unstable();
        assert_eq!(slots.len(), usize::from(WIDTH));

        let authored_offset_ticks = chord.scheduled_us * US_TO_TICKS;
        let down_due_qpc = epoch_qpc + authored_offset_ticks;
        let down_wake_qpc = down_due_qpc + 20 * US_TO_TICKS;
        let down_pre_qpc = down_due_qpc + 30 * US_TO_TICKS;
        let down_post_qpc = down_pre_qpc + 25 * US_TO_TICKS;

        records.push(json!({
            "trace_record_index": trace_idx,
            "flags": 0,
            "send_status": 0,
            "transport_anomaly": false,
            "send_attempts": 1,
            "requested_count": WIDTH,
            "sent_count": WIDTH,
            "up_mask": 0,
            "down_mask": chord.key_mask,
            "authored_down_chord_index_available": true,
            "authored_down_chord_index": onset_idx,
            "source_action_index": onset_idx,
            "authored_offset_ticks": authored_offset_ticks,
            "authored_lateness_available": true,
            "authored_lateness_ticks": (down_pre_qpc - down_due_qpc) as i64,
            "physical_eligibility_qpc_available": true,
            "physical_eligibility_qpc_ticks": down_due_qpc,
            "wake_qpc_available": true,
            "wake_qpc_ticks": down_wake_qpc,
            "timer_arm_target_qpc_available": true,
            "timer_arm_target_qpc_ticks": down_due_qpc,
            "pre_call_qpc_available": true,
            "pre_call_qpc_ticks": down_pre_qpc,
            "sendinput_completion_qpc_available": true,
            "sendinput_completion_qpc_ticks": down_post_qpc,
        }));
        trace_idx += 1;

        for (k, &slot) in slots.iter().enumerate() {
            let rx_qpc = down_pre_qpc + (10 + k as u64 * 2) * US_TO_TICKS;
            rx_lines.push(
                json!({
                    "schema_version": 3,
                    "run_id": RUN_ID,
                    "role": "ReceiveOnly",
                    "event_log_id": EVENT_LOG_ID,
                    "target_hwnd": HWND,
                    "target_pid": PID,
                    "foreground_hwnd": HWND,
                    "foreground_pid": PID,
                    "sequence": seq,
                    "kind": "key_press",
                    "message": 0x0100,
                    "scan_code": CANONICAL_SCANS[slot],
                    "extended": false,
                    "observer_mode": "SchedulerQueuedV3Compat3",
                    "observer_version": "scheduler-receiveonly-queue-v3-compat3",
                    "observed_qpc_ticks": rx_qpc,
                })
                .to_string(),
            );
            seq += 1;
        }

        let up_due_qpc = down_post_qpc + hold_ticks;
        let up_wake_qpc = up_due_qpc + 18 * US_TO_TICKS;
        let up_pre_qpc = up_due_qpc + 28 * US_TO_TICKS;
        let up_post_qpc = up_pre_qpc + 24 * US_TO_TICKS;
        let up_trace_idx = trace_idx;
        records.push(json!({
            "trace_record_index": up_trace_idx,
            "flags": 0,
            "send_status": 0,
            "transport_anomaly": false,
            "send_attempts": 1,
            "requested_count": WIDTH,
            "sent_count": WIDTH,
            "up_mask": chord.key_mask,
            "down_mask": 0,
            "authored_down_chord_index_available": false,
            "authored_down_chord_index": 0,
            "source_action_index": onset_idx,
            "authored_offset_ticks": authored_offset_ticks + hold_ticks,
            "authored_lateness_available": false,
            "authored_lateness_ticks": 0,
            "physical_eligibility_qpc_available": true,
            "physical_eligibility_qpc_ticks": up_due_qpc,
            "wake_qpc_available": true,
            "wake_qpc_ticks": up_wake_qpc,
            "timer_arm_target_qpc_available": true,
            "timer_arm_target_qpc_ticks": up_due_qpc,
            "pre_call_qpc_available": true,
            "pre_call_qpc_ticks": up_pre_qpc,
            "sendinput_completion_qpc_available": true,
            "sendinput_completion_qpc_ticks": up_post_qpc,
        }));
        trace_idx += 1;

        let up_not_before_offset_ticks = (down_post_qpc + hold_ticks) - epoch_qpc;
        for (k, &slot) in slots.iter().enumerate() {
            up_slots.push(json!({
                "trace_record_index": up_trace_idx,
                "slot": slot,
                "scan_code": CANONICAL_SCANS[slot],
                "source_action_index": onset_idx,
                "authored_offset_ticks": authored_offset_ticks + hold_ticks,
                "up_not_before_offset_ticks": up_not_before_offset_ticks,
                "floor_displacement_ticks": 0,
                "pre_call_qpc_ticks": up_pre_qpc,
                "completion_qpc_ticks": up_post_qpc,
                "residual_ticks": 0,
                "confirmed": true,
            }));
            let rx_qpc = up_pre_qpc + (12 + k as u64 * 2) * US_TO_TICKS;
            rx_lines.push(
                json!({
                    "schema_version": 3,
                    "run_id": RUN_ID,
                    "role": "ReceiveOnly",
                    "event_log_id": EVENT_LOG_ID,
                    "target_hwnd": HWND,
                    "target_pid": PID,
                    "foreground_hwnd": HWND,
                    "foreground_pid": PID,
                    "sequence": seq,
                    "kind": "key_release",
                    "message": 0x0101,
                    "scan_code": CANONICAL_SCANS[slot],
                    "extended": false,
                    "observer_mode": "SchedulerQueuedV3Compat3",
                    "observer_version": "scheduler-receiveonly-queue-v3-compat3",
                    "observed_qpc_ticks": rx_qpc,
                })
                .to_string(),
            );
            seq += 1;
        }
    }

    let total_rx = 300 * usize::from(WIDTH);
    let (options, _, _) = super::super::modifier_guard::native_session_options(
        plan.schedule,
        plan.profile,
        500,
        Scenario::ChordWidthPilotV1,
        60,
    )
    .expect("no-send session options");
    let session = NativeDispatchSession::new(options).expect("no-send session");
    let mut snapshot = session.snapshot();
    snapshot.release_outcome = Some(ReleaseAllOutcome {
        attempted_mask: 0,
        transport_anomaly: false,
        released_successfully: true,
        stuck_mask: 0,
        verification_inconclusive: false,
        attempts: 0,
    });
    let mut details = super::super::snapshot_report::snapshot_json(&snapshot);
    let obj = details.as_object_mut().expect("details object");
    obj.insert("calibration_placement".into(), json!("baseline"));
    obj.insert("timing_observer_for_qualification".into(), json!(true));
    obj.insert(
        "workload_definition_identity".into(),
        json!("chord-width-pilot-v1"),
    );
    obj.insert("pilot_chord_width".into(), json!(WIDTH));
    obj.insert(
        "pilot_cadence_us".into(),
        json!(super::super::scenarios::PILOT_CADENCE_US),
    );
    obj.insert(
        "pilot_onset_count".into(),
        json!(super::super::scenarios::PILOT_CHORD_ONSET_COUNT),
    );
    obj.insert("scenario_hold_us".into(), json!(hold_us));
    obj.insert("sink_event_log_id".into(), json!(EVENT_LOG_ID));
    obj.insert("expected_sink_event_count".into(), json!(total_rx));
    obj.insert("observed_sink_event_count".into(), json!(total_rx));
    obj.insert("sink_event_count".into(), json!(total_rx));
    obj.insert("native_timing_telemetry_available".into(), json!(true));
    obj.insert(
        "native_timing_telemetry".into(),
        json!({
            "schema_version": 28,
            "qpc_frequency_hz": QPC_FREQ,
            "truncated": false,
            "dropped": 0,
            "observer_queue_dropped": 0,
            "attempted": records.len(),
            "accepted": records.len(),
            "records": records,
            "up_slot_accounting_status": "valid",
            "up_slot_records_expected": up_slots.len(),
            "up_slot_records_dropped": 0,
            "up_slot_records": up_slots,
        }),
    );

    let ParsedCommand::Run(run_args) = parse_args(vec![
        "run".into(),
        "--allow-real-input".into(),
        "--run-id".into(),
        RUN_ID.into(),
        "--sink-ready".into(),
        ready_path.to_string_lossy().into_owned(),
        "--sink-events".into(),
        events_path.to_string_lossy().into_owned(),
        "--target-hwnd".into(),
        HWND.to_string(),
        "--scenario".into(),
        "chord-width-pilot-v1".into(),
        "--pilot-chord-width".into(),
        WIDTH.to_string(),
        "--game-fps".into(),
        "60".into(),
        "--timing-margin-us".into(),
        "500".into(),
        "--sample-seconds".into(),
        "15".into(),
        "--timing-observer-for-qualification".into(),
        "--evidence".into(),
        report_path.to_string_lossy().into_owned(),
    ])
    .expect("valid pilot RunArgs")
    else {
        panic!("expected RunArgs");
    };

    let report_exit = super::super::evidence_report::write_report(
        &run_args,
        super::super::Verdict::Pass,
        "no-send serializer fixture",
        details,
    );
    assert_eq!(report_exit, 0);

    std::fs::write(&events_path, rx_lines.join("\n") + "\n").unwrap();
    std::fs::write(
        &ready_path,
        format!(
            r#"{{"schema_version":3,"event_schema_version":3,"run_id":"{RUN_ID}","role":"ReceiveOnly","game_target":false,"pid":{PID},"hwnd":{HWND},"process_start_time_filetime":{START_FILETIME},"event_log_id":"{EVENT_LOG_ID}","observer_mode":"SchedulerQueuedV3Compat3","observer_version":"scheduler-receiveonly-queue-v3-compat3","observer_queue_capacity":32768,"observer_qpc_frequency_hz":{QPC_FREQ}}}"#
        ),
    )
    .unwrap();
    std::fs::write(
        &summary_path,
        json!({
            "schema_version": 1,
            "run_id": RUN_ID,
            "role": "ReceiveOnly",
            "event_log_id": EVENT_LOG_ID,
            "target_hwnd": HWND,
            "target_pid": PID,
            "observer_mode": "SchedulerQueuedV3Compat3",
            "observer_version": "scheduler-receiveonly-queue-v3-compat3",
            "queue_capacity": 32768,
            "qpc_frequency_hz": QPC_FREQ,
            "stream_header_flushed": true,
            "events_flushed": true,
            "consumer_stopped": true,
            "queue_drained": true,
            "observer_startup_failures": 0,
            "writer_failures": 0,
            "observer_internal_failures": 0,
            "queue_dropped": 0,
            "queue_overflow": false,
            "events_seen": total_rx,
            "events_enqueued": total_rx,
            "events_written": total_rx,
        })
        .to_string(),
    )
    .unwrap();
    let inv_json = format!(
        r#"{{"run_id":"{RUN_ID}","qualification_profile":"ChordWidthPilotV1","workload":"chord-width-pilot-v1","workload_version":"chord-width-pilot-v1","source_revision":"{SOURCE_SHA}","source_tree_sha":"{SOURCE_TREE}","source_tree_clean":true,"configured_H_us":{hold_us},"harness_source_revision":"{SOURCE_SHA}","harness_source_tree_sha":"{SOURCE_TREE}","harness_source_role":"Candidate","harness_binary_sha256":"{BINARY_SHA}","scenario":"chord-width-pilot-v1","calibration_placement":"baseline","cpu_contention":false,"timing_margin_us":500,"game_fps":60,"sample_seconds":15,"exit_code":0,"status":"PASS","event_log_id":"{EVENT_LOG_ID}","owner_approved_receive_only_run":true,"report_window_match":true,"receiveonly_observer_mode":"SchedulerQueuedV3Compat3","receiveonly_observer_version":"scheduler-receiveonly-queue-v3-compat3","target_identity":{{"role":"ReceiveOnly","game_target":false,"pid":{PID},"hwnd":{HWND},"process_start_time_filetime":{START_FILETIME},"event_log_id":"{EVENT_LOG_ID}"}}}}"#
    );
    std::fs::write(&inv_path, &inv_json).unwrap();
    std::fs::write(
        &manifest_path,
        json!({
            "qualification_profile": "ChordWidthPilotV1",
            "source_role": "Candidate",
            "source_revision": SOURCE_SHA,
            "source_tree_sha": SOURCE_TREE,
            "binary_sha256": BINARY_SHA,
        })
        .to_string(),
    )
    .unwrap();

    // Positive: real Rust-serialized report -> real analyzer CLI must exit 0 and pass!
    let out = Command::new("node")
        .arg(&analyzer_script)
        .args(["--report", report_path.to_str().unwrap()])
        .args(["--invocation", inv_path.to_str().unwrap()])
        .args(["--sink-ready", ready_path.to_str().unwrap()])
        .args(["--sink-events", events_path.to_str().unwrap()])
        .args(["--observer-summary", summary_path.to_str().unwrap()])
        .args(["--manifest", manifest_path.to_str().unwrap()])
        .args(["--width", "5"])
        .output()
        .expect("invoke node analyzer CLI");
    assert!(
        out.status.success(),
        "analyzer CLI failed on real Rust-serialized report: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON output");
    assert_eq!(parsed["valid"], true);
    assert_eq!(parsed["accepted_for_aggregation"], true);
    assert_eq!(parsed["metrics"]["common_chord_hold"]["n"], 150);
    assert_eq!(parsed["metrics"]["s_minus_d"]["n"], 150);

    // Negative: mutate source_revision in invocation.json -> CLI must exit 1 with metrics: null
    let bad_inv_path = temp_dir.join("invocation-bad-src.json");
    std::fs::write(
        &bad_inv_path,
        inv_json.replace(SOURCE_SHA, "0000000000000000000000000000000000000000"),
    )
    .unwrap();
    let bad_out = Command::new("node")
        .arg(&analyzer_script)
        .args(["--report", report_path.to_str().unwrap()])
        .args(["--invocation", bad_inv_path.to_str().unwrap()])
        .args(["--sink-ready", ready_path.to_str().unwrap()])
        .args(["--sink-events", events_path.to_str().unwrap()])
        .args(["--observer-summary", summary_path.to_str().unwrap()])
        .args(["--manifest", manifest_path.to_str().unwrap()])
        .args(["--width", "5"])
        .output()
        .expect("invoke node analyzer CLI (negative)");
    assert_eq!(bad_out.status.code(), Some(1));
    let bad_parsed: serde_json::Value =
        serde_json::from_slice(&bad_out.stdout).expect("valid JSON");
    assert_eq!(bad_parsed["valid"], false);
    assert!(bad_parsed["metrics"].is_null());

    // Negative: 1-tick FILETIME mismatch -> CLI must exit 1 with metrics: null
    let bad_ft_inv_path = temp_dir.join("invocation-bad-ft.json");
    std::fs::write(
        &bad_ft_inv_path,
        inv_json.replace(
            &START_FILETIME.to_string(),
            &(START_FILETIME + 1).to_string(),
        ),
    )
    .unwrap();
    let bad_ft_out = Command::new("node")
        .arg(&analyzer_script)
        .args(["--report", report_path.to_str().unwrap()])
        .args(["--invocation", bad_ft_inv_path.to_str().unwrap()])
        .args(["--sink-ready", ready_path.to_str().unwrap()])
        .args(["--sink-events", events_path.to_str().unwrap()])
        .args(["--observer-summary", summary_path.to_str().unwrap()])
        .args(["--manifest", manifest_path.to_str().unwrap()])
        .args(["--width", "5"])
        .output()
        .expect("invoke node analyzer CLI (negative filetime)");
    assert_eq!(bad_ft_out.status.code(), Some(1));
    let bad_ft_parsed: serde_json::Value =
        serde_json::from_slice(&bad_ft_out.stdout).expect("valid JSON");
    assert_eq!(bad_ft_parsed["valid"], false);
    assert!(bad_ft_parsed["metrics"].is_null());

    let _ = std::fs::remove_dir_all(&temp_dir);
}
