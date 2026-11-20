use super::support::{candidate_at, target_at, test_action, wait_until_qpc};
use crate::engine::test_support::ProductionDispatchTestHarness;
use crate::engine::worker::StagedDispatchOutcome;
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::{QpcTicks, TimelineTicks};
use sky_dispatch_win32::input::{
    PacketRetryReason, PhysicalPacket, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
enum UpOnlyQpcFault {
    ReversedCompletion,
    MissingCompletion,
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

fn assert_up_only_anomaly_terminates(fault: UpOnlyQpcFault) {
    let actions = [
        test_action(10, ActionKind::Down, 0, 0x15),
        test_action(11, ActionKind::Up, 20_000, 0x15),
        test_action(12, ActionKind::Down, 70_000, 0x16),
        test_action(13, ActionKind::Up, 90_000, 0x16),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor");
    let cleanup_capture = harness.configure_mock_cleanup_emitter_for_test();
    let source = Arc::new(AtomicU32::new(u32::MAX));
    let captured_source = Arc::clone(&source);
    let packet_attempts = Arc::new(Mutex::new(Vec::with_capacity(3)));
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

    let up_a_target = target_at(&harness, epoch, 20_000);
    wait_until_qpc(&harness, up_a_target);
    let up_a = candidate_at(&mut harness, up_a_target, epoch);
    assert_eq!(up_a.source_action_index, 11);
    assert_eq!(up_a.packet_masks, PhysicalPacket::new(1, 0));
    let qualified_before = (
        harness.local_metrics.authored_lateness_samples,
        harness.local_metrics.residual_lateness_samples,
        harness.local_metrics.hol_delay_lower_bound_samples,
        harness.local_metrics.sendinput_envelope_samples,
        harness.local_metrics.physical_floor_displacement_samples,
    );
    source.store(11, Ordering::Release);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(up_a, false),
        StagedDispatchOutcome::Exit,
        "an invalid completion timestamp is terminal after one attempted Up"
    );
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
    assert!(
        anomaly.contains("up_mask=0x0001 down_mask=0x0000"),
        "{anomaly}"
    );
    assert!(
        anomaly.contains("first_inserted=1 attempts=1 status=Complete"),
        "{anomaly}"
    );
    assert!(anomaly.contains("timing_error=None"), "{anomaly}");
    if matches!(fault, UpOnlyQpcFault::ReversedCompletion) {
        let pre = anomaly
            .split("pre_call_qpc=")
            .nth(1)
            .and_then(|value| value.split_whitespace().next())
            .expect("pre-call QPC");
        let completion = anomaly
            .split("completion_qpc=")
            .nth(1)
            .and_then(|value| value.split_whitespace().next())
            .expect("completion QPC");
        assert!(
            pre.parse::<u64>().expect("pre tick")
                > completion.parse::<u64>().expect("completion tick"),
            "{anomaly}"
        );
    } else {
        assert!(anomaly.contains("completion_qpc=unavailable"), "{anomaly}");
    }
    assert_eq!(
        packet_attempts
            .lock()
            .expect("packet attempt log")
            .as_slice(),
        &[
            (10, PhysicalPacket::new(0, 1)),
            (11, PhysicalPacket::new(1, 0)),
        ],
        "no retry of the Up and no later Down after the first anomaly"
    );
    assert!(
        cleanup_capture
            .lock()
            .expect("safety cleanup capture")
            .len()
            <= 1
    );
    assert!(
        harness
            .runtime
            .terminal_error
            .as_deref()
            .is_some_and(|error| {
                error.contains("musical playback terminated after first sender anomaly")
            })
    );
    let finalized = harness.finalize_for_test();
    assert_eq!(finalized.outcome_code, crate::engine::OUTCOME_ERROR);
}
#[test]
fn full_count_up_only_reversed_qpc_terminates_after_one_attempt() {
    assert_up_only_anomaly_terminates(UpOnlyQpcFault::ReversedCompletion);
}

#[test]
fn up_only_missing_completion_qpc_terminates_after_one_attempt() {
    assert_up_only_anomaly_terminates(UpOnlyQpcFault::MissingCompletion);
}

#[test]
fn sender_anomaly_cause_survives_skip_arriving_before_finalize() {
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
        StagedDispatchOutcome::Exit
    ));
    harness.skip_requested.store(true, Ordering::Release);

    let mut observation = crate::engine::worker::FinalizeTestObservation::default();
    let outcome = harness.finalize_worker_for_test(false, &mut observation);
    assert_eq!(outcome, crate::engine::OUTCOME_ERROR);
    assert_eq!(
        observation.terminal_trigger_reason.as_deref(),
        Some("sender_anomaly")
    );
    assert!(observation.terminal_error.as_deref().is_some_and(|error| {
        error.contains("musical playback terminated after first sender anomaly")
    }));
    assert!(
        observation
            .secondary_errors
            .iter()
            .any(|error| { error.contains("total=1 omitted=0 retained=1") })
    );
    assert!(observation.secondary_errors.iter().any(|error| {
        error.contains("post-call attempted anomaly") && error.contains("source_action=0")
    }));
}

#[test]
fn first_up_only_anomaly_terminates_and_later_downs_are_never_attempted() {
    let actions = [
        test_action(0, ActionKind::Down, 0, 0x15),
        test_action(1, ActionKind::Up, 20_000, 0x15),
        test_action(2, ActionKind::Down, 70_000, 0x16),
        test_action(3, ActionKind::Up, 90_000, 0x16),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor");
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness
        .resources
        .backend
        .set_probe(|_, _| sky_dispatch_win32::input::InstrumentPhysicalState::Inconclusive);
    let source = Arc::new(AtomicU32::new(u32::MAX));
    let captured_source = Arc::clone(&source);
    let packet_attempts = Arc::new(Mutex::new(Vec::with_capacity(3)));
    let captured_attempts = Arc::clone(&packet_attempts);
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        let source_action = captured_source.load(Ordering::Acquire);
        captured_attempts
            .lock()
            .expect("packet attempt log")
            .push((source_action, packet));
        let started = clock.now().expect("sender QPC");
        let completed = if source_action == 1 {
            Some(QpcTicks::from_raw(
                started.as_u64().checked_sub(1).expect("reversed QPC"),
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

    let first = candidate_at(&mut harness, epoch, epoch);
    dispatch_with_source(&mut harness, first, source.as_ref());
    let up_target = target_at(&harness, epoch, 20_000);
    wait_until_qpc(&harness, up_target);
    let up = candidate_at(&mut harness, up_target, epoch);
    assert_eq!(up.source_action_index, 1);
    source.store(1, Ordering::Release);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(up, false),
        StagedDispatchOutcome::Exit
    );

    assert_eq!(
        packet_attempts
            .lock()
            .expect("packet attempt log")
            .as_slice(),
        &[
            (0, PhysicalPacket::new(0, 1)),
            (1, PhysicalPacket::new(1, 0)),
        ],
        "the attempted Up is not replayed and no later Down is sent"
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    let cleanup_sends = cleanup.lock().expect("safety cleanup capture").clone();
    assert!(
        cleanup_sends.is_empty(),
        "needs_probe must not retry Up: {cleanup_sends:?}"
    );
    assert_eq!(
        harness.runtime.attempted_dispatch_anomalies.total_count(),
        1
    );

    let finalized = harness.finalize_for_test();
    assert_eq!(finalized.outcome_code, crate::engine::OUTCOME_ERROR);
    assert_eq!(finalized.release_obligation_mask, 1);
}

fn chord_action(index: u32, kind: ActionKind, scheduled_us: u64, scans: &[u16]) -> KeyActionInput {
    KeyActionInput {
        source_action_index: index,
        kind,
        scheduled_us,
        scan_codes: smallvec::SmallVec::from_slice(scans),
        reason: "production dispatch regression".into(),
    }
}

#[test]
fn suspend_during_partial_progress_down_terminates_error_with_retained_suspend_context() {
    let actions = [
        chord_action(0, ActionKind::Down, 0, &[0x15, 0x16]),
        chord_action(1, ActionKind::Up, 20_000, &[0x15, 0x16]),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    let power = Arc::clone(&harness.system_power);
    let interrupt = harness.interrupt.clone();
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        // Suspend notification injected during sender call
        power.notify(true, &interrupt);
        let started = clock.now().expect("sender QPC");
        let completed = Some(started);
        SendTransactionOutcome {
            status: SendTransactionStatus::PartialProgress,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: 1, // 2-key chord has mask 3, only key 0 (mask 1) confirmed
                skipped_mask: 0,
                first_inserted: 1,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: Some(1),
                last_win32_error: Some(1),
                started_ticks: Some(started),
                completed_ticks: completed,
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
    assert_eq!(candidate.packet_masks, PhysicalPacket::new(0, 3));

    let dispatch_outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    assert_eq!(
        dispatch_outcome,
        StagedDispatchOutcome::Exit,
        "partial progress Down exits immediately"
    );

    let mut observation = crate::engine::worker::FinalizeTestObservation::default();
    let outcome = harness.finalize_worker_for_test(false, &mut observation);
    assert_eq!(
        outcome,
        crate::engine::OUTCOME_ERROR,
        "partial progress with concurrent suspend must terminate with Error, not SystemSuspended"
    );
    assert_eq!(
        observation.terminal_trigger_reason.as_deref(),
        Some("sender_anomaly")
    );
    assert!(
        observation
            .terminal_error
            .as_deref()
            .is_some_and(|e| e.contains("musical playback terminated after first sender anomaly")),
        "nonempty attempted anomaly terminal error: {:?}",
        observation.terminal_error
    );
    assert!(
        observation
            .secondary_errors
            .iter()
            .any(|e| e.contains("system suspend")),
        "suspend context retained in secondary errors: {:?}",
        observation.secondary_errors
    );
    assert!(
        observation
            .secondary_errors
            .iter()
            .any(|e| e.contains("post-call attempted anomaly")),
        "attempted anomaly details retained: {:?}",
        observation.secondary_errors
    );
    assert_eq!(
        cleanup.lock().expect("cleanup lock").as_slice(),
        &[(vec![0x15, 0x16], true)],
        "verified terminal release safely cleans up confirmed partial key"
    );
}

#[test]
fn suspend_before_send_or_zero_progress_terminates_system_suspended_without_attempted_down() {
    let actions = [
        chord_action(0, ActionKind::Down, 0, &[0x15, 0x16]),
        chord_action(1, ActionKind::Up, 20_000, &[0x15, 0x16]),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    let packet_attempts = Arc::new(Mutex::new(0u32));
    let attempts_clone = Arc::clone(&packet_attempts);
    let power = Arc::clone(&harness.system_power);
    let interrupt = harness.interrupt.clone();
    harness
        .resources
        .backend
        .set_packet_emitter(move |_packet| {
            *attempts_clone.lock().unwrap() += 1;
            // Suspend concurrent with zero-progress before syscall
            power.notify(true, &interrupt);
            SendTransactionOutcome {
                status: SendTransactionStatus::ZeroProgress,
                evidence: SendEvidence {
                    requested_mask: 0,
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

    let dispatch_outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    assert_eq!(dispatch_outcome, StagedDispatchOutcome::Exit);

    let mut observation = crate::engine::worker::FinalizeTestObservation::default();
    let outcome = harness.finalize_worker_for_test(false, &mut observation);
    assert_eq!(
        outcome,
        crate::engine::OUTCOME_SYSTEM_SUSPENDED,
        "zero-progress with concurrent suspend terminates with SystemSuspended"
    );
    assert_eq!(
        observation.terminal_trigger_reason.as_deref(),
        Some("system_suspend")
    );
    assert_eq!(observation.terminal_error, None);
    assert_eq!(
        *packet_attempts.lock().unwrap(),
        1,
        "emitter was called once and reported attempts=0"
    );
}

#[test]
fn healthy_send_with_suspend_control_case_terminates_system_suspended() {
    let actions = [
        chord_action(0, ActionKind::Down, 0, &[0x15, 0x16]),
        chord_action(1, ActionKind::Up, 20_000, &[0x15, 0x16]),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    let power = Arc::clone(&harness.system_power);
    let interrupt = harness.interrupt.clone();
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        // Healthy send with suspend notification injected during emission
        power.notify(true, &interrupt);
        let started = clock.now().expect("sender QPC");
        let completed = Some(started);
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

    let candidate = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage Down")
        .expect("Down candidate");

    let dispatch_outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    assert_eq!(dispatch_outcome, StagedDispatchOutcome::Exit);

    let mut observation = crate::engine::worker::FinalizeTestObservation::default();
    let outcome = harness.finalize_worker_for_test(false, &mut observation);
    assert_eq!(
        outcome,
        crate::engine::OUTCOME_SYSTEM_SUSPENDED,
        "healthy send with concurrent suspend terminates with clean SystemSuspended"
    );
    assert_eq!(
        observation.terminal_trigger_reason.as_deref(),
        Some("system_suspend")
    );
    assert_eq!(observation.terminal_error, None);
}
