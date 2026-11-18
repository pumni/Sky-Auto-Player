use super::replan_timing::timing_action;
use super::support::{
    ActionKind, ProductionDispatchTestHarness, StagedDispatchOutcome, TimelineTicks,
};
use std::sync::atomic::Ordering;
#[test]
fn due_candidate_rechecks_stop_skip_focus_and_owner_before_sender() {
    use sky_dispatch_win32::wait::{WaitOutcome, WaitResult};

    struct ForegroundReset;
    impl Drop for ForegroundReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
            sky_dispatch_win32::focus::set_foreground_owner_for_test(None);
        }
    }
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundReset;

    for change in ["stop", "skip", "focus", "owner"] {
        let actions = [
            timing_action(6690, ActionKind::Down, 50_000),
            timing_action(6691, ActionKind::Up, 500_000),
        ];
        let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
        let clock = harness.resources.clock;
        let epoch = clock
            .now()
            .expect("final-gate wake epoch sample")
            .checked_add_duration(
                clock
                    .duration_from_us(1_000_000)
                    .expect("future final-gate epoch offset"),
            )
            .expect("future final-gate epoch");
        harness
            .reanchor_playback_clock_for_test(epoch)
            .expect("future final-gate playback epoch");
        if matches!(change, "focus" | "owner") {
            harness.configure_focus_owner_for_test(1, 101);
        }
        let packets = harness.configure_mock_packet_sender_for_test(&[0]);
        harness.attach_executor_for_test().expect("attach executor");
        let candidate = harness
            .stage_candidate_for_test(
                0,
                harness.current_executor_chord_for_test(),
                TimelineTicks::ZERO,
            )
            .expect("stage future final-gate candidate")
            .expect("future Down candidate");
        assert_eq!(candidate.source_action_index, 6690);
        assert_eq!(
            harness.pre_wait_candidate_admission_for_test(&candidate),
            StagedDispatchOutcome::Dispatched
        );
        let target_qpc = candidate.timing_window.packet_not_before_qpc;
        let wake_qpc = target_qpc
            .checked_add_duration(clock.duration_from_us(10_000).expect("late wake offset"))
            .expect("late wake QPC");
        let _waiter_override = crate::engine::worker::wait::install_waiter_result_for_test(
            target_qpc,
            WaitResult {
                outcome: WaitOutcome::Deadline,
                wake_qpc: Some(wake_qpc),
                spin_ticks: sky_dispatch_core::time::DurationTicks::ZERO,
                timer_return_qpc: Some(wake_qpc),
                timer_arm_target_qpc: Some(target_qpc),
                timer_arm_qpc: None,
                timer_arm_generation: None,
                timer_return_generation: None,
            },
        );
        let boundary = super::super::wait_for_next_boundary(super::super::WaitBoundaryInput {
            deadline: super::super::WaitDeadline {
                physical_target_qpc: Some(target_qpc),
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
        assert!(matches!(boundary, super::super::WaitBoundary::Due { .. }));
        drop(_waiter_override);

        match change {
            "stop" => harness.quit_requested.store(true, Ordering::Release),
            "skip" => harness.skip_requested.store(true, Ordering::Release),
            "focus" => {
                sky_dispatch_win32::focus::set_foreground_window_for_test(Some(2));
                sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(202)));
            }
            "owner" => harness.invalidate_focus_owner_for_test(1, 202),
            _ => unreachable!(),
        }
        harness.set_mock_sender_start_qpc_for_test(wake_qpc);
        assert_eq!(
            harness.dispatch_staged_candidate_for_test(candidate, false),
            StagedDispatchOutcome::Replan,
            "{change} is rechecked after Due and before SendInput"
        );
        assert!(
            packets
                .lock()
                .expect("no rejected sender attempt")
                .is_empty()
        );
        assert_eq!(harness.resources.scheduler.chord_cursor(), 0);
        assert_eq!(harness.resources.backend.release_obligation_mask(), 0);
        assert_eq!(harness.local_metrics.precision_packet_attempts, 0);
        match change {
            "stop" | "skip" => assert!(harness.process_command_control_for_test()),
            "focus" => assert!(
                harness
                    .resources
                    .playback
                    .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus)
            ),
            "owner" => {
                assert!(
                    !harness
                        .resources
                        .playback
                        .has_pause_reason(sky_dispatch_core::clock::PauseReason::Focus)
                );
                assert_eq!(harness.local_metrics.final_gate_target_changes, 1);
            }
            _ => unreachable!(),
        }
    }
}
