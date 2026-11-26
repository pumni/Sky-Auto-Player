use super::replan_timing::{
    elapsed_us, stage_from_worker_outer_evaluation, timing_chord, timing_harness_without_focus,
};
use super::support::{ProductionDispatchTestHarness, QpcTicks, StagedDispatchOutcome};

fn wait_to_packet_deadline(
    harness: &mut ProductionDispatchTestHarness,
    candidate: &crate::engine::worker::StagedCandidate,
    scheduler_wait_target_qpc: QpcTicks,
) -> (QpcTicks, super::super::WaitBoundary) {
    assert_eq!(
        harness.pre_wait_candidate_admission_for_test(candidate),
        StagedDispatchOutcome::Dispatched,
        "future candidate passes production pre-wait admission"
    );
    let wait_target_qpc =
        scheduler_wait_target_qpc.max(candidate.timing_window.packet_not_before_qpc);
    let wake = super::super::wait_for_next_boundary(super::super::WaitBoundaryInput {
        deadline: super::super::WaitDeadline {
            physical_target_qpc: Some(wait_target_qpc),
            spin_threshold_ticks: harness.timing.effective_spin_threshold_ticks,
            qpc_clock: harness.resources.clock,
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
    (wait_target_qpc, wake)
}

#[test]
fn scheduler_h_floor_and_authored_repeat_trace_uses_single_sender_commits() {
    let chords = [timing_chord(6670, 0), timing_chord(6672, 20_000)];
    let (mut harness, packets, epoch) = timing_harness_without_focus(&chords, &[40_000, 0, 0]);
    let clock = harness.resources.clock;

    let down_eval_qpc = harness.qpc_now_for_test().expect("Down scheduler QPC");
    let (down, down_eval_count) = stage_from_worker_outer_evaluation(&mut harness, down_eval_qpc);
    assert_eq!(down_eval_count, 1);
    assert!(down.timing_window.packet_not_before_qpc <= down_eval_qpc);
    let down_sender_start_qpc = harness.qpc_now_for_test().expect("Down sender start");
    harness.set_mock_sender_start_qpc_for_test(down_sender_start_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(down, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.resources.backend.active_mask, 1);

    // Mock transport evidence places the confirmed Down completion 40 ms
    // after sender start. The production scheduler must retain its completion
    // relative H floor and project a future Up packet without advancing it.
    // Evaluate at the recorded completion QPC. The H projection oracle is
    // independent of a runner being preempted between the Down commit and
    // this scheduler evaluation.
    let down_completed_qpc = packets
        .lock()
        .expect("Down completion capture")
        .first()
        .and_then(|(_, evidence)| evidence.completed_ticks)
        .expect("confirmed Down completion QPC");
    let up_eval_qpc = down_completed_qpc;
    super::super::reset_scheduler_evaluation_count_for_test();
    let (up_action, up_scheduler_deadline, projected_up) =
        super::super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            up_eval_qpc,
            epoch,
        )
        .expect("Up scheduler projection");
    let up_projection_evaluations = super::super::scheduler_evaluation_count_for_test();
    assert!(matches!(
        up_action,
        super::super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc > up_eval_qpc
    ));
    assert_eq!(up_projection_evaluations, 2, "now plus deadline projection");
    let up = super::super::stage_scheduler_send_candidate(
        projected_up.expect("Up projects to a due action at its scheduler deadline"),
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            capture_up_slot_timings: false,
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage future Up")
    .expect("future Up has a candidate");
    let h_floor_qpc = up.timing_window.packet_not_before_qpc;
    assert!(h_floor_qpc > up_eval_qpc);
    assert_ne!(up.timing_window.hold_floor_mask, 0);
    let one_tick = sky_dispatch_core::time::DurationTicks::from_raw(1);
    let before_h = QpcTicks::from_raw(h_floor_qpc.as_u64() - 1);
    assert!(matches!(
        super::super::evaluate_scheduler_action(&harness.resources.scheduler, before_h, epoch)
            .expect("H floor minus one tick"),
        super::super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc == h_floor_qpc
    ));
    assert!(matches!(
        super::super::evaluate_scheduler_action(&harness.resources.scheduler, h_floor_qpc, epoch)
            .expect("exact H floor"),
        super::super::SchedulerAction::Send { .. }
    ));
    let after_h = h_floor_qpc
        .checked_add_duration(one_tick)
        .expect("H floor plus one tick");
    assert!(matches!(
        super::super::evaluate_scheduler_action(&harness.resources.scheduler, after_h, epoch)
            .expect("H floor plus one tick"),
        super::super::SchedulerAction::Send { .. }
    ));

    let (selected_h_deadline, h_wake) =
        wait_to_packet_deadline(&mut harness, &up, up_scheduler_deadline);
    assert_eq!(selected_h_deadline, h_floor_qpc);
    assert!(matches!(h_wake, super::super::WaitBoundary::Due { .. }));
    let h_wake_qpc = harness.qpc_now_for_test().expect("H wait wake QPC");
    assert!(
        h_wake_qpc >= h_floor_qpc,
        "real waiter wakes no earlier than H"
    );
    harness.set_mock_sender_start_qpc_for_test(h_wake_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(up, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.backend.active_mask, 0);

    // The same-key Down shares the Up's authored timestamp. It becomes eligible
    // on a fresh scheduler evaluation after the Up transport commit, with no
    // additional completion-relative musical gap.
    let down_b_eval_qpc = h_wake_qpc;
    super::super::reset_scheduler_evaluation_count_for_test();
    let (down_b_action, _down_b_scheduler_deadline, projected_down_b) =
        super::super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            down_b_eval_qpc,
            epoch,
        )
        .expect("Down B scheduler projection");
    let down_b_projection_evaluations = super::super::scheduler_evaluation_count_for_test();
    assert!(matches!(
        down_b_action,
        super::super::SchedulerAction::Send {
            up_mask: 0,
            down_chord: Some(chord),
            timing_window,
            ..
        } if chord.source_action_index == 6672
            && timing_window.down_not_before_qpc <= down_b_eval_qpc
    ));
    assert_eq!(down_b_projection_evaluations, 1);
    let down_b = super::super::stage_scheduler_send_candidate(
        projected_down_b.expect("authored Down B is immediately due after Up commit"),
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            capture_up_slot_timings: false,
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage Down B")
    .expect("Down B has a projected candidate");
    assert!(down_b.timing_window.packet_not_before_qpc <= down_b_eval_qpc);
    let repeat_not_before = down_b.timing_window.packet_not_before_qpc.as_u64();
    harness.set_mock_sender_start_qpc_for_test(down_b_eval_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(down_b, false),
        StagedDispatchOutcome::Dispatched
    );

    let sent = packets.lock().expect("packet capture");
    assert_eq!(sent.len(), 3);
    assert_eq!(
        sent[0].0,
        sky_dispatch_win32::input::PhysicalPacket::new(0, 1)
    );
    assert_eq!(
        sent[1].0,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 0)
    );
    assert_eq!(
        sent[2].0,
        sky_dispatch_win32::input::PhysicalPacket::new(0, 1)
    );
    assert!(sent.iter().all(|(_, evidence)| evidence.attempts == 1));
    assert!(
        sent.iter()
            .all(|(_, evidence)| evidence.first_inserted == 1)
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 2);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.resources.backend.active_mask, 1);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert!(harness.runtime.terminal_error.is_none());
    println!(
        "deadline/wake/final-sender trace: current_down_evaluations=1, future_up_projection_evaluations={up_projection_evaluations}, H_eval@{heval}, H_floor_tick={h}, H_minus_1=Wait/H=Send/H_plus_1=Send, H_selected_deadline={hsel}, H_wake@{hwake}/delta={hwait}us, H_sender_start@{hstart}, authored_repeat_evaluations={down_b_projection_evaluations}, repeat_eval@{repeat_eval}, repeat_not_before@{repeat_not_before}, repeat_sender_start@{repeat_start}, sender_raw_inserted=[1,1,1], sender_calls=3, final_masks=(active:1, release:1, inflight:0), terminal=None",
        heval = up_eval_qpc.as_u64(),
        h = h_floor_qpc.as_u64(),
        hsel = selected_h_deadline.as_u64(),
        hwake = h_wake_qpc.as_u64(),
        hstart = sent[1]
            .1
            .started_ticks
            .expect("Up sender start QPC")
            .as_u64(),
        hwait = elapsed_us(clock, up_eval_qpc, h_wake_qpc),
        down_b_projection_evaluations = down_b_projection_evaluations,
        repeat_eval = down_b_eval_qpc.as_u64(),
        repeat_not_before = repeat_not_before,
        repeat_start = sent[2]
            .1
            .started_ticks
            .expect("Down B sender start QPC")
            .as_u64(),
    );
}
