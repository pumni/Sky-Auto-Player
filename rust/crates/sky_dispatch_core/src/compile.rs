//! Generation compiler for turning authored KeyAction sequences into RuntimeSchedule.

use std::collections::HashMap;

use thiserror::Error;

use crate::model::*;

pub const MAX_ACTIONS: usize = 1_000_000;
pub const MAX_REASON_BYTES: usize = 128;
const MAX_REASON_TABLE_ENTRIES: usize = u16::MAX as usize + 1;

fn checked_capacity_bytes(count: usize, element_size: usize) -> Result<usize, CompileError> {
    count
        .checked_mul(element_size)
        .ok_or_else(|| CompileError::MemoryBudgetExceeded("capacity size overflow".into()))
}

fn checked_arc_str_capacity_bytes(length: usize) -> Result<usize, CompileError> {
    let alignment = std::mem::align_of::<usize>();
    let bytes = length
        .checked_add(2 * std::mem::size_of::<usize>())
        .and_then(|bytes| bytes.checked_add(alignment - 1))
        .ok_or_else(|| {
            CompileError::MemoryBudgetExceeded("compiler reason allocation size overflow".into())
        })?;
    Ok(bytes / alignment * alignment)
}

/// Conservative payload bound for the standard HashMap table allocation.
/// HashMap capacity is the maximum entry count before growth; bucket storage
/// also includes one control byte per bucket and a control-group tail.
fn hash_map_table_capacity_bytes(
    capacity: usize,
    entry_size: usize,
) -> Result<usize, CompileError> {
    if capacity == 0 {
        return Ok(0);
    }
    let bucket_count = capacity
        .checked_mul(8)
        .and_then(|value| value.checked_add(6))
        .map(|value| value / 7)
        .and_then(usize::checked_next_power_of_two)
        .ok_or_else(|| {
            CompileError::MemoryBudgetExceeded("reason map table capacity overflow".into())
        })?;
    bucket_count
        .checked_mul(entry_size.saturating_add(1))
        .and_then(|bytes| bytes.checked_add(16 + std::mem::align_of::<usize>() - 1))
        .ok_or_else(|| CompileError::MemoryBudgetExceeded("reason map table bytes overflow".into()))
}

fn hash_map_table_growth_peak_extra_bytes(
    capacity: usize,
    entry_size: usize,
) -> Result<usize, CompileError> {
    // HashMap growth may keep the old bucket array alive while it allocates
    // and populates the new one. A previous table is never larger than the
    // final table, so one additional final-table bound covers that overlap.
    hash_map_table_capacity_bytes(capacity, entry_size)
}

fn check_memory_budget(required: usize, budget: usize) -> Result<(), CompileError> {
    if required > budget {
        return Err(CompileError::MemoryBudgetExceeded(format!(
            "calculated dispatch storage ({required} bytes) exceeds budget of {budget} bytes"
        )));
    }
    Ok(())
}

fn checked_sum(values: impl IntoIterator<Item = usize>) -> Result<usize, CompileError> {
    values.into_iter().try_fold(0usize, |total, value| {
        total.checked_add(value).ok_or_else(|| {
            CompileError::MemoryBudgetExceeded("combined compiler storage size overflow".into())
        })
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CompileError {
    #[error("runtime compilation cancelled")]
    Cancelled,
    #[error("allowed_scan_codes must contain between 1 and {MAX_KEYS} entries")]
    InvalidAllowedScanCodeCount,
    #[error("allowed_scan_codes contains duplicate scan code {0}")]
    DuplicateAllowedScanCode(u16),
    #[error("action count exceeds the configured cap of {MAX_ACTIONS}")]
    TooManyActions,
    #[error(
        "source_action_index must be strictly increasing: previous={previous}, current={current}"
    )]
    NonMonotonicSourceIndex { previous: u32, current: u32 },
    #[error("scheduled_us must be nondecreasing: previous={previous}, current={current}")]
    NonMonotonicSchedule { previous: u64, current: u64 },
    #[error(
        "multiple same-timestamp down actions are not one atomic chord; merge them before dispatch: timestamp={scheduled_us}"
    )]
    SameTimestampDownBatch { scheduled_us: u64 },
    #[error(
        "duplicate same-timestamp up action for scan code {scan_code}: timestamp={scheduled_us}"
    )]
    DuplicateSameTimestampUp { scan_code: u16, scheduled_us: u64 },
    #[error("too many authored batches at timestamp {scheduled_us} for packet metadata")]
    TooManyBatchesAtTimestamp { scheduled_us: u64 },
    #[error("action {action_index} must contain between 1 and {MAX_KEYS} scan codes")]
    InvalidBatchSize { action_index: u32 },
    #[error("action {action_index} contains duplicate scan code {scan_code}")]
    DuplicateBatchScanCode { action_index: u32, scan_code: u16 },
    #[error("action {action_index} uses scan code {scan_code} outside the prepared allowlist")]
    ScanCodeNotAllowed { action_index: u32, scan_code: u16 },
    #[error("action {action_index} reason exceeds {MAX_REASON_BYTES} UTF-8 bytes")]
    ReasonTooLong { action_index: u32 },
    #[error("too many unique reasons for the u16 reason table")]
    TooManyReasons,
    #[error("generation identifier overflow")]
    GenerationOverflow,
    #[error(
        "unclosed musical generation for scan code {scan_code}: down action {down_source_action_index} at {down_scheduled_us}us"
    )]
    UnclosedGeneration {
        scan_code: u16,
        down_source_action_index: u32,
        down_scheduled_us: u64,
    },
    #[error(
        "overlapping same-key down actions on scan code {scan_code}: first down at index {first_down_action_index} (scheduled_us={first_scheduled_us}), second down at index {second_down_action_index} (scheduled_us={second_scheduled_us})"
    )]
    OverlappingSameKeyDown {
        scan_code: u16,
        first_down_action_index: u32,
        second_down_action_index: u32,
        first_scheduled_us: u64,
        second_scheduled_us: u64,
    },
    #[error(
        "matched same-timestamp Up+Down for scan code {scan_code}: up action {up_source_action_index}, down action {down_source_action_index}, timestamp={scheduled_us}"
    )]
    MatchedSameTimestampUpDown {
        scan_code: u16,
        scheduled_us: u64,
        up_source_action_index: u32,
        down_source_action_index: u32,
    },
    #[error("runtime simulation failed: {0}")]
    Simulation(String),
    #[error("memory budget exceeded or allocation failed: {0}")]
    MemoryBudgetExceeded(String),
}

pub fn compile_runtime_intents(
    actions: &[KeyActionInput],
    allowed_scan_codes: &[u16],
) -> Result<RuntimeSchedule, CompileError> {
    compile_runtime_intents_with_cancellation(actions, allowed_scan_codes, || false)
}

pub fn compile_runtime_intents_with_cancellation(
    actions: &[KeyActionInput],
    allowed_scan_codes: &[u16],
    cancelled: impl FnMut() -> bool,
) -> Result<RuntimeSchedule, CompileError> {
    compile_runtime_intents_with_memory_metrics(
        actions,
        allowed_scan_codes,
        actions.len(),
        0,
        cancelled,
    )
}

/// Compile while accounting for the caller's actual input-vector capacity and
/// other caller-owned schedule capacity that remains live during compilation.
/// The returned peak is a calculated capacity bound for this phase, not RSS.
pub fn compile_runtime_intents_with_memory_metrics(
    actions: &[KeyActionInput],
    allowed_scan_codes: &[u16],
    action_vector_capacity: usize,
    caller_retained_bytes: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<RuntimeSchedule, CompileError> {
    if cancelled() {
        return Err(CompileError::Cancelled);
    }
    if allowed_scan_codes.is_empty() || allowed_scan_codes.len() > MAX_KEYS {
        return Err(CompileError::InvalidAllowedScanCodeCount);
    }
    for (index, &scan_code) in allowed_scan_codes.iter().enumerate() {
        if allowed_scan_codes[..index].contains(&scan_code) {
            return Err(CompileError::DuplicateAllowedScanCode(scan_code));
        }
    }
    if actions.len() > MAX_ACTIONS {
        return Err(CompileError::TooManyActions);
    }
    if action_vector_capacity < actions.len() {
        return Err(CompileError::MemoryBudgetExceeded(
            "compiler input capacity is smaller than the action slice".into(),
        ));
    }

    let key_registry = KeyRegistry::new(allowed_scan_codes);
    let mut next_generation_id: GenerationId = 0;

    #[derive(Clone, Copy, Debug)]
    struct OpenGeneration {
        generation_id: GenerationId,
        down_action_index: u32,
        down_scheduled_us: u64,
        down_chord_index: usize,
        down_intent_index: usize,
    }
    let mut open_generation_by_slot: [Option<OpenGeneration>; MAX_KEYS] = [None; MAX_KEYS];

    let mut reason_table: Vec<String> = Vec::new();
    let mut reason_map: HashMap<String, ReasonId> = HashMap::new();
    let mut intent_capacity = 0usize;
    let mut down_action_count = 0usize;
    let mut reason_input_bytes = 0usize;
    let mut scan_code_input_bytes = 0usize;
    let mut current_group_len = 0usize;
    let mut max_group_len = 0usize;
    let mut previous_group_time = None;
    for (index, action) in actions.iter().enumerate() {
        if index.is_multiple_of(256) && cancelled() {
            return Err(CompileError::Cancelled);
        }
        intent_capacity = intent_capacity
            .checked_add(action.scan_codes.len())
            .ok_or(CompileError::TooManyActions)?;
        down_action_count = down_action_count
            .checked_add(usize::from(action.kind == ActionKind::Down))
            .ok_or(CompileError::TooManyActions)?;
        reason_input_bytes = reason_input_bytes
            .checked_add(checked_arc_str_capacity_bytes(action.reason.len())?)
            .ok_or(CompileError::TooManyActions)?;
        if previous_group_time == Some(action.scheduled_us) {
            current_group_len = current_group_len
                .checked_add(1)
                .ok_or(CompileError::TooManyActions)?;
        } else {
            current_group_len = 1;
            previous_group_time = Some(action.scheduled_us);
        }
        max_group_len = max_group_len.max(current_group_len);
        if action.scan_codes.spilled() {
            scan_code_input_bytes = scan_code_input_bytes
                .checked_add(checked_capacity_bytes(
                    action.scan_codes.capacity(),
                    std::mem::size_of::<u16>(),
                )?)
                .ok_or_else(|| {
                    CompileError::MemoryBudgetExceeded(
                        "compiler scan-code storage size overflow".into(),
                    )
                })?;
        }
    }
    let max_reason_entries = actions.len().min(MAX_REASON_TABLE_ENTRIES);
    // Include both HashMap tables during a bucket-array growth and both text
    // copies (schedule reason + map key) at the per-reason maximum.
    let reason_map_growth_bound = checked_sum([
        checked_capacity_bytes(
            hash_map_table_capacity_bytes(
                max_reason_entries,
                std::mem::size_of::<(String, ReasonId)>(),
            )?,
            2,
        )?,
        checked_capacity_bytes(max_reason_entries, MAX_REASON_BYTES.saturating_mul(2))?,
    ])?;
    // Vec growth for the reason table can likewise overlap its previous
    // header buffer while the final schedule buffer is allocated.
    let reason_table_growth_buffer_bound =
        checked_capacity_bytes(max_reason_entries, std::mem::size_of::<String>())?;
    let compiler_input_bytes = checked_sum([
        checked_capacity_bytes(
            action_vector_capacity,
            std::mem::size_of::<KeyActionInput>(),
        )?,
        reason_input_bytes,
        scan_code_input_bytes,
    ])?;
    let max_group_batch_scratch_bytes =
        checked_capacity_bytes(max_group_len, std::mem::size_of::<CompiledBatch>())?;
    let compile_preflight_bytes = checked_sum([
        caller_retained_bytes,
        compiler_input_bytes,
        checked_capacity_bytes(actions.len(), std::mem::size_of::<CompiledBatch>())?,
        checked_capacity_bytes(actions.len(), std::mem::size_of::<CompiledPacket>())?,
        checked_capacity_bytes(intent_capacity, std::mem::size_of::<CompactIntent>())?,
        checked_capacity_bytes(down_action_count, std::mem::size_of::<CompiledDownChord>())?,
        checked_capacity_bytes(max_reason_entries, std::mem::size_of::<String>())?,
        reason_table_growth_buffer_bound,
        reason_map_growth_bound,
        max_group_batch_scratch_bytes,
        std::mem::size_of::<RuntimeSchedule>(),
    ])?;
    check_memory_budget(compile_preflight_bytes, MAX_RETAINED_DISPATCH_BYTES)?;

    let mut batches = Vec::new();
    reserve_exact(&mut batches, actions.len(), "batches")?;
    let mut intents = Vec::new();
    reserve_exact(&mut intents, intent_capacity, "intents")?;
    let mut packets = Vec::new();
    reserve_exact(&mut packets, actions.len(), "packets")?;
    let mut down_chords: Vec<CompiledDownChord> = Vec::new();
    reserve_exact(&mut down_chords, down_action_count, "down_chords")?;

    let mut get_or_insert_reason = |reason: &str| -> Result<ReasonId, CompileError> {
        if let Some(&id) = reason_map.get(reason) {
            Ok(id)
        } else {
            let id =
                ReasonId::try_from(reason_table.len()).map_err(|_| CompileError::TooManyReasons)?;
            reason_table.try_reserve(1).map_err(|e| {
                CompileError::MemoryBudgetExceeded(format!("reason table reservation: {e}"))
            })?;
            reason_map.try_reserve(1).map_err(|e| {
                CompileError::MemoryBudgetExceeded(format!("reason map reservation: {e}"))
            })?;
            let mut table_reason = String::new();
            table_reason.try_reserve(reason.len()).map_err(|e| {
                CompileError::MemoryBudgetExceeded(format!("reason string reservation: {e}"))
            })?;
            table_reason.push_str(reason);
            let mut map_reason = String::new();
            map_reason.try_reserve(reason.len()).map_err(|e| {
                CompileError::MemoryBudgetExceeded(format!("reason key reservation: {e}"))
            })?;
            map_reason.push_str(reason);
            reason_table.push(table_reason);
            reason_map.insert(map_reason, id);
            Ok(id)
        }
    };

    let mut previous_source_index: Option<u32> = None;
    let mut previous_scheduled_us: Option<u64> = None;
    let mut group_start = 0usize;
    let mut max_group_batch_capacity = 0usize;

    while group_start < actions.len() {
        if cancelled() {
            return Err(CompileError::Cancelled);
        }
        let scheduled_us = actions[group_start].scheduled_us;
        let mut group_end = group_start + 1;
        while group_end < actions.len() && actions[group_end].scheduled_us == scheduled_us {
            if (group_end - group_start).is_multiple_of(256) && cancelled() {
                return Err(CompileError::Cancelled);
            }
            group_end += 1;
        }

        let packet_id =
            PacketId::try_from(packets.len()).map_err(|_| CompileError::TooManyActions)?;
        let first_batch_index =
            u32::try_from(batches.len()).map_err(|_| CompileError::TooManyActions)?;
        let mut down_action: Option<usize> = None;
        for (offset, action) in actions[group_start..group_end].iter().enumerate() {
            if offset.is_multiple_of(256) && cancelled() {
                return Err(CompileError::Cancelled);
            }
            if let Some(previous) = previous_source_index
                && action.source_action_index <= previous
            {
                return Err(CompileError::NonMonotonicSourceIndex {
                    previous,
                    current: action.source_action_index,
                });
            }
            if let Some(previous) = previous_scheduled_us
                && action.scheduled_us < previous
            {
                return Err(CompileError::NonMonotonicSchedule {
                    previous,
                    current: action.scheduled_us,
                });
            }
            if action.scan_codes.is_empty() || action.scan_codes.len() > MAX_KEYS {
                return Err(CompileError::InvalidBatchSize {
                    action_index: action.source_action_index,
                });
            }
            if action.reason.len() > MAX_REASON_BYTES {
                return Err(CompileError::ReasonTooLong {
                    action_index: action.source_action_index,
                });
            }
            let mut batch_seen_mask: u16 = 0;
            for &scan_code in &action.scan_codes {
                let Some(key_slot) = key_registry.slot_for(scan_code) else {
                    return Err(CompileError::ScanCodeNotAllowed {
                        action_index: action.source_action_index,
                        scan_code,
                    });
                };
                let bit = 1u16 << key_slot;
                if batch_seen_mask & bit != 0 {
                    return Err(CompileError::DuplicateBatchScanCode {
                        action_index: action.source_action_index,
                        scan_code,
                    });
                }
                batch_seen_mask |= bit;
            }
            get_or_insert_reason(&action.reason)?;
            if action.kind == ActionKind::Down {
                if down_action.is_some() {
                    return Err(CompileError::SameTimestampDownBatch { scheduled_us });
                }
                down_action = Some(group_start + offset);
            }
            previous_source_index = Some(action.source_action_index);
            previous_scheduled_us = Some(action.scheduled_us);
        }

        // Build all release intents first. This is the compiler's canonical
        // physical order even when authored actions arrived Down-before-Up.
        let up_intent_start =
            u32::try_from(intents.len()).map_err(|_| CompileError::TooManyActions)?;
        let mut up_mask = 0u16;
        let mut seen_up_mask = 0u16;
        let mut physical_up_source_action_index_by_slot: [Option<u32>; MAX_KEYS] = [None; MAX_KEYS];
        let mut group_batches: Vec<CompiledBatch> = Vec::new();
        reserve_exact(&mut group_batches, group_end - group_start, "group batches")?;
        max_group_batch_capacity = max_group_batch_capacity.max(group_batches.capacity());
        for (offset, action) in actions[group_start..group_end].iter().enumerate() {
            if offset.is_multiple_of(256) && cancelled() {
                return Err(CompileError::Cancelled);
            }
            if action.kind != ActionKind::Up {
                continue;
            }
            let reason_id = get_or_insert_reason(&action.reason)?;
            let intent_start =
                u32::try_from(intents.len()).map_err(|_| CompileError::TooManyActions)?;
            let mut sorted_scan_codes = smallvec::SmallVec::<[u16; MAX_KEYS]>::new();
            sorted_scan_codes.extend_from_slice(&action.scan_codes);
            sorted_scan_codes.sort_by_key(|&sc| key_registry.slot_for(sc).unwrap_or(0xFF));
            for &scan_code in &sorted_scan_codes {
                let key_slot = key_registry
                    .slot_for(scan_code)
                    .expect("allowlist validation must precede packet compilation");
                let bit = 1u16 << key_slot;
                if seen_up_mask & bit != 0 {
                    return Err(CompileError::DuplicateSameTimestampUp {
                        scan_code,
                        scheduled_us,
                    });
                }
                seen_up_mask |= bit;
                let open = open_generation_by_slot[key_slot as usize].take();
                let generation_id = open.map(|g| g.generation_id);
                if let Some(open) = open {
                    up_mask |= bit;
                    physical_up_source_action_index_by_slot[key_slot as usize] =
                        Some(action.source_action_index);
                    down_chords[open.down_chord_index].intents[open.down_intent_index]
                        .paired_up_offset_us = scheduled_us;
                    down_chords[open.down_chord_index].intents[open.down_intent_index]
                        .paired_up_source_action_index = action.source_action_index;
                }
                intents.push(CompactIntent::new(
                    generation_id.unwrap_or(NO_GENERATION_ID),
                    key_slot,
                ));
            }
            group_batches.push(CompiledBatch {
                source_action_index: action.source_action_index,
                kind: ActionKind::Up,
                scheduled_us,
                reason_id,
                intent_start,
                intent_len: u8::try_from(action.scan_codes.len())
                    .expect("validated batch length is at most MAX_KEYS"),
                packet_id,
            });
        }
        let up_intent_len = u8::try_from(
            intents.len() - usize::try_from(up_intent_start).expect("u32 range is usize-safe"),
        )
        .expect("one timestamp has at most fifteen release intents");

        let down_intent_start =
            u32::try_from(intents.len()).map_err(|_| CompileError::TooManyActions)?;
        let mut down_mask = 0u16;
        let mut down_source_action_index = None;
        if let Some(down_index) = down_action {
            let action = &actions[down_index];
            let reason_id = get_or_insert_reason(&action.reason)?;
            let intent_start =
                u32::try_from(intents.len()).map_err(|_| CompileError::TooManyActions)?;
            let mut sorted_scan_codes = smallvec::SmallVec::<[u16; MAX_KEYS]>::new();
            sorted_scan_codes.extend_from_slice(&action.scan_codes);
            sorted_scan_codes.sort_by_key(|&sc| key_registry.slot_for(sc).unwrap_or(0xFF));
            let chord_index = down_chords.len();
            let mut chord_intents: smallvec::SmallVec<[CompiledDownIntent; MAX_KEYS]> =
                smallvec::SmallVec::new();
            for &scan_code in &sorted_scan_codes {
                let key_slot = key_registry
                    .slot_for(scan_code)
                    .expect("allowlist validation must precede packet compilation");
                let bit = 1u16 << key_slot;
                if let Some(up_source_action_index) =
                    physical_up_source_action_index_by_slot[key_slot as usize]
                {
                    return Err(CompileError::MatchedSameTimestampUpDown {
                        scan_code,
                        scheduled_us,
                        up_source_action_index,
                        down_source_action_index: action.source_action_index,
                    });
                }
                if let Some(open) = open_generation_by_slot[key_slot as usize] {
                    return Err(CompileError::OverlappingSameKeyDown {
                        scan_code,
                        first_down_action_index: open.down_action_index,
                        second_down_action_index: action.source_action_index,
                        first_scheduled_us: open.down_scheduled_us,
                        second_scheduled_us: scheduled_us,
                    });
                }
                if next_generation_id > MAX_COMPACT_GENERATION_ID {
                    return Err(CompileError::GenerationOverflow);
                }
                let generation_id = next_generation_id;
                next_generation_id = next_generation_id
                    .checked_add(1)
                    .ok_or(CompileError::GenerationOverflow)?;
                let down_intent_index = chord_intents.len();
                open_generation_by_slot[key_slot as usize] = Some(OpenGeneration {
                    generation_id,
                    down_action_index: action.source_action_index,
                    down_scheduled_us: scheduled_us,
                    down_chord_index: chord_index,
                    down_intent_index,
                });
                down_mask |= bit;
                intents.push(CompactIntent::new(generation_id, key_slot));
                chord_intents.push(CompiledDownIntent {
                    generation_id,
                    key_slot,
                    scan_code,
                    source_action_index: action.source_action_index,
                    reason_id,
                    paired_up_offset_us: 0,
                    paired_up_source_action_index: 0,
                });
            }
            down_chords.push(CompiledDownChord {
                chord_index,
                scheduled_us,
                down_mask,
                source_action_index: action.source_action_index,
                reason_id,
                intents: chord_intents,
            });
            group_batches.push(CompiledBatch {
                source_action_index: action.source_action_index,
                kind: ActionKind::Down,
                scheduled_us,
                reason_id,
                intent_start,
                intent_len: u8::try_from(action.scan_codes.len())
                    .expect("validated batch length is at most MAX_KEYS"),
                packet_id,
            });
            down_source_action_index = Some(action.source_action_index);
        }
        group_batches.sort_unstable_by_key(|batch| batch.source_action_index);
        batches.extend(group_batches);
        debug_assert_eq!(
            up_mask & down_mask,
            0,
            "compiled physical packet direction masks must be disjoint"
        );
        let batch_count = u16::try_from(group_end - group_start)
            .map_err(|_| CompileError::TooManyBatchesAtTimestamp { scheduled_us })?;
        packets.push(CompiledPacket {
            packet_id,
            scheduled_us,
            first_batch_index,
            batch_count,
            up_mask,
            down_mask,
            up_intent_start,
            up_intent_len,
            down_intent_start,
            down_intent_len: u8::try_from(
                intents.len()
                    - usize::try_from(down_intent_start).expect("u32 range is usize-safe"),
            )
            .expect("one timestamp has at most fifteen activation intents"),
            down_source_action_index,
        });
        group_start = group_end;
    }

    for (slot, open) in open_generation_by_slot.iter().enumerate() {
        if let Some(open) = open {
            let scan_code = key_registry
                .scan_code_for(slot as KeySlot)
                .expect("open generation slot must be present in the key registry");
            return Err(CompileError::UnclosedGeneration {
                scan_code,
                down_source_action_index: open.down_action_index,
                down_scheduled_us: open.down_scheduled_us,
            });
        }
    }

    let schedule_end_offset_us = actions
        .iter()
        .map(|action| action.scheduled_us)
        .max()
        .unwrap_or(0);

    let mut schedule = RuntimeSchedule {
        packets,
        batches,
        intents,
        generation_count: next_generation_id,
        key_registry,
        reason_table,
        down_chords,
        schedule_end_offset_us,
        compile_memory: CompileMemoryMetrics::default(),
    };
    let schedule_metrics = schedule.memory_metrics().map_err(|error| {
        CompileError::MemoryBudgetExceeded(format!("schedule memory accounting: {error}"))
    })?;
    let reason_map_bytes = checked_sum([
        hash_map_table_capacity_bytes(
            reason_map.capacity(),
            std::mem::size_of::<(String, ReasonId)>(),
        )?,
        reason_map.iter().try_fold(0usize, |total, (key, _)| {
            let key_bytes = key.capacity();
            total.checked_add(key_bytes).ok_or_else(|| {
                CompileError::MemoryBudgetExceeded("reason map key capacity overflow".into())
            })
        })?,
    ])?;
    let reason_map_growth_extra_bytes = hash_map_table_growth_peak_extra_bytes(
        reason_map.capacity(),
        std::mem::size_of::<(String, ReasonId)>(),
    )?;
    let reason_table_growth_extra_bytes = checked_capacity_bytes(
        schedule.reason_table.capacity(),
        std::mem::size_of::<String>(),
    )?;
    let group_scratch_bytes = checked_capacity_bytes(
        max_group_batch_capacity,
        std::mem::size_of::<CompiledBatch>(),
    )?;
    let temporary_capacity_bytes = checked_sum([
        reason_map_bytes,
        reason_map_growth_extra_bytes,
        reason_table_growth_extra_bytes,
        group_scratch_bytes,
    ])?;
    let schedule_capacity_bytes = checked_sum([
        schedule_metrics.heap_payload_bytes,
        schedule_metrics.struct_bytes,
    ])?;
    let phase_peak_capacity_bytes = checked_sum([
        caller_retained_bytes,
        compiler_input_bytes,
        schedule_capacity_bytes,
        temporary_capacity_bytes,
    ])?;
    check_memory_budget(phase_peak_capacity_bytes, MAX_RETAINED_DISPATCH_BYTES)?;
    let metrics = CompileMemoryMetrics {
        caller_retained_capacity_bytes: caller_retained_bytes,
        input_capacity_bytes: compiler_input_bytes,
        temporary_capacity_bytes,
        schedule_capacity_bytes,
        phase_peak_capacity_bytes,
    };
    schedule.compile_memory = metrics;
    Ok(schedule)
}

fn reserve_exact<T>(
    values: &mut Vec<T>,
    additional: usize,
    label: &str,
) -> Result<(), CompileError> {
    values.try_reserve_exact(additional).map_err(|error| {
        CompileError::MemoryBudgetExceeded(format!("{label} reservation failed: {error}"))
    })
}

#[cfg(test)]
mod tests;
