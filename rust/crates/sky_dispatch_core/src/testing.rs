//! Fake clock and deterministic authored-packet simulation harness.

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
    // Mirror the native session admission boundary.  Invalid authored holds
    // must be rejected before the simulation can produce a physical event.
    crate::validation::validate_min_hold_feasibility(&schedule, min_hold_us)
        .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
    let mut coordinator = RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        min_hold_us,
        crate::time::DurationTicks::from_raw(min_hold_us),
        |microseconds| Ok(crate::time::TimelineTicks::from_raw(microseconds)),
    )
    .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;

    let mut events = Vec::new();
    let mut step = 0u32;
    let mut iterations = 0u32;
    let mut now_us = coordinator
        .schedule
        .batches
        .first()
        .map_or(0, |batch| batch.scheduled_us);

    while !coordinator.is_finished() {
        iterations = iterations.saturating_add(1);
        if iterations > 20_000 {
            return Err(crate::compile::CompileError::Simulation(
                "simulation step budget exceeded".to_string(),
            ));
        }

        let pending_target = coordinator.earliest_pending_release_ticks();
        if let Some(prepared) = coordinator
            .prepare_current_stale_packet()
            .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?
            .filter(|prepared| {
                pending_target.is_none_or(|pending| prepared.effective_scheduled_ticks <= pending)
            })
        {
            let packet = coordinator
                .schedule
                .view_packet_ticks(prepared.packet_index, prepared.effective_scheduled_ticks)
                .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
            let scan_codes = packet
                .up_intents
                .iter()
                .map(|intent| {
                    packet
                        .registry
                        .scan_code_for(intent.key_slot())
                        .expect("compiled stale key slot must belong to key registry")
                })
                .collect();
            let generation_ids = packet
                .up_intents
                .iter()
                .map(|intent| {
                    (intent.generation_id() != NO_GENERATION_ID).then_some(intent.generation_id())
                })
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
                generation_ids,
                outcome: "suppressed_stale_up".to_string(),
            });
            step += 1;
            continue;
        }

        let authored = coordinator
            .prepare_current_authored_frame()
            .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
        let authored_target = authored.map(|frame| frame.authored_ticks);
        let target = match (pending_target, authored_target) {
            (Some(pending), Some(authored)) => pending.min(authored),
            (Some(pending), None) => pending,
            (None, Some(authored)) => authored,
            (None, None) => break,
        };
        now_us = now_us.max(target.as_u64());
        let pending_mask = if pending_target == Some(target) {
            coordinator.pending_release_mask_due_at(target)
        } else {
            0
        };

        let authored_frame = authored.filter(|frame| frame.authored_ticks == target);
        let (up_mask, down_mask, scan_codes, generation_ids, authored_commit) = if let Some(frame) =
            authored_frame
        {
            let commit = coordinator
                .prepare_authored_commit(frame)
                .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
            let packet_data = {
                let packet = coordinator
                    .schedule
                    .view_packet_ticks(frame.packet_index, frame.authored_ticks)
                    .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
                let up_mask = frame.immediate_up_mask | pending_mask;
                let down_mask = frame.down_mask;
                let mut scan_codes =
                    Vec::with_capacity((up_mask.count_ones() + down_mask.count_ones()) as usize);
                let mut generation_ids = Vec::with_capacity(scan_codes.capacity());
                for slot in 0..MAX_KEYS {
                    let bit = 1u16 << slot;
                    if up_mask & bit == 0 {
                        continue;
                    }
                    let authored_intent = packet
                        .up_intents
                        .iter()
                        .find(|intent| intent.key_slot() == slot as KeySlot)
                        .copied();
                    let (scan_code, generation_id) = authored_intent
                        .map(|intent| {
                            (
                                packet
                                    .registry
                                    .scan_code_for(intent.key_slot())
                                    .expect("compiled key slot must belong to key registry"),
                                (intent.generation_id() != NO_GENERATION_ID)
                                    .then_some(intent.generation_id()),
                            )
                        })
                        .or_else(|| {
                            coordinator
                                .pending_release_for_slot(slot as KeySlot)
                                .map(|pending| {
                                    (
                                        packet
                                            .registry
                                            .scan_code_for(pending.key_slot)
                                            .expect("pending key slot must belong to registry"),
                                        Some(pending.generation_id),
                                    )
                                })
                        })
                        .ok_or_else(|| {
                            crate::compile::CompileError::Simulation(
                                "selected Up has no frozen generation evidence".to_string(),
                            )
                        })?;
                    scan_codes.push(scan_code);
                    generation_ids.push(generation_id);
                }
                for intent in packet.down_intents.iter().copied() {
                    if down_mask & (1u16 << intent.key_slot()) == 0 {
                        continue;
                    }
                    scan_codes.push(
                        packet
                            .registry
                            .scan_code_for(intent.key_slot())
                            .expect("compiled key slot must belong to key registry"),
                    );
                    generation_ids.push(
                        (intent.generation_id() != NO_GENERATION_ID)
                            .then_some(intent.generation_id()),
                    );
                }
                (up_mask, down_mask, scan_codes, generation_ids)
            };
            (
                packet_data.0,
                packet_data.1,
                packet_data.2,
                packet_data.3,
                Some(commit),
            )
        } else {
            let mut scan_codes = Vec::with_capacity(pending_mask.count_ones() as usize);
            let mut generation_ids = Vec::with_capacity(scan_codes.capacity());
            for slot in 0..MAX_KEYS {
                let bit = 1u16 << slot;
                if pending_mask & bit == 0 {
                    continue;
                }
                let pending = coordinator
                    .pending_release_for_slot(slot as KeySlot)
                    .ok_or_else(|| {
                        crate::compile::CompileError::Simulation(
                            "pending release mask has no generation evidence".to_string(),
                        )
                    })?;
                scan_codes.push(
                    coordinator
                        .schedule
                        .key_registry
                        .scan_code_for(pending.key_slot)
                        .expect("pending key slot must belong to registry"),
                );
                generation_ids.push(Some(pending.generation_id));
            }
            (pending_mask, 0, scan_codes, generation_ids, None)
        };
        if up_mask == 0 && down_mask == 0 {
            let commit = authored_commit.ok_or_else(|| {
                crate::compile::CompileError::Simulation(
                    "empty authored boundary has no metadata token".to_string(),
                )
            })?;
            coordinator
                .commit_prepared_authored_frame_metadata_frozen(&commit)
                .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
            continue;
        }
        let completed_us = now_us.checked_add(send_latency_us).ok_or_else(|| {
            crate::compile::CompileError::Simulation("simulation timestamp overflow".to_string())
        })?;
        if pending_mask != 0 {
            coordinator
                .commit_pending_release_success(
                    pending_mask,
                    crate::time::TimelineTicks::from_raw(now_us),
                )
                .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
        }
        if let Some(commit) = authored_commit {
            coordinator
                .commit_prepared_authored_frame_success_frozen(
                    &commit,
                    crate::time::TimelineTicks::from_raw(now_us),
                    crate::time::TimelineTicks::from_raw(completed_us),
                )
                .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
        }
        let kind = match (up_mask != 0, down_mask != 0) {
            (true, true) => "mixed",
            (true, false) => "up",
            (false, true) => "down",
            (false, false) => unreachable!("empty physical packet handled above"),
        };
        events.push(TraceEvent {
            step,
            kind: kind.to_string(),
            scheduled_us: target.as_u64(),
            actual_us: now_us,
            completed_us,
            scan_codes,
            generation_ids,
            outcome: "sent".to_string(),
        });
        step += 1;
        now_us = completed_us;
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
