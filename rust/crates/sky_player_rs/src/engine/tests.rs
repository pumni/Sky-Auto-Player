use super::telemetry::metrics::RecentLatencyRing;
use super::test_support::command_timing::{
    CommandTimingError, CommandTimingLookup as PauseTimingLookup, PauseTimingPhase,
};
use super::worker::dispatch::{effective_pending_cohort_lead, effective_pending_lead};
use super::{
    BackendConfig, CommandTimingResult, CommandTimingState, DispatchPath, DownAdmission,
    EstimatorOptions, FaultInjectionScript, FinalControlAdmission, FinalControlSignals,
    FinalTargetSignals, FocusOptions, HealthWindow, HealthWindowPolicy, InjectedSendOutcome,
    NativeDispatchSession, NativeSessionOptions, PlatformSendResult, PriorityOptions,
    RELEASE_RETRY_BACKOFF_US, RtTraceRecord, SharedMetrics, TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN,
    TargetStamp, TelemetryCollector, TelemetryMode, TelemetryOptions, TimingOptions, TraceContext,
    TraceDelivery, TraceTiming, TrackedKeyState, WaitOptions, WakeErrorStats, Worker,
    WorkerMetricsLocal, adjust_spin_threshold, anchored_dispatch_target_ticks,
    cpu_metrics_sample_due, deadline_target_ticks, derive_spin_threshold_us,
    ensure_preflight_for_target, exact_sender_durations, final_control_admission_with_lease,
    final_down_target_admission, focus_gate_matches, focus_matches, focus_matches_hwnd,
    record_input_path_health, record_termination_error, release_runtime_outcome,
    signed_timeline_delta_ticks, supervisor_lease_expired, target_stamp_still_current,
    trace_outcome_code, try_publish_metrics, wake_lateness_ticks,
};
use sky_dispatch_core::coordinator::PendingRelease;
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
            game_fps: 60,
            min_hold_us: 0,
            max_lead_us: 2_000,
            dispatch_lead_us: 0,
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
        estimator: EstimatorOptions {
            state_json: None,
            enable_dispatch_cost_lead: false,
        },
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
fn adaptive_startup_preserves_sublead_authored_order_in_mock_session() {
    use sky_dispatch_core::compile::compile_runtime_intents;
    use sky_dispatch_core::estimator::ESTIMATOR_STATE_VERSION;
    use sky_dispatch_core::estimator::{DispatchCostEstimator, SendPath};

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "adaptive-startup-a".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![0x16],
                reason: "adaptive-startup-b".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "adaptive-startup-a-up".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 1_100,
                scan_codes: smallvec::smallvec![0x16],
                reason: "adaptive-startup-b-up".to_string().into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid adaptive startup schedule");

    let mut seeded = DispatchCostEstimator::try_new(2_000, 30).expect("create estimator");
    for _ in 0..5 {
        seeded
            .update(SendPath::DownOnly, 1, 500)
            .expect("seed estimator");
    }
    let seeded_state = serde_json::to_string(&seeded.export_state()).expect("seeded state JSON");
    assert!(seeded_state.contains(&format!("\"version\":{ESTIMATOR_STATE_VERSION}")));

    let mut options = test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.estimator.state_json = Some(seeded_state);
    options.estimator.enable_dispatch_cost_lead = true;
    options.telemetry.capacity = 8;
    options.wait.supervisor_lease_timeout_us = 3_000_000;

    let session = NativeDispatchSession::new(options).expect("adaptive session admission");
    session.start().expect("adaptive worker start");
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("adaptive worker join")
    );

    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.outcome,
        Some("finished".to_string()),
        "terminal error: {:?}",
        snapshot.terminal_error
    );
    assert_eq!(snapshot.terminal_error, None);
    assert_eq!(snapshot.keys_dropped, 0);
    assert_eq!(snapshot.chord_split_events, 0);
    assert_eq!(snapshot.sendinput_partial_events, 0);
    assert_eq!(snapshot.sendinput_zero_progress_failures, 0);
    assert_eq!(snapshot.authored_keys_rejected, 0);
    assert_eq!(snapshot.chord_integrity_lost, 0);
    assert_eq!(snapshot.active_count, 0);
    assert_eq!(snapshot.possibly_active_count, 0);

    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("valid telemetry JSON");
    let records = telemetry["records"].as_array().expect("records array");
    let first = records
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(0))
        .expect("first authored record");
    let second = records
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(1))
        .expect("second authored record");
    let clock = QpcClock::initialize().expect("QPC");
    let second_authored_ticks = clock.duration_from_us(100).expect("100us ticks").as_u64();
    let startup_lead_ticks = clock.duration_from_us(500).expect("500us ticks").as_u64();

    assert_eq!(first["kind"].as_u64(), Some(0));
    assert_eq!(second["kind"].as_u64(), Some(0));
    assert_eq!(first["effective_deadline_ticks"].as_u64(), Some(0));
    assert_eq!(
        first["applied_lead_ticks"].as_u64(),
        Some(startup_lead_ticks)
    );
    assert_eq!(
        second["effective_deadline_ticks"].as_u64(),
        Some(second_authored_ticks)
    );
    assert_ne!(
        second["effective_deadline_ticks"].as_u64(),
        first["effective_deadline_ticks"].as_u64()
    );
    assert_eq!(second["applied_lead_ticks"].as_u64(), Some(0));

    let exported: serde_json::Value = serde_json::from_str(
        &session
            .estimator_state_json()
            .expect("exported estimator state"),
    )
    .expect("valid exported estimator state");
    assert_eq!(
        exported["version"].as_u64(),
        Some(u64::from(ESTIMATOR_STATE_VERSION))
    );
}

fn pending_for_lead_test(
    scheduled: u64,
    release_not_before: u64,
    next_retry: u64,
) -> PendingRelease {
    PendingRelease {
        generation_id: 1,
        scan_code: 0x15,
        key_slot: 0,
        source_action_index: 0,
        packet_id: 0,
        scheduled_release_us: scheduled,
        scheduled_release_ticks: TimelineTicks::from_raw(scheduled),
        down_dispatch_started_ticks: TimelineTicks::ZERO,
        release_not_before_ticks: TimelineTicks::from_raw(release_not_before),
        reason_id: 0,
        retry_count: 0,
        next_retry_ticks: TimelineTicks::from_raw(next_retry),
        first_failure_ticks: None,
        last_win32_error: None,
    }
}

#[test]
fn pending_floors_suppress_applied_lead_and_saturation() {
    for (release, deadline, expected) in [
        (
            pending_for_lead_test(1_000, 700, 0),
            700,
            (DurationTicks::ZERO, false),
        ),
        (
            pending_for_lead_test(1_000, 0, 700),
            700,
            (DurationTicks::ZERO, false),
        ),
        (
            pending_for_lead_test(1_000, 0, 0),
            500,
            (DurationTicks::from_raw(500), true),
        ),
    ] {
        assert_eq!(
            effective_pending_lead(
                &release,
                DurationTicks::from_raw(500),
                true,
                TimelineTicks::from_raw(deadline),
            ),
            expected
        );
    }

    let release = pending_for_lead_test(1_000, 700, 0);
    let mut streak = 0u8;
    for _ in 0..3 {
        let (applied_lead, saturated) = effective_pending_lead(
            &release,
            DurationTicks::from_raw(500),
            true,
            TimelineTicks::from_raw(700),
        );
        assert_eq!(applied_lead, DurationTicks::ZERO);
        streak = if saturated {
            streak.saturating_add(1)
        } else {
            0
        };
    }
    assert_eq!(streak, 0);
}

#[test]
fn pending_cohort_saturation_is_independent_of_source_order() {
    let mut lead_controlled = pending_for_lead_test(1_000, 0, 0);
    lead_controlled.source_action_index = 1;
    let mut floor_tie = pending_for_lead_test(1_000, 500, 0);
    floor_tie.source_action_index = 2;
    let first_lead: smallvec::SmallVec<[PendingRelease; 15]> =
        smallvec::smallvec![lead_controlled.clone(), floor_tie.clone()];
    let first_floor: smallvec::SmallVec<[PendingRelease; 15]> =
        smallvec::smallvec![floor_tie, lead_controlled];
    let expected = (DurationTicks::ZERO, false);
    for cohort in [&first_lead, &first_floor] {
        assert_eq!(
            effective_pending_cohort_lead(
                cohort,
                DurationTicks::from_raw(500),
                true,
                TimelineTicks::from_raw(500),
            ),
            expected
        );
    }
    let mut streaks = [0u8; 2];
    for _ in 0..3 {
        for streak in &mut streaks {
            *streak = if expected.1 {
                streak.saturating_add(1)
            } else {
                0
            };
        }
    }
    assert_eq!(streaks, [0, 0]);
}

fn progress_idle_gap_schedule() -> sky_dispatch_core::model::RuntimeSchedule {
    sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
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

    session.pause().expect("pause request");
    let pause_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if session.snapshot_lite().is_paused {
            break;
        }
        assert!(
            Instant::now() < pause_deadline,
            "pause was not acknowledged"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let paused_at = session.snapshot_lite().elapsed_us;
    std::thread::sleep(Duration::from_millis(100));
    let paused_after = session.snapshot_lite().elapsed_us;
    assert!(paused_after.saturating_sub(paused_at) < 20_000);

    session.resume().expect("resume request");
    let resume_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if !session.snapshot_lite().is_paused {
            break;
        }
        assert!(
            Instant::now() < resume_deadline,
            "resume was not acknowledged"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    std::thread::sleep(Duration::from_millis(100));
    let resumed_after = session.snapshot_lite().elapsed_us;
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
        "drop-oldest queue should retain observations near the end of the schedule"
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
fn retry_backoff_values_use_exact_qpc_conversion() {
    let clock = QpcClock::initialize().expect("QPC clock");
    let expected_us = [2_000, 5_000, 10_000, 20_000];
    for (delay_us, expected) in RELEASE_RETRY_BACKOFF_US.into_iter().zip(expected_us) {
        let ticks = clock
            .duration_from_us(delay_us)
            .expect("retry delay conversion");
        assert_eq!(
            clock.duration_to_us(ticks).expect("round-trip conversion"),
            expected
        );
    }
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
fn pending_release_uses_final_control_and_lease_admission() {
    use super::test_support::ProductionDispatchTestHarness;

    for command in ["pause", "quit", "skip", "panic"] {
        let mut harness = ProductionDispatchTestHarness::new_uponly_release();
        let calls = harness.configure_send_counter();
        harness.advance_playback_time_us(10_000);
        harness.seed_pending_release_for_test();
        let plan = harness.plan_current_dispatch();
        let pending_plan = plan.pending().expect("pending release plan");
        let due_pending = harness.pop_due_pending_for_plan(harness.current_effective_time(), &plan);
        match command {
            "pause" => harness.desired_pause.store(true, Ordering::Release),
            "quit" => harness.quit_requested.store(true, Ordering::Release),
            "skip" => harness.skip_requested.store(true, Ordering::Release),
            "panic" => harness.panic_requested.store(true, Ordering::Release),
            _ => unreachable!("test command table"),
        }
        let step = harness.dispatch_pending_release_with_plan(due_pending, Some(pending_plan));
        assert!(
            matches!(step, super::worker::DispatchStep::Continue),
            "{command} must reject pending release before transport: {step:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0, "{command} sent physically");
    }

    let mut harness = ProductionDispatchTestHarness::new_uponly_release();
    let calls = harness.configure_send_counter();
    harness.advance_playback_time_us(10_000);
    harness.seed_pending_release_for_test();
    let plan = harness.plan_current_dispatch();
    let pending_plan = plan.pending().expect("pending release plan");
    let due_pending = harness.pop_due_pending_for_plan(harness.current_effective_time(), &plan);
    harness
        .supervisor_heartbeat_ticks
        .store(1, Ordering::Release);
    let step = harness.dispatch_pending_release_with_plan_and_lease(
        due_pending,
        Some(pending_plan),
        DurationTicks::from_raw(1),
    );
    assert!(matches!(step, super::worker::DispatchStep::Continue));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let mut harness = ProductionDispatchTestHarness::new_uponly_release();
    harness.config.focus.require_focus = true;
    harness.focus_active.store(false, Ordering::Release);
    let calls = harness.configure_send_counter();
    harness.advance_playback_time_us(10_000);
    harness.seed_pending_release_for_test();
    let plan = harness.plan_current_dispatch();
    let pending_plan = plan.pending().expect("pending release plan");
    let due_pending = harness.pop_due_pending_for_plan(harness.current_effective_time(), &plan);
    let step = harness.dispatch_pending_release_with_plan(due_pending, Some(pending_plan));
    assert!(matches!(step, super::worker::DispatchStep::Dispatched));
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

    let mut pending_future = ProductionDispatchTestHarness::new_uponly_release();
    pending_future.advance_playback_time_us(10_000);
    pending_future.seed_pending_release_for_test();
    let pending_future_plan = pending_future.plan_current_dispatch();
    pending_future.set_effective_time_for_test(TimelineTicks::ZERO);
    assert!(matches!(
        pending_future.dispatch_due_from_plan_for_test(&pending_future_plan),
        super::worker::DispatchStep::NoWork
    ));

    let mut pending = ProductionDispatchTestHarness::new_uponly_release();
    let pending_calls = pending.configure_send_counter();
    pending.advance_playback_time_us(10_000);
    pending.seed_pending_release_for_test();
    let pending_plan = pending.plan_current_dispatch();
    assert!(pending_plan.authored.is_none());
    assert!(matches!(
        pending.dispatch_due_from_plan_for_test(&pending_plan),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(pending_calls.load(Ordering::SeqCst), 1);

    let mut invalid_pending_plan = pending_plan.clone();
    invalid_pending_plan.pending_budget = None;
    assert!(!super::worker::plan_structure_is_valid(
        &invalid_pending_plan
    ));
    assert!(matches!(
        pending.dispatch_due_from_plan_for_test(&invalid_pending_plan),
        super::worker::DispatchStep::Terminate(_)
    ));

    let mut both = ProductionDispatchTestHarness::new_pending_future_with_authored_due();
    let initial_plan = both.plan_current_dispatch();
    assert!(matches!(
        both.dispatch_authored_with_plan(&initial_plan),
        super::worker::DispatchStep::Dispatched
    ));
    both.advance_playback_time_us(1_000);
    both.resources.coordinator.min_hold_ticks = both
        .resources
        .clock
        .duration_from_us(10_000)
        .expect("minimum hold conversion");
    both.seed_pending_release_for_test();
    let both_plan = both.plan_current_dispatch();
    let both_calls = both.configure_send_counter();
    assert!(both_plan.pending.is_some());
    assert!(both_plan.authored.is_some());
    both.advance_playback_time_us(10_000);
    assert!(matches!(
        both.dispatch_due_from_plan_for_test(&both_plan),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(both_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn frozen_plan_pending_future_allows_authored_due_dispatch() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_pending_future_with_authored_due();
    let initial_plan = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_authored_with_plan(&initial_plan),
        super::worker::DispatchStep::Dispatched
    ));
    harness.advance_playback_time_us(1_000);
    harness.resources.coordinator.min_hold_ticks = harness
        .resources
        .clock
        .duration_from_us(10_000)
        .expect("minimum hold conversion");
    harness.seed_pending_release_for_test();
    let plan = harness.plan_current_dispatch();
    assert!(plan.pending.is_some());
    assert!(plan.authored.is_some());
    harness.advance_playback_time_us(1_000);
    let calls = harness.configure_send_counter();

    let step = harness.dispatch_due_from_plan_for_test(&plan);

    assert!(
        matches!(step, super::worker::DispatchStep::Dispatched),
        "pending-future authored-due step: {step:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
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
            send_started_ticks: Some(TimelineTicks::from_raw(20)),
            send_completed_ticks: Some(TimelineTicks::from_raw(25)),
            dispatch_cost_us: 5,
            core_post_send_duration_us: 4,
            post_send_metrics_available: true,
            completion_error_ticks: 1,
            authored_completion_error_ticks: 2,
            applied_lead_ticks: DurationTicks::from_raw(2),
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
            send_started_ticks: None,
            send_completed_ticks: None,
            dispatch_cost_us: 0,
            core_post_send_duration_us: 0,
            post_send_metrics_available: false,
            completion_error_ticks: 0,
            authored_completion_error_ticks: 0,
            applied_lead_ticks: DurationTicks::ZERO,
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
            send_started_ticks: None,
            send_completed_ticks: None,
            dispatch_cost_us: 0,
            core_post_send_duration_us: 0,
            post_send_metrics_available: false,
            completion_error_ticks: 0,
            authored_completion_error_ticks: 0,
            applied_lead_ticks: DurationTicks::ZERO,
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
        ("worker/dispatch/release.rs", 900),
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
fn observer_drain_rebuilds_production_plan_before_mixed_dispatch() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_down_then_mixed();
    harness.config.estimator.enable_dispatch_cost_lead = true;
    harness.config.timing.dispatch_lead_us = 0;

    let plan_a = harness.plan_current_dispatch_projected();
    assert!(matches!(
        plan_a.authored_path(),
        Some(DispatchPath::Mixed { .. })
    ));
    let estimator_a = serde_json::to_string(&harness.resources.estimator.export_state())
        .expect("estimator state");

    let drain_result = harness.drain_observer().expect("observer drain");
    assert!(
        drain_result.is_some(),
        "draining an observation must invalidate the plan"
    );

    let plan_b = harness.plan_current_dispatch_projected();
    let estimator_b = serde_json::to_string(&harness.resources.estimator.export_state())
        .expect("updated estimator state");
    assert_ne!(
        estimator_a, estimator_b,
        "observer must mutate the estimator"
    );
    harness.advance_playback_time_us(2_000);
    assert!(matches!(
        harness.dispatch_authored_with_plan(&plan_b),
        super::worker::DispatchStep::Dispatched
    ));
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
        0,
        DurationTicks::ZERO,
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
    let estimator =
        sky_dispatch_core::estimator::DispatchCostEstimator::try_new(2_000, 1).expect("estimator");
    let telemetry = TelemetryCollector::new(TelemetryMode::Ring, 64);
    let waiter = sky_dispatch_win32::wait::HybridWaiter::with_options(true, true);

    let resources = WorkerResources {
        clock: qpc_clock,
        waiter,
        backend,
        coordinator,
        playback,
        estimator,
        telemetry,
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
    let mut coordinator = RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        min_hold_us,
        min_hold_ticks,
        0,
        DurationTicks::ZERO,
        |us| Ok(TimelineTicks::from_raw(us)),
    )
    .expect("coordinator");

    let prepared = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(1_000), DurationTicks::ZERO)
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
    let mut coordinator = RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        min_hold_us,
        min_hold_ticks,
        0,
        DurationTicks::ZERO,
        |us| Ok(TimelineTicks::from_raw(us)),
    )
    .expect("coordinator");

    let prepared = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(1_000), DurationTicks::ZERO)
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
        0,
        DurationTicks::ZERO,
        |us| Ok(TimelineTicks::from_raw(us)),
    )
    .expect("coordinator");

    let p1 = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(11_000), DurationTicks::ZERO)
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
        .prepare_next_due_authored(TimelineTicks::from_raw(20_000), DurationTicks::ZERO)
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
        .prepare_next_due_authored(TimelineTicks::from_raw(50_000), DurationTicks::ZERO)
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
        0,
        DurationTicks::ZERO,
        |us| Ok(TimelineTicks::from_raw(us)),
    )
    .expect("coordinator");

    let p_down_a = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(1_000), DurationTicks::ZERO)
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
        .prepare_next_due_authored(TimelineTicks::from_raw(25_000), DurationTicks::ZERO)
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
        .prepare_next_due_authored(TimelineTicks::from_raw(30_000), DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(
        p_down_b.effective_scheduled_ticks,
        TimelineTicks::from_raw(30_000)
    );
}

#[test]
fn explicit_release_recovery_may_shift_timeline() {
    use sky_dispatch_core::compile::compile_runtime_intents;
    use sky_dispatch_core::coordinator::{RuntimeDispatchCoordinator, TimelineRebaseReason};
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
        ],
        &[0x15],
    )
    .expect("schedule");

    let mut coordinator = RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        10_000,
        DurationTicks::from_raw(10_000),
        0,
        DurationTicks::ZERO,
        |us| Ok(TimelineTicks::from_raw(us)),
    )
    .expect("coordinator");

    let p_down_a = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(1_000), DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    coordinator
        .commit_packet_success(
            p_down_a,
            TimelineTicks::from_raw(1_000),
            TimelineTicks::from_raw(1_050),
        )
        .unwrap();

    let p_up_a = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(20_000), DurationTicks::ZERO)
        .unwrap()
        .unwrap();
    let (requested, _) = coordinator.commit_up_request(p_up_a).unwrap();
    assert_eq!(requested.len(), 1);

    let plan = coordinator
        .plan_pending_dispatch_ticks(|_| Ok((DurationTicks::ZERO, false)))
        .unwrap()
        .unwrap();
    let due = coordinator
        .pop_due_pending_ticks(TimelineTicks::from_raw(20_000), &plan)
        .unwrap();
    assert_eq!(due.len(), 1);

    let _ = coordinator
        .requeue_unconfirmed_releases_ticks(
            &due,
            0,
            TimelineTicks::from_raw(20_000),
            TimelineTicks::from_raw(25_000),
            &[DurationTicks::from_raw(5_000)],
            Some(5),
        )
        .unwrap();

    coordinator.complete_releases_mask(&due, 1 << 0).unwrap();
    let pause = coordinator
        .finish_release_recovery_ticks(TimelineTicks::from_raw(25_000))
        .unwrap();
    assert_eq!(
        coordinator.last_timeline_rebase_reason(),
        Some(TimelineRebaseReason::ReleaseRecovery)
    );
    assert_eq!(pause, Some(DurationTicks::from_raw(5_000)));
    assert_eq!(
        coordinator.recovery_offset_ticks(),
        DurationTicks::from_raw(5_000)
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
        0,
        DurationTicks::ZERO,
        |us| Ok(TimelineTicks::from_raw(us)),
    )
    .expect("coordinator");

    let p1 = coordinator
        .prepare_next_due_authored(TimelineTicks::from_raw(1_000), DurationTicks::ZERO)
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
