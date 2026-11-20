use super::support::{ProductionDispatchTestHarness, wait_until_qpc};
use sky_dispatch_core::executor::DispatchTimingWindow;
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

#[test]
fn pre_wait_up_prefix_uses_its_own_floor_and_keeps_down_pending() {
    let actions = [
        timing_action(810, ActionKind::Down, 0),
        timing_action(811, ActionKind::Up, 20_000),
        KeyActionInput {
            source_action_index: 812,
            kind: ActionKind::Down,
            scheduled_us: 100_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "pre-wait independent Down".into(),
        },
        KeyActionInput {
            source_action_index: 813,
            kind: ActionKind::Up,
            scheduled_us: 120_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "pre-wait independent Up".into(),
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

    let up_target = epoch
        .checked_add_duration(DurationTicks::from_raw(
            harness
                .qpc_duration_from_us_for_test(20_000)
                .expect("Up target conversion")
                .as_u64(),
        ))
        .expect("Up target QPC");
    let down_floor = epoch
        .checked_add_duration(DurationTicks::from_raw(
            harness
                .qpc_duration_from_us_for_test(100_000)
                .expect("Down floor conversion")
                .as_u64(),
        ))
        .expect("Down floor QPC");
    let mixed = harness
        .stage_candidate_with_scheduler_window_for_test(
            1,
            harness.current_executor_chord_for_test(),
            DispatchTimingWindow {
                authored_target_qpc: up_target,
                musical_up_not_before_qpc: up_target,
                down_not_before_qpc: down_floor,
                packet_not_before_qpc: down_floor,
                hold_floor_mask: 0,
                release_floor_mask: 0,
            },
        )
        .expect("stage Mixed scheduler candidate")
        .expect("Mixed candidate");
    let now = harness.qpc_now_for_test().expect("pre-wait QPC");
    assert!(now < down_floor);
    let controls = super::super::FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let mut mixed = mixed;
    let up_deadline = super::super::maybe_select_earlier_up_prefix_before_wait(
        &mut mixed,
        now,
        &harness.target,
        harness.config.focus.require_focus,
        &harness.focus_active,
        controls,
    )
    .expect("pre-wait Up-prefix selection")
    .expect("Up has an earlier independent deadline");
    assert!(mixed.up_prefix_selected);
    assert!(up_deadline >= up_target);
    assert!(up_deadline < down_floor);

    wait_until_qpc(&harness, up_deadline);
    let send_start = harness.qpc_now_for_test().expect("Up-prefix sender QPC");
    harness.set_mock_sender_start_qpc_for_test(send_start);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(mixed, false),
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
            .is_some_and(|started| started >= up_deadline)
    );
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|chord| chord.down_mask),
        Some(2)
    );
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
}
