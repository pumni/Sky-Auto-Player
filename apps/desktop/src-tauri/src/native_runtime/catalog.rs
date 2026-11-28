use super::{
    CatalogCompositionSnapshot, CatalogLoadClaim, CatalogLoadState, CatalogLoadWork,
    NativeDesktopRuntime, bounded_text,
};
use crate::commands::CatalogReloadDto;
use crate::ui_events::{
    CatalogChangedPayload, CatalogLoadFailedPayload, CatalogReadiness, UiEvent,
};
use sha2::{Digest, Sha256};
use sky_app_core::catalog::{CatalogError, CatalogSourceIdentity, StableSongId};
use sky_app_core::library::{ImportedSourceKind, ImportedSourceRef};
use sky_native_adapters::{
    AppResources, BuiltinCatalogFailureCode, BuiltinCatalogStatus, CatalogCache, CatalogCacheEntry,
    CatalogCacheSourceKind, CatalogComposition, ImportedSourceCatalogStatus,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::Ordering;

mod analysis;
mod library;
mod viewport;

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

    pub(crate) fn builtin_catalog_status(&self) -> Result<BuiltinCatalogStatus, String> {
        self.ensure_catalog_loaded()?;
        self.catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())
            .map(|catalog| catalog.builtin_status.clone())
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
