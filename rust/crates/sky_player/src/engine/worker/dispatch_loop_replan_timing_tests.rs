use super::support::ProductionDispatchTestHarness;
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::QpcTicks;

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
    let candidate = super::super::stage_scheduler_action_preserving_due_up(
        send_action,
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
