//! Canonical authored Down generations and their paired Up provenance.

use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::sync::Arc;

pub const MAX_KEYS: usize = 15;
/// Combined capacity budget for retained schedule, coordinator and executor
/// storage. This is a calculated allocation bound, not a process RSS limit.
pub const MAX_RETAINED_DISPATCH_BYTES: usize = 512 * 1024 * 1024;

pub type GenerationId = u64;
pub type ReasonId = u16;
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
            .position(|&code| code == scan_code)
            .map(|position| position as KeySlot)
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

/// One Down generation paired with the exact authored Up that closes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledDownIntent {
    pub generation_id: GenerationId,
    pub key_slot: KeySlot,
    pub scan_code: u16,
    pub source_action_index: u32,
    pub reason_id: ReasonId,
    pub paired_up_offset_us: u64,
    pub paired_up_source_action_index: u32,
    pub paired_up_reason_id: ReasonId,
}

/// One authored Down chord and the paired Up provenance for every key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledDownChord {
    pub chord_index: usize,
    pub scheduled_us: u64,
    pub down_mask: u16,
    pub source_action_index: u32,
    pub reason_id: ReasonId,
    pub intents: SmallVec<[CompiledDownIntent; MAX_KEYS]>,
}

/// Minimal provenance for an authored Up that did not close a Down generation.
/// Such an Up is retained for diagnostics and never becomes physical work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnmatchedUpMetadata {
    pub source_action_index: u32,
    pub scheduled_us: u64,
    pub reason_id: ReasonId,
    pub key_slot: KeySlot,
    pub scan_code: u16,
}

/// Capacity-based storage report for the canonical compiled schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeScheduleMemoryMetrics {
    pub down_chords_capacity: usize,
    pub unmatched_ups_capacity: usize,
    pub reason_table_capacity: usize,
    pub reason_text_capacity_bytes: usize,
    pub heap_payload_bytes: usize,
    pub struct_bytes: usize,
    pub compile_memory: CompileMemoryMetrics,
}

#[derive(Debug, Clone)]
pub struct RuntimeSchedule {
    pub down_chords: Vec<CompiledDownChord>,
    pub unmatched_ups: Vec<UnmatchedUpMetadata>,
    pub generation_count: u64,
    pub authored_timestamp_group_count: usize,
    pub authored_action_count: usize,
    pub key_registry: KeyRegistry,
    pub reason_table: Vec<String>,
    pub schedule_end_offset_us: u64,
    /// Compile-phase capacity accounting carried into startup admission.
    pub compile_memory: CompileMemoryMetrics,
}

/// Calculated capacity for overlapping compilation allocations. This is not
/// a complete application/process peak and excludes allocator metadata and
/// unrelated caller allocations not supplied to the compiler.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompileMemoryMetrics {
    pub caller_retained_capacity_bytes: usize,
    pub input_capacity_bytes: usize,
    pub temporary_capacity_bytes: usize,
    pub schedule_capacity_bytes: usize,
    pub phase_peak_capacity_bytes: usize,
}

impl RuntimeSchedule {
    /// Calculate retained vector capacities without cancellation checkpoints.
    pub fn memory_metrics(&self) -> Result<RuntimeScheduleMemoryMetrics, String> {
        self.memory_metrics_with_checkpoint(|| Ok(()))
    }

    /// Calculate retained vector capacities while checking cancellation during
    /// the only element-wise part of accounting, the reason-string table walk.
    pub fn memory_metrics_with_checkpoint(
        &self,
        mut checkpoint: impl FnMut() -> Result<(), String>,
    ) -> Result<RuntimeScheduleMemoryMetrics, String> {
        let down_chords = self
            .down_chords
            .capacity()
            .checked_mul(std::mem::size_of::<CompiledDownChord>())
            .ok_or_else(|| "schedule Down chord capacity size overflow".to_string())?;
        let unmatched_ups = self
            .unmatched_ups
            .capacity()
            .checked_mul(std::mem::size_of::<UnmatchedUpMetadata>())
            .ok_or_else(|| "schedule unmatched Up capacity size overflow".to_string())?;
        let reason_headers = self
            .reason_table
            .capacity()
            .checked_mul(std::mem::size_of::<String>())
            .ok_or_else(|| "schedule reason table capacity size overflow".to_string())?;
        let mut reason_text_capacity_bytes = 0usize;
        for (index, reason) in self.reason_table.iter().enumerate() {
            if index % 4_096 == 0 {
                checkpoint()?;
            }
            reason_text_capacity_bytes = reason_text_capacity_bytes
                .checked_add(reason.capacity())
                .ok_or_else(|| "schedule reason text capacity size overflow".to_string())?;
        }
        let heap_payload_bytes = [
            down_chords,
            unmatched_ups,
            reason_headers,
            reason_text_capacity_bytes,
        ]
        .into_iter()
        .try_fold(0usize, |total, value| {
            total
                .checked_add(value)
                .ok_or_else(|| "schedule heap payload size overflow".to_string())
        })?;
        Ok(RuntimeScheduleMemoryMetrics {
            down_chords_capacity: self.down_chords.capacity(),
            unmatched_ups_capacity: self.unmatched_ups.capacity(),
            reason_table_capacity: self.reason_table.capacity(),
            reason_text_capacity_bytes,
            heap_payload_bytes,
            struct_bytes: std::mem::size_of::<Self>(),
            compile_memory: self.compile_memory,
        })
    }

    #[inline]
    pub fn down_chords(&self) -> &[CompiledDownChord] {
        &self.down_chords
    }

    #[inline]
    pub fn schedule_end_offset_us(&self) -> u64 {
        self.schedule_end_offset_us
    }
}

use smallvec::SmallVec as InputSmallVec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyActionInput {
    pub source_action_index: u32,
    pub kind: ActionKind,
    pub scheduled_us: u64,
    pub scan_codes: InputSmallVec<[u16; 4]>,
    pub reason: Arc<str>,
}
