use super::support::{candidate_at, target_at, test_chord, wait_until_qpc};
use crate::engine::test_support::ProductionDispatchTestHarness;
use crate::engine::worker::StagedDispatchOutcome;
use sky_dispatch_win32::input::PhysicalPacket;

struct ForegroundTestReset;

impl Drop for ForegroundTestReset {
    fn drop(&mut self) {
        sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
    }
}

#[test]
fn immutable_session_target_loss_withholds_origin_up_and_keeps_release_obligation() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;
    let chords = [test_chord(1, 0, 0x15)];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let down = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(down, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.release_obligation_mask_for_test(), 1);

    let up_at = target_at(&harness, epoch, 20_000);
    wait_until_qpc(&harness, up_at);
    let up = candidate_at(&mut harness, up_at, epoch);
    // A changed foreground cannot replace the session's HWND/PID proof. The
    // harness invalidates the original target to model terminal owner loss.
    harness.invalidate_focus_owner_for_test(2, 202);
    harness.set_mock_sender_start_qpc_for_test(up_at);
    assert_ne!(
        harness.dispatch_staged_candidate_for_test(up, false),
        StagedDispatchOutcome::Dispatched,
        "an Up authored under the original target must never be sent to a replacement"
    );

    let captured = packets.lock().expect("mock sender capture");
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].0, PhysicalPacket::new(0, 1));
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
}
