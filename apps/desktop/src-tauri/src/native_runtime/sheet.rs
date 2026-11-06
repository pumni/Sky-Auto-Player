use crate::commands::{
    PlaybackAdmission, PlaybackConfigDto, PreparedPlaybackDto, RiskSummaryDto, SheetAnalysisDto,
    SheetConversionModeDto, SheetPipelineStageDto, SheetPitchDto, SheetReadinessDto, SheetTrackDto,
    SongDetailDto,
};
use sha2::{Digest, Sha256};
use sky_app_core::song::{
    ConversionMode, PianoPitch, PianoPlan, PlanLossReason, PlanStatus, PlannerError, RiskReport,
    ScheduleMetadata, Song, SongDecodeError, analyze_schedule_with_context,
};
use std::collections::BTreeSet;
use std::path::Path;

pub(super) const TIMING_RECONSENT_NOTE: &str = "To use changed timing, update the playback profile, prepare again, and review conversion consent again.";

pub(super) struct SheetConfirmationIdentity<'a> {
    pub(super) song_id: &'a str,
    pub(super) generation: u64,
    pub(super) path: &'a Path,
    pub(super) bytes: &'a [u8],
    pub(super) mode: Option<SheetConversionModeDto>,
    pub(super) pitch: Option<SheetPitchDto>,
    pub(super) fps: u16,
    pub(super) hold_frames: f64,
    pub(super) timing_margin_us: u64,
    pub(super) tempo_scale: f64,
}

pub(super) fn blocked_sheet_prepare(
    song_id: &str,
    title: &str,
    config: PlaybackConfigDto,
    sheet_analysis: SheetAnalysisDto,
) -> PreparedPlaybackDto {
    let message = sheet_analysis
        .error_message
        .clone()
        .unwrap_or_else(|| "No playable 15-key schedule could be built for this sheet.".into());
    let risk = RiskSummaryDto {
        level: "unknown".into(),
        headline: "Sheet cannot be prepared".into(),
        reasons: vec![message.clone()],
        recommendations: Vec::new(),
    };
    let error_code = match sheet_analysis.readiness {
        SheetReadinessDto::Unsupported => "unsupported_sheet",
        SheetReadinessDto::Malformed => "malformed_sheet",
        _ => "sheet_unplayable",
    };
    PreparedPlaybackDto {
        prepared_id: None,
        song: SongDetailDto {
            song_id: song_id.to_owned(),
            title: title.to_owned(),
            duration_us: 0,
            note_count: sheet_analysis.source_note_count.unwrap_or_default(),
            format_label: sheet_analysis.format_label.clone(),
            risk: risk.clone(),
            recommendation: None,
            sheet_analysis: Some(sheet_analysis),
        },
        config,
        admission: PlaybackAdmission::Blocked,
        risk,
        decisions: Vec::new(),
        plan_fingerprint: None,
        variants: Vec::new(),
        error_code: Some(error_code.into()),
        error_message: Some(message),
    }
}

pub(super) fn planner_options(
    conversion_mode: Option<SheetConversionModeDto>,
    target_pitch: Option<SheetPitchDto>,
    tempo_scale: f64,
) -> sky_app_core::song::PianoPlannerOptions {
    sky_app_core::song::PianoPlannerOptions {
        mode: conversion_mode.map(|mode| match mode {
            SheetConversionModeDto::MergePlayable => ConversionMode::MergePlayable,
            SheetConversionModeDto::SingleTrack { track_id } => ConversionMode::SingleTrack {
                track_id: track_id as usize,
            },
        }),
        target_pitch: target_pitch.map(sheet_pitch_to_core),
        tempo_scale,
    }
}

fn sheet_pitch_to_core(pitch: SheetPitchDto) -> PianoPitch {
    match pitch {
        SheetPitchDto::C => PianoPitch::C,
        SheetPitchDto::Db => PianoPitch::Db,
        SheetPitchDto::D => PianoPitch::D,
        SheetPitchDto::Eb => PianoPitch::Eb,
        SheetPitchDto::E => PianoPitch::E,
        SheetPitchDto::F => PianoPitch::F,
        SheetPitchDto::Gb => PianoPitch::Gb,
        SheetPitchDto::G => PianoPitch::G,
        SheetPitchDto::Ab => PianoPitch::Ab,
        SheetPitchDto::A => PianoPitch::A,
        SheetPitchDto::Bb => PianoPitch::Bb,
        SheetPitchDto::B => PianoPitch::B,
    }
}

fn core_pitch_to_sheet(pitch: PianoPitch) -> SheetPitchDto {
    match pitch {
        PianoPitch::C => SheetPitchDto::C,
        PianoPitch::Db => SheetPitchDto::Db,
        PianoPitch::D => SheetPitchDto::D,
        PianoPitch::Eb => SheetPitchDto::Eb,
        PianoPitch::E => SheetPitchDto::E,
        PianoPitch::F => SheetPitchDto::F,
        PianoPitch::Gb => SheetPitchDto::Gb,
        PianoPitch::G => SheetPitchDto::G,
        PianoPitch::Ab => SheetPitchDto::Ab,
        PianoPitch::A => SheetPitchDto::A,
        PianoPitch::Bb => SheetPitchDto::Bb,
        PianoPitch::B => SheetPitchDto::B,
    }
}

fn core_mode_to_sheet(mode: ConversionMode) -> SheetConversionModeDto {
    match mode {
        ConversionMode::MergePlayable => SheetConversionModeDto::MergePlayable,
        ConversionMode::SingleTrack { track_id } => SheetConversionModeDto::SingleTrack {
            track_id: u32::try_from(track_id).unwrap_or(u32::MAX),
        },
    }
}

pub(super) fn piano_plan_risk(
    plan: &PianoPlan,
    hold_frames: f64,
    tempo_scale: f64,
) -> Option<RiskReport> {
    plan.schedule
        .as_ref()
        .map(|schedule| analyze_schedule_with_context(schedule, None, hold_frames, tempo_scale))
}

pub(super) fn timing_reconsent_required(
    risk: &RiskReport,
    hold_frames: f64,
    tempo_scale: f64,
) -> bool {
    matches!(
        (risk.suggested_hold_frames, risk.suggested_tempo_scale),
        (Some(hold), Some(tempo)) if hold != hold_frames || tempo != tempo_scale
    )
}

pub(super) fn piano_plan_sheet_analysis(
    plan: &PianoPlan,
    confirmation_fingerprint: Option<String>,
    risk: Option<&RiskReport>,
) -> SheetAnalysisDto {
    let format_label = match plan.format {
        sky_app_core::song::SongFormat::LegacyFlat => "Legacy flat".to_owned(),
        sky_app_core::song::SongFormat::Composed { version } => format!("Composed v{version}"),
        sky_app_core::song::SongFormat::Recorded { version } => format!("Recorded v{version}"),
    };
    let schema_version = match plan.format {
        sky_app_core::song::SongFormat::LegacyFlat => None,
        sky_app_core::song::SongFormat::Composed { version }
        | sky_app_core::song::SongFormat::Recorded { version } => Some(version),
    };
    let readiness = match plan.status {
        PlanStatus::Unsupported => SheetReadinessDto::Unplayable,
        PlanStatus::RequiresConfirmation => SheetReadinessDto::RequiresConfirmation,
        PlanStatus::Ready if risk.is_some_and(|risk| risk.severity != "low") => {
            SheetReadinessDto::PhysicalRisk
        }
        PlanStatus::Ready => SheetReadinessDto::Ready,
    };
    let loss_reasons = plan
        .diagnostics
        .losses
        .iter()
        .map(|loss| match loss.reason {
            PlanLossReason::TrackSelectionOmission => "Events from unselected tracks are omitted.",
            PlanLossReason::SourceUnpitched => "Some source events have no playable pitch.",
            PlanLossReason::SourceUnresolved => "Some source pitches could not be resolved.",
            PlanLossReason::TargetPianoUnplayable => {
                "Some pitches fall outside the selected Piano range."
            }
            PlanLossReason::AuthoredSustainNotReproduced => {
                "Authored sustain is not reproduced by Piano one-shots."
            }
            PlanLossReason::TempoQuantizationCollapse => {
                "Different authored onsets collapse to the same scheduled onset."
            }
            PlanLossReason::NoPlayableNotes => "No source events map to a playable Piano key.",
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(str::to_owned)
        .collect();
    SheetAnalysisDto {
        stage: if plan.schedule.is_some() {
            SheetPipelineStageDto::ScheduleAdmitted
        } else if plan.events.is_empty() {
            SheetPipelineStageDto::Parsed
        } else {
            SheetPipelineStageDto::MappedTo15Key
        },
        readiness,
        format_label,
        schema_version,
        source_note_count: Some(plan.diagnostics.source_tuple_count as u64),
        source_track_count: Some(plan.source_tracks.len() as u64),
        tracks: plan
            .source_tracks
            .iter()
            .map(|track| SheetTrackDto {
                track_id: u32::try_from(track.index).unwrap_or(u32::MAX),
                instrument: track.instrument.clone(),
                effective_pitch: track.effective_pitch.clone(),
            })
            .collect(),
        conversion_mode: Some(core_mode_to_sheet(plan.mode)),
        target_pitch: Some(core_pitch_to_sheet(plan.target_pitch.pitch)),
        target_pitch_assumed_from_source: plan.target_pitch.selection
            == sky_app_core::song::TargetPitchSelection::SourceSongPitch,
        output_onset_count: Some(plan.diagnostics.mapped_onset_count as u64),
        merged_duplicate_count: Some(plan.diagnostics.merged_duplicate_count as u64),
        tempo_quantization_collapse_count: Some(
            plan.diagnostics.tempo_quantization_collapse_count as u64,
        ),
        source_unpitched_count: Some(plan.diagnostics.source_unpitched_count as u64),
        source_unresolved_count: Some(plan.diagnostics.source_unresolved_count as u64),
        target_unplayable_count: Some(plan.diagnostics.target_unplayable_count as u64),
        omitted_source_event_count: Some(plan.diagnostics.omitted_source_event_count as u64),
        authored_sustain_lost_count: Some(plan.diagnostics.authored_sustain_lost_count as u64),
        impossible_same_key_repeats: Some(plan.diagnostics.physical_timing_conflict_count as u64),
        loss_reasons,
        error_message: None,
        confirmation_fingerprint,
        confirmation_received: false,
    }
}

pub(super) fn legacy_sheet_analysis(song: &Song, schedule: &ScheduleMetadata) -> SheetAnalysisDto {
    SheetAnalysisDto {
        stage: SheetPipelineStageDto::ScheduleAdmitted,
        readiness: SheetReadinessDto::Ready,
        format_label: "Legacy flat".into(),
        schema_version: None,
        source_note_count: Some(song.notes.len() as u64),
        source_track_count: None,
        tracks: Vec::new(),
        conversion_mode: None,
        target_pitch: None,
        target_pitch_assumed_from_source: false,
        output_onset_count: Some(schedule.note_count as u64),
        merged_duplicate_count: Some(schedule.duplicate_note_count as u64),
        tempo_quantization_collapse_count: Some(0),
        source_unpitched_count: Some(0),
        source_unresolved_count: Some(0),
        target_unplayable_count: Some(0),
        omitted_source_event_count: Some(0),
        authored_sustain_lost_count: Some(0),
        impossible_same_key_repeats: Some(schedule.impossible_same_key_repeats as u64),
        loss_reasons: Vec::new(),
        error_message: None,
        confirmation_fingerprint: None,
        confirmation_received: false,
    }
}

pub(super) fn failed_sheet_analysis(format_label: &str, error: &PlannerError) -> SheetAnalysisDto {
    let (format_label, schema_version) = match error {
        PlannerError::Source(SongDecodeError::UnsupportedVersion { kind, version }) => {
            let label = match kind.as_str() {
                "composed" => "Composed",
                "recorded" => "Recorded",
                _ => "Unknown sheet",
            };
            (format!("{label} v{version}"), u8::try_from(*version).ok())
        }
        _ => (format_label.to_owned(), None),
    };
    let unsupported = matches!(
        error,
        PlannerError::Source(
            SongDecodeError::Encrypted
                | SongDecodeError::UnsupportedFormat { .. }
                | SongDecodeError::UnsupportedVersion { .. }
                | SongDecodeError::MultipleSongsUnsupported { .. }
                | SongDecodeError::UnsupportedSourceGame { .. }
                | SongDecodeError::AmbiguousSourceGame
                | SongDecodeError::UnsupportedInstrument { .. }
                | SongDecodeError::ResourceLimit { .. }
                | SongDecodeError::PayloadTooLarge { .. }
        )
    );
    SheetAnalysisDto {
        stage: SheetPipelineStageDto::Discovered,
        readiness: if unsupported {
            SheetReadinessDto::Unsupported
        } else {
            SheetReadinessDto::Malformed
        },
        format_label,
        schema_version,
        source_note_count: None,
        source_track_count: None,
        tracks: Vec::new(),
        conversion_mode: None,
        target_pitch: None,
        target_pitch_assumed_from_source: false,
        output_onset_count: None,
        merged_duplicate_count: None,
        tempo_quantization_collapse_count: None,
        source_unpitched_count: None,
        source_unresolved_count: None,
        target_unplayable_count: None,
        omitted_source_event_count: None,
        authored_sustain_lost_count: None,
        impossible_same_key_repeats: None,
        loss_reasons: Vec::new(),
        error_message: Some(bounded_text(error.to_string())),
        confirmation_fingerprint: None,
        confirmation_received: false,
    }
}

pub(super) fn sheet_confirmation_fingerprint(
    identity: SheetConfirmationIdentity<'_>,
) -> Result<String, String> {
    let SheetConfirmationIdentity {
        song_id,
        generation,
        path,
        bytes,
        mode,
        pitch,
        fps,
        hold_frames,
        timing_margin_us,
        tempo_scale,
    } = identity;
    let identity = serde_json::to_vec(&(
        song_id,
        generation,
        path.to_string_lossy(),
        mode,
        pitch,
        fps,
        hold_frames,
        timing_margin_us,
        tempo_scale,
    ))
    .map_err(|error| format!("sheet conversion identity failed: {error}"))?;
    let mut digest = Sha256::new();
    digest.update(b"sky-sheet-conversion-consent-v1\0");
    digest.update(identity);
    digest.update([0]);
    digest.update(bytes);
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn bounded_text(value: impl AsRef<str>) -> String {
    value.as_ref().chars().take(4096).collect()
}
