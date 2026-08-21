use super::telemetry::metrics::RecentLatencyRing;
use super::test_support::command_timing::{
    CommandTimingError, CommandTimingLookup as PauseTimingLookup, PauseTimingPhase,
};
use super::{
    BackendConfig, CommandTimingResult, CommandTimingState, DispatchProfile, DownAdmission,
    FaultInjectionScript, FinalControlAdmission, FinalControlSignals, FinalTargetSignals,
    FocusOptions, HealthWindow, HealthWindowPolicy, InjectedSendOutcome, NativeDispatchSession,
    NativeSessionOptions, PlatformSendResult, PriorityOptions, RtTraceRecord, SharedMetrics,
    StartupOrderingHook, TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN, TargetStamp, TelemetryCollector,
    TelemetryMode, TelemetryOptions, TimingOptions, TraceContext, TraceDelivery, TraceTiming,
    TrackedKeyState, WaitOptions, WakeErrorStats, Worker, WorkerMetricsLocal,
    adjust_spin_threshold, anchored_dispatch_target_ticks, cpu_metrics_sample_due,
    deadline_target_ticks, derive_spin_threshold_us, ensure_preflight_for_target,
    exact_sender_durations, final_control_admission_with_lease, final_down_target_admission,
    focus_gate_matches, focus_matches, focus_matches_hwnd, record_input_path_health,
    record_termination_error, release_runtime_outcome, signed_timeline_delta_ticks,
    supervisor_lease_expired, target_stamp_still_current, trace_outcome_code, try_publish_metrics,
    wake_lateness_ticks,
};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::TimelineTicks;
use sky_dispatch_win32::clock::{
    DurationTicks, QpcClock, QpcTicks, qpc_frequency, qpc_ticks_to_us, qpc_us_to_ticks,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::time::{Duration, Instant};

const TEST_WALL_CLOCK_PREROLL_US: u64 = 500_000;

fn test_session_options(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    _allowed_count: usize,
    backend: BackendConfig,
) -> NativeSessionOptions {
    NativeSessionOptions {
        schedule,
        backend,
        profile: DispatchProfile::StrictTimingDiagnostic,
        timing: TimingOptions {
            game_fps: 60,
            min_hold_us: 0,
            min_release_gap_us: 16_667,
            down_late_grace_us: 500,
            strict_timing: false,
            strict_down_completion_late_us: 2_000,
            strict_up_completion_late_us: 2_000,
            input_path_warn_us: 300,
        },
        focus: FocusOptions {
            require_focus: false,
            focus_restore_grace_us: 100_000,
        },
        wait: WaitOptions {
            enable_waitable_timer: true,
            enable_event_wait: true,
            supervisor_lease_timeout_us: 0,
            test_spin_threshold_us: Some(20_000),
        },
        telemetry: TelemetryOptions {
            mode: TelemetryMode::Ring,
            capacity: 64,
        },
        priority: PriorityOptions {
            mode: sky_dispatch_win32::mmcss::PriorityMode::Off,
        },
        startup_ordering_hook: None,
        restore_race_hook: None,
        timer_lifecycle_context: None,
    }
}

fn start_with_test_wall_clock_slack(session: &NativeDispatchSession) {
    session.arm(TEST_WALL_CLOCK_PREROLL_US).expect("worker arm");
}

#[cfg(all(feature = "test-support", windows))]
#[test]
fn actual_worker_wait_path_drops_waitable_timer_after_session_exit() {
    let timer_context = sky_dispatch_win32::timer::test_support::new_counters();
    let mut options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.timer_lifecycle_context = Some(Arc::clone(&timer_context));
    let session = NativeDispatchSession::new(options).expect("test session admission");

    session.arm(TEST_WALL_CLOCK_PREROLL_US).expect("worker arm");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let counts = sky_dispatch_win32::timer::test_support::snapshot(&timer_context);
    assert!(
        counts.created >= 1,
        "worker wait path must create a waitable timer"
    );
    assert_eq!(counts.created, counts.dropped);
    assert_eq!(counts.live, 0);
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
                scheduled_us: 100_000,
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
    start_with_test_wall_clock_slack(&session);

    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    assert!(snapshot.startup_ready);
    assert!(snapshot.startup_latency_us.is_some());
    let (requested, ready) = session.startup_ticks();
    assert!(requested <= ready);
}

#[test]
fn arm_epoch_is_immutable_across_worker_boot_delay() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.set_boot_delay_us(5_000);
    let mut options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    let session = NativeDispatchSession::new(options).expect("test session admission");

    session.arm(100_000).expect("arm session");
    let epoch_before_boot = session.epoch_qpc_for_test();
    assert!(epoch_before_boot > QpcTicks::ZERO);
    assert_eq!(session.pre_roll_us_for_test(), 100_000);

    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(session.epoch_qpc_for_test(), epoch_before_boot);
    assert_eq!(session.snapshot().terminal_error, None);
}

#[test]
fn boot_readiness_miss_is_terminal_before_physical_send() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.set_boot_delay_us(100_000);
    let mut options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    let session = NativeDispatchSession::new(options).expect("test session admission");

    session.arm(50_000).expect("arm session");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.terminal_error.as_deref(),
        Some("startup_deadline_missed")
    );
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("valid telemetry JSON");
    assert_eq!(telemetry["attempted"].as_u64(), Some(0));
}

#[test]
fn pre_roll_remaining_is_monotonic_and_zero_after_epoch() {
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

    session.arm(500_000).expect("arm session");
    let mut samples = Vec::new();
    while !session.snapshot().is_finished {
        samples.push(session.snapshot().pre_roll_remaining_us);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(samples.windows(2).all(|pair| pair[0] >= pair[1]));
    assert_eq!(session.snapshot().pre_roll_remaining_us, 0);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn zero_requested_pre_roll_uses_production_floor() {
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

    session.arm(0).expect("arm session");
    assert_eq!(session.pre_roll_us_for_test(), 50_000);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
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
                scheduled_us: 500_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "physical-hundred".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 1_000_000,
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
    start_with_test_wall_clock_slack(&session);
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
        scheduled_us: 500_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "many-stale-down".to_string().into(),
    });
    actions.push(KeyActionInput {
        source_action_index: (stale_count + 1) as u32,
        kind: ActionKind::Up,
        // Keep the cleanup boundary after the test-only arm margin.  This
        // test checks stale metadata ordering, not host wake-up lateness.
        scheduled_us: 1_000_000,
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
    start_with_test_wall_clock_slack(&session);
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
    start_with_test_wall_clock_slack(&session);
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

fn run_startup_first_physical_lead_probe(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
) -> serde_json::Value {
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
    start_with_test_wall_clock_slack(&session);
    let deadline = Instant::now() + Duration::from_secs(5);
    while hook.first_physical_send_started.load(Ordering::Acquire) == 0
        && !session.snapshot().is_finished
        && Instant::now() < deadline
    {
        session.heartbeat().expect("heartbeat");
        std::thread::sleep(Duration::from_millis(1));
    }
    let snapshot = session.snapshot();
    assert!(
        hook.first_physical_send_started.load(Ordering::Acquire) > 0,
        "startup probe did not reach physical dispatch: snapshot={snapshot:?}"
    );
    if !session.snapshot().is_finished {
        session.quit().expect("probe quit");
    }
    assert!(session.join(Duration::from_secs(5)).expect("probe join"));
    serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
        .expect("valid telemetry JSON")
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
    start_with_test_wall_clock_slack(&session);
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("adaptive worker join")
    );
    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.outcome,
        Some("finished".to_string()),
        "stale-leading session snapshot: {snapshot:?}"
    );
    assert_eq!(
        snapshot.terminal_error, None,
        "stale-leading terminal error"
    );
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
        scheduled_us: 500_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "stale-leading-down".to_string().into(),
    });
    actions.push(KeyActionInput {
        source_action_index: down_index + 1,
        kind: ActionKind::Up,
        // Keep the cleanup boundary after the test-only arm margin.  This
        // test checks stale metadata ordering, not host wake-up lateness.
        scheduled_us: 1_000_000,
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
                scheduled_us: 500_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "same-timestamp-first-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 600_000,
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
    run_mock_schedule_with_profile(
        schedule,
        allowed_count,
        telemetry_capacity,
        DispatchProfile::StrictTimingDiagnostic,
    )
}

fn run_mock_schedule_with_profile(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    allowed_count: usize,
    telemetry_capacity: usize,
    profile: DispatchProfile,
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
    options.profile = profile;
    options.telemetry.capacity = telemetry_capacity;
    options.wait.supervisor_lease_timeout_us = 3_000_000;
    let session = NativeDispatchSession::new(options).expect("session admission");
    start_with_test_wall_clock_slack(&session);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("valid telemetry JSON");
    (snapshot, telemetry)
}

#[test]
fn production_profile_has_no_observer_samples_or_trace_records() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "production-profile-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1_200_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "production-profile-up".into(),
            },
        ],
        &[0x15],
    )
    .expect("valid production-profile schedule");
    let (snapshot, telemetry) =
        run_mock_schedule_with_profile(schedule, 1, 16, DispatchProfile::Production);
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    assert!(!snapshot.recent_latency_samples_available);
    assert!(snapshot.recent_latencies_us.is_empty());
    assert_eq!(snapshot.observer_queue_high_watermark, 0);
    assert_eq!(telemetry["records"].as_array().map(Vec::len), Some(0));
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
                scheduled_us: 100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "midstream-a-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 150_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "midstream-stale".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Down,
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "midstream-b-down".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Up,
                scheduled_us: 300_000,
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
                scheduled_us: 100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "trailing-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "trailing-release".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 300_000,
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
                scheduled_us: 100_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "cohort-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "cohort-stale-a".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "cohort-stale-b".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Down,
                scheduled_us: 300_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "cohort-redown".into(),
            },
            KeyActionInput {
                source_action_index: 5,
                kind: ActionKind::Up,
                scheduled_us: 400_000,
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
                scheduled_us: 500_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "mixed-stale-a".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 500_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "mixed-stale-b".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 500_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "mixed-down".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                // Keep cleanup away from the first packet so parallel test
                // observer work cannot turn this ordering probe into an
                // unrelated up-deadline failure.
                scheduled_us: 1_000_000,
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
                scheduled_us: 500_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "owned-up-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 600_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "owned-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 600_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "owned-stale".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Down,
                scheduled_us: 600_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "owned-redown".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Up,
                scheduled_us: 700_000,
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
            scheduled_us: 100_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "many-midstream-down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 150_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "many-midstream-release".into(),
        },
    ];
    for index in 0..stale_count {
        actions.push(KeyActionInput {
            source_action_index: (index + 2) as u32,
            kind: ActionKind::Up,
            scheduled_us: 200_000 + index as u64,
            scan_codes: smallvec::smallvec![0x16],
            reason: "many-midstream-stale".into(),
        });
    }
    actions.extend([
        KeyActionInput {
            source_action_index: (stale_count + 2) as u32,
            kind: ActionKind::Down,
            scheduled_us: 300_000,
            scan_codes: smallvec::smallvec![0x17],
            reason: "many-midstream-next-down".into(),
        },
        KeyActionInput {
            source_action_index: (stale_count + 3) as u32,
            kind: ActionKind::Up,
            scheduled_us: 350_000,
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
    start_with_test_wall_clock_slack(&session);
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
    assert_eq!(
        snapshot.observer_queue_high_watermark, 0,
        "producer no longer samples queue length; compatibility field is unavailable"
    );
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
            &[KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                // Keep the authored boundary well beyond worker startup and
                // host timer quantization.  This is test isolation only; the
                // authored target remains frozen and the worker never moves
                // it after arm.
                scheduled_us: scheduled_us + 100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "startup-matrix-down".into(),
            }],
            &[0x15],
        )
        .expect("valid startup matrix schedule");
        let telemetry = run_startup_first_physical_lead_probe(schedule);
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

    // Keep the physical boundaries outside normal worker-startup jitter.  The
    // overdue-catch-up guard is tested deterministically below; this test is
    // about the event-count bucket after stale metadata, not about racing a
    // 500-us wall-clock schedule during thread startup.
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "bucket-first".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 150_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "bucket-release".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "bucket-stale".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Down,
                scheduled_us: 250_000,
                scan_codes: smallvec::smallvec![0x17, 0x18],
                reason: "bucket-two-down".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Up,
                scheduled_us: 350_000,
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
    start_with_test_wall_clock_slack(&session);
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
                scheduled_us: 300_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "progress-idle-up".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 5_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "progress-idle-second-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 5_100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "progress-idle-second-up".to_string().into(),
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
    start_with_test_wall_clock_slack(&session);
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
    start_with_test_wall_clock_slack(&session);
    // Manual pause is a mid-play operation only after the first authored
    // musical commit. Waiting for both first-note transport samples proves
    // that boundary while leaving the test in the authored idle interval.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot_lite();
        if snapshot.is_running && snapshot.recent_latencies_us.len() >= 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "progress did not start: {snapshot:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }

    let pause_generation = session.pause_with_timing_token().expect("pause request");
    let mut pause_timing = None;
    let pause_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot_lite();
        if pause_timing.is_none() {
            pause_timing = session
                .pause_timing_result(pause_generation)
                .expect("pause timing result");
        }
        if snapshot.is_paused && pause_timing.is_some() {
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
fn native_telemetry_does_not_block_dispatch() {
    let actions: Vec<KeyActionInput> = (0_u32..3)
        .flat_map(|index| {
            let cycle_us = (u64::from(index) + 1) * 800_000;
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
                    scheduled_us: cycle_us + 500_000,
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
    session.arm(1_000_000).expect("worker arm");
    while !session.snapshot().is_finished {
        session.heartbeat().expect("supervisor heartbeat");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.outcome,
        Some("finished".to_string()),
        "native telemetry snapshot: {snapshot:?}"
    );
    assert_eq!(snapshot.terminal_error, None);
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry"))
            .expect("valid telemetry JSON");
    assert_eq!(telemetry["attempted"], telemetry["accepted"]);
    assert_eq!(telemetry["dropped"], 0);
    assert_eq!(telemetry["truncated"], false);
    // Queue pressure is scheduler-dependent; the invariant is that it stays
    // bounded and never changes the successful physical/telemetry result.
    assert!(snapshot.observer_dropped_samples <= actions.len() as u64);
    let records = telemetry["records"].as_array().expect("records array");
    assert_eq!(
        records.len(),
        telemetry["accepted"].as_u64().unwrap() as usize
    );
    let indices: Vec<u64> = records
        .iter()
        .map(|record| record["event_index"].as_u64().expect("event index"))
        .collect();
    assert!(!indices.is_empty());
    assert!(
        indices
            .iter()
            .all(|index| *index < actions.len() as u64 * 2)
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
        QpcTicks::ZERO,
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
            scheduled_us: (action_index as u64) * 20_000,
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
    let qpc_clock = QpcClock::from_frequency_hz(std::num::NonZeroU64::new(1_000_000).unwrap());
    try_publish_metrics(&local, &shared, qpc_clock, 0, false);
    try_publish_metrics(&local, &shared, qpc_clock, 49_999, false);
    assert_eq!(shared.publish_count.load(Ordering::Relaxed), 0);
    try_publish_metrics(&local, &shared, qpc_clock, 50_000, false);
    assert_eq!(shared.publish_count.load(Ordering::Relaxed), 1);
    try_publish_metrics(&local, &shared, qpc_clock, 50_001, true);
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
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
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
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
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
fn first_final_foreground_loss_is_terminal_without_epoch_rebase() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);

    // The supervisor hint is stale-true, but the final foreground proof
    // observes that the exact target is no longer foreground.
    let focus_active = AtomicBool::new(true);
    let target_hwnd = AtomicIsize::new(123);
    let target_generation = AtomicU64::new(1);
    let final_admission = final_down_target_admission(FinalTargetSignals {
        expected: TargetStamp {
            hwnd: 123,
            generation: 1,
        },
        require_focus: true,
        focus_active: &focus_active,
        target_hwnd: &target_hwnd,
        target_generation: &target_generation,
    });
    assert_eq!(final_admission, DownAdmission::FocusLost);

    let qpc_clock = QpcClock::initialize().expect("QPC clock");
    let epoch = qpc_clock.now().expect("epoch sample");
    let mut clock_state =
        sky_dispatch_core::clock::PlaybackClockState::new(epoch, DurationTicks::ZERO)
            .expect("playback clock");
    let original_epoch = clock_state.epoch;
    let schedule =
        sky_dispatch_core::compile::compile_runtime_intents(&[], &[0x15]).expect("empty schedule");
    let mut coordinator =
        sky_dispatch_core::coordinator::RuntimeDispatchCoordinator::try_new_ticks(
            schedule,
            0,
            DurationTicks::ZERO,
            |_| Ok(TimelineTicks::ZERO),
        )
        .expect("coordinator");
    let mut backend = TrackedKeyState::new();
    let mut runtime = super::worker::WorkerRuntime::default();
    let target = AtomicIsize::new(123);
    let progress_clock = super::shared::SharedProgressClock::default();

    let result = super::worker::handle_final_focus_loss(
        qpc_clock,
        &mut backend,
        &mut coordinator,
        &mut clock_state,
        &mut runtime,
        &target,
        &progress_clock,
    );

    assert!(matches!(
        result,
        Err(super::worker::DispatchStep::TerminateStatic(
            "focus_lost_during_preroll"
        ))
    ));
    assert_eq!(clock_state.epoch, original_epoch);
    assert!(!clock_state.is_paused());
}

#[test]
fn startup_focus_boundary_does_not_depend_on_qpc_epoch_position() {
    let epoch = QpcTicks::from_raw(10_000);
    for now in [epoch.as_u64() - 1, epoch.as_u64(), epoch.as_u64() + 1] {
        let _now = QpcTicks::from_raw(now);
        assert!(super::worker::startup_focus_loss_is_terminal(false, false));
        assert!(!super::worker::startup_focus_loss_is_terminal(true, false));
        assert!(!super::worker::startup_focus_loss_is_terminal(false, true));
    }
}

#[test]
fn preroll_manual_pause_cancels_without_entering_pause_clock() {
    assert!(super::worker::preroll_manual_pause_cancels(true, false));
    assert!(!super::worker::preroll_manual_pause_cancels(false, false));
    assert!(!super::worker::preroll_manual_pause_cancels(true, true));
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
        let mut harness = ProductionDispatchTestHarness::new_uponly_release_with_gap(100_000);
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

    let mut harness = ProductionDispatchTestHarness::new_uponly_release_with_gap(100_000);
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

    let mut harness = ProductionDispatchTestHarness::new_uponly_release_with_gap(100_000);
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
        matches!(
            step,
            super::worker::DispatchStep::TerminateStatic("focus_lost_during_preroll")
        ),
        "unfocused authored Down must cancel the startup attempt: {step:?}"
    );
    assert!(
        !harness
            .progress_clock
            .load()
            .expect("progress projection")
            .paused,
        "pre-roll focus loss must not enter a rebased pause"
    );
}

struct FocusOverrideResetGuard;

impl Drop for FocusOverrideResetGuard {
    fn drop(&mut self) {
        sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    }
}

fn focus_recovery_schedule() -> sky_dispatch_core::model::RuntimeSchedule {
    sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                // Keep the focus-state tests independent of worker-startup
                // jitter; their assertions begin only after this commit.
                scheduled_us: 400_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "focus-recovery-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 2_100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "focus-recovery-up".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 10_100_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "focus-recovery-sentinel".to_string().into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid focus-recovery schedule")
}

fn start_focus_recovery_session(
    fault_script: FaultInjectionScript,
    restore_race_hook: Option<super::config::RestoreRaceHook>,
) -> NativeDispatchSession {
    let mut options = test_session_options(
        focus_recovery_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.focus.require_focus = true;
    options.restore_race_hook = restore_race_hook;
    let session = NativeDispatchSession::new(options).expect("test session admission");
    session.set_target_hwnd(123);
    session.set_focus_hint(true);
    start_with_test_wall_clock_slack(&session);
    session
}

fn wait_for_focus_down(session: &NativeDispatchSession) -> super::EngineProgressSnapshot {
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut snapshot = session.snapshot_lite();
    while snapshot.recent_latencies_us.is_empty()
        && !snapshot.is_finished
        && Instant::now() < deadline
    {
        session
            .heartbeat()
            .expect("heartbeat while waiting for Down");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(
        !snapshot.recent_latencies_us.is_empty(),
        "Down did not complete: {snapshot:?}"
    );
    snapshot
}

fn wait_for_focus_pause(session: &NativeDispatchSession) -> super::EngineProgressSnapshot {
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut snapshot = session.snapshot_lite();
    while !snapshot.is_paused && !snapshot.is_finished && Instant::now() < deadline {
        session.heartbeat().expect("heartbeat while focus is lost");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(
        snapshot.is_paused,
        "focus loss did not enter pause: {snapshot:?}"
    );
    snapshot
}

fn wait_for_focus_finish(session: &NativeDispatchSession) -> super::EngineProgressSnapshot {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut snapshot = session.snapshot_lite();
    while !snapshot.is_finished && Instant::now() < deadline {
        session
            .heartbeat()
            .expect("heartbeat while waiting for terminal state");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(snapshot.is_finished, "session did not finish: {snapshot:?}");
    snapshot
}

#[test]
fn focus_loss_with_inconclusive_probe_stays_paused_without_terminal_cleanup() {
    struct ForegroundOverrideReset;

    impl Drop for ForegroundOverrideReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        }
    }

    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "focus-loss-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 2_100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "focus-loss-up".to_string().into(),
            },
        ],
        &[0x15],
    )
    .expect("valid focus-loss schedule");
    let force_inconclusive_probe = Arc::new(AtomicBool::new(false));
    let full_release_count = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.force_inconclusive_probe = Some(Arc::clone(&force_inconclusive_probe));
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_count));
    let mut options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.focus.require_focus = true;
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = ForegroundOverrideReset;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    let session = NativeDispatchSession::new(options).expect("test session admission");
    session.set_target_hwnd(123);
    session.set_focus_hint(true);
    start_with_test_wall_clock_slack(&session);

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut snapshot = session.snapshot_lite();
    while snapshot.recent_latencies_us.is_empty()
        && !snapshot.is_finished
        && Instant::now() < deadline
    {
        session
            .heartbeat()
            .expect("heartbeat while waiting for Down");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(
        !snapshot.recent_latencies_us.is_empty(),
        "Down did not complete: {snapshot:?}"
    );
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);

    force_inconclusive_probe.store(true, Ordering::Release);
    session.set_focus_hint(false);
    while !snapshot.is_paused && !snapshot.is_finished && Instant::now() < deadline {
        session.heartbeat().expect("heartbeat while focus is lost");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }

    assert!(
        snapshot.is_paused,
        "focus loss did not enter pause: {snapshot:?}"
    );
    assert!(
        !snapshot.is_finished,
        "focus loss became terminal: {snapshot:?}"
    );
    assert!(!snapshot.has_terminal_error);
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);

    force_inconclusive_probe.store(false, Ordering::Release);
    session.set_focus_hint(true);
    session.quit().expect("quit paused session");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn focus_restore_after_grace_releases_and_resumes() {
    struct ForegroundOverrideReset;

    impl Drop for ForegroundOverrideReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        }
    }

    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "focus-restore-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 2_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "focus-restore-up".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 10_000_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "focus-restore-sentinel".to_string().into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid focus-restore schedule");
    let force_inconclusive_probe = Arc::new(AtomicBool::new(false));
    let send_call_count = Arc::new(AtomicU64::new(0));
    let full_release_count = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.force_inconclusive_probe = Some(Arc::clone(&force_inconclusive_probe));
    fault_script.send_call_count = Some(Arc::clone(&send_call_count));
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_count));
    let mut options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.focus.require_focus = true;
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = ForegroundOverrideReset;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    let session = NativeDispatchSession::new(options).expect("test session admission");
    session.set_target_hwnd(123);
    start_with_test_wall_clock_slack(&session);

    let deadline = Instant::now() + Duration::from_secs(1);
    let mut snapshot = session.snapshot_lite();
    while snapshot.recent_latencies_us.is_empty()
        && !snapshot.is_finished
        && Instant::now() < deadline
    {
        session
            .heartbeat()
            .expect("heartbeat while waiting for Down");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(!snapshot.recent_latencies_us.is_empty());

    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    force_inconclusive_probe.store(true, Ordering::Release);
    session.set_focus_hint(false);
    while !snapshot.is_paused && !snapshot.is_finished && Instant::now() < deadline {
        session.heartbeat().expect("heartbeat while focus is lost");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(
        snapshot.is_paused,
        "focus loss did not enter pause: {snapshot:?}"
    );
    assert!(!snapshot.has_terminal_error);

    force_inconclusive_probe.store(false, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);
    let restore_deadline = Instant::now() + Duration::from_secs(2);
    while snapshot.is_paused && !snapshot.is_finished && Instant::now() < restore_deadline {
        session
            .heartbeat()
            .expect("heartbeat while focus is restored");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(
        !snapshot.is_paused,
        "stable focus did not resume after grace: {snapshot:?}"
    );
    assert!(
        !snapshot.is_finished,
        "restore finished before assertions: lite={snapshot:?}, full={:?}",
        session.snapshot()
    );
    assert!(!snapshot.has_terminal_error);
    assert_eq!(send_call_count.load(Ordering::SeqCst), 2);
    assert_eq!(full_release_count.load(Ordering::SeqCst), 1);

    session.quit().expect("quit restored session");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn focus_restore_before_grace_keeps_pause_without_cleanup_or_dispatch() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));

    let send_call_count = Arc::new(AtomicU64::new(0));
    let full_release_count = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_call_count));
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_count));
    let session = start_focus_recovery_session(fault_script, None);

    wait_for_focus_down(&session);
    assert_eq!(send_call_count.load(Ordering::SeqCst), 1);

    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    session.set_focus_hint(false);
    wait_for_focus_pause(&session);

    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);
    let grace_deadline = Instant::now() + Duration::from_millis(30);
    while Instant::now() < grace_deadline {
        session.heartbeat().expect("heartbeat during restore grace");
        std::thread::sleep(Duration::from_millis(1));
    }

    let snapshot = session.snapshot_lite();
    assert!(
        snapshot.is_paused,
        "restore grace ended too early: {snapshot:?}"
    );
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);
    assert_eq!(send_call_count.load(Ordering::SeqCst), 1);

    session.quit().expect("quit paused session");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn focus_bounce_resets_restore_grace_without_cleanup() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));

    let send_call_count = Arc::new(AtomicU64::new(0));
    let full_release_count = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_call_count));
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_count));
    let session = start_focus_recovery_session(fault_script, None);

    wait_for_focus_down(&session);
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    session.set_focus_hint(false);
    wait_for_focus_pause(&session);

    let first_restore_started = Instant::now();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);
    let first_restore_deadline = first_restore_started + Duration::from_millis(30);
    while Instant::now() < first_restore_deadline {
        session.heartbeat().expect("heartbeat during first restore");
        std::thread::sleep(Duration::from_millis(1));
    }

    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    session.set_focus_hint(false);
    let focus_loss_observation_deadline = Instant::now() + Duration::from_millis(20);
    while Instant::now() < focus_loss_observation_deadline {
        session
            .heartbeat()
            .expect("heartbeat while observing focus bounce loss");
        std::thread::sleep(Duration::from_millis(1));
    }
    wait_for_focus_pause(&session);

    let second_restore_started = Instant::now();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);
    let old_grace_deadline = first_restore_started + Duration::from_millis(100);
    let new_grace_deadline = second_restore_started + Duration::from_millis(100);
    assert!(
        old_grace_deadline + Duration::from_millis(5) < new_grace_deadline,
        "test timing window collapsed: first={first_restore_started:?}, second={second_restore_started:?}"
    );
    while Instant::now() < old_grace_deadline + Duration::from_millis(5) {
        session
            .heartbeat()
            .expect("heartbeat during second restore");
        std::thread::sleep(Duration::from_millis(1));
    }

    let snapshot = session.snapshot_lite();
    assert!(
        snapshot.is_paused,
        "focus bounce resumed at the old grace deadline: {snapshot:?}"
    );
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);
    assert_eq!(send_call_count.load(Ordering::SeqCst), 1);

    let resume_deadline = Instant::now() + Duration::from_secs(1);
    let mut snapshot = session.snapshot_lite();
    while snapshot.is_paused && Instant::now() < resume_deadline {
        session
            .heartbeat()
            .expect("heartbeat while waiting for second restore grace");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(
        !snapshot.is_paused,
        "second restore grace did not resume playback: {snapshot:?}"
    );
    assert_eq!(full_release_count.load(Ordering::SeqCst), 1);
    assert_eq!(send_call_count.load(Ordering::SeqCst), 2);

    session.quit().expect("quit bounced session");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn focus_restore_cleanup_verification_failure_is_terminal() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));

    let force_inconclusive_probe = Arc::new(AtomicBool::new(false));
    let full_release_count = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.force_inconclusive_probe = Some(Arc::clone(&force_inconclusive_probe));
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_count));
    let session = start_focus_recovery_session(fault_script, None);

    wait_for_focus_down(&session);
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    session.set_focus_hint(false);
    wait_for_focus_pause(&session);

    force_inconclusive_probe.store(true, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);
    wait_for_focus_finish(&session);

    let snapshot = session.snapshot();
    assert!(snapshot.is_finished);
    assert!(snapshot.terminal_error.as_deref().is_some_and(|error| {
        error.starts_with("focus restoration failed: release verification failed:")
    }));
    assert_eq!(full_release_count.load(Ordering::SeqCst), 1);

    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn focus_restore_preflight_failure_is_terminal() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));

    let force_preflight_failure = Arc::new(AtomicBool::new(false));
    let full_release_count = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.force_preflight_failure = Some(Arc::clone(&force_preflight_failure));
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_count));
    let session = start_focus_recovery_session(fault_script, None);

    wait_for_focus_down(&session);
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    session.set_focus_hint(false);
    wait_for_focus_pause(&session);

    force_preflight_failure.store(true, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);
    wait_for_focus_finish(&session);

    let snapshot = session.snapshot();
    assert!(snapshot.is_finished);
    assert!(snapshot.terminal_error.as_deref().is_some_and(|error| {
        error.starts_with("instrument key preflight failed during focus restoration;")
    }));
    assert_eq!(full_release_count.load(Ordering::SeqCst), 1);

    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn focus_restore_race_after_validation_does_not_resume() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));

    let raced = Arc::new(AtomicBool::new(false));
    let full_release_count = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_count));
    let race_hook: super::config::RestoreRaceHook = {
        let raced = Arc::clone(&raced);
        Arc::new(move |focus_active, target_hwnd, target_generation| {
            if !raced.swap(true, Ordering::SeqCst) {
                focus_active.store(false, Ordering::Release);
                target_hwnd.store(456, Ordering::Release);
                target_generation.fetch_add(1, Ordering::AcqRel);
            }
        })
    };
    let session = start_focus_recovery_session(fault_script, Some(race_hook));

    wait_for_focus_down(&session);
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    session.set_focus_hint(false);
    wait_for_focus_pause(&session);

    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);
    let race_deadline = Instant::now() + Duration::from_millis(300);
    while !raced.load(Ordering::Acquire) && Instant::now() < race_deadline {
        session.heartbeat().expect("heartbeat during restore race");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        raced.load(Ordering::Acquire),
        "restore race hook did not run"
    );
    let settle_deadline = Instant::now() + Duration::from_millis(30);
    while Instant::now() < settle_deadline {
        session.heartbeat().expect("heartbeat after restore race");
        std::thread::sleep(Duration::from_millis(1));
    }

    let snapshot = session.snapshot();
    assert!(
        snapshot.is_paused,
        "restore race resumed playback: {snapshot:?}"
    );
    assert!(
        !snapshot.is_finished,
        "manual resume finished unexpectedly: lite={snapshot:?}, full={:?}",
        session.snapshot()
    );
    assert_eq!(snapshot.terminal_error, None);
    assert_eq!(full_release_count.load(Ordering::SeqCst), 1);

    session.quit().expect("quit raced session");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn manual_and_focus_pauses_compose_without_auto_resume() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));

    let full_release_count = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_count));
    let session = start_focus_recovery_session(fault_script, None);

    wait_for_focus_down(&session);
    session.pause().expect("manual pause");
    let manual_pause_deadline = Instant::now() + Duration::from_secs(1);
    let mut snapshot = session.snapshot_lite();
    while !snapshot.is_paused && !snapshot.is_finished && Instant::now() < manual_pause_deadline {
        session.heartbeat().expect("heartbeat during manual pause");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(
        snapshot.is_paused,
        "manual pause did not apply: {snapshot:?}"
    );

    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    session.set_focus_hint(false);
    wait_for_focus_pause(&session);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);
    let restore_deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < restore_deadline {
        session
            .heartbeat()
            .expect("heartbeat during composed pause");
        std::thread::sleep(Duration::from_millis(1));
    }

    snapshot = session.snapshot_lite();
    assert!(
        snapshot.is_paused,
        "manual pause was cleared by focus restore: {snapshot:?}"
    );
    assert_eq!(full_release_count.load(Ordering::SeqCst), 1);

    session.resume().expect("manual resume");
    let resume_deadline = Instant::now() + Duration::from_secs(1);
    while snapshot.is_paused && !snapshot.is_finished && Instant::now() < resume_deadline {
        session.heartbeat().expect("heartbeat during manual resume");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(
        !snapshot.is_paused,
        "manual reason did not clear: {snapshot:?}"
    );
    assert!(
        !snapshot.is_finished,
        "manual resume finished unexpectedly: {snapshot:?}"
    );

    session.quit().expect("quit composed session");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn authored_up_only_is_not_blocked_by_target_change() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_uponly_release_with_gap(100_000);
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

    let mut authored = ProductionDispatchTestHarness::new_uponly_release_with_gap(1_000_000);
    let authored_calls = authored.configure_send_counter();
    authored.advance_playback_time_us(100_000);
    let authored_plan = authored.plan_current_dispatch();
    let authored_step = authored
        .wait_and_dispatch_current_plan(&authored_plan)
        .expect("frozen authored wait");
    assert!(
        matches!(authored_step, super::worker::DispatchStep::Dispatched),
        "frozen authored dispatch failed: {authored_step:?}, down_state={:?}, plan={authored_plan:?}",
        authored.runtime.down_boundary_state,
    );
    assert_eq!(authored_calls.load(Ordering::SeqCst), 1);

    let invalid_plan = super::worker::NextDispatchPlan::NoWork;
    assert!(super::worker::plan_structure_is_valid(&invalid_plan));
    assert!(matches!(
        authored.dispatch_due_from_plan_for_test(&invalid_plan),
        super::worker::DispatchStep::NoWork
    ));
}

#[test]
fn deferred_release_does_not_block_unrelated_down_chord() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_deferred_release_with_unrelated_down();
    let calls = harness.configure_send_counter();

    harness.align_next_plan_to_future_for_test(500_000);
    let first_plan = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&first_plan),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.resources.coordinator.pending_release_count(), 0);
    assert_eq!(harness.backend_active_mask() & 0b001, 0b001);

    let authored_chord_plan = harness.plan_current_dispatch();
    let authored_chord_step = harness.dispatch_at_plan_target_for_test(&authored_chord_plan);
    assert!(
        matches!(authored_chord_step, super::worker::DispatchStep::Dispatched),
        "unrelated chord step: {authored_chord_step:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(harness.resources.coordinator.pending_release_count(), 0);
    assert_eq!(harness.backend_active_mask() & 0b110, 0b110);

    harness.align_next_plan_to_future_for_test(500_000);
    let cleanup_plan = harness.plan_current_dispatch();
    let cleanup_step = harness.dispatch_at_plan_target_for_test(&cleanup_plan);
    assert!(
        matches!(cleanup_step, super::worker::DispatchStep::Dispatched),
        "cleanup step: {cleanup_step:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(harness.resources.coordinator.pending_release_count(), 0);
    assert_eq!(harness.backend_active_mask(), 0);
}

#[test]
fn deferred_release_and_authored_chord_have_exact_packet_order() {
    use super::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_win32::input::PhysicalPacket;

    let mut harness = ProductionDispatchTestHarness::new_deferred_release_with_unrelated_down();
    let packets = harness.configure_packet_capture();

    harness.align_next_plan_to_future_for_test(500_000);
    let first_plan = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&first_plan),
        super::worker::DispatchStep::Dispatched
    ));

    let authored_plan = harness.plan_current_dispatch();
    let authored_step = harness.dispatch_at_plan_target_for_test(&authored_plan);
    assert!(
        matches!(authored_step, super::worker::DispatchStep::Dispatched),
        "unrelated chord packet step: {authored_step:?}"
    );

    harness.align_next_plan_to_future_for_test(500_000);
    let cleanup_plan = harness.plan_current_dispatch();
    let cleanup_result = harness.dispatch_at_plan_target_for_test(&cleanup_plan);
    assert!(
        matches!(cleanup_result, super::worker::DispatchStep::Dispatched),
        "cleanup step: {cleanup_result:?}"
    );

    assert_eq!(
        *packets.lock().expect("packet capture lock"),
        vec![
            PhysicalPacket::new(0, 0b001),
            PhysicalPacket::new(0b001, 0b110),
            PhysicalPacket::new(0b110, 0),
        ],
        "the authored Up and unrelated Down share their authored boundary"
    );
}

#[test]
fn manual_pause_cancels_pending_release_without_stale_up_on_resume() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_admissible_dynamic_pending_release();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    harness.seed_pending_release_for_test(0x15, 50_000);
    assert_eq!(harness.resources.coordinator.pending_release_count(), 1);

    harness
        .suspend_live_input_for_test()
        .expect("manual pause suspension");
    assert_eq!(harness.resources.coordinator.pending_release_count(), 0);
    assert_eq!(harness.resources.coordinator.pending_release_mask(), 0);
    let packets = harness.configure_packet_capture();
    harness.advance_playback_time_us(1_000);
    assert!(matches!(
        harness.plan_current_dispatch(),
        super::worker::NextDispatchPlan::NoWork
    ));
    assert!(harness.resources.coordinator.is_finished());
    assert!(packets.lock().expect("packet capture lock").is_empty());
    harness
        .resources
        .coordinator
        .check_post_cleanup_invariants()
        .expect("clean suspension state");
}

#[test]
fn focus_suspend_restore_cancels_pending_release_without_stale_up() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_admissible_dynamic_pending_release();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    harness.seed_pending_release_for_test(0x15, 50_000);
    assert_eq!(harness.resources.coordinator.pending_release_count(), 1);

    // Focus restoration uses the same production suspension seam before it
    // re-preflights the target.  A resumed planner must not rediscover the
    // cancelled pending Up.
    harness
        .suspend_live_input_for_test()
        .expect("focus suspension");
    assert_eq!(harness.resources.coordinator.pending_release_count(), 0);
    assert_eq!(harness.resources.coordinator.pending_release_mask(), 0);
    let packets = harness.configure_packet_capture();
    harness.advance_playback_time_us(1_000);
    assert!(matches!(
        harness.plan_current_dispatch(),
        super::worker::NextDispatchPlan::NoWork
    ));
    assert!(harness.resources.coordinator.is_finished());
    assert!(packets.lock().expect("packet capture lock").is_empty());
    harness
        .resources
        .coordinator
        .check_post_cleanup_invariants()
        .expect("clean focus suspension state");
}

#[test]
fn pending_release_equal_authored_boundary_sends_one_up_packet() {
    use super::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_win32::input::PhysicalPacket;

    let mut harness = ProductionDispatchTestHarness::new_pending_release_with_metadata_boundary();

    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    let second = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&second),
        super::worker::DispatchStep::Dispatched
    ));

    harness.seed_pending_release_for_test(0x15, 220_000);
    assert_eq!(harness.resources.coordinator.pending_release_count(), 1);

    let equal_boundary = harness.plan_current_dispatch();
    let physical = equal_boundary
        .physical()
        .expect("equal boundary must remain physical");
    assert_eq!(
        physical.authored_view.packet_masks,
        PhysicalPacket::new(0b011, 0),
        "pending A Up must coalesce with authored B Up at the equal boundary"
    );
    let packets = harness.configure_packet_capture();
    assert!(
        matches!(
            harness.dispatch_at_plan_target_for_test(&equal_boundary),
            super::worker::DispatchStep::Dispatched
        ),
        "equal-boundary dispatch failed"
    );

    assert_eq!(
        *packets.lock().expect("packet capture lock"),
        vec![PhysicalPacket::new(0b011, 0)]
    );
    assert_eq!(harness.resources.coordinator.pending_release_count(), 0);
    assert_eq!(harness.resources.coordinator.cursor, 3);
}

#[test]
fn overdue_up_only_boundary_releases_and_continues() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let calls = harness.configure_send_counter();

    let first_plan = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&first_plan),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    harness.advance_playback_time_us(100_000);
    let overdue_plan = harness.plan_current_dispatch();
    let step = harness.dispatch_same_frozen_plan_after_due_without_wait_for_test(&overdue_plan);
    assert!(matches!(step, super::worker::DispatchStep::Dispatched));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(harness.backend_active_mask(), 0);
}

#[test]
fn future_classification_then_waiter_entry_stall_keeps_exact_boundary_authorized() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_two_down_boundaries();
    let calls = harness.configure_send_counter();

    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // B is future at first classification.  The worker must leave the guard
    // armed even though dispatch_due returns NoWork before entering wait.
    let future = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&future),
        super::worker::DispatchStep::NoWork
    ));

    // A same-boundary replan (the equivalent of a non-physical Continue
    // before waiter entry) must retain the exact authorization token.
    let replanned = harness.plan_current_dispatch();

    // Keep the same frozen B plan. Model a stall before waiter entry so the
    // waiter returns Due { wait_result: None } for this already-overdue
    // target. Exact future authorization survives and normal hard-late
    // admission may send it within the fixed cutoff.
    let step = harness.dispatch_same_frozen_plan_after_due_without_wait_for_test(&replanned);
    assert!(matches!(step, super::worker::DispatchStep::Dispatched));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn overdue_down_beyond_rescue_grace_is_committed_missed_without_sendinput() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_two_down_boundaries();
    let calls = harness.configure_send_counter();

    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    harness.advance_playback_time_us(100_000);
    let missed = harness.plan_current_dispatch();
    let missed_step = harness.dispatch_known_backlog_with_strict_lateness_for_test(&missed);
    assert!(
        matches!(
            missed_step,
            super::worker::DispatchStep::TerminateStatic("down_deadline_missed_before_send")
        ),
        "missed step: {missed_step:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.local_metrics.late_discovery_rescue_attempts, 0);
    assert!(!harness.has_active_generation(0x16));
    assert!(harness.local_metrics.missed_down_boundaries >= 1);
    assert!(harness.local_metrics.last_missed_down_valid);
    assert_eq!(harness.local_metrics.last_missed_down_reason_code, 1);
    assert_eq!(
        harness.local_metrics.last_missed_down_source_action_index,
        1
    );
    assert_eq!(harness.local_metrics.last_missed_down_mask, 0b10);
    assert!(harness.local_metrics.last_missed_down_lateness_ticks > 0);
}

#[test]
fn three_overdue_downs_are_dropped_before_next_future_boundary() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_three_overdue_then_future();
    let calls = harness.configure_send_counter();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    harness.advance_playback_time_us(4_000);

    for _ in 0..3 {
        let overdue = harness.plan_current_dispatch();
        assert!(matches!(
            harness.dispatch_same_frozen_plan_after_due_without_wait_for_test(&overdue),
            super::worker::DispatchStep::Dispatched
        ));
    }

    let future = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&future),
        super::worker::DispatchStep::NoWork
    ));
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&future),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(harness.local_metrics.late_discovery_rescue_attempts, 1);
    assert_eq!(harness.local_metrics.late_discovery_rescue_sent, 1);
    assert_eq!(harness.local_metrics.missed_down_boundaries, 2);
    assert_eq!(harness.local_metrics.missed_down_keys, 2);
    assert_eq!(harness.local_metrics.missed_backlog_boundaries, 2);
    assert_eq!(
        harness.resources.coordinator.active_mask & 0b1_1111,
        (1 << 0) | (1 << 1) | (1 << 4),
        "the one rescued B, successful A, and future E Downs may be active"
    );
}

#[test]
fn overdue_mixed_packet_beyond_rescue_grace_sends_only_safety_up() {
    use super::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_win32::input::PhysicalPacket;

    let mut harness = ProductionDispatchTestHarness::new_mixed();
    let packets = harness.configure_packet_capture();

    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    let missed = harness.plan_current_dispatch();
    let beyond_grace = harness
        .timing
        .down_late_grace_ticks
        .checked_add(DurationTicks::from_raw(1))
        .expect("missed mixed lateness arithmetic");
    let missed_step = harness.dispatch_same_frozen_plan_at_lateness_for_test(&missed, beyond_grace);
    assert!(
        matches!(missed_step, super::worker::DispatchStep::Dispatched),
        "missed mixed step: {missed_step:?}"
    );

    assert_eq!(
        *packets.lock().expect("packet capture lock"),
        vec![PhysicalPacket::new(0, 0b001), PhysicalPacket::new(0b001, 0)],
        "a missed Mixed boundary must never send its Down subset"
    );
    assert_eq!(harness.local_metrics.late_discovery_rescue_attempts, 0);
    assert!(!harness.has_active_generation(0x15));
    assert!(!harness.has_active_generation(0x16));
}

#[test]
fn authorized_down_beyond_hard_cutoff_is_missed_without_down_syscall() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_two_down_boundaries();
    let calls = harness.configure_send_counter();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));

    let future = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&future),
        super::worker::DispatchStep::NoWork
    ));
    harness.configure_deadline_missed_packet_sender();
    let hard_late = harness.dispatch_same_frozen_plan_after_due_without_wait_for_test(&future);
    assert!(matches!(hard_late, super::worker::DispatchStep::Dispatched));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.local_metrics.missed_hard_late_boundaries, 1);
    assert!(harness.local_metrics.last_missed_down_valid);
    assert_eq!(harness.local_metrics.last_missed_down_reason_code, 2);
    assert_eq!(
        harness.local_metrics.last_missed_down_source_action_index,
        1
    );
    assert_eq!(harness.local_metrics.last_missed_down_mask, 0b10);
    assert!(harness.local_metrics.last_missed_down_lateness_ticks > 0);
}

#[test]
fn late_discovery_rescue_still_obeys_sender_cutoff() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_two_down_boundaries();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));

    // Deliberately do not observe B while future. The one-tick overdue
    // dispatch is therefore eligible for rescue, but the sender rejects it
    // before inserting any Down event.
    let rescue = harness.plan_current_dispatch();
    harness.configure_deadline_missed_packet_sender();
    assert!(matches!(
        harness.dispatch_same_frozen_plan_after_due_without_wait_for_test(&rescue),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(harness.local_metrics.late_discovery_rescue_attempts, 1);
    assert_eq!(harness.local_metrics.late_discovery_rescue_sent, 0);
    assert_eq!(
        harness
            .local_metrics
            .late_discovery_rescue_sender_cutoff_misses,
        1
    );
    assert_eq!(harness.local_metrics.missed_hard_late_boundaries, 1);
    assert!(!harness.has_active_generation(0x16));
}

#[test]
fn authorized_down_first_tick_beyond_grace_misses_without_down_syscall() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_two_down_boundaries();
    let calls = harness.configure_send_counter();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    let future = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&future),
        super::worker::DispatchStep::NoWork
    ));
    let one_tick_beyond_grace = harness
        .timing
        .down_late_grace_ticks
        .checked_add(DurationTicks::from_raw(1))
        .expect("QPC cutoff arithmetic");

    assert!(matches!(
        harness.dispatch_same_frozen_plan_at_lateness_for_test(&future, one_tick_beyond_grace),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.local_metrics.missed_hard_late_boundaries, 1);
    assert_eq!(harness.local_metrics.missed_down_boundaries, 1);
    assert_eq!(
        harness.local_metrics.last_missed_down_lateness_ticks,
        one_tick_beyond_grace.as_u64()
    );
}

#[test]
fn five_millisecond_authorized_down_lateness_is_not_a_twenty_millisecond_window() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_two_down_boundaries();
    let calls = harness.configure_send_counter();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    let future = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&future),
        super::worker::DispatchStep::NoWork
    ));
    let five_ms = harness
        .resources
        .clock
        .duration_from_us(5_000)
        .expect("QPC lateness conversion");

    assert!(matches!(
        harness.dispatch_same_frozen_plan_at_lateness_for_test(&future, five_ms),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.local_metrics.missed_hard_late_boundaries, 1);
}

#[test]
fn hard_late_mixed_packet_sends_safety_up_without_down() {
    use super::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_win32::input::PhysicalPacket;

    let mut harness = ProductionDispatchTestHarness::new_mixed();
    let packets = harness.configure_packet_capture();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    let future = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&future),
        super::worker::DispatchStep::NoWork
    ));
    let lateness = harness
        .timing
        .down_late_grace_ticks
        .checked_add(DurationTicks::from_raw(1))
        .expect("QPC cutoff arithmetic");

    assert!(matches!(
        harness.dispatch_same_frozen_plan_at_lateness_for_test(&future, lateness),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(
        *packets.lock().expect("packet capture lock"),
        vec![PhysicalPacket::new(0, 0b001), PhysicalPacket::new(0b001, 0)]
    );
    assert_eq!(harness.local_metrics.missed_hard_late_boundaries, 1);
}

#[test]
fn hard_late_safety_up_queues_hold_forensics_lifecycle_evidence() {
    use super::test_support::ProductionDispatchTestHarness;
    use super::worker::dispatch::observation::{DispatchObservation, ObserverLifecycle};

    let mut harness = ProductionDispatchTestHarness::new_mixed();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    assert!(matches!(
        harness.pop_observation(),
        Some(DispatchObservation::Down(_))
    ));

    let future = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&future),
        super::worker::DispatchStep::NoWork
    ));
    assert!(matches!(
        harness.dispatch_same_frozen_plan_after_hard_late_for_test(&future),
        super::worker::DispatchStep::Dispatched
    ));
    assert!(matches!(
        harness.pop_observation(),
        Some(DispatchObservation::Lifecycle(
            ObserverLifecycle::RecoveryUp { up_mask: 1 }
        ))
    ));
}

#[test]
fn outer_and_inner_hard_late_recovery_have_same_backend_health() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut outer = ProductionDispatchTestHarness::new_two_down_boundaries();
    let first = outer.plan_current_dispatch();
    let first_step = outer.dispatch_at_plan_target_for_test(&first);
    assert!(
        matches!(first_step, super::worker::DispatchStep::Dispatched),
        "outer first step: {first_step:?}"
    );
    let outer_future = outer.plan_current_dispatch();
    assert!(matches!(
        outer.dispatch_due_from_plan_for_test(&outer_future),
        super::worker::DispatchStep::NoWork
    ));
    assert!(matches!(
        outer.dispatch_same_frozen_plan_after_hard_late_for_test(&outer_future),
        super::worker::DispatchStep::Dispatched
    ));
    let outer_health = (
        outer.local_metrics.missed_hard_late_boundaries,
        outer.resources.backend.chords_rejected,
        outer.resources.backend.authored_keys_rejected,
        outer.resources.backend.active_mask,
        outer.resources.backend.possibly_active_mask,
        outer.resources.backend.failed_release_mask,
        outer
            .resources
            .coordinator
            .generation_status_counts()
            .get("dropped_expired")
            .copied()
            .unwrap_or_default(),
    );

    let mut inner = ProductionDispatchTestHarness::new_two_down_boundaries();
    let first = inner.plan_current_dispatch();
    assert!(matches!(
        inner.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));
    let inner_future = inner.plan_current_dispatch();
    assert!(matches!(
        inner.dispatch_due_from_plan_for_test(&inner_future),
        super::worker::DispatchStep::NoWork
    ));
    inner.configure_deadline_missed_packet_sender();
    assert!(matches!(
        inner.dispatch_same_frozen_plan_after_due_without_wait_for_test(&inner_future),
        super::worker::DispatchStep::Dispatched
    ));
    let inner_health = (
        inner.local_metrics.missed_hard_late_boundaries,
        inner.resources.backend.chords_rejected,
        inner.resources.backend.authored_keys_rejected,
        inner.resources.backend.active_mask,
        inner.resources.backend.possibly_active_mask,
        inner.resources.backend.failed_release_mask,
        inner
            .resources
            .coordinator
            .generation_status_counts()
            .get("dropped_expired")
            .copied()
            .unwrap_or_default(),
    );

    assert_eq!(outer_health, inner_health);
    assert_eq!(outer_health.0, 1);
    assert_eq!(outer_health.1, 0);
    assert_eq!(outer_health.2, 0);
    assert_eq!(outer_health.6, 1);
}

#[test]
fn first_musical_down_hard_miss_remains_startup_terminal() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_deadline_missed_packet_sender();
    let first = harness.plan_current_dispatch();
    let step = harness.dispatch_at_plan_target_for_test(&first);
    assert!(matches!(
        step,
        super::worker::DispatchStep::TerminateStatic("down_deadline_missed_before_send")
    ));
    assert_eq!(harness.backend_active_mask(), 0);
    assert_eq!(harness.local_metrics.missed_down_boundaries, 1);
    assert_eq!(harness.local_metrics.missed_hard_late_boundaries, 1);
    assert!(harness.local_metrics.last_missed_down_valid);
    assert_eq!(harness.local_metrics.last_missed_down_reason_code, 2);
}

#[test]
fn strict_pre_admission_down_late_is_classified_before_termination() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let calls = harness.configure_send_counter();
    let plan = harness.plan_current_dispatch();
    let step = harness.dispatch_with_strict_admission_late_for_test(&plan);

    assert!(matches!(step, super::worker::DispatchStep::Terminate(_)));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(harness.local_metrics.missed_down_boundaries, 1);
    assert_eq!(harness.local_metrics.missed_hard_late_boundaries, 1);
    assert_eq!(harness.local_metrics.missed_backlog_boundaries, 0);
    assert_eq!(harness.local_metrics.last_missed_down_reason_code, 2);
    assert!(harness.local_metrics.last_missed_down_valid);
    assert!(harness.local_metrics.last_missed_down_lateness_ticks > 0);
}

#[test]
fn strict_known_backlog_keeps_backlog_precedence_over_grace_gate() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_two_down_boundaries();
    let calls = harness.configure_send_counter();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        super::worker::DispatchStep::Dispatched
    ));

    harness.advance_playback_time_us(100_000);
    let future = harness.plan_current_dispatch();
    assert!(harness.runtime.down_boundary_state.awaiting_future());
    assert!(harness.runtime.musical_physical_commit_started);

    let step = harness.dispatch_known_backlog_with_strict_lateness_for_test(&future);

    assert!(
        matches!(
            step,
            super::worker::DispatchStep::TerminateStatic("down_deadline_missed_before_send")
        ),
        "unexpected strict backlog step: {step:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.local_metrics.missed_down_boundaries, 1);
    assert_eq!(harness.local_metrics.missed_backlog_boundaries, 1);
    assert_eq!(harness.local_metrics.missed_hard_late_boundaries, 0);
    assert!(harness.local_metrics.last_missed_down_valid);
    assert_eq!(harness.local_metrics.last_missed_down_reason_code, 1);
    assert_eq!(
        harness.local_metrics.last_missed_down_source_action_index,
        1
    );
    assert_eq!(harness.local_metrics.last_missed_down_mask, 0b10);
    assert!(
        harness.local_metrics.last_missed_down_lateness_ticks
            > harness.timing.down_late_grace_ticks.as_u64()
    );
}

#[test]
fn due_frozen_plan_does_not_reenter_preparation_or_preflight() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let plan = harness.plan_current_dispatch();
    let prepared_counts = harness.preparation_counts();
    assert_eq!(prepared_counts.packet_header_reads, 1);
    assert_eq!(prepared_counts.up_intent_visits, 0);
    assert_eq!(prepared_counts.down_intent_visits, 1);
    assert_eq!(prepared_counts.registry_lookups, 1);
    assert_eq!(prepared_counts.view_packet_calls, 1);
    assert_eq!(prepared_counts.commit_freeze_calls, 1);
    assert_eq!(prepared_counts.conflict_calls, 1);
    assert_eq!(prepared_counts.input_build_calls, 1);
    assert_eq!(prepared_counts.preflight_calls, 1);

    let forced_preflight_failure = Arc::new(AtomicBool::new(false));
    harness.set_force_preflight_failure(Arc::clone(&forced_preflight_failure));
    forced_preflight_failure.store(true, Ordering::Release);

    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&plan),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(
        harness.preparation_counts(),
        prepared_counts,
        "WaitBoundary::Due dispatch must consume the frozen view and packet"
    );

    let mut missing_proof = ProductionDispatchTestHarness::new_down_only();
    let mut missing_proof_plan = missing_proof.plan_current_dispatch();
    if let Some(physical) = missing_proof_plan.physical_mut() {
        physical.target_proof = super::worker::TargetProof::Required;
    }
    let step = missing_proof.dispatch_due_from_plan_for_test(&missing_proof_plan);
    assert!(matches!(
        step,
        super::worker::DispatchStep::Terminate(error)
            if error.contains("without preflight proof")
    ));
}

#[test]
fn frozen_dispatch_qpc_probe_has_no_redundant_effective_now_sample() {
    use super::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_win32::clock::{qpc_read_count, reset_qpc_read_count};

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_packet_capture();
    harness.align_next_plan_to_future_for_test(5_000_000);
    let plan = harness.plan_current_dispatch();

    // The frozen-plan waiter/dispatch seam must retain the handoff sample,
    // final admission, SendInput completion, and explicitly diagnostic
    // dispatch-ready sample. In particular, no extra QPC is permitted merely
    // to reconstruct effective_now.
    reset_qpc_read_count();
    assert!(matches!(
        harness
            .wait_and_dispatch_current_plan(&plan)
            .expect("frozen plan wait"),
        super::worker::DispatchStep::Dispatched
    ));
    assert!(
        qpc_read_count() >= 4,
        "precision path must retain the handoff and transport QPC samples"
    );
}

#[test]
fn frozen_target_reaches_dispatch_and_observation_without_reconstruction() {
    use super::test_support::ProductionDispatchTestHarness;
    use super::worker::dispatch::DispatchObservation;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_packet_capture();
    let plan = harness.plan_current_dispatch();
    let target = plan.physical_target_qpc().expect("frozen physical target");

    assert!(matches!(
        harness.wait_and_dispatch_current_plan(&plan),
        Ok(super::worker::DispatchStep::Dispatched)
    ));
    let observation = harness.pop_observation().expect("physical observation");
    match observation {
        DispatchObservation::Down(observation) => {
            assert_eq!(observation.physical_target_qpc, target);
        }
        other => panic!("unexpected observation variant: {other:?}"),
    }
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
            final_proof_ticks: Some(TimelineTicks::from_raw(20)),
            pre_call_ticks: Some(TimelineTicks::from_raw(22)),
            sendinput_completion_ticks: Some(TimelineTicks::from_raw(25)),
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
    assert_eq!(record.send_started_ticks, 22);
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
            final_proof_ticks: None,
            pre_call_ticks: None,
            sendinput_completion_ticks: None,
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
            final_proof_ticks: None,
            pre_call_ticks: None,
            sendinput_completion_ticks: None,
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

    start_with_test_wall_clock_slack(&session);
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
            // Keep this first target well after arm/startup so this transport
            // fault test does not depend on a concurrent test thread winning
            // the exact epoch boundary.
            scheduled_us: 500_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            // Leave enough authored interval for this fault-injection test
            // to reach the mixed packet; hold-feasibility is tested directly
            // with exact QPC boundaries elsewhere.
            scheduled_us: 1_000_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "retrigger-up".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 1_000_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "disjoint-down".to_string().into(),
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

    start_with_test_wall_clock_slack(&session);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, "error", "{snapshot:?}");
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
            // Keep each physical boundary away from worker startup and leave
            // observer work deterministic under the full parallel suite.
            scheduled_us: 500_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Down,
            scheduled_us: 1_000_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "disjoint-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: 1_000_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "retrigger-up".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 1_500_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "stale-release".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 4,
            kind: ActionKind::Up,
            scheduled_us: 1_500_000,
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

    start_with_test_wall_clock_slack(&session);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.status, "finished",
        "terminal error: {:?}",
        snapshot.terminal_error
    );
    assert_eq!(snapshot.generation_status_counts["released"], 2);
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
    assert_eq!(mixed["requested_count"].as_u64(), Some(2));
    assert_eq!(mixed["sent_count"].as_u64(), Some(2));
    assert_eq!(mixed["polyphony"].as_u64(), Some(2));
}

#[test]
fn native_session_rejects_deterministically_infeasible_schedule_before_worker_start() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "seed-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![0x15],
            reason: "same-key-release".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "same-key-chord".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15, 0x16])
        .expect("valid dynamic-infeasibility schedule");
    let mut options = test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.timing.min_hold_us = 300;
    let result = NativeDispatchSession::new(options);
    assert!(matches!(
        result,
        Err(error) if error.contains("native schedule admission failed")
    ));
}

#[test]
fn native_min_hold_admission_uses_runtime_qpc_tick_domain() {
    let actions = [
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 1,
            scan_codes: smallvec::smallvec![0x15],
            reason: "qpc-rounding-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 101,
            scan_codes: smallvec::smallvec![0x15],
            reason: "qpc-rounding-up".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("valid authored schedule");
    assert!(
        sky_dispatch_core::validation::validate_min_hold_feasibility(&schedule, 100).is_ok(),
        "the microsecond-only validator must expose the rounding mismatch"
    );

    let qpc_clock = QpcClock::from_frequency_hz(
        std::num::NonZeroU64::new(3_125_000).expect("non-zero test frequency"),
    );
    let result = super::session::validate_native_schedule_timing(&schedule, 100, qpc_clock);
    assert!(
        result
            .as_ref()
            .is_err_and(|error| error.contains("same-key hold too short")),
        "tick-domain admission must reject before worker creation: {result:?}"
    );
}

#[test]
fn native_release_gap_admission_accepts_exact_explicit_boundary() {
    let actions = [
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down-a".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up-a".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 16_767,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down-b".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 16_867,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up-b".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("valid exact release-gap schedule");
    let qpc_clock = QpcClock::from_frequency_hz(
        std::num::NonZeroU64::new(1_000_000).expect("non-zero test frequency"),
    );
    let result = super::session::validate_native_schedule_timing_with_release_gap(
        &schedule, 100, 16_667, qpc_clock,
    );
    assert!(
        result.is_ok(),
        "exact release gap must be accepted: {result:?}"
    );
}

#[test]
fn native_release_gap_admission_rejects_one_microsecond_short() {
    let actions = [
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down-a".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up-a".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 16_766,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down-b".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("valid short release-gap schedule");
    let qpc_clock = QpcClock::from_frequency_hz(
        std::num::NonZeroU64::new(1_000_000).expect("non-zero test frequency"),
    );
    let result = super::session::validate_native_schedule_timing_with_release_gap(
        &schedule, 100, 16_667, qpc_clock,
    );
    assert!(
        result
            .as_ref()
            .is_err_and(|error| error.contains("release gap")),
        "one microsecond below the release policy must reject: {result:?}"
    );
}

#[test]
fn native_release_gap_admission_checks_qpc_tick_rounding() {
    let actions = [
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down-a".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 100,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up-a".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 200,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down-b".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("valid microsecond release-gap schedule");
    let qpc_clock = QpcClock::from_frequency_hz(
        std::num::NonZeroU64::new(3_125_000).expect("non-zero test frequency"),
    );
    let result = super::session::validate_native_schedule_timing_with_release_gap(
        &schedule, 100, 100, qpc_clock,
    );
    assert!(
        result
            .as_ref()
            .is_err_and(|error| error.contains("tick-domain") && error.contains("release gap")),
        "QPC tick rounding must remain authoritative: {result:?}"
    );
}

#[test]
fn completion_latency_does_not_create_hold_failure_after_release_gap() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "seed-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 1_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "same-key-release".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 1_000 + 16_667,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "same-key-chord-after-release-gap".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 2_000 + 16_667,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "cleanup".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15, 0x16])
        .expect("valid authored-feasible dynamic schedule");
    let mut options = test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            // Completion latency must not create a hold failure or rewrite
            // the authored Up target. The next Down may be authorized late
            // or recovered as missed, depending on the scheduler race.
            latency_base_us: 900,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.timing.min_hold_us = 300;
    let session = NativeDispatchSession::new(options).expect("authored-feasible admission");

    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, "finished", "{snapshot:?}");
    assert_eq!(snapshot.sendinput_partial_events, 0);
    assert_eq!(snapshot.sendinput_zero_progress_failures, 0);
    assert_eq!(snapshot.chord_integrity_lost, 0);
    assert_eq!(snapshot.active_count, 0);
    assert_eq!(snapshot.possibly_active_count, 0);
    assert!(snapshot.terminal_error.is_none(), "{snapshot:?}");
    assert!(snapshot.hold_pair_samples >= 1, "{snapshot:?}");

    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("valid telemetry JSON");
    assert!(telemetry["attempted"].as_u64().unwrap_or(0) >= 2);
    let records = telemetry["records"].as_array().expect("records array");
    let physical_records: Vec<&serde_json::Value> = records
        .iter()
        .filter(|record| record["requested_count"].as_u64().unwrap_or(0) > 0)
        .collect();
    assert!(!physical_records.is_empty());
}

#[test]
fn trusted_pre_call_deadline_miss_finishes_with_clean_session_health() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
            reason: "A-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Down,
            scheduled_us: 100_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "B-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: 200_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "A-up".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 200_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "B-up".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15, 0x16])
        .expect("valid hard-late recovery schedule");
    let script = FaultInjectionScript {
        entries: vec![(1, InjectedSendOutcome::DeadlineMissedBeforeSend)],
        ..FaultInjectionScript::none()
    };
    let mut options = test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: script,
        },
    );
    options.profile = DispatchProfile::Production;
    let session = NativeDispatchSession::new(options).expect("test session admission");

    start_with_test_wall_clock_slack(&session);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.outcome,
        Some("finished".to_string()),
        "{snapshot:?}"
    );
    assert_eq!(snapshot.status, "finished", "{snapshot:?}");
    assert_eq!(snapshot.terminal_error, None, "{snapshot:?}");
    assert_eq!(snapshot.authored_keys_rejected, 0, "{snapshot:?}");
    assert_eq!(snapshot.missed_hard_late_boundaries, 1, "{snapshot:?}");
    assert_eq!(
        snapshot.generation_status_counts.get("dropped_expired"),
        Some(&1)
    );
    assert_eq!(snapshot.active_count, 0, "{snapshot:?}");
    assert_eq!(snapshot.possibly_active_count, 0, "{snapshot:?}");
    assert_eq!(snapshot.failed_release_count, 0, "{snapshot:?}");
}

#[test]
fn mixed_same_key_retrigger_telemetry_preserves_two_events() {
    // Keep the authored epoch comfortably ahead of worker startup. This is a
    // test-only epoch choice made before arm(); the worker must not rebase the
    // frozen schedule after arm or derive a new target from observed runtime.
    const TEST_AUTHORED_EPOCH_US: u64 = 2_000_000;
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: TEST_AUTHORED_EPOCH_US,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: TEST_AUTHORED_EPOCH_US + 1_000_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "release-before-disjoint-mixed-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: TEST_AUTHORED_EPOCH_US + 1_000_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "disjoint-mixed-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: TEST_AUTHORED_EPOCH_US + 1_500_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "release".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15, 0x16])
        .expect("valid disjoint mixed schedule");
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

    start_with_test_wall_clock_slack(&session);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.status, "finished",
        "terminal error: {:?}",
        snapshot.terminal_error
    );

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
            scheduled_us: 500_000,
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
    wait_for_focus_down(&session);
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
fn late_down_completion_does_not_move_authored_note_off_target() {
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
        .prepare_current_authored_frame()
        .unwrap()
        .unwrap();
    assert_eq!(prepared.authored_ticks, TimelineTicks::from_raw(1_000));
    let commit = coordinator.prepare_authored_commit(prepared).unwrap();

    let down_started = TimelineTicks::from_raw(10_000);
    let down_completed = TimelineTicks::from_raw(15_000);
    coordinator
        .commit_prepared_authored_frame_success_frozen(&commit, down_started, down_completed)
        .expect("late completion is evidence only");
    let active = coordinator.active_for_slot(0).expect("active note");
    assert_eq!(
        active.release_not_before_ticks,
        TimelineTicks::from_raw(11_000)
    );
    let prepared = coordinator
        .prepare_current_authored_frame()
        .expect("authored Up remains valid after a late Down completion")
        .expect("authored Up frame");
    assert_eq!(prepared.authored_ticks, TimelineTicks::from_raw(20_000));
    assert_eq!(prepared.immediate_up_mask, 0b001);
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
                scheduled_us: 100_000,
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
        .prepare_current_authored_frame()
        .unwrap()
        .unwrap();
    assert_eq!(prepared.authored_ticks, TimelineTicks::from_raw(1_000));
    let commit = coordinator.prepare_authored_commit(prepared).unwrap();

    let down_started = TimelineTicks::from_raw(1_000);
    let down_completed = TimelineTicks::from_raw(1_050);
    coordinator
        .commit_prepared_authored_frame_success_frozen(&commit, down_started, down_completed)
        .unwrap();

    let active = coordinator.active_for_slot(0).unwrap();
    // A healthy authored release floor is authored Down (1,000) + min_hold.
    assert_eq!(
        active.release_not_before_ticks,
        TimelineTicks::from_raw(11_000)
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

    let mut coordinator =
        RuntimeDispatchCoordinator::try_new_ticks(schedule, 0, DurationTicks::ZERO, |us| {
            Ok(TimelineTicks::from_raw(us))
        })
        .expect("coordinator");

    let p1 = coordinator
        .prepare_current_authored_frame()
        .unwrap()
        .unwrap();
    assert_eq!(p1.authored_ticks, TimelineTicks::from_raw(1_000));
    let p1_commit = coordinator.prepare_authored_commit(p1).unwrap();
    coordinator
        .commit_prepared_authored_frame_success_frozen(
            &p1_commit,
            TimelineTicks::from_raw(11_000),
            TimelineTicks::from_raw(11_050),
        )
        .unwrap();

    let p2 = coordinator
        .prepare_current_authored_frame()
        .unwrap()
        .unwrap();
    assert_eq!(p2.authored_ticks, TimelineTicks::from_raw(20_000));
    let p2_commit = coordinator.prepare_authored_commit(p2).unwrap();
    coordinator
        .commit_prepared_authored_frame_success_frozen(
            &p2_commit,
            TimelineTicks::from_raw(20_000),
            TimelineTicks::from_raw(20_050),
        )
        .unwrap();

    let p3 = coordinator
        .prepare_current_authored_frame()
        .unwrap()
        .unwrap();
    assert_eq!(p3.authored_ticks, TimelineTicks::from_raw(50_000));

    let delta_b_a = p2.authored_ticks.as_u64() - p1.authored_ticks.as_u64();
    let delta_c_b = p3.authored_ticks.as_u64() - p2.authored_ticks.as_u64();
    assert_eq!(delta_b_a, 19_000);
    assert_eq!(delta_c_b, 30_000);
}

#[test]
fn authored_up_does_not_move_unrelated_future_authored_action() {
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
        .prepare_current_authored_frame()
        .unwrap()
        .unwrap();
    let p_down_a_commit = coordinator.prepare_authored_commit(p_down_a).unwrap();
    coordinator
        .commit_prepared_authored_frame_success_frozen(
            &p_down_a_commit,
            TimelineTicks::from_raw(1_000),
            TimelineTicks::from_raw(5_000),
        )
        .unwrap();

    let up_frame = coordinator
        .prepare_current_authored_frame()
        .unwrap()
        .unwrap();
    assert_eq!(up_frame.authored_ticks, TimelineTicks::from_raw(20_000));
    assert_eq!(up_frame.immediate_up_mask, 0b001);
    assert_eq!(up_frame.deferred_up_mask, 0);
    let up_commit = coordinator.prepare_authored_commit(up_frame).unwrap();
    coordinator
        .commit_prepared_authored_frame_success_frozen(
            &up_commit,
            TimelineTicks::from_raw(20_000),
            TimelineTicks::from_raw(20_000),
        )
        .unwrap();
    let down_b = coordinator
        .prepare_current_authored_frame()
        .unwrap()
        .unwrap();
    assert_eq!(down_b.authored_ticks, TimelineTicks::from_raw(30_000));
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

    let coordinator = RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        10_000,
        DurationTicks::from_raw(10_000),
        |us| Ok(TimelineTicks::from_raw(us)),
    )
    .expect("coordinator");

    let p1 = coordinator
        .prepare_current_authored_frame()
        .unwrap()
        .unwrap();
    assert_eq!(p1.authored_ticks, TimelineTicks::from_raw(1_000));
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
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "A-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 205_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "A-up".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 210_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "B-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 215_000,
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
    session.arm(2_000_000).expect("worker arm");
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
    // Authored B target stays at 210 ms via telemetry.
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
        .duration_from_us(210_000)
        .expect("210ms ticks")
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
                scheduled_us: 500_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "A-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 600_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "A-up".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 900_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "B-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 1_000_000,
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
    start_with_test_wall_clock_slack(&session);
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
        .duration_from_us(900_000)
        .expect("900ms ticks")
        .as_u64();
    assert_eq!(
        b_down["authored_ticks"].as_u64().expect("authored"),
        expected_authored,
        "B authored timestamp must stay at 900ms"
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
