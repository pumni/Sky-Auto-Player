use super::support::*;
use super::*;

#[test]
fn engine_snapshots_project_always_on_timing_with_trace_on_and_off() {
    for telemetry_mode in [TelemetryMode::Ring, TelemetryMode::Off] {
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
        options.telemetry.mode = telemetry_mode;
        let session = NativeDispatchSession::new(options).expect("production-normal test session");
        session.arm(0).expect("zero-count-in arm");

        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.poll_state().status.is_terminal() {
            assert!(Instant::now() < deadline, "prepared session did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(session.join(Duration::from_secs(5)).expect("worker join"));

        let lite = session.snapshot_lite();
        let full = session.snapshot();
        assert_eq!(lite.precision_packet_attempts, 2);
        assert_eq!(full.precision_packet_attempts, 2);
        assert_eq!(lite.authored_lateness_samples, 2);
        assert_eq!(full.authored_lateness_samples, 2);
        assert_eq!(lite.physical_floor_displacement_samples, 2);
        assert_eq!(full.physical_floor_displacement_samples, 2);
        assert_eq!(lite.sendinput_envelope_samples, 2);
        assert_eq!(full.sendinput_envelope_samples, 2);
        assert_eq!(full.pre_call_qpc_unavailable_attempts, 0);
        assert_eq!(full.sendinput_completion_qpc_unavailable_attempts, 0);

        let telemetry: serde_json::Value =
            serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
                .expect("valid telemetry JSON");
        let records = telemetry["records"].as_array().expect("trace records");
        if telemetry_mode == TelemetryMode::Ring {
            assert_eq!(records.len(), 2);
            assert_eq!(records[0]["down_mask"].as_u64(), Some(1));
            assert_eq!(records[0]["up_mask"].as_u64(), Some(0));
            assert_eq!(records[1]["down_mask"].as_u64(), Some(0));
            assert_eq!(records[1]["up_mask"].as_u64(), Some(1));
            assert!(
                records
                    .iter()
                    .all(|record| record["send_attempts"].as_u64() == Some(1))
            );
        } else {
            assert!(records.is_empty());
        }
    }
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
        session
            .publish_supervisor_progress()
            .expect("supervisor heartbeat");
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
    let send_calls = Arc::new(AtomicU64::new(0));
    let full_release_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.wait_failure = true;
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_calls));
    let session = NativeDispatchSession::new(test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    ))
    .expect("test session admission");
    session.start().expect("worker start");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert!(!session.snapshot().startup_ready);
    assert_eq!(session.snapshot().startup_latency_us, None);
    assert_eq!(session.snapshot().status, EnginePollStatus::Error.as_str());
    assert_eq!(session.snapshot().outcome.as_deref(), Some("error"));
    assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    assert_eq!(full_release_calls.load(Ordering::SeqCst), 0);
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
    assert_eq!(session.snapshot().status, EnginePollStatus::Error.as_str());
    assert_eq!(session.snapshot().outcome.as_deref(), Some("error"));
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
    let worker_options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    let admitted_options = super::config::AdmittedNativeSessionOptions {
        options: worker_options,
        instrument_key_profile:
            sky_dispatch_win32::input::MaterializedInstrumentKeyProfile::canonical(),
    };
    let mut worker = Worker::new(admitted_options, session.shared_for_test());

    assert!(worker.take_schedule_for_test().is_ok());
    assert!(matches!(
        worker.take_schedule_for_test(),
        Err("worker runtime schedule was already consumed")
    ));
}

#[test]
fn invalid_instrument_profile_is_rejected_before_worker_start() {
    let mut options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    let mut spec = InstrumentKeyProfileSpec::canonical();
    spec.keys[0] = PhysicalKey {
        scan_code: 0x3a,
        extended: false,
    };
    options.instrument_key_profile = Some(spec);
    let error = NativeDispatchSession::new(options)
        .err()
        .expect("invalid profile must fail at admission");
    assert!(error.contains("native instrument profile admission failed"));
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
    let snapshot = session.snapshot();
    assert_eq!(snapshot.outcome.as_deref(), Some("quit"));
    assert_eq!(snapshot.terminal_trigger_reason.as_deref(), Some("quit"));
    let progress = session.snapshot_lite();
    assert_eq!(progress.terminal_outcome, Some(EnginePollStatus::Quit));
    assert!(!progress.has_terminal_error);
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
                trace_record_index: 0,
                compiled_packet_index: 0,
                compiled_packet_index_available: false,
                source_action_index: 0,
                event_index: 0,
                kind: TRACE_KIND_DOWN,
                outcome: 0,
                polyphony: 1,
                flags: TRACE_FLAG_SENT_FULL,
                send_status: 0,
                up_mask: 0,
                down_mask: 1,
                requested_count: 1,
                sent_count: 1,
                skipped_count: 0,
                send_attempts: 1,
                ..RtTraceRecord::default()
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
fn native_trace_constructor_rejects_inconsistent_counts() {
    let result = RtTraceRecord::dispatched(
        TraceContext {
            event_index: 0,
            source_action_index: 0,
            compiled_packet_index: None,
            kind: TRACE_KIND_DOWN,
            outcome: trace_outcome_code("sent"),
            polyphony: 1,
            flags: 0,
            send_status: 0,
            win32_error: 0,
            up_mask: 0,
            down_mask: 1,
        },
        TraceTiming {
            authored_ticks: TimelineTicks::ZERO,
            effective_deadline_ticks: TimelineTicks::ZERO,
            wake_ticks: None,
            physical_target_qpc_ticks: None,
            physical_not_before_qpc_ticks: None,
            hold_floor_qpc_ticks: None,
            release_floor_qpc_ticks: None,
            hold_floor_mask: 0,
            release_floor_mask: 0,
            pre_call_qpc_ticks: None,
            sendinput_completion_qpc_ticks: None,
            observation_qpc_ticks: None,
            pre_call_ticks: None,
            sendinput_completion_ticks: None,
            completion_residual_us: 0,
            core_post_send_duration_us: 0,
            post_send_metrics_available: false,
            completion_error_ticks: 0,
            authored_completion_error_ticks: 0,
            ..TraceTiming::default()
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
            source_action_index: 0,
            compiled_packet_index: None,
            kind: TRACE_KIND_DOWN,
            outcome: trace_outcome_code("blocked_unfocused"),
            polyphony: 3,
            flags: 0,
            send_status: 8,
            win32_error: 0,
            up_mask: 0,
            down_mask: 0b111,
        },
        TraceTiming {
            authored_ticks: TimelineTicks::ZERO,
            effective_deadline_ticks: TimelineTicks::ZERO,
            wake_ticks: None,
            physical_target_qpc_ticks: None,
            physical_not_before_qpc_ticks: None,
            hold_floor_qpc_ticks: None,
            release_floor_qpc_ticks: None,
            hold_floor_mask: 0,
            release_floor_mask: 0,
            pre_call_qpc_ticks: None,
            sendinput_completion_qpc_ticks: None,
            observation_qpc_ticks: None,
            pre_call_ticks: None,
            sendinput_completion_ticks: None,
            completion_residual_us: 0,
            core_post_send_duration_us: 0,
            post_send_metrics_available: false,
            completion_error_ticks: 0,
            authored_completion_error_ticks: 0,
            ..TraceTiming::default()
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
    assert!(
        snapshot
            .terminal_error
            .as_deref()
            .is_some_and(|error| !error.starts_with("clean completion contract failed:")),
        "transport fault identity must survive finalization: {snapshot:?}"
    );
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
fn mixed_disjoint_packet_telemetry_preserves_two_events() {
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
    let mut options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    // This test asserts mixed-packet telemetry cardinality, not the production
    // 500-us deadline policy. Keep the real worker/QPC path, but give the
    // Windows test runner enough scheduling margin that host preemption does
    // not turn the telemetry assertion into an unrelated startup deadline
    // failure. Production configuration remains unchanged.
    set_test_timing_margin(&mut options, 20_000);
    let session = NativeDispatchSession::new(options).expect("test session admission");

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
        .expect("successful disjoint mixed record");
    assert_eq!(mixed["requested_count"].as_u64(), Some(2));
    assert_eq!(mixed["sent_count"].as_u64(), Some(2));
    assert_eq!(mixed["polyphony"].as_u64(), Some(2));
}
