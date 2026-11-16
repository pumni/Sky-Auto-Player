use crate::engine::test_support::{PacketCapture, ProductionDispatchTestHarness};
use crate::engine::worker::{DispatchAttemptOutcome, StagedDispatchOutcome};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::QpcTicks;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy)]
enum FinalPermitLock {
    Writer,
    OwnerAuthority,
}

struct HeldFinalPermit {
    release: std::sync::mpsc::Sender<()>,
    supervisor: std::thread::JoinHandle<bool>,
}

struct DueBAttempt {
    source_action_index: u32,
    target_stamp: Option<crate::engine::worker::TargetStamp>,
    retry_deadline_qpc: QpcTicks,
    outcome: DispatchAttemptOutcome,
    retry_allowed: bool,
}

struct PreflightOwnerForegroundReset;

impl Drop for PreflightOwnerForegroundReset {
    fn drop(&mut self) {
        sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
    }
}

fn preflight_action(
    index: u32,
    kind: ActionKind,
    scheduled_us: u64,
    scan_code: u16,
) -> KeyActionInput {
    KeyActionInput {
        source_action_index: index,
        kind,
        scheduled_us,
        scan_codes: vec![scan_code].into(),
        reason: format!("preflight-owner-busy-{index}").into(),
    }
}

fn make_active_a_preflight_harness() -> (
    ProductionDispatchTestHarness,
    PacketCapture,
    QpcTicks,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
) {
    let actions = [
        preflight_action(6690, ActionKind::Down, 0, 0x15),
        preflight_action(6691, ActionKind::Down, 30_000, 0x16),
        preflight_action(6692, ActionKind::Up, 80_000, 0x15),
        preflight_action(6693, ActionKind::Up, 2_000_000, 0x16),
    ];
    let (mut harness, packets, epoch) = timing_owner_harness(&actions, &[]);
    // The general harness assumes its initial stamp has already been checked.
    // This fixture explicitly proves and counts the first successful preflight.
    harness.runtime.verified_target = None;

    let preflight_calls = Arc::new(AtomicUsize::new(0));
    let physical_mask = Arc::new(AtomicUsize::new(0));
    let probe_calls = Arc::clone(&preflight_calls);
    let probe_mask = Arc::clone(&physical_mask);
    harness.set_preflight_physical_mask_probe_for_test(move || {
        probe_calls.fetch_add(1, Ordering::SeqCst);
        probe_mask.load(Ordering::Acquire) as u16
    });

    let lease_control_checks = Arc::new(AtomicUsize::new(0));
    let lease_checks = Arc::clone(&lease_control_checks);
    harness.runtime.final_gate_post_lease_race_hook = Some(Arc::new(
        move |focus, target, quit, skip, panic, pause, power| {
            assert!(focus.load(Ordering::Acquire));
            assert!(!quit.load(Ordering::Acquire));
            assert!(!skip.load(Ordering::Acquire));
            assert!(!panic.load(Ordering::Acquire));
            assert!(!pause.load(Ordering::Acquire));
            assert!(!power.down_blocked());
            let _ = target;
            lease_checks.fetch_add(1, Ordering::SeqCst);
        },
    ));

    let first_eval = harness.qpc_now_for_test().expect("A scheduler QPC");
    let (a, _) = stage_from_worker_outer_evaluation(&mut harness, first_eval);
    assert_eq!(a.packet_masks.down_mask, 1, "first authored Down is A");
    assert_eq!(preflight_calls.load(Ordering::Acquire), 1);
    assert_eq!(
        harness.runtime.verified_target_for_test(),
        a.target_stamp,
        "successful first preflight records its exact target stamp"
    );
    harness.set_mock_sender_start_qpc_for_test(first_eval);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(a, false),
        StagedDispatchOutcome::Dispatched
    );
    assert_eq!(packets.lock().expect("A packet capture").len(), 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.resources.backend.active_mask, 1);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);

    // A repeat all-keys scan would report the session's own held A as an
    // external key and reject the otherwise-safe B onset.
    physical_mask.store(1, Ordering::Release);
    lease_control_checks.store(0, Ordering::Release);
    (
        harness,
        packets,
        epoch,
        preflight_calls,
        physical_mask,
        lease_control_checks,
    )
}

fn install_final_permit_contention(
    harness: &mut ProductionDispatchTestHarness,
    lock_kind: FinalPermitLock,
) -> HeldFinalPermit {
    let target = Arc::clone(&harness.target);
    let (at_final_permit_tx, at_final_permit_rx) = std::sync::mpsc::channel();
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let locked_rx = Arc::new(Mutex::new(locked_rx));
    let locked_rx_for_hook = Arc::clone(&locked_rx);
    harness.set_final_gate_before_owner_permit_hook(move || {
        at_final_permit_tx
            .send(())
            .expect("supervisor is waiting at final permit");
        locked_rx_for_hook
            .lock()
            .expect("final permit barrier mutex")
            .recv_timeout(Duration::from_secs(5))
            .expect("supervisor acquired final permit mutex");
    });

    let supervisor = std::thread::spawn(move || {
        if at_final_permit_rx
            .recv_timeout(Duration::from_secs(5))
            .is_err()
        {
            return false;
        }
        match lock_kind {
            FinalPermitLock::Writer => {
                let _guard = target.lock_writer_for_test();
                if locked_tx.send(()).is_err() {
                    return false;
                }
                release_rx.recv_timeout(Duration::from_secs(5)).is_ok()
            }
            FinalPermitLock::OwnerAuthority => {
                let _guard = target.lock_owner_authority_for_test();
                if locked_tx.send(()).is_err() {
                    return false;
                }
                release_rx.recv_timeout(Duration::from_secs(5)).is_ok()
            }
        }
    });
    HeldFinalPermit {
        release: release_tx,
        supervisor,
    }
}

fn attempt_due_b_with_policy(
    harness: &mut ProductionDispatchTestHarness,
    epoch: QpcTicks,
    retry_pending: &mut bool,
) -> DueBAttempt {
    let retry_allowed = std::mem::take(retry_pending);
    if !retry_allowed {
        let b_deadline = epoch
            .checked_add_duration(
                harness
                    .resources
                    .clock
                    .duration_from_us(30_000)
                    .expect("B authored deadline"),
            )
            .expect("B authored QPC");
        wait_until_qpc(harness, b_deadline);
    }
    let eval_qpc = harness.qpc_now_for_test().expect("B outer-loop QPC");
    let (candidate, evaluations) = stage_from_worker_outer_evaluation(harness, eval_qpc);
    assert_eq!(evaluations, 1, "the due B onset uses one scheduler evaluation");
    assert_eq!(candidate.packet_masks.up_mask, 0);
    assert_eq!(candidate.packet_masks.down_mask, 2);
    assert!(candidate.timing_window.packet_not_before_qpc <= eval_qpc);
    let source_action_index = candidate.source_action_index;
    let target_stamp = candidate.target_stamp;
    let retry_deadline_qpc = candidate.timing_window.packet_not_before_qpc;
    harness.set_mock_sender_start_qpc_for_test(eval_qpc);
    let outcome = harness.dispatch_staged_candidate_with_reason_for_test(candidate, false);
    DueBAttempt {
        source_action_index,
        target_stamp,
        retry_deadline_qpc,
        outcome,
        retry_allowed,
    }
}

fn wait_until_qpc(harness: &mut ProductionDispatchTestHarness, deadline_qpc: QpcTicks) {
    loop {
        let now = harness.qpc_now_for_test().expect("wait QPC");
        if now >= deadline_qpc {
            return;
        }
        match super::super::wait_after_unstable_target(
            now,
            deadline_qpc,
            harness.resources.clock,
            &harness.resources.waiter,
            &harness.interrupt,
            &mut harness.local_metrics,
            &mut harness.runtime.terminal_error,
        ) {
            super::super::WaitBoundary::Due { .. } => {}
            super::super::WaitBoundary::Interrupted(_, _, _) => {
                let _ = harness.interrupt.try_take();
            }
            super::super::WaitBoundary::Exit => panic!("authored deadline wait exited"),
        }
    }
}

fn assert_a_obligation_unchanged(
    harness: &ProductionDispatchTestHarness,
    packets: &PacketCapture,
    preflight_calls: &AtomicUsize,
    expected_preflight_calls: usize,
    origin: crate::engine::worker::TargetStamp,
) {
    assert_eq!(packets.lock().expect("packet capture").len(), 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.resources.backend.active_mask, 1);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(
        harness.runtime.verified_target_for_test(),
        Some(origin),
        "only the previously validated, still-current preflight stamp survives"
    );
    assert_eq!(
        preflight_calls.load(Ordering::Acquire),
        expected_preflight_calls,
        "a transient owner Busy must not repeat the all-keys startup scan"
    );
}

fn assert_fresh_b_admission_ran(
    harness: &ProductionDispatchTestHarness,
    lease_control_checks: &AtomicUsize,
) {
    let (arc_clones, authority_attempts, writer_permits, generation_checks) =
        harness.target_proof_counts_for_test();
    assert!(arc_clones > 0, "retry acquired a fresh owner authority proof");
    assert!(authority_attempts > 0, "retry freshly read owner authority");
    assert!(writer_permits > 0, "retry entered the final writer permit");
    assert!(generation_checks > 0, "retry revalidated target generation");
    assert!(
        sky_dispatch_win32::focus::foreground_query_count() > 0,
        "retry performed a fresh foreground query"
    );
    assert!(
        sky_dispatch_win32::focus::owner_query_count() > 0,
        "retry queried foreground owner PID"
    );
    assert!(
        sky_dispatch_win32::focus::process_identity_check_count_for_test() > 0,
        "retry checked retained process identity continuity"
    );
    assert_eq!(
        sky_dispatch_win32::focus::get_process_times_call_count_for_test(),
        0,
        "synthetic authority does not call the native GetProcessTimes API"
    );
    assert_eq!(
        lease_control_checks.load(Ordering::Acquire),
        1,
        "retry passed the post-lease fresh control/power seam exactly once"
    );
}

fn finish_b_retry_and_release_a(
    harness: &mut ProductionDispatchTestHarness,
    packets: &PacketCapture,
    epoch: QpcTicks,
    preflight_calls: &AtomicUsize,
    lease_control_checks: &AtomicUsize,
    retry_pending: &mut bool,
    a_origin: &sky_dispatch_win32::input::ReleaseOwnerProof,
) {
    harness.runtime.final_gate_before_owner_permit_hook = None;
    harness.reset_target_proof_counts_for_test();
    sky_dispatch_win32::focus::reset_foreground_query_count();
    sky_dispatch_win32::focus::reset_process_identity_counts_for_test();
    lease_control_checks.store(0, Ordering::Release);

    let b = attempt_due_b_with_policy(harness, epoch, retry_pending);
    assert!(b.retry_allowed, "the accepted immediate retry token was consumed");
    assert_eq!(b.outcome, DispatchAttemptOutcome::Dispatched);
    assert_fresh_b_admission_ran(harness, lease_control_checks);
    assert_eq!(preflight_calls.load(Ordering::Acquire), 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 2);
    assert_eq!(harness.resources.backend.active_mask, 3);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 3);
    let retained_a_origin = harness
        .resources
        .backend
        .release_owner_proof_for_mask(1)
        .expect("A release proof lookup after B")
        .expect("A release proof retained after B");
    assert!(retained_a_origin.same_owner(a_origin));
    assert!(!*retry_pending);
    {
        let sent = packets.lock().expect("A/B packet capture");
        assert_eq!(sent.len(), 2, "A and B each have one sender attempt");
        assert_eq!(sent[0].0.down_mask, 1);
        assert_eq!(sent[1].0.down_mask, 2);
        assert!(sent.iter().all(|(_, evidence)| evidence.attempts == 1));
        assert!(sent.iter().all(|(_, evidence)| evidence.first_inserted == 1));
    }

    let up_a_target = epoch
        .checked_add_duration(
            harness
                .resources
                .clock
                .duration_from_us(80_000)
                .expect("A authored Up offset"),
        )
        .expect("A authored Up QPC");
    loop {
        let now = harness.qpc_now_for_test().expect("wait for authored Up(A)");
        if now >= up_a_target {
            break;
        }
        match super::super::wait_after_unstable_target(
            now,
            up_a_target,
            harness.resources.clock,
            &harness.resources.waiter,
            &harness.interrupt,
            &mut harness.local_metrics,
            &mut harness.runtime.terminal_error,
        ) {
            super::super::WaitBoundary::Due { .. } => {}
            super::super::WaitBoundary::Interrupted(_, _, _) => {
                let _ = harness.interrupt.try_take();
            }
            super::super::WaitBoundary::Exit => panic!("authored Up(A) wait exited"),
        }
    }
    let up_eval = harness.qpc_now_for_test().expect("Up(A) scheduler QPC");
    let (up_a, evaluations) = stage_from_worker_outer_evaluation(harness, up_eval);
    assert_eq!(evaluations, 1);
    assert_eq!(up_a.packet_masks.up_mask, 1);
    assert_eq!(up_a.packet_masks.down_mask, 0);
    harness.set_mock_sender_start_qpc_for_test(up_eval);
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(up_a, false),
        StagedDispatchOutcome::Dispatched,
        "A's Up remains independently governed by its authored time and origin"
    );
    {
        let sent = packets.lock().expect("A/B/Up(A) packet capture");
        assert_eq!(sent.len(), 3);
        assert_eq!(sent[2].0.up_mask, 1);
        assert_eq!(sent[2].0.down_mask, 0);
        assert!(sent.iter().all(|(_, evidence)| evidence.attempts == 1));
        assert!(sent.iter().all(|(_, evidence)| evidence.first_inserted == 1));
    }
    assert_eq!(harness.resources.backend.active_mask, 2);
    assert_eq!(harness.resources.backend.possibly_active_mask, 0);
    assert_eq!(harness.resources.backend.in_flight_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 2);
    assert!(harness.runtime.terminal_error.is_none());
}

fn exercise_busy_preserving_preflight(lock_kind: FinalPermitLock, early_read: bool) {
    let (
        mut harness,
        packets,
        epoch,
        preflight_calls,
        _physical_mask,
        lease_control_checks,
    ) = make_active_a_preflight_harness();
    let origin = harness
        .runtime
        .verified_target_for_test()
        .expect("validated A-session preflight stamp");
    let a_origin = harness
        .resources
        .backend
        .release_owner_proof_for_mask(1)
        .expect("A release proof lookup before B")
        .expect("A release proof before B");
    assert_eq!(a_origin.target_hwnd(), origin.hwnd);
    assert_eq!(a_origin.target_generation(), origin.generation);
    let mut retry_pending = false;

    let held_final_permit = if early_read {
        None
    } else {
        Some(install_final_permit_contention(&mut harness, lock_kind))
    };
    let early_lock_target = Arc::clone(&harness.target);
    let early_authority_guard = if early_read {
        Some(early_lock_target.lock_owner_authority_for_test())
    } else {
        None
    };

    let attempt = attempt_due_b_with_policy(&mut harness, epoch, &mut retry_pending);
    assert!(!attempt.retry_allowed);
    assert_eq!(attempt.source_action_index, 6691);
    assert_eq!(attempt.outcome, DispatchAttemptOutcome::PermitBusy);
    assert_a_obligation_unchanged(&harness, &packets, &preflight_calls, 1, origin);
    assert!(harness
        .resources
        .backend
        .release_owner_proof_for_mask(1)
        .expect("A release proof lookup after Busy")
        .expect("A release proof after Busy")
        .same_owner(&a_origin));
    assert_eq!(lease_control_checks.load(Ordering::Acquire), 1);

    let replan = apply_outer_replan_policy(
        &mut harness,
        attempt.outcome,
        attempt.target_stamp,
        attempt.retry_deadline_qpc,
        attempt.retry_allowed,
        &mut retry_pending,
    );
    assert_eq!(
        replan,
        super::super::DispatchReplanAction::ContinueImmediately,
        "the first stable-target Busy follows the accepted immediate retry policy"
    );
    assert!(retry_pending);

    drop(early_authority_guard);
    if let Some(held) = held_final_permit {
        held.release
            .send(())
            .expect("release final owner/writer permit lock");
        assert!(held.supervisor.join().expect("permit supervisor joins"));
    }
    finish_b_retry_and_release_a(
        &mut harness,
        &packets,
        epoch,
        &preflight_calls,
        &lease_control_checks,
        &mut retry_pending,
        &a_origin,
    );
}

#[test]
fn early_owner_proof_read_busy_preserves_active_a_preflight_across_b_restage() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = PreflightOwnerForegroundReset;
    exercise_busy_preserving_preflight(FinalPermitLock::OwnerAuthority, true);
}

#[test]
fn final_owner_authority_mutex_busy_preserves_active_a_preflight_across_b_restage() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = PreflightOwnerForegroundReset;
    exercise_busy_preserving_preflight(FinalPermitLock::OwnerAuthority, false);
}

#[test]
fn final_writer_mutex_busy_preserves_active_a_preflight_across_b_restage() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = PreflightOwnerForegroundReset;
    exercise_busy_preserving_preflight(FinalPermitLock::Writer, false);
}

#[test]
fn same_hwnd_generation_aba_repreflights_new_target_stamp() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = PreflightOwnerForegroundReset;
    let (
        mut harness,
        packets,
        epoch,
        preflight_calls,
        physical_mask,
        _lease_control_checks,
    ) = make_active_a_preflight_harness();
    let original = harness
        .runtime
        .verified_target_for_test()
        .expect("old generation preflight");

    harness.republish_focus_owner_for_test(original.hwnd, 101);
    let rebound = super::super::load_target_stamp(&harness.target).expect("new target stamp");
    assert_eq!(rebound.hwnd, original.hwnd);
    assert_ne!(rebound.generation, original.generation);
    physical_mask.store(0, Ordering::Release);

    let b_deadline = epoch
        .checked_add_duration(
            harness
                .resources
                .clock
                .duration_from_us(30_000)
                .expect("B authored deadline"),
        )
        .expect("B authored QPC");
    wait_until_qpc(&mut harness, b_deadline);
    let b_eval = harness.qpc_now_for_test().expect("rebound B QPC");
    let (b, evaluations) = stage_from_worker_outer_evaluation(&mut harness, b_eval);
    assert_eq!(evaluations, 1);
    assert_eq!(b.source_action_index, 6691);
    assert_eq!(b.target_stamp, Some(rebound));
    assert_eq!(preflight_calls.load(Ordering::Acquire), 2);
    assert_eq!(
        harness.runtime.verified_target_for_test(),
        Some(rebound),
        "the old preflight stamp cannot authorize the ABA generation"
    );
    assert!(packets.lock().expect("packet capture").len() == 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.resources.backend.active_mask, 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    let origin = harness
        .resources
        .backend
        .release_owner_proof_for_mask(1)
        .expect("A release proof lookup")
        .expect("A retains its frozen proof");
    assert_eq!(origin.target_hwnd(), original.hwnd);
    assert_eq!(origin.target_generation(), original.generation);
}

#[test]
fn focus_loss_invalidates_preflight_and_recovery_rechecks_after_cleanup() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = PreflightOwnerForegroundReset;
    let (
        mut harness,
        packets,
        epoch,
        preflight_calls,
        physical_mask,
        _lease_control_checks,
    ) = make_active_a_preflight_harness();
    let origin = harness
        .runtime
        .verified_target_for_test()
        .expect("validated preflight before focus loss");
    let b_deadline = epoch
        .checked_add_duration(
            harness
                .resources
                .clock
                .duration_from_us(30_000)
                .expect("B authored deadline"),
        )
        .expect("B authored QPC");
    wait_until_qpc(&mut harness, b_deadline);
    let b_eval = harness.qpc_now_for_test().expect("B focus-loss QPC");
    let (b, _) = stage_from_worker_outer_evaluation(&mut harness, b_eval);
    assert_eq!(b.source_action_index, 6691);
    assert_eq!(preflight_calls.load(Ordering::Acquire), 1);

    harness.focus_active.store(false, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(2));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));
    assert_eq!(
        harness.dispatch_staged_candidate_for_test(b, false),
        StagedDispatchOutcome::Replan
    );
    assert_eq!(packets.lock().expect("no B during focus loss").len(), 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.resources.backend.active_mask, 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(
        harness.runtime.verified_target_for_test(),
        None,
        "focus loss invalidates the startup preflight stamp"
    );
    assert!(harness
        .resources
        .playback
        .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus));

    let (cleanup, _) = harness.configure_mock_cleanup_emitter_with_completion_qpc_for_test();
    harness.focus_active.store(true, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(origin.hwnd));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(101)));
    harness
        .apply_scheduler_resumable_lifecycle_transition_for_test()
        .expect("verified focus-resume cleanup");
    assert_eq!(cleanup.lock().expect("focus cleanup capture").len(), 1);
    assert_eq!(harness.resources.backend.active_mask, 0);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);

    // The recovered physical state is all-up only after cleanup. Recovery's
    // normal preflight must therefore run again against the same current stamp.
    physical_mask.store(0, Ordering::Release);
    let current = super::super::load_target_stamp(&harness.target).expect("focus-restored target");
    super::super::ensure_preflight_for_target(
        &harness.resources.backend,
        current,
        &mut harness.runtime.verified_target,
    )
    .expect("focus restoration preflight after cleanup");
    assert_eq!(preflight_calls.load(Ordering::Acquire), 2);
    assert_eq!(harness.runtime.verified_target_for_test(), Some(current));
    assert!(harness.runtime.terminal_error.is_none());
}

#[test]
fn lease_expiry_during_busy_replan_preempts_retry_without_rescanning_keys() {
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = PreflightOwnerForegroundReset;
    let (
        mut harness,
        packets,
        epoch,
        preflight_calls,
        _physical_mask,
        lease_control_checks,
    ) = make_active_a_preflight_harness();
    let origin = harness
        .runtime
        .verified_target_for_test()
        .expect("validated preflight before lease expiry");
    let lock_target = Arc::clone(&harness.target);
    let owner_guard = lock_target.lock_owner_authority_for_test();
    let mut retry_pending = false;
    let attempt = attempt_due_b_with_policy(&mut harness, epoch, &mut retry_pending);
    assert!(!attempt.retry_allowed);
    assert_eq!(attempt.outcome, DispatchAttemptOutcome::PermitBusy);
    assert_a_obligation_unchanged(&harness, &packets, &preflight_calls, 1, origin);
    drop(owner_guard);

    harness.supervisor_expired.latch_expired();
    let action = apply_outer_replan_policy(
        &mut harness,
        attempt.outcome,
        attempt.target_stamp,
        attempt.retry_deadline_qpc,
        attempt.retry_allowed,
        &mut retry_pending,
    );
    assert_eq!(action, super::super::DispatchReplanAction::ContinueImmediately);
    assert!(!retry_pending, "expired lease clears the contention retry token");
    assert_eq!(packets.lock().expect("no B after lease expiry").len(), 1);
    assert_eq!(preflight_calls.load(Ordering::Acquire), 1);
    assert_eq!(harness.resources.scheduler.chord_cursor(), 1);
    assert_eq!(harness.resources.backend.active_mask, 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 1);
    assert_eq!(lease_control_checks.load(Ordering::Acquire), 1);
}
