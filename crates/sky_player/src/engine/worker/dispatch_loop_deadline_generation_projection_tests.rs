use super::replan_timing::{
    elapsed_us, prepare_from_worker_direct_evaluation, timing_chord, timing_harness_without_focus,
};
use super::support::{DispatchOutcome, ProductionDispatchTestHarness, QpcTicks};

fn wait_to_deadline(
    harness: &mut ProductionDispatchTestHarness,
    deadline_qpc: QpcTicks,
) -> super::super::WaitBoundary {
    assert_eq!(
        harness.pre_wait_admission_for_test(),
        super::super::PreWaitAdmission::Proceed,
        "future deadline passes production pre-wait admission"
    );
    super::super::wait_for_next_boundary(super::super::WaitBoundaryInput {
        deadline: super::super::WaitDeadline {
            physical_target_qpc: Some(deadline_qpc),
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
    })
}

#[test]
fn scheduler_h_floor_and_authored_repeat_trace_uses_single_sender_commits() {
    let chords = [timing_chord(6670, 0), timing_chord(6672, 20_000)];
    let (mut harness, packets, epoch) = timing_harness_without_focus(&chords, &[40_000, 0, 0]);
    let clock = harness.resources.clock;

    let down_eval_qpc = harness.qpc_now_for_test().expect("Down scheduler QPC");
    let (down, down_eval_count) =
        prepare_from_worker_direct_evaluation(&mut harness, down_eval_qpc);
    assert_eq!(down_eval_count, 1);
    assert!(down.timing_window.packet_not_before_qpc <= down_eval_qpc);
    let down_sender_start_qpc = harness.qpc_now_for_test().expect("Down sender start");
    harness.set_mock_sender_start_qpc_for_test(down_sender_start_qpc);
    assert_eq!(
        harness.dispatch_candidate_for_test(down, false),
        DispatchOutcome::Dispatched
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.resources.backend.active_mask, 1);

    // Mock transport evidence places the confirmed Down completion 40 ms
    // after sender start. Direct evaluation at completion returns Wait until
    // completion + H with 1 evaluation (no speculative future staging).
    let down_completed_qpc = packets
        .lock()
        .expect("Down completion capture")
        .first()
        .and_then(|(_, evidence)| evidence.completed_ticks)
        .expect("confirmed Down completion QPC");
    let up_eval_qpc = down_completed_qpc;
    super::super::reset_scheduler_evaluation_count_for_test();
    let up_action =
        super::super::evaluate_scheduler_action(&harness.resources.scheduler, up_eval_qpc, epoch)
            .expect("Up scheduler decision");
    let up_wait_evaluations = super::super::scheduler_evaluation_count_for_test();
    let sky_dispatch_core::executor::DispatchDecision::Wait {
        deadline_qpc: h_floor_qpc,
    } = up_action
    else {
        panic!("expected Wait before H floor, got {up_action:?}");
    };
    assert!(h_floor_qpc > up_eval_qpc);
    assert_eq!(
        up_wait_evaluations, 1,
        "direct deadline dispatch performs one evaluation before waiting"
    );
    let one_tick = sky_dispatch_core::time::DurationTicks::from_raw(1);
    let before_h = QpcTicks::from_raw(h_floor_qpc.as_u64() - 1);
    assert!(matches!(
        super::super::evaluate_scheduler_action(&harness.resources.scheduler, before_h, epoch)
            .expect("H floor minus one tick"),
        sky_dispatch_core::executor::DispatchDecision::Wait { deadline_qpc } if deadline_qpc == h_floor_qpc
    ));
    assert!(matches!(
        super::super::evaluate_scheduler_action(&harness.resources.scheduler, h_floor_qpc, epoch)
            .expect("exact H floor"),
        sky_dispatch_core::executor::DispatchDecision::Send { .. }
    ));
    let after_h = h_floor_qpc
        .checked_add_duration(one_tick)
        .expect("H floor plus one tick");
    assert!(matches!(
        super::super::evaluate_scheduler_action(&harness.resources.scheduler, after_h, epoch)
            .expect("H floor plus one tick"),
        sky_dispatch_core::executor::DispatchDecision::Send { .. }
    ));

    let h_wake = wait_to_deadline(&mut harness, h_floor_qpc);
    assert!(matches!(h_wake, super::super::WaitBoundary::Due { .. }));
    let h_wake_qpc = harness.qpc_now_for_test().expect("H wait wake QPC");
    assert!(
        h_wake_qpc >= h_floor_qpc,
        "real waiter wakes no earlier than H"
    );
    let (up, up_post_wake_evaluations) =
        prepare_from_worker_direct_evaluation(&mut harness, h_wake_qpc);
    assert_eq!(up_post_wake_evaluations, 1);
    assert_eq!(up.timing_window.packet_not_before_qpc, h_floor_qpc);
    assert_ne!(up.timing_window.hold_floor_mask, 0);
    harness.set_mock_sender_start_qpc_for_test(h_wake_qpc);
    assert_eq!(
        harness.dispatch_candidate_for_test(up, false),
        DispatchOutcome::Dispatched
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.backend.active_mask, 0);

    // The same-key Down shares the Up's authored timestamp. It becomes eligible
    // on a fresh scheduler evaluation after the Up transport commit, with no
    // additional completion-relative musical gap.
    let down_b_eval_qpc = h_wake_qpc;
    let (down_b, down_b_evaluations) =
        prepare_from_worker_direct_evaluation(&mut harness, down_b_eval_qpc);
    assert_eq!(down_b_evaluations, 1);
    assert_eq!(down_b.source_action_index(), 6672);
    assert!(down_b.timing_window.down_not_before_qpc <= down_b_eval_qpc);
    assert!(down_b.timing_window.packet_not_before_qpc <= down_b_eval_qpc);
    let repeat_not_before = down_b.timing_window.packet_not_before_qpc.as_u64();
    harness.set_mock_sender_start_qpc_for_test(down_b_eval_qpc);
    assert_eq!(
        harness.dispatch_candidate_for_test(down_b, false),
        DispatchOutcome::Dispatched
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
        "deadline/wake/final-sender trace: current_down_evaluations=1, future_up_wait_evaluations={up_wait_evaluations}, H_eval@{heval}, H_floor_tick={h}, H_minus_1=Wait/H=Send/H_plus_1=Send, H_wake@{hwake}/delta={hwait}us, H_sender_start@{hstart}, authored_repeat_evaluations={down_b_evaluations}, repeat_eval@{repeat_eval}, repeat_not_before@{repeat_not_before}, repeat_sender_start@{repeat_start}, sender_raw_inserted=[1,1,1], sender_calls=3, final_masks=(active:1, release:1, inflight:0), terminal=None",
        heval = up_eval_qpc.as_u64(),
        h = h_floor_qpc.as_u64(),
        hwake = h_wake_qpc.as_u64(),
        hstart = sent[1]
            .1
            .started_ticks
            .expect("Up sender start QPC")
            .as_u64(),
        hwait = elapsed_us(clock, up_eval_qpc, h_wake_qpc),
        down_b_evaluations = down_b_evaluations,
        repeat_eval = down_b_eval_qpc.as_u64(),
        repeat_not_before = repeat_not_before,
        repeat_start = sent[2]
            .1
            .started_ticks
            .expect("Down B sender start QPC")
            .as_u64(),
    );
}
