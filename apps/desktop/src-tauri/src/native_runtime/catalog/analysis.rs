use crate::commands::{CatalogDetailRequest, RiskSummaryDto, SheetReadinessDto, SongDetailDto};
use crate::native_runtime::catalog::{catalog_error, format_label_for_path};
use crate::native_runtime::{
    AnalysisKey, AnalysisPriority, AnalysisRequest, CachedSongAnalysis, CatalogMetadata,
    NativeDesktopRuntime, SongAnalysisRequest,
};
use sky_app_core::song::{SongPlan, analyze_schedule_with_context, build_song_plan};
use std::fs;
use std::path::PathBuf;

impl NativeDesktopRuntime {
    pub(in crate::native_runtime) fn detail(
        &self,
        request: CatalogDetailRequest,
    ) -> Result<SongDetailDto, String> {
        self.ensure_catalog_loaded()?;
        self.playback.invalidate_prepared_for_selection()?;
        let (entry, generation) = {
            let catalog = self
                .catalog
                .lock()
                .map_err(|_| "native catalog lock poisoned".to_string())?;
            (
                catalog
                    .index
                    .entry_for_song_id(&request.song_id, request.generation)
                    .map_err(catalog_error)?
                    .clone(),
                catalog.index.generation(),
            )
        };
        let path = PathBuf::from(&entry.canonical_path);
        let title = entry.row.title;
        let song_id = entry.row.song_id;
        let (settings, _, _, settings_revision) = self.catalog_settings_snapshot()?;
        let policy = self.timing_policy(
            settings.playback_defaults.fps,
            settings.playback_defaults.hold_frames,
            settings.playback_defaults.timing_margin_us,
        )?;
        let source_bytes = fs::read(&path).map_err(|error| format!("song read failed: {error}"))?;
        let selection = serde_json::to_string(&(request.conversion_mode, request.target_pitch))
            .map_err(|error| format!("sheet selection encoding failed: {error}"))?;
        let key = AnalysisKey {
            song_id: if request.conversion_mode.is_none() && request.target_pitch.is_none() {
                song_id.clone()
            } else {
                format!("{song_id}\0sheet:{selection}")
            },
            generation,
            settings_revision,
        };
        let cached = self.analysis_for_request(AnalysisRequest {
            key,
            analysis_song_id: &song_id,
            path: &path,
            source_bytes: Some(&source_bytes),
            title: &title,
            failure_format_label: &format_label_for_path(&path),
            settings: &settings,
            policy: &policy,
            conversion_mode: request.conversion_mode,
            target_pitch: request.target_pitch,
            viewport_revision: None,
            priority: AnalysisPriority::Selected,
            cache_failures: false,
        })?;
        let detail = cached
            .detail
            .clone()
            .ok_or_else(|| "song detail analysis was unavailable".to_string())?;
        if self
            .analysis_revision
            .load(std::sync::atomic::Ordering::Acquire)
            != settings_revision
            || self
                .catalog
                .lock()
                .map_err(|_| "native catalog lock poisoned".to_string())?
                .index
                .generation()
                != generation
        {
            return Err("catalog detail analysis became stale".into());
        }
        Ok(detail)
    }
    pub(in crate::native_runtime) fn analyze_song(
        &self,
        request: SongAnalysisRequest<'_>,
    ) -> Result<CachedSongAnalysis, String> {
        let SongAnalysisRequest {
            path,
            source_bytes,
            song_id,
            title,
            fallback,
            generation,
            settings,
            policy,
            conversion_mode,
            target_pitch,
        } = request;
        let owned_bytes;
        let bytes = if let Some(source_bytes) = source_bytes {
            source_bytes
        } else {
            owned_bytes = fs::read(path).map_err(|error| format!("song read failed: {error}"))?;
            owned_bytes.as_slice()
        };
        let source_hash = crate::native_runtime::sheet::sha256_hex(bytes);
        let options = crate::native_runtime::sheet::planner_options(
            conversion_mode,
            target_pitch,
            settings.playback_defaults.tempo_scale,
        );
        let plan = match build_song_plan(bytes, fallback, &options, policy) {
            Ok(plan) => plan,
            Err(error) => {
                let sheet_analysis = crate::native_runtime::sheet::failed_sheet_analysis(
                    &format_label_for_path(path),
                    &error,
                );
                let state = match sheet_analysis.readiness {
                    SheetReadinessDto::Unsupported => "unsupported",
                    _ => "malformed",
                };
                let detail = SongDetailDto {
                    song_id: song_id.to_owned(),
                    title: title.to_owned(),
                    duration_us: 0,
                    note_count: 0,
                    format_label: format_label_for_path(path),
                    risk: RiskSummaryDto {
                        level: "unknown".into(),
                        headline: "Sheet cannot be prepared".into(),
                        reasons: vec![
                            sheet_analysis
                                .error_message
                                .clone()
                                .unwrap_or_else(|| "The sheet could not be analyzed.".into()),
                        ],
                        recommendations: Vec::new(),
                        min_same_key_slack_us: None,
                        same_key_repeat_count: 0,
                        zero_slack_repeat_count: 0,
                    },
                    recommendation: None,
                    sheet_analysis: Some(sheet_analysis),
                };
                return Ok(CachedSongAnalysis {
                    metadata: CatalogMetadata {
                        format_label: format_label_for_path(path),
                        duration_us: None,
                        note_count: None,
                        risk_level: "unknown".into(),
                        state,
                    },
                    detail: Some(detail),
                    source_hash: Some(source_hash),
                });
            }
        };
        let (note_count, duration_us, risk, sheet_analysis, state) = match plan {
            SongPlan::LegacyCompatibility(plan) => {
                let sheet =
                    crate::native_runtime::sheet::legacy_sheet_analysis(&plan.song, &plan.schedule);
                let risk = analyze_schedule_with_context(
                    &plan.schedule,
                    Some(&plan.song.notes),
                    settings.playback_defaults.hold_frames,
                    settings.playback_defaults.tempo_scale,
                );
                (
                    plan.song.notes.len() as u64,
                    plan.schedule.source_duration_us,
                    risk,
                    sheet,
                    "ready",
                )
            }
            SongPlan::PianoConversion(plan) => {
                let fingerprint = crate::native_runtime::sheet::sheet_confirmation_fingerprint(
                    crate::native_runtime::sheet::SheetConfirmationIdentity {
                        song_id,
                        generation,
                        path,
                        bytes,
                        mode: conversion_mode,
                        pitch: target_pitch,
                        fps: settings.playback_defaults.fps,
                        hold_frames: settings.playback_defaults.hold_frames,
                        timing_margin_us: settings.playback_defaults.timing_margin_us,
                        tempo_scale: settings.playback_defaults.tempo_scale,
                    },
                )?;
                let contextual_risk = crate::native_runtime::sheet::piano_plan_risk(
                    &plan,
                    settings.playback_defaults.hold_frames,
                    settings.playback_defaults.tempo_scale,
                );
                let sheet = crate::native_runtime::sheet::piano_plan_sheet_analysis(
                    &plan,
                    Some(fingerprint),
                    contextual_risk.as_ref(),
                );
                let state = match sheet.readiness {
                    SheetReadinessDto::RequiresConfirmation => "requires_confirmation",
                    SheetReadinessDto::PhysicalRisk => "physical_risk",
                    SheetReadinessDto::Unplayable => "unplayable",
                    _ => "ready",
                };
                let schedule = plan.schedule.as_ref();
                let risk = contextual_risk.unwrap_or_else(unknown_risk_report);
                (
                    plan.diagnostics.source_tuple_count as u64,
                    schedule
                        .map(|value| value.source_duration_us)
                        .unwrap_or_default(),
                    risk,
                    sheet,
                    state,
                )
            }
        };
        let risk_level = match risk.severity.as_str() {
            "low" | "medium" | "high" => risk.severity.clone(),
            _ => "unknown".to_owned(),
        };
        let timing_reconsent_required = sheet_analysis.schema_version.is_some()
            && crate::native_runtime::sheet::timing_reconsent_required(
                &risk,
                settings.playback_defaults.hold_frames,
                settings.playback_defaults.tempo_scale,
            );
        let mut recommendations = risk.recommendations.clone();
        if timing_reconsent_required {
            recommendations.push(crate::native_runtime::sheet::TIMING_RECONSENT_NOTE.to_owned());
        }
        let reasons = if risk_level == "low" {
            Vec::new()
        } else {
            recommendations.clone()
        };
        let recommendation = Some(crate::commands::PlaybackRecommendationDto {
            recommended_hold_frames: risk.suggested_hold_frames,
            recommended_tempo_scale: risk.suggested_tempo_scale,
            summary: if timing_reconsent_required {
                let suggestion = risk
                    .recommendations
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Consider a timing profile change.".into());
                format!(
                    "{suggestion} {}",
                    crate::native_runtime::sheet::TIMING_RECONSENT_NOTE
                )
            } else {
                recommendations
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Keep the selected settings.".into())
            },
        });
        let format_label = format_label_for_path(path);
        let detail = SongDetailDto {
            song_id: song_id.to_owned(),
            title: title.to_owned(),
            duration_us,
            note_count,
            format_label: format_label.clone(),
            risk: RiskSummaryDto {
                level: risk_level.clone(),
                headline: match risk_level.as_str() {
                    "low" => "Low timing risk".into(),
                    "medium" => "Medium timing risk".into(),
                    "high" => "High timing risk".into(),
                    _ => "Risk unavailable".into(),
                },
                reasons,
                recommendations,
                min_same_key_slack_us: risk.min_same_key_slack_us,
                same_key_repeat_count: risk.same_key_repeat_count as u64,
                zero_slack_repeat_count: risk.zero_slack_repeat_count as u64,
            },
            recommendation,
            sheet_analysis: Some(sheet_analysis),
        };
        Ok(CachedSongAnalysis {
            metadata: CatalogMetadata {
                format_label,
                duration_us: Some(duration_us),
                note_count: Some(note_count),
                risk_level,
                state,
            },
            detail: Some(detail),
            source_hash: Some(source_hash),
        })
    }
}
fn unknown_risk_report() -> sky_app_core::song::RiskReport {
    sky_app_core::song::RiskReport {
        severity: "unknown".into(),
        reason: "No playable schedule was produced.".into(),
        recommendations: Vec::new(),
        dense_clusters: Vec::new(),
        average_notes_per_second: 0.0,
        peak_notes_per_second_1s: 0.0,
        max_polyphony: 0,
        max_chord_size: 0,
        chords_count: 0,
        timing_stress_rate: 0.0,
        min_same_key_slack_us: None,
        same_key_repeat_count: 0,
        zero_slack_repeat_count: 0,
        min_any_note_gap_us: None,
        min_same_key_gap_us: None,
        suggested_hold_frames: None,
        suggested_tempo_scale: None,
    }
}
