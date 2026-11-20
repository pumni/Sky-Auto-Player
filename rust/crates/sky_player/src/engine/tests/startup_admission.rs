use super::support::*;
use super::*;

#[cfg(feature = "real-input-acceptance")]
#[test]
fn native_acceptance_start_evidence_tracks_worker_armed_publication() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::ArmedPublished, 1);
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
    let session = NativeDispatchSession::new(options).expect("acceptance observation session");

    assert_eq!(
        session
            .native_acceptance_lifecycle_evidence()
            .playback_armed_qpc_ticks,
        None,
        "pre-arm caller time is not a playback admission timestamp"
    );
    session.arm(0).expect("session arm request");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
    let armed_ticks = session
        .native_acceptance_lifecycle_evidence()
        .playback_armed_qpc_ticks
        .expect("worker QPC recorded after successful Armed publication");
    assert!(armed_ticks > 0);

    session.quit().expect("quit at Armed barrier");
    hook.release_startup_checkpoint();
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let terminal_evidence = session.native_acceptance_lifecycle_evidence();
    assert_eq!(
        terminal_evidence.playback_armed_qpc_ticks,
        Some(armed_ticks)
    );
    assert_eq!(terminal_evidence.raw_up_syscall_count, Some(0));
}

#[cfg(feature = "test-support")]
#[test]
fn playback_cpu_baselines_precede_epoch_qpc_without_rebasing_observer_metrics() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::ArmedPublished, 1);
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
    let session = NativeDispatchSession::new(options).expect("CPU ordering session");

    session.arm(0).expect("worker arm");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
    let trace = hook.cpu_baseline_trace_for_test();
    let (startup_order, startup_thread, startup_process) = trace.startup.expect("startup pair");
    let (observer_order, observer_thread, observer_process) =
        trace.observer.expect("observer startup pair");
    let (playback_order, _, _) = trace.playback.expect("playback pair");
    let epoch_qpc_order = trace
        .epoch_qpc_sample_completed
        .expect("selected playback epoch QPC sample");

    assert!(startup_order < observer_order);
    assert!(observer_order < playback_order);
    assert!(playback_order < epoch_qpc_order);
    assert_eq!(
        (observer_thread, observer_process),
        (startup_thread, startup_process),
        "observer metrics retain their pre-Ready CPU attribution pair"
    );
    assert_eq!(send_calls.load(Ordering::Acquire), 0);

    session.quit().expect("quit before dispatch");
    hook.release_startup_checkpoint();
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
}

#[cfg(feature = "test-support")]
#[test]
fn worker_final_gate_rechecks_retained_process_identity_before_down() {
    struct ForegroundOverrideReset;

    impl Drop for ForegroundOverrideReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        }
    }

    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(77)));

    for (identity_status, expected_status, expected_attempts) in [
        (
            sky_dispatch_win32::focus::WindowIdentityContinuity::Continuous,
            EnginePollStatus::Finished,
            2,
        ),
        (
            sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessTerminated,
            EnginePollStatus::Error,
            0,
        ),
    ] {
        let schedule = sky_dispatch_core::compile::compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 10,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "session-owner-final-proof-down".into(),
                },
                KeyActionInput {
                    source_action_index: 11,
                    kind: ActionKind::Up,
                    scheduled_us: 40_000,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "session-owner-final-proof-up".into(),
                },
            ],
            &[0x15],
        )
        .expect("valid identity-gated schedule");
        let send_call_count = Arc::new(AtomicU64::new(0));
        let mut fault_script = FaultInjectionScript::none();
        fault_script.send_call_count = Some(Arc::clone(&send_call_count));
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
        let session = NativeDispatchSession::new(options).expect("test session admission");
        session.bind_initial_target_for_test(123);
        session
            .bind_window_identity_authority_for_test(
                sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 77),
                77,
            )
            .expect("bind target owner proof");
        session.arm(0).expect("session arm");
        assert!(session.set_window_process_identity_status_for_test(identity_status));
        assert!(
            session
                .join(Duration::from_secs(5))
                .expect("bounded worker join")
        );
        let snapshot = session.snapshot();
        assert_eq!(snapshot.status, expected_status.as_str());
        assert_eq!(snapshot.precision_packet_attempts, expected_attempts);
        assert_eq!(send_call_count.load(Ordering::Acquire), expected_attempts);
        if identity_status == sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessTerminated
        {
            assert!(
                snapshot
                    .terminal_error
                    .as_deref()
                    .is_some_and(|error| error.contains("prepared_down_owner_process_terminated"))
            );
        } else {
            assert!(snapshot.terminal_error.is_none());
        }
    }
}

#[cfg(feature = "test-support")]
#[test]
fn dropped_session_owner_leaves_watchdog_to_release_then_drops_target_binding() {
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 20,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "owner-unwind-down".into(),
            },
            KeyActionInput {
                source_action_index: 21,
                kind: ActionKind::Up,
                scheduled_us: 5_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "owner-unwind-later-up".into(),
            },
        ],
        &[0x15],
    )
    .expect("valid owner-unwind schedule");
    let send_call_count = Arc::new(AtomicU64::new(0));
    let captures = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_call_count));
    fault_script.send_input_capture = Some(Arc::clone(&captures));
    let mut options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.wait.supervisor_lease_timeout_us = 300_000;
    let session = NativeDispatchSession::new(options).expect("test session admission");
    session.bind_initial_target_for_test(123);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 77),
            77,
        )
        .expect("bind process authority into session target");
    let weak_authority = session
        .window_identity_authority_weak_for_test()
        .expect("session target owns process authority");
    session.arm(0).expect("session arm");

    let first_down_deadline = Instant::now() + Duration::from_secs(2);
    while send_call_count.load(Ordering::Acquire) == 0 && Instant::now() < first_down_deadline {
        session
            .publish_supervisor_progress()
            .expect("owner keeps lease current before disappearance");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        send_call_count.load(Ordering::Acquire),
        1,
        "first Down sent once"
    );

    drop(session);
    assert!(
        weak_authority.upgrade().is_some(),
        "worker/watchdog shared state retains the target binding after owner drop"
    );
    let terminal_cleanup_deadline = Instant::now() + Duration::from_secs(3);
    while (send_call_count.load(Ordering::Acquire) < 2 || weak_authority.upgrade().is_some())
        && Instant::now() < terminal_cleanup_deadline
    {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        send_call_count.load(Ordering::Acquire),
        2,
        "watchdog wake causes one tracked release attempt after owner disappearance"
    );
    assert!(
        weak_authority.upgrade().is_none(),
        "the retained process object is released after worker and watchdog exit"
    );
    let captures = captures.lock().expect("owner-unwind sender capture");
    assert!(matches!(
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
    ));
}

#[cfg(feature = "test-support")]
#[test]
fn musical_up_only_rechecks_process_identity_and_cleanup_remains_fail_closed() {
    struct ForegroundOverrideReset;

    impl Drop for ForegroundOverrideReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        }
    }

    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(77)));
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 30,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up-only-process-proof-down".into(),
            },
            KeyActionInput {
                source_action_index: 31,
                kind: ActionKind::Up,
                scheduled_us: 40_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up-only-process-proof-up".into(),
            },
        ],
        &[0x15],
    )
    .expect("valid Up-only proof schedule");
    let send_call_count = Arc::new(AtomicU64::new(0));
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownCommitted, 1);
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_call_count));
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
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    let session = NativeDispatchSession::new(options).expect("test session admission");
    session.bind_initial_target_for_test(123);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 77),
            77,
        )
        .expect("bind target owner proof");
    session.arm(0).expect("session arm");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(2)));
    assert_eq!(send_call_count.load(Ordering::Acquire), 1);

    assert!(session.set_window_process_identity_status_for_test(
        sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessTerminated,
    ));
    hook.release_startup_checkpoint();
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(snapshot.precision_packet_attempts, 1);
    assert_eq!(
        send_call_count.load(Ordering::Acquire),
        1,
        "unexpected Up or cleanup syscall after process identity loss: {snapshot:?}"
    );
    assert_eq!(snapshot.final_release_obligation_mask, 1);
    assert!(
        snapshot
            .terminal_error
            .as_deref()
            .is_some_and(|error| error.contains("prepared_down_owner_process_terminated"))
    );
}

#[cfg(feature = "test-support")]
#[test]
fn terminal_tracked_cleanup_uses_owner_proof_without_foreground_requirement() {
    struct ForegroundOverrideReset;

    impl Drop for ForegroundOverrideReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        }
    }

    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(77)));
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 40,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "terminal-owner-proof-down".into(),
            },
            KeyActionInput {
                source_action_index: 41,
                kind: ActionKind::Up,
                scheduled_us: 5_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "terminal-owner-proof-up".into(),
            },
        ],
        &[0x15],
    )
    .expect("valid terminal cleanup schedule");
    let send_call_count = Arc::new(AtomicU64::new(0));
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownCommitted, 1);
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_call_count));
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
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    let session = NativeDispatchSession::new(options).expect("test session admission");
    session.bind_initial_target_for_test(123);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 77),
            77,
        )
        .expect("bind target owner proof");
    session.arm(0).expect("session arm");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(2)));
    assert_eq!(send_call_count.load(Ordering::Acquire), 1);

    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    session.skip().expect("request skip while focus is absent");
    hook.release_startup_checkpoint();
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("bounded worker join")
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(send_call_count.load(Ordering::Acquire), 2);
    assert_eq!(snapshot.final_release_obligation_mask, 1);
}

#[cfg(feature = "test-support")]
fn ownership_review_schedule(up_us: u64) -> sky_dispatch_core::model::RuntimeSchedule {
    sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 700,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "ownership-review-down".into(),
            },
            KeyActionInput {
                source_action_index: 701,
                kind: ActionKind::Up,
                scheduled_us: up_us,
                scan_codes: smallvec::smallvec![0x15],
                reason: "ownership-review-up".into(),
            },
        ],
        &[0x15],
    )
    .expect("valid ownership review schedule")
}

#[cfg(feature = "test-support")]
fn bind_synthetic_owner(session: &NativeDispatchSession, hwnd: isize, pid: u32) {
    session.bind_initial_target_for_test(hwnd);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(hwnd, pid),
            pid,
        )
        .expect("bind synthetic process authority");
}

#[cfg(feature = "test-support")]
#[test]
fn worker_pause_drain_stop_upgrade_preserves_budget_origin_and_residual() {
    use std::sync::mpsc;

    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }

    #[derive(Clone, Copy)]
    enum FirstUp {
        Fail,
        Full,
    }

    #[derive(Clone, Copy)]
    enum Probe {
        Normal,
        Held,
        Inconclusive,
    }

    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;

    // These cases exercise the real NativeDispatchSession worker. The sender
    // gate stops the first Pause Up at the syscall boundary; the test then
    // publishes Stop (and, in one case, origin drift) before allowing the
    // worker to continue its same cleanup invocation.
    for (case, first_up, foreground_drift, probe, total_up, finalizer_up) in [
        ("stop_foreground", FirstUp::Fail, false, Probe::Normal, 4, 0),
        (
            "stop_origin_drift",
            FirstUp::Fail,
            true,
            Probe::Normal,
            2,
            0,
        ),
        ("stop_probe_held", FirstUp::Full, false, Probe::Held, 1, 0),
        (
            "stop_probe_inconclusive",
            FirstUp::Full,
            false,
            Probe::Inconclusive,
            1,
            0,
        ),
    ] {
        const TARGET: isize = 123;
        const OWNER_PID: u32 = 77;
        sky_dispatch_win32::focus::set_foreground_window_for_test(Some(TARGET));
        sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(OWNER_PID)));

        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let capture = Arc::new(std::sync::Mutex::new(Vec::new()));
        let send_call_count = Arc::new(AtomicU64::new(0));
        let force_held_probe = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let force_inconclusive_probe = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let mut fault_script = FaultInjectionScript::none();
        fault_script.entries = match first_up {
            FirstUp::Fail => (1..=16)
                .map(|call| {
                    (
                        call,
                        InjectedSendOutcome::Zero {
                            latency_ticks: 0,
                            win32_error: 5,
                        },
                    )
                })
                .collect(),
            FirstUp::Full => vec![(1, InjectedSendOutcome::Full { latency_ticks: 0 })],
        };
        fault_script.first_release_send_entered = Some(Arc::new(entered_sender));
        fault_script.release_first_release_send =
            Some(Arc::new(std::sync::Mutex::new(release_receiver)));
        fault_script.send_call_count = Some(Arc::clone(&send_call_count));
        fault_script.send_input_capture = Some(Arc::clone(&capture));
        match probe {
            Probe::Normal => {}
            Probe::Held => {
                fault_script.force_held_probe = Some(Arc::clone(&force_held_probe));
            }
            Probe::Inconclusive => {
                fault_script.force_inconclusive_probe = Some(Arc::clone(&force_inconclusive_probe));
            }
        }

        let hook = Arc::new(StartupOrderingHook::default());
        hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownCommitted, 1);
        let mut options = test_session_options(
            ownership_review_schedule(5_000_000),
            1,
            BackendConfig::Mock {
                latency_base_us: 0,
                latency_per_key_us: 0,
                fault_script,
            },
        );
        options.profile = DispatchProfile::MockTest;
        options.focus.require_focus = true;
        options.startup_ordering_hook = Some(Arc::clone(&hook));
        let session = NativeDispatchSession::new_with_bound_window_identity_for_test(
            options,
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                TARGET, OWNER_PID,
            ),
        )
        .expect("bound mock worker session admission");

        session.arm(0).expect("worker arm");
        assert!(
            hook.wait_for_startup_checkpoint(Duration::from_secs(3)),
            "{case}"
        );
        assert_eq!(
            send_call_count.load(Ordering::Acquire),
            1,
            "{case}: one Down"
        );
        session.pause().expect("request Pause");
        hook.release_startup_checkpoint();
        entered_receiver
            .recv_timeout(Duration::from_secs(3))
            .unwrap_or_else(|error| panic!("{case}: Pause Up did not reach the gate: {error}"));

        session.quit().expect("publish Stop inside Pause Drain");
        if foreground_drift {
            sky_dispatch_win32::focus::set_foreground_window_for_test(Some(456));
            sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(88)));
        }
        release_sender
            .send(())
            .unwrap_or_else(|error| panic!("{case}: release Pause Up gate: {error}"));
        assert!(
            session
                .join(Duration::from_secs(5))
                .expect("bounded worker join"),
            "{case}: terminal cleanup must finish"
        );

        let snapshot = session.snapshot();
        let terminal = session.terminal_result().expect("typed terminal result");
        let captures = capture.lock().expect("worker send capture");
        let down_packets = captures
            .iter()
            .filter(|capture| {
                matches!(capture, MockSendInputCapture::Packet { packet, .. } if packet.down_mask != 0)
            })
            .count();
        let down_packet_is_one_full_attempt = captures.iter().any(|capture| {
            matches!(
                capture,
                MockSendInputCapture::Packet {
                    packet,
                    status: sky_dispatch_win32::input::SendTransactionStatus::Complete,
                    confirmed_mask: 1,
                    attempts: 1,
                } if packet.down_mask == 1 && packet.up_mask == 0
            )
        });
        let up_calls = captures
            .iter()
            .filter(|capture| {
                matches!(
                    capture,
                    MockSendInputCapture::SingleEmitter { key_up: true, .. }
                )
            })
            .count();
        let calls_outside_journal = captures.iter().filter(|capture| match capture {
            MockSendInputCapture::Packet { packet, .. } => {
                packet.up_mask & !terminal.residual.outstanding_mask != 0
            }
            MockSendInputCapture::SingleEmitter {
                scan_codes,
                key_up: true,
            } => scan_codes.as_slice() != [0x15],
            MockSendInputCapture::SingleEmitter { .. } => false,
        });
        assert_eq!(down_packets, 1, "{case}: attempted onset was not replayed");
        assert!(down_packet_is_one_full_attempt, "{case}: Down evidence");
        assert_eq!(up_calls, total_up, "{case}: total physical Up calls");
        let pause_drain_up = total_up - finalizer_up;
        assert!(
            pause_drain_up <= 4,
            "{case}: Pause drain keeps its four-call budget"
        );
        assert_eq!(
            send_call_count.load(Ordering::Acquire),
            1 + total_up as u64,
            "{case}: every call is accounted as one Down or one Up"
        );
        assert_eq!(
            calls_outside_journal.count(),
            0,
            "{case}: Up outside journal"
        );
        assert_eq!(snapshot.release_attempts_total, total_up as u64, "{case}");
        assert_eq!(
            snapshot.final_release_obligation_mask, 1,
            "{case}: residual retained"
        );
        assert_eq!(
            snapshot
                .release_outcome
                .as_ref()
                .map(|outcome| outcome.attempts),
            Some(finalizer_up as u8),
            "{case}: finalizer's own bounded drain evidence"
        );
        assert_eq!(terminal.residual.outstanding_mask, 1, "{case}");
        assert_eq!(
            terminal.residual.needs_up_mask,
            u16::from(matches!(first_up, FirstUp::Fail)),
            "{case}"
        );
        assert_eq!(
            terminal.residual.needs_probe_mask,
            u16::from(matches!(first_up, FirstUp::Full)),
            "{case}"
        );
        let terminal_error = terminal
            .terminal_error
            .as_deref()
            .expect("uncertain Pause cleanup is retained as terminal evidence");
        assert!(
            terminal_error.contains("evidence=TerminalReleaseEvidence"),
            "{case}: {terminal_error}"
        );
        assert!(
            terminal_error.contains("started_ticks: Some("),
            "{case}: {terminal_error}"
        );
        assert!(
            terminal_error.contains("completed_ticks: Some("),
            "{case}: {terminal_error}"
        );
        assert!(
            terminal_error.contains("qpc_error: None"),
            "{case}: {terminal_error}"
        );
        assert_eq!(
            terminal_error
                .matches("TerminalSendAttemptEvidence { raw_calls: 1")
                .count(),
            pause_drain_up,
            "{case}: upgraded Pause evidence retains every raw Up attempt"
        );
        match probe {
            Probe::Normal => {
                assert!(
                    terminal_error.contains("inserted_events: 0"),
                    "{case}: {terminal_error}"
                );
                assert!(
                    terminal_error.contains("win32_error: Some(5)"),
                    "{case}: {terminal_error}"
                );
            }
            Probe::Held => assert!(
                terminal_error.contains("Held(1)"),
                "{case}: {terminal_error}"
            ),
            Probe::Inconclusive => assert!(
                terminal_error.contains("Inconclusive(1)"),
                "{case}: {terminal_error}"
            ),
        }
        let owner = terminal
            .residual
            .owner
            .as_ref()
            .expect("residual owner proof");
        assert_eq!(owner.target_hwnd(), TARGET, "{case}");
        assert_eq!(owner.owner_pid(), Some(OWNER_PID), "{case}");
        assert_eq!(owner.target_generation(), 1, "{case}");
        assert!(
            terminal
                .residual
                .profile
                .iter()
                .any(|key| key.scan_code == 0x15 && !key.extended),
            "{case}: recovery profile is retained"
        );
    }
}

#[cfg(feature = "test-support")]
#[test]
fn current_owner_poll_failure_still_fails_closed_before_first_down() {
    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }

    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    let (session, hook, send_calls) = checkpointed_session(
        ownership_review_schedule(100_000),
        StartupCheckpoint::FirstDownAdmission,
        1,
        true,
    );
    bind_synthetic_owner(&session, 123, 77);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(77)));
    session.arm(0).expect("session arm");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(2)));

    assert!(session.set_window_process_identity_status_for_test(
        sky_dispatch_win32::focus::WindowIdentityContinuity::QueryUnavailable,
    ));
    assert_eq!(
        session.poll_window_identity_authority_result(),
        crate::engine::OwnerIdentityPollResult::CurrentInvalid
    );
    hook.release_startup_checkpoint();
    assert!(
        session
            .join(Duration::from_millis(750))
            .expect("bounded worker join")
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(snapshot.precision_packet_attempts, 0);
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
    assert_eq!(snapshot.final_release_obligation_mask, 0);
    assert!(snapshot.terminal_error.is_some());
}

#[cfg(feature = "test-support")]
#[test]
fn target_replacement_during_long_physical_wait_autonomously_stops_held_origin() {
    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }

    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    let (session, hook, send_calls) = checkpointed_session(
        ownership_review_schedule(5_000_000),
        StartupCheckpoint::DispatchWait,
        2,
        true,
    );
    bind_synthetic_owner(&session, 123, 77);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(77)));
    session.arm(0).expect("session arm");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(2)));
    assert_eq!(send_calls.load(Ordering::Acquire), 1);

    session.bind_initial_target_for_test(456);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(456));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(88)));
    let replacement_binding = session.bind_window_identity_authority_for_test(
        sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(456, 88),
        88,
    );
    assert!(
        replacement_binding.is_err(),
        "a live session cannot bind a new owner"
    );
    hook.release_startup_checkpoint();
    assert!(
        session
            .join(Duration::from_millis(750))
            .expect("bounded worker join")
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(snapshot.precision_packet_attempts, 1);
    assert_eq!(send_calls.load(Ordering::Acquire), 1);
    assert_eq!(snapshot.final_release_obligation_mask, 1);
    assert_eq!(snapshot.release_attempts_total, 0);
    assert!(snapshot.terminal_error.is_some());
    assert!(
        hook.interrupted_wait_count() > 0,
        "the native worker's long wait woke on invalidation"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn owner_identity_loss_during_long_wait_wakes_worker_and_preserves_release_obligation() {
    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }

    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    let (session, hook, send_calls) = checkpointed_session(
        ownership_review_schedule(5_000_000),
        StartupCheckpoint::DispatchWait,
        2,
        true,
    );
    bind_synthetic_owner(&session, 123, 77);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(77)));
    session.arm(0).expect("session arm");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(2)));
    assert_eq!(send_calls.load(Ordering::Acquire), 1);

    assert!(session.set_window_process_identity_status_for_test(
        sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessTerminated,
    ));
    assert_eq!(
        session.poll_window_identity_authority_result(),
        crate::engine::OwnerIdentityPollResult::CurrentInvalid
    );
    hook.release_startup_checkpoint();
    assert!(
        session
            .join(Duration::from_millis(750))
            .expect("bounded worker join")
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(snapshot.precision_packet_attempts, 1);
    assert_eq!(
        send_calls.load(Ordering::Acquire),
        1,
        "no musical or cleanup Up after owner loss"
    );
    assert_eq!(snapshot.final_release_obligation_mask, 1);
    assert_eq!(snapshot.release_attempts_total, 0);
    assert!(snapshot.terminal_error.is_some());
    assert!(
        hook.interrupted_wait_count() > 0,
        "the process poll wakes the physical wait"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn same_hwnd_owner_aba_is_terminal_and_never_sends_origin_up_to_replacement() {
    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }

    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    let (session, hook, send_calls) = checkpointed_session(
        ownership_review_schedule(5_000_000),
        StartupCheckpoint::FirstDownCommitted,
        1,
        true,
    );
    bind_synthetic_owner(&session, 123, 77);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(77)));
    session.arm(0).expect("session arm");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(2)));
    assert_eq!(send_calls.load(Ordering::Acquire), 1);

    // Windows may reuse the numeric HWND for a different process. The old
    // retained identity reports drift while foreground reports the new PID.
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(88)));
    assert!(session.set_window_process_identity_status_for_test(
        sky_dispatch_win32::focus::WindowIdentityContinuity::OwnerMismatch,
    ));
    assert_eq!(
        session.poll_window_identity_authority_result(),
        crate::engine::OwnerIdentityPollResult::CurrentInvalid
    );
    hook.release_startup_checkpoint();
    assert!(
        session
            .join(Duration::from_millis(750))
            .expect("bounded worker join")
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(snapshot.precision_packet_attempts, 1);
    assert_eq!(
        send_calls.load(Ordering::Acquire),
        1,
        "replacement owner never receives Up"
    );
    assert_eq!(snapshot.final_release_obligation_mask, 1);
    assert_eq!(snapshot.release_attempts_total, 0);
    assert!(snapshot.terminal_error.is_some());
}

#[cfg(feature = "test-support")]
#[test]
fn target_invalidated_before_scheduler_staging_finishes_without_input() {
    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }

    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(77)));
    let (session, hook, send_calls) = checkpointed_session(
        ownership_review_schedule(100_000),
        StartupCheckpoint::ArmedPublished,
        1,
        true,
    );
    bind_synthetic_owner(&session, 123, 77);
    session.arm(0).expect("session arm");
    if !hook.wait_for_startup_checkpoint(Duration::from_secs(2)) {
        let snapshot = session.snapshot();
        panic!(
            "ArmedPublished checkpoint not reached: status={} trigger={:?} terminal_error={:?}",
            snapshot.status, snapshot.terminal_trigger_reason, snapshot.terminal_error
        );
    }

    session.bind_initial_target_for_test(456);
    hook.release_startup_checkpoint();
    assert!(
        session
            .join(Duration::from_millis(750))
            .expect("bounded worker join")
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(snapshot.precision_packet_attempts, 0);
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
    assert_eq!(snapshot.final_release_obligation_mask, 0);
    assert!(snapshot.terminal_error.is_some());
}

#[cfg(feature = "test-support")]
#[test]
fn stop_skip_panic_and_suspend_interrupt_staged_long_wait_before_input() {
    let cases = [
        ("stop", EnginePollStatus::Quit),
        ("skip", EnginePollStatus::Skipped),
        ("panic", EnginePollStatus::Error),
        ("suspend", EnginePollStatus::SystemSuspended),
    ];
    let long_wait_schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 710,
                kind: ActionKind::Down,
                scheduled_us: 1_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "long-wait-control-down".into(),
            },
            KeyActionInput {
                source_action_index: 711,
                kind: ActionKind::Up,
                scheduled_us: 2_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "long-wait-control-up".into(),
            },
        ],
        &[0x15],
    )
    .expect("valid long-wait control schedule");

    for (control, expected_status) in cases {
        let (session, hook, send_calls) = checkpointed_session(
            long_wait_schedule.clone(),
            StartupCheckpoint::DispatchWait,
            1,
            false,
        );
        session.arm(0).expect("session arm");
        assert!(
            hook.wait_for_startup_checkpoint(Duration::from_secs(2)),
            "{control}"
        );
        assert_eq!(send_calls.load(Ordering::Acquire), 0);

        match control {
            "stop" => session.quit().expect("stop request"),
            "skip" => session.skip().expect("skip request"),
            "panic" => session.panic_release().expect("panic release request"),
            "suspend" => assert!(session.notify_system_power(true)),
            _ => unreachable!(),
        }
        hook.release_startup_checkpoint();
        assert!(
            session
                .join(Duration::from_millis(750))
                .expect("bounded worker join")
        );

        let snapshot = session.snapshot();
        assert_eq!(snapshot.status, expected_status.as_str(), "{control}");
        assert_eq!(snapshot.precision_packet_attempts, 0, "{control}");
        assert_eq!(send_calls.load(Ordering::Acquire), 0, "{control}");
        assert_eq!(snapshot.final_release_obligation_mask, 0, "{control}");
        assert!(
            hook.interrupted_wait_count() > 0,
            "{control} wakes the real wait"
        );
    }
}

#[cfg(all(feature = "test-support", windows))]
#[test]
fn production_require_focus_arm_rejects_missing_failed_wrong_or_late_owner_proof() {
    fn production_session() -> NativeDispatchSession {
        let schedule = sky_dispatch_core::compile::compile_runtime_intents(&[], &[0x15])
            .expect("empty production admission schedule");
        let mut options = test_session_options(schedule, 1, BackendConfig::Production);
        options.focus.require_focus = true;
        NativeDispatchSession::new(options).expect("production focus session admission")
    }

    let missing_target = production_session();
    assert!(missing_target.arm(0).is_err());
    assert_eq!(
        missing_target.spawn_handles_present_for_test(),
        (false, false)
    );

    let missing_owner = production_session();
    missing_owner.bind_initial_target_for_test(123);
    assert!(missing_owner.arm(0).is_err());
    assert_eq!(
        missing_owner.spawn_handles_present_for_test(),
        (false, false)
    );
    assert!(
        missing_owner
            .bind_window_identity_authority_for_test(
                sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 77),
                77,
            )
            .is_err(),
        "proof arriving after rejected Arm cannot reopen admission"
    );
    assert_eq!(
        missing_owner.spawn_handles_present_for_test(),
        (false, false)
    );

    let wrong_owner = production_session();
    wrong_owner.bind_initial_target_for_test(123);
    assert!(
        wrong_owner
            .bind_window_identity_authority_for_test(
                sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 88),
                77,
            )
            .is_err()
    );
    assert!(wrong_owner.arm(0).is_err());
    assert_eq!(wrong_owner.spawn_handles_present_for_test(), (false, false));

    let failed_owner = production_session();
    failed_owner.bind_initial_target_for_test(123);
    failed_owner
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 77),
            77,
        )
        .expect("bind exact test authority");
    assert!(failed_owner.set_window_process_identity_status_for_test(
        sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessTerminated,
    ));
    assert!(failed_owner.arm(0).is_err());
    assert_eq!(
        failed_owner.spawn_handles_present_for_test(),
        (false, false)
    );
}

#[cfg(all(feature = "test-support", windows))]
#[test]
fn production_require_focus_arm_accepts_exact_continuous_bound_owner() {
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&[], &[0x15])
        .expect("empty production admission schedule");
    let mut options = test_session_options(schedule, 1, BackendConfig::Production);
    options.focus.require_focus = true;
    let session = NativeDispatchSession::new(options).expect("production focus session admission");
    session.bind_initial_target_for_test(123);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 77),
            77,
        )
        .expect("bind exact continuous test identity");

    session
        .arm(0)
        .expect("bound target passes production Arm admission");
    assert!(session.spawn_handles_present_for_test().0);
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("bounded worker join")
    );
    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Finished.as_str());
    assert_eq!(snapshot.precision_packet_attempts, 0);
    assert_eq!(snapshot.final_release_obligation_mask, 0);
}

#[cfg(feature = "test-support")]
#[test]
fn zero_release_obligation_needs_no_owner_or_safety_syscall() {
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&[], &[0x15])
        .expect("empty schedule is valid");
    let send_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
    let options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    let session = NativeDispatchSession::new(options).expect("empty session admission");
    session.arm(0).expect("empty session arm");
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("bounded worker join")
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Finished.as_str());
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
    assert_eq!(snapshot.final_release_obligation_mask, 0);
    assert_eq!(snapshot.release_attempts_total, 0);
}

#[test]
fn production_worker_focus_restore_resumes_after_grace_with_mock_sender() {
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
                kind: ActionKind::Down,
                scheduled_us: 10_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "focus-restore-next-down".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 300_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "focus-restore-first-up".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 320_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "focus-restore-next-up".to_string().into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid focus-restore schedule");
    let force_inconclusive_probe = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let send_call_count = Arc::new(AtomicU64::new(0));
    let captures = Arc::new(std::sync::Mutex::new(Vec::new()));
    let startup_hook = Arc::new(StartupOrderingHook::default());
    startup_hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownCommitted, 1);
    let resume_barrier_qpc = Arc::new(AtomicU64::new(0));
    let resume_barrier_capture = Arc::clone(&resume_barrier_qpc);
    let mut fault_script = FaultInjectionScript::none();
    fault_script.force_inconclusive_probe = Some(Arc::clone(&force_inconclusive_probe));
    fault_script.send_call_count = Some(Arc::clone(&send_call_count));
    fault_script.send_input_capture = Some(Arc::clone(&captures));
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
    options.focus.focus_restore_grace_us = 120_000;
    options.startup_ordering_hook = Some(Arc::clone(&startup_hook));
    options.restore_race_hook = Some(Arc::new(move |_, _, barrier| {
        resume_barrier_capture.store(barrier.map_or(0, |ticks| ticks.as_u64()), Ordering::Release);
    }));
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = ForegroundOverrideReset;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(1)));
    let session = NativeDispatchSession::new(options).expect("test session admission");
    session.bind_initial_target_for_test(123);
    session
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 1),
            1,
        )
        .expect("bind mock focus owner");
    session.arm(0).expect("session arm");
    assert!(
        startup_hook.wait_for_startup_checkpoint(Duration::from_secs(2)),
        "first Down did not commit: {:?}",
        session.snapshot()
    );
    assert_eq!(send_call_count.load(Ordering::Acquire), 1);

    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(456));
    force_inconclusive_probe.store(true, Ordering::Release);
    session.set_focus_hint(false);
    startup_hook.release_startup_checkpoint();
    let focus_loss_deadline = Instant::now() + Duration::from_secs(1);
    let mut snapshot = session.snapshot_lite();
    while !snapshot.is_paused && !snapshot.is_finished && Instant::now() < focus_loss_deadline {
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
    assert_eq!(send_call_count.load(Ordering::Acquire), 1);

    let pause_token = session
        .pause_with_timing_token()
        .expect("manual pause request while focus is lost");
    let pause_deadline = Instant::now() + Duration::from_secs(1);
    let mut pause_observed = false;
    while !pause_observed && Instant::now() < pause_deadline {
        pause_observed = session
            .pause_timing_result(pause_token)
            .expect("manual pause timing")
            .is_some();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        pause_observed,
        "manual pause was not observed while focus was lost"
    );
    std::thread::sleep(Duration::from_millis(40));
    assert_eq!(
        send_call_count.load(Ordering::Acquire),
        1,
        "focus loss must not send a musical Up or start cleanup in the background"
    );

    force_inconclusive_probe.store(false, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    session.set_focus_hint(true);
    std::thread::sleep(Duration::from_millis(30));
    assert!(
        session.snapshot_lite().is_paused,
        "focus grace remains active"
    );
    assert_eq!(send_call_count.load(Ordering::Acquire), 1);

    let restore_deadline = Instant::now() + Duration::from_secs(2);
    while resume_barrier_qpc.load(Ordering::Acquire) == 0
        && !snapshot.is_finished
        && Instant::now() < restore_deadline
    {
        session
            .publish_supervisor_progress()
            .expect("heartbeat while focus returns through grace");
        std::thread::sleep(Duration::from_millis(1));
        snapshot = session.snapshot_lite();
    }
    let barrier_qpc = resume_barrier_qpc.load(Ordering::Acquire);
    assert_ne!(
        barrier_qpc, 0,
        "verified focus cleanup published its G barrier"
    );
    assert!(
        session.snapshot_lite().is_paused,
        "manual pause still holds playback"
    );
    assert_eq!(send_call_count.load(Ordering::SeqCst), 2);
    assert_eq!(
        captures
            .lock()
            .expect("mock input capture")
            .as_slice()
            .len(),
        2,
        "the worker captured Down A and the verified focus cleanup Up"
    );

    session
        .resume()
        .expect("manual resume after focus restoration");
    let down_deadline = Instant::now() + Duration::from_secs(2);
    while startup_hook.down_sender_attempt_count() < 2 && Instant::now() < down_deadline {
        session
            .publish_supervisor_progress()
            .expect("heartbeat while waiting for post-cleanup Down");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(startup_hook.down_sender_attempt_count(), 2);
    assert!(
        startup_hook.last_down_sender_start_qpc_ticks() >= barrier_qpc,
        "next Down must not precede the verified cleanup + G barrier"
    );
    assert_eq!(send_call_count.load(Ordering::Acquire), 3);

    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(456));
    session.set_focus_hint(false);
    session.quit().expect("quit restored session");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(snapshot.precision_packet_attempts, 2);
    assert_eq!(snapshot.active_count, 0);
    assert_eq!(snapshot.possibly_active_count, 0);
    assert_eq!(snapshot.sendinput_partial_events, 0);
    assert_eq!(snapshot.generation_accounting.active, 0);
    assert_eq!(snapshot.final_release_obligation_mask, 2);
    assert!(snapshot.generation_accounting.cancelled >= 1);
    let captures = captures.lock().expect("final mock input captures");
    assert!(matches!(
        captures.last(),
        Some(MockSendInputCapture::SingleEmitter { key_up: true, .. })
    ));
    assert_eq!(send_call_count.load(Ordering::SeqCst), 4);
}

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
            StartupCheckpoint::PrepareDiagnosticChunk,
            1,
            false,
        );
        session.arm(0).expect("start actual B worker");
        assert!(
            hook.wait_for_startup_checkpoint(Duration::from_secs(5)),
            "stale packet count {stale_count}"
        );
        assert_eq!(hook.unmatched_up_prepare_count.load(Ordering::SeqCst), 32);
        assert_eq!(send_calls.load(Ordering::Acquire), 0);

        session.quit().expect("late quit accepted");
        hook.release_startup_checkpoint();
        assert!(session.join(Duration::from_secs(5)).expect("B worker join"));
        let snapshot = session.snapshot();
        assert_eq!(snapshot.status, EnginePollStatus::Quit.as_str());
        assert_eq!(snapshot.outcome.as_deref(), Some("quit"));
        assert_eq!(hook.unmatched_up_prepare_count.load(Ordering::SeqCst), 32);
        assert_eq!(
            send_calls.load(Ordering::Acquire),
            0,
            "final Down must not send after late quit with {stale_count} stale packets"
        );
    }
}

#[cfg(all(feature = "test-support", windows))]
#[test]
fn actual_b_worker_records_stale_diagnostics_during_prepare_and_continues_dispatch() {
    let hook = Arc::new(StartupOrderingHook::default());
    let send_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
    let mut options = test_session_options(
        post_wake_stale_metadata_schedule(1),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.wait.test_spin_threshold_us = None;
    options.wait.test_wait_policy = super::TestWaitPolicy::ProductionCalibrated;
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    let session = NativeDispatchSession::new(options).expect("actual B worker session");

    session.arm(0).expect("worker arm");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(session.snapshot().outcome.as_deref(), Some("finished"));
    assert_eq!(hook.unmatched_up_prepare_count.load(Ordering::Acquire), 1);
    assert_eq!(send_calls.load(Ordering::Acquire), 6);
    assert_eq!(hook.down_sender_attempt_count(), 3);
    let (_, authored_target, packet_not_before, _) = hook.last_down_sender_qpc_evidence();
    assert!(authored_target.is_some());
    assert!(packet_not_before.is_some());
}

#[cfg(all(feature = "test-support", windows))]
#[test]
fn first_down_committed_checkpoint_follows_successful_backend_send() {
    use super::MockSendInputCapture;

    let send_calls = Arc::new(AtomicU64::new(0));
    let captures = Arc::new(std::sync::Mutex::new(Vec::new()));
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownCommitted, 1);
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
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
    let session = NativeDispatchSession::new(options).expect("actual B worker session");

    session.arm(TEST_WALL_CLOCK_PREROLL_US).expect("worker arm");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
    assert_eq!(send_calls.load(Ordering::Acquire), 1);
    assert!(matches!(
        captures.lock().expect("mock input captures").as_slice(),
        [MockSendInputCapture::Packet {
            packet: PhysicalPacket {
                down_mask: 1,
                up_mask: 0
            },
            ..
        }]
    ));

    session
        .quit()
        .expect("quit while first Down is checkpointed");
    hook.release_startup_checkpoint();
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(send_calls.load(Ordering::Acquire), 2);
    let captures = captures.lock().expect("terminal mock input captures");
    assert!(matches!(
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
    ));
}

#[cfg(all(feature = "test-support", windows))]
#[test]
fn prepared_schedule_passes_session_admission_and_worker_arm_without_rematerializing() {
    let schedule = startup_boundary_schedule();
    let qpc = sky_dispatch_win32::clock::QpcClock::initialize().expect("QPC clock");
    let prepared = sky_dispatch_core::executor::PreparedDispatchSchedule::prepare(
        &schedule,
        qpc.frequency_hz(),
        500,
        17_167,
        0,
    )
    .expect("Prepare materializes the physical schedule");
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::ArmedPublished, 1);
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
    options.schedule = prepared;
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    let session = NativeDispatchSession::new(options).expect("prepared session admission");

    session.arm(0).expect("worker arm request");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
    assert_eq!(hook.prepared_schedule_executor_consumption_count(), 1);
    assert_eq!(send_calls.load(Ordering::Acquire), 0);

    session
        .quit()
        .expect("quit at successful Armed publication");
    hook.release_startup_checkpoint();
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(session.snapshot().outcome.as_deref(), Some("quit"));
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
}

#[test]
fn prepared_schedule_cancellation_completes_before_session_construction() {
    let schedule = large_startup_materialization_schedule(5_000);
    let qpc = sky_dispatch_win32::clock::QpcClock::initialize().expect("QPC clock");
    let mut checkpoints = 0;
    let result = sky_dispatch_core::executor::PreparedDispatchSchedule::prepare_with_cancellation(
        &schedule,
        qpc.frequency_hz(),
        500,
        17_167,
        schedule.compile_memory.caller_retained_capacity_bytes,
        || {
            checkpoints += 1;
            if checkpoints == 8 {
                Err("test cancellation during Prepare".to_string())
            } else {
                Ok(())
            }
        },
    );
    assert!(
        result
            .unwrap_err()
            .contains("test cancellation during Prepare")
    );
    assert_eq!(checkpoints, 8);
}

#[cfg(all(feature = "test-support", windows))]
#[test]
fn actual_b_worker_publishes_interrupted_wait_before_resumed_packet_dispatch() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 150_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "interrupted-physical-wait-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 190_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "interrupted-physical-wait-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 250_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "interrupted-physical-wait-resumed-down".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 300_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "interrupted-physical-wait-resumed-up".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("interrupted-wait regression schedule");
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::PhysicalWait, 2);
    let mut options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.profile = DispatchProfile::Production;
    options.timing_observer_for_qualification = true;
    options.telemetry.mode = TelemetryMode::Ring;
    options.telemetry.capacity = 128;
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    let session = NativeDispatchSession::new(options).expect("actual B worker session");

    session.arm(0).expect("worker arm");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
    let (staged_sequence, staged_authored_target, staged_not_before) =
        hook.last_down_window_snapshot();
    assert!(
        staged_sequence > 0,
        "B worker staged a Down window before its physical wait"
    );
    assert!(staged_authored_target > 0);
    assert!(staged_not_before >= staged_authored_target);
    session.pause().expect("interrupt physical wait with pause");
    hook.release_startup_checkpoint();
    let pause_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot_lite();
        if snapshot.is_paused {
            break;
        }
        assert!(
            !snapshot.is_finished,
            "interrupted physical wait lost the worker: {snapshot:?}; terminal_error={:?}",
            session.snapshot().terminal_error
        );
        assert!(
            Instant::now() < pause_deadline,
            "pause acknowledgment timed out: {snapshot:?}"
        );
        session
            .publish_supervisor_progress()
            .expect("heartbeat while waiting for pause");
        std::thread::yield_now();
    }

    let interrupted = hook
        .last_interrupted_wait_evidence()
        .expect("actual B worker interrupted-wait evidence");
    assert!(
        hook.interrupted_wait_count() >= 1,
        "worker recorded at least one interrupted physical wait"
    );
    assert_eq!(
        interrupted.outcome,
        sky_dispatch_win32::wait::WaitOutcome::Interrupted
    );
    assert!(interrupted.planned_wait_ticks > 0);
    assert!(interrupted.spin_ticks <= interrupted.planned_wait_ticks);
    assert_eq!(interrupted.wake_qpc_ticks, None);
    assert_eq!(interrupted.deadline_crossing_qpc_ticks, None);
    assert_eq!(interrupted.waiter_return_qpc_ticks, None);
    assert!(interrupted.physical_target_qpc_ticks > 0);

    session.resume().expect("resume after interrupted wait");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    let (resumed_sequence, resumed_authored_target, resumed_not_before) =
        hook.last_down_window_snapshot();
    assert!(resumed_sequence > staged_sequence);
    assert!(resumed_authored_target > 0);
    assert!(resumed_not_before >= resumed_authored_target);
    assert!(snapshot.wait_interrupted_count >= 1);
    assert!(snapshot.spin_observation_expected_samples >= 2);
    assert_eq!(snapshot.spin_observation_dropped_samples, 0);
    assert_eq!(snapshot.spin_observation_missing_samples, 0);
    assert_eq!(snapshot.spin_observation_pending_samples, 0);

    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("worker telemetry"))
            .expect("telemetry JSON");
    let down = telemetry["records"]
        .as_array()
        .expect("trace records")
        .iter()
        .find(|record| record["down_mask"].as_u64().unwrap_or_default() != 0)
        .expect("resumed physical Down trace");
    assert_ne!(down["wait_sequence"].as_u64(), Some(interrupted.sequence));
    assert_eq!(down["waiter_return_qpc_available"], false);
    assert_eq!(down["waiter_return_qpc_provenance"], "unavailable");
}

#[cfg(all(feature = "test-support", windows))]
#[test]
fn actual_b_worker_timestamp_hook_keeps_mixed_floor_held_down_targets_distinct() {
    let actions = [
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 5_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "timestamp-floor-initial-down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Down,
            scheduled_us: 6_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "timestamp-floor-secondary-down".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: 25_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "timestamp-floor-initial-up".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 45_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "timestamp-floor-mixed-up".into(),
        },
        KeyActionInput {
            source_action_index: 4,
            kind: ActionKind::Down,
            scheduled_us: 45_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "timestamp-floor-mixed-down".into(),
        },
        KeyActionInput {
            source_action_index: 5,
            kind: ActionKind::Up,
            scheduled_us: 80_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "timestamp-floor-final-up".into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15, 0x16])
        .expect("mixed floor-held timestamp schedule");
    assert!(schedule.packets.iter().any(|packet| {
        packet.scheduled_us == 45_000 && packet.up_mask != 0 && packet.down_mask != 0
    }));
    let hook = Arc::new(StartupOrderingHook::default());
    let mut options = test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 30_000,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.wait.test_spin_threshold_us = None;
    options.wait.test_wait_policy = super::TestWaitPolicy::ProductionCalibrated;
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    let session = NativeDispatchSession::new(options).expect("actual B worker session");

    session.arm(0).expect("worker arm");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(session.snapshot().outcome.as_deref(), Some("finished"));
    assert_eq!(hook.down_sender_attempt_count(), 3);
    let (sender_start, authored_target, packet_not_before, _) =
        hook.last_down_sender_qpc_evidence();
    let sender_start = sender_start.expect("mixed Down sender pre-call QPC");
    let authored_target = authored_target.expect("mixed Down authored QPC");
    let packet_not_before = packet_not_before.expect("mixed Down eligibility QPC");
    assert!(
        packet_not_before > authored_target,
        "floor-held mixed Down must preserve separate authored and packet eligibility QPCs"
    );
    let authored_to_pre_call = i128::from(sender_start) - i128::from(authored_target);
    let eligibility_to_pre_call = i128::from(sender_start) - i128::from(packet_not_before);
    assert_ne!(
        authored_to_pre_call, eligibility_to_pre_call,
        "floor-held mixed Down must preserve separate pre-call residuals"
    );
}

#[cfg(all(feature = "test-support", windows))]
#[test]
fn actual_b_worker_timestamp_hook_records_up_only_sender_provenance() {
    let actions = [
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 5_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "timestamp-up-only-initial-down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 35_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "timestamp-up-only-release".into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("Up-only timestamp schedule");
    assert!(schedule.packets.iter().any(|packet| {
        packet.up_mask != 0 && packet.down_mask == 0 && packet.scheduled_us == 35_000
    }));

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
    options.wait.test_spin_threshold_us = None;
    options.wait.test_wait_policy = super::TestWaitPolicy::ProductionCalibrated;
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    let session = NativeDispatchSession::new(options).expect("actual B worker session");

    session.arm(0).expect("worker arm");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(session.snapshot().outcome.as_deref(), Some("finished"));
    assert_eq!(hook.down_sender_attempt_count(), 1);

    let (sender_start, authored_target, packet_not_before) = hook
        .up_only_sender_attempt_evidence()
        .expect("Up-only sender timing evidence");
    let sender_start = sender_start.expect("sender measured pre-call QPC");
    assert_eq!(authored_target, packet_not_before);
    assert!(sender_start >= packet_not_before);
}

#[cfg(all(feature = "test-support", windows))]
#[test]
fn actual_b_worker_prepare_diagnostic_matrix_has_no_runtime_stale_scan() {
    use serde_json::json;

    let mut scenarios = Vec::new();
    for unmatched_up_count in [0usize, 1, 32, 33, 10_000] {
        let hook = Arc::new(StartupOrderingHook::default());
        let mut options = test_session_options(
            post_wake_stale_metadata_schedule(unmatched_up_count),
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
        options.telemetry.capacity = 64;
        let session = NativeDispatchSession::new(options).expect("actual B worker session");
        session.arm(0).expect("worker arm");
        assert!(session.join(Duration::from_secs(10)).expect("worker join"));
        assert_eq!(session.snapshot().outcome.as_deref(), Some("finished"));
        assert_eq!(
            hook.unmatched_up_prepare_count.load(Ordering::Acquire),
            unmatched_up_count as u64
        );
        assert_eq!(hook.down_sender_attempt_count(), 3);
        scenarios.push(json!({
            "unmatched_up_count": unmatched_up_count,
            "prepare_diagnostic_count": hook.unmatched_up_prepare_count.load(Ordering::Acquire),
            "physical_down_attempt_count": hook.down_sender_attempt_count(),
        }));
    }

    eprintln!(
        "prepare_diagnostic_matrix_json={}",
        serde_json::to_string(&json!({
            "evidence_scope": "actual_b_worker_test_support_mock_sender",
            "runtime_stale_scan": false,
            "scenarios": scenarios,
        }))
        .expect("prepare diagnostic matrix JSON")
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
        if kind == "target" {
            session.bind_initial_target_for_test(123);
        }
        session.arm(100_000).expect("worker arm request");
        assert!(
            hook.wait_for_startup_checkpoint(Duration::from_secs(5)),
            "{kind}"
        );
        match kind {
            "skip" => session.skip().expect("skip request"),
            "panic" => session.panic_release().expect("panic release request"),
            "suspend" => assert!(session.notify_system_power(true)),
            "target" => session.bind_initial_target_for_test(456),
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
    session.bind_initial_target_for_test(123);
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
    session.bind_initial_target_for_test(123);
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

#[test]
fn guarded_calibration_qpc_origin_failure_fails_closed_without_reusing_stale_now() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.fail_next_guarded_calibration_origin();
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
    options.startup_calibration_placement =
        crate::engine::StartupCalibrationPlacement::AfterSchedulingGuards;
    options.wait.test_wait_policy = crate::engine::TestWaitPolicy::ProductionCalibrated;
    options.startup_ordering_hook = Some(hook);
    let session = NativeDispatchSession::new(options).expect("session admission");
    session.arm(100_000).expect("worker startup request");
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("calibration failure join")
    );

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, EnginePollStatus::Error.as_str());
    assert_eq!(snapshot.outcome.as_deref(), Some("error"));
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some("startup_failure")
    );
    assert!(
        snapshot
            .terminal_error
            .as_ref()
            .unwrap()
            .contains("guarded calibration")
    );
    assert_eq!(send_calls.load(Ordering::Acquire), 0);
}
