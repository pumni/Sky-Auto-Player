use super::*;
use crate::model::OnsetChordInput;
use crate::plan::build_onset_plan;
use crate::time::{DurationTicks, QpcTicks};

#[test]
fn large_plan_20000_chords_executes_cleanly_within_budget() {
    let chord_count = 20_000usize;
    let hold_us = 16_667u64;
    let allowed: Vec<u16> = (0..MAX_KEYS).map(|i| 0x15 + i as u16).collect();
    let mut inputs = Vec::with_capacity(chord_count);
    for i in 0..chord_count {
        let slot = i % MAX_KEYS;
        inputs.push(OnsetChordInput {
            source_action_index: i as u32,
            scheduled_us: i as u64 * 2_000,
            scan_codes: smallvec::smallvec![allowed[slot]],
        });
    }
    let plan = build_onset_plan(&inputs, &allowed, hold_us).expect("20k plan");
    let prepared =
        PreparedDispatchSchedule::prepare(&plan, NonZeroU64::new(10_000_000).unwrap(), 0)
            .expect("prepared 20k plan");
    let mut exec = DispatchExecutor::from_prepared_schedule(prepared);
    let epoch = QpcTicks::from_raw(1_000_000);
    let mut now = epoch;
    let mut sends = 0usize;

    loop {
        match exec.evaluate_at(now, epoch).expect("eval") {
            DispatchDecision::Finished => break,
            DispatchDecision::Wait { deadline_qpc } => {
                assert!(deadline_qpc > now);
                now = deadline_qpc;
            }
            DispatchDecision::Send {
                up_mask,
                down_chord,
                ..
            } => {
                let down_mask = down_chord.map_or(0, |c| c.key_mask);
                now = now
                    .checked_add_duration(DurationTicks::from_raw(100))
                    .unwrap();
                exec.on_send_completed_at(up_mask, down_mask, now)
                    .expect("commit");
                sends += 1;
            }
        }
    }

    assert!(sends >= chord_count);
    exec.check_invariants().expect("invariants");
    let accounting = exec.accounting();
    assert_eq!(accounting.total, chord_count as u64);
    assert_eq!(accounting.activated, chord_count as u64);
    assert_eq!(accounting.released, chord_count as u64);
    assert_eq!(accounting.active, 0);
    assert_eq!(accounting.scheduled, 0);
    assert_eq!(accounting.cancelled, 0);
}

#[test]
fn scale_fixture_1000_generations_width_5_noncanonical() {
    // Noncanonical scan codes (e.g., 1..=5 row keys) at 5-key chord width, 200 chords = 1,000 generations.
    let noncanonical: [u16; 5] = [0x02, 0x03, 0x04, 0x05, 0x06];
    let chord_count = 200usize;
    let hold_us = 17_167u64;
    let inputs: Vec<OnsetChordInput> = (0..chord_count)
        .map(|i| OnsetChordInput {
            source_action_index: i as u32,
            scheduled_us: (i as u64) * 20_000,
            scan_codes: smallvec::SmallVec::from_slice(&noncanonical),
        })
        .collect();
    let plan = build_onset_plan(&inputs, &noncanonical, hold_us).expect("noncanonical scale plan");
    assert_eq!(plan.generation_count(), 1_000);
    let prepared = PreparedDispatchSchedule::prepare(&plan, NonZeroU64::new(3_579_545).unwrap(), 0)
        .expect("prepared noncanonical plan");
    let mut exec = DispatchExecutor::from_prepared_schedule(prepared);
    let epoch = QpcTicks::from_raw(50_000);
    let mut now = epoch;
    loop {
        match exec.evaluate_at(now, epoch).expect("eval") {
            DispatchDecision::Finished => break,
            DispatchDecision::Wait { deadline_qpc } => now = deadline_qpc,
            DispatchDecision::Send {
                up_mask,
                down_chord,
                ..
            } => {
                let down_mask = down_chord.map_or(0, |c| c.key_mask);
                now = now
                    .checked_add_duration(DurationTicks::from_raw(50))
                    .unwrap();
                exec.on_send_completed_at(up_mask, down_mask, now)
                    .expect("commit");
            }
        }
    }
    exec.check_invariants().expect("invariants");
    let acc = exec.accounting();
    assert_eq!(acc.total, 1_000);
    assert_eq!(acc.activated, 1_000);
    assert_eq!(acc.released, 1_000);
    assert_eq!(acc.active, 0);
}
