use super::replan_timing::{
    apply_outer_replan_policy, stage_from_worker_outer_evaluation, timing_action,
    timing_harness_without_focus,
};
use super::support::{
    ActionKind, ProductionDispatchTestHarness, StagedDispatchOutcome, TimelineTicks,
};
use std::sync::atomic::Ordering;
#[test]
fn interrupted_focus_wake_discards_staged_candidate_for_outer_loop_handling() {
    use sky_dispatch_win32::wait::WaitResult;

    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;
    let actions = [
        timing_action(6692, ActionKind::Down, 50_000),
        timing_action(6693, ActionKind::Up, 500_000),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let clock = harness.resources.clock;
    let epoch = clock
        .now()
        .expect("interrupted wait epoch sample")
        .checked_add_duration(
            clock
                .duration_from_us(1_000_000)
                .expect("future interrupted-wait epoch offset"),
        )
        .expect("future interrupted-wait epoch");
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("future interrupted-wait playback epoch");
    harness.configure_focus_owner_for_test(1, 101);
    let packets = harness.configure_mock_packet_sender_for_test(&[0]);
    harness.attach_executor_for_test().expect("attach executor");
    let candidate = harness
        .stage_candidate_for_test(
            0,
            harness.current_executor_chord_for_test(),
            TimelineTicks::ZERO,
        )
        .expect("stage interrupted candidate")
        .expect("future Down candidate");
    assert_eq!(
        harness.pre_wait_candidate_admission_for_test(&candidate),
        StagedDispatchOutcome::Dispatched
    );
    harness.focus_active.store(false, Ordering::Release);
    sky_dispatch_win32::focus::set_foreground_window_for_test(Some(2));
    sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));
    let _waiter_override = crate::engine::worker::wait::install_waiter_result_for_test(
        candidate.timing_window.packet_not_before_qpc,
        WaitResult::interrupted(),
    );
    let interrupted = super::super::wait_for_next_boundary(super::super::WaitBoundaryInput {
        deadline: super::super::WaitDeadline {
            physical_target_qpc: Some(candidate.timing_window.packet_not_before_qpc),
            spin_threshold_ticks: harness.timing.effective_spin_threshold_ticks,
            qpc_clock: clock,
        },
        signals: super::super::WaitSignals {
            waiter: &harness.resources.waiter,
            interrupt: &harness.interrupt,
        },
        mutable: super::super::WaitMutable {
            local_metrics: &mut harness.local_metrics,
            terminal_error: &mut harness.runtime.terminal_error,
        },
    });
    assert!(matches!(
        interrupted,
        super::super::WaitBoundary::Interrupted(..)
    ));
    drop(_waiter_override);
    drop(candidate);

    assert!(
        packets
            .lock()
            .expect("interrupted candidate not sent")
            .is_empty()
    );
    assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
    assert_eq!(harness.resources.backend.release_obligation_mask(), 0);
    let focus_now = clock.now().expect("focus wake QPC");
    super::super::enter_focus_pause(
        &mut harness.resources.playback,
        &mut harness.runtime,
        focus_now,
        &harness.progress_clock,
    )
    .expect("outer loop handles interrupted focus wake");
    assert!(
        harness
            .resources
            .playback
            .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus)
    );
    assert_eq!(harness.runtime.verified_target_for_test(), None);
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
                super::super::DispatchAttemptOutcome::PermitBusy,
                candidate.target_stamp,
                candidate.timing_window.packet_not_before_qpc,
                false,
                &mut retry_pending,
            ),
            super::super::DispatchReplanAction::ContinueImmediately,
            "first transient miss gets one immediate full re-evaluation"
        );
        assert!(retry_pending);
        assert!(matches!(
            apply_outer_replan_policy(
                &mut harness,
                super::super::DispatchAttemptOutcome::PermitBusy,
                candidate.target_stamp,
                candidate.timing_window.packet_not_before_qpc,
                std::mem::take(&mut retry_pending),
                &mut retry_pending,
            ),
            super::super::DispatchReplanAction::WaitCompleted
        ));
        let wait_trace =
            super::super::last_dispatch_replan_wait_for_test().expect("bounded wait trace");
        assert!(!wait_trace.interrupted);
        assert!(
            !retry_pending,
            "bounded backoff does not leave a spin token"
        );

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
            super::super::DispatchAttemptOutcome::PermitBusy,
            candidate.target_stamp,
            candidate.timing_window.packet_not_before_qpc,
            std::mem::take(&mut retry_pending),
            &mut retry_pending,
        );
        assert_eq!(
            action,
            super::super::DispatchReplanAction::ContinueImmediately,
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
    let action = super::super::wait_after_dispatch_replan(
        super::super::DispatchAttemptOutcome::PermitBusy,
        true,
        &mut retry_pending,
        super::super::DispatchReplanWaitContext {
            target: &harness.target,
            expected_target: candidate.target_stamp,
            retry_deadline_qpc: candidate.timing_window.packet_not_before_qpc,
            require_focus: harness.config.focus.require_focus,
            focus_active: &harness.focus_active,
            control_signals: super::super::FinalControlSignals {
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
        super::super::DispatchReplanAction::WaitCompleted
    ));
    assert!(
        super::super::last_dispatch_replan_wait_for_test()
            .expect("interrupted wait trace")
            .interrupted
    );
    assert_eq!(harness.local_metrics.wait_interrupted_count, 1);
    assert!(packets.lock().expect("packet capture").is_empty());
}
