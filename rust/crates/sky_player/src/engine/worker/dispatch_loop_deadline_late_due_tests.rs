use super::replan_timing::elapsed_us;
use super::support::{
    DispatchOutcome, OnsetChordInput, ProductionDispatchTestHarness, QpcTicks, TimelineTicks,
    qpc_offset, send_decision_at,
};
#[test]
fn late_due_evaluates_newly_due_up_with_disjoint_down_directly() {
    use sky_dispatch_win32::wait::{WaitOutcome, WaitResult};

    fn chord(index: u32, scheduled_us: u64, scans: &[u16]) -> OnsetChordInput {
        OnsetChordInput {
            source_action_index: index,
            scheduled_us,
            scan_codes: scans.iter().copied().collect(),
        }
    }

    // Slot ordering (sorted scan codes): 0x15 -> bit 1 (A), 0x16 -> bit 2 (B), 0x17 -> bit 4 (C).
    // B1 goes Down at 0 and is released at +25ms (after H=17.167ms).
    // A goes Down at +35ms so its fixed-pulse Up floor is +52.167ms.
    // B2 is scheduled for +42.5ms while A is still held.
    // A simulated late Due wake at +60ms overshoots both A's Up (+52.167ms) and C's Down (+55ms).
    let chords = [
        chord(6680, 0, &[0x16]),
        chord(6681, 35_000, &[0x15]),
        chord(6682, 42_500, &[0x16]),
        chord(6683, 55_000, &[0x17]),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    let clock = harness.resources.clock;
    let epoch = clock
        .now()
        .expect("late-Due epoch sample")
        .checked_add_duration(
            clock
                .duration_from_us(1_000_000)
                .expect("future test epoch offset"),
        )
        .expect("future test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor at future QPC epoch");
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 5_000, 0, 0, 0]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = harness
        .prepare_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage initial B1 Down")
        .expect("initial chord candidate");
    assert_eq!(first.source_action_index(), 6680);
    assert_eq!(
        first.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(0, 2)
    );
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_candidate_for_test(first, false),
        DispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.backend.active_mask, 2);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 2);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);

    let b1_up_eval = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 25_000).as_u64(),
        ))
        .expect("B1 Up target");
    let (up_b_mask, down_chord, up_b_window) = send_decision_at(&mut harness, b1_up_eval, epoch);
    assert_eq!(up_b_mask, 2);
    assert!(down_chord.is_none());
    let up_b = harness
        .prepare_candidate_with_scheduler_window_for_test(up_b_mask, down_chord, up_b_window)
        .expect("stage B1 Up")
        .expect("B1 Up candidate");
    let h_ticks = clock
        .duration_from_us(harness.config.timing.min_hold_us)
        .expect("H floor conversion");
    let b1_down_completion = packets
        .lock()
        .expect("initial Down evidence")
        .first()
        .and_then(|(_, evidence)| evidence.completed_ticks)
        .expect("confirmed B1 Down completion");
    let b_h_floor = b1_down_completion
        .checked_add_duration(h_ticks)
        .expect("B1 H floor");
    assert_eq!(up_b.timing_window.packet_not_before_qpc, b_h_floor);
    harness.set_mock_sender_start_qpc_for_test(b1_up_eval);
    assert_eq!(
        harness.dispatch_candidate_for_test(up_b, false),
        DispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.backend.active_mask, 0);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);

    let a_down_authored = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 35_000).as_u64(),
        ))
        .expect("A authored Down target");
    let (a_up_mask, a_down_chord, a_window) =
        send_decision_at(&mut harness, a_down_authored, epoch);
    assert_eq!(a_up_mask, 0);
    assert_eq!(a_down_chord.map(|c| c.source_action_index), Some(6681));
    let a_down = harness
        .prepare_candidate_with_scheduler_window_for_test(a_up_mask, a_down_chord, a_window)
        .expect("stage A Down")
        .expect("A Down candidate");
    assert_eq!(
        a_down.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(0, 1)
    );
    harness.set_mock_sender_start_qpc_for_test(a_down_authored);
    assert_eq!(
        harness.dispatch_candidate_for_test(a_down, false),
        DispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.backend.active_mask, 1);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 2);
    let origin = harness
        .resources
        .backend
        .release_owner_proof_for_mask(1)
        .expect("A origin proof lookup")
        .expect("A origin proof")
        .clone();
    let a_down_completion = packets
        .lock()
        .expect("A Down evidence")
        .get(2)
        .and_then(|(_, evidence)| evidence.completed_ticks)
        .expect("confirmed A Down completion");

    let b2_authored = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 42_500).as_u64(),
        ))
        .expect("B2 authored Down target");
    let b2_eval = QpcTicks::from_raw(b2_authored.as_u64() - 1);
    super::super::reset_scheduler_evaluation_count_for_test();
    let b2_action =
        super::super::evaluate_scheduler_action(&harness.resources.scheduler, b2_eval, epoch)
            .expect("evaluate future B2");
    assert_eq!(
        super::super::scheduler_evaluation_count_for_test(),
        1,
        "direct wait performs a single evaluation with no speculative future projection"
    );
    assert!(matches!(
        b2_action,
        sky_dispatch_core::executor::DispatchDecision::Wait { deadline_qpc } if deadline_qpc == b2_authored
    ));
    assert_eq!(
        harness.pre_wait_admission_for_test(),
        super::super::PreWaitAdmission::Proceed
    );

    let a_up_physical_target = a_down_completion
        .checked_add_duration(h_ticks)
        .expect("A H floor");
    let late_wake_target = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 60_000).as_u64(),
        ))
        .expect("late wake target after A Up and C Down");
    assert!(late_wake_target >= a_up_physical_target);
    let _waiter_override = crate::engine::worker::wait::install_waiter_result_for_test(
        b2_authored,
        WaitResult {
            outcome: WaitOutcome::Deadline,
            wake_qpc: Some(late_wake_target),
            spin_ticks: sky_dispatch_core::time::DurationTicks::ZERO,
            timer_return_qpc: Some(late_wake_target),
            timer_arm_target_qpc: Some(b2_authored),
            timer_arm_qpc: None,
            timer_arm_generation: None,
            timer_return_generation: None,
        },
    );
    let due = super::super::wait_for_next_boundary(super::super::WaitBoundaryInput {
        deadline: super::super::WaitDeadline {
            physical_target_qpc: Some(b2_authored),
            spin_threshold_ticks: harness.timing.effective_spin_threshold_ticks,
            qpc_clock: clock,
        },
        signals: super::super::WaitSignals {
            waiter: &harness.resources.waiter,
            interrupt: &harness.interrupt,
        },
        mutable: super::super::WaitMutable {
            local_metrics: &mut harness.local_metrics,
            terminal_error: &mut harness.runtime.terminal_error,
        },
    });
    let late_wake_qpc = match due {
        super::super::WaitBoundary::Due {
            wait_result: Some(result),
            target_qpc,
            dispatch_qpc,
            ..
        } => {
            assert_eq!(target_qpc, b2_authored);
            assert_eq!(result.outcome, WaitOutcome::Deadline);
            assert_eq!(result.wake_qpc, Some(late_wake_target));
            assert!(
                dispatch_qpc > target_qpc,
                "Due wake overshoots B2's authored deadline"
            );
            assert!(
                dispatch_qpc >= a_up_physical_target,
                "A's Up is physically eligible"
            );
            dispatch_qpc
        }
        _ => panic!("expected controlled late Due boundary"),
    };
    drop(_waiter_override);

    // At the actual Due wake, evaluate directly against the authoritative
    // observed QPC (`late_wake_qpc`). A's Up (+52.167ms) is now eligible and
    // disjoint from B2 (+42.5ms), so the worker builds Up A + Down B2
    // (Up-before-Down) in one packet, then C at the next evaluation.
    let direct_mixed = harness
        .evaluate_and_prepare_at_wake_for_test(late_wake_qpc, false)
        .expect("post-wake direct evaluation")
        .expect("expected post-wake Send(Up A + Down B2)");
    assert_eq!(direct_mixed.source_action_index(), 6682);
    assert_eq!(
        direct_mixed.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(1, 2)
    );
    assert!(direct_mixed.timing_window.packet_not_before_qpc >= a_up_physical_target);
    assert!(late_wake_qpc >= direct_mixed.timing_window.packet_not_before_qpc);
    harness.runtime.last_dispatch_deadline_wake_qpc = Some(late_wake_qpc);
    harness.runtime.last_dispatch_deadline_target_qpc = Some(b2_authored);
    harness.set_mock_sender_start_qpc_for_test(late_wake_qpc);
    assert_eq!(
        harness.dispatch_candidate_for_test(direct_mixed, false),
        DispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 3);
    assert_eq!(harness.resources.backend.active_mask, 2);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 2);
    assert!(
        harness
            .resources
            .backend
            .release_owner_proof_for_mask(1)
            .expect("session owner proof lookup after A release")
            .expect("session owner proof remains after A release")
            .same_owner(&origin)
    );
    assert!(
        harness
            .resources
            .backend
            .release_owner_proof_for_mask(2)
            .expect("B2 proof retained")
            .expect("B2 remains active")
            .same_owner(&origin)
    );

    // Next outer scheduler pass selects C's due Down (cursor=3 -> 4).
    let c_down = harness
        .evaluate_and_prepare_at_wake_for_test(late_wake_qpc, false)
        .expect("fresh next-iteration direct evaluation")
        .expect("next iteration selects C Down");
    assert_eq!(c_down.source_action_index(), 6683);
    assert_eq!(
        c_down.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(0, 4)
    );
    harness.set_mock_sender_start_qpc_for_test(late_wake_qpc);
    assert_eq!(
        harness.dispatch_candidate_for_test(c_down, false),
        DispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 4);
    assert_eq!(harness.resources.backend.active_mask, 6);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 6);

    let sent = packets.lock().expect("late-Due packet trace");
    assert_eq!(sent.len(), 5);
    assert_eq!(
        sent[0].0,
        sky_dispatch_win32::input::PhysicalPacket::new(0, 2)
    );
    assert_eq!(
        sent[1].0,
        sky_dispatch_win32::input::PhysicalPacket::new(2, 0)
    );
    assert_eq!(
        sent[2].0,
        sky_dispatch_win32::input::PhysicalPacket::new(0, 1)
    );
    assert_eq!(
        sent[3].0,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 2)
    );
    assert_eq!(
        sent[4].0,
        sky_dispatch_win32::input::PhysicalPacket::new(0, 4)
    );
    assert!(sent.iter().all(|(_, evidence)| evidence.attempts == 1));
    assert_eq!(sent[0].1.started_ticks, Some(epoch));
    assert!(sent[1].1.started_ticks.expect("B1 Up sender QPC") >= b_h_floor);
    assert_eq!(sent[2].1.started_ticks, Some(a_down_authored));
    assert!(sent[3].1.started_ticks.expect("Up A + Down B2 sender QPC") >= a_up_physical_target);
    assert!(sent[4].1.started_ticks.expect("C Down sender QPC") >= late_wake_qpc);
    assert_eq!(
        sent.iter()
            .skip(3)
            .filter(|(packet, _)| packet.down_mask & 2 != 0)
            .count(),
        1,
        "the B2 onset crosses its sender boundary at most once"
    );
    assert_eq!(harness.local_metrics.precision_packet_attempts, 5);
    assert!(harness.runtime.terminal_error.is_none());

    println!(
        "late-Due trace: H=+{}us B1_Up=+{}us B2_authored=+{}us Due_wake=+{}us A_Up_floor=+{}us direct_packet=(up:0x0001,down:0x0002) next_iteration=(up:0x0000,down:0x0004) cursor=4 masks=(active:0x0006,possible:0,inflight:0,release:0x0006) attempts(B2)=1",
        elapsed_us(clock, epoch, b_h_floor),
        elapsed_us(clock, epoch, b1_up_eval),
        elapsed_us(clock, epoch, b2_authored),
        elapsed_us(clock, epoch, late_wake_qpc),
        elapsed_us(clock, epoch, a_up_physical_target),
    );
}

#[test]
fn direct_post_wake_evaluation_handles_multiple_ups_timely_wake_early_wake_and_modifier_prefix() {
    fn chord(index: u32, scheduled_us: u64, scans: &[u16]) -> OnsetChordInput {
        OnsetChordInput {
            source_action_index: index,
            scheduled_us,
            scan_codes: scans.iter().copied().collect(),
        }
    }

    // Slot 0 = 0x15, Slot 1 = 0x16, Slot 2 = 0x17.
    // Chord 0 (0x15) at 0us, Chord 1 (0x16) at 10ms, Chord 2 (0x17) at 30ms,
    // Chord 3 (0x15) at 35ms (same key as Chord 0).
    let chords = [
        chord(100, 0, &[0x15]),
        chord(101, 10_000, &[0x16]),
        chord(102, 30_000, &[0x17]),
        chord(103, 35_000, &[0x15]),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    let clock = harness.resources.clock;
    let epoch = clock
        .now()
        .expect("epoch")
        .checked_add_duration(clock.duration_from_us(1_000_000).expect("offset"))
        .expect("future epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor");
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0, 0, 0, 0]);
    harness.attach_executor_for_test().expect("attach");

    // Dispatch Chord 0 at epoch, Chord 1 at epoch + 10ms.
    let c0 = harness
        .prepare_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage c0")
        .expect("c0");
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_candidate_for_test(c0, false),
        DispatchOutcome::Dispatched
    );

    let t10 = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 10_000).as_u64(),
        ))
        .expect("t10");
    let (u1, d1, w1) = send_decision_at(&mut harness, t10, epoch);
    let c1 = harness
        .prepare_candidate_with_scheduler_window_for_test(u1, d1, w1)
        .expect("stage c1")
        .expect("c1");
    harness.set_mock_sender_start_qpc_for_test(t10);
    assert_eq!(
        harness.dispatch_candidate_for_test(c1, false),
        DispatchOutcome::Dispatched
    );

    // 1. Before +17.167ms, direct evaluation returns Wait (earliest deadline is slot 0's Up).
    //    Simulate a late wake at +30ms where slot 0 Up, slot 1 Up, and Chord 2 Down are all due.
    let t30 = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 30_000).as_u64(),
        ))
        .expect("t30");
    assert!(matches!(
        super::super::evaluate_scheduler_action(
            &harness.resources.scheduler,
            QpcTicks::from_raw(t10.as_u64() + 1),
            epoch,
        )
        .expect("direct Wait before +17.167ms"),
        sky_dispatch_core::executor::DispatchDecision::Wait { .. }
    ));
    let mixed_multi = harness
        .evaluate_and_prepare_at_wake_for_test(t30, false)
        .expect("direct evaluation at +30ms")
        .expect("expected Send(Up 0b11 + Down 0b100)");
    assert_eq!(
        mixed_multi.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(0b011, 0b100)
    );
    // Modifier held on Mixed packet -> dispatches Up-only prefix (0b011) and keeps Chord 2 unattempted.
    harness.set_modifier_key_state_query_for_test(|_| -0x8000);
    harness.set_mock_sender_start_qpc_for_test(t30);
    assert_eq!(
        harness.dispatch_candidate_for_test(mixed_multi, false),
        DispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 2);
    assert_eq!(harness.resources.backend.active_mask, 0);

    // Release modifier, now evaluate Chord 2 directly at t30 and dispatch.
    harness.set_modifier_key_state_query_for_test(|_| 0);
    let c2_direct = harness
        .evaluate_and_prepare_at_wake_for_test(t30, false)
        .expect("direct timely evaluation")
        .expect("expected timely Send");
    assert_eq!(
        c2_direct.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(0, 0b100)
    );
    harness.set_mock_sender_start_qpc_for_test(t30);
    assert_eq!(
        harness.dispatch_candidate_for_test(c2_direct, false),
        DispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 3);

    // Verify early wake before Chord 3 (+35ms) never sends early (returns None / Wait).
    let t34 = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 34_000).as_u64(),
        ))
        .expect("t34");
    let t35 = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 35_000).as_u64(),
        ))
        .expect("t35");
    assert!(
        harness
            .evaluate_and_prepare_at_wake_for_test(t34, false)
            .expect("early wake check")
            .is_none(),
        "early wake before Chord 3 must not prepare or send a packet"
    );

    // Verify focus, manual pause, quit, skip, panic, lease expiry, system suspend,
    // and target invalidation after post-wake evaluation all fail closed / replan
    // without sending a new Down.
    let (u3, d3, w3) = send_decision_at(&mut harness, t35, epoch);
    let stage_c3 = |h: &mut ProductionDispatchTestHarness| {
        h.prepare_candidate_with_scheduler_window_for_test(u3, d3, w3)
            .expect("stage c3")
            .expect("c3")
    };

    // Focus loss -> Replan
    let c3_focus = stage_c3(&mut harness);
    harness.config.focus.require_focus = true;
    harness
        .focus_active
        .store(false, std::sync::atomic::Ordering::Release);
    assert_eq!(
        harness.dispatch_candidate_for_test(c3_focus, false),
        DispatchOutcome::Replan
    );
    harness
        .focus_active
        .store(true, std::sync::atomic::Ordering::Release);
    harness.config.focus.require_focus = false;

    // Manual pause -> Replan
    let c3_pause = stage_c3(&mut harness);
    harness
        .desired_pause
        .store(true, std::sync::atomic::Ordering::Release);
    assert_eq!(
        harness.dispatch_candidate_for_test(c3_pause, false),
        DispatchOutcome::Replan
    );
    harness
        .desired_pause
        .store(false, std::sync::atomic::Ordering::Release);

    // Quit, Skip, Panic -> Exit with no new Down
    for kind in ["quit", "skip", "panic"] {
        let c3_ctrl = stage_c3(&mut harness);
        match kind {
            "quit" => harness
                .quit_requested
                .store(true, std::sync::atomic::Ordering::Release),
            "skip" => harness
                .skip_requested
                .store(true, std::sync::atomic::Ordering::Release),
            _ => harness
                .panic_requested
                .store(true, std::sync::atomic::Ordering::Release),
        }
        // Down-bearing final admission returns Replan so the outer worker loop
        // exits via process_command_control without sending the Down.
        assert_eq!(
            harness.dispatch_candidate_for_test(c3_ctrl, false),
            DispatchOutcome::Replan
        );
        match kind {
            "quit" => harness
                .quit_requested
                .store(false, std::sync::atomic::Ordering::Release),
            "skip" => harness
                .skip_requested
                .store(false, std::sync::atomic::Ordering::Release),
            _ => harness
                .panic_requested
                .store(false, std::sync::atomic::Ordering::Release),
        }
    }

    // Supervisor lease expiry -> Replan (outer loop exits via process_command_control)
    let c3_lease = stage_c3(&mut harness);
    harness.supervisor_expired.latch_expired();
    assert_eq!(
        harness.dispatch_candidate_for_test(c3_lease, false),
        DispatchOutcome::Replan
    );

    // System suspend -> Replan (outer loop exits via process_command_control), no send
    let mut suspend_harness = ProductionDispatchTestHarness::create_harness(&chords);
    let suspend_packets = suspend_harness.configure_mock_packet_sender_for_test(&[0]);
    suspend_harness
        .attach_executor_for_test()
        .expect("attach suspend");
    suspend_harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor suspend");
    let c0_s = suspend_harness
        .prepare_candidate_for_test(
            0,
            suspend_harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage suspend c0")
        .expect("c0_s");
    suspend_harness
        .system_power
        .notify(true, &suspend_harness.interrupt);
    assert_eq!(
        suspend_harness.dispatch_candidate_for_test(c0_s, false),
        DispatchOutcome::Replan
    );
    assert!(suspend_packets.lock().expect("suspend packets").is_empty());

    // Target invalidation during post-wake preparation -> None, no send
    let mut inv_harness = ProductionDispatchTestHarness::create_harness(&chords);
    inv_harness.attach_executor_for_test().expect("attach inv");
    inv_harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor inv");
    inv_harness.target.invalidate_target();
    assert!(
        inv_harness
            .evaluate_and_prepare_at_wake_for_test(epoch, false)
            .expect("invalidated target preparation")
            .is_none()
    );

    let sent = packets.lock().expect("packets");
    assert_eq!(sent.len(), 4);
    assert_eq!(
        sent[2].0,
        sky_dispatch_win32::input::PhysicalPacket::new(0b011, 0)
    );
    assert_eq!(
        sent[3].0,
        sky_dispatch_win32::input::PhysicalPacket::new(0, 0b100)
    );
}

#[test]
fn direct_post_wake_with_observer_preserves_original_wait_and_fresh_dispatch_evidence() {
    use crate::engine::worker::dispatch::observation::DispatchObservation;
    use sky_dispatch_win32::wait::{WaitOutcome, WaitResult};

    fn chord(index: u32, scheduled_us: u64, scans: &[u16]) -> OnsetChordInput {
        OnsetChordInput {
            source_action_index: index,
            scheduled_us,
            scan_codes: scans.iter().copied().collect(),
        }
    }

    // Slot 0 = 0x15, Slot 1 = 0x16, Slot 2 = 0x17.
    // Chord 0 (0x15 | 0x17 = 0b101) at 0us; Chord 1 (0x16 = 0b010) at 10ms.
    let chords = [chord(200, 0, &[0x15, 0x17]), chord(201, 10_000, &[0x16])];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    let clock = harness.resources.clock;
    let epoch = clock
        .now()
        .expect("epoch")
        .checked_add_duration(clock.duration_from_us(1_000_000).expect("offset"))
        .expect("future epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor");
    let _packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    harness.attach_executor_for_test().expect("attach");

    // Dispatch Chord 0 (slots 0 & 2 active).
    let c0 = harness
        .prepare_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage c0")
        .expect("c0");
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_candidate_for_test(c0, false),
        DispatchOutcome::Dispatched
    );

    // Wait for Chord 1 (+10ms) without speculative staging, then wake late at +25ms
    // (beyond Chord 0's H=17.167ms).
    let c1_authored = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 10_000).as_u64(),
        ))
        .expect("c1_authored");
    let c1_authored_ticks = qpc_offset(&harness, 10_000);
    let late_wake = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 25_000).as_u64(),
        ))
        .expect("late_wake");
    let observer = crate::engine::worker::dispatch::observer::PendingObservationQueue::default();
    harness.runtime.pending_wait_observation = crate::engine::worker::dispatch::record_wait_attempt(
        Some(&observer),
        Some(WaitResult {
            outcome: WaitOutcome::Deadline,
            wake_qpc: Some(late_wake),
            spin_ticks: sky_dispatch_core::time::DurationTicks::ZERO,
            timer_return_qpc: Some(late_wake),
            timer_arm_target_qpc: Some(c1_authored),
            timer_arm_qpc: None,
            timer_arm_generation: None,
            timer_return_generation: None,
        }),
        c1_authored,
        sky_dispatch_core::time::DurationTicks::from_raw(100),
        c1_authored_ticks,
        epoch,
        None,
        &mut harness.local_metrics,
    );
    harness.runtime.last_dispatch_deadline_wake_qpc = Some(late_wake);
    harness.runtime.last_dispatch_deadline_target_qpc = Some(c1_authored);

    let direct_mixed = harness
        .evaluate_and_prepare_at_wake_for_test(late_wake, true)
        .expect("direct evaluation with observer")
        .expect("direct mixed candidate");
    let expected_timing_window = direct_mixed.timing_window;
    assert_eq!(
        direct_mixed.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(0b101, 0b010)
    );
    harness.set_mock_sender_start_qpc_for_test(late_wake);
    let ctx = super::super::DispatchAttemptContext {
        target: &harness.target,
        config: &harness.config,
        control: super::super::DispatchControlSignals {
            focus_active: &harness.focus_active,
            quit_requested: &harness.quit_requested,
            skip_requested: &harness.skip_requested,
            panic_requested: &harness.panic_requested,
            desired_pause: &harness.desired_pause,
            supervisor_expired: &harness.supervisor_expired,
            system_power: &harness.system_power,
        },
        timing: &harness.timing,
        qpc_clock: clock,
        progress_clock: &harness.progress_clock,
        metrics: &harness.metrics,
    };
    assert_eq!(
        super::super::dispatch_candidate(
            direct_mixed,
            ctx,
            &mut harness.resources,
            &mut harness.runtime,
            &mut harness.local_metrics,
            Some(&observer),
        ),
        super::super::DispatchAttemptOutcome::Dispatched
    );

    let mut wait_obs = None;
    let mut down_obs = None;
    let mut wait_count = 0usize;
    let mut down_count = 0usize;
    let mut up_count = 0usize;
    while let Some(item) = observer.pop_front() {
        match item {
            DispatchObservation::Wait(w) => {
                wait_count += 1;
                wait_obs = Some(w);
            }
            DispatchObservation::Down(d) => {
                down_count += 1;
                down_obs = Some(d);
            }
            DispatchObservation::Up(_) => {
                up_count += 1;
            }
        }
    }
    assert_eq!(wait_count, 1, "expected exactly 1 WaitObservation");
    assert_eq!(down_count, 1, "expected exactly 1 DownObservation");
    assert_eq!(up_count, 0, "expected 0 UpObservations");
    let wait_obs = wait_obs.expect("WaitObservation");
    let down_obs = down_obs.expect("DownObservation");
    assert_eq!(
        wait_obs.physical_target_qpc, c1_authored,
        "original wait target must not be rewritten after late wake"
    );
    assert_eq!(
        down_obs.physical_target_qpc,
        expected_timing_window.authored_target_qpc
    );
    assert_eq!(down_obs.physical_timing_window, expected_timing_window);
    assert_eq!(down_obs.up_slot_count, 2);
    assert!(down_obs.up_slots.is_some(), "expected pooled up_slots");
    assert!(
        harness.runtime.pending_wait_observation.is_none(),
        "pending_wait_observation must be consumed by dispatch"
    );
    assert_eq!(down_obs.wait_sequence, wait_obs.sequence);
    assert_eq!(down_obs.wake_qpc, Some(late_wake));
    assert_eq!(
        down_obs.requested_packet,
        sky_dispatch_win32::input::PhysicalPacket::new(0b101, 0b010)
    );
    assert_eq!(down_obs.up_slot_count, 2);
}
