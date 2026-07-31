//! Fake clock and deterministic simulation harness for differential testing.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;

use crate::coordinator::*;
use crate::model::*;

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

pub fn simulate_schedule(
    actions: &[KeyActionInput],
    allowed_scan_codes: &[u16],
    min_hold_us: u64,
    send_latency_us: u64,
) -> Result<SimulationResult, crate::compile::CompileError> {
    let schedule = crate::compile::compile_runtime_intents(actions, allowed_scan_codes)?;
    let mut coordinator = RuntimeDispatchCoordinator::new(schedule, min_hold_us, crate::time::TimelineTicks);


    let mut events = Vec::new();
    let mut step: u32 = 0;

    let mut now_us: u64 = if let Some(batch) = coordinator.schedule.batches.first() {
        batch.scheduled_us
    } else {
        0
    };

    while !coordinator.is_finished() {
        if let Some(dl) = coordinator.next_deadline_us(0, 0) {
            now_us = now_us.max(dl);
        }

        // 1. Drain pending releases due
        let due_pending = coordinator.pop_due_pending(now_us, 0);
        if !due_pending.is_empty() {
            let scan_codes: Vec<u16> = due_pending.iter().map(|p| p.scan_code).collect();
            let gen_ids: Vec<Option<u64>> =
                due_pending.iter().map(|p| Some(p.generation_id)).collect();
            let completed_us = now_us + send_latency_us;

            coordinator.complete_releases(&due_pending, &scan_codes, &[]);

            events.push(TraceEvent {
                step,
                kind: "up".to_string(),
                scheduled_us: due_pending[0].scheduled_release_us,
                actual_us: now_us,
                completed_us,
                scan_codes,
                generation_ids: gen_ids,
                outcome: "released".to_string(),
            });
            step += 1;
            now_us = completed_us;
            continue;
        }

        // 2. Drain authored batch
        if let Some((batch, _lead)) = coordinator.pop_next_due_authored(now_us, 0) {
            match batch.kind {
                ActionKind::Down => {
                    let (playable, conflicts) = coordinator.split_down_intents(&batch.intents);
                    if !playable.is_empty() {
                        let scan_codes: Vec<u16> = playable.iter().map(|i| i.scan_code).collect();
                        let gen_ids: Vec<Option<u64>> =
                            playable.iter().map(|i| i.generation_id).collect();
                        let completed_us = now_us + send_latency_us;

                        coordinator.activate_sent_downs(
                            &playable,
                            &scan_codes,
                            now_us,
                            crate::time::TimelineTicks(now_us),
                            completed_us,
                            crate::time::TimelineTicks(completed_us),
                        );

                        events.push(TraceEvent {
                            step,
                            kind: "down".to_string(),
                            scheduled_us: batch.scheduled_us,
                            actual_us: now_us,
                            completed_us,
                            scan_codes,
                            generation_ids: gen_ids,
                            outcome: "sent".to_string(),
                        });
                        step += 1;
                        now_us = completed_us;
                    }
                    if !conflicts.is_empty() {
                        let scan_codes: Vec<u16> = conflicts.iter().map(|i| i.scan_code).collect();
                        let gen_ids: Vec<Option<u64>> =
                            conflicts.iter().map(|i| i.generation_id).collect();
                        events.push(TraceEvent {
                            step,
                            kind: "down".to_string(),
                            scheduled_us: batch.scheduled_us,
                            actual_us: now_us,
                            completed_us: now_us,
                            scan_codes,
                            generation_ids: gen_ids,
                            outcome: "dropped_conflict".to_string(),
                        });
                        step += 1;
                    }
                }
                ActionKind::Up => {
                    let (requested, suppressed) = coordinator.request_releases(&batch.intents);
                    if !suppressed.is_empty() {
                        let scan_codes: Vec<u16> = suppressed.iter().map(|i| i.scan_code).collect();
                        let gen_ids: Vec<Option<u64>> =
                            suppressed.iter().map(|i| i.generation_id).collect();
                        events.push(TraceEvent {
                            step,
                            kind: "up".to_string(),
                            scheduled_us: batch.scheduled_us,
                            actual_us: now_us,
                            completed_us: now_us,
                            scan_codes,
                            generation_ids: gen_ids,
                            outcome: "suppressed_stale_up".to_string(),
                        });
                        step += 1;
                    }
                    let _ = requested;
                }
            }
        } else {
            // Advance time if nothing moved
            now_us += 100;
        }
    }

    Ok(SimulationResult {
        events,
        status_counts: coordinator.generation_status_counts(),
        total_generations: coordinator.schedule.generation_count,
        is_finished: coordinator.is_finished(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_simulation() {
        let allowed = vec![1, 2];
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1000,
                scan_codes: vec![1],
                reason: "note".to_string(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1100,
                scan_codes: vec![1],
                reason: "note".to_string(),
            },
        ];

        let result = simulate_schedule(&actions, &allowed, 50, 10).unwrap();
        assert!(result.is_finished);
        assert_eq!(result.total_generations, 1);
        assert_eq!(result.status_counts.get("released"), Some(&1));
        assert_eq!(result.events.len(), 2);
    }
}
