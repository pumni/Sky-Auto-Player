use super::telemetry::metrics::RecentLatencyRing;
use super::test_support::command_timing::{
    CommandTimingError, CommandTimingLookup as PauseTimingLookup, PauseTimingPhase,
};
use super::{
    BackendConfig, CommandTimingResult, CommandTimingState, DispatchPath, DownAdmission,
    EstimatorOptions, FaultInjectionScript, FocusOptions, HealthWindow, HealthWindowPolicy,
    InjectedSendOutcome, NativeDispatchSession, NativeSessionOptions, PlatformSendResult,
    PriorityOptions, RELEASE_RETRY_BACKOFF_US, RtTraceRecord, SharedMetrics, TRACE_FLAG_SENT_FULL,
    TRACE_KIND_DOWN, TargetStamp, TelemetryCollector, TelemetryMode, TelemetryOptions,
    TimingOptions, TraceContext, TraceDelivery, TraceTiming, TrackedKeyState, WaitOptions,
    WakeErrorStats, Worker, WorkerMetricsLocal, adjust_spin_threshold,
    anchored_dispatch_target_ticks, classify_latency_class, cpu_metrics_sample_due,
    deadline_target_ticks, derive_spin_threshold_us, ensure_preflight_for_target,
    estimator_kind_for_path, exact_sender_durations, final_down_admission, focus_gate_matches,
    focus_matches_hwnd, record_input_path_health, record_termination_error,
    release_runtime_outcome, signed_timeline_delta_ticks, supervisor_lease_expired,
    target_stamp_still_current, trace_outcome_code, try_publish_metrics,
    update_estimator_after_send, wake_lateness_ticks,
};
use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::TimelineTicks;
use sky_dispatch_win32::clock::{
    DurationTicks, QpcClock, QpcTicks, qpc_frequency, qpc_ticks_to_us, qpc_us_to_ticks,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::time::Duration;

fn test_session_options(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    allowed_count: usize,
    backend: BackendConfig,
) -> NativeSessionOptions {
    NativeSessionOptions {
        schedule,
        backend,
        allowed_count,
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
            core_warmup_budget_us: 0,
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
            enable_adaptive_lead: false,
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
fn native_telemetry_records_are_complete_beyond_303_events() {
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
    assert_eq!(telemetry["attempted"], actions.len());
    assert_eq!(telemetry["accepted"], actions.len());
    assert_eq!(telemetry["dropped"], 0);
    assert_eq!(telemetry["truncated"], false);
    let records = telemetry["records"].as_array().expect("records array");
    assert_eq!(records.len(), actions.len());
    let indices: Vec<u64> = records
        .iter()
        .map(|record| record["event_index"].as_u64().expect("event index"))
        .collect();
    assert_eq!(indices, (0..actions.len() as u64).collect::<Vec<_>>());
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
                bookkeeping_duration_us: 0,
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
fn final_down_admission_rejects_target_change_before_send() {
    let target = AtomicIsize::new(456);
    let generation = AtomicU64::new(2);
    let focus_active = AtomicBool::new(false);
    let quit_requested = AtomicBool::new(false);
    let skip_requested = AtomicBool::new(false);
    let panic_requested = AtomicBool::new(false);
    let desired_pause = AtomicBool::new(false);
    let expected = TargetStamp {
        hwnd: 123,
        generation: 1,
    };

    assert_eq!(
        final_down_admission(
            expected,
            false,
            &focus_active,
            &target,
            &generation,
            &quit_requested,
            &skip_requested,
            &panic_requested,
            &desired_pause,
        ),
        DownAdmission::TargetChanged
    );
}

#[test]
fn final_down_admission_checks_expected_focus_before_target() {
    let target = AtomicIsize::new(456);
    let generation = AtomicU64::new(2);
    let focus_active = AtomicBool::new(false);
    let quit_requested = AtomicBool::new(false);
    let skip_requested = AtomicBool::new(false);
    let panic_requested = AtomicBool::new(false);
    let desired_pause = AtomicBool::new(false);
    let expected = TargetStamp {
        hwnd: 0,
        generation: 1,
    };

    assert_eq!(
        final_down_admission(
            expected,
            true,
            &focus_active,
            &target,
            &generation,
            &quit_requested,
            &skip_requested,
            &panic_requested,
            &desired_pause,
        ),
        DownAdmission::FocusLost
    );
}

#[test]
fn final_down_admission_rejects_each_command_state() {
    let target = AtomicIsize::new(123);
    let generation = AtomicU64::new(1);
    let focus_active = AtomicBool::new(false);
    let quit_requested = AtomicBool::new(false);
    let skip_requested = AtomicBool::new(false);
    let panic_requested = AtomicBool::new(false);
    let desired_pause = AtomicBool::new(false);
    let expected = TargetStamp {
        hwnd: 123,
        generation: 1,
    };

    let admission = || {
        final_down_admission(
            expected,
            false,
            &focus_active,
            &target,
            &generation,
            &quit_requested,
            &skip_requested,
            &panic_requested,
            &desired_pause,
        )
    };

    assert_eq!(admission(), DownAdmission::Allowed);
    quit_requested.store(true, Ordering::Release);
    assert_eq!(admission(), DownAdmission::QuitRequested);
    quit_requested.store(false, Ordering::Release);
    skip_requested.store(true, Ordering::Release);
    assert_eq!(admission(), DownAdmission::SkipRequested);
    skip_requested.store(false, Ordering::Release);
    panic_requested.store(true, Ordering::Release);
    assert_eq!(admission(), DownAdmission::PanicRequested);
    panic_requested.store(false, Ordering::Release);
    desired_pause.store(true, Ordering::Release);
    assert_eq!(admission(), DownAdmission::PauseRequested);
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
            bookkeeping_duration_us: 4,
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
    assert_eq!(record.bookkeeping_duration_us, 4);

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
            bookkeeping_duration_us: 0,
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
            bookkeeping_duration_us: 0,
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
fn cold_classification_uses_physical_gap_after_logical_pause() {
    let last = QpcTicks::from_raw(qpc_us_to_ticks(100_000).unwrap());
    let threshold =
        DurationTicks::from_raw(qpc_us_to_ticks(super::SEND_COLD_THRESHOLD_US).unwrap());
    let hot_now = last
        .checked_add_duration(threshold)
        .expect("test timestamp");
    let cold_now = QpcTicks::from_raw(hot_now.as_u64() + 1);
    assert_eq!(
        classify_latency_class(Some(last), hot_now, threshold).unwrap(),
        LatencyClass::Hot
    );
    assert_eq!(
        classify_latency_class(Some(last), cold_now, threshold).unwrap(),
        LatencyClass::Cold
    );
    assert_eq!(
        classify_latency_class(None, QpcTicks::ZERO, threshold).unwrap(),
        LatencyClass::Cold
    );
    assert!(classify_latency_class(Some(cold_now), last, threshold).is_err());
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
fn failed_send_does_not_seed_estimator_or_residual() {
    let mut estimator = SendLatencyEstimator::try_new(0.2, 2_000, 6).unwrap();

    update_estimator_after_send(&mut estimator, ActionKind::Down, 900, 0, 3, 500, 120, false);
    let state = estimator.export_state();
    assert_eq!(state.hist_down[3].hot_pairs, Vec::<[u64; 2]>::new());
    assert_eq!(state.residuals[0].count, 0);

    update_estimator_after_send(&mut estimator, ActionKind::Down, 900, 1, 3, 0, 120, false);
    let state = estimator.export_state();
    assert_eq!(state.hist_down[3].hot_pairs, Vec::<[u64; 2]>::new());
    assert_eq!(state.residuals[0].count, 0);

    update_estimator_after_send(&mut estimator, ActionKind::Down, 900, 1, 3, 500, 120, true);
    let state = estimator.export_state();
    assert_eq!(state.hist_down[3].hot_pairs, vec![[36, 1]]);
    assert_eq!(estimator.export_state().residuals[0].count, 1);
}

#[test]
fn directional_estimator_training_is_not_cross_contaminated() {
    let mut estimator = SendLatencyEstimator::try_new(0.2, 2_000, 6).unwrap();

    update_estimator_after_send(&mut estimator, ActionKind::Up, 900, 2, 2, 500, 120, true);
    let after_up = estimator.export_state();
    assert_eq!(after_up.hist_up[2].hot_pairs, vec![[36, 1]]);
    assert_eq!(after_up.hist_down[2].hot_pairs, Vec::<[u64; 2]>::new());

    // Mixed packets deliberately have no estimator kind in this phase.
    let before_mixed = estimator.export_state();
    if let Some(kind) = estimator_kind_for_path(DispatchPath::Mixed {
        up_count: 2,
        down_count: 2,
    }) {
        update_estimator_after_send(&mut estimator, kind, 900, 2, 2, 500, 120, true);
    }
    let after_mixed = estimator.export_state();
    assert_eq!(
        serde_json::to_string(&after_mixed.hist_up).unwrap(),
        serde_json::to_string(&before_mixed.hist_up).unwrap()
    );
    assert_eq!(
        serde_json::to_string(&after_mixed.hist_down).unwrap(),
        serde_json::to_string(&before_mixed.hist_down).unwrap()
    );
}

#[test]
fn mixed_path_estimator_trains_on_mixed_observations_only() {
    let mut estimator = SendLatencyEstimator::try_new(0.2, 2_000, 6).unwrap();
    let before = estimator.export_state();

    update_estimator_after_send(&mut estimator, ActionKind::Up, 900, 2, 2, 500, 120, true);
    let after_up = estimator.export_state();
    assert_ne!(
        serde_json::to_string(&after_up.hist_up).unwrap(),
        serde_json::to_string(&before.hist_up).unwrap()
    );
    assert_eq!(
        serde_json::to_string(&after_up.hist_down).unwrap(),
        serde_json::to_string(&before.hist_down).unwrap()
    );

    update_estimator_after_send(&mut estimator, ActionKind::Down, 900, 2, 2, 500, 120, true);
    let after_down = estimator.export_state();
    assert_ne!(
        serde_json::to_string(&after_down.hist_down).unwrap(),
        serde_json::to_string(&after_up.hist_down).unwrap()
    );
}

#[test]
fn estimator_v9_predicts_path_specific_leads() {
    let estimator = SendLatencyEstimator::try_new(0.2, 2_000, 6).unwrap();
    let down_lead = estimator.estimate_lead(ActionKind::Down, 2).applied_us;
    let up_lead = estimator.estimate_lead(ActionKind::Up, 2).applied_us;
    assert!(down_lead > 0);
    assert!(up_lead > 0);
}

#[test]
fn architecture_layers_strict_boundary_enforced() {
    let estimator = SendLatencyEstimator::try_new(0.2, 2_000, 6).unwrap();
    assert_eq!(estimator.export_state().max_poly, 6);
}

#[test]
fn module_line_limits_strictly_respected() {
    let dispatch = [
        ("worker/dispatch/mod.rs", 250),
        ("worker/dispatch/authored.rs", 700),
        ("worker/dispatch/observer.rs", 700),
        ("worker/dispatch/release.rs", 700),
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
    let estimator = SendLatencyEstimator::try_new(0.2, 2_000, 1).expect("estimator");
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
        path: crate::engine::worker::DispatchPath::UpOnly { up_count: 1 },
        send_warn_us: 2000,
        bookkeeping_warn_us: 1000,
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

    assert_eq!(local_metrics.bookkeeping_degraded_samples, 0);
}

#[test]
fn up_estimator_receives_exact_syscall_duration_sample() {
    use super::worker::update_estimator_after_send_class;
    let mut estimator = SendLatencyEstimator::try_new(0.2, 2_000, 6).unwrap();

    // Up sample of 425us should register in estimator for Up (index 0)
    update_estimator_after_send_class(
        &mut estimator,
        ActionKind::Up,
        425,
        1,
        1,
        500,
        100,
        true,
        LatencyClass::Hot,
    )
    .unwrap();

    let state = estimator.export_state();
    assert_eq!(state.hist_up[1].hot_pairs, vec![[17, 1]]);
    assert_eq!(state.hist_down[1].hot_pairs, Vec::<[u64; 2]>::new());
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
