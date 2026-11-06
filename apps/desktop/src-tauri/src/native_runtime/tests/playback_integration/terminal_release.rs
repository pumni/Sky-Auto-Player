use super::*;

#[cfg(feature = "tauri-test")]
#[test]
fn panic_release_before_armed_has_no_physical_release_obligation() {
    use sky_player::engine::{
        DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US, StartupCheckpoint, StartupOrderingHook,
    };

    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::BeforeReady, 1);
    let player = terminal_projection_session(
        DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US,
        30_100_000,
        Some(Arc::clone(&hook)),
    );
    player.arm(0).expect("start panic-before-Armed worker");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(15)));

    player
        .panic_release()
        .expect("cancel pre-Armed physical startup");
    hook.release_startup_checkpoint();
    assert!(
        player
            .join(Duration::from_secs(5))
            .expect("pre-Armed panic worker join")
    );

    let snapshot = player.snapshot();
    assert!(!snapshot.startup_armed);
    assert_eq!(snapshot.final_release_obligation_mask, 0);
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some("panic_release_requested")
    );
    assert!(matches!(
        terminal_release_evidence_after_startup(true, &snapshot),
        TerminalReleaseEvidence::Verified { obligation_mask: 0 }
            | TerminalReleaseEvidence::NotRequired
    ));
    let mut missing_result = snapshot.clone();
    missing_result.release_outcome = None;
    assert_eq!(
        terminal_release_evidence_after_startup(true, &missing_result),
        TerminalReleaseEvidence::NotRequired
    );
}

#[test]
fn recovered_join_error_cannot_be_masked_by_a_requested_stop() {
    let mut service = NativePlaybackService::new(ActivityCoordinator::default());
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    service.diagnostics_gate = Arc::clone(&diagnostics_gate);
    let active = attach_shared_test_presentation(
        active_for_control(PlaybackSessionState::Playing, None),
        &events,
        diagnostics_gate,
        #[cfg(feature = "tauri-test")]
        None,
    );
    active.request_dry_run_terminal(DryRunTerminalRequest::Quit);
    *service.active.lock().expect("active session slot") = Some(active.clone());

    let join_calls = Arc::new(AtomicU64::new(0));
    let hook_calls = Arc::clone(&join_calls);
    service.monitor_join_test_hook = Some(Arc::new(move |_| {
        if hook_calls.fetch_add(1, Ordering::AcqRel) == 0 {
            Err("injected worker join panic".into())
        } else {
            Ok(true)
        }
    }));
    let (activation_sender, activation_receiver) = mpsc::channel();
    activation_sender
        .send(PlaybackActivation::Started)
        .expect("monitor activation receiver");

    service.monitor(Arc::clone(&active), events, activation_receiver);

    assert!(wait_for_playback_done(&active, Duration::from_secs(2)));
    let terminal = service
        .status()
        .expect("terminal status after recovered join error")
        .last_terminal
        .expect("join failure outcome");
    assert_eq!(terminal.state, PlaybackSessionState::Failed);
    assert_eq!(
        terminal.failure_code.as_deref(),
        Some("native_worker_join_failed")
    );
    assert_eq!(join_calls.load(Ordering::Acquire), 2);
}

#[test]
fn shutdown_timeout_keeps_the_physical_reservation_and_omits_release_proof() {
    let activity = ActivityCoordinator::default();
    let service = NativePlaybackService::new(activity.clone());
    let active = active_for_control_with_physical(PlaybackSessionState::Playing, None, true);
    *active.activity_lease.lock().expect("activity lease slot") = Some(
        activity
            .reserve_playback(active.session_id.clone())
            .expect("physical reservation"),
    );
    *service.active.lock().expect("active session slot") = Some(active.clone());

    let shutdown_reported_release =
        service.shutdown(Arc::new(Mutex::new(NativeEventHub::default())));

    assert!(!shutdown_reported_release);
    assert!(!active.done.load(Ordering::Acquire));
    assert!(
        service
            .status()
            .expect("active status")
            .last_terminal
            .is_none()
    );
    assert!(matches!(
        activity.reserve_playback("successor"),
        Err(crate::app_state::ActivityReservationError::PhysicalPlaybackActive)
    ));
}

#[test]
fn pre_exit_release_marker_requires_verified_shutdown_result() {
    let source = include_str!("../../../native_runtime.rs");
    let shutdown_gate = source
        .find("if safety_playback.shutdown(safety_events.clone()) {")
        .expect("verified shutdown gate");
    let release_marker = source
        .find("record_update_safety_phase(\"playback.keys_released\")")
        .expect("release proof marker");
    let admission_marker = source
        .find("record_update_safety_phase(\"activity.admission_closed\")")
        .expect("admission closure marker");
    assert!(admission_marker < shutdown_gate);
    assert!(shutdown_gate < release_marker);
}

#[cfg(feature = "tauri-test")]
#[test]
fn stop_skip_and_shutdown_cancel_the_published_pre_arm_session() {
    use sky_player::engine::{
        EnginePollStatus, FaultInjectionScript, StartupCheckpoint, StartupOrderingHook,
    };

    #[derive(Clone, Copy)]
    enum PendingControl {
        Stop,
        Skip,
        Shutdown,
    }

    for (control, expected_outcome) in [
        (PendingControl::Stop, "quit"),
        (PendingControl::Skip, "skipped"),
        (PendingControl::Shutdown, "quit"),
    ] {
        let activity = ActivityCoordinator::default();
        let mut service = NativePlaybackService::new(activity.clone());
        let worker_hook = Arc::new(StartupOrderingHook::default());
        worker_hook.arm_startup_checkpoint(StartupCheckpoint::ReadyToArmed, 1);
        let captures = Arc::new(Mutex::new(Vec::new()));
        let player = native_handoff_test_player(
            worker_hook,
            Arc::clone(&captures),
            FaultInjectionScript::none(),
        );
        service.test_player = Some(Arc::clone(&player));

        let at_arm = Arc::new(Barrier::new(2));
        let continue_arm = Arc::new(Barrier::new(2));
        let hook_at_arm = Arc::clone(&at_arm);
        let hook_continue_arm = Arc::clone(&continue_arm);
        service.before_arm_test_hook = Some(Arc::new(move || {
            hook_at_arm.wait();
            hook_continue_arm.wait();
        }));
        let (retired_sender, retired_receiver) = mpsc::channel();
        service.monitor_retired_test_hook = Some(Arc::new(move || {
            let _ = retired_sender.send(());
        }));

        let service = Arc::new(service);
        let settings = ApplicationSettings::default();
        let prepared_id = prepare_physical_failure_plan(&service, &settings, &"c".repeat(32));
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let start_service = Arc::clone(&service);
        let start_settings = settings.clone();
        let start_events = Arc::clone(&events);
        let start_thread = thread::spawn(move || {
            start_service.start(
                PlaybackStartRequest {
                    prepared_id,
                    decisions: Vec::new(),
                },
                &start_settings,
                start_events,
            )
        });

        // The start thread has published Starting, reserved physical
        // activity, and spawned its parked monitor, but has not called arm.
        at_arm.wait();
        let active = service
            .active
            .lock()
            .expect("active slot")
            .clone()
            .expect("published active playback");
        let session_id = active.session_id.clone();
        assert_eq!(player.poll_state().status, EnginePollStatus::Ready);
        assert!(matches!(
            activity.reserve_playback("successor-before-arm"),
            Err(crate::app_state::ActivityReservationError::PhysicalPlaybackActive)
        ));

        match control {
            PendingControl::Stop => {
                let ack = service
                    .command("playback.stop", session_id.clone(), Arc::clone(&events))
                    .expect("pre-arm Stop must be acknowledged");
                assert!(ack.accepted);
            }
            PendingControl::Skip => {
                let ack = service
                    .command("playback.skip", session_id.clone(), Arc::clone(&events))
                    .expect("pre-arm Skip must be acknowledged");
                assert!(ack.accepted);
            }
            PendingControl::Shutdown => {
                assert!(!service.shutdown(Arc::clone(&events)));
                assert!(!active.done.load(Ordering::Acquire));
            }
        }

        assert_eq!(player.poll_state().status, EnginePollStatus::Ready);
        continue_arm.wait();
        start_thread
            .join()
            .expect("start command thread")
            .expect("pre-arm cancellation still completes start handoff");
        retired_receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("monitor must retire the cancelled worker");

        assert!(player.join(Duration::ZERO).expect("retired worker join"));
        let snapshot = player.snapshot();
        assert_eq!(snapshot.status, expected_outcome);
        assert_eq!(snapshot.outcome.as_deref(), Some(expected_outcome));
        assert!(!snapshot.startup_armed);
        assert!(captures.lock().expect("mock input capture").is_empty());
        assert!(active.worker_quiescent.load(Ordering::Acquire));
        assert!(active.release_verified.load(Ordering::Acquire));
        let status = service.status().expect("terminal status");
        assert!(status.active.is_none());
        let terminal = status
            .last_terminal
            .expect("authoritative pre-arm terminal result");
        assert_eq!(terminal.state, PlaybackSessionState::Finished);
        assert_eq!(terminal.outcome.as_deref(), Some(expected_outcome));
        assert_eq!(terminal.terminal_release_verified, Some(true));
        assert!(activity.reserve_playback("successor-after-arm").is_ok());
    }
}

#[cfg(feature = "tauri-test")]
#[test]
fn native_worker_cleanup_barrier_keeps_session_exclusive_until_join_and_release() {
    use sky_player::engine::{
        EnginePollStatus, FaultInjectionScript, MockSendInputCapture, StartupCheckpoint,
        StartupOrderingHook,
    };

    let activity = ActivityCoordinator::default();
    let captures = Arc::new(Mutex::new(Vec::<MockSendInputCapture>::new()));
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownCommitted, 1);
    let player = native_handoff_test_player(
        Arc::clone(&hook),
        Arc::clone(&captures),
        FaultInjectionScript::none(),
    );
    let (retired_sender, retired_receiver) = mpsc::channel();
    let mut service = NativePlaybackService::new(activity.clone());
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    service.diagnostics_gate = Arc::clone(&diagnostics_gate);
    service.monitor_retired_test_hook = Some(Arc::new(move || {
        let _ = retired_sender.send(());
    }));
    let service = Arc::new(service);
    let active = attach_shared_test_presentation(
        active_for_control_with_player(
            PlaybackSessionState::Playing,
            None,
            true,
            activity.clone(),
            Some(Arc::clone(&player)),
        ),
        &events,
        diagnostics_gate,
        None,
    );
    *active.activity_lease.lock().expect("activity lease slot") = Some(
        activity
            .reserve_playback(&active.session_id)
            .expect("session A physical reservation"),
    );
    *service.active.lock().expect("active slot") = Some(Arc::clone(&active));

    let (activation_sender, activation_receiver) = mpsc::channel();
    let monitor_service = Arc::clone(&service);
    let monitor_active = Arc::clone(&active);
    let monitor_events = Arc::clone(&events);
    let monitor = thread::spawn(move || {
        monitor_service.monitor(monitor_active, monitor_events, activation_receiver);
    });
    player.arm(0).expect("session A arm");
    activation_sender
        .send(PlaybackActivation::Started)
        .expect("session A monitor activation");

    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(15)));
    let before_cleanup = captures.lock().expect("captured mock packets").clone();
    assert!(before_cleanup.iter().any(|capture| matches!(
        capture,
        MockSendInputCapture::Packet { packet, .. } if packet.down_mask != 0
    )));
    assert!(!before_cleanup.iter().any(|capture| matches!(
        capture,
        MockSendInputCapture::Packet { packet, .. } if packet.up_mask != 0
    )));

    hook.arm_startup_checkpoint(StartupCheckpoint::TerminalCleanup, 1);
    let ack = service
        .command(
            "playback.stop",
            active.session_id.clone(),
            Arc::clone(&events),
        )
        .expect("session A Stop");
    assert!(ack.accepted);
    hook.release_startup_checkpoint();
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(15)));

    assert!(!player.join(Duration::ZERO).expect("bounded join timeout"));
    assert!(!active.worker_quiescent.load(Ordering::Acquire));
    assert!(!active.release_verified.load(Ordering::Acquire));
    assert!(
        service
            .active
            .lock()
            .expect("active slot during cleanup")
            .is_some()
    );
    assert!(matches!(
        activity.reserve_playback("session-b-before-a-cleanup"),
        Err(crate::app_state::ActivityReservationError::PhysicalPlaybackActive)
    ));

    hook.release_startup_checkpoint();
    retired_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("monitor must retire after worker and release completion");
    monitor.join().expect("monitor thread");
    assert!(active.worker_quiescent.load(Ordering::Acquire));
    assert!(active.release_verified.load(Ordering::Acquire));
    assert!(player.join(Duration::ZERO).expect("eventual worker join"));
    let terminal = service
        .status()
        .expect("session A terminal status")
        .last_terminal
        .expect("session A result");
    assert_eq!(terminal.state, PlaybackSessionState::Finished);
    assert_eq!(terminal.outcome.as_deref(), Some("quit"));
    assert_eq!(terminal.terminal_release_verified, Some(true));
    assert_eq!(player.snapshot().status, EnginePollStatus::Quit.as_str());

    let lease_b = activity
        .reserve_playback("session-b-after-a-cleanup")
        .expect("successor reservation after verified release");
    let hook_b = Arc::new(StartupOrderingHook::default());
    hook_b.arm_startup_checkpoint(StartupCheckpoint::FirstDownCommitted, 1);
    let player_b = native_handoff_test_player(
        Arc::clone(&hook_b),
        Arc::clone(&captures),
        FaultInjectionScript::none(),
    );
    player_b.arm(0).expect("session B arm");
    assert!(hook_b.wait_for_startup_checkpoint(Duration::from_secs(15)));
    player_b.quit().expect("session B Stop");
    hook_b.release_startup_checkpoint();
    assert!(
        player_b
            .join(Duration::from_secs(5))
            .expect("session B join")
    );
    drop(lease_b);

    let packet_events = captures
        .lock()
        .expect("all mock packets")
        .iter()
        .flat_map(|capture| match capture {
            MockSendInputCapture::Packet { packet, .. } => {
                let mut events = Vec::new();
                if packet.down_mask != 0 {
                    events.push("down");
                }
                if packet.up_mask != 0 {
                    events.push("up");
                }
                events
            }
            MockSendInputCapture::SingleEmitter { key_up, .. } => {
                vec![if *key_up { "up" } else { "down" }]
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(packet_events, ["down", "up", "down", "up"]);
}

#[cfg(feature = "tauri-test")]
#[test]
fn native_worker_release_error_after_stop_quarantines_successor_admission() {
    use sky_player::engine::{
        EnginePollStatus, FaultInjectionScript, StartupCheckpoint, StartupOrderingHook,
    };

    let activity = ActivityCoordinator::default();
    let captures = Arc::new(Mutex::new(Vec::new()));
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownCommitted, 1);
    let player = native_handoff_test_player(
        Arc::clone(&hook),
        captures,
        FaultInjectionScript::persistent_release(),
    );
    let mut service = NativePlaybackService::new(activity.clone());
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    service.diagnostics_gate = Arc::clone(&diagnostics_gate);
    let (retired_sender, retired_receiver) = mpsc::channel();
    service.monitor_retired_test_hook = Some(Arc::new(move || {
        let _ = retired_sender.send(());
    }));
    let service = Arc::new(service);
    let active = attach_shared_test_presentation(
        active_for_control_with_player(
            PlaybackSessionState::Playing,
            None,
            true,
            activity.clone(),
            Some(Arc::clone(&player)),
        ),
        &events,
        diagnostics_gate,
        None,
    );
    *active.activity_lease.lock().expect("activity lease slot") = Some(
        activity
            .reserve_playback(&active.session_id)
            .expect("session A physical reservation"),
    );
    *service.active.lock().expect("active slot") = Some(Arc::clone(&active));

    let (activation_sender, activation_receiver) = mpsc::channel();
    let monitor_service = Arc::clone(&service);
    let monitor_active = Arc::clone(&active);
    let monitor_events = Arc::clone(&events);
    let monitor = thread::spawn(move || {
        monitor_service.monitor(monitor_active, monitor_events, activation_receiver);
    });
    player.arm(0).expect("session A arm");
    activation_sender
        .send(PlaybackActivation::Started)
        .expect("session A monitor activation");

    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(15)));
    hook.arm_startup_checkpoint(StartupCheckpoint::TerminalCleanup, 1);
    let ack = service
        .command(
            "playback.stop",
            active.session_id.clone(),
            Arc::clone(&events),
        )
        .expect("session A Stop");
    assert!(ack.accepted);
    hook.release_startup_checkpoint();
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(15)));
    assert!(!player.join(Duration::ZERO).expect("bounded join timeout"));
    assert!(matches!(
        activity.reserve_playback("successor-before-error-retirement"),
        Err(crate::app_state::ActivityReservationError::PhysicalPlaybackActive)
    ));

    hook.release_startup_checkpoint();
    retired_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("monitor must publish the worker cleanup error");
    monitor.join().expect("monitor thread");
    assert!(player.join(Duration::ZERO).expect("eventual worker join"));
    assert_eq!(player.poll_state().status, EnginePollStatus::Error);
    let terminal = service
        .status()
        .expect("session A failed terminal status")
        .last_terminal
        .expect("worker error result");
    assert_eq!(terminal.state, PlaybackSessionState::Failed);
    assert_eq!(terminal.terminal_release_verified, Some(false));
    assert!(activity.physical_release_is_quarantined());
    assert!(matches!(
        activity.reserve_playback("successor-after-release-error"),
        Err(crate::app_state::ActivityReservationError::PhysicalReleaseUnverified)
    ));
}

#[test]
fn physical_startup_target_failure_retires_without_reentrant_active_lock() {
    let activity = ActivityCoordinator::default();
    let mut service = NativePlaybackService::new(activity.clone());
    service.startup_failure = Some("no admissible visible Sky window was found".into());
    let service = Arc::new(service);
    let settings = ApplicationSettings::default();
    let events = Arc::new(Mutex::new(NativeEventHub::default()));

    let start_attempt = |prepared_id: String| {
        let service = Arc::clone(&service);
        let settings = settings.clone();
        let events = Arc::clone(&events);
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = service.start(
                PlaybackStartRequest {
                    prepared_id,
                    decisions: Vec::new(),
                },
                &settings,
                events,
            );
            sender.send(result).expect("start result receiver");
        });
        let result = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("target-not-found start must settle promptly");
        worker.join().expect("start worker should return cleanly");
        result.expect_err("physical startup should fail without a Sky window")
    };
    let read_status = || {
        let service = Arc::clone(&service);
        let (sender, receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            sender
                .send(service.status())
                .expect("status result receiver");
        });
        let status = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("playback status must remain responsive after startup failure")
            .expect("playback status should be valid");
        worker.join().expect("status worker should return cleanly");
        status
    };
    let read_retired_status = || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = read_status();
            if status.active.is_none() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "startup failure did not retire its active slot"
            );
            thread::sleep(Duration::from_millis(1));
        }
    };

    let first_prepared_id = prepare_physical_failure_plan(&service, &settings, &"c".repeat(32));
    let first_error = start_attempt(first_prepared_id.clone());
    assert!(
        first_error.starts_with("target_not_found:"),
        "{first_error}"
    );
    let first_status = read_retired_status();
    let first_terminal = first_status
        .last_terminal
        .expect("pre-activation failure should be authoritative");
    assert_eq!(first_terminal.prepared_id, first_prepared_id);
    assert_eq!(first_terminal.state, PlaybackSessionState::Failed);
    assert_eq!(
        first_terminal.failure_code.as_deref(),
        Some("target_not_found")
    );
    assert!(
        first_terminal
            .failure_message
            .as_deref()
            .is_some_and(|message| message.contains("Sky window was not found"))
    );
    assert!(
        events
            .lock()
            .expect("event hub")
            .buffered_for_test()
            .iter()
            .any(
                |event| matches!(event, UiEvent::PlaybackFailed { payload, .. }
                if payload.session_id == first_terminal.session_id
                    && payload.code == "target_not_found")
            )
    );

    let reservation = activity
        .reserve_playback("after-target-not-found")
        .expect("pre-activation failure must release the playback activity lease");
    drop(reservation);

    let service_for_stop = Arc::clone(&service);
    let events_for_stop = Arc::clone(&events);
    let retired_session_id = first_terminal.session_id.clone();
    let (sender, receiver) = mpsc::channel();
    let stop_worker = thread::spawn(move || {
        let result = service_for_stop.command("playback.stop", retired_session_id, events_for_stop);
        sender.send(result).expect("stop result receiver");
    });
    let stop = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("idempotent Stop for a retired session must not hang")
        .expect("idempotent Stop should acknowledge the retired session");
    stop_worker
        .join()
        .expect("idempotent Stop worker should return cleanly");
    assert!(stop.accepted);
    assert_eq!(stop.state, PlaybackSessionState::Failed);

    let second_prepared_id = prepare_physical_failure_plan(&service, &settings, &"d".repeat(32));
    let second_error = start_attempt(second_prepared_id.clone());
    assert!(
        second_error.starts_with("target_not_found:"),
        "{second_error}"
    );
    let second_status = read_retired_status();
    let second_terminal = second_status
        .last_terminal
        .expect("repeated target failure should also retire");
    assert_eq!(second_terminal.prepared_id, second_prepared_id);
    assert_eq!(
        second_terminal.failure_code.as_deref(),
        Some("target_not_found")
    );
}

#[test]
fn terminal_handoff_uses_worker_result_and_release_evidence_after_join() {
    use sky_dispatch_win32::input::ReleaseAllOutcome;
    use sky_player::engine::EnginePollStatus;

    let verified = ReleaseAllOutcome {
        attempted_mask: 1,
        transport_anomaly: false,
        released_successfully: true,
        stuck_mask: 0,
        verification_inconclusive: false,
        attempts: 1,
    };
    let release = terminal_release_evidence(true, Some(&verified), 0);
    assert_eq!(
        release,
        TerminalReleaseEvidence::Verified { obligation_mask: 0 }
    );

    let timeout = decide_playback_handoff(&Ok(false), Some(EnginePollStatus::Quit), None, true);
    assert_eq!(timeout, super::PlaybackHandoffDecision::AwaitQuiescence);
    let join_error = decide_playback_handoff(
        &Err("worker join is unconfirmed".into()),
        Some(EnginePollStatus::Quit),
        None,
        true,
    );
    assert_eq!(join_error, super::PlaybackHandoffDecision::AwaitQuiescence);

    for (status, expected_code) in [
        (EnginePollStatus::Error, "native_player_failed"),
        (EnginePollStatus::Panicked, "native_player_panicked"),
        (EnginePollStatus::Poisoned, "native_player_poisoned"),
        (EnginePollStatus::Invalid, "native_player_failed"),
    ] {
        let publication =
            terminal_publication_after_worker(status, &release, None, None, Some("quit"), 0);
        match publication {
            PlaybackTerminalPublication::Failed { code, .. } => {
                assert_eq!(code, expected_code, "status {}", status.as_str());
            }
            publication => panic!(
                "worker outcome {} was masked by Stop/Skip intent: {publication:?}",
                status.as_str()
            ),
        }
    }

    let stuck = ReleaseAllOutcome {
        attempted_mask: 1,
        transport_anomaly: true,
        released_successfully: false,
        stuck_mask: 1,
        verification_inconclusive: true,
        attempts: 2,
    };
    let unverified = terminal_release_evidence(true, Some(&stuck), 1);
    let quit_with_residue = terminal_publication_after_worker(
        EnginePollStatus::Quit,
        &unverified,
        None,
        None,
        Some("quit"),
        0,
    );
    match quit_with_residue {
        PlaybackTerminalPublication::Failed { code, message } => {
            assert_eq!(code, "terminal_release_unverified");
            assert!(message.contains("residual obligation mask=0x0001"));
            assert!(message.contains("terminal trigger: quit"));
        }
        publication => panic!("unverified release was reported as {publication:?}"),
    }

    let clean_quit = terminal_publication_after_worker(
        EnginePollStatus::Quit,
        &release,
        None,
        None,
        Some("quit"),
        0,
    );
    assert!(matches!(
        clean_quit,
        PlaybackTerminalPublication::Finished { outcome, .. } if outcome == "quit"
    ));

    let clean_suspend = terminal_publication_after_worker(
        EnginePollStatus::SystemSuspended,
        &release,
        None,
        None,
        Some("system_suspend"),
        0,
    );
    assert!(matches!(
        clean_suspend,
        PlaybackTerminalPublication::Failed { code, .. } if code == "system_suspend"
    ));
    let suspend_with_residue = terminal_publication_after_worker(
        EnginePollStatus::SystemSuspended,
        &unverified,
        None,
        None,
        Some("system_suspend"),
        0,
    );
    assert!(matches!(
        suspend_with_residue,
        PlaybackTerminalPublication::Failed { code, .. }
            if code == "terminal_release_unverified"
    ));
}

#[test]
fn joined_worker_with_unverified_release_quarantines_future_physical_activity() {
    let activity = ActivityCoordinator::default();
    let service = NativePlaybackService::new(activity.clone());
    let active = active_for_control_with_physical(PlaybackSessionState::Stopping, None, true);
    *active.activity_lease.lock().expect("activity lease slot") = Some(
        activity
            .reserve_playback(active.session_id.clone())
            .expect("physical session reservation"),
    );
    *service.active.lock().expect("active session slot") = Some(active.clone());
    let retirement = TerminalRetirement {
        quiescence: WorkerQuiescenceProof::WorkerJoined,
        release: TerminalReleaseEvidence::Unverified {
            obligation_mask: 1,
            detail: "residual obligation mask=0x0001".into(),
        },
        publication: PlaybackTerminalPublication::Failed {
            code: "terminal_release_unverified".into(),
            message: "Terminal release could not be verified; residual obligation mask=0x0001"
                .into(),
        },
    };

    release_terminal_ownership(&service.active, &service.last_terminal, &active, retirement)
        .expect("quiescent worker can be retired into quarantine");

    assert!(active.done.load(Ordering::Acquire));
    assert!(!active.release_verified.load(Ordering::Acquire));
    assert!(activity.physical_release_is_quarantined());
    let terminal = service
        .status()
        .expect("terminal status")
        .last_terminal
        .expect("authoritative failed terminal");
    assert_eq!(terminal.state, PlaybackSessionState::Failed);
    assert_eq!(terminal.terminal_release_verified, Some(false));
    assert_eq!(terminal.release_obligation_mask, Some(1));
    assert!(
        terminal
            .failure_message
            .as_deref()
            .is_some_and(|message| message.contains("residual obligation mask=0x0001"))
    );
    assert!(matches!(
        activity.reserve_playback("successor"),
        Err(crate::app_state::ActivityReservationError::PhysicalReleaseUnverified)
    ));
    assert_eq!(
        activity.reserve_calibration().map(|_| ()),
        Err(crate::app_state::ActivityReservationError::PhysicalReleaseUnverified)
    );
    assert_eq!(
        activity.reserve_update().map(|_| ()),
        Err(crate::app_state::ActivityReservationError::PhysicalReleaseUnverified)
    );
}

#[test]
fn join_timeout_keeps_session_exclusive_until_delayed_terminal_cleanup() {
    let activity = ActivityCoordinator::default();
    let mut service = NativePlaybackService::new(activity.clone());
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    service.diagnostics_gate = Arc::clone(&diagnostics_gate);
    let active = attach_shared_test_presentation(
        active_for_control(PlaybackSessionState::Playing, None),
        &events,
        diagnostics_gate,
        #[cfg(feature = "tauri-test")]
        None,
    );
    *active.activity_lease.lock().expect("activity lease slot") = Some(
        activity
            .reserve_playback(active.session_id.clone())
            .expect("session A physical reservation"),
    );
    *service.active.lock().expect("active session slot") = Some(active.clone());

    service
        .command("playback.stop", active.session_id.clone(), events.clone())
        .expect("first Stop request");
    let repeated = service
        .command("playback.stop", active.session_id.clone(), events.clone())
        .expect("repeated Stop is idempotent while stopping");
    assert!(repeated.accepted);

    let join_calls = Arc::new(AtomicU64::new(0));
    let hook_calls = Arc::clone(&join_calls);
    let (timeout_tx, timeout_rx) = mpsc::channel();
    let release_gate = Arc::new((Mutex::new(false), Condvar::new()));
    let hook_gate = Arc::clone(&release_gate);
    let physical_events = Arc::new(Mutex::new(Vec::new()));
    let hook_physical_events = Arc::clone(&physical_events);
    service.monitor_join_test_hook = Some(Arc::new(move |_| {
        if hook_calls.fetch_add(1, Ordering::AcqRel) == 0 {
            timeout_tx.send(()).expect("join timeout receiver");
            return Ok(false);
        }
        let (released, changed) = &*hook_gate;
        let mut released = released.lock().expect("terminal cleanup gate");
        while !*released {
            released = changed.wait(released).expect("terminal cleanup gate wait");
        }
        hook_physical_events
            .lock()
            .expect("physical event trace")
            .push("session-a-terminal-up");
        Ok(true)
    }));
    let service = Arc::new(service);
    let monitor_service = Arc::clone(&service);
    let monitor_active = Arc::clone(&active);
    let monitor_events = Arc::clone(&events);
    let (activation_sender, activation_receiver) = mpsc::channel();
    activation_sender
        .send(super::PlaybackActivation::Started)
        .expect("monitor activation receiver");
    let (monitor_done_sender, monitor_done_receiver) = mpsc::channel();
    let monitor = thread::spawn(move || {
        monitor_service.monitor(monitor_active, monitor_events, activation_receiver);
        monitor_done_sender
            .send(())
            .expect("monitor completion receiver");
    });

    timeout_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("monitor should observe a deterministic join timeout");
    let waiting = service.status().expect("status during join timeout");
    assert_eq!(
        waiting.active.as_ref().map(|active| active.state),
        Some(PlaybackSessionState::Stopping)
    );
    assert!(waiting.last_terminal.is_none());
    assert!(!active.done.load(Ordering::Acquire));
    assert!(matches!(
        activity.reserve_playback("session-b"),
        Err(crate::app_state::ActivityReservationError::PhysicalPlaybackActive)
    ));
    assert!(matches!(
        activity.reserve_calibration(),
        Err(crate::app_state::ActivityReservationError::PhysicalPlaybackActive)
    ));
    assert!(matches!(
        activity.reserve_update(),
        Err(crate::app_state::ActivityReservationError::PhysicalPlaybackActive)
    ));
    assert!(
        physical_events
            .lock()
            .expect("physical event trace")
            .is_empty()
    );

    let (released, changed) = &*release_gate;
    *released.lock().expect("terminal cleanup gate") = true;
    changed.notify_all();
    monitor_done_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("monitor should retire only after delayed cleanup completes");
    monitor.join().expect("monitor thread");

    let session_b = activity
        .reserve_playback("session-b")
        .expect("session B starts after session A is quiescent");
    physical_events
        .lock()
        .expect("physical event trace")
        .push("session-b-down");
    drop(session_b);
    assert_eq!(
        *physical_events.lock().expect("physical event trace"),
        ["session-a-terminal-up", "session-b-down"]
    );
    assert!(active.done.load(Ordering::Acquire));
    assert_eq!(join_calls.load(Ordering::Acquire), 2);
}
