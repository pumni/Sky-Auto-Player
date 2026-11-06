use super::{AnalysisKey, CatalogSearchKey, CatalogSearchScope, NativeDesktopRuntime};
use crate::commands::{
    CatalogRowDto, CatalogSearchDto, CatalogSearchRequest, CatalogSourceId, LibrarySource,
};
use crate::native_runtime::catalog::{catalog_error, format_label_for_path};
use sky_app_core::catalog::{CatalogIndex, WRatioRanker};
use sky_native_adapters::BuiltinCatalogStatus;
use std::collections::BTreeSet;
use std::path::Path;

impl NativeDesktopRuntime {
    pub(super) fn search(&self, request: CatalogSearchRequest) -> Result<CatalogSearchDto, String> {
        self.ensure_catalog_loaded()?;
        let (settings, liked_revision, _, analysis_revision) = self.catalog_settings_snapshot()?;
        let (playlist_ids, playlist_revision) = match &request.source {
            LibrarySource::Smart {
                id: CatalogSourceId::Liked,
            } => (None, None),
            LibrarySource::Smart {
                id: CatalogSourceId::All,
            } => (None, None),
            LibrarySource::Smart {
                id: CatalogSourceId::Builtin,
            } => (None, None),
            LibrarySource::Playlist { id } => {
                if !sky_app_core::library::is_valid_collection_id(id) {
                    return Err("invalid_params: playlist source ID is invalid".into());
                }
                let manifest = self
                    .library_manifest
                    .lock()
                    .map_err(|_| "native library manifest lock poisoned".to_string())?;
                let ids = manifest
                    .snapshot()
                    .collections
                    .iter()
                    .find(|collection| collection.id == *id)
                    .ok_or_else(|| "library playlist was not found".to_string())?
                    .song_ids
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>();
                let playlist_revision = self
                    .catalog_search_cache
                    .lock()
                    .map_err(|_| "native catalog search cache lock poisoned".to_string())?
                    .playlist_membership_revision;
                (Some(ids), Some(playlist_revision))
            }
        };
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let allowed_ids = match &request.source {
            LibrarySource::Smart {
                id: CatalogSourceId::Liked,
            } => Some(settings.liked_songs.ids()),
            LibrarySource::Smart {
                id: CatalogSourceId::All,
            } => {
                // Library membership is a subset of this generation's index. Equal
                // cardinality means there are no imported-only rows to filter out.
                (catalog.library_membership.len() != catalog.index.len())
                    .then_some(&catalog.library_membership)
            }
            LibrarySource::Smart {
                id: CatalogSourceId::Builtin,
            } => {
                if !matches!(
                    &catalog.builtin_status,
                    BuiltinCatalogStatus::Available { .. }
                ) {
                    return Err("builtin songs are unavailable".into());
                }
                Some(&catalog.builtin_membership)
            }
            LibrarySource::Playlist { .. } => playlist_ids.as_ref(),
        };
        let liked_total = catalog
            .index
            .count_allowed_ids(settings.liked_songs.ids(), request.generation)
            .map_err(catalog_error)?;
        catalog
            .index
            .validate_search_window(
                &request.query,
                request.offset as usize,
                request.limit as usize,
                request.generation,
            )
            .map_err(catalog_error)?;
        let scope = match &request.source {
            LibrarySource::Smart {
                id: CatalogSourceId::Liked,
            } => CatalogSearchScope::Liked,
            LibrarySource::Smart {
                id: CatalogSourceId::All,
            } => CatalogSearchScope::All,
            LibrarySource::Smart {
                id: CatalogSourceId::Builtin,
            } => CatalogSearchScope::Builtin,
            LibrarySource::Playlist { id } => CatalogSearchScope::Playlist(id.clone()),
        };
        let membership_revision = match &scope {
            CatalogSearchScope::Liked => liked_revision,
            CatalogSearchScope::Playlist(_) => playlist_revision.unwrap_or_default(),
            CatalogSearchScope::All | CatalogSearchScope::Builtin => 0,
        };
        let is_fuzzy =
            CatalogIndex::is_fuzzy_search_query(&request.query).map_err(catalog_error)?;
        let search_key = CatalogSearchKey {
            generation: catalog.index.generation(),
            scope: scope.clone(),
            membership_revision,
            query: request.query.clone(),
        };
        let page = if is_fuzzy {
            let cache_hit = {
                let mut cache = self
                    .catalog_search_cache
                    .lock()
                    .map_err(|_| "native catalog search cache lock poisoned".to_string())?;
                cache
                    .get(&search_key)
                    .map(|order| {
                        catalog.index.page_from_search_order(
                            order,
                            &request.query,
                            request.offset as usize,
                            request.limit as usize,
                            request.generation,
                        )
                    })
                    .transpose()
                    .map_err(catalog_error)?
            };
            if let Some(page) = cache_hit {
                page
            } else {
                #[cfg(test)]
                self.catalog_search_order_test_computations
                    .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
                let order = catalog
                    .index
                    .search_order_with_allowed_ids(
                        &WRatioRanker,
                        &request.query,
                        request.generation,
                        allowed_ids,
                    )
                    .map_err(catalog_error)?;
                let page = catalog
                    .index
                    .page_from_search_order(
                        &order,
                        &request.query,
                        request.offset as usize,
                        request.limit as usize,
                        request.generation,
                    )
                    .map_err(catalog_error)?;
                let mut cache = self
                    .catalog_search_cache
                    .lock()
                    .map_err(|_| "native catalog search cache lock poisoned".to_string())?;
                let membership_is_current = match scope {
                    CatalogSearchScope::Liked => {
                        cache.liked_membership_revision == membership_revision
                    }
                    CatalogSearchScope::Playlist(_) => {
                        cache.playlist_membership_revision == membership_revision
                    }
                    CatalogSearchScope::All | CatalogSearchScope::Builtin => true,
                };
                if catalog.index.generation() == search_key.generation && membership_is_current {
                    cache.insert(search_key, order);
                }
                page
            }
        } else {
            catalog
                .index
                .search_with_allowed_ids(
                    &WRatioRanker,
                    &request.query,
                    request.offset as usize,
                    request.limit as usize,
                    request.generation,
                    allowed_ids,
                )
                .map_err(catalog_error)?
        };
        let mut cache = self
            .analysis_cache
            .lock()
            .map_err(|_| "native analysis cache lock poisoned".to_string())?;
        Ok(CatalogSearchDto {
            items: page
                .items
                .into_iter()
                .map(|row| {
                    let analysis_key = AnalysisKey {
                        song_id: row.song_id.clone(),
                        generation: page.generation,
                        settings_revision: analysis_revision,
                    };
                    let metadata = cache.get(&analysis_key).map(|value| value.metadata);
                    let format_label = metadata
                        .as_ref()
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
                        duration_us: metadata.as_ref().and_then(|value| value.duration_us),
                        note_count: metadata.as_ref().and_then(|value| value.note_count),
                        risk_level: metadata
                            .as_ref()
                            .map(|value| value.risk_level.clone())
                            .unwrap_or_else(|| "unknown".into()),
                        metadata_state: metadata
                            .as_ref()
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
}
