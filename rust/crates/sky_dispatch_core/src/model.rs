//! Domain types for dispatch schedule, key intents, and key registry.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

pub const MAX_KEYS: usize = 15;

pub type GenerationId = u64;
pub type ReasonId = u16;
pub type PacketId = u32;
pub type KeySlot = u8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Down,
    Up,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyRegistry {
    scan_codes: SmallVec<[u16; MAX_KEYS]>,
}

impl KeyRegistry {
    pub fn new(scan_codes: &[u16]) -> Self {
        let mut vec = SmallVec::new();
        for &code in scan_codes {
            if !vec.contains(&code) {
                vec.push(code);
            }
        }
        Self { scan_codes: vec }
    }

    pub fn slot_for(&self, scan_code: u16) -> Option<KeySlot> {
        self.scan_codes
            .iter()
            .position(|&c| c == scan_code)
            .map(|p| p as KeySlot)
    }

    pub fn scan_code_for(&self, slot: KeySlot) -> Option<u16> {
        self.scan_codes.get(slot as usize).copied()
    }

    pub fn scan_codes(&self) -> &[u16] {
        &self.scan_codes
    }

    pub fn len(&self) -> usize {
        self.scan_codes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scan_codes.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeKeyIntent {
    pub source_action_index: u32,
    pub generation_id: Option<GenerationId>,
    pub kind: ActionKind,
    pub scan_code: u16,
    pub key_slot: KeySlot,
    pub scheduled_us: u64,
    pub reason_id: ReasonId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeBatch {
    pub source_action_index: u32,
    pub kind: ActionKind,
    pub scheduled_us: u64,
    pub reason_id: ReasonId,
    pub intents: SmallVec<[RuntimeKeyIntent; 8]>,
    pub packet_id: PacketId,
}

#[derive(Debug, Clone)]
pub struct RuntimeSchedule {
    pub batches: Vec<RuntimeBatch>,
    pub generation_count: u64,
    pub key_registry: KeyRegistry,
    pub reason_table: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyActionInput {
    pub source_action_index: u32,
    pub kind: ActionKind,
    pub scheduled_us: u64,
    pub scan_codes: Vec<u16>,
    pub reason: String,
}
