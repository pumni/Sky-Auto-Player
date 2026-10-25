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

#[test]
fn direct_native_player_keeps_the_qualified_supervisor_lease() {
    assert_eq!(
        sky_player::engine::DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US,
        3_000_000
    );
}
