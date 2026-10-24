use super::{
    CachedSongAnalysis, CatalogCompositionSnapshot, CatalogLoadClaim, CatalogLoadState,
    CatalogLoadWork, CatalogMetadata, NativeDesktopRuntime, NativeLibraryImportRequest,
    bounded_text, opaque_native_id, settings_error,
};
use crate::commands::{
    CatalogDetailRequest, CatalogReloadDto, CatalogRowDto, CatalogSearchDto, CatalogSearchRequest,
    CatalogSetLikedDto, CatalogSetLikedRequest, CatalogSourceId, CatalogViewportDto,
    CatalogViewportRequest, LibraryCreatePlaylistRequest, LibraryNavigationDto,
    LibraryPlaylistIdRequest, LibraryPlaylistImportResultDto, LibraryPlaylistSongsRequest,
    LibraryPlaylistSummaryDto, LibraryRenamePlaylistRequest, LibrarySource, RiskSummaryDto,
    SongDetailDto,
};
use crate::ui_events::{
    CatalogChangedPayload, CatalogLoadFailedPayload, CatalogReadiness, UiEvent,
};
use sha2::{Digest, Sha256};
use sky_app_core::catalog::{
    CatalogError, CatalogSourceIdentity, StableSongId, WRatioRanker, song_id_for_canonical_path,
};
use sky_app_core::library::{ImportedSourceKind, ImportedSourceRef, LibraryError};
use sky_app_core::settings::ApplicationSettings;
use sky_app_core::song::{
    analyze_schedule_with_context, build_schedule_with_policy, parse_song_json,
};
use sky_app_core::timing::MaterializedTimingPolicy;
use sky_native_adapters::{
    AppResources, BuiltinCatalogFailureCode, BuiltinCatalogStatus, CATALOG_CACHE_SCHEMA_VERSION,
    CatalogCache, CatalogCacheEntry, CatalogCacheSource, CatalogCacheSourceKind,
    CatalogComposition, ImportedSourceCatalogStatus,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::UNIX_EPOCH;
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

    fn catalog_cache_from_composition(
        &self,
        composition: &CatalogComposition,
        source_fingerprint: &str,
    ) -> Result<CatalogCache, String> {
        // Size and mtime are persisted for cache metadata and diagnostics only.
        // Reconciliation always comes from a complete CatalogComposer pass, so
        // a same-size, same-mtime content mutation cannot be hidden by a
        // metadata-only validity shortcut.
        let mut sources = vec![
            CatalogCacheSource {
                source_id: "builtin".into(),
                kind: CatalogCacheSourceKind::Builtin,
                import_kind: None,
                display_name: "Built-in library".into(),
                available: matches!(
                    composition.builtin_status,
                    BuiltinCatalogStatus::Available { .. }
                ),
                song_ids: composition.builtin_membership.iter().cloned().collect(),
            },
            CatalogCacheSource {
                source_id: "user".into(),
                kind: CatalogCacheSourceKind::User,
                import_kind: None,
                display_name: "User library".into(),
                available: true,
                song_ids: composition.user_membership.iter().cloned().collect(),
            },
        ];
        for status in &composition.imported_status {
            sources.push(CatalogCacheSource {
                source_id: status.source_id.clone(),
                kind: CatalogCacheSourceKind::Imported,
                import_kind: Some(status.kind),
                display_name: status.display_name.clone(),
                available: status.available,
                song_ids: composition
                    .imported_membership
                    .get(&status.source_id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
            });
        }
        let previous_cache = self.cached_catalog_snapshot()?;
        let mut entries = Vec::with_capacity(composition.entries.len());
        for entry in &composition.entries {
            let song_id = match &entry.identity {
                CatalogSourceIdentity::PathDerived => {
                    song_id_for_canonical_path(&entry.canonical_path)
                }
                CatalogSourceIdentity::Stable(song_id) => song_id.as_str().to_owned(),
            };
            let (source_id, kind) = cache_source_for_song(composition, &song_id);
            let previous_entry = previous_cache.as_ref().and_then(|cache| {
                cache.entries.iter().find(|cached| {
                    cached.canonical_path == entry.canonical_path && cached.song_id == song_id
                })
            });
            let metadata = fs::metadata(&entry.canonical_path).ok();
            let (file_size, modified_unix_ms) = if let Some(metadata) = metadata {
                let modified_unix_ms = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
                    .or_else(|| previous_entry.map(|cached| cached.modified_unix_ms))
                    .ok_or_else(|| "catalog cache metadata timestamp unavailable".to_string())?;
                (metadata.len(), modified_unix_ms)
            } else if let Some(previous_entry) = previous_entry {
                (previous_entry.file_size, previous_entry.modified_unix_ms)
            } else {
                return Err("catalog cache metadata unavailable".into());
            };
            entries.push(CatalogCacheEntry {
                source_id,
                kind,
                canonical_path: entry.canonical_path.clone(),
                song_id,
                title: entry.title.clone(),
                file_size,
                modified_unix_ms,
            });
        }
        let cache_generation = self
            .cached_catalog
            .lock()
            .ok()
            .and_then(|cache| {
                cache
                    .as_ref()
                    .map(|cache| cache.cache_generation.saturating_add(1).max(1))
            })
            .unwrap_or(1);
        Ok(CatalogCache {
            schema_version: CATALOG_CACHE_SCHEMA_VERSION,
            cache_generation,
            source_fingerprint: source_fingerprint.to_owned(),
            sources,
            entries,
        })
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

    pub(super) fn ensure_catalog_loaded(
        &self,
    ) -> Result<sky_app_core::catalog::CatalogSnapshot, String> {
        let claim = {
            let mut state = self
                .catalog_load
                .lock()
                .map_err(|_| "native catalog load state lock poisoned".to_string())?;
            match &*state {
                CatalogLoadState::Cached { .. }
                | CatalogLoadState::Reconciling { .. }
                | CatalogLoadState::Ready { .. } => {
                    return self
                        .catalog
                        .lock()
                        .map_err(|_| "native catalog lock poisoned".to_string())
                        .map(|catalog| catalog.index.snapshot());
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
                *state = CatalogLoadState::Ready {
                    generation: snapshot.generation,
                };
                Ok(snapshot)
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
            self.invalidate_analysis_cache();
        }
        if record_ready {
            crate::startup_telemetry::record("catalog.ready");
        }
        Ok((snapshot, library_total?))
    }

    fn cached_catalog_snapshot(&self) -> Result<Option<CatalogCache>, String> {
        self.cached_catalog
            .lock()
            .map_err(|_| "native catalog cache lock poisoned".to_string())
            .map(|cache| cache.clone())
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
        Ok(LibraryNavigationDto { playlists })
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
        let collection = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?
            .create_collection(id, request.name)
            .map_err(library_error)?;
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
        self.library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?
            .delete_collection(&request.playlist_id)
            .map_err(library_error)
    }

    pub(super) fn add_playlist_songs(
        &self,
        request: LibraryPlaylistSongsRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let collection = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?
            .add_songs(&request.playlist_id, &request.song_ids)
            .map_err(library_error)?;
        self.playlist_summary(collection)
    }

    pub(super) fn remove_playlist_songs(
        &self,
        request: LibraryPlaylistSongsRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let collection = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?
            .remove_songs(&request.playlist_id, &request.song_ids)
            .map_err(library_error)?;
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
            (playlist, has_new_import)
        };

        let generation = if has_new_import {
            self.reload()?.generation
        } else {
            self.ensure_catalog_loaded()?.generation
        };
        Ok(LibraryPlaylistImportResultDto {
            playlist: self.playlist_summary(playlist)?,
            imported_song_count: song_ids.len() as u64,
            catalog_generation: generation,
        })
    }

    pub(super) fn search(&self, request: CatalogSearchRequest) -> Result<CatalogSearchDto, String> {
        self.ensure_catalog_loaded()?;
        let settings = self.settings_snapshot()?;
        let allowed_ids = match &request.source {
            LibrarySource::Smart {
                id: CatalogSourceId::Liked,
            } => Some(settings.liked_songs.ids().clone()),
            LibrarySource::Smart {
                id: CatalogSourceId::All,
            } => {
                let catalog = self
                    .catalog
                    .lock()
                    .map_err(|_| "native catalog lock poisoned".to_string())?;
                Some(catalog.library_membership.clone())
            }
            LibrarySource::Playlist { id } => {
                if !sky_app_core::library::is_valid_collection_id(id) {
                    return Err("invalid_params: playlist source ID is invalid".into());
                }
                let manifest = self
                    .library_manifest
                    .lock()
                    .map_err(|_| "native library manifest lock poisoned".to_string())?;
                Some(
                    manifest
                        .snapshot()
                        .collections
                        .iter()
                        .find(|collection| collection.id == *id)
                        .ok_or_else(|| "library playlist was not found".to_string())?
                        .song_ids
                        .iter()
                        .cloned()
                        .collect(),
                )
            }
        };
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let liked_total = catalog
            .index
            .count_allowed_ids(settings.liked_songs.ids(), request.generation)
            .map_err(catalog_error)?;
        let page = catalog
            .index
            .search_with_allowed_ids(
                &WRatioRanker,
                &request.query,
                request.offset as usize,
                request.limit as usize,
                request.generation,
                allowed_ids.as_ref(),
            )
            .map_err(catalog_error)?;
        let cache = self
            .analysis_cache
            .lock()
            .map_err(|_| "native analysis cache lock poisoned".to_string())?;
        Ok(CatalogSearchDto {
            items: page
                .items
                .into_iter()
                .map(|row| {
                    let metadata = cache.get(&row.song_id).map(|value| &value.metadata);
                    let format_label = metadata
                        .map(|value| value.format_label.clone())
                        .or_else(|| {
                            catalog
                                .index
                                .entry_for_song_id(&row.song_id, Some(page.generation))
                                .ok()
                                .map(|entry| {
                                    format_label_for_path(Path::new(&entry.canonical_path))
                                })
                        })
                        .unwrap_or_else(|| "UNKNOWN".into());
                    CatalogRowDto {
                        liked: settings.liked_songs.contains(&row.song_id),
                        song_id: row.song_id.clone(),
                        title: row.title,
                        format_label,
                        duration_us: metadata.and_then(|value| value.duration_us),
                        note_count: metadata.and_then(|value| value.note_count),
                        risk_level: metadata
                            .map(|value| value.risk_level.clone())
                            .unwrap_or_else(|| "unknown".into()),
                        metadata_state: metadata
                            .map(|value| value.state.into())
                            .unwrap_or_else(|| "pending".into()),
                    }
                })
                .collect(),
            offset: request.offset,
            limit: request.limit,
            total: page.total as u64,
            liked_total: liked_total as u64,
            generation: page.generation,
        })
    }

    pub(super) fn detail(&self, request: CatalogDetailRequest) -> Result<SongDetailDto, String> {
        self.ensure_catalog_loaded()?;
        let entry = {
            let catalog = self
                .catalog
                .lock()
                .map_err(|_| "native catalog lock poisoned".to_string())?;
            catalog
                .index
                .entry_for_song_id(&request.song_id, request.generation)
                .map_err(catalog_error)?
        };
        if let Some(detail) = self
            .analysis_cache
            .lock()
            .map_err(|_| "native analysis cache lock poisoned".to_string())?
            .get(&request.song_id)
            .and_then(|cached| cached.detail.clone())
        {
            return Ok(detail);
        }

        let path = PathBuf::from(&entry.canonical_path);
        let fallback = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or(&entry.row.title);
        let settings = self.settings_snapshot()?;
        let policy = self.timing_policy(
            settings.playback_defaults.fps,
            settings.playback_defaults.hold_frames,
            settings.playback_defaults.timing_margin_us,
        )?;
        let cached = self.analyze_song(
            &path,
            &entry.row.song_id,
            &entry.row.title,
            fallback,
            &settings,
            &policy,
        )?;
        let detail = cached
            .detail
            .clone()
            .ok_or_else(|| "song detail analysis was unavailable".to_string())?;
        self.cache_analysis(&request.song_id, cached);
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
        let entries = {
            let catalog = self
                .catalog
                .lock()
                .map_err(|_| "native catalog lock poisoned".to_string())?;
            let snapshot = catalog.index.snapshot();
            if snapshot.generation != request.generation {
                return Err("catalog generation is stale".into());
            }
            let song_ids = if request.song_ids.is_empty() {
                if snapshot.total == 0 {
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
                        || request.last_index as u64 >= snapshot.total as u64
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
            if let Some(song_id) = &request.selected_song_id {
                catalog
                    .index
                    .canonical_path_for_song_id(song_id, Some(request.generation))
                    .map_err(catalog_error)?;
            }
            song_ids
                .into_iter()
                .map(|song_id| {
                    catalog
                        .index
                        .entry_for_song_id(&song_id, Some(request.generation))
                        .map(|entry| (entry.row.song_id, entry.row.title, entry.canonical_path))
                        .map_err(catalog_error)
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let settings = self.settings_snapshot()?;
        let policy = self.timing_policy(
            settings.playback_defaults.fps,
            settings.playback_defaults.hold_frames,
            settings.playback_defaults.timing_margin_us,
        )?;
        for (song_id, title, path) in &entries {
            let already_cached = self
                .analysis_cache
                .lock()
                .map_err(|_| "native analysis cache lock poisoned".to_string())?
                .contains_key(song_id);
            if already_cached {
                continue;
            }
            let format_label = format_label_for_path(Path::new(path));
            let analysis = self
                .analyze_song(Path::new(path), song_id, title, title, &settings, &policy)
                .unwrap_or_else(|_| CachedSongAnalysis {
                    metadata: CatalogMetadata::error(format_label),
                    detail: None,
                });
            self.cache_analysis(song_id, analysis);
        }
        let cache = self
            .analysis_cache
            .lock()
            .map_err(|_| "native analysis cache lock poisoned".to_string())?;
        let items = entries
            .into_iter()
            .filter_map(|(song_id, title, _)| {
                cache.get(&song_id).map(|analysis| CatalogRowDto {
                    liked: settings.liked_songs.contains(&song_id),
                    song_id,
                    title,
                    format_label: analysis.metadata.format_label.clone(),
                    duration_us: analysis.metadata.duration_us,
                    note_count: analysis.metadata.note_count,
                    risk_level: analysis.metadata.risk_level.clone(),
                    metadata_state: analysis.metadata.state.into(),
                })
            })
            .collect();
        Ok(CatalogViewportDto {
            accepted: true,
            generation: request.generation,
            first_index: request.first_index,
            last_index: request.last_index,
            selected_song_id: request.selected_song_id,
            items,
        })
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
        settings
            .reload()
            .map_err(|error| format!("native settings reload failed: {error}"))?;
        settings
            .set_song_liked(&request.song_id, request.liked)
            .map_err(settings_error)?;
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

    pub(super) fn invalidate_analysis_cache(&self) {
        if let Ok(mut cache) = self.analysis_cache.lock() {
            cache.clear();
        }
    }

    fn cache_analysis(&self, song_id: &str, analysis: CachedSongAnalysis) {
        if let Ok(mut cache) = self.analysis_cache.lock() {
            cache.insert(song_id.to_owned(), analysis);
        }
    }

    fn analyze_song(
        &self,
        path: &Path,
        song_id: &str,
        title: &str,
        fallback: &str,
        settings: &ApplicationSettings,
        policy: &MaterializedTimingPolicy,
    ) -> Result<CachedSongAnalysis, String> {
        let bytes = fs::read(path).map_err(|error| format!("song read failed: {error}"))?;
        let song = parse_song_json(&bytes, fallback).map_err(|error| error.to_string())?;
        let schedule =
            build_schedule_with_policy(&song, settings.playback_defaults.tempo_scale, policy)
                .map_err(|error| error.to_string())?;
        let risk = analyze_schedule_with_context(
            &schedule,
            Some(&song.notes),
            settings.playback_defaults.hold_frames,
            settings.playback_defaults.tempo_scale,
        );
        let risk_level = match risk.severity.as_str() {
            "low" | "medium" | "high" => risk.severity.clone(),
            _ => return Err("song risk analysis returned an unknown level".into()),
        };
        let recommendations = risk.recommendations.clone();
        let reasons = if risk_level == "low" {
            Vec::new()
        } else {
            recommendations.clone()
        };
        let recommendation = Some(crate::commands::PlaybackRecommendationDto {
            recommended_hold_frames: risk.suggested_hold_frames,
            recommended_tempo_scale: risk.suggested_tempo_scale,
            summary: recommendations
                .first()
                .cloned()
                .unwrap_or_else(|| "Keep the selected settings.".into()),
        });
        let format_label = format_label_for_path(path);
        let detail = SongDetailDto {
            song_id: song_id.to_owned(),
            title: title.to_owned(),
            duration_us: schedule.source_duration_us,
            note_count: song.notes.len() as u64,
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
        };
        Ok(CachedSongAnalysis {
            metadata: CatalogMetadata {
                format_label,
                duration_us: Some(schedule.source_duration_us),
                note_count: Some(song.notes.len() as u64),
                risk_level,
                state: "ready",
            },
            detail: Some(detail),
        })
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

fn cache_source_for_song(
    composition: &CatalogComposition,
    song_id: &str,
) -> (String, CatalogCacheSourceKind) {
    if composition.builtin_membership.contains(song_id) {
        return ("builtin".into(), CatalogCacheSourceKind::Builtin);
    }
    if composition.user_membership.contains(song_id) {
        return ("user".into(), CatalogCacheSourceKind::User);
    }
    if let Some(source_id) = composition
        .imported_membership
        .iter()
        .filter_map(|(source_id, song_ids)| song_ids.contains(song_id).then_some(source_id))
        .min()
    {
        return (source_id.clone(), CatalogCacheSourceKind::Imported);
    }
    ("user".into(), CatalogCacheSourceKind::User)
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

fn format_label_for_path(path: &Path) -> String {
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
