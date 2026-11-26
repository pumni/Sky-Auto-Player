//! Independent mathematical reference oracle property tests for `DispatchExecutor`.
//!
//! The oracle implements D1-D6 and D9 directly from first principles using a
//! simple independent state machine and NEVER invokes `DispatchExecutor::evaluate_at`.

use sky_dispatch_core::executor::{DispatchDecision, DispatchExecutor, PreparedDispatchSchedule};
use sky_dispatch_core::model::{MAX_KEYS, OnsetChordInput};
use sky_dispatch_core::plan::build_onset_plan;
use sky_dispatch_core::time::{DurationTicks, QpcTicks};
use std::num::NonZeroU64;

const ALLOWED_15: [u16; MAX_KEYS] = [
    0x15, 0x16, 0x17, 0x18, 0x19, 0x23, 0x24, 0x25, 0x26, 0x27, 0x31, 0x32, 0x33, 0x34, 0x35,
];

/// Simple deterministic xorshift64 PRNG so property runs are 100% reproducible.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn range(&mut self, upper: u64) -> u64 {
        self.next_u64() % upper
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OracleDecision {
    Send { up_mask: u16, down_mask: u16 },
    Wait { deadline_qpc: u64 },
    Finished,
}

/// Independent mathematical specification of the completion-anchored pulse engine.
struct MathematicalOracle {
    /// `(onset_ticks, key_mask)`
    chords: Vec<(u64, u16)>,
    cursor: usize,
    /// `Some(release_not_before_qpc)` if slot is active.
    active_deadlines: [Option<u64>; MAX_KEYS],
    hold_ticks: u64,
    end_offset_ticks: u64,
}

impl MathematicalOracle {
    fn decide(&self, now_qpc: u64, epoch_qpc: u64) -> OracleDecision {
        let mut up_ready = 0u16;
        let mut earliest_future_up: Option<u64> = None;

        for (slot, deadline) in self.active_deadlines.iter().enumerate() {
            if let Some(due) = *deadline {
                if now_qpc >= due {
                    up_ready |= 1u16 << slot;
                } else {
                    earliest_future_up = Some(earliest_future_up.map_or(due, |e| e.min(due)));
                }
            }
        }

        let mut down_ready_mask = 0u16;
        let mut next_chord_bound: Option<u64> = None;
        if let Some(&(onset_ticks, key_mask)) = self.chords.get(self.cursor) {
            let target = epoch_qpc + onset_ticks;
            let mut blocked = false;
            let mut bound = target;
            for (slot, deadline) in self.active_deadlines.iter().enumerate() {
                if (key_mask & (1u16 << slot)) != 0
                    && let Some(due) = *deadline
                {
                    blocked = true;
                    bound = bound.max(due);
                }
            }
            if !blocked && now_qpc >= target {
                down_ready_mask = key_mask;
            } else {
                next_chord_bound = Some(bound);
            }
        }

        if up_ready != 0 || down_ready_mask != 0 {
            return OracleDecision::Send {
                up_mask: up_ready,
                down_mask: down_ready_mask,
            };
        }

        let next_wake = match (earliest_future_up, next_chord_bound) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        if let Some(wake) = next_wake {
            return OracleDecision::Wait { deadline_qpc: wake };
        }

        let end_qpc = epoch_qpc + self.end_offset_ticks;
        if now_qpc < end_qpc {
            OracleDecision::Wait {
                deadline_qpc: end_qpc,
            }
        } else {
            OracleDecision::Finished
        }
    }

    fn commit(&mut self, up_mask: u16, down_mask: u16, completion_qpc: u64) {
        assert_eq!(up_mask & down_mask, 0);
        for slot in 0..MAX_KEYS {
            if (up_mask & (1u16 << slot)) != 0 {
                assert!(self.active_deadlines[slot].is_some());
                self.active_deadlines[slot] = None;
            }
        }
        if down_mask != 0 {
            let (_, expected_mask) = self.chords[self.cursor];
            assert_eq!(down_mask, expected_mask);
            for slot in 0..MAX_KEYS {
                if (down_mask & (1u16 << slot)) != 0 {
                    assert!(self.active_deadlines[slot].is_none());
                    self.active_deadlines[slot] = Some(completion_qpc + self.hold_ticks);
                }
            }
            self.cursor += 1;
        }
    }
}

#[test]
fn candidate_executor_matches_independent_oracle_across_randomized_workloads() {
    let mut rng = Rng(0x00C0_FFEE_2026_1126);
    let qpc_frequencies = [1_000_000u64, 3_579_545, 10_000_000];
    let hold_us = 16_667u64;

    for trial in 0..120usize {
        let qpc_hz = qpc_frequencies[trial % qpc_frequencies.len()];
        let freq = NonZeroU64::new(qpc_hz).unwrap();
        let chord_count = (rng.range(40) + 1) as usize;

        let mut inputs = Vec::with_capacity(chord_count);
        let mut last_by_slot: [Option<u64>; MAX_KEYS] = [None; MAX_KEYS];
        let mut t = rng.range(5_000);
        for idx in 0..chord_count {
            t += rng.range(25_000) + 1;
            let width = (rng.range(MAX_KEYS as u64) + 1) as usize;
            let mut scans = smallvec::SmallVec::<[u16; 4]>::new();
            let mut mask = 0u16;
            for _ in 0..width {
                let slot = rng.range(MAX_KEYS as u64) as usize;
                let bit = 1u16 << slot;
                if (mask & bit) != 0 {
                    continue;
                }
                if let Some(prev) = last_by_slot[slot]
                    && t - prev < hold_us
                {
                    continue;
                }
                mask |= bit;
                scans.push(ALLOWED_15[slot]);
            }
            if scans.is_empty() {
                // Pick the earliest-released slot and advance t to satisfy H.
                let slot = rng.range(MAX_KEYS as u64) as usize;
                if let Some(prev) = last_by_slot[slot]
                    && t - prev < hold_us
                {
                    t = prev + hold_us;
                }
                scans.push(ALLOWED_15[slot]);
                mask = 1u16 << slot;
            }
            for (slot, last_onset) in last_by_slot.iter_mut().enumerate() {
                if (mask & (1u16 << slot)) != 0 {
                    *last_onset = Some(t);
                }
            }
            inputs.push(OnsetChordInput {
                source_action_index: idx as u32,
                scheduled_us: t,
                scan_codes: scans,
            });
        }

        let plan = build_onset_plan(&inputs, &ALLOWED_15, hold_us).expect("valid random plan");
        let prepared = PreparedDispatchSchedule::prepare(&plan, freq, 0).expect("prepared plan");
        let hold_ticks = prepared.hold_ticks().as_u64();
        let end_offset_ticks = sky_dispatch_core::executor::us_to_timeline_ticks_ceil(
            plan.schedule_end_offset_us(),
            freq,
        )
        .unwrap()
        .as_u64();
        let oracle_chords: Vec<(u64, u16)> = plan
            .chords()
            .iter()
            .map(|c| {
                (
                    sky_dispatch_core::executor::us_to_timeline_ticks_ceil(c.scheduled_us, freq)
                        .unwrap()
                        .as_u64(),
                    c.key_mask,
                )
            })
            .collect();
        let mut oracle = MathematicalOracle {
            chords: oracle_chords,
            cursor: 0,
            active_deadlines: [None; MAX_KEYS],
            hold_ticks,
            end_offset_ticks,
        };
        let mut exec = DispatchExecutor::from_prepared_schedule(prepared);

        let mut epoch = QpcTicks::from_raw(50_000_000 + rng.range(10_000));
        let mut now = epoch;

        let mut steps = 0usize;
        loop {
            steps += 1;
            assert!(steps < 50_000, "trial {trial} did not terminate");

            // Occasionally simulate a pause/resume epoch rebase when idle or after cleanup.
            if trial % 7 == 0 && steps == 5 && exec.active_mask() == 0 {
                let pause_delta = DurationTicks::from_raw(rng.range(50_000) + 1);
                epoch = epoch.checked_add_duration(pause_delta).unwrap();
                now = now.checked_add_duration(pause_delta).unwrap();
            }

            let expected = oracle.decide(now.as_u64(), epoch.as_u64());
            let actual = exec.evaluate_at(now, epoch).expect("candidate eval");

            match (expected, actual) {
                (OracleDecision::Finished, DispatchDecision::Finished) => break,
                (
                    OracleDecision::Wait {
                        deadline_qpc: exp_d,
                    },
                    DispatchDecision::Wait {
                        deadline_qpc: act_d,
                    },
                ) => {
                    assert_eq!(
                        act_d.as_u64(),
                        exp_d,
                        "trial {trial} step {steps} wait mismatch"
                    );
                    assert!(act_d > now, "wait must strictly advance");
                    // Simulate occasional late timer wake!
                    let wake_jitter = rng.range(500);
                    now = QpcTicks::from_raw(act_d.as_u64() + wake_jitter);
                }
                (
                    OracleDecision::Send {
                        up_mask: exp_u,
                        down_mask: exp_d,
                    },
                    DispatchDecision::Send {
                        up_mask: act_u,
                        down_chord,
                        ..
                    },
                ) => {
                    let act_d = down_chord.map_or(0, |c| c.key_mask);
                    assert_eq!(
                        (act_u, act_d),
                        (exp_u, exp_d),
                        "trial {trial} step {steps} send mask mismatch"
                    );
                    let latency = rng.range(2_000);
                    let completion = now.as_u64() + latency;
                    oracle.commit(act_u, act_d, completion);
                    exec.on_send_completed_at(act_u, act_d, QpcTicks::from_raw(completion))
                        .expect("candidate commit");
                    now = QpcTicks::from_raw(completion);
                }
                (exp, act) => panic!(
                    "trial {trial} step {steps} decision mismatch: oracle={exp:?}, candidate={act:?}"
                ),
            }
        }

        exec.check_invariants().expect("invariants");
        let acc = exec.accounting();
        assert_eq!(acc.total, plan.generation_count());
        assert_eq!(acc.activated, plan.generation_count());
        assert_eq!(acc.released, plan.generation_count());
        assert_eq!(acc.active, 0);
        assert_eq!(acc.scheduled, 0);
    }
}
