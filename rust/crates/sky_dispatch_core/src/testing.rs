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

        if let Some(prepared) = coordinator
            .prepare_current_stale_packet()
            .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?
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

        let Some(deadline) = coordinator
            .next_uncompensated_deadline_ticks()
            .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?
        else {
            break;
        };
        now_us = now_us.max(deadline.as_u64());
        #[cfg(test)]
        let prepared = coordinator.prepare_next_due_authored(
            crate::time::TimelineTicks::from_raw(now_us),
            crate::time::DurationTicks::ZERO,
        );
        #[cfg(not(test))]
        let prepared =
            coordinator.prepare_next_due_authored(crate::time::TimelineTicks::from_raw(now_us));
        let Some(prepared) = prepared
            .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?
        else {
            now_us = now_us.checked_add(100).ok_or_else(|| {
                crate::compile::CompileError::Simulation(
                    "simulation timestamp overflow".to_string(),
                )
            })?;
            continue;
        };
        let packet = coordinator
            .schedule
            .view_packet_ticks(prepared.packet_index, prepared.effective_scheduled_ticks)
            .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
        let mut scan_codes =
            Vec::with_capacity(packet.up_intents.len() + packet.down_intents.len());
        let mut generation_ids = Vec::with_capacity(scan_codes.capacity());
        for intent in packet.up_intents.iter().chain(packet.down_intents.iter()) {
            scan_codes.push(
                packet
                    .registry
                    .scan_code_for(intent.key_slot())
                    .expect("compiled key slot must belong to key registry"),
            );
            generation_ids.push(
                (intent.generation_id() != NO_GENERATION_ID).then_some(intent.generation_id()),
            );
        }
        let completed_us = now_us.checked_add(send_latency_us).ok_or_else(|| {
            crate::compile::CompileError::Simulation("simulation timestamp overflow".to_string())
        })?;
        coordinator
            .commit_packet_success(
                prepared,
                crate::time::TimelineTicks::from_raw(now_us),
                crate::time::TimelineTicks::from_raw(completed_us),
            )
            .map_err(|error| crate::compile::CompileError::Simulation(error.to_string()))?;
        let kind = match prepared.packet_kind {
            PhysicalPacketKind::UpOnly => "up",
            PhysicalPacketKind::DownOnly => "down",
            PhysicalPacketKind::Mixed => "mixed",
        };
        events.push(TraceEvent {
            step,
            kind: kind.to_string(),
            scheduled_us: prepared.effective_scheduled_ticks.as_u64(),
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
