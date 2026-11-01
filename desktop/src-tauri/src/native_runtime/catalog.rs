use super::{
    CachedSongAnalysis, CatalogCompositionSnapshot, CatalogLoadClaim, CatalogLoadState,
    CatalogLoadWork, CatalogMetadata, NativeDesktopRuntime, NativeLibraryImportRequest,
    SheetReadinessDto, SongAnalysisRequest, SongPlan, bounded_text, opaque_native_id,
    settings_error,
};
use crate::commands::{
    CatalogDetailRequest, CatalogReloadDto, CatalogRowDto, CatalogSetLikedDto,
    CatalogSetLikedRequest, CatalogViewportDto, CatalogViewportRequest,
    LibraryCreatePlaylistRequest, LibraryNavigationDto, LibraryPlaylistIdRequest,
    LibraryPlaylistImportResultDto, LibraryPlaylistSongsRequest, LibraryPlaylistSummaryDto,
    LibraryRenamePlaylistRequest, RiskSummaryDto, SongDetailDto,
};
use crate::ui_events::{
    CatalogChangedPayload, CatalogLoadFailedPayload, CatalogReadiness, UiEvent,
};
use sha2::{Digest, Sha256};
use sky_app_core::catalog::{CatalogError, CatalogSourceIdentity, StableSongId};
use sky_app_core::library::{ImportedSourceKind, ImportedSourceRef, LibraryError};
use sky_app_core::song::{analyze_schedule_with_context, build_song_plan};
use sky_native_adapters::{
    AppResources, BuiltinCatalogFailureCode, BuiltinCatalogStatus, CatalogCache, CatalogCacheEntry,
    CatalogCacheSourceKind, CatalogComposition, ImportedSourceCatalogStatus,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
impl NativeDesktopRuntime {
    pub(crate) fn catalog_readiness(&self) -> Result<(CatalogReadiness, Option<u64>), String> {
        let state = self
            .catalog_load
            .lock()
            .map_err(|_| "native catalog load state lock poisoned".to_string())?;
        Ok(match &*state {
            CatalogLoadState::Uninitialized => (CatalogReadiness::Uninitialized, None),
            CatalogLoadState::Loading => (CatalogReadiness::Loading, None),
            CatalogLoadState::Cached { generation }
            | CatalogLoadState::Reconciling { generation }
            | CatalogLoadState::Ready { generation } => {
                (CatalogReadiness::Ready, Some(*generation))
            }
            CatalogLoadState::Failed { .. } | CatalogLoadState::Closing => {
                (CatalogReadiness::Failed, None)
            }
        })
    }

    pub(crate) fn start_catalog_load(self: &Arc<Self>) -> Result<(), String> {
        if self.closed.load(Ordering::Acquire) {
            return Err("closing: desktop application is closing".into());
        }
        let work = self.claim_catalog_load()?;
        if let Some(claim) = work {
            let runtime = Arc::clone(self);
            tauri::async_runtime::spawn_blocking(move || runtime.run_catalog_load(claim));
        }
        Ok(())
    }

    pub(super) fn claim_catalog_load(&self) -> Result<Option<CatalogLoadClaim>, String> {
        let mut state = self
            .catalog_load
            .lock()
            .map_err(|_| "native catalog load state lock poisoned".to_string())?;
        match &*state {
            CatalogLoadState::Loading
            | CatalogLoadState::Reconciling { .. }
            | CatalogLoadState::Ready { .. } => Ok(None),
            CatalogLoadState::Closing => Err("closing: desktop application is closing".into()),
            CatalogLoadState::Cached { generation } => {
                let epoch = self.catalog_load_epoch.fetch_add(1, Ordering::AcqRel) + 1;
                *state = CatalogLoadState::Reconciling {
                    generation: *generation,
                };
                Ok(Some(CatalogLoadClaim {
                    work: CatalogLoadWork::Reconcile,
                    epoch,
                }))
            }
            CatalogLoadState::Uninitialized | CatalogLoadState::Failed { .. } => {
                let epoch = self.catalog_load_epoch.fetch_add(1, Ordering::AcqRel) + 1;
                *state = CatalogLoadState::Loading;
                Ok(Some(CatalogLoadClaim {
                    work: CatalogLoadWork::Rebuild,
                    epoch,
                }))
            }
        }
    }

    fn catalog_load_claim_is_current(
        &self,
        state: &CatalogLoadState,
        claim: CatalogLoadClaim,
    ) -> bool {
        if self.catalog_load_epoch.load(Ordering::Acquire) != claim.epoch {
            return false;
        }
        match claim.work {
            CatalogLoadWork::Rebuild => matches!(state, CatalogLoadState::Loading),
            CatalogLoadWork::Reconcile => matches!(state, CatalogLoadState::Reconciling { .. }),
        }
    }

    pub(super) fn run_catalog_load(&self, claim: CatalogLoadClaim) {
        let Ok(state) = self.catalog_load.lock() else {
            return;
        };
        if !self.catalog_load_claim_is_current(&state, claim) {
            return;
        }
        drop(state);

        crate::startup_telemetry::record(match claim.work {
            CatalogLoadWork::Reconcile => "catalog.reconcile.start",
            CatalogLoadWork::Rebuild => "catalog.rebuild.start",
        });
        crate::startup_telemetry::record("catalog.compose.start");
        let composition = self.catalog_composition_snapshot();
        crate::startup_telemetry::record("catalog.compose.end");
        let Ok(mut state) = self.catalog_load.lock() else {
            return;
        };
        if !self.catalog_load_claim_is_current(&state, claim) {
            return;
        }
        if self.closed.load(Ordering::Acquire) || matches!(*state, CatalogLoadState::Closing) {
            *state = CatalogLoadState::Closing;
            return;
        }
        let result = match composition {
            Ok(snapshot) => self.apply_catalog_snapshot_if_current(snapshot, false, true),
            Err(error) => Err(error),
        };
        match result {
            Ok(Some((snapshot, library_total))) => {
                *state = CatalogLoadState::Ready {
                    generation: snapshot.generation,
                };
                crate::startup_telemetry::record(match claim.work {
                    CatalogLoadWork::Reconcile => "catalog.reconcile.end",
                    CatalogLoadWork::Rebuild => "catalog.rebuild.end",
                });
                crate::startup_telemetry::record("catalog.reconciled");
                let _ = self.publish(UiEvent::CatalogChanged {
                    v: crate::DESKTOP_PROTOCOL_VERSION,
                    payload: CatalogChangedPayload {
                        generation: snapshot.generation,
                        total: library_total as u64,
                    },
                });
            }
            Ok(None) => {}
            Err(error) => {
                *state = CatalogLoadState::Failed {
                    message: error.clone(),
                };
                let event_message = bounded_text(&error);
                let _ = self.publish(UiEvent::CatalogLoadFailed {
                    v: crate::DESKTOP_PROTOCOL_VERSION,
                    payload: CatalogLoadFailedPayload {
                        message: event_message,
                    },
                });
            }
        }
    }

    fn apply_catalog_snapshot_if_current(
        &self,
        snapshot: CatalogCompositionSnapshot,
        invalidate: bool,
        record_ready: bool,
    ) -> Result<Option<(sky_app_core::catalog::CatalogSnapshot, usize)>, String> {
        let manifest = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?;
        let current_fingerprint = compute_catalog_source_fingerprint(
            &self.catalog_source_static_fingerprint,
            &manifest.snapshot().imports,
        );
        if current_fingerprint != snapshot.source_fingerprint {
            return Ok(None);
        }
        let cache = self
            .catalog_cache_from_composition(&snapshot.composition, &snapshot.source_fingerprint)
            .ok();
        let result =
            self.apply_catalog_composition(snapshot.composition, invalidate, record_ready)?;
        if let Some(cache) = cache {
            self.persist_catalog_cache_locked(&cache);
        }
        Ok(Some(result))
    }

    pub(super) fn hydrate_cached_catalog(&self, cache: CatalogCache) -> Result<(), String> {
        if cache.source_fingerprint != self.catalog_source_fingerprint()? {
            return Err("catalog cache source fingerprint mismatch".into());
        }
        let composition = cached_composition(&cache)?;
        let (snapshot, _) = self.apply_catalog_composition(composition, false, false)?;
        let mut state = self
            .catalog_load
            .lock()
            .map_err(|_| "native catalog load state lock poisoned".to_string())?;
        *state = CatalogLoadState::Cached {
            generation: snapshot.generation,
        };
        crate::startup_telemetry::record("catalog.cached_available");
        Ok(())
    }

    fn persist_catalog_cache_locked(&self, cache: &CatalogCache) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let _ = self.catalog_cache.save(cache);
        if !self.closed.load(Ordering::Acquire)
            && let Ok(mut cached_catalog) = self.cached_catalog.lock()
        {
            *cached_catalog = Some(cache.clone());
        }
    }

    pub(super) fn catalog_source_fingerprint(&self) -> Result<String, String> {
        let imports = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?
            .snapshot()
            .imports
            .clone();
        Ok(compute_catalog_source_fingerprint(
            &self.catalog_source_static_fingerprint,
            &imports,
        ))
    }

    #[cfg(test)]
    pub(super) fn set_catalog_compose_barrier(
        &self,
        entered: Arc<std::sync::Barrier>,
        release: Arc<std::sync::Barrier>,
    ) {
        *self
            .catalog_compose_barrier
            .lock()
            .expect("catalog compose barrier") = Some((entered, release));
    }

    fn catalog_composition_snapshot(&self) -> Result<CatalogCompositionSnapshot, String> {
        let (imports, source_fingerprint) = {
            let manifest = self
                .library_manifest
                .lock()
                .map_err(|_| "native library manifest lock poisoned".to_string())?;
            let imports = manifest.snapshot().imports.clone();
            let source_fingerprint = compute_catalog_source_fingerprint(
                &self.catalog_source_static_fingerprint,
                &imports,
            );
            (imports, source_fingerprint)
        };
        #[cfg(test)]
        let compose_barrier = self
            .catalog_compose_barrier
            .lock()
            .expect("catalog compose barrier")
            .take();
        #[cfg(test)]
        if let Some((entered, release)) = compose_barrier {
            entered.wait();
            release.wait();
        }
        let mut composition = if crate::startup_telemetry::enabled() {
            self.catalog_composer
                .compose_with_observer(&imports, |event| match event {
                    sky_native_adapters::CatalogCompositionEvent::SourceStarted(source) => {
                        crate::startup_telemetry::record(&catalog_source_marker(source, "start"));
                    }
                    sky_native_adapters::CatalogCompositionEvent::SourceFinished {
                        source,
                        metrics,
                        duration_ms,
                    } => {
                        let marker = catalog_source_marker(source, "end");
                        crate::startup_telemetry::record_counters(
                            &marker,
                            metrics.directories_visited,
                            metrics.files_visited,
                            metrics.supported_files,
                            Some(duration_ms),
                        );
                    }
                })
                .map_err(catalog_error)?
        } else {
            self.catalog_composer
                .compose(&imports)
                .map_err(catalog_error)?
        };
        self.preserve_unavailable_cached_imports(&mut composition);
        Ok(CatalogCompositionSnapshot {
            composition,
            source_fingerprint,
        })
    }

    fn preserve_unavailable_cached_imports(&self, composition: &mut CatalogComposition) {
        let Ok(Some(cache)) = self.cached_catalog_snapshot() else {
            return;
        };
        for status in composition
            .imported_status
            .iter_mut()
            .filter(|status| !status.available)
        {
            let Some(source) = cache.sources.iter().find(|source| {
                source.kind == CatalogCacheSourceKind::Imported
                    && source.source_id == status.source_id
            }) else {
                continue;
            };
            let cached_ids = source.song_ids.iter().collect::<BTreeSet<_>>();
            status.song_count = cached_ids.len();
            for entry in &cache.entries {
                if entry.kind == CatalogCacheSourceKind::Imported
                    && cached_ids.contains(&entry.song_id)
                    && !composition
                        .entries
                        .iter()
                        .any(|current| current.canonical_path == entry.canonical_path)
                {
                    composition.entries.push(cached_entry_to_source(entry));
                }
            }
            composition.imported_membership.insert(
                status.source_id.clone(),
                source.song_ids.iter().cloned().collect(),
            );
            composition
                .library_membership
                .extend(source.song_ids.iter().cloned());
        }
        composition
            .entries
            .sort_by(|left, right| left.canonical_path.cmp(&right.canonical_path));
    }

    pub(super) fn ensure_catalog_loaded(&self) -> Result<u64, String> {
        let claim = {
            let mut state = self
                .catalog_load
                .lock()
                .map_err(|_| "native catalog load state lock poisoned".to_string())?;
            match &*state {
                CatalogLoadState::Cached { generation }
                | CatalogLoadState::Reconciling { generation }
                | CatalogLoadState::Ready { generation } => {
                    return Ok(*generation);
                }
                CatalogLoadState::Loading => {
                    return Err("catalog_loading: catalog is still loading".into());
                }
                CatalogLoadState::Failed { message } => {
                    return Err(format!("catalog_failed: {message}"));
                }
                CatalogLoadState::Closing => {
                    return Err("closing: desktop application is closing".into());
                }
                CatalogLoadState::Uninitialized => {
                    let epoch = self.catalog_load_epoch.fetch_add(1, Ordering::AcqRel) + 1;
                    *state = CatalogLoadState::Loading;
                    CatalogLoadClaim {
                        work: CatalogLoadWork::Rebuild,
                        epoch,
                    }
                }
            }
        };

        crate::startup_telemetry::record("catalog.compose.start");
        let composition = self.catalog_composition_snapshot();
        crate::startup_telemetry::record("catalog.compose.end");
        let mut state = self
            .catalog_load
            .lock()
            .map_err(|_| "native catalog load state lock poisoned".to_string())?;
        if self.closed.load(Ordering::Acquire) || matches!(*state, CatalogLoadState::Closing) {
            *state = CatalogLoadState::Closing;
            return Err("closing: desktop application is closing".into());
        }
        if !self.catalog_load_claim_is_current(&state, claim) {
            return Err("catalog_loading: catalog load was superseded".into());
        }
        let result = match composition {
            Ok(snapshot) => self.apply_catalog_snapshot_if_current(snapshot, false, true),
            Err(error) => Err(error),
        };
        match result {
            Ok(Some((snapshot, _))) => {
                let generation = snapshot.generation;
                *state = CatalogLoadState::Ready { generation };
                Ok(generation)
            }
            Ok(None) => {
                *state = CatalogLoadState::Uninitialized;
                Err("catalog source changed during load".into())
            }
            Err(error) => {
                *state = CatalogLoadState::Failed {
                    message: error.clone(),
                };
                Err(error)
            }
        }
    }

    pub(super) fn reload(&self) -> Result<CatalogReloadDto, String> {
        let claim = {
            let mut state = self
                .catalog_load
                .lock()
                .map_err(|_| "native catalog load state lock poisoned".to_string())?;
            if self.closed.load(Ordering::Acquire) || matches!(*state, CatalogLoadState::Closing) {
                return Err("closing: desktop application is closing".into());
            }
            if matches!(*state, CatalogLoadState::Loading) {
                return Err("catalog_loading: catalog is still loading".into());
            }
            let epoch = self.catalog_load_epoch.fetch_add(1, Ordering::AcqRel) + 1;
            *state = CatalogLoadState::Loading;
            CatalogLoadClaim {
                work: CatalogLoadWork::Rebuild,
                epoch,
            }
        };

        crate::startup_telemetry::record("catalog.compose.start");
        let composition = self.catalog_composition_snapshot();
        crate::startup_telemetry::record("catalog.compose.end");
        let mut state = self
            .catalog_load
            .lock()
            .map_err(|_| "native catalog load state lock poisoned".to_string())?;
        if self.closed.load(Ordering::Acquire) || matches!(*state, CatalogLoadState::Closing) {
            *state = CatalogLoadState::Closing;
            return Err("closing: desktop application is closing".into());
        }
        if !self.catalog_load_claim_is_current(&state, claim) {
            return Err("catalog_loading: catalog reload was superseded".into());
        }
        let result = match composition {
            Ok(snapshot) => self.apply_catalog_snapshot_if_current(snapshot, true, true),
            Err(error) => Err(error),
        };
        match result {
            Ok(Some((snapshot, library_total))) => {
                *state = CatalogLoadState::Ready {
                    generation: snapshot.generation,
                };
                self.publish(UiEvent::CatalogChanged {
                    v: crate::DESKTOP_PROTOCOL_VERSION,
                    payload: CatalogChangedPayload {
                        generation: snapshot.generation,
                        total: library_total as u64,
                    },
                })?;
                Ok(CatalogReloadDto {
                    generation: snapshot.generation,
                    total: library_total as u64,
                })
            }
            Ok(None) => {
                *state = CatalogLoadState::Uninitialized;
                Err("catalog source changed during reload".into())
            }
            Err(error) => {
                *state = CatalogLoadState::Failed {
                    message: error.clone(),
                };
                let event_message = bounded_text(&error);
                let _ = self.publish(UiEvent::CatalogLoadFailed {
                    v: crate::DESKTOP_PROTOCOL_VERSION,
                    payload: CatalogLoadFailedPayload {
                        message: event_message,
                    },
                });
                Err(error)
            }
        }
    }

    fn apply_catalog_composition(
        &self,
        composition: CatalogComposition,
        invalidate: bool,
        record_ready: bool,
    ) -> Result<(sky_app_core::catalog::CatalogSnapshot, usize), String> {
        // Lock order: Prepare publication gate, then catalog state, then the
        // bounded prepared-plan cache. Cache insertion uses publication gate
        // then prepared-plan cache and never acquires catalog state.
        let mut prepare_publication = self
            .playback
            .prepare_publication
            .lock()
            .map_err(|_| "native Prepare publication lock poisoned".to_string())?;
        let mut catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        crate::startup_telemetry::record("catalog.index.start");
        let snapshot = catalog
            .index
            .replace_entries(composition.entries)
            .map_err(catalog_error)?;
        crate::startup_telemetry::record("catalog.index.end");
        catalog.library_membership = composition.library_membership;
        catalog.builtin_membership = composition.builtin_membership;
        catalog.user_membership = composition.user_membership;
        catalog.imported_membership = composition.imported_membership;
        catalog.imported_status = composition.imported_status;
        catalog.builtin_status = composition.builtin_status;
        let library_total = catalog
            .index
            .count_allowed_ids(&catalog.library_membership, Some(snapshot.generation))
            .map_err(catalog_error);
        // Publish generation validity while the same gate still excludes a
        // Prepare cache commit. This also cancels any analysis based on the
        // previous generation.
        self.playback.invalidate_catalog_locked(
            &mut prepare_publication,
            snapshot.generation,
            invalidate,
        );
        drop(catalog);
        drop(prepare_publication);
        if invalidate {
            self.catalog_search_cache
                .lock()
                .map_err(|_| "native catalog search cache lock poisoned".to_string())?
                .invalidate_all();
            self.invalidate_analysis_cache();
        }
        if record_ready {
            crate::startup_telemetry::record("catalog.ready");
        }
        Ok((snapshot, library_total?))
    }

    fn invalidate_playlist_search_orders(&self) -> Result<(), String> {
        self.catalog_search_cache
            .lock()
            .map_err(|_| "native catalog search cache lock poisoned".to_string())?
            .invalidate_playlists();
        self.invalidate_viewport_projection();
        Ok(())
    }

    pub(crate) fn builtin_catalog_status(&self) -> Result<BuiltinCatalogStatus, String> {
        self.ensure_catalog_loaded()?;
        self.catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())
            .map(|catalog| catalog.builtin_status.clone())
    }

    pub(super) fn list_playlists(&self) -> Result<LibraryNavigationDto, String> {
        self.ensure_catalog_loaded()?;
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let manifest = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?;
        let generation = catalog.index.generation();
        let builtin_available = matches!(
            &catalog.builtin_status,
            BuiltinCatalogStatus::Available { .. }
        );
        let builtin_song_count = if builtin_available {
            catalog
                .index
                .count_allowed_ids(&catalog.builtin_membership, Some(generation))
                .map_err(catalog_error)? as u64
        } else {
            0
        };
        let playlists = manifest
            .snapshot()
            .collections
            .iter()
            .map(|collection| {
                let ids = collection.song_ids.iter().cloned().collect::<BTreeSet<_>>();
                LibraryPlaylistSummaryDto {
                    id: collection.id.clone(),
                    name: collection.name.clone(),
                    song_count: catalog
                        .index
                        .count_allowed_ids(&ids, Some(generation))
                        .unwrap_or_default() as u64,
                }
            })
            .collect();
        Ok(LibraryNavigationDto {
            builtin_available,
            builtin_song_count,
            playlists,
        })
    }

    fn playlist_summary(
        &self,
        collection: sky_app_core::library::Collection,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        self.ensure_catalog_loaded()?;
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let ids = collection.song_ids.iter().cloned().collect::<BTreeSet<_>>();
        Ok(LibraryPlaylistSummaryDto {
            id: collection.id,
            name: collection.name,
            song_count: catalog
                .index
                .count_allowed_ids(&ids, Some(catalog.index.generation()))
                .map_err(catalog_error)? as u64,
        })
    }

    pub(super) fn create_playlist(
        &self,
        request: LibraryCreatePlaylistRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let id = opaque_native_id()?;
        let mut manifest = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?;
        let collection = manifest
            .create_collection(id, request.name)
            .map_err(library_error)?;
        drop(manifest);
        self.playlist_summary(collection)
    }

    pub(super) fn rename_playlist(
        &self,
        request: LibraryRenamePlaylistRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let collection = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?
            .rename_collection(&request.playlist_id, request.name)
            .map_err(library_error)?;
        self.playlist_summary(collection)
    }

    pub(super) fn delete_playlist(
        &self,
        request: LibraryPlaylistIdRequest,
    ) -> Result<bool, String> {
        let mut manifest = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?;
        let deleted = manifest
            .delete_collection(&request.playlist_id)
            .map_err(library_error)?;
        if deleted {
            self.invalidate_playlist_search_orders()?;
        }
        Ok(deleted)
    }

    pub(super) fn add_playlist_songs(
        &self,
        request: LibraryPlaylistSongsRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let mut manifest = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?;
        let collection = manifest
            .add_songs(&request.playlist_id, &request.song_ids)
            .map_err(library_error)?;
        self.invalidate_playlist_search_orders()?;
        drop(manifest);
        self.playlist_summary(collection)
    }

    pub(super) fn remove_playlist_songs(
        &self,
        request: LibraryPlaylistSongsRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let mut manifest = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?;
        let collection = manifest
            .remove_songs(&request.playlist_id, &request.song_ids)
            .map_err(library_error)?;
        self.invalidate_playlist_search_orders()?;
        drop(manifest);
        self.playlist_summary(collection)
    }

    pub(super) fn import_paths(
        &self,
        request: NativeLibraryImportRequest,
        kind: ImportedSourceKind,
    ) -> Result<LibraryPlaylistImportResultDto, String> {
        const MAX_IMPORT_SELECTIONS: usize = 1_024;
        if request.paths.len() > MAX_IMPORT_SELECTIONS {
            return Err("invalid_params: import selection is too large".into());
        }

        let mut imports = Vec::with_capacity(request.paths.len());
        for raw_path in request.paths {
            let path = PathBuf::from(raw_path);
            let canonical = fs::canonicalize(&path)
                .map_err(|_| "import source cannot be resolved".to_owned())?;
            let valid = match kind {
                ImportedSourceKind::File => {
                    canonical.is_file() && is_supported_catalog_path(&canonical)
                }
                ImportedSourceKind::Folder => canonical.is_dir(),
            };
            if !valid {
                return Err("invalid_params: import source has an unsupported type".into());
            }
            let canonical_path = canonical.to_string_lossy().into_owned();
            imports.push(ImportedSourceRef {
                source_id: imported_source_id(&canonical_path, kind),
                canonical_path,
                kind,
            });
        }

        // Resolve the selected native paths before touching the manifest. The
        // resulting IDs are opaque and path-free at the application boundary.
        let composition = self
            .catalog_composer
            .compose(&imports)
            .map_err(catalog_error)?;
        let song_ids = imports
            .iter()
            .filter_map(|import| composition.imported_membership.get(&import.source_id))
            .flat_map(|ids| ids.iter().cloned())
            .collect::<BTreeSet<_>>();
        let song_ids = song_ids.into_iter().collect::<Vec<_>>();

        let (playlist, has_new_import) = {
            let mut manifest = self
                .library_manifest
                .lock()
                .map_err(|_| "native library manifest lock poisoned".to_string())?;
            let has_new_import = imports.iter().any(|import| {
                !manifest.snapshot().imports.iter().any(|existing| {
                    existing
                        .canonical_path
                        .eq_ignore_ascii_case(&import.canonical_path)
                })
            });
            let playlist = manifest
                .import_and_add_songs(&request.playlist_id, imports, &song_ids)
                .map_err(library_error)?;
            self.invalidate_playlist_search_orders()?;
            (playlist, has_new_import)
        };

        let generation = if has_new_import {
            self.reload()?.generation
        } else {
            self.ensure_catalog_loaded()?
        };
        Ok(LibraryPlaylistImportResultDto {
            playlist: self.playlist_summary(playlist)?,
            imported_song_count: song_ids.len() as u64,
            catalog_generation: generation,
        })
    }

    pub(super) fn detail(&self, request: CatalogDetailRequest) -> Result<SongDetailDto, String> {
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
        let key = super::AnalysisKey {
            song_id: if request.conversion_mode.is_none() && request.target_pitch.is_none() {
                song_id.clone()
            } else {
                format!("{song_id}\0sheet:{selection}")
            },
            generation,
            settings_revision,
        };
        let cached = self.analysis_for_request(super::AnalysisRequest {
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
            priority: super::AnalysisPriority::Selected,
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

    pub(super) fn set_viewport(
        &self,
        request: CatalogViewportRequest,
    ) -> Result<CatalogViewportDto, String> {
        self.ensure_catalog_loaded()?;
        if request.song_ids.len() > 2_000 {
            return Err("catalog viewport contains too many metadata hydration IDs".into());
        }
        let viewport_revision = self.begin_viewport_request()?;
        let (entries, selected_entry, ordered_song_ids) = {
            let catalog = self
                .catalog
                .lock()
                .map_err(|_| "native catalog lock poisoned".to_string())?;
            let generation = catalog.index.generation();
            let total = catalog.index.len();
            if generation != request.generation {
                return Err("catalog generation is stale".into());
            }
            let song_ids = if request.song_ids.is_empty() {
                if total == 0 {
                    if request.first_index != 0
                        || request.last_index != -1
                        || request.selected_song_id.is_some()
                    {
                        return Err(
                            "empty catalog viewport must be 0..-1 with no selected song".into()
                        );
                    }
                    Vec::new()
                } else {
                    if request.last_index < request.first_index as i64
                        || request.last_index as u64 >= total as u64
                        || request
                            .last_index
                            .saturating_sub(request.first_index as i64)
                            .saturating_add(1)
                            > 2_000
                    {
                        return Err("catalog viewport is outside bounded index range".into());
                    }
                    catalog
                        .index
                        .song_ids_in_range(
                            request.first_index as usize,
                            request.last_index as usize,
                            Some(request.generation),
                        )
                        .map_err(catalog_error)?
                }
            } else {
                request.song_ids.clone()
            };
            let selected_entry = if let Some(song_id) = &request.selected_song_id {
                Some(
                    catalog
                        .index
                        .entry_for_song_id(song_id, Some(request.generation))
                        .map(|entry| {
                            (
                                entry.row.song_id.clone(),
                                entry.row.title.clone(),
                                entry.canonical_path.clone(),
                            )
                        })
                        .map_err(catalog_error)?,
                )
            } else {
                None
            };
            let mut seen = BTreeSet::new();
            let entries = song_ids
                .into_iter()
                .filter(|song_id| seen.insert(song_id.clone()))
                .map(|song_id| {
                    catalog
                        .index
                        .entry_for_song_id(&song_id, Some(request.generation))
                        .map(|entry| (entry.row.song_id, entry.row.title, entry.canonical_path))
                        .map_err(catalog_error)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let ordered_song_ids = entries
                .iter()
                .map(|entry| entry.0.clone())
                .collect::<Vec<_>>();
            (entries, selected_entry, ordered_song_ids)
        };
        let (settings, like_revision, playlist_revision, settings_revision) =
            self.catalog_settings_snapshot()?;
        let projection_key = super::ViewportProjectionKey {
            generation: request.generation,
            settings_revision,
            liked_membership_revision: like_revision,
            playlist_membership_revision: playlist_revision,
            first_index: request.first_index,
            last_index: request.last_index,
            selected_song_id: request.selected_song_id.clone(),
            request_song_ids: request.song_ids.clone(),
            ordered_song_ids,
        };
        if let Some(response) =
            self.cached_viewport_projection(&projection_key, viewport_revision)?
        {
            return Ok(response);
        }
        if self
            .viewport_revision
            .load(std::sync::atomic::Ordering::Acquire)
            != viewport_revision
        {
            return Ok(CatalogViewportDto {
                accepted: false,
                generation: request.generation,
                first_index: request.first_index,
                last_index: request.last_index,
                selected_song_id: request.selected_song_id,
                items: Vec::new(),
            });
        }
        let shifted_rows = self.shifted_viewport_rows(&projection_key, viewport_revision)?;
        let policy = self.timing_policy(
            settings.playback_defaults.fps,
            settings.playback_defaults.hold_frames,
            settings.playback_defaults.timing_margin_us,
        )?;
        let mut analysis_entries = if let Some(shift) = &shifted_rows {
            entries
                .iter()
                .filter(|entry| entry.0 == shift.entering_song_id)
                .cloned()
                .collect()
        } else {
            entries.clone()
        };
        if let Some(selected) = selected_entry {
            if let Some(index) = analysis_entries
                .iter()
                .position(|entry| entry.0 == selected.0)
            {
                analysis_entries.remove(index);
            }
            analysis_entries.insert(0, selected);
        }
        let selected_id = request.selected_song_id.as_deref();
        let selected_analysis_key = selected_id.map(|song_id| super::AnalysisKey {
            song_id: song_id.to_owned(),
            generation: request.generation,
            settings_revision,
        });
        let mut analyses = std::collections::HashMap::with_capacity(analysis_entries.len());
        let mut analyses_since_selected_refresh = 0_usize;
        let mut incomplete = false;
        for (song_id, title, path) in &analysis_entries {
            let key = super::AnalysisKey {
                song_id: song_id.clone(),
                generation: request.generation,
                settings_revision,
            };
            match self.analysis_for_request(super::AnalysisRequest {
                key,
                analysis_song_id: song_id,
                path: Path::new(path),
                source_bytes: None,
                title,
                failure_format_label: &format_label_for_path(Path::new(path)),
                settings: &settings,
                policy: &policy,
                conversion_mode: None,
                target_pitch: None,
                viewport_revision: Some(viewport_revision),
                priority: if selected_id == Some(song_id.as_str()) {
                    super::AnalysisPriority::Selected
                } else {
                    super::AnalysisPriority::Visible
                },
                cache_failures: true,
            }) {
                Ok(analysis) => {
                    analyses.insert(song_id.clone(), analysis);
                    if let Some(selected_key) = &selected_analysis_key {
                        analyses_since_selected_refresh += 1;
                        if selected_id == Some(song_id.as_str())
                            || analyses_since_selected_refresh
                                >= super::MAX_CACHED_SONG_ANALYSES / 2
                        {
                            self.touch_analysis_cache(selected_key)?;
                            analyses_since_selected_refresh = 0;
                        }
                    }
                }
                Err(error)
                    if error == "catalog analysis queue is full"
                        || error == "catalog analysis request was superseded" =>
                {
                    incomplete = true;
                    break;
                }
                Err(_) => {
                    incomplete = true;
                    break;
                }
            }
        }
        let (
            latest_settings,
            latest_like_revision,
            latest_playlist_revision,
            latest_analysis_revision,
        ) = self.catalog_settings_snapshot()?;
        let generation_is_current = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?
            .index
            .generation()
            == request.generation;
        let accepted = !incomplete
            && self
                .viewport_revision
                .load(std::sync::atomic::Ordering::Acquire)
                == viewport_revision
            && latest_like_revision == like_revision
            && latest_playlist_revision == playlist_revision
            && latest_analysis_revision == settings_revision
            && generation_is_current;
        let row_dto =
            |song_id: String, title: String, analysis: &CachedSongAnalysis| CatalogRowDto {
                liked: latest_settings.liked_songs.contains(&song_id),
                song_id,
                title,
                format_label: analysis.metadata.format_label.clone(),
                duration_us: analysis.metadata.duration_us,
                note_count: analysis.metadata.note_count,
                risk_level: analysis.metadata.risk_level.clone(),
                metadata_state: analysis.metadata.state.into(),
            };
        let items = if !accepted {
            Vec::new()
        } else if let Some(mut shifted_rows) = shifted_rows {
            let (song_id, title, _) = entries
                .iter()
                .find(|entry| entry.0 == shifted_rows.entering_song_id)
                .expect("shifted viewport has one entering catalog row");
            let analysis = analyses
                .get(song_id)
                .expect("shifted viewport analyzed its entering row");
            shifted_rows.reusable_rows.insert(
                shifted_rows.insertion_index,
                row_dto(song_id.clone(), title.clone(), analysis),
            );
            shifted_rows.reusable_rows
        } else {
            entries
                .into_iter()
                .filter_map(|(song_id, title, _)| {
                    analyses
                        .get(&song_id)
                        .map(|analysis| row_dto(song_id, title, analysis))
                })
                .collect()
        };
        let response = CatalogViewportDto {
            accepted,
            generation: request.generation,
            first_index: request.first_index,
            last_index: request.last_index,
            selected_song_id: request.selected_song_id,
            items,
        };
        if accepted {
            self.publish_viewport_projection(projection_key, response.clone(), viewport_revision)?;
        }
        Ok(response)
    }

    pub(super) fn set_song_liked(
        &self,
        request: CatalogSetLikedRequest,
    ) -> Result<CatalogSetLikedDto, String> {
        self.ensure_catalog_loaded()?;
        {
            let catalog = self
                .catalog
                .lock()
                .map_err(|_| "native catalog lock poisoned".to_string())?;
            catalog
                .index
                .canonical_path_for_song_id(&request.song_id, request.generation)
                .map_err(catalog_error)?;
        }
        let mut settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        let previous_likes = settings.snapshot().liked_songs.clone();
        settings
            .reload()
            .map_err(|error| format!("native settings reload failed: {error}"))?;
        settings
            .set_song_liked(&request.song_id, request.liked)
            .map_err(settings_error)?;
        if previous_likes != settings.snapshot().liked_songs {
            self.catalog_search_cache
                .lock()
                .map_err(|_| "native catalog search cache lock poisoned".to_string())?
                .invalidate_liked();
            self.invalidate_viewport_projection();
        }
        let liked_total = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?
            .index
            .count_allowed_ids(settings.snapshot().liked_songs.ids(), request.generation)
            .map_err(catalog_error)?;
        Ok(CatalogSetLikedDto {
            song_id: request.song_id,
            liked: request.liked,
            total: liked_total as u64,
        })
    }

    pub(super) fn analyze_song(
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
        let source_hash = super::sheet::sha256_hex(bytes);
        let options = super::sheet::planner_options(
            conversion_mode,
            target_pitch,
            settings.playback_defaults.tempo_scale,
        );
        let plan = match build_song_plan(bytes, fallback, &options, policy) {
            Ok(plan) => plan,
            Err(error) => {
                let sheet_analysis =
                    super::sheet::failed_sheet_analysis(&format_label_for_path(path), &error);
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
                let sheet = super::sheet::legacy_sheet_analysis(&plan.song, &plan.schedule);
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
                let fingerprint = super::sheet::sheet_confirmation_fingerprint(
                    super::sheet::SheetConfirmationIdentity {
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
                let contextual_risk = super::sheet::piano_plan_risk(
                    &plan,
                    settings.playback_defaults.hold_frames,
                    settings.playback_defaults.tempo_scale,
                );
                let sheet = super::sheet::piano_plan_sheet_analysis(
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
            && super::sheet::timing_reconsent_required(
                &risk,
                settings.playback_defaults.hold_frames,
                settings.playback_defaults.tempo_scale,
            );
        let mut recommendations = risk.recommendations.clone();
        if timing_reconsent_required {
            recommendations.push(super::sheet::TIMING_RECONSENT_NOTE.to_owned());
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
                format!("{suggestion} {}", super::sheet::TIMING_RECONSENT_NOTE)
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
        min_any_note_gap_us: None,
        min_same_key_gap_us: None,
        suggested_hold_frames: None,
        suggested_tempo_scale: None,
    }
}

fn cached_composition(cache: &CatalogCache) -> Result<CatalogComposition, String> {
    cache.validate()?;
    let mut composition = CatalogComposition::default();
    for source in &cache.sources {
        match source.kind {
            CatalogCacheSourceKind::Builtin => {
                composition
                    .builtin_membership
                    .extend(source.song_ids.iter().cloned());
                composition
                    .library_membership
                    .extend(source.song_ids.iter().cloned());
                composition.builtin_status = if source.available {
                    BuiltinCatalogStatus::Available {
                        song_count: source.song_ids.len(),
                    }
                } else {
                    BuiltinCatalogStatus::Unavailable {
                        code: BuiltinCatalogFailureCode::ResourceUnavailable,
                    }
                };
            }
            CatalogCacheSourceKind::User => {
                composition
                    .user_membership
                    .extend(source.song_ids.iter().cloned());
                composition
                    .library_membership
                    .extend(source.song_ids.iter().cloned());
            }
            CatalogCacheSourceKind::Imported => {
                composition.imported_membership.insert(
                    source.source_id.clone(),
                    source.song_ids.iter().cloned().collect(),
                );
                composition
                    .library_membership
                    .extend(source.song_ids.iter().cloned());
                composition
                    .imported_status
                    .push(ImportedSourceCatalogStatus {
                        source_id: source.source_id.clone(),
                        kind: source.import_kind.unwrap_or(ImportedSourceKind::Folder),
                        display_name: source.display_name.clone(),
                        song_count: source.song_ids.len(),
                        available: source.available,
                    });
            }
        }
    }
    composition.entries = cache.entries.iter().map(cached_entry_to_source).collect();
    composition
        .entries
        .sort_by(|left, right| left.canonical_path.cmp(&right.canonical_path));
    Ok(composition)
}

fn cached_entry_to_source(entry: &CatalogCacheEntry) -> sky_app_core::catalog::CatalogSourceEntry {
    let identity = match entry.kind {
        CatalogCacheSourceKind::Builtin => CatalogSourceIdentity::Stable(
            StableSongId::new(entry.song_id.clone()).expect("validated cached stable song ID"),
        ),
        CatalogCacheSourceKind::User | CatalogCacheSourceKind::Imported => {
            CatalogSourceIdentity::PathDerived
        }
    };
    sky_app_core::catalog::CatalogSourceEntry {
        canonical_path: entry.canonical_path.clone(),
        title: entry.title.clone(),
        identity,
    }
}

fn library_error(error: LibraryError) -> String {
    format!("library operation failed: {error}")
}

fn is_supported_catalog_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            sky_app_core::catalog::SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn imported_source_id(canonical_path: &str, kind: ImportedSourceKind) -> String {
    let mut hasher = Sha256::new();
    hasher.update(match kind {
        ImportedSourceKind::File => b"file\0".as_slice(),
        ImportedSourceKind::Folder => b"folder\0".as_slice(),
    });
    hasher.update(canonical_path.as_bytes());
    hasher
        .finalize()
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn compute_catalog_source_static_fingerprint(
    resources: &AppResources,
    user_root: &Path,
) -> String {
    let builtin_manifest_fingerprint = fs::read(resources.builtin_catalog_manifest_path())
        .map(|bytes| digest_hex(&bytes))
        .unwrap_or_else(|_| "unavailable".into());
    let payload = serde_json::json!({
        "version": 1,
        "builtin_root": source_path_identity(resources.builtin_catalog_root()),
        "builtin_manifest": builtin_manifest_fingerprint,
        "user_root": source_path_identity(user_root),
    });
    let bytes = serde_json::to_vec(&payload).expect("catalog source static fingerprint payload");
    digest_hex(&bytes)
}

pub(super) fn compute_catalog_source_fingerprint(
    static_fingerprint: &str,
    imports: &[ImportedSourceRef],
) -> String {
    let mut normalized_imports = imports.to_vec();
    for import in &mut normalized_imports {
        import.canonical_path = source_path_identity(Path::new(&import.canonical_path));
    }
    normalized_imports.sort_by(|left, right| {
        (&left.source_id, &left.canonical_path, left.kind as u8).cmp(&(
            &right.source_id,
            &right.canonical_path,
            right.kind as u8,
        ))
    });
    let payload = serde_json::json!({
        "version": 2,
        "static": static_fingerprint,
        "imports": normalized_imports,
    });
    let bytes = serde_json::to_vec(&payload).expect("catalog source fingerprint payload");
    digest_hex(&bytes)
}

fn source_path_identity(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn digest_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn format_label_for_path(path: &Path) -> String {
    path.extension()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown")
        .to_ascii_uppercase()
}

#[allow(dead_code)]
fn catalog_source_marker(
    source: sky_native_adapters::CatalogCompositionSource,
    suffix: &str,
) -> String {
    match source {
        sky_native_adapters::CatalogCompositionSource::Builtin => {
            format!("catalog.builtin.{suffix}")
        }
        sky_native_adapters::CatalogCompositionSource::User => {
            format!("catalog.user.{suffix}")
        }
        sky_native_adapters::CatalogCompositionSource::Imported { index } => {
            format!("catalog.import.source_{index}.{suffix}")
        }
    }
}

pub(super) fn catalog_error(error: CatalogError) -> String {
    match error {
        CatalogError::SourceUnavailable(_) => "catalog source is unavailable".into(),
        other => other.to_string(),
    }
}
