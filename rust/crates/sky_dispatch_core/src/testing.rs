//! Fake-clock driver for the production dispatch executor.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;

use crate::executor::{DispatchDecision, DispatchExecutor};
use crate::model::{KeyActionInput, MAX_KEYS};
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
    actions: &[KeyActionInput],
    allowed_scan_codes: &[u16],
    min_hold_us: u64,
    send_latency_us: u64,
) -> Result<SimulationResult, crate::compile::CompileError> {
    let schedule = crate::compile::compile_runtime_intents(actions, allowed_scan_codes)?;
    crate::validation::validate_min_hold_feasibility(&schedule, min_hold_us)
        .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;

    let min_hold = DurationTicks::from_raw(min_hold_us);
    let mut executor = DispatchExecutor::new(&schedule, min_hold, min_hold, |microseconds| {
        Ok::<_, std::convert::Infallible>(TimelineTicks::from_raw(microseconds))
    })
    .map_err(crate::compile::CompileError::Simulation)?;

    let mut events = Vec::new();
    let mut step = 0u32;
    let mut now_us = 0u64;
    let mut iterations = 0u32;

    loop {
        iterations = iterations.saturating_add(1);
        if iterations > 20_000 {
            return Err(crate::compile::CompileError::Simulation(
                "simulation step budget exceeded".to_string(),
            ));
        }

        match executor
            .evaluate_at(QpcTicks::from_raw(now_us), QpcTicks::ZERO)
            .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?
        {
            DispatchDecision::Finished => break,
            DispatchDecision::Wait { deadline_qpc } => {
                let deadline_us = deadline_qpc.as_u64();
                if deadline_us <= now_us {
                    return Err(crate::compile::CompileError::Simulation(
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
                    crate::compile::CompileError::Simulation(
                        "simulation timestamp overflow".to_string(),
                    )
                })?;
                let down_mask = down_chord.map_or(0, |chord| chord.down_mask);
                if up_mask == 0 && down_mask == 0 {
                    return Err(crate::compile::CompileError::Simulation(
                        "executor produced an empty send decision".to_string(),
                    ));
                }

                let mut scan_codes = Vec::with_capacity(MAX_KEYS);
                let mut generation_ids = Vec::with_capacity(MAX_KEYS);
                for slot in 0..MAX_KEYS {
                    let bit = 1u16 << slot;
                    if up_mask & bit == 0 {
                        continue;
                    }
                    let musical_slot = &executor.slots()[slot];
                    let generation_id = musical_slot.active_generation.ok_or_else(|| {
                        crate::compile::CompileError::Simulation(format!(
                            "selected Up for inactive slot {slot}"
                        ))
                    })?;
                    let scan_code = executor
                        .key_registry()
                        .scan_code_for(slot as u8)
                        .ok_or_else(|| {
                            crate::compile::CompileError::Simulation(format!(
                                "selected Up slot {slot} has no scan code"
                            ))
                        })?;
                    scan_codes.push(scan_code);
                    generation_ids.push(Some(generation_id));
                }
                if let Some(chord) = down_chord {
                    for intent in chord.intents {
                        scan_codes.push(intent.scan_code);
                        generation_ids.push(Some(intent.generation_id));
                    }
                }

                executor
                    .on_send_completed_at(up_mask, down_mask, QpcTicks::from_raw(completed_us))
                    .map_err(crate::compile::CompileError::Simulation)?;

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
                    crate::compile::CompileError::Simulation(
                        "simulation event count overflow".to_string(),
                    )
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
        .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;

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
    use crate::model::ActionKind;

    #[test]
    fn test_simple_simulation() {
        let allowed = vec![1, 2];
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: vec![1].into(),
                reason: "note".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1_100,
                scan_codes: vec![1].into(),
                reason: "note".into(),
            },
        ];

        let result = simulate_schedule(&actions, &allowed, 50, 10).unwrap();
        assert!(result.is_finished);
        assert_eq!(result.total_generations, 1);
        assert_eq!(result.status_counts.get("released"), Some(&1));
        assert_eq!(result.events.len(), 2);
    }
}
