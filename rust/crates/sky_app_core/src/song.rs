//! Pure song decoding, deterministic Piano planning, scheduling, and bounded risk analysis.
//!
//! This module deliberately owns no filesystem, delivery, Win32, or
//! realtime-player concerns. A caller supplies bytes and receives an
//! immutable plan that an outer native adapter may hand to the qualified
//! realtime engine.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub mod decode;
pub use decode::*;
mod planner;
pub use planner::*;
mod numeric;
pub(crate) use numeric::round_scaled_onset_checked;
use numeric::{bounded_tempo_scale_recommendation, checked_note_gap_us, truncated_f64_to_i64};

pub const SKY_SCAN_CODES: [u16; 15] = [
    0x15, 0x16, 0x17, 0x18, 0x19, 0x23, 0x24, 0x25, 0x26, 0x27, 0x31, 0x32, 0x33, 0x34, 0x35,
];
pub const KEY_NAMES: [&str; 15] = [
    "Key0", "Key1", "Key2", "Key3", "Key4", "Key5", "Key6", "Key7", "Key8", "Key9", "Key10",
    "Key11", "Key12", "Key13", "Key14",
];
pub const VALID_FPS: [u16; 7] = [30, 60, 90, 120, 144, 165, 240];
pub const HOLD_FRAMES: [f64; 3] = [1.0, 1.25, 1.5];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub time_ms: i64,
    pub key: String,
    pub source_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Song {
    pub name: String,
    pub notes: Vec<Note>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SongError {
    #[error("invalid JSON: {0}")]
    InvalidJson(String),
    #[error("song root must be an object")]
    InvalidRoot,
    #[error("song list is empty")]
    EmptySongList,
    #[error("songNotes is missing")]
    MissingNotes,
    #[error("songNotes must be an array")]
    InvalidNotes,
    #[error("note {index} must be an object")]
    InvalidNote { index: usize },
    #[error("note {index} is missing time")]
    MissingTime { index: usize },
    #[error("note {index} is missing key")]
    MissingKey { index: usize },
    #[error("note {index} has invalid time")]
    InvalidTime { index: usize },
    #[error("note {index} has negative time")]
    NegativeTime { index: usize },
    #[error("note {index} has an unmapped key: {key}")]
    UnmappedKey { index: usize, key: String },
    #[error("schedule tempo must be finite and positive")]
    InvalidTempo,
    #[error("schedule FPS is unsupported")]
    InvalidFps,
    #[error("schedule hold frame value is unsupported")]
    InvalidHold,
    #[error("schedule timing margin is invalid")]
    InvalidTimingMargin,
    #[error("schedule timing value overflowed")]
    TimingOverflow,
    #[error("same-key repeat is infeasible: {interval_us}us")]
    ImpossibleRepeat { interval_us: u64 },
    #[error("schedule contains no actions")]
    EmptySchedule,
}

pub fn parse_song_json(bytes: &[u8], fallback_name: &str) -> Result<Song, SongError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| SongError::InvalidJson(error.to_string()))?;
    let object = match value {
        Value::Array(values) => values.into_iter().next().ok_or(SongError::EmptySongList)?,
        other => other,
    };
    let object = object.as_object().ok_or(SongError::InvalidRoot)?;
    let notes_value = object.get("songNotes").ok_or(SongError::MissingNotes)?;
    let notes = notes_value.as_array().ok_or(SongError::InvalidNotes)?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(fallback_name)
        .to_owned();
    let mut parsed = Vec::with_capacity(notes.len());
    for (index, raw) in notes.iter().enumerate() {
        let note = raw.as_object().ok_or(SongError::InvalidNote { index })?;
        let raw_time = note.get("time").ok_or(SongError::MissingTime { index })?;
        let time_ms =
            parse_legacy_timestamp_ms(raw_time).ok_or(SongError::InvalidTime { index })?;
        if time_ms < 0 {
            return Err(SongError::NegativeTime { index });
        }
        let raw_key = note.get("key").ok_or(SongError::MissingKey { index })?;
        let key = raw_key
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| SongError::UnmappedKey {
                index,
                key: format_legacy_song_value(raw_key),
            })?;
        if scan_code_for_key(&key).is_none() {
            return Err(SongError::UnmappedKey { index, key });
        }
        parsed.push(Note {
            time_ms,
            key,
            source_index: index,
        });
    }
    parsed.sort_by_key(|note| note.time_ms);
    Ok(Song {
        name,
        notes: parsed,
    })
}

fn parse_legacy_timestamp_ms(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(value) => Some(i64::from(*value)),
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            .or_else(|| number.as_f64().and_then(truncated_f64_to_i64)),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn format_legacy_song_value(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(value) => value.to_string().to_ascii_lowercase(),
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(format_legacy_song_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(_) => "{...}".into(),
    }
}

pub fn scan_code_for_key(key: &str) -> Option<u16> {
    let key = key
        .strip_prefix("1Key")
        .or_else(|| key.strip_prefix("2Key"))
        .or_else(|| key.strip_prefix("3Key"))
        .map(|suffix| format!("Key{suffix}"))
        .unwrap_or_else(|| key.to_owned());
    KEY_NAMES
        .iter()
        .position(|candidate| *candidate == key)
        .map(|index| SKY_SCAN_CODES[index])
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnsetChord {
    pub at_us: u64,
    pub scan_codes: Vec<u16>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleMetadata {
    pub chords: Vec<OnsetChord>,
    pub hold_us: u64,
    pub source_duration_us: u64,
    pub playback_duration_us: u64,
    pub duration_us: u64,
    pub note_count: usize,
    pub deduplicated_note_count: usize,
    pub duplicate_note_count: usize,
    pub impossible_same_key_repeats: usize,
    pub max_polyphony: usize,
    pub shortest_same_key_interval_us: Option<u64>,
    pub min_same_key_up_gap_us: Option<u64>,
    /// Physically feasible repeats whose nominal Up-to-Down gap is shorter
    /// than one configured frame.
    pub short_up_gap_repeat_count: usize,
    pub recommended_hold_frames: Option<f64>,
    pub recommended_tempo_scale: Option<f64>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DenseCluster {
    pub start_us: u64,
    pub end_us: u64,
    pub note_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskReport {
    pub severity: String,
    pub reason: String,
    pub recommendations: Vec<String>,
    pub dense_clusters: Vec<DenseCluster>,
    pub average_notes_per_second: f64,
    pub peak_notes_per_second_1s: f64,
    pub max_polyphony: usize,
    pub max_chord_size: usize,
    pub chords_count: usize,
    pub timing_stress_rate: f64,
    pub short_up_gap_repeat_count: usize,
    pub min_any_note_gap_us: Option<u64>,
    pub min_same_key_gap_us: Option<u64>,
    pub suggested_hold_frames: Option<f64>,
    pub suggested_tempo_scale: Option<f64>,
}

pub fn frame_us(fps: u16) -> Result<u64, SongError> {
    if VALID_FPS.contains(&fps) {
        Ok(1_000_000_u64.div_ceil(u64::from(fps)))
    } else {
        Err(SongError::InvalidFps)
    }
}

pub fn build_schedule(
    song: &Song,
    hold_frames: f64,
    tempo_scale: f64,
    fps: u16,
) -> Result<ScheduleMetadata, SongError> {
    let policy = crate::timing::MaterializedTimingPolicy::from_user_margin(
        fps,
        hold_frames,
        crate::settings::DEFAULT_TIMING_MARGIN_US,
    )?;
    build_schedule_with_policy(song, tempo_scale, &policy)
}

pub fn build_schedule_with_policy(
    song: &Song,
    tempo_scale: f64,
    policy: &crate::timing::MaterializedTimingPolicy,
) -> Result<ScheduleMetadata, SongError> {
    if !tempo_scale.is_finite() || tempo_scale <= 0.0 {
        return Err(SongError::InvalidTempo);
    }
    let mut drafts = Vec::with_capacity(song.notes.len());
    for note in &song.notes {
        let scan_code = scan_code_for_key(&note.key).ok_or_else(|| SongError::UnmappedKey {
            index: note.source_index,
            key: note.key.clone(),
        })?;
        let source_at_us = u64::try_from(note.time_ms)
            .map_err(|_| SongError::NegativeTime {
                index: note.source_index,
            })?
            .checked_mul(1_000)
            .ok_or(SongError::TimingOverflow)?;
        let at_us = round_scaled_onset_checked(source_at_us, tempo_scale)
            .ok_or(SongError::TimingOverflow)?;
        drafts.push((at_us, scan_code, note.source_index));
    }
    build_schedule_from_scaled_onsets(drafts, tempo_scale, policy)
}

/// Reuses the established scheduler for already-scaled planner onsets.
/// `source_index` must be unique per onset.
pub(crate) fn build_schedule_from_scaled_onsets(
    mut drafts: Vec<(u64, u16, usize)>,
    tempo_scale: f64,
    policy: &crate::timing::MaterializedTimingPolicy,
) -> Result<ScheduleMetadata, SongError> {
    if !tempo_scale.is_finite() || tempo_scale <= 0.0 {
        return Err(SongError::InvalidTempo);
    }
    // The authored Up follows the Down by H. Same-key repeats need no second
    // post-Up floor; an Up at the next Down timestamp is ordered first.
    let target_hold_us = policy.min_hold_us;
    let min_hold_us = policy.min_hold_us;
    let raw_count = drafts.len();
    let mut seen = BTreeSet::new();
    drafts.retain(|(at, scan, _)| seen.insert((*at, *scan)));

    let mut next_by_source = HashMap::<usize, Option<u64>>::new();
    let mut next = HashMap::<u16, u64>::new();
    for (at, scan, source_index) in drafts.iter().rev() {
        next_by_source.insert(*source_index, next.get(scan).copied());
        next.insert(*scan, *at);
    }

    let mut chords_by_time = BTreeMap::<u64, Vec<u16>>::new();
    let mut impossible = 0;
    let mut short_up_gap_repeat_count = 0;
    let mut shortest = None;
    let mut min_up_gap = None;
    for (at, scan, source_index) in &drafts {
        let next_at = next_by_source.get(source_index).copied().flatten();
        if let Some(next_at) = next_at {
            let interval = next_at.saturating_sub(*at);
            shortest = Some(shortest.map_or(interval, |current: u64| current.min(interval)));
            if interval < min_hold_us {
                impossible += 1;
            } else if interval - min_hold_us < policy.frame_us {
                short_up_gap_repeat_count += 1;
            }
        }
        let actual_hold = target_hold_us;
        if let Some(next_at) = next_at {
            let gap = next_at.saturating_sub(*at).saturating_sub(actual_hold);
            min_up_gap = Some(min_up_gap.map_or(gap, |current: u64| current.min(gap)));
        }
        // Validate that every onset can accommodate the materialized pulse hold.
        at.checked_add(actual_hold)
            .ok_or(SongError::TimingOverflow)?;
        let entry = chords_by_time.entry(*at).or_default();
        if !entry.contains(scan) {
            entry.push(*scan);
        }
    }
    let chords = chords_by_time
        .into_iter()
        .map(|(at_us, scan_codes)| OnsetChord { at_us, scan_codes })
        .collect::<Vec<_>>();
    let max_polyphony = max_polyphony(&chords, target_hold_us);
    let duration = chords
        .last()
        .map(|chord| {
            chord
                .at_us
                .checked_add(target_hold_us)
                .ok_or(SongError::TimingOverflow)
        })
        .transpose()?
        .unwrap_or_default();
    let mut warnings = Vec::new();
    let (recommended_hold_frames, recommended_tempo_scale) = if impossible > 0 {
        if let Some(shortest) = shortest {
            let suggestion = bounded_tempo_scale_recommendation(if min_hold_us == 0 {
                tempo_scale
            } else {
                tempo_scale * shortest as f64 / min_hold_us as f64
            });
            if suggestion.is_none() {
                warnings.push(
                    "Tempo recommendation omitted because its value was not finite and representable."
                        .into(),
                );
            }
            (Some(1.0), suggestion)
        } else {
            (Some(1.0), None)
        }
    } else {
        (None, None)
    };
    let source_duration_us = drafts
        .iter()
        .map(|(at, _, _)| *at)
        .max()
        .unwrap_or_default()
        .checked_add(target_hold_us)
        .ok_or(SongError::TimingOverflow)?;
    Ok(ScheduleMetadata {
        chords,
        hold_us: target_hold_us,
        source_duration_us,
        playback_duration_us: duration,
        duration_us: duration,
        note_count: raw_count,
        deduplicated_note_count: drafts.len(),
        duplicate_note_count: raw_count.saturating_sub(drafts.len()),
        impossible_same_key_repeats: impossible,
        max_polyphony,
        shortest_same_key_interval_us: shortest,
        min_same_key_up_gap_us: min_up_gap,
        short_up_gap_repeat_count,
        recommended_hold_frames,
        recommended_tempo_scale,
        warnings,
    })
}

fn max_polyphony(chords: &[OnsetChord], hold_us: u64) -> usize {
    let mut active_release_by_scan = HashMap::<u16, u64>::new();
    let mut maximum = 0;
    for chord in chords {
        // Equal release and onset timestamps order Up before Down.
        active_release_by_scan.retain(|_, release_at| *release_at > chord.at_us);
        let release_at = chord.at_us.saturating_add(hold_us);
        for &scan in &chord.scan_codes {
            active_release_by_scan.insert(scan, release_at);
        }
        maximum = maximum.max(active_release_by_scan.len());
    }
    maximum
}

pub fn analyze_schedule(schedule: &ScheduleMetadata) -> RiskReport {
    analyze_schedule_with_context(schedule, None, 1.0, 1.0)
}

/// Analyze executable schedule metrics while retaining raw authored notes for
/// onset-gap metrics. The scheduler deduplicates executable slots, while raw
/// notes preserve gaps between authored events when available.
pub fn analyze_schedule_with_notes(schedule: &ScheduleMetadata, raw_notes: &[Note]) -> RiskReport {
    analyze_schedule_with_context(schedule, Some(raw_notes), 1.0, 1.0)
}

pub fn analyze_schedule_with_context(
    schedule: &ScheduleMetadata,
    raw_notes: Option<&[Note]>,
    current_hold_frames: f64,
    current_tempo_scale: f64,
) -> RiskReport {
    let downs = &schedule.chords;
    let mut dense: Vec<DenseCluster> = Vec::new();
    let mut left = 0usize;
    for right in 0..downs.len() {
        while downs[right].at_us.saturating_sub(downs[left].at_us) > 100_000 {
            left += 1;
        }
        let count = right - left + 1;
        if count > 6 {
            let start = downs[left].at_us;
            let end = downs[right].at_us;
            if let Some(previous) = dense.last_mut()
                && start <= previous.end_us.saturating_add(50_000)
            {
                previous.end_us = end;
                previous.note_count = previous.note_count.max(count);
                continue;
            }
            dense.push(DenseCluster {
                start_us: start,
                end_us: end,
                note_count: count,
            });
        }
    }
    let mut recommendations = Vec::new();
    let mut reasons = Vec::new();
    let mut severity = "low";
    if schedule.impossible_same_key_repeats > 0 {
        severity = "high";
        reasons.push("infeasible same-key repeats");
        recommendations.push(format!(
            "{} same-key repeat(s) are infeasible under the materialized hold policy; sender-side timing does not prove game observation.",
            schedule.impossible_same_key_repeats
        ));
        recommendations.push(
            "Reduce tempo or edit the arrangement so the same key has more time between downs."
                .into(),
        );
    }
    if dense.len() > 5 {
        if severity != "high" {
            severity = if dense.len() > 15 { "high" } else { "medium" };
        }
        reasons.push("dense clusters");
        recommendations.push(format!(
            "Detected {} distinct dense cluster(s) (more than 6 notes in 100ms).",
            dense.len()
        ));
    }
    if schedule.max_polyphony > 8 {
        severity = if severity == "low" {
            "medium"
        } else {
            severity
        };
        reasons.push("high polyphony");
        recommendations.push(format!(
            "High polyphony detected (max {} simultaneous keys).",
            schedule.max_polyphony
        ));
    }
    let severity_before_short_gap = severity;
    if schedule.short_up_gap_repeat_count > 0 {
        if severity == "low" {
            severity = "medium";
        }
        reasons.push("short same-key Up gaps");
        recommendations.push(format!(
            "{} physically feasible same-key repeat(s) have an Up-to-Down gap shorter than one frame; the game may miss a short Up pulse.",
            schedule.short_up_gap_repeat_count
        ));
    }
    if let Some(interval) = schedule.shortest_same_key_interval_us {
        recommendations.push(format!(
            "Shortest same-key repeat interval: {:.1}ms.",
            interval as f64 / 1_000.0
        ));
    }
    let short_gap_only =
        schedule.short_up_gap_repeat_count > 0 && severity_before_short_gap == "low";
    let (suggested_hold_frames, suggested_tempo_scale) = if short_gap_only {
        // A short authored Up gap requires user acknowledgement. Automatically
        // changing hold or tempo would change the risk under review.
        (None, None)
    } else if schedule.impossible_same_key_repeats > 0 {
        (
            schedule.recommended_hold_frames,
            schedule.recommended_tempo_scale,
        )
    } else if schedule.max_polyphony >= 5
        || (!dense.is_empty() && severity_before_short_gap != "low")
    {
        (
            Some(1.5),
            Some(if severity_before_short_gap != "low" {
                current_tempo_scale.min(0.95)
            } else {
                current_tempo_scale
            }),
        )
    } else if severity_before_short_gap == "medium" {
        (Some(1.25), Some(current_tempo_scale.min(0.95)))
    } else {
        (Some(current_hold_frames), Some(current_tempo_scale))
    };
    let suggested_hold_frames = suggested_hold_frames.filter(|value| value.is_finite());
    let suggested_tempo_scale = if current_tempo_scale.is_finite() {
        suggested_tempo_scale.filter(|value| value.is_finite())
    } else {
        None
    };
    if suggested_hold_frames.is_none() && !current_hold_frames.is_finite() {
        recommendations.push(
            "Hold recommendation omitted because the current hold value was not finite.".into(),
        );
    }
    if suggested_tempo_scale.is_none() && !current_tempo_scale.is_finite() {
        recommendations.push(
            "Tempo recommendation omitted because the current tempo scale was not finite.".into(),
        );
    }
    if schedule.impossible_same_key_repeats > 0 {
        recommendations.push(
            "One or more same-key repeats are shorter than the required hold; reduce tempo or edit the arrangement."
                .into(),
        );
    }
    if recommendations.is_empty() {
        recommendations.push("No timing conflicts detected. Keep the selected hold.".into());
    }
    let mut min_any = downs
        .windows(2)
        .map(|pair| pair[1].at_us.saturating_sub(pair[0].at_us))
        .min();
    let mut min_same = schedule.shortest_same_key_interval_us;
    if let Some(raw_notes) = raw_notes {
        let mut onsets = raw_notes
            .iter()
            .map(|note| note.time_ms)
            .collect::<Vec<_>>();
        onsets.sort_unstable();
        onsets.dedup();
        min_any = onsets
            .windows(2)
            .filter_map(|pair| checked_note_gap_us(pair[0], pair[1]))
            .min()
            .or(min_any);

        let mut last_by_key = HashMap::<&str, i64>::new();
        for note in raw_notes {
            if let Some(previous) = last_by_key.insert(note.key.as_str(), note.time_ms)
                && let Some(gap) = checked_note_gap_us(previous, note.time_ms)
            {
                min_same = Some(min_same.map_or(gap, |current| current.min(gap)));
            }
        }
    }
    let duration = schedule.source_duration_us as f64 / 1_000_000.0;
    let average = if duration > 0.0 {
        downs.len() as f64 / duration
    } else {
        0.0
    };
    RiskReport {
        severity: severity.into(),
        reason: if reasons.is_empty() {
            "No timing conflicts detected.".into()
        } else {
            format!("{} detected", reasons.join(" and "))
        },
        recommendations,
        dense_clusters: dense,
        average_notes_per_second: average,
        peak_notes_per_second_1s: peak_density(downs),
        max_polyphony: schedule.max_polyphony,
        max_chord_size: downs
            .iter()
            .map(|chord| chord.scan_codes.len())
            .max()
            .unwrap_or_default(),
        chords_count: downs
            .iter()
            .filter(|chord| chord.scan_codes.len() > 1)
            .count(),
        timing_stress_rate: if schedule.note_count == 0 {
            0.0
        } else {
            schedule.impossible_same_key_repeats as f64 / schedule.note_count as f64 * 100.0
        },
        short_up_gap_repeat_count: schedule.short_up_gap_repeat_count,
        min_any_note_gap_us: min_any,
        min_same_key_gap_us: min_same,
        suggested_hold_frames,
        suggested_tempo_scale,
    }
}

fn peak_density(downs: &[OnsetChord]) -> f64 {
    let mut peak = 0usize;
    for (index, chord) in downs.iter().enumerate() {
        let count = downs[index..]
            .iter()
            .take_while(|next| next.at_us.saturating_sub(chord.at_us) <= 1_000_000)
            .count();
        peak = peak.max(count);
    }
    peak as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_accepts_legacy_aliases_and_stably_sorts() {
        let song = parse_song_json(br#"{"name":"Demo","songNotes":[{"time":20,"key":"1Key1"},{"time":"10","key":"Key0"}]}"#, "fallback").unwrap();
        assert_eq!(song.notes[0].time_ms, 10);
        assert_eq!(scan_code_for_key(&song.notes[1].key), Some(0x16));
    }

    #[test]
    fn schedule_preserves_chords_and_deduplicates_same_key_timestamp() {
        let song = Song {
            name: "Demo".into(),
            notes: vec![
                Note {
                    time_ms: 0,
                    key: "Key0".into(),
                    source_index: 0,
                },
                Note {
                    time_ms: 0,
                    key: "Key1".into(),
                    source_index: 1,
                },
                Note {
                    time_ms: 0,
                    key: "Key1".into(),
                    source_index: 2,
                },
            ],
        };
        let schedule = build_schedule(&song, 1.0, 1.0, 60).unwrap();
        assert_eq!(schedule.duplicate_note_count, 1);
        assert_eq!(schedule.chords[0].scan_codes, vec![0x15, 0x16]);
    }

    #[test]
    fn schedule_preserves_authored_chord_scan_order() {
        let song = Song {
            name: "Demo".into(),
            notes: vec![
                Note {
                    time_ms: 0,
                    key: "Key1".into(),
                    source_index: 0,
                },
                Note {
                    time_ms: 0,
                    key: "Key0".into(),
                    source_index: 1,
                },
            ],
        };
        let schedule = build_schedule(&song, 1.0, 1.0, 60).unwrap();
        assert_eq!(schedule.chords[0].scan_codes, vec![0x16, 0x15]);
    }

    #[test]
    fn same_key_repeat_requires_only_the_hold_floor_after_up() {
        let song = Song {
            name: "Repeated key".into(),
            notes: vec![
                Note {
                    time_ms: 0,
                    key: "Key0".into(),
                    source_index: 0,
                },
                Note {
                    time_ms: 35,
                    key: "Key0".into(),
                    source_index: 1,
                },
            ],
        };
        let margin_800 = crate::timing::MaterializedTimingPolicy::from_user_margin(60, 1.0, 800)
            .expect("valid margin");
        let margin_900 = crate::timing::MaterializedTimingPolicy::from_user_margin(60, 1.0, 900)
            .expect("valid margin");
        let feasible = build_schedule_with_policy(&song, 1.0, &margin_800).expect("schedule");
        assert_eq!(feasible.impossible_same_key_repeats, 0);
        assert_eq!(feasible.hold_us, margin_800.min_hold_us);

        let margin_change = build_schedule_with_policy(&song, 1.0, &margin_900).expect("schedule");
        assert_eq!(margin_change.impossible_same_key_repeats, 0);
        assert_eq!(margin_change.hold_us, margin_900.min_hold_us);
    }

    #[test]
    fn repeat_up_is_down_plus_h_and_equal_deadlines_order_up_before_down() {
        let policy = crate::timing::MaterializedTimingPolicy::from_user_margin(60, 1.0, 0)
            .expect("valid policy");
        let h = policy.min_hold_us;
        for (interval, conflicts, up_gap, short_gap_count) in
            [(h - 1, 1, 0, 0), (h, 0, 0, 1), (h + 1, 0, 1, 1)]
        {
            let schedule = build_schedule_from_scaled_onsets(
                vec![(0, SKY_SCAN_CODES[0], 0), (interval, SKY_SCAN_CODES[0], 1)],
                1.0,
                &policy,
            )
            .expect("schedule timestamps fit");
            assert_eq!(
                schedule.impossible_same_key_repeats, conflicts,
                "interval={interval}"
            );
            assert_eq!(schedule.hold_us, h, "interval={interval}");
            assert_eq!(schedule.chords.len(), 2, "interval={interval}");
            assert_eq!(schedule.chords[0].at_us, 0, "interval={interval}");
            assert_eq!(schedule.chords[1].at_us, interval, "interval={interval}");
            assert_eq!(schedule.min_same_key_up_gap_us, Some(up_gap));
            assert_eq!(
                schedule.short_up_gap_repeat_count, short_gap_count,
                "interval={interval}"
            );
            if interval == h {
                assert_eq!(schedule.max_polyphony, 1);
                let risk = analyze_schedule(&schedule);
                assert_eq!(risk.severity, "medium");
                assert_eq!(risk.short_up_gap_repeat_count, 1);
                assert!(risk.recommendations.iter().any(|recommendation| {
                    recommendation.contains("game may miss a short Up pulse")
                }));
            }
        }
    }

    #[test]
    fn short_gap_only_does_not_recommend_timing_changes_with_one_to_five_dense_clusters() {
        let policy = crate::timing::MaterializedTimingPolicy::from_user_margin(60, 1.0, 0)
            .expect("valid policy");
        for cluster_count in 1usize..=5 {
            let mut notes = Vec::new();
            let mut source_index = 0;
            for cluster in 0..cluster_count {
                let cluster_start_ms = i64::try_from(cluster * 200).expect("test time range");
                for (key_index, key) in KEY_NAMES.iter().take(7).enumerate() {
                    notes.push(Note {
                        time_ms: cluster_start_ms
                            + i64::try_from(key_index * 10).expect("test time range"),
                        key: (*key).into(),
                        source_index,
                    });
                    source_index += 1;
                }
                notes.push(Note {
                    time_ms: cluster_start_ms + 17,
                    key: KEY_NAMES[0].into(),
                    source_index,
                });
                source_index += 1;
            }
            let schedule = build_schedule_with_policy(
                &Song {
                    name: "Short Up gap with sparse dense clusters".into(),
                    notes,
                },
                1.0,
                &policy,
            )
            .expect("short-gap schedule");
            assert_eq!(schedule.short_up_gap_repeat_count, cluster_count);
            assert_eq!(schedule.impossible_same_key_repeats, 0);

            let risk = analyze_schedule(&schedule);
            assert_eq!(risk.dense_clusters.len(), cluster_count);
            assert!(risk.max_polyphony <= 8);
            assert_eq!(risk.severity, "medium");
            assert!(
                risk.recommendations
                    .iter()
                    .any(|reason| { reason.contains("game may miss a short Up pulse") })
            );
            assert_eq!(risk.suggested_hold_frames, None);
            assert_eq!(risk.suggested_tempo_scale, None);
        }
    }

    #[test]
    fn empty_song_has_the_materialized_hold_duration() {
        let schedule = build_schedule(
            &Song {
                name: "Empty".into(),
                notes: Vec::new(),
            },
            1.0,
            1.0,
            60,
        )
        .expect("empty songs produce an empty schedule");
        assert!(schedule.chords.is_empty());
        assert_eq!(schedule.source_duration_us, 17_167);
    }

    #[test]
    fn risk_reports_dense_cluster() {
        let song = Song {
            name: "Dense".into(),
            notes: (0usize..7)
                .map(|index| Note {
                    time_ms: index as i64,
                    key: KEY_NAMES[index].into(),
                    source_index: index,
                })
                .collect(),
        };
        let report = analyze_schedule(&build_schedule(&song, 1.0, 1.0, 60).unwrap());
        assert_eq!(report.dense_clusters.len(), 1);
    }

    #[test]
    fn legacy_timestamp_coercion_preserves_documented_json_values() {
        let song = parse_song_json(
            br#"{"songNotes":[{"time":true,"key":"Key0"},{"time":2.9,"key":"Key1"},{"time":" 3 ","key":"Key2"}]}"#,
            "fallback",
        )
        .expect("legacy timestamp forms remain accepted");
        assert_eq!(
            song.notes
                .iter()
                .map(|note| note.time_ms)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        let invalid_time = parse_song_json(
            br#"{"songNotes":[{"time":9223372036854775808,"key":"Key0"}]}"#,
            "fallback",
        );
        assert!(matches!(
            invalid_time,
            Err(SongError::InvalidTime { index: 0 })
        ));
    }
}
