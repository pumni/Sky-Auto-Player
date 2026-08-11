use super::telemetry::metrics::RecentLatencyRing;
use super::test_support::command_timing::{
    CommandTimingError, CommandTimingLookup as PauseTimingLookup, PauseTimingPhase,
};
use super::{
    BackendConfig, CommandTimingResult, CommandTimingState, DownAdmission, FaultInjectionScript,
    FinalControlAdmission, FinalControlSignals, FinalTargetSignals, FocusOptions, HealthWindow,
    HealthWindowPolicy, InjectedSendOutcome, NativeDispatchSession, NativeSessionOptions,
    PlatformSendResult, PriorityOptions, RtTraceRecord, SharedMetrics, StartupOrderingHook,
    TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN, TargetStamp, TelemetryCollector, TelemetryMode,
    TelemetryOptions, TimingOptions, TraceContext, TraceDelivery, TraceTiming, TrackedKeyState,
    WaitOptions, WakeErrorStats, Worker, WorkerMetricsLocal, adjust_spin_threshold,
    anchored_dispatch_target_ticks, cpu_metrics_sample_due, deadline_target_ticks,
    derive_spin_threshold_us, ensure_preflight_for_target, exact_sender_durations,
    final_control_admission_with_lease, final_down_target_admission, focus_gate_matches,
    focus_matches, focus_matches_hwnd, record_input_path_health, record_termination_error,
    release_runtime_outcome, signed_timeline_delta_ticks, supervisor_lease_expired,
    target_stamp_still_current, trace_outcome_code, try_publish_metrics, wake_lateness_ticks,
};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::TimelineTicks;
use sky_dispatch_win32::clock::{
    DurationTicks, QpcClock, QpcTicks, qpc_frequency, qpc_ticks_to_us, qpc_us_to_ticks,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::time::{Duration, Instant};

fn test_session_options(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    _allowed_count: usize,
    backend: BackendConfig,
) -> NativeSessionOptions {
    NativeSessionOptions {
        schedule,
        backend,
        timing: TimingOptions {
            min_hold_us: 0,
            strict_timing: false,
            strict_down_completion_late_us: 2_000,
            strict_up_completion_late_us: 2_000,
            input_path_warn_us: 300,
            spin_threshold_us: 150,
            spin_floor_us: 700,
        },
        focus: FocusOptions {
            require_focus: false,
            focus_restore_grace_us: 100_000,
        },
        wait: WaitOptions {
            enable_waitable_timer: true,
            enable_event_wait: true,
            enable_adaptive_spin: false,
            supervisor_lease_timeout_us: 0,
        },
        telemetry: TelemetryOptions {
            mode: TelemetryMode::Ring,
            capacity: 64,
        },
        priority: PriorityOptions {
            mode: sky_dispatch_win32::mmcss::PriorityMode::Off,
        },
        startup_ordering_hook: None,
    }
}

#[test]
fn supervisor_lease_treats_future_heartbeat_as_fresh() {
    let heartbeat = AtomicU64::new(1_001);
    assert_eq!(
        supervisor_lease_expired(
            QpcTicks::from_raw(1_000),
            DurationTicks::from_raw(100),
            &heartbeat,
        ),
        Ok(false)
    );
}

fn startup_boundary_schedule() -> sky_dispatch_core::model::RuntimeSchedule {
    sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "startup-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 5_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "startup-up".to_string().into(),
            },
        ],
        &[0x15],
    )
    .expect("valid startup boundary schedule")
}

#[test]
fn startup_boundary_is_unpublished_before_start() {
    let session = NativeDispatchSession::new(test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    ))
    .expect("test session admission");

    let snapshot = session.snapshot();
    assert!(!snapshot.startup_ready);
    assert_eq!(snapshot.startup_latency_us, None);
}

#[test]
fn startup_boundary_publishes_after_worker_startup() {
    let session = NativeDispatchSession::new(test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    ))
    .expect("test session admission");
    session.start().expect("worker start");

    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    assert!(snapshot.startup_ready);
    assert!(snapshot.startup_latency_us.is_some());
    let (requested, ready) = session.startup_ticks();
    assert!(requested <= ready);
}

#[test]
fn stale_metadata_commits_before_first_physical_send() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Up,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "stale-zero".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 50,
                scan_codes: smallvec::smallvec![0x15],
                reason: "stale-fifty".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![0x15],
                reason: "physical-hundred".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "cleanup".to_string().into(),
            },
        ],
        &[0x15],
    )
    .expect("valid stale-prefix schedule");
    let hook = Arc::new(StartupOrderingHook::default());
    let mut options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    options.wait.supervisor_lease_timeout_us = 3_000_000;

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    assert_eq!(snapshot.outcome, Some("finished".to_string()));
    assert_eq!(snapshot.terminal_error, None);
    assert_eq!(
        hook.stale_packet_committed.load(Ordering::SeqCst),
        2,
        "ordering hook values stale={} physical={}",
        hook.stale_packet_committed.load(Ordering::SeqCst),
        hook.first_physical_send_started.load(Ordering::SeqCst),
    );
    assert!(hook.first_physical_send_started.load(Ordering::SeqCst) > 0);
    assert!(
        hook.stale_packet_committed.load(Ordering::SeqCst)
            <= hook.first_physical_send_started.load(Ordering::SeqCst)
    );
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("valid telemetry JSON");
    let stale_records = telemetry["records"]
        .as_array()
        .expect("records")
        .iter()
        .filter(|record| matches!(record["event_index"].as_u64(), Some(0) | Some(1)))
        .collect::<Vec<_>>();
    assert_eq!(stale_records.len(), 2);
    assert!(
        stale_records
            .iter()
            .all(|record| record["applied_lead_ticks"].as_u64() == Some(0))
    );
}

#[test]
fn many_leading_stale_packets_are_drained_before_precision_handoff() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let stale_count = 32usize;
    let mut actions = Vec::with_capacity(stale_count + 2);
    for index in 0..stale_count {
        actions.push(KeyActionInput {
            source_action_index: index as u32,
            kind: ActionKind::Up,
            scheduled_us: index as u64,
            scan_codes: smallvec::smallvec![0x15],
            reason: "many-stale".to_string().into(),
        });
    }
    actions.push(KeyActionInput {
        source_action_index: stale_count as u32,
        kind: ActionKind::Down,
        scheduled_us: 1_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "many-stale-down".to_string().into(),
    });
    actions.push(KeyActionInput {
        source_action_index: (stale_count + 1) as u32,
        kind: ActionKind::Up,
        scheduled_us: 2_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "many-stale-cleanup".to_string().into(),
    });
    let schedule = compile_runtime_intents(&actions, &[0x15]).expect("valid many-stale schedule");
    let hook = Arc::new(StartupOrderingHook::default());
    let mut options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    options.telemetry.capacity = 64;
    let session = NativeDispatchSession::new(options).expect("session admission");
    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(session.snapshot().terminal_error, None);
    assert_eq!(
        hook.stale_packet_committed.load(Ordering::SeqCst),
        stale_count as u64
    );
    assert!(hook.first_physical_send_started.load(Ordering::SeqCst) > 0);
}

#[test]
fn all_stale_schedule_finishes_without_precision_or_physical_work() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let actions = (0..8)
        .map(|index| KeyActionInput {
            source_action_index: index,
            kind: ActionKind::Up,
            scheduled_us: u64::from(index),
            scan_codes: smallvec::smallvec![0x15],
            reason: "all-stale".to_string().into(),
        })
        .collect::<Vec<_>>();
    let schedule = compile_runtime_intents(&actions, &[0x15]).expect("valid all-stale schedule");
    let mut options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.telemetry.capacity = 16;
    let session = NativeDispatchSession::new(options).expect("session admission");
    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    assert_eq!(snapshot.outcome, Some("finished".to_string()));
    assert_eq!(snapshot.terminal_error, None);
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("valid telemetry JSON");
    let records = telemetry["records"].as_array().expect("records");
    assert_eq!(records.len(), 8);
    assert!(records.iter().all(|record| {
        record["kind"].as_u64() == Some(1)
            && record["requested_count"].as_u64() == Some(0)
            && record["send_attempts"].as_u64() == Some(0)
            && record["applied_lead_ticks"].as_u64() == Some(0)
    }));
}

fn run_seeded_adaptive_startup_schedule(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    allowed_count: usize,
) -> serde_json::Value {
    run_seeded_adaptive_startup_schedule_with_capacity(schedule, allowed_count, 16).1
}

fn run_seeded_adaptive_startup_schedule_with_capacity(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    allowed_count: usize,
    telemetry_capacity: usize,
) -> (super::EngineSnapshot, serde_json::Value) {
    let mut options = test_session_options(
        schedule,
        allowed_count,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.telemetry.capacity = telemetry_capacity;
    options.wait.supervisor_lease_timeout_us = 3_000_000;

    let session = NativeDispatchSession::new(options).expect("adaptive session admission");
    session.start().expect("adaptive worker start");
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("adaptive worker join")
    );
    let snapshot = session.snapshot();
    assert_eq!(snapshot.outcome, Some("finished".to_string()));
    assert_eq!(snapshot.terminal_error, None);
    assert_eq!(snapshot.keys_dropped, 0);
    assert_eq!(snapshot.sendinput_partial_events, 0);
    assert_eq!(snapshot.sendinput_zero_progress_failures, 0);
    assert_eq!(snapshot.authored_keys_rejected, 0);
    assert_eq!(snapshot.chord_integrity_lost, 0);
    let telemetry = serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
        .expect("valid telemetry JSON");
    (snapshot, telemetry)
}

fn stale_leading_up_schedule(
    stale_timestamps: &[u64],
) -> sky_dispatch_core::model::RuntimeSchedule {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let mut actions = Vec::with_capacity(stale_timestamps.len() + 2);
    for (index, scheduled_us) in stale_timestamps.iter().copied().enumerate() {
        actions.push(KeyActionInput {
            source_action_index: index as u32,
            kind: ActionKind::Up,
            scheduled_us,
            scan_codes: smallvec::smallvec![0x15],
            reason: "stale-leading-up".to_string().into(),
        });
    }
    let down_index = stale_timestamps.len() as u32;
    actions.push(KeyActionInput {
        source_action_index: down_index,
        kind: ActionKind::Down,
        scheduled_us: 100,
        scan_codes: smallvec::smallvec![0x15],
        reason: "stale-leading-down".to_string().into(),
    });
    actions.push(KeyActionInput {
        source_action_index: down_index + 1,
        kind: ActionKind::Up,
        scheduled_us: 1_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "stale-leading-cleanup".to_string().into(),
    });
    compile_runtime_intents(&actions, &[0x15]).expect("valid stale-leading schedule")
}

fn same_timestamp_stale_leading_up_schedule() -> sky_dispatch_core::model::RuntimeSchedule {
    use sky_dispatch_core::compile::compile_runtime_intents;

    compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Up,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "same-timestamp-stale-a".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x16],
                reason: "same-timestamp-stale-b".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![0x15],
                reason: "same-timestamp-first-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "same-timestamp-cleanup".to_string().into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid same-timestamp stale-leading schedule")
}

fn assert_stale_leading_up_did_not_send(telemetry: &serde_json::Value, stale_count: usize) {
    let records = telemetry["records"].as_array().expect("records array");
    for index in 0..stale_count {
        let record = records
            .iter()
            .find(|record| record["event_index"].as_u64() == Some(index as u64))
            .expect("stale Up record");
        assert_eq!(record["kind"].as_u64(), Some(1));
        assert_eq!(record["requested_count"].as_u64(), Some(0));
        assert_eq!(record["send_attempts"].as_u64(), Some(0));
    }
    let down_index = stale_count as u64;
    let down = records
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(down_index))
        .expect("first physical Down record");
    assert_eq!(down["kind"].as_u64(), Some(0));
    assert_eq!(down["applied_lead_ticks"].as_u64(), Some(0));
}

#[test]
fn same_timestamp_stale_leading_up_packet_is_suppressed_atomically() {
    let telemetry =
        run_seeded_adaptive_startup_schedule(same_timestamp_stale_leading_up_schedule(), 1);
    let records = telemetry["records"].as_array().expect("records array");
    let stale = records
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(0))
        .expect("same-timestamp stale packet record");
    assert_eq!(stale["kind"].as_u64(), Some(1));
    assert_eq!(stale["requested_count"].as_u64(), Some(0));
    assert_eq!(stale["send_attempts"].as_u64(), Some(0));
    assert!(
        records
            .iter()
            .all(|record| record["event_index"].as_u64() != Some(1)),
        "same packet must produce one stale telemetry record"
    );
    let down = records
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(2))
        .expect("first physical Down record");
    assert_eq!(down["kind"].as_u64(), Some(0));
    assert_eq!(down["applied_lead_ticks"].as_u64(), Some(0));
}

#[test]
fn stale_leading_up_does_not_consume_startup_target() {
    let telemetry = run_seeded_adaptive_startup_schedule(stale_leading_up_schedule(&[0]), 1);
    assert_stale_leading_up_did_not_send(&telemetry, 1);
}

#[test]
fn multiple_stale_leading_ups_do_not_consume_startup_target() {
    let telemetry = run_seeded_adaptive_startup_schedule(stale_leading_up_schedule(&[0, 50]), 1);
    assert_stale_leading_up_did_not_send(&telemetry, 2);
}

fn run_mock_schedule(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    allowed_count: usize,
    telemetry_capacity: usize,
) -> (super::EngineSnapshot, serde_json::Value) {
    let mut options = test_session_options(
        schedule,
        allowed_count,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.telemetry.capacity = telemetry_capacity;
    options.wait.supervisor_lease_timeout_us = 3_000_000;
    let session = NativeDispatchSession::new(options).expect("session admission");
    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("valid telemetry JSON");
    (snapshot, telemetry)
}

#[test]
fn midstream_stale_packet_is_metadata_not_physical_work() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "midstream-a-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "midstream-a-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 1_500,
                scan_codes: smallvec::smallvec![0x15],
                reason: "midstream-stale".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Down,
                scheduled_us: 2_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "midstream-b-down".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Up,
                scheduled_us: 3_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "midstream-b-up".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid midstream stale schedule");
    let (snapshot, telemetry) = run_mock_schedule(schedule, 2, 16);
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    let records = telemetry["records"].as_array().expect("records");
    let stale = records
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(2))
        .expect("midstream stale record");
    assert_eq!(stale["outcome"].as_u64(), Some(4), "stale record: {stale}");
    assert_eq!(stale["requested_count"].as_u64(), Some(0));
    assert_eq!(stale["sent_count"].as_u64(), Some(0));
    assert_eq!(stale["send_attempts"].as_u64(), Some(0));
    assert_eq!(stale["applied_lead_ticks"].as_u64(), Some(0));
    assert!(stale["wake_ticks"].as_u64().is_some_and(|ticks| ticks > 0));
    assert_eq!(
        records
            .iter()
            .filter(|record| record["event_index"].as_u64() == Some(3))
            .count(),
        1
    );
}

#[test]
fn trailing_stale_packet_finishes_without_physical_dispatch() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "trailing-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "trailing-release".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 1_500,
                scan_codes: smallvec::smallvec![0x15],
                reason: "trailing-stale".into(),
            },
        ],
        &[0x15],
    )
    .expect("valid trailing stale schedule");
    let (snapshot, telemetry) = run_mock_schedule(schedule, 1, 8);
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    let stale = telemetry["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(2))
        .expect("trailing stale record");
    assert_eq!(stale["outcome"].as_u64(), Some(4));
    assert_eq!(stale["requested_count"].as_u64(), Some(0));
    assert_eq!(stale["send_attempts"].as_u64(), Some(0));
}

#[test]
fn same_timestamp_midstream_stale_packet_is_committed_once() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x17],
                reason: "cohort-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 500,
                scan_codes: smallvec::smallvec![0x17],
                reason: "cohort-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "cohort-stale-a".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "cohort-stale-b".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Down,
                scheduled_us: 1_500,
                scan_codes: smallvec::smallvec![0x17],
                reason: "cohort-redown".into(),
            },
            KeyActionInput {
                source_action_index: 5,
                kind: ActionKind::Up,
                scheduled_us: 2_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "cohort-cleanup".into(),
            },
        ],
        &[0x15, 0x16, 0x17],
    )
    .expect("valid same-timestamp stale schedule");
    let (snapshot, telemetry) = run_mock_schedule(schedule, 3, 16);
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    let records = telemetry["records"].as_array().expect("records");
    let stale = records
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(2))
        .expect("same-timestamp stale record");
    assert_eq!(stale["outcome"].as_u64(), Some(4));
    assert_eq!(stale["polyphony"].as_u64(), Some(2));
    assert_eq!(
        records
            .iter()
            .filter(|record| record["outcome"].as_u64() == Some(4))
            .count(),
        1
    );
    assert!(
        records
            .iter()
            .all(|record| record["event_index"].as_u64() != Some(3))
    );
}

#[test]
fn stale_ups_with_down_same_timestamp_use_one_down_packet() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "mixed-stale-a".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "mixed-stale-b".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "mixed-down".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 2_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "mixed-cleanup".into(),
            },
        ],
        &[0x15, 0x16, 0x17],
    )
    .expect("valid stale-plus-down schedule");
    let (snapshot, telemetry) = run_seeded_adaptive_startup_schedule_with_capacity(schedule, 3, 8);
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    let records = telemetry["records"].as_array().expect("records");
    assert_eq!(
        records
            .iter()
            .filter(|record| record["outcome"].as_u64() == Some(4))
            .count(),
        0
    );
    let physical = records
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(2))
        .expect("physical down record");
    assert_eq!(physical["kind"].as_u64(), Some(0));
    assert_eq!(physical["requested_count"].as_u64(), Some(1));
}

#[test]
fn owned_up_stale_up_and_down_same_timestamp_count_only_physical_events() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "owned-up-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "owned-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "owned-stale".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "owned-redown".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Up,
                scheduled_us: 2_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "owned-cleanup".into(),
            },
        ],
        &[0x15, 0x16, 0x17],
    )
    .expect("valid owned-plus-stale schedule");
    let (snapshot, telemetry) = run_seeded_adaptive_startup_schedule_with_capacity(schedule, 3, 8);
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    let packet = telemetry["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|record| {
            record["kind"].as_u64() == Some(2) && record["requested_count"].as_u64() == Some(2)
        })
        .expect("mixed physical record");
    assert_eq!(packet["kind"].as_u64(), Some(2));
    assert_eq!(packet["requested_count"].as_u64(), Some(2));
    assert_ne!(packet["requested_count"].as_u64(), Some(3));
}

#[test]
fn many_midstream_stale_packets_remain_linear_and_physical_work_continues() {
    use crate::engine::observer_test_hooks::{
        observer_test_hook_guard, set_observer_artificial_cost_us,
    };
    use sky_dispatch_core::compile::compile_runtime_intents;

    let _observer_hooks = observer_test_hook_guard();
    set_observer_artificial_cost_us(20_000);
    let stale_count = 64usize;
    let expected_stale_packets = stale_count + 1;
    let mut actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "many-midstream-down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 500,
            scan_codes: smallvec::smallvec![0x15],
            reason: "many-midstream-release".into(),
        },
    ];
    for index in 0..stale_count {
        actions.push(KeyActionInput {
            source_action_index: (index + 2) as u32,
            kind: ActionKind::Up,
            scheduled_us: 1_000 + index as u64,
            scan_codes: smallvec::smallvec![0x16],
            reason: "many-midstream-stale".into(),
        });
    }
    actions.extend([
        KeyActionInput {
            source_action_index: (stale_count + 2) as u32,
            kind: ActionKind::Down,
            scheduled_us: 2_000,
            scan_codes: smallvec::smallvec![0x17],
            reason: "many-midstream-next-down".into(),
        },
        KeyActionInput {
            source_action_index: (stale_count + 3) as u32,
            kind: ActionKind::Up,
            scheduled_us: 3_000,
            scan_codes: smallvec::smallvec![0x17],
            reason: "many-midstream-cleanup".into(),
        },
    ]);
    let schedule = compile_runtime_intents(&actions, &[0x15, 0x16, 0x17])
        .expect("valid many-midstream stale schedule");
    let hook = Arc::new(StartupOrderingHook::default());
    let mut options = test_session_options(
        schedule,
        3,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.telemetry.capacity = 128;
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    options.wait.supervisor_lease_timeout_us = 3_000_000;
    let session = NativeDispatchSession::new(options).expect("session admission");
    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    assert_eq!(
        hook.stale_packet_committed.load(Ordering::SeqCst),
        expected_stale_packets as u64
    );
    assert!(
        hook.first_physical_send_started.load(Ordering::SeqCst) > 0,
        "physical dispatch must begin: stale={}, physical={}",
        hook.stale_packet_committed.load(Ordering::SeqCst),
        hook.first_physical_send_started.load(Ordering::SeqCst),
    );
    assert!(snapshot.observer_dropped_samples > 0);
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("valid telemetry JSON");
    let records = telemetry["records"].as_array().expect("records");
    let stale_records = records
        .iter()
        .filter(|record| record["outcome"].as_u64() == Some(4))
        .count();
    assert!(stale_records <= expected_stale_packets);
}

#[test]
fn production_startup_matrix_preserves_first_physical_lead() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    for scheduled_us in [0, 100, 499, 500, 501] {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "startup-matrix-down".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: scheduled_us + 1_000,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "startup-matrix-cleanup".into(),
                },
            ],
            &[0x15],
        )
        .expect("valid startup matrix schedule");
        let telemetry = run_seeded_adaptive_startup_schedule(schedule, 1);
        let first = telemetry["records"]
            .as_array()
            .expect("records")
            .iter()
            .find(|record| record["event_index"].as_u64() == Some(0))
            .expect("first physical record");
        assert_eq!(first["applied_lead_ticks"].as_u64(), Some(0));
    }
}

#[test]
fn physical_bucket_after_stale_metadata_uses_event_count_not_metadata() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "bucket-first".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 500,
                scan_codes: smallvec::smallvec![0x15],
                reason: "bucket-release".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "bucket-stale".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Down,
                scheduled_us: 1_500,
                scan_codes: smallvec::smallvec![0x17, 0x18],
                reason: "bucket-two-down".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Up,
                scheduled_us: 2_500,
                scan_codes: smallvec::smallvec![0x17, 0x18],
                reason: "bucket-cleanup".into(),
            },
        ],
        &[0x15, 0x16, 0x17, 0x18],
    )
    .expect("valid bucket schedule");
    let mut options = test_session_options(
        schedule,
        4,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.telemetry.capacity = 16;
    options.wait.supervisor_lease_timeout_us = 3_000_000;
    let session = NativeDispatchSession::new(options).expect("session admission");
    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(session.snapshot().terminal_error, None);
    let records: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("records JSON");
    let two_down = records["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(3))
        .expect("two-down record");
    assert_eq!(two_down["requested_count"].as_u64(), Some(2));
    assert_eq!(two_down["applied_lead_ticks"].as_u64(), Some(0));
}

fn progress_idle_gap_schedule() -> sky_dispatch_core::model::RuntimeSchedule {
    sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                // Keep the pause/resume lifecycle in an idle interval. No
                // physical generation is owned while the test acknowledges
                // the pause, so the test does not conflate pause observation
                // with terminal cleanup of an active key.
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "progress-idle-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 5_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "progress-idle-up".to_string().into(),
            },
        ],
        &[0x15],
    )
    .expect("valid progress idle-gap schedule")
}

#[test]
fn progress_snapshot_advances_during_idle_gap_with_telemetry_off() {
    let mut options = test_session_options(
        progress_idle_gap_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.telemetry.mode = TelemetryMode::Off;

    let session = NativeDispatchSession::new(options).expect("test session admission");
    session.start().expect("worker start");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot_lite();
        if snapshot.is_running && snapshot.elapsed_us >= 50_000 {
            break;
        }
        assert!(Instant::now() < deadline, "progress did not start");
        std::thread::sleep(Duration::from_millis(5));
    }

    let first_lite = session.snapshot_lite();
    let first_full = session.snapshot();
    std::thread::sleep(Duration::from_millis(100));
    let second_lite = session.snapshot_lite();
    let second_full = session.snapshot();

    assert!(second_lite.elapsed_us > first_lite.elapsed_us);
    assert!(second_full.elapsed_us > first_full.elapsed_us);
    assert!(second_lite.elapsed_us >= first_lite.elapsed_us);
    assert!(second_full.elapsed_us >= first_full.elapsed_us);
    assert!(second_lite.elapsed_us > 0);
    assert!(second_full.elapsed_us > 0);

    session.quit().expect("quit request");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn progress_snapshot_freezes_for_manual_pause_and_resumes_afterward() {
    let mut options = test_session_options(
        progress_idle_gap_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.telemetry.mode = TelemetryMode::Off;

    let session = NativeDispatchSession::new(options).expect("test session admission");
    session.start().expect("worker start");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot_lite();
        if snapshot.is_running && snapshot.elapsed_us >= 50_000 {
            break;
        }
        assert!(Instant::now() < deadline, "progress did not start");
        std::thread::sleep(Duration::from_millis(5));
    }

    let pause_generation = session.pause_with_timing_token().expect("pause request");
    let pause_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot_lite();
        let pause_acknowledged = session
            .pause_timing_result(pause_generation)
            .expect("pause timing result")
            .is_some();
        if snapshot.is_paused && pause_acknowledged {
            break;
        }
        assert!(
            Instant::now() < pause_deadline,
            "pause was not committed: snapshot={snapshot:?}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let paused_at = session.snapshot_lite().elapsed_us;
    let paused_sample_deadline = Instant::now() + Duration::from_millis(100);
    while Instant::now() < paused_sample_deadline {
        std::thread::sleep(Duration::from_millis(2));
    }
    let paused_after = session.snapshot_lite().elapsed_us;
    assert!(paused_after.saturating_sub(paused_at) < 20_000);

    session.resume().expect("resume request");
    let resume_deadline = Instant::now() + Duration::from_secs(2);
    let resumed_after = loop {
        let snapshot = session.snapshot_lite();
        if !snapshot.is_paused && snapshot.elapsed_us > paused_after + 20_000 {
            break snapshot.elapsed_us;
        }
        assert!(
            Instant::now() < resume_deadline,
            "resume did not commit progress: snapshot={snapshot:?}, terminal_error={:?}",
            session.snapshot().terminal_error
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    assert!(resumed_after > paused_after + 20_000);

    session.quit().expect("quit request");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn native_telemetry_drops_observations_without_blocking_dispatch() {
    let actions: Vec<KeyActionInput> = (0_u32..160)
        .flat_map(|index| {
            let cycle_us = u64::from(index) * 10_000;
            [
                KeyActionInput {
                    source_action_index: index * 2,
                    kind: ActionKind::Down,
                    scheduled_us: cycle_us,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "long-down".to_string().into(),
                },
                KeyActionInput {
                    source_action_index: index * 2 + 1,
                    kind: ActionKind::Up,
                    scheduled_us: cycle_us + 5_000,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "long-up".to_string().into(),
                },
            ]
        })
        .collect();
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("long telemetry schedule must compile");
    let mut options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.wait.supervisor_lease_timeout_us = 3_000_000;
    options.telemetry.capacity = actions.len();

    let session = NativeDispatchSession::new(options).expect("test session admission");
    session.start().expect("worker start");
    while !session.snapshot().is_finished {
        session.heartbeat().expect("supervisor heartbeat");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.outcome,
        Some("finished".to_string()),
        "terminal error: {:?}",
        snapshot.terminal_error
    );
    assert_eq!(snapshot.terminal_error, None);
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry"))
            .expect("valid telemetry JSON");
    assert_eq!(telemetry["attempted"], telemetry["accepted"]);
    assert_eq!(telemetry["dropped"], 0);
    assert_eq!(telemetry["truncated"], false);
    assert!(snapshot.observer_dropped_samples > 0);
    let records = telemetry["records"].as_array().expect("records array");
    assert_eq!(
        records.len(),
        telemetry["accepted"].as_u64().unwrap() as usize
    );
    let indices: Vec<u64> = records
        .iter()
        .map(|record| record["event_index"].as_u64().expect("event index"))
        .collect();
    assert!(
        indices.iter().copied().max().unwrap_or_default() >= 300,
        "full observer queue should retain its earliest observations"
    );
}

#[test]
fn startup_failure_does_not_publish_ready_boundary() {
    let session = NativeDispatchSession::new(test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript {
                wait_failure: true,
                ..FaultInjectionScript::none()
            },
        },
    ))
    .expect("test session admission");
    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert!(!session.snapshot().startup_ready);
    assert_eq!(session.snapshot().startup_latency_us, None);
}

#[test]
fn startup_failure_prevents_dispatch_ready_publication() {
    let session = NativeDispatchSession::new(test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript {
                wait_failure: true,
                ..FaultInjectionScript::none()
            },
        },
    ))
    .expect("test session admission");
    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert!(!session.snapshot().startup_ready);
    assert_eq!(session.snapshot().startup_latency_us, None);
}

#[test]
fn dispatch_ready_boundary_publishes_only_after_full_bootstrap() {
    let session = NativeDispatchSession::new(test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    ))
    .expect("test session admission");
    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert!(session.snapshot().startup_ready);
    assert!(session.snapshot().startup_latency_us.is_some());
}

#[test]
fn worker_takes_runtime_schedule_only_once() {
    let session = NativeDispatchSession::new(test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    ))
    .expect("test session admission");
    let mut worker = Worker::new(
        test_session_options(
            startup_boundary_schedule(),
            1,
            BackendConfig::Mock {
                latency_base_us: 0,
                latency_per_key_us: 0,
                fault_script: FaultInjectionScript::none(),
            },
        ),
        session.shared_for_test(),
    );

    assert!(worker.take_schedule_for_test().is_ok());
    assert!(matches!(
        worker.take_schedule_for_test(),
        Err("worker runtime schedule was already consumed")
    ));
}

#[test]
fn large_runtime_schedule_starts_and_quits_cleanly() {
    const AUTHORED_ACTIONS: usize = 100_000;
    let mut actions = Vec::with_capacity(AUTHORED_ACTIONS);
    for action_index in 0..AUTHORED_ACTIONS {
        actions.push(KeyActionInput {
            source_action_index: action_index as u32,
            kind: if action_index % 2 == 0 {
                ActionKind::Down
            } else {
                ActionKind::Up
            },
            scheduled_us: action_index as u64,
            scan_codes: smallvec::smallvec![0x15],
            reason: "large-schedule".to_string().into(),
        });
    }
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("large schedule must compile");
    assert_eq!(schedule.batches.len(), AUTHORED_ACTIONS);
    assert_eq!(schedule.intents.len(), AUTHORED_ACTIONS);

    let session = NativeDispatchSession::new(test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    ))
    .expect("large schedule session admission");
    session.start().expect("large schedule worker start");
    session.quit().expect("large schedule quit request");
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("large schedule join")
    );
    assert!(session.snapshot().is_finished);
}

#[test]
fn request_moves_idle_to_requested() {
    let timing = CommandTimingState::default();
    let generation = timing
        .request_pause(QpcTicks::from_raw(100))
        .expect("request must succeed");
    assert_eq!(generation, 1);
    assert_eq!(
        *timing.phase.lock(),
        PauseTimingPhase::Requested {
            generation,
            requested_ticks: QpcTicks::from_raw(100),
        }
    );
}

#[test]
fn second_request_coalesces_to_same_generation() {
    let timing = CommandTimingState::default();
    let first = timing.request_pause(QpcTicks::from_raw(100)).unwrap();
    let second = timing.request_pause(QpcTicks::from_raw(200)).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        *timing.phase.lock(),
        PauseTimingPhase::Requested {
            generation: first,
            requested_ticks: QpcTicks::from_raw(100),
        }
    );
}

#[test]
fn observe_moves_requested_to_observed_and_is_idempotent() {
    let timing = CommandTimingState::default();
    let generation = timing.request_pause(QpcTicks::from_raw(100)).unwrap();
    assert_eq!(
        timing.observe_pause(QpcTicks::from_raw(120)),
        Some(generation)
    );
    assert_eq!(timing.observe_pause(QpcTicks::from_raw(130)), None);
    assert_eq!(
        *timing.phase.lock(),
        PauseTimingPhase::Observed {
            generation,
            requested_ticks: QpcTicks::from_raw(100),
            observed_ticks: QpcTicks::from_raw(120),
        }
    );
}

#[test]
fn ack_before_observe_does_nothing() {
    let timing = CommandTimingState::default();
    timing.request_pause(QpcTicks::from_raw(100)).unwrap();
    assert_eq!(timing.acknowledge_pause(QpcTicks::from_raw(120)), None);
}

#[test]
fn ack_moves_observed_to_acknowledged_and_is_not_rewritten() {
    let timing = CommandTimingState::default();
    let generation = timing.request_pause(QpcTicks::from_raw(100)).unwrap();
    timing.observe_pause(QpcTicks::from_raw(120));
    assert_eq!(
        timing.acknowledge_pause(QpcTicks::from_raw(150)),
        Some(generation)
    );
    assert_eq!(timing.acknowledge_pause(QpcTicks::from_raw(200)), None);
    assert_eq!(
        *timing.phase.lock(),
        PauseTimingPhase::Acknowledged {
            generation,
            requested_ticks: QpcTicks::from_raw(100),
            observed_ticks: QpcTicks::from_raw(120),
            acknowledged_ticks: QpcTicks::from_raw(150),
        }
    );
}

#[test]
fn observation_then_later_acknowledgment_succeeds() {
    let timing = CommandTimingState::default();
    let generation = timing.request_pause(QpcTicks::from_raw(100)).unwrap();
    timing.observe_pause(QpcTicks::from_raw(120));
    assert_eq!(
        timing.acknowledge_pause(QpcTicks::from_raw(150)),
        Some(generation)
    );
    assert!(matches!(
        *timing.phase.lock(),
        PauseTimingPhase::Acknowledged { .. }
    ));
}

#[test]
fn resume_cancels_requested_and_observed() {
    let requested = CommandTimingState::default();
    let requested_generation = requested.request_pause(QpcTicks::from_raw(100)).unwrap();
    assert_eq!(requested.cancel_pause_request(), Some(requested_generation));
    assert_eq!(
        *requested.phase.lock(),
        PauseTimingPhase::Cancelled {
            generation: requested_generation
        }
    );

    let observed = CommandTimingState::default();
    let observed_generation = observed.request_pause(QpcTicks::from_raw(100)).unwrap();
    observed.observe_pause(QpcTicks::from_raw(120));
    assert_eq!(observed.cancel_pause_request(), Some(observed_generation));
}

#[test]
fn completed_result_is_not_cancelled_by_resume() {
    let timing = CommandTimingState::default();
    let generation = timing.request_pause(QpcTicks::from_raw(100)).unwrap();
    timing.observe_pause(QpcTicks::from_raw(120));
    timing.acknowledge_pause(QpcTicks::from_raw(150));
    assert_eq!(timing.cancel_pause_request(), None);
    assert!(matches!(
        timing
            .result(generation, QpcClock::initialize().unwrap())
            .unwrap(),
        PauseTimingLookup::Complete(CommandTimingResult { generation: value, .. })
            if value == generation
    ));
}

#[test]
fn cancelled_token_returns_cancelled() {
    let timing = CommandTimingState::default();
    let generation = timing.request_pause(QpcTicks::from_raw(100)).unwrap();
    timing.cancel_pause_request();
    assert_eq!(
        timing
            .result(generation, QpcClock::initialize().unwrap())
            .unwrap(),
        PauseTimingLookup::Cancelled
    );
}

#[test]
fn consume_completed_result_returns_to_idle() {
    let timing = CommandTimingState::default();
    let generation = timing.request_pause(QpcTicks::from_raw(100)).unwrap();
    timing.observe_pause(QpcTicks::from_raw(120));
    timing.acknowledge_pause(QpcTicks::from_raw(150));
    assert!(matches!(
        timing
            .result(generation, QpcClock::initialize().unwrap())
            .unwrap(),
        PauseTimingLookup::Complete(_)
    ));
    let next_generation = timing.request_pause(QpcTicks::from_raw(200)).unwrap();
    assert_ne!(next_generation, generation);
    assert!(matches!(
        *timing.phase.lock(),
        PauseTimingPhase::Requested { .. }
    ));
}

#[test]
fn generation_never_uses_zero_and_wrap_skips_zero() {
    let timing = CommandTimingState::default();
    assert_eq!(timing.next_generation(), 1);
    timing.next_generation.store(u64::MAX, Ordering::Relaxed);
    assert_eq!(timing.next_generation(), 1);
    assert_eq!(timing.next_generation(), 2);
}

#[test]
fn unknown_generation_is_rejected() {
    let timing = CommandTimingState::default();
    assert_eq!(
        timing.result(1, QpcClock::initialize().unwrap()).unwrap(),
        PauseTimingLookup::UnknownGeneration
    );
    assert!(matches!(
        timing.result(0, QpcClock::initialize().unwrap()),
        Err(CommandTimingError::InvalidGeneration)
    ));
}

#[test]
fn telemetry_off_does_not_build_trace_records() {
    let mut telemetry = TelemetryCollector::new(TelemetryMode::Off, 4);
    let mut builds = 0;
    telemetry
        .try_push(|| {
            builds += 1;
            Err(sky_dispatch_core::time::TimeArithmeticError::Overflow)
        })
        .expect("telemetry off must not evaluate the builder");
    assert_eq!(builds, 0);
    assert_eq!(telemetry.output.attempted, 1);
    assert!(telemetry.output.records.is_empty());
}

#[test]
fn telemetry_ring_builds_once_and_propagates_build_error() {
    let mut telemetry = TelemetryCollector::new(TelemetryMode::Ring, 4);
    let mut builds = 0;
    telemetry
        .try_push(|| {
            builds += 1;
            Ok(RtTraceRecord {
                event_index: 0,
                kind: TRACE_KIND_DOWN,
                outcome: 0,
                polyphony: 1,
                flags: TRACE_FLAG_SENT_FULL,
                authored_ticks: 0,
                effective_deadline_ticks: 0,
                wake_ticks: 0,
                send_started_ticks: 0,
                send_completed_ticks: 0,
                dispatch_cost_us: 0,
                core_post_send_duration_us: 0,
                post_send_metrics_available: false,
                dispatch_start_error_ticks: 0,
                completion_error_ticks: 0,
                authored_completion_error_ticks: 0,
                applied_lead_ticks: 0,
                win32_error: 0,
                requested_count: 1,
                sent_count: 1,
                skipped_count: 0,
                send_attempts: 1,
            })
        })
        .unwrap();
    assert_eq!(builds, 1);
    assert_eq!(telemetry.output.accepted, 1);

    let result = telemetry.try_push(|| Err(sky_dispatch_core::time::TimeArithmeticError::Overflow));
    assert!(result.is_err());
    assert_eq!(telemetry.output.accepted, 1);
}

#[test]
fn cpu_metrics_sampling_is_due_only_at_the_interval_boundary() {
    assert!(!cpu_metrics_sample_due(99_999, 0, 100_000));
    assert!(cpu_metrics_sample_due(100_000, 0, 100_000));
    assert!(cpu_metrics_sample_due(100_001, 0, 100_000));
    assert!(!cpu_metrics_sample_due(u64::MAX, u64::MAX, 100_000));
    assert!(cpu_metrics_sample_due(u64::MAX, 0, 100_000));
}

#[test]
fn healthy_metric_publication_is_throttled_but_force_is_immediate() {
    let shared = SharedMetrics::default();
    let local = WorkerMetricsLocal::default();
    try_publish_metrics(&local, &shared, 0, false);
    try_publish_metrics(&local, &shared, 49_999, false);
    assert_eq!(shared.publish_count.load(Ordering::Relaxed), 0);
    try_publish_metrics(&local, &shared, 50_000, false);
    assert_eq!(shared.publish_count.load(Ordering::Relaxed), 1);
    try_publish_metrics(&local, &shared, 50_001, true);
    assert_eq!(shared.publish_count.load(Ordering::Relaxed), 2);
}

#[test]
fn target_stamp_rearms_preflight_without_rechecking_steady_state() {
    let backend = TrackedKeyState::with_emitter(|codes, _key_up| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: codes.len() as u8,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 0,
        timing_error: None,
    });
    let stamp = TargetStamp {
        hwnd: 123,
        generation: 1,
    };
    let mut verified = None;
    ensure_preflight_for_target(&backend, stamp, &mut verified).unwrap();
    assert_eq!(verified, Some(stamp));

    ensure_preflight_for_target(&backend, stamp, &mut verified).unwrap();
    assert_eq!(verified, Some(stamp));

    let next_generation = TargetStamp {
        generation: 2,
        ..stamp
    };
    ensure_preflight_for_target(&backend, next_generation, &mut verified).unwrap();
    assert_eq!(verified, Some(next_generation));
}

#[test]
fn admission_epoch_invalidation_requires_new_preflight_even_for_same_stamp() {
    let backend = TrackedKeyState::with_emitter(|codes, _key_up| PlatformSendResult {
        requested: codes.len() as u8,
        inserted: codes.len() as u8,
        started_ticks: QpcTicks::ZERO,
        completed_ticks: Some(QpcTicks::ZERO),
        win32_error: 0,
        timing_error: None,
    });
    let stamp = TargetStamp {
        hwnd: 123,
        generation: 1,
    };
    let mut verified = Some(stamp);
    assert_eq!(verified, Some(stamp));
    verified = None;
    ensure_preflight_for_target(&backend, stamp, &mut verified).unwrap();
    assert_eq!(verified, Some(stamp));
}

#[test]
fn failed_preflight_does_not_cache_a_stamp() {
    let backend = TrackedKeyState::new();
    let stamp = TargetStamp {
        hwnd: 0,
        generation: 1,
    };
    let mut verified = Some(TargetStamp {
        hwnd: 123,
        generation: 1,
    });
    assert!(ensure_preflight_for_target(&backend, stamp, &mut verified).is_err());
    assert_eq!(verified, None);
}

#[test]
fn target_change_is_rejected_at_the_final_send_boundary() {
    let target = AtomicIsize::new(123);
    let generation = AtomicU64::new(1);
    let stamp = TargetStamp {
        hwnd: 123,
        generation: 1,
    };
    assert!(target_stamp_still_current(&target, &generation, stamp));
    generation.store(2, Ordering::Release);
    assert!(!target_stamp_still_current(&target, &generation, stamp));
    target.store(456, Ordering::Release);
    assert!(!target_stamp_still_current(&target, &generation, stamp));
}

#[test]
fn focus_admission_uses_the_expected_hwnd_without_reloading_target() {
    let focus_active = AtomicBool::new(false);
    assert!(focus_matches_hwnd(false, &focus_active, 123));
}

#[test]
fn early_focus_gate_is_atomic_only_and_final_admission_queries_once() {
    sky_dispatch_win32::focus::reset_foreground_query_count();
    let focus_active = AtomicBool::new(true);
    let target = AtomicIsize::new(123);
    assert!(focus_matches(true, &focus_active));
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 0);

    let generation = AtomicU64::new(1);
    let expected = TargetStamp {
        hwnd: 123,
        generation: 1,
    };
    let _ = final_down_target_admission(FinalTargetSignals {
        expected,
        require_focus: true,
        focus_active: &focus_active,
        target_hwnd: &target,
        target_generation: &generation,
    });
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 1);
}

#[test]
fn final_down_admission_rejects_target_change_before_send() {
    let target = AtomicIsize::new(456);
    let generation = AtomicU64::new(2);
    let focus_active = AtomicBool::new(false);
    let expected = TargetStamp {
        hwnd: 123,
        generation: 1,
    };

    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected,
            require_focus: false,
            focus_active: &focus_active,
            target_hwnd: &target,
            target_generation: &generation,
        }),
        DownAdmission::TargetChanged
    );
}

#[test]
fn final_down_target_admission_checks_target_before_focus() {
    let target = AtomicIsize::new(123);
    let generation = AtomicU64::new(1);
    let focus_active = AtomicBool::new(false);
    let expected = TargetStamp {
        hwnd: 123,
        generation: 1,
    };

    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected,
            require_focus: true,
            focus_active: &focus_active,
            target_hwnd: &target,
            target_generation: &generation,
        }),
        DownAdmission::FocusLost
    );
}

#[test]
fn final_control_admission_rejects_each_command_state_in_priority_order() {
    let qpc_clock = QpcClock::initialize().expect("QPC clock");
    let quit_requested = AtomicBool::new(false);
    let skip_requested = AtomicBool::new(false);
    let panic_requested = AtomicBool::new(true);
    let desired_pause = AtomicBool::new(false);
    let heartbeat = AtomicU64::new(1);

    let admission = || {
        final_control_admission_with_lease(
            qpc_clock,
            DurationTicks::ZERO,
            FinalControlSignals {
                quit_requested: &quit_requested,
                skip_requested: &skip_requested,
                panic_requested: &panic_requested,
                desired_pause: &desired_pause,
                supervisor_heartbeat_ticks: &heartbeat,
            },
        )
        .expect("control gate")
        .0
    };

    assert_eq!(admission(), FinalControlAdmission::PanicRequested);
    panic_requested.store(false, Ordering::Release);
    quit_requested.store(true, Ordering::Release);
    assert_eq!(admission(), FinalControlAdmission::QuitRequested);
    quit_requested.store(false, Ordering::Release);
    skip_requested.store(true, Ordering::Release);
    assert_eq!(admission(), FinalControlAdmission::SkipRequested);
    skip_requested.store(false, Ordering::Release);
    desired_pause.store(true, Ordering::Release);
    assert_eq!(admission(), FinalControlAdmission::PauseRequested);
}

#[test]
fn authoritative_final_control_gate_uses_fresh_qpc_for_lease() {
    let qpc_clock = QpcClock::initialize().expect("QPC clock");
    let quit_requested = AtomicBool::new(true);
    let skip_requested = AtomicBool::new(true);
    let panic_requested = AtomicBool::new(true);
    let desired_pause = AtomicBool::new(true);
    let heartbeat = AtomicU64::new(1);
    let signals = || FinalControlSignals {
        quit_requested: &quit_requested,
        skip_requested: &skip_requested,
        panic_requested: &panic_requested,
        desired_pause: &desired_pause,
        supervisor_heartbeat_ticks: &heartbeat,
    };

    assert_eq!(
        final_control_admission_with_lease(qpc_clock, DurationTicks::from_raw(1), signals())
            .expect("gate query")
            .0,
        FinalControlAdmission::PanicRequested
    );

    panic_requested.store(false, Ordering::Release);
    assert_eq!(
        final_control_admission_with_lease(qpc_clock, DurationTicks::from_raw(1), signals())
            .expect("gate query")
            .0,
        FinalControlAdmission::QuitRequested
    );

    quit_requested.store(false, Ordering::Release);
    skip_requested.store(false, Ordering::Release);
    desired_pause.store(false, Ordering::Release);
    assert_eq!(
        final_control_admission_with_lease(qpc_clock, DurationTicks::from_raw(1), signals())
            .expect("gate query")
            .0,
        FinalControlAdmission::LeaseExpired
    );
}

#[test]
fn authored_up_only_does_not_send_after_final_control_rejection() {
    use super::test_support::ProductionDispatchTestHarness;

    for command in ["pause", "quit", "skip", "panic"] {
        let mut harness = ProductionDispatchTestHarness::new_uponly_release_with_gap(1_000);
        let calls = harness.configure_send_counter();
        harness.advance_playback_time_us(100_000);
        let plan = harness.plan_current_dispatch();
        match command {
            "pause" => harness.desired_pause.store(true, Ordering::Release),
            "quit" => harness.quit_requested.store(true, Ordering::Release),
            "skip" => harness.skip_requested.store(true, Ordering::Release),
            "panic" => harness.panic_requested.store(true, Ordering::Release),
            _ => unreachable!("test command table"),
        }
        let step = harness.dispatch_authored_with_plan(&plan);
        assert!(
            matches!(step, super::worker::DispatchStep::Continue),
            "{command} must reject UpOnly before transport: {step:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0, "{command} sent physically");
    }
}

#[test]
fn authored_up_only_does_not_send_after_lease_expiry() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_uponly_release_with_gap(1_000);
    let calls = harness.configure_send_counter();
    harness.advance_playback_time_us(100_000);
    let plan = harness.plan_current_dispatch();
    harness
        .supervisor_heartbeat_ticks
        .store(1, Ordering::Release);

    let step = harness.dispatch_authored_with_plan_and_lease(&plan, DurationTicks::from_raw(1));
    assert!(
        matches!(step, super::worker::DispatchStep::Continue),
        "lease must reject UpOnly before transport: {step:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn authored_up_only_is_not_blocked_by_focus_loss() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_uponly_release_with_gap(1_000);
    harness.config.focus.require_focus = true;
    let calls = harness.configure_send_counter();
    harness.advance_playback_time_us(100_000);
    let plan = harness.plan_current_dispatch();
    harness.focus_active.store(false, Ordering::Release);

    let step = harness.dispatch_authored_with_plan(&plan);

    assert!(
        matches!(step, super::worker::DispatchStep::Dispatched),
        "focus loss must not block UpOnly cleanup: {step:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn authored_focus_pause_publishes_progress_anchor() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.config.focus.require_focus = true;
    harness
        .resources
        .backend
        .set_probe(|_, _| sky_dispatch_win32::input::InstrumentPhysicalState::AllUp);
    harness.focus_active.store(false, Ordering::Release);
    harness.advance_playback_time_us(100_000);
    let plan = harness.plan_current_dispatch();

    let step = harness.dispatch_authored_with_plan(&plan);

    assert!(
        matches!(step, super::worker::DispatchStep::Continue),
        "unfocused authored Down must pause and replan: {step:?}"
    );
    let anchor = harness
        .progress_clock
        .load()
        .expect("progress anchor after focus pause");
    assert!(
        anchor.paused,
        "focus pause must be visible in the projection"
    );
}

#[test]
fn authored_up_only_is_not_blocked_by_target_change() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_uponly_release_with_gap(1_000);
    let calls = harness.configure_send_counter();
    harness.advance_playback_time_us(100_000);
    let plan = harness.plan_current_dispatch();
    harness.target_generation.store(1, Ordering::Release);
    harness.target_hwnd.store(99, Ordering::Release);

    let step = harness.dispatch_authored_with_plan(&plan);

    assert!(
        matches!(step, super::worker::DispatchStep::Dispatched),
        "target changes must not block UpOnly cleanup: {step:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn frozen_plan_dispatch_is_total_and_sends_at_most_once() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut empty = ProductionDispatchTestHarness::new_uponly_release();
    let empty_plan = super::worker::NextDispatchPlan::default();
    assert!(matches!(
        empty.dispatch_due_from_plan_for_test(&empty_plan),
        super::worker::DispatchStep::NoWork
    ));

    let mut authored = ProductionDispatchTestHarness::new_uponly_release();
    let authored_calls = authored.configure_send_counter();
    authored.advance_playback_time_us(100_000);
    let authored_plan = authored.plan_current_dispatch();
    assert!(matches!(
        authored.dispatch_due_from_plan_for_test(&authored_plan),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(authored_calls.load(Ordering::SeqCst), 1);

    let mut invalid_plan = authored_plan.clone();
    invalid_plan.authored_budget = None;
    assert!(!super::worker::plan_structure_is_valid(&invalid_plan));
    assert!(matches!(
        authored.dispatch_due_from_plan_for_test(&invalid_plan),
        super::worker::DispatchStep::Terminate(_)
    ));
}

#[test]
fn supervisor_lease_treats_equal_heartbeat_as_fresh() {
    let heartbeat = AtomicU64::new(1_000);
    assert_eq!(
        supervisor_lease_expired(
            QpcTicks::from_raw(1_000),
            DurationTicks::from_raw(100),
            &heartbeat,
        ),
        Ok(false)
    );
}

#[test]
fn supervisor_lease_preserves_fresh_boundary_and_expiration() {
    let heartbeat = AtomicU64::new(1_000);
    let timeout = DurationTicks::from_raw(100);
    assert_eq!(
        supervisor_lease_expired(QpcTicks::from_raw(1_050), timeout, &heartbeat),
        Ok(false)
    );
    assert_eq!(
        supervisor_lease_expired(QpcTicks::from_raw(1_100), timeout, &heartbeat),
        Ok(false)
    );
    assert_eq!(
        supervisor_lease_expired(QpcTicks::from_raw(1_101), timeout, &heartbeat),
        Ok(true)
    );
}

#[test]
fn supervisor_lease_disabled_is_never_expired() {
    let heartbeat = AtomicU64::new(1);
    assert_eq!(
        supervisor_lease_expired(QpcTicks::from_raw(2), DurationTicks::ZERO, &heartbeat),
        Ok(false)
    );
}

#[test]
fn recent_latency_ring_is_bounded_and_keeps_latest_values() {
    let mut ring = RecentLatencyRing::default();
    for value in 0..40 {
        ring.push(value);
    }
    assert_eq!(ring.to_vec().len(), 32);
    assert_eq!(ring.to_vec().first(), Some(&8));
    assert_eq!(ring.to_vec().last(), Some(&39));
}

#[test]
fn supervisor_lease_concurrent_publication_never_reports_clock_error() {
    let heartbeat = Arc::new(AtomicU64::new(1_000));
    let publisher_heartbeat = Arc::clone(&heartbeat);
    let publisher = std::thread::spawn(move || {
        for index in 0..10_000 {
            publisher_heartbeat.store(
                if index % 2 == 0 { 1_000 } else { 1_001 },
                Ordering::Release,
            );
        }
    });

    for _ in 0..10_000 {
        let result = supervisor_lease_expired(
            QpcTicks::from_raw(1_000),
            DurationTicks::from_raw(100),
            &heartbeat,
        );
        assert!(result.is_ok(), "concurrent heartbeat sample: {result:?}");
    }
    publisher.join().expect("heartbeat publisher must finish");
}

#[test]
fn exact_sender_duration_distinguishes_single_call_from_operation() {
    let clock = QpcClock::from_frequency_hz(std::num::NonZeroU64::new(qpc_frequency()).unwrap());
    let started = QpcTicks::from_raw(1_000);
    let completed = started
        .checked_add_duration(DurationTicks::from_raw(qpc_us_to_ticks(20).unwrap()))
        .unwrap();

    assert_eq!(
        exact_sender_durations(clock, Some(started), Some(completed), 1, false).unwrap(),
        (Some(20), Some(20))
    );
    assert_eq!(
        exact_sender_durations(clock, Some(started), Some(completed), 2, false).unwrap(),
        (Some(20), None)
    );
    assert_eq!(
        exact_sender_durations(clock, Some(started), Some(completed), 2, true).unwrap(),
        (Some(20), None)
    );
    assert!(exact_sender_durations(clock, None, Some(completed), 1, false).is_err());
}

#[test]
fn completion_error_ticks_preserves_signed_timeline_delta() {
    assert_eq!(
        signed_timeline_delta_ticks(TimelineTicks::from_raw(120), TimelineTicks::from_raw(100),),
        Ok(20)
    );
    assert_eq!(
        signed_timeline_delta_ticks(TimelineTicks::from_raw(100), TimelineTicks::from_raw(120),),
        Ok(-20)
    );
}

#[test]
fn completion_error_ticks_rejects_unrepresentable_signed_delta() {
    assert_eq!(
        signed_timeline_delta_ticks(
            TimelineTicks::from_raw(u64::MAX),
            TimelineTicks::from_raw(0),
        ),
        Err(sky_dispatch_core::time::TimeArithmeticError::Overflow)
    );
    assert_eq!(
        signed_timeline_delta_ticks(
            TimelineTicks::from_raw(0),
            TimelineTicks::from_raw((i64::MAX as u64) + 1),
        ),
        Ok(i64::MIN)
    );
}

#[test]
fn completion_residuals_keep_authored_and_effective_deadlines_distinct() {
    let completed = TimelineTicks::from_raw(950);
    let effective_deadline = TimelineTicks::from_raw(900);
    let authored_deadline = TimelineTicks::from_raw(1_000);

    assert_eq!(
        signed_timeline_delta_ticks(completed, effective_deadline),
        Ok(50)
    );
    assert_eq!(
        signed_timeline_delta_ticks(completed, authored_deadline),
        Ok(-50)
    );
}

#[test]
fn wait_error_is_relative_to_deadline() {
    assert_eq!(
        wake_lateness_ticks(
            TimelineTicks::from_raw(20_000),
            TimelineTicks::from_raw(20_000),
        )
        .expect("on-time wake")
        .as_u64(),
        0
    );
    assert_eq!(
        wake_lateness_ticks(
            TimelineTicks::from_raw(20_500),
            TimelineTicks::from_raw(20_000),
        )
        .expect("late wake")
        .as_u64(),
        500
    );
    assert_eq!(
        wake_lateness_ticks(
            TimelineTicks::from_raw(19_900),
            TimelineTicks::from_raw(20_000),
        )
        .expect("early wake")
        .as_u64(),
        0
    );
}

#[test]
fn native_trace_counts_are_semantic_and_summary_uses_them() {
    let record = RtTraceRecord::dispatched(
        TraceContext {
            event_index: 7,
            kind: TRACE_KIND_DOWN,
            outcome: trace_outcome_code("sent"),
            polyphony: 3,
            flags: TRACE_FLAG_SENT_FULL,
            win32_error: 0,
        },
        TraceTiming {
            authored_ticks: TimelineTicks::from_raw(10),
            effective_deadline_ticks: TimelineTicks::from_raw(12),
            wake_ticks: TimelineTicks::from_raw(13),
            final_admission_ticks: Some(TimelineTicks::from_raw(20)),
            sendinput_completed_ticks: Some(TimelineTicks::from_raw(25)),
            completion_residual_us: 5,
            core_post_send_duration_us: 4,
            post_send_metrics_available: true,
            dispatch_start_error_ticks: 8,
            completion_error_ticks: 1,
            authored_completion_error_ticks: 2,
        },
        TraceDelivery {
            requested: 3,
            sent: 2,
            skipped: 1,
            send_attempts: 2,
        },
    )
    .unwrap();

    assert_eq!(record.requested_count, 3);
    assert_eq!(record.sent_count, 2);
    assert_eq!(record.skipped_count, 1);
    assert_eq!(record.send_attempts, 2);
    assert_eq!(record.send_started_ticks, 20);
    assert_eq!(record.send_completed_ticks, 25);
    assert_eq!(record.core_post_send_duration_us, 4);

    let mut summary = super::NativeTelemetrySummary::default();
    summary.observe(&record);
    assert_eq!(summary.requested_key_count, 3);
    assert_eq!(summary.sent_key_count, 2);
    assert_eq!(summary.skipped_key_count, 1);
}

#[test]
fn native_trace_constructor_rejects_inconsistent_counts() {
    let result = RtTraceRecord::dispatched(
        TraceContext {
            event_index: 0,
            kind: TRACE_KIND_DOWN,
            outcome: trace_outcome_code("sent"),
            polyphony: 1,
            flags: 0,
            win32_error: 0,
        },
        TraceTiming {
            authored_ticks: TimelineTicks::ZERO,
            effective_deadline_ticks: TimelineTicks::ZERO,
            wake_ticks: TimelineTicks::ZERO,
            final_admission_ticks: None,
            sendinput_completed_ticks: None,
            completion_residual_us: 0,
            core_post_send_duration_us: 0,
            post_send_metrics_available: false,
            dispatch_start_error_ticks: 0,
            completion_error_ticks: 0,
            authored_completion_error_ticks: 0,
        },
        TraceDelivery {
            requested: 1,
            sent: 2,
            skipped: 0,
            send_attempts: 1,
        },
    );
    assert!(matches!(
        result,
        Err(sky_dispatch_core::time::TimeArithmeticError::Overflow)
    ));
}

#[test]
fn native_summary_ignores_non_backend_trace() {
    let record = RtTraceRecord::dispatched(
        TraceContext {
            event_index: 0,
            kind: TRACE_KIND_DOWN,
            outcome: trace_outcome_code("blocked_unfocused"),
            polyphony: 3,
            flags: 0,
            win32_error: 0,
        },
        TraceTiming {
            authored_ticks: TimelineTicks::ZERO,
            effective_deadline_ticks: TimelineTicks::ZERO,
            wake_ticks: TimelineTicks::ZERO,
            final_admission_ticks: None,
            sendinput_completed_ticks: None,
            completion_residual_us: 0,
            core_post_send_duration_us: 0,
            post_send_metrics_available: false,
            dispatch_start_error_ticks: 0,
            completion_error_ticks: 0,
            authored_completion_error_ticks: 0,
        },
        TraceDelivery {
            requested: 0,
            sent: 0,
            skipped: 0,
            send_attempts: 0,
        },
    )
    .unwrap();
    let mut summary = super::NativeTelemetrySummary::default();

    summary.observe(&record);

    assert_eq!(summary.dispatch_count, 0);
    assert_eq!(summary.requested_key_count, 0);
}

#[test]
fn deadline_mapper_uses_the_current_sample_without_overhead_drift() {
    let now_ticks = QpcTicks::from_raw(10_000_000);
    let logical_now_us = qpc_ticks_to_us(now_ticks).unwrap();
    let target = deadline_target_ticks(now_ticks, logical_now_us, logical_now_us + 1_000);
    assert_eq!(
        target.as_u64() - now_ticks.as_u64(),
        qpc_us_to_ticks(1_000).unwrap()
    );
}

#[test]
fn first_authored_zero_timestamp_can_use_a_future_physical_anchor() {
    let now_ticks = QpcTicks::from_raw(10_000_000);
    let now_qpc_us = qpc_ticks_to_us(now_ticks).unwrap();
    let startup_guard_us = 1_000;
    let lead_us = 500;
    let anchor_us = now_qpc_us
        .saturating_add(startup_guard_us)
        .saturating_add(lead_us);
    let clock = QpcClock::from_frequency_hz(std::num::NonZeroU64::new(qpc_frequency()).unwrap());
    let target =
        anchored_dispatch_target_ticks(clock, now_ticks, now_qpc_us, anchor_us, 0, lead_us)
            .unwrap();

    assert_eq!(
        target.as_u64() - now_ticks.as_u64(),
        qpc_us_to_ticks(startup_guard_us).unwrap()
    );
}

#[test]
fn startup_completion_cost_uses_exact_waited_target_after_wake() {
    let clock = QpcClock::from_frequency_hz(std::num::NonZeroU64::new(1_000_000).unwrap());
    let target =
        anchored_dispatch_target_ticks(clock, QpcTicks::from_raw(9_540), 9_540, 10_000, 0, 500)
            .unwrap();

    assert_eq!(target, QpcTicks::from_raw(9_500));
    assert_eq!(
        QpcTicks::from_raw(9_700)
            .checked_duration_since(target)
            .unwrap()
            .as_u64(),
        200
    );
}

#[test]
fn focus_gate_requires_both_validation_and_foreground_match() {
    assert!(!focus_gate_matches(true, false, 123, true));
    assert!(!focus_gate_matches(true, true, 123, false));
    assert!(focus_gate_matches(true, true, 123, true));
    assert!(!focus_gate_matches(true, true, 0, false));
    assert!(focus_gate_matches(false, false, 123, false));
}

#[test]
fn input_path_degraded_is_not_key_drop_state() {
    let mut window = HealthWindow::<64>::default();
    let policy = HealthWindowPolicy {
        minimum_samples: 64,
        bad_sample_count: 4,
        degrade_hold_us: 1_000_000,
        recovery_hold_us: 2_000_000,
    };

    for elapsed_us in (0..=1_070_000).step_by(1_000) {
        record_input_path_health(400, 300, elapsed_us, policy, &mut window);
    }

    assert!(window.is_degraded());
}

#[test]
fn input_path_health_window_stays_bounded_and_tracks_latest_samples() {
    let mut window = HealthWindow::<64>::default();
    let policy = HealthWindowPolicy {
        minimum_samples: 64,
        bad_sample_count: 4,
        degrade_hold_us: 1_000_000,
        recovery_hold_us: 2_000_000,
    };

    for _ in 0..10_000 {
        record_input_path_health(400, 300, 0, policy, &mut window);
    }

    assert_eq!(window.sample_count(), 64);
    assert_eq!(window.bad_count(), 64);

    for _ in 0..64 {
        record_input_path_health(100, 300, 0, policy, &mut window);
    }

    assert_eq!(window.sample_count(), 64);
    assert_eq!(window.bad_count(), 0);
}

#[test]
fn input_path_health_uses_full_window_and_recovers_without_latching() {
    let mut window = HealthWindow::<64>::default();
    let policy = HealthWindowPolicy {
        minimum_samples: 64,
        bad_sample_count: 4,
        degrade_hold_us: 1_000_000,
        recovery_hold_us: 2_000_000,
    };

    for sample in 0..64 {
        record_input_path_health(400, 300, sample as u64 * 1_000, policy, &mut window);
    }
    assert!(
        !window.is_degraded(),
        "a partial duration must not degrade the warning"
    );
    record_input_path_health(400, 300, 1_063_000, policy, &mut window);
    assert!(window.is_degraded());

    for elapsed_us in (1_064_000..=1_127_000).step_by(1_000) {
        record_input_path_health(100, 300, elapsed_us, policy, &mut window);
    }
    assert!(
        window.is_degraded(),
        "healthy samples need the full recovery duration"
    );
    record_input_path_health(100, 300, 3_127_000, policy, &mut window);
    assert!(!window.is_degraded());

    let disabled = HealthWindowPolicy {
        minimum_samples: 0,
        ..policy
    };
    record_input_path_health(400, 0, 3_128_000, disabled, &mut window);
    assert_eq!(window.sample_count(), 0);
}

#[test]
fn spin_threshold_rises_immediately_and_decays_with_hysteresis() {
    assert_eq!(adjust_spin_threshold(700, 1_000), 1_000);
    assert_eq!(adjust_spin_threshold(1_000, 700), 950);
    assert_eq!(adjust_spin_threshold(1_000, 100), 950);
}

#[test]
fn single_outlier_in_periodic_reprobe_does_not_raise_threshold_to_cap() {
    let stats = WakeErrorStats {
        p50_us: 300,
        p95_us: 1_500,
        p99_us: 1_500,
        max_us: 1_500,
        robust_us: 300,
    };
    assert_eq!(derive_spin_threshold_us(stats.robust_us, 700), 700);
}

#[test]
fn module_line_limits_strictly_respected() {
    let dispatch = [
        ("worker/dispatch/mod.rs", 250),
        ("worker/dispatch/authored.rs", 900),
        ("worker/dispatch/observer.rs", 900),
        ("worker/dispatch/timing.rs", 700),
    ];
    for (relative, hard_limit) in dispatch {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/engine")
            .join(relative);
        let text = std::fs::read_to_string(&path).expect("dispatch source is present");
        let line_count = text.lines().count();
        assert!(line_count > 0, "{relative} is empty");
        assert!(
            line_count <= hard_limit,
            "{relative} has {line_count} lines (dispatch hard limit {hard_limit})"
        );
    }
    for legacy in [
        "worker/downs.rs",
        "worker/releases.rs",
        "worker/down_outcome.rs",
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/engine")
            .join(legacy);
        assert!(
            !path.exists(),
            "legacy dispatch module must not exist: {legacy}"
        );
    }
}

#[test]
fn failed_release_outcome_and_completion_metrics_are_distinguishable() {
    assert_eq!(release_runtime_outcome(0, 1, 1, false), "sent");
    assert_eq!(
        release_runtime_outcome(100, 1, 2, false),
        "deferred_partial_note_off"
    );
    assert_eq!(release_runtime_outcome(0, 0, 1, true), "failed_note_off");
    assert_eq!(
        release_runtime_outcome(100, 0, 1, true),
        "deferred_failed_note_off"
    );
}

#[test]
fn termination_error_aggregation_preserves_primary_and_secondary_errors() {
    let mut primary = None;
    let mut secondary = Vec::new();
    record_termination_error(
        &mut primary,
        &mut secondary,
        "coordinator pre-cleanup mismatch".to_string(),
    );
    record_termination_error(
        &mut primary,
        &mut secondary,
        "release verification failed".to_string(),
    );
    record_termination_error(
        &mut primary,
        &mut secondary,
        "release verification failed".to_string(),
    );

    assert_eq!(primary.as_deref(), Some("coordinator pre-cleanup mismatch"));
    assert_eq!(secondary, vec!["release verification failed"]);
}

#[test]
fn persistent_zero_progress_down_aborts_before_the_next_authored_chord() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down-1".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 1_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up-1".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 2_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "down-2".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 3_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "up-2".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15, 0x16])
        .expect("valid fault-injection schedule");
    let script = FaultInjectionScript {
        entries: vec![
            (
                0,
                InjectedSendOutcome::Zero {
                    latency_ticks: 0,
                    win32_error: 1460,
                },
            ),
            (
                1,
                InjectedSendOutcome::Zero {
                    latency_ticks: 0,
                    win32_error: 1460,
                },
            ),
        ],
        ..FaultInjectionScript::default()
    };
    let session = NativeDispatchSession::new(test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: script,
        },
    ))
    .expect("test session admission");

    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, "error", "zero progress must be terminal");
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry"))
            .expect("valid telemetry JSON");
    let records = telemetry["records"].as_array().expect("records array");
    assert!(
        !records
            .iter()
            .any(|record| { record["event_index"] == 2 && record["runtime_outcome"] == "sent" })
    );
}

#[test]
fn mixed_packet_partial_fault_stops_before_committing_retrigger() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 1_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "retrigger-up".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 1_000,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "retrigger-down".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15, 0x16])
        .expect("valid mixed packet schedule");
    let script = FaultInjectionScript {
        entries: vec![(
            1,
            InjectedSendOutcome::Partial {
                inserted: 1,
                latency_ticks: 0,
                win32_error: 5,
            },
        )],
        ..FaultInjectionScript::none()
    };
    let session = NativeDispatchSession::new(test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: script,
        },
    ))
    .expect("test session admission");

    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, "error");
    assert!(snapshot.sendinput_partial_events >= 1);
    assert!(snapshot.chord_split_events >= 1);
    assert_eq!(snapshot.possibly_active_count, 0);
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry"))
            .expect("valid telemetry JSON");
    let records = telemetry["records"].as_array().expect("records array");
    assert!(
        !records
            .iter()
            .any(|record| { record["event_index"] == 2 && record["runtime_outcome"] == "sent" })
    );
}

#[test]
fn mixed_same_key_retrigger_success_commits_new_generation() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Down,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "retrigger-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![0x15],
            reason: "retrigger-up".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 1_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "release-one".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 4,
            kind: ActionKind::Up,
            scheduled_us: 1_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "release-two".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15, 0x16])
        .expect("valid mixed retrigger schedule");
    let session = NativeDispatchSession::new(test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    ))
    .expect("test session admission");

    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.status, "finished",
        "terminal error: {:?}",
        snapshot.terminal_error
    );
    assert_eq!(snapshot.generation_status_counts["released"], 3);
    assert_eq!(snapshot.active_count, 0);
    assert_eq!(snapshot.possibly_active_count, 0);

    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry"))
            .expect("valid telemetry JSON");
    let mixed = telemetry["records"]
        .as_array()
        .expect("records array")
        .iter()
        .find(|record| record["kind"].as_u64() == Some(2))
        .expect("successful mixed record");
    assert_eq!(mixed["requested_count"].as_u64(), Some(3));
    assert_eq!(mixed["sent_count"].as_u64(), Some(3));
    assert_eq!(mixed["polyphony"].as_u64(), Some(3));
}

#[test]
fn mixed_same_key_retrigger_telemetry_preserves_two_events() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![0x15],
            reason: "retrigger-up".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![0x15],
            reason: "retrigger-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 1_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "release".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("valid same-key mixed schedule");
    let session = NativeDispatchSession::new(test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    ))
    .expect("test session admission");

    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(session.snapshot().status, "finished");

    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry"))
            .expect("valid telemetry JSON");
    let mixed = telemetry["records"]
        .as_array()
        .expect("records array")
        .iter()
        .find(|record| record["kind"].as_u64() == Some(2))
        .expect("successful same-key mixed record");
    assert_eq!(mixed["requested_count"].as_u64(), Some(2));
    assert_eq!(mixed["sent_count"].as_u64(), Some(2));
    assert_eq!(mixed["polyphony"].as_u64(), Some(2));
}

#[test]
fn join_timeout_does_not_poison_running_session() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 100_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("valid lifecycle test schedule");
    let session = NativeDispatchSession::new(test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 100_000,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    ))
    .expect("test session admission");

    session.start().expect("worker start");
    assert!(!session.join(Duration::from_millis(1)).expect("timed join"));
    assert!(session.snapshot().is_running);
    session.pause().expect("pause after join timeout");
    session.resume().expect("resume after join timeout");
    session.quit().expect("quit after join timeout");
    assert!(session.join(Duration::from_secs(5)).expect("final join"));
    assert!(session.join(Duration::from_millis(1)).expect("second join"));
    assert_eq!(session.terminal_outcome(), Some("quit"));
}

#[test]
fn worker_scheduling_guards_lifetime_is_preserved_until_resources_drop() {
    use crate::engine::worker::{WorkerResources, WorkerSchedulingGuards};
    use sky_dispatch_win32::mmcss::{MmcssGuard, PriorityMode};
    use sky_dispatch_win32::power::PowerThrottlingGuard;
    use std::sync::atomic::AtomicUsize;

    let drop_counter = Arc::new(AtomicUsize::new(0));

    let guards = WorkerSchedulingGuards {
        priority: MmcssGuard::acquire(PriorityMode::Off),
        power: PowerThrottlingGuard::disable_current_thread(),
        drop_probe: Some(Arc::clone(&drop_counter)),
    };

    assert_eq!(drop_counter.load(Ordering::SeqCst), 0);
    assert_eq!(guards.is_priority_active(), guards.priority.is_active());
    assert_eq!(guards.is_power_active(), guards.power.is_active());
    assert_eq!(guards.priority_label(), guards.priority.acquired());

    let qpc_clock = QpcClock::initialize().expect("qpc clock");
    let backend = TrackedKeyState::with_qpc_clock(qpc_clock);
    let schedule = startup_boundary_schedule();
    let min_hold_ticks = qpc_clock.duration_from_us(10_000).expect("min hold ticks");
    let coordinator = sky_dispatch_core::coordinator::RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        10_000,
        min_hold_ticks,
        |us| {
            qpc_clock
                .timeline_from_us(us)
                .map_err(|e| super::CoordinatorError::TimeConversion(format!("{e:?}")))
        },
    )
    .expect("coordinator");
    let playback =
        super::PlaybackClockState::new(qpc_clock.now().expect("qpc now"), DurationTicks::ZERO)
            .expect("playback");
    let telemetry = TelemetryCollector::new(TelemetryMode::Ring, 64);
    let waiter = sky_dispatch_win32::wait::HybridWaiter::with_options(true, true);

    let resources = WorkerResources {
        clock: qpc_clock,
        waiter,
        backend,
        coordinator,
        playback,
        telemetry: Arc::new(parking_lot::Mutex::new(telemetry)),
        scheduling: guards,
    };

    // Before resources drop, guards are still active and drop counter is 0
    assert_eq!(drop_counter.load(Ordering::SeqCst), 0);

    // Drop WorkerResources explicitly (mimicking terminal worker finalization drop)
    drop(resources);

    // Guard drop must happen exactly once on resources drop
    assert_eq!(drop_counter.load(Ordering::SeqCst), 1);
}

#[test]
fn session_live_snapshot_reflects_scheduling_guard_lifetime() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 1_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("valid schedule");
    let session = NativeDispatchSession::new(test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 100,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    ))
    .expect("test session admission");

    session.start().expect("worker start");
    session
        .join(Duration::from_secs(5))
        .expect("session finish");

    let snapshot = session.snapshot();
    assert_eq!(snapshot.rt_priority_acquired, "off");
    assert!(!snapshot.power_throttling_disabled);
}

#[cfg(not(windows))]
#[test]
fn production_session_rejects_non_windows() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 1_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("valid platform admission schedule");
    let result =
        NativeDispatchSession::new(test_session_options(schedule, 1, BackendConfig::Production));
    assert!(matches!(
        result,
        Err(error) if error == "production native dispatch is supported only on Windows"
    ));
}

#[test]
fn large_epoch_timing_derivation_keeps_send_and_post_send_durations_distinct() {
    use super::WorkerMetricsLocal;
    use super::worker::{
        DispatchHealthObservation, DispatchHealthOptions, observe_dispatch_health,
    };

    let send_duration_us = 400u64;
    let send_completed_elapsed_us = 10_000_500u64;
    let iteration_ready_us = 10_000_620u64;

    let post_send_duration_us = iteration_ready_us.saturating_sub(send_completed_elapsed_us);
    assert_eq!(post_send_duration_us, 120);

    let obs = DispatchHealthObservation {
        send_duration_us,
        post_send_duration_us,
        post_send_metrics_available: true,
        path: crate::engine::worker::DispatchPath::UpOnly { up_count: 1 },
        send_warn_us: 2000,
        core_post_send_warn_us: 1000,
        elapsed_us: send_completed_elapsed_us,
    };

    let mut send_window = Default::default();
    let mut bk_window = Default::default();
    let mut local_metrics = WorkerMetricsLocal::default();
    let options = DispatchHealthOptions::default();

    observe_dispatch_health(
        obs,
        options.window_policy(),
        &mut send_window,
        &mut bk_window,
        &mut local_metrics,
    );

    assert_eq!(local_metrics.core_post_send_degraded_samples, 0);
}

#[test]
fn note_on_lateness_shifts_note_off_floor_one_to_one() {
    use sky_dispatch_core::compile::compile_runtime_intents;
    use sky_dispatch_core::coordinator::RuntimeDispatchCoordinator;
    use sky_dispatch_core::model::{ActionKind, KeyActionInput};
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 20_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up".to_string().into(),
            },
        ],
        &[0x15],
    )
    .expect("schedule");

    let min_hold_us = 10_000u64;
    let min_hold_ticks = DurationTicks::from_raw(min_hold_us);
    let mut coordinator =
        RuntimeDispatchCoordinator::try_new_ticks(schedule, min_hold_us, min_hold_ticks, |us| {
            Ok(TimelineTicks::from_raw(us))
        })
        .expect("coordinator");

    let prepared = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(1_000))
        .unwrap()
        .unwrap();

    let down_started = TimelineTicks::from_raw(10_000);
    let down_completed = TimelineTicks::from_raw(15_000);
    coordinator
        .commit_packet_success(prepared, down_started, down_completed)
        .unwrap();

    let active = coordinator.active_for_slot(0).unwrap();
    // Min-hold floor is down_completed (15,000) + min_hold (10,000) = 25,000us
    assert_eq!(
        active.release_not_before_ticks,
        TimelineTicks::from_raw(25_000)
    );
}

#[test]
fn fast_note_on_preserves_authored_note_off() {
    use sky_dispatch_core::compile::compile_runtime_intents;
    use sky_dispatch_core::coordinator::RuntimeDispatchCoordinator;
    use sky_dispatch_core::model::{ActionKind, KeyActionInput};
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 50_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up".to_string().into(),
            },
        ],
        &[0x15],
    )
    .expect("schedule");

    let min_hold_us = 10_000u64;
    let min_hold_ticks = DurationTicks::from_raw(min_hold_us);
    let mut coordinator =
        RuntimeDispatchCoordinator::try_new_ticks(schedule, min_hold_us, min_hold_ticks, |us| {
            Ok(TimelineTicks::from_raw(us))
        })
        .expect("coordinator");

    let prepared = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(1_000))
        .unwrap()
        .unwrap();

    let down_started = TimelineTicks::from_raw(1_000);
    let down_completed = TimelineTicks::from_raw(1_050);
    coordinator
        .commit_packet_success(prepared, down_started, down_completed)
        .unwrap();

    let active = coordinator.active_for_slot(0).unwrap();
    // Min-hold floor is down_completed (1,050) + min_hold (10,000) = 11,050us
    assert_eq!(
        active.release_not_before_ticks,
        TimelineTicks::from_raw(11_050)
    );
}

#[test]
fn late_first_event_does_not_move_second_event() {
    use sky_dispatch_core::compile::compile_runtime_intents;
    use sky_dispatch_core::coordinator::RuntimeDispatchCoordinator;
    use sky_dispatch_core::model::{ActionKind, KeyActionInput};
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "chord1".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 20_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "chord2".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 50_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "chord3".to_string().into(),
            },
        ],
        &[0x15, 0x16, 0x17],
    )
    .expect("schedule");

    let mut coordinator = RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        10_000,
        DurationTicks::from_raw(10_000),
        |us| Ok(TimelineTicks::from_raw(us)),
    )
    .expect("coordinator");

    let p1 = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(11_000))
        .unwrap()
        .unwrap();
    assert_eq!(p1.effective_scheduled_ticks, TimelineTicks::from_raw(1_000));
    coordinator
        .commit_packet_success(
            p1,
            TimelineTicks::from_raw(11_000),
            TimelineTicks::from_raw(11_050),
        )
        .unwrap();

    let p2 = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(20_000))
        .unwrap()
        .unwrap();
    assert_eq!(
        p2.effective_scheduled_ticks,
        TimelineTicks::from_raw(20_000)
    );
    coordinator
        .commit_packet_success(
            p2,
            TimelineTicks::from_raw(20_000),
            TimelineTicks::from_raw(20_050),
        )
        .unwrap();

    let p3 = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(50_000))
        .unwrap()
        .unwrap();
    assert_eq!(
        p3.effective_scheduled_ticks,
        TimelineTicks::from_raw(50_000)
    );

    let delta_b_a = p2.effective_scheduled_ticks.as_u64() - p1.effective_scheduled_ticks.as_u64();
    let delta_c_b = p3.effective_scheduled_ticks.as_u64() - p2.effective_scheduled_ticks.as_u64();
    assert_eq!(delta_b_a, 19_000);
    assert_eq!(delta_c_b, 30_000);
}

#[test]
fn release_floor_does_not_move_unrelated_future_action() {
    use sky_dispatch_core::compile::compile_runtime_intents;
    use sky_dispatch_core::coordinator::RuntimeDispatchCoordinator;
    use sky_dispatch_core::model::{ActionKind, KeyActionInput};
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "down A".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 20_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up A".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 30_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "down B".to_string().into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("schedule");

    let mut coordinator = RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        10_000,
        DurationTicks::from_raw(10_000),
        |us| Ok(TimelineTicks::from_raw(us)),
    )
    .expect("coordinator");

    let p_down_a = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(1_000))
        .unwrap()
        .unwrap();
    coordinator
        .commit_packet_success(
            p_down_a,
            TimelineTicks::from_raw(1_000),
            TimelineTicks::from_raw(15_000),
        )
        .unwrap();

    let p_up_a = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(25_000))
        .unwrap()
        .unwrap();
    assert_eq!(
        p_up_a.effective_scheduled_ticks,
        TimelineTicks::from_raw(25_000)
    );
    coordinator
        .commit_packet_success(
            p_up_a,
            TimelineTicks::from_raw(25_000),
            TimelineTicks::from_raw(25_050),
        )
        .unwrap();

    let p_down_b = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(30_000))
        .unwrap()
        .unwrap();
    assert_eq!(
        p_down_b.effective_scheduled_ticks,
        TimelineTicks::from_raw(30_000)
    );
}

#[test]
fn zero_lateness_preserves_exact_authored_timestamps() {
    use sky_dispatch_core::compile::compile_runtime_intents;
    use sky_dispatch_core::coordinator::RuntimeDispatchCoordinator;
    use sky_dispatch_core::model::{ActionKind, KeyActionInput};
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "chord1".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 20_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "chord2".to_string().into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("schedule");

    let mut coordinator = RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        10_000,
        DurationTicks::from_raw(10_000),
        |us| Ok(TimelineTicks::from_raw(us)),
    )
    .expect("coordinator");

    let p1 = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(1_000))
        .unwrap()
        .unwrap();
    assert_eq!(p1.effective_scheduled_ticks, TimelineTicks::from_raw(1_000));
}

/// §8.12 — slow observer must not shift the next authored dispatch when slack
/// is insufficient.  A 10 ms A→B gap with a 15 ms observer budget (+0.5 ms
/// margin) defers the drain; artificial 20 ms observer cost therefore cannot
/// push B's physical send by ~20 ms, cannot rewrite B's authored timestamp,
/// and must not trigger a WorkerLate timeline rebase.
#[test]
fn slow_observer_defers_when_slack_is_insufficient() {
    use crate::engine::observer_test_hooks::{
        observer_test_hook_guard, set_observer_artificial_cost_us,
    };

    let _observer_hooks = observer_test_hook_guard();
    set_observer_artificial_cost_us(20_000);

    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "A-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 105_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "A-up".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 110_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "B-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 115_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "B-up".to_string().into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("slow-observer schedule");

    let mut options = test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.wait.supervisor_lease_timeout_us = 3_000_000;
    options.telemetry.capacity = 16;

    let session = NativeDispatchSession::new(options).expect("session");
    session.start().expect("start");
    while !session.snapshot().is_finished {
        session.heartbeat().expect("heartbeat");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(session.join(Duration::from_secs(5)).expect("join"));

    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.outcome,
        Some("finished".to_string()),
        "terminal error: {:?}",
        snapshot.terminal_error
    );
    assert_eq!(
        snapshot.timeline_rebase_count, 0,
        "forbidden authored timeline rebase must be 0"
    );
    assert_eq!(
        snapshot.timeline_rebase_last_reason, None,
        "no timeline rebase reason expected"
    );

    // With insufficient slack the 20 ms artificial cost must not dominate the
    // observer duration metric (drain is deferred until later idle/end).
    // Authored B target stays at 110 ms via telemetry.
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry"))
            .expect("valid telemetry JSON");
    let records = telemetry["records"].as_array().expect("records");
    let b_down = records
        .iter()
        .find(|r| r["event_index"].as_u64() == Some(2))
        .expect("B-down record");
    let authored = b_down["authored_ticks"].as_u64().expect("authored");
    let started = b_down["send_started_ticks"].as_u64().expect("started");
    let clock = QpcClock::initialize().expect("QPC");
    let expected_authored = clock
        .duration_from_us(110_000)
        .expect("110ms ticks")
        .as_u64();
    assert_eq!(
        authored, expected_authored,
        "B authored timestamp must not change"
    );
    // Physical start must not be shifted by the full artificial 20 ms observer cost.
    let start_error_ticks = started.saturating_sub(authored);
    let start_error_us = clock
        .duration_to_us(DurationTicks::from_raw(start_error_ticks))
        .unwrap_or(0);
    assert!(
        start_error_us < 12_000,
        "B physical start slipped {start_error_us}us; observer must not add ~20ms"
    );
}

/// §8.12 — when gap slack is ample, the observer may drain and the worker
/// must rebuild the plan from a fresh QPC sample without shifting authored
/// timestamps or rebasing the timeline.
#[test]
fn slow_observer_drains_in_ample_slack_without_rebase() {
    use crate::engine::observer_test_hooks::{
        observer_test_hook_guard, set_observer_artificial_cost_us,
    };

    let _observer_hooks = observer_test_hook_guard();
    // 2 ms artificial cost with default 5 ms budget; 50 ms gap is ample.
    set_observer_artificial_cost_us(2_000);

    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "A-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 105_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "A-up".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 150_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "B-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 155_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "B-up".to_string().into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("ample-slack schedule");

    let mut options = test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.wait.supervisor_lease_timeout_us = 3_000_000;
    options.telemetry.capacity = 16;

    let session = NativeDispatchSession::new(options).expect("session");
    session.start().expect("start");
    while !session.snapshot().is_finished {
        session.heartbeat().expect("heartbeat");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(session.join(Duration::from_secs(5)).expect("join"));

    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.outcome,
        Some("finished".to_string()),
        "terminal error: {:?}",
        snapshot.terminal_error
    );
    assert_eq!(snapshot.timeline_rebase_count, 0);
    // Observer did run (ample slack); duration should be observed.
    assert!(
        snapshot.observer_duration_max_us >= 1_000,
        "expected observer drain under ample slack, got observer_duration_max_us={}",
        snapshot.observer_duration_max_us
    );
    assert_eq!(snapshot.observer_dropped_samples, 0);

    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry"))
            .expect("valid telemetry JSON");
    let records = telemetry["records"].as_array().expect("records");
    let b_down = records
        .iter()
        .find(|r| r["event_index"].as_u64() == Some(2))
        .expect("B-down record");
    let clock = QpcClock::initialize().expect("QPC");
    let expected_authored = clock
        .duration_from_us(150_000)
        .expect("150ms ticks")
        .as_u64();
    assert_eq!(
        b_down["authored_ticks"].as_u64().expect("authored"),
        expected_authored,
        "B authored timestamp must stay at 150ms"
    );
}

#[test]
fn invariant_mismatch_prevents_sender_invocation() {
    use sky_dispatch_win32::input::{PlatformSendResult, TrackedKeyState};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    let send_counter = Arc::new(AtomicU64::new(0));
    let counter_clone = send_counter.clone();
    let backend = TrackedKeyState::with_emitter(move |codes, _key_up| {
        counter_clone.fetch_add(1, Ordering::SeqCst);
        PlatformSendResult {
            requested: codes.len() as u8,
            inserted: codes.len() as u8,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            win32_error: 0,
            timing_error: None,
        }
    });

    let target = AtomicIsize::new(456);
    let generation = AtomicU64::new(2);
    let focus_active = AtomicBool::new(false);
    let expected = TargetStamp {
        hwnd: 123,
        generation: 1,
    };

    let admission = final_down_target_admission(FinalTargetSignals {
        expected,
        require_focus: false,
        focus_active: &focus_active,
        target_hwnd: &target,
        target_generation: &generation,
    });

    assert_eq!(admission, DownAdmission::TargetChanged);
    assert_eq!(
        send_counter.load(Ordering::SeqCst),
        0,
        "sender seam must not be invoked on target stamp mismatch"
    );
    let _ = backend;
}
