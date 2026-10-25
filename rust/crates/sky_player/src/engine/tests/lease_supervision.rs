use super::support::*;
use super::*;

#[test]
fn supervisor_lease_treats_future_heartbeat_as_fresh() {
    let heartbeat = super::shared::SupervisorLeaseState::new(QpcTicks::from_raw(1_001));
    assert_eq!(
        supervisor_lease_expired(
            QpcTicks::from_raw(1_000),
            DurationTicks::from_raw(100),
            &heartbeat,
        ),
        Ok(false)
    );
}

#[test]
fn late_supervisor_progress_cannot_heal_a_missed_deadline() {
    let lease = super::shared::SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    let timeout = DurationTicks::from_raw(100);
    let late_progress = QpcTicks::from_raw(1_101);

    assert!(!lease.publish_progress(late_progress, timeout));
    assert!(lease.is_expired(), "late progress must latch expiry itself");
    assert!(lease.check_expired(late_progress, timeout));
}

#[test]
fn unrelated_heartbeat_cannot_keep_a_stalled_monitor_lease_alive() {
    let lease = super::shared::SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    let unrelated_helper_progress = AtomicU64::new(1_000);
    unrelated_helper_progress.store(1_100, Ordering::Release);
    let now = QpcTicks::from_raw(1_101);

    assert!(unrelated_helper_progress.load(Ordering::Acquire) >= now.as_u64() - 1);
    assert!(lease.check_expired(now, DurationTicks::from_raw(100)));
}

#[test]
fn supervisor_lease_progress_extends_on_time_and_keeps_strict_equality_alive() {
    let lease = super::shared::SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    let timeout = DurationTicks::from_raw(100);

    assert!(lease.publish_progress(QpcTicks::from_raw(1_050), timeout));
    assert_eq!(lease.last_progress_ticks(), 1_050);
    assert!(lease.publish_progress(QpcTicks::from_raw(1_150), timeout));
    assert!(!lease.check_expired(QpcTicks::from_raw(1_250), timeout));
    assert!(lease.publish_progress(QpcTicks::from_raw(1_250), timeout));
    assert!(!lease.check_expired(QpcTicks::from_raw(1_350), timeout));
    assert!(lease.check_expired(QpcTicks::from_raw(1_351), timeout));
}

#[test]
fn terminal_session_does_not_accept_more_supervisor_progress() {
    let mut options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.wait.supervisor_lease_timeout_us = 0;
    let session = NativeDispatchSession::new(options).expect("terminal-progress admission");
    session.arm(0).expect("terminal-progress arm");
    session.quit().expect("terminal-progress stop");
    assert!(session.join(Duration::from_secs(2)).expect("terminal join"));

    let terminal_progress = session.supervisor_heartbeat_qpc_for_test();
    session
        .publish_supervisor_progress()
        .expect("progress after terminal ownership is a no-op");
    assert_eq!(
        session.supervisor_heartbeat_qpc_for_test(),
        terminal_progress
    );
}

#[test]
fn lifecycle_supervisor_lease_down_proof_is_bounded_and_fails_closed() {
    use super::shared::{SupervisorLeaseAdmission, SupervisorLeaseState};

    let timeout = DurationTicks::from_raw(100);
    let expired = SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    let expired_proof = expired.prove_down_age(QpcTicks::from_raw(1_101), timeout);
    assert_eq!(expired_proof.admission, SupervisorLeaseAdmission::Expired);
    assert_eq!(expired_proof.state_loads, 1);
    assert_eq!(expired_proof.compare_exchanges, 1);
    assert!(expired.is_expired());

    let equality = SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    let equality_proof = equality.prove_down_age(QpcTicks::from_raw(1_100), timeout);
    assert_eq!(equality_proof.admission, SupervisorLeaseAdmission::Fresh);
    assert_eq!(equality_proof.state_loads, 1);
    assert_eq!(equality_proof.compare_exchanges, 0);

    let future = SupervisorLeaseState::new(QpcTicks::from_raw(1_101));
    assert_eq!(
        future
            .prove_down_age(QpcTicks::from_raw(1_100), timeout)
            .admission,
        SupervisorLeaseAdmission::Fresh,
        "newer-than-Q progress is fresh and must not underflow"
    );

    let missing = SupervisorLeaseState::new(QpcTicks::from_raw(0));
    let missing_proof = missing.prove_down_age(QpcTicks::from_raw(1_000), timeout);
    assert_eq!(missing_proof.admission, SupervisorLeaseAdmission::Unproven);
    assert_eq!(missing_proof.state_loads, 1);
    assert_eq!(missing_proof.compare_exchanges, 0);

    let disabled = SupervisorLeaseState::new(QpcTicks::from_raw(0));
    let disabled_proof =
        disabled.prove_down_age(QpcTicks::from_raw(1_000_000), DurationTicks::ZERO);
    assert_eq!(disabled_proof.admission, SupervisorLeaseAdmission::Fresh);
    assert_eq!(disabled_proof.state_loads, 0);
    assert_eq!(disabled_proof.compare_exchanges, 0);

    let raced_fresh = SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    raced_fresh.force_down_proof_cas_failure_for_test(Some(QpcTicks::from_raw(1_150)), false);
    let fresh_replacement = raced_fresh.prove_down_age(QpcTicks::from_raw(1_201), timeout);
    assert_eq!(fresh_replacement.admission, SupervisorLeaseAdmission::Fresh);
    assert_eq!(fresh_replacement.state_loads, 2);
    assert_eq!(fresh_replacement.compare_exchanges, 1);

    let raced_expired = SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    raced_expired.force_down_proof_cas_failure_for_test(None, true);
    let expired_replacement = raced_expired.prove_down_age(QpcTicks::from_raw(1_201), timeout);
    assert_eq!(
        expired_replacement.admission,
        SupervisorLeaseAdmission::Expired
    );
    assert_eq!(expired_replacement.state_loads, 2);
    assert_eq!(expired_replacement.compare_exchanges, 1);

    let raced_aged = SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    raced_aged.force_down_proof_cas_failure_for_test(Some(QpcTicks::from_raw(900)), false);
    let aged_replacement = raced_aged.prove_down_age(QpcTicks::from_raw(1_201), timeout);
    assert_eq!(
        aged_replacement.admission,
        SupervisorLeaseAdmission::Unproven
    );
    assert_eq!(aged_replacement.state_loads, 2);
    assert_eq!(aged_replacement.compare_exchanges, 1);
    assert!(!raced_aged.is_expired(), "Unproven does not claim expiry");

    let raced_missing = SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    raced_missing.force_down_proof_cas_failure_for_test(None, false);
    let missing_replacement = raced_missing.prove_down_age(QpcTicks::from_raw(1_201), timeout);
    assert_eq!(
        missing_replacement.admission,
        SupervisorLeaseAdmission::Unproven
    );
    assert_eq!(missing_replacement.state_loads, 2);
    assert_eq!(missing_replacement.compare_exchanges, 1);
}

#[test]
fn lifecycle_final_down_lease_sample_is_single_and_clock_failure_is_distinct() {
    use super::shared::{SupervisorLeaseAdmission, SupervisorLeaseState};

    let lease = SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    let clock = QpcClock::from_frequency_hz(std::num::NonZeroU64::new(1_000_000).unwrap());
    let mut sample_count = 0;
    let mut sample_override = Some(Err(sky_dispatch_win32::clock::QpcError::CounterUnavailable));
    let result = super::worker::final_down_lease_age_proof(
        &lease,
        DurationTicks::from_raw(100),
        clock,
        &mut sample_count,
        &mut sample_override,
    );
    assert_eq!(
        result,
        Err(sky_dispatch_win32::clock::QpcError::CounterUnavailable)
    );
    assert_eq!(sample_count, 1);

    let mut disabled_sample_count = 0;
    let mut disabled_override = Some(Err(sky_dispatch_win32::clock::QpcError::CounterUnavailable));
    let proof = super::worker::final_down_lease_age_proof(
        &lease,
        DurationTicks::ZERO,
        clock,
        &mut disabled_sample_count,
        &mut disabled_override,
    )
    .expect("disabled diagnostic lease takes no age sample");
    assert_eq!(proof.admission, SupervisorLeaseAdmission::Fresh);
    assert_eq!(disabled_sample_count, 0);
    assert!(disabled_override.is_some());
}

#[test]
fn overdue_supervisor_lease_blocks_prepared_down() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness =
        ProductionDispatchTestHarness::new_prepared_down_or_uponly_for_test(1, 17_167)
            .expect("prepared Down lease fixture");
    harness
        .set_supervisor_lease_timeout_for_test(100)
        .expect("enabled supervisor lease timeout");

    let now = harness.qpc_now_for_test().expect("lease fixture QPC");
    let timeout = harness
        .qpc_duration_from_us_for_test(100)
        .expect("lease timeout conversion");
    assert!(now.as_u64() > timeout.as_u64() + 1);
    let stale_stamp = QpcTicks::from_raw(now.as_u64() - timeout.as_u64() - 1);
    harness.set_supervisor_heartbeat_for_test(stale_stamp);
    assert!(
        !harness.supervisor_expired.is_expired(),
        "the watchdog and publisher remain delayed; only the final Down gate can observe age"
    );

    harness.prepare_prepared_stream_for_test();
    let send_calls = harness.configure_prepared_zero_progress_sender_for_test();
    let cursor_before = harness.prepared_cursor_for_test();
    let accounting_before = harness.generation_accounting_for_test();
    let due = harness
        .prepared_physical_floor_for_test()
        .expect("prepared Down floor")
        .packet_not_before_qpc;
    let step =
        harness.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(due);

    assert_eq!(
        send_calls.load(Ordering::SeqCst),
        0,
        "an overdue unlatched lease must be rejected before the prepared sender"
    );
    assert!(matches!(
        step,
        super::worker::DispatchStep::TerminateStatic("supervisor_lease_expired")
    ));
    assert_eq!(harness.local_metrics.final_gate_lease_expirations, 1);
    assert_eq!(harness.final_lease_qpc_sample_count_for_test(), 1);
    assert_eq!(harness.prepared_cursor_for_test(), cursor_before);
    assert_eq!(harness.generation_accounting_for_test(), accounting_before);
}

#[test]
fn lifecycle_unproven_lease_blocks_prepared_down_without_latching_expiry() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness =
        ProductionDispatchTestHarness::new_prepared_down_or_uponly_for_test(1, 17_167)
            .expect("prepared Down lease fixture");
    harness
        .set_supervisor_lease_timeout_for_test(100)
        .expect("enabled supervisor lease timeout");
    harness.set_supervisor_heartbeat_for_test(QpcTicks::from_raw(0));
    assert!(!harness.supervisor_expired.is_expired());
    harness.prepare_prepared_stream_for_test();
    let send_calls = harness.configure_prepared_zero_progress_sender_for_test();
    let cursor_before = harness.prepared_cursor_for_test();
    let accounting_before = harness.generation_accounting_for_test();
    let due = harness
        .prepared_physical_floor_for_test()
        .expect("prepared Down floor")
        .packet_not_before_qpc;

    let step =
        harness.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(due);

    assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        step,
        super::worker::DispatchStep::TerminateStatic("supervisor_lease_unproven")
    ));
    assert!(!harness.supervisor_expired.is_expired());
    assert_eq!(harness.final_lease_qpc_sample_count_for_test(), 1);
    assert_eq!(harness.prepared_cursor_for_test(), cursor_before);
    assert_eq!(harness.generation_accounting_for_test(), accounting_before);
    assert_eq!(harness.local_metrics.final_gate_lease_expirations, 1);
}

#[test]
fn lifecycle_overdue_lease_after_modifier_stage_blocks_prepared_down() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness =
        ProductionDispatchTestHarness::new_prepared_down_or_uponly_for_test(1, 17_167)
            .expect("prepared Down lease fixture");
    harness
        .set_supervisor_lease_timeout_for_test(100)
        .expect("enabled supervisor lease timeout");
    harness.set_supervisor_heartbeat_for_test(harness.qpc_now_for_test().expect("fresh lease"));
    harness.set_final_lease_age_hook_for_test(|lease| {
        lease.reset(QpcTicks::from_raw(1));
    });
    harness.prepare_prepared_stream_for_test();
    let send_calls = harness.configure_prepared_zero_progress_sender_for_test();
    let due = harness
        .prepared_physical_floor_for_test()
        .expect("prepared Down floor")
        .packet_not_before_qpc;

    let step =
        harness.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(due);

    assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        step,
        super::worker::DispatchStep::TerminateStatic("supervisor_lease_expired")
    ));
    assert_eq!(harness.final_lease_qpc_sample_count_for_test(), 1);
}

#[test]
fn lifecycle_prepared_down_qpc_failure_terminates_before_sender() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness =
        ProductionDispatchTestHarness::new_prepared_down_or_uponly_for_test(1, 17_167)
            .expect("prepared Down lease fixture");
    harness
        .set_supervisor_lease_timeout_for_test(100)
        .expect("enabled supervisor lease timeout");
    harness.set_supervisor_heartbeat_for_test(harness.qpc_now_for_test().expect("fresh lease"));
    harness.set_final_lease_qpc_sample_override_for_test(Err(
        sky_dispatch_win32::clock::QpcError::CounterUnavailable,
    ));
    harness.prepare_prepared_stream_for_test();
    let send_calls = harness.configure_prepared_zero_progress_sender_for_test();
    let due = harness
        .prepared_physical_floor_for_test()
        .expect("prepared Down floor")
        .packet_not_before_qpc;

    let step =
        harness.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(due);

    assert_eq!(send_calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        step,
        super::worker::DispatchStep::TerminateStatic("prepared_down_qpc_failure")
    ));
    assert_eq!(harness.final_lease_qpc_sample_count_for_test(), 1);
}

#[test]
fn lifecycle_prepared_uponly_does_not_take_lease_age_sample() {
    use super::test_support::ProductionDispatchTestHarness;

    let mut harness =
        ProductionDispatchTestHarness::new_prepared_down_or_uponly_for_test(1, 17_167)
            .expect("prepared Down and UpOnly lease fixture");
    harness
        .set_supervisor_lease_timeout_for_test(1_000_000)
        .expect("enabled supervisor lease timeout");
    harness.set_supervisor_heartbeat_for_test(harness.qpc_now_for_test().expect("fresh lease"));
    harness.prepare_prepared_stream_for_test();
    let calls = harness.configure_send_counter();
    let down_due = harness
        .prepared_physical_floor_for_test()
        .expect("prepared Down floor")
        .packet_not_before_qpc;
    let down_step = harness
        .dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(down_due);
    assert!(matches!(down_step, super::worker::DispatchStep::Dispatched));
    assert_eq!(harness.final_lease_qpc_sample_count_for_test(), 1);

    let up_due = harness
        .prepared_physical_floor_for_test()
        .expect("prepared UpOnly floor")
        .packet_not_before_qpc;
    let up_step =
        harness.dispatch_prepared_current_at_synthetic_wall_qpc_with_sender_start_for_test(up_due);
    assert!(matches!(up_step, super::worker::DispatchStep::Dispatched));
    assert_eq!(harness.final_lease_qpc_sample_count_for_test(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn lifecycle_late_up_empty_pause_resume_preserves_release_floor() {
    use super::test_support::ProductionDispatchTestHarness;

    for (frequency_hz, odd_vector) in [(1_000_000, false), (10_000_000, false), (10_000_003, true)]
    {
        let clock = QpcClock::from_frequency_hz(std::num::NonZeroU64::new(frequency_hz).unwrap());
        let at_us = |us| {
            QpcTicks::from_raw(
                clock
                    .duration_from_us(us)
                    .expect("vector QPC conversion")
                    .as_u64(),
            )
        };
        let mut harness = ProductionDispatchTestHarness::new_lifecycle_sequence_for_test(
            frequency_hz,
            odd_vector,
        )
        .expect("lifecycle vector admission");
        let packets = harness.configure_mock_packet_sender_for_test(&[0, 50, 0, 0]);
        let cleanup = harness.configure_mock_cleanup_emitter_for_test();
        harness.prepare_prepared_stream_for_test();
        harness
            .reanchor_playback_clock_for_test(at_us(82_700))
            .expect("lifecycle playback epoch");

        let down_at = at_us(100_000);
        harness.set_mock_sender_start_qpc_for_test(down_at);
        assert!(matches!(
            harness
                .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_test(down_at)
                .expect("first Down due check"),
            Some(super::worker::DispatchStep::Dispatched)
        ));
        let up_floor = harness
            .prepared_physical_floor_for_test()
            .expect("authored Up floor");
        let up_at = up_floor.packet_not_before_qpc;
        let physical_hold_floor = at_us(100_000)
            .checked_add_duration(clock.duration_from_us(17_167).expect("hold conversion"))
            .expect("physical hold floor arithmetic");
        assert_eq!(up_at, physical_hold_floor);
        harness.set_mock_sender_start_qpc_for_test(up_at);
        assert!(matches!(
            harness
                .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_test(up_at)
                .expect("authored Up due check"),
            Some(super::worker::DispatchStep::Dispatched)
        ));

        let up_completion = packets
            .lock()
            .expect("sender capture")
            .get(1)
            .and_then(|(_, evidence)| evidence.completed_ticks)
            .expect("Up completion QPC");
        let completion_latency = clock.duration_from_us(50).expect("latency conversion");
        assert_eq!(
            up_completion,
            up_at.checked_add_duration(completion_latency).unwrap()
        );
        let frame_ticks = clock.duration_from_us(16_667).expect("frame conversion");
        let required_down_floor = up_completion
            .checked_add_duration(frame_ticks)
            .expect("release floor arithmetic");

        let frame_offsets_before_pause = harness.prepared_frame_offsets_for_test();
        let pause_at = at_us(117_300);
        let resume_at = at_us(118_300);
        harness
            .manual_pause_resume_for_test(pause_at, resume_at)
            .expect("actual manual pause and verified cleanup lifecycle");
        assert!(
            cleanup.lock().expect("cleanup capture").is_empty(),
            "empty-ownership pause must not call the keyboard transport"
        );
        assert_eq!(
            harness.prepared_frame_offsets_for_test(),
            frame_offsets_before_pause,
            "resume must preserve authored offsets"
        );
        assert_eq!(harness.prepared_cursor_for_test(), 2);
        assert_eq!(packets.lock().expect("sender capture").len(), 2);

        harness.set_mock_sender_start_qpc_for_test(resume_at);
        assert!(
            harness
                .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_test(resume_at)
                .expect("resumed Down due check")
                .is_none(),
            "resume must hold the next Down until the cleanup-relative frame barrier at {frequency_hz}Hz"
        );
        assert_eq!(harness.prepared_cursor_for_test(), 2);
        assert_eq!(packets.lock().expect("sender capture").len(), 2);
        let resume_floor = harness
            .prepared_physical_floor_for_test()
            .expect("post-cleanup Down floor");
        let expected_barrier = pause_at
            .checked_add_duration(frame_ticks)
            .expect("post-cleanup barrier arithmetic");
        assert_eq!(resume_floor.down_not_before_qpc, expected_barrier);
        harness.set_mock_sender_start_qpc_for_test(resume_floor.packet_not_before_qpc);
        assert!(matches!(
            harness
                .dispatch_prepared_current_at_synthetic_wall_qpc_if_due_for_test(
                    resume_floor.packet_not_before_qpc,
                )
                .expect("post-barrier resumed Down due check"),
            Some(super::worker::DispatchStep::Dispatched)
        ));
        let next_down_pre_call = packets
            .lock()
            .expect("sender capture")
            .get(2)
            .and_then(|(_, evidence)| evidence.started_ticks)
            .expect("resumed Down pre-call QPC");
        assert!(
            next_down_pre_call >= required_down_floor,
            "resumed Down at {next_down_pre_call:?} preceded last Up completion plus one frame {required_down_floor:?} at {frequency_hz}Hz"
        );
    }
}

#[test]
fn supervisor_lease_treats_equal_heartbeat_as_fresh() {
    let heartbeat = super::shared::SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    assert_eq!(
        supervisor_lease_expired(
            QpcTicks::from_raw(1_000),
            DurationTicks::from_raw(100),
            &heartbeat,
        ),
        Ok(false)
    );
}

#[test]
fn supervisor_lease_preserves_fresh_boundary_and_expiration() {
    let heartbeat = super::shared::SupervisorLeaseState::new(QpcTicks::from_raw(1_000));
    let timeout = DurationTicks::from_raw(100);
    assert_eq!(
        supervisor_lease_expired(QpcTicks::from_raw(1_050), timeout, &heartbeat),
        Ok(false)
    );
    assert_eq!(
        supervisor_lease_expired(QpcTicks::from_raw(1_100), timeout, &heartbeat),
        Ok(false)
    );
    assert_eq!(
        supervisor_lease_expired(QpcTicks::from_raw(1_101), timeout, &heartbeat),
        Ok(true)
    );
}

#[test]
fn supervisor_heartbeat_keeps_worker_alive_and_expiry_runs_cleanup() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 200_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "lease-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 300_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "lease-up".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("valid lease lifecycle schedule");

    let mut heartbeat_options = test_session_options(
        schedule.clone(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    heartbeat_options.wait.supervisor_lease_timeout_us = 50_000;
    let heartbeat_session =
        NativeDispatchSession::new(heartbeat_options).expect("heartbeat session admission");
    heartbeat_session.arm(0).expect("heartbeat session arm");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !heartbeat_session.snapshot().is_finished && Instant::now() < deadline {
        heartbeat_session
            .publish_supervisor_progress()
            .expect("supervisor heartbeat");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        heartbeat_session
            .join(Duration::from_secs(2))
            .expect("heartbeat join")
    );
    let heartbeat_snapshot = heartbeat_session.snapshot();
    assert_ne!(
        heartbeat_snapshot.terminal_trigger_reason.as_deref(),
        Some("supervisor_lease_expired"),
        "periodic heartbeat must keep the worker admitted: {heartbeat_snapshot:?}"
    );
    assert_eq!(heartbeat_snapshot.active_count, 0);
    assert_eq!(heartbeat_snapshot.possibly_active_count, 0);
    assert_eq!(heartbeat_snapshot.wait_interrupted_count, 0);
    assert_eq!(heartbeat_snapshot.timeline_rebase_count, 0);

    let long_gap_actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 5_000_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "watchdog-long-gap-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 5_100_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "watchdog-long-gap-up".to_string().into(),
        },
    ];
    let long_gap_schedule =
        sky_dispatch_core::compile::compile_runtime_intents(&long_gap_actions, &[0x15])
            .expect("valid long-gap watchdog schedule");
    let mut expiry_options = test_session_options(
        long_gap_schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    expiry_options.wait.supervisor_lease_timeout_us = 50_000;
    let expiry_session = NativeDispatchSession::new(expiry_options).expect("expiry admission");
    let expiry_started = Instant::now();
    expiry_session.arm(0).expect("expiry session arm");
    assert!(
        expiry_session
            .join(Duration::from_secs(2))
            .expect("expiry join")
    );
    let expiry_snapshot = expiry_session.snapshot();
    assert_eq!(
        expiry_snapshot.terminal_trigger_reason.as_deref(),
        Some("supervisor_lease_expired"),
        "stopped heartbeat must terminate the worker: {expiry_snapshot:?}"
    );
    assert_eq!(expiry_snapshot.status, "error");
    assert_eq!(expiry_snapshot.outcome.as_deref(), Some("error"));
    assert!(expiry_snapshot.terminal_error.is_none());
    assert!(expiry_snapshot.secondary_errors.is_empty());
    assert_eq!(expiry_snapshot.active_count, 0);
    assert_eq!(expiry_snapshot.possibly_active_count, 0);
    assert_eq!(expiry_snapshot.release_attempts_total, 0);
    let release = expiry_snapshot
        .release_outcome
        .as_ref()
        .expect("lease expiry publishes terminal release evidence");
    assert!(release.released_successfully);
    assert_eq!(release.attempted_mask, 0);
    assert_eq!(release.attempts, 0);
    assert_eq!(release.stuck_mask, 0);
    assert!(!release.verification_inconclusive);
    assert!(!release.transport_anomaly);
    let expiry_progress = expiry_session.snapshot_lite();
    assert_eq!(expiry_progress.status, "error");
    assert_eq!(
        expiry_progress.terminal_outcome,
        Some(EnginePollStatus::Error)
    );
    assert!(
        !expiry_progress.has_terminal_error,
        "clean hard-stop identity must not be encoded as cleanup failure"
    );
    let expiry_elapsed = expiry_started.elapsed();
    eprintln!("watchdog expiry latency: {expiry_elapsed:?}");
    assert!(
        expiry_elapsed < Duration::from_millis(500),
        "watchdog must interrupt the long musical wait promptly: {expiry_elapsed:?}"
    );
    assert!(expiry_snapshot.wait_interrupted_count >= 1);
    assert_eq!(expiry_snapshot.timeline_rebase_count, 0);
    assert!(expiry_snapshot.release_outcome.is_some());
    let telemetry: serde_json::Value = serde_json::from_str(
        &expiry_session
            .take_telemetry_json()
            .expect("expiry telemetry"),
    )
    .expect("valid expiry telemetry");
    assert!(
        telemetry["records"]
            .as_array()
            .expect("expiry records")
            .iter()
            .all(|record| record["send_attempts"].as_u64() == Some(0))
    );
}

#[test]
fn watchdog_age_begins_at_arm_heartbeat_not_construction() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 5_000_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "delayed-arm-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 5_100_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "delayed-arm-up".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("valid delayed-arm schedule");
    let mut options = test_session_options(
        schedule,
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.wait.supervisor_lease_timeout_us = 20_000;
    let session = NativeDispatchSession::new(options).expect("delayed-arm admission");
    let construction_heartbeat = session.supervisor_heartbeat_qpc_for_test();
    std::thread::sleep(Duration::from_millis(50));
    let now = QpcClock::initialize()
        .expect("QPC clock")
        .now()
        .expect("QPC now");
    let timeout = DurationTicks::from_raw(
        QpcClock::initialize()
            .expect("QPC clock")
            .duration_from_us(20_000)
            .expect("lease timeout")
            .as_u64(),
    );
    let stale_construction_heartbeat =
        super::shared::SupervisorLeaseState::new(construction_heartbeat);
    assert!(
        supervisor_lease_expired(now, timeout, &stale_construction_heartbeat)
            .expect("construction heartbeat age")
    );

    session.arm(0).expect("delayed-arm worker arm");
    let arm_heartbeat = session.supervisor_heartbeat_qpc_for_test();
    assert!(arm_heartbeat > construction_heartbeat);
    let fresh_arm_heartbeat = super::shared::SupervisorLeaseState::new(arm_heartbeat);
    assert!(
        !supervisor_lease_expired(
            QpcClock::initialize()
                .expect("QPC clock")
                .now()
                .expect("QPC now"),
            timeout,
            &fresh_arm_heartbeat,
        )
        .expect("arm heartbeat age")
    );

    session.quit().expect("quit delayed-arm session");
    assert!(
        session
            .join(Duration::from_secs(2))
            .expect("delayed-arm join")
    );
    assert_ne!(
        session.snapshot().terminal_trigger_reason.as_deref(),
        Some("supervisor_lease_expired"),
        "pre-arm construction heartbeat must not terminate the session"
    );
    assert_eq!(session.snapshot().timeline_rebase_count, 0);
}

#[test]
fn lease_timeout_conversion_failure_stays_pre_spawn_and_non_running() {
    let qpc_clock = QpcClock::initialize().expect("QPC clock");
    let frequency = u128::from(qpc_clock.frequency_hz().get());
    let overflow_lease_us = u64::try_from(
        (u128::from(u64::MAX) * 1_000_000 / frequency)
            .checked_add(1)
            .expect("QPC conversion overflow input must fit in u64 microseconds"),
    )
    .expect("Windows QPC frequency must expose a representable overflow input");
    assert!(qpc_clock.duration_from_us(overflow_lease_us).is_err());

    let mut options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.wait.supervisor_lease_timeout_us = overflow_lease_us;
    let session = NativeDispatchSession::new(options).expect("session admission");

    let error = session
        .arm(0)
        .expect_err("lease conversion must reject arm");
    assert!(error.contains("lease timeout conversion failed"), "{error}");

    let poll = session.poll_state();
    assert_eq!(poll.status.as_str(), "ready");
    assert!(!poll.is_finished);
    assert!(!session.snapshot().is_running);
    assert_eq!(session.spawn_handles_present_for_test(), (false, false));
    assert_eq!(session.snapshot().sendinput_partial_events, 0);
    assert_eq!(session.snapshot().sendinput_zero_progress_failures, 0);
    assert_eq!(
        session
            .join(Duration::from_millis(1))
            .expect_err("pre-spawn failure must remain unstarted"),
        "session has not been started"
    );
}

#[test]
fn explicit_panic_release_keeps_user_terminal_identity() {
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 5_000_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "explicit-panic-down".to_string().into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 5_100_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "explicit-panic-up".to_string().into(),
        },
    ];
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(&actions, &[0x15])
        .expect("valid explicit-panic schedule");
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::InitialWait, 1);
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
    let session = NativeDispatchSession::new(options).expect("explicit-panic admission");
    session.arm(0).expect("explicit-panic worker arm");
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
    session.panic_release().expect("explicit panic release");
    hook.release_startup_checkpoint();
    assert!(
        session
            .join(Duration::from_secs(2))
            .expect("explicit-panic join")
    );
    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.terminal_trigger_reason.as_deref(),
        Some("panic_release_requested")
    );
    assert_eq!(snapshot.outcome.as_deref(), Some("error"));
    assert!(snapshot.terminal_error.is_none());
    assert!(snapshot.secondary_errors.is_empty());
    assert_eq!(snapshot.active_count, 0);
    assert_eq!(snapshot.possibly_active_count, 0);
    assert_eq!(snapshot.timeline_rebase_count, 0);
    let release = snapshot
        .release_outcome
        .as_ref()
        .expect("explicit panic publishes terminal release evidence");
    assert_eq!(release.attempted_mask, 0);
    assert!(release.released_successfully);
    assert_eq!(release.stuck_mask, 0);
    assert!(!release.verification_inconclusive);
    assert!(!release.transport_anomaly);
    let progress = session.snapshot_lite();
    assert_eq!(progress.status, "error");
    assert_eq!(progress.terminal_outcome, Some(EnginePollStatus::Error));
    assert!(!progress.has_terminal_error);
}

#[test]
fn supervisor_lease_disabled_is_never_expired() {
    let heartbeat = super::shared::SupervisorLeaseState::new(QpcTicks::from_raw(1));
    assert_eq!(
        supervisor_lease_expired(QpcTicks::from_raw(2), DurationTicks::ZERO, &heartbeat),
        Ok(false)
    );

    let mut options = test_session_options(
        startup_boundary_schedule(),
        1,
        BackendConfig::Mock {
            latency_base_us: 0,
            latency_per_key_us: 0,
            fault_script: FaultInjectionScript::none(),
        },
    );
    options.wait.supervisor_lease_timeout_us = 0;
    let session = NativeDispatchSession::new(options).expect("disabled lease admission");
    session.arm(0).expect("disabled lease arm");
    assert!(
        session
            .join(Duration::from_secs(5))
            .expect("disabled lease join")
    );
    let snapshot = session.snapshot();
    assert_eq!(snapshot.terminal_error, None);
    assert_eq!(snapshot.outcome, Some("finished".to_string()));
}

#[test]
fn recent_latency_ring_is_bounded_and_keeps_latest_values() {
    let mut ring = RecentLatencyRing::default();
    for value in 0..40 {
        ring.push(value);
    }
    assert_eq!(ring.to_vec().len(), 32);
    assert_eq!(ring.to_vec().first(), Some(&8));
    assert_eq!(ring.to_vec().last(), Some(&39));
}

#[test]
fn supervisor_lease_concurrent_publication_never_reports_clock_error() {
    let heartbeat = Arc::new(super::shared::SupervisorLeaseState::new(
        QpcTicks::from_raw(1_000),
    ));
    let publisher_heartbeat = Arc::clone(&heartbeat);
    let publisher = std::thread::spawn(move || {
        for index in 0..10_000 {
            publisher_heartbeat.publish_progress(
                QpcTicks::from_raw(if index % 2 == 0 { 1_000 } else { 1_001 }),
                DurationTicks::from_raw(100),
            );
        }
    });

    for _ in 0..10_000 {
        let result = supervisor_lease_expired(
            QpcTicks::from_raw(1_000),
            DurationTicks::from_raw(100),
            &heartbeat,
        );
        assert!(result.is_ok(), "concurrent heartbeat sample: {result:?}");
    }
    publisher.join().expect("heartbeat publisher must finish");
}

#[test]
fn exact_sender_duration_distinguishes_single_call_from_operation() {
    let clock = QpcClock::from_frequency_hz(std::num::NonZeroU64::new(qpc_frequency()).unwrap());
    let started = QpcTicks::from_raw(1_000);
    let completed = started
        .checked_add_duration(DurationTicks::from_raw(qpc_us_to_ticks(20).unwrap()))
        .unwrap();

    assert_eq!(
        exact_sender_durations(clock, Some(started), Some(completed), 1, false).unwrap(),
        (Some(20), Some(20))
    );
    assert_eq!(
        exact_sender_durations(clock, Some(started), Some(completed), 2, false).unwrap(),
        (Some(20), None)
    );
    assert_eq!(
        exact_sender_durations(clock, Some(started), Some(completed), 2, true).unwrap(),
        (Some(20), None)
    );
    assert!(exact_sender_durations(clock, None, Some(completed), 1, false).is_err());
}

#[test]
fn completion_error_ticks_preserves_signed_timeline_delta() {
    assert_eq!(
        signed_timeline_delta_ticks(TimelineTicks::from_raw(120), TimelineTicks::from_raw(100),),
        Ok(20)
    );
    assert_eq!(
        signed_timeline_delta_ticks(TimelineTicks::from_raw(100), TimelineTicks::from_raw(120),),
        Ok(-20)
    );
}

#[test]
fn completion_error_ticks_rejects_unrepresentable_signed_delta() {
    assert_eq!(
        signed_timeline_delta_ticks(
            TimelineTicks::from_raw(u64::MAX),
            TimelineTicks::from_raw(0),
        ),
        Err(sky_dispatch_core::time::TimeArithmeticError::Overflow)
    );
    assert_eq!(
        signed_timeline_delta_ticks(
            TimelineTicks::from_raw(0),
            TimelineTicks::from_raw((i64::MAX as u64) + 1),
        ),
        Ok(i64::MIN)
    );
}

#[test]
fn completion_residuals_keep_authored_and_effective_deadlines_distinct() {
    let completed = TimelineTicks::from_raw(950);
    let effective_deadline = TimelineTicks::from_raw(900);
    let authored_deadline = TimelineTicks::from_raw(1_000);

    assert_eq!(
        signed_timeline_delta_ticks(completed, effective_deadline),
        Ok(50)
    );
    assert_eq!(
        signed_timeline_delta_ticks(completed, authored_deadline),
        Ok(-50)
    );
}

#[test]
fn wait_error_is_relative_to_deadline() {
    assert_eq!(
        wake_lateness_ticks(
            TimelineTicks::from_raw(20_000),
            TimelineTicks::from_raw(20_000),
        )
        .expect("on-time wake")
        .as_u64(),
        0
    );
    assert_eq!(
        wake_lateness_ticks(
            TimelineTicks::from_raw(20_500),
            TimelineTicks::from_raw(20_000),
        )
        .expect("late wake")
        .as_u64(),
        500
    );
    assert_eq!(
        wake_lateness_ticks(
            TimelineTicks::from_raw(19_900),
            TimelineTicks::from_raw(20_000),
        )
        .expect("early wake")
        .as_u64(),
        0
    );
}

#[test]
fn native_trace_counts_are_semantic_and_summary_uses_them() {
    let record = RtTraceRecord::dispatched(
        TraceContext {
            event_index: 7,
            source_action_index: 7,
            compiled_packet_index: None,
            kind: TRACE_KIND_DOWN,
            outcome: trace_outcome_code("sent"),
            polyphony: 3,
            flags: TRACE_FLAG_SENT_FULL,
            send_status: 0,
            win32_error: 0,
            up_mask: 0,
            down_mask: 0b111,
        },
        TraceTiming {
            authored_ticks: TimelineTicks::from_raw(10),
            effective_deadline_ticks: TimelineTicks::from_raw(12),
            wake_ticks: Some(TimelineTicks::from_raw(13)),
            wake_qpc_ticks: Some(114),
            target_crossing_qpc_ticks: Some(106),
            physical_target_qpc_ticks: Some(100),
            physical_not_before_qpc_ticks: Some(105),
            hold_floor_qpc_ticks: Some(103),
            release_floor_qpc_ticks: Some(105),
            previous_sendinput_completion_qpc_ticks: Some(110),
            previous_sender_completion_known: true,
            hold_floor_mask: 1,
            release_floor_mask: 2,
            final_policy_qpc_ticks: Some(118),
            pre_call_qpc_ticks: Some(120),
            sendinput_completion_qpc_ticks: Some(125),
            post_send_qpc_ticks: Some(130),
            observation_qpc_ticks: None,
            pre_call_ticks: Some(TimelineTicks::from_raw(22)),
            sendinput_completion_ticks: Some(TimelineTicks::from_raw(25)),
            completion_residual_us: 5,
            core_post_send_duration_us: 4,
            post_send_metrics_available: true,
            completion_error_ticks: 1,
            authored_completion_error_ticks: 2,
        },
        TraceDelivery {
            requested: 3,
            sent: 2,
            skipped: 1,
            send_attempts: 2,
        },
    )
    .unwrap();

    assert_eq!(record.requested_count, 3);
    assert_eq!(record.sent_count, 2);
    assert_eq!(record.skipped_count, 1);
    assert_eq!(record.send_attempts, 2);
    assert_eq!(record.send_started_ticks, 22);
    assert_eq!(record.send_completed_ticks, 25);
    assert_eq!(record.authored_offset_ticks, 10);
    assert_eq!(record.physical_floor_displacement_ticks, 5);
    assert_eq!(record.hold_floor_displacement_ticks, 3);
    assert_eq!(record.release_floor_displacement_ticks, 5);
    assert_eq!(record.residual_lateness_ticks, 15);
    assert_eq!(record.hol_delay_lower_bound_ticks, 5);
    assert_eq!(record.unattributed_lateness_ticks, 10);
    assert_eq!(record.sendinput_envelope_ticks, 5);
    assert_eq!(record.authored_lateness_ticks, 20);
    assert!(record.authored_target_overdue);
    assert!(record.transport_anomaly);
    assert_eq!(record.wake_qpc_ticks, 114);
    assert_eq!(record.target_crossing_qpc_ticks, 106);
    assert_eq!(
        record.final_policy_qpc_provenance,
        TimestampProvenance::Measured
    );
    assert_eq!(record.wake_qpc_provenance, TimestampProvenance::Measured);
    assert_eq!(
        record.send_started_ticks_provenance,
        TimestampProvenance::DerivedFromPreCall
    );
    assert_eq!(record.hold_floor_mask, 1);
    assert_eq!(record.release_floor_mask, 2);
    assert_eq!(record.pre_call_qpc_ticks, 120);
    assert!(record.pre_call_qpc_available);
    assert_eq!(record.sendinput_completion_qpc_ticks, 125);
    assert!(record.sendinput_completion_qpc_available);
    assert!(!record.observation_qpc_available);
    assert_eq!(record.core_post_send_duration_us, 4);

    let mut summary = super::NativeTelemetrySummary::default();
    summary.observe(&record);
    assert_eq!(summary.requested_key_count, 3);
    assert_eq!(summary.sent_key_count, 2);
    assert_eq!(summary.skipped_key_count, 1);
    assert_eq!(summary.authored_lateness_samples, 1);
    assert_eq!(summary.max_authored_lateness_ticks, 20);
    assert_eq!(summary.overdue_packet_count, 1);
    assert_eq!(summary.floor_displacement_samples, 1);
    assert_eq!(summary.max_physical_floor_displacement_ticks, 5);
    assert_eq!(summary.residual_lateness_samples, 1);
    assert_eq!(summary.max_residual_lateness_ticks, 15);
    assert_eq!(summary.hol_delay_samples, 1);
    assert_eq!(summary.max_hol_delay_lower_bound_ticks, 5);
    assert_eq!(summary.unattributed_lateness_samples, 1);
    assert_eq!(summary.max_unattributed_lateness_ticks, 10);
    assert_eq!(summary.sender_envelope_samples, 1);
    assert_eq!(summary.max_sendinput_envelope_ticks, 5);
    assert_eq!(summary.transport_anomaly_count, 1);
}
