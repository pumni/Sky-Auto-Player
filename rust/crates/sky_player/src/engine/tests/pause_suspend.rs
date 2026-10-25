use super::support::*;
use super::*;

#[test]
fn final_down_foreground_proof_rejects_invalid_target_without_query() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(1));
    sky_dispatch_win32::focus::reset_foreground_query_count();
    let focus_active = AtomicBool::new(true);
    let target = SessionTarget::new(0, 1);
    assert_eq!(
        final_down_target_admission(FinalTargetSignals {
            expected: TargetStamp {
                hwnd: 0,
                generation: 1,
            },
            require_focus: true,
            focus_active: &focus_active,
            target: &target,
            post_focus_race_hook: None,
            post_focus_control_signals: None,
        }),
        FinalDownVerdict::FocusLost
    );
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 0);
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
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
    let quit_requested = AtomicBool::new(false);
    let skip_requested = AtomicBool::new(false);
    let panic_requested = AtomicBool::new(true);
    let desired_pause = AtomicBool::new(false);
    let supervisor_expired = super::shared::SupervisorLeaseState::new(QpcTicks::from_raw(1));

    let admission = || {
        final_control_precheck(FinalControlSignals {
            quit_requested: &quit_requested,
            skip_requested: &skip_requested,
            panic_requested: &panic_requested,
            desired_pause: &desired_pause,
            supervisor_expired: &supervisor_expired,
            system_power: None,
        })
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
fn panic_arriving_after_outer_sample_is_caught_by_final_control_gate() {
    let quit_requested = AtomicBool::new(false);
    let skip_requested = AtomicBool::new(false);
    let panic_requested = AtomicBool::new(false);
    let desired_pause = AtomicBool::new(false);
    let supervisor_expired = super::shared::SupervisorLeaseState::new(QpcTicks::from_raw(1));
    let signals = || FinalControlSignals {
        quit_requested: &quit_requested,
        skip_requested: &skip_requested,
        panic_requested: &panic_requested,
        desired_pause: &desired_pause,
        supervisor_expired: &supervisor_expired,
        system_power: None,
    };

    assert!(!panic_requested.load(Ordering::Acquire));
    // The worker's read-mostly outer sample saw false. The interrupt/final
    // gate remains the authority for a signal arriving immediately after it.
    panic_requested.store(true, Ordering::Release);
    assert_eq!(
        final_control_precheck(signals()),
        FinalControlAdmission::PanicRequested
    );
}

#[test]
fn system_suspend_after_wait_wake_blocks_final_down_admission_after_resume() {
    use super::shared::SystemPowerState;
    use sky_dispatch_win32::event::OwnedEvent;

    let interrupt = OwnedEvent::new_auto_reset().expect("interrupt event");
    let system_power = SystemPowerState::default();
    let quit_requested = AtomicBool::new(false);
    let skip_requested = AtomicBool::new(false);
    let panic_requested = AtomicBool::new(false);
    let desired_pause = AtomicBool::new(false);
    let supervisor_expired = super::shared::SupervisorLeaseState::new(QpcTicks::from_raw(1));
    let signals = || FinalControlSignals {
        quit_requested: &quit_requested,
        skip_requested: &skip_requested,
        panic_requested: &panic_requested,
        desired_pause: &desired_pause,
        supervisor_expired: &supervisor_expired,
        system_power: Some(&system_power),
    };

    // The direct wait has already woken. A suspend callback arriving before
    // final admission must still win the final atomic gate.
    assert!(system_power.notify(true, &interrupt));
    assert_eq!(
        final_control_precheck(signals()),
        FinalControlAdmission::SystemSuspendRequested
    );
    quit_requested.store(true, Ordering::Release);
    skip_requested.store(true, Ordering::Release);
    assert_eq!(
        final_control_precheck(signals()),
        FinalControlAdmission::SystemSuspendRequested,
        "accepted suspend wins over nominal Stop and Skip at final admission"
    );
    system_power.take_pending();
    assert!(system_power.notify(false, &interrupt));
    assert_eq!(
        final_control_precheck(signals()),
        FinalControlAdmission::SystemSuspendRequested
    );
    assert!(!system_power.complete_resume());
    assert_eq!(
        final_control_precheck(signals()),
        FinalControlAdmission::SystemSuspendRequested
    );
}

#[test]
fn system_suspend_precedes_stop_skip_and_shutdown_terminal_requests() {
    for request in ["stop", "skip", "shutdown"] {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        match request {
            "stop" => harness.quit_requested.store(true, Ordering::Release),
            "skip" => harness.skip_requested.store(true, Ordering::Release),
            "shutdown" => {
                // The desktop shutdown path requests Quit and then asks for
                // panic-release as its final physical safety signal.
                harness.quit_requested.store(true, Ordering::Release);
                harness.panic_requested.store(true, Ordering::Release);
            }
            _ => unreachable!(),
        }

        assert!(harness.process_command_control_for_test());
        assert_eq!(
            harness.runtime.terminal_trigger,
            Some(if request == "skip" {
                super::worker::TerminalTrigger::Skip
            } else {
                super::worker::TerminalTrigger::Quit
            }),
            "the worker must first record the requested terminal command"
        );
        assert!(harness.notify_system_power_for_test(true));
        assert!(harness.notify_system_power_for_test(false));
        let mut observation = super::worker::FinalizeTestObservation::default();
        assert_eq!(
            harness.finalize_worker_for_test(false, &mut observation),
            super::OUTCOME_SYSTEM_SUSPENDED,
            "{request} request replaced clean suspend outcome"
        );
        assert_eq!(
            observation.terminal_trigger_reason.as_deref(),
            Some("system_suspend")
        );
    }
}

#[test]
fn stale_epoch_resume_cannot_authorize_due_worker_down() {
    use super::shared::SystemPowerEndpoint;
    use super::test_support::ProductionDispatchTestHarness;
    use super::worker::DispatchStep;
    use std::sync::Arc;

    let endpoint = SystemPowerEndpoint::new().expect("stable power endpoint");
    endpoint
        .reset_for_new_session()
        .expect("open old session epoch");
    let state = endpoint.state();
    assert!(endpoint.notify_system_power(true, Some(QpcTicks::from_raw(10_000))));

    // Pause after the old callback captures its epoch, before it takes an
    // in-flight reference. The lifecycle reset can retire that authority.
    state.arm_epoch_capture_pause_for_test();
    let stale_endpoint = Arc::clone(&endpoint);
    let stale_resume = std::thread::spawn(move || stale_endpoint.notify_system_power(false, None));
    while !state.epoch_capture_pause_reached_for_test() {
        std::thread::yield_now();
    }

    endpoint
        .reset_for_new_session()
        .expect("rundown old epoch and open new session");
    assert!(endpoint.notify_system_power(true, Some(QpcTicks::from_raw(20_000))));
    let current_signal_generation = endpoint.interrupt().signal_generation();

    let mut worker = ProductionDispatchTestHarness::new_down_only();
    worker.attach_system_power_endpoint_for_test(&endpoint);
    let sender_calls = worker.configure_send_counter();
    let mut due_down = worker.build_prepared_stream_for_test();

    state.release_epoch_capture_pause_for_test();
    assert!(!stale_resume.join().expect("stale callback is ignored"));
    assert!(state.os_suspended());
    assert!(state.down_blocked());
    assert_eq!(
        state.suspend_boundary_qpc(),
        Some(QpcTicks::from_raw(20_000))
    );
    assert_eq!(state.snapshot(), (true, true, 1, 0, 0));
    assert_eq!(
        endpoint.interrupt().signal_generation(),
        current_signal_generation,
        "stale callback must not signal the new session"
    );

    let blocked_step = worker.dispatch_prepared_current_at_lateness_for_test(&mut due_down, 2_000);
    assert!(
        !matches!(blocked_step, DispatchStep::Dispatched),
        "new-session suspend admitted a due Down: {blocked_step:?}"
    );
    assert_eq!(sender_calls.load(Ordering::SeqCst), 0);

    assert_eq!(
        worker.take_system_power_pending_for_test(),
        super::shared::SYSTEM_POWER_SUSPEND_PENDING
    );
    assert!(endpoint.notify_system_power(false, None));
    assert_eq!(
        worker.take_system_power_pending_for_test(),
        super::shared::SYSTEM_POWER_RESUME_PENDING
    );
    assert!(!worker.complete_system_resume_for_test());
    assert!(state.down_blocked());

    let resumed_step = worker.dispatch_prepared_current_at_lateness_for_test(&mut due_down, 2_000);
    assert!(
        !matches!(resumed_step, DispatchStep::Dispatched),
        "current-epoch resume reopened terminal Down admission: {resumed_step:?}"
    );
    assert_eq!(sender_calls.load(Ordering::SeqCst), 0);
    assert!(worker.process_command_control_for_test());
    let mut observation = super::worker::FinalizeTestObservation::default();
    assert_eq!(
        worker.finalize_worker_for_test(false, &mut observation),
        super::OUTCOME_SYSTEM_SUSPENDED
    );
    assert_eq!(
        observation.terminal_trigger_reason.as_deref(),
        Some("system_suspend")
    );
}

#[test]
fn shutdown_after_system_suspend_keeps_the_terminal_outcome_and_releases_held_input() {
    use super::MockSendInputCapture;

    let send_call_count = Arc::new(AtomicU64::new(0));
    let captures = Arc::new(std::sync::Mutex::new(Vec::new()));
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownCommitted, 1);
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_call_count));
    fault_script.send_input_capture = Some(Arc::clone(&captures));
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
    session
        .arm(TEST_WALL_CLOCK_PREROLL_US)
        .expect("arm session");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(3)));
    assert_eq!(send_call_count.load(Ordering::SeqCst), 1);

    assert!(session.notify_system_power(true));
    assert!(session.notify_system_power(false));
    let _stop_result = session.quit();
    let _panic_release_result = session.panic_release();
    hook.release_startup_checkpoint();
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let terminal = session.snapshot_lite();
    assert!(terminal.is_finished);
    assert_eq!(terminal.status, EnginePollStatus::SystemSuspended.as_str());
    assert_eq!(session.terminal_outcome(), Some("system_suspend"));
    assert_eq!(terminal.active_count, 0);
    assert!(session.system_power_snapshot().down_blocked);
    assert_eq!(send_call_count.load(Ordering::SeqCst), 2);
    let captures = captures.lock().expect("mock input capture");
    assert!(
        matches!(
            captures.as_slice(),
            [
                MockSendInputCapture::Packet {
                    packet: PhysicalPacket {
                        down_mask: 1,
                        up_mask: 0
                    },
                    ..
                },
                MockSendInputCapture::SingleEmitter { key_up: true, .. }
            ]
        ),
        "unexpected mock input sequence: {captures:?}"
    );
}

#[test]
fn suspend_during_admitted_mock_send_accounts_in_flight_down_and_releases_it() {
    use super::MockSendInputCapture;
    use std::sync::Barrier;

    let entered = Arc::new(Barrier::new(2));
    let release_sender = Arc::new(Barrier::new(2));
    let send_call_count = Arc::new(AtomicU64::new(0));
    let captures = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_call_count));
    fault_script.send_input_capture = Some(Arc::clone(&captures));
    fault_script.first_packet_send_entered = Some(Arc::clone(&entered));
    fault_script.release_first_packet_send = Some(Arc::clone(&release_sender));
    let options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    let session = NativeDispatchSession::new(options).expect("test session admission");
    session
        .arm(TEST_WALL_CLOCK_PREROLL_US)
        .expect("arm session");

    // The worker has passed final Down admission and entered its first mock
    // packet send. Hold it there while the OS callback accepts Suspend/Resume.
    entered.wait();
    assert_eq!(send_call_count.load(Ordering::SeqCst), 1);
    assert!(session.notify_system_power(true));
    assert!(session.notify_system_power(false));
    release_sender.wait();

    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::SystemSuspended.as_str());
    assert_eq!(snapshot.outcome.as_deref(), Some("system_suspend"));
    assert_eq!(snapshot.final_release_obligation_mask, 0);
    assert!(
        snapshot
            .release_outcome
            .as_ref()
            .is_some_and(|release| release.released_successfully && release.stuck_mask == 0)
    );
    assert_eq!(send_call_count.load(Ordering::SeqCst), 2);
    let captures = captures.lock().expect("mock input capture");
    assert!(
        matches!(
            captures.as_slice(),
            [
                MockSendInputCapture::Packet {
                    packet: PhysicalPacket {
                        down_mask: 1,
                        up_mask: 0
                    },
                    ..
                },
                MockSendInputCapture::SingleEmitter { key_up: true, .. }
            ]
        ),
        "unexpected in-flight and terminal release sequence: {captures:?}"
    );
}

#[test]
fn system_suspend_with_unverified_terminal_release_stays_a_worker_error() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownCommitted, 1);
    let send_call_count = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::persistent_release();
    fault_script.send_call_count = Some(Arc::clone(&send_call_count));
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

    session
        .arm(TEST_WALL_CLOCK_PREROLL_US)
        .expect("arm session");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(3)));
    assert_eq!(send_call_count.load(Ordering::SeqCst), 1);
    assert!(session.notify_system_power(true));
    assert!(session.notify_system_power(false));
    hook.release_startup_checkpoint();

    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    assert_eq!(session.poll_state().status, EnginePollStatus::Error);
    assert_eq!(session.terminal_outcome(), Some("error"));
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some("system_suspend")
    );
    assert!(snapshot.final_release_obligation_mask != 0);
    assert!(snapshot.terminal_error.is_some());
    assert!(send_call_count.load(Ordering::SeqCst) > 1);
}

#[test]
fn final_control_gate_uses_atomic_supervisor_hard_stop_without_lease_qpc() {
    let quit_requested = AtomicBool::new(false);
    let skip_requested = AtomicBool::new(false);
    let panic_requested = AtomicBool::new(false);
    let desired_pause = AtomicBool::new(false);
    let supervisor_expired = super::shared::SupervisorLeaseState::new(QpcTicks::from_raw(1));
    supervisor_expired.latch_expired();
    let signals = || FinalControlSignals {
        quit_requested: &quit_requested,
        skip_requested: &skip_requested,
        panic_requested: &panic_requested,
        desired_pause: &desired_pause,
        supervisor_expired: &supervisor_expired,
        system_power: None,
    };

    assert_eq!(
        final_control_precheck(signals()),
        FinalControlAdmission::PanicRequested
    );

    supervisor_expired.reset(QpcTicks::from_raw(1));
    assert_eq!(
        final_control_precheck(signals()),
        FinalControlAdmission::Allowed
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
fn prepared_up_only_has_one_final_gate_for_every_hard_stop() {
    use super::test_support::ProductionDispatchTestHarness;

    for control in 0..7 {
        let mut harness = ProductionDispatchTestHarness::new_uponly_release_with_gap(100_000);
        let calls = harness.configure_send_counter();
        harness.advance_playback_time_us(100_000);
        match control {
            0 => harness.quit_requested.store(true, Ordering::Release),
            1 => harness.skip_requested.store(true, Ordering::Release),
            2 => harness.desired_pause.store(true, Ordering::Release),
            3 => harness.panic_requested.store(true, Ordering::Release),
            4 => harness.supervisor_expired.latch_expired(),
            5 => assert!(harness.notify_system_power_for_test(true)),
            6 => harness.set_final_gate_race_hook(
                |_focus_active, _target, quit, _skip, _panic, _pause, _system_power| {
                    quit.store(true, Ordering::Release);
                },
            ),
            _ => unreachable!(),
        }

        let mut stream = harness.build_prepared_stream_for_test();
        let step = harness.dispatch_prepared_current_at_lateness_for_test(&mut stream, 10_000);
        assert!(
            !matches!(step, super::worker::DispatchStep::Dispatched),
            "UpOnly hard-stop {control} sent: {step:?}"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "UpOnly hard-stop {control}"
        );
    }
}

#[test]
fn authored_up_only_ignores_stale_heartbeat_without_watchdog_hard_stop() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_uponly_release_with_gap(100_000);
    let calls = harness.configure_send_counter();
    harness.advance_playback_time_us(100_000);
    let plan = harness.plan_current_dispatch();
    harness.supervisor_expired.reset(QpcTicks::from_raw(1));

    let step = harness.dispatch_authored_with_plan(&plan);
    assert!(
        matches!(step, super::worker::DispatchStep::Dispatched),
        "stale heartbeat must not be a final per-note policy: {step:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
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

#[test]
fn authored_post_start_focus_loss_pauses_without_resumable_cleanup() {
    use super::test_support::ProductionDispatchTestHarness;
    use super::worker::dispatch::DispatchObservation;

    let mut harness = ProductionDispatchTestHarness::new_deferred_release_with_unrelated_down();
    let calls = harness.configure_send_counter();
    harness.align_next_plan_to_future_for_test(500_000);
    let seed_plan = harness.plan_current_dispatch();
    let seed_step = harness.dispatch_at_plan_target_for_test(&seed_plan);
    assert!(
        matches!(seed_step, super::worker::DispatchStep::Dispatched),
        "seed dispatch: {seed_step:?}"
    );
    while harness.pop_observation().is_some() {}
    harness.config.focus.require_focus = true;
    let seed_send_calls = calls.load(Ordering::SeqCst);
    let release_calls = harness.full_instrument_release_calls();
    let active_mask = harness.backend_active_mask();
    let coordinator_active_mask = harness.resources.coordinator.active_mask;
    assert_ne!(
        active_mask, 0,
        "seed Down must establish physical ownership"
    );
    while harness.pop_observation().is_some() {}

    let plan = harness.plan_current_dispatch();
    harness.focus_active.store(false, Ordering::Release);

    let step = harness.dispatch_authored_with_plan(&plan);

    assert!(matches!(step, super::worker::DispatchStep::Continue));
    assert_eq!(calls.load(Ordering::SeqCst), seed_send_calls);
    assert_eq!(harness.full_instrument_release_calls(), release_calls);
    assert_eq!(harness.backend_active_mask(), active_mask);
    assert_eq!(
        harness.resources.coordinator.active_mask,
        coordinator_active_mask
    );
    assert!(
        harness
            .progress_clock
            .load()
            .expect("progress projection")
            .paused
    );
    assert!(matches!(
        harness.pop_observation(),
        Some(DispatchObservation::BlockedUnfocused(_))
    ));
    assert!(harness.pop_observation().is_none());
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
            .publish_supervisor_progress()
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
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 10_100_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "focus-restore-sentinel-up".to_string().into(),
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
            .publish_supervisor_progress()
            .expect("heartbeat while waiting for Down");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(!snapshot.recent_latencies_us.is_empty());

    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    force_inconclusive_probe.store(true, Ordering::Release);
    session.set_focus_hint(false);
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
    assert!(!snapshot.has_terminal_error);

    force_inconclusive_probe.store(false, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);
    let restore_deadline = Instant::now() + Duration::from_secs(2);
    while snapshot.is_paused && !snapshot.is_finished && Instant::now() < restore_deadline {
        session
            .publish_supervisor_progress()
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
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);

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
        session
            .publish_supervisor_progress()
            .expect("heartbeat during restore grace");
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
        session
            .publish_supervisor_progress()
            .expect("heartbeat during first restore");
        std::thread::sleep(Duration::from_millis(1));
    }

    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    session.set_focus_hint(false);
    let focus_loss_observation_deadline = Instant::now() + Duration::from_millis(20);
    while Instant::now() < focus_loss_observation_deadline {
        session
            .publish_supervisor_progress()
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
            .publish_supervisor_progress()
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
            .publish_supervisor_progress()
            .expect("heartbeat while waiting for second restore grace");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    assert!(
        !snapshot.is_paused,
        "second restore grace did not resume playback: {snapshot:?}"
    );
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);
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
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);

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
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);

    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn preroll_preflight_rejection_emits_no_unowned_cleanup() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));

    let force_preflight_failure = Arc::new(AtomicBool::new(true));
    let full_release_count = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.force_preflight_failure = Some(force_preflight_failure);
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_count));
    let session = start_focus_recovery_session(fault_script, None);

    let snapshot = wait_for_focus_finish(&session);
    assert_eq!(snapshot.keys_inserted_before_failure, 0);
    let last_error = session.snapshot().last_error;
    assert!(
        last_error.as_deref().is_some_and(|error| {
            error.starts_with("instrument key preflight failed during preroll;")
        }),
        "unexpected preroll result: {last_error:?}; snapshot={:?}; release_count={}",
        session.snapshot(),
        full_release_count.load(Ordering::SeqCst)
    );
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn user_held_preroll_preflight_emits_no_unowned_cleanup() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));

    let send_calls = Arc::new(AtomicU64::new(0));
    let full_release_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.preflight_user_held_mask = Some(0b10);
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_calls));
    let session = start_focus_recovery_session(fault_script, None);

    let snapshot = wait_for_focus_finish(&session);
    assert_eq!(snapshot.keys_inserted_before_failure, 0);
    assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    assert_eq!(full_release_calls.load(Ordering::SeqCst), 0);
    assert!(
        session
            .snapshot()
            .last_error
            .as_deref()
            .is_some_and(|error| {
                error.starts_with("instrument key preflight failed during preroll;")
                    && error.contains("physically held before playback")
            })
    );
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
        Arc::new(move |focus_active, target| {
            if !raced.swap(true, Ordering::SeqCst) {
                focus_active.store(false, Ordering::Release);
                target.publish(456);
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
        session
            .publish_supervisor_progress()
            .expect("heartbeat during restore race");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        raced.load(Ordering::Acquire),
        "restore race hook did not run"
    );
    let settle_deadline = Instant::now() + Duration::from_millis(30);
    while Instant::now() < settle_deadline {
        session
            .publish_supervisor_progress()
            .expect("heartbeat after restore race");
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
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);

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
        session
            .publish_supervisor_progress()
            .expect("heartbeat during manual pause");
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
            .publish_supervisor_progress()
            .expect("heartbeat during composed pause");
        std::thread::sleep(Duration::from_millis(1));
    }

    snapshot = session.snapshot_lite();
    assert!(
        snapshot.is_paused,
        "manual pause was cleared by focus restore: {snapshot:?}"
    );
    assert_eq!(full_release_count.load(Ordering::SeqCst), 0);

    session.resume().expect("manual resume");
    let resume_deadline = Instant::now() + Duration::from_secs(1);
    while snapshot.is_paused && !snapshot.is_finished && Instant::now() < resume_deadline {
        session
            .publish_supervisor_progress()
            .expect("heartbeat during manual resume");
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
    harness.target.publish(99);

    let step = harness.dispatch_authored_with_plan(&plan);

    assert!(
        matches!(step, super::worker::DispatchStep::Dispatched),
        "target changes must not block UpOnly cleanup: {step:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn authored_down_final_control_races_never_reach_transport() {
    use super::test_support::ProductionDispatchTestHarness;

    for command in ["pause", "quit", "skip", "panic"] {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let calls = harness.configure_send_counter();
        harness.advance_playback_time_us(100_000);
        let plan = harness.plan_current_dispatch();
        harness.set_final_gate_race_hook(move |_, _, quit, skip, panic, pause, _system_power| {
            match command {
                "pause" => pause.store(true, Ordering::Release),
                "quit" => quit.store(true, Ordering::Release),
                "skip" => skip.store(true, Ordering::Release),
                "panic" => panic.store(true, Ordering::Release),
                _ => unreachable!("control race table"),
            }
        });

        let step = harness.dispatch_at_plan_target_for_test(&plan);

        assert!(!matches!(step, super::worker::DispatchStep::Dispatched));
        assert_eq!(calls.load(Ordering::SeqCst), 0, "{command} sent Down");
        assert_eq!(harness.local_metrics.final_gate_control_rejections, 1);
    }
}

#[test]
fn authored_down_target_change_after_crossing_never_reaches_transport() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let calls = harness.configure_send_counter();
    harness.advance_playback_time_us(100_000);
    let plan = harness.plan_current_dispatch();
    harness.set_final_gate_race_hook(|_, target, _, _, _, _, _| {
        target.publish(456);
    });

    let step = harness.dispatch_at_plan_target_for_test(&plan);

    assert!(matches!(step, super::worker::DispatchStep::Continue));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(harness.local_metrics.final_gate_target_changes, 1);
}

#[test]
fn authored_down_focus_loss_after_crossing_never_reaches_transport() {
    use super::test_support::ProductionDispatchTestHarness;

    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    sky_dispatch_win32::focus::reset_foreground_query_count();
    let mut harness = ProductionDispatchTestHarness::new_deferred_release_with_unrelated_down();
    let calls = harness.configure_send_counter();
    harness.align_next_plan_to_future_for_test(500_000);
    let seed_plan = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&seed_plan),
        super::worker::DispatchStep::Dispatched
    ));
    harness.config.focus.require_focus = true;
    let seed_send_calls = calls.load(Ordering::SeqCst);
    let release_calls = harness.full_instrument_release_calls();
    let active_mask = harness.backend_active_mask();
    let coordinator_active_mask = harness.resources.coordinator.active_mask;
    while harness.pop_observation().is_some() {}
    let plan = harness.plan_current_dispatch();
    harness.set_final_gate_race_hook(|focus, _, _, _, _, _, _| {
        focus.store(false, Ordering::Release);
    });

    let step = harness.dispatch_at_plan_target_for_test(&plan);

    assert!(matches!(step, super::worker::DispatchStep::Continue));
    assert_eq!(calls.load(Ordering::SeqCst), seed_send_calls);
    assert_eq!(harness.full_instrument_release_calls(), release_calls);
    assert_eq!(harness.backend_active_mask(), active_mask);
    assert_eq!(
        harness.resources.coordinator.active_mask,
        coordinator_active_mask
    );
    assert!(
        harness
            .progress_clock
            .load()
            .expect("progress projection")
            .paused
    );
    assert_eq!(harness.local_metrics.final_gate_focus_losses, 1);
    assert_eq!(sky_dispatch_win32::focus::foreground_query_count(), 0);
    assert!(
        !std::iter::from_fn(|| harness.pop_observation()).any(|_| true),
        "final-gate focus loss must not enqueue a diagnostics observation"
    );
}

#[test]
fn authored_down_post_foreground_revalidation_blocks_atomic_races() {
    use super::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_win32::event::OwnedEvent;

    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(1));
    for race in ["target", "quit", "skip", "panic", "pause", "suspend"] {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        harness.config.focus.require_focus = true;
        let calls = harness.configure_send_counter();
        harness.advance_playback_time_us(100_000);
        let plan = harness.plan_current_dispatch();
        sky_dispatch_win32::focus::reset_foreground_query_count();
        let interrupt = Arc::new(OwnedEvent::new_auto_reset().expect("post-focus interrupt"));
        harness.set_post_focus_revalidation_race_hook(
            move |_, target, quit, skip, panic, pause, system_power| match race {
                "target" => {
                    target.publish(456);
                }
                "quit" => quit.store(true, Ordering::Release),
                "skip" => skip.store(true, Ordering::Release),
                "panic" => panic.store(true, Ordering::Release),
                "pause" => pause.store(true, Ordering::Release),
                "suspend" => {
                    assert!(system_power.notify_at(true, None, interrupt.as_ref()));
                }
                _ => unreachable!("post-focus race table"),
            },
        );

        let step = harness.dispatch_at_plan_target_for_test(&plan);

        assert!(matches!(step, super::worker::DispatchStep::Continue));
        assert_eq!(calls.load(Ordering::SeqCst), 0, "{race} sent Down");
        assert_eq!(
            sky_dispatch_win32::focus::foreground_query_count(),
            1,
            "{race} query count"
        );
        assert_eq!(
            harness.local_metrics.final_gate_target_changes,
            u64::from(race == "target")
        );
        assert_eq!(
            harness.local_metrics.final_gate_control_rejections,
            u64::from(race != "target")
        );
    }
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
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
