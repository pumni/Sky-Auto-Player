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
    let mut coordinator = RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        min_hold_us,
        crate::time::DurationTicks::from_raw(min_hold_us),
        0,
        crate::time::DurationTicks::ZERO,
        |microseconds| Ok(crate::time::TimelineTicks::from_raw(microseconds)),
    )
    .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;

    let mut events = Vec::new();
    let mut step: u32 = 0;
    let mut iterations: u32 = 0;

    let mut now_us: u64 = if let Some(batch) = coordinator.schedule.batches.first() {
        batch.scheduled_us
    } else {
        0
    };

    while !coordinator.is_finished() {
        iterations = iterations.saturating_add(1);
        if iterations > 20_000 {
            return Err(crate::compile::CompileError::Simulation(
                "simulation step budget exceeded".to_string(),
            ));
        }
        if coordinator.schedule.batches.len() <= coordinator.cursor
            && coordinator.pending_mask == 0
            && coordinator.active_mask != 0
        {
            return Err(crate::compile::CompileError::Simulation(
                "simulation incomplete: active generations remain".to_string(),
            ));
        }
        let current_stale = coordinator
            .prepare_current_stale_packet()
            .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
        let pending_deadline = coordinator
            .next_pending_release_ticks(crate::time::DurationTicks::ZERO)
            .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
        let pending_plan = pending_deadline.map(|deadline_ticks| PendingDispatchPlan {
            deadline_ticks,
            polyphony: 1,
        });
        if current_stale.is_none()
            && let Some(dl) = coordinator
                .next_deadline_ticks(crate::time::DurationTicks::ZERO, pending_plan.as_ref())
                .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?
        {
            now_us = now_us.max(dl.as_u64());
        }

        // 1. Drain pending releases due
        let plan = PendingDispatchPlan {
            deadline_ticks: crate::time::TimelineTicks::from_raw(now_us),
            polyphony: 1,
        };
        let due_pending = coordinator
            .pop_due_pending_ticks(crate::time::TimelineTicks::from_raw(now_us), &plan)
            .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
        if !due_pending.is_empty() {
            let scan_codes: Vec<u16> = due_pending.iter().map(|p| p.scan_code).collect();
            let gen_ids: Vec<Option<u64>> =
                due_pending.iter().map(|p| Some(p.generation_id)).collect();
            let completed_us = now_us.checked_add(send_latency_us).ok_or_else(|| {
                crate::compile::CompileError::Simulation(
                    "simulation timestamp overflow".to_string(),
                )
            })?;

            coordinator
                .complete_releases(&due_pending, &scan_codes)
                .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;

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

        // Stale metadata is never physical work.  Consume exactly one
        // compiled packet at the current cursor without assigning it an
        // authored deadline or a physical dispatch path.
        if let Some(prepared) = current_stale {
            let packet = coordinator
                .schedule
                .view_packet_ticks(prepared.packet_index, prepared.effective_scheduled_ticks)
                .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
            let scan_codes: Vec<u16> = packet
                .up_intents
                .iter()
                .map(|i| {
                    packet
                        .registry
                        .scan_code_for(i.key_slot())
                        .expect("compiled stale key slot must belong to key registry")
                })
                .collect();
            let gen_ids: Vec<Option<u64>> = packet
                .up_intents
                .iter()
                .map(|i| (i.generation_id() != NO_GENERATION_ID).then_some(i.generation_id()))
                .collect();
            coordinator
                .commit_stale_packet(prepared)
                .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
            events.push(TraceEvent {
                step,
                kind: "up".to_string(),
                scheduled_us: prepared.effective_scheduled_ticks.as_u64(),
                actual_us: now_us,
                completed_us: now_us,
                scan_codes,
                generation_ids: gen_ids,
                outcome: "suppressed_stale_up".to_string(),
            });
            step += 1;
            continue;
        }

        // 2. Drain authored batch
        if let Some((batch_index, _lead)) = coordinator
            .pop_next_due_authored_ticks(
                crate::time::TimelineTicks::from_raw(now_us),
                crate::time::DurationTicks::ZERO,
            )
            .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?
        {
            let batch = coordinator
                .schedule
                .try_materialize_batch_authored(batch_index)
                .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
            match batch.kind {
                ActionKind::Down => {
                    let (playable, conflicts) = coordinator
                        .split_down_intents(&batch.intents)
                        .map_err(|error| {
                            crate::compile::CompileError::Simulation(error.to_string())
                        })?;
                    if !playable.is_empty() {
                        let scan_codes: Vec<u16> = playable.iter().map(|i| i.scan_code).collect();
                        let gen_ids: Vec<Option<u64>> =
                            playable.iter().map(|i| i.generation_id).collect();
                        let completed_us =
                            now_us.checked_add(send_latency_us).ok_or_else(|| {
                                crate::compile::CompileError::Simulation(
                                    "simulation timestamp overflow".to_string(),
                                )
                            })?;

                        coordinator
                            .activate_sent_downs_ticks(
                                &playable,
                                &scan_codes,
                                crate::time::TimelineTicks::from_raw(now_us),
                                crate::time::TimelineTicks::from_raw(completed_us),
                            )
                            .map_err(|error| {
                                crate::compile::CompileError::Simulation(error.to_string())
                            })?;

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
                    let (requested, suppressed) = coordinator
                        .request_releases(&batch.intents)
                        .map_err(|error| {
                            crate::compile::CompileError::Simulation(error.to_string())
                        })?;
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
            now_us = now_us.checked_add(100).ok_or_else(|| {
                crate::compile::CompileError::Simulation(
                    "simulation timestamp overflow".to_string(),
                )
            })?;
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
                scan_codes: vec![1].into(),
                reason: "note".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1100,
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
