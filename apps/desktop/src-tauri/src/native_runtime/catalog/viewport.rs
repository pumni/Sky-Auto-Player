use crate::commands::{
    CatalogRowDto, CatalogSetLikedDto, CatalogSetLikedRequest, CatalogViewportDto,
    CatalogViewportRequest,
};
use crate::native_runtime::catalog::{catalog_error, format_label_for_path};
use crate::native_runtime::{
    AnalysisKey, AnalysisPriority, AnalysisRequest, CachedSongAnalysis, MAX_CACHED_SONG_ANALYSES,
    NativeDesktopRuntime, ViewportProjectionKey, settings_error,
};
use std::collections::BTreeSet;
use std::path::Path;

impl NativeDesktopRuntime {
    pub(in crate::native_runtime) fn set_viewport(
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
        let projection_key = ViewportProjectionKey {
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
        let selected_analysis_key = selected_id.map(|song_id| AnalysisKey {
            song_id: song_id.to_owned(),
            generation: request.generation,
            settings_revision,
        });
        let mut analyses = std::collections::HashMap::with_capacity(analysis_entries.len());
        let mut analyses_since_selected_refresh = 0_usize;
        let mut incomplete = false;
        for (song_id, title, path) in &analysis_entries {
            let key = AnalysisKey {
                song_id: song_id.clone(),
                generation: request.generation,
                settings_revision,
            };
            match self.analysis_for_request(AnalysisRequest {
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
                    AnalysisPriority::Selected
                } else {
                    AnalysisPriority::Visible
                },
                cache_failures: true,
            }) {
                Ok(analysis) => {
                    analyses.insert(song_id.clone(), analysis);
                    if let Some(selected_key) = &selected_analysis_key {
                        analyses_since_selected_refresh += 1;
                        if selected_id == Some(song_id.as_str())
                            || analyses_since_selected_refresh >= MAX_CACHED_SONG_ANALYSES / 2
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

    pub(in crate::native_runtime) fn set_song_liked(
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
}
