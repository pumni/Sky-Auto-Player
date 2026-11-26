//! Canonical onset-only chord plan builder and admission validator.
//!
//! Validates allowed key profiles, strictly increasing onset timestamps,
//! non-empty 15-slot chord masks, same-key repeat intervals against `H`,
//! checked microsecond end-offset arithmetic, and bounded memory budgets.

use thiserror::Error;

use crate::model::{
    CompileMemoryMetrics, KeyRegistry, MAX_KEYS, MAX_RETAINED_DISPATCH_BYTES, OnsetChord,
    OnsetChordInput, OnsetPlan, generation_id_for,
};

pub const MAX_CHORDS: usize = 1_000_000;
const BUILD_CHECKPOINT_INTERVAL: usize = 256;

fn checked_capacity_bytes(count: usize, element_size: usize) -> Result<usize, PlanError> {
    count
        .checked_mul(element_size)
        .ok_or_else(|| PlanError::MemoryBudgetExceeded("capacity size overflow".into()))
}

fn checked_sum(values: impl IntoIterator<Item = usize>) -> Result<usize, PlanError> {
    values.into_iter().try_fold(0usize, |total, value| {
        total.checked_add(value).ok_or_else(|| {
            PlanError::MemoryBudgetExceeded("combined plan storage size overflow".into())
        })
    })
}

fn check_memory_budget(required: usize, budget: usize) -> Result<(), PlanError> {
    if required > budget {
        return Err(PlanError::MemoryBudgetExceeded(format!(
            "calculated dispatch storage ({required} bytes) exceeds budget of {budget} bytes"
        )));
    }
    Ok(())
}

fn reserve_exact<T>(values: &mut Vec<T>, additional: usize, label: &str) -> Result<(), PlanError> {
    values.try_reserve_exact(additional).map_err(|error| {
        PlanError::MemoryBudgetExceeded(format!("{label} reservation failed: {error}"))
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanError {
    #[error("onset plan construction cancelled")]
    Cancelled,
    #[error("allowed_scan_codes must contain between 1 and {MAX_KEYS} entries")]
    InvalidAllowedScanCodeCount,
    #[error("allowed_scan_codes contains duplicate scan code {0}")]
    DuplicateAllowedScanCode(u16),
    #[error("chord count exceeds the configured cap of {MAX_CHORDS}")]
    TooManyChords,
    #[error(
        "source_action_index must be strictly increasing: previous={previous}, current={current}"
    )]
    NonMonotonicSourceIndex { previous: u32, current: u32 },
    #[error("scheduled_us must be strictly increasing: previous={previous}, current={current}")]
    NonMonotonicSchedule { previous: u64, current: u64 },
    #[error(
        "multiple same-timestamp onset chords are not one atomic chord; merge them before dispatch: timestamp={scheduled_us}"
    )]
    DuplicateOnsetTimestamp { scheduled_us: u64 },
    #[error("chord {action_index} must contain between 1 and {MAX_KEYS} scan codes")]
    InvalidChordSize { action_index: u32 },
    #[error("chord {action_index} contains duplicate scan code {scan_code}")]
    DuplicateChordScanCode { action_index: u32, scan_code: u16 },
    #[error("chord {action_index} uses scan code {scan_code} outside the prepared allowlist")]
    ScanCodeNotAllowed { action_index: u32, scan_code: u16 },
    #[error("generation identifier overflow")]
    GenerationOverflow,
    #[error("schedule and timing configuration exceed supported timestamp range")]
    TimestampOverflow,
    #[error(
        "same-key repeat interval too short for scan code {scan_code}: first chord {first_source_action_index} at {first_scheduled_us}us, second chord {second_source_action_index} at {second_scheduled_us}us, interval={interval_us}us, required={required_min_hold_us}us"
    )]
    SameKeyIntervalTooShort {
        scan_code: u16,
        first_source_action_index: u32,
        second_source_action_index: u32,
        first_scheduled_us: u64,
        second_scheduled_us: u64,
        interval_us: u64,
        required_min_hold_us: u64,
    },
    #[error("dispatch test scenario failed: {0}")]
    Simulation(String),
    #[error("memory budget exceeded or allocation failed: {0}")]
    MemoryBudgetExceeded(String),
}

pub fn build_onset_plan(
    chords: &[OnsetChordInput],
    allowed_scan_codes: &[u16],
    hold_us: u64,
) -> Result<OnsetPlan, PlanError> {
    build_onset_plan_with_cancellation(chords, allowed_scan_codes, hold_us, || false)
}

pub fn build_onset_plan_with_cancellation(
    chords: &[OnsetChordInput],
    allowed_scan_codes: &[u16],
    hold_us: u64,
    cancelled: impl FnMut() -> bool,
) -> Result<OnsetPlan, PlanError> {
    build_onset_plan_with_memory_metrics(
        chords,
        allowed_scan_codes,
        hold_us,
        chords.len(),
        0,
        cancelled,
    )
}

/// Build and validate a canonical onset-only plan while accounting for input
/// and output storage that remains live together. Reported values are capacity
/// bounds, not process RSS.
pub fn build_onset_plan_with_memory_metrics(
    input_chords: &[OnsetChordInput],
    allowed_scan_codes: &[u16],
    hold_us: u64,
    input_vector_capacity: usize,
    caller_retained_bytes: usize,
    mut cancelled: impl FnMut() -> bool,
) -> Result<OnsetPlan, PlanError> {
    if cancelled() {
        return Err(PlanError::Cancelled);
    }
    if allowed_scan_codes.is_empty() || allowed_scan_codes.len() > MAX_KEYS {
        return Err(PlanError::InvalidAllowedScanCodeCount);
    }
    for (index, &scan_code) in allowed_scan_codes.iter().enumerate() {
        if allowed_scan_codes[..index].contains(&scan_code) {
            return Err(PlanError::DuplicateAllowedScanCode(scan_code));
        }
    }
    if input_chords.len() > MAX_CHORDS {
        return Err(PlanError::TooManyChords);
    }
    if input_vector_capacity < input_chords.len() {
        return Err(PlanError::MemoryBudgetExceeded(
            "onset plan input capacity is smaller than the chord slice".into(),
        ));
    }

    let key_registry = KeyRegistry::new(allowed_scan_codes);
    let mut scan_code_input_bytes = 0usize;
    for (index, chord) in input_chords.iter().enumerate() {
        if index.is_multiple_of(BUILD_CHECKPOINT_INTERVAL) && cancelled() {
            return Err(PlanError::Cancelled);
        }
        if chord.scan_codes.spilled() {
            scan_code_input_bytes = scan_code_input_bytes
                .checked_add(checked_capacity_bytes(
                    chord.scan_codes.capacity(),
                    std::mem::size_of::<u16>(),
                )?)
                .ok_or_else(|| {
                    PlanError::MemoryBudgetExceeded(
                        "onset plan scan-code storage size overflow".into(),
                    )
                })?;
        }
    }

    let input_capacity_bytes = checked_sum([
        checked_capacity_bytes(
            input_vector_capacity,
            std::mem::size_of::<OnsetChordInput>(),
        )?,
        scan_code_input_bytes,
    ])?;
    let preflight_bytes = checked_sum([
        caller_retained_bytes,
        input_capacity_bytes,
        checked_capacity_bytes(input_chords.len(), std::mem::size_of::<OnsetChord>())?,
        std::mem::size_of::<OnsetPlan>(),
    ])?;
    check_memory_budget(preflight_bytes, MAX_RETAINED_DISPATCH_BYTES)?;

    let mut chords: Vec<OnsetChord> = Vec::new();
    reserve_exact(&mut chords, input_chords.len(), "onset_chords")?;

    let mut last_onset_by_slot: [Option<(u32, u64)>; MAX_KEYS] = [None; MAX_KEYS];
    let mut previous_source_index: Option<u32> = None;
    let mut previous_scheduled_us: Option<u64> = None;
    let mut generation_count = 0u64;

    for (chord_index, input) in input_chords.iter().enumerate() {
        if chord_index.is_multiple_of(BUILD_CHECKPOINT_INTERVAL) && cancelled() {
            return Err(PlanError::Cancelled);
        }
        if let Some(previous) = previous_source_index
            && input.source_action_index <= previous
        {
            return Err(PlanError::NonMonotonicSourceIndex {
                previous,
                current: input.source_action_index,
            });
        }
        if let Some(previous) = previous_scheduled_us {
            if input.scheduled_us < previous {
                return Err(PlanError::NonMonotonicSchedule {
                    previous,
                    current: input.scheduled_us,
                });
            }
            if input.scheduled_us == previous {
                return Err(PlanError::DuplicateOnsetTimestamp {
                    scheduled_us: input.scheduled_us,
                });
            }
        }
        if input.scan_codes.is_empty() || input.scan_codes.len() > MAX_KEYS {
            return Err(PlanError::InvalidChordSize {
                action_index: input.source_action_index,
            });
        }
        // Prove the nominal pulse end fits in u64 microseconds for every chord.
        input
            .scheduled_us
            .checked_add(hold_us)
            .ok_or(PlanError::TimestampOverflow)?;

        let mut key_mask = 0u16;
        for &scan_code in &input.scan_codes {
            let Some(slot) = key_registry.slot_for(scan_code) else {
                return Err(PlanError::ScanCodeNotAllowed {
                    action_index: input.source_action_index,
                    scan_code,
                });
            };
            let bit = 1u16 << slot;
            if key_mask & bit != 0 {
                return Err(PlanError::DuplicateChordScanCode {
                    action_index: input.source_action_index,
                    scan_code,
                });
            }
            if generation_id_for(chord_index, slot).is_none() {
                return Err(PlanError::GenerationOverflow);
            }
            key_mask |= bit;
        }

        // Check D4 same-key repeat feasibility in canonical slot order.
        let mut remaining = key_mask;
        while remaining != 0 {
            let slot = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            if let Some((prev_source_action_index, prev_scheduled_us)) = last_onset_by_slot[slot] {
                let interval_us = input
                    .scheduled_us
                    .checked_sub(prev_scheduled_us)
                    .ok_or(PlanError::TimestampOverflow)?;
                if interval_us < hold_us {
                    let scan_code = key_registry
                        .scan_code_for(slot as u8)
                        .expect("slot validated in allowlist");
                    return Err(PlanError::SameKeyIntervalTooShort {
                        scan_code,
                        first_source_action_index: prev_source_action_index,
                        second_source_action_index: input.source_action_index,
                        first_scheduled_us: prev_scheduled_us,
                        second_scheduled_us: input.scheduled_us,
                        interval_us,
                        required_min_hold_us: hold_us,
                    });
                }
            }
            last_onset_by_slot[slot] = Some((input.source_action_index, input.scheduled_us));
        }

        generation_count = generation_count
            .checked_add(u64::from(key_mask.count_ones()))
            .ok_or(PlanError::GenerationOverflow)?;
        chords.push(OnsetChord {
            chord_index,
            scheduled_us: input.scheduled_us,
            key_mask,
            source_action_index: input.source_action_index,
        });
        previous_source_index = Some(input.source_action_index);
        previous_scheduled_us = Some(input.scheduled_us);
    }

    let schedule_end_offset_us = match chords.last() {
        Some(last) => last
            .scheduled_us
            .checked_add(hold_us)
            .ok_or(PlanError::TimestampOverflow)?,
        None => 0,
    };

    let mut plan = OnsetPlan {
        chords,
        generation_count,
        key_registry,
        hold_us,
        schedule_end_offset_us,
        compile_memory: CompileMemoryMetrics::default(),
    };
    let plan_metrics = plan.memory_metrics().map_err(|error| {
        PlanError::MemoryBudgetExceeded(format!("plan memory accounting: {error}"))
    })?;
    let schedule_capacity_bytes =
        checked_sum([plan_metrics.heap_payload_bytes, plan_metrics.struct_bytes])?;
    let phase_peak_capacity_bytes = checked_sum([
        caller_retained_bytes,
        input_capacity_bytes,
        schedule_capacity_bytes,
    ])?;
    check_memory_budget(phase_peak_capacity_bytes, MAX_RETAINED_DISPATCH_BYTES)?;
    plan.compile_memory = CompileMemoryMetrics {
        caller_retained_capacity_bytes: caller_retained_bytes,
        input_capacity_bytes,
        temporary_capacity_bytes: 0,
        schedule_capacity_bytes,
        phase_peak_capacity_bytes,
    };
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smallvec::smallvec;

    fn chord(index: u32, scheduled_us: u64, scan_codes: &[u16]) -> OnsetChordInput {
        OnsetChordInput {
            source_action_index: index,
            scheduled_us,
            scan_codes: scan_codes.iter().copied().collect(),
        }
    }

    #[test]
    fn validates_same_key_repeat_boundaries_h_minus_1_h_and_h_plus_1() {
        let h = 16_667u64;
        let below = build_onset_plan(&[chord(0, 100, &[1]), chord(1, 100 + h - 1, &[1])], &[1], h);
        assert_eq!(
            below,
            Err(PlanError::SameKeyIntervalTooShort {
                scan_code: 1,
                first_source_action_index: 0,
                second_source_action_index: 1,
                first_scheduled_us: 100,
                second_scheduled_us: 100 + h - 1,
                interval_us: h - 1,
                required_min_hold_us: h,
            })
        );

        let equal = build_onset_plan(&[chord(0, 100, &[1]), chord(1, 100 + h, &[1])], &[1], h)
            .expect("exact H interval is allowed");
        assert_eq!(equal.chords.len(), 2);
        assert_eq!(equal.generation_count, 2);
        assert_eq!(equal.schedule_end_offset_us, 100 + 2 * h);

        let above = build_onset_plan(&[chord(0, 100, &[1]), chord(1, 100 + h + 1, &[1])], &[1], h)
            .expect("H+1 interval is allowed");
        assert_eq!(above.schedule_end_offset_us, 100 + 2 * h + 1);
    }

    #[test]
    fn empty_plan_has_zero_generations_and_zero_end_offset() {
        let plan = build_onset_plan(&[], &[0x15], 16_667).expect("empty plan is valid");
        assert!(plan.chords.is_empty());
        assert_eq!(plan.generation_count, 0);
        assert_eq!(plan.schedule_end_offset_us, 0);
    }

    #[test]
    fn rejects_duplicate_timestamp_invalid_scan_code_and_overflow() {
        assert_eq!(
            build_onset_plan(&[chord(0, 100, &[1]), chord(1, 100, &[2])], &[1, 2], 10),
            Err(PlanError::DuplicateOnsetTimestamp { scheduled_us: 100 })
        );
        assert_eq!(
            build_onset_plan(&[chord(0, 100, &[99])], &[1], 10),
            Err(PlanError::ScanCodeNotAllowed {
                action_index: 0,
                scan_code: 99,
            })
        );
        assert_eq!(
            build_onset_plan(
                &[OnsetChordInput {
                    source_action_index: 0,
                    scheduled_us: u64::MAX - 5,
                    scan_codes: smallvec![1],
                }],
                &[1],
                10,
            ),
            Err(PlanError::TimestampOverflow)
        );
    }

    #[test]
    fn rejects_non_monotonic_source_index_and_timestamps() {
        assert_eq!(
            build_onset_plan(&[chord(2, 100, &[1]), chord(2, 200, &[2])], &[1, 2], 10),
            Err(PlanError::NonMonotonicSourceIndex {
                previous: 2,
                current: 2,
            })
        );
        assert_eq!(
            build_onset_plan(&[chord(0, 200, &[1]), chord(1, 100, &[2])], &[1, 2], 10),
            Err(PlanError::NonMonotonicSchedule {
                previous: 200,
                current: 100,
            })
        );
    }
}
