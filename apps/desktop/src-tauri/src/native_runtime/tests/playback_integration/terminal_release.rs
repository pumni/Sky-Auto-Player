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
    let compile_checkpoints = Arc::new(AtomicU64::new(0));
    let compile_checkpoints_hook = Arc::clone(&compile_checkpoints);
    *service
        .prepare_compile_test_hook
        .lock()
        .expect("Prepare compiler hook") = Some(Arc::new(move |_| {
        compile_checkpoints_hook.fetch_add(1, Ordering::AcqRel);
    }));
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
    let first_prepare_compilations = compile_checkpoints.load(Ordering::Acquire);
    assert!(first_prepare_compilations > 0);
    {
        let prepared = service.prepared.lock().expect("prepared cache");
        let plan = prepared
            .iter()
            .find(|(id, _)| id == &first_prepared_id)
            .and_then(|(_, record)| {
                record
                    .variants
                    .get(&crate::commands::PlaybackDecision::Proceed)
            })
            .and_then(|variant| variant.prepared_schedule.as_ref())
            .expect("Start must consume a prepared physical runtime");
        assert!(plan.has_down());
    }
    let first_error = start_attempt(first_prepared_id.clone());
    assert_eq!(
        compile_checkpoints.load(Ordering::Acquire),
        first_prepare_compilations,
        "Start must not compile or materialize the authored schedule"
    );
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
    let second_prepare_compilations = compile_checkpoints.load(Ordering::Acquire);
    assert!(second_prepare_compilations > first_prepare_compilations);
    let second_error = start_attempt(second_prepared_id.clone());
    assert_eq!(
        compile_checkpoints.load(Ordering::Acquire),
        second_prepare_compilations,
        "a second Start must not compile or materialize the authored schedule"
    );
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

#[cfg(feature = "tauri-test")]
fn quarantined_recovery_fixture(
    session_id: &str,
    foreground_required: bool,
    needs_up_mask: u16,
    needs_probe_mask: u16,
) -> (
    NativePlaybackService,
    Arc<NativeActivePlayback>,
    Arc<sky_dispatch_win32::focus::WindowIdentityAuthority>,
) {
    use sky_player::engine::{
        TerminalReleaseResidual, WorkerTerminalCause, WorkerTerminalOutcome, WorkerTerminalResult,
    };

    let activity = ActivityCoordinator::default();
    let service = NativePlaybackService::new(activity.clone());
    let mut active = active_for_control_with_physical(PlaybackSessionState::Stopping, None, true);
    let active_mut = Arc::get_mut(&mut active).expect("unique recovery fixture active slot");
    active_mut.activity = activity.clone();
    active_mut.session_id = session_id.to_string();
    let hwnd = 0x7788;
    let owner_pid = 0x1199;
    let authority =
        sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(hwnd, owner_pid);
    let authority = Arc::new(authority);
    let owner = sky_dispatch_win32::input::ReleaseOwnerProof::new_with_foreground_policy(
        hwnd,
        17,
        Some(owner_pid),
        Some(Arc::clone(&authority)),
        foreground_required,
    )
    .expect("recovery owner proof");
    let materialized = sky_dispatch_win32::input::MaterializedInstrumentKeyProfile::canonical();
    let profile = std::array::from_fn(|slot| materialized.physical_key(slot));
    let outstanding_mask = needs_up_mask | needs_probe_mask;
    *active_mut.terminal_result.lock().expect("typed result") = Some(WorkerTerminalResult {
        outcome: WorkerTerminalOutcome::Error,
        cause: WorkerTerminalCause::SenderAnomaly,
        terminal_error: Some("injected terminal sender anomaly".into()),
        residual: TerminalReleaseResidual {
            outstanding_mask,
            needs_up_mask,
            needs_probe_mask,
            owner: Some(owner),
            profile,
        },
    });
    active_mut.worker_quiescent.store(true, Ordering::Release);
    *active_mut.activity_lease.lock().expect("activity lease") = Some(
        activity
            .reserve_playback(session_id)
            .expect("reserve recovery session"),
    );
    *service.active.lock().expect("active slot") = Some(Arc::clone(&active));
    release_terminal_ownership(
        &service.active,
        &service.last_terminal,
        &active,
        TerminalRetirement {
            quiescence: WorkerQuiescenceProof::WorkerJoined,
            release: TerminalReleaseEvidence::Unverified {
                obligation_mask: outstanding_mask,
                detail: "injected unverified release".into(),
            },
            publication: PlaybackTerminalPublication::Failed {
                code: "terminal_release_unverified".into(),
                message: "release remains quarantined".into(),
            },
        },
    )
    .expect("joined session commits terminal quarantine");
    (service, active, authority)
}

#[cfg(feature = "tauri-test")]
#[test]
fn post_join_recovery_observes_focus_return_after_terminal_publication_without_winevent() {
    let _send_lock = recovery_test_send_lock();
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let session_id = "a".repeat(32);
    let (service, active, _authority) = quarantined_recovery_fixture(&session_id, false, 0x0001, 0);
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let terminal_publication = PlaybackTerminalPublication::Failed {
        code: "terminal_release_unverified".into(),
        message: "release remains quarantined".into(),
    };

    // The worker has joined and the session-id quarantine is committed before
    // the terminal event. Recovery starts only after this event boundary.
    assert!(active.worker_quiescent.load(Ordering::Acquire));
    assert!(active.done.load(Ordering::Acquire));
    publish_terminal_event(&events, &active, &terminal_publication)
        .expect("publish terminal event before post-join focus return");
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    reset_recovery_test_raw_sends();
    service
        .post_join_recovery
        .start_without_winevent_for_test(active.clone(), service.last_terminal.clone())
        .expect("start persistent polling recovery owner");
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(recovery_test_raw_sends(), 0, "background must not send Up");

    // This fixture deliberately disables WinEvent. The exact foreground poll
    // must discover the return and trigger one bounded attempt.
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(0x7788));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(0x1199)));
    let deadline = Instant::now() + Duration::from_secs(2);
    while recovery_test_raw_sends() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(recovery_test_raw_sends(), 1, "focus return gets one raw Up");

    // Repeated polls/duplicate hints while the original target stays
    // foreground are the same transition, not new recovery attempts.
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(0x7788));
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(recovery_test_raw_sends(), 1);
    assert!(service.activity.physical_release_is_quarantined());
    assert!(matches!(
        service
            .activity
            .reserve_playback("successor-after-probe-held"),
        Err(crate::app_state::ActivityReservationError::PhysicalReleaseUnverified)
    ));
    service.post_join_recovery.shutdown();
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
}

#[cfg(feature = "tauri-test")]
#[test]
fn monitor_starts_post_join_recovery_after_stop_retirement_and_later_focus_return() {
    use sky_player::engine::{
        FaultInjectionScript, MockSendInputCapture, StartupCheckpoint, StartupOrderingHook,
    };

    let _send_lock = recovery_test_send_lock();
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    sky_dispatch_win32::focus::set_foreground_owner_for_test(None);

    let activity = ActivityCoordinator::default();
    let mut service = NativePlaybackService::new(activity.clone());
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    service.diagnostics_gate = Arc::clone(&diagnostics_gate);
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::FirstDownCommitted, 1);
    let player = native_handoff_test_player(
        Arc::clone(&hook),
        Arc::new(Mutex::new(Vec::<MockSendInputCapture>::new())),
        FaultInjectionScript::persistent_release(),
    );
    player.bind_initial_target_for_test(123);
    player
        .bind_window_identity_authority_for_test(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 77),
            77,
        )
        .expect("bind the original recovery authority");
    let mut active_value = terminal_projection_active(Arc::clone(&player));
    active_value.target_hwnd = Some(123);
    active_value.activity = activity.clone();
    active_value.activity_lease = Mutex::new(Some(
        activity
            .reserve_playback(&active_value.session_id)
            .expect("physical session reservation"),
    ));
    let (retired_sender, retired_receiver) = mpsc::channel();
    service.monitor_retired_test_hook = Some(Arc::new(move || {
        let _ = retired_sender.send(());
    }));
    let active =
        attach_shared_test_presentation(Arc::new(active_value), &events, diagnostics_gate, None);
    *service.active.lock().expect("active slot") = Some(Arc::clone(&active));
    let service = Arc::new(service);

    let (activation_sender, activation_receiver) = mpsc::channel();
    let monitor_service = Arc::clone(&service);
    let monitor_active = Arc::clone(&active);
    let monitor_events = Arc::clone(&events);
    let monitor = thread::spawn(move || {
        monitor_service.monitor(monitor_active, monitor_events, activation_receiver);
    });
    player.arm(0).expect("arm physical mock worker");
    activation_sender
        .send(PlaybackActivation::Started)
        .expect("monitor activation");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));

    hook.arm_startup_checkpoint(StartupCheckpoint::TerminalCleanup, 1);
    let ack = service
        .command(
            "playback.stop",
            active.session_id.clone(),
            Arc::clone(&events),
        )
        .expect("Stop active physical session");
    assert!(ack.accepted);
    hook.release_startup_checkpoint();
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
    assert!(
        !player
            .join(Duration::ZERO)
            .expect("terminal cleanup is gated")
    );
    assert!(!active.worker_quiescent.load(Ordering::Acquire));
    assert!(matches!(
        activity.reserve_playback("successor-before-monitor-join"),
        Err(crate::app_state::ActivityReservationError::PhysicalPlaybackActive)
    ));

    reset_recovery_test_raw_sends();
    hook.release_startup_checkpoint();
    retired_receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("terminal presentation follows joined quarantine commit");
    monitor.join().expect("monitor thread");
    assert!(active.worker_quiescent.load(Ordering::Acquire));
    assert!(active.done.load(Ordering::Acquire));
    assert!(activity.physical_release_is_quarantined());
    assert_eq!(
        service
            .status()
            .expect("retired terminal status")
            .last_terminal
            .expect("terminal publication committed")
            .terminal_release_verified,
        Some(false)
    );
    assert_eq!(
        recovery_test_raw_sends(),
        0,
        "background Stop must not send Up"
    );

    // The monitor must have transferred recovery ownership before it returned.
    // A later exact foreground poll should trigger one attempt even though the
    // real desktop foreground and WinEvent hook are not used by this fixture.
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(123));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(77)));
    let deadline = Instant::now() + Duration::from_secs(2);
    while recovery_test_raw_sends() == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(recovery_test_raw_sends(), 1);
    thread::sleep(Duration::from_millis(150));
    assert_eq!(
        recovery_test_raw_sends(),
        1,
        "polls do not replay the same edge"
    );
    assert!(activity.physical_release_is_quarantined());
    assert!(matches!(
        activity.reserve_playback("successor-after-monitor-recovery"),
        Err(crate::app_state::ActivityReservationError::PhysicalReleaseUnverified)
    ));

    service.post_join_recovery.shutdown();
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
}

#[cfg(feature = "tauri-test")]
#[test]
fn post_join_recovery_rejects_foreign_foreground_owner_drift_and_prejoin_start() {
    let _send_lock = recovery_test_send_lock();
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let (service, active, authority) =
        quarantined_recovery_fixture("post-join-origin-reject", false, 0x0001, 0);
    reset_recovery_test_raw_sends();

    active.worker_quiescent.store(false, Ordering::Release);
    active.done.store(false, Ordering::Release);
    assert!(
        service
            .post_join_recovery
            .start_without_winevent_for_test(active.clone(), service.last_terminal.clone())
            .is_err()
    );
    assert_eq!(recovery_test_raw_sends(), 0);
    active.worker_quiescent.store(true, Ordering::Release);
    active.done.store(true, Ordering::Release);

    // A different foreground HWND cannot authorize the old session's Up.
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(0x9999));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(0x2222)));
    service
        .post_join_recovery
        .start_without_winevent_for_test(active.clone(), service.last_terminal.clone())
        .expect("start recovery owner against original authority");
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(recovery_test_raw_sends(), 0);

    // Reusing the original HWND with a different PID is invalid origin, not a
    // foreground return. The recovery owner tears down and keeps quarantine.
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(0x7788));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(0x2222)));
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(recovery_test_raw_sends(), 0);
    assert!(service.activity.physical_release_is_quarantined());
    authority.set_process_identity_status_for_test(
        sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessTerminated,
    );
    service.post_join_recovery.shutdown();
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
}

#[test]
fn retirement_commit_failure_publishes_failed_status_and_retains_quarantine() {
    use sky_player::engine::{
        TerminalReleaseResidual, WorkerTerminalCause, WorkerTerminalOutcome, WorkerTerminalResult,
    };

    let activity = ActivityCoordinator::default();
    let service = NativePlaybackService::new(activity.clone());
    let mut active = active_for_control_with_physical(PlaybackSessionState::Stopping, None, true);
    let active_mut = Arc::get_mut(&mut active).expect("unique failing-retirement session");
    active_mut.activity = activity.clone();
    active_mut.session_id = "b".repeat(32);
    let authority = Arc::new(
        sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(0x7711, 0x1199),
    );
    let owner = sky_dispatch_win32::input::ReleaseOwnerProof::new(
        0x7711,
        31,
        Some(0x1199),
        Some(authority),
    )
    .expect("original owner proof");
    let materialized = sky_dispatch_win32::input::MaterializedInstrumentKeyProfile::canonical();
    *active_mut.terminal_result.lock().expect("terminal result") = Some(WorkerTerminalResult {
        outcome: WorkerTerminalOutcome::Error,
        cause: WorkerTerminalCause::SenderAnomaly,
        terminal_error: Some("injected worker cause".into()),
        // Invalid mask partition forces quarantine storage to fail while the
        // physical reservation itself remains fail-closed.
        residual: TerminalReleaseResidual {
            outstanding_mask: 1,
            needs_up_mask: 0,
            needs_probe_mask: 0,
            owner: Some(owner),
            profile: std::array::from_fn(|slot| materialized.physical_key(slot)),
        },
    });
    active_mut.worker_quiescent.store(true, Ordering::Release);
    *active_mut.activity_lease.lock().expect("activity lease") = Some(
        activity
            .reserve_playback("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .expect("reserve physical session"),
    );
    *service.active.lock().expect("active session slot") = Some(active.clone());

    let retirement = TerminalRetirement {
        quiescence: WorkerQuiescenceProof::WorkerJoined,
        release: TerminalReleaseEvidence::Unverified {
            obligation_mask: 1,
            detail: "worker release is unverified".into(),
        },
        publication: PlaybackTerminalPublication::Finished {
            outcome: "finished".into(),
            message: "pre-commit success intention".into(),
        },
    };
    let error = release_terminal_ownership(
        &service.active,
        &service.last_terminal,
        &active,
        retirement.clone(),
    )
    .expect_err("invalid residual must reject retirement commit");
    let publication = terminal_retirement_commit_failure(
        &service.active,
        &service.last_terminal,
        &active,
        &retirement,
        &error,
    );

    assert!(matches!(
        &publication,
        PlaybackTerminalPublication::Failed { code, message }
            if code == "terminal_retirement_commit_failed"
                && message.contains("pre-commit success intention")
                && message.contains("injected worker cause")
                && message.contains("residual")
    ));
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    publish_terminal_event(&events, &active, &publication)
        .expect("publish failure derived from committed retirement result");
    let published = events.lock().expect("event hub").buffered_for_test();
    assert!(
        !published
            .iter()
            .any(|event| matches!(event, crate::ui_events::UiEvent::PlaybackFinished { .. }))
    );
    assert!(published.iter().any(|event| matches!(
        event,
        crate::ui_events::UiEvent::PlaybackFailed { payload, .. }
            if payload.session_id == active.session_id
                && payload.code == "terminal_retirement_commit_failed"
                && payload.message.contains("injected worker cause")
    )));
    assert!(!active.release_verified.load(Ordering::Acquire));
    assert!(!active.done.load(Ordering::Acquire));
    assert!(activity.physical_release_is_quarantined());
    let status = service.status().expect("native status after failed commit");
    assert_eq!(
        status.active.as_ref().map(|active| active.state),
        Some(PlaybackSessionState::Failed)
    );
    let status = status
        .last_terminal
        .expect("session-id failure status committed");
    assert_eq!(status.session_id, active.session_id);
    assert_eq!(status.state, PlaybackSessionState::Failed);
    assert_eq!(
        status.failure_code.as_deref(),
        Some("terminal_retirement_commit_failed")
    );
    assert_eq!(status.terminal_release_verified, Some(false));
    assert_eq!(status.release_obligation_mask, Some(1));
    assert!(matches!(
        activity.reserve_playback("successor-after-storage-failure"),
        Err(crate::app_state::ActivityReservationError::PhysicalReleaseUnverified)
    ));
    assert!(matches!(
        activity.reserve_calibration(),
        Err(crate::app_state::ActivityReservationError::PhysicalReleaseUnverified)
    ));
    assert!(matches!(
        activity.reserve_update(),
        Err(crate::app_state::ActivityReservationError::PhysicalReleaseUnverified)
    ));
}

#[test]
fn stale_retirement_failure_cannot_rewrite_successor_status() {
    let activity = ActivityCoordinator::default();
    let service = NativePlaybackService::new(activity.clone());
    let stale = active_for_control_with_physical(PlaybackSessionState::Stopping, None, true);
    *stale.activity_lease.lock().expect("stale activity lease") = Some(
        activity
            .reserve_playback(stale.session_id.clone())
            .expect("reserve stale physical session"),
    );
    let successor = active_for_control(PlaybackSessionState::Starting, None);
    *service.active.lock().expect("active slot") = Some(successor.clone());
    let stale_retirement = TerminalRetirement {
        quiescence: WorkerQuiescenceProof::WorkerJoined,
        release: TerminalReleaseEvidence::Unverified {
            obligation_mask: 1,
            detail: "stale session retains an unresolved release obligation".into(),
        },
        publication: PlaybackTerminalPublication::Finished {
            outcome: "finished".into(),
            message: "stale worker result".into(),
        },
    };

    let error = release_terminal_ownership(
        &service.active,
        &service.last_terminal,
        &stale,
        stale_retirement.clone(),
    )
    .expect_err("stale session no longer owns active slot");
    let publication = terminal_retirement_commit_failure(
        &service.active,
        &service.last_terminal,
        &stale,
        &stale_retirement,
        &error,
    );

    assert!(matches!(
        publication,
        PlaybackTerminalPublication::Failed { .. }
    ));
    assert_eq!(
        *successor.state.lock().expect("successor state"),
        PlaybackSessionState::Starting
    );
    assert!(
        service
            .last_terminal
            .lock()
            .expect("terminal status")
            .is_none()
    );
    assert!(!successor.release_verified.load(Ordering::Acquire));
    assert!(activity.physical_release_is_quarantined());
    assert!(matches!(
        activity.reserve_playback("successor-physical-input"),
        Err(crate::app_state::ActivityReservationError::PhysicalReleaseUnverified)
    ));
}

#[test]
fn poisoned_retirement_slot_keeps_unverified_release_failed() {
    let activity = ActivityCoordinator::default();
    let service = NativePlaybackService::new(activity.clone());
    let active = active_for_control_with_physical(PlaybackSessionState::Stopping, None, true);
    *active.activity_lease.lock().expect("activity lease") = Some(
        activity
            .reserve_playback(active.session_id.clone())
            .expect("reserve session"),
    );
    *service.active.lock().expect("active slot") = Some(active.clone());
    let poisoned_slot = Arc::clone(&service.active);
    let _ = std::thread::spawn(move || {
        let _slot = poisoned_slot.lock().expect("poison active slot");
        panic!("injected retirement slot poison");
    })
    .join();

    release_terminal_ownership(
        &service.active,
        &service.last_terminal,
        &active,
        TerminalRetirement {
            quiescence: WorkerQuiescenceProof::WorkerJoined,
            release: TerminalReleaseEvidence::Unverified {
                obligation_mask: 1,
                detail: "release proof unavailable".into(),
            },
            publication: PlaybackTerminalPublication::Failed {
                code: "terminal_release_unverified".into(),
                message: "release proof unavailable".into(),
            },
        },
    )
    .expect("retirement recovers poisoned mutex without clearing quarantine");
    assert!(active.done.load(Ordering::Acquire));
    assert!(!active.release_verified.load(Ordering::Acquire));
    assert_eq!(
        service
            .status()
            .expect_err("poisoned status lock fails closed"),
        "native active-playback lock poisoned"
    );
    let terminal = service
        .last_terminal
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .expect("terminal status committed despite active-slot poison");
    assert_eq!(terminal.state, PlaybackSessionState::Failed);
    assert_eq!(terminal.terminal_release_verified, Some(false));
    assert!(activity.physical_release_is_quarantined());
}

#[cfg(feature = "tauri-test")]
#[test]
fn recovery_focus_loss_preserves_needs_up_without_a_raw_attempt() {
    let _send_lock = recovery_test_send_lock();
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let (service, active, _authority) =
        quarantined_recovery_fixture("focus-lost-recovery", true, 0x0001, 0);
    reset_recovery_test_raw_sends();

    let error = recover_quarantined_release_once(&active, &service.last_terminal)
        .expect_err("focus-required recovery waits for original foreground");
    assert!(error.contains("needs_up=0x0001"));
    assert_eq!(recovery_test_raw_sends(), 0);
    assert!(service.activity.physical_release_is_quarantined());
    let residual = service
        .activity
        .claim_quarantined_release_residual(&active.session_id)
        .expect("failed recovery retains its residual");
    assert_eq!(residual.needs_up_mask, 0x0001);
    assert_eq!(residual.needs_probe_mask, 0);
    service
        .activity
        .abandon_quarantined_recovery(&active.session_id);
}

#[cfg(feature = "tauri-test")]
#[test]
fn recovery_from_focus_free_session_still_checks_origin_and_never_replays_probe_only_residual() {
    let _send_lock = recovery_test_send_lock();
    let (service, active, _authority) =
        quarantined_recovery_fixture("focus-free-recovery", false, 0x0001, 0);
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(0x7788));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(0x1199)));
    reset_recovery_test_raw_sends();

    let _ = recover_quarantined_release_once(&active, &service.last_terminal)
        .expect_err("synthetic probe remains inconclusive and quarantined");
    assert_eq!(recovery_test_raw_sends(), 1);
    let residual = service
        .activity
        .claim_quarantined_release_residual(&active.session_id)
        .expect("probe-only residual stays quarantined");
    assert_eq!(residual.needs_up_mask, 0);
    assert_eq!(residual.needs_probe_mask, 0x0001);
    assert_eq!(residual.outstanding_mask, 0x0001);
    service
        .activity
        .abandon_quarantined_recovery(&active.session_id);
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
}

#[cfg(feature = "tauri-test")]
#[test]
fn recovery_process_death_keeps_original_residual_and_never_sends() {
    let _send_lock = recovery_test_send_lock();
    let (service, active, authority) =
        quarantined_recovery_fixture("dead-origin-recovery", false, 0x0001, 0);
    authority.set_process_identity_status_for_test(
        sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessTerminated,
    );
    reset_recovery_test_raw_sends();

    let error = recover_quarantined_release_once(&active, &service.last_terminal)
        .expect_err("dead process origin cannot be recovered");
    assert!(error.contains("identity is no longer continuous"));
    assert_eq!(recovery_test_raw_sends(), 0);
    let residual = service
        .activity
        .claim_quarantined_release_residual(&active.session_id)
        .expect("dead-origin residual remains available");
    assert_eq!(residual.needs_up_mask, 0x0001);
    service
        .activity
        .abandon_quarantined_recovery(&active.session_id);
}

#[cfg(feature = "tauri-test")]
#[test]
fn recovery_owner_drift_keeps_original_residual_and_never_sends() {
    let _send_lock = recovery_test_send_lock();
    let (service, active, authority) =
        quarantined_recovery_fixture("drifted-origin-recovery", false, 0x0001, 0);
    authority.set_process_identity_status_for_test(
        sky_dispatch_win32::focus::WindowIdentityContinuity::OwnerMismatch,
    );
    reset_recovery_test_raw_sends();

    let error = recover_quarantined_release_once(&active, &service.last_terminal)
        .expect_err("owner drift cannot authorize recovery");
    assert!(error.contains("identity is no longer continuous"));
    assert_eq!(recovery_test_raw_sends(), 0);
    let residual = service
        .activity
        .claim_quarantined_release_residual(&active.session_id)
        .expect("drifted-origin residual remains quarantined");
    assert_eq!(residual.needs_up_mask, 0x0001);
    assert_eq!(residual.needs_probe_mask, 0);
    service
        .activity
        .abandon_quarantined_recovery(&active.session_id);
}

#[cfg(feature = "tauri-test")]
#[test]
fn recovery_attempt_does_not_retry_after_a_full_transport_and_inconclusive_probe() {
    let _send_lock = recovery_test_send_lock();
    let (service, active, _authority) =
        quarantined_recovery_fixture("qpc-failed-recovery", false, 0x0001, 0);
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(0x7788));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(0x1199)));
    reset_recovery_test_raw_sends();
    let result = recover_quarantined_release_once(&active, &service.last_terminal);

    let error = result.expect_err("inconclusive physical probe keeps the residual");
    assert!(error.contains("remains quarantined"));
    assert!(recovery_test_raw_sends() <= 1);
    assert!(service.activity.physical_release_is_quarantined());
    let residual = service
        .activity
        .claim_quarantined_release_residual(&active.session_id)
        .expect("QPC-failed recovery preserves the origin journal");
    assert_eq!(residual.outstanding_mask, 0x0001);
    assert_eq!(residual.needs_up_mask | residual.needs_probe_mask, 0x0001);
    service
        .activity
        .abandon_quarantined_recovery(&active.session_id);
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
}

#[cfg(feature = "tauri-test")]
#[test]
fn recovery_cannot_claim_a_residual_before_worker_quiescence() {
    let (service, active, _authority) =
        quarantined_recovery_fixture("not-yet-joined-recovery", false, 0x0001, 0);
    active.worker_quiescent.store(false, Ordering::Release);

    let error = recover_quarantined_release_once(&active, &service.last_terminal)
        .expect_err("recovery must wait for worker join");
    assert!(error.contains("requires a joined worker"));
    let residual = service
        .activity
        .claim_quarantined_release_residual(&active.session_id)
        .expect("a pre-join failure must not take recovery ownership");
    assert_eq!(residual.outstanding_mask, 0x0001);
    service
        .activity
        .abandon_quarantined_recovery(&active.session_id);
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
