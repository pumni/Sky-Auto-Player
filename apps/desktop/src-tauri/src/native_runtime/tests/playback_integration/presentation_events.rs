use super::*;

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
