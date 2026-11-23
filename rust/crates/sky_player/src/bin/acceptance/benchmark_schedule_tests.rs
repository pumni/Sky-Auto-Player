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
        assert!(repeated.schedule.down_chords.len() > base.schedule.down_chords.len());
        assert!(last.saturating_sub(first) >= 15_000_000, "{scenario:?}");
        assert_eq!(
            repeated.expected_down_slots.len(),
            repeated
                .schedule
                .down_chords
                .iter()
                .map(|packet| packet.down_mask.count_ones() as usize)
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
    let base_downs = &base.schedule.down_chords;
    let repeated_downs = &repeated.schedule.down_chords;

    assert_eq!(base_downs.len(), 4);
    assert_eq!(repeated_downs.len() % base_downs.len(), 0);
    assert_eq!(base_downs[1].scheduled_us - base_downs[0].scheduled_us, hold_us * 2);
    let first_cycle_span = base_downs
        .last()
        .expect("the base workload has a final generation")
        .intents
        .last()
        .expect("the final generation has a paired Up")
        .paired_up_offset_us
        + hold_us
        - base_downs[0].scheduled_us;
    assert_eq!(
        repeated_downs[base_downs.len()].scheduled_us - repeated_downs[0].scheduled_us,
        first_cycle_span
    );
}

#[test]
fn paired_workloads_match_the_immutable_pr_base_authored_streams() {
    use sha2::{Digest, Sha256};

    // Full authored-stream projections captured from baseline commit
    // 1d00c3b71fbf1f94003865bad6c4eba6d8dcd902, tree
    // 25a20291b9cf84b1283804f451b33e3c4605507c, at 60 FPS / 500 us.
    // FNV-independent SHA-256 fixtures cover every event and every sample tail.
    let fixtures = [
        (
            "mixed-up-down",
            Scenario::MixedUpDown,
            516,
            387,
            50_000,
            15_147_376,
            "7d83f78e1166fefd9ac797fd945c612fc169de114eff1f39e86f90d37bde6a0b",
        ),
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
            "canonical-max-chord",
            Scenario::CanonicalMaxChord,
            526,
            526,
            50_000,
            15_067_754,
            "c6a439e78f5da268af4f272884ba9acc7dde937ba8d46d4a6cbc199ab59e05c0",
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
        fixtures
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
