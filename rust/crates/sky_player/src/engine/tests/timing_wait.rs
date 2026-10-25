use super::support::*;
use super::*;

#[test]
fn native_timing_contract_rejects_mismatched_hold_and_release_floors() {
    let schedule = startup_boundary_schedule();
    let backend = || BackendConfig::Mock {
        latency_base_us: 0,
        latency_per_key_us: 0,
        fault_script: FaultInjectionScript::none(),
    };
    let mut hold_mismatch = test_session_options(schedule.clone(), 1, backend());
    hold_mismatch.timing.min_hold_us += 1;
    let hold_error = NativeDispatchSession::new(hold_mismatch)
        .err()
        .expect("native admission must reject a mismatched hold floor");
    assert!(hold_error.contains("min_hold_us"), "{hold_error}");

    let mut release_mismatch = test_session_options(schedule, 1, backend());
    release_mismatch.timing.min_release_gap_us += 1;
    let release_error = NativeDispatchSession::new(release_mismatch)
        .err()
        .expect("native admission must reject a mismatched release floor");
    assert!(
        release_error.contains("min_release_gap_us"),
        "{release_error}"
    );
}

#[test]
fn native_timing_contract_rejects_checked_add_overflow() {
    let backend = || BackendConfig::Mock {
        latency_base_us: 0,
        latency_per_key_us: 0,
        fault_script: FaultInjectionScript::none(),
    };
    let mut hold_overflow = test_session_options(startup_boundary_schedule(), 1, backend());
    hold_overflow.timing.frame_base_hold_us = u64::MAX;
    hold_overflow.timing.min_hold_us = u64::MAX;
    let hold_error = NativeDispatchSession::new(hold_overflow)
        .err()
        .expect("native admission must reject hold equation overflow");
    assert!(hold_error.contains("frame_base_hold_us + timing_margin_us"));

    let mut release_overflow = test_session_options(startup_boundary_schedule(), 1, backend());
    release_overflow.timing.frame_us = u64::MAX;
    release_overflow.timing.min_release_gap_us = u64::MAX;
    let release_error = NativeDispatchSession::new(release_overflow)
        .err()
        .expect("native admission must reject release equation overflow");
    assert!(release_error.contains("frame_us + timing_margin_us"));
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
            scheduled_us: 101,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "short-hold-chord".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 200,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "short-hold-chord-up".to_string().into(),
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
    options.timing.frame_base_hold_us = 300;
    set_test_timing_margin(&mut options, 0);
    let result = NativeDispatchSession::new(options);
    assert!(matches!(
        result,
        Err(error) if error.contains("native schedule admission failed")
    ));
}

#[test]
fn native_session_admission_rejects_positive_short_release_gap_before_worker_start() {
    fn schedule(next_down_us: u64) -> sky_dispatch_core::model::RuntimeSchedule {
        sky_dispatch_core::compile::compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 1_000,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "down-a".to_string().into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 2_000,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "up-a".to_string().into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Down,
                    scheduled_us: next_down_us,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "down-b".to_string().into(),
                },
                KeyActionInput {
                    source_action_index: 3,
                    kind: ActionKind::Up,
                    scheduled_us: next_down_us + 1_000,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "up-b".to_string().into(),
                },
            ],
            &[0x15],
        )
        .expect("positive-gap schedule must compile")
    }

    let backend = || BackendConfig::Mock {
        latency_base_us: 0,
        latency_per_key_us: 0,
        fault_script: FaultInjectionScript::none(),
    };
    let short_schedule = schedule(2_000 + 17_166);
    let short_result =
        NativeDispatchSession::new(test_session_options(short_schedule, 1, backend()));
    assert!(
        short_result
            .as_ref()
            .is_err_and(|error| error.contains("release gap")),
        "positive but too-short release gap must be rejected before worker start"
    );

    let legal_schedule = schedule(2_000 + 17_167);
    let legal_result =
        NativeDispatchSession::new(test_session_options(legal_schedule, 1, backend()));
    assert!(
        legal_result.is_ok(),
        "release gap at the configured boundary must be admitted"
    );
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
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 16_867,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up-b".to_string().into(),
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
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 300,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up-b".to_string().into(),
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
    // Keep worker startup/preemption away from the first authored boundary.
    // This test validates completion-latency/release-gap behavior, not
    // zero-slack startup scheduling.
    const TEST_AUTHORED_EPOCH_US: u64 = 2_000_000;
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: TEST_AUTHORED_EPOCH_US,
            scan_codes: smallvec::smallvec![0x15],
            reason: "seed-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: TEST_AUTHORED_EPOCH_US + 1_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "same-key-release".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: TEST_AUTHORED_EPOCH_US + 1_000 + 17_167,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "same-key-chord-after-release-gap".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: TEST_AUTHORED_EPOCH_US + 2_000 + 17_167,
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
    set_test_timing_margin(&mut options, 500);
    let session = NativeDispatchSession::new(options).expect("authored-feasible admission");

    start_with_test_wall_clock_slack(&session);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, "finished", "{snapshot:?}");
    assert_eq!(snapshot.sendinput_partial_events, 0);
    assert_eq!(snapshot.sendinput_zero_progress_failures, 0);
    assert_eq!(snapshot.chord_integrity_lost, 0);
    assert_eq!(snapshot.active_count, 0);
    assert_eq!(snapshot.possibly_active_count, 0);
    assert!(snapshot.terminal_error.is_none(), "{snapshot:?}");
    assert!(snapshot.production_hold_pair_samples >= 1, "{snapshot:?}");

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
fn strict_late_authorized_down_is_attempted_and_finishes() {
    const TEST_AUTHORED_START_US: u64 = 50_000;
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: TEST_AUTHORED_START_US,
            scan_codes: smallvec::smallvec![0x15],
            reason: "A-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Down,
            scheduled_us: TEST_AUTHORED_START_US + 100_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "B-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: TEST_AUTHORED_START_US + 200_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "A-up".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: TEST_AUTHORED_START_US + 200_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "B-up".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15, 0x16])
        .expect("valid physical-window recovery schedule");
    let mut options = test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.profile = DispatchProfile::Production;
    options.timing.strict_timing = true;
    set_test_timing_margin(&mut options, 20_000);
    let session = NativeDispatchSession::new(options).expect("test session admission");

    start_with_test_wall_clock_slack(&session);
    assert!(session.join(Duration::from_secs(8)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.outcome,
        Some("finished".to_string()),
        "{snapshot:?}"
    );
    assert_eq!(snapshot.status, "finished", "{snapshot:?}");
    assert!(snapshot.terminal_error.is_none(), "{snapshot:?}");
    assert_eq!(snapshot.authored_keys_rejected, 0, "{snapshot:?}");
    assert_eq!(
        snapshot.missed_physical_window_boundaries, 0,
        "{snapshot:?}"
    );
    assert_eq!(snapshot.final_sender_window_expirations, 0, "{snapshot:?}");
    assert_eq!(
        snapshot.release_floor_infeasible_boundaries, 0,
        "{snapshot:?}"
    );
    let progress = session.snapshot_lite();
    assert_eq!(progress.missed_physical_window_boundaries, 0);
    assert_eq!(progress.final_sender_window_expirations, 0);
    assert_eq!(progress.release_floor_infeasible_boundaries, 0);
    assert_eq!(snapshot.active_count, 0, "{snapshot:?}");
    assert_eq!(snapshot.possibly_active_count, 0, "{snapshot:?}");
    assert_eq!(snapshot.failed_release_count, 0, "{snapshot:?}");
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
        startup_ordering_hook: None,
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
        prepared_stream: None,
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
fn worker_scheduling_guards_drop_on_the_worker_thread_that_acquired_them() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::InitialWait, 1);
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
    let session = NativeDispatchSession::new(options).expect("scheduling guard session");
    session.arm(200_000).expect("worker startup request");

    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
    session.quit().expect("quit at initial wait");
    hook.release_startup_checkpoint();
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("worker finalization join")
    );

    let (acquired, dropped) = hook
        .scheduling_guard_threads()
        .expect("worker scheduling guard acquisition and drop records");
    assert_eq!(acquired, dropped);
    assert_eq!(session.terminal_outcome(), Some("quit"));
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
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 60_000,
                scan_codes: smallvec::smallvec![0x15, 0x16, 0x17],
                reason: "chord-cleanup".to_string().into(),
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

    let first_frame = coordinator
        .prepare_current_authored_frame()
        .unwrap()
        .unwrap();
    assert_eq!(first_frame.authored_ticks, TimelineTicks::from_raw(1_000));
    let first_commit = coordinator.prepare_authored_commit(first_frame).unwrap();
    coordinator
        .commit_prepared_authored_frame_success_frozen(
            &first_commit,
            TimelineTicks::from_raw(11_000),
            TimelineTicks::from_raw(11_050),
        )
        .unwrap();

    let second_frame = coordinator
        .prepare_current_authored_frame()
        .unwrap()
        .unwrap();
    assert_eq!(second_frame.authored_ticks, TimelineTicks::from_raw(20_000));
    let second_commit = coordinator.prepare_authored_commit(second_frame).unwrap();
    coordinator
        .commit_prepared_authored_frame_success_frozen(
            &second_commit,
            TimelineTicks::from_raw(20_000),
            TimelineTicks::from_raw(20_050),
        )
        .unwrap();

    let third_frame = coordinator
        .prepare_current_authored_frame()
        .unwrap()
        .unwrap();
    assert_eq!(third_frame.authored_ticks, TimelineTicks::from_raw(50_000));

    let delta_second_first =
        second_frame.authored_ticks.as_u64() - first_frame.authored_ticks.as_u64();
    let delta_third_second =
        third_frame.authored_ticks.as_u64() - second_frame.authored_ticks.as_u64();
    assert_eq!(delta_second_first, 19_000);
    assert_eq!(delta_third_second, 30_000);
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
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 40_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "up B".to_string().into(),
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
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 30_000,
                scan_codes: smallvec::smallvec![0x15, 0x16],
                reason: "chord2-up".to_string().into(),
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
        session.publish_supervisor_progress().expect("heartbeat");
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
    let authored = b_down["authored_offset_ticks"].as_u64().expect("authored");
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
        session.publish_supervisor_progress().expect("heartbeat");
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
        b_down["authored_offset_ticks"].as_u64().expect("authored"),
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

    let target = SessionTarget::new(456, 2);
    let focus_active = AtomicBool::new(false);
    let expected = TargetStamp {
        hwnd: 123,
        generation: 1,
    };

    let admission = final_down_target_admission(FinalTargetSignals {
        expected,
        require_focus: false,
        focus_active: &focus_active,
        target: &target,
        post_focus_race_hook: None,
        post_focus_control_signals: None,
    });

    assert_eq!(admission, FinalDownVerdict::TargetChanged);
    assert_eq!(
        send_counter.load(Ordering::SeqCst),
        0,
        "sender seam must not be invoked on target stamp mismatch"
    );
    let _ = backend;
}

#[test]
fn focus_then_manual_then_focus_restore_suspends_once_and_keeps_manual_pause() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));

    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
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
                scheduled_us: 20_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 25_000_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "next-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 25_100_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "next-up".to_string().into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("schedule");

    let force_inconclusive_probe = Arc::new(AtomicBool::new(false));
    let send_call_count = Arc::new(AtomicU64::new(0));
    let full_release_count = Arc::new(AtomicU64::new(0));
    let send_input_capture = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.force_inconclusive_probe = Some(Arc::clone(&force_inconclusive_probe));
    fault_script.send_call_count = Some(Arc::clone(&send_call_count));
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_count));
    fault_script.send_input_capture = Some(Arc::clone(&send_input_capture));

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

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.set_target_hwnd(123);
    start_with_test_wall_clock_slack(&session);

    wait_for_focus_down(&session);
    assert!(
        send_call_count.load(Ordering::SeqCst) >= 1,
        "note Down was sent"
    );

    // 1. Lose focus -> Focus pause
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    force_inconclusive_probe.store(true, Ordering::Release);
    session.set_focus_hint(false);
    wait_for_focus_pause(&session);
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);

    // 2. Request Manual pause before focus restoration
    let pause_generation = session
        .pause_with_timing_token()
        .expect("request manual pause");
    let pause_ack = wait_for_pause_ack(&session, pause_generation);
    assert!(pause_ack.completion_latency_us >= pause_ack.observation_latency_us);
    assert_eq!(
        full_release_count.load(Ordering::SeqCst),
        0,
        "no release while unfocused"
    );
    let send_count_before_restore = send_call_count.load(Ordering::SeqCst);

    // 3. Restore exact target/focus and pass grace
    force_inconclusive_probe.store(false, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);

    let snap = wait_for_clean_suspension(&session);
    assert_eq!(
        full_release_count.load(Ordering::SeqCst),
        0,
        "suspension uses tracked ownership"
    );
    assert_eq!(
        send_call_count.load(Ordering::SeqCst),
        send_count_before_restore + 1,
        "focus restoration sends one tracked release call"
    );
    let captures = send_input_capture
        .lock()
        .expect("send input capture lock")
        .clone();
    assert!(
        captures.iter().any(|capture| matches!(
            capture,
            MockSendInputCapture::SingleEmitter { scan_codes, key_up: true }
                if scan_codes.as_slice() == [0x15]
        )),
        "tracked suspension cleanup must reach the backend as a key-up call: {captures:?}"
    );
    assert!(snap.is_paused, "session remains paused due to manual pause");
    assert_eq!(snap.active_count, 0, "active keys clean after suspension");
    assert_eq!(snap.possibly_active_count, 0);
    assert_eq!(snap.failed_release_count, 0);
    assert!(!snap.has_terminal_error);

    let send_count_before = send_call_count.load(Ordering::SeqCst);
    session.publish_supervisor_progress().expect("heartbeat");
    assert_eq!(
        send_call_count.load(Ordering::SeqCst),
        send_count_before,
        "no gameplay Down during pause"
    );
    assert_eq!(
        full_release_count.load(Ordering::SeqCst),
        0,
        "no full-instrument sweep during pause"
    );

    // Resume manual pause
    session.resume().expect("resume");
    let resume_deadline = Instant::now() + Duration::from_secs(2);
    let mut post_snap = session.snapshot_lite();
    while post_snap.is_paused && Instant::now() < resume_deadline {
        session.publish_supervisor_progress().expect("heartbeat");
        std::thread::yield_now();
        post_snap = session.snapshot_lite();
    }
    assert!(!post_snap.is_paused, "playback resumed after manual resume");
    assert!(
        post_snap.is_running,
        "worker stayed alive after manual resume"
    );
    assert!(!post_snap.has_terminal_error);
    assert_eq!(
        full_release_count.load(Ordering::SeqCst),
        0,
        "no full-instrument sweep on manual resume"
    );

    session.quit().expect("quit");
    assert!(session.join(Duration::from_secs(5)).expect("join"));
}

#[test]
fn focus_then_manual_then_unfocused_resume_suspends_before_manual_resume() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));

    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
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
                scheduled_us: 20_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 22_000_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "next-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 22_100_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "next-up".to_string().into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("schedule");

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

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.set_target_hwnd(123);
    start_with_test_wall_clock_slack(&session);

    wait_for_focus_down(&session);

    // 1. Lose focus
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    force_inconclusive_probe.store(true, Ordering::Release);
    session.set_focus_hint(false);
    wait_for_focus_pause(&session);
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);

    // 2. Request manual pause
    session.pause().expect("request manual pause");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !session
        .shared_for_test()
        .commands
        .desired_pause
        .load(Ordering::Acquire)
        && Instant::now() < deadline
    {
        session.publish_supervisor_progress().expect("heartbeat");
        std::thread::sleep(Duration::from_millis(1));
    }
    for _ in 0..10 {
        session.publish_supervisor_progress().expect("heartbeat");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);

    // 3. Request resume while still unfocused
    session.resume().expect("request resume while unfocused");
    assert!(
        !session
            .shared_for_test()
            .commands
            .desired_pause
            .load(Ordering::Acquire)
    );
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);

    // 4. Restore focus
    force_inconclusive_probe.store(false, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);

    let mut snap = session.snapshot_lite();
    while snap.is_paused && Instant::now() < deadline {
        session.publish_supervisor_progress().expect("heartbeat");
        std::thread::sleep(Duration::from_millis(1));
        snap = session.snapshot_lite();
    }
    assert!(!snap.is_paused, "session resumed after focus restored");
    assert_eq!(
        full_release_count.load(Ordering::SeqCst),
        0,
        "focus restore uses tracked ownership"
    );
    assert!(!snap.has_terminal_error);

    session.quit().expect("quit");
    assert!(session.join(Duration::from_secs(5)).expect("join"));
}

#[test]
fn manual_then_focus_then_focus_restore_does_not_duplicate_suspension() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));

    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
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
                scheduled_us: 20_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 25_000_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "lifecycle-sentinel".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 25_100_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "lifecycle-sentinel-up".to_string().into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("schedule");

    let force_inconclusive_probe = Arc::new(AtomicBool::new(false));
    let full_release_count = Arc::new(AtomicU64::new(0));
    let focus_paused = Arc::new(AtomicBool::new(false));
    let restored = Arc::new(AtomicBool::new(false));
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
    options.focus_pause_hook = Some({
        let focus_paused = Arc::clone(&focus_paused);
        Arc::new(move || {
            focus_paused.store(true, Ordering::Release);
        })
    });
    options.restore_race_hook = Some({
        let restored = Arc::clone(&restored);
        Arc::new(move |_, _| {
            restored.store(true, Ordering::Release);
        })
    });

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.set_target_hwnd(123);
    start_with_test_wall_clock_slack(&session);

    wait_for_focus_down(&session);

    // 1. Enter Manual pause while focused
    let pause_generation = session.pause_with_timing_token().expect("manual pause");
    let pause_ack = wait_for_pause_ack(&session, pause_generation);
    assert!(pause_ack.completion_latency_us >= pause_ack.observation_latency_us);
    let snap = session.snapshot_lite();
    assert!(snap.is_paused);
    assert!(
        snap.is_running,
        "manual pause acknowledgment kept worker alive"
    );
    assert_eq!(
        full_release_count.load(Ordering::SeqCst),
        0,
        "manual pause uses tracked ownership"
    );
    assert_eq!(session.snapshot_lite().active_count, 0);

    // 2. Lose focus while Manual pause is active
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    force_inconclusive_probe.store(true, Ordering::Release);
    session.set_focus_hint(false);
    let focus_snapshot = wait_for_lifecycle_ack(&session, &focus_paused, "focus pause");
    assert!(focus_snapshot.is_running);
    assert_eq!(
        full_release_count.load(Ordering::SeqCst),
        0,
        "no full-instrument sweep while unfocused"
    );

    // 3. Restore focus
    force_inconclusive_probe.store(false, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);

    let restored_snapshot = wait_for_lifecycle_ack(&session, &restored, "focus restore");
    assert!(
        restored_snapshot.is_running,
        "focus restore acknowledgment kept worker alive: {restored_snapshot:?}"
    );
    assert!(restored_snapshot.is_paused, "still manual paused");

    assert_eq!(
        full_release_count.load(Ordering::SeqCst),
        0,
        "no full-instrument sweep on focus restore"
    );
    assert!(session.snapshot_lite().is_paused, "still manual paused");

    // Resume
    session.resume().expect("resume");
    let resume_deadline = Instant::now() + Duration::from_secs(2);
    let mut post_snap = session.snapshot_lite();
    while post_snap.is_paused && Instant::now() < resume_deadline {
        session.publish_supervisor_progress().expect("heartbeat");
        std::thread::yield_now();
        post_snap = session.snapshot_lite();
    }
    assert!(!post_snap.is_paused, "resumed");
    assert!(
        post_snap.is_running,
        "worker stayed alive after manual resume"
    );
    assert!(!post_snap.has_terminal_error);

    session.quit().expect("quit");
    assert!(session.join(Duration::from_secs(5)).expect("join"));
}

#[test]
fn system_suspend_terminates_without_rewriting_manual_or_focus_pause_state() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let mixed_target = QpcTicks::from_raw(1_000);
    harness.runtime.musical_physical_commit_started = true;
    harness.runtime.manual_pause_suspension_pending = true;
    harness
        .resources
        .playback
        .enter_pause(PauseReason::Focus, mixed_target)
        .expect("focus pause");
    harness
        .resources
        .playback
        .enter_pause(PauseReason::Manual, mixed_target)
        .expect("manual pause");

    assert!(harness.runtime.manual_pause_suspension_pending);
    assert!(
        harness
            .resources
            .playback
            .has_pause_reason(PauseReason::Focus)
    );
    assert!(
        harness
            .resources
            .playback
            .has_pause_reason(PauseReason::Manual)
    );

    assert!(harness.notify_system_power_for_test(true));
    assert!(
        harness.runtime.manual_pause_suspension_pending,
        "terminal system suspend does not rewrite the resumable manual pause state"
    );
    assert!(
        harness
            .resources
            .playback
            .has_pause_reason(PauseReason::Manual)
    );
    assert!(
        harness
            .resources
            .playback
            .has_pause_reason(PauseReason::Focus)
    );
    assert!(harness.process_command_control_for_test());
    let mut observation = super::worker::FinalizeTestObservation::default();
    assert_eq!(
        harness.finalize_worker_for_test(false, &mut observation),
        super::OUTCOME_SYSTEM_SUSPENDED
    );
}

#[test]
fn deferred_manual_suspension_failure_terminates_fail_closed() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));

    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
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
                scheduled_us: 20_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up".to_string().into(),
            },
        ],
        &[0x15],
    )
    .expect("schedule");

    let force_inconclusive_probe = Arc::new(AtomicBool::new(false));
    let force_preflight_failure = Arc::new(AtomicBool::new(false));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.force_inconclusive_probe = Some(Arc::clone(&force_inconclusive_probe));
    fault_script.force_preflight_failure = Some(Arc::clone(&force_preflight_failure));

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

    let session = NativeDispatchSession::new(options).expect("session admission");
    session.set_target_hwnd(123);
    start_with_test_wall_clock_slack(&session);

    wait_for_focus_down(&session);

    // 1. Lose focus -> Focus pause
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    force_inconclusive_probe.store(true, Ordering::Release);
    session.set_focus_hint(false);
    wait_for_focus_pause(&session);

    // 2. Request manual pause
    session.pause().expect("request manual pause");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !session
        .shared_for_test()
        .commands
        .desired_pause
        .load(Ordering::Acquire)
        && Instant::now() < deadline
    {
        session.publish_supervisor_progress().expect("heartbeat");
        std::thread::sleep(Duration::from_millis(1));
    }
    for _ in 0..10 {
        session.publish_supervisor_progress().expect("heartbeat");
        std::thread::sleep(Duration::from_millis(2));
    }

    // 3. Inject preflight failure for manual resume path
    force_preflight_failure.store(true, Ordering::Release);

    // 4. Restore focus
    force_inconclusive_probe.store(false, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);

    for _ in 0..20 {
        session.publish_supervisor_progress().expect("heartbeat");
        std::thread::sleep(Duration::from_millis(2));
    }

    // Now attempt manual resume with failing preflight -> must fail closed
    session.resume().expect("resume");
    let resume_deadline = Instant::now() + Duration::from_secs(2);
    let mut snap = session.snapshot_lite();
    while !snap.has_terminal_error && !snap.is_finished && Instant::now() < resume_deadline {
        let _ = session.publish_supervisor_progress();
        std::thread::sleep(Duration::from_millis(1));
        snap = session.snapshot_lite();
    }
    assert!(
        snap.has_terminal_error || snap.is_finished,
        "preflight failure terminates fail-closed"
    );

    let _ = session.quit();
    let _ = session.join(Duration::from_secs(5));
}
