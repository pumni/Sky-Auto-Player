#[derive(Clone, Copy)]
enum UpOnlyQpcFault {
    ReversedCompletion,
    MissingCompletion,
}

fn target_at(
    harness: &ProductionDispatchTestHarness,
    epoch: QpcTicks,
    at_us: u64,
) -> QpcTicks {
    epoch
        .checked_add_duration(DurationTicks::from_raw(qpc_offset(harness, at_us).as_u64()))
        .expect("authored QPC target")
}

fn candidate_at(
    harness: &mut ProductionDispatchTestHarness,
    at: QpcTicks,
    epoch: QpcTicks,
) -> crate::engine::worker::StagedCandidate {
    let (up_mask, down_chord, timing_window) = staged_send_at(harness, at, epoch);
    harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage production candidate")
        .expect("candidate is due")
}

fn dispatch_with_source(
    harness: &mut ProductionDispatchTestHarness,
    candidate: crate::engine::worker::StagedCandidate,
    expected_source_action: &AtomicU32,
) {
    expected_source_action.store(candidate.source_action_index, Ordering::Release);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Dispatched
    ));
}

fn assert_up_only_anomaly_recovers_and_latches_terminal_error(fault: UpOnlyQpcFault) {
    let actions = [
        test_action(10, ActionKind::Down, 0, 0x15),
        test_action(11, ActionKind::Up, 20_000, 0x15),
        test_action(12, ActionKind::Down, 70_000, 0x16),
        test_action(13, ActionKind::Up, 90_000, 0x16),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness.reanchor_playback_clock_for_test(epoch).expect("reanchor");
    let (cleanup_capture, cleanup_completion_qpc) =
        harness.configure_mock_cleanup_emitter_with_completion_qpc_for_test();
    let source = Arc::new(AtomicU32::new(u32::MAX));
    let captured_source = Arc::clone(&source);
    let packet_attempts = Arc::new(Mutex::new(Vec::with_capacity(4)));
    let captured_attempts = Arc::clone(&packet_attempts);
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        let source_action = captured_source.load(Ordering::Acquire);
        captured_attempts
            .lock()
            .expect("packet attempt log")
            .push((source_action, packet));
        let started = clock.now().expect("pre-call QPC");
        let completed = if source_action != 11 {
            Some(started)
        } else {
            match fault {
                UpOnlyQpcFault::ReversedCompletion => Some(QpcTicks::from_raw(
                    started.as_u64().checked_sub(1).expect("nonzero QPC"),
                )),
                UpOnlyQpcFault::MissingCompletion => None,
            }
        };
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
                completed_ticks: completed,
                timing_error: None,
            },
        }
    });
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    assert_eq!(first.source_action_index, 10);
    dispatch_with_source(&mut harness, first, source.as_ref());

    // The second packet is the full-count musical Up(A). Make its identity
    // visible to the deterministic mock before crossing the syscall seam.
    let up_a_target = target_at(&harness, epoch, 20_000);
    wait_until_qpc(&harness, up_a_target);
    let up_a = candidate_at(&mut harness, up_a_target, epoch);
    assert_eq!(up_a.source_action_index, 11);
    assert_eq!(up_a.packet_masks, PhysicalPacket::new(0b0001, 0));
    let qualified_before = (
        harness.local_metrics.authored_lateness_samples,
        harness.local_metrics.residual_lateness_samples,
        harness.local_metrics.hol_delay_lower_bound_samples,
        harness.local_metrics.sendinput_envelope_samples,
        harness.local_metrics.physical_floor_displacement_samples,
    );
    source.store(11, Ordering::Release);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(up_a, false),
        StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(
        (
            harness.local_metrics.authored_lateness_samples,
            harness.local_metrics.residual_lateness_samples,
            harness.local_metrics.hol_delay_lower_bound_samples,
            harness.local_metrics.sendinput_envelope_samples,
            harness.local_metrics.physical_floor_displacement_samples,
        ),
        qualified_before,
        "invalid QPC evidence contributes no qualified timing sample"
    );

    let anomaly = &harness.runtime.attempted_dispatch_anomalies.details()[0];
    assert!(anomaly.contains("source_action=11"), "{anomaly}");
    assert!(anomaly.contains("up_mask=0x0001 down_mask=0x0000"), "{anomaly}");
    assert!(
        anomaly.contains("first_inserted=1 attempts=1 status=Complete"),
        "{anomaly}"
    );
    assert!(anomaly.contains("timing_error=None"), "{anomaly}");
    if matches!(fault, UpOnlyQpcFault::ReversedCompletion) {
        let pre = anomaly
            .split("pre_call_qpc=")
            .nth(1)
            .and_then(|v| v.split_whitespace().next())
            .expect("pre-call QPC");
        let completion = anomaly
            .split("completion_qpc=")
            .nth(1)
            .and_then(|v| v.split_whitespace().next())
            .expect("completion QPC");
        assert!(
            pre.parse::<u64>().expect("pre tick")
                > completion.parse::<u64>().expect("completion tick"),
            "{anomaly}"
        );
    } else {
        assert!(anomaly.contains("completion_qpc=unavailable"), "{anomaly}");
    }
    assert!(!cleanup_capture
        .lock()
        .expect("verified safety cleanup")
        .is_empty());
    let cleanup_completion = QpcTicks::from_raw(cleanup_completion_qpc.load(Ordering::Acquire));
    let barrier = harness
        .resume_barrier_qpc_for_test()
        .expect("verified cleanup resume barrier");
    assert!(barrier >= cleanup_completion);

    let next_down_target = target_at(&harness, epoch, 70_000);
    let next_down_at = barrier.max(next_down_target);
    wait_until_qpc(&harness, next_down_at);
    let down_b = candidate_at(&mut harness, next_down_at, epoch);
    assert_eq!(down_b.source_action_index, 12);
    assert_eq!(down_b.packet_masks, PhysicalPacket::new(0, 0b0010));
    dispatch_with_source(&mut harness, down_b, source.as_ref());

    let up_b_target = target_at(&harness, epoch, 90_000);
    wait_until_qpc(&harness, up_b_target);
    let up_b = candidate_at(&mut harness, up_b_target, epoch);
    assert_eq!(up_b.source_action_index, 13);
    assert_eq!(up_b.packet_masks, PhysicalPacket::new(0b0010, 0));
    dispatch_with_source(&mut harness, up_b, source.as_ref());

    assert_eq!(
        packet_attempts.lock().expect("packet attempt log").as_slice(),
        &[
            (10, PhysicalPacket::new(0, 0b0001)),
            (11, PhysicalPacket::new(0b0001, 0)),
            (12, PhysicalPacket::new(0, 0b0010)),
            (13, PhysicalPacket::new(0b0010, 0)),
        ],
        "the anomalous Up has one syscall and the later Down is sent once"
    );

    let mut observation = crate::engine::worker::FinalizeTestObservation::default();
    let outcome = harness.finalize_worker_for_test(false, &mut observation);
    assert_eq!(outcome, crate::engine::OUTCOME_ERROR);
    assert!(observation
        .terminal_error
        .as_deref()
        .is_some_and(|error| error.contains("total=1 omitted=0 retained=1")));
    assert!(observation.secondary_errors.iter().any(|error| {
        error.contains("post-call attempted anomaly")
            && error.contains("source_action=11")
            && error.contains("timing_error=None")
    }));
}

#[test]
fn full_count_up_only_reversed_qpc_recovers_then_finishes_as_error() {
    assert_up_only_anomaly_recovers_and_latches_terminal_error(UpOnlyQpcFault::ReversedCompletion);
}

#[test]
fn up_only_missing_completion_qpc_without_timing_error_still_latches_error() {
    assert_up_only_anomaly_recovers_and_latches_terminal_error(UpOnlyQpcFault::MissingCompletion);
}

#[test]
fn attempted_anomaly_error_preserves_concurrent_skip_cause_at_finalize() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        let started = clock.now().expect("pre-call QPC");
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
    let candidate = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage Down")
        .expect("Down candidate");
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Dispatched
    ));
    harness.skip_requested.store(true, Ordering::Release);

    let mut observation = crate::engine::worker::FinalizeTestObservation::default();
    let outcome = harness.finalize_worker_for_test(false, &mut observation);
    assert_eq!(outcome, crate::engine::OUTCOME_ERROR);
    assert_eq!(observation.terminal_trigger_reason.as_deref(), Some("skipped"));
    assert!(observation
        .terminal_error
        .as_deref()
        .is_some_and(|error| error.contains("total=1 omitted=0 retained=1")));
    assert!(observation.secondary_errors.iter().any(|error| {
        error.contains("post-call attempted anomaly") && error.contains("source_action=0")
    }));
}

#[test]
fn forty_up_only_anomalies_keep_bounded_first_last_evidence_and_never_replay_downs() {
    const COUNT: usize = 40;
    const PERIOD_US: u64 = 50_000;
    const UP_AFTER_US: u64 = 20_000;

    let mut actions = Vec::with_capacity(COUNT * 2);
    for pair in 0..COUNT {
        let down = u32::try_from(pair * 2).expect("Down identity");
        let at = pair as u64 * PERIOD_US;
        actions.push(test_action(down, ActionKind::Down, at, 0x15));
        actions.push(test_action(
            down + 1,
            ActionKind::Up,
            at + UP_AFTER_US,
            0x15,
        ));
    }
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness.reanchor_playback_clock_for_test(epoch).expect("reanchor");
    let (cleanup_capture, cleanup_completion_qpc) =
        harness.configure_mock_cleanup_emitter_with_completion_qpc_for_test();
    let source = Arc::new(AtomicU32::new(u32::MAX));
    let captured_source = Arc::clone(&source);
    let packet_attempts = Arc::new(Mutex::new(Vec::with_capacity(COUNT * 2)));
    let captured_attempts = Arc::clone(&packet_attempts);
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        let source_action = captured_source.load(Ordering::Acquire);
        captured_attempts
            .lock()
            .expect("packet attempt log")
            .push((source_action, packet));
        let started = clock.now().expect("pre-call QPC");
        let completed = if packet.up_mask != 0 && packet.down_mask == 0 {
            Some(QpcTicks::from_raw(
                started.as_u64().checked_sub(1).expect("nonzero QPC"),
            ))
        } else {
            Some(started)
        };
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
                completed_ticks: completed,
                timing_error: None,
            },
        }
    });
    harness.attach_executor_for_test().expect("attach executor");

    let mut cleanup_sends = 0;
    for pair in 0..COUNT {
        let down_index = u32::try_from(pair * 2).expect("Down identity");
        let down_target = target_at(&harness, epoch, pair as u64 * PERIOD_US);
        let dispatch_at = harness
            .resume_barrier_qpc_for_test()
            .map_or(down_target, |barrier| barrier.max(down_target));
        wait_until_qpc(&harness, dispatch_at);
        let down = candidate_at(&mut harness, dispatch_at, epoch);
        assert_eq!(down.source_action_index, down_index);
        assert_eq!(down.packet_masks, PhysicalPacket::new(0, 1));
        dispatch_with_source(&mut harness, down, source.as_ref());

        let up_target = target_at(&harness, epoch, pair as u64 * PERIOD_US + UP_AFTER_US);
        wait_until_qpc(&harness, up_target);
        let up = candidate_at(&mut harness, up_target, epoch);
        assert_eq!(up.source_action_index, down_index + 1);
        assert_eq!(up.packet_masks, PhysicalPacket::new(1, 0));
        dispatch_with_source(&mut harness, up, source.as_ref());

        let history = &harness.runtime.attempted_dispatch_anomalies;
        assert_eq!(history.total_count(), pair as u64 + 1);
        assert_eq!(history.retained_count(), (pair + 1).min(16));
        assert_eq!(history.omitted_count(), (pair + 1).saturating_sub(16) as u64);
        let cleanup_count = cleanup_capture
            .lock()
            .expect("verified cleanup")
            .len();
        assert!(cleanup_count > cleanup_sends, "anomaly {pair} must be cleaned up");
        cleanup_sends = cleanup_count;
        let completed = QpcTicks::from_raw(cleanup_completion_qpc.load(Ordering::Acquire));
        let barrier = harness
            .resume_barrier_qpc_for_test()
            .expect("cleanup + G barrier");
        assert!(barrier >= completed);
        if pair + 1 < COUNT {
            assert!(
                barrier < target_at(&harness, epoch, (pair as u64 + 1) * PERIOD_US),
                "cleanup barrier must precede the next onset"
            );
        }
    }

    let attempts = packet_attempts.lock().expect("packet attempt log");
    assert_eq!(attempts.len(), COUNT * 2);
    let mut down_sources = std::collections::HashSet::new();
    for pair in 0..COUNT {
        let down = u32::try_from(pair * 2).expect("Down identity");
        assert_eq!(attempts[pair * 2], (down, PhysicalPacket::new(0, 1)));
        assert!(down_sources.insert(down), "authored Down {down} replayed");
        assert_eq!(
            attempts[pair * 2 + 1],
            (down + 1, PhysicalPacket::new(1, 0))
        );
    }
    assert_eq!(down_sources.len(), COUNT);
    drop(attempts);

    let history = &harness.runtime.attempted_dispatch_anomalies;
    assert_eq!(history.total_count(), 40);
    assert_eq!(history.retained_count(), 16);
    assert_eq!(history.omitted_count(), 24);
    assert!(history.details()[0].contains("source_action=1"));
    assert!(history.details()[7].contains("source_action=15"));
    assert!(history.details()[8].contains("source_action=65"));
    assert!(history.details()[15].contains("source_action=79"));
    assert!(history.details().iter().all(|record| {
        record.contains("first_inserted=1 attempts=1 status=Complete")
            && record.contains("timing_error=None")
            && record.contains("pre_call_qpc=")
            && record.contains("completion_qpc=")
    }));

    let mut observation = crate::engine::worker::FinalizeTestObservation::default();
    let outcome = harness.finalize_worker_for_test(false, &mut observation);
    assert_eq!(outcome, crate::engine::OUTCOME_ERROR);
    let mut anomaly_summary =
        std::iter::once(observation.terminal_error.as_deref().unwrap_or_default())
        .chain(observation.secondary_errors.iter().map(String::as_str));
    assert!(anomaly_summary.any(|error| {
        error.contains("total=40 omitted=24 retained=16")
    }));
    let details = observation
        .secondary_errors
        .iter()
        .filter(|error| error.contains("post-call attempted anomaly"))
        .collect::<Vec<_>>();
    assert_eq!(details.len(), 16);
    assert!(details.iter().any(|record| record.contains("source_action=1")));
    assert!(details.iter().any(|record| record.contains("source_action=79")));
}
