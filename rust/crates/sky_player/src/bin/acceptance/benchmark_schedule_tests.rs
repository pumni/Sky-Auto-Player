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
        let first = repeated
            .schedule
            .packets
            .first()
            .expect("timed schedule has packets")
            .scheduled_us;
        let last = repeated
            .schedule
            .packets
            .last()
            .expect("timed schedule has packets")
            .scheduled_us;
        assert!(repeated.schedule.packets.len() > base.schedule.packets.len());
        assert!(last.saturating_sub(first) >= 15_000_000, "{scenario:?}");
        assert_eq!(
            repeated.expected_down_slots.len(),
            repeated
                .schedule
                .packets
                .iter()
                .map(|packet| packet.down_mask.count_ones() as usize)
                .sum::<usize>()
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
