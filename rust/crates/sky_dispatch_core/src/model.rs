//! Canonical onset-only chord plan and key registry for fixed-pulse dispatch.

use smallvec::SmallVec;

pub const MAX_KEYS: usize = 15;
/// Bitmask covering all 15 valid instrument key slots.
pub const ALL_KEYS_MASK: u16 = (1u16 << MAX_KEYS) - 1;
/// Combined capacity budget for retained schedule, plan and executor storage.
/// This is a calculated allocation bound, not a process RSS limit.
pub const MAX_RETAINED_DISPATCH_BYTES: usize = 512 * 1024 * 1024;

pub type GenerationId = u64;
pub type KeySlot = u8;

/// Derive a stable, collision-free generation identifier from `(chord_index, slot)`.
/// Returns `None` if `slot >= MAX_KEYS` or `chord_index` exceeds the packed range.
#[inline]
pub fn generation_id_for(chord_index: usize, slot: KeySlot) -> Option<GenerationId> {
    if usize::from(slot) >= MAX_KEYS {
        return None;
    }
    let shifted = u64::try_from(chord_index).ok()?.checked_shl(4)?;
    Some(shifted | u64::from(slot))
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

/// One authored musical onset group supplied to the onset-plan builder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnsetChordInput {
    pub source_action_index: u32,
    pub scheduled_us: u64,
    pub scan_codes: SmallVec<[u16; 4]>,
}

/// One validated immutable onset chord in the microsecond timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnsetChord {
    pub chord_index: usize,
    pub scheduled_us: u64,
    pub key_mask: u16,
    pub source_action_index: u32,
}

impl OnsetChord {
    #[inline]
    pub const fn key_count(self) -> usize {
        self.key_mask.count_ones() as usize
    }
}

/// Calculated capacity for overlapping plan-build allocations. This is not
/// a complete application/process peak and excludes allocator metadata and
/// unrelated caller allocations not supplied to the builder.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompileMemoryMetrics {
    pub caller_retained_capacity_bytes: usize,
    pub input_capacity_bytes: usize,
    pub temporary_capacity_bytes: usize,
    pub schedule_capacity_bytes: usize,
    pub phase_peak_capacity_bytes: usize,
}

/// Capacity-based storage report for the canonical onset plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OnsetPlanMemoryMetrics {
    pub chords_capacity: usize,
    pub heap_payload_bytes: usize,
    pub struct_bytes: usize,
    pub compile_memory: CompileMemoryMetrics,
}

/// Validated, immutable onset-only chord plan in the microsecond domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnsetPlan {
    pub chords: Vec<OnsetChord>,
    pub generation_count: u64,
    pub key_registry: KeyRegistry,
    pub hold_us: u64,
    pub schedule_end_offset_us: u64,
    /// Build-phase capacity accounting carried into QPC preparation and startup admission.
    pub compile_memory: CompileMemoryMetrics,
}

impl OnsetPlan {
    /// Calculate retained vector capacities.
    pub fn memory_metrics(&self) -> Result<OnsetPlanMemoryMetrics, String> {
        let heap_payload_bytes = self
            .chords
            .capacity()
            .checked_mul(std::mem::size_of::<OnsetChord>())
            .ok_or_else(|| "onset plan chord capacity size overflow".to_string())?;
        Ok(OnsetPlanMemoryMetrics {
            chords_capacity: self.chords.capacity(),
            heap_payload_bytes,
            struct_bytes: std::mem::size_of::<Self>(),
            compile_memory: self.compile_memory,
        })
    }

    #[inline]
    pub fn chords(&self) -> &[OnsetChord] {
        &self.chords
    }

    #[inline]
    pub fn schedule_end_offset_us(&self) -> u64 {
        self.schedule_end_offset_us
    }

    #[inline]
    pub fn hold_us(&self) -> u64 {
        self.hold_us
    }

    /// Extend the plan's schedule end offset for explicit trailing-idle tests.
    /// Rejects any offset shorter than the plan's existing end offset.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_trailing_end_offset_us(mut self, end_offset_us: u64) -> Result<Self, String> {
        if end_offset_us < self.schedule_end_offset_us {
            return Err(format!(
                "trailing end offset {end_offset_us}us precedes final pulse end {}us",
                self.schedule_end_offset_us
            ));
        }
        self.schedule_end_offset_us = end_offset_us;
        Ok(self)
    }
}
