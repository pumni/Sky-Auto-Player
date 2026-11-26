use super::*;

pub(super) const TEST_WALL_CLOCK_PREROLL_US: u64 = 500_000;

pub(super) fn test_session_options(
    schedule: sky_dispatch_core::model::OnsetPlan,
    _allowed_count: usize,
    backend: BackendConfig,
) -> NativeSessionOptions {
    test_session_options_with_timing_margin(schedule, backend, 500)
}

pub(super) fn test_session_options_with_timing_margin(
    schedule: sky_dispatch_core::model::OnsetPlan,
    backend: BackendConfig,
    timing_margin_us: u64,
) -> NativeSessionOptions {
    let effective_margin_us = timing_margin_us.min(schedule.hold_us());
    let timing = TimingOptions {
        game_fps: 60,
        min_hold_us: schedule.hold_us(),
        frame_us: 16_667,
        frame_base_hold_us: schedule.hold_us().saturating_sub(effective_margin_us),
        timing_margin_us: effective_margin_us,
        strict_timing: false,
        strict_down_completion_late_us: 2_000,
        strict_up_completion_late_us: 2_000,
        input_path_warn_us: 300,
    };
    let qpc = sky_dispatch_win32::clock::QpcClock::initialize().expect("QPC clock");
    let prepared_schedule = sky_dispatch_core::executor::PreparedDispatchSchedule::prepare(
        &schedule,
        qpc.frequency_hz(),
        schedule.compile_memory().caller_retained_capacity_bytes,
    )
    .expect("prepare native test schedule before session admission");
    NativeSessionOptions {
        schedule: prepared_schedule,
        backend,
        profile: DispatchProfile::StrictTimingDiagnostic,
        timing,
        focus: FocusOptions {
            require_focus: false,
            focus_restore_grace_us: 100_000,
        },
        wait: WaitOptions {
            enable_waitable_timer: true,
            enable_event_wait: true,
            supervisor_lease_timeout_us: 0,
            modifier_hold_timeout_us: crate::engine::config::DEFAULT_MODIFIER_HOLD_TIMEOUT_US,
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
        focus_pause_hook: None,
        timer_lifecycle_context: None,
        prepared_packet_ambiguity_mask: None,
        preflight_user_held_mask: None,
        modifier_key_state_query_for_test: Some(Arc::new(|_| 0)),
    }
}

pub(super) fn start_with_test_wall_clock_slack(session: &NativeDispatchSession) {
    session.arm(TEST_WALL_CLOCK_PREROLL_US).expect("worker arm");
}

pub(super) fn startup_boundary_schedule() -> sky_dispatch_core::model::OnsetPlan {
    sky_dispatch_core::plan::build_onset_plan(
        &[OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![0x15],
        }],
        &[0x15],
        100_000,
    )
    .expect("valid startup boundary schedule")
}

pub(super) fn large_startup_materialization_schedule(
    pair_count: usize,
) -> sky_dispatch_core::model::OnsetPlan {
    let mut chords = Vec::with_capacity(pair_count);
    for pair in 0..pair_count {
        let down_us = (pair as u64).saturating_mul(40_000);
        chords.push(OnsetChordInput {
            source_action_index: u32::try_from(pair).expect("chord index"),
            scheduled_us: down_us,
            scan_codes: smallvec::smallvec![0x15],
        });
    }
    sky_dispatch_core::plan::build_onset_plan(&chords, &[0x15], 20_000)
        .expect("large startup schedule must compile")
}

pub(super) fn checkpointed_session(
    schedule: sky_dispatch_core::model::OnsetPlan,
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

pub(super) fn progress_idle_gap_schedule() -> sky_dispatch_core::model::OnsetPlan {
    sky_dispatch_core::plan::build_onset_plan(
        &[
            OnsetChordInput {
                source_action_index: 0,
                // Keep the pause/resume lifecycle in an idle interval. No
                // physical generation is owned while the test acknowledges
                // the pause, so the test does not conflate pause observation
                // with terminal cleanup of an active key.
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x15],
            },
            OnsetChordInput {
                source_action_index: 1,
                scheduled_us: 5_000_000,
                scan_codes: smallvec::smallvec![0x15],
            },
        ],
        &[0x15],
        100_000,
    )
    .expect("valid progress idle-gap schedule")
}
