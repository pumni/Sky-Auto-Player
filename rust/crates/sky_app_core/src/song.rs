//! Pure song parsing, deterministic scheduling, and bounded risk analysis.
//!
//! This module deliberately owns no filesystem, delivery, Win32, Python, or
//! realtime-player concerns.  A caller supplies bytes and receives an
//! immutable plan that an outer native adapter may hand to the qualified
//! realtime engine.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const SKY_SCAN_CODES: [u16; 15] = [
    0x15, 0x16, 0x17, 0x18, 0x19, 0x23, 0x24, 0x25, 0x26, 0x27, 0x31, 0x32, 0x33, 0x34, 0x35,
];
pub const KEY_NAMES: [&str; 15] = [
    "Key0", "Key1", "Key2", "Key3", "Key4", "Key5", "Key6", "Key7", "Key8", "Key9", "Key10",
    "Key11", "Key12", "Key13", "Key14",
];
pub const VALID_FPS: [u16; 7] = [30, 60, 90, 120, 144, 165, 240];
pub const HOLD_FRAMES: [f64; 3] = [1.0, 1.25, 1.5];
pub const MIN_TRANSPORT_MARGIN_US: u64 = 300;
pub const DOWN_LATE_GRACE_US: u64 = 500;

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
        let time_ms = python_int(raw_time).ok_or(SongError::InvalidTime { index })?;
        if time_ms < 0 {
            return Err(SongError::NegativeTime { index });
        }
        let raw_key = note.get("key").ok_or(SongError::MissingKey { index })?;
        let key = raw_key
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| SongError::UnmappedKey {
                index,
                key: python_string(raw_key),
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

fn python_int(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(value) => Some(i64::from(*value)),
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            .or_else(|| {
                number
                    .as_f64()
                    .map(f64::trunc)
                    .filter(|value| value.is_finite())
                    .and_then(|value| i64::try_from(value as i128).ok())
            }),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn python_string(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(value) => value.to_string().to_ascii_lowercase(),
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(python_string)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Down,
    Up,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyAction {
    pub at_us: u64,
    pub scan_codes: Vec<u16>,
    pub kind: ActionKind,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScheduleMetadata {
    pub actions: Vec<KeyAction>,
    pub source_duration_us: u64,
    pub playback_duration_us: u64,
    pub duration_us: u64,
    pub note_count: usize,
    pub deduplicated_note_count: usize,
    pub duplicate_note_count: usize,
    pub compressed_holds: usize,
    pub impossible_same_key_repeats: usize,
    pub risky_same_key_repeats: usize,
    pub max_polyphony: usize,
    pub shortest_same_key_interval_us: Option<u64>,
    pub min_same_key_up_gap_us: Option<u64>,
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
    if !tempo_scale.is_finite() || tempo_scale <= 0.0 {
        return Err(SongError::InvalidTempo);
    }
    if !hold_frames.is_finite() || !HOLD_FRAMES.contains(&hold_frames) {
        return Err(SongError::InvalidHold);
    }
    let frame = frame_us(fps)?;
    let base_hold = (hold_frames * frame as f64).ceil() as u64;
    let hold_us = base_hold
        .saturating_add(DOWN_LATE_GRACE_US)
        .saturating_add(MIN_TRANSPORT_MARGIN_US);
    let release_gap = frame
        .saturating_add(DOWN_LATE_GRACE_US)
        .saturating_add(MIN_TRANSPORT_MARGIN_US);

    let mut drafts = Vec::with_capacity(song.notes.len());
    for note in &song.notes {
        let scan_code = scan_code_for_key(&note.key).ok_or_else(|| SongError::UnmappedKey {
            index: note.source_index,
            key: note.key.clone(),
        })?;
        let at_us = python_round_nonnegative((note.time_ms as f64) * 1_000.0 / tempo_scale);
        drafts.push((at_us, scan_code, note.source_index));
    }
    let raw_count = drafts.len();
    let mut seen = BTreeSet::new();
    drafts.retain(|(at, scan, _)| seen.insert((*at, *scan)));

    let mut next_by_source = HashMap::<usize, Option<u64>>::new();
    let mut next = HashMap::<u16, u64>::new();
    for (at, scan, source_index) in drafts.iter().rev() {
        next_by_source.insert(*source_index, next.get(scan).copied());
        next.insert(*scan, *at);
    }

    let mut actions = Vec::with_capacity(drafts.len() * 2);
    let compressed = 0;
    let mut impossible = 0;
    let risky = 0;
    let mut shortest = None;
    let mut min_up_gap = None;
    for (at, scan, source_index) in &drafts {
        let next_at = next_by_source.get(source_index).copied().flatten();
        let actual_hold = if let Some(next_at) = next_at {
            let interval = next_at.saturating_sub(*at);
            shortest = Some(shortest.map_or(interval, |current: u64| current.min(interval)));
            let max_hold = interval.saturating_sub(release_gap);
            if max_hold < hold_us {
                impossible += 1;
                hold_us
            } else {
                hold_us
            }
        } else {
            hold_us
        };
        if let Some(next_at) = next_at {
            let gap = next_at.saturating_sub(*at).saturating_sub(actual_hold);
            min_up_gap = Some(min_up_gap.map_or(gap, |current: u64| current.min(gap)));
        }
        actions.push(KeyAction {
            at_us: *at,
            scan_codes: vec![*scan],
            kind: ActionKind::Down,
            reason: "onset".into(),
        });
        actions.push(KeyAction {
            at_us: at.saturating_add(actual_hold),
            scan_codes: vec![*scan],
            kind: ActionKind::Up,
            reason: if next_at.is_some() {
                "repeat_release".into()
            } else {
                "release".into()
            },
        });
    }
    let mut grouped = BTreeMap::<(u64, ActionKind), (Vec<u16>, BTreeSet<String>)>::new();
    for action in actions {
        let entry = grouped.entry((action.at_us, action.kind)).or_default();
        entry.0.extend(action.scan_codes);
        entry.1.insert(action.reason);
    }
    let mut final_actions = grouped
        .into_iter()
        .map(|((at_us, kind), (mut scans, reasons))| {
            let mut unique_scans = Vec::with_capacity(scans.len());
            for scan in scans.drain(..) {
                if !unique_scans.contains(&scan) {
                    unique_scans.push(scan);
                }
            }
            KeyAction {
                at_us,
                scan_codes: unique_scans,
                kind,
                reason: if reasons.len() == 1 {
                    reasons.into_iter().next().unwrap_or_default()
                } else {
                    "mixed".into()
                },
            }
        })
        .collect::<Vec<_>>();
    final_actions.sort_by_key(|action| (action.at_us, matches!(action.kind, ActionKind::Down)));
    let max_polyphony = max_polyphony(&final_actions);
    let duration = final_actions
        .last()
        .map(|action| action.at_us)
        .unwrap_or_default();
    Ok(ScheduleMetadata {
        actions: final_actions,
        source_duration_us: drafts
            .iter()
            .map(|(at, _, _)| *at)
            .max()
            .unwrap_or_default()
            .saturating_add(hold_us),
        playback_duration_us: duration,
        duration_us: duration,
        note_count: song.notes.len(),
        deduplicated_note_count: drafts.len(),
        duplicate_note_count: raw_count.saturating_sub(drafts.len()),
        compressed_holds: compressed,
        impossible_same_key_repeats: impossible,
        risky_same_key_repeats: risky,
        max_polyphony,
        shortest_same_key_interval_us: shortest,
        min_same_key_up_gap_us: min_up_gap,
        warnings: Vec::new(),
    })
}

/// Python's `round(float)` uses bankers rounding.  Schedule timestamps are
/// non-negative, so this small helper mirrors that observable rule without
/// pulling floating-point tie behavior from Rust's `round()`.
fn python_round_nonnegative(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    let lower = value.floor();
    let fraction = value - lower;
    let rounded = if fraction < 0.5 {
        lower
    } else if fraction > 0.5 || (lower as u64) % 2 == 1 {
        lower + 1.0
    } else {
        lower
    };
    rounded as u64
}

fn max_polyphony(actions: &[KeyAction]) -> usize {
    let mut active = BTreeSet::new();
    let mut maximum = 0;
    for action in actions {
        match action.kind {
            ActionKind::Down => {
                active.extend(action.scan_codes.iter().copied());
                maximum = maximum.max(active.len());
            }
            ActionKind::Up => {
                for scan in &action.scan_codes {
                    active.remove(scan);
                }
            }
        }
    }
    maximum
}

pub fn analyze_schedule(schedule: &ScheduleMetadata) -> RiskReport {
    analyze_schedule_with_context(schedule, None, 1.0, 1.0)
}

/// Analyze executable schedule metrics while retaining the raw authored notes
/// for the two Python-visible gap metrics.  The scheduler deliberately
/// deduplicates executable slots, but the Python analyzer computes minimum
/// onset gaps from the raw note stream when it is available.
pub fn analyze_schedule_with_notes(schedule: &ScheduleMetadata, raw_notes: &[Note]) -> RiskReport {
    analyze_schedule_with_context(schedule, Some(raw_notes), 1.0, 1.0)
}

pub fn analyze_schedule_with_context(
    schedule: &ScheduleMetadata,
    raw_notes: Option<&[Note]>,
    current_hold_frames: f64,
    current_tempo_scale: f64,
) -> RiskReport {
    let downs = schedule
        .actions
        .iter()
        .filter(|action| matches!(action.kind, ActionKind::Down))
        .collect::<Vec<_>>();
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
            "{} same-key repeat(s) are infeasible under the materialized hold/release policy; sender-side policy does not prove game observation.",
            schedule.impossible_same_key_repeats
        ));
        recommendations.push(
            "Reduce tempo or edit the arrangement so the same key has more time between downs."
                .into(),
        );
    }
    if schedule.risky_same_key_repeats > 0 {
        if severity == "low" {
            severity = "medium";
        }
        reasons.push("same-key hold compression");
        recommendations.push(format!(
            "Compressed {} same-key hold(s) to release before the next down.",
            schedule.risky_same_key_repeats
        ));
    }
    if schedule.compressed_holds > 5 {
        if severity == "low" {
            severity = "medium";
        }
        reasons.push("compressed holds");
        recommendations.push(format!(
            "{} note holds were compressed due to dense scheduling.",
            schedule.compressed_holds
        ));
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
    if let Some(interval) = schedule.shortest_same_key_interval_us {
        recommendations.push(format!(
            "Shortest same-key repeat interval: {:.1}ms.",
            interval as f64 / 1_000.0
        ));
    }
    let (suggested_hold_frames, suggested_tempo_scale) = if schedule.impossible_same_key_repeats > 0
    {
        (Some(1.0), Some(current_tempo_scale.min(0.92)))
    } else if schedule.risky_same_key_repeats > 5 || schedule.compressed_holds > 10 {
        (Some(1.0), Some(current_tempo_scale.min(0.95)))
    } else if schedule.max_polyphony >= 5 || (!dense.is_empty() && severity != "low") {
        (
            Some(1.5),
            Some(if severity != "low" {
                current_tempo_scale.min(0.95)
            } else {
                current_tempo_scale
            }),
        )
    } else if severity == "medium" {
        (Some(1.25), Some(current_tempo_scale.min(0.95)))
    } else {
        (Some(current_hold_frames), Some(current_tempo_scale))
    };
    if schedule.impossible_same_key_repeats > 0 {
        recommendations.push(
            "Even the shortest supported hold cannot make a sub-frame repeat feasible; reduce tempo or edit the arrangement."
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
            .map(|pair| pair[1].saturating_sub(pair[0]) as u64 * 1_000)
            .min()
            .or(min_any);

        let mut last_by_key = HashMap::<&str, i64>::new();
        for note in raw_notes {
            if let Some(previous) = last_by_key.insert(note.key.as_str(), note.time_ms) {
                let gap = note.time_ms.saturating_sub(previous) as u64 * 1_000;
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
        peak_notes_per_second_1s: peak_density(&downs),
        max_polyphony: schedule.max_polyphony,
        max_chord_size: downs
            .iter()
            .map(|action| action.scan_codes.len())
            .max()
            .unwrap_or_default(),
        chords_count: downs
            .iter()
            .filter(|action| action.scan_codes.len() > 1)
            .count(),
        timing_stress_rate: if schedule.note_count == 0 {
            0.0
        } else {
            schedule.impossible_same_key_repeats as f64 / schedule.note_count as f64 * 100.0
        },
        min_any_note_gap_us: min_any,
        min_same_key_gap_us: min_same,
        suggested_hold_frames,
        suggested_tempo_scale,
    }
}

fn peak_density(downs: &[&KeyAction]) -> f64 {
    let mut peak = 0usize;
    for (index, action) in downs.iter().enumerate() {
        let count = downs[index..]
            .iter()
            .take_while(|next| next.at_us.saturating_sub(action.at_us) <= 1_000_000)
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
        assert_eq!(schedule.actions[0].scan_codes, vec![0x15, 0x16]);
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
        assert_eq!(schedule.actions[0].scan_codes, vec![0x16, 0x15]);
    }

    #[test]
    fn empty_song_matches_python_empty_schedule_behavior() {
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
        assert!(schedule.actions.is_empty());
        assert_eq!(schedule.source_duration_us, 17_467);
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
}
