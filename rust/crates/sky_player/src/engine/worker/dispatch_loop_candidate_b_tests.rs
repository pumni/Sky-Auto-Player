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
    }
}

#[test]
fn candidate_b_fast_crossing_single_query_succeeds() {
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
    harness.configure_focus_owner_for_test(1, 101);
    harness.config.admission_variant = AdmissionVariant::B;
    let _packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let candidate = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);

    // Perform precheck ahead of crossing
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
        .unwrap_or(DurationTicks::ZERO);
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::DownOnly,
        max_proof_age_ticks,
    )
    .expect("Candidate B precheck should succeed");

    harness.runtime.candidate_b_pending_precheck = Some(proof);

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(harness.runtime.candidate_b_fast_crossings, 1);
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
    harness.configure_focus_owner_for_test(1, 101);
    harness.config.admission_variant = AdmissionVariant::B;
    let _packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let candidate = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);

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
        .unwrap_or(DurationTicks::ZERO);
    let mut proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::DownOnly,
        max_proof_age_ticks,
    )
    .expect("Candidate B precheck should succeed");

    // Artificially age the proof past the 500 us maximum
    let offset_600us = harness
        .resources
        .clock
        .duration_from_us(600)
        .unwrap_or(DurationTicks::ZERO);
    proof.sample_qpc = sky_dispatch_core::time::QpcTicks::from_raw(
        proof
            .sample_qpc
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
    harness.configure_focus_owner_for_test(1, 101);
    harness.config.admission_variant = AdmissionVariant::B;
    let _packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let candidate = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);

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
    harness.configure_focus_owner_for_test(1, 101);
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let candidate = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);

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
        .unwrap_or(DurationTicks::ZERO);
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::DownOnly,
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
    assert_eq!(packets.lock().expect("packet capture").len(), 0);
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
    harness.configure_focus_owner_for_test(1, 101);
    harness.config.admission_variant = AdmissionVariant::B;
    let packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let candidate = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);

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
        .unwrap_or(DurationTicks::ZERO);
    let proof = perform_candidate_b_precheck(
        &harness.target,
        candidate.target_stamp.unwrap(),
        harness.config.focus.require_focus,
        &harness.focus_active,
        precheck_signals,
        harness.timing.lease_timeout_ticks,
        harness.resources.clock,
        &harness.resources.backend,
        MusicalPacketKind::DownOnly,
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
    assert_eq!(packets.lock().expect("packet capture").len(), 0);
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
    // Focus-free game session without retained owner process authority
    harness.config.admission_variant = AdmissionVariant::B;
    let _packets = harness.configure_mock_packet_sender_for_test(&[]);
    harness.attach_executor_for_test().expect("attach executor");

    let candidate = candidate_at(&mut harness, epoch, epoch);
    harness.set_mock_sender_start_qpc_for_test(epoch);

    assert_eq!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Dispatched
    );
    // Because owner_process_identity_required is false, Candidate B was bypassed
    assert_eq!(harness.runtime.candidate_b_fast_crossings, 0);
    assert_eq!(harness.runtime.candidate_b_fallbacks_to_a, 0);
}
