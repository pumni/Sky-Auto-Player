use super::*;

pub(super) const TEST_WALL_CLOCK_PREROLL_US: u64 = 500_000;

pub(super) fn bind_test_owner_identity(
    target: &SessionTarget,
    hwnd: isize,
    generation: u64,
    pid: u32,
) {
    target.require_owner_identity();
    assert!(target.bind_owner_identity(hwnd, generation, pid));
}

pub(super) fn issue_379_materialized_policy() -> MaterializedTimingPolicy {
    MaterializedTimingPolicy::from_user_margin(60, 1.0, 500)
        .expect("issue #379 materialized timing policy")
}

pub(super) fn test_session_options(
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
            min_hold_us: 500,
            min_release_gap_us: 17_167,
            frame_us: 16_667,
            frame_base_hold_us: 0,
            timing_margin_us: 500,
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
            #[cfg(any(test, feature = "test-support"))]
            test_wait_policy: super::TestWaitPolicy::LegacyTestWideSpin,
        },
        telemetry: TelemetryOptions {
            mode: TelemetryMode::Ring,
            capacity: 64,
        },
        priority: PriorityOptions {
            mode: sky_dispatch_win32::mmcss::PriorityMode::Off,
        },
        instrument_key_profile: None,
        startup_calibration_placement:
            super::config::StartupCalibrationPlacement::BeforeSchedulingGuards,
        timing_observer_for_qualification: false,
        startup_ordering_hook: None,
        restore_race_hook: None,
        focus_pause_hook: None,
        timer_lifecycle_context: None,
        prepared_packet_ambiguity_mask: None,
        preflight_user_held_mask: None,
        modifier_key_state_query_for_test: Some(Arc::new(|_| 0)),
    }
}

pub(super) fn set_test_timing_margin(options: &mut NativeSessionOptions, timing_margin_us: u64) {
    options.timing.timing_margin_us = timing_margin_us;
    options.timing.min_hold_us = options
        .timing
        .frame_base_hold_us
        .checked_add(timing_margin_us)
        .expect("test hold timing equation");
    options.timing.min_release_gap_us = options
        .timing
        .frame_us
        .checked_add(timing_margin_us)
        .expect("test release timing equation");
}

pub(super) fn start_with_test_wall_clock_slack(session: &NativeDispatchSession) {
    session.arm(TEST_WALL_CLOCK_PREROLL_US).expect("worker arm");
}

pub(super) fn startup_boundary_schedule() -> sky_dispatch_core::model::RuntimeSchedule {
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

pub(super) fn large_startup_materialization_schedule(
    pair_count: usize,
) -> sky_dispatch_core::model::RuntimeSchedule {
    let mut actions = Vec::with_capacity(pair_count.saturating_mul(2));
    for pair in 0..pair_count {
        let down_us = (pair as u64).saturating_mul(40_000);
        actions.push(KeyActionInput {
            source_action_index: u32::try_from(pair.saturating_mul(2)).expect("down action index"),
            kind: ActionKind::Down,
            scheduled_us: down_us,
            scan_codes: smallvec::smallvec![0x15],
            reason: "large-startup-down".into(),
        });
        actions.push(KeyActionInput {
            source_action_index: u32::try_from(pair.saturating_mul(2).saturating_add(1))
                .expect("up action index"),
            kind: ActionKind::Up,
            scheduled_us: down_us + 20_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "large-startup-up".into(),
        });
    }
    sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("large startup schedule must compile")
}

pub(super) fn checkpointed_session(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    checkpoint: StartupCheckpoint,
    occurrence: usize,
    require_focus: bool,
) -> (
    NativeDispatchSession,
    Arc<StartupOrderingHook>,
    Arc<AtomicU64>,
) {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(checkpoint, occurrence);
    let send_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
    let mut options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.profile = DispatchProfile::MockTest;
    options.focus.require_focus = require_focus;
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    (
        NativeDispatchSession::new(options).expect("checkpointed session admission"),
        hook,
        send_calls,
    )
}

pub(super) fn assert_checkpointed_terminal_stop(
    session: &NativeDispatchSession,
    hook: &StartupOrderingHook,
    send_calls: &AtomicU64,
    expected_status: EnginePollStatus,
    expected_trigger: &str,
) {
    assert_eq!(session.poll_state().status, EnginePollStatus::Preparing);
    assert_eq!(session.epoch_qpc_for_test(), QpcTicks::ZERO);
    assert_eq!(session.pre_roll_us_for_test(), 0);
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
    hook.release_startup_checkpoint();
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("checkpointed worker join")
    );
    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, expected_status.as_str());
    let expected_outcome = if expected_status == EnginePollStatus::SystemSuspended {
        "system_suspend"
    } else {
        expected_status.as_str()
    };
    assert_eq!(snapshot.outcome.as_deref(), Some(expected_outcome));
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some(expected_trigger)
    );
    assert!(!snapshot.startup_armed);
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
    assert_eq!(session.epoch_qpc_for_test(), QpcTicks::ZERO);
}

pub(super) fn run_seeded_adaptive_startup_schedule(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    allowed_count: usize,
) -> serde_json::Value {
    run_seeded_adaptive_startup_schedule_with_capacity(schedule, allowed_count, 16).1
}

pub(super) fn startup_probe_diagnostics(
    session: &NativeDispatchSession,
    hook: &StartupOrderingHook,
    scheduled_us: u64,
    elapsed_before_join: Duration,
    join_elapsed: Option<Duration>,
    join_result: &str,
    quit_result: &str,
) -> String {
    let snapshot = session.snapshot();
    let shared = session.shared_for_test();
    let startup = shared.commands.system_power.startup_snapshot();
    let join_state = session.join_state_for_test();
    let power = session.system_power_snapshot();
    let terminal_error = snapshot
        .terminal_error
        .as_deref()
        .map(|error| error.chars().take(160).collect::<String>());

    format!(
        "scheduled_us={scheduled_us} elapsed_before_join_us={} join_elapsed_us={:?} join_result={join_result} quit_request={quit_result} lifecycle_raw={} status={} startup={startup:?} completion={:?} worker_handle_present={} worker_finished={:?} watchdog_handle_present={} watchdog_finished={:?} control={{quit_requested={},skip_requested={},panic_requested={},supervisor_expired={},last_progress_ticks={}}} power={power:?} cleanup={{release={:?},obligation_mask=0x{:04x},release_attempts={},active={},possibly_active={},failed_release={}}} timing={{first_physical_send_marker={},ready={},armed={},startup_latency_us={:?},session_elapsed_us={}}} terminal={{worker_outcome={:?},trigger={:?},error={terminal_error:?}}}",
        elapsed_before_join.as_micros(),
        join_elapsed.map(|elapsed| elapsed.as_micros()),
        shared.lifecycle.lifecycle.load(Ordering::Acquire),
        snapshot.status,
        join_state.completion_signaled,
        join_state.worker_handle_present,
        join_state.worker_handle_finished,
        join_state.watchdog_handle_present,
        join_state.watchdog_handle_finished,
        shared.commands.quit_requested.load(Ordering::Acquire),
        shared.commands.skip_requested.load(Ordering::Acquire),
        shared.commands.panic_requested.load(Ordering::Acquire),
        shared.commands.supervisor_expired.is_expired(),
        shared.commands.supervisor_expired.last_progress_ticks(),
        snapshot.release_outcome,
        snapshot.final_release_obligation_mask,
        snapshot.release_attempts_total,
        snapshot.active_count,
        snapshot.possibly_active_count,
        snapshot.failed_release_count,
        hook.first_physical_send_started.load(Ordering::Acquire),
        snapshot.startup_ready,
        snapshot.startup_armed,
        snapshot.startup_latency_us,
        snapshot.elapsed_us,
        snapshot.outcome,
        snapshot.terminal_trigger_reason,
    )
}

pub(super) fn run_startup_first_physical_lead_probe(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    scheduled_us: u64,
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
    let probe_started = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(5);
    while hook.first_physical_send_started.load(Ordering::Acquire) == 0
        && !session.snapshot().is_finished
        && Instant::now() < deadline
    {
        if let Err(error) = session.publish_supervisor_progress() {
            panic!(
                "startup probe heartbeat failed ({error}): {}",
                startup_probe_diagnostics(
                    &session,
                    &hook,
                    scheduled_us,
                    probe_started.elapsed(),
                    None,
                    "not attempted",
                    "not requested",
                )
            );
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let snapshot = session.snapshot();
    assert!(
        hook.first_physical_send_started.load(Ordering::Acquire) > 0,
        "startup probe did not reach physical dispatch: {}; snapshot status={} outcome={:?} terminal_error={:?}",
        startup_probe_diagnostics(
            &session,
            &hook,
            scheduled_us,
            probe_started.elapsed(),
            None,
            "not attempted",
            "not requested",
        ),
        snapshot.status,
        snapshot.outcome,
        snapshot.terminal_error,
    );
    let quit_result = if !session.snapshot().is_finished {
        match session.quit() {
            Ok(()) => "accepted".to_string(),
            Err(error)
                if error == "session commands require a running worker"
                    && session.snapshot().is_finished =>
            {
                "worker finished before request".to_string()
            }
            Err(error) => panic!(
                "startup probe quit failed ({error}): {}",
                startup_probe_diagnostics(
                    &session,
                    &hook,
                    scheduled_us,
                    probe_started.elapsed(),
                    None,
                    "not attempted",
                    "rejected",
                )
            ),
        }
    } else {
        "worker already finished".to_string()
    };
    let join_started = Instant::now();
    let join_result = session.join(Duration::from_secs(5));
    let join_elapsed = join_started.elapsed();
    assert!(
        join_result.as_ref().is_ok_and(|joined| *joined),
        "startup probe join failed: {}",
        startup_probe_diagnostics(
            &session,
            &hook,
            scheduled_us,
            probe_started.elapsed(),
            Some(join_elapsed),
            &format!("{join_result:?}"),
            &quit_result,
        )
    );
    serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
        .expect("valid telemetry JSON")
}

pub(super) fn run_seeded_adaptive_startup_schedule_with_capacity(
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
    // The widened authored window keeps startup jitter out of this ordering
    // probe; leave enough lease time for the delayed cleanup boundary too.
    options.wait.supervisor_lease_timeout_us = 5_000_000;

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

pub(super) fn stale_leading_up_schedule(
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

pub(super) fn same_timestamp_stale_leading_up_schedule() -> sky_dispatch_core::model::RuntimeSchedule
{
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

pub(super) fn assert_stale_leading_up_did_not_send(
    telemetry: &serde_json::Value,
    stale_count: usize,
) {
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

pub(super) fn run_mock_schedule(
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

pub(super) fn run_mock_schedule_with_profile(
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

pub(super) fn progress_idle_gap_schedule() -> sky_dispatch_core::model::RuntimeSchedule {
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

pub(super) struct FocusOverrideResetGuard;

impl Drop for FocusOverrideResetGuard {
    fn drop(&mut self) {
        sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    }
}

pub(super) fn focus_recovery_schedule() -> sky_dispatch_core::model::RuntimeSchedule {
    sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                // Keep the focus-state tests independent of worker-startup
                // jitter; their assertions begin only after this commit.
                scheduled_us: 5_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "focus-recovery-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 6_700_000,
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
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 10_200_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "focus-recovery-sentinel-up".to_string().into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid focus-recovery schedule")
}

pub(super) fn start_focus_recovery_session(
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

pub(super) fn wait_for_focus_down(
    session: &NativeDispatchSession,
) -> super::EngineProgressSnapshot {
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut snapshot = session.snapshot_lite();
    while snapshot.recent_latencies_us.is_empty()
        && !snapshot.is_finished
        && Instant::now() < deadline
    {
        session
            .publish_supervisor_progress()
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

pub(super) fn wait_for_focus_pause(
    session: &NativeDispatchSession,
) -> super::EngineProgressSnapshot {
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut snapshot = session.snapshot_lite();
    while !snapshot.is_paused && !snapshot.is_finished && Instant::now() < deadline {
        session
            .publish_supervisor_progress()
            .expect("heartbeat while focus is lost");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(
        snapshot.is_paused,
        "focus loss did not enter pause: {snapshot:?}"
    );
    snapshot
}

pub(super) fn wait_for_pause_ack(
    session: &NativeDispatchSession,
    generation: u64,
) -> CommandTimingResult {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot_lite();
        if let Some(result) = session
            .pause_timing_result(generation)
            .expect("pause timing result")
        {
            return result;
        }
        assert!(
            !snapshot.is_finished,
            "pause acknowledgment lost the worker: {snapshot:?}; terminal_error={:?}",
            session.snapshot().terminal_error
        );
        assert!(
            Instant::now() < deadline,
            "pause acknowledgment timed out: {snapshot:?}"
        );
        session
            .publish_supervisor_progress()
            .expect("heartbeat while waiting for pause");
        std::thread::yield_now();
    }
}

pub(super) fn wait_for_clean_suspension(
    session: &NativeDispatchSession,
) -> super::EngineProgressSnapshot {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot_lite();
        if snapshot.active_count == 0
            && snapshot.possibly_active_count == 0
            && snapshot.failed_release_count == 0
        {
            return snapshot;
        }
        assert!(
            !snapshot.is_finished,
            "suspension did not complete before worker exit: {snapshot:?}; terminal_error={:?}",
            session.snapshot().terminal_error
        );
        assert!(
            Instant::now() < deadline,
            "suspension state publication timed out: {snapshot:?}"
        );
        session
            .publish_supervisor_progress()
            .expect("heartbeat while waiting for suspension");
        std::thread::yield_now();
    }
}

pub(super) fn wait_for_lifecycle_ack(
    session: &NativeDispatchSession,
    observed: &AtomicBool,
    transition: &str,
) -> super::EngineProgressSnapshot {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot_lite();
        if observed.load(Ordering::Acquire) {
            return snapshot;
        }
        assert!(
            !snapshot.is_finished,
            "{transition} lost the worker: {snapshot:?}; terminal_error={:?}",
            session.snapshot().terminal_error
        );
        assert!(
            Instant::now() < deadline,
            "{transition} acknowledgment timed out: {snapshot:?}"
        );
        session
            .publish_supervisor_progress()
            .expect("heartbeat while waiting for lifecycle transition");
        std::thread::yield_now();
    }
}

pub(super) fn wait_for_focus_finish(
    session: &NativeDispatchSession,
) -> super::EngineProgressSnapshot {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut snapshot = session.snapshot_lite();
    while !snapshot.is_finished && Instant::now() < deadline {
        session
            .publish_supervisor_progress()
            .expect("heartbeat while waiting for terminal state");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(snapshot.is_finished, "session did not finish: {snapshot:?}");
    snapshot
}
