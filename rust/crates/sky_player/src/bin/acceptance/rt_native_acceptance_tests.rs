use super::{
    ACCEPTANCE_DOWN_LATE_GRACE_US, ACCEPTANCE_FRAME_US, ACCEPTANCE_TIMING_MARGIN_US,
    DRAIN_DEADLINE_MS, DrainClock, DrainMode, DrainResult, EVENT_SCHEMA_VERSION, EventRecord,
    EventWindow, EventWindowReader, FULL_INSTRUMENT_MASK, INPUT_POLICY, MAX_KEYS,
    MaterializedInstrumentKeyProfile, NativeCleanupEvidence, PHYSICAL_INSTRUMENT_SCAN_CODES,
    PRETERMINAL_DEADLINE_MS, PROBE_KIND, PROBE_TITLE, ParsedCommand, PhysicalExpectation,
    PreTerminalResult, READY_SCHEMA_VERSION, RECEIVE_ONLY_ROLE, ReadyRecord, SCRIPT_MARKER,
    SINK_KIND, SINK_TITLE, Scenario, Verdict, acceptance_min_hold_us,
    acceptance_min_release_gap_us, cleanup_evidence_clean, drain_event_window_with,
    expected_physical_keys, focus_evidence_clean, parse_args, preterminal_verdict,
    production_options, reconcile_events, scenario_plan,
    validate_event_stream, validate_ready_record, w4_profile_spec, wait_for_sink_events_with,
};

fn base_arguments(scenario: &str) -> Vec<String> {
    [
        "run",
        "--allow-real-input",
        "--run-id",
        "test-run",
        "--sink-ready",
        "sink.json",
        "--sink-events",
        "sink-events.json",
        "--target-hwnd",
        "0x42",
        "--scenario",
        scenario,
        "--evidence",
        "evidence.jsonl",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}
fn ready(role: &str) -> ReadyRecord {
    let (sink_kind, title) = if role == RECEIVE_ONLY_ROLE {
        (SINK_KIND, SINK_TITLE)
    } else {
        (PROBE_KIND, PROBE_TITLE)
    };
    ReadyRecord {
        schema_version: READY_SCHEMA_VERSION,
        run_id: "test-run".into(),
        role: role.into(),
        sink_kind: sink_kind.into(),
        pid: 7,
        hwnd: 0x42,
        title: title.into(),
        process: SCRIPT_MARKER.into(),
        input_policy: INPUT_POLICY.into(),
        event_log_id: "event-stream-test".into(),
        event_schema_version: EVENT_SCHEMA_VERSION,
        process_start_time_filetime: 123,
    }
}
fn physical(scan_code: u16) -> PhysicalExpectation {
    PhysicalExpectation {
        scan_code,
        extended: false,
    }
}
fn event_with(
    sequence: u64,
    kind: &str,
    scan_code: u16,
    extended: bool,
    virtual_key: i32,
) -> EventRecord {
    let message = match kind {
        "key_press" => 0x0100,
        "key_release" => 0x0101,
        "sys_key_press" => 0x0104,
        "sys_key_release" => 0x0105,
        _ => 0,
    };
    EventRecord {
        schema_version: EVENT_SCHEMA_VERSION,
        run_id: "test-run".into(),
        role: RECEIVE_ONLY_ROLE.into(),
        event_log_id: "event-stream-test".into(),
        sequence,
        kind: kind.into(),
        scan_code,
        extended,
        virtual_key,
        message,
        observed_utc: "2026-01-01T00:00:00Z".into(),
    }
}
fn event(sequence: u64, kind: &str, scan_code: u16) -> EventRecord {
    event_with(sequence, kind, scan_code, false, 89)
}
#[derive(Debug)]
struct FakeReader {
    samples: Vec<EventWindow>,
    index: usize,
}
impl EventWindowReader for FakeReader {
    fn read(&mut self) -> Result<EventWindow, String> {
        let sample = self
            .samples
            .get(self.index)
            .or_else(|| self.samples.last())
            .cloned()
            .ok_or_else(|| "fake reader has no samples".to_string())?;
        self.index = self.index.saturating_add(1);
        Ok(sample)
    }
}
struct FailingReader;
impl EventWindowReader for FailingReader {
    fn read(&mut self) -> Result<EventWindow, String> {
        Err("event log sequence has a duplicate or gap".into())
    }
}
#[derive(Debug)]
struct FakeClock {
    now_ms: u64,
    step_ms: u64,
}
impl DrainClock for FakeClock {
    fn elapsed_ms(&self) -> u64 {
        self.now_ms
    }
    fn wait_poll(&mut self) {
        self.now_ms = self.now_ms.saturating_add(self.step_ms);
    }
}
fn complete_window(events: Vec<EventRecord>) -> EventWindow {
    EventWindow {
        events,
        complete: true,
    }
}
fn partial_window(events: Vec<EventRecord>) -> EventWindow {
    EventWindow {
        events,
        complete: false,
    }
}
#[test]
fn schema_v3_rejects_v2_and_requires_bound_header() {
    let ready = ready(RECEIVE_ONLY_ROLE);
    let mut old = ready.clone();
    old.schema_version = 2;
    old.event_schema_version = 2;
    assert!(validate_ready_record(&old, "test-run", 0x42, RECEIVE_ONLY_ROLE).is_err());
    assert_eq!(
        validate_event_stream(&[event(0, "stream_start", 0)], &ready, RECEIVE_ONLY_ROLE),
        Ok(0)
    );
    assert!(validate_event_stream(&[], &ready, RECEIVE_ONLY_ROLE).is_err());
    let mut wrong = event(0, "stream_start", 0);
    wrong.event_log_id = "rebound".into();
    assert!(validate_event_stream(&[wrong], &ready, RECEIVE_ONLY_ROLE).is_err());
    assert!(
        validate_event_stream(
            &[event(0, "stream_start", 0), event(2, "key_press", 0x15)],
            &ready,
            RECEIVE_ONLY_ROLE
        )
        .is_err()
    );
}
#[test]
fn canonical_and_w4_expectations_are_physical() {
    assert_eq!(
        expected_physical_keys(None, &[0]),
        vec![physical(PHYSICAL_INSTRUMENT_SCAN_CODES[0])]
    );
    let profile = w4_profile_spec();
    assert_eq!(
        expected_physical_keys(Some(&profile), &[0]),
        vec![physical(0x02)]
    );
    for name in [
        "canonical-max-chord",
        "hold",
        "rapid-retrigger",
        "mixed-up-down",
        "target-hwnd-change",
        "pause-resume",
        "stop-cleanup",
        "skip-cleanup",
        "timing-margin-sweep",
    ] {
        assert!(
            scenario_plan(Scenario::parse(name).unwrap(), ACCEPTANCE_TIMING_MARGIN_US).is_ok(),
            "scenario {name}"
        );
    }
}
#[test]
fn pause_resume_expects_the_full_instrument_suspension_sweep() {
    let plan = scenario_plan(Scenario::PauseResume, ACCEPTANCE_TIMING_MARGIN_US).unwrap();
    let mut expected_up = vec![0, 1];
    expected_up.extend(0..MAX_KEYS);
    assert_eq!(plan.expected_down_slots, vec![0, 1]);
    assert_eq!(plan.expected_up_slots, expected_up);
    assert!(plan.allow_unpaired_cleanup_ups);
}
#[test]
fn processkey_correct_scan_passes_and_wrong_scan_fails() {
    let e = [physical(0x15)];
    assert!(
        reconcile_events(
            &[
                event_with(1, "key_press", 0x15, false, 0xE5),
                event_with(2, "key_release", 0x15, false, 0x59)
            ],
            &e,
            &e,
            false
        )
        .is_ok()
    );
    assert!(
        reconcile_events(
            &[
                event_with(1, "key_press", 0x16, false, 0xE5),
                event_with(2, "key_release", 0x16, false, 0x59)
            ],
            &e,
            &e,
            false
        )
        .is_err()
    );
}
#[test]
fn physical_reconciliation_rejects_extended_duplicate_direction_and_syskey() {
    let e = [physical(0x15)];
    assert!(
        reconcile_events(
            &[
                event_with(1, "key_press", 0x15, true, 0xE5),
                event_with(2, "key_release", 0x15, true, 0x59)
            ],
            &e,
            &e,
            false
        )
        .is_err()
    );
    assert!(
        reconcile_events(
            &[
                event(1, "key_press", 0x15),
                event(2, "key_press", 0x15),
                event(3, "key_release", 0x15)
            ],
            &e,
            &e,
            false
        )
        .is_err()
    );
    assert!(
        reconcile_events(
            &[event_with(1, "sys_key_press", 0x15, false, 0xE5)],
            &e,
            &[],
            false
        )
        .is_err()
    );
    let retrigger = [
        event(1, "key_press", 0x15),
        event(2, "key_release", 0x15),
        event(3, "key_press", 0x15),
        event(4, "key_release", 0x15),
    ];
    assert!(reconcile_events(&retrigger, &[e[0], e[0]], &[e[0], e[0]], false).is_ok());
}
#[test]
fn expected_drain_waits_for_late_event_and_quiet() {
    let e = [physical(0x15)];
    let events = vec![event(1, "key_press", 0x15), event(2, "key_release", 0x15)];
    let mut r = FakeReader {
        samples: vec![complete_window(Vec::new()), complete_window(events.clone())],
        index: 0,
    };
    let mut c = FakeClock {
        now_ms: 0,
        step_ms: 10,
    };
    assert_eq!(
        drain_event_window_with(
            &mut r,
            &mut c,
            DrainMode::ExpectedEvents {
                allow_unpaired_cleanup_ups: false
            },
            &e,
            &e
        ),
        DrainResult::Pass(events)
    );
}
#[test]
fn expected_drain_missing_fails_and_partial_is_inconclusive() {
    let e = [physical(0x15)];
    let mut r = FakeReader {
        samples: vec![complete_window(Vec::new())],
        index: 0,
    };
    let mut c = FakeClock {
        now_ms: 0,
        step_ms: 10,
    };
    assert!(matches!(
        drain_event_window_with(
            &mut r,
            &mut c,
            DrainMode::ExpectedEvents {
                allow_unpaired_cleanup_ups: false
            },
            &e,
            &e
        ),
        DrainResult::Fail(_, _)
    ));
    let mut r = FakeReader {
        samples: vec![partial_window(Vec::new())],
        index: 0,
    };
    let mut c = FakeClock {
        now_ms: 0,
        step_ms: 10,
    };
    assert!(matches!(
        drain_event_window_with(
            &mut r,
            &mut c,
            DrainMode::ExpectedEvents {
                allow_unpaired_cleanup_ups: false
            },
            &e,
            &e
        ),
        DrainResult::Inconclusive(_)
    ));
}
#[test]
fn preterminal_classification_is_typed_and_fail_closed() {
    let e = [physical(0x15)];
    let events = vec![event(1, "key_press", 0x15), event(2, "key_release", 0x15)];
    let mut r = FakeReader {
        samples: vec![complete_window(Vec::new()), complete_window(events)],
        index: 0,
    };
    let mut c = FakeClock {
        now_ms: 0,
        step_ms: 10,
    };
    assert_eq!(
        wait_for_sink_events_with(&mut r, &mut c, &e, &e),
        PreTerminalResult::Satisfied
    );
    let mut r = FakeReader {
        samples: vec![complete_window(vec![event(1, "key_press", 0x16)])],
        index: 0,
    };
    let mut c = FakeClock {
        now_ms: 0,
        step_ms: 10,
    };
    assert!(matches!(
        wait_for_sink_events_with(&mut r, &mut c, &e, &e),
        PreTerminalResult::TrustedFailure(_)
    ));
    let mut r = FakeReader {
        samples: vec![complete_window(vec![event(1, "sys_key_press", 0x15)])],
        index: 0,
    };
    let mut c = FakeClock {
        now_ms: 0,
        step_ms: 10,
    };
    assert!(matches!(
        wait_for_sink_events_with(&mut r, &mut c, &e, &e),
        PreTerminalResult::TrustedFailure(_)
    ));
    let mut r = FakeReader {
        samples: vec![complete_window(Vec::new())],
        index: 0,
    };
    let mut c = FakeClock {
        now_ms: PRETERMINAL_DEADLINE_MS,
        step_ms: 10,
    };
    let result = wait_for_sink_events_with(&mut r, &mut c, &e, &e);
    assert!(matches!(
        result,
        PreTerminalResult::IncompleteTimeout { complete: true }
    ));
    assert_eq!(result.verdict(), Verdict::Fail);
    let mut r = FakeReader {
        samples: vec![partial_window(Vec::new())],
        index: 0,
    };
    let mut c = FakeClock {
        now_ms: PRETERMINAL_DEADLINE_MS,
        step_ms: 10,
    };
    let result = wait_for_sink_events_with(&mut r, &mut c, &e, &e);
    assert!(matches!(
        result,
        PreTerminalResult::IncompleteTimeout { complete: false }
    ));
    assert_eq!(result.verdict(), Verdict::Inconclusive);
    let mut r = FailingReader;
    let mut c = FakeClock {
        now_ms: 0,
        step_ms: 10,
    };
    assert!(matches!(
        wait_for_sink_events_with(&mut r, &mut c, &e, &e),
        PreTerminalResult::ObservationInconclusive(_)
    ));
}
#[test]
fn preterminal_cleanup_verdict_precedence_is_fail_closed() {
    let clean = NativeCleanupEvidence {
        terminal_error: false,
        partial_events: 0,
        zero_progress_events: 0,
        active_count: 0,
        possibly_active_count: 0,
        failed_release_count: 0,
        release_failed: false,
        stuck_mask: 0,
        verification_inconclusive: false,
        transport_anomaly: false,
    };
    assert_eq!(
        preterminal_verdict(Verdict::Inconclusive, clean.is_anomalous()),
        Verdict::Inconclusive
    );
    let mut transport = clean;
    transport.transport_anomaly = true;
    assert_eq!(
        preterminal_verdict(Verdict::Inconclusive, transport.is_anomalous()),
        Verdict::Fail
    );
    let mut residue = clean;
    residue.active_count = 1;
    residue.possibly_active_count = 1;
    assert_eq!(
        preterminal_verdict(Verdict::Inconclusive, residue.is_anomalous()),
        Verdict::Fail
    );
    assert_eq!(
        preterminal_verdict(Verdict::Fail, clean.is_anomalous()),
        Verdict::Fail
    );
    assert_eq!(
        preterminal_verdict(Verdict::Fail, transport.is_anomalous()),
        Verdict::Fail
    );
}
#[test]
fn zero_event_safety_uses_full_deadline_and_catches_delayed_event() {
    let mut r = FakeReader {
        samples: vec![complete_window(Vec::new())],
        index: 0,
    };
    let mut c = FakeClock {
        now_ms: 0,
        step_ms: 10,
    };
    assert_eq!(
        drain_event_window_with(&mut r, &mut c, DrainMode::ZeroEventSafety, &[], &[]),
        DrainResult::Pass(Vec::new())
    );
    assert_eq!(c.now_ms, DRAIN_DEADLINE_MS);
    let mut s = vec![complete_window(Vec::new()); 20];
    s.push(complete_window(vec![event(1, "key_press", 0x15)]));
    let mut r = FakeReader {
        samples: s,
        index: 0,
    };
    let mut c = FakeClock {
        now_ms: 0,
        step_ms: 10,
    };
    assert!(matches!(
        drain_event_window_with(&mut r, &mut c, DrainMode::ZeroEventSafety, &[], &[]),
        DrainResult::Fail(_, _)
    ));
}
#[test]
fn cleanup_reconciles_fifteen_down_and_up_records() {
    let e = (0..MAX_KEYS)
        .map(|slot| physical(PHYSICAL_INSTRUMENT_SCAN_CODES[slot]))
        .collect::<Vec<_>>();
    let mut a = Vec::new();
    for (i, k) in e.iter().enumerate() {
        a.push(event_with(
            i as u64 + 1,
            "key_press",
            k.scan_code,
            k.extended,
            0,
        ));
    }
    for (i, k) in e.iter().enumerate() {
        a.push(event_with(
            MAX_KEYS as u64 + i as u64 + 1,
            "key_release",
            k.scan_code,
            k.extended,
            0,
        ));
    }
    assert!(reconcile_events(&a, &e, &e, false).is_ok());
}
#[test]
fn w4_profile_and_cleanup_verdicts_remain_valid() {
    let p = MaterializedInstrumentKeyProfile::from_validated(
        sky_dispatch_win32::input::InstrumentKeyProfile::try_from_spec(w4_profile_spec()).unwrap(),
    );
    assert_eq!(p.physical_key(0).scan_code, 0x02);
    let plan = scenario_plan(Scenario::CleanupFullRelease, ACCEPTANCE_TIMING_MARGIN_US).unwrap();
    assert_eq!(plan.schedule.packets[0].down_mask, FULL_INSTRUMENT_MASK);
    assert!(cleanup_evidence_clean(
        true,
        FULL_INSTRUMENT_MASK,
        1,
        true,
        0,
        false,
        false
    ));
}
#[test]
fn authorization_and_timing_contracts_remain_bounded() {
    let mut a = base_arguments("focus-loss");
    a.extend(
        [
            "--focus-probe-ready",
            "p",
            "--focus-probe-events",
            "e",
            "--focus-probe-hwnd",
            "0x43",
        ]
        .into_iter()
        .map(str::to_owned),
    );
    let ParsedCommand::Run(args) = parse_args(a).expect("valid acceptance args") else {
        panic!("expected run command")
    };
    assert_eq!(args.down_late_grace_us, ACCEPTANCE_DOWN_LATE_GRACE_US);
    assert_eq!(args.timing_margin_us, ACCEPTANCE_TIMING_MARGIN_US);
    assert_eq!(acceptance_min_hold_us(800), 17_467);
    assert_eq!(acceptance_min_release_gap_us(800), 17_467);
    assert_eq!(acceptance_min_hold_us(0), 16_667);
    assert_eq!(acceptance_min_release_gap_us(0), 16_667);
    assert!(focus_evidence_clean(true, 1, 0, true, true));
}
#[test]
fn grace_override_accepts_only_controlled_ab_values() {
    for grace in ["500", "750", "1000"] {
        let mut args = base_arguments("canonical-single");
        args.extend(
            ["--down-late-grace-us", grace]
                .into_iter()
                .map(str::to_owned),
        );
        let ParsedCommand::Run(run) = parse_args(args).expect("valid grace override") else {
            panic!("expected run command")
        };
        assert_eq!(run.down_late_grace_us, grace.parse::<u64>().unwrap());
    }
    let mut args = base_arguments("canonical-single");
    args.extend(
        ["--down-late-grace-us", "600"]
            .into_iter()
            .map(str::to_owned),
    );
    assert!(parse_args(args).is_err());
}
#[test]
fn timing_margin_override_accepts_only_bounded_hundred_microsecond_values() {
    for margin in ["0", "500", "800", "1000", "3000"] {
        let mut args = base_arguments("timing-margin-sweep");
        args.extend(
            ["--timing-margin-us", margin]
                .into_iter()
                .map(str::to_owned),
        );
        let ParsedCommand::Run(run) = parse_args(args).expect("valid timing margin") else {
            panic!("expected run command")
        };
        assert_eq!(run.timing_margin_us, margin.parse::<u64>().unwrap());
    }
    for margin in ["1", "799", "3001"] {
        let mut args = base_arguments("timing-margin-sweep");
        args.extend(
            ["--timing-margin-us", margin]
                .into_iter()
                .map(str::to_owned),
        );
        assert!(parse_args(args).is_err(), "invalid margin {margin}");
    }
}
#[test]
fn timing_margin_sweep_authors_exact_hold_and_release_targets() {
    for margin in [0, 500, 800, 1_000, 3_000] {
        let plan = scenario_plan(Scenario::TimingMarginSweep, margin).unwrap();
        let targets = plan
            .schedule
            .packets
            .iter()
            .map(|packet| packet.scheduled_us)
            .collect::<Vec<_>>();
        let hold = ACCEPTANCE_FRAME_US + margin;
        let gap = ACCEPTANCE_FRAME_US + margin;
        assert_eq!(targets.len(), 4);
        assert_eq!(targets[1] - targets[0], hold);
        assert_eq!(targets[2] - targets[1], gap);
        assert_eq!(targets[3] - targets[2], hold);
        assert_eq!(plan.expected_down_slots, vec![0, 0]);
        assert_eq!(plan.expected_up_slots, vec![0, 0]);
    }
}
#[test]
fn changing_down_grace_changes_only_the_cutoff_not_authored_timing() {
    let authored_hold = acceptance_min_hold_us(ACCEPTANCE_TIMING_MARGIN_US);
    let authored_gap = acceptance_min_release_gap_us(ACCEPTANCE_TIMING_MARGIN_US);
    let plan = scenario_plan(Scenario::CanonicalSingle, ACCEPTANCE_TIMING_MARGIN_US).unwrap();
    for grace in [500, 750, 1_000] {
        let options = production_options(
            plan.schedule.clone(),
            None,
            grace,
            ACCEPTANCE_TIMING_MARGIN_US,
        );
        assert_eq!(options.timing.min_hold_us, authored_hold);
        assert_eq!(options.timing.min_release_gap_us, authored_gap);
        assert_eq!(options.timing.down_late_grace_us, grace);
    }
}
