//! Domain types for dispatch schedule, key intents, and key registry.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

pub const MAX_KEYS: usize = 15;

pub type GenerationId = u64;
pub type ReasonId = u16;
pub type PacketId = u32;
pub type KeySlot = u8;
pub const NO_GENERATION_ID: GenerationId = GenerationId::MAX;
/// CompactIntent reserves the all-ones 60-bit generation value as the
/// unmatched-up sentinel.  This is still far beyond the configured action
/// cap, but keeping the bound explicit prevents a silent alias at the packed
/// representation boundary.
pub const MAX_COMPACT_GENERATION_ID: GenerationId = (u64::MAX >> 4) - 1;

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
    pub packet_id: PacketId,
}

/// Compact immutable intent stored in the schedule arena.
///
/// The batch header owns the fields shared by every intent in a chord.  The
/// full `RuntimeKeyIntent` remains the short-lived materialized view consumed
/// by the coordinator and worker, so the million-action schedule does not
/// inline fifteen copies of those fields for every action.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactIntent(u64);

impl CompactIntent {
    pub fn new(generation_id: GenerationId, key_slot: KeySlot) -> Self {
        debug_assert!(key_slot < 16, "compact key slot must fit in four bits");
        let generation_bits = if generation_id == NO_GENERATION_ID {
            u64::MAX >> 4
        } else {
            debug_assert!(generation_id <= MAX_COMPACT_GENERATION_ID);
            generation_id
        };
        Self((generation_bits << 4) | u64::from(key_slot))
    }

    pub fn generation_id(self) -> GenerationId {
        let generation = self.0 >> 4;
        if generation == u64::MAX >> 4 {
            NO_GENERATION_ID
        } else {
            generation
        }
    }

    pub fn key_slot(self) -> KeySlot {
        (self.0 & 0x0f) as KeySlot
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledBatch {
    pub source_action_index: u32,
    pub kind: ActionKind,
    pub scheduled_us: u64,
    pub reason_id: ReasonId,
    pub intent_start: u32,
    pub intent_len: u8,
    pub packet_id: PacketId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeBatch {
    pub source_action_index: u32,
    pub kind: ActionKind,
    pub scheduled_us: u64,
    pub reason_id: ReasonId,
    pub intents: SmallVec<[RuntimeKeyIntent; MAX_KEYS]>,
    pub packet_id: PacketId,
}

#[derive(Debug, Clone)]
pub struct RuntimeSchedule {
    pub batches: Vec<CompiledBatch>,
    pub intents: Vec<CompactIntent>,
    pub generation_count: u64,
    pub key_registry: KeyRegistry,
    pub reason_table: Vec<String>,
}

impl RuntimeSchedule {
    pub fn materialize_batch(&self, index: usize, scheduled_offset_us: u64) -> RuntimeBatch {
        let header = self
            .batches
            .get(index)
            .expect("runtime batch index must be valid");
        let scheduled_us = header.scheduled_us.saturating_add(scheduled_offset_us);
        let start = header.intent_start as usize;
        let end = start + header.intent_len as usize;
        let intents = self.intents[start..end]
            .iter()
            .map(|compact| RuntimeKeyIntent {
                source_action_index: header.source_action_index,
                generation_id: (compact.generation_id() != NO_GENERATION_ID)
                    .then_some(compact.generation_id()),
                kind: header.kind,
                scan_code: self
                    .key_registry
                    .scan_code_for(compact.key_slot())
                    .expect("compiled key slot must belong to key registry"),
                key_slot: compact.key_slot(),
                scheduled_us,
                reason_id: header.reason_id,
                packet_id: header.packet_id,
            })
            .collect();
        RuntimeBatch {
            source_action_index: header.source_action_index,
            kind: header.kind,
            scheduled_us,
            reason_id: header.reason_id,
            intents,
            packet_id: header.packet_id,
        }
    }

    pub fn intent_slice(&self, batch: &CompiledBatch) -> &[CompactIntent] {
        let start = batch.intent_start as usize;
        let end = start + batch.intent_len as usize;
        &self.intents[start..end]
    }

    /// Borrow a view into batch `index` without allocating.
    ///
    /// Lifetime is tied to `self`.  Use `BatchView::materialize` only
    /// when telemetry requires the full `RuntimeBatch` representation.
    pub fn view_batch(&self, index: usize, scheduled_offset_us: u64) -> BatchView<'_> {
        let header = self
            .batches
            .get(index)
            .expect("runtime batch index must be valid");
        let start = header.intent_start as usize;
        let end = start + header.intent_len as usize;
        BatchView {
            header,
            intents: &self.intents[start..end],
            registry: &self.key_registry,
            scheduled_us: header.scheduled_us.saturating_add(scheduled_offset_us),
        }
    }
}

/// Borrowed zero-copy view of a compiled batch.
///
/// Lifetime is tied to the `RuntimeSchedule` that owns the arena.
/// No heap allocation — header and intents are slices into the flat schedule.
/// Call [`BatchView::materialize`] only when the owned `RuntimeBatch`
/// representation is required (e.g. when full-rate telemetry is enabled).
#[derive(Debug, Clone, Copy)]
pub struct BatchView<'a> {
    pub header: &'a CompiledBatch,
    /// Compact intents stored in the schedule arena (no scan codes inline).
    pub intents: &'a [CompactIntent],
    /// Key registry shared by the whole session (scan-code lookup).
    pub registry: &'a KeyRegistry,
    /// Effective scheduled time including any recovery offset (microseconds).
    pub scheduled_us: u64,
}

impl<'a> BatchView<'a> {
    /// Action kind of this batch.
    #[inline]
    pub fn kind(&self) -> ActionKind {
        self.header.kind
    }

    /// Source action index from the authored schedule.
    #[inline]
    pub fn source_action_index(&self) -> u32 {
        self.header.source_action_index
    }

    /// Reason identifier for telemetry labelling.
    #[inline]
    pub fn reason_id(&self) -> ReasonId {
        self.header.reason_id
    }

    /// Packet identifier.
    #[inline]
    pub fn packet_id(&self) -> PacketId {
        self.header.packet_id
    }

    /// Build a stack-only scan code buffer from this batch.
    ///
    /// `conflict_mask` is a bitmask of key slots to skip (already-active
    /// slots identified by `check_down_conflicts_compact`).  Pass `0` to
    /// include all slots.
    pub fn scan_code_batch_excluding_mask(&self, conflict_mask: u16) -> ScanCodeBatch {
        let mut batch = ScanCodeBatch::new_empty();
        for compact in self.intents {
            let slot = compact.key_slot();
            if conflict_mask & (1u16 << slot) != 0 {
                continue;
            }
            if let Some(sc) = self.registry.scan_code_for(slot) {
                batch.push(sc);
            }
        }
        batch
    }

    /// Build a scan code buffer for ALL intents (no conflict exclusion).
    #[inline]
    pub fn scan_code_batch_all(&self) -> ScanCodeBatch {
        self.scan_code_batch_excluding_mask(0)
    }

    /// Fully materialize into an owned `RuntimeBatch`.
    ///
    /// This allocates a `SmallVec<[RuntimeKeyIntent; MAX_KEYS]>`.
    /// Call only off the hot path (telemetry or diagnostic logging).
    pub fn materialize(&self) -> RuntimeBatch {
        let intents = self
            .intents
            .iter()
            .map(|compact| RuntimeKeyIntent {
                source_action_index: self.header.source_action_index,
                generation_id: (compact.generation_id() != NO_GENERATION_ID)
                    .then_some(compact.generation_id()),
                kind: self.header.kind,
                scan_code: self
                    .registry
                    .scan_code_for(compact.key_slot())
                    .expect("compiled key slot must belong to key registry"),
                key_slot: compact.key_slot(),
                scheduled_us: self.scheduled_us,
                reason_id: self.header.reason_id,
                packet_id: self.header.packet_id,
            })
            .collect();
        RuntimeBatch {
            source_action_index: self.header.source_action_index,
            kind: self.header.kind,
            scheduled_us: self.scheduled_us,
            reason_id: self.header.reason_id,
            intents,
            packet_id: self.header.packet_id,
        }
    }
}

/// Stack-only scan code buffer for a single dispatch batch.
///
/// Avoids any heap allocation on the RT path.  The capacity matches
/// `MAX_KEYS` (15 instrument slots).
#[derive(Debug, Clone, Copy)]
pub struct ScanCodeBatch {
    values: [u16; MAX_KEYS],
    len: u8,
}

impl ScanCodeBatch {
    /// Create an empty buffer.
    #[inline]
    pub fn new_empty() -> Self {
        Self {
            values: [0u16; MAX_KEYS],
            len: 0,
        }
    }

    /// Append a scan code.  Panics (debug) if over capacity.
    #[inline]
    pub fn push(&mut self, code: u16) {
        debug_assert!(
            (self.len as usize) < MAX_KEYS,
            "ScanCodeBatch overflow: len={} capacity={}",
            self.len,
            MAX_KEYS
        );
        self.values[self.len as usize] = code;
        self.len += 1;
    }

    /// Slice view of the populated entries.
    #[inline]
    pub fn as_slice(&self) -> &[u16] {
        &self.values[..self.len as usize]
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyActionInput {
    pub source_action_index: u32,
    pub kind: ActionKind,
    pub scheduled_us: u64,
    pub scan_codes: SmallVec<[u16; 4]>,
    pub reason: Arc<str>,
}
