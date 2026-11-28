use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CatalogSearchRequest {
    pub query: String,
    pub offset: u64,
    pub limit: u16,
    pub generation: Option<u64>,
    #[serde(default)]
    pub source: LibrarySource,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum CatalogSourceId {
    #[default]
    All,
    Liked,
    Builtin,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq, Eq)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LibrarySource {
    Smart { id: CatalogSourceId },
    Playlist { id: String },
}

impl Default for LibrarySource {
    fn default() -> Self {
        Self::Smart {
            id: CatalogSourceId::All,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CatalogDetailRequest {
    pub song_id: String,
    pub generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub conversion_mode: Option<SheetConversionModeDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub target_pitch: Option<SheetPitchDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SheetConversionModeDto {
    MergePlayable,
    SingleTrack { track_id: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum SheetPitchDto {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SheetPipelineStageDto {
    Discovered,
    Parsed,
    MappedTo15Key,
    ScheduleAdmitted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum SheetReadinessDto {
    Loading,
    Malformed,
    Unsupported,
    Unplayable,
    RequiresConfirmation,
    PhysicalRisk,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct SheetTrackDto {
    pub track_id: u32,
    pub instrument: Option<String>,
    pub effective_pitch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(deny_unknown_fields)]
pub struct SheetAnalysisDto {
    pub stage: SheetPipelineStageDto,
    pub readiness: SheetReadinessDto,
    pub format_label: String,
    pub schema_version: Option<u8>,
    pub source_note_count: Option<u64>,
    pub source_track_count: Option<u64>,
    pub tracks: Vec<SheetTrackDto>,
    pub conversion_mode: Option<SheetConversionModeDto>,
    pub target_pitch: Option<SheetPitchDto>,
    pub target_pitch_assumed_from_source: bool,
    pub output_onset_count: Option<u64>,
    pub merged_duplicate_count: Option<u64>,
    pub tempo_quantization_collapse_count: Option<u64>,
    pub source_unpitched_count: Option<u64>,
    pub source_unresolved_count: Option<u64>,
    pub target_unplayable_count: Option<u64>,
    pub omitted_source_event_count: Option<u64>,
    pub authored_sustain_lost_count: Option<u64>,
    pub impossible_same_key_repeats: Option<u64>,
    pub loss_reasons: Vec<String>,
    pub error_message: Option<String>,
    pub confirmation_fingerprint: Option<String>,
    pub confirmation_received: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CatalogViewportRequest {
    pub generation: u64,
    pub first_index: u64,
    pub last_index: i64,
    pub selected_song_id: Option<String>,
    pub song_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CatalogSetLikedRequest {
    pub song_id: String,
    pub liked: bool,
    pub generation: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogRowDto {
    pub song_id: String,
    pub title: String,
    pub format_label: String,
    pub duration_us: Option<u64>,
    pub note_count: Option<u64>,
    pub risk_level: String,
    pub metadata_state: String,
    pub liked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogSearchDto {
    pub items: Vec<CatalogRowDto>,
    pub offset: u64,
    pub limit: u16,
    pub total: u64,
    pub liked_total: u64,
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RiskSummaryDto {
    pub level: String,
    pub headline: String,
    pub reasons: Vec<String>,
    pub recommendations: Vec<String>,
    pub min_same_key_slack_us: Option<u64>,
    pub same_key_repeat_count: u64,
    pub zero_slack_repeat_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PlaybackRecommendationDto {
    pub recommended_hold_frames: Option<f64>,
    pub recommended_tempo_scale: Option<f64>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SongDetailDto {
    pub song_id: String,
    pub title: String,
    pub duration_us: u64,
    pub note_count: u64,
    pub format_label: String,
    pub risk: RiskSummaryDto,
    pub recommendation: Option<PlaybackRecommendationDto>,
    #[serde(default)]
    pub sheet_analysis: Option<SheetAnalysisDto>,
}
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogReloadDto {
    pub generation: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogViewportDto {
    pub accepted: bool,
    pub generation: u64,
    pub first_index: u64,
    pub last_index: i64,
    pub selected_song_id: Option<String>,
    pub items: Vec<CatalogRowDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogSetLikedDto {
    pub song_id: String,
    pub liked: bool,
    pub total: u64,
}
