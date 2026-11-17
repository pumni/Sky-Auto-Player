use super::replan_timing::{timing_action, timing_harness_without_focus};
use super::support::ActionKind;
#[test]
fn real_waiter_smoke_wakes_for_a_future_qpc_deadline() {
    let actions = [
        timing_action(6674, ActionKind::Down, 0),
        timing_action(6675, ActionKind::Up, 1_000_000),
    ];
    let (mut harness, _, _) = timing_harness_without_focus(&actions, &[]);
    let start_qpc = harness.qpc_now_for_test().expect("real waiter start QPC");
    let deadline_qpc = start_qpc
        .checked_add_duration(
            harness
                .resources
                .clock
                .duration_from_us(5_000)
                .expect("real waiter deadline offset"),
        )
        .expect("real waiter deadline");
    let wake = super::super::wait_for_next_boundary(super::super::WaitBoundaryInput {
        deadline: super::super::WaitDeadline {
            physical_target_qpc: Some(deadline_qpc),
            spin_threshold_ticks: harness.timing.effective_spin_threshold_ticks,
            qpc_clock: harness.resources.clock,
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
    let wake_qpc = harness.qpc_now_for_test().expect("real waiter wake QPC");
    assert!(matches!(wake, super::super::WaitBoundary::Due { .. }));
    assert!(
        wake_qpc >= deadline_qpc,
        "waiter returned after its real QPC deadline"
    );
}
