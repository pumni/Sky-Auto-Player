//! Native desktop application runtime.
//!
//! This is the composition root for commands that have crossed the strangler
//! boundary.  It owns live application state and calls pure app-core services
//! plus outer adapters.  It never delegates a native-owned command to the
//! Python Core.

use crate::app_state::{ActivityCoordinator, ActivityReservationError, PhysicalActivityLease};
use crate::commands::{
    BootstrapDto, CalibrationCancelAckDto, CalibrationCancelRequest, CalibrationStartAckDto,
    CalibrationStartRequest, CatalogDetailRequest, CatalogSearchRequest, CatalogSetLikedRequest,
    CatalogViewportRequest, DiagnosticsEnabledDto, DiagnosticsSetEnabledRequest,
    LibraryCreatePlaylistRequest, LibraryPlaylistIdRequest, LibraryPlaylistSongsRequest,
    LibraryRenamePlaylistRequest, PlaybackActiveStatusDto, PlaybackAdmission,
    PlaybackCommandAckDto, PlaybackConfigDto, PlaybackDecision, PlaybackDecisionAcceptanceDto,
    PlaybackDefaultsDto, PlaybackPendingControl, PlaybackPlanVariantDto, PlaybackPrepareRequest,
    PlaybackSessionDto, PlaybackSessionState, PlaybackStartRequest, PlaybackStatusDto,
    PlaybackTerminalStatusDto, PreparedPlaybackDto, RiskDecisionDto, RiskSummaryDto, SettingsDto,
    SettingsPatch, SongDetailDto, UpdateCheckAckDto, UpdateInstallAckDto, UpdatePreferencesDto,
    UpdatePreferencesPatch,
};
use crate::power_lifecycle::{
    PowerLifecycleDiagnostics, PowerLifecycleSnapshot, SuspendResumeRegistration,
    SystemRequiredPowerRequest, acquire_playback_power_resources, system_power_endpoint,
    system_power_platform,
};
use crate::ui_events::{
    CalibrationState, CoreReadyPayload, DiagnosticsBackendStatus, NativeBuildPayload,
    PlaybackEventState, PlaybackFailedPayload, PlaybackFinishedPayload, PlaybackFocusState,
    PlaybackHealthState, PlaybackSnapshotPayload, PlaybackStateChangedPayload, UiEvent,
    UpdateCheckRequest, UpdateSnapshotPayload,
};
#[cfg(test)]
use calibration::{
    CALIBRATION_DEFAULT_TIMEOUT_SECONDS, CALIBRATION_MIN_FULL_TIMEOUT_SECONDS,
    CALIBRATION_MIN_NATIVE_TOTAL_SECONDS, CALIBRATION_MIN_SINGLE_TIMEOUT_SECONDS,
    CALIBRATION_TIMING_MARGIN_GUARD_US, CalibrationRunError, calibration_budget,
    publish_calibration_cache, recommended_calibrated_timing_margin_us, safe_calibration_evidence,
};
use calibration::{NativeCalibrationService, timing_margin_recommendation};
#[cfg(test)]
use catalog::compute_catalog_source_fingerprint;
use catalog::{catalog_error, compute_catalog_source_static_fingerprint};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sky_app_core::catalog::CatalogIndex;
use sky_app_core::library::{ImportedSourceKind, LibraryManifestService};
use sky_app_core::settings::{
    ApplicationSettings, PlaybackDefaultsPatch, SettingsError, SettingsService,
    UpdatePreferencesPatch as CoreUpdatePreferencesPatch,
};
use sky_app_core::song::{
    ActionKind, RiskReport, ScheduleMetadata, Song, analyze_schedule_with_context,
    build_schedule_with_policy, parse_song_json,
};
use sky_app_core::timing::MaterializedTimingPolicy;
use sky_native_adapters::{
    AppPaths, AppResources, BuiltinCatalogSource, BuiltinCatalogStatus, CatalogCache,
    CatalogCacheStore, CatalogComposer, CatalogComposition, FileCatalogSource,
    ImportedSourceCatalogStatus, JsonLibraryManifestStore, JsonSettingsStore,
};
use sky_player::adapter_support::{
    ActionKind as DispatchActionKind, KeyActionInput, PriorityMode, compile_runtime_intents,
};
use sky_player::engine::{
    BackendConfig, DispatchProfile, EnginePollStatus, EngineProgressSnapshot, EngineSnapshot,
    FocusOptions, NativeDispatchSession, NativeSessionOptions, PriorityOptions, TelemetryMode,
    TelemetryOptions, TimingOptions, WaitOptions,
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
use tauri::ipc::Channel;

pub(crate) const MAX_NATIVE_EVENTS: usize = 128;
const SENDER_TRACE_CAPTURE_CAPACITY: usize = 4_096;
const MAX_PREPARED_PLANS: usize = 64;
const MAX_DECISION_COUNT: usize = 8;
const PLAYBACK_JOIN_RETRY_INTERVAL: Duration = Duration::from_millis(100);
const PLAYBACK_SHUTDOWN_GRACE: Duration = Duration::from_millis(250);
const EVENT_PUBLISHER_CLOSE_BUDGET: Duration = Duration::from_millis(50);
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
/// Production composition has no synthetic calibration/update behavior.  The
/// package harness enters `SafePackage` only through the hidden selftest
/// composition root, never through an environment variable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum TestSeams {
    #[default]
    Disabled,
    SafePackage,
}

pub(crate) struct NativeDesktopRuntime {
    #[allow(dead_code)]
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
    calibration: Arc<NativeCalibrationService>,
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
}

impl NativeDesktopRuntime {
    #[allow(dead_code)]
    pub(crate) fn for_current_install() -> Result<Self, String> {
        let paths = AppPaths::resolve()?;
        let resources = AppResources::from_resource_dir(paths.install_root());
        Self::from_paths_with_activity_and_seams(
            paths,
            resources,
            ActivityCoordinator::default(),
            TestSeams::Disabled,
        )
    }

    #[allow(dead_code)]
    #[cfg(test)]
    pub(crate) fn from_install_root(install_root: PathBuf) -> Result<Self, String> {
        Self::from_install_root_with_activity_and_seams(
            install_root,
            ActivityCoordinator::default(),
            TestSeams::Disabled,
        )
    }

    #[allow(dead_code)]
    pub(crate) fn for_current_install_with_activity(
        activity: ActivityCoordinator,
    ) -> Result<Self, String> {
        let paths = AppPaths::resolve()?;
        let resources = AppResources::from_resource_dir(paths.install_root());
        Self::from_paths_with_activity_and_seams(paths, resources, activity, TestSeams::Disabled)
    }

    #[allow(dead_code)]
    #[cfg(test)]
    pub(crate) fn from_install_root_with_activity(
        install_root: PathBuf,
        activity: ActivityCoordinator,
    ) -> Result<Self, String> {
        Self::from_install_root_with_activity_and_seams(install_root, activity, TestSeams::Disabled)
    }

    pub(crate) fn from_paths_with_activity_and_seams(
        paths: AppPaths,
        resources: AppResources,
        activity: ActivityCoordinator,
        test_seams: TestSeams,
    ) -> Result<Self, String> {
        Self::from_paths_with_activity_and_seams_and_update_service(
            paths, resources, activity, test_seams, None,
        )
    }

    pub(crate) fn from_paths_with_activity_and_seams_and_update_service(
        paths: AppPaths,
        resources: AppResources,
        activity: ActivityCoordinator,
        test_seams: TestSeams,
        update_service: Option<Arc<crate::native_update::UpdateService<crate::ShellRuntime>>>,
    ) -> Result<Self, String> {
        Self::from_paths_with_activity_and_seams_and_update_service_with_events(
            paths,
            resources,
            activity,
            test_seams,
            update_service,
            Arc::new(Mutex::new(NativeEventHub::default())),
        )
    }

    pub(crate) fn from_paths_with_activity_and_seams_and_update_service_with_events(
        paths: AppPaths,
        resources: AppResources,
        activity: ActivityCoordinator,
        test_seams: TestSeams,
        update_service: Option<Arc<crate::native_update::UpdateService<crate::ShellRuntime>>>,
        events: Arc<Mutex<NativeEventHub>>,
    ) -> Result<Self, String> {
        paths.assert_clean_boundary()?;
        Self::from_paths_internal(
            paths,
            resources,
            activity,
            test_seams,
            update_service,
            events,
        )
    }

    fn from_paths_internal(
        paths: AppPaths,
        resources: AppResources,
        activity: ActivityCoordinator,
        test_seams: TestSeams,
        update_service: Option<Arc<crate::native_update::UpdateService<crate::ShellRuntime>>>,
        events: Arc<Mutex<NativeEventHub>>,
    ) -> Result<Self, String> {
        let settings_path = paths.settings_path();
        let settings_store = JsonSettingsStore::new(settings_path);
        crate::startup_telemetry::record("settings.load.start");
        let mut settings = SettingsService::load(settings_store)
            .map_err(|error| format!("native settings startup failed: {error}"))?;
        crate::startup_telemetry::record("settings.load.end");
        if settings.snapshot().update.channel == sky_app_core::settings::UpdateChannel::Beta {
            settings
                .patch(&sky_app_core::settings::SettingsPatch {
                    update: Some(CoreUpdatePreferencesPatch {
                        auto_check: None,
                        channel: Some(sky_app_core::settings::UpdateChannel::Stable),
                        skip_version: None,
                    }),
                    ..Default::default()
                })
                .map_err(|e| format!("native settings beta repair failed: {e}"))?;
        }
        let manifest_store = JsonLibraryManifestStore::new(paths.library_manifest_path());
        crate::startup_telemetry::record("manifest.load.start");
        let library_manifest = LibraryManifestService::load(manifest_store)
            .map_err(|error| format!("native library manifest startup failed: {error}"))?;
        crate::startup_telemetry::record("manifest.load.end");
        let catalog_cache = CatalogCacheStore::new(paths.catalog_cache_path());
        let persisted_catalog = catalog_cache.load().ok().flatten();
        let songs_dir = paths
            .resolve_songs_dir(&settings.snapshot().songs_dir)
            .unwrap_or_else(|_| paths.user_music_root().to_path_buf());
        let catalog_source_static_fingerprint =
            compute_catalog_source_static_fingerprint(&resources, &songs_dir);
        let playback = Arc::new(NativePlaybackService::new(activity.clone()));
        let calibration = Arc::new(NativeCalibrationService::new(
            paths.clone(),
            activity.clone(),
            events.clone(),
            playback.clone(),
            test_seams,
        ));
        let safety_activity = activity.clone();
        let safety_calibration = calibration.clone();
        let safety_playback = playback.clone();
        let safety_events = events.clone();
        let pre_exit_safety: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            safety_activity.begin_shutdown();
            crate::record_update_safety_phase("activity.admission_closed");
            safety_calibration.shutdown();
            if safety_playback.shutdown(safety_events.clone()) {
                crate::record_update_safety_phase("playback.keys_released");
            }
            // Settings mutations use the durable Native-owned store before
            // this hook can run. The marker records that the close boundary
            // has crossed the persistence phase.
            crate::record_update_safety_phase("state.persisted");
            if let Ok(mut events) = safety_events.lock() {
                events.close();
            }
            crate::record_update_safety_phase("resources.closed");
        });
        if let Some(service) = &update_service {
            service.set_pre_exit_safety(Arc::clone(&pre_exit_safety));
        }
        let runtime = Self {
            paths,
            settings: Mutex::new(settings),
            library_manifest: Mutex::new(library_manifest),
            catalog_composer: CatalogComposer::new(
                BuiltinCatalogSource::new(resources),
                FileCatalogSource::new(songs_dir),
            ),
            catalog_cache,
            catalog_source_static_fingerprint,
            catalog_load_epoch: AtomicU64::new(0),
            #[cfg(test)]
            catalog_compose_barrier: Mutex::new(None),
            #[cfg(test)]
            analysis_test_gate: Mutex::new(None),
            #[cfg(test)]
            analysis_test_runs: AtomicU64::new(0),
            #[cfg(test)]
            analysis_test_queue_rejections: AtomicU64::new(0),
            #[cfg(test)]
            catalog_search_order_test_computations: AtomicU64::new(0),
            #[cfg(test)]
            viewport_projection_test_hits: AtomicU64::new(0),
            #[cfg(test)]
            analysis_test_order: Mutex::new(Vec::new()),
            cached_catalog: Mutex::new(None),
            catalog: Mutex::new(CatalogState::default()),
            catalog_load: Mutex::new(CatalogLoadState::default()),
            catalog_search_cache: Mutex::new(CatalogSearchCache::default()),
            analysis_cache: Mutex::new(AnalysisCacheState::default()),
            viewport_projection: Mutex::new(None),
            analysis_admission: Mutex::new(AnalysisAdmissionState::default()),
            analysis_admission_changed: Condvar::new(),
            analysis_revision: AtomicU64::new(0),
            viewport_revision: AtomicU64::new(0),
            events,
            playback,
            calibration,
            update_service,
            pre_exit_safety,
            ready_emitted: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        };
        if let Some(cache) = persisted_catalog
            && runtime.hydrate_cached_catalog(cache.clone()).is_ok()
            && let Ok(mut cached_catalog) = runtime.cached_catalog.lock()
        {
            *cached_catalog = Some(cache);
        }
        Ok(runtime)
    }

    #[allow(dead_code)]
    #[cfg(test)]
    pub(crate) fn from_install_root_with_activity_and_seams(
        install_root: PathBuf,
        activity: ActivityCoordinator,
        test_seams: TestSeams,
    ) -> Result<Self, String> {
        let paths = AppPaths::from_roots(
            install_root.clone(),
            install_root.clone(),
            install_root.clone(),
            install_root.join("cache"),
            install_root.join("logs"),
            install_root.join("songs"),
        );
        let resources = AppResources::from_resource_dir(&install_root);
        Self::from_paths_internal(
            paths,
            resources,
            activity,
            test_seams,
            None,
            Arc::new(Mutex::new(NativeEventHub::default())),
        )
    }

    #[allow(dead_code)]
    pub(crate) fn paths(&self) -> &AppPaths {
        &self.paths
    }

    #[allow(dead_code)]
    pub(crate) fn install_root(&self) -> &Path {
        self.paths.install_root()
    }

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
            "calibration.start" => {
                let request: CalibrationStartRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.start_calibration(request))
            }
            "calibration.cancel" => {
                let request: CalibrationCancelRequest =
                    serde_json::from_value(params).map_err(json_error)?;
                encode_result(self.cancel_calibration(request))
            }
            _ => Err(format!("native command is not implemented: {method}")),
        }
    }

    pub(crate) fn bootstrap(&self) -> Result<BootstrapDto, String> {
        crate::startup_telemetry::record("bootstrap.start");
        let settings = self.settings_snapshot()?;
        let (catalog_state, catalog_generation) = self.catalog_readiness()?;
        let settings_dto = settings_dto(&settings, self.timing_margin_recommendation());
        let result = BootstrapDto {
            app_version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: crate::DESKTOP_PROTOCOL_VERSION,
            native_build: native_build_dto(),
            settings: settings_dto.clone(),
            playback_defaults: settings_dto.playback_defaults.clone(),
            timing_margin_recommendation: settings_dto.timing_margin_recommendation.clone(),
            option_sets: crate::commands::PlaybackOptionSetsDto {
                hold_frames: vec![1.0, 1.25, 1.5],
                tempo_scales: vec![0.90, 0.95, 1.0, 1.05, 1.10],
                fps: sky_app_core::settings::VALID_FPS.to_vec(),
                timing_margin_min_us: sky_app_core::settings::MIN_TIMING_MARGIN_US,
                timing_margin_max_us: sky_app_core::settings::MAX_TIMING_MARGIN_US,
                timing_margin_step_us: sky_app_core::settings::TIMING_MARGIN_STEP_US,
            },
            palette: settings_dto.palette.clone(),
            theme: settings_dto.palette.clone(),
            telemetry_enabled: settings_dto.telemetry_enabled,
            update_preferences: settings_dto.update_preferences.clone(),
            catalog_state,
            catalog_generation,
            gui_smoke: false,
        };
        if !self.ready_emitted.swap(true, Ordering::AcqRel) {
            self.publish(UiEvent::CoreReady {
                v: crate::DESKTOP_PROTOCOL_VERSION,
                payload: CoreReadyPayload {
                    app_version: result.app_version.clone(),
                    protocol_version: result.protocol_version,
                    native_build: NativeBuildPayload {
                        native_build_commit: result.native_build.native_build_commit.clone(),
                        native_version: result.native_build.native_version.clone(),
                        schema_version: result.native_build.schema_version,
                        native_abi: result.native_build.native_abi.clone(),
                        rustc_version: result.native_build.rustc_version.clone(),
                        win32_backend: result.native_build.win32_backend,
                    },
                },
            })?;
        }
        crate::startup_telemetry::record("bootstrap.end");
        Ok(result)
    }

    fn settings_snapshot(&self) -> Result<ApplicationSettings, String> {
        let service = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        Ok(service.snapshot().clone())
    }

    fn reload_settings_snapshot(&self) -> Result<ApplicationSettings, String> {
        // External settings-file edits become authoritative only at explicit
        // refresh boundaries. Normal startup reads use the coherent in-memory
        // snapshot so background work cannot silently change startup state.
        crate::startup_telemetry::record("settings.reload.start");
        let mut prepare_publication = self
            .playback
            .prepare_publication
            .lock()
            .map_err(|_| "native Prepare publication lock poisoned".to_string())?;
        let mut service = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        let before = service.snapshot().clone();
        service
            .reload()
            .map_err(|error| format!("native settings reload failed: {error}"))?;
        let snapshot = service.snapshot().clone();
        if before.liked_songs != snapshot.liked_songs {
            self.catalog_search_cache
                .lock()
                .map_err(|_| "native catalog search cache lock poisoned".to_string())?
                .invalidate_liked();
            self.invalidate_viewport_projection();
        }
        if before.playback_defaults != snapshot.playback_defaults {
            self.invalidate_analysis_cache();
        }
        let fingerprint = settings_fingerprint(&snapshot)?;
        if prepare_publication
            .settings_fingerprint
            .as_ref()
            .is_some_and(|current| current != &fingerprint)
        {
            self.playback
                .invalidate_settings_locked(&mut prepare_publication, Some(fingerprint));
        } else {
            self.playback
                .set_settings_fingerprint_locked(&mut prepare_publication, fingerprint);
        }
        crate::startup_telemetry::record("settings.reload.end");
        Ok(snapshot)
    }

    fn reload_settings_dto(&self) -> Result<SettingsDto, String> {
        let settings = self.reload_settings_snapshot()?;
        Ok(settings_dto(&settings, self.timing_margin_recommendation()))
    }

    fn timing_margin_recommendation(&self) -> crate::commands::TimingMarginRecommendationDto {
        timing_margin_recommendation(self.paths.calibration_cache_path())
    }

    pub(crate) fn patch_settings(&self, patch: SettingsPatch) -> Result<SettingsDto, String> {
        if patch.update_preferences.as_ref().and_then(|u| u.channel)
            == Some(crate::ui_events::UpdateChannel::Beta)
        {
            return Err(
                "channel_unavailable: beta update channel is not supported in production".into(),
            );
        }
        let preparation_safe_patch = (patch.palette.is_some() || patch.auto_play.is_some())
            && patch.telemetry_enabled.is_none()
            && patch.verbose_hud.is_none()
            && patch.playback_defaults.is_none()
            && patch.update_preferences.is_none();
        let core_patch = sky_app_core::settings::SettingsPatch {
            palette: patch.palette,
            telemetry_enabled: patch.telemetry_enabled,
            verbose_hud: patch.verbose_hud,
            playback_defaults: patch.playback_defaults.map(|value| PlaybackDefaultsPatch {
                hold_frames: value.hold_frames,
                timing_margin_us: value.timing_margin_us,
                tempo_scale: value.tempo_scale,
                fps: value.fps,
            }),
            auto_play: patch.auto_play,
            update: patch
                .update_preferences
                .map(|value| CoreUpdatePreferencesPatch {
                    auto_check: value.auto_check,
                    channel: value.channel.map(|channel| match channel {
                        crate::ui_events::UpdateChannel::Stable => {
                            sky_app_core::settings::UpdateChannel::Stable
                        }
                        crate::ui_events::UpdateChannel::Beta => {
                            sky_app_core::settings::UpdateChannel::Beta
                        }
                    }),
                    skip_version: value.skip_version,
                }),
        };
        // The Prepare publication gate precedes the settings lock everywhere
        // settings can affect prepared plans. This makes mutation plus cache
        // invalidation atomic with respect to Prepare's final publication.
        let mut prepare_publication = self
            .playback
            .prepare_publication
            .lock()
            .map_err(|_| "native Prepare publication lock poisoned".to_string())?;
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        let before_fingerprint = settings_fingerprint(settings.snapshot())?;
        let before_channel = settings.snapshot().update.channel.clone();
        let before_skip_version = settings.snapshot().update.skip_version.clone();
        let snapshot = settings.patch(&core_patch).map_err(settings_error)?.clone();
        let candidate_invalidation_needed = before_channel != snapshot.update.channel
            || before_skip_version != snapshot.update.skip_version;
        let after_fingerprint = settings_fingerprint(&snapshot)?;
        if !preparation_safe_patch || before_fingerprint != after_fingerprint {
            self.playback
                .invalidate_settings_locked(&mut prepare_publication, Some(after_fingerprint));
        } else {
            self.playback
                .set_settings_fingerprint_locked(&mut prepare_publication, after_fingerprint);
        }
        if !preparation_safe_patch {
            self.invalidate_analysis_cache();
        }
        drop(settings);
        drop(prepare_publication);
        if candidate_invalidation_needed && let Some(update_service) = &self.update_service {
            let channel = match snapshot.update.channel {
                sky_app_core::settings::UpdateChannel::Stable => {
                    crate::ui_events::UpdateChannel::Stable
                }
                sky_app_core::settings::UpdateChannel::Beta => {
                    crate::ui_events::UpdateChannel::Beta
                }
            };
            update_service.reset_and_publish_idle(channel, |event| self.publish(event))?;
        }
        Ok(settings_dto(&snapshot, self.timing_margin_recommendation()))
    }

    fn update_preferences(&self) -> Result<UpdatePreferencesDto, String> {
        let settings = self.reload_settings_snapshot()?;
        Ok(update_preferences_dto(&settings))
    }

    fn check_update(&self, request: UpdateCheckRequest) -> Result<UpdateCheckAckDto, String> {
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        self.update_service
            .as_ref()
            .ok_or_else(|| {
                "update_service_unavailable: Rust-owned UpdateService is not configured".to_string()
            })?
            .check(&request, &mut settings, |event| self.publish(event))
    }

    pub(crate) fn update_snapshot(&self) -> Result<UpdateSnapshotPayload, String> {
        self.update_service
            .as_ref()
            .ok_or_else(|| {
                "update_service_unavailable: Rust-owned UpdateService is not configured".to_string()
            })
            .map(|service| service.current_snapshot())
    }

    fn begin_update_handoff(&self, target_version: String) -> Result<UpdateInstallAckDto, String> {
        if target_version.is_empty() || target_version.len() > 64 || target_version.contains('\0') {
            return Err("invalid_params: target_version is invalid".into());
        }
        let settings = self.reload_settings_snapshot()?;
        self.update_service
            .as_ref()
            .ok_or_else(|| {
                "update_service_unavailable: Rust-owned UpdateService is not configured".to_string()
            })?
            .install(&settings, &target_version, |event| self.publish(event))
    }

    pub(crate) fn patch_update_preferences(
        &self,
        patch: UpdatePreferencesPatch,
    ) -> Result<UpdatePreferencesDto, String> {
        if patch.channel == Some(crate::ui_events::UpdateChannel::Beta) {
            return Err(
                "channel_unavailable: beta update channel is not supported in production".into(),
            );
        }
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        let before_channel = settings.snapshot().update.channel.clone();
        let before_skip_version = settings.snapshot().update.skip_version.clone();
        let snapshot = settings
            .patch(&sky_app_core::settings::SettingsPatch {
                update: Some(CoreUpdatePreferencesPatch {
                    auto_check: patch.auto_check,
                    channel: patch.channel.map(|channel| match channel {
                        crate::ui_events::UpdateChannel::Stable => {
                            sky_app_core::settings::UpdateChannel::Stable
                        }
                        crate::ui_events::UpdateChannel::Beta => {
                            sky_app_core::settings::UpdateChannel::Beta
                        }
                    }),
                    skip_version: patch.skip_version,
                }),
                ..Default::default()
            })
            .map_err(settings_error)?
            .clone();
        let candidate_invalidation_needed = before_channel != snapshot.update.channel
            || before_skip_version != snapshot.update.skip_version;
        drop(settings);
        if candidate_invalidation_needed && let Some(update_service) = &self.update_service {
            let channel = match snapshot.update.channel {
                sky_app_core::settings::UpdateChannel::Stable => {
                    crate::ui_events::UpdateChannel::Stable
                }
                sky_app_core::settings::UpdateChannel::Beta => {
                    crate::ui_events::UpdateChannel::Beta
                }
            };
            update_service.reset_and_publish_idle(channel, |event| self.publish(event))?;
        }
        Ok(update_preferences_dto(&snapshot))
    }

    fn prepare_playback(
        &self,
        request: crate::commands::PlaybackPrepareRequest,
    ) -> Result<PreparedPlaybackDto, String> {
        let cancelled = self.playback.begin_preparation()?;
        let result = (|| {
            self.ensure_catalog_loaded()?;
            let (source_epoch, (path, fallback, source_fingerprint)) =
                self.playback.capture_preparation_sources(&cancelled, || {
                    let (path, fallback, generation) = {
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
                            catalog.index.generation(),
                        )
                    };
                    let settings = self.settings_snapshot()?;
                    let fingerprint = settings_fingerprint(&settings)?;
                    Ok((
                        generation,
                        fingerprint.clone(),
                        (path, fallback, fingerprint),
                    ))
                })?;

            // The catalog and settings locks are no longer held across file
            // I/O, parsing, schedule construction, or whole-stream analysis.
            let bytes = fs::read(&path).map_err(|error| format!("song read failed: {error}"))?;
            let fallback = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or(&fallback);
            let song = parse_song_json(&bytes, fallback).map_err(|error| error.to_string())?;
            let policy = self.timing_policy(
                request.config.fps,
                request.config.hold_frames,
                request.config.timing_margin_us,
            )?;
            let timing_margin_recommendation = self.timing_margin_recommendation();
            let schedule = build_schedule_with_policy(&song, request.config.tempo_scale, &policy)
                .map_err(|error| error.to_string())?;
            let risk = analyze_schedule_with_context(
                &schedule,
                Some(&song.notes),
                request.config.hold_frames,
                request.config.tempo_scale,
            );
            self.playback.prepare_with_token(
                NativePreparedInput {
                    song_id: request.song_id,
                    generation: request.generation,
                    config: request.config,
                    song,
                    schedule,
                    risk,
                    timing_policy: policy,
                    timing_margin_recommendation,
                    settings_fingerprint: source_fingerprint,
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

    fn start_calibration(
        &self,
        request: CalibrationStartRequest,
    ) -> Result<CalibrationStartAckDto, String> {
        self.calibration.start(request)
    }

    fn cancel_calibration(
        &self,
        request: CalibrationCancelRequest,
    ) -> Result<CalibrationCancelAckDto, String> {
        self.calibration.cancel(request)
    }

    pub(crate) fn wait_for_calibration_terminal(
        &self,
        timeout: Duration,
    ) -> Result<CalibrationState, String> {
        self.calibration.wait_for_terminal(timeout)
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

mod calibration;
mod catalog;
mod catalog_cache;
mod catalog_reconcile;
mod catalog_search;
#[cfg(test)]
use catalog_cache::AnalysisTestGate;
use catalog_cache::{
    AnalysisAdmissionState, AnalysisCacheState, AnalysisKey, AnalysisPriority, AnalysisRequest,
    CatalogSearchCache, CatalogSearchKey, CatalogSearchScope, ViewportProjectionKey,
    ViewportResponseProjection,
};
mod playback;
#[cfg(test)]
use playback::*;
use playback::{
    NativePlaybackPrepareRequest, NativePlaybackService, NativePlaybackSessionRequest,
    NativePlaybackStartRequest, NativePreparedInput, opaque_native_id, settings_fingerprint,
    validate_playback_start_request, validate_public_playback_start_request,
};

#[cfg(test)]
pub(crate) fn resolve_install_root() -> Result<PathBuf, String> {
    AppPaths::resolve().map(|paths| paths.install_root().to_path_buf())
}

pub(crate) fn native_build_dto() -> crate::commands::NativeBuildDto {
    crate::commands::NativeBuildDto {
        native_build_commit: option_env!("SKY_NATIVE_BUILD_COMMIT")
            .unwrap_or("unknown")
            .into(),
        native_version: env!("CARGO_PKG_VERSION").into(),
        schema_version: sky_dispatch_win32::calibration::CALIBRATION_SCHEMA_VERSION as u64,
        native_abi: option_env!("SKY_NATIVE_ABI")
            .unwrap_or("native-win32")
            .into(),
        rustc_version: option_env!("SKY_RUSTC_VERSION").unwrap_or("unknown").into(),
        win32_backend: sky_dispatch_win32::win32_available(),
    }
}

fn playback_defaults(settings: &ApplicationSettings) -> PlaybackDefaultsDto {
    PlaybackDefaultsDto {
        hold_frames: settings.playback_defaults.hold_frames,
        timing_margin_us: settings.playback_defaults.timing_margin_us,
        tempo_scale: settings.playback_defaults.tempo_scale,
        fps: settings.playback_defaults.fps,
        dry_run: false,
    }
}

fn update_preferences_dto(settings: &ApplicationSettings) -> UpdatePreferencesDto {
    UpdatePreferencesDto {
        auto_check: settings.update.auto_check,
        channel: match settings.update.channel {
            sky_app_core::settings::UpdateChannel::Stable => {
                crate::ui_events::UpdateChannel::Stable
            }
            sky_app_core::settings::UpdateChannel::Beta => crate::ui_events::UpdateChannel::Beta,
        },
        skip_version: settings.update.skip_version.clone(),
        check_interval_s: settings.update.check_interval_s,
        last_check_ts: settings.update.last_check_ts,
        last_error_ts: settings.update.last_error_ts,
    }
}

fn settings_dto(
    settings: &ApplicationSettings,
    timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto,
) -> SettingsDto {
    SettingsDto {
        palette: settings.palette.clone(),
        ui_background_mode: settings.ui_background_mode.clone(),
        playback_defaults: playback_defaults(settings),
        auto_play: settings.playback_behavior.auto_play,
        timing_margin_recommendation,
        telemetry_enabled: settings.telemetry_enabled,
        verbose_hud: settings.verbose_hud,
        update_preferences: update_preferences_dto(settings),
    }
}

fn json_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn bounded_text(value: impl AsRef<str>) -> String {
    value.as_ref().chars().take(4096).collect()
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct NativeCatalogDetailRequest {
    song_id: String,
    generation: Option<u64>,
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
        ActivityReservationError::CalibrationAlreadyActive => {
            "calibration_active: calibration is active".into()
        }
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct NativeEventSnapshotMetrics {
    snapshot_coalesced: u64,
    snapshot_dropped: u64,
}

#[derive(Default)]
struct NativeEventHubState {
    buffered: VecDeque<UiEvent>,
    channel: Option<Channel<UiEvent>>,
    closed: bool,
    in_flight: bool,
    in_flight_snapshot: bool,
    delivery_failure: Option<String>,
    playback_player: Option<std::sync::Weak<NativeDispatchSession>>,
}

#[derive(Default)]
struct NativeEventQueue {
    state: Mutex<NativeEventHubState>,
    changed: Condvar,
    publisher_finished: AtomicBool,
    delivery_failure: Arc<AtomicBool>,
    snapshot_coalesced: AtomicU64,
    snapshot_dropped: AtomicU64,
}

impl NativeEventQueue {
    fn fail_delivery(
        &self,
        reason: &str,
        in_flight_send_failed: bool,
    ) -> Option<Arc<NativeDispatchSession>> {
        self.delivery_failure.store(true, Ordering::Release);
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state
            .delivery_failure
            .get_or_insert_with(|| reason.to_string());
        state.closed = true;
        let buffered_snapshots = count_buffered_snapshots(&state.buffered);
        if buffered_snapshots > 0 {
            self.snapshot_dropped
                .fetch_add(buffered_snapshots, Ordering::Relaxed);
        }
        if in_flight_send_failed && state.in_flight_snapshot {
            self.snapshot_dropped.fetch_add(1, Ordering::Relaxed);
        }
        if in_flight_send_failed {
            state.in_flight = false;
            state.in_flight_snapshot = false;
        }
        state.channel = None;
        state.buffered.clear();
        let player = state
            .playback_player
            .as_ref()
            .and_then(std::sync::Weak::upgrade);
        drop(state);
        self.changed.notify_all();
        player
    }
}

pub(crate) struct NativeEventHub {
    queue: Arc<NativeEventQueue>,
    publisher: Option<thread::JoinHandle<()>>,
}

impl Default for NativeEventHub {
    fn default() -> Self {
        Self {
            queue: Arc::new(NativeEventQueue::default()),
            publisher: None,
        }
    }
}

impl NativeEventHub {
    fn publish(&mut self, event: UiEvent) -> Result<(), String> {
        validate_ui_event(&event)?;
        let is_snapshot = snapshot_key(&event).is_some();
        let mut failed_player = None;
        let result = {
            let mut state = match self.queue.state.lock() {
                Ok(state) => state,
                Err(_) => {
                    let player = self
                        .queue
                        .fail_delivery("native event queue lock poisoned", false);
                    if let Some(player) = player {
                        request_event_delivery_termination(&player);
                    }
                    return Err("native event queue lock poisoned".into());
                }
            };
            if state.closed {
                state
                    .delivery_failure
                    .get_or_insert_with(|| "native event hub is closed".into());
                self.queue.delivery_failure.store(true, Ordering::Release);
                return Err("native event hub is closed".into());
            }
            if let Some(error) = &state.delivery_failure {
                self.queue.delivery_failure.store(true, Ordering::Release);
                return Err(error.clone());
            }

            if let Some(key) = snapshot_key(&event)
                && let Some(existing) = state
                    .buffered
                    .iter_mut()
                    .find(|candidate| snapshot_key(candidate).as_ref() == Some(&key))
            {
                *existing = event;
                self.queue
                    .snapshot_coalesced
                    .fetch_add(1, Ordering::Relaxed);
                self.queue.snapshot_dropped.fetch_add(1, Ordering::Relaxed);
                Ok(())
            } else {
                let occupied = state.buffered.len() + usize::from(state.in_flight);
                let at_capacity = occupied >= MAX_NATIVE_EVENTS;
                let evicted_snapshot = at_capacity && remove_oldest_snapshot(&mut state.buffered);
                if evicted_snapshot {
                    self.queue.snapshot_dropped.fetch_add(1, Ordering::Relaxed);
                }
                let has_room = !at_capacity || evicted_snapshot;
                if has_room {
                    state.buffered.push_back(event);
                    Ok(())
                } else if is_snapshot {
                    // Snapshots are best-effort. A full queue containing only
                    // ordered lifecycle events leaves no older snapshot to
                    // evict, so this update is dropped without affecting the
                    // lifecycle delivery contract.
                    self.queue.snapshot_dropped.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                } else {
                    let error = "native event hub lifecycle buffer overflow".to_string();
                    state.delivery_failure = Some(error.clone());
                    self.queue.delivery_failure.store(true, Ordering::Release);
                    failed_player = state
                        .playback_player
                        .as_ref()
                        .and_then(std::sync::Weak::upgrade);
                    Err(error)
                }
            }
        };
        self.queue.changed.notify_one();
        if let Some(player) = failed_player {
            request_event_delivery_termination(&player);
        }
        result
    }

    pub(crate) fn subscribe(&mut self, channel: Channel<UiEvent>) -> Result<(), String> {
        {
            let state = match self.queue.state.lock() {
                Ok(state) => state,
                Err(_) => {
                    if let Some(player) = self.queue.fail_delivery(
                        "native event queue lock poisoned during subscription",
                        false,
                    ) {
                        request_event_delivery_termination(&player);
                    }
                    return Err("native event queue lock poisoned".into());
                }
            };
            if state.closed {
                return Err("native event hub is closed".into());
            }
            if state.channel.is_some() {
                return Err("native event hub already has a subscriber".into());
            }
        }

        if self.publisher.is_none() {
            let queue = Arc::clone(&self.queue);
            match thread::Builder::new()
                .name("sky-native-ui-event-publisher".into())
                .spawn(move || publish_native_events(queue))
            {
                Ok(publisher) => self.publisher = Some(publisher),
                Err(error) => {
                    let failure = format!("could not start native UI event publisher: {error}");
                    let player = self.queue.fail_delivery(&failure, false);
                    if let Some(player) = player {
                        request_event_delivery_termination(&player);
                    }
                    return Err(failure);
                }
            }
        }

        let mut state = match self.queue.state.lock() {
            Ok(state) => state,
            Err(_) => {
                if let Some(player) = self
                    .queue
                    .fail_delivery("native event queue lock poisoned while subscribing", false)
                {
                    request_event_delivery_termination(&player);
                }
                return Err("native event queue lock poisoned".into());
            }
        };
        if state.closed {
            return Err("native event hub is closed".into());
        }
        state.channel = Some(channel);
        drop(state);
        self.queue.changed.notify_one();
        Ok(())
    }

    fn register_playback_player(
        &mut self,
        player: std::sync::Weak<NativeDispatchSession>,
    ) -> Result<(), String> {
        let mut state = match self.queue.state.lock() {
            Ok(state) => state,
            Err(_) => {
                self.queue.fail_delivery(
                    "native event queue lock poisoned while registering playback",
                    false,
                );
                return Err("native event queue lock poisoned".into());
            }
        };
        if state.closed {
            return Err("native event hub is closed".into());
        }
        state.playback_player = Some(player);
        Ok(())
    }

    fn failure_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.queue.delivery_failure)
    }

    fn snapshot_metrics(&self) -> NativeEventSnapshotMetrics {
        NativeEventSnapshotMetrics {
            snapshot_coalesced: self.queue.snapshot_coalesced.load(Ordering::Acquire),
            snapshot_dropped: self.queue.snapshot_dropped.load(Ordering::Acquire),
        }
    }

    #[cfg(test)]
    fn delivery_failure(&self) -> Option<String> {
        match self.queue.state.lock() {
            Ok(state) => state.delivery_failure.clone().or_else(|| {
                self.queue
                    .delivery_failure
                    .load(Ordering::Acquire)
                    .then(|| "native lifecycle event delivery failed".into())
            }),
            Err(_) => {
                self.queue
                    .fail_delivery("native event queue lock poisoned", false);
                Some("native event queue lock poisoned".to_string())
            }
        }
    }

    pub(crate) fn close(&mut self) {
        let player = match self.queue.state.lock() {
            Ok(mut state) => {
                state.closed = true;
                let buffered_snapshots = count_buffered_snapshots(&state.buffered);
                if buffered_snapshots > 0 {
                    self.queue
                        .snapshot_dropped
                        .fetch_add(buffered_snapshots, Ordering::Relaxed);
                }
                state.buffered.clear();
                let player = state
                    .playback_player
                    .as_ref()
                    .and_then(std::sync::Weak::upgrade);
                state.playback_player = None;
                player
            }
            Err(poisoned) => {
                self.queue.delivery_failure.store(true, Ordering::Release);
                let mut state = poisoned.into_inner();
                state.closed = true;
                let buffered_snapshots = count_buffered_snapshots(&state.buffered);
                if buffered_snapshots > 0 {
                    self.queue
                        .snapshot_dropped
                        .fetch_add(buffered_snapshots, Ordering::Relaxed);
                }
                state.buffered.clear();
                let player = state
                    .playback_player
                    .as_ref()
                    .and_then(std::sync::Weak::upgrade);
                state.playback_player = None;
                player
            }
        };
        if player.is_some() {
            self.queue.delivery_failure.store(true, Ordering::Release);
        }
        if let Some(player) = player {
            request_event_delivery_termination(&player);
        }
        self.queue.changed.notify_all();

        // A platform channel send is not cancellable. Never make physical
        // shutdown wait indefinitely for it; this is one app-scoped publisher,
        // not a per-session thread or a physical input owner.
        if let Some(publisher) = self.publisher.take() {
            let deadline = Instant::now() + EVENT_PUBLISHER_CLOSE_BUDGET;
            while !publisher.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            if publisher.is_finished() {
                let _ = publisher.join();
            }
        }
        report_native_event_snapshot_metrics(&self.queue);
    }

    #[cfg(test)]
    fn buffered_for_test(&self) -> VecDeque<UiEvent> {
        self.queue
            .state
            .lock()
            .expect("event queue")
            .buffered
            .iter()
            .cloned()
            .collect()
    }

    #[cfg(test)]
    fn publisher_finished_for_test(&self) -> bool {
        self.queue.publisher_finished.load(Ordering::Acquire)
    }

    #[cfg(test)]
    fn wait_idle_for_test(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let Ok(mut state) = self.queue.state.lock() else {
            return false;
        };
        loop {
            if state.buffered.is_empty() && !state.in_flight {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            match self.queue.changed.wait_timeout(state, remaining) {
                Ok((next, result)) => {
                    state = next;
                    if result.timed_out() {
                        return state.buffered.is_empty() && !state.in_flight;
                    }
                }
                Err(_) => return false,
            }
        }
    }
}

impl Drop for NativeEventHub {
    fn drop(&mut self) {
        self.close();
    }
}

fn publish_native_events(queue: Arc<NativeEventQueue>) {
    loop {
        let (channel, event) = {
            let mut state = match queue.state.lock() {
                Ok(state) => state,
                Err(_) => {
                    fail_native_event_publisher(&queue, "native event queue lock poisoned");
                    return;
                }
            };
            loop {
                if state.closed && state.buffered.is_empty() {
                    let channel = state.channel.take();
                    drop(state);
                    drop(channel);
                    queue.publisher_finished.store(true, Ordering::Release);
                    queue.changed.notify_all();
                    return;
                }
                if !state.buffered.is_empty()
                    && let Some(channel) = state.channel.clone()
                    && let Some(event) = state.buffered.pop_front()
                {
                    state.in_flight = true;
                    state.in_flight_snapshot = snapshot_key(&event).is_some();
                    break (channel, event);
                }
                state = match queue.changed.wait(state) {
                    Ok(state) => state,
                    Err(_) => {
                        fail_native_event_publisher(
                            &queue,
                            "native event queue lock poisoned while waiting",
                        );
                        return;
                    }
                };
            }
        };

        let sent = catch_unwind(AssertUnwindSafe(|| channel.send(event)))
            .map_err(|_| "native UI event delivery panicked".to_string())
            .and_then(|result| {
                result.map_err(|error| format!("native UI event delivery failed: {error}"))
            });
        match sent {
            Ok(()) => {
                match queue.state.lock() {
                    Ok(mut state) => {
                        state.in_flight = false;
                        state.in_flight_snapshot = false;
                    }
                    Err(poisoned) => {
                        let mut state = poisoned.into_inner();
                        state.in_flight = false;
                        state.in_flight_snapshot = false;
                        drop(state);
                        fail_native_event_publisher(
                            &queue,
                            "native event queue lock poisoned after channel delivery",
                        );
                        return;
                    }
                }
                queue.changed.notify_all();
            }
            Err(error) => {
                fail_native_event_publisher(&queue, &error);
                return;
            }
        }
    }
}

fn fail_native_event_publisher(queue: &NativeEventQueue, reason: &str) {
    if let Some(player) = queue.fail_delivery(reason, true) {
        request_event_delivery_termination(&player);
    }
    queue.publisher_finished.store(true, Ordering::Release);
    report_native_event_snapshot_metrics(queue);
}

fn report_native_event_snapshot_metrics(queue: &NativeEventQueue) {
    let _ = writeln!(
        std::io::stderr().lock(),
        "native_event_snapshots coalesced={} dropped={}",
        queue.snapshot_coalesced.load(Ordering::Acquire),
        queue.snapshot_dropped.load(Ordering::Acquire),
    );
}

fn request_event_delivery_termination(player: &NativeDispatchSession) {
    let _ = player.panic_release();
    let _ = player.quit();
}

fn validate_ui_event(event: &UiEvent) -> Result<(), String> {
    match event {
        UiEvent::CoreReady { payload, .. } => UiEvent::validate_ready(payload),
        UiEvent::CoreFatal { payload, .. } => UiEvent::validate_fatal(payload),
        UiEvent::CatalogChanged { payload, .. } => UiEvent::validate_catalog_changed(payload),
        UiEvent::CatalogLoadFailed { payload, .. } => {
            UiEvent::validate_catalog_load_failed(payload)
        }
        UiEvent::PlaybackStateChanged { payload, .. } => {
            UiEvent::validate_playback_state_changed(payload)
        }
        UiEvent::PlaybackSnapshot { payload, .. } => UiEvent::validate_playback_snapshot(payload),
        UiEvent::PlaybackFinished { payload, .. } => UiEvent::validate_playback_finished(payload),
        UiEvent::PlaybackFailed { payload, .. } => UiEvent::validate_playback_failed(payload),
        UiEvent::DiagnosticsSnapshot { payload, .. } => {
            UiEvent::validate_diagnostics_snapshot(payload)
        }
        UiEvent::CalibrationProgress { payload, .. } => {
            UiEvent::validate_calibration_progress(payload)
        }
        UiEvent::CalibrationFinished { payload, .. } => {
            UiEvent::validate_calibration_finished(payload)
        }
        UiEvent::UpdateChanged { payload, .. } => UiEvent::validate_update_changed(payload),
    }
}

fn snapshot_key(event: &UiEvent) -> Option<(u8, String)> {
    match event {
        UiEvent::PlaybackSnapshot { payload, .. } => Some((1, payload.session_id.clone())),
        UiEvent::DiagnosticsSnapshot { payload, .. } => {
            Some((2, payload.session_id.clone().unwrap_or_default()))
        }
        UiEvent::CalibrationProgress { payload, .. } => Some((3, payload.operation_id.clone())),
        _ => None,
    }
}

fn count_buffered_snapshots(buffered: &VecDeque<UiEvent>) -> u64 {
    buffered
        .iter()
        .filter(|event| snapshot_key(event).is_some())
        .count() as u64
}

/// Match the Python diagnostics percentile contract.  Python's ``round``
/// uses ties-to-even, so using `f64::round()` here would diverge for small
/// sample sets at an exact half index.
fn percentile_ms(values: &[i64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let position = fraction * (ordered.len() - 1) as f64;
    let lower = position.floor();
    let fractional = position - lower;
    let index = if fractional < 0.5 {
        lower as usize
    } else if fractional > 0.5 || (lower as usize) % 2 == 1 {
        (lower as usize + 1).min(ordered.len() - 1)
    } else {
        lower as usize
    };
    ordered[index] as f64 / 1000.0
}

fn population_sigma_ms(values: &[i64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().map(|value| *value as f64).sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| {
            let delta = *value as f64 - mean;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt() / 1000.0
}

fn remove_oldest_snapshot(buffered: &mut VecDeque<UiEvent>) -> bool {
    let Some(index) = buffered
        .iter()
        .position(|event| snapshot_key(event).is_some())
    else {
        return false;
    };
    buffered.remove(index).is_some()
}

#[cfg(test)]
#[path = "native_runtime/tests.rs"]
mod tests;
