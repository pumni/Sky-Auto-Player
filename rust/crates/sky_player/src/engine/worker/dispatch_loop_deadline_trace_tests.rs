use super::replan_timing::{
    apply_outer_replan_policy, elapsed_us, stage_from_worker_outer_evaluation, timing_action,
    timing_harness_without_focus,
};
use super::support::{
    ActionKind, ProductionDispatchTestHarness, QpcTicks, StagedDispatchOutcome, TimelineTicks,
    qpc_offset, staged_send_at,
};
use std::sync::atomic::Ordering;

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
fn scheduler_h_floor_and_g_floor_trace_wait_projection_and_single_sender_commit() {
    let actions = [
        timing_action(6670, ActionKind::Down, 0),
        timing_action(6671, ActionKind::Up, 20_000),
        timing_action(6672, ActionKind::Down, 60_000),
        timing_action(6673, ActionKind::Up, 80_000),
    ];
    let (mut harness, packets, epoch) = timing_harness_without_focus(&actions, &[40_000, 0, 0]);
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
    let up = super::super::stage_scheduler_action_preserving_due_up(
        projected_up.expect("Up projects to a due action at its scheduler deadline"),
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            require_focus: harness.config.focus.require_focus,
            focus_active: &harness.focus_active,
            capture_up_slot_timings: false,
            control_signals: super::super::FinalControlSignals {
                quit_requested: &harness.quit_requested,
                skip_requested: &harness.skip_requested,
                panic_requested: &harness.panic_requested,
                desired_pause: &harness.desired_pause,
                supervisor_expired: &harness.supervisor_expired,
                system_power: Some(&harness.system_power),
            },
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

    // The confirmed Up completion starts the G floor. A same-key Down authored
    // at 60 ms remains pending through G-1 and becomes eligible at G exactly.
    // The real H waiter returns at or after the selected deadline. Use its
    // recorded wake as the deterministic G evaluation input: G starts from
    // the Up's confirmed completion and is therefore still future at this
    // exact synthetic boundary, even if the test thread is subsequently
    // stalled before projection.
    let down_b_eval_qpc = h_wake_qpc;
    super::super::reset_scheduler_evaluation_count_for_test();
    let (down_b_action, down_b_scheduler_deadline, projected_down_b) =
        super::super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            down_b_eval_qpc,
            epoch,
        )
        .expect("Down B scheduler projection");
    let g_projection_evaluations = super::super::scheduler_evaluation_count_for_test();
    assert!(matches!(
        down_b_action,
        super::super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc > down_b_eval_qpc
    ));
    assert_eq!(g_projection_evaluations, 2);
    let down_b = super::super::stage_scheduler_action_preserving_due_up(
        projected_down_b.expect("Down B projects to its G deadline"),
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            require_focus: harness.config.focus.require_focus,
            focus_active: &harness.focus_active,
            capture_up_slot_timings: false,
            control_signals: super::super::FinalControlSignals {
                quit_requested: &harness.quit_requested,
                skip_requested: &harness.skip_requested,
                panic_requested: &harness.panic_requested,
                desired_pause: &harness.desired_pause,
                supervisor_expired: &harness.supervisor_expired,
                system_power: Some(&harness.system_power),
            },
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage Down B")
    .expect("Down B has a projected candidate");
    let g_floor_qpc = down_b.timing_window.packet_not_before_qpc;
    assert!(g_floor_qpc > down_b_eval_qpc);
    assert_ne!(down_b.timing_window.release_floor_mask, 0);
    let before_g = QpcTicks::from_raw(g_floor_qpc.as_u64() - 1);
    assert!(matches!(
        super::super::evaluate_scheduler_action(&harness.resources.scheduler, before_g, epoch)
            .expect("G floor minus one tick"),
        super::super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc == g_floor_qpc
    ));
    assert!(matches!(
        super::super::evaluate_scheduler_action(&harness.resources.scheduler, g_floor_qpc, epoch)
            .expect("exact G floor"),
        super::super::SchedulerAction::Send { .. }
    ));
    assert!(matches!(
        super::super::evaluate_scheduler_action(
            &harness.resources.scheduler,
            g_floor_qpc
                .checked_add_duration(one_tick)
                .expect("G plus one tick"),
            epoch,
        )
        .expect("G floor plus one tick"),
        super::super::SchedulerAction::Send { .. }
    ));

    let (selected_g_deadline, g_wake) =
        wait_to_packet_deadline(&mut harness, &down_b, down_b_scheduler_deadline);
    assert_eq!(selected_g_deadline, g_floor_qpc);
    assert!(matches!(g_wake, super::super::WaitBoundary::Due { .. }));
    let g_wake_qpc = harness.qpc_now_for_test().expect("G wait wake QPC");
    assert!(
        g_wake_qpc >= g_floor_qpc,
        "real waiter wakes no earlier than G"
    );
    harness.set_mock_sender_start_qpc_for_test(g_wake_qpc);
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
        "deadline/wake/final-sender trace: current_down_evaluations=1, future_up_projection_evaluations={up_projection_evaluations}, H_eval@{heval}, H_floor_tick={h}, H_minus_1=Wait/H=Send/H_plus_1=Send, H_selected_deadline={hsel}, H_wake@{hwake}/delta={hwait}us, H_sender_start@{hstart}, future_down_projection_evaluations={g_projection_evaluations}, G_eval@{geval}, G_floor_tick={g}, G_minus_1=Wait/G=Send/G_plus_1=Send, G_selected_deadline={gsel}, G_wake@{gwake}/delta={gwait}us, G_sender_start@{gstart}, sender_raw_inserted=[1,1,1], sender_calls=3, final_masks=(active:1, release:1, inflight:0), terminal=None",
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
        geval = down_b_eval_qpc.as_u64(),
        g = g_floor_qpc.as_u64(),
        gsel = selected_g_deadline.as_u64(),
        gwake = g_wake_qpc.as_u64(),
        gstart = sent[2]
            .1
            .started_ticks
            .expect("Down B sender start QPC")
            .as_u64(),
        gwait = elapsed_us(clock, down_b_eval_qpc, g_wake_qpc),
    );
}

#[test]
fn late_due_preserves_staged_down_then_reselects_eligible_up_and_down() {
    use sky_dispatch_core::model::KeyActionInput;
    use sky_dispatch_win32::wait::{WaitOutcome, WaitResult};

    fn action(index: u32, kind: ActionKind, scheduled_us: u64, scans: &[u16]) -> KeyActionInput {
        KeyActionInput {
            source_action_index: index,
            kind,
            scheduled_us,
            scan_codes: scans.to_vec().into(),
            reason: format!("late-Due-contract-{index}").into(),
        }
    }

    let actions = [
        action(6680, ActionKind::Down, 0, &[0x15, 0x16]),
        action(6681, ActionKind::Up, 25_000, &[0x16]),
        action(6682, ActionKind::Down, 42_500, &[0x16]),
        action(6683, ActionKind::Down, 90_000, &[0x17]),
        action(6684, ActionKind::Up, 100_000, &[0x15]),
        action(6685, ActionKind::Up, 130_000, &[0x16]),
        action(6686, ActionKind::Up, 160_000, &[0x17]),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
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
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 5_000, 0, 0]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage initial A+B Down")
        .expect("initial chord candidate");
    assert_eq!(first.source_action_index, 6680);
    assert_eq!(
        first.packet_masks,
        sky_dispatch_win32::input::PhysicalPacket::new(0, 3)
    );
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.backend.active_mask, 3);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 3);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    let origin = harness
        .resources
        .backend
        .release_owner_proof_for_mask(1)
        .expect("A origin proof lookup")
        .expect("A origin proof");

    let authored_b_up = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 25_000).as_u64(),
        ))
        .expect("B1 authored Up target");
    let (up_b_mask, down_chord, up_b_window) = staged_send_at(&mut harness, authored_b_up, epoch);
    assert_eq!(up_b_mask, 2);
    assert!(down_chord.is_none());
    let up_b = harness
        .stage_candidate_with_scheduler_window_for_test(up_b_mask, down_chord, up_b_window)
        .expect("stage B1 Up")
        .expect("B1 Up candidate");
    let h_ticks = clock
        .duration_from_us(harness.config.timing.min_hold_us)
        .expect("H floor conversion");
    let a_b_down_completion = packets
        .lock()
        .expect("initial Down evidence")
        .first()
        .and_then(|(_, evidence)| evidence.completed_ticks)
        .expect("confirmed A+B Down completion");
    let b_h_floor = a_b_down_completion
        .checked_add_duration(h_ticks)
        .expect("B1 H floor");
    assert!(up_b.timing_window.packet_not_before_qpc >= authored_b_up.max(b_h_floor));
    harness.set_mock_sender_start_qpc_for_test(up_b.timing_window.packet_not_before_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(up_b, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.backend.active_mask, 1);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);

    let b2_authored = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 42_500).as_u64(),
        ))
        .expect("B2 authored Down target");
    let g_ticks = clock
        .duration_from_us(harness.config.timing.min_release_gap_us)
        .expect("G floor conversion");
    let b1_up_completion = packets
        .lock()
        .expect("B1 Up evidence")
        .get(1)
        .and_then(|(_, evidence)| evidence.completed_ticks)
        .expect("confirmed B1 Up completion");
    let b2_g_floor = b1_up_completion
        .checked_add_duration(g_ticks)
        .expect("B2 G floor");
    assert!(b2_g_floor > b2_authored, "B2 is G limited");
    let b2_eval = QpcTicks::from_raw(b2_g_floor.as_u64() - 1);
    let (b2_action, scheduler_wait_target, projected_b2) =
        super::super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            b2_eval,
            epoch,
        )
        .expect("evaluate future B2");
    assert!(matches!(
        b2_action,
        super::super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc == b2_g_floor
    ));
    assert_eq!(scheduler_wait_target, b2_g_floor);
    let projected_b2 = projected_b2.expect("scheduler projects B2 at its G boundary");
    assert!(matches!(
        projected_b2,
        super::super::SchedulerAction::Send {
            up_mask: 0,
            down_chord: Some(chord),
            timing_window,
        } if chord.source_action_index == 6682
            && timing_window.packet_not_before_qpc == b2_g_floor
    ));
    let b2 = super::super::stage_scheduler_action_preserving_due_up(
        projected_b2,
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            require_focus: harness.config.focus.require_focus,
            focus_active: &harness.focus_active,
            capture_up_slot_timings: false,
            control_signals: super::super::FinalControlSignals {
                quit_requested: &harness.quit_requested,
                skip_requested: &harness.skip_requested,
                panic_requested: &harness.panic_requested,
                desired_pause: &harness.desired_pause,
                supervisor_expired: &harness.supervisor_expired,
                system_power: Some(&harness.system_power),
            },
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage projected B2")
    .expect("B2 is staged for its future G deadline");
    assert_eq!(b2.source_action_index, 6682);
    assert_eq!(
        b2.packet_masks,
        sky_dispatch_win32::input::PhysicalPacket::new(0, 2)
    );
    assert_eq!(b2.timing_window.authored_target_qpc, b2_authored);
    assert_eq!(b2.timing_window.packet_not_before_qpc, b2_g_floor);
    assert_ne!(b2.timing_window.release_floor_mask & 2, 0);
    assert_eq!(
        harness.pre_wait_candidate_admission_for_test(&b2),
        StagedDispatchOutcome::Dispatched
    );

    let authored_a_up = epoch
        .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
            qpc_offset(&harness, 100_000).as_u64(),
        ))
        .expect("A authored Up target");
    let a_h_floor = a_b_down_completion
        .checked_add_duration(h_ticks)
        .expect("A H floor");
    let a_up_physical_target = authored_a_up.max(a_h_floor);
    assert_eq!(a_up_physical_target, authored_a_up);
    let _waiter_override = crate::engine::worker::wait::install_waiter_result_for_test(
        b2_g_floor,
        WaitResult {
            outcome: WaitOutcome::Deadline,
            wake_qpc: Some(a_up_physical_target),
            spin_ticks: sky_dispatch_core::time::DurationTicks::ZERO,
            timer_return_qpc: Some(a_up_physical_target),
            timer_arm_target_qpc: Some(b2_g_floor),
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
            assert_eq!(target_qpc, b2_g_floor);
            assert_eq!(result.outcome, WaitOutcome::Deadline);
            assert_eq!(result.wake_qpc, Some(a_up_physical_target));
            assert!(
                dispatch_qpc > target_qpc,
                "Due wake overshoots B2's G deadline"
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
    harness.runtime.last_dispatch_deadline_target_qpc = Some(b2_g_floor);
    harness.set_mock_sender_start_qpc_for_test(late_wake_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(b2, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 2);
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
    let mixed = super::super::stage_scheduler_action_preserving_due_up(
        next_to_stage.expect("fresh scheduler stages A Up plus C Down"),
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            require_focus: harness.config.focus.require_focus,
            focus_active: &harness.focus_active,
            capture_up_slot_timings: false,
            control_signals: super::super::FinalControlSignals {
                quit_requested: &harness.quit_requested,
                skip_requested: &harness.skip_requested,
                panic_requested: &harness.panic_requested,
                desired_pause: &harness.desired_pause,
                supervisor_expired: &harness.supervisor_expired,
                system_power: Some(&harness.system_power),
            },
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
    assert_eq!(harness.resources.scheduler.chord_cursor(), 3);
    assert_eq!(harness.resources.backend.active_mask, 6);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 6);
    assert!(
        harness
            .resources
            .backend
            .release_owner_proof_for_mask(1)
            .is_err()
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
    assert_eq!(sent.len(), 4);
    assert_eq!(
        sent[0].0,
        sky_dispatch_win32::input::PhysicalPacket::new(0, 3)
    );
    assert_eq!(
        sent[1].0,
        sky_dispatch_win32::input::PhysicalPacket::new(2, 0)
    );
    assert_eq!(
        sent[2].0,
        sky_dispatch_win32::input::PhysicalPacket::new(0, 2)
    );
    assert_eq!(
        sent[3].0,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 4)
    );
    assert!(sent.iter().all(|(_, evidence)| evidence.attempts == 1));
    assert_eq!(sent[0].1.started_ticks, Some(epoch));
    assert!(sent[1].1.started_ticks.expect("B1 Up sender QPC") >= authored_b_up.max(b_h_floor));
    assert!(sent[2].1.started_ticks.expect("B2 sender QPC") >= b2_g_floor);
    assert!(sent[3].1.started_ticks.expect("A Up/C Down sender QPC") >= a_up_physical_target);
    assert_eq!(
        sent.iter()
            .skip(2)
            .filter(|(packet, _)| packet.down_mask & 2 != 0)
            .count(),
        1,
        "the staged B2 onset crosses its sender boundary at most once"
    );
    assert_eq!(harness.local_metrics.precision_packet_attempts, 4);
    assert!(harness.runtime.terminal_error.is_none());

    println!(
        "late-Due trace: H=+{}us B1_Up=+{}us B2_authored=+{}us B2_G_deadline=+{}us Due_wake=+{}us A_Up_authored=+{}us staged_down=0x0002 first_late_packet=(up:0x0000,down:0x0002) next_iteration=(up:0x0001,down:0x0004) cursor=3 masks=(active:0x0006,possible:0,inflight:0,release:0x0006) attempts(B2)=1",
        elapsed_us(clock, epoch, a_h_floor),
        elapsed_us(clock, epoch, authored_b_up),
        elapsed_us(clock, epoch, b2_authored),
        elapsed_us(clock, epoch, b2_g_floor),
        elapsed_us(clock, epoch, late_wake_qpc),
        elapsed_us(clock, epoch, authored_a_up),
    );
}

#[test]
fn due_candidate_rechecks_stop_skip_focus_and_owner_before_sender() {
    use sky_dispatch_win32::wait::{WaitOutcome, WaitResult};

    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;

    for change in ["stop", "skip", "focus", "owner"] {
        let actions = [
            timing_action(6690, ActionKind::Down, 50_000),
            timing_action(6691, ActionKind::Up, 500_000),
        ];
        let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
        let clock = harness.resources.clock;
        let epoch = clock
            .now()
            .expect("final-gate wake epoch sample")
            .checked_add_duration(
                clock
                    .duration_from_us(1_000_000)
                    .expect("future final-gate epoch offset"),
            )
            .expect("future final-gate epoch");
        harness
            .reanchor_playback_clock_for_test(epoch)
            .expect("future final-gate playback epoch");
        if matches!(change, "focus" | "owner") {
            harness.configure_focus_owner_for_test(1, 101);
        }
        let packets = harness.configure_mock_packet_sender_for_test(&[0]);
        harness.attach_executor_for_test().expect("attach executor");
        let candidate = harness
            .stage_candidate_for_test(
                0,
                harness.current_executor_chord_for_test(),
                TimelineTicks::ZERO,
            )
            .expect("stage future final-gate candidate")
            .expect("future Down candidate");
        assert_eq!(candidate.source_action_index, 6690);
        assert_eq!(
            harness.pre_wait_candidate_admission_for_test(&candidate),
            StagedDispatchOutcome::Dispatched
        );
        let target_qpc = candidate.timing_window.packet_not_before_qpc;
        let wake_qpc = target_qpc
            .checked_add_duration(clock.duration_from_us(10_000).expect("late wake offset"))
            .expect("late wake QPC");
        let _waiter_override = crate::engine::worker::wait::install_waiter_result_for_test(
            target_qpc,
            WaitResult {
                outcome: WaitOutcome::Deadline,
                wake_qpc: Some(wake_qpc),
                spin_ticks: sky_dispatch_core::time::DurationTicks::ZERO,
                timer_return_qpc: Some(wake_qpc),
                timer_arm_target_qpc: Some(target_qpc),
                timer_arm_qpc: None,
                timer_arm_generation: None,
                timer_return_generation: None,
            },
        );
        let boundary = super::super::wait_for_next_boundary(super::super::WaitBoundaryInput {
            deadline: super::super::WaitDeadline {
                physical_target_qpc: Some(target_qpc),
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
        assert!(matches!(boundary, super::super::WaitBoundary::Due { .. }));
        drop(_waiter_override);

        match change {
            "stop" => harness.quit_requested.store(true, Ordering::Release),
            "skip" => harness.skip_requested.store(true, Ordering::Release),
            "focus" => {
                sky_dispatch_win32::focus::set_foreground_window_for_test(Some(2));
                sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));
            }
            "owner" => harness.republish_focus_owner_for_test(1, 202),
            _ => unreachable!(),
        }
        harness.set_mock_sender_start_qpc_for_test(wake_qpc);
        assert_eq!(
            harness.dispatch_staged_candidate_for_test(candidate, false),
            StagedDispatchOutcome::Replan,
            "{change} is rechecked after Due and before SendInput"
        );
        assert!(
            packets
                .lock()
                .expect("no rejected sender attempt")
                .is_empty()
        );
        assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
        assert_eq!(harness.resources.backend.release_obligation_mask(), 0);
        assert_eq!(harness.local_metrics.precision_packet_attempts, 0);
        match change {
            "stop" | "skip" => assert!(harness.process_command_control_for_test()),
            "focus" => assert!(
                harness
                    .resources
                    .playback
                    .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus)
            ),
            "owner" => {
                assert!(
                    !harness
                        .resources
                        .playback
                        .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus)
                );
                assert_eq!(harness.local_metrics.final_gate_target_changes, 1);
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn interrupted_focus_wake_discards_staged_candidate_for_outer_loop_handling() {
    use sky_dispatch_win32::wait::WaitResult;

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
        timing_action(6692, ActionKind::Down, 50_000),
        timing_action(6693, ActionKind::Up, 500_000),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let clock = harness.resources.clock;
    let epoch = clock
        .now()
        .expect("interrupted wait epoch sample")
        .checked_add_duration(
            clock
                .duration_from_us(1_000_000)
                .expect("future interrupted-wait epoch offset"),
        )
        .expect("future interrupted-wait epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("future interrupted-wait playback epoch");
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[0]);
    harness.attach_executor_for_test().expect("attach executor");
    let candidate = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage interrupted candidate")
        .expect("future Down candidate");
    assert_eq!(
        harness.pre_wait_candidate_admission_for_test(&candidate),
        StagedDispatchOutcome::Dispatched
    );
    harness.focus_active.store(false, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(2));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));
    let _waiter_override = crate::engine::worker::wait::install_waiter_result_for_test(
        candidate.timing_window.packet_not_before_qpc,
        WaitResult::interrupted(),
    );
    let interrupted = super::super::wait_for_next_boundary(super::super::WaitBoundaryInput {
        deadline: super::super::WaitDeadline {
            physical_target_qpc: Some(candidate.timing_window.packet_not_before_qpc),
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
    assert!(matches!(
        interrupted,
        super::super::WaitBoundary::Interrupted(..)
    ));
    drop(_waiter_override);
    drop(candidate);

    assert!(
        packets
            .lock()
            .expect("interrupted candidate not sent")
            .is_empty()
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
    assert_eq!(harness.resources.backend.release_obligation_mask(), 0);
    let focus_now = clock.now().expect("focus wake QPC");
    super::super::enter_focus_pause(
        &mut harness.resources.playback,
        &mut harness.runtime,
        focus_now,
        &harness.progress_clock,
    )
    .expect("outer loop handles interrupted focus wake");
    assert!(
        harness
            .resources
            .playback
            .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus)
    );
    assert_eq!(harness.runtime.verified_target_for_test(), None);
}

#[test]
fn real_waiter_smoke_wakes_for_a_future_qpc_deadline() {
    let actions = [
        timing_action(6674, ActionKind::Down, 0),
        timing_action(6675, ActionKind::Up, 1_000_000),
    ];
    let (mut harness, _, _) = timing_harness_without_focus(&actions, &[]);
    let start_qpc = harness.qpc_now_for_test().expect("real waiter start QPC");
    let deadline_qpc = start_qpc
        .checked_add_duration(
            harness
                .resources
                .clock
                .duration_from_us(5_000)
                .expect("real waiter deadline offset"),
        )
        .expect("real waiter deadline");
    let wake = super::super::wait_for_next_boundary(super::super::WaitBoundaryInput {
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
    });
    let wake_qpc = harness.qpc_now_for_test().expect("real waiter wake QPC");
    assert!(matches!(wake, super::super::WaitBoundary::Due { .. }));
    assert!(
        wake_qpc >= deadline_qpc,
        "waiter returned after its real QPC deadline"
    );
}

#[test]
fn stop_skip_panic_and_expired_lease_preempt_busy_permit_backoff() {
    let actions = [
        timing_action(6680, ActionKind::Down, 0),
        timing_action(6681, ActionKind::Up, 1_000_000),
    ];
    for control in ["stop", "skip", "panic", "lease"] {
        let (mut harness, packets, _) = timing_harness_without_focus(&actions, &[]);
        let now = harness.qpc_now_for_test().expect("outer-loop QPC");
        let (candidate, _) = stage_from_worker_outer_evaluation(&mut harness, now);

        let mut retry_pending = false;
        assert_eq!(
            apply_outer_replan_policy(
                &mut harness,
                super::super::DispatchAttemptOutcome::PermitBusy,
                candidate.target_stamp,
                candidate.timing_window.packet_not_before_qpc,
                false,
                &mut retry_pending,
            ),
            super::super::DispatchReplanAction::ContinueImmediately,
            "first transient miss gets one immediate full re-evaluation"
        );
        assert!(retry_pending);
        assert!(matches!(
            apply_outer_replan_policy(
                &mut harness,
                super::super::DispatchAttemptOutcome::PermitBusy,
                candidate.target_stamp,
                candidate.timing_window.packet_not_before_qpc,
                std::mem::take(&mut retry_pending),
                &mut retry_pending,
            ),
            super::super::DispatchReplanAction::WaitCompleted
        ));
        let wait_trace =
            super::super::last_dispatch_replan_wait_for_test().expect("bounded wait trace");
        assert!(!wait_trace.interrupted);
        assert!(
            !retry_pending,
            "bounded backoff does not leave a spin token"
        );

        match control {
            "stop" => harness
                .quit_requested
                .store(true, std::sync::atomic::Ordering::Release),
            "skip" => harness
                .skip_requested
                .store(true, std::sync::atomic::Ordering::Release),
            "panic" => harness
                .panic_requested
                .store(true, std::sync::atomic::Ordering::Release),
            "lease" => harness.supervisor_expired.latch_expired(),
            _ => unreachable!("test enumerates supported control cases"),
        }
        let action = apply_outer_replan_policy(
            &mut harness,
            super::super::DispatchAttemptOutcome::PermitBusy,
            candidate.target_stamp,
            candidate.timing_window.packet_not_before_qpc,
            std::mem::take(&mut retry_pending),
            &mut retry_pending,
        );
        assert_eq!(
            action,
            super::super::DispatchReplanAction::ContinueImmediately,
            "{control} must preempt permit contention backoff"
        );
        assert!(!retry_pending, "{control} clears the one-shot retry token");
        assert_eq!(harness.local_metrics.wait_interrupted_count, 0);
        assert!(packets.lock().expect("packet capture").is_empty());
    }
}

#[test]
fn interrupt_wakes_the_bounded_owner_permit_retry_wait() {
    let actions = [
        timing_action(6682, ActionKind::Down, 0),
        timing_action(6683, ActionKind::Up, 1_000_000),
    ];
    let (mut harness, packets, _) = timing_harness_without_focus(&actions, &[]);
    let now = harness.qpc_now_for_test().expect("outer-loop QPC");
    let (candidate, _) = stage_from_worker_outer_evaluation(&mut harness, now);
    assert!(harness.interrupt.signal(), "signal retry-wait interrupt");
    let mut retry_pending = false;
    let action = super::super::wait_after_dispatch_replan(
        super::super::DispatchAttemptOutcome::PermitBusy,
        true,
        &mut retry_pending,
        super::super::DispatchReplanWaitContext {
            target: &harness.target,
            expected_target: candidate.target_stamp,
            retry_deadline_qpc: candidate.timing_window.packet_not_before_qpc,
            require_focus: harness.config.focus.require_focus,
            focus_active: &harness.focus_active,
            control_signals: super::super::FinalControlSignals {
                quit_requested: &harness.quit_requested,
                skip_requested: &harness.skip_requested,
                panic_requested: &harness.panic_requested,
                desired_pause: &harness.desired_pause,
                supervisor_expired: &harness.supervisor_expired,
                system_power: Some(&harness.system_power),
            },
            qpc_clock: harness.resources.clock,
            waiter: &harness.resources.waiter,
            interrupt: &harness.interrupt,
            local_metrics: &mut harness.local_metrics,
            terminal_error: &mut harness.runtime.terminal_error,
        },
    );
    assert!(matches!(
        action,
        super::super::DispatchReplanAction::WaitCompleted
    ));
    assert!(
        super::super::last_dispatch_replan_wait_for_test()
            .expect("interrupted wait trace")
            .interrupted
    );
    assert_eq!(harness.local_metrics.wait_interrupted_count, 1);
    assert!(packets.lock().expect("packet capture").is_empty());
}
