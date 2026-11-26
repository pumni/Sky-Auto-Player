use super::replan_timing::{timing_chord, timing_harness_without_focus};
#[test]
fn real_waiter_smoke_wakes_for_a_future_qpc_deadline() {
    let chords = [timing_chord(6674, 0)];
    let (mut harness, _, _) = timing_harness_without_focus(&chords, &[]);
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
