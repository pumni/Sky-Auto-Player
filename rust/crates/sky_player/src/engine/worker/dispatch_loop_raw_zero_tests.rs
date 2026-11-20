use super::support::wait_until_qpc;
use crate::engine::test_support::ProductionDispatchTestHarness;
use crate::engine::worker::StagedDispatchOutcome;
use sky_dispatch_core::executor::{DispatchDecision, DownChordMetadata};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::{QpcTicks, TimelineTicks};
use sky_dispatch_win32::input::{
    PacketRetryReason, PhysicalPacket, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn raw_zero_action(
    source_action_index: u32,
    kind: ActionKind,
    scheduled_us: u64,
    scan_codes: &[u16],
) -> KeyActionInput {
    KeyActionInput {
        source_action_index,
        kind,
        scheduled_us,
        scan_codes: scan_codes.iter().copied().collect(),
        reason: "Issue 666 proven raw-zero regression".into(),
    }
}

fn raw_zero_sequence() -> [KeyActionInput; 6] {
    [
        raw_zero_action(600, ActionKind::Down, 0, &[0x15]),
        raw_zero_action(601, ActionKind::Down, 20_000, &[0x16]),
        raw_zero_action(602, ActionKind::Down, 30_000, &[0x17]),
        raw_zero_action(603, ActionKind::Up, 40_000, &[0x16]),
        raw_zero_action(604, ActionKind::Up, 50_000, &[0x17]),
        raw_zero_action(605, ActionKind::Up, 100_000, &[0x15]),
    ]
}

fn qpc_at_offset(
    harness: &ProductionDispatchTestHarness,
    epoch: QpcTicks,
    offset_us: u64,
) -> QpcTicks {
    epoch
        .checked_add_duration(harness.qpc_duration_from_us_for_test(offset_us).unwrap())
        .unwrap()
}

fn stage_at_qpc(
    harness: &mut ProductionDispatchTestHarness,
    at: QpcTicks,
    epoch: QpcTicks,
) -> (
    u16,
    Option<DownChordMetadata>,
    crate::engine::StagedCandidate,
) {
    match harness
        .evaluate_live_scheduler_for_test(at, epoch)
        .expect("evaluate production scheduler")
    {
        DispatchDecision::Send {
            up_mask,
            down_chord,
            timing_window,
        } => {
            let down_chord = down_chord.map(Into::into);
            let candidate = harness
                .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
                .expect("stage production candidate")
                .expect("candidate is available");
            (up_mask, down_chord, candidate)
        }
        decision => panic!("expected staged send at {at:?}, got {decision:?}"),
    }
}

fn install_success_then_raw_zero_sender(
    harness: &mut ProductionDispatchTestHarness,
    after_raw_zero: impl Fn() + Send + Sync + 'static,
) -> Arc<Mutex<Vec<(PhysicalPacket, SendEvidence)>>> {
    let packets = Arc::new(Mutex::new(Vec::with_capacity(8)));
    let captured = Arc::clone(&packets);
    let calls = Arc::new(AtomicUsize::new(0));
    let call_index = Arc::clone(&calls);
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        let index = call_index.fetch_add(1, Ordering::AcqRel);
        let started = clock.now().expect("mock packet start QPC");
        let completed = clock.now().expect("mock packet completion QPC");
        let requested_mask = packet.up_mask | packet.down_mask;
        let is_raw_zero = index == 1;
        if is_raw_zero {
            after_raw_zero();
        }
        let evidence = SendEvidence {
            requested_mask,
            confirmed_mask: if is_raw_zero { 0 } else { requested_mask },
            skipped_mask: 0,
            first_inserted: if is_raw_zero { 0 } else { packet.event_count() },
            attempts: 1,
            zero_progress_retries: 0,
            retry_reason: if is_raw_zero {
                PacketRetryReason::ZeroProgress
            } else {
                PacketRetryReason::None
            },
            first_win32_error: is_raw_zero.then_some(5),
            last_win32_error: is_raw_zero.then_some(5),
            started_ticks: Some(started),
            completed_ticks: Some(completed),
            timing_error: None,
        };
        captured
            .lock()
            .expect("packet capture lock")
            .push((packet, evidence));
        SendTransactionOutcome {
            status: if is_raw_zero {
                SendTransactionStatus::ZeroProgress
            } else {
                SendTransactionStatus::Complete
            },
            evidence,
        }
    });
    packets
}

#[test]
fn proven_raw_zero_down_only_clears_only_the_new_obligation_and_terminates() {
    let mut harness = ProductionDispatchTestHarness::create_harness(&raw_zero_sequence());
    let epoch = harness.qpc_now_for_test().expect("QPC epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback epoch");
    let packets = install_success_then_raw_zero_sender(&mut harness, || {});
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");

    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage A Down")
        .expect("A Down candidate");
    assert_eq!(first.source_action_index, 600);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.backend.active_mask, 0b0001);

    let b_at = qpc_at_offset(&harness, epoch, 20_000);
    wait_until_qpc(&harness, b_at);
    let (b_up_mask, b_down, b) = stage_at_qpc(&mut harness, b_at, epoch);
    assert_eq!(b_up_mask, 0);
    assert_eq!(b_down.map(|chord| chord.source_action_index), Some(601));
    assert_eq!(b.packet_masks, PhysicalPacket::new(0, 0b0010));
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(b, false),
        StagedDispatchOutcome::Exit
    );

    let captured = packets.lock().expect("captured musical packets");
    assert_eq!(captured.len(), 2, "no onset is attempted after raw zero");
    assert_eq!(captured[0].0, PhysicalPacket::new(0, 0b0001));
    assert_eq!(captured[1].0, PhysicalPacket::new(0, 0b0010));
    assert_eq!(captured[1].1.first_inserted, 0);
    assert_eq!(captured[1].1.attempts, 1);
    drop(captured);
    assert_eq!(harness.release_obligation_mask_for_test() & 0b0010, 0);
    assert_eq!(harness.runtime.last_physical_send_completion_qpc, None);
    assert_eq!(
        harness
            .runtime
            .attempted_dispatch_anomalies
            .retained_count(),
        1
    );
    assert_eq!(
        cleanup.lock().expect("cleanup capture").as_slice(),
        &[(vec![0x15], true)],
        "only the older journaled A duty is safety-released"
    );

    let finalized = harness.finalize_for_test();
    assert_eq!(finalized.outcome_code, crate::engine::OUTCOME_ERROR);
    assert_eq!(finalized.release_obligation_mask & 0b0010, 0);
}

#[test]
fn raw_zero_does_not_create_a_completion_floor_or_send_later_events() {
    let mut harness = ProductionDispatchTestHarness::create_harness(&raw_zero_sequence());
    let epoch = harness.qpc_now_for_test().expect("QPC epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback epoch");
    let packets = install_success_then_raw_zero_sender(&mut harness, || {});
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");

    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage A Down")
        .expect("A Down candidate");
    assert_eq!(first.source_action_index, 600);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let b_at = qpc_at_offset(&harness, epoch, 20_000);
    wait_until_qpc(&harness, b_at);
    let (_, b_down, b) = stage_at_qpc(&mut harness, b_at, epoch);
    assert_eq!(b_down.map(|chord| chord.source_action_index), Some(601));
    assert_eq!(b.packet_masks, PhysicalPacket::new(0, 0b0010));
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(b, false),
        StagedDispatchOutcome::Exit
    );

    let captured = packets.lock().expect("captured musical packets");
    assert_eq!(
        captured.len(),
        2,
        "anomaly terminates before any later event"
    );
    assert_eq!(captured[0].0, PhysicalPacket::new(0, 0b0001));
    assert_eq!(captured[1].0, PhysicalPacket::new(0, 0b0010));
    assert_eq!(captured[1].1.first_inserted, 0);
    drop(captured);
    assert_eq!(harness.runtime.last_physical_send_completion_qpc, None);
    assert_eq!(
        harness
            .runtime
            .attempted_dispatch_anomalies
            .retained_count(),
        1
    );
    assert!(
        harness
            .runtime
            .terminal_error
            .as_deref()
            .is_some_and(|error| {
                error.contains("musical playback terminated after proven raw-zero Down")
            })
    );

    let finalized = harness.finalize_for_test();
    assert_eq!(finalized.outcome_code, crate::engine::OUTCOME_ERROR);
    assert_eq!(finalized.release_obligation_mask & 0b0010, 0);
    assert!(cleanup.lock().expect("cleanup capture").len() <= 1);
}
#[test]
fn h_g_real_waiter_smoke_wakes_on_a_future_boundary() {
    let actions = [
        raw_zero_action(616, ActionKind::Down, 0, &[0x15]),
        raw_zero_action(617, ActionKind::Up, 1_000_000, &[0x15]),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let now = harness.qpc_now_for_test().expect("wait smoke QPC");
    let deadline = now
        .checked_add_duration(
            harness
                .resources
                .clock
                .duration_from_us(5_000)
                .expect("5 ms smoke interval"),
        )
        .expect("wait smoke deadline");
    let result = super::super::wait_after_unstable_target(
        now,
        deadline,
        harness.resources.clock,
        &harness.resources.waiter,
        &harness.interrupt,
        &mut harness.local_metrics,
        &mut harness.runtime.terminal_error,
    );
    assert!(matches!(result, super::super::WaitBoundary::Due { .. }));
    assert!(harness.qpc_now_for_test().expect("wake QPC") >= deadline);
    assert!(harness.runtime.terminal_error.is_none());
}

#[test]
fn raw_zero_classifier_rejects_short_packet_and_contradictory_qpc() {
    for contradictory_qpc in [false, true] {
        let actions = [
            raw_zero_action(620, ActionKind::Down, 0, &[0x15]),
            raw_zero_action(621, ActionKind::Down, 20_000, &[0x16, 0x17]),
            raw_zero_action(622, ActionKind::Up, 40_000, &[0x16, 0x17]),
            raw_zero_action(623, ActionKind::Up, 100_000, &[0x15]),
        ];
        let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
        let epoch = harness.qpc_now_for_test().expect("QPC epoch");
        harness
            .reanchor_playback_clock_for_test(epoch)
            .expect("reanchor playback epoch");
        let cleanup = harness.configure_mock_cleanup_emitter_for_test();
        harness.attach_executor_for_test().expect("attach executor");
        let calls = Arc::new(AtomicUsize::new(0));
        let captured_calls = Arc::clone(&calls);
        let clock = harness.resources.clock;
        harness.resources.backend.set_packet_emitter(move |packet| {
            let index = captured_calls.fetch_add(1, Ordering::AcqRel);
            let started = clock.now().expect("sender QPC");
            let completed = clock.now().expect("completion QPC");
            let requested_mask = packet.up_mask | packet.down_mask;
            let (status, inserted, retry_reason, timing_error) = if index == 0 {
                (
                    SendTransactionStatus::Complete,
                    packet.event_count(),
                    PacketRetryReason::None,
                    None,
                )
            } else if contradictory_qpc {
                (
                    SendTransactionStatus::ZeroProgress,
                    0,
                    PacketRetryReason::ZeroProgress,
                    Some(sky_dispatch_win32::clock::QpcError::CounterUnavailable),
                )
            } else {
                (
                    SendTransactionStatus::PartialProgress,
                    1,
                    PacketRetryReason::PartialProgress { inserted_count: 1 },
                    None,
                )
            };
            let is_complete = status == SendTransactionStatus::Complete;
            SendTransactionOutcome {
                status,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: if is_complete { requested_mask } else { 0 },
                    skipped_mask: 0,
                    first_inserted: inserted,
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason,
                    first_win32_error: (!is_complete).then_some(5),
                    last_win32_error: (!is_complete).then_some(5),
                    started_ticks: Some(started),
                    completed_ticks: Some(if contradictory_qpc && index != 0 {
                        QpcTicks::from_raw(started.as_u64().saturating_sub(1))
                    } else {
                        completed
                    }),
                    timing_error,
                },
            }
        });

        let first = harness
            .stage_candidate_for_test(
                0,
                harness.current_executor_chord_for_test(),
                TimelineTicks::ZERO,
            )
            .expect("stage A Down")
            .expect("A Down candidate");
        assert_eq!(
            harness.dispatch_staged_candidate_for_test(first, false),
            StagedDispatchOutcome::Dispatched
        );
        let b_at = qpc_at_offset(&harness, epoch, 20_000);
        wait_until_qpc(&harness, b_at);
        let (_, _, b) = stage_at_qpc(&mut harness, b_at, epoch);
        assert_eq!(b.packet_masks.down_mask.count_ones(), 2);
        assert_eq!(
            harness.dispatch_staged_candidate_for_test(b, false),
            StagedDispatchOutcome::Exit
        );

        assert_eq!(calls.load(Ordering::Acquire), 2);
        assert_eq!(harness.resources.backend.in_flight_mask, 0);
        assert_eq!(harness.release_obligation_mask_for_test(), 0);
        assert!(harness.resume_barrier_qpc_for_test().is_some());
        assert!(!cleanup.lock().expect("cleanup capture").is_empty());
        let anomaly = harness
            .runtime
            .attempted_dispatch_anomalies
            .details()
            .first()
            .expect("attempted anomaly remains sticky");
        if contradictory_qpc {
            assert!(
                anomaly.contains("timing_error=Some(CounterUnavailable)"),
                "{anomaly}"
            );
        } else {
            assert!(anomaly.contains("status=PartialProgress"));
            assert!(anomaly.contains("inserted=1"));
        }
        let _ = harness.finalize_for_test();
    }
}

#[test]
fn mixed_up_a_down_b_raw_zero_uses_conservative_reconciliation() {
    let actions = [
        raw_zero_action(640, ActionKind::Down, 0, &[0x15]),
        raw_zero_action(641, ActionKind::Up, 20_000, &[0x15]),
        raw_zero_action(642, ActionKind::Down, 20_000, &[0x16]),
        raw_zero_action(643, ActionKind::Up, 40_000, &[0x16]),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("QPC epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback epoch");
    let packets = Arc::new(Mutex::new(Vec::with_capacity(2)));
    let captured = Arc::clone(&packets);
    let calls = Arc::new(AtomicUsize::new(0));
    let call_index = Arc::clone(&calls);
    let clock = harness.resources.clock;
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    harness.resources.backend.set_packet_emitter(move |packet| {
        let index = call_index.fetch_add(1, Ordering::AcqRel);
        let started = clock.now().expect("sender QPC");
        let completed = clock.now().expect("completion QPC");
        let requested_mask = packet.up_mask | packet.down_mask;
        let raw_zero = index == 1;
        let evidence = SendEvidence {
            requested_mask,
            confirmed_mask: if raw_zero { 0 } else { requested_mask },
            skipped_mask: 0,
            first_inserted: if raw_zero { 0 } else { packet.event_count() },
            attempts: 1,
            zero_progress_retries: 0,
            retry_reason: if raw_zero {
                PacketRetryReason::ZeroProgress
            } else {
                PacketRetryReason::None
            },
            first_win32_error: raw_zero.then_some(5),
            last_win32_error: raw_zero.then_some(5),
            started_ticks: Some(started),
            completed_ticks: Some(completed),
            timing_error: None,
        };
        captured
            .lock()
            .expect("packet capture")
            .push((packet, evidence));
        SendTransactionOutcome {
            status: if raw_zero {
                SendTransactionStatus::ZeroProgress
            } else {
                SendTransactionStatus::Complete
            },
            evidence,
        }
    });

    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage A Down")
        .expect("A Down candidate");
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );
    let mixed_at = qpc_at_offset(&harness, epoch, 20_000);
    wait_until_qpc(&harness, mixed_at);
    let (up_mask, down, mixed) = stage_at_qpc(&mut harness, mixed_at, epoch);
    assert_eq!(up_mask, 0b0001);
    assert_eq!(down.map(|chord| chord.source_action_index), Some(642));
    assert_eq!(mixed.packet_masks, PhysicalPacket::new(0b0001, 0b0010));
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(mixed, false),
        StagedDispatchOutcome::Exit
    );
    assert_eq!(calls.load(Ordering::Acquire), 2);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert!(harness.resume_barrier_qpc_for_test().is_some());
    assert!(!cleanup.lock().expect("cleanup capture").is_empty());
    let captured = packets.lock().expect("packet capture");
    assert_eq!(captured[1].0, PhysicalPacket::new(0b0001, 0b0010));
    assert_eq!(captured[1].1.first_inserted, 0);
    drop(captured);
    let _ = harness.finalize_for_test();
}

#[test]
fn raw_zero_candidate_owner_drift_after_identity_query_fails_before_sender() {
    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    let actions = raw_zero_sequence();
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("QPC epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback epoch");
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[0]);
    harness.attach_executor_for_test().expect("attach executor");

    // Invalidate the one-time target at the final admission hook. Even though
    // this sender is configured to report raw zero, the candidate must never
    // cross the packet edge.
    let target = Arc::clone(&harness.target);
    harness.set_final_gate_before_syscall_hook(move || {
        target.invalidate_for_test();
    });
    let candidate = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage Down candidate")
        .expect("Down candidate");
    assert_eq!(
        harness.pre_wait_candidate_admission_for_test(&candidate),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    );
    assert!(packets.lock().expect("mock packet capture").is_empty());
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|chord| chord.source_action_index),
        Some(600)
    );
    assert!(!harness.runtime.musical_physical_attempt_started);
}

#[derive(Clone, Copy)]
enum ConcurrentAbortSignal {
    Stop,
    Skip,
    LeaseExpired,
}

#[test]
fn raw_zero_with_concurrent_stop_skip_or_lease_expiry_cleans_up_and_does_not_continue() {
    for signal in [
        ConcurrentAbortSignal::Stop,
        ConcurrentAbortSignal::Skip,
        ConcurrentAbortSignal::LeaseExpired,
    ] {
        let mut harness = ProductionDispatchTestHarness::create_harness(&raw_zero_sequence());
        let epoch = harness.qpc_now_for_test().expect("QPC epoch");
        harness
            .reanchor_playback_clock_for_test(epoch)
            .expect("reanchor playback epoch");
        let packets = Arc::new(AtomicUsize::new(0));
        let calls = Arc::clone(&packets);
        let quit = Arc::clone(&harness.quit_requested);
        let skip = Arc::clone(&harness.skip_requested);
        let lease = Arc::clone(&harness.supervisor_expired);
        let clock = harness.resources.clock;
        let cleanup = harness.configure_mock_cleanup_emitter_for_test();
        harness.attach_executor_for_test().expect("attach executor");
        harness.resources.backend.set_packet_emitter(move |packet| {
            let index = calls.fetch_add(1, Ordering::AcqRel);
            let started = clock.now().expect("sender QPC");
            let completed = clock.now().expect("completion QPC");
            let requested_mask = packet.up_mask | packet.down_mask;
            let raw_zero = index == 1;
            if raw_zero {
                match signal {
                    ConcurrentAbortSignal::Stop => quit.store(true, Ordering::Release),
                    ConcurrentAbortSignal::Skip => skip.store(true, Ordering::Release),
                    ConcurrentAbortSignal::LeaseExpired => lease.latch_expired(),
                }
            }
            let evidence = SendEvidence {
                requested_mask,
                confirmed_mask: if raw_zero { 0 } else { requested_mask },
                skipped_mask: 0,
                first_inserted: if raw_zero { 0 } else { packet.event_count() },
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: if raw_zero {
                    PacketRetryReason::ZeroProgress
                } else {
                    PacketRetryReason::None
                },
                first_win32_error: raw_zero.then_some(5),
                last_win32_error: raw_zero.then_some(5),
                started_ticks: Some(started),
                completed_ticks: Some(completed),
                timing_error: None,
            };
            SendTransactionOutcome {
                status: if raw_zero {
                    SendTransactionStatus::ZeroProgress
                } else {
                    SendTransactionStatus::Complete
                },
                evidence,
            }
        });

        let first = harness
            .stage_candidate_for_test(
                0,
                harness.current_executor_chord_for_test(),
                TimelineTicks::ZERO,
            )
            .expect("stage A Down")
            .expect("A Down candidate");
        assert_eq!(
            harness.dispatch_staged_candidate_for_test(first, false),
            StagedDispatchOutcome::Dispatched
        );
        let b_at = qpc_at_offset(&harness, epoch, 20_000);
        wait_until_qpc(&harness, b_at);
        let (_, _, b) = stage_at_qpc(&mut harness, b_at, epoch);
        assert_eq!(
            harness.dispatch_staged_candidate_for_test(b, false),
            StagedDispatchOutcome::Exit
        );
        assert_eq!(packets.load(Ordering::Acquire), 2);
        assert_eq!(harness.resources.backend.in_flight_mask, 0);
        assert_eq!(harness.release_obligation_mask_for_test(), 0);
        assert_eq!(
            harness
                .current_executor_chord_for_test()
                .map(|chord| chord.source_action_index),
            Some(602),
            "the attempted B cursor advances, but C is not attempted after a terminal signal"
        );
        assert!(!cleanup.lock().expect("cleanup capture").is_empty());
        assert_eq!(
            harness
                .runtime
                .attempted_dispatch_anomalies
                .retained_count(),
            1
        );
        let _ = harness.finalize_for_test();
    }
}

#[cfg(feature = "qpc-read-counting")]
#[test]
fn measured_final_admission_operations_for_healthy_packet_shapes() {
    use super::support::{qpc_offset, staged_send_at};
    use crate::engine::test_support::PacketCapture;
    use std::sync::atomic::AtomicBool;

    fn reset_counts(harness: &ProductionDispatchTestHarness) {
        crate::engine::worker::admission::reset_final_admission_atomic_reads_for_test();
        sky_dispatch_core::executor::reset_scheduler_evaluation_count_for_test();
        sky_dispatch_win32::clock::reset_qpc_read_count();
        sky_dispatch_win32::focus::reset_foreground_query_count();
        sky_dispatch_win32::focus::reset_process_identity_counts_for_test();
        sky_dispatch_win32::input::reset_release_owner_proof_arc_clone_count_for_test();
        harness.reset_target_proof_counts_for_test();
    }

    fn enable_fresh_lease(harness: &mut ProductionDispatchTestHarness, epoch: QpcTicks) {
        const LEASE_TIMEOUT_US: u64 = 1_000_000;
        harness.config.wait.supervisor_lease_timeout_us = LEASE_TIMEOUT_US;
        harness.timing.lease_timeout_ticks = harness
            .qpc_duration_from_us_for_test(LEASE_TIMEOUT_US)
            .expect("lease timeout conversion");
        harness.supervisor_expired.reset(epoch);
    }

    fn report_counts(
        label: &str,
        harness: &ProductionDispatchTestHarness,
        packets: &PacketCapture,
    ) {
        let sender_attempts: u64 = packets
            .lock()
            .expect("packet capture")
            .iter()
            .map(|(_, evidence)| u64::from(evidence.attempts))
            .sum();
        let generation_checks = harness.target_proof_generation_revalidations_for_test();
        let target_atomic_reads = harness.target_proof_atomic_reads_for_test();
        let admission_atomic_reads =
            crate::engine::worker::admission::final_admission_atomic_reads_for_test();
        let release_identity_arc_clones =
            sky_dispatch_win32::input::release_owner_proof_arc_clone_count_for_test();
        let expected_identity_proofs = u64::from(sender_attempts != 0);
        assert_eq!(
            sky_dispatch_win32::focus::foreground_query_count(),
            expected_identity_proofs,
            "a healthy focus-required packet performs one fresh foreground query"
        );
        assert_eq!(
            sky_dispatch_win32::focus::owner_query_count(),
            expected_identity_proofs,
            "a healthy focus-required packet resolves one foreground owner PID"
        );
        assert_eq!(
            sky_dispatch_win32::focus::process_identity_check_count_for_test(),
            expected_identity_proofs,
            "a healthy focus-required packet checks its retained process once"
        );
        assert_eq!(
            sky_dispatch_win32::focus::process_liveness_check_count_for_test(),
            expected_identity_proofs,
            "a healthy focus-required packet proves process liveness once"
        );
        println!(
            "proof-count label={label} scheduler_evaluations={} qpc_reads={} foreground_queries={} real_GetForegroundWindow={} owner_pid_checks={} process_identity_checks={} process_liveness_proofs={} GetProcessTimes={} admission_atomic_reads={admission_atomic_reads} target_atomic_reads={target_atomic_reads} release_identity_arc_clones={release_identity_arc_clones} generation_revalidations={generation_checks} session_owner_bound={} sender_attempts={sender_attempts} allocations=0",
            sky_dispatch_core::executor::scheduler_evaluation_count_for_test(),
            sky_dispatch_win32::clock::qpc_read_count(),
            sky_dispatch_win32::focus::foreground_query_count(),
            sky_dispatch_win32::focus::real_foreground_query_count(),
            sky_dispatch_win32::focus::owner_query_count(),
            sky_dispatch_win32::focus::process_identity_check_count_for_test(),
            sky_dispatch_win32::focus::process_liveness_check_count_for_test(),
            sky_dispatch_win32::focus::get_process_times_call_count_for_test(),
            harness
                .resources
                .backend
                .session_release_owner_proof()
                .is_some(),
        );
    }

    fn clear_packets(packets: &PacketCapture) {
        packets.lock().expect("packet capture").clear();
    }

    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();

    // Down-only.
    let actions = [
        raw_zero_action(700, ActionKind::Down, 0, &[0x15]),
        raw_zero_action(701, ActionKind::Up, 100_000, &[0x15]),
    ];
    let mut down = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = down.qpc_now_for_test().expect("epoch");
    down.reanchor_playback_clock_for_test(epoch).unwrap();
    down.configure_focus_owner_for_test(1, 177);
    let down_packets = down.configure_mock_packet_sender_for_test(&[0]);
    down.attach_executor_for_test().unwrap();
    down.set_mock_sender_start_qpc_for_test(epoch);
    enable_fresh_lease(&mut down, epoch);
    reset_counts(&down);
    let candidate = down
        .stage_candidate_for_test(
            0,
            down.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        down.pre_wait_candidate_admission_for_test(&candidate),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(
        down.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Dispatched
    );
    report_counts("down-only", &down, &down_packets);
    let _ = down.finalize_for_test();

    // Up-only. Setup Down and wait occur before the measured suffix.
    let actions = [
        raw_zero_action(710, ActionKind::Down, 0, &[0x15]),
        raw_zero_action(711, ActionKind::Up, 20_000, &[0x15]),
    ];
    let mut up = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = up.qpc_now_for_test().expect("epoch");
    up.reanchor_playback_clock_for_test(epoch).unwrap();
    up.configure_focus_owner_for_test(1, 177);
    let up_packets = up.configure_mock_packet_sender_for_test(&[0, 0]);
    up.attach_executor_for_test().unwrap();
    up.set_mock_sender_start_qpc_for_test(epoch);
    let first = up
        .stage_candidate_for_test(0, up.current_executor_chord_for_test(), TimelineTicks::ZERO)
        .unwrap()
        .unwrap();
    assert_eq!(
        up.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );
    let up_at = qpc_at_offset(&up, epoch, 20_000);
    wait_until_qpc(&up, up_at);
    up.set_mock_sender_start_qpc_for_test(up.qpc_now_for_test().unwrap());
    clear_packets(&up_packets);
    let up_measurement_epoch = up.qpc_now_for_test().unwrap();
    enable_fresh_lease(&mut up, up_measurement_epoch);
    reset_counts(&up);
    let up_candidate = up
        .stage_candidate_for_test(0b0001, None, qpc_offset(&up, 20_000))
        .unwrap()
        .unwrap();
    assert_eq!(
        up.pre_wait_candidate_admission_for_test(&up_candidate),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(
        up.dispatch_staged_candidate_for_test(up_candidate, false),
        StagedDispatchOutcome::Dispatched
    );
    report_counts("up-only", &up, &up_packets);
    let _ = up.finalize_for_test();

    // Mixed Up(A)+Down(B).
    let actions = [
        raw_zero_action(720, ActionKind::Down, 0, &[0x15]),
        raw_zero_action(721, ActionKind::Up, 20_000, &[0x15]),
        raw_zero_action(722, ActionKind::Down, 20_000, &[0x16]),
        raw_zero_action(723, ActionKind::Up, 40_000, &[0x16]),
    ];
    let mut mixed = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = mixed.qpc_now_for_test().expect("epoch");
    mixed.reanchor_playback_clock_for_test(epoch).unwrap();
    mixed.configure_focus_owner_for_test(1, 177);
    let mixed_packets = mixed.configure_mock_packet_sender_for_test(&[0, 0]);
    mixed.attach_executor_for_test().unwrap();
    mixed.set_mock_sender_start_qpc_for_test(epoch);
    let first = mixed
        .stage_candidate_for_test(
            0,
            mixed.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        mixed.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );
    let mixed_at = qpc_at_offset(&mixed, epoch, 20_000);
    wait_until_qpc(&mixed, mixed_at);
    mixed.set_mock_sender_start_qpc_for_test(mixed.qpc_now_for_test().unwrap());
    clear_packets(&mixed_packets);
    let mixed_measurement_epoch = mixed.qpc_now_for_test().unwrap();
    enable_fresh_lease(&mut mixed, mixed_measurement_epoch);
    reset_counts(&mixed);
    let (up_mask, down_chord, timing_window) = staged_send_at(&mut mixed, mixed_at, epoch);
    let mixed_candidate = mixed
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .unwrap()
        .unwrap();
    assert_eq!(
        mixed.pre_wait_candidate_admission_for_test(&mixed_candidate),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(
        mixed.dispatch_staged_candidate_for_test(mixed_candidate, false),
        StagedDispatchOutcome::Dispatched
    );
    report_counts("mixed", &mixed, &mixed_packets);
    let _ = mixed.finalize_for_test();

    // Mixed modifier block replans to the independent Up-only packet.
    let actions = [
        raw_zero_action(730, ActionKind::Down, 0, &[0x15]),
        raw_zero_action(731, ActionKind::Up, 20_000, &[0x15]),
        raw_zero_action(732, ActionKind::Down, 20_000, &[0x16]),
        raw_zero_action(733, ActionKind::Up, 40_000, &[0x16]),
    ];
    let mut fallback = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = fallback.qpc_now_for_test().expect("epoch");
    fallback.reanchor_playback_clock_for_test(epoch).unwrap();
    fallback.configure_focus_owner_for_test(1, 177);
    let fallback_packets = fallback.configure_mock_packet_sender_for_test(&[0, 0]);
    fallback.attach_executor_for_test().unwrap();
    fallback.set_mock_sender_start_qpc_for_test(epoch);
    let first = fallback
        .stage_candidate_for_test(
            0,
            fallback.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        fallback.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );
    let fallback_at = qpc_at_offset(&fallback, epoch, 20_000);
    wait_until_qpc(&fallback, fallback_at);
    fallback.set_mock_sender_start_qpc_for_test(fallback.qpc_now_for_test().unwrap());
    clear_packets(&fallback_packets);
    let modifier_held = Arc::new(AtomicBool::new(false));
    let held_for_query = Arc::clone(&modifier_held);
    fallback
        .resources
        .backend
        .set_modifier_key_state_query_for_test(move |_| {
            if held_for_query.load(Ordering::Acquire) {
                i16::MIN
            } else {
                0
            }
        });
    let held_for_gate = Arc::clone(&modifier_held);
    fallback.set_final_gate_before_syscall_hook(move || {
        held_for_gate.store(true, Ordering::Release);
    });
    let fallback_measurement_epoch = fallback.qpc_now_for_test().unwrap();
    enable_fresh_lease(&mut fallback, fallback_measurement_epoch);
    reset_counts(&fallback);
    let (up_mask, down_chord, timing_window) = staged_send_at(&mut fallback, fallback_at, epoch);
    let candidate = fallback
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .unwrap()
        .unwrap();
    assert_eq!(
        fallback.pre_wait_candidate_admission_for_test(&candidate),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(
        fallback.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Dispatched
    );
    let selected_packets = fallback_packets.lock().expect("selected packet capture");
    assert_eq!(selected_packets.len(), 1, "Up-prefix uses one sender call");
    assert_eq!(selected_packets[0].0, PhysicalPacket::new(1, 0));
    assert_eq!(selected_packets[0].1.attempts, 1);
    drop(selected_packets);
    assert_eq!(
        fallback
            .current_executor_chord_for_test()
            .map(|chord| chord.down_mask),
        Some(2),
        "Up-prefix must leave the staged Down onset pending"
    );
    assert_eq!(fallback.release_obligation_mask_for_test(), 0);
    report_counts("mixed-to-up-only", &fallback, &fallback_packets);
    let _ = fallback.finalize_for_test();

    // Target invalidation at final admission rejects the candidate before
    // the mock packet boundary. No sender call is counted.
    let actions = [
        raw_zero_action(740, ActionKind::Down, 0, &[0x15]),
        raw_zero_action(741, ActionKind::Up, 100_000, &[0x15]),
    ];
    let mut blocked = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = blocked.qpc_now_for_test().expect("epoch");
    blocked.reanchor_playback_clock_for_test(epoch).unwrap();
    blocked.configure_focus_owner_for_test(1, 177);
    let blocked_packets = blocked.configure_mock_packet_sender_for_test(&[0]);
    blocked.attach_executor_for_test().unwrap();
    blocked.set_mock_sender_start_qpc_for_test(epoch);
    let target = Arc::clone(&blocked.target);
    blocked.set_final_gate_before_syscall_hook(move || {
        target.invalidate_for_test();
    });
    enable_fresh_lease(&mut blocked, epoch);
    reset_counts(&blocked);
    let candidate = blocked
        .stage_candidate_for_test(
            0,
            blocked.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        blocked.pre_wait_candidate_admission_for_test(&candidate),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(
        blocked.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    );
    report_counts("blocked-generation-replan", &blocked, &blocked_packets);
    assert!(blocked_packets.lock().expect("packet capture").is_empty());
    let _ = blocked.finalize_for_test();

    // A Down blocked by a held modifier is the common pre-call replan path.
    // It must stop before lease QPC, foreground/process queries, owner proof,
    // or the sender, then leave the authored onset eligible.
    let actions = [
        raw_zero_action(750, ActionKind::Down, 0, &[0x15]),
        raw_zero_action(751, ActionKind::Up, 100_000, &[0x15]),
    ];
    let mut modifier_blocked = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = modifier_blocked.qpc_now_for_test().expect("epoch");
    modifier_blocked
        .reanchor_playback_clock_for_test(epoch)
        .unwrap();
    modifier_blocked.configure_focus_owner_for_test(1, 177);
    let blocked_packets = modifier_blocked.configure_mock_packet_sender_for_test(&[]);
    modifier_blocked.attach_executor_for_test().unwrap();
    modifier_blocked.set_mock_sender_start_qpc_for_test(epoch);
    modifier_blocked
        .resources
        .backend
        .set_modifier_key_state_query_for_test(|_| i16::MIN);
    reset_counts(&modifier_blocked);
    let candidate = modifier_blocked
        .stage_candidate_for_test(
            0,
            modifier_blocked.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        modifier_blocked.pre_wait_candidate_admission_for_test(&candidate),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(
        modifier_blocked.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Replan
    );
    report_counts(
        "blocked-modifier-replan",
        &modifier_blocked,
        &blocked_packets,
    );
    assert!(
        blocked_packets
            .lock()
            .expect("blocked packet capture")
            .is_empty()
    );
    assert_eq!(
        modifier_blocked
            .current_executor_chord_for_test()
            .unwrap()
            .source_action_index,
        750
    );
    let _ = modifier_blocked.finalize_for_test();
}
