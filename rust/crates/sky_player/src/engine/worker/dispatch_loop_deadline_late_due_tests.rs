use super::replan_timing::elapsed_us;
use super::support::{
    OnsetChordInput, ProductionDispatchTestHarness, QpcTicks, StagedDispatchOutcome, TimelineTicks,
    qpc_offset, staged_send_at,
};
#[test]
fn late_due_reselects_newly_due_up_with_staged_down() {
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
    // B2 is staged for +42.5ms while A is still held.
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
        .stage_candidate_for_test(
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
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
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
    let (up_b_mask, down_chord, up_b_window) = staged_send_at(&mut harness, b1_up_eval, epoch);
    assert_eq!(up_b_mask, 2);
    assert!(down_chord.is_none());
    let up_b = harness
        .stage_candidate_with_scheduler_window_for_test(up_b_mask, down_chord, up_b_window)
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
        harness.dispatch_staged_candidate_for_test(up_b, false),
        StagedDispatchOutcome::Dispatched
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
    let (a_up_mask, a_down_chord, a_window) = staged_send_at(&mut harness, a_down_authored, epoch);
    assert_eq!(a_up_mask, 0);
    assert_eq!(a_down_chord.map(|c| c.source_action_index), Some(6681));
    let a_down = harness
        .stage_candidate_with_scheduler_window_for_test(a_up_mask, a_down_chord, a_window)
        .expect("stage A Down")
        .expect("A Down candidate");
    assert_eq!(
        a_down.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(0, 1)
    );
    harness.set_mock_sender_start_qpc_for_test(a_down_authored);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(a_down, false),
        StagedDispatchOutcome::Dispatched
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
    let (b2_action, scheduler_wait_target, projected_b2) =
        super::super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            b2_eval,
            epoch,
        )
        .expect("evaluate future B2");
    assert!(matches!(
        b2_action,
        super::super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc == b2_authored
    ));
    assert_eq!(scheduler_wait_target, b2_authored);
    let projected_b2 = projected_b2.expect("scheduler projects B2 at its authored target");
    assert!(matches!(
        projected_b2,
        super::super::SchedulerAction::Send {
            up_mask: 0,
            down_chord: Some(chord),
            timing_window,
            ..
        } if chord.source_action_index == 6682
            && timing_window.packet_not_before_qpc == b2_authored
    ));
    let b2 = super::super::stage_scheduler_send_candidate(
        projected_b2,
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            capture_up_slot_timings: false,
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage projected B2")
    .expect("B2 is staged for its authored deadline");
    assert_eq!(b2.source_action_index(), 6682);
    assert_eq!(
        b2.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(0, 2)
    );
    assert_eq!(b2.timing_window.authored_target_qpc, b2_authored);
    assert_eq!(b2.timing_window.packet_not_before_qpc, b2_authored);
    assert_eq!(
        harness.pre_wait_candidate_admission_for_test(&b2),
        StagedDispatchOutcome::Dispatched
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
            physical_target_qpc: Some(
                scheduler_wait_target.max(b2.timing_window.packet_not_before_qpc),
            ),
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

    // B2: At the actual Due wake, reselect against the authoritative observed
    // QPC (`late_wake_qpc`). A's Up (+52.167ms) is now eligible and disjoint
    // from staged B2 (+42.5ms), so the worker rebuilds into Up A + Down B2
    // (Up-before-Down) in one packet, then C at the next evaluation.
    let mut reselected = Some(b2);
    let outcome = super::super::reselect_candidate_after_wait(
        &mut reselected,
        late_wake_qpc,
        epoch,
        false,
        &harness.target,
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("post-wake reselection");
    assert_eq!(outcome, super::super::PostWaitSelection::SendReady);
    let reselected = reselected.expect("expected post-wake Send(Up A + Down B2)");
    assert_eq!(reselected.source_action_index(), 6682);
    assert_eq!(
        reselected.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(1, 2)
    );
    assert!(reselected.timing_window.packet_not_before_qpc >= a_up_physical_target);
    assert!(late_wake_qpc >= reselected.timing_window.packet_not_before_qpc);
    harness.runtime.last_dispatch_deadline_wake_qpc = Some(late_wake_qpc);
    harness.runtime.last_dispatch_deadline_target_qpc = Some(b2_authored);
    harness.set_mock_sender_start_qpc_for_test(late_wake_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(reselected, false),
        StagedDispatchOutcome::Dispatched
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
    let (next_action, _, next_to_stage) = super::super::evaluate_scheduler_action_for_staging(
        &harness.resources.scheduler,
        late_wake_qpc,
        epoch,
    )
    .expect("fresh next-iteration scheduler evaluation");
    assert!(matches!(
        next_action,
        super::super::SchedulerAction::Send {
            up_mask: 0,
            down_chord: Some(chord),
            ..
        } if chord.source_action_index == 6683
    ));
    let c_down = super::super::stage_scheduler_send_candidate(
        next_to_stage.expect("fresh scheduler stages C Down"),
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            capture_up_slot_timings: false,
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage fresh next-iteration packet")
    .expect("next iteration selects C Down");
    assert_eq!(c_down.source_action_index(), 6683);
    assert_eq!(
        c_down.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(0, 4)
    );
    harness.set_mock_sender_start_qpc_for_test(late_wake_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(c_down, false),
        StagedDispatchOutcome::Dispatched
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
        "the staged B2 onset crosses its sender boundary at most once"
    );
    assert_eq!(harness.local_metrics.precision_packet_attempts, 5);
    assert!(harness.runtime.terminal_error.is_none());

    println!(
        "late-Due trace: H=+{}us B1_Up=+{}us B2_authored=+{}us Due_wake=+{}us A_Up_floor=+{}us staged_down=0x0002 reselected_packet=(up:0x0001,down:0x0002) next_iteration=(up:0x0000,down:0x0004) cursor=4 masks=(active:0x0006,possible:0,inflight:0,release:0x0006) attempts(B2)=1",
        elapsed_us(clock, epoch, b_h_floor),
        elapsed_us(clock, epoch, b1_up_eval),
        elapsed_us(clock, epoch, b2_authored),
        elapsed_us(clock, epoch, late_wake_qpc),
        elapsed_us(clock, epoch, a_up_physical_target),
    );
}

#[test]
fn timely_due_reuses_staged_candidate_and_late_due_handles_multiple_ups_blocked_repeat_and_modifier_prefix()
 {
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
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage c0")
        .expect("c0");
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(c0, false),
        StagedDispatchOutcome::Dispatched
    );

    let t10 = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 10_000).as_u64(),
        ))
        .expect("t10");
    let (u1, d1, w1) = staged_send_at(&mut harness, t10, epoch);
    let c1 = harness
        .stage_candidate_with_scheduler_window_for_test(u1, d1, w1)
        .expect("stage c1")
        .expect("c1");
    harness.set_mock_sender_start_qpc_for_test(t10);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(c1, false),
        StagedDispatchOutcome::Dispatched
    );

    // 1. Stage slot 0's Up before +17.167ms, then simulate a late wake at +30ms
    //    where slot 0 Up, slot 1 Up, and Chord 2 Down are all due.
    let t30 = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 30_000).as_u64(),
        ))
        .expect("t30");
    let (_, _, staged_c2_action) = super::super::evaluate_scheduler_action_for_staging(
        &harness.resources.scheduler,
        QpcTicks::from_raw(t10.as_u64() + 1),
        epoch,
    )
    .expect("project before +17.167ms");
    // At t10 + 1, earliest deadline is slot 0's Up (+17.167ms), not Chord 2.
    let staged_up0 = super::super::stage_scheduler_send_candidate(
        staged_c2_action.expect("projected Up0"),
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            capture_up_slot_timings: false,
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage up0")
    .expect("up0");
    // Simulate late wake at +30ms: BOTH slot 0 Up (+17.167ms), slot 1 Up (+27.167ms), AND Chord 2 (+30ms) are due!
    let mut mixed_multi = Some(staged_up0);
    assert_eq!(
        super::super::reselect_candidate_after_wait(
            &mut mixed_multi,
            t30,
            epoch,
            false,
            &harness.target,
            &mut harness.resources,
            &mut harness.runtime,
        )
        .expect("reselect multiple Ups + Chord 2"),
        super::super::PostWaitSelection::SendReady
    );
    let mixed_multi = mixed_multi.expect("expected Send(Up 0b11 + Down 0b100)");
    assert_eq!(
        mixed_multi.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(0b011, 0b100)
    );
    // Modifier held on Mixed packet -> dispatches Up-only prefix (0b011) and keeps Chord 2 unattempted.
    harness.set_modifier_key_state_query_for_test(|_| -0x8000);
    harness.set_mock_sender_start_qpc_for_test(t30);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(mixed_multi, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 2);
    assert_eq!(harness.resources.backend.active_mask, 0);

    // Release modifier, now stage Chord 2 at t30 and verify timely unchanged reuse!
    harness.set_modifier_key_state_query_for_test(|_| 0);
    let (u2, d2, w2) = staged_send_at(&mut harness, t30, epoch);
    let c2 = harness
        .stage_candidate_with_scheduler_window_for_test(u2, d2, w2)
        .expect("stage c2")
        .expect("c2");
    let mut c2_reused = Some(c2);
    assert_eq!(
        super::super::reselect_candidate_after_wait(
            &mut c2_reused,
            t30,
            epoch,
            false,
            &harness.target,
            &mut harness.resources,
            &mut harness.runtime,
        )
        .expect("timely unchanged reuse"),
        super::super::PostWaitSelection::SendReady
    );
    let c2_reused = c2_reused.expect("expected timely reuse Send");
    assert_eq!(
        c2_reused.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(0, 0b100)
    );
    harness.set_mock_sender_start_qpc_for_test(t30);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(c2_reused, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 3);

    // Verify early wake before Chord 3 (+35ms) never sends early (returns Replan).
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
    let (u3, d3, w3) = staged_send_at(&mut harness, t35, epoch);
    let c3 = harness
        .stage_candidate_with_scheduler_window_for_test(u3, d3, w3)
        .expect("stage c3")
        .expect("c3");
    let mut c3_early = Some(c3);
    assert_eq!(
        super::super::reselect_candidate_after_wait(
            &mut c3_early,
            t34,
            epoch,
            false,
            &harness.target,
            &mut harness.resources,
            &mut harness.runtime,
        )
        .expect("early wake check"),
        super::super::PostWaitSelection::Replan
    );
    assert!(c3_early.is_none());

    // Verify focus, manual pause, quit, skip, panic, lease expiry, system suspend,
    // and target invalidation after post-wake reselection all fail closed / replan
    // without sending a new Down.
    let (u3, d3, w3) = staged_send_at(&mut harness, t35, epoch);
    let stage_c3 = |h: &mut ProductionDispatchTestHarness| {
        h.stage_candidate_with_scheduler_window_for_test(u3, d3, w3)
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
        harness.dispatch_staged_candidate_for_test(c3_focus, false),
        StagedDispatchOutcome::Replan
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
        harness.dispatch_staged_candidate_for_test(c3_pause, false),
        StagedDispatchOutcome::Replan
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
            harness.dispatch_staged_candidate_for_test(c3_ctrl, false),
            StagedDispatchOutcome::Replan
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
        harness.dispatch_staged_candidate_for_test(c3_lease, false),
        StagedDispatchOutcome::Replan
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
        .stage_candidate_for_test(
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
        suspend_harness.dispatch_staged_candidate_for_test(c0_s, false),
        StagedDispatchOutcome::Replan
    );
    assert!(suspend_packets.lock().expect("suspend packets").is_empty());

    // Target invalidation during post-wake reselection -> TargetInvalidated, no send
    let mut inv_harness = ProductionDispatchTestHarness::create_harness(&chords);
    inv_harness.attach_executor_for_test().expect("attach inv");
    inv_harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor inv");
    let mut inv_staged = None;
    inv_harness.target.invalidate_target();
    assert_eq!(
        super::super::reselect_candidate_after_wait(
            &mut inv_staged,
            epoch,
            epoch,
            false,
            &inv_harness.target,
            &mut inv_harness.resources,
            &mut inv_harness.runtime,
        )
        .expect("invalidated target reselect"),
        super::super::PostWaitSelection::TargetInvalidated
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
fn post_wake_reselect_with_observer_preserves_original_wait_and_fresh_dispatch_evidence() {
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
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage c0")
        .expect("c0");
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(c0, false),
        StagedDispatchOutcome::Dispatched
    );

    // Stage Chord 1 (slot 1 Down) at +10ms before Chord 0's Ups are due.
    let c1_authored = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 10_000).as_u64(),
        ))
        .expect("c1_authored");
    let (u1, d1, w1) = staged_send_at(&mut harness, c1_authored, epoch);
    let c1 = harness
        .stage_candidate_with_scheduler_window_for_test(u1, d1, w1)
        .expect("stage c1")
        .expect("c1");
    let c1_authored_ticks = c1.authored_target_ticks;

    // Simulate Due wake at +25ms (beyond Chord 0's H=17.167ms).
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

    let mut reselected = Some(c1);
    assert_eq!(
        super::super::reselect_candidate_after_wait(
            &mut reselected,
            late_wake,
            epoch,
            true,
            &harness.target,
            &mut harness.resources,
            &mut harness.runtime,
        )
        .expect("reselect with observer"),
        super::super::PostWaitSelection::SendReady
    );
    let reselected = reselected.expect("reselected mixed candidate");
    assert_eq!(
        reselected.packet_masks(),
        sky_dispatch_win32::input::PhysicalPacket::new(0b101, 0b010)
    );
    harness.set_mock_sender_start_qpc_for_test(late_wake);
    let ctx = super::super::StagedDispatchContext {
        target: &harness.target,
        config: &harness.config,
        control: super::super::StagedControlSignals {
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
        super::super::dispatch_staged_candidate(
            reselected,
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
    while let Some(item) = observer.pop_front() {
        match item {
            DispatchObservation::Wait(w) => wait_obs = Some(w),
            DispatchObservation::Down(d) => down_obs = Some(d),
            DispatchObservation::Up(_) => {}
        }
    }
    let wait_obs = wait_obs.expect("WaitObservation");
    let down_obs = down_obs.expect("DownObservation");
    assert_eq!(
        wait_obs.physical_target_qpc, c1_authored,
        "original wait target must not be rewritten after late wake"
    );
    assert_eq!(down_obs.wait_sequence, wait_obs.sequence);
    assert_eq!(down_obs.wake_qpc, Some(late_wake));
    assert_eq!(
        down_obs.requested_packet,
        sky_dispatch_win32::input::PhysicalPacket::new(0b101, 0b010)
    );
    assert_eq!(down_obs.up_slot_count, 2);
}
