use super::preflight_busy::{
    FinalPermitLock, PreflightOwnerForegroundReset, assert_a_obligation_unchanged,
    assert_fresh_b_admission_ran, attempt_due_b_with_policy, authored_qpc_at,
    install_final_permit_contention, make_active_a_preflight_harness, wait_until_qpc,
};
use super::replan_timing::{apply_outer_replan_policy, stage_from_worker_outer_evaluation};
use crate::engine::test_support::{PacketCapture, ProductionDispatchTestHarness};
use crate::engine::worker::DispatchAttemptOutcome;
use sky_dispatch_core::time::QpcTicks;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
fn stage_late_mixed_after_final_authority_busy(
    harness: &mut ProductionDispatchTestHarness,
    packets: &PacketCapture,
    epoch: QpcTicks,
    preflight_calls: &AtomicUsize,
    lease_control_checks: &AtomicUsize,
) -> (
    crate::engine::worker::StagedCandidate,
    crate::engine::worker::TargetStamp,
    sky_dispatch_win32::input::ReleaseOwnerProof,
) {
    let origin = harness
        .runtime
        .verified_target_for_test()
        .expect("validated A-session preflight stamp");
    let a_origin = harness
        .resources
        .backend
        .release_owner_proof_for_mask(1)
        .expect("A release proof lookup before Busy")
        .expect("A release proof before Busy");
    assert_eq!(a_origin.target_hwnd(), origin.hwnd);
    assert_eq!(a_origin.target_generation(), origin.generation);

    let held = install_final_permit_contention(harness, FinalPermitLock::OwnerAuthority);
    let mut retry_pending = false;
    let first = attempt_due_b_with_policy(harness, epoch, &mut retry_pending);
    assert!(!first.retry_allowed);
    assert_eq!(first.outcome, DispatchAttemptOutcome::PermitBusy);
    assert_eq!(first.source_action_index, 6691);
    assert_eq!(first.target_stamp, Some(origin));
    assert_a_obligation_unchanged(harness, packets, preflight_calls, 1, origin);
    assert!(
        harness
            .resources
            .backend
            .release_owner_proof_for_mask(1)
            .expect("A release proof lookup after Busy")
            .expect("A release proof after Busy")
            .same_owner(&a_origin)
    );

    assert_eq!(
        apply_outer_replan_policy(
            harness,
            first.outcome,
            first.target_stamp,
            first.retry_deadline_qpc,
            first.retry_allowed,
            &mut retry_pending,
        ),
        super::super::DispatchReplanAction::ContinueImmediately,
        "the first stable-target Busy keeps the accepted immediate retry"
    );
    assert!(retry_pending);
    held.release
        .send(())
        .expect("release final owner-authority mutex");
    assert!(held.supervisor.join().expect("permit supervisor joins"));
    harness.runtime.final_gate_before_owner_permit_hook = None;

    // Model a worker stall by advancing the next scheduler evaluation to the
    // current QPC after authored Up(A) and its H floor are both eligible.
    // The scheduler's Send result itself proves its H/G admission at this QPC.
    let authored_up_a_qpc = authored_qpc_at(harness, epoch, 80_000);
    wait_until_qpc(harness, authored_up_a_qpc);
    let retry_eval_qpc = harness
        .qpc_now_for_test()
        .expect("fresh retry scheduler QPC");
    assert!(retry_eval_qpc >= authored_up_a_qpc);
    let a_completion_qpc = packets
        .lock()
        .expect("A packet capture")
        .first()
        .expect("A Down evidence")
        .1
        .completed_ticks
        .expect("A Down completion QPC");
    let min_hold_ticks = harness
        .qpc_duration_from_us_for_test(harness.config.timing.min_hold_us)
        .expect("A's configured H floor conversion");
    let a_h_floor_qpc = a_completion_qpc
        .checked_add_duration(min_hold_ticks)
        .expect("A H floor QPC");
    assert!(
        retry_eval_qpc >= a_h_floor_qpc,
        "fresh scheduler evaluation is also at or beyond A's physical H floor"
    );

    harness.reset_target_proof_counts_for_test();
    sky_dispatch_win32::focus::reset_foreground_query_count();
    sky_dispatch_win32::focus::reset_process_identity_counts_for_test();
    lease_control_checks.store(0, Ordering::Release);
    let (candidate, evaluations) = stage_from_worker_outer_evaluation(harness, retry_eval_qpc);
    assert_eq!(
        evaluations, 1,
        "fresh-QPC retry uses one production evaluation"
    );
    assert_eq!(
        candidate.packet_masks.up_mask, 1,
        "authored Up(A) is now eligible"
    );
    assert_eq!(
        candidate.packet_masks.down_mask, 2,
        "pending Down(B) joins the retry"
    );
    assert!(candidate.timing_window.packet_not_before_qpc <= retry_eval_qpc);
    assert_eq!(candidate.target_stamp, Some(origin));
    assert_a_obligation_unchanged(harness, packets, preflight_calls, 1, origin);
    assert!(
        harness
            .resources
            .backend
            .release_owner_proof_for_mask(1)
            .expect("A release proof lookup after mixed staging")
            .expect("A release proof survives mixed staging")
            .same_owner(&a_origin)
    );
    (candidate, origin, a_origin)
}

#[test]
fn final_permit_busy_retries_at_fresh_qpc_as_mixed_a_up_b_down_once() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = PreflightOwnerForegroundReset;
    let (mut harness, packets, epoch, preflight_calls, _, lease_control_checks) =
        make_active_a_preflight_harness();

    let (mixed, origin, a_origin) = stage_late_mixed_after_final_authority_busy(
        &mut harness,
        &packets,
        epoch,
        &preflight_calls,
        &lease_control_checks,
    );
    harness.set_mock_sender_start_qpc_for_test(
        harness.qpc_now_for_test().expect("mixed packet sender QPC"),
    );
    assert_eq!(
        harness.dispatch_staged_candidate_with_reason_for_test(mixed, false),
        DispatchAttemptOutcome::Dispatched,
        "late retry commits the eligible Up(A)+Down(B) packet"
    );
    assert_fresh_b_admission_ran(&harness, &lease_control_checks);
    assert_eq!(preflight_calls.load(Ordering::Acquire), 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 2);
    assert_eq!(harness.resources.backend.active_mask, 2);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 2);
    assert!(
        harness
            .resources
            .backend
            .release_owner_proof_for_mask(1)
            .is_err(),
        "the A proof is removed only after its successful Up commit"
    );
    let b_origin = harness
        .resources
        .backend
        .release_owner_proof_for_mask(2)
        .expect("B release proof lookup")
        .expect("B release proof committed");
    assert!(b_origin.same_owner(&a_origin));
    assert_eq!(b_origin.target_hwnd(), origin.hwnd);
    assert_eq!(b_origin.target_generation(), origin.generation);
    let sent = packets.lock().expect("A/B packet capture");
    assert_eq!(sent.len(), 2, "one A Down then one mixed retry");
    assert_eq!(sent[0].0.up_mask, 0);
    assert_eq!(sent[0].0.down_mask, 1);
    assert_eq!(sent[1].0.up_mask, 1);
    assert_eq!(sent[1].0.down_mask, 2);
    assert!(sent.iter().all(|(_, evidence)| evidence.attempts == 1));
    assert_eq!(sent[0].1.first_inserted, 1);
    assert_eq!(sent[1].1.first_inserted, 2);
    assert!(harness.runtime.terminal_error.is_none());
}

#[test]
fn late_mixed_retry_falls_back_to_up_a_then_dispatches_pending_b_once() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = PreflightOwnerForegroundReset;
    let (mut harness, packets, epoch, preflight_calls, _, lease_control_checks) =
        make_active_a_preflight_harness();
    let (mixed, origin, a_origin) = stage_late_mixed_after_final_authority_busy(
        &mut harness,
        &packets,
        epoch,
        &preflight_calls,
        &lease_control_checks,
    );

    let modifier_held = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let held_for_query = Arc::clone(&modifier_held);
    harness
        .resources
        .backend
        .set_modifier_key_state_query_for_test(move |_| {
            if held_for_query.load(Ordering::Acquire) {
                i16::MIN
            } else {
                0
            }
        });
    let held_for_gate = Arc::clone(&modifier_held);
    harness.set_final_gate_before_syscall_hook(move || {
        held_for_gate.store(true, Ordering::Release);
    });
    harness.set_mock_sender_start_qpc_for_test(
        harness
            .qpc_now_for_test()
            .expect("mixed fallback sender QPC"),
    );
    assert_eq!(
        harness.dispatch_staged_candidate_with_reason_for_test(mixed, false),
        DispatchAttemptOutcome::Dispatched,
        "modifier rejection of Down(B) still admits the independent Up(A)"
    );
    assert_eq!(preflight_calls.load(Ordering::Acquire), 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.resources.backend.active_mask, 0);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    assert_eq!(harness.runtime.verified_target_for_test(), Some(origin));
    let after_up = packets.lock().expect("fallback packet capture");
    assert_eq!(after_up.len(), 2, "the mixed attempt emits only Up(A)");
    assert_eq!(after_up[0].0.down_mask, 1);
    assert_eq!(
        after_up[1].0,
        sky_dispatch_win32::input::PhysicalPacket::new(1, 0)
    );
    assert!(after_up.iter().all(|(_, evidence)| evidence.attempts == 1));
    drop(after_up);

    assert!(a_origin.target_generation() == origin.generation);
    modifier_held.store(false, Ordering::Release);
    harness.runtime.final_gate_before_syscall_hook = None;
    let retry_eval_qpc = harness.qpc_now_for_test().expect("Down(B) restage QPC");
    let (down_b, evaluations) = stage_from_worker_outer_evaluation(&mut harness, retry_eval_qpc);
    assert_eq!(evaluations, 1);
    assert_eq!(down_b.packet_masks.up_mask, 0);
    assert_eq!(down_b.packet_masks.down_mask, 2);
    harness.set_mock_sender_start_qpc_for_test(retry_eval_qpc);
    assert_eq!(
        harness.dispatch_staged_candidate_with_reason_for_test(down_b, false),
        DispatchAttemptOutcome::Dispatched
    );
    assert_eq!(preflight_calls.load(Ordering::Acquire), 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 2);
    assert_eq!(harness.resources.backend.active_mask, 2);
    assert_eq!(harness.release_obligation_mask_for_test(), 2);
    let sent = packets.lock().expect("A/Up(A)/B packet capture");
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
        sky_dispatch_win32::input::PhysicalPacket::new(0, 2)
    );
    assert!(sent.iter().all(|(_, evidence)| evidence.attempts == 1));
    assert!(harness.runtime.terminal_error.is_none());
}

#[test]
fn late_mixed_retry_cannot_send_a_up_under_rebound_or_aba_owner() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = PreflightOwnerForegroundReset;
    for rebound_hwnd in [2, 1] {
        let (mut harness, packets, epoch, preflight_calls, _, lease_control_checks) =
            make_active_a_preflight_harness();
        let (mixed, origin, a_origin) = stage_late_mixed_after_final_authority_busy(
            &mut harness,
            &packets,
            epoch,
            &preflight_calls,
            &lease_control_checks,
        );
        harness.republish_focus_owner_for_test(rebound_hwnd, 202);
        let rebound = super::super::load_target_stamp(&harness.target)
            .expect("new owner binding after mixed staging");
        assert_ne!(rebound.generation, origin.generation);
        assert_eq!(rebound.hwnd == origin.hwnd, rebound_hwnd == origin.hwnd);
        assert_eq!(mixed.packet_masks.up_mask, 1);
        assert_eq!(mixed.packet_masks.down_mask, 2);
        let outcome = harness.dispatch_staged_candidate_with_reason_for_test(mixed, false);
        assert_eq!(
            outcome,
            DispatchAttemptOutcome::Exit,
            "A→B and same-HWND ABA fail closed on the old Up origin"
        );
        assert!(
            harness
                .runtime
                .terminal_error
                .as_deref()
                .is_some_and(|error| error.contains("musical Up origin owner differs"))
        );
        assert_eq!(packets.lock().expect("no cross-owner syscall").len(), 1);
        assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
        assert_eq!(harness.resources.backend.active_mask, 1);
        assert_eq!(harness.resources.backend.possibly_active_mask, 0);
        assert_eq!(harness.resources.backend.in_flight_mask, 0);
        assert_eq!(harness.release_obligation_mask_for_test(), 1);
        let retained = harness
            .resources
            .backend
            .release_owner_proof_for_mask(1)
            .expect("A origin after rebind")
            .expect("A obligation remains");
        assert!(retained.same_owner(&a_origin));
        assert_eq!(retained.target_hwnd(), origin.hwnd);
        assert_eq!(retained.target_generation(), origin.generation);
        assert_eq!(preflight_calls.load(Ordering::Acquire), 1);
        assert_ne!(harness.runtime.verified_target_for_test(), Some(rebound));
    }
}
