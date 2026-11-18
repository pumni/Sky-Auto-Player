use super::support::{
    PacketRetryReason, ProductionDispatchTestHarness, SendEvidence, SendTransactionOutcome,
    SendTransactionStatus, TimelineTicks,
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
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
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
        harness.dispatch_staged_candidate_for_test(candidate, false),
        super::super::StagedDispatchOutcome::Dispatched
    ));
    let record = harness
        .runtime
        .attempted_dispatch_anomalies
        .details()
        .first()
        .expect("recovered sender anomaly record")
        .clone();
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
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
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
        harness.dispatch_staged_candidate_for_test(candidate, false),
        super::super::StagedDispatchOutcome::Exit
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
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
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
        harness.dispatch_staged_candidate_for_test(candidate, false),
        super::super::StagedDispatchOutcome::Exit
    ));
    assert_eq!(
        harness.runtime.terminal_trigger,
        Some(crate::engine::worker::TerminalTrigger::SystemSuspend)
    );
    assert_eq!(harness.local_metrics.precision_packet_attempts, 1);
    assert_eq!(harness.local_metrics.sendinput_envelope_samples, 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);

    let evidence = harness.finalize_for_test();
    assert_eq!(evidence.release_obligation_mask, 0);
    assert!(!cleanup.lock().expect("terminal cleanup capture").is_empty());
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
            .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
            .expect("stage candidate")
            .expect("Down candidate");
        let packet_mask = candidate.packet_masks.down_mask;
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
            harness.dispatch_staged_candidate_for_test(candidate, false),
            super::super::StagedDispatchOutcome::Dispatched
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
                .attempted_dispatch_anomalies
                .details()
                .first()
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
            .stage_candidate_for_test(
                0,
                harness.current_executor_chord_for_test(),
                TimelineTicks::ZERO,
            )
            .expect("stage production first Down")
            .expect("first Down candidate");
        harness.set_mock_sender_start_qpc_for_test(first_now);
        assert!(matches!(
            harness.dispatch_staged_candidate_for_test(first, false),
            super::super::StagedDispatchOutcome::Dispatched
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
            Option<sky_dispatch_core::executor::DownChordMetadata>,
            sky_dispatch_core::executor::DispatchTimingWindow,
        ) = match harness
            .evaluate_live_scheduler_for_test(mixed_target, epoch)
            .expect("live scheduler mixed evaluation")
        {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
            } => (up_mask, down_chord.map(Into::into), timing_window),
            decision => panic!("expected production mixed Up+Down, got {decision:?}"),
        };
        assert_eq!(up_mask, 0b01);
        assert_eq!(down_chord.as_ref().map(|chord| chord.down_mask), Some(0b10));
        let candidate = harness
            .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
            .expect("stage production mixed packet")
            .expect("mixed packet candidate");
        assert_eq!(candidate.packet_masks.up_mask, 0b01);
        assert_eq!(candidate.packet_masks.down_mask, 0b10);

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
            harness.dispatch_staged_candidate_for_test(candidate, false),
            super::super::StagedDispatchOutcome::Dispatched
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
            .attempted_dispatch_anomalies
            .details()
            .first()
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
            anomaly.contains("source_action=2 source_chord=1 up_mask=0x0001 down_mask=0x0002"),
            "the attempted mixed B onset is identified exactly: {anomaly}"
        );
        let _ = harness.finalize_for_test();
    }
}

#[test]
fn invalidated_session_target_does_not_send_origin_up() {
    use sky_dispatch_core::executor::DispatchDecision;
    use sky_dispatch_core::model::{ActionKind, KeyActionInput};

    let actions = [
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: vec![0x15].into(),
            reason: "initial-down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Down,
            scheduled_us: 19_000,
            scan_codes: vec![0x16].into(),
            reason: "earlier-mixed-down".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: 20_000,
            scan_codes: vec![0x15].into(),
            reason: "mixed-up".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 39_000,
            scan_codes: vec![0x16].into(),
            reason: "later-up".into(),
        },
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    harness
        .attach_executor_for_test()
        .expect("attach live scheduler");
    let epoch = harness.resources.playback.epoch;

    let first_now = harness.qpc_now_for_test().expect("first Down QPC");
    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage first Down")
        .expect("first Down candidate");
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        super::super::StagedDispatchOutcome::Dispatched
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
        Option<sky_dispatch_core::executor::DownChordMetadata>,
        sky_dispatch_core::executor::DispatchTimingWindow,
    ) = match harness
        .evaluate_live_scheduler_for_test(mixed_target, epoch)
        .expect("live mixed scheduler evaluation")
    {
        DispatchDecision::Send {
            up_mask,
            down_chord,
            timing_window,
        } => (up_mask, down_chord.map(Into::into), timing_window),
        decision => panic!("expected mixed Up+Down candidate, got {decision:?}"),
    };
    assert_eq!(up_mask, 0b01);
    let mixed = harness
        .stage_candidate_with_scheduler_window_and_observer_for_test(
            up_mask,
            down_chord,
            timing_window,
            true,
        )
        .expect("stage mixed candidate")
        .expect("mixed candidate");
    assert_eq!(mixed.packet_masks.down_mask, 0b10);
    let accounting_before_replan = harness.generation_accounting_for_test();

    harness.invalidate_focus_owner_for_test(2, 202);
    harness.set_mock_sender_start_qpc_for_test(
        harness.qpc_now_for_test().expect("fallback Up sender QPC"),
    );
    assert_ne!(
        harness.dispatch_staged_candidate_for_test(mixed, true),
        super::super::StagedDispatchOutcome::Dispatched
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
