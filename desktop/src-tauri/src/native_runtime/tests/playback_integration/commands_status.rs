use super::*;

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
