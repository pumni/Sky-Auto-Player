//! Deterministic conversion from decoded source events to Sky Piano onsets.
//!
//! The planner is pure domain code. It maps musical events to the fixed
//! fifteen-key layout and hands the resulting microsecond onsets to the
//! existing schedule builder; it does not implement a second timing engine.

use super::{
    CanonicalEvent, CanonicalPitch, CanonicalSong, DecodeWarning, Pitchability, RawNoteIdentity,
    RiskReport, SKY_SCAN_CODES, ScheduleMetadata, Song, SongDecodeError, SongError, SourcePosition,
    SourceTrack, analyze_schedule, analyze_schedule_with_notes, build_schedule_from_scaled_onsets,
    build_schedule_with_policy, decode_song_json, detect_song_format, parse_song_json,
};
use crate::timing::MaterializedTimingPolicy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Default mode for newly decoded, versioned sheets.
pub const DEFAULT_NEW_SHEET_MODE: ConversionMode = ConversionMode::MergePlayable;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConversionMode {
    /// Combine playable events from every source track.
    MergePlayable,
    /// Keep exactly the source track identified by its stable track index.
    SingleTrack { track_id: usize },
}

/// Pitch at which the target Sky Piano is assumed to be tuned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PianoPitch {
    #[serde(rename = "C")]
    C,
    #[serde(rename = "Db")]
    Db,
    #[serde(rename = "D")]
    D,
    #[serde(rename = "Eb")]
    Eb,
    #[serde(rename = "E")]
    E,
    #[serde(rename = "F")]
    F,
    #[serde(rename = "Gb")]
    Gb,
    #[serde(rename = "G")]
    G,
    #[serde(rename = "Ab")]
    Ab,
    #[serde(rename = "A")]
    A,
    #[serde(rename = "Bb")]
    Bb,
    #[serde(rename = "B")]
    B,
}

impl PianoPitch {
    fn from_source_pitch(value: &str) -> Option<Self> {
        match value {
            "C" => Some(Self::C),
            "Db" => Some(Self::Db),
            "D" => Some(Self::D),
            "Eb" => Some(Self::Eb),
            "E" => Some(Self::E),
            "F" => Some(Self::F),
            "Gb" => Some(Self::Gb),
            "G" => Some(Self::G),
            "Ab" => Some(Self::Ab),
            "A" => Some(Self::A),
            "Bb" => Some(Self::Bb),
            "B" => Some(Self::B),
            _ => None,
        }
    }

    fn semitone_offset(self) -> i32 {
        match self {
            Self::C => 0,
            Self::Db => 1,
            Self::D => 2,
            Self::Eb => 3,
            Self::E => 4,
            Self::F => 5,
            Self::Gb => 6,
            Self::G => 7,
            Self::Ab => 8,
            Self::A => 9,
            Self::Bb => 10,
            Self::B => 11,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PianoPlannerOptions {
    /// `None` selects [`DEFAULT_NEW_SHEET_MODE`].
    pub mode: Option<ConversionMode>,
    /// `None` uses the source song pitch. No actual in-game tuning is detected.
    pub target_pitch: Option<PianoPitch>,
    /// Applied once by the existing schedule builder.
    pub tempo_scale: f64,
}

impl Default for PianoPlannerOptions {
    fn default() -> Self {
        Self {
            mode: None,
            target_pitch: None,
            tempo_scale: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetPitchSelection {
    Explicit,
    SourceSongPitch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PianoTargetPitch {
    pub pitch: PianoPitch,
    pub selection: TargetPitchSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    Ready,
    RequiresConfirmation,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanLossReason {
    TrackSelectionOmission,
    SourceUnpitched,
    SourceUnresolved,
    TargetPianoUnplayable,
    AuthoredSustainNotReproduced,
    TempoQuantizationCollapse,
    NoPlayableNotes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceEventProvenance {
    pub source_index: usize,
    pub track_id: usize,
    pub source_onset_us: u64,
    pub instrument: Option<String>,
    pub source_position: SourcePosition,
    pub raw_identity: RawNoteIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanLoss {
    pub reason: PlanLossReason,
    /// Empty only for the aggregate `NoPlayableNotes` result.
    pub source_event: Option<SourceEventProvenance>,
}

/// One deduplicated physical key onset with every source event that produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PianoOnset {
    /// First authored onset represented by this deduplicated physical event.
    pub onset_us: u64,
    /// Resulting schedule time after the requested tempo scale is applied once.
    pub scheduled_onset_us: u64,
    pub button_index: u8,
    pub scan_code: u16,
    pub provenance: Vec<SourceEventProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PianoPlanDiagnostics {
    /// Serialized source note tuples before legacy layer expansion.
    pub source_tuple_count: usize,
    /// Canonical events after decoder layer expansion.
    pub decoded_event_count: usize,
    pub selected_source_event_count: usize,
    pub omitted_source_event_count: usize,
    /// Selected events that have a source-resolved pitch or legacy physical key.
    pub resolved_event_count: usize,
    /// Resolved source events mapped before same-onset/key deduplication.
    pub mapped_source_event_count: usize,
    pub mapped_onset_count: usize,
    /// Mapped source events collapsed from a distinct authored onset by scaling.
    pub tempo_quantization_collapse_count: usize,
    /// Mapped events merged at the same authored onset and target button.
    pub merged_duplicate_count: usize,
    pub source_unpitched_count: usize,
    pub source_unresolved_count: usize,
    pub target_unplayable_count: usize,
    /// Existing scheduler's infeasible same-key repeat count.
    pub physical_timing_conflict_count: usize,
    /// Mapped source notes whose authored duration is not reproduced by Piano.
    pub authored_sustain_lost_count: usize,
    pub losses: Vec<PlanLoss>,
    /// Decoder warnings remain separate from physical timing risk.
    pub source_warnings: Vec<DecodeWarning>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PianoPlan {
    pub name: String,
    /// Authoritative versioned family selected by the decoder.
    pub format: super::SongFormat,
    /// Decoder-resolved source roster used by the UI for explicit selection.
    pub source_tracks: Vec<SourceTrack>,
    pub mode: ConversionMode,
    pub target_pitch: PianoTargetPitch,
    pub status: PlanStatus,
    /// Stable ascending-onset order. Equal-onset order follows source identity.
    pub events: Vec<PianoOnset>,
    pub diagnostics: PianoPlanDiagnostics,
    /// Absent only when no playable physical onset exists.
    pub schedule: Option<ScheduleMetadata>,
    /// Existing physical risk analysis, kept separate from source losses.
    pub physical_risk: Option<RiskReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegacyCompatibilityPlan {
    pub status: PlanStatus,
    pub song: Song,
    pub schedule: ScheduleMetadata,
    pub physical_risk: RiskReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "path", content = "plan", rename_all = "snake_case")]
pub enum SongPlan {
    /// Uses the unchanged `parse_song_json` and `build_schedule_with_policy` path.
    LegacyCompatibility(LegacyCompatibilityPlan),
    PianoConversion(PianoPlan),
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PlannerError {
    #[error(transparent)]
    Source(#[from] SongDecodeError),
    #[error(transparent)]
    Schedule(#[from] SongError),
    #[error("canonical source event {event_index} refers to missing track {track_id}")]
    InvalidSourceTrack { event_index: usize, track_id: usize },
    #[error("canonical source track id {track_id} is duplicated")]
    AmbiguousTrackId { track_id: usize },
    #[error("selected track id {track_id} is not present in the source roster")]
    InvalidTrackSelection { track_id: usize },
    #[error("canonical legacy key index {key_index} is outside the Sky 15-key layout")]
    InvalidLegacyKeyIndex { key_index: u8 },
    #[error("canonical source pitch `{value}` cannot be used as the default target pitch")]
    InvalidSourcePitch { value: String },
    #[error(
        "scaled onset for source event {source_index} on track {track_id} at {source_onset_us} us is outside the u64 schedule range"
    )]
    ScaledOnsetOutOfRange {
        source_index: usize,
        track_id: usize,
        source_onset_us: u64,
    },
    #[error(
        "scaled onset {scheduled_onset_us} us for source event {source_index} on track {track_id} cannot accommodate the required {minimum_hold_us} us physical hold"
    )]
    ReleaseTimeOutOfRange {
        source_index: usize,
        track_id: usize,
        scheduled_onset_us: u64,
        minimum_hold_us: u64,
    },
}

/// Selects the unchanged legacy compatibility route or plans a versioned sheet.
/// A malformed authoritative versioned sheet is returned as an error and is
/// never retried through the legacy `songNotes` projection.
pub fn build_song_plan(
    bytes: &[u8],
    fallback_name: &str,
    options: &PianoPlannerOptions,
    policy: &MaterializedTimingPolicy,
) -> Result<SongPlan, PlannerError> {
    let format = detect_song_format(bytes)?;
    if format == super::SongFormat::LegacyFlat {
        let song = parse_song_json(bytes, fallback_name)?;
        let schedule = build_schedule_with_policy(&song, options.tempo_scale, policy)?;
        let physical_risk = analyze_schedule_with_notes(&schedule, &song.notes);
        return Ok(SongPlan::LegacyCompatibility(LegacyCompatibilityPlan {
            status: PlanStatus::Ready,
            song,
            schedule,
            physical_risk,
        }));
    }

    let source = decode_song_json(bytes, fallback_name)?;
    let plan = plan_canonical_song(&source, options, policy)?;
    Ok(SongPlan::PianoConversion(plan))
}

/// Maps canonical source events and passes exact source-microsecond onsets to
/// the existing scheduler. Conversion mode defaults to `MergePlayable`.
pub fn plan_canonical_song(
    source: &CanonicalSong,
    options: &PianoPlannerOptions,
    policy: &MaterializedTimingPolicy,
) -> Result<PianoPlan, PlannerError> {
    let mode = options.mode.unwrap_or(DEFAULT_NEW_SHEET_MODE);
    let target_pitch = match options.target_pitch {
        Some(pitch) => PianoTargetPitch {
            pitch,
            selection: TargetPitchSelection::Explicit,
        },
        None => PianoTargetPitch {
            pitch: PianoPitch::from_source_pitch(&source.source_pitch).ok_or_else(|| {
                PlannerError::InvalidSourcePitch {
                    value: source.source_pitch.clone(),
                }
            })?,
            selection: TargetPitchSelection::SourceSongPitch,
        },
    };
    let target_note_numbers = (0..SKY_SCAN_CODES.len())
        .map(|key_index| {
            super::decode::sky_piano_note_number(key_index, target_pitch.pitch.semitone_offset())
                .expect("the fixed Sky Piano table has fifteen entries")
        })
        .collect::<Vec<_>>();

    if !options.tempo_scale.is_finite() || options.tempo_scale <= 0.0 {
        return Err(SongError::InvalidTempo.into());
    }

    let mut track_ids = BTreeMap::<usize, usize>::new();
    for track in &source.tracks {
        let count = track_ids.entry(track.index).or_default();
        *count += 1;
        if *count > 1 {
            return Err(PlannerError::AmbiguousTrackId {
                track_id: track.index,
            });
        }
    }

    if let ConversionMode::SingleTrack { track_id } = mode {
        let matches = source
            .tracks
            .iter()
            .filter(|track| track.index == track_id)
            .count();
        if matches == 0 {
            return Err(PlannerError::InvalidTrackSelection { track_id });
        }
        if matches > 1 {
            return Err(PlannerError::AmbiguousTrackId { track_id });
        }
    }

    let mut ordered_events = source.events.iter().collect::<Vec<_>>();
    ordered_events.sort_by_key(|event| event_order_key(event));
    let mut diagnostics = PianoPlanDiagnostics {
        source_tuple_count: source.diagnostics.source_note_count,
        decoded_event_count: source.events.len(),
        selected_source_event_count: 0,
        omitted_source_event_count: 0,
        resolved_event_count: 0,
        mapped_source_event_count: 0,
        mapped_onset_count: 0,
        tempo_quantization_collapse_count: 0,
        merged_duplicate_count: 0,
        source_unpitched_count: 0,
        source_unresolved_count: 0,
        target_unplayable_count: 0,
        physical_timing_conflict_count: 0,
        authored_sustain_lost_count: 0,
        losses: Vec::new(),
        source_warnings: source.diagnostics.warnings.clone(),
    };
    let mut events = Vec::<PianoOnset>::new();
    let mut onset_indices = BTreeMap::<(u64, u8), usize>::new();

    for event in ordered_events {
        let track = source
            .tracks
            .iter()
            .find(|track| track.index == event.track_index)
            .ok_or(PlannerError::InvalidSourceTrack {
                event_index: event.source_index,
                track_id: event.track_index,
            })?;
        let provenance = source_provenance(event, track);
        if let ConversionMode::SingleTrack { track_id } = mode
            && event.track_index != track_id
        {
            diagnostics.omitted_source_event_count += 1;
            diagnostics.losses.push(PlanLoss {
                reason: PlanLossReason::TrackSelectionOmission,
                source_event: Some(provenance),
            });
            continue;
        }
        diagnostics.selected_source_event_count += 1;

        let button_index = match &event.pitch {
            CanonicalPitch::SkyKeyIndex(index) => {
                if usize::from(*index) >= SKY_SCAN_CODES.len() {
                    return Err(PlannerError::InvalidLegacyKeyIndex { key_index: *index });
                }
                diagnostics.resolved_event_count += 1;
                Some(*index)
            }
            CanonicalPitch::NoteNumber {
                number,
                source_pitchability: Pitchability::Pitched,
            } => {
                diagnostics.resolved_event_count += 1;
                target_note_numbers
                    .iter()
                    .position(|target_number| *target_number == *number)
                    .map(|index| index as u8)
                    .or_else(|| {
                        diagnostics.target_unplayable_count += 1;
                        diagnostics.losses.push(PlanLoss {
                            reason: PlanLossReason::TargetPianoUnplayable,
                            source_event: Some(provenance.clone()),
                        });
                        None
                    })
            }
            CanonicalPitch::NoteNumber {
                source_pitchability: Pitchability::Unpitched,
                ..
            } => {
                diagnostics.source_unpitched_count += 1;
                diagnostics.losses.push(PlanLoss {
                    reason: PlanLossReason::SourceUnpitched,
                    source_event: Some(provenance.clone()),
                });
                None
            }
            CanonicalPitch::NoteNumber {
                source_pitchability: Pitchability::Unresolved,
                ..
            } => {
                diagnostics.source_unresolved_count += 1;
                diagnostics.losses.push(PlanLoss {
                    reason: PlanLossReason::SourceUnresolved,
                    source_event: Some(provenance.clone()),
                });
                None
            }
        };
        let Some(button_index) = button_index else {
            continue;
        };

        diagnostics.mapped_source_event_count += 1;
        if event.duration_us > 0 {
            diagnostics.authored_sustain_lost_count += 1;
            diagnostics.losses.push(PlanLoss {
                reason: PlanLossReason::AuthoredSustainNotReproduced,
                source_event: Some(provenance.clone()),
            });
        }
        let scheduled_onset_us = checked_scale_source_onset_us(event.onset_us, options.tempo_scale)
            .ok_or(PlannerError::ScaledOnsetOutOfRange {
                source_index: event.source_index,
                track_id: event.track_index,
                source_onset_us: event.onset_us,
            })?;
        scheduled_onset_us.checked_add(policy.min_hold_us).ok_or(
            PlannerError::ReleaseTimeOutOfRange {
                source_index: event.source_index,
                track_id: event.track_index,
                scheduled_onset_us,
                minimum_hold_us: policy.min_hold_us,
            },
        )?;
        if let Some(existing_index) = onset_indices.get(&(scheduled_onset_us, button_index)) {
            let existing = &mut events[*existing_index];
            let simultaneous_source_onset = existing
                .provenance
                .iter()
                .any(|source| source.source_onset_us == event.onset_us);
            if simultaneous_source_onset {
                diagnostics.merged_duplicate_count += 1;
            } else {
                diagnostics.tempo_quantization_collapse_count += 1;
                diagnostics.losses.push(PlanLoss {
                    reason: PlanLossReason::TempoQuantizationCollapse,
                    source_event: Some(provenance.clone()),
                });
            }
            existing.provenance.push(provenance);
        } else {
            let event_index = events.len();
            onset_indices.insert((scheduled_onset_us, button_index), event_index);
            events.push(PianoOnset {
                onset_us: event.onset_us,
                scheduled_onset_us,
                button_index,
                scan_code: SKY_SCAN_CODES[usize::from(button_index)],
                provenance: vec![provenance],
            });
        }
    }

    diagnostics.mapped_onset_count = events.len();
    let (status, schedule, physical_risk) = if events.is_empty() {
        if diagnostics.losses.is_empty() {
            diagnostics.losses.push(PlanLoss {
                reason: PlanLossReason::NoPlayableNotes,
                source_event: None,
            });
        }
        (PlanStatus::Unsupported, None, None)
    } else {
        let schedule_inputs = events
            .iter()
            .enumerate()
            .map(|(index, event)| (event.scheduled_onset_us, event.scan_code, index))
            .collect();
        let schedule =
            build_schedule_from_scaled_onsets(schedule_inputs, options.tempo_scale, policy)?;
        diagnostics.physical_timing_conflict_count = schedule.impossible_same_key_repeats;
        let physical_risk = analyze_schedule(&schedule);
        let status = if diagnostics.losses.is_empty() {
            PlanStatus::Ready
        } else {
            PlanStatus::RequiresConfirmation
        };
        (status, Some(schedule), Some(physical_risk))
    };

    Ok(PianoPlan {
        name: source.name.clone(),
        format: source.format,
        source_tracks: source.tracks.clone(),
        mode,
        target_pitch,
        status,
        events,
        diagnostics,
        schedule,
        physical_risk,
    })
}

fn checked_scale_source_onset_us(source_at_us: u64, tempo_scale: f64) -> Option<u64> {
    if tempo_scale == 1.0 {
        return Some(source_at_us);
    }

    let scaled = source_at_us as f64 / tempo_scale;
    if !scaled.is_finite() || scaled < 0.0 || scaled >= u64::MAX as f64 {
        return None;
    }

    let lower = scaled.floor();
    let fraction = scaled - lower;
    let lower_integer = lower as u64;
    let rounded = if fraction < 0.5 {
        lower
    } else if fraction > 0.5 || lower_integer % 2 == 1 {
        lower + 1.0
    } else {
        lower
    };
    if !rounded.is_finite() || !(0.0..u64::MAX as f64).contains(&rounded) {
        return None;
    }
    Some(rounded as u64)
}

fn source_provenance(event: &CanonicalEvent, track: &SourceTrack) -> SourceEventProvenance {
    SourceEventProvenance {
        source_index: event.source_index,
        track_id: event.track_index,
        source_onset_us: event.onset_us,
        instrument: track.instrument.clone(),
        source_position: event.source_position.clone(),
        raw_identity: event.raw_identity.clone(),
    }
}

type EventOrderKey = (
    u64,
    usize,
    usize,
    (u8, usize, usize, usize),
    (u8, i64),
    (u8, i32, u8),
    u64,
    u16,
    usize,
);

fn event_order_key(event: &CanonicalEvent) -> EventOrderKey {
    let position = match &event.source_position {
        SourcePosition::FlatSongNote { note_index } => (0, *note_index, 0, 0),
        SourcePosition::ComposedColumn {
            column_index,
            note_index,
        } => (1, *column_index, *note_index, 0),
        SourcePosition::RecordedNote { note_index } => (2, *note_index, 0, 0),
        SourcePosition::TrackNote {
            track_index,
            note_index,
            column_index,
        } => (
            3,
            *track_index,
            *note_index,
            column_index.unwrap_or(usize::MAX),
        ),
    };
    let raw_identity = match &event.raw_identity {
        RawNoteIdentity::LegacyKeyIndex(index) => (0, i64::from(*index)),
        RawNoteIdentity::LegacyIndex(index) => (1, i64::from(*index)),
        RawNoteIdentity::NominalId(number) => (2, i64::from(*number)),
        RawNoteIdentity::AbsoluteNoteNumber(number) => (3, i64::from(*number)),
    };
    let pitch = match &event.pitch {
        CanonicalPitch::SkyKeyIndex(index) => (0, i32::from(*index), 0),
        CanonicalPitch::NoteNumber {
            number,
            source_pitchability,
        } => (
            1,
            *number,
            match source_pitchability {
                Pitchability::Pitched => 0,
                Pitchability::Unpitched => 1,
                Pitchability::Unresolved => 2,
            },
        ),
    };
    (
        event.onset_us,
        event.source_index,
        event.track_index,
        position,
        raw_identity,
        pitch,
        event.duration_us,
        event.layer_mask.unwrap_or_default(),
        event.span_columns.unwrap_or_default(),
    )
}
