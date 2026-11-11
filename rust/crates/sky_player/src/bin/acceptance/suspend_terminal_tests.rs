use super::{
    ACCEPTANCE_TIMING_MARGIN_US, EVENT_SCHEMA_VERSION, EventRecord, RECEIVE_ONLY_ROLE, Scenario,
    expected_physical_keys, expected_sink_physical_keys, reconcile_events, scenario_plan,
};

fn event(sequence: u64, kind: &str, scan_code: u16) -> EventRecord {
    EventRecord {
        schema_version: EVENT_SCHEMA_VERSION,
        run_id: "test-run".into(),
        role: RECEIVE_ONLY_ROLE.into(),
        event_log_id: "event-stream-test".into(),
        sequence,
        kind: kind.into(),
        scan_code,
        extended: false,
        virtual_key: 89,
        message: if kind == "key_press" { 0x0100 } else { 0x0101 },
        observed_utc: "2026-01-01T00:00:00Z".into(),
    }
}

#[test]
fn terminal_suspend_reconciles_only_committed_down_and_cleanup_up() {
    let plan = scenario_plan(Scenario::SuspendResume, ACCEPTANCE_TIMING_MARGIN_US).unwrap();
    let authored_down = expected_physical_keys(plan.profile.as_ref(), &plan.expected_down_slots);
    let authored_up = expected_physical_keys(plan.profile.as_ref(), &plan.expected_up_slots);
    let (sink_down, sink_up) = expected_sink_physical_keys(
        Scenario::SuspendResume,
        plan.profile.as_ref(),
        &authored_down,
        &authored_up,
    );
    let observed = [
        event(1, "key_press", sink_down[0].scan_code),
        event(2, "key_release", sink_up[0].scan_code),
    ];

    assert_eq!(sink_down.len(), 1);
    assert_eq!(sink_up, sink_down);
    assert!(reconcile_events(&observed, &sink_down, &sink_up, true).is_ok());
    assert!(reconcile_events(&observed, &authored_down, &authored_up, true).is_err());
}

#[test]
fn timed_session_heartbeat_preserves_intentional_lease_expiry() {
    for scenario in [Scenario::CanonicalMaxChord, Scenario::DenseAlternating, Scenario::NearMinimumRetrigger, Scenario::MixedUpDown, Scenario::LongIdle] {
        assert!(super::needs_supervisor_progress_heartbeat(scenario, Some(15)));
        assert!(!super::needs_supervisor_progress_heartbeat(scenario, None));
    }
    for scenario in [Scenario::ReleaseGapStress, Scenario::FocusLoss, Scenario::SuspendResume] {
        assert!(super::needs_supervisor_progress_heartbeat(scenario, None));
    }
    for scenario in [Scenario::SupervisorLeaseExpiry, Scenario::LeaseAgeWatchdogDelayed] {
        assert!(!super::needs_supervisor_progress_heartbeat(scenario, None));
        assert!(!super::needs_supervisor_progress_heartbeat(scenario, Some(15)));
    }
}
