use super::*;

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
