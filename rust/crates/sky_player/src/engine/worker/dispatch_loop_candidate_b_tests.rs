use super::support::{candidate_at, test_action};
use crate::engine::config::AdmissionVariant;
use crate::engine::test_support::ProductionDispatchTestHarness;
use crate::engine::worker::admission::{MusicalPacketKind, perform_candidate_b_precheck};
use crate::engine::worker::{FinalControlSignals, StagedDispatchOutcome};
use sky_dispatch_core::model::ActionKind;
use sky_dispatch_core::time::DurationTicks;

struct ForegroundTestReset;

impl Drop for ForegroundTestReset {
    fn drop(&mut self) {
        sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        sky_dispatch_win32::focus::set_window_live_for_test(None);
    }
}

#[test]
fn candidate_b_up_only_fast_crossing_single_query_succeeds() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    // First Down dispatched via Control A to create release obligation
    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    // Perform precheck ahead of crossing for Up-only packet
    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration_from_us");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("Candidate B precheck for Up-only succeeds");

    harness.runtime.candidate_b_pending_precheck = Some(proof);

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.runtime.candidate_b_fast_crossings, 1);
    assert_eq!(harness.runtime.candidate_b_fallbacks_to_a, 0);
    assert_eq!(packets.lock().expect("packet capture").len(), 2);
}

#[test]
fn candidate_b_down_packet_is_strictly_forced_through_control_a() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    // Configure Candidate B
    harness.config.admission_variant = AdmissionVariant::B;
    let _packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let candidate = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Dispatched
    );
    // P0-3 invariant: Even with AdmissionVariant::B on ReceiveOnly, Down packets
    // are strictly forced through Control A. candidate_b_fast_crossings must be 0!
    assert_eq!(harness.runtime.candidate_b_fast_crossings, 0);
    assert_eq!(harness.runtime.candidate_b_fallbacks_to_a, 0);
}

#[test]
fn candidate_b_stale_proof_due_to_preemption_falls_back_to_variant_a() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let _packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    // Create a stale precheck proof (sample time 600 us in the past)
    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration_from_us");
    let mut proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("Candidate B precheck should succeed");

    // Artificially age the proof past the 500 us maximum
    let offset_600us = harness
        .resources
        .clock
        .duration_from_us(600)
        .expect("duration_from_us");
    proof.precheck_begin_qpc = sky_dispatch_core::time::QpcTicks::from_raw(
        proof
            .precheck_begin_qpc
            .as_u64()
            .saturating_sub(offset_600us.as_u64()),
    );

    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Crossing detects stale proof and falls back to Control A without dropping the packet
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.runtime.candidate_b_fast_crossings, 0);
    assert_eq!(harness.runtime.candidate_b_fallbacks_to_a, 1);
}

#[test]
fn candidate_b_missing_precheck_falls_back_to_variant_a() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let _packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    // No precheck proof provided (missed precheck)
    harness.runtime.candidate_b_pending_precheck = None;

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.runtime.candidate_b_fast_crossings, 0);
    assert_eq!(harness.runtime.candidate_b_fallbacks_to_a, 1);
}

#[test]
fn candidate_b_focus_lost_before_crossing_enters_focus_pause_and_replans() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration_from_us");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("Candidate B precheck should succeed");

    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Change foreground right before crossing
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(2));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Replan
    );
    assert_eq!(packets.lock().expect("packet capture").len(), 1);
}

#[test]
fn candidate_b_owner_mismatch_at_crossing_fails_closed() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration_from_us");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("Candidate B precheck should succeed");

    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Same HWND 1, but owner PID drifted to 999
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(999)));

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    );
    assert_eq!(packets.lock().expect("packet capture").len(), 1);
    assert!(
        harness
            .runtime
            .terminal_error
            .as_ref()
            .unwrap()
            .contains("prepared_down_owner_mismatch")
    );
}

#[test]
fn candidate_b_target_invalidated_at_crossing_fails_closed() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration_from_us");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("Candidate B precheck should succeed");

    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Target is invalidated before crossing
    harness.target.invalidate_for_test();

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    );
    assert_eq!(packets.lock().expect("packet capture").len(), 1);
    assert!(
        harness
            .runtime
            .terminal_error
            .as_ref()
            .unwrap()
            .contains("prepared_down_target_invalidated")
    );
}

#[test]
fn candidate_b_release_owner_unbind_at_crossing_fails_closed() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration_from_us");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("Candidate B precheck should succeed");

    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Release owner proof is dropped / forgotten before crossing
    harness
        .resources
        .backend
        .forget_release_owner_proof_for_test(0xffff);

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    );
    assert_eq!(packets.lock().expect("packet capture").len(), 1);
    assert!(
        harness
            .runtime
            .terminal_error
            .as_ref()
            .unwrap()
            .contains("owner proof is missing")
    );
}

#[test]
fn candidate_b_target_process_exit_at_crossing_fails_closed() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration_from_us");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("Candidate B precheck should succeed");

    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Target process exits before crossing (handle signaled)
    let (_, generation) = harness.target.load_stable().unwrap();
    if let Some(authority) = harness.target.owner_authority_for_generation(generation) {
        authority.set_process_identity_status_for_test(
            sky_dispatch_win32::focus::WindowIdentityContinuity::ProcessTerminated,
        );
    }

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    );
    assert_eq!(packets.lock().expect("packet capture").len(), 1);
    assert!(
        harness
            .runtime
            .terminal_error
            .as_ref()
            .unwrap()
            .contains("prepared_down_owner_process_terminated")
    );
}

#[test]
fn candidate_b_lease_expiry_tied_to_supervisor_progress_boundary() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let _packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    // Supervisor last progress was at epoch; timeout is 10_000 ticks.
    // The lease expires strictly at epoch + 10_000 ticks, tied to supervisor progress.
    harness.supervisor_expired.reset(epoch);
    let lease_timeout_ticks = DurationTicks::from_raw(10_000);
    let (proof, expiry, progress) = harness
        .supervisor_expired
        .prove_down_age_coherent(epoch, lease_timeout_ticks);
    assert_eq!(
        proof.admission,
        crate::engine::shared::SupervisorLeaseAdmission::Fresh
    );
    assert_eq!(progress, epoch.as_u64());
    assert_eq!(
        expiry.expect("lease expiry ticks"),
        sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 10_000)
    );
}

#[test]
fn candidate_b_game_session_strictly_retains_control_a() {
    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    // Standard game session: NOT marked as receive_only_sink
    harness.config.admission_variant = AdmissionVariant::B;
    let _packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let candidate = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Dispatched
    );
    // Because target is not an identity-verified ReceiveOnly sink, Candidate B was strictly bypassed
    assert_eq!(harness.runtime.candidate_b_fast_crossings, 0);
    assert_eq!(harness.runtime.candidate_b_fallbacks_to_a, 0);
}

#[test]
fn candidate_b_qpc_clock_failure_in_precheck_fails_closed() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let _packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration_from_us");

    // Injected QPC fault causes QPC read in precheck to fail closed
    sky_dispatch_win32::clock::set_qpc_now_fault_for_test(true);
    let precheck_result = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    );
    sky_dispatch_win32::clock::set_qpc_now_fault_for_test(false);

    assert!(matches!(
        precheck_result,
        Err(crate::engine::worker::admission::FinalDownAdmissionResult::Terminal(ref err))
            if err.contains("prepared_down_qpc_failure")
    ));
}

#[test]
fn candidate_b_post_focus_quit_race_fails_closed_zero_send() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(packets.lock().expect("packets").len(), 1);

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("precheck proof");
    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Inject Quit during post-focus hook
    harness.set_post_focus_revalidation_race_hook(|_, _, quit, _, _, _, _| {
        quit.store(true, std::sync::atomic::Ordering::Release);
    });

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    );

    // Invariants: ZERO additional raw sends, release obligation preserved without regression
    assert_eq!(packets.lock().expect("packets").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 0x0001);
}

#[test]
fn candidate_b_post_focus_suspend_race_fails_closed_zero_send() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("precheck proof");
    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Inject SystemSuspend during post-focus hook
    let interrupt = harness.interrupt.clone();
    harness.set_post_focus_revalidation_race_hook(move |_, _, _, _, _, _, power| {
        power.notify(true, &interrupt);
    });

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    );

    assert_eq!(
        harness.runtime.terminal_trigger,
        Some(crate::engine::worker::control::TerminalTrigger::SystemSuspend)
    );
    assert_eq!(packets.lock().expect("packets").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 0x0001);
}

#[test]
fn candidate_b_post_focus_pause_race_replans_zero_send() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("precheck proof");
    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Inject Pause during post-focus hook
    harness.set_post_focus_revalidation_race_hook(|_, _, _, _, _, pause, _| {
        pause.store(true, std::sync::atomic::Ordering::Release);
    });

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Replan
    );

    assert_eq!(packets.lock().expect("packets").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 0x0001);
}

#[test]
fn candidate_b_post_focus_target_invalidated_race_fails_closed_zero_send() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("precheck proof");
    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Invalidate target during post-focus hook
    harness.set_post_focus_revalidation_race_hook(|_, target, _, _, _, _, _| {
        target.invalidate_target();
    });

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    );

    assert_eq!(packets.lock().expect("packets").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 0x0001);
}

#[test]
fn candidate_b_post_focus_lease_expired_race_fails_closed_zero_send() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("precheck proof");
    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Latch supervisor lease expired
    harness.supervisor_expired.latch_expired();

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Exit
    );

    assert_eq!(packets.lock().expect("packets").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 0x0001);
}

#[test]
fn candidate_b_precheck_zero_progress_or_timeout_fails_closed() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let _packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration");

    // 1. Zero timeout cleanly returns Replan (falling back to Control A to preserve disabled-lease semantics)
    let zero_timeout_res = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        DurationTicks::ZERO,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    );
    assert!(matches!(
        zero_timeout_res,
        Err(crate::engine::worker::admission::FinalDownAdmissionResult::Replan)
    ));

    // 2. Zero progress supervisor state fails closed as unproven
    let uninitialized_supervisor =
        crate::engine::shared::SupervisorLeaseState::new(sky_dispatch_core::time::QpcTicks::ZERO);
    let zero_progress_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &uninitialized_supervisor,
        system_power: Some(&harness.system_power),
    };
    let zero_progress_res = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        zero_progress_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    );
    assert!(matches!(
        zero_progress_res,
        Err(crate::engine::worker::admission::FinalDownAdmissionResult::Terminal(ref err))
            if err.contains("supervisor_lease_unproven")
    ));
}

#[test]
fn candidate_b_crossing_qpc_clock_failure_fails_closed_zero_send() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("precheck proof");
    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Injected QPC fault causes crossing QPC read to fail closed
    sky_dispatch_win32::clock::set_qpc_now_fault_for_test(true);
    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    sky_dispatch_win32::clock::set_qpc_now_fault_for_test(false);

    assert_eq!(outcome, StagedDispatchOutcome::Exit);
    assert_eq!(packets.lock().expect("packets").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 0x0001);
}

#[test]
fn candidate_b_precheck_callsite_qpc_clock_failure_sets_terminal_error_and_fails_closed() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    // Injected QPC fault causes caller seam execution to fail closed and set terminal_error
    sky_dispatch_win32::clock::set_qpc_now_fault_for_test(true);
    let precheck_res = harness.execute_candidate_b_precheck_for_test(&candidate);
    sky_dispatch_win32::clock::set_qpc_now_fault_for_test(false);

    assert!(precheck_res.is_err());
    assert!(
        harness
            .runtime
            .terminal_error
            .as_ref()
            .is_some_and(|err| err.contains("prepared_down_qpc_failure"))
    );
    assert!(harness.runtime.candidate_b_pending_precheck.is_none());
    assert_eq!(packets.lock().expect("packets").len(), 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 0x0001);
}

#[test]
fn candidate_b_post_focus_proof_age_at_cap_500us_succeeds_fast_path() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("precheck proof");
    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Override QPC at final boundary to exactly precheck_begin + max_age (equality at cap)
    let at_cap_qpc = sky_dispatch_core::time::QpcTicks::from_raw(
        proof.precheck_begin_qpc.as_u64() + proof.max_age_ticks.as_u64(),
    );
    harness.set_post_focus_revalidation_race_hook(move |_, _, _, _, _, _, _| {
        sky_dispatch_win32::clock::set_qpc_now_override_for_test(Some(at_cap_qpc));
    });

    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    sky_dispatch_win32::clock::set_qpc_now_override_for_test(None);

    assert_eq!(outcome, StagedDispatchOutcome::Dispatched);
    assert_eq!(harness.runtime.candidate_b_fast_crossings, 1);
    assert_eq!(harness.runtime.candidate_b_fallbacks_to_a, 0);
    assert_eq!(packets.lock().expect("packets").len(), 2);
}

#[test]
fn candidate_b_post_focus_proof_age_beyond_cap_500us_falls_back_to_control_a() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundTestReset;

    let actions = [
        test_action(1, ActionKind::Down, 0, 0x15),
        test_action(2, ActionKind::Up, 20_000, 0x15),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let epoch = harness.qpc_now_for_test().expect("test epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("reanchor playback");
    harness.timing.lease_timeout_ticks = harness
        .resources
        .clock
        .duration_from_us(100_000)
        .expect("lease timeout");
    harness.supervisor_expired.reset(epoch);
    harness.configure_focus_owner_for_test(1, 101);
    harness.mark_as_receive_only_sink_for_test();
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let first = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(first, false),
        StagedDispatchOutcome::Dispatched
    );

    let up_epoch = sky_dispatch_core::time::QpcTicks::from_raw(epoch.as_u64() + 20_000);
    let candidate = harness
        .stage_candidate_for_test(
            0x0001,
            None,
            sky_dispatch_core::time::TimelineTicks::from_raw(20_000),
        )
        .expect("stage Up")
        .expect("Up candidate");
    harness.set_mock_sender_start_qpc_for_test(up_epoch);

    let precheck_signals = FinalControlSignals {
        quit_requested: &harness.quit_requested,
        skip_requested: &harness.skip_requested,
        panic_requested: &harness.panic_requested,
        desired_pause: &harness.desired_pause,
        supervisor_expired: &harness.supervisor_expired,
        system_power: Some(&harness.system_power),
    };
    let max_proof_age_ticks = harness
        .resources
        .clock
        .duration_from_us(500)
        .expect("duration");
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::UpOnly,
        max_proof_age_ticks,
    )
    .expect("precheck proof");
    harness.runtime.candidate_b_pending_precheck = Some(proof);

    // Override QPC at final boundary to precheck_begin + max_age + 1 (beyond cap: 501 µs)
    let beyond_cap_qpc = sky_dispatch_core::time::QpcTicks::from_raw(
        proof.precheck_begin_qpc.as_u64() + proof.max_age_ticks.as_u64() + 1,
    );
    harness.set_post_focus_revalidation_race_hook(move |_, _, _, _, _, _, _| {
        sky_dispatch_win32::clock::set_qpc_now_override_for_test(Some(beyond_cap_qpc));
    });

    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    sky_dispatch_win32::clock::set_qpc_now_override_for_test(None);

    // Must cleanly fall back through full Control A, not take Candidate B fast path
    assert_eq!(outcome, StagedDispatchOutcome::Dispatched);
    assert_eq!(harness.runtime.candidate_b_fast_crossings, 0);
    assert_eq!(harness.runtime.candidate_b_fallbacks_to_a, 1);
    assert_eq!(packets.lock().expect("packets").len(), 2);
}
