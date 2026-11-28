use super::{Note, OnsetChord, ScheduleMetadata, checked_note_gap_us};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    pub min_same_key_slack_us: Option<u64>,
    pub same_key_repeat_count: usize,
    pub zero_slack_repeat_count: usize,
    pub min_any_note_gap_us: Option<u64>,
    pub min_same_key_gap_us: Option<u64>,
    pub suggested_hold_frames: Option<f64>,
    pub suggested_tempo_scale: Option<f64>,
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
    if let Some(interval) = schedule.shortest_same_key_interval_us {
        recommendations.push(format!(
            "Shortest same-key repeat interval: {:.1}ms.",
            interval as f64 / 1_000.0
        ));
    }
    let (suggested_hold_frames, suggested_tempo_scale) = if schedule.impossible_same_key_repeats > 0
    {
        (
            schedule.recommended_hold_frames,
            schedule.recommended_tempo_scale,
        )
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
        min_same_key_slack_us: schedule.min_same_key_slack_us,
        same_key_repeat_count: schedule.same_key_repeat_count,
        zero_slack_repeat_count: schedule.zero_slack_repeat_count,
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
