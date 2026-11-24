use super::replan_timing::stage_from_worker_outer_evaluation;
use super::support::{qpc_offset, staged_send_at, test_action};
use crate::engine::test_support::ProductionDispatchTestHarness;
use crate::engine::worker::{FinalDownAdmissionResult, StagedDispatchOutcome};
use sky_dispatch_core::executor::DispatchDecision;
use sky_dispatch_core::model::ActionKind;
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use sky_dispatch_win32::input::{
    InstrumentPhysicalState, PacketRetryReason, PhysicalPacket, SendEvidence,
    SendTransactionOutcome, SendTransactionStatus,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

#[test]
fn attempted_mixed_down_terminates_after_one_packet_and_verified_cleanup() {
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
    let cleanup_capture = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");

    let first_now = harness.qpc_now_for_test().expect("first Down QPC");
    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
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
        .checked_add_duration(DurationTicks::from_raw(
            qpc_offset(&harness, 20_000).as_u64(),
        ))
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
        StagedDispatchOutcome::Exit
    ));
    assert_eq!(anomaly_calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        anomaly_packets
            .lock()
            .expect("mixed packet capture")
            .as_slice(),
        &[PhysicalPacket::new(0b0001, 0b0010)]
    );
    assert_eq!(first_capture.lock().expect("first packet capture").len(), 1);
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|c| c.source_action_index),
        Some(13)
    );
    assert_eq!(harness.generation_accounting_for_test().activated, 1);
    assert_eq!(harness.generation_accounting_for_test().cancelled, 2);
    assert_eq!(harness.generation_accounting_for_test().active, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    let anomaly = harness
        .runtime
        .attempted_dispatch_anomaly
        .detail()
        .expect("preserved raw anomaly evidence");
    assert!(anomaly.contains("source_action=12"), "{anomaly}");
    assert!(
        anomaly.contains("up_mask=0x0001 down_mask=0x0002"),
        "{anomaly}"
    );
    assert!(
        anomaly.contains("attempts=1 status=ClockFailureAfterSend"),
        "{anomaly}"
    );
    assert!(anomaly.contains("completion_qpc=unavailable"), "{anomaly}");
    assert!(!cleanup_capture.lock().expect("cleanup capture").is_empty());

    assert_eq!(anomaly_calls.load(Ordering::Relaxed), 1);
    assert_eq!(first_capture.lock().expect("first packet capture").len(), 1);
    assert_eq!(
        harness.finalize_for_test().outcome_code,
        crate::engine::OUTCOME_ERROR
    );
}

#[test]
fn failed_post_call_cleanup_keeps_attempted_down_blocked_with_pending_release_obligation() {
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let _cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let candidate = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
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
                completed_ticks: Some(QpcTicks::from_raw(now.as_u64().saturating_sub(1))),
                timing_error: None,
            },
        }
    });

    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|c| c.source_action_index),
        Some(0)
    );
    assert_eq!(harness.resources.scheduler.accounting().activated, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test() & mask, mask);
    assert!(harness.runtime.musical_physical_attempt_started);
    assert!(
        harness
            .runtime
            .terminal_error
            .as_deref()
            .is_some_and(|error| error.contains("tracked safety cleanup failed"))
    );
    assert!(harness.runtime.attempted_dispatch_anomaly.is_recorded());
}

#[test]
fn musical_up_only_fresh_foreground_loss_pauses_without_crossing_sender_boundary() {
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
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage initial Down")
        .expect("Down candidate");
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    ));

    let up_target = epoch
        .checked_add_duration(DurationTicks::from_raw(
            qpc_offset(&harness, 20_000).as_u64(),
        ))
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
    harness.set_final_gate_before_syscall_hook(move || {
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
    assert!(harness.focus_active.load(Ordering::Acquire));
    assert!(
        harness
            .resources
            .playback
            .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus)
    );
    assert_eq!(harness.runtime.verified_target_for_test(), None);
    let finalized = harness.finalize_for_test();
    assert_eq!(finalized.attempted_mask, 1);
    assert_eq!(finalized.release_obligation_mask, 1);
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
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
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
    assert!(
        harness
            .resources
            .playback
            .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus)
    );

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
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
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
fn fresh_foreground_rejection_enters_focus_pause_and_recovers_after_grace_cleanup() {
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
        test_action(6660, ActionKind::Down, 0, 0x15),
        test_action(6661, ActionKind::Down, 150_000, 0x16),
        test_action(6662, ActionKind::Up, 500_000, 0x15),
        test_action(6663, ActionKind::Up, 600_000, 0x16),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let clock = harness.resources.clock;
    let sampled_now = clock.now().expect("current QPC");
    let historical_offset = clock
        .duration_from_us(250_000)
        .expect("historical playback offset");
    let epoch = QpcTicks::from_raw(
        sampled_now
            .as_u64()
            .checked_sub(historical_offset.as_u64())
            .expect("historical playback epoch"),
    );
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback before B is due");
    harness.configure_focus_owner_for_test(1, 101);
    harness.timing.focus_restore_grace_ticks = clock
        .duration_from_us(10_000)
        .expect("focus restoration grace");
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");

    let origin = super::super::load_target_stamp(&harness.target).expect("original owner stamp");
    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage committed A")
        .expect("A candidate");
    assert_eq!(first.packet_masks, PhysicalPacket::new(0, 1));
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.backend.active_mask, 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    let a_origin = harness
        .resources
        .backend
        .release_owner_proof_for_mask(1)
        .expect("A release proof lookup")
        .expect("A release proof committed")
        .clone();
    assert_eq!(a_origin.target_hwnd(), origin.hwnd);
    assert_eq!(a_origin.target_generation(), origin.generation);
    assert_eq!(a_origin.owner_pid(), Some(101));

    let b_eval_qpc = clock.now().expect("due B scheduler QPC");
    let (b, evaluations) = stage_from_worker_outer_evaluation(&mut harness, b_eval_qpc);
    assert_eq!(evaluations, 1);
    assert_eq!(b.source_action_index, 6661);
    assert_eq!(b.packet_masks, PhysicalPacket::new(0, 2));
    assert!(b.timing_window.packet_not_before_qpc <= b_eval_qpc);
    assert_eq!(
        harness.pre_wait_candidate_admission_for_test(&b),
        StagedDispatchOutcome::Dispatched,
        "monitor-published focus is still the original HWND"
    );

    // The foreground leaves and returns before the monitor publishes a new
    // hint. The final owner proof observes the brief loss directly.
    sky_dispatch_win32::focus::reset_foreground_query_count();
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(2));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));
    let admission = super::super::verify_final_down_admission(
        &harness.target,
        origin,
        true,
        &harness.focus_active,
        super::super::FinalControlSignals {
            quit_requested: &harness.quit_requested,
            skip_requested: &harness.skip_requested,
            panic_requested: &harness.panic_requested,
            desired_pause: &harness.desired_pause,
            supervisor_expired: &harness.supervisor_expired,
            system_power: Some(&harness.system_power),
        },
        harness.timing.lease_timeout_ticks,
        clock,
        &harness.progress_clock,
        &harness.resources.backend,
        &mut harness.resources.playback,
        &mut harness.runtime,
        &mut harness.local_metrics,
    );
    assert!(matches!(admission, FinalDownAdmissionResult::Replan));
    drop(admission);
    assert_eq!(
        sky_dispatch_win32::focus::foreground_query_count(),
        1,
        "the fresh gate performs one foreground-owner query"
    );
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(1));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(101)));

    assert!(harness.focus_active.load(Ordering::Acquire));
    assert!(
        harness
            .resources
            .playback
            .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus)
    );
    assert_eq!(harness.runtime.verified_target_for_test(), None);
    assert_eq!(harness.runtime.focus_restore_started_ticks, None);
    assert_eq!(harness.local_metrics.final_gate_focus_losses, 1);
    assert_eq!(harness.local_metrics.precision_packet_attempts, 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.generation_accounting_for_test().activated, 1);
    assert_eq!(harness.resources.backend.active_mask, 1);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert!(
        harness
            .resources
            .backend
            .release_owner_proof_for_mask(1)
            .expect("A proof lookup after rejection")
            .expect("A proof remains after rejection")
            .same_owner(&a_origin),
        "A keeps the frozen original HWND/PID/generation proof"
    );
    {
        let sent = packets.lock().expect("packet capture");
        assert_eq!(sent.len(), 1, "B never reached the sender");
        assert_eq!(sent[0].0, PhysicalPacket::new(0, 1));
    }
    let paused_progress = harness
        .progress_clock
        .load()
        .expect("published Focus pause progress");
    assert!(paused_progress.paused);
    assert!(paused_progress.pause_started_qpc.is_some());

    // Recovery starts its grace clock only after the foreground has returned.
    let restored_at = clock.now().expect("focus restoration observation QPC");
    harness.runtime.focus_restore_started_ticks = Some(restored_at);
    let grace_deadline = super::super::paused_focus_grace_deadline(
        true,
        harness.focus_active.load(Ordering::Acquire),
        super::super::load_target_stamp(&harness.target).is_some(),
        harness.runtime.focus_restore_started_ticks,
        harness.timing.focus_restore_grace_ticks,
    )
    .expect("focus grace deadline calculation")
    .expect("restored focus has a grace deadline");
    assert_eq!(
        grace_deadline,
        restored_at
            .checked_add_duration(harness.timing.focus_restore_grace_ticks)
            .expect("focus grace arithmetic")
    );
    assert_eq!(cleanup.lock().expect("no early safety cleanup").len(), 0);
    while clock.now().expect("focus grace QPC") < grace_deadline {
        std::thread::yield_now();
    }

    let order = Arc::new(Mutex::new(Vec::with_capacity(2)));
    let order_for_cleanup = Arc::clone(&order);
    let cleanups = Arc::new(Mutex::new(Vec::with_capacity(1)));
    let cleanups_for_emitter = Arc::clone(&cleanups);
    harness
        .resources
        .backend
        .set_emitter(move |scan_codes, key_up| {
            order_for_cleanup
                .lock()
                .expect("recovery order lock")
                .push("cleanup");
            cleanups_for_emitter
                .lock()
                .expect("recovery cleanup capture")
                .push((scan_codes.to_vec(), key_up));
            let now = clock.now().expect("verified cleanup QPC");
            sky_dispatch_win32::input::PlatformSendResult {
                requested: scan_codes.len() as u8,
                inserted: scan_codes.len() as u8,
                started_ticks: now,
                completed_ticks: Some(now),
                win32_error: 0,
                timing_error: None,
            }
        });
    let preflight_calls = Arc::new(AtomicUsize::new(0));
    let preflight_calls_for_probe = Arc::clone(&preflight_calls);
    let order_for_preflight = Arc::clone(&order);
    harness.set_preflight_physical_mask_probe_for_test(move || {
        order_for_preflight
            .lock()
            .expect("recovery order lock")
            .push("preflight");
        preflight_calls_for_probe.fetch_add(1, Ordering::AcqRel);
        0
    });

    super::super::suspend_live_input_for_dispatch(
        &mut harness.resources.backend,
        &mut harness.resources.scheduler,
        super::super::super::cleanup::CleanupTarget::for_session(
            &harness.target,
            Some(origin),
            true,
        ),
    )
    .expect("verified cleanup under A's frozen owner proof");
    assert_eq!(
        cleanups
            .lock()
            .expect("verified cleanup capture")
            .as_slice(),
        &[(vec![0x15], true)]
    );
    assert_eq!(
        order.lock().expect("cleanup before preflight").as_slice(),
        &["cleanup"]
    );
    assert_eq!(harness.resources.backend.active_mask, 0);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert!(
        harness
            .resources
            .backend
            .release_owner_proof_for_mask(1)
            .expect("session owner proof lookup after verified cleanup")
            .expect("session owner proof remains after verified cleanup")
            .same_owner(&a_origin),
        "physical cleanup does not retire the session authority proof"
    );

    let current = super::super::load_target_stamp(&harness.target).expect("same restored owner");
    assert_eq!(current, origin);
    super::super::ensure_preflight_for_target(
        &harness.resources.backend,
        current,
        &mut harness.runtime.verified_target,
    )
    .expect("focus recovery preflight after cleanup");
    assert_eq!(preflight_calls.load(Ordering::Acquire), 1);
    assert_eq!(
        order.lock().expect("cleanup then preflight").as_slice(),
        &["cleanup", "preflight"]
    );
    assert_eq!(harness.runtime.verified_target_for_test(), Some(origin));

    let resumed_at = clock.now().expect("resume clock QPC");
    harness
        .resources
        .playback
        .exit_pause(sky_dispatch_core::clock::PauseReason::Focus, resumed_at)
        .expect("close Focus pause after grace and cleanup");
    harness.progress_clock.publish(&harness.resources.playback);
    harness.runtime.focus_restore_started_ticks = None;
    assert!(!harness.resources.playback.is_paused());
    assert!(
        !harness
            .progress_clock
            .load()
            .expect("resumed progress")
            .paused
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.generation_accounting_for_test().cancelled, 1);
    assert_eq!(harness.resources.scheduler.active_mask(), 0);

    assert!(matches!(
        harness
            .evaluate_live_scheduler_for_test(resumed_at, harness.resources.playback.epoch)
            .expect("B after Resume commits"),
        DispatchDecision::Send {
            up_mask: 0,
            down_chord: Some(chord),
            timing_window,
        } if chord.source_action_index == 6661
            && timing_window.packet_not_before_qpc <= resumed_at
    ));
    let resumed_qpc = clock.now().expect("resumed B scheduler QPC");
    let (b, evaluations) = stage_from_worker_outer_evaluation(&mut harness, resumed_qpc);
    assert_eq!(evaluations, 1);
    assert_eq!(b.source_action_index, 6661);
    assert_eq!(b.packet_masks, PhysicalPacket::new(0, 2));
    assert!(b.timing_window.packet_not_before_qpc <= resumed_qpc);
    assert_eq!(
        harness.pre_wait_candidate_admission_for_test(&b),
        StagedDispatchOutcome::Dispatched
    );
    harness.set_mock_sender_start_qpc_for_test(resumed_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(b, false),
        StagedDispatchOutcome::Dispatched
    );
    let sent = packets.lock().expect("A then recovered B capture");
    assert_eq!(sent.len(), 2, "B has no pre-recovery sender attempt");
    assert_eq!(sent[0].0, PhysicalPacket::new(0, 1));
    assert_eq!(sent[1].0, PhysicalPacket::new(0, 2));
    assert_eq!(sent[1].1.attempts, 1);
    assert!(sent[1].1.started_ticks.expect("B sender QPC") >= resumed_at);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 2);
    assert_eq!(harness.resources.backend.active_mask, 2);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 2);
    let b_origin = harness
        .resources
        .backend
        .release_owner_proof_for_mask(2)
        .expect("B owner proof after recovery")
        .expect("B release obligation");
    assert_eq!(b_origin.target_hwnd(), origin.hwnd);
    assert_eq!(b_origin.target_generation(), origin.generation);
    assert_eq!(b_origin.owner_pid(), Some(101));
    assert!(harness.runtime.terminal_error.is_none());
}

#[test]
fn cleanup_probe_rechecks_owner_before_accepting_all_up() {
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

    let actions = [
        test_action(6668, ActionKind::Down, 0, 0x15),
        test_action(6669, ActionKind::Up, 500_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    harness.configure_focus_owner_for_test(1, 101);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.resources.backend.active_mask = 1;

    let probe_calls = Arc::new(AtomicUsize::new(0));
    let probe_calls_for_probe = Arc::clone(&probe_calls);
    harness.resources.backend.set_probe(move |_, _| {
        match probe_calls_for_probe.fetch_add(1, Ordering::AcqRel) {
            0 => InstrumentPhysicalState::Inconclusive,
            1 => {
                // Model owner drift after the retry's pre-probe check but
                // before AllUp can be accepted.
                sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));
                InstrumentPhysicalState::AllUp
            }
            _ => InstrumentPhysicalState::AllUp,
        }
    });
    let origin = super::super::load_target_stamp(&harness.target).expect("retained origin");

    let error = super::super::suspend_live_input_for_dispatch(
        &mut harness.resources.backend,
        &mut harness.resources.scheduler,
        super::super::super::cleanup::CleanupTarget::for_session(
            &harness.target,
            Some(origin),
            true,
        ),
    )
    .expect_err("owner drift during the probe cannot clear cleanup debt");

    assert!(error.contains("owner identity drifted"), "{error}");
    assert_eq!(probe_calls.load(Ordering::Acquire), 2);
    assert_eq!(cleanup.lock().expect("cleanup send capture").len(), 1);
    assert_eq!(harness.resources.backend.needs_up_mask(), 0);
    assert_eq!(harness.resources.backend.needs_probe_mask, 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
}

#[test]
fn focus_recovery_cleanup_rejects_invalidated_session_target() {
    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;

    for replacement_hwnd in [2, 1] {
        let actions = [
            test_action(6670, ActionKind::Down, 0, 0x15),
            test_action(6671, ActionKind::Up, 500_000, 0x15),
        ];
        let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
        let epoch = harness.qpc_now_for_test().expect("owner recovery epoch");
        harness
            .reanchor_playback_clock_for_test(epoch)
            .expect("owner recovery epoch");
        harness.configure_focus_owner_for_test(1, 101);
        let packets = harness.configure_mock_packet_sender_for_test(&[0]);
        let cleanup = harness.configure_mock_cleanup_emitter_for_test();
        harness.attach_executor_for_test().expect("attach executor");
        let origin = super::super::load_target_stamp(&harness.target).expect("origin stamp");
        let first = harness
            .stage_candidate_for_test(
                0,
                harness.current_executor_chord_for_test(),
                TimelineTicks::ZERO,
            )
            .expect("stage origin Down")
            .expect("origin Down candidate");
        harness.set_mock_sender_start_qpc_for_test(epoch);
        assert_eq!(
            harness.dispatch_staged_candidate_for_test(first, false),
            StagedDispatchOutcome::Dispatched
        );
        let original_proof = harness
            .resources
            .backend
            .release_owner_proof_for_mask(1)
            .expect("origin proof lookup")
            .expect("origin proof")
            .clone();

        // Neither a different HWND nor same-HWND reuse can replace the fixed
        // session target or authorize focus-recovery cleanup.
        harness.invalidate_focus_owner_for_test(replacement_hwnd, 202);
        let error = super::super::suspend_live_input_for_dispatch(
            &mut harness.resources.backend,
            &mut harness.resources.scheduler,
            super::super::super::cleanup::CleanupTarget::for_session(
                &harness.target,
                Some(origin),
                true,
            ),
        )
        .expect_err("recovery cannot release through an invalidated session target");
        assert!(error.contains("target HWND/generation changed"));
        assert!(cleanup.lock().expect("no cross-owner cleanup").is_empty());
        assert_eq!(packets.lock().expect("only original Down").len(), 1);
        assert_eq!(harness.resources.backend.active_mask, 1);
        assert_eq!(harness.release_obligation_mask_for_test(), 1);
        assert!(
            harness
                .resources
                .backend
                .release_owner_proof_for_mask(1)
                .expect("retained origin proof lookup")
                .expect("retained origin proof")
                .same_owner(&original_proof)
        );
    }
}
#[test]
fn fresh_focus_rejection_keeps_qpc_failure_terminal() {
    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
            sky_dispatch_win32::clock::set_qpc_now_fault_for_test(false);
        }
    }
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    let actions = [
        test_action(6688, ActionKind::Down, 0, 0x15),
        test_action(6689, ActionKind::Up, 100_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness
        .qpc_now_for_test()
        .expect("QPC failure fixture epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("QPC failure fixture epoch");
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[0]);
    harness.attach_executor_for_test().expect("attach executor");
    let expected = super::super::load_target_stamp(&harness.target).expect("current owner");
    let candidate = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage pending onset")
        .expect("Down candidate");
    assert_eq!(candidate.source_action_index, 6688);

    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(2));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));
    sky_dispatch_win32::clock::set_qpc_now_fault_for_test(true);
    let admission = super::super::verify_final_down_admission(
        &harness.target,
        expected,
        true,
        &harness.focus_active,
        super::super::FinalControlSignals {
            quit_requested: &harness.quit_requested,
            skip_requested: &harness.skip_requested,
            panic_requested: &harness.panic_requested,
            desired_pause: &harness.desired_pause,
            supervisor_expired: &harness.supervisor_expired,
            system_power: Some(&harness.system_power),
        },
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.progress_clock,
        &harness.resources.backend,
        &mut harness.resources.playback,
        &mut harness.runtime,
        &mut harness.local_metrics,
    );
    assert!(matches!(
        admission,
        FinalDownAdmissionResult::Terminal(error) if error.contains("QPC failure")
    ));
    assert!(
        !harness
            .resources
            .playback
            .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus)
    );
    assert!(
        packets
            .lock()
            .expect("no sender after QPC failure")
            .is_empty()
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
}

#[test]
fn attempted_musical_up_only_is_not_replayed_and_terminates_before_next_down() {
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
    let cleanup_capture = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");
    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage A Down")
        .expect("A candidate");
    let first_now = harness.qpc_now_for_test().expect("A sender QPC");
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    ));

    let up_target = epoch
        .checked_add_duration(DurationTicks::from_raw(
            qpc_offset(&harness, 20_000).as_u64(),
        ))
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
        StagedDispatchOutcome::Exit
    ));
    assert_eq!(
        up_packets.lock().expect("Up-only capture").as_slice(),
        &[PhysicalPacket::new(1, 0)]
    );
    assert_eq!(first_capture.lock().expect("initial Down capture").len(), 1);
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|c| c.source_action_index),
        Some(42)
    );
    assert_eq!(harness.generation_accounting_for_test().activated, 1);
    assert_eq!(harness.generation_accounting_for_test().released, 0);
    assert_eq!(harness.generation_accounting_for_test().cancelled, 1);
    assert_eq!(harness.generation_accounting_for_test().active, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    let anomaly = harness
        .runtime
        .attempted_dispatch_anomaly
        .detail()
        .expect("Up anomaly");
    assert!(anomaly.contains("source_action=41"), "{anomaly}");
    assert!(
        anomaly.contains("up_mask=0x0001 down_mask=0x0000"),
        "{anomaly}"
    );
    assert!(
        anomaly.contains("attempts=1 status=IntegrityLost"),
        "{anomaly}"
    );
    assert!(!cleanup_capture.lock().expect("cleanup capture").is_empty());

    assert_eq!(up_packets.lock().expect("Up-only capture").len(), 1);
    assert_eq!(first_capture.lock().expect("initial Down capture").len(), 1);
    assert_eq!(
        harness.finalize_for_test().outcome_code,
        crate::engine::OUTCOME_ERROR
    );
}

#[test]
fn target_invalidation_keeps_down_unattempted_and_cannot_restage() {
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
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage target-bound Down")
        .expect("Down candidate");
    harness.invalidate_focus_owner_for_test(2, 222);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(stale, false),
        StagedDispatchOutcome::Replan
    );
    assert!(packets.lock().expect("sender capture").is_empty());
    assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
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
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage first Down")
        .expect("first Down candidate");
    let first_now = harness.qpc_now_for_test().expect("first Down QPC");
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    ));
    let mixed_target = epoch
        .checked_add_duration(DurationTicks::from_raw(
            qpc_offset(&harness, 20_000).as_u64(),
        ))
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
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|chord| chord.source_action_index),
        Some(34)
    );
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
                harness.set_post_focus_revalidation_race_hook(
                    move |_, _, quit, skip, _, _, power| match race {
                        Race::Quit => quit.store(true, Ordering::Release),
                        Race::Skip => skip.store(true, Ordering::Release),
                        Race::SystemSuspend => {
                            assert!(power.notify(true, &interrupt));
                        }
                        Race::PanicRelease => unreachable!(),
                        Race::LeaseExpiry => unreachable!(),
                    },
                );
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

#[test]
fn terminal_control_races_at_final_gate_do_not_attempt_up_only() {
    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;

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
        let actions = [
            test_action(1, ActionKind::Down, 0, 0x15),
            test_action(2, ActionKind::Up, 20_000, 0x15),
        ];
        let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
        let epoch = harness.qpc_now_for_test().expect("test epoch");
        harness
            .reanchor_playback_clock_for_test(epoch)
            .expect("reanchor playback");
        harness.configure_focus_owner_for_test(1, 101);
        let packets = harness.configure_mock_packet_sender_for_test(&[]);
        harness.attach_executor_for_test().expect("attach executor");

        let first = harness
            .stage_candidate_for_test(
                0,
                harness.current_executor_chord_for_test(),
                TimelineTicks::ZERO,
            )
            .expect("stage Down")
            .expect("Down candidate");
        harness.set_mock_sender_start_qpc_for_test(epoch);
        assert_eq!(
            harness.dispatch_staged_candidate_for_test(first, false),
            StagedDispatchOutcome::Dispatched
        );
        assert_eq!(packets.lock().expect("packet capture").len(), 1);

        let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
        let candidate = harness
            .stage_candidate_for_test(
                0x0001,
                None,
                sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
            )
            .expect("stage Up")
            .expect("Up candidate");
        harness.set_mock_sender_start_qpc_for_test(up_epoch);

        match race {
            Race::Quit | Race::Skip | Race::PanicRelease | Race::SystemSuspend => {
                let interrupt = Arc::clone(&harness.interrupt);
                harness.set_post_focus_revalidation_race_hook(
                    move |_, _, quit, skip, panic, _, power| match race {
                        Race::Quit => quit.store(true, Ordering::Release),
                        Race::Skip => skip.store(true, Ordering::Release),
                        Race::PanicRelease => panic.store(true, Ordering::Release),
                        Race::SystemSuspend => {
                            assert!(power.notify(true, &interrupt));
                        }
                        Race::LeaseExpiry => unreachable!(),
                    },
                );
            }
            Race::LeaseExpiry => harness.supervisor_expired.latch_expired(),
        }

        let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
        match race {
            Race::Quit | Race::Skip | Race::PanicRelease | Race::SystemSuspend => {
                assert!(
                    matches!(outcome, StagedDispatchOutcome::Replan),
                    "expected Replan for race {race_name}, got {outcome:?}",
                    race_name = match race {
                        Race::Quit => "Quit",
                        Race::Skip => "Skip",
                        Race::PanicRelease => "PanicRelease",
                        Race::SystemSuspend => "SystemSuspend",
                        _ => unreachable!(),
                    }
                );
            }
            Race::LeaseExpiry => {
                assert!(
                    matches!(outcome, StagedDispatchOutcome::Exit),
                    "expected Exit for LeaseExpiry, got {outcome:?}"
                );
            }
        }
        assert_eq!(
            packets.lock().expect("packet capture").len(),
            1,
            "terminal control race must not send Up packet"
        );
        assert_eq!(
            harness.release_obligation_mask_for_test(),
            1,
            "release obligation must remain latched"
        );
    }
}

#[test]
fn up_only_target_invalidated_or_owner_mismatch_at_crossing_fails_closed() {
    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;

    // Case 1: Owner mismatch at crossing
    {
        let actions = [
            test_action(1, ActionKind::Down, 0, 0x15),
            test_action(2, ActionKind::Up, 20_000, 0x15),
        ];
        let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
        let epoch = harness.qpc_now_for_test().expect("test epoch");
        harness
            .reanchor_playback_clock_for_test(epoch)
            .expect("reanchor playback");
        harness.configure_focus_owner_for_test(1, 101);
        let packets = harness.configure_mock_packet_sender_for_test(&[]);
        harness.attach_executor_for_test().expect("attach executor");

        let first = harness
            .stage_candidate_for_test(
                0,
                harness.current_executor_chord_for_test(),
                TimelineTicks::ZERO,
            )
            .expect("stage Down")
            .expect("Down candidate");
        harness.set_mock_sender_start_qpc_for_test(epoch);
        assert_eq!(
            harness.dispatch_staged_candidate_for_test(first, false),
            StagedDispatchOutcome::Dispatched
        );

        let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
        let candidate = harness
            .stage_candidate_for_test(
                0x0001,
                None,
                sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
            )
            .expect("stage Up")
            .expect("Up candidate");
        harness.set_mock_sender_start_qpc_for_test(up_epoch);

        // Owner PID drifts to 999
        sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(999)));

        assert_eq!(
            harness.dispatch_staged_candidate_for_test(candidate, false),
            StagedDispatchOutcome::Exit
        );
        assert_eq!(packets.lock().expect("packet capture").len(), 1);
        assert!(
            harness
                .runtime
                .terminal_error
                .as_ref()
                .unwrap()
                .contains("prepared_down_owner_mismatch")
        );
    }

    // Case 2: Target invalidated at crossing
    {
        let actions = [
            test_action(1, ActionKind::Down, 0, 0x15),
            test_action(2, ActionKind::Up, 20_000, 0x15),
        ];
        let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
        let epoch = harness.qpc_now_for_test().expect("test epoch");
        harness
            .reanchor_playback_clock_for_test(epoch)
            .expect("reanchor playback");
        harness.configure_focus_owner_for_test(1, 101);
        let packets = harness.configure_mock_packet_sender_for_test(&[]);
        harness.attach_executor_for_test().expect("attach executor");

        let first = harness
            .stage_candidate_for_test(
                0,
                harness.current_executor_chord_for_test(),
                TimelineTicks::ZERO,
            )
            .expect("stage Down")
            .expect("Down candidate");
        harness.set_mock_sender_start_qpc_for_test(epoch);
        assert_eq!(
            harness.dispatch_staged_candidate_for_test(first, false),
            StagedDispatchOutcome::Dispatched
        );

        let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
        let candidate = harness
            .stage_candidate_for_test(
                0x0001,
                None,
                sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
            )
            .expect("stage Up")
            .expect("Up candidate");
        harness.set_mock_sender_start_qpc_for_test(up_epoch);

        harness.target.invalidate_target();

        assert_eq!(
            harness.dispatch_staged_candidate_for_test(candidate, false),
            StagedDispatchOutcome::Replan
        );
        assert_eq!(packets.lock().expect("packet capture").len(), 1);
        assert!(!harness.target.is_valid());
    }
}

#[test]
fn up_only_process_exit_at_crossing_fails_closed() {
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
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage Down")
        .expect("Down candidate");
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let (_, generation) = harness.target.load_stable().unwrap();
    if let Some(authority) = harness.target.owner_authority_for_generation(generation) {
        authority.set_process_identity_status_for_test(
            sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessTerminated,
        );
    }

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    );
    assert_eq!(packets.lock().expect("packet capture").len(), 1);
    assert!(
        harness
            .runtime
            .terminal_error
            .as_ref()
            .unwrap()
            .contains("prepared_down_owner_process_terminated")
    );
}

#[test]
fn post_focus_race_hook_flips_focus_active_aborts_down_only() {
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
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage Down")
        .expect("Down candidate");

    harness.set_post_focus_revalidation_race_hook(|focus_active, _target, _, _, _, _, _| {
        focus_active.store(false, Ordering::Release);
    });
    harness.set_mock_sender_start_qpc_for_test(epoch);

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Replan
    );
    assert_eq!(packets.lock().expect("packet capture").len(), 0);
    assert_eq!(harness.local_metrics.final_gate_focus_losses, 1);
    assert!(
        harness
            .resources
            .playback
            .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus)
    );
    assert_eq!(harness.runtime.verified_target_for_test(), None);
}

#[test]
fn post_focus_race_hook_invalidates_target_fails_closed_down_only() {
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
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage Down")
        .expect("Down candidate");

    harness.set_post_focus_revalidation_race_hook(|_focus_active, target, _, _, _, _, _| {
        target.invalidate_target();
    });
    harness.set_mock_sender_start_qpc_for_test(epoch);

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Exit
    );
    assert_eq!(packets.lock().expect("packet capture").len(), 0);
    assert_eq!(harness.local_metrics.final_gate_target_changes, 1);
    assert!(
        harness
            .runtime
            .terminal_error
            .as_ref()
            .unwrap()
            .contains("prepared_down_target_changed")
    );
}

#[test]
fn post_focus_race_hook_flips_focus_active_mixed_dispatches_safe_up_and_aborts_down() {
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
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");

    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage first Down")
        .expect("first Down candidate");
    let first_now = harness.qpc_now_for_test().expect("first Down QPC");
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    ));

    let mixed_target = epoch
        .checked_add_duration(DurationTicks::from_raw(
            qpc_offset(&harness, 20_000).as_u64(),
        ))
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

    harness.set_post_focus_revalidation_race_hook(|focus_active, _target, _, _, _, _, _| {
        focus_active.store(false, Ordering::Release);
    });
    harness.set_mock_sender_start_qpc_for_test(mixed_target);

    let outcome = harness.dispatch_staged_candidate_for_test(mixed, false);
    assert_eq!(
        outcome,
        StagedDispatchOutcome::Replan,
        "focus rejection of mixed Down replans without sending Down while focus is lost"
    );
    assert_eq!(
        packets
            .lock()
            .expect("packet capture")
            .iter()
            .map(|(packet, _)| *packet)
            .collect::<Vec<_>>(),
        vec![PhysicalPacket::new(0, 1)],
        "the rejected Down key is absent and musical Up is held until focus restore or safety cleanup"
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.local_metrics.final_gate_focus_losses, 1);
    assert!(
        harness
            .resources
            .playback
            .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus)
    );
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
fn post_focus_race_hook_invalidates_target_mixed_fails_closed_without_down() {
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
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");

    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage first Down")
        .expect("first Down candidate");
    let first_now = harness.qpc_now_for_test().expect("first Down QPC");
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    ));
    // Keep this case focused on generation revalidation. Separate focus-required
    // target races cover fresh HWND/PID admission; the shared test foreground
    // override must not make the target-generation assertion scheduling-sensitive.
    harness.config.focus.require_focus = false;

    let mixed_target = epoch
        .checked_add_duration(DurationTicks::from_raw(
            qpc_offset(&harness, 20_000).as_u64(),
        ))
        .expect("mixed target");
    while harness.qpc_now_for_test().expect("wait for mixed target") < mixed_target {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let (up_mask, down_chord, timing_window) = staged_send_at(&mut harness, mixed_target, epoch);
    let mixed = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage mixed packet")
        .expect("mixed candidate");
    assert_eq!(mixed.packet_masks, PhysicalPacket::new(1, 2));

    harness.set_post_focus_revalidation_race_hook(|_focus_active, target, _, _, _, _, _| {
        target.invalidate_target();
    });
    harness.set_mock_sender_start_qpc_for_test(mixed_target);

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(mixed, false),
        StagedDispatchOutcome::Exit
    );
    assert_eq!(
        packets
            .lock()
            .expect("packet capture")
            .iter()
            .map(|(packet, _)| *packet)
            .collect::<Vec<_>>(),
        vec![PhysicalPacket::new(0, 1)],
        "zero Down packets sent after target invalidation"
    );
    assert_eq!(harness.local_metrics.final_gate_target_changes, 1);
    assert!(
        harness
            .runtime
            .terminal_error
            .as_ref()
            .unwrap()
            .contains("prepared_down_target_changed")
    );
    let finalized = harness.finalize_for_test();
    assert_eq!(finalized.attempted_mask, 0);
    assert_eq!(finalized.release_obligation_mask, 1);
    assert_eq!(finalized.failed_release_mask, 1);
    assert!(
        cleanup.lock().expect("safety cleanup capture").is_empty(),
        "invalid target prevents unsafe key emission during terminal cleanup"
    );
}

#[test]
fn post_focus_race_hook_flips_focus_active_up_only_focus_required_replans_and_aborts_up() {
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
        test_action(40, ActionKind::Down, 0, 0x15),
        test_action(41, ActionKind::Up, 20_000, 0x15),
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

    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage first Down")
        .expect("first Down candidate");
    let first_now = harness.qpc_now_for_test().expect("first Down QPC");
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(harness.release_obligation_mask_for_test(), 1);

    let up_target = epoch
        .checked_add_duration(DurationTicks::from_raw(
            qpc_offset(&harness, 20_000).as_u64(),
        ))
        .expect("up target");
    let (up_mask, down_chord, timing_window) = staged_send_at(&mut harness, up_target, epoch);
    assert_eq!(up_mask, 1);
    assert_eq!(down_chord, None);
    let up_only = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage up-only packet")
        .expect("up-only candidate");
    assert_eq!(up_only.packet_masks, PhysicalPacket::new(1, 0));

    harness.set_post_focus_revalidation_race_hook(|focus_active, _target, _, _, _, _, _| {
        focus_active.store(false, Ordering::Release);
    });
    harness.set_mock_sender_start_qpc_for_test(up_target);

    let outcome = harness.dispatch_staged_candidate_for_test(up_only, false);
    assert_eq!(
        outcome,
        StagedDispatchOutcome::Replan,
        "focus rejection of Up-only replans without sending raw Up while focus is lost"
    );
    assert_eq!(
        packets
            .lock()
            .expect("packet capture")
            .iter()
            .map(|(packet, _)| *packet)
            .collect::<Vec<_>>(),
        vec![PhysicalPacket::new(0, 1)],
        "zero musical Up sent after focus lost"
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.local_metrics.final_gate_focus_losses, 1);
    assert!(
        harness
            .resources
            .playback
            .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus)
    );
    let finalized = harness.finalize_for_test();
    assert_eq!(finalized.attempted_mask, 1);
    assert_eq!(finalized.release_obligation_mask, 0);
    assert_eq!(
        cleanup.lock().expect("safety cleanup capture").as_slice(),
        &[(vec![0x15], true)],
        "terminal tracked safety Up cleans up retained obligation safely"
    );
}

#[test]
fn post_focus_race_hook_invalidates_target_up_only_focus_required_fails_closed_without_up() {
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
        test_action(50, ActionKind::Down, 0, 0x15),
        test_action(51, ActionKind::Up, 20_000, 0x15),
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

    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage first Down")
        .expect("first Down candidate");
    let first_now = harness.qpc_now_for_test().expect("first Down QPC");
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(harness.release_obligation_mask_for_test(), 1);

    let up_target = epoch
        .checked_add_duration(DurationTicks::from_raw(
            qpc_offset(&harness, 20_000).as_u64(),
        ))
        .expect("up target");
    let (up_mask, down_chord, timing_window) = staged_send_at(&mut harness, up_target, epoch);
    assert_eq!(up_mask, 1);
    let up_only = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage up-only packet")
        .expect("up-only candidate");
    assert_eq!(up_only.packet_masks, PhysicalPacket::new(1, 0));

    harness.set_post_focus_revalidation_race_hook(|_focus_active, target, _, _, _, _, _| {
        target.invalidate_target();
    });
    harness.set_mock_sender_start_qpc_for_test(up_target);

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(up_only, false),
        StagedDispatchOutcome::Exit
    );
    assert_eq!(
        packets
            .lock()
            .expect("packet capture")
            .iter()
            .map(|(packet, _)| *packet)
            .collect::<Vec<_>>(),
        vec![PhysicalPacket::new(0, 1)],
        "zero Up packets sent after target invalidation"
    );
    assert_eq!(harness.local_metrics.final_gate_target_changes, 1);
    assert!(
        harness
            .runtime
            .terminal_error
            .as_ref()
            .unwrap()
            .contains("prepared_down_target_changed")
    );
    let finalized = harness.finalize_for_test();
    assert_eq!(finalized.attempted_mask, 0);
    assert_eq!(finalized.release_obligation_mask, 1);
    assert_eq!(finalized.failed_release_mask, 1);
    assert!(
        cleanup.lock().expect("safety cleanup capture").is_empty(),
        "invalid target prevents unsafe key emission during terminal cleanup"
    );
}

#[test]
fn post_focus_race_hook_invalidates_target_up_only_focus_not_required_fails_closed_without_up() {
    let actions = [
        test_action(60, ActionKind::Down, 0, 0x15),
        test_action(61, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    harness.config.focus.require_focus = false;
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    let cleanup = harness.configure_mock_cleanup_emitter_for_test();
    harness.attach_executor_for_test().expect("attach executor");

    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage first Down")
        .expect("first Down candidate");
    let first_now = harness.qpc_now_for_test().expect("first Down QPC");
    harness.set_mock_sender_start_qpc_for_test(first_now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    ));
    assert_eq!(harness.release_obligation_mask_for_test(), 1);

    let up_target = epoch
        .checked_add_duration(DurationTicks::from_raw(
            qpc_offset(&harness, 20_000).as_u64(),
        ))
        .expect("up target");
    let (up_mask, down_chord, timing_window) = staged_send_at(&mut harness, up_target, epoch);
    assert_eq!(up_mask, 1);
    let up_only = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage up-only packet")
        .expect("up-only candidate");

    harness.set_post_focus_revalidation_race_hook(|_focus_active, target, _, _, _, _, _| {
        target.invalidate_target();
    });
    harness.set_mock_sender_start_qpc_for_test(up_target);

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(up_only, false),
        StagedDispatchOutcome::Exit
    );
    assert_eq!(
        packets
            .lock()
            .expect("packet capture")
            .iter()
            .map(|(packet, _)| *packet)
            .collect::<Vec<_>>(),
        vec![PhysicalPacket::new(0, 1)],
        "zero Up packets sent after target invalidation"
    );
    assert_eq!(harness.local_metrics.final_gate_target_changes, 1);
    assert!(
        harness
            .runtime
            .terminal_error
            .as_ref()
            .unwrap()
            .contains("prepared_down_target_changed")
    );
    let finalized = harness.finalize_for_test();
    assert_eq!(finalized.attempted_mask, 0);
    assert_eq!(finalized.release_obligation_mask, 1);
    assert_eq!(finalized.failed_release_mask, 1);
    assert!(
        cleanup.lock().expect("safety cleanup capture").is_empty(),
        "invalid target prevents unsafe key emission during terminal cleanup"
    );
}
