use super::support::*;
use super::*;

#[cfg(all(feature = "test-support", windows))]
fn stale_metadata_race_schedule(
    stale_packet_count: usize,
) -> sky_dispatch_core::model::RuntimeSchedule {
    let mut actions = Vec::with_capacity(stale_packet_count + 4);
    actions.push(KeyActionInput {
        source_action_index: 0,
        kind: ActionKind::Down,
        scheduled_us: 100_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "late-control-initial-down".into(),
    });
    actions.push(KeyActionInput {
        source_action_index: 1,
        kind: ActionKind::Up,
        scheduled_us: 150_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "late-control-initial-up".into(),
    });
    for index in 0..stale_packet_count {
        actions.push(KeyActionInput {
            source_action_index: u32::try_from(index + 2).expect("stale source index"),
            kind: ActionKind::Up,
            scheduled_us: 200_000 + index as u64,
            scan_codes: smallvec::smallvec![0x15],
            reason: "late-control-stale-up".into(),
        });
    }
    let final_down_index = u32::try_from(stale_packet_count + 2).expect("final Down index");
    actions.push(KeyActionInput {
        source_action_index: final_down_index,
        kind: ActionKind::Down,
        scheduled_us: 500_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "late-control-final-down".into(),
    });
    actions.push(KeyActionInput {
        source_action_index: final_down_index + 1,
        kind: ActionKind::Up,
        scheduled_us: 550_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "late-control-final-up".into(),
    });
    sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("stale metadata race schedule")
}

#[cfg(all(feature = "test-support", windows))]
fn post_wake_stale_metadata_schedule(
    stale_packet_count: usize,
) -> sky_dispatch_core::model::RuntimeSchedule {
    let mut actions = Vec::with_capacity(stale_packet_count + 6);
    actions.push(KeyActionInput {
        source_action_index: 0,
        kind: ActionKind::Down,
        scheduled_us: 5_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "post-wake-initial-down".into(),
    });
    actions.push(KeyActionInput {
        source_action_index: 1,
        kind: ActionKind::Up,
        scheduled_us: 10_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "post-wake-initial-up".into(),
    });
    actions.push(KeyActionInput {
        source_action_index: 2,
        kind: ActionKind::Down,
        scheduled_us: 14_000,
        scan_codes: smallvec::smallvec![0x16],
        reason: "post-wake-intermediate-down".into(),
    });
    actions.push(KeyActionInput {
        source_action_index: 3,
        kind: ActionKind::Up,
        scheduled_us: 14_500,
        scan_codes: smallvec::smallvec![0x16],
        reason: "post-wake-intermediate-up".into(),
    });
    for index in 0..stale_packet_count {
        actions.push(KeyActionInput {
            source_action_index: u32::try_from(index + 4).expect("stale source index"),
            kind: ActionKind::Up,
            scheduled_us: 15_000 + index as u64,
            scan_codes: smallvec::smallvec![0x15],
            reason: "post-wake-stale-up".into(),
        });
    }
    let final_down_index = u32::try_from(stale_packet_count + 4).expect("final Down index");
    actions.push(KeyActionInput {
        source_action_index: final_down_index,
        kind: ActionKind::Down,
        scheduled_us: 200_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "post-wake-final-down".into(),
    });
    actions.push(KeyActionInput {
        source_action_index: final_down_index + 1,
        kind: ActionKind::Up,
        scheduled_us: 210_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "post-wake-final-up".into(),
    });
    sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15, 0x16])
        .expect("32 stale metadata post-wake schedule")
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

#[cfg(all(feature = "test-support", windows))]
#[test]
fn production_wait_timer_creation_failure_is_terminal_before_armed() {
    let timer_context = sky_dispatch_win32::timer::test_support::new_counters();
    sky_dispatch_win32::timer::test_support::fail_next_creation(&timer_context);
    let mut options =
        test_session_options(startup_boundary_schedule(), 1, BackendConfig::Production);
    options.profile = DispatchProfile::Production;
    options.timer_lifecycle_context = Some(Arc::clone(&timer_context));
    let session = NativeDispatchSession::new(options).expect("production boundary session");

    session.arm(100_000).expect("worker startup request");
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("timer failure join")
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(snapshot.outcome.as_deref(), Some("error"));
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some("startup_failure")
    );
    assert!(!snapshot.startup_ready);
    assert!(!snapshot.startup_armed);
    assert_eq!(session.epoch_qpc_for_test(), QpcTicks::ZERO);
    let counts = sky_dispatch_win32::timer::test_support::snapshot(&timer_context);
    assert_eq!(counts.created, 0);
    assert_eq!(counts.dropped, 0);
    assert_eq!(counts.live, 0);
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

#[cfg(all(feature = "test-support", windows))]
#[test]
fn actual_b_worker_honors_late_quit_after_32_33_and_10k_stale_metadata() {
    for stale_count in [32usize, 33, 10_000] {
        let (session, hook, send_calls) = checkpointed_session(
            stale_metadata_race_schedule(stale_count),
            StartupCheckpoint::StaleMetadataChunk,
            1,
            false,
        );
        session.arm(0).expect("start actual B worker");
        assert!(
            hook.wait_for_startup_checkpoint(Duration::from_secs(5)),
            "stale packet count {stale_count}"
        );
        assert_eq!(hook.stale_packet_commit_count.load(Ordering::SeqCst), 32);
        assert_eq!(send_calls.load(Ordering::Acquire), 2);

        session.quit().expect("late quit accepted");
        hook.release_startup_checkpoint();
        assert!(session.join(Duration::from_secs(5)).expect("B worker join"));
        let snapshot = session.snapshot();
        assert_eq!(snapshot.status, EnginePollStatus::Quit.as_str());
        assert_eq!(snapshot.outcome.as_deref(), Some("quit"));
        assert_eq!(hook.stale_packet_commit_count.load(Ordering::SeqCst), 32);
        assert_eq!(
            send_calls.load(Ordering::Acquire),
            2,
            "final Down must not send after late quit with {stale_count} stale packets"
        );
    }
}

#[cfg(all(feature = "test-support", windows))]
#[test]
fn actual_b_worker_reports_stale_dispatch_timing_matrix() {
    use serde_json::{Value, json};

    fn delta_us(later: u64, earlier: u64, frequency_hz: u64) -> (i64, i64) {
        let ticks =
            i64::try_from(i128::from(later) - i128::from(earlier)).expect("QPC delta fits i64");
        let micros = i64::try_from((i128::from(ticks) * 1_000_000) / i128::from(frequency_hz))
            .expect("QPC microsecond delta fits i64");
        (ticks, micros)
    }

    fn nearest_rank(values: &[i64], percentile: usize) -> Option<i64> {
        if values.is_empty() {
            return None;
        }
        let mut ordered = values.to_vec();
        ordered.sort_unstable();
        let rank = (percentile * ordered.len()).div_ceil(100).max(1);
        Some(ordered[rank - 1])
    }

    let mut matrix = Vec::new();
    let mut qpc_frequency_hz = None;
    for stale_count in [0usize, 1, 32, 33, 10_000] {
        let repetitions = if stale_count == 10_000 { 20 } else { 100 };
        let mut samples = Vec::with_capacity(repetitions);
        let mut wake_to_pre_call_us = Vec::with_capacity(repetitions);
        let mut deadline_to_pre_call_us = Vec::with_capacity(repetitions);

        for repetition in 0..repetitions {
            let hook = Arc::new(StartupOrderingHook::default());
            let mut options = test_session_options(
                post_wake_stale_metadata_schedule(stale_count),
                1,
                BackendConfig::Mock {
                    latency_base_us: 0,
                    latency_per_key_us: 0,
                    fault_script: FaultInjectionScript::none(),
                },
            );
            options.wait.test_spin_threshold_us = None;
            options.wait.test_wait_policy = super::TestWaitPolicy::ProductionCalibrated;
            options.startup_ordering_hook = Some(Arc::clone(&hook));
            let session = NativeDispatchSession::new(options).expect("actual B worker session");
            session.arm(0).expect("worker arm");
            assert!(session.join(Duration::from_secs(10)).expect("worker join"));
            assert_eq!(session.snapshot().outcome.as_deref(), Some("finished"));

            let stale_committed = hook.stale_packet_commit_count.load(Ordering::Acquire);
            let stale_after_wake = hook
                .stale_packet_committed_after_wake_count
                .load(Ordering::Acquire);
            assert_eq!(stale_committed, stale_count as u64);
            assert!(stale_after_wake <= stale_committed);

            let telemetry: Value =
                serde_json::from_str(&session.take_telemetry_json().expect("worker telemetry"))
                    .expect("telemetry JSON");
            let frequency = telemetry["qpc_frequency_hz"]
                .as_u64()
                .expect("QPC frequency");
            if let Some(previous) = qpc_frequency_hz {
                assert_eq!(frequency, previous, "QPC frequency must be stable");
            } else {
                qpc_frequency_hz = Some(frequency);
            }
            assert_eq!(hook.down_sender_attempt_count(), 3);
            let (pre_call, deadline, wake) = hook.last_down_sender_qpc_evidence();
            let pre_call = pre_call.expect("final Down sender-boundary pre-call QPC");
            let deadline = deadline.or_else(|| {
                let (deadline, _) = hook.last_down_window();
                (deadline != 0).then_some(deadline)
            });
            let wake_delta = wake.map(|wake| delta_us(pre_call, wake, frequency));
            let deadline_delta = deadline.map(|deadline| delta_us(pre_call, deadline, frequency));
            if let Some((_, micros)) = wake_delta {
                wake_to_pre_call_us.push(micros);
            }
            if let Some((_, micros)) = deadline_delta {
                deadline_to_pre_call_us.push(micros);
            }
            samples.push(json!({
                "repetition": repetition,
                "stale_committed_before_final_send": stale_committed,
                "stale_committed_after_attributed_wake": stale_after_wake,
                "stale_committed_without_attributed_wake": stale_committed - stale_after_wake,
                "wake_available": wake.is_some(),
                "wake_qpc_ticks": wake,
                "physical_deadline_qpc_ticks": deadline,
                "pre_call_qpc_ticks": pre_call,
                "wake_to_pre_call_ticks": wake_delta.map(|delta| delta.0),
                "wake_to_pre_call_us": wake_delta.map(|delta| delta.1),
                "deadline_to_pre_call_ticks": deadline_delta.map(|delta| delta.0),
                "deadline_to_pre_call_us": deadline_delta.map(|delta| delta.1),
            }));
        }

        matrix.push(json!({
            "stale_metadata_count": stale_count,
            "repetitions": repetitions,
            "total_stale_scanned_per_final_send": stale_count,
            "qpc_frequency_hz": qpc_frequency_hz.expect("QPC frequency"),
            "wake_to_pre_call": {
                "measured_count": wake_to_pre_call_us.len(),
                "missing_wake_count": repetitions - wake_to_pre_call_us.len(),
                "p50_us": nearest_rank(&wake_to_pre_call_us, 50),
                "p95_us": nearest_rank(&wake_to_pre_call_us, 95),
                "p99_us": nearest_rank(&wake_to_pre_call_us, 99),
                "max_us": wake_to_pre_call_us.iter().max(),
                "estimator": "nearest_rank"
            },
            "deadline_to_pre_call": {
                "measured_count": deadline_to_pre_call_us.len(),
                "missing_deadline_count": repetitions - deadline_to_pre_call_us.len(),
                "p50_us": nearest_rank(&deadline_to_pre_call_us, 50),
                "p95_us": nearest_rank(&deadline_to_pre_call_us, 95),
                "p99_us": nearest_rank(&deadline_to_pre_call_us, 99),
                "max_us": deadline_to_pre_call_us.iter().max(),
                "estimator": "nearest_rank"
            },
            "samples": samples,
        }));
    }
    eprintln!(
        "stale_dispatch_timing_matrix_json={}",
        serde_json::to_string(&json!({
            "evidence_scope": "actual_b_worker_test_support_mock_sender_production_calibrated_frozen_wait",
            "qualification": false,
            "scan_attribution": "test-support hook counts stale commits while a deadline wake attribution is present; missing wake samples remain missing",
            "scenarios": matrix,
        }))
        .expect("stale matrix JSON")
    );
}

#[test]
fn startup_calibration_placement_stays_before_ready_and_tracks_guard_context() {
    for (placement, guards_precede_calibration) in [
        (
            super::config::StartupCalibrationPlacement::BeforeSchedulingGuards,
            false,
        ),
        (
            super::config::StartupCalibrationPlacement::AfterSchedulingGuards,
            true,
        ),
    ] {
        let hook = Arc::new(StartupOrderingHook::default());
        hook.arm_startup_checkpoint(StartupCheckpoint::WakeCalibration, 1);
        hook.complete_startup_calibration_after_checkpoint();
        let mut options = test_session_options(
            startup_boundary_schedule(),
            1,
            BackendConfig::Mock {
                latency_base_us: 0,
                latency_per_key_us: 0,
                fault_script: FaultInjectionScript::none(),
            },
        );
        options.profile = DispatchProfile::MockTest;
        options.wait.test_spin_threshold_us = None;
        options.wait.test_wait_policy = super::TestWaitPolicy::ProductionCalibrated;
        options.startup_calibration_placement = placement;
        options.startup_ordering_hook = Some(Arc::clone(&hook));
        let session = NativeDispatchSession::new(options).expect("calibration placement session");
        session.arm(100_000).expect("calibration placement arm");

        assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
        assert!(!session.snapshot().startup_ready);
        assert_eq!(session.epoch_qpc_for_test(), QpcTicks::ZERO);
        assert_eq!(
            hook.scheduling_guard_acquired_thread().is_some(),
            guards_precede_calibration,
            "placement {placement:?}"
        );
        hook.release_startup_checkpoint();
        assert!(session.join(Duration::from_secs(5)).expect("worker join"));

        let snapshot = session.snapshot();
        assert_eq!(snapshot.outcome.as_deref(), Some("finished"));
        assert!(snapshot.startup_ready);
        assert!(snapshot.startup_armed);
        assert!(snapshot.startup_calibration_executed);
        assert_eq!(snapshot.startup_calibration_sample_count, 6);
        assert_eq!(snapshot.startup_calibration_result, "completed");
        assert_eq!(snapshot.startup_priority_mode_acquired, "off");
        let (acquired, dropped) = hook
            .scheduling_guard_threads()
            .expect("guard lifetime is observable");
        assert_eq!(acquired, dropped);
    }
}

#[test]
fn accepted_stop_or_skip_during_candidate_calibration_cannot_publish_armed() {
    for (skip, expected_status, expected_outcome) in [
        (false, EnginePollStatus::Quit, "quit"),
        (true, EnginePollStatus::Skipped, "skipped"),
    ] {
        let hook = Arc::new(StartupOrderingHook::default());
        hook.arm_startup_checkpoint(StartupCheckpoint::WakeCalibration, 1);
        // Complete the controlled probe successfully after the command races
        // it. The post-probe startup gate must still give cancellation priority.
        hook.complete_startup_calibration_after_checkpoint();
        let send_calls = Arc::new(AtomicU64::new(0));
        let mut fault_script = FaultInjectionScript::none();
        fault_script.send_call_count = Some(Arc::clone(&send_calls));
        let mut options = test_session_options(
            startup_boundary_schedule(),
            1,
            BackendConfig::Mock {
                latency_base_us: 0,
                latency_per_key_us: 0,
                fault_script,
            },
        );
        options.profile = DispatchProfile::MockTest;
        options.wait.test_spin_threshold_us = None;
        options.wait.test_wait_policy = super::TestWaitPolicy::ProductionCalibrated;
        options.startup_calibration_placement =
            super::config::StartupCalibrationPlacement::AfterSchedulingGuards;
        options.startup_ordering_hook = Some(Arc::clone(&hook));
        let session = NativeDispatchSession::new(options).expect("candidate calibration session");
        session.arm(100_000).expect("candidate calibration arm");

        assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
        assert!(hook.scheduling_guard_acquired_thread().is_some());
        assert!(!session.snapshot().startup_ready);
        assert_eq!(session.epoch_qpc_for_test(), QpcTicks::ZERO);
        if skip {
            session.skip().expect("accept skip during calibration");
        } else {
            session.quit().expect("accept stop during calibration");
        }
        hook.release_startup_checkpoint();
        assert!(session.join(Duration::from_secs(5)).expect("worker join"));

        let snapshot = session.snapshot();
        assert_eq!(snapshot.status, expected_status.as_str());
        assert_eq!(snapshot.outcome.as_deref(), Some(expected_outcome));
        assert!(!snapshot.startup_armed);
        assert_eq!(session.epoch_qpc_for_test(), QpcTicks::ZERO);
        assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn arm_epoch_is_immutable_across_worker_boot_delay() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::BeforeReady, 1);
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
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(2)));
    assert_eq!(session.epoch_qpc_for_test(), QpcTicks::ZERO);
    assert_eq!(session.pre_roll_us_for_test(), 0);
    assert_eq!(session.poll_state().status, EnginePollStatus::Preparing);
    assert!(!session.snapshot().startup_ready);
    std::thread::sleep(Duration::from_millis(60));
    hook.release_startup_checkpoint();
    let ready_deadline = Instant::now() + Duration::from_secs(2);
    while session.epoch_qpc_for_test() == QpcTicks::ZERO {
        assert!(
            Instant::now() < ready_deadline,
            "worker never published Armed"
        );
        std::thread::yield_now();
    }
    let epoch_before_boot = session.epoch_qpc_for_test();
    assert_eq!(session.pre_roll_us_for_test(), 100_000);
    assert!(session.snapshot().startup_ready);

    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(session.epoch_qpc_for_test(), epoch_before_boot);
    assert_eq!(session.snapshot().terminal_error, None);
}

#[test]
fn suspend_at_epoch_prepared_barrier_prevents_armed_publication_and_down() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::EpochPrepared, 1);
    let send_call_count = Arc::new(AtomicU64::new(0));
    let send_input_capture = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_call_count));
    fault_script.send_input_capture = Some(Arc::clone(&send_input_capture));
    let mut options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    let session = NativeDispatchSession::new(options).expect("test session admission");

    session.arm(0).expect("arm worker");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(3)));
    assert!(!session.snapshot().startup_armed);
    assert!(session.notify_system_power(true));
    // Windows may deliver Resume before the worker leaves its pre-Armed
    // checkpoint. The monotonic session decision must survive that ordering.
    assert!(session.notify_system_power(false));
    hook.release_startup_checkpoint();

    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(
        session.poll_state().status,
        EnginePollStatus::SystemSuspended
    );
    assert_eq!(session.terminal_outcome(), Some("system_suspend"));
    assert!(!session.snapshot().startup_armed);
    assert_eq!(session.epoch_qpc_for_test(), QpcTicks::ZERO);
    assert_eq!(send_call_count.load(Ordering::SeqCst), 0);
    assert!(
        send_input_capture
            .lock()
            .expect("mock send capture")
            .is_empty()
    );
}

#[test]
fn slow_preparation_does_not_consume_published_preroll_or_shift_authored_onset() {
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
    options.profile = DispatchProfile::MockTest;
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    let session = NativeDispatchSession::new(options).expect("test session admission");

    session.arm(50_000).expect("arm session");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(snapshot.outcome.as_deref(), Some("finished"));
    assert_eq!(snapshot.terminal_error, None);
    assert!(snapshot.startup_ready);
    assert!(snapshot.startup_armed);
    assert!(snapshot.startup_latency_us.unwrap_or_default() > 48_000);
    let epoch = session.epoch_qpc_for_test();
    assert!(epoch > QpcTicks::ZERO);
    let (authored_target, _) = hook.last_down_window();
    assert_eq!(authored_target, epoch.as_u64());
    assert!(hook.last_down_sender_start_qpc_ticks() >= epoch.as_u64());
}

#[test]
fn accepted_quit_cannot_arm_from_preparation_ready_arming_or_epoch_staging() {
    let cases = [
        (StartupCheckpoint::StreamBuild, 1),
        (StartupCheckpoint::StreamBuild, 2),
        (StartupCheckpoint::BeforeReady, 1),
        (StartupCheckpoint::ReadyToArmed, 1),
        (StartupCheckpoint::ReadyToArmed, 2),
        (StartupCheckpoint::EpochPrepared, 1),
    ];
    for (checkpoint, occurrence) in cases {
        let (session, hook, send_calls) = checkpointed_session(
            large_startup_materialization_schedule(64),
            checkpoint,
            occurrence,
            false,
        );
        session.arm(100_000).expect("worker arm request");
        assert!(
            hook.wait_for_startup_checkpoint(Duration::from_secs(5)),
            "{checkpoint:?}"
        );
        session.quit().expect("accepted pre-arm quit");
        assert_checkpointed_terminal_stop(
            &session,
            &hook,
            &send_calls,
            EnginePollStatus::Quit,
            "quit",
        );
    }
}

#[test]
fn quit_and_skip_accepted_before_arm_remain_sticky_through_worker_startup() {
    for (skip, expected_status, expected_trigger) in [
        (false, EnginePollStatus::Quit, "quit"),
        (true, EnginePollStatus::Skipped, "skipped"),
    ] {
        let (session, _hook, send_calls) = checkpointed_session(
            startup_boundary_schedule(),
            StartupCheckpoint::ReadyToArmed,
            1,
            false,
        );
        let accepted = if skip { session.skip() } else { session.quit() };
        accepted.expect("pre-arm terminal request must be accepted");

        session
            .arm(100_000)
            .expect("cancelled session still retires");
        assert!(session.join(Duration::from_secs(5)).expect("worker join"));

        let snapshot = session.snapshot();
        assert_eq!(snapshot.status, expected_status.as_str());
        assert_eq!(snapshot.outcome.as_deref(), Some(expected_status.as_str()));
        assert_eq!(
            snapshot.terminal_trigger_reason.as_deref(),
            Some(expected_trigger)
        );
        assert!(!snapshot.startup_armed);
        assert_eq!(send_calls.load(Ordering::Acquire), 0);
    }
}

#[test]
fn skip_panic_suspend_target_owner_and_lease_stops_win_ready_to_armed() {
    let checkpoint = StartupCheckpoint::ReadyToArmed;
    let cases = [
        ("skip", EnginePollStatus::Skipped, "skipped"),
        ("panic", EnginePollStatus::Error, "panic_release_requested"),
        (
            "suspend",
            EnginePollStatus::SystemSuspended,
            "system_suspend",
        ),
        (
            "target",
            EnginePollStatus::Error,
            "target_authority_invalidated_during_preparation",
        ),
        (
            "owner",
            EnginePollStatus::Error,
            "owner_authority_invalidated_during_preparation",
        ),
        ("lease", EnginePollStatus::Error, "supervisor_lease_expired"),
    ];
    for (kind, expected_status, expected_trigger) in cases {
        let (session, hook, send_calls) =
            checkpointed_session(startup_boundary_schedule(), checkpoint, 2, false);
        session.arm(100_000).expect("worker arm request");
        assert!(
            hook.wait_for_startup_checkpoint(Duration::from_secs(5)),
            "{kind}"
        );
        match kind {
            "skip" => session.skip().expect("skip request"),
            "panic" => session.panic_release().expect("panic release request"),
            "suspend" => assert!(session.notify_system_power(true)),
            "target" => session.set_target_hwnd(456),
            "owner" => session.invalidate_window_identity_authority_for_test(),
            "lease" => {
                session
                    .shared_for_test()
                    .commands
                    .supervisor_expired
                    .latch_expired();
                assert!(session.publish_supervisor_progress().is_err());
            }
            _ => unreachable!(),
        }
        assert_checkpointed_terminal_stop(
            &session,
            &hook,
            &send_calls,
            expected_status,
            expected_trigger,
        );
    }
}

#[test]
fn focus_hint_invalidation_cancels_arming_before_epoch_visibility() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    let (session, hook, send_calls) = checkpointed_session(
        startup_boundary_schedule(),
        StartupCheckpoint::ReadyToArmed,
        2,
        true,
    );
    session.set_target_hwnd(123);
    session.set_focus_hint(true);
    session.arm(100_000).expect("worker arm request");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
    session.set_focus_hint(false);
    assert_checkpointed_terminal_stop(
        &session,
        &hook,
        &send_calls,
        EnginePollStatus::Error,
        "focus_authority_invalidated_during_preparation",
    );
}

#[test]
fn quit_after_armed_publication_wait_and_first_down_gate_never_sends() {
    for checkpoint in [
        StartupCheckpoint::ArmedPublished,
        StartupCheckpoint::InitialWait,
        StartupCheckpoint::FirstDownAdmission,
    ] {
        let (session, hook, send_calls) =
            checkpointed_session(startup_boundary_schedule(), checkpoint, 1, false);
        session.arm(50_000).expect("worker arm request");
        assert!(
            hook.wait_for_startup_checkpoint(Duration::from_secs(5)),
            "{checkpoint:?}"
        );
        let published_epoch = session.epoch_qpc_for_test();
        assert!(published_epoch > QpcTicks::ZERO);
        assert!(session.snapshot().startup_ready);
        assert!(session.snapshot().startup_armed);
        session.quit().expect("quit after Armed publication");
        hook.release_startup_checkpoint();
        assert!(session.join(Duration::from_secs(5)).expect("post-arm join"));
        let snapshot = session.snapshot();
        assert_eq!(snapshot.status, EnginePollStatus::Quit.as_str());
        assert_eq!(snapshot.outcome.as_deref(), Some("quit"));
        assert_eq!(snapshot.terminal_trigger_reason.as_deref(), Some("quit"));
        assert_eq!(send_calls.load(Ordering::Acquire), 0, "{checkpoint:?}");
        assert_eq!(session.epoch_qpc_for_test(), published_epoch);
    }
}

#[test]
fn focus_loss_at_first_down_admission_never_sends_physical_down() {
    let (session, hook, send_calls) = checkpointed_session(
        startup_boundary_schedule(),
        StartupCheckpoint::FirstDownAdmission,
        1,
        false,
    );
    session.arm(50_000).expect("worker arm request");
    assert!(
        hook.wait_for_startup_checkpoint(Duration::from_secs(5)),
        "reached FirstDownAdmission"
    );
    // Simulate focus loss immediately before SendInput
    session.set_focus_hint(false);
    hook.release_startup_checkpoint();
    // Cleanly tear down session
    let _ = session.quit();
    let _ = session.join(Duration::from_secs(2));
    assert_eq!(
        send_calls.load(Ordering::Acquire),
        0,
        "zero physical down sender attempts after focus loss at final gate"
    );
}

#[test]
fn startup_panic_is_contained_before_armed_and_produces_error_not_finished() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.panic_at_startup_checkpoint(StartupCheckpoint::BeforeReady);
    let send_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
    let mut options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.startup_ordering_hook = Some(hook);
    let session = NativeDispatchSession::new(options).expect("panic test session admission");
    session.arm(100_000).expect("panic test arm request");
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("panic worker join")
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Panicked.as_str());
    assert_eq!(snapshot.outcome.as_deref(), Some("error"));
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some("worker_panic")
    );
    assert!(!snapshot.startup_ready);
    assert!(!snapshot.startup_armed);
    assert_eq!(session.epoch_qpc_for_test(), QpcTicks::ZERO);
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
}

#[test]
fn worker_and_watchdog_spawn_failures_publish_error_not_finished() {
    for fail_watchdog in [false, true] {
        let hook = Arc::new(StartupOrderingHook::default());
        if fail_watchdog {
            hook.fail_next_watchdog_spawn();
        } else {
            hook.fail_next_worker_spawn();
        }
        let send_calls = Arc::new(AtomicU64::new(0));
        let mut fault_script = FaultInjectionScript::none();
        fault_script.send_call_count = Some(Arc::clone(&send_calls));
        let mut options = test_session_options(
            startup_boundary_schedule(),
            1,
            BackendConfig::Mock {
                latency_base_us: 0,
                latency_per_key_us: 0,
                fault_script,
            },
        );
        options.startup_ordering_hook = Some(Arc::clone(&hook));
        if fail_watchdog {
            options.wait.supervisor_lease_timeout_us = 100_000;
        }
        let session = NativeDispatchSession::new(options).expect("spawn-failure session");
        assert!(session.arm(50_000).is_err());
        assert!(
            session
                .join(Duration::from_secs(2))
                .expect("spawn-failure join")
        );
        let snapshot = session.snapshot();
        assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
        assert_eq!(snapshot.outcome.as_deref(), Some("error"));
        assert_eq!(
            snapshot.terminal_trigger_reason.as_deref(),
            Some(if fail_watchdog {
                "startup_watchdog_spawn_failure"
            } else {
                "startup_worker_spawn_failure"
            })
        );
        assert!(!snapshot.startup_ready);
        assert_eq!(send_calls.load(Ordering::Acquire), 0);
        assert_eq!(session.spawn_handles_present_for_test(), (false, false));
    }
}

#[test]
fn observer_creation_failure_runs_admission_cleanup_without_ready_or_input() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.fail_next_observer_spawn();
    let send_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
    let mut options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.profile = DispatchProfile::MockTest;
    options.startup_ordering_hook = Some(hook);
    let session = NativeDispatchSession::new(options).expect("observer failure session");
    session.arm(50_000).expect("worker arm request");
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("observer failure join")
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(snapshot.outcome.as_deref(), Some("error"));
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some("startup_failure")
    );
    assert!(!snapshot.startup_ready);
    assert_eq!(session.epoch_qpc_for_test(), QpcTicks::ZERO);
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
}

#[test]
fn startup_epoch_qpc_conversion_failure_stays_unarmed_and_preserves_cleanup() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.fail_next_startup_epoch_conversion();
    let send_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
    let mut options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.startup_ordering_hook = Some(hook);
    let session = NativeDispatchSession::new(options).expect("QPC conversion session");
    session.arm(100_000).expect("worker startup request");
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("conversion failure join")
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(snapshot.outcome.as_deref(), Some("error"));
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some("startup_failure")
    );
    assert!(snapshot.startup_ready);
    assert!(!snapshot.startup_armed);
    assert_eq!(session.epoch_qpc_for_test(), QpcTicks::ZERO);
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
    let release = snapshot.release_outcome.expect("admission cleanup outcome");
    assert!(release.released_successfully);
    assert_eq!(snapshot.release_attempts_total, u64::from(release.attempts));
}

#[test]
fn unrepresentable_preroll_qpc_conversion_is_rejected_before_worker_start() {
    let session = NativeDispatchSession::new(test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    ))
    .expect("QPC conversion session admission");
    let error = session
        .arm(u64::MAX)
        .expect_err("unrepresentable QPC duration");
    assert!(error.contains("pre-roll conversion failed"));
    assert_eq!(session.epoch_qpc_for_test(), QpcTicks::ZERO);
    assert_eq!(session.snapshot().status, EnginePollStatus::Ready.as_str());
    assert_eq!(session.terminal_outcome(), None);
}

#[test]
fn focus_loss_during_post_wait_hook_never_sends_physical_down() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownAdmission, 1);
    let send_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));

    let mut options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.profile = DispatchProfile::MockTest;
    options.focus.require_focus = true;
    options.focus.focus_restore_grace_us = 0;
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    let session = Arc::new(NativeDispatchSession::new(options).expect("session created"));
    let session_for_hook = Arc::clone(&session);

    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    session.set_target_hwnd(123);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);
    session.arm(0).expect("session arm");

    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(1)));
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(456));
    session_for_hook.set_focus_hint(false);
    hook.release_startup_checkpoint();

    let _ = session.quit();
    let _ = session.join(Duration::from_millis(500));
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    assert_eq!(
        send_calls.load(Ordering::Acquire),
        0,
        "physical send must NEVER be called when focus is lost during the post-wait hook"
    );
}
