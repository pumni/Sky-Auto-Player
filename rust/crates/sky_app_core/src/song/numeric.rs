pub(super) fn truncated_f64_to_i64(value: f64) -> Option<i64> {
    const I64_EXCLUSIVE_UPPER_BOUND: f64 = 9_223_372_036_854_775_808.0;
    if !value.is_finite() || value < i64::MIN as f64 || value >= I64_EXCLUSIVE_UPPER_BOUND {
        return None;
    }
    Some(value.trunc() as i64)
}

/// Scale an onset and round to the nearest representable microsecond using
/// nearest-even ties. Invalid or out-of-range values are rejected.
pub(crate) fn round_scaled_onset_checked(source_at_us: u64, tempo_scale: f64) -> Option<u64> {
    if !tempo_scale.is_finite() || tempo_scale <= 0.0 {
        return None;
    }
    if tempo_scale == 1.0 {
        return Some(source_at_us);
    }

    checked_round_nonnegative_u64(source_at_us as f64 / tempo_scale)
}

fn checked_round_nonnegative_u64(value: f64) -> Option<u64> {
    const U64_EXCLUSIVE_UPPER_BOUND: f64 = 18_446_744_073_709_551_616.0;
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    let rounded = value.round_ties_even();
    if !rounded.is_finite() || rounded < 0.0 || rounded >= U64_EXCLUSIVE_UPPER_BOUND {
        return None;
    }
    Some(rounded as u64)
}

fn round_scaled_value_to_hundredths(value: f64) -> Option<f64> {
    if !value.is_finite() {
        return None;
    }
    let scaled = value * 100.0;
    if !scaled.is_finite() {
        return None;
    }
    let rounded = scaled.round_ties_even() / 100.0;
    rounded.is_finite().then_some(rounded)
}

pub(super) fn bounded_tempo_scale_recommendation(value: f64) -> Option<f64> {
    round_scaled_value_to_hundredths(value).map(|rounded| rounded.clamp(0.1, 1.0))
}

pub(super) fn checked_note_gap_us(previous_ms: i64, current_ms: i64) -> Option<u64> {
    if previous_ms < 0 || current_ms < 0 {
        return None;
    }
    let gap_ms = current_ms.checked_sub(previous_ms)?;
    u64::try_from(gap_ms).ok()?.checked_mul(1_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{
        ActionKind, Note, SKY_SCAN_CODES, Song, SongError, analyze_schedule_with_context,
        build_schedule, build_schedule_from_scaled_onsets, build_schedule_with_policy,
    };

    #[test]
    fn checked_nearest_even_timestamp_rounding_covers_ties_edges_and_u64_bound() {
        assert_eq!(checked_round_nonnegative_u64(0.5), Some(0));
        assert_eq!(checked_round_nonnegative_u64(1.5), Some(2));
        assert_eq!(checked_round_nonnegative_u64(2.5), Some(2));
        assert_eq!(checked_round_nonnegative_u64(3.5), Some(4));
        assert_eq!(checked_round_nonnegative_u64(-0.0), Some(0));
        assert_eq!(checked_round_nonnegative_u64(-0.5), None);

        for (tie, lower, upper) in [(0.5_f64, 0, 1), (1.5_f64, 1, 2), (2.5_f64, 2, 3)] {
            assert_eq!(
                checked_round_nonnegative_u64(f64::from_bits(tie.to_bits() - 1)),
                Some(lower)
            );
            assert_eq!(
                checked_round_nonnegative_u64(tie),
                Some(if lower % 2 == 0 { lower } else { upper })
            );
            assert_eq!(
                checked_round_nonnegative_u64(f64::from_bits(tie.to_bits() + 1)),
                Some(upper)
            );
        }

        assert_eq!(checked_round_nonnegative_u64(f64::NAN), None);
        assert_eq!(checked_round_nonnegative_u64(f64::INFINITY), None);
        assert_eq!(checked_round_nonnegative_u64(f64::NEG_INFINITY), None);
        let largest_f64_below_u64_upper = f64::from_bits((u64::MAX as f64).to_bits() - 1);
        assert_eq!(
            checked_round_nonnegative_u64(largest_f64_below_u64_upper),
            Some(largest_f64_below_u64_upper as u64)
        );
        assert_eq!(checked_round_nonnegative_u64(u64::MAX as f64), None);
    }

    #[test]
    fn scaled_onset_conversion_rejects_nonfinite_and_unrepresentable_values() {
        assert_eq!(round_scaled_onset_checked(1, 2.0), Some(0));
        assert_eq!(round_scaled_onset_checked(3, 2.0), Some(2));
        assert_eq!(round_scaled_onset_checked(5, 2.0), Some(2));
        assert_eq!(round_scaled_onset_checked(u64::MAX, 1.0), Some(u64::MAX));
        assert_eq!(round_scaled_onset_checked(1, 1e-300), None);
        assert_eq!(round_scaled_onset_checked(1, f64::INFINITY), None);
        assert_eq!(round_scaled_onset_checked(1, f64::NAN), None);
        assert_eq!(round_scaled_onset_checked(1, 0.0), None);
    }

    #[test]
    fn hundredths_quantization_uses_nearest_even_and_omits_nonfinite_results() {
        assert_eq!(round_scaled_value_to_hundredths(0.015), Some(0.02));
        assert_eq!(round_scaled_value_to_hundredths(0.025), Some(0.02));
        assert_eq!(round_scaled_value_to_hundredths(0.035), Some(0.04));
        assert_eq!(round_scaled_value_to_hundredths(2.675), Some(2.68));
        assert_eq!(round_scaled_value_to_hundredths(0.995), Some(1.0));
        assert_eq!(
            round_scaled_value_to_hundredths(0.014999999999999998),
            Some(0.01)
        );
        assert_eq!(
            round_scaled_value_to_hundredths(0.015000000000000003),
            Some(0.02)
        );
        assert_eq!(
            round_scaled_value_to_hundredths(f64::from_bits(1)),
            Some(0.0)
        );
        let negative_zero = round_scaled_value_to_hundredths(-0.0).expect("finite zero");
        assert!(negative_zero.is_sign_negative());
        assert_eq!(round_scaled_value_to_hundredths(f64::INFINITY), None);
        assert_eq!(round_scaled_value_to_hundredths(f64::NAN), None);
        assert_eq!(round_scaled_value_to_hundredths(f64::MAX), None);
        assert_eq!(bounded_tempo_scale_recommendation(2.675), Some(1.0));
        assert_eq!(bounded_tempo_scale_recommendation(f64::MAX), None);
    }

    #[test]
    fn legacy_schedule_rejects_invalid_source_and_release_time_overflow() {
        let policy = crate::timing::MaterializedTimingPolicy::from_user_margin(60, 1.0, 0)
            .expect("valid policy");
        let song = |time_ms| Song {
            name: "Boundary".into(),
            notes: vec![Note {
                time_ms,
                key: "Key0".into(),
                source_index: 0,
            }],
        };

        assert!(matches!(
            build_schedule_with_policy(&song(-1), 1.0, &policy),
            Err(SongError::NegativeTime { index: 0 })
        ));
        assert!(matches!(
            build_schedule_with_policy(&song(i64::MAX), 1.0, &policy),
            Err(SongError::TimingOverflow)
        ));
        assert!(matches!(
            build_schedule_with_policy(&song(1), 1e-300, &policy),
            Err(SongError::TimingOverflow)
        ));
        assert!(matches!(
            build_schedule_with_policy(&song(18_446_744_073_709_550), 1.0, &policy),
            Err(SongError::TimingOverflow)
        ));
    }

    #[test]
    fn checked_schedule_accepts_maximum_release_and_rejects_the_next_timestamp() {
        let policy = crate::timing::MaterializedTimingPolicy {
            fps: 60,
            frame_us: 16_667,
            hold_frames: 1.0,
            frame_base_hold_us: u64::MAX,
            timing_margin_us: 0,
            min_hold_us: u64::MAX,
            min_release_gap_us: 0,
            focus_restore_grace_us: 0,
        };
        let at_zero = Song {
            name: "Maximum hold".into(),
            notes: vec![Note {
                time_ms: 0,
                key: "Key0".into(),
                source_index: 0,
            }],
        };
        let schedule = build_schedule_with_policy(&at_zero, 1.0, &policy).expect("exact max");
        assert_eq!(schedule.actions[1].at_us, u64::MAX);
        assert_eq!(schedule.source_duration_us, u64::MAX);
        assert_eq!(schedule.actions[0].kind, ActionKind::Down);
        assert_eq!(schedule.actions[1].kind, ActionKind::Up);

        let after_zero = Song {
            notes: vec![Note {
                time_ms: 1,
                key: "Key0".into(),
                source_index: 0,
            }],
            ..at_zero
        };
        assert!(matches!(
            build_schedule_with_policy(&after_zero, 1.0, &policy),
            Err(SongError::TimingOverflow)
        ));
    }

    #[test]
    fn risk_gap_overflow_is_omitted_and_nonfinite_suggestions_are_diagnosed() {
        let song = Song {
            name: "Repeat".into(),
            notes: vec![
                Note {
                    time_ms: 0,
                    key: "Key0".into(),
                    source_index: 0,
                },
                Note {
                    time_ms: 1,
                    key: "Key0".into(),
                    source_index: 1,
                },
            ],
        };
        let schedule = build_schedule(&song, 1.0, 1.0, 60).expect("valid schedule");
        let raw_extremes = [
            Note {
                time_ms: i64::MIN,
                key: "Key0".into(),
                source_index: 2,
            },
            Note {
                time_ms: i64::MAX,
                key: "Key0".into(),
                source_index: 3,
            },
        ];
        let risk =
            analyze_schedule_with_context(&schedule, Some(&raw_extremes), f64::NAN, f64::INFINITY);
        assert_eq!(risk.severity, "high");
        assert_eq!(risk.min_any_note_gap_us, Some(1_000));
        assert_eq!(
            risk.min_same_key_gap_us,
            schedule.shortest_same_key_interval_us
        );
        assert_eq!(risk.suggested_tempo_scale, None);
        assert!(risk.suggested_hold_frames.is_some());
        assert!(
            risk.recommendations
                .iter()
                .any(|text| text.contains("Tempo recommendation omitted"))
        );
    }

    #[test]
    fn unrepresentable_tempo_advice_is_omitted_with_a_schedule_warning() {
        let policy = crate::timing::MaterializedTimingPolicy {
            fps: 60,
            frame_us: 16_667,
            hold_frames: 1.0,
            frame_base_hold_us: 2_000_000,
            timing_margin_us: 0,
            min_hold_us: 2_000_000,
            min_release_gap_us: 0,
            focus_restore_grace_us: 0,
        };
        let schedule = build_schedule_from_scaled_onsets(
            vec![(0, SKY_SCAN_CODES[0], 0), (1_000_000, SKY_SCAN_CODES[0], 1)],
            f64::MAX,
            &policy,
        )
        .expect("timestamps and releases are representable");
        assert_eq!(schedule.impossible_same_key_repeats, 1);
        assert_eq!(schedule.recommended_tempo_scale, None);
        assert!(
            schedule
                .warnings
                .iter()
                .any(|text| text.contains("Tempo recommendation omitted"))
        );
    }
}
