use super::{
    Song, SongError, VALID_FPS, bounded_tempo_scale_recommendation, round_scaled_onset_checked,
    scan_code_for_key,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

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
    /// Minimum nominal same-key scheduling slack (`I - H`) in microseconds across
    /// feasible same-key repeats (`I >= H`). `None` when there are no feasible
    /// same-key repeats. Diagnostic only; imposes no runtime gate.
    pub min_same_key_slack_us: Option<u64>,
    /// Number of feasible same-key repeats (`I >= H`).
    pub same_key_repeat_count: usize,
    /// Number of feasible same-key repeats with zero nominal slack (`I == H`).
    pub zero_slack_repeat_count: usize,
    pub recommended_hold_frames: Option<f64>,
    pub recommended_tempo_scale: Option<f64>,
    pub warnings: Vec<String>,
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
    // All onset chords share the materialized hold floor H; nominal same-key
    // scheduling slack (I - H) and polyphony below are diagnostic only.
    let hold_us = policy.min_hold_us;
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
    let mut same_key_repeat_count = 0;
    let mut zero_slack_repeat_count = 0;
    let mut shortest = None;
    let mut min_same_key_slack_us = None;
    for (at, scan, source_index) in &drafts {
        let next_at = next_by_source.get(source_index).copied().flatten();
        if let Some(next_at) = next_at {
            let interval = next_at.saturating_sub(*at);
            shortest = Some(shortest.map_or(interval, |current: u64| current.min(interval)));
            if interval < hold_us {
                impossible += 1;
            } else {
                same_key_repeat_count += 1;
                let slack = interval - hold_us;
                if slack == 0 {
                    zero_slack_repeat_count += 1;
                }
                min_same_key_slack_us =
                    Some(min_same_key_slack_us.map_or(slack, |current: u64| current.min(slack)));
            }
        }
        // Validate that every onset can accommodate the materialized pulse hold.
        at.checked_add(hold_us).ok_or(SongError::TimingOverflow)?;
        let entry = chords_by_time.entry(*at).or_default();
        if !entry.contains(scan) {
            entry.push(*scan);
        }
    }
    let chords = chords_by_time
        .into_iter()
        .map(|(at_us, scan_codes)| OnsetChord { at_us, scan_codes })
        .collect::<Vec<_>>();
    let max_polyphony = max_polyphony(&chords, hold_us);
    let duration = chords
        .last()
        .map(|chord| {
            chord
                .at_us
                .checked_add(hold_us)
                .ok_or(SongError::TimingOverflow)
        })
        .transpose()?
        .unwrap_or_default();
    let mut warnings = Vec::new();
    let (recommended_hold_frames, recommended_tempo_scale) = if impossible > 0 {
        if let Some(shortest) = shortest {
            let suggestion = bounded_tempo_scale_recommendation(if hold_us == 0 {
                tempo_scale
            } else {
                tempo_scale * shortest as f64 / hold_us as f64
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
        .checked_add(hold_us)
        .ok_or(SongError::TimingOverflow)?;
    Ok(ScheduleMetadata {
        chords,
        hold_us,
        source_duration_us,
        playback_duration_us: duration,
        duration_us: duration,
        note_count: raw_count,
        deduplicated_note_count: drafts.len(),
        duplicate_note_count: raw_count.saturating_sub(drafts.len()),
        impossible_same_key_repeats: impossible,
        max_polyphony,
        shortest_same_key_interval_us: shortest,
        min_same_key_slack_us,
        same_key_repeat_count,
        zero_slack_repeat_count,
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
