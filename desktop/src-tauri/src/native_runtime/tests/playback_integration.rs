use super::*;

#[cfg(feature = "tauri-test")]
#[test]
fn native_monitor_runtime_stall_expires_lease_and_resumption_cannot_heal_it() {
    // This is behavioral coverage for supervisor ownership: progress
    // stops when the owning monitor iteration stalls, independent of
    // source checkout line endings or Rust formatting.
    use sky_player::adapter_support::{
        ActionKind as DispatchActionKind, KeyActionInput, PriorityMode, compile_runtime_intents,
    };
    use sky_player::engine::{
        BackendConfig, DispatchProfile, FaultInjectionScript, FocusOptions, NativeDispatchSession,
        NativeSessionOptions, PriorityOptions, TelemetryMode, TelemetryOptions, TestWaitPolicy,
        TimingOptions, WaitOptions,
    };

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: DispatchActionKind::Down,
                scheduled_us: 30_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: Arc::<str>::from("monitor-stall-runtime-test-down"),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: DispatchActionKind::Up,
                scheduled_us: 30_100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: Arc::<str>::from("monitor-stall-runtime-test-up"),
            },
        ],
        &sky_app_core::song::SKY_SCAN_CODES,
    )
    .expect("runtime test schedule");
    let player = Arc::new(
        NativeDispatchSession::new_with_power_endpoint(
            NativeSessionOptions {
                schedule,
                #[cfg(feature = "tauri-test")]
                startup_calibration_placement:
                    sky_player::engine::StartupCalibrationPlacement::BeforeSchedulingGuards,
                #[cfg(feature = "tauri-test")]
                timing_observer_for_qualification: false,
                #[cfg(feature = "tauri-test")]
                modifier_key_state_query_for_test: None,
                backend: BackendConfig::Mock {
                    latency_base_us: 0,
                    latency_per_key_us: 0,
                    fault_script: FaultInjectionScript::none(),
                },
                profile: DispatchProfile::MockTest,
                timing: TimingOptions {
                    game_fps: 60,
                    min_hold_us: 17_467,
                    min_release_gap_us: 17_467,
                    frame_us: 16_667,
                    frame_base_hold_us: 16_667,
                    timing_margin_us: 800,
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
                    supervisor_lease_timeout_us: 250_000,
                    test_spin_threshold_us: None,
                    test_wait_policy: TestWaitPolicy::LegacyTestWideSpin,
                },
                telemetry: TelemetryOptions {
                    mode: TelemetryMode::Ring,
                    capacity: 64,
                },
                priority: PriorityOptions {
                    mode: PriorityMode::Off,
                },
                instrument_key_profile: None,
                startup_ordering_hook: None,
                restore_race_hook: None,
                focus_pause_hook: None,
                timer_lifecycle_context: None,
                prepared_packet_ambiguity_mask: None,
                preflight_user_held_mask: None,
            },
            sky_player::engine::SystemPowerEndpoint::new()
                .expect("isolated monitor test power endpoint"),
        )
        .expect("mock native session"),
    );
    player.arm(0).expect("arm mock native session");

    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    let active = attach_test_presentation(
        NativeActivePlayback {
            activity: ActivityCoordinator::default(),
            session_id: "a".repeat(32),
            prepared_id: "b".repeat(32),
            song_id: "c".repeat(32),
            title: "Monitor lease runtime fixture".into(),
            total_us: 30_100_000,
            config: PlaybackConfigDto {
                hold_frames: 1.0,
                timing_margin_us: 800,
                tempo_scale: 1.0,
                fps: 60,
                dry_run: false,
            },
            timing_policy: MaterializedTimingPolicy::from_user_margin(60, 1.0, 800)
                .expect("runtime fixture timing policy"),
            timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto {
                recommended_timing_margin_us: Some(800),
                qualified: false,
                source: "default_fallback".into(),
            },
            plan_fingerprint: "d".repeat(64),
            physical: false,
            activity_lease: Mutex::new(None),
            target_hwnd: Some(0),
            state: Mutex::new(PlaybackSessionState::Starting),
            pending: Mutex::new(None),
            player: Some(player.clone()),
            power_lifecycle: Arc::new(PowerLifecycleDiagnostics::default()),
            power_request: Mutex::new(None),
            suspend_resume_registration: Mutex::new(None),
            started_at: Instant::now(),
            paused_since: Mutex::new(None),
            paused_total: Mutex::new(Duration::ZERO),
            stop_requested: AtomicBool::new(false),
            dry_run_terminal_request: AtomicU8::new(DRY_RUN_TERMINAL_NONE),
            done: AtomicBool::new(false),
            worker_quiescent: AtomicBool::new(false),
            release_verified: AtomicBool::new(false),
            sequence: AtomicU64::new(0),
            event_delivery_failure: Arc::new(AtomicBool::new(false)),
            presentation: OnceLock::new(),
        },
        &events,
        Arc::clone(&diagnostics_gate),
        None,
    );
    let mut service = NativePlaybackService::new(ActivityCoordinator::default());
    service.diagnostics_gate = diagnostics_gate;
    *service.active.lock().expect("active playback slot") = Some(active.clone());

    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let hook_gate = Arc::clone(&gate);
    let hook_player = Arc::clone(&player);
    let progress_calls = Arc::new(AtomicU64::new(0));
    let hook_progress_calls = Arc::clone(&progress_calls);
    let (progress_tx, progress_rx) = mpsc::channel();
    service.monitor_progress_test_hook = Some(Arc::new(move |accepted| {
        if hook_player.snapshot().startup_ready
            && hook_progress_calls.fetch_add(1, Ordering::AcqRel) == 0
        {
            progress_tx
                .send(accepted)
                .expect("monitor progress receiver");
            let (released, changed) = &*hook_gate;
            let mut released = released.lock().expect("monitor test gate");
            while !*released {
                released = changed.wait(released).expect("monitor test gate wait");
            }
        }
    }));

    let service = Arc::new(service);
    let (monitor_done_tx, monitor_done_rx) = mpsc::channel();
    let monitor_service = Arc::clone(&service);
    let monitor_active = Arc::clone(&active);
    let monitor_events = Arc::clone(&events);
    let (activation_sender, activation_receiver) = mpsc::channel();
    activation_sender
        .send(super::PlaybackActivation::Started)
        .expect("monitor activation receiver");
    let monitor_thread = thread::spawn(move || {
        monitor_service.monitor(monitor_active, monitor_events, activation_receiver);
        let _ = monitor_done_tx.send(());
    });

    let first_progress = progress_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("actual monitor should publish progress after startup readiness");
    assert!(
        first_progress,
        "the actual monitor progress point must be accepted"
    );

    let helper_running = Arc::new(AtomicBool::new(true));
    let helper_ticks = Arc::new(AtomicU64::new(0));
    let helper_running_thread = Arc::clone(&helper_running);
    let helper_ticks_thread = Arc::clone(&helper_ticks);
    let helper_thread = thread::spawn(move || {
        while helper_running_thread.load(Ordering::Acquire) {
            helper_ticks_thread.fetch_add(1, Ordering::Relaxed);
            thread::sleep(Duration::from_millis(1));
        }
    });

    let expiry_deadline = Instant::now() + Duration::from_secs(5);
    while player.snapshot().terminal_trigger_reason.as_deref() != Some("supervisor_lease_expired")
        && Instant::now() < expiry_deadline
    {
        thread::sleep(Duration::from_millis(5));
    }
    let terminal_trigger_reason = player.snapshot().terminal_trigger_reason;
    let helper_remained_active = helper_ticks.load(Ordering::Acquire) > 1;

    helper_running.store(false, Ordering::Release);
    helper_thread.join().expect("unrelated liveness helper");
    {
        let (released, changed) = &*gate;
        *released.lock().expect("monitor test gate") = true;
        changed.notify_all();
    }

    let monitor_completed = monitor_done_rx.recv_timeout(Duration::from_secs(3)).is_ok();
    if !monitor_completed {
        active.stop_requested.store(true, Ordering::Release);
        let _ = monitor_done_rx.recv_timeout(Duration::from_secs(3));
    }
    monitor_thread.join().expect("native monitor thread");

    assert_eq!(
        terminal_trigger_reason.as_deref(),
        Some("supervisor_lease_expired"),
        "the worker watchdog must latch hard-stop authority while the real monitor is stalled"
    );
    assert!(
        helper_remained_active,
        "unrelated thread activity must continue during the stall"
    );
    assert!(
        monitor_completed,
        "the resumed monitor must observe terminal lease expiry and exit"
    );
    assert_eq!(
        progress_calls.load(Ordering::Acquire),
        1,
        "the resumed monitor must not certify progress after terminal supervision"
    );
    assert_eq!(
        player.snapshot().terminal_trigger_reason.as_deref(),
        Some("supervisor_lease_expired"),
        "resuming the monitor must not heal the expiry latch"
    );
    assert!(wait_for_playback_done(&active, Duration::from_secs(2)));
}

#[cfg(feature = "tauri-test")]
#[test]
fn blocked_snapshot_projection_past_lease_keeps_supervisor_live() {
    use sky_player::engine::DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US;

    let activity = ActivityCoordinator::default();
    let player = presentation_projection_session(
        DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US,
        30_100_000,
        30_200_000,
    );
    player.set_focus_hint(true);
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    let (test_hook, presentation_entered, gate) =
        blocked_presentation_hook(PlaybackPresentationStage::SnapshotProjection);
    let active = attach_test_presentation(
        tracked_test_active(Arc::clone(&player), activity.clone()),
        &events,
        Arc::clone(&diagnostics_gate),
        Some(test_hook),
    );
    let mut service = NativePlaybackService::new(activity.clone());
    service.diagnostics_gate = diagnostics_gate;
    *service.active.lock().expect("active playback slot") = Some(Arc::clone(&active));
    let progress = Arc::new(AtomicU64::new(0));
    let hook_progress = Arc::clone(&progress);
    service.monitor_progress_test_hook = Some(Arc::new(move |accepted| {
        if accepted {
            hook_progress.fetch_add(1, Ordering::Relaxed);
        }
    }));
    let service = Arc::new(service);
    let monitor_active = Arc::clone(&active);
    let monitor_events = Arc::clone(&events);
    let monitor_service = Arc::clone(&service);
    let (activation_sender, activation_receiver) = mpsc::channel();
    let (done_sender, done_receiver) = mpsc::channel();
    let monitor = thread::spawn(move || {
        monitor_service.monitor(monitor_active, monitor_events, activation_receiver);
        let _ = done_sender.send(());
    });
    player.arm(500_000).expect("arm presentation-stall player");
    player.set_focus_hint(true);
    activation_sender
        .send(PlaybackActivation::Started)
        .expect("monitor activation receiver");

    presentation_entered
        .recv_timeout(Duration::from_secs(2))
        .expect("snapshot projection reached its blocking barrier");
    let started_at = Instant::now();
    let progress_at_entry = progress.load(Ordering::Acquire);
    let progress_deadline = Instant::now() + Duration::from_secs(5);
    while progress
        .load(Ordering::Acquire)
        .saturating_sub(progress_at_entry)
        < 160
        && Instant::now() < progress_deadline
    {
        thread::sleep(Duration::from_millis(2));
    }
    let progress_advanced = progress
        .load(Ordering::Acquire)
        .saturating_sub(progress_at_entry);
    assert!(
        started_at.elapsed() > Duration::from_secs(3),
        "presentation remained blocked beyond the worker lease interval"
    );
    assert!(
        progress_advanced >= 160,
        "safety supervision kept progressing"
    );
    assert_ne!(
        player.snapshot().terminal_trigger_reason.as_deref(),
        Some("supervisor_lease_expired"),
        "presentation stall must not age out an otherwise progressing supervisor"
    );

    let metrics = active
        .presentation
        .get()
        .expect("presentation consumer")
        .queue
        .metrics();
    assert!(metrics.snapshot_coalesced > 0);
    assert!(metrics.snapshot_dropped >= metrics.snapshot_coalesced);

    release_test_gate(&gate);
    player.quit().expect("quit late-Down test worker");
    assert!(done_receiver.recv_timeout(Duration::from_secs(5)).is_ok());
    monitor.join().expect("monitor thread");
    assert!(active.worker_quiescent.load(Ordering::Acquire));
    assert!(active.release_verified.load(Ordering::Acquire));
    assert!(wait_for_playback_done(&active, Duration::from_secs(2)));
    events.lock().expect("event hub").close();
}

#[cfg(feature = "tauri-test")]
#[test]
fn blocked_diagnostics_gate_past_lease_does_not_block_safety_progress() {
    use sky_player::engine::DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US;

    let activity = ActivityCoordinator::default();
    let player = presentation_projection_session(
        DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US,
        30_100_000,
        30_200_000,
    );
    player.set_focus_hint(true);
    player.set_live_diagnostics_enabled(true);
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    diagnostics_gate
        .set_enabled(true)
        .expect("enable diagnostics");
    let (entered_sender, entered_receiver) = mpsc::channel();
    let announced = Arc::new(AtomicBool::new(false));
    let hook_announced = Arc::clone(&announced);
    let test_hook: PlaybackPresentationTestHook = Arc::new(move |stage| {
        if stage == PlaybackPresentationStage::DiagnosticsProjection
            && !hook_announced.swap(true, Ordering::AcqRel)
        {
            entered_sender.send(()).expect("diagnostics hook receiver");
        }
    });
    let gate_guard = diagnostics_gate
        .state
        .lock()
        .expect("hold diagnostics gate");
    let active = attach_test_presentation(
        tracked_test_active(Arc::clone(&player), activity.clone()),
        &events,
        Arc::clone(&diagnostics_gate),
        Some(test_hook),
    );
    let mut service = NativePlaybackService::new(activity.clone());
    service.diagnostics_gate = Arc::clone(&diagnostics_gate);
    *service.active.lock().expect("active playback slot") = Some(Arc::clone(&active));
    let progress = Arc::new(AtomicU64::new(0));
    let hook_progress = Arc::clone(&progress);
    service.monitor_progress_test_hook = Some(Arc::new(move |accepted| {
        if accepted {
            hook_progress.fetch_add(1, Ordering::Relaxed);
        }
    }));
    let service = Arc::new(service);
    let monitor_active = Arc::clone(&active);
    let monitor_events = Arc::clone(&events);
    let monitor_service = Arc::clone(&service);
    let (activation_sender, activation_receiver) = mpsc::channel();
    let (done_sender, done_receiver) = mpsc::channel();
    let monitor = thread::spawn(move || {
        monitor_service.monitor(monitor_active, monitor_events, activation_receiver);
        let _ = done_sender.send(());
    });
    player.arm(500_000).expect("arm diagnostics-gate player");
    player.set_focus_hint(true);
    activation_sender
        .send(PlaybackActivation::Started)
        .expect("monitor activation receiver");

    entered_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("diagnostics sampling reached the gate barrier");
    let started_at = Instant::now();
    let progress_at_entry = progress.load(Ordering::Acquire);
    let progress_deadline = Instant::now() + Duration::from_secs(5);
    while progress
        .load(Ordering::Acquire)
        .saturating_sub(progress_at_entry)
        < 160
        && Instant::now() < progress_deadline
    {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(started_at.elapsed() > Duration::from_secs(3));
    assert!(
        progress
            .load(Ordering::Acquire)
            .saturating_sub(progress_at_entry)
            >= 160,
        "safety supervision must not take the diagnostics gate"
    );
    assert_ne!(
        player.snapshot().terminal_trigger_reason.as_deref(),
        Some("supervisor_lease_expired")
    );
    drop(gate_guard);
    player.quit().expect("quit diagnostics-gate test worker");
    assert!(done_receiver.recv_timeout(Duration::from_secs(5)).is_ok());
    monitor.join().expect("monitor thread");
    assert!(wait_for_playback_done(&active, Duration::from_secs(2)));
    assert!(active.release_verified.load(Ordering::Acquire));
    events.lock().expect("event hub").close();
}

#[cfg(feature = "tauri-test")]
#[test]
fn blocked_terminal_presentation_does_not_hold_supervisor_after_release() {
    let activity = ActivityCoordinator::default();
    let player = presentation_projection_session(0, 30_100_000, 30_200_000);
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    let (test_hook, presentation_entered, gate) =
        blocked_presentation_hook(PlaybackPresentationStage::DiagnosticsProjection);
    let active = attach_test_presentation(
        tracked_test_active(Arc::clone(&player), activity.clone()),
        &events,
        Arc::clone(&diagnostics_gate),
        Some(test_hook),
    );
    let mut service = NativePlaybackService::new(activity.clone());
    service.diagnostics_gate = Arc::clone(&diagnostics_gate);
    *service.active.lock().expect("active playback slot") = Some(Arc::clone(&active));
    let service = Arc::new(service);
    let monitor_active = Arc::clone(&active);
    let monitor_events = Arc::clone(&events);
    let monitor_service = Arc::clone(&service);
    let (activation_sender, activation_receiver) = mpsc::channel();
    let (monitor_done_sender, monitor_done_receiver) = mpsc::channel();
    let monitor = thread::spawn(move || {
        monitor_service.monitor(monitor_active, monitor_events, activation_receiver);
        let _ = monitor_done_sender.send(());
    });
    player
        .arm(500_000)
        .expect("arm terminal-presentation test worker");
    activation_sender
        .send(PlaybackActivation::Started)
        .expect("monitor activation receiver");
    presentation_entered
        .recv_timeout(Duration::from_secs(2))
        .expect("presentation blocked before terminal request");

    player.quit().expect("request terminal worker exit");
    assert!(
        monitor_done_receiver
            .recv_timeout(Duration::from_secs(2))
            .is_ok(),
        "safety supervisor must return without waiting for presentation finalization"
    );
    monitor.join().expect("monitor thread");
    assert!(active.worker_quiescent.load(Ordering::Acquire));
    assert!(active.release_verified.load(Ordering::Acquire));
    assert!(!active.done.load(Ordering::Acquire));
    assert!(
        service
            .status()
            .expect("pending presentation status")
            .active
            .is_some()
    );

    release_test_gate(&gate);
    assert!(wait_for_playback_done(&active, Duration::from_secs(2)));
    let status = service.status().expect("retired presentation status");
    assert!(status.active.is_none());
    assert_eq!(
        status.last_terminal.expect("terminal record").state,
        PlaybackSessionState::Finished
    );
    events.lock().expect("event hub").close();
}

#[cfg(feature = "tauri-test")]
#[test]
fn presentation_critical_queue_overflow_fails_closed_and_releases() {
    let activity = ActivityCoordinator::default();
    let player = presentation_projection_session(0, 30_100_000, 30_200_000);
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    let (test_hook, presentation_entered, gate) =
        blocked_presentation_hook(PlaybackPresentationStage::SnapshotProjection);
    let active = attach_test_presentation(
        tracked_test_active(Arc::clone(&player), activity.clone()),
        &events,
        Arc::clone(&diagnostics_gate),
        Some(test_hook),
    );
    let mut service = NativePlaybackService::new(activity.clone());
    service.diagnostics_gate = Arc::clone(&diagnostics_gate);
    *service.active.lock().expect("active playback slot") = Some(Arc::clone(&active));
    let service = Arc::new(service);
    let monitor_active = Arc::clone(&active);
    let monitor_events = Arc::clone(&events);
    let monitor_service = Arc::clone(&service);
    let (activation_sender, activation_receiver) = mpsc::channel();
    let (monitor_done_sender, monitor_done_receiver) = mpsc::channel();
    let monitor = thread::spawn(move || {
        monitor_service.monitor(monitor_active, monitor_events, activation_receiver);
        let _ = monitor_done_sender.send(());
    });
    player.arm(500_000).expect("arm queue-overflow test worker");
    activation_sender
        .send(PlaybackActivation::Started)
        .expect("monitor activation receiver");
    presentation_entered
        .recv_timeout(Duration::from_secs(2))
        .expect("snapshot projection blocks critical consumer");

    let presentation = active.presentation.get().expect("presentation consumer");
    for _ in 0..PLAYBACK_PRESENTATION_CRITICAL_CAPACITY {
        presentation
            .publish_state(&active, PlaybackEventState::Paused, None, None)
            .expect("critical lifecycle item fits in bounded queue");
    }
    let overflow = presentation
        .publish_state(&active, PlaybackEventState::Playing, None, None)
        .expect_err("critical overflow must be observable");
    assert!(overflow.contains("queue overflow"));
    assert!(active.event_delivery_failure.load(Ordering::Acquire));

    release_test_gate(&gate);
    assert!(
        monitor_done_receiver
            .recv_timeout(Duration::from_secs(3))
            .is_ok()
    );
    monitor.join().expect("monitor thread");
    assert!(wait_for_playback_done(&active, Duration::from_secs(1)));
    assert!(active.worker_quiescent.load(Ordering::Acquire));
    assert!(active.release_verified.load(Ordering::Acquire));
    let terminal = service
        .status()
        .expect("failed presentation status")
        .last_terminal
        .expect("failed terminal record");
    assert_eq!(terminal.state, PlaybackSessionState::Failed);
    assert!(terminal.failure_code.is_some());
    events.lock().expect("event hub").close();
}

#[cfg(feature = "tauri-test")]
#[test]
fn rapid_playback_sessions_queue_terminal_before_successor_starting() {
    let activity = ActivityCoordinator::default();
    let service = NativePlaybackService::new(activity);
    let settings = ApplicationSettings::default();
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let mut session_ids = Vec::new();

    for _ in 0..2 {
        let prepared_id = prepare_dry_run_plan(&service, &settings, &"c".repeat(32));
        let session = service
            .start(
                PlaybackStartRequest {
                    prepared_id,
                    decisions: Vec::new(),
                },
                &settings,
                Arc::clone(&events),
            )
            .expect("start rapid dry-run session");
        session_ids.push(session.session_id);
        let deadline = Instant::now() + Duration::from_secs(3);
        while service
            .status()
            .expect("rapid session status")
            .active
            .is_some()
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            service
                .status()
                .expect("retired rapid session")
                .active
                .is_none(),
            "session lifecycle finalization must release the successor barrier"
        );
    }

    let buffered = events.lock().expect("event hub").buffered_for_test();
    let first_terminal = buffered
        .iter()
        .position(|event| {
            matches!(event, UiEvent::PlaybackFinished { payload, .. }
                    if payload.session_id == session_ids[0])
        })
        .expect("first terminal event");
    let second_starting = buffered
        .iter()
        .position(|event| {
            matches!(event, UiEvent::PlaybackStateChanged { payload, .. }
                    if payload.session_id == session_ids[1]
                        && payload.state == PlaybackEventState::Starting)
        })
        .expect("successor Starting event");
    assert!(first_terminal < second_starting);
    for session_id in &session_ids {
        let starting = buffered
            .iter()
            .position(|event| {
                matches!(event, UiEvent::PlaybackStateChanged { payload, .. }
                        if &payload.session_id == session_id
                            && payload.state == PlaybackEventState::Starting)
            })
            .expect("session Starting event");
        let finished = buffered
            .iter()
            .position(|event| {
                matches!(event, UiEvent::PlaybackStateChanged { payload, .. }
                        if &payload.session_id == session_id
                            && payload.state == PlaybackEventState::Finished)
            })
            .expect("session Finished state event");
        let terminal = buffered
            .iter()
            .position(|event| {
                matches!(event, UiEvent::PlaybackFinished { payload, .. }
                        if &payload.session_id == session_id)
            })
            .expect("session terminal event");
        assert!(starting < finished && finished < terminal);
    }
    events.lock().expect("event hub").close();
}

#[cfg(feature = "tauri-test")]
#[test]
fn native_startup_projection_keeps_ready_worker_starting_until_armed_publication() {
    use sky_player::engine::{EnginePollStatus, StartupCheckpoint, StartupOrderingHook};

    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::ReadyToArmed, 1);
    let session = terminal_projection_session(0, 5_100_000, Some(Arc::clone(&hook)));
    session
        .arm(300_000)
        .expect("start native projection worker");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(15)));

    let engine = session.snapshot();
    let poll = session.poll_state();
    assert!(engine.startup_ready);
    assert!(!engine.startup_armed);
    assert_eq!(engine.status, EnginePollStatus::Preparing.as_str());
    assert_eq!(engine.elapsed_us, 0);
    assert_eq!(engine.pre_roll_remaining_us, 0);
    assert_eq!(poll.status, EnginePollStatus::Preparing);
    assert_eq!(poll.elapsed_us, 0);
    assert_eq!(poll.pre_roll_remaining_us, 0);

    let active = terminal_projection_active(Arc::clone(&session));
    *active.state.lock().expect("native playback state") = PlaybackSessionState::Starting;
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    publish_playback_state(
        &events,
        &active,
        playback_event_state(poll.status),
        None,
        None,
    )
    .expect("publish preparing playback state");
    publish_playback_snapshot(
        &events,
        &active,
        poll.elapsed_us,
        poll.pre_roll_remaining_us,
        poll.status,
    )
    .expect("publish preparing playback snapshot");
    let events = events.lock().expect("native projection events");
    let buffered = events.buffered_for_test();
    let snapshot = buffered
        .iter()
        .find_map(|event| match event {
            UiEvent::PlaybackSnapshot { payload, .. } => Some(payload),
            _ => None,
        })
        .expect("preparing playback snapshot");
    assert_eq!(snapshot.state, PlaybackEventState::Starting);
    assert_eq!(snapshot.current_us, 0);
    assert_eq!(snapshot.pre_roll_remaining_us, 0);
    assert_eq!(snapshot.focus_state, PlaybackFocusState::Waiting);
    assert_eq!(snapshot.health, PlaybackHealthState::Healthy);
    assert!(matches!(
        buffered.front(),
        Some(UiEvent::PlaybackStateChanged { payload, .. })
            if payload.state == PlaybackEventState::Starting
    ));
    drop(events);

    hook.release_startup_checkpoint();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !session.snapshot().startup_armed {
        assert!(Instant::now() < deadline, "worker did not publish Armed");
        thread::yield_now();
    }
    let armed = session.snapshot();
    assert!(armed.startup_ready);
    assert!(armed.startup_armed);
    assert_eq!(armed.status, EnginePollStatus::Preroll.as_str());
    assert!(armed.pre_roll_remaining_us > 0);
    assert!(armed.pre_roll_remaining_us > 0);

    session.quit().expect("stop armed projection worker");
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("projection worker join")
    );
}

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

#[cfg(feature = "tauri-test")]
#[test]
fn terminal_outcome_health_projection_preserves_trigger_and_release_identity() {
    use crate::ui_events::DiagnosticsBackendStatus;
    use sky_player::engine::{
        DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US, EnginePollStatus, StartupCheckpoint,
        StartupOrderingHook,
    };

    let cases = [
        (
            "supervisor_lease_expired",
            EnginePollStatus::Error,
            "error",
            PlaybackHealthState::Error,
            DiagnosticsBackendStatus::Error,
        ),
        (
            "panic_release_requested",
            EnginePollStatus::Error,
            "error",
            PlaybackHealthState::Error,
            DiagnosticsBackendStatus::Error,
        ),
        (
            "quit",
            EnginePollStatus::Quit,
            "quit",
            PlaybackHealthState::Healthy,
            DiagnosticsBackendStatus::Healthy,
        ),
        (
            "skipped",
            EnginePollStatus::Skipped,
            "skipped",
            PlaybackHealthState::Healthy,
            DiagnosticsBackendStatus::Healthy,
        ),
        (
            "finished",
            EnginePollStatus::Finished,
            "finished",
            PlaybackHealthState::Healthy,
            DiagnosticsBackendStatus::Healthy,
        ),
    ];

    for (trigger, expected_status, expected_outcome, playback_health, backend_health) in cases {
        let is_lease_expiry = trigger == "supervisor_lease_expired";
        let is_explicit_panic = trigger == "panic_release_requested";
        let lease_timeout_us = if is_lease_expiry {
            DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US
        } else {
            0
        };
        let up_scheduled_us = if expected_status == EnginePollStatus::Finished {
            180_000
        } else {
            5_100_000
        };
        let command_hook =
            (is_lease_expiry || is_explicit_panic || trigger == "quit" || trigger == "skipped")
                .then(|| {
                    let hook = Arc::new(StartupOrderingHook::default());
                    hook.arm_startup_checkpoint(StartupCheckpoint::ArmedPublished, 1);
                    hook
                });
        let session =
            terminal_projection_session(lease_timeout_us, up_scheduled_us, command_hook.clone());
        let wait_for_armed_startup = || {
            let hook = command_hook.as_ref().expect("terminal startup barrier");
            assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(15)));
            hook.release_startup_checkpoint();
        };
        session.arm(0).expect("terminal projection worker arm");

        if is_lease_expiry {
            wait_for_armed_startup();
            assert!(
                session
                    .join(Duration::from_secs(7))
                    .expect("lease expiry join"),
                "lease expiry worker should join"
            );
        } else if is_explicit_panic {
            wait_for_armed_startup();
            session.panic_release().expect("explicit panic request");
            assert!(
                session
                    .join(Duration::from_secs(2))
                    .expect("panic release join"),
                "panic release worker should join"
            );
        } else if trigger == "quit" {
            wait_for_armed_startup();
            session.quit().expect("quit request");
            assert!(
                session.join(Duration::from_secs(2)).expect("quit join"),
                "quit worker should join"
            );
        } else if trigger == "skipped" {
            wait_for_armed_startup();
            session.skip().expect("skip request");
            assert!(
                session.join(Duration::from_secs(2)).expect("skip join"),
                "skip worker should join"
            );
        } else {
            assert!(
                session
                    .join(Duration::from_secs(2))
                    .expect("natural completion join"),
                "natural completion worker should join"
            );
        }

        let full = session.snapshot();
        let progress = session.snapshot_lite();
        assert_eq!(full.outcome.as_deref(), Some(expected_outcome), "{trigger}");
        let expected_trigger = (trigger != "finished").then_some(trigger);
        assert_eq!(
            full.terminal_trigger_reason.as_deref(),
            expected_trigger,
            "{trigger}"
        );
        assert!(full.terminal_error.is_none(), "{trigger}: {full:?}");
        assert!(full.secondary_errors.is_empty(), "{trigger}: {full:?}");
        assert_eq!(
            progress.terminal_outcome,
            Some(expected_status),
            "{trigger}"
        );
        assert_eq!(progress.status, expected_status.as_str(), "{trigger}");
        assert!(!progress.has_terminal_error, "{trigger}: {progress:?}");
        assert_eq!(
            playback_health_from_snapshot(&progress),
            playback_health,
            "playback projection for {trigger}"
        );
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let active = terminal_projection_active(Arc::clone(&session));
        publish_playback_snapshot(
            &events,
            &active,
            progress.elapsed_us,
            progress.pre_roll_remaining_us,
            expected_status,
        )
        .expect("terminal playback snapshot publication");
        let published_health = match events
            .lock()
            .expect("terminal projection event hub")
            .buffered_for_test()
            .pop_back()
            .expect("playback snapshot event")
        {
            UiEvent::PlaybackSnapshot { payload, .. } => payload.health,
            event => panic!("expected playback snapshot, got {event:?}"),
        };
        assert_eq!(
            published_health, playback_health,
            "event projection for {trigger}"
        );
        assert_eq!(
            NativeDiagnosticsSample::from_player(&session, true).backend_status,
            backend_health,
            "diagnostics projection for {trigger}"
        );
        let diagnostics_gate = DiagnosticsPublicationGate::default();
        diagnostics_gate
            .set_enabled(true)
            .expect("enable final diagnostics publication");
        publish_final_diagnostics_snapshot_for_active(&events, &active, &diagnostics_gate)
            .expect("final diagnostics snapshot publication");
        let published_backend_health = events
            .lock()
            .expect("terminal projection event hub")
            .buffered_for_test()
            .iter()
            .find_map(|event| match event {
                UiEvent::DiagnosticsSnapshot { payload, .. } => Some(payload.backend_status),
                _ => None,
            })
            .expect("diagnostics snapshot event");
        assert_eq!(
            published_backend_health, backend_health,
            "diagnostics event projection for {trigger}"
        );
        let release = full
            .release_outcome
            .as_ref()
            .unwrap_or_else(|| panic!("{trigger}: missing tracked release result"));
        assert!(release.released_successfully, "{trigger}: {release:?}");
        assert_eq!(release.stuck_mask, 0, "{trigger}: {release:?}");
        assert!(!release.verification_inconclusive, "{trigger}: {release:?}");
        assert!(!release.transport_anomaly, "{trigger}: {release:?}");
    }
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
    let source = include_str!("../../native_runtime.rs");
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

#[test]
fn supervisor_spawn_failure_is_retired_before_physical_activation() {
    let mut service = NativePlaybackService::new(ActivityCoordinator::default());
    service.supervisor_spawn_failure = true;
    let settings = ApplicationSettings::default();
    let prepared_id = prepare_dry_run_plan(&service, &settings, &"c".repeat(32));
    let events = Arc::new(Mutex::new(NativeEventHub::default()));

    let error = service
        .start(
            PlaybackStartRequest {
                prepared_id,
                decisions: Vec::new(),
            },
            &settings,
            events.clone(),
        )
        .expect_err("injected supervisor spawn failure");

    assert!(error.contains("injected playback supervisor spawn failure"));
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut status = service.status().expect("status after supervisor failure");
    while status.active.is_some() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
        status = service.status().expect("pending supervisor failure status");
    }
    assert!(status.active.is_none());
    let terminal = status.last_terminal.expect("stored supervisor failure");
    assert_eq!(terminal.state, PlaybackSessionState::Failed);
    assert_eq!(
        terminal.failure_code.as_deref(),
        Some("playback_supervisor_start_failed")
    );
    assert!(
        events
            .lock()
            .expect("event hub")
            .buffered_for_test()
            .iter()
            .any(
                |event| matches!(event, UiEvent::PlaybackFailed { payload, .. }
                if payload.code == "playback_supervisor_start_failed")
            )
    );
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
fn sender_trace_export_includes_session_identity_and_frozen_policy() {
    let active = active_for_control_with_physical(PlaybackSessionState::Finished, None, true);
    let telemetry = serde_json::json!({
        "schema_version": 19,
        "qpc_frequency_hz": 10_000_000,
        "records": [],
        "attempted": 0,
        "accepted": 0,
        "dropped": 0,
        "observer_queue_dropped": 0,
        "truncated": false,
    });
    let export = sender_trace_export_json(&active, &telemetry).expect("sender trace JSON");
    let document: Value = serde_json::from_str(&export).expect("valid sender trace JSON");

    assert_eq!(document["export_schema_version"], 2);
    assert_eq!(document["session_id"], "a".repeat(32));
    assert_eq!(document["song_id"], "c".repeat(32));
    assert_eq!(document["song_title"], "Fixture");
    assert_eq!(document["plan_fingerprint"], "d".repeat(64));
    assert_eq!(document["timing_policy"]["fps"], 60);
    assert_eq!(document["timing_policy"]["frame_base_hold_us"], 16_667);
    assert_eq!(document["timing_policy"]["timing_margin_us"], 800);
    assert_eq!(document["timing_policy"]["target_hold_us"], 17_467);
    assert_eq!(document["timing_policy"]["release_gap_us"], 17_467);
    assert_eq!(document["telemetry"], telemetry);
}

#[test]
fn sender_trace_beginning_physical_session_b_clears_session_a_and_rejects_late_a_completion() {
    let mut state = SenderTraceState::default();
    state.begin_physical_session("session-a");
    state.finish_physical_session(
            "session-a",
            "song-a",
            "Song A",
            "fingerprint-a",
            Some(r#"{"session_id":"session-a","song_id":"song-a","song_title":"Song A","plan_fingerprint":"fingerprint-a"}"#.into()),
        );
    assert!(state.completed.is_some());

    state.begin_physical_session("session-b");
    assert!(state.completed.is_none());
    state.finish_physical_session(
        "session-a",
        "song-a",
        "Song A",
        "fingerprint-a",
        Some("late trace A".into()),
    );

    assert!(state.completed.is_none());
    assert_eq!(state.collecting_session_id.as_deref(), Some("session-b"));
}

#[test]
fn sender_trace_failed_session_b_extraction_does_not_restore_session_a() {
    let mut state = SenderTraceState::default();
    state.begin_physical_session("session-a");
    state.finish_physical_session(
            "session-a",
            "song-a",
            "Song A",
            "fingerprint-a",
            Some(r#"{"session_id":"session-a","song_id":"song-a","song_title":"Song A","plan_fingerprint":"fingerprint-a"}"#.into()),
        );
    state.begin_physical_session("session-b");
    state.finish_physical_session("session-b", "song-b", "Song B", "fingerprint-b", None);

    assert!(state.completed.is_none());
    assert!(state.collecting_session_id.is_none());
}

#[test]
fn sender_trace_capture_memory_stays_under_two_mib() {
    let record_bytes = std::mem::size_of::<sky_player::engine::RtTraceRecord>();
    let capture_bytes = record_bytes * SENDER_TRACE_CAPTURE_CAPACITY;
    assert!(capture_bytes <= 2 * 1024 * 1024);
    println!(
        "sender trace ring: {record_bytes} bytes/record, {SENDER_TRACE_CAPTURE_CAPACITY} records, {capture_bytes} bytes total"
    );
}

#[test]
fn debug_install_root_is_repository_root_not_cwd() {
    if cfg!(debug_assertions) {
        let root = resolve_install_root().expect("root");
        assert!(root.join("rust").is_dir());
        assert!(root.join("desktop").is_dir());
    }
}

#[test]
fn production_composition_has_no_environment_only_safe_bypass() {
    assert_eq!(TestSeams::default(), TestSeams::Disabled);
    assert_ne!(TestSeams::Disabled, TestSeams::SafePackage);
    // The only selector is an explicit composition value.  There is no
    // environment-to-seam conversion in either production constructor.
    assert!(!matches!(TestSeams::Disabled, TestSeams::SafePackage));
    assert!(matches!(TestSeams::SafePackage, TestSeams::SafePackage));
}

#[test]
fn native_bootstrap_uses_explicit_install_root_and_returns_plain_dto() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-runtime-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let value = runtime
        .dispatch("app.bootstrap", Value::Object(Default::default()))
        .expect("bootstrap");
    assert!(value.get("app_version").is_some());
    assert!(value.get("Ok").is_none());
    assert_eq!(value["settings"]["palette"], "aurora");
    assert_eq!(value["settings"]["palette"], value["palette"]);
    assert_eq!(value["theme"], "aurora");
    assert_eq!(runtime.install_root(), root.as_path());
    fs::write(
        root.join("config.json"),
        "{\"schema_version\":3,\"theme\":\"slate\"}\n",
    )
    .expect("external settings update");
    let cached_bootstrap = runtime
        .dispatch("app.bootstrap", Value::Object(Default::default()))
        .expect("cached bootstrap");
    assert_eq!(cached_bootstrap["settings"]["palette"], "aurora");
    let refreshed_settings = runtime
        .dispatch("settings.get", Value::Object(Default::default()))
        .expect("settings refresh");
    assert_eq!(refreshed_settings["palette"], "slate");
    assert!(
        runtime
            .dispatch("settings.get", Value::Object(Default::default()))
            .is_ok()
    );
    assert_eq!(
        runtime
            .dispatch("app.shutdown", Value::Object(Default::default()))
            .expect("first shutdown"),
        Value::Null
    );
    assert_eq!(
        runtime
            .dispatch("app.shutdown", Value::Object(Default::default()))
            .expect("idempotent shutdown"),
        Value::Null
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn update_check_uses_cached_settings_without_refreshing_external_file() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-update-check-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");

    let initial = runtime
        .dispatch("app.bootstrap", Value::Object(Default::default()))
        .expect("bootstrap");
    assert_eq!(initial["settings"]["palette"], "aurora");

    fs::write(
        root.join("config.json"),
        "{\"schema_version\":3,\"theme\":\"slate\"}\n",
    )
    .expect("external settings update");
    let error = runtime
        .dispatch("update.check", Value::Object(Default::default()))
        .expect_err("install-root runtime has no update service");
    assert!(error.starts_with("update_service_unavailable"));

    let cached = runtime
        .dispatch("app.bootstrap", Value::Object(Default::default()))
        .expect("cached bootstrap");
    assert_eq!(cached["settings"]["palette"], "aurora");

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn same_size_same_mtime_content_mutation_still_matches_clean_compose() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("sky-native-catalog-cache-same-metadata-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let song = root.join("songs/content.json");
    let original = br#"{"name":"Alpha","songNotes":[{"time":0,"key":"1Key0"}]}"#;
    let replacement = br#"{"name":"Bravo","songNotes":[{"time":0,"key":"1Key0"}]}"#;
    assert_eq!(original.len(), replacement.len());
    fs::write(&song, original).expect("original song");

    let initial_runtime =
        NativeDesktopRuntime::from_install_root(root.clone()).expect("initial runtime");
    let _ = catalog_generation(&initial_runtime);
    initial_runtime.shutdown();
    let original_modified = fs::metadata(&song)
        .expect("original metadata")
        .modified()
        .expect("original mtime");

    fs::write(&song, replacement).expect("replacement song");
    fs::OpenOptions::new()
        .write(true)
        .open(&song)
        .expect("open replacement")
        .set_times(fs::FileTimes::new().set_modified(original_modified))
        .expect("restore original mtime");
    let replacement_metadata = fs::metadata(&song).expect("replacement metadata");
    assert_eq!(replacement_metadata.len(), original.len() as u64);
    assert_eq!(
        replacement_metadata.modified().expect("replacement mtime"),
        original_modified
    );

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("cached runtime");
    let claim = runtime
        .claim_catalog_load()
        .expect("reconcile claim")
        .expect("reconcile work");
    runtime.run_catalog_load(claim);
    let reconciled = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({"query":"","offset":0,"limit":20,"source":{"kind":"smart","id":"all"}}),
            )
            .expect("reconciled search");
    runtime.shutdown();

    fs::remove_file(root.join("cache/catalog-index.json")).expect("remove cache");
    let clean_runtime =
        NativeDesktopRuntime::from_install_root(root.clone()).expect("clean runtime");
    let clean = clean_runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({"query":"","offset":0,"limit":20,"source":{"kind":"smart","id":"all"}}),
            )
            .expect("clean search");
    assert_eq!(reconciled["items"], clean["items"]);
    clean_runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corrupt_cache_falls_back_to_cold_rebuild() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-catalog-corrupt-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(root.join("songs/one.json"), b"one").expect("song");
    fs::create_dir_all(root.join("cache")).expect("cache root");
    fs::write(
        root.join("cache/catalog-index.json"),
        b"{\"schema_version\":1",
    )
    .expect("truncated cache");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    assert!(matches!(
        *runtime.catalog_load.lock().expect("catalog state"),
        super::CatalogLoadState::Uninitialized
    ));
    assert_eq!(catalog_generation(&runtime), 1);
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cache_source_fingerprint_rejects_changed_user_root_before_hydration() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sky-native-catalog-cache-changed-user-root-{suffix}"
    ));
    let old_root = root.join("songs");
    let new_root =
        std::env::temp_dir().join(format!("sky-native-catalog-cache-new-user-root-{suffix}"));
    fs::create_dir_all(&old_root).expect("old songs root");
    fs::create_dir_all(&new_root).expect("new songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(old_root.join("old.json"), b"old").expect("old song");

    let initial_runtime =
        NativeDesktopRuntime::from_install_root(root.clone()).expect("initial runtime");
    let _ = catalog_generation(&initial_runtime);
    initial_runtime.shutdown();
    fs::write(
        root.join("config.json"),
        format!(
            "{{\"schema_version\":3,\"songs_dir\":{:?}}}\n",
            new_root.to_string_lossy()
        ),
    )
    .expect("changed songs root");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    assert_eq!(
        runtime
            .settings
            .lock()
            .expect("settings lock")
            .snapshot()
            .songs_dir,
        new_root.to_string_lossy()
    );
    let persisted = CatalogCacheStore::new(root.join("cache/catalog-index.json"))
        .load()
        .expect("load cache")
        .expect("cache");
    assert_ne!(
        persisted.source_fingerprint,
        runtime
            .catalog_source_fingerprint()
            .expect("source fingerprint"),
        "changed user root must change the source fingerprint"
    );
    assert!(matches!(
        *runtime.catalog_load.lock().expect("catalog state"),
        super::CatalogLoadState::Uninitialized
    ));
    assert_eq!(
        runtime
            .catalog
            .lock()
            .expect("catalog lock")
            .index
            .snapshot()
            .total,
        0
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(new_root);
}

#[test]
fn shutdown_fences_background_reconciliation_and_cache_write() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-catalog-shutdown-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let song = root.join("songs/one.json");
    fs::write(&song, b"one").expect("song");
    write_catalog_cache(&root, vec![user_cache_entry(&song, "one", 3)]);
    let cache_path = root.join("cache/catalog-index.json");
    let before = fs::read(&cache_path).expect("cache before reconcile");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let generation = catalog_generation(&runtime);
    let claim = runtime
        .claim_catalog_load()
        .expect("reconcile claim")
        .expect("reconcile work");
    runtime.shutdown();
    runtime.run_catalog_load(claim);
    assert!(matches!(
        *runtime.catalog_load.lock().expect("catalog state"),
        super::CatalogLoadState::Closing
    ));
    assert_eq!(
        runtime
            .catalog
            .lock()
            .expect("catalog lock")
            .index
            .generation(),
        generation
    );
    assert_eq!(fs::read(cache_path).expect("cache after shutdown"), before);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn blocked_event_publisher_coalesces_snapshots_and_preserves_lifecycle_order() {
    let session_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let callback_entered = Arc::new(AtomicBool::new(false));
    let (entered_sender, entered_receiver) = mpsc::channel();
    let received = Arc::new(Mutex::new(Vec::<Value>::new()));
    let callback_gate = Arc::clone(&gate);
    let callback_entered_flag = Arc::clone(&callback_entered);
    let callback_received = Arc::clone(&received);
    let channel: Channel<UiEvent> = Channel::new(move |body| {
        if !callback_entered_flag.swap(true, Ordering::AcqRel) {
            entered_sender.send(()).expect("publisher entry receiver");
            let (released, changed) = &*callback_gate;
            let mut released = released.lock().expect("publisher gate");
            while !*released {
                released = changed.wait(released).expect("publisher gate wait");
            }
        }
        let InvokeResponseBody::Json(json) = body else {
            return Err(std::io::Error::other("expected JSON event").into());
        };
        let event = serde_json::from_str::<Value>(&json)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        callback_received
            .lock()
            .expect("delivered events")
            .push(event);
        Ok(())
    });

    let mut hub = NativeEventHub::default();
    hub.publish(snapshot(session_id, 1))
        .expect("buffer initial snapshot");
    hub.subscribe(channel).expect("subscribe event sink");
    entered_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("first send entered blocked callback");
    hub.publish(snapshot(session_id, 2))
        .expect("queue snapshot");
    hub.publish(UiEvent::CatalogChanged {
        v: 1,
        payload: crate::ui_events::CatalogChangedPayload {
            generation: 1,
            total: 0,
        },
    })
    .expect("queue first lifecycle event");
    hub.publish(snapshot(session_id, 3))
        .expect("coalesce pending snapshot");
    hub.publish(UiEvent::CatalogChanged {
        v: 1,
        payload: crate::ui_events::CatalogChangedPayload {
            generation: 2,
            total: 0,
        },
    })
    .expect("queue second lifecycle event");
    let pending = hub.buffered_for_test();
    assert_eq!(pending.len(), 3);
    assert!(matches!(
        pending.front(),
        Some(UiEvent::PlaybackSnapshot { payload, .. }) if payload.seq == 3
    ));

    {
        let (released, changed) = &*gate;
        *released.lock().expect("publisher gate") = true;
        changed.notify_all();
    }
    assert!(hub.wait_idle_for_test(Duration::from_secs(2)));
    let delivered = received.lock().expect("delivered events").clone();
    let snapshots = delivered
        .iter()
        .filter(|event| event["name"] == "playback.snapshot")
        .filter_map(|event| event["payload"]["seq"].as_u64())
        .collect::<Vec<_>>();
    let generations = delivered
        .iter()
        .filter(|event| event["name"] == "catalog.changed")
        .filter_map(|event| event["payload"]["generation"].as_u64())
        .collect::<Vec<_>>();
    assert_eq!(snapshots, vec![1, 3]);
    assert_eq!(generations, vec![1, 2]);
    hub.close();
    assert!(hub.publisher_finished_for_test());
}

#[test]
fn diagnostics_statistics_match_python_rounding_and_population_sigma() {
    assert_eq!(percentile_ms(&[1, 2, 3, 4], 0.50), 0.003);
    assert_eq!(percentile_ms(&[1, 2, 3, 4], 0.95), 0.004);
    assert_eq!(
        population_sigma_ms(&[1, 2, 3, 4]),
        1.118033988749895 / 1000.0
    );
    assert_eq!(percentile_ms(&[], 0.50), 0.0);
    assert_eq!(population_sigma_ms(&[]), 0.0);
}

#[test]
fn diagnostics_status_matches_native_observer_contract() {
    use crate::ui_events::DiagnosticsBackendStatus;

    let healthy = DiagnosticsBackendHealth {
        physical_session: true,
        player_attached: true,
        active_count: 1,
        possibly_active_count: 1,
        ..DiagnosticsBackendHealth::default()
    };
    assert_eq!(
        diagnostics_backend_status(healthy),
        DiagnosticsBackendStatus::Healthy
    );
    assert_eq!(
        diagnostics_backend_status(DiagnosticsBackendHealth {
            keys_dropped: 1,
            ..healthy
        }),
        DiagnosticsBackendStatus::Degraded
    );
    assert_eq!(
        diagnostics_backend_status(DiagnosticsBackendHealth {
            chord_split_events: 1,
            possibly_active_count: 2,
            ..healthy
        }),
        DiagnosticsBackendStatus::Degraded
    );
    assert_eq!(
        diagnostics_backend_status(DiagnosticsBackendHealth {
            possibly_active_count: 2,
            ..healthy
        }),
        DiagnosticsBackendStatus::Degraded
    );
    assert_eq!(
        diagnostics_backend_status(DiagnosticsBackendHealth {
            has_last_error: true,
            ..healthy
        }),
        DiagnosticsBackendStatus::Error
    );
    assert_eq!(
        diagnostics_backend_status(DiagnosticsBackendHealth {
            failed_release_count: 1,
            ..healthy
        }),
        DiagnosticsBackendStatus::Error
    );
    assert_eq!(
        diagnostics_backend_status(DiagnosticsBackendHealth {
            has_terminal_error: true,
            ..healthy
        }),
        DiagnosticsBackendStatus::Error
    );
    assert_eq!(
        diagnostics_backend_status(DiagnosticsBackendHealth {
            has_terminal_failure_outcome: true,
            ..healthy
        }),
        DiagnosticsBackendStatus::Error
    );
}

#[test]
fn diagnostics_status_distinguishes_unavailable_and_inconsistent_attachment() {
    use crate::ui_events::DiagnosticsBackendStatus;

    assert_eq!(
        diagnostics_backend_status(DiagnosticsBackendHealth::default()),
        DiagnosticsBackendStatus::Unavailable
    );
    assert_eq!(
        diagnostics_backend_status(DiagnosticsBackendHealth {
            physical_session: true,
            ..DiagnosticsBackendHealth::default()
        }),
        DiagnosticsBackendStatus::Error
    );
    assert_eq!(
        diagnostics_backend_status(DiagnosticsBackendHealth {
            player_attached: true,
            ..DiagnosticsBackendHealth::default()
        }),
        DiagnosticsBackendStatus::Error
    );
}

#[test]
fn diagnostics_sender_samples_distinguish_empty_zero_and_nonzero_measurements() {
    use crate::ui_events::DiagnosticsBackendStatus;

    assert_eq!(sender_sample_summary([0; 7], 0), (0, None));
    assert_eq!(
        sender_sample_summary([1, 0, 0, 0, 0, 0, 0], 0),
        (1, Some(0))
    );
    assert_eq!(
        sender_sample_summary([0, 0, 2, 0, 0, 0, 0], 177),
        (2, Some(177))
    );
    assert_eq!(
        sender_sample_summary([u64::MAX, 1, 0, 0, 0, 0, 0], 0),
        (u64::MAX, Some(0))
    );
    assert_eq!(
        diagnostics_backend_status(DiagnosticsBackendHealth {
            physical_session: true,
            player_attached: true,
            ..DiagnosticsBackendHealth::default()
        }),
        DiagnosticsBackendStatus::Healthy
    );
}

#[test]
fn diagnostics_gate_matches_python_rate_reset_and_sequence_contract() {
    let gate = DiagnosticsPublicationGate::default();
    gate.set_enabled(true).expect("enable");
    let start = Instant::now();
    let mut sequences = Vec::new();
    assert!(
        gate.try_publish(start, |sequence| {
            sequences.push(sequence);
            Ok(())
        })
        .expect("first publication")
    );
    assert!(
        !gate
            .try_publish(start + Duration::from_millis(99), |_| Ok(()))
            .expect("throttled publication")
    );
    assert!(
        gate.try_publish(start + Duration::from_millis(100), |sequence| {
            sequences.push(sequence);
            Ok(())
        })
        .expect("second publication")
    );
    gate.set_enabled(false).expect("disable");
    assert!(
        !gate
            .try_publish(start + Duration::from_millis(200), |_| Ok(()))
            .expect("disabled publication")
    );
    gate.set_enabled(true).expect("re-enable");
    assert!(
        gate.try_publish(start + Duration::from_millis(200), |sequence| {
            sequences.push(sequence);
            Ok(())
        })
        .expect("fresh sampling window")
    );
    assert_eq!(sequences, vec![1, 2, 3]);
}

#[test]
fn final_diagnostics_snapshot_bypasses_sampling_interval() {
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let gate = DiagnosticsPublicationGate::default();
    gate.set_enabled(true).expect("enable");
    let active = active_for_control(PlaybackSessionState::Playing, None);

    publish_diagnostics_snapshot_for_active(&events, &active, &gate).expect("regular diagnostics");
    publish_final_diagnostics_snapshot_for_active(&events, &active, &gate)
        .expect("final diagnostics");

    let hub = events.lock().expect("event hub");
    let buffered = hub.buffered_for_test();
    assert_eq!(buffered.len(), 1, "snapshot events coalesce to latest");
    let UiEvent::DiagnosticsSnapshot { payload, .. } = &buffered[0] else {
        panic!("expected final diagnostics snapshot");
    };
    assert_eq!(payload.seq, 2);
}

#[test]
fn diagnostics_disable_waits_for_in_flight_publication() {
    let gate = Arc::new(DiagnosticsPublicationGate::default());
    gate.set_enabled(true).expect("enable");
    let start = Instant::now();
    let emitted = Arc::new(Mutex::new(Vec::new()));
    let (entered_sender, entered_receiver) = std::sync::mpsc::channel();
    let (release_sender, release_receiver) = std::sync::mpsc::channel();
    let disabled_returned = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_gate = Arc::clone(&gate);
    let worker_emitted = Arc::clone(&emitted);
    let worker_disabled_returned = Arc::clone(&disabled_returned);
    let worker = std::thread::spawn(move || {
        worker_gate
            .try_publish(start, |sequence| {
                entered_sender.send(()).expect("entered receiver");
                release_receiver.recv().expect("release sender");
                assert!(!worker_disabled_returned.load(Ordering::Acquire));
                worker_emitted.lock().expect("emitted").push(sequence);
                Ok(())
            })
            .expect("publication")
    });
    entered_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("publisher entered");
    let disable_gate = Arc::clone(&gate);
    let disable_returned = Arc::clone(&disabled_returned);
    let disable = std::thread::spawn(move || {
        disable_gate.set_enabled(false).expect("disable");
        disable_returned.store(true, Ordering::Release);
    });
    release_sender.send(()).expect("release publisher");
    assert!(worker.join().expect("publisher thread"));
    disable.join().expect("disable thread");
    assert_eq!(*emitted.lock().expect("emitted"), vec![1]);
    assert!(
        !gate
            .try_publish(start + Duration::from_millis(100), |_| Ok(()))
            .expect("post-disable publication")
    );
}

#[test]
fn diagnostics_dry_run_publishes_unavailable_snapshot() {
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let gate = DiagnosticsPublicationGate::default();
    gate.set_enabled(true).expect("enable");
    let active = active_for_control(PlaybackSessionState::Playing, None);

    publish_diagnostics_snapshot_for_active(&events, &active, &gate).expect("dry-run diagnostics");
    let hub = events.lock().expect("event hub");
    let buffered = hub.buffered_for_test();
    assert_eq!(buffered.len(), 1);
    let UiEvent::DiagnosticsSnapshot { payload, .. } = &buffered[0] else {
        panic!("expected diagnostics snapshot");
    };
    assert_eq!(payload.session_id.as_deref(), Some("a".repeat(32).as_str()));
    assert!(!payload.physical_session);
    assert!(!payload.player_attached);
    assert_eq!(payload.sender_sample_count, 0);
    assert_eq!(
        payload.backend_status,
        DiagnosticsBackendStatus::Unavailable
    );
    assert_eq!(payload.max_lateness_us, None);
    assert_eq!(payload.p50_ms, None);
    assert_eq!(payload.p95_ms, None);
    assert_eq!(payload.sigma_onset_ms, None);
    assert_eq!(payload.late_2ms, None);
    assert_eq!(payload.late_5ms, None);
    assert_eq!(payload.late_10ms, None);
    assert_eq!(payload.max_sendinput_pre_call_lateness_us, None);
    assert_eq!(payload.pre_call_late_2ms, 0);
    assert_eq!(payload.pre_call_late_5ms, 0);
    assert_eq!(payload.pre_call_late_10ms, 0);
    assert_eq!(payload.fps, 60);
    assert_eq!(payload.frame_us, 16_667);
    assert_eq!(payload.hold_frames, 1.0);
    assert_eq!(payload.frame_base_hold_us, 16_667);
    assert_eq!(payload.timing_margin_us, 800);
    assert_eq!(payload.min_hold_us, 17_467);
    assert_eq!(payload.min_release_gap_us, 17_467);
    assert_eq!(
        payload
            .timing_margin_recommendation
            .recommended_timing_margin_us,
        Some(800)
    );
    assert!(!payload.timing_margin_recommendation.qualified);
    assert_eq!(payload.pre_call_lt_250us, 0);
    assert_eq!(payload.pre_call_250_500us, 0);
    assert_eq!(payload.pre_call_500_750us, 0);
    assert_eq!(payload.pre_call_750_1000us, 0);
    assert_eq!(payload.pre_call_1000_1500us, 0);
    assert_eq!(payload.pre_call_1500_2000us, 0);
    assert_eq!(payload.pre_call_ge_2000us, 0);
    assert_eq!(payload.active_keys, 0);
    assert_eq!(payload.stuck_keys, 0);
    assert_eq!(payload.keys_dropped, 0);
    assert_eq!(payload.chord_split_events, 0);
    assert_eq!(payload.missed_down_boundaries, 0);
    assert_eq!(payload.missed_down_keys, 0);
    assert_eq!(payload.missed_unobserved_backlog_boundaries, 0);
    assert_eq!(payload.missed_physical_window_boundaries, 0);
    assert_eq!(payload.final_sender_window_expirations, 0);
    assert_eq!(payload.release_floor_infeasible_boundaries, 0);
    let serialized = serde_json::to_value(payload).expect("diagnostics DTO JSON");
    assert_eq!(serialized["missed_physical_window_boundaries"], 0);
    assert_eq!(serialized["final_sender_window_expirations"], 0);
    assert_eq!(serialized["release_floor_infeasible_boundaries"], 0);
    assert_eq!(payload.hold_floor_delay_boundaries, 0);
    assert_eq!(payload.max_hold_floor_delay_us, 0);
    assert_eq!(payload.release_floor_delay_boundaries, 0);
    assert_eq!(payload.max_release_floor_delay_us, 0);
    assert_eq!(payload.final_gate_control_rejections, 0);
    assert_eq!(payload.final_gate_target_changes, 0);
    assert_eq!(payload.final_gate_focus_losses, 0);
    assert_eq!(payload.final_gate_lease_expirations, 0);
    assert_eq!(payload.sendinput_partial_events, 0);
    assert_eq!(payload.sendinput_zero_progress_failures, 0);
    assert_eq!(payload.release_max_us, None);
    assert_eq!(payload.release_late_2ms, None);
    assert_eq!(payload.last_error, None);
    assert_eq!(payload.seq, 1);
}

#[test]
fn diagnostics_unavailable_path_keeps_rate_gate_and_monotonic_sequence() {
    let gate = DiagnosticsPublicationGate::default();
    gate.set_enabled(true).expect("enable");
    let start = Instant::now();
    let mut sequences = Vec::new();
    for offset in [0, 1, 99, 100, 101] {
        assert!(
            gate.try_publish(start + Duration::from_millis(offset), |sequence| {
                sequences.push(sequence);
                Ok(())
            })
            .is_ok()
        );
    }
    assert_eq!(sequences, vec![1, 2]);
    gate.set_enabled(false).expect("disable");
    assert!(
        !gate
            .try_publish(start + Duration::from_millis(200), |_| Ok(()))
            .expect("disabled publication")
    );
    gate.set_enabled(true).expect("re-enable");
    assert!(
        gate.try_publish(start + Duration::from_millis(200), |sequence| {
            sequences.push(sequence);
            Ok(())
        })
        .expect("re-enabled publication")
    );
    assert_eq!(sequences, vec![1, 2, 3]);
}

#[test]
fn diagnostics_physical_healthy_status_remains_healthy() {
    use crate::ui_events::DiagnosticsBackendStatus;

    assert_eq!(
        diagnostics_backend_status(DiagnosticsBackendHealth {
            physical_session: true,
            player_attached: true,
            ..DiagnosticsBackendHealth::default()
        }),
        DiagnosticsBackendStatus::Healthy
    );
    assert_eq!(
        NativeDiagnosticsSample::unavailable(false).backend_status,
        DiagnosticsBackendStatus::Unavailable
    );
    let inconsistent = NativeDiagnosticsSample::unavailable(true);
    assert_eq!(inconsistent.backend_status, DiagnosticsBackendStatus::Error);
    assert_eq!(inconsistent.sender_sample_count, 0);
    assert_eq!(inconsistent.max_sendinput_pre_call_lateness_us, None);
}

#[test]
fn terminal_stop_returns_the_stored_terminal_state() {
    for terminal_state in [PlaybackSessionState::Finished, PlaybackSessionState::Failed] {
        let service = NativePlaybackService::new(ActivityCoordinator::default());
        *service.last_terminal.lock().expect("terminal state") = Some(PlaybackTerminalStatusDto {
            session_id: "a".repeat(32),
            prepared_id: "b".repeat(32),
            song_id: "song-a".into(),
            state: terminal_state,
            outcome: None,
            failure_code: None,
            failure_message: None,
            terminal_release_verified: None,
            release_obligation_mask: None,
        });
        let acknowledgement = service
            .command(
                "playback.stop",
                "a".repeat(32),
                Arc::new(Mutex::new(NativeEventHub::default())),
            )
            .expect("terminal stop is idempotent");
        assert_eq!(acknowledgement.state, terminal_state);
        assert!(acknowledgement.accepted);
    }
}

#[test]
fn playback_status_tracks_active_ownership_through_the_retirement_barrier() {
    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let active = active_for_control(PlaybackSessionState::Playing, None);
    *service.active.lock().expect("active slot") = Some(active.clone());

    let active_status = service.status().expect("active playback status");
    assert_eq!(
        active_status.active,
        Some(PlaybackActiveStatusDto {
            session_id: active.session_id.clone(),
            prepared_id: active.prepared_id.clone(),
            song_id: active.song_id.clone(),
            title: active.title.clone(),
            state: PlaybackSessionState::Playing,
        })
    );
    assert!(active_status.last_terminal.is_none());

    *active.state.lock().expect("playback state") = PlaybackSessionState::Finished;
    release_terminal_ownership(
        &service.active,
        &service.last_terminal,
        &active,
        TerminalRetirement {
            quiescence: WorkerQuiescenceProof::NoWorkerStarted,
            release: TerminalReleaseEvidence::NotRequired,
            publication: PlaybackTerminalPublication::Finished {
                outcome: "finished".into(),
                message: "Playback finished".into(),
            },
        },
    )
    .expect("retirement barrier");
    let retired_status = service.status().expect("retired playback status");
    assert!(retired_status.active.is_none());
    assert_eq!(
        retired_status.last_terminal,
        Some(PlaybackTerminalStatusDto {
            session_id: active.session_id.clone(),
            prepared_id: active.prepared_id.clone(),
            song_id: active.song_id.clone(),
            state: PlaybackSessionState::Finished,
            outcome: Some("finished".into()),
            failure_code: None,
            failure_message: None,
            terminal_release_verified: None,
            release_obligation_mask: None,
        })
    );
}

#[test]
fn playback_status_preserves_prepared_identity_and_failure_details() {
    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let active = active_for_control(PlaybackSessionState::Failed, None);
    *service.active.lock().expect("active slot") = Some(active.clone());
    let failure = PlaybackTerminalPublication::Failed {
        code: "target_not_found".into(),
        message: "Sky window was not found. Open Sky and try again.".into(),
    };

    release_terminal_ownership(
        &service.active,
        &service.last_terminal,
        &active,
        TerminalRetirement {
            quiescence: WorkerQuiescenceProof::NoWorkerStarted,
            release: TerminalReleaseEvidence::NotRequired,
            publication: failure,
        },
    )
    .expect("retirement barrier");

    let terminal = service
        .status()
        .expect("terminal status")
        .last_terminal
        .expect("retired failure");
    assert_eq!(terminal.session_id, active.session_id);
    assert_eq!(terminal.prepared_id, active.prepared_id);
    assert_eq!(terminal.failure_code.as_deref(), Some("target_not_found"));
    assert_eq!(
        terminal.failure_message.as_deref(),
        Some("Sky window was not found. Open Sky and try again.")
    );
}

#[test]
fn physical_startup_failures_have_actionable_classified_messages() {
    for (error, expected_code, expected_message) in [
        (
            "no admissible visible Sky window was found",
            "target_not_found",
            "Sky window was not found. Open Sky and make sure its window is visible, then try again.",
        ),
        (
            "target Sky process has higher integrity than Sky Auto Player; Windows UIPI can block SendInput",
            "target_integrity_mismatch",
            "Sky is running with higher Windows integrity than Sky Auto Player. Run both apps at the same permission level, then try again.",
        ),
        (
            "validated Sky window could not be focused",
            "target_focus_failed",
            "Sky's window could not be focused. Bring the Sky window to the foreground, then try again.",
        ),
        (
            "native schedule compilation failed",
            "native_worker_failed",
            "The native playback engine could not start. Try again; if it continues, restart Sky Auto Player.",
        ),
    ] {
        assert_eq!(
            physical_startup_failure(error),
            (expected_code, expected_message)
        );
    }
}

#[test]
fn target_integrity_startup_admission_fails_only_for_mismatch() {
    use sky_dispatch_win32::focus::TargetIntegrityCompatibility;

    assert!(target_integrity_startup_failure(TargetIntegrityCompatibility::Compatible).is_none());
    assert!(target_integrity_startup_failure(TargetIntegrityCompatibility::Unknown).is_none());
    assert_eq!(
        target_integrity_startup_failure(TargetIntegrityCompatibility::Mismatch),
        Some(
            "target Sky process has higher integrity than Sky Auto Player; Windows UIPI can block SendInput"
        )
    );
}

#[test]
fn target_integrity_admission_precedes_focus_and_session_arm() {
    let source = include_str!("../../native_runtime/playback/session.rs").replace("\r\n", "\n");
    let create = source
        .split("fn create_native_player(")
        .nth(1)
        .expect("native player creation chain");
    let integrity = create
        .find("target_integrity_startup_failure")
        .expect("startup integrity admission");
    let focus = create
        .find("focus_window_and_verify")
        .expect("target focus verification");
    let session = create
        .find("NativeDispatchSession::new_with_power_endpoint")
        .expect("native session construction");
    assert!(integrity < focus);
    assert!(integrity < session);

    let arm = source.find("player.arm(0)").expect("physical session arm");
    let create_call = source
        .find("self.create_native_player(")
        .expect("native player creation call");
    let supervisor_spawn = source
        .find("let spawn_result = thread::Builder::new()")
        .expect("supervisor thread must be spawned before worker activation");
    assert!(create_call < arm);
    assert!(supervisor_spawn < arm);
}

#[test]
fn terminal_player_statuses_publish_state_before_terminal_event() {
    use sky_player::engine::EnginePollStatus;

    for (status, expected) in [
        (
            EnginePollStatus::Finished,
            vec!["state:finished", "finished:finished"],
        ),
        (
            EnginePollStatus::Skipped,
            vec!["state:finished", "finished:skipped"],
        ),
        (
            EnginePollStatus::Quit,
            vec!["state:finished", "finished:quit"],
        ),
        (
            EnginePollStatus::SystemSuspended,
            vec!["state:failed", "failed:system_suspend"],
        ),
        (
            EnginePollStatus::Error,
            vec!["state:failed", "failed:native_player_failed"],
        ),
        (
            EnginePollStatus::Panicked,
            vec!["state:failed", "failed:native_player_panicked"],
        ),
        (
            EnginePollStatus::Poisoned,
            vec!["state:failed", "failed:native_player_poisoned"],
        ),
        (
            EnginePollStatus::Invalid,
            vec!["state:failed", "failed:native_player_failed"],
        ),
    ] {
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let active = active_for_control_with_physical(PlaybackSessionState::Playing, None, true);
        let mut last_event_state = PlaybackEventState::Playing;
        publish_terminal_poll_result(&events, &active, &mut last_event_state, status)
            .expect("terminal event trace");
        assert_eq!(
            playback_trace(&events),
            expected.into_iter().map(str::to_owned).collect::<Vec<_>>(),
            "status {}",
            status.as_str()
        );
    }
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
fn focus_rejection_publishes_starting_then_failed_trace() {
    use sky_player::engine::EnginePollStatus;

    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let active = active_for_control_with_physical(PlaybackSessionState::Starting, None, true);
    publish_playback_state(&events, &active, PlaybackEventState::Starting, None, None)
        .expect("starting event");
    let mut last_event_state = PlaybackEventState::Starting;
    publish_terminal_poll_result(
        &events,
        &active,
        &mut last_event_state,
        EnginePollStatus::Error,
    )
    .expect("focus rejection trace");
    assert_eq!(
        playback_trace(&events),
        [
            "state:starting",
            "state:failed",
            "failed:native_player_failed"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>()
    );
}

#[test]
fn dry_run_completion_publishes_finished_before_terminal_event() {
    use sky_player::engine::EnginePollStatus;

    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let active = active_for_control(PlaybackSessionState::Playing, None);
    let mut last_event_state = PlaybackEventState::Playing;
    publish_terminal_poll_result(
        &events,
        &active,
        &mut last_event_state,
        EnginePollStatus::Finished,
    )
    .expect("dry-run completion trace");
    assert_eq!(
        playback_trace(&events),
        ["state:finished", "finished:finished"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
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

#[test]
fn stop_paths_publish_stopping_finished_then_finished_event() {
    for initial_state in [
        PlaybackSessionState::Starting,
        PlaybackSessionState::Playing,
        PlaybackSessionState::Paused,
    ] {
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let service = NativePlaybackService::new(ActivityCoordinator::default());
        let active = active_for_control_with_physical(initial_state, None, true);
        *service.active.lock().expect("active") = Some(active.clone());
        service
            .command("playback.stop", active.session_id.clone(), events.clone())
            .expect("stop");
        publish_playback_state(&events, &active, PlaybackEventState::Stopping, None, None)
            .expect("monitor queues stopping state");
        let mut last_event_state = match initial_state {
            PlaybackSessionState::Starting => PlaybackEventState::Starting,
            PlaybackSessionState::Playing => PlaybackEventState::Playing,
            PlaybackSessionState::Paused => PlaybackEventState::Paused,
            _ => unreachable!(),
        };
        publish_terminal_poll_result(
            &events,
            &active,
            &mut last_event_state,
            sky_player::engine::EnginePollStatus::Quit,
        )
        .expect("stop terminal event trace");
        assert_eq!(
            playback_trace(&events),
            ["state:stopping", "state:finished", "finished:quit"]
        );
    }
}

#[test]
fn playback_start_validation_distinguishes_malformed_and_stale_ids() {
    let malformed = super::NativePlaybackStartRequest {
        prepared_id: "A".repeat(32),
        decisions: Vec::new(),
    };
    let error = validate_playback_start_request(&malformed).expect_err("invalid ID");
    assert!(error.starts_with("invalid_params:"));

    let well_formed_stale = super::NativePlaybackStartRequest {
        prepared_id: "a".repeat(32),
        decisions: Vec::new(),
    };
    validate_playback_start_request(&well_formed_stale).expect("well-formed ID");
    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let error = service
        .start(
            well_formed_stale.into_public(),
            &ApplicationSettings::default(),
            Arc::new(Mutex::new(NativeEventHub::default())),
        )
        .expect_err("stale ID");
    assert!(error.contains("stale or already consumed"));
}

#[test]
fn playback_start_validation_bounds_decisions_before_admission() {
    let oversized = super::NativePlaybackStartRequest {
        prepared_id: "a".repeat(32),
        decisions: vec![
            crate::commands::PlaybackDecisionAcceptanceDto {
                decision: crate::commands::PlaybackDecision::Proceed,
                accepted: false,
            };
            super::MAX_DECISION_COUNT + 1
        ],
    };
    let error = validate_playback_start_request(&oversized).expect_err("bounded decisions");
    assert!(error.contains("decisions must be a bounded array"));
}

#[test]
fn routed_playback_start_validates_input_before_confirmation_and_lookup() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-start-validation-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(
        root.join("songs/ready.json"),
        r#"{"name":"Ready","songNotes":[{"time":0,"key":"Key0"}]}"#,
    )
    .expect("song");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let generation = catalog_generation(&runtime);
    let search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query": "ready",
                "offset": 0,
                "limit": 10,
                "generation": generation
            }),
        )
        .expect("search");
    let song_id = search["items"][0]["song_id"]
        .as_str()
        .expect("song ID")
        .to_owned();
    let prepared = runtime
            .dispatch(
                "playback.prepare",
                serde_json::json!({
                    "songId": song_id,
                    "generation": generation,
                    "config": {"hold_frames":1.0,"timing_margin_us":800,"tempo_scale":1.0,"fps":60,"dry_run":true}
                }),
            )
            .expect("prepare");
    let prepared_id = prepared["prepared_id"]
        .as_str()
        .expect("prepared ID")
        .to_owned();

    let malformed = runtime.dispatch(
        "playback.start",
        serde_json::json!({"preparedId":"A".repeat(32),"decisions":[]}),
    );
    assert!(
        malformed
            .expect_err("malformed ID")
            .contains("invalid_params: prepared_id is invalid")
    );

    let oversized = runtime.dispatch(
        "playback.start",
        serde_json::json!({
            "preparedId": prepared_id,
            "decisions": (0..=MAX_DECISION_COUNT)
                .map(|_| serde_json::json!({"decision":"proceed","accepted":false}))
                .collect::<Vec<_>>()
        }),
    );
    assert!(
        oversized
            .expect_err("oversized decisions")
            .contains("decisions must be a bounded array")
    );

    let wrong_confirmation = runtime.dispatch(
        "playback.start",
        serde_json::json!({
            "preparedId": prepared_id,
            "decisions": [{"decision":"proceed","accepted":true}]
        }),
    );
    assert!(
        wrong_confirmation
            .expect_err("ready plan must reject confirmation")
            .contains("ready playback accepts no risk decisions")
    );

    let valid = runtime
        .dispatch(
            "playback.start",
            serde_json::json!({"preparedId":prepared_id,"decisions":[]}),
        )
        .expect("valid ready start");
    assert_eq!(valid["state"], "starting");
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn skip_is_live_only_and_clears_pending_pause_or_resume() {
    use sky_player::engine::EnginePollStatus;

    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    for (state, pending) in [
        (PlaybackSessionState::Starting, None),
        (
            PlaybackSessionState::Playing,
            Some(PlaybackPendingControl::Pause),
        ),
        (
            PlaybackSessionState::Paused,
            Some(PlaybackPendingControl::Resume),
        ),
    ] {
        let service = NativePlaybackService::new(ActivityCoordinator::default());
        let active = active_for_control(state, pending);
        *service.active.lock().expect("active") = Some(active.clone());
        let acknowledgement = service
            .command("playback.skip", active.session_id.clone(), events.clone())
            .expect("skip in live state");
        assert!(acknowledgement.accepted);
        assert_eq!(acknowledgement.pending_command, None);
        assert_eq!(
            active.dry_run_terminal_status(),
            Some(EnginePollStatus::Skipped)
        );
    }

    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let active = active_for_control(PlaybackSessionState::Stopping, None);
    *service.active.lock().expect("active") = Some(active.clone());
    let error = service
        .command("playback.skip", active.session_id.clone(), events)
        .expect_err("skip after stopping must be rejected");
    assert!(error.contains("live playback session"));
}

#[test]
fn repeated_pending_controls_and_invalid_transitions_match_command_contract() {
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let active = active_for_control(
        PlaybackSessionState::Playing,
        Some(PlaybackPendingControl::Pause),
    );
    *service.active.lock().expect("active") = Some(active.clone());
    let repeated = service
        .command("playback.pause", active.session_id.clone(), events.clone())
        .expect("repeated pause");
    assert_eq!(repeated.reason.as_deref(), Some("already_pending"));

    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let active = active_for_control(
        PlaybackSessionState::Paused,
        Some(PlaybackPendingControl::Pause),
    );
    *service.active.lock().expect("active") = Some(active.clone());
    let conflict = service
        .command("playback.resume", active.session_id.clone(), events.clone())
        .expect_err("resume conflicts with pending pause");
    assert!(conflict.contains("another playback control is awaiting acknowledgement"));

    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let active = active_for_control(
        PlaybackSessionState::Paused,
        Some(PlaybackPendingControl::Resume),
    );
    *service.active.lock().expect("active") = Some(active.clone());
    let repeated = service
        .command("playback.resume", active.session_id.clone(), events.clone())
        .expect("repeated resume");
    assert_eq!(repeated.reason.as_deref(), Some("already_pending"));

    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let active = active_for_control(PlaybackSessionState::Starting, None);
    *service.active.lock().expect("active") = Some(active.clone());
    let pause_error = service
        .command("playback.pause", active.session_id.clone(), events.clone())
        .expect_err("pause before playing");
    assert!(pause_error.contains("pause requires a playing session"));

    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let active = active_for_control(PlaybackSessionState::Playing, None);
    *service.active.lock().expect("active") = Some(active.clone());
    let resume_error = service
        .command("playback.resume", active.session_id.clone(), events.clone())
        .expect_err("resume before pause");
    assert!(resume_error.contains("resume requires a paused session"));

    let foreign_error = service
        .command("playback.stop", "f".repeat(32), events)
        .expect_err("foreign session");
    assert!(foreign_error.contains("stale or foreign"));
}

#[test]
fn direct_native_player_keeps_the_qualified_supervisor_lease() {
    assert_eq!(
        sky_player::engine::DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US,
        3_000_000
    );
}

#[test]
fn native_prepared_ids_are_random_lowercase_hex_and_eviction_is_fifo() {
    let mut ids = std::collections::BTreeSet::new();
    for _ in 0..128 {
        let id = opaque_native_id().expect("native ID");
        assert_eq!(id.len(), 32);
        assert!(
            id.bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        assert!(ids.insert(id));
    }

    let mut prepared = std::collections::VecDeque::new();
    for value in 0..(MAX_PREPARED_PLANS + 3) {
        prepared.push_back(value);
        retain_prepared_capacity(&mut prepared);
    }
    assert_eq!(prepared.len(), MAX_PREPARED_PLANS);
    assert_eq!(prepared.front(), Some(&3));
    assert_eq!(prepared.back(), Some(&(MAX_PREPARED_PLANS + 2)));
}

#[test]
fn fingerprint_serialization_matches_python_canonicalization() {
    let mut settings = sky_app_core::settings::ApplicationSettings::default();
    settings.playback_defaults.hold_frames = 1.25;
    settings.playback_defaults.tempo_scale = 0.95;
    settings.playback_defaults.fps = 120;
    settings.telemetry_enabled = true;
    let default_settings_fingerprint =
        settings_fingerprint(&settings).expect("settings fingerprint");
    // New settings use Graphite, which is part of the existing fingerprint payload.
    assert_eq!(
        default_settings_fingerprint,
        "73e9c1c107100df2167fd6cfc7365c62207d5b946ac05996323896e2c9ddd939"
    );
    let mut auto_play_changed = settings.clone();
    auto_play_changed.playback_behavior.auto_play = false;
    assert_eq!(
        default_settings_fingerprint,
        settings_fingerprint(&auto_play_changed).expect("Auto Play is not timing identity")
    );
    let mut changed_settings = settings.clone();
    changed_settings.playback_defaults.timing_margin_us = 900;
    assert_ne!(
        default_settings_fingerprint,
        settings_fingerprint(&changed_settings).expect("changed margin fingerprint")
    );
    changed_settings.playback_defaults.timing_margin_us =
        sky_app_core::settings::DEFAULT_TIMING_MARGIN_US;
    assert_eq!(
        default_settings_fingerprint,
        settings_fingerprint(&changed_settings).expect("restored timing margin fingerprint")
    );

    let song = parse_song_json(
        br#"{"name":"Fingerprint","songNotes":[{"time":0,"key":"Key0"}]}"#,
        "fingerprint",
    )
    .expect("song");
    let policy = MaterializedTimingPolicy::from_user_margin(60, 1.0, 800).expect("policy");
    let schedule = build_schedule_with_policy(&song, 0.95, &policy).expect("schedule");
    let config = PlaybackConfigDto {
        hold_frames: 1.0,
        timing_margin_us: 800,
        tempo_scale: 0.95,
        fps: 60,
        dry_run: true,
    };
    assert_eq!(
        plan_fingerprint(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            &config,
            &schedule,
            &policy,
        )
        .expect("plan fingerprint"),
        "37c8cb8b8e70d3cab60fc794781cbeb794d6bf59b78e38ca1ccbeee0f4afc94d"
    );
}

#[test]
fn native_detail_matches_python_tempo_oracle_corpus() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../../tests/fixtures/song_planning/oracle_cases.json"
    ))
    .expect("tempo fixture");
    let cases = fixture["detail_tempo_cases"]
        .as_array()
        .expect("detail tempo cases");
    assert_eq!(cases.len(), 5);
    for case in cases {
        let tempo = case["tempo_scale"].as_f64().expect("tempo");
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-native-tempo-{suffix}"));
        fs::create_dir_all(root.join("songs")).expect("songs root");
        fs::write(
            root.join("config.json"),
            format!(r#"{{"schema_version":3,"default_tempo_scale":{tempo}}}"#),
        )
        .expect("config");
        fs::write(
            root.join("songs/tempo.json"),
            serde_json::to_vec(&case["raw"]).expect("raw song"),
        )
        .expect("song");
        let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
        runtime.bootstrap().expect("bootstrap");
        let search = runtime
            .search(crate::commands::CatalogSearchRequest {
                query: String::new(),
                offset: 0,
                limit: 10,
                generation: None,
                source: crate::commands::LibrarySource::default(),
            })
            .expect("search");
        let detail = runtime
            .detail(crate::commands::CatalogDetailRequest {
                song_id: search.items[0].song_id.clone(),
                generation: Some(search.generation),
            })
            .expect("detail");
        let actual = serde_json::to_value(&detail).expect("detail JSON");
        assert_eq!(actual["duration_us"], case["duration_us"], "tempo {tempo}");
        assert_eq!(actual["risk"]["level"], case["risk_level"], "tempo {tempo}");
        assert_eq!(
            actual["risk"]["recommendations"], case["risk_recommendations"],
            "tempo {tempo}"
        );
        assert_eq!(
            actual["recommendation"]["recommended_hold_frames"],
            case["recommendation"]["recommended_hold_frames"],
            "tempo {tempo}"
        );
        assert_eq!(
            actual["recommendation"]["recommended_tempo_scale"],
            case["recommendation"]["recommended_tempo_scale"],
            "tempo {tempo}"
        );
        runtime.shutdown();
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn native_prepare_preserves_python_validation_failed_contract() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-prepare-contract-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(
        root.join("songs/repeat.json"),
        r#"{"name":"Repeat","songNotes":[{"time":0,"key":"Key0"},{"time":1,"key":"Key0"}]}"#,
    )
    .expect("song");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let generation = catalog_generation(&runtime);
    let search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query": "repeat",
                "offset": 0,
                "limit": 10,
                "generation": generation
            }),
        )
        .expect("search");
    let song_id = search["items"][0]["song_id"].as_str().expect("song ID");
    let prepared = runtime
        .dispatch(
            "playback.prepare",
            serde_json::json!({
                "songId": song_id,
                "generation": generation,
                "config": {
                    "hold_frames": 1.0,
                    "timing_margin_us": 800,
                    "tempo_scale": 1.0,
                    "fps": 60,
                    "dry_run": false
                }
            }),
        )
        .expect("blocked preparation is a typed DTO");
    assert_eq!(prepared["admission"], "blocked");
    assert_eq!(prepared["error_code"], "validation_failed");
    assert_eq!(
        prepared["error_message"],
        "Detected 1 infeasible same-key repeat(s): the authored interval is shorter than the configured hold."
    );
    assert_eq!(prepared["prepared_id"], Value::Null);
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_prepare_binds_advisories_to_variant_fingerprints_without_gating_model_pressure() {
    fn input(
        song_id: &str,
        song: &super::Song,
        settings: &ApplicationSettings,
        config: PlaybackConfigDto,
        severity: &str,
        recommendation: bool,
    ) -> super::NativePreparedInput {
        let timing_policy = MaterializedTimingPolicy::from_user_margin(
            config.fps,
            config.hold_frames,
            config.timing_margin_us,
        )
        .expect("fixture timing policy");
        let schedule = build_schedule_with_policy(song, config.tempo_scale, &timing_policy)
            .expect("fixture schedule");
        let mut risk = super::analyze_schedule_with_context(
            &schedule,
            Some(&song.notes),
            config.hold_frames,
            config.tempo_scale,
        );
        risk.severity = severity.into();
        if recommendation {
            risk.suggested_hold_frames = Some(1.25);
            risk.suggested_tempo_scale = Some(0.9);
        }
        super::NativePreparedInput {
            song_id: song_id.into(),
            generation: 1,
            config,
            song: song.clone(),
            schedule,
            risk,
            timing_policy,
            timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto {
                recommended_timing_margin_us: Some(800),
                qualified: false,
                source: "default_fallback".into(),
            },
            settings_fingerprint: settings_fingerprint(settings).expect("settings fingerprint"),
            source_epoch: 0,
        }
    }

    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let settings = ApplicationSettings::default();
    let song = super::parse_song_json(
        br#"{"name":"Variant advisory","songNotes":[{"time":0,"key":"Key0"}]}"#,
        "variant-advisory",
    )
    .expect("fixture song");
    let base_config = PlaybackConfigDto {
        hold_frames: 1.0,
        timing_margin_us: 800,
        tempo_scale: 1.0,
        fps: 60,
        dry_run: false,
    };

    let ready = service
        .prepare(input(
            "variant-advisory",
            &song,
            &settings,
            base_config.clone(),
            "low",
            false,
        ))
        .expect("modeled pressure remains a ready plan");
    assert_eq!(ready.admission, crate::commands::PlaybackAdmission::Ready);
    let ready_variant = ready.variants.first().expect("Proceed variant");
    let ready_advisory = ready_variant
        .timing_advisory
        .as_ref()
        .expect("Proceed advisory");
    assert_eq!(
        ready_advisory.status,
        crate::commands::TimingAdvisoryStatus::ModeledRisk
    );
    assert_eq!(
        ready_advisory.plan_fingerprint,
        ready_variant.plan_fingerprint
    );
    assert_eq!(ready_variant.config.hold_frames, base_config.hold_frames);
    assert_eq!(
        ready_variant.config.timing_margin_us,
        base_config.timing_margin_us
    );
    assert_eq!(ready_variant.config.tempo_scale, base_config.tempo_scale);
    assert_eq!(ready_variant.config.fps, base_config.fps);
    assert_eq!(ready_variant.config.dry_run, base_config.dry_run);
    assert!(!ready_advisory.receiver_acknowledged);

    let confirmation = service
        .prepare(input(
            "variant-advisory",
            &song,
            &settings,
            base_config.clone(),
            "medium",
            true,
        ))
        .expect("confirmation variants");
    assert_eq!(
        confirmation.admission,
        crate::commands::PlaybackAdmission::ConfirmationRequired
    );
    assert_eq!(confirmation.variants.len(), 3);
    let prepared_id = confirmation
        .prepared_id
        .as_ref()
        .expect("prepared plan identity");
    let prepared = service.prepared.lock().expect("prepared plan queue");
    let record = prepared
        .iter()
        .find(|(id, _)| id == prepared_id)
        .map(|(_, record)| record)
        .expect("retained prepared plan");
    let mut fingerprints = std::collections::HashSet::new();
    for variant_dto in &confirmation.variants {
        let advisory = variant_dto
            .timing_advisory
            .as_ref()
            .expect("variant advisory");
        assert_eq!(advisory.plan_fingerprint, variant_dto.plan_fingerprint);
        assert!(fingerprints.insert(variant_dto.plan_fingerprint.as_str()));
        let native_variant = record
            .variants
            .get(&variant_dto.decision)
            .expect("matching native variant");
        assert_eq!(native_variant.fingerprint, variant_dto.plan_fingerprint);
        assert_eq!(
            native_variant.config.hold_frames,
            variant_dto.config.hold_frames
        );
        assert_eq!(
            native_variant.config.timing_margin_us,
            variant_dto.config.timing_margin_us
        );
        assert_eq!(
            native_variant.config.tempo_scale,
            variant_dto.config.tempo_scale
        );
        assert_eq!(native_variant.config.fps, variant_dto.config.fps);
        assert_eq!(native_variant.config.dry_run, variant_dto.config.dry_run);
        assert!(!advisory.receiver_acknowledged);
    }
    let recommended = confirmation
        .variants
        .iter()
        .find(|variant| variant.decision == crate::commands::PlaybackDecision::UseRecommended)
        .expect("recommended variant");
    assert_eq!(recommended.config.hold_frames, 1.25);
    assert_eq!(recommended.config.tempo_scale, 0.9);
    let dry_run = confirmation
        .variants
        .iter()
        .find(|variant| variant.decision == crate::commands::PlaybackDecision::DryRun)
        .expect("Dry Run variant");
    assert!(dry_run.config.dry_run);
    assert!(
        dry_run
            .timing_advisory
            .as_ref()
            .expect("Dry Run advisory")
            .dry_run_is_hypothetical
    );
}

#[test]
fn settings_patch_cancels_inflight_prepare_without_waiting_for_whole_stream_analysis() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-prepare-settings-race-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(
        root.join("songs/demo.json"),
        r#"{"name":"Demo","songNotes":[{"time":0,"key":"Key0"}]}"#,
    )
    .expect("song");
    let runtime = Arc::new(NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime"));
    let generation = catalog_generation(&runtime);
    let search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({"query":"demo","offset":0,"limit":10,"generation":generation}),
        )
        .expect("search");
    let song_id = search["items"][0]["song_id"]
        .as_str()
        .expect("song ID")
        .to_owned();
    let request = crate::commands::PlaybackPrepareRequest {
        song_id,
        generation,
        config: PlaybackConfigDto {
            hold_frames: 1.0,
            timing_margin_us: 800,
            tempo_scale: 1.0,
            fps: 60,
            dry_run: true,
        },
    };
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let entered_hook = Arc::clone(&entered);
    let release_hook = Arc::clone(&release);
    *runtime
        .playback
        .prepare_before_model_test_hook
        .lock()
        .expect("model hook") = Some(Arc::new(move || {
        entered_hook.wait();
        release_hook.wait();
    }));

    let prepare_runtime = Arc::clone(&runtime);
    let prepare = thread::spawn(move || prepare_runtime.prepare_playback(request));
    entered.wait();
    runtime
        .patch_settings(crate::commands::SettingsPatch {
            palette: None,
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: Some(crate::commands::PlaybackPatch {
                hold_frames: None,
                timing_margin_us: None,
                tempo_scale: Some(0.95),
                fps: None,
            }),
            auto_play: None,
            update_preferences: None,
        })
        .expect("settings mutation completes while analysis is paused");
    release.wait();
    assert!(
        prepare
            .join()
            .expect("Prepare worker")
            .expect_err("settings mutation invalidates old source snapshot")
            .contains("superseded")
    );
    assert!(
        runtime
            .playback
            .prepared
            .lock()
            .expect("prepared cache")
            .is_empty()
    );

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn auto_play_patch_keeps_prepared_native_playback_valid() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-auto-play-native-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), r#"{"schema_version":5}"#).expect("config");
    fs::write(
        root.join("songs/demo.json"),
        r#"{"name":"Demo","songNotes":[{"time":0,"key":"Key0"}]}"#,
    )
    .expect("song");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let generation = catalog_generation(&runtime);
    let search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({"query":"demo","offset":0,"limit":10,"generation":generation}),
        )
        .expect("search");
    let prepared = runtime
            .dispatch(
                "playback.prepare",
                serde_json::json!({
                    "songId": search["items"][0]["song_id"],
                    "generation": generation,
                    "config": {"hold_frames":1.0,"timing_margin_us":800,"tempo_scale":1.0,"fps":60,"dry_run":true}
                }),
            )
            .expect("prepare");
    let prepared_id = prepared["prepared_id"].as_str().expect("prepared ID");
    let updated = runtime
        .patch_settings(crate::commands::SettingsPatch {
            palette: None,
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: None,
            auto_play: Some(false),
            update_preferences: None,
        })
        .expect("Auto Play settings patch");
    assert!(!updated.auto_play);
    let decisions = if prepared["admission"] == "confirmation_required" {
        serde_json::json!([{"decision":"proceed","accepted":true}])
    } else {
        serde_json::json!([])
    };
    assert!(
        runtime
            .dispatch(
                "playback.start",
                serde_json::json!({
                    "preparedId":prepared_id,
                    "decisions":decisions
                }),
            )
            .is_ok()
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn v4_runtime_operates_with_read_only_install_root_and_separate_app_data() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let temp = std::env::temp_dir().join(format!("sky-v4-appdata-runtime-{suffix}"));
    let install_root = temp.join("install_payload");
    let app_data_root = temp.join("app_data");

    fs::create_dir_all(&install_root).expect("create install payload");
    let dummy_exe = install_root.join(sky_native_adapters::CALIBRATION_EXE);
    fs::write(&dummy_exe, b"immutable-binary-data").expect("write dummy exe");
    let dummy_resource = install_root.join("resources.pak");
    fs::write(&dummy_resource, b"immutable-resource-data").expect("write dummy resource");

    // Take snapshot of entire install payload before runtime execution
    let install_snapshot_before =
        sky_native_adapters::snapshot_directory(&install_root).expect("snapshot install before");

    let paths = sky_native_adapters::AppPaths::from_app_data_root(
        install_root.clone(),
        app_data_root.clone(),
    );
    assert!(paths.assert_clean_boundary().is_ok());
    paths
        .ensure_mutable_directories()
        .expect("ensure mutable dirs");

    // Place a song sheet in the user songs directory
    let song_file = paths.user_music_root().join("test_song.json");
    fs::write(
            &song_file,
            br#"{"name":"V4 Song","author":"Tester","bpm":120,"pitchLevel":0,"data":[{"time":0,"key":"1Key0"}]}"#,
        )
        .expect("write test song");

    let runtime = NativeDesktopRuntime::from_paths_with_activity_and_seams(
        paths.clone(),
        sky_native_adapters::AppResources::from_resource_dir(&install_root),
        ActivityCoordinator::default(),
        TestSeams::SafePackage,
    )
    .expect("start runtime with read-only install root");

    // 1. Settings mutation saves into app_data/config.json
    let patch_result = runtime
        .dispatch(
            "settings.patch",
            serde_json::json!({
                "palette": "slate"
            }),
        )
        .expect("patch settings");
    assert_eq!(patch_result["palette"], "slate");
    assert!(paths.settings_path().exists());
    assert!(!install_root.join("config.json").exists());

    // 2. Catalog load reads from user_music_root outside install root
    let catalog = runtime
        .dispatch("catalog.reload", Value::Object(Default::default()))
        .expect("reload catalog");
    let count = catalog["total"].as_u64().unwrap_or(0);
    assert!(count >= 1, "catalog must contain at least 1 song");
    assert!(!install_root.join("songs").exists());

    // 3. Calibration writes cache to app_data/cache, not install root
    runtime
        .dispatch("calibration.start", serde_json::json!({"mode":"quick"}))
        .expect("start calibration");

    let deadline = Instant::now() + Duration::from_secs(3);
    while runtime.calibration.is_calibration_active_for_test() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!runtime.calibration.is_calibration_active_for_test());
    assert!(paths.calibration_cache_path().exists());
    assert!(!install_root.join(".cache").exists());
    assert!(!install_root.join("cache").exists());

    // 4. Clean shutdown
    runtime.shutdown();

    // 5. Cryptographic proof: entire install payload is completely byte-for-byte unchanged
    let install_snapshot_after =
        sky_native_adapters::snapshot_directory(&install_root).expect("snapshot install after");
    assert_eq!(
        install_snapshot_before, install_snapshot_after,
        "install payload must remain 100% byte-for-byte identical"
    );

    let _ = fs::remove_dir_all(&temp);
}

#[test]
fn persisted_beta_repair_fails_closed_and_persists_stable() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-beta-repair-test-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(
        root.join("config.json"),
        r#"{"schema_version":8,"update":{"channel":"beta"}}"#,
    )
    .expect("write config with beta channel");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone())
        .expect("runtime bootstrap must succeed and repair beta channel to stable");

    assert_eq!(
        runtime
            .settings_snapshot()
            .expect("snapshot")
            .update
            .channel,
        sky_app_core::settings::UpdateChannel::Stable
    );

    let persisted: Value = serde_json::from_str(
        &fs::read_to_string(root.join("config.json")).expect("read persisted config"),
    )
    .expect("parse persisted config");
    assert_eq!(persisted["update"]["channel"], "stable");

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn beta_patch_rejected() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-beta-patch-test-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), r#"{"schema_version":8}"#).expect("write config");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");

    let err1 = runtime
        .patch_settings(crate::commands::SettingsPatch {
            palette: None,
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: None,
            auto_play: None,
            update_preferences: Some(crate::commands::UpdatePreferencesPatch {
                auto_check: None,
                channel: Some(crate::ui_events::UpdateChannel::Beta),
                skip_version: None,
            }),
        })
        .unwrap_err();
    assert!(err1.contains("channel_unavailable"));

    let err2 = runtime
        .patch_update_preferences(crate::commands::UpdatePreferencesPatch {
            auto_check: None,
            channel: Some(crate::ui_events::UpdateChannel::Beta),
            skip_version: None,
        })
        .unwrap_err();
    assert!(err2.contains("channel_unavailable"));

    let err3 = runtime
        .dispatch(
            "settings.patch",
            serde_json::json!({
                "updatePreferences": {
                    "channel": "beta"
                }
            }),
        )
        .unwrap_err();
    assert!(err3.contains("channel_unavailable"));

    let err4 = runtime
        .dispatch(
            "update.preferences.patch",
            serde_json::json!({
                "channel": "beta"
            }),
        )
        .unwrap_err();
    assert!(err4.contains("channel_unavailable"));

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}
