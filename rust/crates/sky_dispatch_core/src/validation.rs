//! Pure admission validation for authored schedules.

use thiserror::Error;

use crate::model::{ActionKind, GenerationId, MAX_KEYS, NO_GENERATION_ID, RuntimeSchedule};
use crate::time::{DurationTicks, TimelineTicks};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ScheduleTimingError {
    #[error("schedule and timing configuration exceed supported timestamp range")]
    TimestampOverflow,
    #[error(
        "same-key hold too short for scan code {scan_code}: down action {down_source_action_index} at {down_scheduled_us}us ({down_scheduled_ticks} ticks), up action {up_source_action_index} at {up_scheduled_us}us ({up_scheduled_ticks} ticks), interval={interval_us}us, required={required_min_hold_us}us ({required_min_hold_ticks} ticks), earliest release={earliest_release_ticks} ticks"
    )]
    SameKeyHoldTooShort {
        scan_code: u16,
        down_source_action_index: u32,
        up_source_action_index: u32,
        down_scheduled_us: u64,
        up_scheduled_us: u64,
        interval_us: u64,
        required_min_hold_us: u64,
        down_scheduled_ticks: u64,
        up_scheduled_ticks: u64,
        required_min_hold_ticks: u64,
        earliest_release_ticks: u64,
    },
    #[error(
        "same-key release gap too short for scan code {scan_code}: previous up action {previous_up_source_action_index} at {previous_up_scheduled_us}us, next down action {next_down_source_action_index} at {next_down_scheduled_us}us, gap={release_gap_us}us, required={required_release_gap_us}us"
    )]
    SameKeyReleaseGapTooShort {
        scan_code: u16,
        previous_up_source_action_index: u32,
        next_down_source_action_index: u32,
        previous_up_scheduled_us: u64,
        next_down_scheduled_us: u64,
        release_gap_us: u64,
        required_release_gap_us: u64,
        previous_up_ticks: u64,
        next_down_ticks: u64,
        required_release_gap_ticks: u64,
    },
    #[error(
        "generation ownership mismatch for scan code {scan_code}: expected generation {expected_generation_id}, found {actual_generation_id} at up action {up_source_action_index}"
    )]
    GenerationOwnershipMismatch {
        scan_code: u16,
        expected_generation_id: GenerationId,
        actual_generation_id: GenerationId,
        up_source_action_index: u32,
    },
    #[error("schedule contains an invalid compiled batch range for packet {packet_index}")]
    InvalidBatchRange { packet_index: usize },
}

#[derive(Debug, Clone, Copy)]
struct OpenGeneration {
    generation_id: GenerationId,
    scan_code: u16,
    down_source_action_index: u32,
    down_scheduled_us: u64,
    down_scheduled_ticks: TimelineTicks,
}

#[derive(Clone, Copy)]
struct ReleaseBoundary {
    source_action_index: u32,
    scheduled_us: u64,
    scheduled_ticks: TimelineTicks,
}

/// Validate every authored same-key Down→Up interval against the native floor.
///
/// The walk follows the compiler's canonical physical order: all Up intents
/// in a packet are closed before its Down chord is opened.  The state is
/// bounded by the fifteen physical key slots and does not depend on the
/// coordinator or any Windows API.
pub fn validate_min_hold_feasibility(
    schedule: &RuntimeSchedule,
    effective_min_hold_us: u64,
) -> Result<(), ScheduleTimingError> {
    validate_min_hold_feasibility_ticks(
        schedule,
        effective_min_hold_us,
        DurationTicks::from_raw(effective_min_hold_us),
        |microseconds| Some(TimelineTicks::from_raw(microseconds)),
    )
}

/// Validate holds and the minimum observable release-to-repress interval.
pub fn validate_min_hold_and_release_gap_feasibility(
    schedule: &RuntimeSchedule,
    effective_min_hold_us: u64,
    min_release_gap_us: u64,
) -> Result<(), ScheduleTimingError> {
    validate_min_hold_and_release_gap_feasibility_ticks(
        schedule,
        effective_min_hold_us,
        DurationTicks::from_raw(effective_min_hold_us),
        min_release_gap_us,
        DurationTicks::from_raw(min_release_gap_us),
        |microseconds| Some(TimelineTicks::from_raw(microseconds)),
    )
}

/// Validate same-key holds in the timing domain used by the runtime.
///
/// Authored timestamps are expressed in microseconds, but the worker places
/// both authored timestamps and the minimum-hold floor into QPC ticks using a
/// ceiling conversion.  Validating the subtraction in microseconds is not
/// equivalent to validating `ceil(down) + ceil(floor) <= ceil(up)`.  Native
/// admission supplies the exact runtime conversion and floor in ticks.
pub fn validate_min_hold_feasibility_ticks<F>(
    schedule: &RuntimeSchedule,
    effective_min_hold_us: u64,
    effective_min_hold_ticks: DurationTicks,
    microseconds_to_ticks: F,
) -> Result<(), ScheduleTimingError>
where
    F: FnMut(u64) -> Option<TimelineTicks>,
{
    validate_min_hold_and_release_gap_feasibility_ticks(
        schedule,
        effective_min_hold_us,
        effective_min_hold_ticks,
        0,
        DurationTicks::from_raw(0),
        microseconds_to_ticks,
    )
}

/// Validate same-key holds and release gaps in the runtime tick domain.
pub fn validate_min_hold_and_release_gap_feasibility_ticks<F>(
    schedule: &RuntimeSchedule,
    effective_min_hold_us: u64,
    effective_min_hold_ticks: DurationTicks,
    min_release_gap_us: u64,
    min_release_gap_ticks: DurationTicks,
    mut microseconds_to_ticks: F,
) -> Result<(), ScheduleTimingError>
where
    F: FnMut(u64) -> Option<TimelineTicks>,
{
    let last_scheduled_us = schedule
        .batches
        .last()
        .map_or(0, |batch| batch.scheduled_us);
    last_scheduled_us
        .checked_add(effective_min_hold_us)
        .ok_or(ScheduleTimingError::TimestampOverflow)?;

    let mut open_by_slot: [Option<OpenGeneration>; MAX_KEYS] = [None; MAX_KEYS];
    let mut last_release_by_slot: [Option<ReleaseBoundary>; MAX_KEYS] = [None; MAX_KEYS];

    for (packet_index, packet) in schedule.packets.iter().enumerate() {
        let batch_start = usize::try_from(packet.first_batch_index)
            .map_err(|_| ScheduleTimingError::InvalidBatchRange { packet_index })?;
        let batch_len = usize::from(packet.batch_count);
        let batch_end = batch_start
            .checked_add(batch_len)
            .ok_or(ScheduleTimingError::InvalidBatchRange { packet_index })?;
        let batches = schedule
            .batches
            .get(batch_start..batch_end)
            .ok_or(ScheduleTimingError::InvalidBatchRange { packet_index })?;

        // The compiler stores Up metadata before Down metadata in the packet
        // arena, but source action order may be the reverse.  Replaying the
        // physical order makes same-timestamp retriggers validate correctly.
        for expected_kind in [ActionKind::Up, ActionKind::Down] {
            for batch in batches.iter().filter(|batch| batch.kind == expected_kind) {
                let batch_ticks = microseconds_to_ticks(batch.scheduled_us)
                    .ok_or(ScheduleTimingError::TimestampOverflow)?;
                let intent_start = usize::try_from(batch.intent_start)
                    .map_err(|_| ScheduleTimingError::InvalidBatchRange { packet_index })?;
                let intent_len = usize::from(batch.intent_len);
                let intent_end = intent_start
                    .checked_add(intent_len)
                    .ok_or(ScheduleTimingError::InvalidBatchRange { packet_index })?;
                let intents = schedule
                    .intents
                    .get(intent_start..intent_end)
                    .ok_or(ScheduleTimingError::InvalidBatchRange { packet_index })?;

                for compact in intents {
                    let slot = usize::from(compact.key_slot());
                    let Some(open) = open_by_slot.get_mut(slot) else {
                        return Err(ScheduleTimingError::InvalidBatchRange { packet_index });
                    };
                    let scan_code = schedule
                        .key_registry
                        .scan_code_for(compact.key_slot())
                        .ok_or(ScheduleTimingError::InvalidBatchRange { packet_index })?;

                    match expected_kind {
                        ActionKind::Up => {
                            let Some(active) = open.take() else {
                                if compact.generation_id() == NO_GENERATION_ID {
                                    continue;
                                }
                                return Err(ScheduleTimingError::GenerationOwnershipMismatch {
                                    scan_code,
                                    expected_generation_id: NO_GENERATION_ID,
                                    actual_generation_id: compact.generation_id(),
                                    up_source_action_index: batch.source_action_index,
                                });
                            };
                            if active.generation_id != compact.generation_id() {
                                return Err(ScheduleTimingError::GenerationOwnershipMismatch {
                                    scan_code,
                                    expected_generation_id: active.generation_id,
                                    actual_generation_id: compact.generation_id(),
                                    up_source_action_index: batch.source_action_index,
                                });
                            }

                            let interval_us =
                                batch.scheduled_us.saturating_sub(active.down_scheduled_us);
                            let earliest_release_ticks = active
                                .down_scheduled_ticks
                                .checked_add_duration(effective_min_hold_ticks)
                                .map_err(|_| ScheduleTimingError::TimestampOverflow)?;
                            if earliest_release_ticks > batch_ticks {
                                return Err(ScheduleTimingError::SameKeyHoldTooShort {
                                    scan_code: active.scan_code,
                                    down_source_action_index: active.down_source_action_index,
                                    up_source_action_index: batch.source_action_index,
                                    down_scheduled_us: active.down_scheduled_us,
                                    up_scheduled_us: batch.scheduled_us,
                                    interval_us,
                                    required_min_hold_us: effective_min_hold_us,
                                    down_scheduled_ticks: active.down_scheduled_ticks.as_u64(),
                                    up_scheduled_ticks: batch_ticks.as_u64(),
                                    required_min_hold_ticks: effective_min_hold_ticks.as_u64(),
                                    earliest_release_ticks: earliest_release_ticks.as_u64(),
                                });
                            }
                            if min_release_gap_us > 0 {
                                last_release_by_slot[slot] = Some(ReleaseBoundary {
                                    source_action_index: batch.source_action_index,
                                    scheduled_us: batch.scheduled_us,
                                    scheduled_ticks: batch_ticks,
                                });
                            }
                        }
                        ActionKind::Down => {
                            if compact.generation_id() == NO_GENERATION_ID || open.is_some() {
                                return Err(ScheduleTimingError::GenerationOwnershipMismatch {
                                    scan_code,
                                    expected_generation_id: NO_GENERATION_ID,
                                    actual_generation_id: compact.generation_id(),
                                    up_source_action_index: batch.source_action_index,
                                });
                            }
                            if let Some(previous_up) = last_release_by_slot[slot]
                                && min_release_gap_us > 0
                            {
                                let release_gap_ticks = batch_ticks
                                    .checked_duration_since(previous_up.scheduled_ticks)
                                    .unwrap_or_else(|_| DurationTicks::from_raw(0));
                                if release_gap_ticks < min_release_gap_ticks {
                                    return Err(ScheduleTimingError::SameKeyReleaseGapTooShort {
                                        scan_code,
                                        previous_up_source_action_index: previous_up
                                            .source_action_index,
                                        next_down_source_action_index: batch.source_action_index,
                                        previous_up_scheduled_us: previous_up.scheduled_us,
                                        next_down_scheduled_us: batch.scheduled_us,
                                        release_gap_us: batch
                                            .scheduled_us
                                            .saturating_sub(previous_up.scheduled_us),
                                        required_release_gap_us: min_release_gap_us,
                                        previous_up_ticks: previous_up.scheduled_ticks.as_u64(),
                                        next_down_ticks: batch_ticks.as_u64(),
                                        required_release_gap_ticks: min_release_gap_ticks.as_u64(),
                                    });
                                }
                            }
                            *open = Some(OpenGeneration {
                                generation_id: compact.generation_id(),
                                scan_code,
                                down_source_action_index: batch.source_action_index,
                                down_scheduled_us: batch.scheduled_us,
                                down_scheduled_ticks: batch_ticks,
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::compile_runtime_intents;
    use crate::model::{ActionKind, KeyActionInput};
    use smallvec::smallvec;
    use std::sync::Arc;

    fn schedule(actions: &[(ActionKind, u64, u16)]) -> RuntimeSchedule {
        let inputs = actions
            .iter()
            .enumerate()
            .map(|(index, &(kind, scheduled_us, scan_code))| KeyActionInput {
                source_action_index: u32::try_from(index).unwrap(),
                kind,
                scheduled_us,
                scan_codes: smallvec![scan_code],
                reason: Arc::from("validation"),
            })
            .collect::<Vec<_>>();
        compile_runtime_intents(&inputs, &[1]).unwrap()
    }

    #[test]
    fn rejects_interval_below_effective_floor_with_source_evidence() {
        let error = validate_min_hold_feasibility(
            &schedule(&[(ActionKind::Down, 100, 1), (ActionKind::Up, 199, 1)]),
            100,
        )
        .unwrap_err();
        assert_eq!(
            error,
            ScheduleTimingError::SameKeyHoldTooShort {
                scan_code: 1,
                down_source_action_index: 0,
                up_source_action_index: 1,
                down_scheduled_us: 100,
                up_scheduled_us: 199,
                interval_us: 99,
                required_min_hold_us: 100,
                down_scheduled_ticks: 100,
                up_scheduled_ticks: 199,
                required_min_hold_ticks: 100,
                earliest_release_ticks: 200,
            }
        );
    }

    #[test]
    fn accepts_exact_and_longer_intervals() {
        assert!(
            validate_min_hold_feasibility(
                &schedule(&[(ActionKind::Down, 100, 1), (ActionKind::Up, 200, 1)]),
                100,
            )
            .is_ok()
        );
        assert!(
            validate_min_hold_feasibility(
                &schedule(&[(ActionKind::Down, 100, 1), (ActionKind::Up, 201, 1)]),
                100,
            )
            .is_ok()
        );
    }

    #[test]
    fn accepts_same_timestamp_retrigger_after_valid_old_hold() {
        let schedule = schedule(&[
            (ActionKind::Down, 100, 1),
            (ActionKind::Up, 200, 1),
            (ActionKind::Down, 200, 1),
            (ActionKind::Up, 300, 1),
        ]);
        assert!(validate_min_hold_feasibility(&schedule, 100).is_ok());
    }

    #[test]
    fn rejects_same_timestamp_retrigger_when_release_gap_is_required() {
        let schedule = schedule(&[
            (ActionKind::Down, 100, 1),
            (ActionKind::Up, 200, 1),
            (ActionKind::Down, 200, 1),
            (ActionKind::Up, 300, 1),
        ]);
        let error = validate_min_hold_and_release_gap_feasibility(&schedule, 100, 100)
            .expect_err("same-key retrigger must leave one frame of release gap");
        assert_eq!(
            error,
            ScheduleTimingError::SameKeyReleaseGapTooShort {
                scan_code: 1,
                previous_up_source_action_index: 1,
                next_down_source_action_index: 2,
                previous_up_scheduled_us: 200,
                next_down_scheduled_us: 200,
                release_gap_us: 0,
                required_release_gap_us: 100,
                previous_up_ticks: 200,
                next_down_ticks: 200,
                required_release_gap_ticks: 100,
            }
        );
    }

    #[test]
    fn accepts_same_key_retrigger_after_one_frame_release_gap() {
        let schedule = schedule(&[
            (ActionKind::Down, 100, 1),
            (ActionKind::Up, 200, 1),
            (ActionKind::Down, 300, 1),
            (ActionKind::Up, 400, 1),
        ]);
        assert!(validate_min_hold_and_release_gap_feasibility(&schedule, 100, 100).is_ok());
    }

    #[test]
    fn rejects_qpc_quantization_mismatch_before_runtime_dispatch() {
        let authored = schedule(&[(ActionKind::Down, 1, 1), (ActionKind::Up, 101, 1)]);
        let frequency_hz = 3_125_000u64;
        let ceil_us_to_ticks = |microseconds: u64| {
            microseconds
                .checked_mul(frequency_hz)
                .and_then(|value| value.checked_add(999_999))
                .map(|value| TimelineTicks::from_raw(value / 1_000_000))
        };
        let min_hold_ticks = ceil_us_to_ticks(100).expect("hold conversion");

        let error = validate_min_hold_feasibility_ticks(
            &authored,
            100,
            DurationTicks::from_raw(min_hold_ticks.as_u64()),
            ceil_us_to_ticks,
        )
        .expect_err("tick-domain rounding must reject the schedule");

        assert_eq!(
            error,
            ScheduleTimingError::SameKeyHoldTooShort {
                scan_code: 1,
                down_source_action_index: 0,
                up_source_action_index: 1,
                down_scheduled_us: 1,
                up_scheduled_us: 101,
                interval_us: 100,
                required_min_hold_us: 100,
                down_scheduled_ticks: 4,
                up_scheduled_ticks: 316,
                required_min_hold_ticks: 313,
                earliest_release_ticks: 317,
            }
        );
        let message = error.to_string();
        assert!(message.contains("interval=100us, required=100us"));
        assert!(message.contains("earliest release=317 ticks"));
    }

    #[test]
    fn rejects_timestamp_overflow_before_walking_generations() {
        let schedule = schedule(&[(ActionKind::Down, u64::MAX, 1)]);
        assert_eq!(
            validate_min_hold_feasibility(&schedule, 1),
            Err(ScheduleTimingError::TimestampOverflow)
        );
    }
}
