use super::support::{
    DispatchOutcome, PacketRetryReason, PreparedChord, ProductionDispatchTestHarness, SendEvidence,
    SendTransactionOutcome, SendTransactionStatus, TimelineTicks, test_chord,
};

fn sender_failure_record_for_test(
    key_count: usize,
    status: SendTransactionStatus,
    inserted_count: u8,
) -> String {
    let mut harness = if key_count == 1 {
        ProductionDispatchTestHarness::new_down_only()
    } else {
        ProductionDispatchTestHarness::new_down_chord(key_count)
    };
    let _cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .prepare_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("Down candidate");
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("sender start QPC");
        let completed = (status != SendTransactionStatus::ClockFailureAfterSend).then_some(started);
        let confirmed_mask = if inserted_count == 0 {
            0
        } else {
            packet.down_mask & packet.down_mask.wrapping_neg()
        };
        SendTransactionOutcome {
            status,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask,
                skipped_mask: 0,
                first_inserted: inserted_count,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: match status {
                    SendTransactionStatus::ZeroProgress => PacketRetryReason::ZeroProgress,
                    SendTransactionStatus::IntegrityLost => {
                        PacketRetryReason::PartialProgress { inserted_count }
                    }
                    _ => PacketRetryReason::None,
                },
                first_win32_error: Some(5),
                last_win32_error: Some(5),
                started_ticks: Some(started),
                completed_ticks: completed,
                timing_error: None,
            },
        }
    });

    assert!(matches!(
        harness.dispatch_candidate_for_test(candidate, false),
        super::super::DispatchOutcome::Exit
    ));
    let record = harness
        .runtime
        .attempted_dispatch_anomaly
        .detail()
        .expect("recovered sender anomaly record")
        .to_owned();
    let cleanup = harness.finalize_for_test();
    assert_eq!(cleanup.release_obligation_mask, 0);
    record
}

#[test]
fn failed_sender_paths_retain_bounded_attempt_and_clock_evidence() {
    let zero = sender_failure_record_for_test(1, SendTransactionStatus::ZeroProgress, 0);
    assert!(
        zero.contains("requested=1 confirmed_count=0 first_inserted=0")
            && zero.contains("attempts=1 status=ZeroProgress"),
        "{zero}"
    );
    assert!(zero.contains("retry_reason=ZeroProgress win32_error=Some(5)"));
    assert!(zero.contains("pre_call_qpc=") && zero.contains("completion_qpc="));

    let partial = sender_failure_record_for_test(2, SendTransactionStatus::IntegrityLost, 1);
    assert!(partial.contains("source_action=0 source_chord=0 up_mask=0x0000 down_mask=0x0003"));
    assert!(partial.contains("requested=2 confirmed_count=1 first_inserted=1"));
    assert!(partial.contains("attempts=1 status=IntegrityLost"));
    assert!(partial.contains("retry_reason=PartialProgress { inserted_count: 1 }"));

    let clock_failure =
        sender_failure_record_for_test(1, SendTransactionStatus::ClockFailureAfterSend, 1);
    assert!(clock_failure.contains("status=ClockFailureAfterSend"));
    assert!(clock_failure.contains("completion_qpc=unavailable"));
}

#[test]
fn pre_call_qpc_failure_records_zero_physical_attempts() {
    use sky_dispatch_win32::clock::QpcError;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .prepare_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("Down candidate");
    harness
        .resources
        .backend
        .set_packet_emitter(|packet| SendTransactionOutcome {
            status: SendTransactionStatus::ClockFailureBeforeSend,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: 0,
                attempts: 0,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: None,
                completed_ticks: None,
                timing_error: Some(QpcError::CounterUnavailable),
            },
        });

    assert!(matches!(
        harness.dispatch_candidate_for_test(candidate, false),
        super::super::DispatchOutcome::Exit
    ));
    assert_eq!(harness.local_metrics.precision_packet_attempts, 0);
    assert_eq!(harness.local_metrics.pre_call_qpc_unavailable_attempts, 0);
    assert_eq!(
        harness
            .local_metrics
            .sendinput_completion_qpc_unavailable_attempts,
        0
    );
    assert_eq!(harness.local_metrics.transport_anomalies, 1);
    assert!(
        harness
            .runtime
            .terminal_error
            .as_deref()
            .is_some_and(|error| error.contains("ClockFailureBeforeSend"))
    );
    let _ = harness.finalize_for_test();
}

#[test]
fn suspend_after_sender_return_keeps_musical_attempt_evidence() {
    use std::sync::Arc;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .prepare_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("Down candidate");
    let clock = harness.resources.clock;
    let system_power = Arc::clone(&harness.system_power);
    let interrupt = Arc::clone(&harness.interrupt);
    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("sender start QPC");
        let completed = clock.now().expect("sender completion QPC");
        assert!(system_power.notify(true, &interrupt));
        SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: packet.up_mask | packet.down_mask,
                skipped_mask: 0,
                first_inserted: packet.event_count(),
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: Some(started),
                completed_ticks: Some(completed),
                timing_error: None,
            },
        }
    });

    assert!(matches!(
        harness.dispatch_candidate_for_test(candidate, false),
        super::super::DispatchOutcome::Exit
    ));
    assert_eq!(
        harness.runtime.terminal_trigger,
        Some(crate::engine::worker::TerminalTrigger::SystemSuspend)
    );
    assert_eq!(harness.local_metrics.precision_packet_attempts, 1);
    assert_eq!(harness.local_metrics.sendinput_envelope_samples, 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);

    let evidence = harness.finalize_for_test();
    assert_eq!(evidence.release_obligation_mask, 0);
    assert!(!cleanup.lock().expect("terminal cleanup capture").is_empty());
    assert_eq!(evidence.terminal_release_raw_up_syscall_count, 1);
}

#[test]
fn inconsistent_complete_evidence_fails_closed_before_scheduler_commit() {
    for (confirmed_mask, attempts, expected_evidence) in [
        (0b0001, 1, "confirmed_count=0 first_inserted=2 attempts=1"),
        (0b0011, 2, "confirmed_count=0 first_inserted=2 attempts=2"),
    ] {
        let mut harness = ProductionDispatchTestHarness::new_down_chord(2);
        let _cleanup = harness.configure_mock_cleanup_emitter_for_test();
        harness
            .attach_executor_for_test()
            .expect("attach scheduler");
        let chord = harness.current_executor_chord_for_test();
        let candidate = harness
            .prepare_candidate_for_test(0, chord, TimelineTicks::ZERO)
            .expect("stage candidate")
            .expect("Down candidate");
        let packet_mask = candidate.packet_masks().down_mask;
        let accounting_before = harness.resources.scheduler.accounting();
        let clock = harness.resources.clock;
        harness.resources.backend.set_packet_emitter(move |packet| {
            let started = clock.now().expect("sender start QPC");
            SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask: packet.up_mask | packet.down_mask,
                    confirmed_mask,
                    skipped_mask: 0,
                    first_inserted: packet.event_count(),
                    attempts,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(started),
                    completed_ticks: Some(started),
                    timing_error: None,
                },
            }
        });

        assert!(matches!(
            harness.dispatch_candidate_for_test(candidate, false),
            super::super::DispatchOutcome::Exit
        ));
        assert_ne!(harness.resources.scheduler.accounting(), accounting_before);
        assert_eq!(harness.resources.scheduler.accounting().activated, 0);
        assert_eq!(harness.resources.scheduler.accounting().cancelled, 2);
        assert_eq!(
            harness
                .current_executor_chord_for_test()
                .map(|chord| chord.chord_index),
            None
        );
        assert_eq!(harness.resources.backend.in_flight_mask, 0);
        assert_eq!(
            harness.resources.backend.release_obligation_mask() & packet_mask,
            0
        );
        assert!(
            harness
                .runtime
                .attempted_dispatch_anomaly
                .detail()
                .is_some_and(|record| {
                    record.contains("status=IntegrityLost") && record.contains(expected_evidence)
                })
        );
        let _ = harness.finalize_for_test();
    }
}

#[test]
fn mixed_post_call_qpc_failure_consumes_attempt_after_verified_cleanup_without_replay() {
    use sky_dispatch_core::executor::DispatchDecision;
    use sky_dispatch_win32::clock::QpcError;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    for inserted_count in 0..=2_u8 {
        let mut harness = ProductionDispatchTestHarness::new_mixed();
        let packets = harness.configure_mock_packet_sender_for_test(&[0]);
        let _cleanup = harness.configure_mock_cleanup_emitter_for_test();
        harness
            .attach_executor_for_test()
            .expect("attach live scheduler");
        let epoch = harness.resources.playback.epoch;
        let first_now = harness.qpc_now_for_test().expect("first Down QPC");
        let first = harness
            .prepare_candidate_for_test(
                0,
                harness.current_executor_chord_for_test(),
                TimelineTicks::ZERO,
            )
            .expect("stage production first Down")
            .expect("first Down candidate");
        harness.set_mock_sender_start_qpc_for_test(first_now);
        assert!(matches!(
            harness.dispatch_candidate_for_test(first, false),
            super::super::DispatchOutcome::Dispatched
        ));
        assert_eq!(
            harness.generation_accounting_for_test().activated,
            1,
            "first Down is committed before the mixed packet"
        );
        assert_eq!(harness.release_obligation_mask_for_test(), 0b01);

        let mixed_target = epoch
            .checked_add_duration(
                harness
                    .qpc_duration_from_us_for_test(20_000)
                    .expect("authored mixed target"),
            )
            .expect("mixed target QPC");
        while harness.qpc_now_for_test().expect("wait for mixed target") < mixed_target {
            std::thread::yield_now();
        }
        let (up_mask, down_chord, timing_window): (
            u16,
            Option<PreparedChord>,
            sky_dispatch_core::executor::DispatchTimingWindow,
        ) = match harness
            .evaluate_live_scheduler_for_test(mixed_target, epoch)
            .expect("live scheduler mixed evaluation")
        {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
                ..
            } => (up_mask, down_chord, timing_window),
            decision => panic!("expected production mixed Up+Down, got {decision:?}"),
        };
        assert_eq!(up_mask, 0b01);
        assert_eq!(down_chord.as_ref().map(|chord| chord.key_mask), Some(0b10));
        let candidate = harness
            .prepare_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
            .expect("stage production mixed packet")
            .expect("mixed packet candidate");
        assert_eq!(candidate.packet_masks().up_mask, 0b01);
        assert_eq!(candidate.packet_masks().down_mask, 0b10);

        let attempts = Arc::new(AtomicUsize::new(0));
        let observed_attempts = Arc::clone(&attempts);
        let clock = harness.resources.clock;
        harness.resources.backend.set_packet_emitter(move |packet| {
            observed_attempts.fetch_add(1, Ordering::Relaxed);
            let started = clock.now().expect("mixed sender start QPC");
            let confirmed_mask = match inserted_count {
                0 => 0,
                1 => packet.up_mask,
                _ => packet.up_mask | packet.down_mask,
            };
            SendTransactionOutcome {
                status: SendTransactionStatus::ClockFailureAfterSend,
                evidence: SendEvidence {
                    requested_mask: packet.up_mask | packet.down_mask,
                    confirmed_mask,
                    skipped_mask: 0,
                    first_inserted: inserted_count,
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: Some(0),
                    last_win32_error: Some(0),
                    started_ticks: Some(started),
                    completed_ticks: None,
                    timing_error: Some(QpcError::CounterUnavailable),
                },
            }
        });

        let accounting_before = harness.generation_accounting_for_test();
        harness.set_mock_sender_start_qpc_for_test(mixed_target);
        assert!(matches!(
            harness.dispatch_candidate_for_test(candidate, false),
            super::super::DispatchOutcome::Exit
        ));
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        assert_eq!(harness.local_metrics.precision_packet_attempts, 2);
        assert_eq!(
            harness
                .local_metrics
                .sendinput_completion_qpc_unavailable_attempts,
            1
        );
        assert_eq!(harness.local_metrics.transport_anomalies, 1);
        let accounting_after = harness.generation_accounting_for_test();
        assert_eq!(accounting_after.total, accounting_before.total);
        assert_eq!(accounting_after.activated, 1);
        assert_eq!(accounting_after.cancelled, 2);
        assert_eq!(accounting_after.active, 0);
        assert_eq!(
            harness
                .current_executor_chord_for_test()
                .map(|chord| chord.chord_index),
            None,
            "verified cleanup consumes the syscall-attempted B onset exactly once"
        );
        assert_eq!(harness.release_obligation_mask_for_test(), 0);
        assert_eq!(harness.resources.backend.in_flight_mask, 0);
        assert_eq!(
            harness.resources.backend.sendinput_partial_events,
            u64::from(inserted_count == 1)
        );
        assert_eq!(harness.runtime.last_physical_send_completion_qpc, None);
        let anomaly = harness
            .runtime
            .attempted_dispatch_anomaly
            .detail()
            .expect("post-call QPC failure record");
        assert!(
            anomaly.contains("status=ClockFailureAfterSend"),
            "{anomaly}"
        );
        assert!(anomaly.contains("first_inserted="), "{anomaly}");
        assert!(anomaly.contains("completion_qpc=unavailable"), "{anomaly}");
        assert_eq!(
            packets.lock().expect("initial packet capture").len(),
            1,
            "the original A onset had exactly one healthy packet"
        );
        assert!(
            anomaly.contains("source_action=1 source_chord=1 up_mask=0x0001 down_mask=0x0002"),
            "the attempted mixed B onset is identified exactly: {anomaly}"
        );
        let _ = harness.finalize_for_test();
    }
}

#[test]
fn invalidated_session_target_does_not_send_origin_up() {
    use sky_dispatch_core::executor::DispatchDecision;

    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
            sky_dispatch_win32::focus::set_window_live_for_test(None);
        }
    }
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;

    let chords = [test_chord(0, 0, 0x15), test_chord(1, 19_000, 0x16)];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    harness
        .attach_executor_for_test()
        .expect("attach live scheduler");
    let epoch = harness.resources.playback.epoch;

    let first_now = harness.qpc_now_for_test().expect("first Down QPC");
    let first = harness
        .prepare_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage first Down")
        .expect("first Down candidate");
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_candidate_for_test(first, false),
        super::super::DispatchOutcome::Dispatched
    ));

    let mixed_target = epoch
        .checked_add_duration(
            harness
                .qpc_duration_from_us_for_test(20_000)
                .expect("mixed target conversion"),
        )
        .expect("mixed target QPC");
    while harness.qpc_now_for_test().expect("wait for mixed target") < mixed_target {
        std::thread::yield_now();
    }
    let (up_mask, down_chord, timing_window): (
        u16,
        Option<PreparedChord>,
        sky_dispatch_core::executor::DispatchTimingWindow,
    ) = match harness
        .evaluate_live_scheduler_for_test(mixed_target, epoch)
        .expect("live mixed scheduler evaluation")
    {
        DispatchDecision::Send {
            up_mask,
            down_chord,
            timing_window,
            ..
        } => (up_mask, down_chord, timing_window),
        decision => panic!("expected mixed Up+Down candidate, got {decision:?}"),
    };
    assert_eq!(up_mask, 0b01);
    let mixed = harness
        .prepare_candidate_with_scheduler_window_and_observer_for_test(
            up_mask,
            down_chord,
            timing_window,
            true,
        )
        .expect("stage mixed candidate")
        .expect("mixed candidate");
    assert_eq!(mixed.packet_masks().down_mask, 0b10);
    let accounting_before_replan = harness.generation_accounting_for_test();

    harness.invalidate_focus_owner_for_test(2, 202);
    harness.set_mock_sender_start_qpc_for_test(
        harness.qpc_now_for_test().expect("fallback Up sender QPC"),
    );
    assert_ne!(
        harness.dispatch_candidate_for_test(mixed, true),
        super::super::DispatchOutcome::Dispatched
    );
    assert!(
        harness.observer.pop_front().is_none(),
        "no Up observation without a sender call"
    );

    let captured = packets.lock().expect("mock packet capture");
    assert_eq!(
        captured.len(),
        1,
        "invalidated target sends no Up through a replacement window"
    );
    assert_eq!(
        captured[0].0,
        sky_dispatch_win32::input::PhysicalPacket::new(0, 1)
    );
    assert_eq!(harness.generation_accounting_for_test().activated, 1);
    assert_eq!(harness.generation_accounting_for_test().released, 0);
    assert_eq!(harness.generation_accounting_for_test().active, 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 0b01);
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|chord| chord.chord_index),
        Some(1),
        "replanning Up-only cannot advance the rejected Down cursor"
    );
    assert_eq!(accounting_before_replan.activated, 1);
    assert_eq!(accounting_before_replan.released, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
}

#[test]
fn unstable_target_retry_wait_is_interruptible_before_deadline() {
    use sky_dispatch_win32::clock::QpcClock;
    use sky_dispatch_win32::event::OwnedEvent;
    use sky_dispatch_win32::wait::HybridWaiter;

    let qpc = QpcClock::initialize().expect("retry wait QPC");
    let now = qpc.now().expect("retry wait start");
    let deadline = now
        .checked_add_duration(qpc.duration_from_us(5_000_000).expect("retry deadline"))
        .expect("retry deadline arithmetic");
    let waiter = HybridWaiter::new();
    let interrupt = OwnedEvent::new_auto_reset().expect("retry interrupt event");
    assert!(interrupt.signal(), "pre-wait target/control event signal");
    let mut metrics = crate::engine::telemetry::WorkerMetricsLocal::default();
    let mut terminal_error = None;

    let result = super::super::wait_after_unstable_target(
        now,
        deadline,
        qpc,
        &waiter,
        &interrupt,
        &mut metrics,
        &mut terminal_error,
    );

    assert!(matches!(
        result,
        super::super::WaitBoundary::Interrupted(..)
    ));
    assert!(terminal_error.is_none());
}

#[test]
fn suspend_after_sender_return_up_only_commits_without_phantom_up() {
    use std::sync::Arc;

    let chords = [test_chord(0, 0, 0x15)];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let epoch = harness.resources.playback.epoch;

    let first = harness
        .prepare_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage first Down")
        .expect("Down candidate");
    assert_eq!(
        harness.dispatch_candidate_for_test(first, false),
        DispatchOutcome::Dispatched
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 1);

    let up_target = epoch
        .checked_add_duration(
            harness
                .qpc_duration_from_us_for_test(20_000)
                .expect("duration conversion"),
        )
        .expect("up target QPC");
    while harness.qpc_now_for_test().expect("now") < up_target {
        std::thread::yield_now();
    }
    let (up_mask, down_chord, timing_window) =
        super::support::send_decision_at(&mut harness, up_target, epoch);
    assert_eq!(up_mask, 1);
    assert!(down_chord.is_none());
    let up_candidate = harness
        .prepare_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage Up")
        .expect("Up candidate");

    let clock = harness.resources.clock;
    let system_power = Arc::clone(&harness.system_power);
    let interrupt = Arc::clone(&harness.interrupt);
    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("sender start QPC");
        let completed = clock.now().expect("sender completion QPC");
        assert!(system_power.notify(true, &interrupt));
        SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: packet.up_mask | packet.down_mask,
                skipped_mask: 0,
                first_inserted: packet.event_count(),
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: Some(started),
                completed_ticks: Some(completed),
                timing_error: None,
            },
        }
    });

    assert_eq!(
        harness.dispatch_candidate_for_test(up_candidate, false),
        DispatchOutcome::Exit
    );
    assert_eq!(
        harness.runtime.terminal_trigger,
        Some(crate::engine::worker::TerminalTrigger::SystemSuspend)
    );
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.generation_accounting_for_test().activated, 1);
    assert_eq!(harness.generation_accounting_for_test().released, 1);
    assert_eq!(harness.generation_accounting_for_test().active, 0);

    let evidence = harness.finalize_for_test();
    assert_eq!(evidence.release_obligation_mask, 0);
    assert!(
        cleanup.lock().expect("terminal cleanup capture").is_empty(),
        "no phantom Up and no unexpected extra terminal raw Up on already completed Up"
    );
    assert_eq!(evidence.terminal_release_raw_up_syscall_count, 0);
}

#[test]
fn suspend_after_sender_return_mixed_commits_and_preserves_remaining_down_safety_release() {
    use std::sync::Arc;

    let chords = [test_chord(0, 0, 0x15), test_chord(2, 20_000, 0x16)];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let epoch = harness.resources.playback.epoch;

    let first = harness
        .prepare_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage first Down")
        .expect("Down candidate");
    assert_eq!(
        harness.dispatch_candidate_for_test(first, false),
        DispatchOutcome::Dispatched
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 0b01);

    let mixed_target = epoch
        .checked_add_duration(
            harness
                .qpc_duration_from_us_for_test(20_000)
                .expect("duration conversion"),
        )
        .expect("mixed target QPC");
    while harness.qpc_now_for_test().expect("now") < mixed_target {
        std::thread::yield_now();
    }
    let (up_mask, down_chord, timing_window) =
        super::support::send_decision_at(&mut harness, mixed_target, epoch);
    assert_eq!(up_mask, 0b01);
    assert!(down_chord.is_some());
    let mixed_candidate = harness
        .prepare_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage Mixed")
        .expect("Mixed candidate");
    assert_eq!(mixed_candidate.packet_masks().up_mask, 0b01);
    assert_eq!(mixed_candidate.packet_masks().down_mask, 0b10);

    let clock = harness.resources.clock;
    let system_power = Arc::clone(&harness.system_power);
    let interrupt = Arc::clone(&harness.interrupt);
    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("sender start QPC");
        let completed = clock.now().expect("sender completion QPC");
        assert!(system_power.notify(true, &interrupt));
        SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: packet.up_mask | packet.down_mask,
                skipped_mask: 0,
                first_inserted: packet.event_count(),
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: Some(started),
                completed_ticks: Some(completed),
                timing_error: None,
            },
        }
    });

    assert_eq!(
        harness.dispatch_candidate_for_test(mixed_candidate, false),
        DispatchOutcome::Exit
    );
    assert_eq!(
        harness.runtime.terminal_trigger,
        Some(crate::engine::worker::TerminalTrigger::SystemSuspend)
    );
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0b10);
    assert_eq!(harness.generation_accounting_for_test().activated, 2);
    assert_eq!(harness.generation_accounting_for_test().released, 1);
    assert_eq!(harness.generation_accounting_for_test().active, 1);

    let evidence = harness.finalize_for_test();
    assert_eq!(evidence.release_obligation_mask, 0);
    let captured = cleanup.lock().expect("terminal cleanup capture");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].0, vec![0x16]);
    assert!(captured[0].1);
    assert_eq!(evidence.terminal_release_raw_up_syscall_count, 1);
}

#[test]
fn suspend_with_malformed_qpc_fails_closed_without_speculative_commit() {
    use std::sync::Arc;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let _cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .prepare_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("Down candidate");
    let clock = harness.resources.clock;
    let system_power = Arc::clone(&harness.system_power);
    let interrupt = Arc::clone(&harness.interrupt);
    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("sender start QPC");
        assert!(system_power.notify(true, &interrupt));
        SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: packet.up_mask | packet.down_mask,
                skipped_mask: 0,
                first_inserted: packet.event_count(),
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: Some(started),
                completed_ticks: None,
                timing_error: None,
            },
        }
    });

    assert_eq!(
        harness.dispatch_candidate_for_test(candidate, false),
        DispatchOutcome::Exit
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.generation_accounting_for_test().activated, 0);
    assert_eq!(
        harness.runtime.terminal_trigger,
        Some(crate::engine::worker::TerminalTrigger::SenderAnomaly)
    );
}

#[test]
fn terminal_budget_foreground_retry_exhaustion_capped_to_four_raw_calls() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let raw_up_calls = Arc::new(AtomicUsize::new(0));
    let raw_up_calls_for_emitter = Arc::clone(&raw_up_calls);
    let clock = harness.resources.clock;
    harness
        .resources
        .backend
        .set_emitter(move |scan_codes, key_up| {
            if key_up {
                raw_up_calls_for_emitter.fetch_add(1, Ordering::SeqCst);
                let now = clock.now().expect("cleanup QPC");
                sky_dispatch_win32::input::PlatformSendResult {
                    requested: scan_codes.len() as u8,
                    inserted: 0,
                    started_ticks: now,
                    completed_ticks: Some(now),
                    win32_error: 5,
                    timing_error: None,
                }
            } else {
                let now = clock.now().expect("cleanup QPC");
                sky_dispatch_win32::input::PlatformSendResult {
                    requested: scan_codes.len() as u8,
                    inserted: scan_codes.len() as u8,
                    started_ticks: now,
                    completed_ticks: Some(now),
                    win32_error: 0,
                    timing_error: None,
                }
            }
        });
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .prepare_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("Down candidate");

    harness.resources.backend.set_probe(|_, _| {
        sky_dispatch_win32::input::InstrumentPhysicalState::Held(smallvec::smallvec![0x15])
    });

    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("sender start QPC");
        let completed = clock.now().expect("sender completion QPC");
        SendTransactionOutcome {
            status: SendTransactionStatus::IntegrityLost,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: 0,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::PartialProgress { inserted_count: 0 },
                first_win32_error: Some(5),
                last_win32_error: Some(5),
                started_ticks: Some(started),
                completed_ticks: Some(completed),
                timing_error: None,
            },
        }
    });

    let outcome = harness.dispatch_candidate_for_test(candidate, false);
    assert_eq!(outcome, DispatchOutcome::Exit);
    assert_eq!(
        raw_up_calls.load(Ordering::Acquire),
        4,
        "in-loop cleanup exhausts exactly 4 foreground retries"
    );

    let mut observation = crate::engine::worker::FinalizeTestObservation::default();
    let outcome_code = harness.finalize_worker_for_test(false, &mut observation);
    assert_eq!(outcome_code, crate::engine::OUTCOME_ERROR);
    assert_eq!(
        raw_up_calls.load(Ordering::Acquire),
        4,
        "finalize_worker must make 0 additional raw calls since 4-call budget was exhausted"
    );
    assert_eq!(observation.terminal_release_raw_up_syscall_count, 4);
    assert_ne!(
        observation.release_obligation_mask, 0,
        "unresolved journal must be quarantined"
    );
}

#[test]
fn terminal_budget_background_drain_capped_to_one_raw_call() {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    let _guard = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_focus_owner_for_test(1, 125);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .prepare_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("Down candidate");

    let clock = harness.resources.clock;
    let focus = Arc::clone(&harness.focus_active);
    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("sender start QPC");
        let completed = clock.now().expect("sender completion QPC");
        focus.store(false, Ordering::Release);
        sky_dispatch_win32::focus::set_foreground_window_for_test(Some(2));
        sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));
        SendTransactionOutcome {
            status: SendTransactionStatus::IntegrityLost,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: 0,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::PartialProgress { inserted_count: 0 },
                first_win32_error: Some(5),
                last_win32_error: Some(5),
                started_ticks: Some(started),
                completed_ticks: Some(completed),
                timing_error: None,
            },
        }
    });

    let outcome = harness.dispatch_candidate_for_test(candidate, false);
    assert_eq!(outcome, DispatchOutcome::Exit);
    assert_eq!(
        cleanup.lock().expect("cleanup lock").len(),
        1,
        "background terminal drain sends exactly 1 raw Up in-loop"
    );

    let mut observation = crate::engine::worker::FinalizeTestObservation::default();
    let outcome_code = harness.finalize_worker_for_test(false, &mut observation);
    assert_eq!(outcome_code, crate::engine::OUTCOME_ERROR);
    assert_eq!(
        cleanup.lock().expect("cleanup lock").len(),
        1,
        "finalize_worker must make 0 additional background raw calls; total background <= 1"
    );
    assert_eq!(observation.terminal_release_raw_up_syscall_count, 1);
    assert_ne!(
        observation.release_obligation_mask, 0,
        "background Up retains residual in failed_release_mask"
    );
    sky_dispatch_win32::focus::set_foreground_window_for_test(None);
    sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
}

#[test]
fn terminal_budget_successful_single_cleanup_matches_aggregate_raw_count() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .prepare_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("Down candidate");

    harness
        .resources
        .backend
        .set_probe(|_, _| sky_dispatch_win32::input::InstrumentPhysicalState::AllUp);

    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("sender start QPC");
        let completed = clock.now().expect("sender completion QPC");
        SendTransactionOutcome {
            status: SendTransactionStatus::IntegrityLost,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: 0,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::PartialProgress { inserted_count: 0 },
                first_win32_error: Some(5),
                last_win32_error: Some(5),
                started_ticks: Some(started),
                completed_ticks: Some(completed),
                timing_error: None,
            },
        }
    });

    let outcome = harness.dispatch_candidate_for_test(candidate, false);
    assert_eq!(outcome, DispatchOutcome::Exit);
    assert_eq!(cleanup.lock().expect("cleanup lock").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);

    let mut observation = crate::engine::worker::FinalizeTestObservation::default();
    let _ = harness.finalize_worker_for_test(false, &mut observation);
    assert_eq!(cleanup.lock().expect("cleanup lock").len(), 1);
    assert_eq!(observation.terminal_release_raw_up_syscall_count, 1);
    assert_eq!(observation.release_obligation_mask, 0);
}

#[test]
fn terminal_budget_proven_raw_zero_down_has_zero_terminal_raw_calls() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .prepare_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("Down candidate");

    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("sender start QPC");
        let completed = clock.now().expect("sender completion QPC");
        SendTransactionOutcome {
            status: SendTransactionStatus::ZeroProgress,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: 0,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::ZeroProgress,
                first_win32_error: Some(5),
                last_win32_error: Some(5),
                started_ticks: Some(started),
                completed_ticks: Some(completed),
                timing_error: None,
            },
        }
    });

    let outcome = harness.dispatch_candidate_for_test(candidate, false);
    assert_eq!(outcome, DispatchOutcome::Exit);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);

    let mut observation = crate::engine::worker::FinalizeTestObservation::default();
    let _ = harness.finalize_worker_for_test(false, &mut observation);
    assert!(cleanup.lock().expect("cleanup").is_empty());
    assert_eq!(observation.terminal_release_raw_up_syscall_count, 0);
    assert_eq!(observation.release_obligation_mask, 0);
}

#[test]
fn manual_resume_background_rechecks_focus_before_preflight() {
    use std::sync::atomic::Ordering;

    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.attach_executor_for_test().expect("attach executor");

    harness.focus_active.store(false, Ordering::Release);

    let stamp = super::super::load_target_stamp(&harness.target).expect("stamp");
    let focus_match = super::super::focus_matches_hwnd(true, &harness.focus_active, stamp.hwnd);
    assert!(
        !focus_match,
        "target in background must fail focus_matches_hwnd"
    );
}
