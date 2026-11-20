use super::support::ProductionDispatchTestHarness;
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::{DurationTicks, QpcTicks};

pub(super) fn timing_action(index: u32, kind: ActionKind, scheduled_us: u64) -> KeyActionInput {
    KeyActionInput {
        source_action_index: index,
        kind,
        scheduled_us,
        scan_codes: vec![0x15].into(),
        reason: format!("replan-timing-{index}").into(),
    }
}

pub(super) fn timing_harness_without_focus(
    actions: &[KeyActionInput],
    sender_latencies_us: &[u64],
) -> (
    ProductionDispatchTestHarness,
    crate::engine::test_support::PacketCapture,
    QpcTicks,
) {
    let mut harness = ProductionDispatchTestHarness::create_harness(actions);
    let epoch = harness.qpc_now_for_test().expect("test QPC epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback clock");
    let packets = harness.configure_mock_packet_sender_for_test(sender_latencies_us);
    harness
        .attach_executor_for_test()
        .expect("production executor invariant");
    (harness, packets, epoch)
}

pub(super) fn stage_from_worker_outer_evaluation(
    harness: &mut ProductionDispatchTestHarness,
    now_qpc: QpcTicks,
) -> (crate::engine::worker::StagedCandidate, u64) {
    super::super::reset_scheduler_evaluation_count_for_test();
    let epoch = harness.resources.playback.epoch;
    let (action, _scheduler_wait_target_qpc, action_to_stage) =
        super::super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            now_qpc,
            epoch,
        )
        .expect("worker scheduler evaluation");
    assert!(matches!(action, super::super::SchedulerAction::Send { .. }));
    let send_action = action_to_stage.expect("due scheduler action stages a packet");
    let candidate = super::super::stage_scheduler_send_candidate(
        send_action,
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            capture_up_slot_timings: false,
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage worker packet")
    .expect("scheduler candidate is stageable");
    let count = super::super::scheduler_evaluation_count_for_test();
    (candidate, count)
}

pub(super) fn elapsed_us(
    clock: sky_dispatch_win32::clock::QpcClock,
    start: QpcTicks,
    end: QpcTicks,
) -> u64 {
    let ticks = end
        .checked_duration_since(start)
        .expect("trace timestamps are monotonic");
    clock
        .duration_to_us(sky_dispatch_core::time::DurationTicks::from_raw(
            ticks.as_u64(),
        ))
        .expect("trace duration converts to microseconds")
}

fn wait_for_deadline(
    harness: &mut ProductionDispatchTestHarness,
    deadline_qpc: QpcTicks,
) -> super::super::WaitBoundary {
    super::super::wait_for_next_boundary(super::super::WaitBoundaryInput {
        deadline: super::super::WaitDeadline {
            physical_target_qpc: Some(deadline_qpc),
            spin_threshold_ticks: DurationTicks::ZERO,
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

fn up_only_send_action(
    action: Option<super::super::SchedulerAction>,
) -> super::super::SchedulerAction {
    let Some(action @ super::super::SchedulerAction::Send { .. }) = action else {
        panic!("scheduler must stage an Up-only packet");
    };
    assert!(matches!(
        action,
        super::super::SchedulerAction::Send {
            up_mask: 1,
            down_chord: None,
            ..
        }
    ));
    action
}

fn stage_scheduler_action(
    harness: &mut ProductionDispatchTestHarness,
    action: super::super::SchedulerAction,
) -> crate::engine::worker::StagedCandidate {
    super::super::stage_scheduler_send_candidate(
        action,
        super::super::SchedulerStageAdmission {
            target: &harness.target,
            capture_up_slot_timings: false,
        },
        &mut harness.resources,
        &mut harness.runtime,
    )
    .expect("stage scheduler-selected packet")
    .expect("scheduler candidate is stageable")
}

#[test]
fn scheduler_selects_independent_up_before_down_and_replans_after_interrupt() {
    let actions = [
        timing_action(810, ActionKind::Down, 0),
        timing_action(811, ActionKind::Up, 300_000),
        KeyActionInput {
            source_action_index: 812,
            kind: ActionKind::Down,
            scheduled_us: 600_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "later scheduler Down".into(),
        },
        KeyActionInput {
            source_action_index: 813,
            kind: ActionKind::Up,
            scheduled_us: 700_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "later scheduler Up".into(),
        },
    ];
    let (mut harness, packets, epoch) = timing_harness_without_focus(&actions, &[0, 0]);
    let initial_now = harness.qpc_now_for_test().expect("initial Down QPC");
    let initial = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            sky_dispatch_core::time::TimelineTicks::ZERO,
        )
        .expect("stage first Down")
        .expect("first Down candidate");
    harness.set_mock_sender_start_qpc_for_test(initial_now);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(initial, false),
        super::super::StagedDispatchOutcome::Dispatched
    );

    let now = harness.qpc_now_for_test().expect("pre-wait QPC");
    let up_authored_target = epoch
        .checked_add_duration(DurationTicks::from_raw(
            harness
                .qpc_duration_from_us_for_test(300_000)
                .expect("Up target conversion")
                .as_u64(),
        ))
        .expect("Up target QPC");
    let down_authored_target = epoch
        .checked_add_duration(DurationTicks::from_raw(
            harness
                .qpc_duration_from_us_for_test(600_000)
                .expect("Down floor conversion")
                .as_u64(),
        ))
        .expect("Down target QPC");
    assert!(now < up_authored_target);

    let (action, up_deadline, action_to_stage) =
        super::super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            now,
            harness.resources.playback.epoch,
        )
        .expect("production scheduler evaluation before Up deadline");
    assert!(matches!(
        action,
        super::super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc == up_deadline
    ));
    assert!(up_deadline >= up_authored_target);
    assert!(up_deadline < down_authored_target);
    let first_candidate =
        stage_scheduler_action(&mut harness, up_only_send_action(action_to_stage));
    assert_eq!(
        first_candidate.packet_masks,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 0)
    );
    assert_eq!(
        first_candidate.timing_window.packet_not_before_qpc,
        up_deadline
    );
    assert_eq!(
        harness.pre_wait_candidate_admission_for_test(&first_candidate),
        super::super::StagedDispatchOutcome::Dispatched
    );

    // Exercise the real interrupt event and production wait boundary. The
    // worker drops this candidate and re-evaluates the scheduler on wake.
    harness.interrupt.try_take();
    assert!(harness.interrupt.signal());
    let interrupted = wait_for_deadline(&mut harness, up_deadline);
    assert!(matches!(
        interrupted,
        super::super::WaitBoundary::Interrupted(..)
    ));
    assert_eq!(harness.local_metrics.wait_interrupted_count, 1);
    harness.interrupt.try_take();
    drop(first_candidate);
    assert_eq!(
        packets.lock().expect("packet capture").len(),
        1,
        "an interrupt before the Up floor must not send the Up"
    );

    let replanned_now = harness.qpc_now_for_test().expect("QPC after interrupt");
    assert!(replanned_now < up_deadline);
    let (replanned_action, replanned_deadline, replanned_to_stage) =
        super::super::evaluate_scheduler_action_for_staging(
            &harness.resources.scheduler,
            replanned_now,
            harness.resources.playback.epoch,
        )
        .expect("scheduler re-evaluation after interrupt");
    assert!(matches!(
        replanned_action,
        super::super::SchedulerAction::Wait { deadline_qpc } if deadline_qpc == replanned_deadline
    ));
    assert_eq!(replanned_deadline, up_deadline);
    let replanned_candidate =
        stage_scheduler_action(&mut harness, up_only_send_action(replanned_to_stage));
    assert_eq!(
        harness.pre_wait_candidate_admission_for_test(&replanned_candidate),
        super::super::StagedDispatchOutcome::Dispatched
    );
    let due = wait_for_deadline(&mut harness, replanned_deadline);
    assert!(matches!(
        due,
        super::super::WaitBoundary::Due { target_qpc, .. } if target_qpc == replanned_deadline
    ));
    let send_start = harness.qpc_now_for_test().expect("Up-only sender QPC");
    assert!(send_start >= replanned_deadline);
    harness.set_mock_sender_start_qpc_for_test(send_start);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(replanned_candidate, false),
        super::super::StagedDispatchOutcome::Dispatched
    );
    let packets = packets.lock().expect("packet capture");
    assert_eq!(packets.len(), 2);
    assert_eq!(
        packets[1].0,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 0)
    );
    assert!(
        packets[1]
            .1
            .started_ticks
            .is_some_and(|started| started >= replanned_deadline)
    );
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|chord| chord.down_mask),
        Some(2)
    );
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
}
