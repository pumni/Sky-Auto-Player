use crate::app_state::ActivityCoordinator;
use crate::commands::BootstrapDto;
use crate::native_runtime::catalog::compute_catalog_source_static_fingerprint;
use crate::native_runtime::{
    AnalysisAdmissionState, AnalysisCacheState, CatalogLoadState, CatalogSearchCache, CatalogState,
    CoreUpdatePreferencesPatch, NativeDesktopRuntime, NativeEventHub, NativePlaybackService,
    native_build_dto, settings_dto,
};
use crate::ui_events::{CoreReadyPayload, NativeBuildPayload, UiEvent};
use sky_app_core::library::LibraryManifestService;
use sky_app_core::settings::SettingsService;
use sky_native_adapters::{
    AppPaths, AppResources, BuiltinCatalogSource, CatalogCacheStore, CatalogComposer,
    FileCatalogSource, JsonLibraryManifestStore, JsonSettingsStore,
};
#[cfg(test)]
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

impl NativeDesktopRuntime {
    #[cfg(test)]
    pub(crate) fn from_install_root(install_root: PathBuf) -> Result<Self, String> {
        Self::from_install_root_with_activity(install_root, ActivityCoordinator::default())
    }

    pub(crate) fn from_paths_with_activity(
        paths: AppPaths,
        resources: AppResources,
        activity: ActivityCoordinator,
    ) -> Result<Self, String> {
        Self::from_paths_with_activity_and_update_service(paths, resources, activity, None)
    }

    pub(crate) fn from_paths_with_activity_and_update_service(
        paths: AppPaths,
        resources: AppResources,
        activity: ActivityCoordinator,
        update_service: Option<Arc<crate::native_update::UpdateService<crate::ShellRuntime>>>,
    ) -> Result<Self, String> {
        Self::from_paths_with_activity_and_update_service_with_events(
            paths,
            resources,
            activity,
            update_service,
            Arc::new(Mutex::new(NativeEventHub::default())),
        )
    }

    pub(crate) fn from_paths_with_activity_and_update_service_with_events(
        paths: AppPaths,
        resources: AppResources,
        activity: ActivityCoordinator,
        update_service: Option<Arc<crate::native_update::UpdateService<crate::ShellRuntime>>>,
        events: Arc<Mutex<NativeEventHub>>,
    ) -> Result<Self, String> {
        paths.assert_clean_boundary()?;
        Self::from_paths_internal(paths, resources, activity, update_service, events)
    }

    fn from_paths_internal(
        paths: AppPaths,
        resources: AppResources,
        activity: ActivityCoordinator,
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
        let safety_activity = activity.clone();
        let safety_playback = playback.clone();
        let safety_events = events.clone();
        let pre_exit_safety: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            safety_activity.begin_shutdown();
            crate::record_update_safety_phase("activity.admission_closed");
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
            #[cfg(test)]
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

    #[cfg(test)]
    pub(crate) fn from_install_root_with_activity(
        install_root: PathBuf,
        activity: ActivityCoordinator,
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
            None,
            Arc::new(Mutex::new(NativeEventHub::default())),
        )
    }

    #[cfg(test)]
    pub(crate) fn install_root(&self) -> &Path {
        self.paths.install_root()
    }
    pub(crate) fn bootstrap(&self) -> Result<BootstrapDto, String> {
        crate::startup_telemetry::record("bootstrap.start");
        let settings = self.settings_snapshot()?;
        let (catalog_state, catalog_generation) = self.catalog_readiness()?;
        let settings_dto = settings_dto(&settings);
        let result = BootstrapDto {
            app_version: env!("CARGO_PKG_VERSION").into(),
            protocol_version: crate::DESKTOP_PROTOCOL_VERSION,
            native_build: native_build_dto(),
            settings: settings_dto.clone(),
            playback_defaults: settings_dto.playback_defaults.clone(),
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
}
