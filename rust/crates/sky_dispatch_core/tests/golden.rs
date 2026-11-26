//! Golden and multi-QPC-frequency reference trace tests for the
//! completion-anchored pulse engine (`sky_dispatch_core`).

use sky_dispatch_core::executor::{
    DispatchDecision, DispatchExecutor, PreparedDispatchSchedule, us_to_duration_ticks_ceil,
    us_to_timeline_ticks_ceil,
};
use sky_dispatch_core::model::{MAX_KEYS, OnsetChordInput};
use sky_dispatch_core::plan::build_onset_plan;
use sky_dispatch_core::time::{DurationTicks, QpcTicks};
use std::num::NonZeroU64;

const ALLOWED_15: [u16; MAX_KEYS] = [
    0x15, 0x16, 0x17, 0x18, 0x19, 0x23, 0x24, 0x25, 0x26, 0x27, 0x31, 0x32, 0x33, 0x34, 0x35,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TraceStep {
    send_qpc: QpcTicks,
    completion_qpc: QpcTicks,
    up_mask: u16,
    down_mask: u16,
}

fn run_trace(
    chords: &[OnsetChordInput],
    hold_us: u64,
    qpc_hz: u64,
    epoch_qpc: QpcTicks,
    latencies_ticks: &[u64],
) -> (Vec<TraceStep>, DispatchExecutor) {
    let freq = NonZeroU64::new(qpc_hz).expect("nonzero QPC frequency");
    let plan = build_onset_plan(chords, &ALLOWED_15, hold_us).expect("valid onset plan");
    let prepared = PreparedDispatchSchedule::prepare(&plan, freq, 0).expect("prepared plan");
    let mut exec = DispatchExecutor::from_prepared_schedule(prepared);

    let mut now = epoch_qpc;
    let mut steps = Vec::new();
    let mut latency_idx = 0usize;

    loop {
        match exec.evaluate_at(now, epoch_qpc).expect("evaluate_at") {
            DispatchDecision::Finished => break,
            DispatchDecision::Wait { deadline_qpc } => {
                assert!(deadline_qpc > now, "wait deadline must strictly advance");
                now = deadline_qpc;
            }
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
                ..
            } => {
                let down_mask = down_chord.map_or(0, |c| c.key_mask);
                assert_eq!(up_mask & down_mask, 0, "Up and Down masks must be disjoint");
                assert_ne!(up_mask | down_mask, 0, "Send must carry Up or Down");
                assert!(
                    now >= timing_window.packet_not_before_qpc,
                    "never send before packet_not_before_qpc"
                );
                let latency = latencies_ticks.get(latency_idx).copied().unwrap_or(0);
                latency_idx += 1;
                let completion = now
                    .checked_add_duration(DurationTicks::from_raw(latency))
                    .expect("completion QPC");
                exec.on_send_completed_at(up_mask, down_mask, completion)
                    .expect("commit");
                steps.push(TraceStep {
                    send_qpc: now,
                    completion_qpc: completion,
                    up_mask,
                    down_mask,
                });
                now = completion;
            }
        }
    }
    exec.check_invariants().expect("invariants");
    (steps, exec)
}

#[test]
fn multi_qpc_frequency_nonintegral_conversions_enforce_no_early_down_and_exact_h_floor() {
    let h_us = 16_667u64;
    let chords = vec![
        OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 1_000,
            scan_codes: vec![ALLOWED_15[0]].into(),
        },
        OnsetChordInput {
            source_action_index: 1,
            scheduled_us: 1_000 + h_us,
            scan_codes: vec![ALLOWED_15[0]].into(),
        },
        OnsetChordInput {
            source_action_index: 2,
            scheduled_us: 1_000 + h_us + h_us,
            scan_codes: vec![ALLOWED_15[1]].into(),
        },
    ];

    for qpc_hz in [1_000_000u64, 3_579_545, 10_000_000, 24_000_000] {
        let freq = NonZeroU64::new(qpc_hz).unwrap();
        let h_ticks = us_to_duration_ticks_ceil(h_us, freq).unwrap().as_u64();
        let d0_ticks = us_to_timeline_ticks_ceil(1_000, freq).unwrap().as_u64();
        let d1_ticks = us_to_timeline_ticks_ceil(1_000 + h_us, freq)
            .unwrap()
            .as_u64();
        let d2_ticks = us_to_timeline_ticks_ceil(1_000 + 2 * h_us, freq)
            .unwrap()
            .as_u64();
        let epoch = QpcTicks::from_raw(100_000_000);

        // Test both zero-latency and late-completion runs.
        for latencies in [&[0u64, 0, 0, 0, 0][..], &[50u64, 20, 120, 10, 5][..]] {
            let (steps, exec) = run_trace(&chords, h_us, qpc_hz, epoch, latencies);

            // Verify every Down is >= epoch + onset_ticks and every Up is >= down_completion + h_ticks.
            let mut down_completion_by_slot: [Option<u64>; MAX_KEYS] = [None; MAX_KEYS];
            let mut down_idx = 0usize;
            let expected_downs = [(d0_ticks, 1u16), (d1_ticks, 1u16), (d2_ticks, 2u16)];
            for step in &steps {
                let mut u = step.up_mask;
                while u != 0 {
                    let s = u.trailing_zeros() as usize;
                    u &= u - 1;
                    let down_comp = down_completion_by_slot[s]
                        .take()
                        .expect("Up must follow an active Down");
                    assert!(
                        step.send_qpc.as_u64() >= down_comp + h_ticks,
                        "Up sent before Down completion + H at qpc_hz={qpc_hz}"
                    );
                }
                if step.down_mask != 0 {
                    let (expected_onset, expected_mask) = expected_downs[down_idx];
                    down_idx += 1;
                    assert_eq!(step.down_mask, expected_mask);
                    assert!(
                        step.send_qpc.as_u64() >= epoch.as_u64() + expected_onset,
                        "Down sent early at qpc_hz={qpc_hz}"
                    );
                    let mut d = step.down_mask;
                    while d != 0 {
                        let s = d.trailing_zeros() as usize;
                        d &= d - 1;
                        assert!(down_completion_by_slot[s].is_none());
                        down_completion_by_slot[s] = Some(step.completion_qpc.as_u64());
                    }
                }
            }
            assert_eq!(down_idx, expected_downs.len());
            assert!(down_completion_by_slot.iter().all(Option::is_none));
            assert_eq!(exec.accounting().total, 3);
            assert_eq!(exec.accounting().released, 3);
        }
    }
}

#[test]
fn golden_exact_h_equality_orders_up_before_re_down_with_no_g() {
    let h = 16_667u64;
    let chords = vec![
        OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 0,
            scan_codes: vec![ALLOWED_15[0]].into(),
        },
        OnsetChordInput {
            source_action_index: 1,
            scheduled_us: h,
            scan_codes: vec![ALLOWED_15[0]].into(),
        },
    ];
    let (steps, _) = run_trace(&chords, h, 1_000_000, QpcTicks::ZERO, &[0, 0, 0, 0]);
    assert_eq!(steps.len(), 4);
    assert_eq!(
        (
            steps[0].send_qpc.as_u64(),
            steps[0].up_mask,
            steps[0].down_mask
        ),
        (0, 0, 1)
    );
    assert_eq!(
        (
            steps[1].send_qpc.as_u64(),
            steps[1].up_mask,
            steps[1].down_mask
        ),
        (h, 1, 0)
    );
    assert_eq!(
        (
            steps[2].send_qpc.as_u64(),
            steps[2].up_mask,
            steps[2].down_mask
        ),
        (h, 0, 1)
    );
    assert_eq!(
        (
            steps[3].send_qpc.as_u64(),
            steps[3].up_mask,
            steps[3].down_mask
        ),
        (2 * h, 1, 0)
    );
}

#[test]
fn golden_disjoint_mixed_batch_combines_up_and_down_at_same_deadline() {
    let h = 16_667u64;
    let chords = vec![
        OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 0,
            scan_codes: vec![ALLOWED_15[0]].into(),
        },
        OnsetChordInput {
            source_action_index: 1,
            scheduled_us: h,
            scan_codes: vec![ALLOWED_15[1]].into(),
        },
    ];
    let (steps, _) = run_trace(&chords, h, 1_000_000, QpcTicks::ZERO, &[0, 0, 0]);
    assert_eq!(steps.len(), 3);
    assert_eq!(
        (
            steps[0].send_qpc.as_u64(),
            steps[0].up_mask,
            steps[0].down_mask
        ),
        (0, 0, 1)
    );
    assert_eq!(
        (
            steps[1].send_qpc.as_u64(),
            steps[1].up_mask,
            steps[1].down_mask
        ),
        (h, 1, 2)
    );
    assert_eq!(
        (
            steps[2].send_qpc.as_u64(),
            steps[2].up_mask,
            steps[2].down_mask
        ),
        (2 * h, 2, 0)
    );
}

#[test]
fn golden_empty_plan_and_large_timestamps() {
    let (empty_steps, empty_exec) =
        run_trace(&[], 16_667, 10_000_000, QpcTicks::from_raw(5_000), &[]);
    assert!(empty_steps.is_empty());
    assert_eq!(empty_exec.accounting().total, 0);

    // Large timestamp (10 hours = 36_000_000_000 us) at 10 MHz QPC.
    let ten_hours_us = 36_000_000_000u64;
    let large = vec![OnsetChordInput {
        source_action_index: 0,
        scheduled_us: ten_hours_us,
        scan_codes: vec![ALLOWED_15[0], ALLOWED_15[14]].into(),
    }];
    let (steps, exec) = run_trace(
        &large,
        16_667,
        10_000_000,
        QpcTicks::from_raw(1_000_000_000_000),
        &[100, 100],
    );
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0].down_mask, (1 << 0) | (1 << 14));
    assert_eq!(steps[1].up_mask, (1 << 0) | (1 << 14));
    assert_eq!(exec.accounting().released, 2);
}
