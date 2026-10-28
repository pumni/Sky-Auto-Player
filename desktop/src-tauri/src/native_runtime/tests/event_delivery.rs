use super::*;

#[cfg(feature = "tauri-test")]
#[test]
fn blocked_ui_publication_past_lease_keeps_supervision_live_and_shutdown_releases() {
    use sky_player::engine::DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US;

    let activity = ActivityCoordinator::default();
    let player = terminal_projection_session(DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US, 30_100_000, None);
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    let active = attach_test_presentation(
        tracked_test_active(Arc::clone(&player), activity.clone()),
        &events,
        Arc::clone(&diagnostics_gate),
        None,
    );
    let mut service = NativePlaybackService::new(activity.clone());
    service.diagnostics_gate = diagnostics_gate;
    *service.active.lock().expect("active playback slot") = Some(Arc::clone(&active));
    let accepted_progress = Arc::new(AtomicU64::new(0));
    let hook_progress = Arc::clone(&accepted_progress);
    service.monitor_progress_test_hook = Some(Arc::new(move |accepted| {
        if accepted {
            hook_progress.fetch_add(1, Ordering::Relaxed);
        }
    }));
    let service = Arc::new(service);

    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let callback_entered = Arc::new(AtomicBool::new(false));
    let (entered_sender, entered_receiver) = mpsc::channel();
    let delivered_events = Arc::new(Mutex::new(Vec::<Value>::new()));
    let callback_gate = Arc::clone(&gate);
    let callback_entered_flag = Arc::clone(&callback_entered);
    let callback_events = Arc::clone(&delivered_events);
    let channel: Channel<UiEvent> = Channel::new(move |body| {
        if !callback_entered_flag.swap(true, Ordering::AcqRel) {
            entered_sender.send(()).expect("callback entry receiver");
            let (released, changed) = &*callback_gate;
            let mut released = released.lock().expect("blocked channel gate");
            while !*released {
                released = changed.wait(released).expect("blocked channel gate wait");
            }
        }
        if let InvokeResponseBody::Json(json) = body {
            callback_events
                .lock()
                .expect("delivered event trace")
                .push(serde_json::from_str(&json).expect("serialize UI event"));
        }
        Ok(())
    });

    publish_playback_state(&events, &active, PlaybackEventState::Starting, None, None)
        .expect("queue starting state");
    {
        let mut hub = events.lock().expect("event hub");
        hub.subscribe(channel).expect("subscribe blocked channel");
    }
    entered_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("publisher entered the blocked channel send");

    let monitor_active = Arc::clone(&active);
    let monitor_events = Arc::clone(&events);
    let monitor_service = Arc::clone(&service);
    let (activation_sender, activation_receiver) = mpsc::channel();
    let monitor = thread::spawn(move || {
        monitor_service.monitor(monitor_active, monitor_events, activation_receiver)
    });
    player.arm(0).expect("arm blocked-UI test worker");
    activation_sender
        .send(PlaybackActivation::Started)
        .expect("monitor activation receiver");

    let blocked_for =
        Duration::from_millis(DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US.saturating_add(300_000) / 1_000);
    thread::sleep(blocked_for);
    let progress_count = accepted_progress.load(Ordering::Acquire);
    assert!(
        progress_count >= 100,
        "the owning monitor must continue authority-checked progress during blocked UI publication (accepted={progress_count})"
    );
    assert_ne!(
        player.snapshot().terminal_trigger_reason.as_deref(),
        Some("supervisor_lease_expired"),
        "presentation backpressure must not expire the live supervisor lease"
    );

    let shutdown_started = Instant::now();
    assert!(service.shutdown(Arc::clone(&events)));
    assert!(
        shutdown_started.elapsed() < Duration::from_secs(2),
        "physical shutdown must not wait for the blocked UI consumer"
    );
    monitor.join().expect("monitor thread exits after shutdown");
    assert!(active.done.load(Ordering::Acquire));
    assert!(active.worker_quiescent.load(Ordering::Acquire));
    assert!(active.release_verified.load(Ordering::Acquire));
    assert!(service.status().expect("retired status").active.is_none());
    drop(
        activity
            .reserve_playback("successor-after-blocked-ui")
            .expect("session lease released after tracked cleanup"),
    );

    let close_started = Instant::now();
    events.lock().expect("event hub").close();
    assert!(
        close_started.elapsed() < Duration::from_millis(500),
        "event hub shutdown must be bounded while Channel::send is blocked"
    );
    {
        let (released, changed) = &*gate;
        *released.lock().expect("blocked channel gate") = true;
        changed.notify_all();
    }
    let publisher_deadline = Instant::now() + Duration::from_secs(2);
    while !events
        .lock()
        .expect("event hub")
        .publisher_finished_for_test()
        && Instant::now() < publisher_deadline
    {
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        events
            .lock()
            .expect("event hub")
            .publisher_finished_for_test(),
        "the app-scoped publisher must exit after its blocked send returns"
    );
    let delivered_events = delivered_events.lock().expect("delivered event trace");
    let states = delivered_events
        .iter()
        .filter(|event| event["name"] == "playback.state_changed")
        .filter_map(|event| event["payload"]["state"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(states, vec!["starting"]);
    assert!(delivered_events.iter().all(|event| {
        event["name"] != "playback.finished" && event["name"] != "playback.failed"
    }));
}

#[cfg(feature = "tauri-test")]
#[test]
fn older_critical_events_preserve_the_freshest_pending_snapshot_in_order() {
    let activity = ActivityCoordinator::default();
    let player = presentation_projection_session(0, 30_100_000, 30_200_000);
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    let (test_hook, selection_entered, gate) =
        blocked_presentation_hook(PlaybackPresentationStage::BeforeWorkSelection);
    let active = attach_test_presentation(
        tracked_test_active(Arc::clone(&player), activity.clone()),
        &events,
        diagnostics_gate,
        Some(test_hook),
    );
    selection_entered
        .recv_timeout(Duration::from_secs(2))
        .expect("consumer is held before ordered work selection");

    let presentation = active.presentation.get().expect("presentation consumer");
    presentation
        .publish_state(&active, PlaybackEventState::Starting, None, None)
        .expect("first critical lifecycle event");
    presentation
        .publish_state(&active, PlaybackEventState::Paused, None, None)
        .expect("second critical lifecycle event");
    assert!(presentation.publish_snapshot(&active, 1_000, 0, EnginePollStatus::Playing,));
    assert!(presentation.publish_snapshot(&active, 2_000, 0, EnginePollStatus::Paused,));
    assert_eq!(
        presentation.queue.metrics(),
        PlaybackPresentationMetrics {
            snapshot_coalesced: 1,
            snapshot_dropped: 1,
        },
        "replacing a best-effort frame is counted before consumer selection"
    );

    release_test_gate(&gate);
    let deadline = Instant::now() + Duration::from_secs(2);
    while events.lock().expect("event hub").buffered_for_test().len() < 3
        && Instant::now() < deadline
    {
        thread::yield_now();
    }
    let buffered = events.lock().expect("event hub").buffered_for_test();
    assert_eq!(
        buffered.len(),
        3,
        "both critical events and the latest frame publish"
    );
    assert!(matches!(
        &buffered[0],
        UiEvent::PlaybackStateChanged { payload, .. }
            if payload.state == PlaybackEventState::Starting
    ));
    assert!(matches!(
        &buffered[1],
        UiEvent::PlaybackStateChanged { payload, .. }
            if payload.state == PlaybackEventState::Paused
    ));
    assert!(matches!(
        &buffered[2],
        UiEvent::PlaybackSnapshot { payload, .. }
            if payload.current_us == 2_000 && payload.state == PlaybackEventState::Paused
    ));
    assert_eq!(
        presentation.queue.metrics(),
        PlaybackPresentationMetrics {
            snapshot_coalesced: 1,
            snapshot_dropped: 1,
        },
        "selecting the earlier critical events must not silently discard the taken frame"
    );

    presentation.queue.closed.store(true, Ordering::Release);
    if let Some(worker) = presentation.queue.worker_thread.get() {
        worker.unpark();
    }
    if let Some(worker) = presentation
        .worker
        .lock()
        .expect("presentation worker handle")
        .take()
    {
        worker
            .join()
            .expect("presentation consumer exits after close");
    }
    events.lock().expect("event hub").close();
}

#[cfg(feature = "tauri-test")]
#[test]
fn taken_snapshot_superseded_by_terminal_close_is_counted() {
    let activity = ActivityCoordinator::default();
    let player = presentation_projection_session(0, 30_100_000, 30_200_000);
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    let (test_hook, projection_entered, gate) =
        blocked_presentation_hook(PlaybackPresentationStage::SnapshotProjection);
    let active = attach_test_presentation(
        tracked_test_active(Arc::clone(&player), activity),
        &events,
        diagnostics_gate,
        Some(test_hook),
    );
    let presentation = active.presentation.get().expect("presentation consumer");
    assert!(presentation.publish_snapshot(&active, 3_000, 0, EnginePollStatus::Playing,));
    projection_entered
        .recv_timeout(Duration::from_secs(2))
        .expect("consumer has taken the frame and entered projection");

    active.done.store(true, Ordering::Release);
    presentation.queue.closed.store(true, Ordering::Release);
    release_test_gate(&gate);
    if let Some(worker) = presentation
        .worker
        .lock()
        .expect("presentation worker handle")
        .take()
    {
        worker
            .join()
            .expect("presentation consumer exits after terminal close");
    }

    assert_eq!(
        presentation.queue.metrics(),
        PlaybackPresentationMetrics {
            snapshot_coalesced: 0,
            snapshot_dropped: 1,
        },
        "a frame already taken by the worker must be counted when terminal closure supersedes it"
    );
    assert!(
        events
            .lock()
            .expect("event hub")
            .buffered_for_test()
            .is_empty()
    );
    events.lock().expect("event hub").close();
}

#[cfg(feature = "tauri-test")]
#[test]
fn newer_snapshot_coalesces_a_frame_held_behind_an_older_critical_event() {
    let activity = ActivityCoordinator::default();
    let player = presentation_projection_session(0, 30_100_000, 30_200_000);
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    let (test_hook, selection_stages, before_selection_gate, after_selection_gate) =
        blocked_presentation_selection_hook();
    let active = attach_test_presentation(
        tracked_test_active(Arc::clone(&player), activity),
        &events,
        diagnostics_gate,
        Some(test_hook),
    );

    let presentation = active.presentation.get().expect("presentation consumer");
    // Keep the worker from selecting the critical event before its older snapshot is queued.
    assert_eq!(
        selection_stages.recv_timeout(Duration::from_secs(2)),
        Ok(PlaybackPresentationStage::BeforeWorkSelection),
        "pause the consumer before publishing the ordered pair"
    );
    presentation
        .publish_state(&active, PlaybackEventState::Starting, None, None)
        .expect("critical event before the snapshot");
    assert!(presentation.publish_snapshot(&active, 1_000, 0, EnginePollStatus::Playing,));
    release_test_gate(&before_selection_gate);
    assert_eq!(
        selection_stages
            .recv_timeout(Duration::from_secs(2))
            .expect("consumer has selected the older critical event and taken its snapshot"),
        PlaybackPresentationStage::AfterWorkSelection,
    );
    assert!(
        presentation
            .queue
            .latest_snapshot
            .lock()
            .expect("snapshot slot")
            .is_none()
    );
    assert!(presentation.publish_snapshot(&active, 2_000, 0, EnginePollStatus::Playing,));
    release_test_gate(&after_selection_gate);

    let deadline = Instant::now() + Duration::from_secs(2);
    while events.lock().expect("event hub").buffered_for_test().len() < 2
        && Instant::now() < deadline
    {
        thread::yield_now();
    }
    let buffered = events.lock().expect("event hub").buffered_for_test();
    assert_eq!(buffered.len(), 2);
    assert!(matches!(
        &buffered[0],
        UiEvent::PlaybackStateChanged { payload, .. }
            if payload.state == PlaybackEventState::Starting
    ));
    assert!(matches!(
        &buffered[1],
        UiEvent::PlaybackSnapshot { payload, .. }
            if payload.current_us == 2_000
    ));
    assert_eq!(
        presentation.queue.metrics(),
        PlaybackPresentationMetrics {
            snapshot_coalesced: 1,
            snapshot_dropped: 1,
        },
        "a fresher frame supersedes the held frame with explicit accounting"
    );

    presentation.queue.closed.store(true, Ordering::Release);
    if let Some(worker) = presentation.queue.worker_thread.get() {
        worker.unpark();
    }
    if let Some(worker) = presentation
        .worker
        .lock()
        .expect("presentation worker handle")
        .take()
    {
        worker
            .join()
            .expect("presentation consumer exits after close");
    }
    events.lock().expect("event hub").close();
}

#[cfg(feature = "tauri-test")]
#[test]
fn failed_critical_channel_delivery_requests_tracked_terminal_cleanup() {
    use sky_player::engine::DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US;

    let activity = ActivityCoordinator::default();
    let player = terminal_projection_session(DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US, 30_100_000, None);
    player.arm(0).expect("arm channel-failure test worker");
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let diagnostics_gate = Arc::new(DiagnosticsPublicationGate::default());
    let active = attach_test_presentation(
        tracked_test_active(Arc::clone(&player), activity.clone()),
        &events,
        Arc::clone(&diagnostics_gate),
        None,
    );
    let mut service = NativePlaybackService::new(activity.clone());
    service.diagnostics_gate = diagnostics_gate;
    *service.active.lock().expect("active playback slot") = Some(Arc::clone(&active));
    let service = Arc::new(service);
    publish_playback_state(&events, &active, PlaybackEventState::Starting, None, None)
        .expect("queue starting state");
    {
        let mut hub = events.lock().expect("event hub");
        let failing_channel: Channel<UiEvent> = Channel::new(|_| {
            Err(std::io::Error::other("injected critical delivery failure").into())
        });
        hub.subscribe(failing_channel)
            .expect("subscribe failing channel");
    }

    let monitor_active = Arc::clone(&active);
    let monitor_events = Arc::clone(&events);
    let monitor_service = Arc::clone(&service);
    let (activation_sender, activation_receiver) = mpsc::channel();
    activation_sender
        .send(PlaybackActivation::Started)
        .expect("monitor activation receiver");
    let monitor = thread::spawn(move || {
        monitor_service.monitor(monitor_active, monitor_events, activation_receiver)
    });

    let deadline = Instant::now() + Duration::from_secs(3);
    while native_event_delivery_failure(&events).is_none() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(1));
    }
    assert!(native_event_delivery_failure(&events).is_some());
    let monitor_deadline = Instant::now() + Duration::from_secs(3);
    while !active.done.load(Ordering::Acquire) && Instant::now() < monitor_deadline {
        thread::sleep(Duration::from_millis(1));
    }
    monitor.join().expect("monitor exits after channel failure");
    assert!(active.done.load(Ordering::Acquire));
    assert!(active.worker_quiescent.load(Ordering::Acquire));
    let terminal = service
        .status()
        .expect("retired terminal status")
        .last_terminal;
    assert!(
        active.release_verified.load(Ordering::Acquire),
        "critical delivery cleanup did not verify release: {terminal:?}"
    );
    assert!(service.status().expect("retired status").active.is_none());
    drop(
        activity
            .reserve_playback("successor-after-channel-failure")
            .expect("failed delivery still releases tracked session ownership"),
    );
    events.lock().expect("event hub").close();
    assert!(
        events
            .lock()
            .expect("event hub")
            .publisher_finished_for_test()
    );
}

#[test]
fn startup_state_event_failure_does_not_start_a_worker_or_drop_terminal_evidence() {
    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let settings = ApplicationSettings::default();
    let prepared_id = prepare_dry_run_plan(&service, &settings, &"c".repeat(32));
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    events.lock().expect("event hub").close();

    let error = service
        .start(
            PlaybackStartRequest {
                prepared_id,
                decisions: Vec::new(),
            },
            &settings,
            events,
        )
        .expect_err("closed startup event delivery should fail the start");

    assert!(error.contains("event hub is closed"), "{error}");
    let status = service.status().expect("status after publication failure");
    assert!(status.active.is_none());
    let terminal = status.last_terminal.expect("stored start failure");
    assert_eq!(terminal.state, PlaybackSessionState::Failed);
    assert_eq!(
        terminal.failure_code.as_deref(),
        Some("playback_start_publication_failed")
    );
}

#[test]
fn event_delivery_failure_preserves_the_terminal_worker_outcome() {
    let activity = ActivityCoordinator::default();
    let mut service = NativePlaybackService::new(activity);
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
    active.stop_requested.store(true, Ordering::Release);
    *service.active.lock().expect("active session slot") = Some(active.clone());
    events.lock().expect("event hub").close();
    let (activation_sender, activation_receiver) = mpsc::channel();
    activation_sender
        .send(PlaybackActivation::Started)
        .expect("monitor activation receiver");

    service.monitor(Arc::clone(&active), events, activation_receiver);

    assert!(wait_for_playback_done(&active, Duration::from_secs(2)));
    let status = service
        .status()
        .expect("terminal status after event failure");
    assert!(status.active.is_none());
    let terminal = status
        .last_terminal
        .expect("authoritative terminal outcome");
    assert_eq!(terminal.state, PlaybackSessionState::Finished);
    assert_eq!(terminal.outcome.as_deref(), Some("quit"));
}

#[test]
fn event_hub_coalesces_snapshots_and_fails_closed_for_lifecycle_overflow() {
    let mut hub = NativeEventHub::default();
    let session_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    hub.publish(snapshot(session_id, 1)).expect("snapshot");
    hub.publish(snapshot(session_id, 2)).expect("coalesce");
    let buffered = hub.buffered_for_test();
    assert_eq!(buffered.len(), 1);
    assert!(
        matches!(buffered.front(), Some(UiEvent::PlaybackSnapshot { payload, .. }) if payload.seq == 2)
    );
    assert_eq!(
        hub.snapshot_metrics(),
        NativeEventSnapshotMetrics {
            snapshot_coalesced: 1,
            snapshot_dropped: 1,
        }
    );

    for index in 0..MAX_NATIVE_EVENTS {
        hub.publish(UiEvent::CatalogChanged {
            v: 1,
            payload: crate::ui_events::CatalogChangedPayload {
                generation: index as u64 + 1,
                total: 0,
            },
        })
        .expect("snapshot slot can be evicted before lifecycle fill");
    }
    hub.publish(snapshot(session_id, 3))
        .expect("best-effort snapshot is dropped when the bounded queue has only lifecycle events");
    assert_eq!(hub.buffered_for_test().len(), MAX_NATIVE_EVENTS);
    assert!(hub.delivery_failure().is_none());
    assert_eq!(
        hub.snapshot_metrics(),
        NativeEventSnapshotMetrics {
            snapshot_coalesced: 1,
            snapshot_dropped: 3,
        }
    );
    assert!(
        hub.publish(UiEvent::CatalogChanged {
            v: 1,
            payload: crate::ui_events::CatalogChangedPayload {
                generation: 999,
                total: 0,
            },
        })
        .is_err()
    );
    assert!(hub.delivery_failure().is_some());
}

#[test]
fn event_hub_counts_snapshot_eviction_on_close_and_failed_channel_send() {
    let session_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let mut closed_hub = NativeEventHub::default();
    closed_hub
        .publish(snapshot(session_id, 1))
        .expect("queue snapshot before close");
    closed_hub.close();
    assert_eq!(
        closed_hub.snapshot_metrics(),
        NativeEventSnapshotMetrics {
            snapshot_coalesced: 0,
            snapshot_dropped: 1,
        },
        "closing the hub must account for snapshots it removes from the queue"
    );

    let mut failed_hub = NativeEventHub::default();
    failed_hub
        .publish(snapshot(session_id, 2))
        .expect("queue snapshot before channel failure");
    let failing_channel: Channel<UiEvent> =
        Channel::new(|_| Err(std::io::Error::other("injected snapshot send failure").into()));
    failed_hub
        .subscribe(failing_channel)
        .expect("subscribe failing channel");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !failed_hub.publisher_finished_for_test() && Instant::now() < deadline {
        thread::yield_now();
    }
    assert!(failed_hub.publisher_finished_for_test());
    assert_eq!(
        failed_hub.snapshot_metrics(),
        NativeEventSnapshotMetrics {
            snapshot_coalesced: 0,
            snapshot_dropped: 1,
        },
        "a snapshot whose channel send fails must be counted as dropped"
    );
    failed_hub.close();
}

#[test]
fn finished_and_failed_events_are_retirement_barriers_for_the_next_reservation() {
    for (state, publication, expected_terminal) in [
        (
            PlaybackSessionState::Finished,
            PlaybackTerminalPublication::Finished {
                outcome: "finished".into(),
                message: "Playback finished".into(),
            },
            "finished:finished",
        ),
        (
            PlaybackSessionState::Failed,
            PlaybackTerminalPublication::Failed {
                code: "native_player_failed".into(),
                message: "Native playback worker failed".into(),
            },
            "failed:native_player_failed",
        ),
    ] {
        let activity = ActivityCoordinator::default();
        let service = NativePlaybackService::new(activity.clone());
        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let active = active_for_control_with_physical(state, None, true);
        *active.activity_lease.lock().expect("activity lease slot") = Some(
            activity
                .reserve_playback(active.session_id.clone())
                .expect("first session reservation"),
        );
        *service.active.lock().expect("active session slot") = Some(active.clone());
        assert!(activity.reserve_playback("next-before-barrier").is_err());

        publish_retirement_barrier(
            &events,
            &service.active,
            &service.last_terminal,
            &active,
            TerminalRetirement {
                quiescence: WorkerQuiescenceProof::NoWorkerStarted,
                release: TerminalReleaseEvidence::Verified { obligation_mask: 0 },
                publication,
            },
        )
        .expect("publish terminal retirement barrier");

        assert_eq!(
            playback_trace(&events).last().map(String::as_str),
            Some(expected_terminal),
        );
        assert!(
            service
                .active
                .lock()
                .expect("active session slot")
                .is_none()
        );
        assert!(active.done.load(Ordering::Acquire));
        assert!(active.pending.lock().expect("pending control").is_none());
        let next = activity
            .reserve_playback("next-after-barrier")
            .expect("next reservation succeeds immediately after terminal event");
        drop(next);
    }
}

#[test]
fn shutdown_completion_uses_the_same_terminal_event_sequence() {
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let active = active_for_control(PlaybackSessionState::Starting, None);
    let mut last_event_state = PlaybackEventState::Starting;
    publish_playback_state(&events, &active, PlaybackEventState::Stopping, None, None)
        .expect("stopping event");
    publish_terminal_poll_result(
        &events,
        &active,
        &mut last_event_state,
        sky_player::engine::EnginePollStatus::Quit,
    )
    .expect("shutdown terminal event trace");
    assert_eq!(
        playback_trace(&events),
        ["state:stopping", "state:finished", "finished:quit"]
    );
}

#[test]
fn event_hub_evicts_snapshots_before_lifecycle_events() {
    let mut hub = NativeEventHub::default();
    hub.publish(snapshot("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 1))
        .expect("snapshot");
    for index in 0..MAX_NATIVE_EVENTS {
        hub.publish(crate::ui_events::UiEvent::CatalogChanged {
            v: 1,
            payload: crate::ui_events::CatalogChangedPayload {
                generation: index as u64 + 1,
                total: 0,
            },
        })
        .expect("snapshot is evictable");
    }
    let buffered = hub.buffered_for_test();
    assert_eq!(buffered.len(), MAX_NATIVE_EVENTS);
    assert!(
        !buffered
            .iter()
            .any(|event| matches!(event, UiEvent::PlaybackSnapshot { .. }))
    );
    let mut buffered = buffered;
    assert!(!remove_oldest_snapshot(&mut buffered));
}
