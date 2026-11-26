pub(super) use crate::engine::test_support::ProductionDispatchTestHarness;
pub(super) use sky_dispatch_core::executor::{
    DispatchDecision, DispatchTimingWindow, PreparedChord,
};
pub(super) use sky_dispatch_core::model::OnsetChordInput;
pub(super) use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
pub(super) use sky_dispatch_win32::input::{
    PacketRetryReason, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
};
pub(super) use std::sync::atomic::Ordering;

pub(super) use super::super::StagedDispatchOutcome;

pub(super) fn staged_send_at(
    harness: &mut ProductionDispatchTestHarness,
    at: QpcTicks,
    epoch: QpcTicks,
) -> (u16, Option<PreparedChord>, DispatchTimingWindow) {
    match harness
        .evaluate_live_scheduler_for_test(at, epoch)
        .expect("evaluate production scheduler")
    {
        DispatchDecision::Send {
            up_mask,
            down_chord,
            timing_window,
            ..
        } => (up_mask, down_chord, timing_window),
        decision => panic!("expected packet send at {at:?}, got {decision:?}"),
    }
}

pub(super) fn target_at(
    harness: &ProductionDispatchTestHarness,
    epoch: QpcTicks,
    at_us: u64,
) -> QpcTicks {
    epoch
        .checked_add_duration(DurationTicks::from_raw(qpc_offset(harness, at_us).as_u64()))
        .expect("authored QPC target")
}

pub(super) fn candidate_at(
    harness: &mut ProductionDispatchTestHarness,
    at: QpcTicks,
    epoch: QpcTicks,
) -> crate::engine::worker::StagedCandidate {
    let (up_mask, down_chord, timing_window) = staged_send_at(harness, at, epoch);
    harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage production candidate")
        .expect("candidate is due")
}

pub(super) fn qpc_offset(harness: &ProductionDispatchTestHarness, us: u64) -> TimelineTicks {
    TimelineTicks::from_raw(
        harness
            .qpc_duration_from_us_for_test(us)
            .expect("convert authored offset")
            .as_u64(),
    )
}

pub(super) fn wait_until_qpc(harness: &ProductionDispatchTestHarness, target: QpcTicks) {
    while harness.qpc_now_for_test().expect("wait QPC") < target {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

pub(super) fn test_chord(index: u32, scheduled_us: u64, scan: u16) -> OnsetChordInput {
    OnsetChordInput {
        source_action_index: index,
        scheduled_us,
        scan_codes: smallvec::smallvec![scan],
    }
}
