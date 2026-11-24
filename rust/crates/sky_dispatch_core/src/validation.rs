//! Pure admission validation for authored generation schedules.

use thiserror::Error;

use crate::model::{RuntimeSchedule, UnmatchedUpMetadata};
use crate::time::{DurationTicks, TimelineTicks};

const VALIDATION_CHECKPOINT_INTERVAL: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ScheduleTimingError {
    #[error("schedule and timing configuration exceed supported timestamp range")]
    TimestampOverflow,
    #[error(
        "same-key hold too short for scan code {scan_code}: down action {down_source_action_index} at {down_scheduled_us}us, up action {up_source_action_index} at {up_scheduled_us}us, interval={interval_us}us, required={required_min_hold_us}us"
    )]
    SameKeyHoldTooShort {
        scan_code: u16,
        down_source_action_index: u32,
        up_source_action_index: u32,
        down_scheduled_us: u64,
        up_scheduled_us: u64,
        interval_us: u64,
        required_min_hold_us: u64,
    },
    #[error(
        "invalid paired Up for scan code {scan_code}: Down action {down_source_action_index} at {down_scheduled_us}us, Up action {up_source_action_index} at {up_scheduled_us}us"
    )]
    InvalidPairedUp {
        scan_code: u16,
        down_source_action_index: u32,
        up_source_action_index: u32,
        down_scheduled_us: u64,
        up_scheduled_us: u64,
    },
    #[error("schedule validation cancelled")]
    Cancelled,
}

/// Validate authored Down→Up intervals against H before QPC normalization.
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

/// Validate authored intervals and prove that timestamps plus H fit the QPC
/// domain. Authored microseconds are checked first; QPC rounding is normalized
/// later by `PreparedDispatchSchedule` instead of turning a valid interval into
/// a planning error.
pub fn validate_min_hold_feasibility_ticks<F>(
    schedule: &RuntimeSchedule,
    effective_min_hold_us: u64,
    effective_min_hold_ticks: DurationTicks,
    microseconds_to_ticks: F,
) -> Result<(), ScheduleTimingError>
where
    F: FnMut(u64) -> Option<TimelineTicks>,
{
    validate_min_hold_feasibility_ticks_with_checkpoint(
        schedule,
        effective_min_hold_us,
        effective_min_hold_ticks,
        microseconds_to_ticks,
        || Ok(()),
    )
}

pub(crate) fn validate_min_hold_feasibility_ticks_with_checkpoint<F, C>(
    schedule: &RuntimeSchedule,
    effective_min_hold_us: u64,
    effective_min_hold_ticks: DurationTicks,
    mut microseconds_to_ticks: F,
    mut checkpoint: C,
) -> Result<(), ScheduleTimingError>
where
    F: FnMut(u64) -> Option<TimelineTicks>,
    C: FnMut() -> Result<(), ScheduleTimingError>,
{
    // Phase one is authored-domain validation and deliberately performs no
    // QPC conversion, so a source-position error wins over tick arithmetic.
    for (chord_index, chord) in schedule.down_chords.iter().enumerate() {
        if chord_index % VALIDATION_CHECKPOINT_INTERVAL == 0 {
            checkpoint()?;
        }
        for intent in &chord.intents {
            let interval_us = intent
                .paired_up_offset_us
                .checked_sub(chord.scheduled_us)
                .ok_or(ScheduleTimingError::InvalidPairedUp {
                    scan_code: intent.scan_code,
                    down_source_action_index: intent.source_action_index,
                    up_source_action_index: intent.paired_up_source_action_index,
                    down_scheduled_us: chord.scheduled_us,
                    up_scheduled_us: intent.paired_up_offset_us,
                })?;
            if interval_us < effective_min_hold_us {
                return Err(ScheduleTimingError::SameKeyHoldTooShort {
                    scan_code: intent.scan_code,
                    down_source_action_index: intent.source_action_index,
                    up_source_action_index: intent.paired_up_source_action_index,
                    down_scheduled_us: chord.scheduled_us,
                    up_scheduled_us: intent.paired_up_offset_us,
                    interval_us,
                    required_min_hold_us: effective_min_hold_us,
                });
            }
        }
    }

    // Phase two validates every authored time used by preparation and proves
    // that H can be added at each Down in the QPC domain.
    for (chord_index, chord) in schedule.down_chords.iter().enumerate() {
        if chord_index % VALIDATION_CHECKPOINT_INTERVAL == 0 {
            checkpoint()?;
        }
        let down_ticks = microseconds_to_ticks(chord.scheduled_us)
            .ok_or(ScheduleTimingError::TimestampOverflow)?;
        down_ticks
            .checked_add_duration(effective_min_hold_ticks)
            .map_err(|_| ScheduleTimingError::TimestampOverflow)?;
        for intent in &chord.intents {
            microseconds_to_ticks(intent.paired_up_offset_us)
                .ok_or(ScheduleTimingError::TimestampOverflow)?;
        }
    }
    for (index, unmatched_up) in schedule.unmatched_ups.iter().enumerate() {
        if index % VALIDATION_CHECKPOINT_INTERVAL == 0 {
            checkpoint()?;
        }
        validate_unmatched_up_timestamp(unmatched_up, &mut microseconds_to_ticks)?;
    }
    microseconds_to_ticks(schedule.schedule_end_offset_us)
        .ok_or(ScheduleTimingError::TimestampOverflow)?;
    Ok(())
}

fn validate_unmatched_up_timestamp<F>(
    unmatched_up: &UnmatchedUpMetadata,
    microseconds_to_ticks: &mut F,
) -> Result<(), ScheduleTimingError>
where
    F: FnMut(u64) -> Option<TimelineTicks>,
{
    microseconds_to_ticks(unmatched_up.scheduled_us)
        .ok_or(ScheduleTimingError::TimestampOverflow)?;
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
    fn validates_hold_boundary_in_authored_microseconds() {
        let below = schedule(&[(ActionKind::Down, 100, 1), (ActionKind::Up, 199, 1)]);
        assert_eq!(
            validate_min_hold_feasibility(&below, 100),
            Err(ScheduleTimingError::SameKeyHoldTooShort {
                scan_code: 1,
                down_source_action_index: 0,
                up_source_action_index: 1,
                down_scheduled_us: 100,
                up_scheduled_us: 199,
                interval_us: 99,
                required_min_hold_us: 100,
            })
        );
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
    fn same_timestamp_source_order_does_not_change_generation_pairing() {
        for actions in [
            [
                (ActionKind::Down, 100, 1),
                (ActionKind::Up, 200, 1),
                (ActionKind::Down, 200, 1),
                (ActionKind::Up, 300, 1),
            ],
            [
                (ActionKind::Down, 100, 1),
                (ActionKind::Down, 200, 1),
                (ActionKind::Up, 200, 1),
                (ActionKind::Up, 300, 1),
            ],
        ] {
            assert!(validate_min_hold_feasibility(&schedule(&actions), 100).is_ok());
        }
    }

    #[test]
    fn authored_h_boundary_can_be_normalized_when_qpc_rounding_needs_it() {
        let schedule = schedule(&[(ActionKind::Down, 1_000, 1), (ActionKind::Up, 18_167, 1)]);
        let frequency_hz = 3_579_545u64;
        let ceil_us_to_ticks = |microseconds: u64| {
            microseconds
                .checked_mul(frequency_hz)
                .and_then(|value| value.checked_add(999_999))
                .map(|value| TimelineTicks::from_raw(value / 1_000_000))
        };
        assert!(
            validate_min_hold_feasibility_ticks(
                &schedule,
                17_167,
                DurationTicks::from_raw(61_451),
                ceil_us_to_ticks,
            )
            .is_ok()
        );
    }

    #[test]
    fn reports_qpc_timestamp_conversion_failure_after_authored_validation() {
        let schedule = schedule(&[
            (ActionKind::Down, u64::MAX - 1, 1),
            (ActionKind::Up, u64::MAX, 1),
        ]);
        assert_eq!(
            validate_min_hold_feasibility_ticks(
                &schedule,
                1,
                DurationTicks::from_raw(1),
                |microseconds| {
                    (microseconds != u64::MAX).then_some(TimelineTicks::from_raw(microseconds))
                }
            ),
            Err(ScheduleTimingError::TimestampOverflow)
        );
    }
}
