fn wait_to_packet_deadline(
    harness: &mut ProductionDispatchTestHarness,
    candidate: &crate::engine::worker::StagedCandidate,
    scheduler_wait_target_qpc: QpcTicks,
) -> (QpcTicks, super::WaitBoundary) {
    assert_eq!(
        harness.pre_wait_candidate_admission_for_test(candidate),
        StagedDispatchOutcome::Dispatched,
        "future candidate passes production pre-wait admission"
    );
    let wait_target_qpc = scheduler_wait_target_qpc
        .max(candidate.timing_window.packet_not_before_qpc);
    let wake = super::wait_for_next_boundary(super::WaitBoundaryInput {
        deadline: super::WaitDeadline {
            physical_target_qpc: Some(wait_target_qpc),
            spin_threshold_ticks: harness.timing.effective_spin_threshold_ticks,
            qpc_clock: harness.resources.clock,
        },
        signals: super::WaitSignals {
            waiter: &harness.resources.waiter,
            interrupt: &harness.interrupt,
        },
        mutable: super::WaitMutable {
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
    harness.set_mock_sender_start_qpc_for_test(
        harness.qpc_now_for_test().expect("Down sender start"),
    );
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(down, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(harness.resources.backend.active_mask, 1);

    // Mock transport evidence places the confirmed Down completion 40 ms
    // after sender start. The production scheduler must retain its completion
    // relative H floor and project a future Up packet without advancing it.
    let up_eval_qpc = harness.qpc_now_for_test().expect("Up scheduler QPC");
    super::reset_scheduler_evaluation_count_for_test();
    let (up_action, up_scheduler_deadline, projected_up) =
        super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            up_eval_qpc,
            epoch,
        )
        .expect("Up scheduler projection");
    let up_projection_evaluations = super::scheduler_evaluation_count_for_test();
    assert!(matches!(
        up_action,
        super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc > up_eval_qpc
    ));
    assert_eq!(up_projection_evaluations, 2, "now plus deadline projection");
    let up = super::stage_scheduler_action_preserving_due_up(
        projected_up.expect("Up projects to a due action at its scheduler deadline"),
        super::SchedulerStageAdmission {
            target: &harness.target,
            require_focus: harness.config.focus.require_focus,
            focus_active: &harness.focus_active,
            capture_up_slot_timings: false,
            control_signals: super::FinalControlSignals {
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
        super::evaluate_scheduler_action(&harness.resources.scheduler, before_h, epoch)
            .expect("H floor minus one tick"),
        super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc == h_floor_qpc
    ));
    assert!(matches!(
        super::evaluate_scheduler_action(&harness.resources.scheduler, h_floor_qpc, epoch)
            .expect("exact H floor"),
        super::SchedulerAction::Send { .. }
    ));
    let after_h = h_floor_qpc
        .checked_add_duration(one_tick)
        .expect("H floor plus one tick");
    assert!(matches!(
        super::evaluate_scheduler_action(&harness.resources.scheduler, after_h, epoch)
            .expect("H floor plus one tick"),
        super::SchedulerAction::Send { .. }
    ));

    let (selected_h_deadline, h_wake) =
        wait_to_packet_deadline(&mut harness, &up, up_scheduler_deadline);
    assert_eq!(selected_h_deadline, h_floor_qpc);
    assert!(matches!(h_wake, super::WaitBoundary::Due { .. }));
    let h_wake_qpc = harness.qpc_now_for_test().expect("H wait wake QPC");
    harness.set_mock_sender_start_qpc_for_test(h_wake_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(up, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.resources.backend.active_mask, 0);

    // The confirmed Up completion starts the G floor. A same-key Down authored
    // at 60 ms remains pending through G-1 and becomes eligible at G exactly.
    let down_b_eval_qpc = harness.qpc_now_for_test().expect("G scheduler QPC");
    super::reset_scheduler_evaluation_count_for_test();
    let (down_b_action, down_b_scheduler_deadline, projected_down_b) =
        super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            down_b_eval_qpc,
            epoch,
        )
        .expect("Down B scheduler projection");
    let g_projection_evaluations = super::scheduler_evaluation_count_for_test();
    assert!(matches!(
        down_b_action,
        super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc > down_b_eval_qpc
    ));
    assert_eq!(g_projection_evaluations, 2);
    let down_b = super::stage_scheduler_action_preserving_due_up(
        projected_down_b.expect("Down B projects to its G deadline"),
        super::SchedulerStageAdmission {
            target: &harness.target,
            require_focus: harness.config.focus.require_focus,
            focus_active: &harness.focus_active,
            capture_up_slot_timings: false,
            control_signals: super::FinalControlSignals {
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
        super::evaluate_scheduler_action(&harness.resources.scheduler, before_g, epoch)
            .expect("G floor minus one tick"),
        super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc == g_floor_qpc
    ));
    assert!(matches!(
        super::evaluate_scheduler_action(&harness.resources.scheduler, g_floor_qpc, epoch)
            .expect("exact G floor"),
        super::SchedulerAction::Send { .. }
    ));
    assert!(matches!(
        super::evaluate_scheduler_action(
            &harness.resources.scheduler,
            g_floor_qpc.checked_add_duration(one_tick).expect("G plus one tick"),
            epoch,
        )
        .expect("G floor plus one tick"),
        super::SchedulerAction::Send { .. }
    ));

    let (selected_g_deadline, g_wake) =
        wait_to_packet_deadline(&mut harness, &down_b, down_b_scheduler_deadline);
    assert_eq!(selected_g_deadline, g_floor_qpc);
    assert!(matches!(g_wake, super::WaitBoundary::Due { .. }));
    let g_wake_qpc = harness.qpc_now_for_test().expect("G wait wake QPC");
    harness.set_mock_sender_start_qpc_for_test(g_wake_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(down_b, false),
        StagedDispatchOutcome::Dispatched
    );

    let sent = packets.lock().expect("packet capture");
    assert_eq!(sent.len(), 3);
    assert_eq!(sent[0].0, sky_dispatch_win32::input::PhysicalPacket::new(0, 1));
    assert_eq!(sent[1].0, sky_dispatch_win32::input::PhysicalPacket::new(1, 0));
    assert_eq!(sent[2].0, sky_dispatch_win32::input::PhysicalPacket::new(0, 1));
    assert!(sent.iter().all(|(_, evidence)| evidence.attempts == 1));
    assert!(sent.iter().all(|(_, evidence)| evidence.first_inserted == 1));
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
        hstart = sent[1].1.started_ticks.expect("Up sender start QPC").as_u64(),
        hwait = elapsed_us(clock, up_eval_qpc, h_wake_qpc),
        geval = down_b_eval_qpc.as_u64(),
        g = g_floor_qpc.as_u64(),
        gsel = selected_g_deadline.as_u64(),
        gwake = g_wake_qpc.as_u64(),
        gstart = sent[2].1.started_ticks.expect("Down B sender start QPC").as_u64(),
        gwait = elapsed_us(clock, down_b_eval_qpc, g_wake_qpc),
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
                super::DispatchAttemptOutcome::PermitBusy,
                candidate.target_stamp,
                candidate.timing_window.packet_not_before_qpc,
                false,
                &mut retry_pending,
            ),
            super::DispatchReplanAction::ContinueImmediately,
            "first transient miss gets one immediate full re-evaluation"
        );
        assert!(retry_pending);
        assert!(matches!(
            apply_outer_replan_policy(
                &mut harness,
                super::DispatchAttemptOutcome::PermitBusy,
                candidate.target_stamp,
                candidate.timing_window.packet_not_before_qpc,
                super::take_permit_busy_retry(&mut retry_pending),
                &mut retry_pending,
            ),
            super::DispatchReplanAction::WaitCompleted {
                interrupted: false,
                ..
            }
        ));
        assert!(!retry_pending, "bounded backoff does not leave a spin token");

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
            super::DispatchAttemptOutcome::PermitBusy,
            candidate.target_stamp,
            candidate.timing_window.packet_not_before_qpc,
            super::take_permit_busy_retry(&mut retry_pending),
            &mut retry_pending,
        );
        assert_eq!(
            action,
            super::DispatchReplanAction::ContinueImmediately,
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
    let action = super::wait_after_dispatch_replan(
        super::DispatchAttemptOutcome::PermitBusy,
        true,
        &mut retry_pending,
        super::DispatchReplanWaitContext {
            target: &harness.target,
            expected_target: candidate.target_stamp,
            retry_deadline_qpc: candidate.timing_window.packet_not_before_qpc,
            require_focus: harness.config.focus.require_focus,
            focus_active: &harness.focus_active,
            control_signals: super::FinalControlSignals {
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
        super::DispatchReplanAction::WaitCompleted {
            interrupted: true,
            ..
        }
    ));
    assert_eq!(harness.local_metrics.wait_interrupted_count, 1);
    assert!(packets.lock().expect("packet capture").is_empty());
}
