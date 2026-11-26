use super::replan_timing::elapsed_us;
use super::support::{
    OnsetChordInput, ProductionDispatchTestHarness, QpcTicks, StagedDispatchOutcome, TimelineTicks,
    qpc_offset, staged_send_at,
};
#[test]
fn late_due_preserves_staged_down_then_reselects_eligible_up_and_down() {
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
    assert_eq!(first.source_action_index, 6680);
    assert_eq!(
        first.packet_masks,
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
        a_down.packet_masks,
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
    assert_eq!(b2.source_action_index, 6682);
    assert_eq!(
        b2.packet_masks,
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

    // The worker's Due arm retains B2. It does not run the scheduler again
    // before dispatching the already staged candidate.
    assert_eq!(b2.source_action_index, 6682);
    assert_eq!(
        b2.packet_masks,
        sky_dispatch_win32::input::PhysicalPacket::new(0, 2)
    );
    harness.runtime.last_dispatch_deadline_wake_qpc = Some(late_wake_qpc);
    harness.runtime.last_dispatch_deadline_target_qpc = Some(b2_authored);
    harness.set_mock_sender_start_qpc_for_test(late_wake_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(b2, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 3);
    assert_eq!(harness.resources.backend.active_mask, 3);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 3);
    assert!(
        harness
            .resources
            .backend
            .release_owner_proof_for_mask(1)
            .expect("A origin proof lookup after B2")
            .expect("A remains independently active")
            .same_owner(&origin)
    );

    // Only after the preserved B2 send does the next outer scheduler pass
    // select A's Up and C's due Down as one up-before-down packet.
    let (next_action, _, next_to_stage) = super::super::evaluate_scheduler_action_for_staging(
        &harness.resources.scheduler,
        late_wake_qpc,
        epoch,
    )
    .expect("fresh next-iteration scheduler evaluation");
    assert!(matches!(
        next_action,
        super::super::SchedulerAction::Send {
            up_mask: 1,
            down_chord: Some(chord),
            ..
        } if chord.source_action_index == 6683
    ));
    let mixed = super::super::stage_scheduler_send_candidate(
        next_to_stage.expect("fresh scheduler stages A Up plus C Down"),
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            capture_up_slot_timings: false,
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage fresh next-iteration packet")
    .expect("next iteration selects A Up and C Down");
    assert_eq!(mixed.source_action_index, 6683);
    assert_eq!(
        mixed.packet_masks,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 4)
    );
    assert!(mixed.timing_window.packet_not_before_qpc >= a_up_physical_target);
    assert!(late_wake_qpc >= mixed.timing_window.packet_not_before_qpc);
    harness.set_mock_sender_start_qpc_for_test(late_wake_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(mixed, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 4);
    assert_eq!(harness.resources.backend.active_mask, 6);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 6);
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
        sky_dispatch_win32::input::PhysicalPacket::new(0, 2)
    );
    assert_eq!(
        sent[4].0,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 4)
    );
    assert!(sent.iter().all(|(_, evidence)| evidence.attempts == 1));
    assert_eq!(sent[0].1.started_ticks, Some(epoch));
    assert!(sent[1].1.started_ticks.expect("B1 Up sender QPC") >= b_h_floor);
    assert_eq!(sent[2].1.started_ticks, Some(a_down_authored));
    assert!(sent[3].1.started_ticks.expect("B2 sender QPC") >= b2_authored);
    assert!(sent[4].1.started_ticks.expect("A Up/C Down sender QPC") >= a_up_physical_target);
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
        "late-Due trace: H=+{}us B1_Up=+{}us B2_authored=+{}us B2_G_deadline=+{}us Due_wake=+{}us A_Up_floor=+{}us staged_down=0x0002 first_late_packet=(up:0x0000,down:0x0002) next_iteration=(up:0x0001,down:0x0004) cursor=4 masks=(active:0x0006,possible:0,inflight:0,release:0x0006) attempts(B2)=1",
        elapsed_us(clock, epoch, b_h_floor),
        elapsed_us(clock, epoch, b1_up_eval),
        elapsed_us(clock, epoch, b2_authored),
        elapsed_us(clock, epoch, b2_authored),
        elapsed_us(clock, epoch, late_wake_qpc),
        elapsed_us(clock, epoch, a_up_physical_target),
    );
}
