use super::support::*;
use super::*;

#[test]
fn native_prepared_normal_resume_sends_frozen_up_and_following_sentinel() {
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "native-prepared-suspension-down-k".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "native-prepared-suspension-up-k".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 2_000_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "native-prepared-suspension-sentinel-j".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 3_000_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "native-prepared-suspension-sentinel-up".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("prepared suspension schedule");
    let send_calls = Arc::new(AtomicU64::new(0));
    let full_release_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_calls));
    let mut options = test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.profile = DispatchProfile::MockTest;
    let session = NativeDispatchSession::new(options).expect("native session admission");
    start_with_test_wall_clock_slack(&session);

    let first_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot();
        if send_calls.load(Ordering::Acquire) >= 1 {
            break;
        }
        assert!(
            !snapshot.is_finished,
            "prepared Down terminated before pause: {snapshot:?}"
        );
        assert!(
            Instant::now() < first_deadline,
            "prepared Down did not commit: {snapshot:?}"
        );
        session
            .publish_supervisor_progress()
            .expect("heartbeat before pause");
        std::thread::yield_now();
    }

    let pause_generation = session.pause_with_timing_token().expect("pause request");
    let _pause_timing = wait_for_pause_ack(&session, pause_generation);
    let paused_snapshot = wait_for_clean_suspension(&session);
    assert!(paused_snapshot.is_paused);
    assert_eq!(paused_snapshot.active_count, 0);
    assert_eq!(paused_snapshot.possibly_active_count, 0);

    session.resume().expect("resume request");
    let resume_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot();
        if !snapshot.is_paused {
            break;
        }
        assert!(
            !snapshot.is_finished,
            "prepared resume terminated before the frozen Up: {snapshot:?}"
        );
        assert!(
            Instant::now() < resume_deadline,
            "prepared resume did not commit"
        );
        session
            .publish_supervisor_progress()
            .expect("heartbeat after resume request");
        std::thread::yield_now();
    }

    let sentinel_deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let snapshot = session.snapshot();
        if send_calls.load(Ordering::Acquire) >= 4 {
            assert!(snapshot.is_running);
            assert_eq!(snapshot.terminal_error, None);
            break;
        }
        assert!(
            !snapshot.is_finished,
            "prepared frozen Up or sentinel Down terminated the worker: {snapshot:?}"
        );
        assert!(
            Instant::now() < sentinel_deadline,
            "sentinel Down did not send"
        );
        session
            .publish_supervisor_progress()
            .expect("heartbeat before sentinel");
        std::thread::yield_now();
    }

    session.quit().expect("explicit quit after sentinel");
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let final_snapshot = session.snapshot();
    assert_eq!(final_snapshot.terminal_error, None);
    assert_eq!(final_snapshot.active_count, 0);
    assert_eq!(final_snapshot.possibly_active_count, 0);
    assert_eq!(final_snapshot.timeline_rebase_count, 0);
    assert!(send_calls.load(Ordering::Acquire) >= 3);
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("valid telemetry JSON");
    let sentinel = telemetry["records"]
        .as_array()
        .expect("telemetry records")
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(2))
        .expect("sentinel Down telemetry");
    assert_eq!(sentinel["send_attempts"].as_u64(), Some(1));
    assert_eq!(sentinel["sent_count"].as_u64(), Some(1));
}

#[test]
fn native_prepared_normal_resume_naturally_finishes_after_reconciled_up() {
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "natural-resume-down-k".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 800_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "natural-resume-up-k".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 1_600_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "natural-resume-down-j".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 2_400_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "natural-resume-up-j".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("natural prepared suspension schedule");
    let send_calls = Arc::new(AtomicU64::new(0));
    let full_release_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
    fault_script.full_instrument_release_calls = Some(Arc::clone(&full_release_calls));
    let mut options = test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.profile = DispatchProfile::MockTest;
    let session = NativeDispatchSession::new(options).expect("native session admission");
    start_with_test_wall_clock_slack(&session);

    let first_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot();
        if send_calls.load(Ordering::Acquire) >= 1 {
            break;
        }
        assert!(
            !snapshot.is_finished,
            "initial Down terminated: {snapshot:?}"
        );
        assert!(Instant::now() < first_deadline, "initial Down did not send");
        session
            .publish_supervisor_progress()
            .expect("heartbeat before natural-resume pause");
        std::thread::yield_now();
    }

    let pause_generation = session.pause_with_timing_token().expect("pause request");
    let _pause_timing = wait_for_pause_ack(&session, pause_generation);
    let paused_snapshot = wait_for_clean_suspension(&session);
    assert!(paused_snapshot.is_paused);
    assert_eq!(paused_snapshot.active_count, 0);
    assert_eq!(paused_snapshot.possibly_active_count, 0);

    session.resume().expect("resume request");
    let resume_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let snapshot = session.snapshot();
        if !snapshot.is_paused {
            break;
        }
        assert!(
            !snapshot.is_finished,
            "resume finished unexpectedly: {snapshot:?}"
        );
        assert!(
            Instant::now() < resume_deadline,
            "resume acknowledgment timed out: {snapshot:?}"
        );
        session
            .publish_supervisor_progress()
            .expect("heartbeat after natural-resume request");
        std::thread::yield_now();
    }

    let finish_deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let snapshot = session.snapshot();
        if snapshot.is_finished {
            break;
        }
        assert!(
            snapshot.terminal_error.is_none(),
            "natural resume terminated with an error: {snapshot:?}"
        );
        assert!(
            Instant::now() < finish_deadline,
            "prepared stream did not naturally finish: {snapshot:?}"
        );
        session
            .publish_supervisor_progress()
            .expect("heartbeat during natural finish");
        std::thread::yield_now();
    }

    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, "finished");
    assert_eq!(snapshot.terminal_error, None);
    assert_eq!(snapshot.generation_status_counts["released"], 2);
    assert_eq!(snapshot.generation_status_counts["cancelled"], 0);
    assert_eq!(snapshot.generation_status_counts["scheduled"], 0);
    assert_eq!(snapshot.generation_status_counts["active"], 0);
    assert_eq!(snapshot.generation_status_counts["dropped_expired"], 0);
    assert_eq!(snapshot.generation_status_counts["dropped_backend"], 0);
    assert_eq!(snapshot.generation_status_counts["dropped_conflict"], 0);
    let accounting = session.generation_accounting_for_test();
    assert_eq!(accounting.total, accounting.activated);
    assert_eq!(accounting.total, accounting.released);
    sky_dispatch_core::testing::assert_clean_generation_completion(accounting);
    assert_eq!(snapshot.active_count, 0);
    assert_eq!(snapshot.possibly_active_count, 0);
    assert_eq!(snapshot.failed_release_count, 0);
    assert_eq!(snapshot.timeline_rebase_count, 0);
    let release = snapshot
        .release_outcome
        .as_ref()
        .expect("natural completion publishes release outcome");
    assert!(release.released_successfully);
    assert_eq!(release.stuck_mask, 0);
    assert!(!release.verification_inconclusive);
    assert_eq!(release.attempted_mask, 0);
    assert_eq!(release.attempts, 0);
    assert_eq!(full_release_calls.load(Ordering::Acquire), 0);

    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("valid telemetry JSON");
    let records = telemetry["records"].as_array().expect("records array");
    for event_index in 0..4 {
        let record = records
            .iter()
            .find(|record| record["event_index"].as_u64() == Some(event_index))
            .unwrap_or_else(|| panic!("missing natural physical event {event_index}"));
        assert_eq!(record["send_attempts"].as_u64(), Some(1));
        assert_eq!(record["sent_count"].as_u64(), Some(1));
    }
}

#[test]
fn normal_prepared_final_control_race_suppresses_send_without_cursor_advance() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let calls = harness.configure_send_counter();
    let mut stream = harness.build_prepared_stream_for_test();
    harness.set_final_gate_race_hook(
        |_focus_active,
         _target,
         quit_requested,
         _skip_requested,
         _panic_requested,
         _desired_pause,
         _system_power| {
            quit_requested.store(true, Ordering::Release);
        },
    );

    let step = harness.dispatch_prepared_current_at_lateness_for_test(&mut stream, 10_000);

    assert!(matches!(step, super::worker::DispatchStep::Continue));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(harness.resources.coordinator.cursor, 0);
}

#[test]
fn prepared_final_admission_rejects_aba_without_send_cursor_or_accounting_advance() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let calls = harness.configure_send_counter();
    let mut stream = harness.build_prepared_stream_for_test();
    let initial_entry = stream.current().expect("prepared physical entry") as *const _;
    let accounting_before = harness.generation_accounting_for_test();
    assert_eq!(harness.target.load_stable(), Some((1, 0)));

    harness.set_post_focus_revalidation_race_hook(|_, target, _, _, _, _, _| {
        target.publish(2);
        target.publish(1);
    });
    let step = harness.dispatch_prepared_current_at_lateness_for_test(&mut stream, 10_000);

    assert!(matches!(step, super::worker::DispatchStep::Continue));
    assert_eq!(harness.target.load_stable(), Some((1, 2)));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(harness.resources.coordinator.cursor, 0);
    assert_eq!(harness.generation_accounting_for_test(), accounting_before);
    assert_eq!(
        stream.current().expect("rejected entry remains current") as *const _,
        initial_entry
    );
}

#[test]
fn normal_prepared_precision_suffix_suppresses_all_final_control_races() {
    for control in 0..8 {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let calls = harness.configure_send_counter();
        match control {
            0 => harness.panic_requested.store(true, Ordering::Release),
            1 => harness.supervisor_expired.latch_expired(),
            2 => harness.quit_requested.store(true, Ordering::Release),
            3 => harness.skip_requested.store(true, Ordering::Release),
            4 => harness.desired_pause.store(true, Ordering::Release),
            5 => {
                harness.config.focus.require_focus = true;
                harness.set_final_gate_race_hook(
                    |focus_active, _target, _quit, _skip, _panic, _pause, _system_power| {
                        focus_active.store(false, Ordering::Release);
                    },
                );
            }
            6 => {
                assert!(harness.notify_system_power_for_test(true));
            }
            7 => harness.set_final_gate_race_hook(
                |_focus_active, target, _quit, _skip, _panic, _pause, _system_power| {
                    target.publish(456);
                },
            ),
            _ => unreachable!(),
        }

        let mut stream = harness.build_prepared_stream_for_test();
        let step = harness.dispatch_prepared_current_at_lateness_for_test(&mut stream, 10_000);

        assert!(!matches!(step, super::worker::DispatchStep::Dispatched));
        assert_eq!(calls.load(Ordering::SeqCst), 0, "control race {control}");
        assert!(
            stream.current().is_some(),
            "control race {control} advanced cursor"
        );
    }
}

#[test]
fn normal_prepared_zero_progress_is_fail_closed_without_cursor_advance() {
    let mut harness =
        ProductionDispatchTestHarness::new_dense_future_boundary_with_gap_for_test(10_000);
    let calls = harness.configure_prepared_zero_progress_sender_for_test();
    let mut stream = harness.build_prepared_stream_for_test();

    let step = harness.dispatch_prepared_current_at_lateness_for_test(&mut stream, 1_000);

    assert!(
        matches!(
            step,
            super::worker::DispatchStep::Terminate(_)
                | super::worker::DispatchStep::TerminateStatic(_)
        ),
        "unexpected zero-progress step: {step:?}, calls: {}",
        calls.load(Ordering::SeqCst)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.resources.coordinator.cursor, 0);
    assert_eq!(harness.backend_active_mask(), 0);
    assert_eq!(harness.backend_possibly_active_mask(), 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(
        harness.resources.coordinator.generation_accounting().active,
        0
    );
}

#[test]
fn midstream_stale_packet_is_metadata_not_physical_work() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "midstream-a-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "midstream-a-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 150_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "midstream-stale".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Down,
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "midstream-b-down".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Up,
                scheduled_us: 300_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "midstream-b-up".into(),
            },
        ],
        &[0x15, 0x16],
    )
    .expect("valid midstream stale schedule");
    let (snapshot, telemetry) = run_mock_schedule(schedule, 2, 16);
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    let records = telemetry["records"].as_array().expect("records");
    let stale = records
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(2))
        .expect("midstream stale record");
    assert_eq!(stale["outcome"].as_u64(), Some(4), "stale record: {stale}");
    assert_eq!(stale["requested_count"].as_u64(), Some(0));
    assert_eq!(stale["sent_count"].as_u64(), Some(0));
    assert_eq!(stale["send_attempts"].as_u64(), Some(0));
    assert_eq!(stale["applied_lead_ticks"].as_u64(), Some(0));
    assert_eq!(stale["wake_offset_available"].as_bool(), Some(false));
    assert_eq!(
        stale["wake_offset_provenance"].as_str(),
        Some("unavailable")
    );
    assert_eq!(
        records
            .iter()
            .filter(|record| record["event_index"].as_u64() == Some(3))
            .count(),
        1
    );
}

#[test]
fn trailing_stale_packet_finishes_without_physical_dispatch() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "trailing-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "trailing-release".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 300_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "trailing-stale".into(),
            },
        ],
        &[0x15],
    )
    .expect("valid trailing stale schedule");
    let (snapshot, telemetry) = run_mock_schedule(schedule, 1, 8);
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    let stale = telemetry["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(2))
        .expect("trailing stale record");
    assert_eq!(stale["outcome"].as_u64(), Some(4));
    assert_eq!(stale["requested_count"].as_u64(), Some(0));
    assert_eq!(stale["send_attempts"].as_u64(), Some(0));
}

#[test]
fn same_timestamp_midstream_stale_packet_is_committed_once() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x17],
                reason: "cohort-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "cohort-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "cohort-stale-a".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "cohort-stale-b".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Down,
                scheduled_us: 300_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "cohort-redown".into(),
            },
            KeyActionInput {
                source_action_index: 5,
                kind: ActionKind::Up,
                scheduled_us: 400_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "cohort-cleanup".into(),
            },
        ],
        &[0x15, 0x16, 0x17],
    )
    .expect("valid same-timestamp stale schedule");
    let (snapshot, telemetry) = run_mock_schedule(schedule, 3, 16);
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    let records = telemetry["records"].as_array().expect("records");
    let stale = records
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(2))
        .expect("same-timestamp stale record");
    assert_eq!(stale["outcome"].as_u64(), Some(4));
    assert_eq!(stale["polyphony"].as_u64(), Some(2));
    assert_eq!(
        records
            .iter()
            .filter(|record| record["outcome"].as_u64() == Some(4))
            .count(),
        1
    );
    assert!(
        records
            .iter()
            .all(|record| record["event_index"].as_u64() != Some(3))
    );
}

#[test]
fn stale_ups_with_down_same_timestamp_use_one_down_packet() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    // Keep worker startup/preemption away from the first authored boundary.
    // This test probes stale-event ordering, not zero-slack startup timing.
    const TEST_AUTHORED_EPOCH_US: u64 = 2_000_000;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Up,
                scheduled_us: TEST_AUTHORED_EPOCH_US,
                scan_codes: smallvec::smallvec![0x15],
                reason: "mixed-stale-a".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: TEST_AUTHORED_EPOCH_US,
                scan_codes: smallvec::smallvec![0x16],
                reason: "mixed-stale-b".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: TEST_AUTHORED_EPOCH_US,
                scan_codes: smallvec::smallvec![0x17],
                reason: "mixed-down".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                // Keep cleanup away from the first packet so parallel test
                // observer work cannot turn this ordering probe into an
                // unrelated up-deadline failure.
                scheduled_us: TEST_AUTHORED_EPOCH_US + 500_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "mixed-cleanup".into(),
            },
        ],
        &[0x15, 0x16, 0x17],
    )
    .expect("valid stale-plus-down schedule");
    let (snapshot, telemetry) = run_seeded_adaptive_startup_schedule_with_capacity(schedule, 3, 8);
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    let records = telemetry["records"].as_array().expect("records");
    assert_eq!(
        records
            .iter()
            .filter(|record| record["outcome"].as_u64() == Some(4))
            .count(),
        0
    );
    let physical = records
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(2))
        .expect("physical down record");
    assert_eq!(physical["kind"].as_u64(), Some(0));
    assert_eq!(physical["requested_count"].as_u64(), Some(1));
}

#[test]
fn owned_up_stale_up_and_down_same_timestamp_count_only_physical_events() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 500_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "owned-up-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 600_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "owned-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 600_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "owned-stale".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Down,
                scheduled_us: 600_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "owned-redown".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Up,
                scheduled_us: 700_000,
                scan_codes: smallvec::smallvec![0x17],
                reason: "owned-cleanup".into(),
            },
        ],
        &[0x15, 0x16, 0x17],
    )
    .expect("valid owned-plus-stale schedule");
    let (snapshot, telemetry) = run_seeded_adaptive_startup_schedule_with_capacity(schedule, 3, 8);
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    let packet = telemetry["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|record| {
            record["kind"].as_u64() == Some(2) && record["requested_count"].as_u64() == Some(2)
        })
        .expect("mixed physical record");
    assert_eq!(packet["kind"].as_u64(), Some(2));
    assert_eq!(packet["requested_count"].as_u64(), Some(2));
    assert_ne!(packet["requested_count"].as_u64(), Some(3));
}

#[test]
fn many_midstream_stale_packets_remain_linear_and_physical_work_continues() {
    use crate::engine::observer_test_hooks::{
        observer_test_hook_guard, set_observer_artificial_cost_us,
    };
    use sky_dispatch_core::compile::compile_runtime_intents;

    let _observer_hooks = observer_test_hook_guard();
    set_observer_artificial_cost_us(20_000);
    let stale_count = 64usize;
    let expected_stale_packets = stale_count + 1;
    let mut actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 100_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "many-midstream-down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 150_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "many-midstream-release".into(),
        },
    ];
    for index in 0..stale_count {
        actions.push(KeyActionInput {
            source_action_index: (index + 2) as u32,
            kind: ActionKind::Up,
            scheduled_us: 200_000 + index as u64,
            scan_codes: smallvec::smallvec![0x16],
            reason: "many-midstream-stale".into(),
        });
    }
    actions.extend([
        KeyActionInput {
            source_action_index: (stale_count + 2) as u32,
            kind: ActionKind::Down,
            scheduled_us: 300_000,
            scan_codes: smallvec::smallvec![0x17],
            reason: "many-midstream-next-down".into(),
        },
        KeyActionInput {
            source_action_index: (stale_count + 3) as u32,
            kind: ActionKind::Up,
            scheduled_us: 350_000,
            scan_codes: smallvec::smallvec![0x17],
            reason: "many-midstream-cleanup".into(),
        },
    ]);
    let schedule = compile_runtime_intents(&actions, &[0x15, 0x16, 0x17])
        .expect("valid many-midstream stale schedule");
    let hook = Arc::new(StartupOrderingHook::default());
    let mut options = test_session_options(
        schedule,
        3,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.telemetry.capacity = 128;
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    options.wait.supervisor_lease_timeout_us = 3_000_000;
    let session = NativeDispatchSession::new(options).expect("session admission");
    start_with_test_wall_clock_slack(&session);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    assert_eq!(
        hook.stale_packet_committed.load(Ordering::SeqCst),
        expected_stale_packets as u64
    );
    assert!(
        hook.first_physical_send_started.load(Ordering::SeqCst) > 0,
        "physical dispatch must begin: stale={}, physical={}",
        hook.stale_packet_committed.load(Ordering::SeqCst),
        hook.first_physical_send_started.load(Ordering::SeqCst),
    );
    assert!(snapshot.observer_dropped_samples > 0);
    assert!(
        snapshot.observer_queue_high_watermark > 0,
        "successful observer enqueue should publish an observed occupancy"
    );
    assert!(
        snapshot.observer_queue_high_watermark
            <= super::worker::dispatch::observation::OBSERVATION_QUEUE_CAPACITY as u64,
        "observer queue high-water mark must not exceed its fixed capacity"
    );
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("valid telemetry JSON");
    let records = telemetry["records"].as_array().expect("records");
    let stale_records = records
        .iter()
        .filter(|record| record["outcome"].as_u64() == Some(4))
        .count();
    assert!(stale_records <= expected_stale_packets);
}

#[test]
fn production_startup_matrix_preserves_first_physical_lead() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    for scheduled_us in [0, 100, 499, 500, 501] {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    // Keep the authored boundary well beyond worker startup and
                    // host timer quantization.  This is test isolation only; the
                    // authored target remains frozen and the worker never moves
                    // it after arm.
                    scheduled_us: scheduled_us + 100_000,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "startup-matrix-down".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: scheduled_us + 101_000,
                    scan_codes: smallvec::smallvec![0x15],
                    reason: "startup-matrix-up".into(),
                },
            ],
            &[0x15],
        )
        .expect("valid startup matrix schedule");
        let telemetry = run_startup_first_physical_lead_probe(schedule, scheduled_us);
        let first = telemetry["records"]
            .as_array()
            .expect("records")
            .iter()
            .find(|record| record["event_index"].as_u64() == Some(0))
            .expect("first physical record");
        assert_eq!(first["applied_lead_ticks"].as_u64(), Some(0));
    }
}

#[test]
fn join_waits_within_the_original_timeout_after_completion_publication() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::WorkerCompletionPublished, 1);
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
    options.wait.supervisor_lease_timeout_us = 3_000_000;
    let session = NativeDispatchSession::new(options).expect("session admission");
    start_with_test_wall_clock_slack(&session);

    let reached = hook.wait_for_startup_checkpoint(Duration::from_secs(5));
    if !reached {
        hook.release_startup_checkpoint();
        let _ = session.join(Duration::from_secs(5));
        panic!(
            "worker did not reach the completion publication barrier: {}",
            startup_probe_diagnostics(
                &session,
                &hook,
                0,
                Duration::ZERO,
                None,
                "not attempted",
                "not requested",
            )
        );
    }

    let terminal_snapshot = session.snapshot();
    let timeout = Duration::from_millis(100);
    let join_started = Instant::now();
    let timed_out_join = session.join(timeout);
    let join_elapsed = join_started.elapsed();
    let join_state_while_held = session.join_state_for_test();

    // Always release the test barrier before asserting, so a failed assertion
    // cannot leave a detached worker behind.
    hook.release_startup_checkpoint();
    let final_join = session.join(Duration::from_secs(5));

    assert_eq!(terminal_snapshot.outcome, Some("finished".to_string()));
    assert!(terminal_snapshot.terminal_error.is_none());
    assert_eq!(terminal_snapshot.active_count, 0);
    assert_eq!(terminal_snapshot.possibly_active_count, 0);
    assert_eq!(terminal_snapshot.failed_release_count, 0);
    assert_eq!(terminal_snapshot.final_release_obligation_mask, 0);
    assert!(
        timed_out_join.as_ref().is_ok_and(|joined| !*joined),
        "join must preserve a timed-out wrapper thread: result={timed_out_join:?}, state={join_state_while_held:?}"
    );
    assert!(
        join_elapsed >= Duration::from_millis(80),
        "join returned before its timeout while the worker wrapper was held: elapsed={join_elapsed:?}, state={join_state_while_held:?}"
    );
    assert_eq!(join_state_while_held.completion_signaled, Some(true));
    assert!(join_state_while_held.worker_handle_present);
    assert_eq!(join_state_while_held.worker_handle_finished, Some(false));
    assert!(
        final_join.as_ref().is_ok_and(|joined| *joined),
        "released worker and watchdog must be joinable: {final_join:?}"
    );
    let joined_state = session.join_state_for_test();
    assert!(!joined_state.worker_handle_present);
    assert!(!joined_state.watchdog_handle_present);
}

#[test]
fn physical_bucket_after_stale_metadata_uses_event_count_not_metadata() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    // Keep the physical boundaries outside normal worker-startup jitter.  The
    // overdue-catch-up guard is tested deterministically below; this test is
    // about the event-count bucket after stale metadata, not about racing a
    // 500-us wall-clock schedule during thread startup.
    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "bucket-first".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 150_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "bucket-release".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 200_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "bucket-stale".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Down,
                scheduled_us: 250_000,
                scan_codes: smallvec::smallvec![0x17, 0x18],
                reason: "bucket-two-down".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Up,
                scheduled_us: 350_000,
                scan_codes: smallvec::smallvec![0x17, 0x18],
                reason: "bucket-cleanup".into(),
            },
        ],
        &[0x15, 0x16, 0x17, 0x18],
    )
    .expect("valid bucket schedule");
    let mut options = test_session_options(
        schedule,
        4,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.telemetry.capacity = 16;
    options.wait.supervisor_lease_timeout_us = 3_000_000;
    let session = NativeDispatchSession::new(options).expect("session admission");
    start_with_test_wall_clock_slack(&session);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(session.snapshot().terminal_error, None);
    let records: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("records JSON");
    let two_down = records["records"]
        .as_array()
        .expect("records")
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(3))
        .expect("two-down record");
    assert_eq!(two_down["requested_count"].as_u64(), Some(2));
    assert_eq!(two_down["applied_lead_ticks"].as_u64(), Some(0));
}
