use crate::engine::test_support::ProductionDispatchTestHarness;
use crate::engine::worker::{StagedCandidate, StagedDispatchOutcome};
use sky_dispatch_core::executor::{DispatchDecision, DownChordMetadata};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use sky_dispatch_win32::input::{
    InstrumentPhysicalState, PacketRetryReason, PhysicalPacket, SendEvidence,
    SendTransactionOutcome, SendTransactionStatus,
};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn staged_send_at(
    harness: &mut ProductionDispatchTestHarness,
    at: QpcTicks,
    epoch: QpcTicks,
) -> (u16, Option<DownChordMetadata>, sky_dispatch_core::executor::DispatchTimingWindow) {
    match harness
        .evaluate_live_scheduler_for_test(at, epoch)
        .expect("evaluate production scheduler")
    {
        DispatchDecision::Send {
            up_mask,
            down_chord,
            timing_window,
        } => (up_mask, down_chord.map(Into::into), timing_window),
        decision => panic!("expected packet send at {at:?}, got {decision:?}"),
    }
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

#[path = "dispatch_loop_m2_review_tests.rs"]
mod transport_outcomes;

#[path = "dispatch_loop_m3_up_owner_tests.rs"]
mod owner_proof;

#[path = "dispatch_loop_m4_zero_tests.rs"]
mod proven_raw_zero;

fn qpc_offset(harness: &ProductionDispatchTestHarness, us: u64) -> TimelineTicks {
    TimelineTicks::from_raw(
        harness
            .qpc_duration_from_us_for_test(us)
            .expect("convert authored offset")
            .as_u64(),
    )
}

fn wait_until_qpc(harness: &ProductionDispatchTestHarness, target: QpcTicks) {
    while harness.qpc_now_for_test().expect("wait QPC") < target {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn test_action(index: u32, kind: ActionKind, scheduled_us: u64, scan: u16) -> KeyActionInput {
    KeyActionInput {
        source_action_index: index,
        kind,
        scheduled_us,
        scan_codes: smallvec::smallvec![scan],
        reason: "M2 deterministic dispatch regression".into(),
    }
}

#[test]
fn attempted_mixed_down_is_reconciled_once_then_next_unattempted_onset_waits_for_cleanup_g() {
    let actions = [
        test_action(10, ActionKind::Down, 0, 0x15),
        test_action(11, ActionKind::Up, 20_000, 0x15),
        test_action(12, ActionKind::Down, 20_000, 0x16),
        test_action(13, ActionKind::Down, 21_000, 0x17),
        test_action(14, ActionKind::Up, 40_000, 0x16),
        test_action(15, ActionKind::Up, 42_000, 0x17),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor at current QPC");
    let first_capture = harness.configure_mock_packet_sender_for_test(&[0]);
    let (cleanup_capture, cleanup_completion_qpc) =
        harness.configure_mock_cleanup_emitter_with_completion_qpc_for_test();
    harness.attach_executor_for_test().expect("attach executor");

    let first_now = harness.qpc_now_for_test().expect("first Down QPC");
    let first = harness
        .stage_candidate_for_test(0, harness.current_executor_chord_for_test(), TimelineTicks::ZERO)
        .expect("stage authored onset 10")
        .expect("first Down candidate");
    assert_eq!(first.source_action_index, 10);
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(harness.generation_accounting_for_test().activated, 1);

    let mixed_target = epoch
        .checked_add_duration(DurationTicks::from_raw(qpc_offset(&harness, 20_000).as_u64()))
        .expect("mixed packet target");
    while harness.qpc_now_for_test().expect("wait for mixed target") < mixed_target {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let (up_mask, down_chord, timing_window) = staged_send_at(&mut harness, mixed_target, epoch);
    assert_eq!(up_mask, 0b0001, "authored A Up stays before B Down");
    assert_eq!(down_chord.map(|chord| chord.source_action_index), Some(12));
    let mixed = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage mixed packet")
        .expect("mixed candidate");
    assert_eq!(mixed.source_action_index, 12);
    assert_eq!(mixed.packet_masks, PhysicalPacket::new(0b0001, 0b0010));

    let anomaly_packets = Arc::new(Mutex::new(Vec::with_capacity(1)));
    let captured_anomaly_packets = Arc::clone(&anomaly_packets);
    let anomaly_calls = Arc::new(AtomicUsize::new(0));
    let captured_calls = Arc::clone(&anomaly_calls);
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        captured_calls.fetch_add(1, Ordering::Relaxed);
        captured_anomaly_packets
            .lock()
            .expect("mixed packet syscall capture")
            .push(packet);
        let started = clock.now().expect("mixed pre-call QPC");
        SendTransactionOutcome {
            status: SendTransactionStatus::ClockFailureAfterSend,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                // The first INPUT was Up(A); Down(B) may or may not have
                // reached the target, so the whole Down onset is consumed.
                confirmed_mask: packet.up_mask,
                skipped_mask: 0,
                first_inserted: 1,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: Some(0),
                last_win32_error: Some(0),
                started_ticks: Some(started),
                completed_ticks: None,
                timing_error: Some(sky_dispatch_win32::clock::QpcError::CounterUnavailable),
            },
        }
    });
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(mixed, false),
        StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(anomaly_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        anomaly_packets.lock().expect("mixed packet capture").as_slice(),
        &[PhysicalPacket::new(0b0001, 0b0010)]
    );
    assert_eq!(first_capture.lock().expect("first packet capture").len(), 1);
    assert_eq!(harness.current_executor_chord_for_test().map(|c| c.source_action_index), Some(13));
    assert_eq!(harness.generation_accounting_for_test().activated, 1);
    assert_eq!(harness.generation_accounting_for_test().cancelled, 2);
    assert_eq!(harness.generation_accounting_for_test().active, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    let anomaly = harness
        .runtime
        .attempted_dispatch_anomalies
        .details()
        .first()
        .expect("preserved raw anomaly evidence");
    assert!(anomaly.contains("source_action=12"), "{anomaly}");
    assert!(anomaly.contains("up_mask=0x0001 down_mask=0x0002"), "{anomaly}");
    assert!(anomaly.contains("attempts=1 status=ClockFailureAfterSend"), "{anomaly}");
    assert!(anomaly.contains("completion_qpc=unavailable"), "{anomaly}");
    assert!(!cleanup_capture
        .lock()
        .expect("cleanup capture")
        .is_empty());
    let cleanup_completion =
        QpcTicks::from_raw(cleanup_completion_qpc.load(Ordering::Acquire));
    let barrier = harness
        .resume_barrier_qpc_for_test()
        .expect("cleanup establishes H/G resume barrier");
    assert!(barrier >= cleanup_completion);
    assert!(barrier > mixed_target, "C is still inside cleanup + G");
    assert_eq!(
        harness
            .evaluate_live_scheduler_for_test(
                QpcTicks::from_raw(barrier.as_u64() - 1),
                epoch,
            )
            .expect("evaluate just before resume barrier"),
        DispatchDecision::Wait { deadline_qpc: barrier }
    );

    let (next_up, next_chord, next_window) = staged_send_at(&mut harness, barrier, epoch);
    assert_eq!(next_up, 0);
    assert_eq!(next_chord.map(|chord| chord.source_action_index), Some(13));
    let next = harness
        .stage_candidate_with_scheduler_window_for_test(next_up, next_chord, next_window)
        .expect("stage never-attempted onset 13")
        .expect("next Down candidate");
    assert_eq!(next.source_action_index, 13);
    let next_packets = Arc::new(Mutex::new(Vec::with_capacity(1)));
    let captured_next_packets = Arc::clone(&next_packets);
    let next_start = barrier;
    harness.resources.backend.set_packet_emitter(move |packet| {
        captured_next_packets
            .lock()
            .expect("next onset syscall capture")
            .push(packet);
        let completed = next_start
            .checked_add_duration(DurationTicks::from_raw(1))
            .expect("next Down completion QPC");
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
                started_ticks: Some(next_start),
                completed_ticks: Some(completed),
                timing_error: None,
            },
        }
    });
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(next, false),
        StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(
        next_packets.lock().expect("next packet capture").as_slice(),
        &[PhysicalPacket::new(0, 0b0100)]
    );
    assert_eq!(harness.generation_accounting_for_test().activated, 2);
    assert_eq!(harness.generation_accounting_for_test().cancelled, 2);
    assert_eq!(harness.generation_accounting_for_test().active, 1);
}

#[test]
fn failed_post_call_cleanup_keeps_attempted_down_blocked_with_pending_release_obligation() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let _cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let candidate = harness
        .stage_candidate_for_test(0, harness.current_executor_chord_for_test(), TimelineTicks::ZERO)
        .expect("stage Down")
        .expect("Down candidate");
    let mask = candidate.packet_masks.down_mask;
    harness
        .resources
        .backend
        .set_probe(|_, _| InstrumentPhysicalState::Inconclusive);
    let calls = Arc::new(AtomicUsize::new(0));
    let captured_calls = Arc::clone(&calls);
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        captured_calls.fetch_add(1, Ordering::Relaxed);
        let now = clock.now().expect("attempt QPC");
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
                started_ticks: Some(now),
                // The raw count looks like zero progress, but contradictory
                // QPC ordering keeps this on the conservative cleanup path.
                completed_ticks: Some(QpcTicks::from_raw(
                    now.as_u64().saturating_sub(1),
                )),
                timing_error: None,
            },
        }
    });

    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(harness.current_executor_chord_for_test().map(|c| c.source_action_index), Some(0));
    assert_eq!(harness.resources.scheduler.accounting().activated, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, mask);
    assert_eq!(harness.release_obligation_mask_for_test() & mask, mask);
    assert!(harness.runtime.musical_physical_attempt_started);
    assert!(harness
        .runtime
        .terminal_error
        .as_deref()
        .is_some_and(|error| error.contains("tracked safety cleanup failed")));
    assert_eq!(harness.runtime.attempted_dispatch_anomalies.retained_count(), 1);
}

#[test]
fn musical_up_only_final_focus_loss_replans_without_crossing_sender_boundary() {
    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    let actions = [
        test_action(20, ActionKind::Down, 0, 0x15),
        test_action(21, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let first_now = harness.qpc_now_for_test().expect("first Down QPC");
    let first = harness
        .stage_candidate_for_test(0, harness.current_executor_chord_for_test(), TimelineTicks::ZERO)
        .expect("stage initial Down")
        .expect("Down candidate");
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    ));

    let up_target = epoch
        .checked_add_duration(DurationTicks::from_raw(qpc_offset(&harness, 20_000).as_u64()))
        .expect("Up target");
    while harness.qpc_now_for_test().expect("wait for Up") < up_target {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let (up_mask, down, window) = staged_send_at(&mut harness, up_target, epoch);
    assert_eq!(up_mask, 1);
    assert!(down.is_none());
    let up = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down, window)
        .expect("stage musical Up-only")
        .expect("Up candidate");
    assert_eq!(up.source_action_index, 21);
    let focus = Arc::clone(&harness.focus_active);
    harness.set_final_gate_before_syscall_hook(move || {
        focus.store(false, Ordering::Release);
        sky_dispatch_win32::focus::set_foreground_window_for_test(Some(2));
        sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));
    });
    harness.set_mock_sender_start_qpc_for_test(up_target);

    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(up, false),
        StagedDispatchOutcome::Replan
    ));
    assert_eq!(packets.lock().expect("sender capture").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.local_metrics.final_gate_focus_losses, 1);
    let finalized = harness.finalize_for_test();
    assert_eq!(finalized.attempted_mask, 1);
    assert_eq!(finalized.release_obligation_mask, 0);
    assert_eq!(
        cleanup.lock().expect("safety cleanup capture").as_slice(),
        &[(vec![0x15], true)],
        "terminal tracked safety Up remains available while musical focus is lost"
    );
}

#[test]
fn focus_loss_before_first_down_keeps_onset_unattempted_until_safe_resume() {
    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    let actions = [
        test_action(25, ActionKind::Down, 0, 0x15),
        test_action(26, ActionKind::Up, 30_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.configure_focus_owner_for_test(1, 125);
    let packets = harness.configure_mock_packet_sender_for_test(&[0]);
    harness.attach_executor_for_test().expect("attach executor");
    let candidate = harness
        .stage_candidate_for_test(0, harness.current_executor_chord_for_test(), TimelineTicks::ZERO)
        .expect("stage first Down")
        .expect("Down candidate");
    let focus = Arc::clone(&harness.focus_active);
    harness.set_final_gate_before_syscall_hook(move || {
        focus.store(false, Ordering::Release);
        sky_dispatch_win32::focus::set_foreground_window_for_test(Some(2));
        sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));
    });

    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Replan
    ));
    assert!(packets.lock().expect("sender capture").is_empty());
    assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert!(harness
        .resources
        .playback
        .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus));

    harness.runtime.final_gate_before_syscall_hook = None;
    harness.focus_active.store(true, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(1));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(125)));
    let restored_at = harness.qpc_now_for_test().expect("focus restore QPC");
    harness
        .resources
        .playback
        .exit_pause(sky_dispatch_core::clock::PauseReason::Focus, restored_at)
        .expect("close focus pause interval");
    harness.progress_clock.publish(&harness.resources.playback);
    let resumed = harness
        .stage_candidate_for_test(0, harness.current_executor_chord_for_test(), TimelineTicks::ZERO)
        .expect("re-stage same onset after focus restore")
        .expect("same unattempted Down");
    assert_eq!(resumed.source_action_index, 25);
    harness.set_mock_sender_start_qpc_for_test(restored_at);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(resumed, false),
        StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(packets.lock().expect("sender capture").len(), 1);
    assert_eq!(harness.resources.scheduler.accounting().activated, 1);
}

#[test]
fn attempted_musical_up_only_is_not_replayed_and_next_down_waits_for_verified_cleanup() {
    let actions = [
        test_action(40, ActionKind::Down, 0, 0x15),
        test_action(41, ActionKind::Up, 20_000, 0x15),
        test_action(42, ActionKind::Down, 30_000, 0x16),
        test_action(43, ActionKind::Up, 50_000, 0x16),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    let first_capture = harness.configure_mock_packet_sender_for_test(&[0]);
    let (cleanup_capture, cleanup_completion) =
        harness.configure_mock_cleanup_emitter_with_completion_qpc_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let first = harness
        .stage_candidate_for_test(0, harness.current_executor_chord_for_test(), TimelineTicks::ZERO)
        .expect("stage A Down")
        .expect("A candidate");
    let first_now = harness.qpc_now_for_test().expect("A sender QPC");
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    ));

    let up_target = epoch
        .checked_add_duration(DurationTicks::from_raw(qpc_offset(&harness, 20_000).as_u64()))
        .expect("authored A Up target");
    while harness.qpc_now_for_test().expect("wait for A Up") < up_target {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let (up_mask, down_chord, timing_window) = staged_send_at(&mut harness, up_target, epoch);
    assert_eq!(up_mask, 1);
    assert!(down_chord.is_none());
    let up = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage A Up-only")
        .expect("Up-only packet");
    assert_eq!(up.source_action_index, 41);

    let up_packets = Arc::new(Mutex::new(Vec::with_capacity(1)));
    let captured_up = Arc::clone(&up_packets);
    let clock = harness.resources.clock;
    harness.resources.backend.set_packet_emitter(move |packet| {
        captured_up
            .lock()
            .expect("Up-only sender capture")
            .push(packet);
        let started = clock.now().expect("Up-only pre-call QPC");
        SendTransactionOutcome {
            status: SendTransactionStatus::IntegrityLost,
            evidence: SendEvidence {
                requested_mask: packet.up_mask,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: 0,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::PartialProgress { inserted_count: 0 },
                first_win32_error: Some(5),
                last_win32_error: Some(5),
                started_ticks: Some(started),
                completed_ticks: None,
                timing_error: Some(sky_dispatch_win32::clock::QpcError::CounterUnavailable),
            },
        }
    });
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(up, false),
        StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(up_packets.lock().expect("Up-only capture").as_slice(), &[PhysicalPacket::new(1, 0)]);
    assert_eq!(first_capture.lock().expect("initial Down capture").len(), 1);
    assert_eq!(harness.current_executor_chord_for_test().map(|c| c.source_action_index), Some(42));
    assert_eq!(harness.generation_accounting_for_test().activated, 1);
    assert_eq!(harness.generation_accounting_for_test().released, 0);
    assert_eq!(harness.generation_accounting_for_test().cancelled, 1);
    assert_eq!(harness.generation_accounting_for_test().active, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    let anomaly = harness
        .runtime
        .attempted_dispatch_anomalies
        .details()
        .first()
        .expect("Up anomaly");
    assert!(anomaly.contains("source_action=41"), "{anomaly}");
    assert!(anomaly.contains("up_mask=0x0001 down_mask=0x0000"), "{anomaly}");
    assert!(anomaly.contains("attempts=1 status=IntegrityLost"), "{anomaly}");
    let barrier = harness.resume_barrier_qpc_for_test().expect("verified cleanup + G");
    assert!(barrier > up_target);
    assert!(!cleanup_capture.lock().expect("cleanup capture").is_empty());
    assert!(QpcTicks::from_raw(cleanup_completion.load(Ordering::Acquire)) <= barrier);

    let (next_up, next_chord, next_window) = staged_send_at(&mut harness, barrier, epoch);
    assert_eq!(next_up, 0);
    assert_eq!(next_chord.map(|chord| chord.source_action_index), Some(42));
    let next = harness
        .stage_candidate_with_scheduler_window_for_test(next_up, next_chord, next_window)
        .expect("stage B after cleanup")
        .expect("B onset remains pending");
    assert_eq!(next.source_action_index, 42);
    let b_start = barrier;
    let b_packets = Arc::new(Mutex::new(Vec::with_capacity(1)));
    let captured_b = Arc::clone(&b_packets);
    harness.resources.backend.set_packet_emitter(move |packet| {
        captured_b
            .lock()
            .expect("B sender capture")
            .push(packet);
        let completed = b_start
            .checked_add_duration(DurationTicks::from_raw(1))
            .expect("B completion QPC");
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
                started_ticks: Some(b_start),
                completed_ticks: Some(completed),
                timing_error: None,
            },
        }
    });
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(next, false),
        StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(b_packets.lock().expect("B sender capture").as_slice(), &[PhysicalPacket::new(0, 2)]);
}

#[test]
fn target_generation_race_and_modifier_race_leave_down_unattempted() {
    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    let actions = [
        test_action(30, ActionKind::Down, 0, 0x15),
        test_action(31, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.configure_focus_owner_for_test(1, 111);
    let packets = harness.configure_mock_packet_sender_for_test(&[0]);
    harness.attach_executor_for_test().expect("attach executor");
    let stale = harness
        .stage_candidate_for_test(0, harness.current_executor_chord_for_test(), TimelineTicks::ZERO)
        .expect("stage target-bound Down")
        .expect("Down candidate");
    harness.republish_focus_owner_for_test(2, 222);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(stale, false),
        StagedDispatchOutcome::Replan
    ));
    assert!(packets.lock().expect("sender capture").is_empty());
    assert_eq!(harness.resources.scheduler.chord_cursor(), 0);

    // Re-stage against the current HWND/PID/generation, then mutate modifier
    // state after owner proof and immediately before the bounded final suffix.
    let current = harness
        .stage_candidate_for_test(0, harness.current_executor_chord_for_test(), TimelineTicks::ZERO)
        .expect("re-stage current target")
        .expect("Down candidate");
    let held = Arc::new(AtomicBool::new(false));
    let held_for_query = Arc::clone(&held);
    harness
        .resources
        .backend
        .set_modifier_key_state_query_for_test(move |_| {
            if held_for_query.load(Ordering::Acquire) {
                i16::MIN
            } else {
                0
            }
        });
    let held_for_gate = Arc::clone(&held);
    harness.set_final_gate_before_syscall_hook(move || {
        held_for_gate.store(true, Ordering::Release);
    });
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(current, false),
        StagedDispatchOutcome::Replan
    ));
    assert!(packets.lock().expect("sender capture").is_empty());
    assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
    assert_eq!(harness.local_metrics.final_gate_control_rejections, 1);
    held.store(false, Ordering::Release);
    harness.runtime.final_gate_before_syscall_hook = None;
    let recovered = harness
        .stage_candidate_for_test(0, harness.current_executor_chord_for_test(), TimelineTicks::ZERO)
        .expect("replan after modifier release")
        .expect("same unattempted Down candidate");
    assert_eq!(recovered.source_action_index, 30);
    harness.set_mock_sender_start_qpc_for_test(harness.qpc_now_for_test().unwrap());
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(recovered, false),
        StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(packets.lock().expect("sender capture").len(), 1);
}

#[test]
fn mixed_packet_with_final_modifier_rejection_replans_as_up_only() {
    let actions = [
        test_action(32, ActionKind::Down, 0, 0x15),
        test_action(33, ActionKind::Up, 20_000, 0x15),
        test_action(34, ActionKind::Down, 20_000, 0x16),
        test_action(35, ActionKind::Up, 40_000, 0x16),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = harness
        .stage_candidate_for_test(0, harness.current_executor_chord_for_test(), TimelineTicks::ZERO)
        .expect("stage first Down")
        .expect("first Down candidate");
    let first_now = harness.qpc_now_for_test().expect("first Down QPC");
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    ));

    let mixed_target = epoch
        .checked_add_duration(DurationTicks::from_raw(qpc_offset(&harness, 20_000).as_u64()))
        .expect("mixed target");
    while harness.qpc_now_for_test().expect("wait for mixed target") < mixed_target {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let (up_mask, down_chord, timing_window) = staged_send_at(&mut harness, mixed_target, epoch);
    assert_eq!(up_mask, 1);
    assert_eq!(down_chord.map(|chord| chord.source_action_index), Some(34));
    let mixed = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage mixed packet")
        .expect("mixed candidate");
    assert_eq!(mixed.packet_masks, PhysicalPacket::new(1, 2));

    let modifier_held = Arc::new(AtomicBool::new(false));
    let held_for_query = Arc::clone(&modifier_held);
    harness
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
    harness.set_final_gate_before_syscall_hook(move || {
        held_for_gate.store(true, Ordering::Release);
    });
    harness.set_mock_sender_start_qpc_for_test(mixed_target);

    assert!(
        matches!(
            harness.dispatch_staged_candidate_for_test(mixed, false),
            StagedDispatchOutcome::Dispatched
        ),
        "modifier rejection of mixed Down replans the independent Up"
    );
    assert_eq!(
        packets
            .lock()
            .expect("mixed fallback packet capture")
            .iter()
            .map(|(packet, _)| *packet)
            .collect::<Vec<_>>(),
        vec![PhysicalPacket::new(0, 1), PhysicalPacket::new(1, 0)],
        "the rejected B Down is absent and A Up remains authored before any later Down"
    );
    assert_eq!(harness.current_executor_chord_for_test().map(|chord| chord.source_action_index), Some(34));
    assert_eq!(harness.generation_accounting_for_test().activated, 1);
    assert_eq!(harness.generation_accounting_for_test().released, 1);
    assert_eq!(harness.generation_accounting_for_test().active, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
}

#[test]
fn terminal_control_races_at_final_gate_do_not_attempt_or_advance_down_onset() {
    #[derive(Clone, Copy)]
    enum Race {
        Quit,
        Skip,
        PanicRelease,
        SystemSuspend,
        LeaseExpiry,
    }

    for race in [
        Race::Quit,
        Race::Skip,
        Race::PanicRelease,
        Race::SystemSuspend,
        Race::LeaseExpiry,
    ] {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let packets = harness.configure_mock_packet_sender_for_test(&[0]);
        harness.attach_executor_for_test().expect("attach executor");
        let candidate = harness
            .stage_candidate_for_test(
                0,
                harness.current_executor_chord_for_test(),
                TimelineTicks::ZERO,
            )
            .expect("stage Down")
            .expect("Down candidate");
        match race {
            Race::Quit | Race::Skip | Race::SystemSuspend => {
                let interrupt = Arc::clone(&harness.interrupt);
                harness.set_post_focus_revalidation_race_hook(move |_, _, quit, skip, _, _, power| {
                    match race {
                        Race::Quit => quit.store(true, Ordering::Release),
                        Race::Skip => skip.store(true, Ordering::Release),
                        Race::SystemSuspend => {
                            assert!(power.notify(true, &interrupt));
                        }
                        Race::PanicRelease => unreachable!(),
                        Race::LeaseExpiry => unreachable!(),
                    }
                });
            }
            Race::PanicRelease => harness.panic_requested.store(true, Ordering::Release),
            Race::LeaseExpiry => harness.supervisor_expired.latch_expired(),
        }
        assert!(matches!(
            harness.dispatch_staged_candidate_for_test(candidate, false),
            StagedDispatchOutcome::Replan
        ));
        assert!(packets.lock().expect("sender capture").is_empty());
        assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
        assert_eq!(harness.local_metrics.precision_packet_attempts, 0);
        assert!(
            harness.process_command_control_for_test(),
            "{race_name}: terminal control is still fail-closed after final gate",
            race_name = match race {
                Race::Quit => "quit",
                Race::Skip => "skip",
                Race::PanicRelease => "panic release",
                Race::SystemSuspend => "system suspend",
                Race::LeaseExpiry => "lease expiry",
            }
        );
    }
}
