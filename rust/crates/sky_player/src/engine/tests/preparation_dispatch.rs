use super::support::*;
use super::*;

#[test]
fn large_stream_materialization_cancels_at_bounded_checkpoints_before_any_input() {
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::StreamMaterialization, 5);
    let send_calls = Arc::new(AtomicU64::new(0));
    let mut fault_script = FaultInjectionScript::none();
    fault_script.send_call_count = Some(Arc::clone(&send_calls));
    let mut options = test_session_options(
        large_startup_materialization_schedule(512),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script,
        },
    );
    options.profile = DispatchProfile::MockTest;
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    let session = NativeDispatchSession::new(options).expect("large session admission");

    session.arm(100_000).expect("large session arm request");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
    assert_eq!(session.poll_state().status, EnginePollStatus::Preparing);
    assert_eq!(session.epoch_qpc_for_test(), QpcTicks::ZERO);
    assert_eq!(session.pre_roll_us_for_test(), 0);
    assert!(!session.snapshot().startup_ready);
    assert_eq!(send_calls.load(Ordering::Acquire), 0);

    std::thread::sleep(Duration::from_millis(80));
    hook.release_startup_checkpoint();
    let ready_deadline = Instant::now() + Duration::from_secs(5);
    while !session.snapshot().startup_armed {
        assert!(
            Instant::now() < ready_deadline,
            "large stream never became Ready"
        );
        assert_eq!(send_calls.load(Ordering::Acquire), 0);
        std::thread::yield_now();
    }
    assert!(session.snapshot().startup_ready);
    let epoch = session.epoch_qpc_for_test();
    assert!(epoch > QpcTicks::ZERO);
    assert_eq!(session.pre_roll_us_for_test(), 100_000);
    assert!(session.snapshot().startup_latency_us.unwrap_or_default() > 48_000);

    let first_send_deadline = Instant::now() + Duration::from_secs(3);
    while hook.down_sender_attempt_count() == 0 {
        assert!(
            Instant::now() < first_send_deadline,
            "first authored Down did not send"
        );
        std::thread::yield_now();
    }
    assert!(send_calls.load(Ordering::Acquire) > 0);
    let (first_authored_target, _) = hook.last_down_window();
    assert_eq!(first_authored_target, epoch.as_u64());
    assert!(hook.last_down_sender_start_qpc_ticks() >= epoch.as_u64());

    session
        .quit()
        .expect("stop the long deterministic schedule");
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("large worker join")
    );
}

#[test]
fn pre_roll_remaining_is_monotonic_and_zero_after_epoch() {
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

    session.arm(500_000).expect("arm session");
    let armed_deadline = Instant::now() + Duration::from_secs(2);
    while !session.snapshot().startup_armed {
        assert!(
            Instant::now() < armed_deadline,
            "worker never published Armed"
        );
        std::thread::yield_now();
    }
    let mut samples = Vec::new();
    while !session.snapshot().is_finished {
        samples.push(session.snapshot().pre_roll_remaining_us);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(samples.windows(2).all(|pair| pair[0] >= pair[1]));
    assert_eq!(session.snapshot().pre_roll_remaining_us, 0);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn zero_requested_pre_roll_uses_production_floor() {
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

    session.arm(0).expect("arm session");
    let ready_deadline = Instant::now() + Duration::from_secs(2);
    while session.pre_roll_us_for_test() == 0 {
        assert!(
            Instant::now() < ready_deadline,
            "worker never published preroll"
        );
        std::thread::yield_now();
    }
    assert_eq!(session.pre_roll_us_for_test(), 50_000);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
}

#[test]
fn stale_metadata_commits_before_first_physical_send() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Up,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "stale-zero".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 50,
                scan_codes: smallvec::smallvec![0x15],
                reason: "stale-fifty".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 500_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "physical-hundred".to_string().into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 1_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "cleanup".to_string().into(),
            },
        ],
        &[0x15],
    )
    .expect("valid stale-prefix schedule");
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
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    options.wait.supervisor_lease_timeout_us = 3_000_000;

    let session = NativeDispatchSession::new(options).expect("session admission");
    start_with_test_wall_clock_slack(&session);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    assert_eq!(snapshot.outcome, Some("finished".to_string()));
    assert_eq!(snapshot.terminal_error, None);
    assert_eq!(
        hook.stale_packet_committed.load(Ordering::SeqCst),
        2,
        "ordering hook values stale={} physical={}",
        hook.stale_packet_committed.load(Ordering::SeqCst),
        hook.first_physical_send_started.load(Ordering::SeqCst),
    );
    assert!(hook.first_physical_send_started.load(Ordering::SeqCst) > 0);
    assert!(
        hook.stale_packet_committed.load(Ordering::SeqCst)
            <= hook.first_physical_send_started.load(Ordering::SeqCst)
    );
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("valid telemetry JSON");
    let stale_records = telemetry["records"]
        .as_array()
        .expect("records")
        .iter()
        .filter(|record| matches!(record["event_index"].as_u64(), Some(0) | Some(1)))
        .collect::<Vec<_>>();
    assert_eq!(stale_records.len(), 2);
    assert!(
        stale_records
            .iter()
            .all(|record| record["applied_lead_ticks"].as_u64() == Some(0))
    );
}

#[test]
fn many_leading_stale_packets_are_drained_before_precision_handoff() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let stale_count = 32usize;
    let mut actions = Vec::with_capacity(stale_count + 2);
    for index in 0..stale_count {
        actions.push(KeyActionInput {
            source_action_index: index as u32,
            kind: ActionKind::Up,
            scheduled_us: index as u64,
            scan_codes: smallvec::smallvec![0x15],
            reason: "many-stale".to_string().into(),
        });
    }
    actions.push(KeyActionInput {
        source_action_index: stale_count as u32,
        kind: ActionKind::Down,
        scheduled_us: 500_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "many-stale-down".to_string().into(),
    });
    actions.push(KeyActionInput {
        source_action_index: (stale_count + 1) as u32,
        kind: ActionKind::Up,
        // Keep the cleanup boundary after the test-only arm margin.  This
        // test checks stale metadata ordering, not host wake-up lateness.
        scheduled_us: 1_000_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "many-stale-cleanup".to_string().into(),
    });
    let schedule = compile_runtime_intents(&actions, &[0x15]).expect("valid many-stale schedule");
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
    options.startup_ordering_hook = Some(Arc::clone(&hook));
    options.telemetry.capacity = 64;
    let session = NativeDispatchSession::new(options).expect("session admission");
    start_with_test_wall_clock_slack(&session);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    assert_eq!(session.snapshot().terminal_error, None);
    assert_eq!(
        hook.stale_packet_committed.load(Ordering::SeqCst),
        stale_count as u64
    );
    assert!(hook.first_physical_send_started.load(Ordering::SeqCst) > 0);
}

#[test]
fn all_stale_schedule_finishes_without_precision_or_physical_work() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let actions = (0..8)
        .map(|index| KeyActionInput {
            source_action_index: index,
            kind: ActionKind::Up,
            scheduled_us: u64::from(index),
            scan_codes: smallvec::smallvec![0x15],
            reason: "all-stale".to_string().into(),
        })
        .collect::<Vec<_>>();
    let schedule = compile_runtime_intents(&actions, &[0x15]).expect("valid all-stale schedule");
    let mut options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.telemetry.capacity = 16;
    let session = NativeDispatchSession::new(options).expect("session admission");
    start_with_test_wall_clock_slack(&session);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));
    let snapshot = session.snapshot();
    assert_eq!(snapshot.outcome, Some("finished".to_string()));
    assert_eq!(snapshot.terminal_error, None);
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry JSON"))
            .expect("valid telemetry JSON");
    let records = telemetry["records"].as_array().expect("records");
    assert_eq!(records.len(), 8);
    assert!(records.iter().all(|record| {
        record["kind"].as_u64() == Some(1)
            && record["requested_count"].as_u64() == Some(0)
            && record["send_attempts"].as_u64() == Some(0)
            && record["applied_lead_ticks"].as_u64() == Some(0)
    }));
}

#[test]
fn same_timestamp_stale_leading_up_packet_is_suppressed_atomically() {
    let telemetry =
        run_seeded_adaptive_startup_schedule(same_timestamp_stale_leading_up_schedule(), 1);
    let records = telemetry["records"].as_array().expect("records array");
    let stale = records
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(0))
        .expect("same-timestamp stale packet record");
    assert_eq!(stale["kind"].as_u64(), Some(1));
    assert_eq!(stale["requested_count"].as_u64(), Some(0));
    assert_eq!(stale["send_attempts"].as_u64(), Some(0));
    assert!(
        records
            .iter()
            .all(|record| record["event_index"].as_u64() != Some(1)),
        "same packet must produce one stale telemetry record"
    );
    let down = records
        .iter()
        .find(|record| record["event_index"].as_u64() == Some(2))
        .expect("first physical Down record");
    assert_eq!(down["kind"].as_u64(), Some(0));
    assert_eq!(down["applied_lead_ticks"].as_u64(), Some(0));
}

#[test]
fn stale_leading_up_does_not_consume_startup_target() {
    let telemetry = run_seeded_adaptive_startup_schedule(stale_leading_up_schedule(&[0]), 1);
    assert_stale_leading_up_did_not_send(&telemetry, 1);
}

#[test]
fn multiple_stale_leading_ups_do_not_consume_startup_target() {
    let telemetry = run_seeded_adaptive_startup_schedule(stale_leading_up_schedule(&[0, 50]), 1);
    assert_stale_leading_up_did_not_send(&telemetry, 2);
}

#[test]
fn production_profile_has_no_observer_samples_or_trace_records() {
    use sky_dispatch_core::compile::compile_runtime_intents;

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1_000_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "production-profile-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1_200_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "production-profile-up".into(),
            },
        ],
        &[0x15],
    )
    .expect("valid production-profile schedule");
    let (snapshot, telemetry) =
        run_mock_schedule_with_profile(schedule, 1, 16, DispatchProfile::Production);
    assert_eq!(snapshot.outcome, Some("finished".into()));
    assert_eq!(snapshot.terminal_error, None);
    assert!(!snapshot.recent_latency_samples_available);
    assert!(snapshot.recent_latencies_us.is_empty());
    assert_eq!(snapshot.observer_queue_high_watermark, 0);
    assert_eq!(telemetry["records"].as_array().map(Vec::len), Some(0));
}

#[test]
fn modifier_guard_rejects_each_modifier_and_exact_multi_mask_without_send() {
    let held_sets: &[(&[i32], u8)] = &[
        (&[0x5B], 0x01),
        (&[0x5C], 0x02),
        (&[0x11], 0x04),
        (&[0x10], 0x08),
        (&[0x12], 0x10),
        (&[0x5B, 0x11, 0x12], 0x15),
    ];

    for &(held_vks, expected_mask) in held_sets {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let packets = harness.configure_packet_capture();
        let query_count = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&query_count);
        let held_vks = held_vks.to_vec();
        harness.set_modifier_key_state_query_for_test(move |virtual_key| {
            counted.fetch_add(1, Ordering::SeqCst);
            if held_vks.contains(&virtual_key) {
                i16::MIN
            } else {
                0
            }
        });
        harness.prepare_prepared_stream_for_test();

        let step = harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000);
        assert!(matches!(
            step,
            super::worker::DispatchStep::Terminate(ref error)
                if error == &format!("physical_modifier_held:{expected_mask:02X}")
        ));
        assert_eq!(query_count.load(Ordering::SeqCst), 5);
        assert!(packets.lock().expect("packet capture").is_empty());
        assert_eq!(harness.local_metrics.final_gate_control_rejections, 1);
    }
}

#[test]
fn modifier_guard_rejects_first_down_without_send_or_unowned_cleanup() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let packets = harness.configure_packet_capture();
    let query_count = Arc::new(AtomicU64::new(0));
    let counted = Arc::clone(&query_count);
    harness.set_modifier_key_state_query_for_test(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        i16::MIN
    });
    harness.prepare_prepared_stream_for_test();
    let accounting_before = harness.generation_accounting_for_test();
    let cursor_before = harness.prepared_cursor_for_test();

    let step = harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000);
    assert!(matches!(
        step,
        super::worker::DispatchStep::Terminate(ref error)
            if error == "physical_modifier_held:1F"
    ));
    assert_eq!(query_count.load(Ordering::SeqCst), 5);
    assert!(packets.lock().expect("packet capture").is_empty());
    assert_eq!(harness.local_metrics.final_gate_control_rejections, 1);
    assert_eq!(harness.backend_active_mask(), 0);
    assert_eq!(harness.prepared_cursor_for_test(), cursor_before);
    assert_eq!(harness.generation_accounting_for_test(), accounting_before);

    let mut cleanup = super::worker::FinalizeTestObservation::default();
    let _ = harness.finalize_worker_for_test(false, &mut cleanup);
    assert_eq!(cleanup.attempted_mask, 0);
    assert!(packets.lock().expect("packet capture").is_empty());
}

#[test]
fn modifier_guard_queries_mixed_packet_once_and_keeps_up_before_down_send() {
    let mut harness = ProductionDispatchTestHarness::new_mixed();
    let packets = harness.configure_packet_capture();
    let query_count = Arc::new(AtomicU64::new(0));
    let counted = Arc::clone(&query_count);
    harness.set_modifier_key_state_query_for_test(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        0
    });
    harness.prepare_prepared_stream_for_test();

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(query_count.load(Ordering::SeqCst), 10);
    assert_eq!(
        *packets.lock().expect("packet capture"),
        vec![
            PhysicalPacket::new(0, 0b001),
            PhysicalPacket::new(0b001, 0b010),
        ]
    );
}

#[test]
fn modifier_guard_rejection_after_owned_down_runs_only_scoped_cleanup() {
    let mut harness = ProductionDispatchTestHarness::new_mixed();
    let packets = harness.configure_packet_capture();
    let query_count = Arc::new(AtomicU64::new(0));
    let counted = Arc::clone(&query_count);
    harness.set_modifier_key_state_query_for_test(move |_| {
        let index = counted.fetch_add(1, Ordering::SeqCst);
        if index < 5 { 0 } else { i16::MIN }
    });
    harness.prepare_prepared_stream_for_test();

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(harness.backend_active_mask(), 0b001);
    let accounting_before_rejection = harness.generation_accounting_for_test();
    let cursor_before_rejection = harness.prepared_cursor_for_test();

    let step = harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000);
    assert!(matches!(
        step,
        super::worker::DispatchStep::Terminate(ref error)
            if error == "physical_modifier_held:1F"
    ));
    assert_eq!(query_count.load(Ordering::SeqCst), 10);
    assert_eq!(harness.prepared_cursor_for_test(), cursor_before_rejection);
    assert_eq!(
        harness.generation_accounting_for_test(),
        accounting_before_rejection
    );
    assert_eq!(
        *packets.lock().expect("packet capture"),
        vec![PhysicalPacket::new(0, 0b001)]
    );

    let mut cleanup = super::worker::FinalizeTestObservation::default();
    let _ = harness.finalize_worker_for_test(false, &mut cleanup);
    assert_eq!(cleanup.attempted_mask, 0b001);
    assert_eq!(cleanup.active_mask, 0);
    assert_eq!(
        *packets.lock().expect("packet capture"),
        vec![PhysicalPacket::new(0, 0b001)]
    );
}

#[test]
fn modifier_guard_up_only_packet_performs_no_additional_modifier_queries() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let packets = harness.configure_packet_capture();
    let query_count = Arc::new(AtomicU64::new(0));
    let counted = Arc::clone(&query_count);
    harness.set_modifier_key_state_query_for_test(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        0
    });
    harness.prepare_prepared_stream_for_test();

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(query_count.load(Ordering::SeqCst), 5);
    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(query_count.load(Ordering::SeqCst), 5);
    assert_eq!(packets.lock().expect("packet capture").len(), 2);
}

#[test]
fn modifier_guard_final_revalidation_rejects_post_probe_target_aba() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let packets = harness.configure_packet_capture();
    let query_count = Arc::new(AtomicU64::new(0));
    let counted = Arc::clone(&query_count);
    harness.set_modifier_key_state_query_for_test(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        0
    });
    harness.set_post_modifier_revalidation_race_hook(|_, target, _, _, _, _, _| {
        target.publish(456);
    });
    harness.prepare_prepared_stream_for_test();
    let accounting_before = harness.generation_accounting_for_test();
    let cursor_before = harness.prepared_cursor_for_test();

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Continue
    ));
    assert_eq!(query_count.load(Ordering::SeqCst), 5);
    assert!(packets.lock().expect("packet capture").is_empty());
    assert_eq!(harness.prepared_cursor_for_test(), cursor_before);
    assert_eq!(harness.generation_accounting_for_test(), accounting_before);
}

#[test]
fn modifier_guard_final_revalidation_rejects_post_probe_owner_loss() {
    let _foreground_override_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_override_reset = FocusOverrideResetGuard;
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(1));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(77)));

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.set_require_focus_for_test(true);
    harness.bind_owner_identity_for_test(77);
    let packets = harness.configure_packet_capture();
    let query_count = Arc::new(AtomicU64::new(0));
    let counted = Arc::clone(&query_count);
    harness.set_modifier_key_state_query_for_test(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        0
    });
    harness.set_post_modifier_revalidation_race_hook(|_, target, _, _, _, _, _| {
        target.invalidate_owner_identity();
    });
    harness.prepare_prepared_stream_for_test();
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(77)));
    let accounting_before = harness.generation_accounting_for_test();
    let cursor_before = harness.prepared_cursor_for_test();

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::TerminateStatic("prepared_down_owner_process_terminated")
    ));
    assert_eq!(query_count.load(Ordering::SeqCst), 5);
    assert!(packets.lock().expect("packet capture").is_empty());
    assert_eq!(harness.prepared_cursor_for_test(), cursor_before);
    assert_eq!(harness.generation_accounting_for_test(), accounting_before);
}

#[test]
fn normal_prepared_overdue_boundaries_are_sent_once() {
    for lateness_us in [2_000, 10_000, 50_000, 100_000] {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let calls = harness.configure_send_counter();
        let mut stream = harness.build_prepared_stream_for_test();

        let step = harness.dispatch_prepared_current_at_lateness_for_test(&mut stream, lateness_us);
        assert!(
            matches!(step, super::worker::DispatchStep::Dispatched),
            "backlog recovery {lateness_us}us step: {step:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1, "lateness {lateness_us}us");
        assert_eq!(harness.missed_unobserved_backlog_boundaries_for_test(), 0);
        assert_eq!(harness.timeline_rebase_count_for_test(), 0);
    }
}

#[test]
fn normal_prepared_overdue_boundaries_send_sequentially() {
    let mut harness =
        ProductionDispatchTestHarness::new_dense_future_boundary_with_gap_for_test(10_000);
    let calls = harness.configure_send_counter();
    let mut stream = harness.build_prepared_stream_for_test();

    for lateness_us in [50_000, 50_000] {
        let step = harness.dispatch_prepared_current_at_lateness_for_test(&mut stream, lateness_us);
        assert!(
            matches!(step, super::worker::DispatchStep::Dispatched),
            "overdue boundary recovery: {step:?}"
        );
    }

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(harness.missed_unobserved_backlog_boundaries_for_test(), 0);
    assert_eq!(harness.timeline_rebase_count_for_test(), 0);
}

#[test]
fn prepared_late_down_keeps_following_up_accounting() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let (packets, evidence) = harness.configure_prepared_packet_capture_with_evidence_for_test();
    let mut stream = harness.build_prepared_stream_for_test();

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_for_test(&mut stream, 2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(
        *packets.lock().expect("packet capture"),
        vec![sky_dispatch_win32::input::PhysicalPacket::new(0, 1)]
    );
    {
        let captured = evidence.lock().expect("prepared Down evidence");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].requested_mask, 1);
        assert_eq!(captured[0].confirmed_mask, 1);
        assert_eq!(captured[0].attempts, 1);
    }
    let after_down = harness.resources.coordinator.generation_accounting();
    assert_eq!(after_down.activated, 1);
    assert_eq!(after_down.active, 1);
    assert_eq!(after_down.released, 0);
    assert_eq!(after_down.dropped_expired, 0);

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_for_test(&mut stream, 2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(
        *packets.lock().expect("packet capture"),
        vec![
            sky_dispatch_win32::input::PhysicalPacket::new(0, 1),
            sky_dispatch_win32::input::PhysicalPacket::new(1, 0),
        ]
    );
    {
        let captured = evidence.lock().expect("prepared Down/Up evidence");
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[1].requested_mask, 1);
        assert_eq!(captured[1].confirmed_mask, 1);
        assert_eq!(captured[1].attempts, 1);
    }
    assert_eq!(harness.missed_unobserved_backlog_boundaries_for_test(), 0);
    assert_eq!(harness.backend_active_mask(), 0);
    let after_up = harness.resources.coordinator.generation_accounting();
    assert_eq!(after_up.activated, 1);
    assert_eq!(after_up.active, 0);
    assert_eq!(after_up.released, 1);
    assert_eq!(after_up.dropped_expired, 0);
}

#[test]
fn prepared_mixed_late_sends_the_frozen_whole_packets() {
    let mut harness = ProductionDispatchTestHarness::new_mixed();
    let packets = harness.configure_packet_capture();
    harness.prepare_prepared_stream_for_test();

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(
        *packets.lock().expect("packet capture"),
        vec![
            sky_dispatch_win32::input::PhysicalPacket::new(0, 0b001),
            sky_dispatch_win32::input::PhysicalPacket::new(0b001, 0b010),
        ]
    );
    assert_eq!(harness.missed_unobserved_backlog_boundaries_for_test(), 0);
}

#[test]
fn prepared_mixed_unobserved_down_sends_only_the_canonical_up_prefix() {
    let mut harness = ProductionDispatchTestHarness::new_mixed_then_future_down();
    let packets = harness.configure_packet_capture();
    harness.prepare_prepared_stream_for_test();

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(0),
        super::worker::DispatchStep::Dispatched
    ));
    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(20_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(
        *packets.lock().expect("packet capture"),
        vec![
            sky_dispatch_win32::input::PhysicalPacket::new(0, 0b001),
            sky_dispatch_win32::input::PhysicalPacket::new(0b001, 0b010),
        ]
    );
    assert_eq!(harness.missed_unobserved_backlog_boundaries_for_test(), 0);
}

#[test]
fn prepared_mixed_transport_failure_is_fail_closed_without_retry() {
    let mut harness = ProductionDispatchTestHarness::new_mixed();
    let evidence =
        harness.configure_prepared_transport_outcome_for_test(SendTransactionStatus::ZeroProgress);
    harness.prepare_prepared_stream_for_test();

    let step = harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000);
    assert!(matches!(
        step,
        super::worker::DispatchStep::Terminate(_) | super::worker::DispatchStep::TerminateStatic(_)
    ));
    assert_eq!(evidence.lock().expect("transport evidence").len(), 1);
    assert_eq!(harness.resources.coordinator.cursor, 0);
    assert_eq!(harness.backend_active_mask(), 0);
}

#[test]
fn prepared_no_catch_up_burst_matrix_is_exact_for_2_3_and_15_boundaries() {
    for boundary_count in [2, 3, 15] {
        let mut harness =
            ProductionDispatchTestHarness::new_overdue_down_burst_for_test(boundary_count);
        let calls = harness.configure_send_counter();
        let mut stream = harness.build_prepared_stream_for_test();

        for _ in 0..boundary_count {
            assert!(matches!(
                harness.dispatch_prepared_current_at_lateness_for_test(&mut stream, 50_000),
                super::worker::DispatchStep::Dispatched
            ));
        }

        assert_eq!(calls.load(Ordering::SeqCst), boundary_count as u64);
        assert_eq!(harness.prepared_normal_backlog_count_for_test(), 0);
        assert_eq!(harness.prepared_up_prefix_recovery_sends_for_test(), 0);
        assert_eq!(harness.timeline_rebase_count_for_test(), 0);
    }
}

#[test]
fn prepared_transport_fault_matrix_is_fail_closed_without_retry() {
    for status in [
        SendTransactionStatus::Complete,
        SendTransactionStatus::ZeroProgress,
        SendTransactionStatus::PartialProgress,
        SendTransactionStatus::IntegrityLost,
        SendTransactionStatus::ClockFailureBeforeSend,
        SendTransactionStatus::ClockFailureAfterSend,
    ] {
        let mut harness = ProductionDispatchTestHarness::new_mixed();
        let evidence = harness.configure_prepared_transport_outcome_for_test(status);
        harness.prepare_prepared_stream_for_test();

        let step = harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000);
        if status == SendTransactionStatus::Complete {
            assert!(matches!(step, super::worker::DispatchStep::Dispatched));
            let step = harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000);
            assert!(matches!(step, super::worker::DispatchStep::Dispatched));
            assert_eq!(harness.resources.coordinator.cursor, 3);
        } else {
            assert!(matches!(
                step,
                super::worker::DispatchStep::Terminate(_)
                    | super::worker::DispatchStep::TerminateStatic(_)
            ));
            assert_eq!(harness.resources.coordinator.cursor, 0);
        }
        assert_eq!(
            evidence.lock().expect("transport evidence").len(),
            if status == SendTransactionStatus::Complete {
                2
            } else {
                1
            },
            "status={status:?} must not retry a failed musical Down"
        );
        let captured = evidence.lock().expect("transport evidence");
        assert!(
            captured.iter().all(|entry| {
                entry.attempts == u8::from(status != SendTransactionStatus::ClockFailureBeforeSend)
            }),
            "status={status:?} evidence={captured:?}"
        );
        assert_eq!(
            harness.backend_active_mask(),
            u16::from(status == SendTransactionStatus::Complete) * 2
        );
        assert_eq!(harness.timeline_rebase_count_for_test(), 0);
    }
}

#[test]
fn prepared_full_packet_transport_outcome_matrix_is_fail_closed() {
    for (status, expected_attempts, expected_possibly_active) in [
        (SendTransactionStatus::Complete, 1, 0),
        (SendTransactionStatus::ZeroProgress, 1, 0),
        (SendTransactionStatus::PartialProgress, 1, 1),
        (SendTransactionStatus::IntegrityLost, 1, 1),
        (SendTransactionStatus::ClockFailureBeforeSend, 0, 0),
        (SendTransactionStatus::ClockFailureAfterSend, 1, 1),
    ] {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let evidence = harness.configure_prepared_transport_outcome_for_test(status);
        harness.prepare_prepared_stream_for_test();

        let step = harness.dispatch_prepared_current_at_lateness_without_stream_for_test(0);
        let captured = evidence.lock().expect("full transport evidence lock");
        assert_eq!(
            captured.len(),
            1,
            "full prepared sender calls for {status:?}"
        );
        assert_eq!(captured[0].requested_mask, 0b001);
        assert_eq!(captured[0].attempts, expected_attempts);
        assert_eq!(
            harness.resources.coordinator.cursor,
            u16::from(status == SendTransactionStatus::Complete) as usize
        );
        assert_eq!(
            harness.backend_active_mask(),
            u16::from(status == SendTransactionStatus::Complete)
        );
        assert_eq!(
            harness.backend_possibly_active_mask(),
            expected_possibly_active
        );
        if status == SendTransactionStatus::Complete {
            assert!(matches!(step, super::worker::DispatchStep::Dispatched));
        } else {
            assert!(matches!(
                step,
                super::worker::DispatchStep::Terminate(_)
                    | super::worker::DispatchStep::TerminateStatic(_)
            ));
        }
    }
}

#[test]
fn prepared_acceptance_counters_cover_late_normal_sends() {
    let mut backlog = ProductionDispatchTestHarness::new_mixed();
    backlog.prepare_prepared_stream_for_test();
    assert!(matches!(
        backlog.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert!(matches!(
        backlog.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Dispatched
    ));

    let mut late_current_down = ProductionDispatchTestHarness::new_mixed_then_future_down();
    late_current_down.prepare_prepared_stream_for_test();
    assert!(matches!(
        late_current_down.dispatch_prepared_current_at_lateness_without_stream_for_test(0),
        super::worker::DispatchStep::Dispatched
    ));
    assert!(matches!(
        late_current_down.dispatch_prepared_current_at_lateness_without_stream_for_test(20_000),
        super::worker::DispatchStep::Dispatched
    ));

    assert_eq!(backlog.prepared_normal_backlog_count_for_test(), 0);
    assert_eq!(backlog.prepared_up_prefix_recovery_sends_for_test(), 0);
    assert_eq!(
        late_current_down.prepared_normal_backlog_count_for_test(),
        0
    );
    assert_eq!(
        late_current_down.prepared_up_prefix_recovery_sends_for_test(),
        0
    );
    assert_eq!(backlog.timeline_rebase_count_for_test(), 0);
    assert_eq!(late_current_down.timeline_rebase_count_for_test(), 0);
}

#[test]
fn prepared_late_sequence_preserves_each_frozen_boundary() {
    let mut harness =
        ProductionDispatchTestHarness::new_prepared_late_causality_sequence_for_test();
    let packets = harness.configure_packet_capture();
    harness.prepare_prepared_stream_for_test();
    harness
        .align_prepared_current_to_benchmark_margin_for_test(10_000)
        .expect("future first prepared boundary");
    let first_step = harness.dispatch_prepared_current_after_stall_for_test(50_000);
    assert!(
        matches!(first_step, super::worker::DispatchStep::Dispatched),
        "late first boundary step: {first_step:?}"
    );
    assert_eq!(
        *packets.lock().expect("first packet capture"),
        vec![sky_dispatch_win32::input::PhysicalPacket::new(0, 0b001)],
        "first boundary must send its full packet after the physical stall"
    );

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(50_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(50_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(harness.prepared_normal_backlog_count_for_test(), 0);
    assert_eq!(harness.prepared_up_prefix_recovery_sends_for_test(), 0);

    harness
        .align_prepared_current_to_benchmark_margin_for_test(10_000)
        .expect("future next prepared boundary");
    assert!(matches!(
        harness.wait_and_dispatch_prepared_current_for_test(),
        Ok(super::worker::DispatchStep::Dispatched)
    ));
    assert_eq!(
        *packets.lock().expect("causality packet capture"),
        vec![
            sky_dispatch_win32::input::PhysicalPacket::new(0, 0b001),
            sky_dispatch_win32::input::PhysicalPacket::new(0, 0b010),
            sky_dispatch_win32::input::PhysicalPacket::new(0, 0b100),
            sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1000),
        ]
    );
    assert_eq!(harness.timeline_rebase_count_for_test(), 0);
}

#[test]
fn prepared_normal_down_lateness_matrix_keeps_one_transport_attempt() {
    let policy = issue_379_materialized_policy();
    assert_eq!(policy.frame_us, 16_667);
    assert_eq!(policy.frame_base_hold_us, 16_667);
    assert_eq!(policy.timing_margin_us, 500);
    assert_eq!(policy.min_hold_us, 17_167);
    assert_eq!(policy.min_release_gap_us, 17_167);

    for (label, lateness_us) in [
        ("exact", Some(0)),
        ("one QPC tick", None),
        ("50 us", Some(50)),
        ("500 us", Some(500)),
        ("2,000 us", Some(2_000)),
        ("20,000 us", Some(20_000)),
        ("100,000 us", Some(100_000)),
    ] {
        let mut harness = ProductionDispatchTestHarness::new_prepared_zero_slack_down_up_for_test(
            policy.min_hold_us,
        );
        let (packets, evidence) =
            harness.configure_prepared_packet_capture_with_evidence_for_test();
        harness.prepare_prepared_stream_for_test();
        let authored_offsets = harness.prepared_frame_offsets_for_test();
        assert_eq!(authored_offsets.len(), 2, "{label}: paired Down/Up frames");
        let expected_hold_ticks = harness
            .qpc_duration_from_us_for_test(policy.min_hold_us)
            .unwrap_or_else(|error| panic!("{label}: hold conversion: {error}"));
        assert_eq!(
            authored_offsets[1] - authored_offsets[0],
            expected_hold_ticks.as_u64(),
            "{label}: authored Up - authored Down must equal policy.min_hold_us"
        );
        harness
            .align_prepared_current_to_benchmark_margin_for_test(10_000)
            .expect("future authored boundary alignment");
        let initial_epoch = harness.playback_epoch_qpc_for_test();

        let authored_target = harness
            .prepared_current_target_qpc_for_test()
            .expect("prepared authored target");
        let now = harness.qpc_now_for_test().expect("prepared target QPC");
        assert!(
            authored_target > now,
            "{label}: boundary A must remain authored and future before dispatch"
        );
        assert_eq!(
            harness.playback_epoch_qpc_for_test(),
            initial_epoch,
            "{label}: dispatch setup must not rebase playback"
        );

        let lateness = lateness_us.map_or(DurationTicks::from_raw(1), |us| {
            harness
                .qpc_duration_from_us_for_test(us)
                .unwrap_or_else(|error| panic!("{label}: lateness conversion: {error}"))
        });
        assert!(matches!(
            harness.dispatch_prepared_current_after_stall_ticks_for_test(lateness),
            super::worker::DispatchStep::Dispatched
        ));
        assert_eq!(
            harness.playback_epoch_qpc_for_test(),
            initial_epoch,
            "{label}: late Down must not rebase playback"
        );
        assert_eq!(
            *packets.lock().expect("boundary A packet capture"),
            vec![sky_dispatch_win32::input::PhysicalPacket::new(0, 0b0001)],
            "{label}: late boundary A must emit one physical Down"
        );
        {
            let captured = evidence.lock().expect("boundary A transport evidence");
            assert_eq!(captured.len(), 1, "{label}: boundary A transport count");
            assert_eq!(
                captured[0].requested_mask, 0b0001,
                "{label}: boundary A mask"
            );
            assert_eq!(captured[0].attempts, 1, "{label}: boundary A send attempts");
        }

        let accounting = harness.resources.coordinator.generation_accounting();
        assert_eq!(accounting.activated, 1, "{label}: activated");
        assert_eq!(accounting.active, 1, "{label}: active");
        assert_eq!(accounting.released, 0, "{label}: released");
        assert_eq!(accounting.dropped_expired, 0, "{label}: dropped_expired");
        assert_eq!(
            harness.prepared_normal_backlog_count_for_test(),
            0,
            "{label}: prepared normal backlog"
        );
        assert_eq!(
            harness.missed_unobserved_backlog_boundaries_for_test(),
            0,
            "{label}: unobserved backlog"
        );
        assert_eq!(harness.timeline_rebase_count_for_test(), 0);
    }
}

#[test]
fn prepared_normal_timing_scalars_survive_diagnostics_and_trace_off_modes() {
    for diagnostics_enabled in [true, false] {
        let mut harness =
            ProductionDispatchTestHarness::new_prepared_zero_slack_down_up_for_test(17_167);
        let (packets, evidence) =
            harness.configure_prepared_packet_capture_with_evidence_for_test();
        harness.set_live_diagnostics_enabled_for_test(diagnostics_enabled);
        *harness.resources.telemetry.lock() = super::telemetry::TelemetryCollector::new(
            if diagnostics_enabled {
                TelemetryMode::Ring
            } else {
                TelemetryMode::Off
            },
            64,
        );
        harness.prepare_prepared_stream_for_test();
        harness
            .align_prepared_current_to_benchmark_margin_for_test(10_000)
            .expect("future authored boundary alignment");
        assert!(matches!(
            harness.dispatch_prepared_current_after_stall_for_test(500),
            super::worker::DispatchStep::Dispatched
        ));
        assert_eq!(
            *packets.lock().expect("packet capture"),
            [sky_dispatch_win32::input::PhysicalPacket::new(0, 1)]
        );
        assert_eq!(evidence.lock().expect("transport evidence")[0].attempts, 1);
        assert_eq!(harness.local_metrics.precision_packet_attempts, 1);
        assert_eq!(harness.local_metrics.authored_lateness_samples, 1);
        assert_eq!(harness.local_metrics.physical_floor_displacement_samples, 1);
        assert_eq!(harness.local_metrics.residual_lateness_samples, 1);
        assert_eq!(harness.local_metrics.sendinput_envelope_samples, 1);

        while harness.pending_observation_count() != 0 {
            harness
                .drain_observer()
                .expect("drain prepared timing observation");
        }
        let records = harness.telemetry_records_for_test();
        assert_eq!(
            records.len(),
            usize::from(diagnostics_enabled),
            "trace mode must follow the selected diagnostic mode"
        );
    }
}

#[test]
fn late_prepared_down_sends_each_unseen_boundary_without_catch_up() {
    let policy = issue_379_materialized_policy();
    let mut harness =
        ProductionDispatchTestHarness::new_prepared_materialized_causality_sequence_for_test(
            policy.min_hold_us,
        );
    let (packets, evidence) = harness.configure_prepared_packet_capture_with_evidence_for_test();
    harness.prepare_prepared_stream_for_test();
    harness
        .align_prepared_current_to_benchmark_margin_for_test(10_000)
        .expect("future authored boundary alignment");
    let initial_epoch = harness.playback_epoch_qpc_for_test();
    let authored_target = harness
        .prepared_current_target_qpc_for_test()
        .expect("prepared authored target");
    assert!(authored_target > harness.qpc_now_for_test().expect("authorization QPC"));
    assert_eq!(harness.playback_epoch_qpc_for_test(), initial_epoch);

    let lateness = harness
        .qpc_duration_from_us_for_test(100_000)
        .expect("100 ms lateness conversion");
    assert!(matches!(
        harness.dispatch_prepared_current_after_stall_ticks_for_test(lateness),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(harness.playback_epoch_qpc_for_test(), initial_epoch);
    assert_eq!(
        *packets.lock().expect("boundary A packet capture"),
        vec![sky_dispatch_win32::input::PhysicalPacket::new(0, 0b0001)]
    );

    let backlog_wall_now = authored_target
        .checked_add_duration(lateness)
        .expect("backlog wall QPC arithmetic");
    for boundary in ["B", "C"] {
        assert!(matches!(
            harness.dispatch_prepared_current_at_synthetic_wall_qpc_for_test(backlog_wall_now),
            super::worker::DispatchStep::Dispatched
        ));
        assert_eq!(
            harness.playback_epoch_qpc_for_test(),
            initial_epoch,
            "overdue {boundary} must not rebase playback"
        );
    }
    assert_eq!(harness.prepared_normal_backlog_count_for_test(), 0);
    assert_eq!(harness.missed_unobserved_backlog_boundaries_for_test(), 0);
    assert_eq!(
        evidence.lock().expect("backlog transport evidence").len(),
        3
    );
    while harness.pending_observation_count() != 0 {
        harness
            .drain_observer()
            .expect("drain causal backlog observation");
    }
    let backlog_records = harness
        .telemetry_records_for_test()
        .into_iter()
        .filter(|record| record.outcome == trace_outcome_code("down_unobserved_backlog"))
        .collect::<Vec<_>>();
    assert!(backlog_records.is_empty());

    let later_target = harness
        .prepared_current_target_qpc_for_test()
        .expect("later D target");
    assert!(
        later_target > harness.qpc_now_for_test().expect("later D QPC"),
        "D must remain naturally future after B/C classification"
    );
    assert_eq!(harness.playback_epoch_qpc_for_test(), initial_epoch);
    assert_eq!(harness.playback_epoch_qpc_for_test(), initial_epoch);
    let later_step =
        harness.dispatch_prepared_current_after_stall_ticks_for_test(DurationTicks::from_raw(1));
    assert!(
        matches!(later_step, super::worker::DispatchStep::Dispatched),
        "later D dispatch step: {later_step:?}"
    );
    assert_eq!(harness.playback_epoch_qpc_for_test(), initial_epoch);
    assert_eq!(
        *packets.lock().expect("causal packet capture"),
        vec![
            sky_dispatch_win32::input::PhysicalPacket::new(0, 0b0001),
            sky_dispatch_win32::input::PhysicalPacket::new(0, 0b0010),
            sky_dispatch_win32::input::PhysicalPacket::new(0, 0b0100),
            sky_dispatch_win32::input::PhysicalPacket::new(0, 0b1000),
        ]
    );
    let captured = evidence.lock().expect("causal transport evidence");
    assert_eq!(captured.len(), 4);
    assert!(captured.iter().all(|entry| entry.attempts == 1));
    assert_eq!(harness.timeline_rebase_count_for_test(), 0);
}

#[test]
fn prepared_unpaired_down_has_no_lateness_cutoff() {
    let mut backlog = ProductionDispatchTestHarness::new_prepared_unpaired_down_for_test();
    let backlog_packets = backlog.configure_packet_capture();
    backlog.prepare_prepared_stream_for_test();
    assert!(matches!(
        backlog.dispatch_prepared_current_at_lateness_without_stream_for_test(20_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(
        *backlog_packets.lock().expect("unpaired packet capture"),
        vec![sky_dispatch_win32::input::PhysicalPacket::new(0, 0b11)]
    );
    assert_eq!(backlog.prepared_normal_backlog_count_for_test(), 0);

    let mut second = ProductionDispatchTestHarness::new_prepared_unpaired_down_for_test();
    let second_packets = second.configure_packet_capture();
    second.prepare_prepared_stream_for_test();
    assert!(matches!(
        second.dispatch_prepared_current_at_lateness_without_stream_for_test(20_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(
        *second_packets
            .lock()
            .expect("second unpaired packet capture"),
        vec![sky_dispatch_win32::input::PhysicalPacket::new(0, 0b11)]
    );
    assert_eq!(second.prepared_normal_backlog_count_for_test(), 0);
}

#[test]
fn prepared_suspension_preserves_cancellation_and_current_packet() {
    let mut harness =
        ProductionDispatchTestHarness::new_prepared_resumable_suspension_mixed_miss_for_test();
    let packets = harness.configure_packet_capture();
    harness.prepare_prepared_stream_for_test();

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(0),
        super::worker::DispatchStep::Dispatched
    ));
    let cancelled = harness
        .suspend_live_input_for_test()
        .expect("resumable suspension cancellation");
    assert_eq!(cancelled, 1);

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(
        *packets.lock().expect("suspension packet capture"),
        vec![
            sky_dispatch_win32::input::PhysicalPacket::new(0, 0b001),
            sky_dispatch_win32::input::PhysicalPacket::new(0b001, 0b010),
        ]
    );
    assert_eq!(harness.prepared_normal_backlog_count_for_test(), 0);
    assert_eq!(harness.prepared_up_prefix_recovery_sends_for_test(), 0);
    assert_eq!(harness.backend_active_mask(), 2);
    assert_eq!(harness.backend_possibly_active_mask(), 0);
    assert_eq!(harness.timeline_rebase_count_for_test(), 0);
}

#[test]
fn prepared_benchmark_stream_build_precedes_alignment_and_wait_entry() {
    let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 10_000);
    harness
        .configure_production_wait_policy(1_000)
        .expect("production wait policy");
    harness.prepare_prepared_stream_for_test();
    let built_qpc = harness
        .prepared_stream_built_qpc_for_test()
        .expect("prepared stream build timestamp");
    let _build_duration_us = harness
        .prepared_stream_build_duration_us_for_test()
        .expect("prepared stream build duration");

    harness
        .align_prepared_current_to_benchmark_margin_for_test(10_000)
        .expect("prepared benchmark alignment");
    let alignment_qpc = harness
        .prepared_alignment_qpc_for_test()
        .expect("prepared alignment timestamp");
    assert!(
        built_qpc <= alignment_qpc,
        "prepared stream construction must finish before target alignment"
    );

    assert!(matches!(
        harness.wait_and_dispatch_prepared_current_for_test(),
        Ok(super::worker::DispatchStep::Dispatched)
    ));
    let (_, target_qpc, wait_entry_qpc) = harness
        .prepared_benchmark_qpc_evidence_for_test()
        .expect("prepared wait-entry evidence");
    assert!(
        alignment_qpc <= wait_entry_qpc,
        "target alignment must precede wait entry"
    );
    assert!(
        target_qpc >= alignment_qpc,
        "stream construction must not reduce the post-alignment target budget"
    );
}

#[test]
fn prepared_normal_resumable_suspension_reconciles_frozen_up_and_continues() {
    let mut harness =
        ProductionDispatchTestHarness::new_prepared_resumable_suspension_sequence_for_test();
    let packets = harness.configure_packet_capture();
    harness.prepare_prepared_stream_for_test();
    let frame_offsets_before = harness.prepared_frame_offsets_for_test();

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(packets.lock().expect("prepared packet capture").len(), 1);

    let cancelled = harness
        .suspend_live_input_for_test()
        .expect("shared resumable suspension");
    assert_eq!(cancelled, 1);
    assert_eq!(
        harness.prepared_suspension_cancellation_count_for_test(),
        1,
        "the prepared stream must retain the shared suspension reconciliation"
    );
    assert_eq!(harness.backend_active_mask(), 0);
    assert_eq!(harness.backend_possibly_active_mask(), 0);

    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert!(matches!(
        harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        super::worker::DispatchStep::Dispatched
    ));
    assert_eq!(packets.lock().expect("prepared packet capture").len(), 3);
    assert_eq!(
        *packets.lock().expect("prepared packet capture"),
        vec![
            sky_dispatch_win32::input::PhysicalPacket::new(0, 0b001),
            sky_dispatch_win32::input::PhysicalPacket::new(0b001, 0),
            sky_dispatch_win32::input::PhysicalPacket::new(0, 0b010),
        ]
    );
    assert_eq!(harness.timeline_rebase_count_for_test(), 0);
    assert_eq!(harness.backend_active_mask(), 0b010);
    assert_eq!(harness.backend_possibly_active_mask(), 0);
    assert_eq!(
        harness.prepared_frame_offsets_for_test(),
        frame_offsets_before,
        "resumable suspension must not rewrite prepared frame offsets"
    );
}

#[test]
fn mixed_packet_partial_fault_stops_before_committing_retrigger() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            // Keep this first target well after arm/startup so this transport
            // fault test does not depend on a concurrent test thread winning
            // the exact epoch boundary.
            scheduled_us: 500_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            // Leave enough authored interval for this fault-injection test
            // to reach the mixed packet; hold-feasibility is tested directly
            // with exact QPC boundaries elsewhere.
            scheduled_us: 1_000_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "retrigger-up".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 1_000_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "disjoint-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 1_100_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "disjoint-up".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15, 0x16])
        .expect("valid mixed packet schedule");
    let script = FaultInjectionScript {
        entries: vec![(
            1,
            InjectedSendOutcome::Partial {
                inserted: 1,
                latency_ticks: 0,
                win32_error: 5,
            },
        )],
        ..FaultInjectionScript::none()
    };
    let session = NativeDispatchSession::new(test_session_options(
        schedule,
        2,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: script,
        },
    ))
    .expect("test session admission");

    start_with_test_wall_clock_slack(&session);
    assert!(session.join(Duration::from_secs(5)).expect("worker join"));

    let snapshot = session.snapshot();
    assert_eq!(snapshot.status, "error", "{snapshot:?}");
    assert!(snapshot.sendinput_partial_events >= 1);
    assert!(snapshot.chord_split_events >= 1);
    assert_eq!(snapshot.possibly_active_count, 0);
    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry"))
            .expect("valid telemetry JSON");
    let records = telemetry["records"].as_array().expect("records array");
    assert!(
        !records
            .iter()
            .any(|record| { record["event_index"] == 2 && record["runtime_outcome"] == "sent" })
    );
}

#[test]
fn mixed_disjoint_packet_success_commits_new_generation() {
    // Keep the first authored boundary out of worker startup/preemption. The
    // assertions below cover generation ownership and mixed ordering, not
    // zero-slack dispatch timing.
    const TEST_AUTHORED_EPOCH_US: u64 = 2_000_000;
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            // Keep each physical boundary away from worker startup and leave
            // observer work deterministic under the full parallel suite.
            scheduled_us: TEST_AUTHORED_EPOCH_US,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Down,
            scheduled_us: TEST_AUTHORED_EPOCH_US + 500_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "disjoint-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: TEST_AUTHORED_EPOCH_US + 500_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "retrigger-up".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: TEST_AUTHORED_EPOCH_US + 1_000_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "stale-release".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 4,
            kind: ActionKind::Up,
            scheduled_us: TEST_AUTHORED_EPOCH_US + 1_000_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "release-two".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15, 0x16])
        .expect("valid disjoint mixed schedule");
    let session = NativeDispatchSession::new(test_session_options(
        schedule,
        2,
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
    assert_eq!(
        snapshot.status, "finished",
        "terminal error: {:?}",
        snapshot.terminal_error
    );
    assert_eq!(snapshot.generation_status_counts["released"], 2);
    assert_eq!(snapshot.active_count, 0);
    assert_eq!(snapshot.possibly_active_count, 0);

    let telemetry: serde_json::Value =
        serde_json::from_str(&session.take_telemetry_json().expect("telemetry"))
            .expect("valid telemetry JSON");
    let mixed = telemetry["records"]
        .as_array()
        .expect("records array")
        .iter()
        .find(|record| record["kind"].as_u64() == Some(2))
        .expect("successful disjoint mixed record");
    assert_eq!(mixed["requested_count"].as_u64(), Some(2));
    assert_eq!(mixed["sent_count"].as_u64(), Some(2));
    assert_eq!(mixed["polyphony"].as_u64(), Some(2));
}
