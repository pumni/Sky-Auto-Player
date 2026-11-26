//! Fake-clock driver for the production onset-only dispatch executor.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;

use crate::executor::{DispatchDecision, DispatchExecutor};
use crate::model::{MAX_KEYS, OnsetChordInput, generation_id_for};
use crate::plan::{PlanError, build_onset_plan};
use crate::time::{DurationTicks, QpcTicks, TimelineTicks};

#[cfg(any(test, feature = "test-support"))]
pub fn assert_clean_generation_completion(accounting: crate::accounting::GenerationAccounting) {
    assert_eq!(accounting.total, accounting.activated);
    assert_eq!(accounting.activated, accounting.released);
    assert_eq!(accounting.scheduled, 0);
    assert_eq!(accounting.active, 0);
    assert_eq!(accounting.dropped_conflict, 0);
    assert_eq!(accounting.dropped_backend, 0);
    assert_eq!(accounting.dropped_expired, 0);
    assert_eq!(accounting.cancelled, 0);
}

#[derive(Debug, Clone)]
pub struct FakeClock {
    now: Rc<RefCell<u64>>,
}

impl FakeClock {
    pub fn new(initial_us: u64) -> Self {
        Self {
            now: Rc::new(RefCell::new(initial_us)),
        }
    }

    pub fn now_us(&self) -> u64 {
        *self.now.borrow()
    }

    pub fn set_now_us(&self, us: u64) {
        *self.now.borrow_mut() = us;
    }

    pub fn advance(&self, us: u64) {
        *self.now.borrow_mut() += us;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceEvent {
    pub step: u32,
    pub kind: String,
    pub scheduled_us: u64,
    pub actual_us: u64,
    pub completed_us: u64,
    pub scan_codes: Vec<u16>,
    pub generation_ids: Vec<Option<u64>>,
    pub outcome: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimulationResult {
    pub events: Vec<TraceEvent>,
    pub status_counts: std::collections::HashMap<String, u64>,
    pub total_generations: u64,
    pub is_finished: bool,
}

/// Drive `DispatchExecutor` with a deterministic sender completion latency.
/// This is an offline executor simulation; worker admission and Win32 delivery
/// are covered by `sky_player`'s production-worker tests.
pub fn simulate_schedule(
    chords: &[OnsetChordInput],
    allowed_scan_codes: &[u16],
    hold_us: u64,
    send_latency_us: u64,
) -> Result<SimulationResult, PlanError> {
    let plan = build_onset_plan(chords, allowed_scan_codes, hold_us)?;
    let hold_ticks = DurationTicks::from_raw(hold_us);
    let mut executor = DispatchExecutor::new(&plan, hold_ticks, |microseconds| {
        Ok::<_, std::convert::Infallible>(TimelineTicks::from_raw(microseconds))
    })
    .map_err(PlanError::Simulation)?;

    let mut events = Vec::new();
    let mut step = 0u32;
    let mut now_us = 0u64;
    let mut iterations = 0u32;

    loop {
        iterations = iterations.saturating_add(1);
        if iterations > 20_000 {
            return Err(PlanError::Simulation(
                "simulation step budget exceeded".to_string(),
            ));
        }

        match executor
            .evaluate_at(QpcTicks::from_raw(now_us), QpcTicks::ZERO)
            .map_err(|error| PlanError::Simulation(error.to_string()))?
        {
            DispatchDecision::Finished => break,
            DispatchDecision::Wait { deadline_qpc } => {
                let deadline_us = deadline_qpc.as_u64();
                if deadline_us <= now_us {
                    return Err(PlanError::Simulation(
                        "executor returned a non-advancing wait deadline".to_string(),
                    ));
                }
                now_us = deadline_us;
            }
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
            } => {
                let scheduled_us = timing_window.authored_target_qpc.as_u64();
                let actual_us = now_us.max(scheduled_us);
                let completed_us = actual_us.checked_add(send_latency_us).ok_or_else(|| {
                    PlanError::Simulation("simulation timestamp overflow".to_string())
                })?;
                let down_mask = down_chord.map_or(0, |chord| chord.key_mask);
                if up_mask == 0 && down_mask == 0 {
                    return Err(PlanError::Simulation(
                        "executor produced an empty send decision".to_string(),
                    ));
                }

                let mut scan_codes = Vec::with_capacity(MAX_KEYS);
                let mut generation_ids = Vec::with_capacity(MAX_KEYS);
                let mut remaining_up = up_mask;
                while remaining_up != 0 {
                    let slot = remaining_up.trailing_zeros() as u8;
                    remaining_up &= remaining_up - 1;
                    let generation_id =
                        executor.active_generation_for_slot(slot).ok_or_else(|| {
                            PlanError::Simulation(format!("selected Up for inactive slot {slot}"))
                        })?;
                    let scan_code =
                        executor.key_registry().scan_code_for(slot).ok_or_else(|| {
                            PlanError::Simulation(format!(
                                "selected Up slot {slot} has no scan code"
                            ))
                        })?;
                    scan_codes.push(scan_code);
                    generation_ids.push(Some(generation_id));
                }
                if let Some(chord) = down_chord {
                    let mut remaining_down = chord.key_mask;
                    while remaining_down != 0 {
                        let slot = remaining_down.trailing_zeros() as u8;
                        remaining_down &= remaining_down - 1;
                        let scan_code =
                            executor.key_registry().scan_code_for(slot).ok_or_else(|| {
                                PlanError::Simulation(format!(
                                    "selected Down slot {slot} has no scan code"
                                ))
                            })?;
                        scan_codes.push(scan_code);
                        generation_ids.push(generation_id_for(chord.chord_index, slot));
                    }
                }

                executor
                    .on_send_completed_at(up_mask, down_mask, QpcTicks::from_raw(completed_us))
                    .map_err(PlanError::Simulation)?;

                events.push(TraceEvent {
                    step,
                    kind: match (up_mask != 0, down_mask != 0) {
                        (true, true) => "mixed",
                        (true, false) => "up",
                        (false, true) => "down",
                        (false, false) => unreachable!("empty send decisions are rejected above"),
                    }
                    .to_string(),
                    scheduled_us,
                    actual_us,
                    completed_us,
                    scan_codes,
                    generation_ids,
                    outcome: "sent".to_string(),
                });
                step = step.checked_add(1).ok_or_else(|| {
                    PlanError::Simulation("simulation event count overflow".to_string())
                })?;
                now_us = completed_us;
            }
        }
    }

    let accounting = executor.accounting();
    let status_counts = [
        ("active", accounting.active),
        ("cancelled", accounting.cancelled),
        ("dropped_backend", accounting.dropped_backend),
        ("dropped_conflict", accounting.dropped_conflict),
        ("dropped_expired", accounting.dropped_expired),
        ("released", accounting.released),
        ("scheduled", accounting.scheduled),
    ]
    .into_iter()
    .map(|(name, count)| (name.to_string(), count))
    .collect();

    let is_finished = executor
        .is_finished_at(QpcTicks::from_raw(now_us), QpcTicks::ZERO)
        .map_err(|error| PlanError::Simulation(error.to_string()))?;

    Ok(SimulationResult {
        events,
        status_counts,
        total_generations: accounting.total,
        is_finished,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_simulation() {
        let allowed = vec![1, 2];
        let chords = vec![OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 1_000,
            scan_codes: vec![1].into(),
        }];

        let result = simulate_schedule(&chords, &allowed, 50, 10).unwrap();
        assert!(result.is_finished);
        assert_eq!(result.total_generations, 1);
        assert_eq!(result.status_counts.get("released"), Some(&1));
        assert_eq!(result.events.len(), 2);
        assert_eq!(result.events[0].kind, "down");
        assert_eq!(result.events[0].actual_us, 1_000);
        assert_eq!(result.events[0].completed_us, 1_010);
        assert_eq!(result.events[1].kind, "up");
        assert_eq!(result.events[1].actual_us, 1_060);
    }
}
