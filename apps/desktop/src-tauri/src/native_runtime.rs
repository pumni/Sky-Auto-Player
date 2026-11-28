//! Native desktop application runtime.
//!
//! This is the composition root for native desktop commands. It owns live
//! application state and calls pure app-core services plus outer adapters.

use crate::app_state::{ActivityCoordinator, ActivityReservationError, PhysicalActivityLease};
use crate::commands::{
    CatalogDetailRequest, CatalogSearchRequest, CatalogSetLikedRequest, CatalogViewportRequest,
    DiagnosticsEnabledDto, DiagnosticsSetEnabledRequest, LibraryCreatePlaylistRequest,
    LibraryPlaylistIdRequest, LibraryPlaylistSongsRequest, LibraryRenamePlaylistRequest,
    PlaybackActiveStatusDto, PlaybackAdmission, PlaybackCommandAckDto, PlaybackConfigDto,
    PlaybackDecision, PlaybackDecisionAcceptanceDto, PlaybackPendingControl,
    PlaybackPlanVariantDto, PlaybackPrepareRequest, PlaybackSessionDto, PlaybackSessionState,
    PlaybackStartRequest, PlaybackStatusDto, PlaybackTerminalStatusDto, PreparedPlaybackDto,
    RiskDecisionDto, RiskSummaryDto, SettingsPatch, SheetConversionModeDto, SheetPitchDto,
    SongDetailDto, UpdatePreferencesPatch,
};
use crate::power_lifecycle::{
    PowerLifecycleDiagnostics, PowerLifecycleSnapshot, SuspendResumeRegistration,
    SystemRequiredPowerRequest, acquire_playback_power_resources, system_power_endpoint,
    system_power_platform,
};
use crate::ui_events::{
    DiagnosticsBackendStatus, PlaybackEventState, PlaybackFailedPayload, PlaybackFinishedPayload,
    PlaybackFocusState, PlaybackHealthState, PlaybackSnapshotPayload, PlaybackStateChangedPayload,
    UiEvent, UpdateCheckRequest,
};
use catalog::catalog_error;
#[cfg(test)]
use catalog::{compute_catalog_source_fingerprint, compute_catalog_source_static_fingerprint};
use serde::Deserialize;
use serde_json::Value;
use sky_app_core::catalog::CatalogIndex;
use sky_app_core::library::{ImportedSourceKind, LibraryManifestService};
use sky_app_core::settings::{
    ApplicationSettings, SettingsError, SettingsService,
    UpdatePreferencesPatch as CoreUpdatePreferencesPatch,
};
use sky_app_core::song::{
    RiskReport, ScheduleMetadata, Song, SongPlan, analyze_schedule_with_context,
    build_schedule_with_policy, build_song_plan,
};
use sky_app_core::timing::MaterializedTimingPolicy;
#[cfg(test)]
use sky_native_adapters::AppPaths;
use sky_native_adapters::{
    BuiltinCatalogStatus, CatalogCache, CatalogCacheStore, CatalogComposer, CatalogComposition,
    ImportedSourceCatalogStatus, JsonLibraryManifestStore, JsonSettingsStore,
};
use sky_player::adapter_support::{OnsetChordInput, PriorityMode};
use sky_player::engine::{
    BackendConfig, DispatchProfile, EnginePollStatus, EngineProgressSnapshot, EngineSnapshot,
    FocusOptions, NativeDispatchSession, NativeSessionOptions, PriorityOptions, TelemetryMode,
    TelemetryOptions, TimingOptions, WaitOptions, WorkerTerminalResult,
};
use smallvec::SmallVec;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fs;
use std::io::Write;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, TryLockError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

mod bootstrap;
mod events;
mod settings;
mod update;
#[cfg(test)]
pub(crate) use events::MAX_NATIVE_EVENTS;
pub(crate) use events::NativeEventHub;
#[cfg(test)]
use events::{NativeEventSnapshotMetrics, remove_oldest_snapshot};
use events::{percentile_ms, population_sigma_ms};

#[cfg(test)]
const SENDER_TRACE_CAPTURE_CAPACITY: usize = 4_096;
const MAX_PREPARED_PLANS: usize = 64;
const MAX_DECISION_COUNT: usize = 8;
const PLAYBACK_JOIN_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const PLAYBACK_SHUTDOWN_GRACE: Duration = Duration::from_millis(250);
const PLAYBACK_PRESENTATION_CRITICAL_CAPACITY: usize = 64;
const MAX_CACHED_CATALOG_SEARCH_ORDERS: usize = 8;
const MAX_CACHED_CATALOG_SEARCH_INDICES: usize = 100_000;
const MAX_CACHED_SONG_ANALYSES: usize = 128;
const MAX_CACHED_VIEWPORT_RESPONSE_ROWS: usize = 200;
const MAX_ACTIVE_SONG_ANALYSES: usize = 2;
const MAX_QUEUED_SONG_ANALYSES: usize = 64;
const DRY_RUN_TERMINAL_NONE: u8 = 0;
#[cfg(test)]
type PlaybackJoinTestHook = Arc<dyn Fn(Duration) -> Result<bool, String> + Send + Sync>;
#[cfg(all(test, feature = "tauri-test"))]
type PlaybackBeforeArmTestHook = Arc<dyn Fn() + Send + Sync>;
#[cfg(all(test, feature = "tauri-test"))]
type PlaybackRetiredTestHook = Arc<dyn Fn() + Send + Sync>;
type PlaybackPresentationCompletion = Box<dyn FnOnce(Result<(), String>) + Send + 'static>;
#[cfg(all(test, feature = "tauri-test"))]
type PlaybackPresentationTestHook = Arc<dyn Fn(PlaybackPresentationStage) + Send + Sync>;
pub(crate) struct NativeDesktopRuntime {
    #[cfg(test)]
    paths: AppPaths,
    settings: Mutex<SettingsService<JsonSettingsStore>>,
    library_manifest: Mutex<LibraryManifestService<JsonLibraryManifestStore>>,
    catalog_composer: CatalogComposer,
    catalog_cache: CatalogCacheStore,
    catalog_source_static_fingerprint: String,
    catalog_load_epoch: AtomicU64,
    #[cfg(test)]
    catalog_compose_barrier: Mutex<Option<(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>)>>,
    #[cfg(test)]
    analysis_test_gate: Mutex<Option<Arc<AnalysisTestGate>>>,
    #[cfg(test)]
    analysis_test_runs: AtomicU64,
    #[cfg(test)]
    analysis_test_queue_rejections: AtomicU64,
    #[cfg(test)]
    catalog_search_order_test_computations: AtomicU64,
    #[cfg(test)]
    viewport_projection_test_hits: AtomicU64,
    #[cfg(test)]
    analysis_test_order: Mutex<Vec<String>>,
    cached_catalog: Mutex<Option<CatalogCache>>,
    catalog: Mutex<CatalogState>,
    catalog_load: Mutex<CatalogLoadState>,
    catalog_search_cache: Mutex<CatalogSearchCache>,
    analysis_cache: Mutex<AnalysisCacheState>,
    viewport_projection: Mutex<Option<ViewportResponseProjection>>,
    analysis_admission: Mutex<AnalysisAdmissionState>,
    analysis_admission_changed: Condvar,
    analysis_revision: AtomicU64,
    viewport_revision: AtomicU64,
    events: Arc<Mutex<NativeEventHub>>,
    playback: Arc<NativePlaybackService>,
    update_service: Option<Arc<crate::native_update::UpdateService<crate::ShellRuntime>>>,
    pre_exit_safety: Arc<dyn Fn() + Send + Sync>,
    ready_emitted: AtomicBool,
    closed: AtomicBool,
}

#[derive(Debug, Clone)]
struct CatalogMetadata {
    format_label: String,
    duration_us: Option<u64>,
    note_count: Option<u64>,
    risk_level: String,
    state: &'static str,
}

pub(super) struct SongAnalysisRequest<'a> {
    pub(super) path: &'a Path,
    pub(super) source_bytes: Option<&'a [u8]>,
    pub(super) song_id: &'a str,
    pub(super) title: &'a str,
    pub(super) fallback: &'a str,
    pub(super) generation: u64,
    pub(super) settings: &'a ApplicationSettings,
    pub(super) policy: &'a MaterializedTimingPolicy,
    pub(super) conversion_mode: Option<SheetConversionModeDto>,
    pub(super) target_pitch: Option<SheetPitchDto>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeLibraryImportRequest {
    pub playlist_id: String,
    pub paths: Vec<String>,
}

#[derive(Default)]
struct CatalogState {
    index: CatalogIndex,
    library_membership: BTreeSet<String>,
    builtin_membership: BTreeSet<String>,
    user_membership: BTreeSet<String>,
    imported_membership: HashMap<String, BTreeSet<String>>,
    imported_status: Vec<ImportedSourceCatalogStatus>,
    builtin_status: BuiltinCatalogStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum CatalogLoadState {
    #[default]
    Uninitialized,
    Cached {
        generation: u64,
    },
    Reconciling {
        generation: u64,
    },
    Loading,
    Ready {
        generation: u64,
    },
    Failed {
        message: String,
    },
    Closing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatalogLoadWork {
    Rebuild,
    Reconcile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CatalogLoadClaim {
    work: CatalogLoadWork,
    epoch: u64,
}

struct CatalogCompositionSnapshot {
    composition: CatalogComposition,
    source_fingerprint: String,
}

impl CachedSongAnalysis {
    fn error(format_label: String) -> Self {
        Self {
            metadata: CatalogMetadata::error(format_label),
            detail: None,
            source_hash: None,
        }
    }
}

impl CatalogMetadata {
    fn error(format_label: String) -> Self {
        Self {
            format_label,
            duration_us: None,
            note_count: None,
            risk_level: "unknown".into(),
            state: "error",
        }
    }
}

#[derive(Debug, Clone)]
struct CachedSongAnalysis {
    metadata: CatalogMetadata,
    detail: Option<SongDetailDto>,
    source_hash: Option<String>,
}

pub(super) struct NativeSheetSource {
    pub(super) bytes: Vec<u8>,
    pub(super) fallback_name: String,
    pub(super) conversion_mode: Option<SheetConversionModeDto>,
    pub(super) target_pitch: Option<SheetPitchDto>,
}

impl NativeDesktopRuntime {
    pub(crate) fn dispatch(&self, method: &str, params: Value) -> Result<Value, String> {
        if self.closed.load(Ordering::Acquire) && method != "app.shutdown" {
            return Err("native desktop runtime is shut down".into());
        }
        if crate::command_ownership::owner_for(method)
            != Some(crate::command_ownership::CommandOwner::Native)
        {
            return Err(format!(
                "native runtime does not own desktop command: {method}"
            ));
        }
        match method {
            "app.bootstrap" => encode_result(self.bootstrap()),
            "app.shutdown" => {
                self.shutdown();
                Ok(Value::Null)
            }
            "catalog.search" => {
                let request: CatalogSearchRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.search(request))
            }
            "catalog.set_liked" => {
                let request: CatalogSetLikedRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.set_song_liked(request))
            }
            "catalog.detail" => {
                let request: NativeCatalogDetailRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.detail(CatalogDetailRequest {
                    song_id: request.song_id,
                    generation: request.generation,
                    conversion_mode: request.conversion_mode,
                    target_pitch: request.target_pitch,
                }))
            }
            "catalog.reload" => encode_result(self.reload()),
            "catalog.set_viewport" => {
                let request: NativeCatalogViewportRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.set_viewport(CatalogViewportRequest {
                    generation: request.generation,
                    first_index: request.first_index,
                    last_index: request.last_index,
                    selected_song_id: request.selected_song_id,
                    song_ids: request.song_ids,
                }))
            }
            "library.list_playlists" => encode_result(self.list_playlists()),
            "library.create_playlist" => {
                let request: LibraryCreatePlaylistRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.create_playlist(request))
            }
            "library.rename_playlist" => {
                let request: LibraryRenamePlaylistRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.rename_playlist(request))
            }
            "library.delete_playlist" => {
                let request: LibraryPlaylistIdRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.delete_playlist(request))
            }
            "library.add_songs" => {
                let request: LibraryPlaylistSongsRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.add_playlist_songs(request))
            }
            "library.remove_songs" => {
                let request: LibraryPlaylistSongsRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.remove_playlist_songs(request))
            }
            "library.import_local_files_to_playlist" => {
                let request: NativeLibraryImportRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.import_paths(request, ImportedSourceKind::File))
            }
            "library.import_local_folder_to_playlist" => {
                let request: NativeLibraryImportRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.import_paths(request, ImportedSourceKind::Folder))
            }
            "settings.get" => encode_result(self.reload_settings_dto()),
            "settings.patch" => {
                let request: NativeSettingsPatch =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.patch_settings(request.into_public()))
            }
            "update.preferences.get" => encode_result(self.update_preferences()),
            "update.preferences.patch" => {
                let request: NativeUpdatePreferencesPatch =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.patch_update_preferences(request.into_public()))
            }
            "update.check" => {
                let request: UpdateCheckRequest = if params.is_null()
                    || params.as_object().is_some_and(|object| object.is_empty())
                {
                    UpdateCheckRequest {
                        origin: crate::ui_events::UpdateCheckOrigin::Background,
                    }
                } else {
                    serde_json::from_value(params).map_err(json_error)?
                };
                encode_result(self.check_update(request))
            }
            "update.begin_handoff" => {
                let request: crate::commands::UpdateBeginHandoffRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.begin_update_handoff(request.target_version))
            }
            "playback.prepare" => {
                let request: NativePlaybackPrepareRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.prepare_playback(request.into_public()))
            }
            "playback.start" => {
                let request: NativePlaybackStartRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                validate_playback_start_request(&request)?;
                encode_result(self.start_playback(request.into_public()))
            }
            "playback.get_status" => {
                if !params.as_object().is_some_and(|object| object.is_empty()) {
                    return Err("invalid_params: playback.get_status takes no parameters".into());
                }
                encode_result(self.get_playback_status())
            }
            "playback.stop" | "playback.pause" | "playback.resume" | "playback.skip" => {
                let request: NativePlaybackSessionRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.playback_command(method, request.session_id))
            }
            "playback.export_sender_trace" => {
                if !params.as_object().is_some_and(|object| object.is_empty()) {
                    return Err(
                        "invalid_params: playback.export_sender_trace takes no parameters".into(),
                    );
                }
                encode_result(self.playback.export_sender_trace())
            }
            "diagnostics.set_enabled" => {
                let request: DiagnosticsSetEnabledRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.set_diagnostics_enabled(request))
            }
            _ => Err(format!("native command is not implemented: {method}")),
        }
    }

    fn prepare_playback(
        &self,
        request: crate::commands::PlaybackPrepareRequest,
    ) -> Result<PreparedPlaybackDto, String> {
        let cancelled = self.playback.begin_preparation()?;
        let result = (|| {
            self.ensure_catalog_loaded()?;
            let (path, indexed_title) = {
                let catalog = self
                    .catalog
                    .lock()
                    .map_err(|_| "native catalog lock poisoned".to_string())?;
                let entry = catalog
                    .index
                    .entry_for_song_id(&request.song_id, Some(request.generation))
                    .map_err(catalog_error)?;
                (
                    PathBuf::from(&entry.canonical_path),
                    entry.row.title.clone(),
                )
            };
            // The catalog and settings locks are not held across file I/O or
            // decoding. Bind the content digest to the plan and recheck it at start.
            let bytes = fs::read(&path).map_err(|error| format!("song read failed: {error}"))?;
            let source_hash = sha256_hex(&bytes);
            let fallback = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or(&indexed_title)
                .to_owned();
            let plan_identity = sheet_confirmation_fingerprint(SheetConfirmationIdentity {
                song_id: &request.song_id,
                generation: request.generation,
                path: &path,
                bytes: &bytes,
                mode: request.conversion_mode,
                pitch: request.target_pitch,
                fps: request.config.fps,
                hold_frames: request.config.hold_frames,
                timing_margin_us: request.config.timing_margin_us,
                tempo_scale: request.config.tempo_scale,
            })?;
            let (source_epoch, (actual_settings_fingerprint, publication_fingerprint)) =
                self.playback.capture_preparation_sources(&cancelled, || {
                    let (current_path, generation) = {
                        let catalog = self
                            .catalog
                            .lock()
                            .map_err(|_| "native catalog lock poisoned".to_string())?;
                        let entry = catalog
                            .index
                            .entry_for_song_id(&request.song_id, Some(request.generation))
                            .map_err(catalog_error)?;
                        (
                            PathBuf::from(&entry.canonical_path),
                            catalog.index.generation(),
                        )
                    };
                    if current_path != path {
                        return Err(
                            "song source path changed while preparation was reading it".into()
                        );
                    }
                    let settings = self.settings_snapshot()?;
                    let actual_settings_fingerprint = settings_fingerprint(&settings)?;
                    Ok((
                        generation,
                        actual_settings_fingerprint.clone(),
                        (
                            actual_settings_fingerprint.clone(),
                            actual_settings_fingerprint,
                        ),
                    ))
                })?;
            let policy = self.timing_policy(
                request.config.fps,
                request.config.hold_frames,
                request.config.timing_margin_us,
            )?;
            let options = planner_options(
                request.conversion_mode,
                request.target_pitch,
                request.config.tempo_scale,
            );
            let plan = match build_song_plan(&bytes, &fallback, &options, &policy) {
                Ok(plan) => plan,
                Err(error) => {
                    let sheet =
                        failed_sheet_analysis(&catalog::format_label_for_path(&path), &error);
                    return Ok(blocked_sheet_prepare(
                        &request.song_id,
                        &fallback,
                        request.config.clone(),
                        sheet,
                    ));
                }
            };
            let (song, schedule, risk, sheet_analysis, plan_source) = match plan {
                SongPlan::LegacyCompatibility(plan) => {
                    let risk = analyze_schedule_with_context(
                        &plan.schedule,
                        Some(&plan.song.notes),
                        request.config.hold_frames,
                        request.config.tempo_scale,
                    );
                    let sheet = legacy_sheet_analysis(&plan.song, &plan.schedule);
                    (plan.song, plan.schedule, risk, sheet, None)
                }
                SongPlan::PianoConversion(plan) => {
                    let risk = piano_plan_risk(
                        &plan,
                        request.config.hold_frames,
                        request.config.tempo_scale,
                    );
                    let sheet = piano_plan_sheet_analysis(
                        &plan,
                        Some(plan_identity.clone()),
                        risk.as_ref(),
                    );
                    let mut sheet = sheet;
                    sheet.confirmation_received =
                        request.conversion_confirmation_fingerprint.as_deref()
                            == sheet.confirmation_fingerprint.as_deref();
                    let Some(schedule) = plan.schedule else {
                        return Ok(blocked_sheet_prepare(
                            &request.song_id,
                            &plan.name,
                            request.config.clone(),
                            sheet,
                        ));
                    };
                    let Some(risk) = risk else {
                        return Ok(blocked_sheet_prepare(
                            &request.song_id,
                            &plan.name,
                            request.config.clone(),
                            sheet,
                        ));
                    };
                    let song = Song {
                        name: plan.name.clone(),
                        notes: Vec::new(),
                    };
                    let source = NativeSheetSource {
                        bytes: bytes.clone(),
                        fallback_name: fallback.clone(),
                        conversion_mode: request.conversion_mode,
                        target_pitch: request.target_pitch,
                    };
                    (song, schedule, risk, sheet, Some(source))
                }
            };
            self.playback.prepare_with_token(
                NativePreparedInput {
                    song_id: request.song_id,
                    generation: request.generation,
                    config: request.config,
                    song,
                    sheet_analysis: Some(sheet_analysis),
                    schedule,
                    risk,
                    timing_policy: policy,
                    settings_fingerprint: actual_settings_fingerprint,
                    publication_fingerprint,
                    source_path: path,
                    source_hash,
                    plan_identity,
                    plan_source,
                    source_epoch,
                },
                &cancelled,
            )
        })();
        self.playback.finish_preparation(&cancelled);
        result
    }

    fn timing_policy(
        &self,
        fps: u16,
        hold_frames: f64,
        timing_margin_us: u64,
    ) -> Result<MaterializedTimingPolicy, String> {
        MaterializedTimingPolicy::from_user_margin(fps, hold_frames, timing_margin_us)
            .map_err(|error| error.to_string())
    }

    fn start_playback(
        &self,
        request: crate::commands::PlaybackStartRequest,
    ) -> Result<PlaybackSessionDto, String> {
        validate_public_playback_start_request(&request)?;
        if let Some((path, expected_hash)) = self
            .playback
            .prepared_source_identity(&request.prepared_id)?
        {
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.playback
                        .invalidate_prepared_source(&request.prepared_id)?;
                    return Err(format!(
                        "prepared sheet source is no longer readable: {error}"
                    ));
                }
            };
            let current_hash = sha256_hex(&bytes);
            if current_hash != expected_hash {
                self.playback
                    .invalidate_prepared_source(&request.prepared_id)?;
                return Err(
                    "prepared playback is stale because the source sheet changed; prepare it again"
                        .into(),
                );
            }
        }
        let settings = self.settings_snapshot()?;
        self.playback.start(request, &settings, self.events.clone())
    }

    fn playback_command(
        &self,
        method: &str,
        session_id: String,
    ) -> Result<PlaybackCommandAckDto, String> {
        self.playback
            .command(method, session_id, self.events.clone())
    }

    fn get_playback_status(&self) -> Result<PlaybackStatusDto, String> {
        self.playback.status()
    }

    fn set_diagnostics_enabled(
        &self,
        request: DiagnosticsSetEnabledRequest,
    ) -> Result<DiagnosticsEnabledDto, String> {
        self.playback
            .set_diagnostics_enabled(request.enabled, self.events.clone())?;
        Ok(DiagnosticsEnabledDto {
            enabled: request.enabled,
        })
    }

    fn publish(&self, event: UiEvent) -> Result<(), String> {
        self.events
            .lock()
            .map_err(|_| "native event hub lock poisoned".to_string())?
            .publish(event)
    }

    pub(crate) fn shutdown(&self) {
        let mut catalog_load = self
            .catalog_load
            .lock()
            .expect("native catalog load state poisoned");
        if !self.closed.swap(true, Ordering::AcqRel) {
            *catalog_load = CatalogLoadState::Closing;
            drop(catalog_load);
            (self.pre_exit_safety)();
        }
    }
}
mod catalog;
mod catalog_cache;
mod catalog_reconcile;
mod catalog_search;
mod dtos;
#[cfg(test)]
use catalog_cache::AnalysisTestGate;
use catalog_cache::{
    AnalysisAdmissionState, AnalysisCacheState, AnalysisKey, AnalysisPriority, AnalysisRequest,
    CatalogSearchCache, CatalogSearchKey, CatalogSearchScope, ViewportProjectionKey,
    ViewportResponseProjection,
};
mod playback;
pub(super) mod sheet;
pub(crate) use dtos::native_build_dto;
use dtos::{settings_dto, update_preferences_dto};
#[cfg(test)]
use playback::*;
use playback::{
    NativePlaybackPrepareRequest, NativePlaybackService, NativePlaybackSessionRequest,
    NativePlaybackStartRequest, NativePreparedInput, opaque_native_id, settings_fingerprint,
    validate_playback_start_request, validate_public_playback_start_request,
};
use sheet::{
    SheetConfirmationIdentity, blocked_sheet_prepare, failed_sheet_analysis, legacy_sheet_analysis,
    piano_plan_risk, piano_plan_sheet_analysis, planner_options, sha256_hex,
    sheet_confirmation_fingerprint,
};

#[cfg(test)]
pub(crate) fn resolve_install_root() -> Result<PathBuf, String> {
    AppPaths::resolve().map(|paths| paths.install_root().to_path_buf())
}

fn json_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub(super) fn bounded_text(value: impl AsRef<str>) -> String {
    value.as_ref().chars().take(4096).collect()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativeCatalogDetailRequest {
    song_id: String,
    generation: Option<u64>,
    conversion_mode: Option<SheetConversionModeDto>,
    target_pitch: Option<SheetPitchDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativeCatalogViewportRequest {
    generation: u64,
    first_index: u64,
    last_index: i64,
    selected_song_id: Option<String>,
    song_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativePlaybackPatch {
    hold_frames: Option<f64>,
    timing_margin_us: Option<u64>,
    tempo_scale: Option<f64>,
    fps: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativeUpdatePreferencesPatch {
    auto_check: Option<bool>,
    channel: Option<crate::ui_events::UpdateChannel>,
    skip_version: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativeSettingsPatch {
    appearance_mode: Option<crate::commands::AppearanceMode>,
    #[serde(alias = "theme")]
    palette: Option<String>,
    telemetry_enabled: Option<bool>,
    verbose_hud: Option<bool>,
    playback_defaults: Option<NativePlaybackPatch>,
    auto_play: Option<bool>,
    update_preferences: Option<NativeUpdatePreferencesPatch>,
}

impl NativeUpdatePreferencesPatch {
    fn into_public(self) -> UpdatePreferencesPatch {
        UpdatePreferencesPatch {
            auto_check: self.auto_check,
            channel: self.channel,
            skip_version: self.skip_version,
        }
    }
}

impl NativeSettingsPatch {
    fn into_public(self) -> SettingsPatch {
        SettingsPatch {
            appearance_mode: self.appearance_mode,
            palette: self.palette,
            telemetry_enabled: self.telemetry_enabled,
            verbose_hud: self.verbose_hud,
            playback_defaults: self
                .playback_defaults
                .map(|value| crate::commands::PlaybackPatch {
                    hold_frames: value.hold_frames,
                    timing_margin_us: value.timing_margin_us,
                    tempo_scale: value.tempo_scale,
                    fps: value.fps,
                }),
            auto_play: self.auto_play,
            update_preferences: self.update_preferences.map(|value| value.into_public()),
        }
    }
}

fn encode_result<T: serde::Serialize>(result: Result<T, String>) -> Result<Value, String> {
    result
        .map_err(|error| error.to_string())
        .and_then(|value| serde_json::to_value(value).map_err(json_error))
}

fn settings_error(error: SettingsError) -> String {
    error.to_string()
}

fn playback_activity_error(error: ActivityReservationError) -> String {
    match error {
        ActivityReservationError::Closing => "closing: desktop application is closing".into(),
        ActivityReservationError::PhysicalPlaybackActive => {
            "already_running: another physical playback session is active".into()
        }
        ActivityReservationError::PhysicalReleaseUnverified => {
            "playback_release_unverified: physical playback is blocked because the previous key release could not be verified; restart the application to restore physical input".into()
        }
        ActivityReservationError::UpdateAlreadyActive => {
            "update_active: playback cannot run during update installation".into()
        }
    }
}

#[cfg(test)]
#[path = "native_runtime/tests.rs"]
mod tests;
