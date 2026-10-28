use super::{CachedSongAnalysis, MaterializedTimingPolicy, NativeDesktopRuntime};
use crate::commands::{CatalogRowDto, CatalogViewportDto};
use sky_app_core::catalog::CatalogSearchOrder;
use sky_app_core::settings::ApplicationSettings;
use std::collections::{HashMap, VecDeque};
use std::path::Path;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(super) enum CatalogSearchScope {
    All,
    Liked,
    Playlist(String),
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(super) struct CatalogSearchKey {
    pub(super) generation: u64,
    pub(super) scope: CatalogSearchScope,
    pub(super) membership_revision: u64,
    pub(super) query: String,
}

pub(super) struct CatalogSearchOrderEntry {
    pub(super) key: CatalogSearchKey,
    pub(super) order: CatalogSearchOrder,
}

#[derive(Default)]
pub(super) struct CatalogSearchCache {
    pub(super) entries: VecDeque<CatalogSearchOrderEntry>,
    pub(super) cached_indices: usize,
    pub(super) liked_membership_revision: u64,
    pub(super) playlist_membership_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ViewportProjectionKey {
    pub(super) generation: u64,
    pub(super) settings_revision: u64,
    pub(super) liked_membership_revision: u64,
    pub(super) playlist_membership_revision: u64,
    pub(super) first_index: u64,
    pub(super) last_index: i64,
    pub(super) selected_song_id: Option<String>,
    pub(super) request_song_ids: Vec<String>,
    pub(super) ordered_song_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ViewportResponseProjection {
    pub(super) key: ViewportProjectionKey,
    pub(super) response: CatalogViewportDto,
}

pub(super) struct ShiftedViewportRows {
    pub(super) reusable_rows: Vec<CatalogRowDto>,
    pub(super) entering_song_id: String,
    pub(super) insertion_index: usize,
}

impl CatalogSearchCache {
    pub(super) fn get(&mut self, key: &CatalogSearchKey) -> Option<&CatalogSearchOrder> {
        let index = self.entries.iter().position(|entry| &entry.key == key)?;
        let entry = self.entries.remove(index)?;
        self.entries.push_back(entry);
        self.entries.back().map(|entry| &entry.order)
    }

    pub(super) fn insert(&mut self, key: CatalogSearchKey, order: CatalogSearchOrder) {
        let index_count = order.index_count();
        if index_count > super::MAX_CACHED_CATALOG_SEARCH_INDICES {
            return;
        }
        if let Some(index) = self.entries.iter().position(|entry| entry.key == key)
            && let Some(entry) = self.entries.remove(index)
        {
            self.cached_indices = self
                .cached_indices
                .saturating_sub(entry.order.index_count());
        }
        while self.entries.len() >= super::MAX_CACHED_CATALOG_SEARCH_ORDERS
            || self.cached_indices.saturating_add(index_count)
                > super::MAX_CACHED_CATALOG_SEARCH_INDICES
        {
            let Some(entry) = self.entries.pop_front() else {
                break;
            };
            self.cached_indices = self
                .cached_indices
                .saturating_sub(entry.order.index_count());
        }
        self.cached_indices += index_count;
        self.entries
            .push_back(CatalogSearchOrderEntry { key, order });
    }

    pub(super) fn invalidate_liked(&mut self) {
        self.liked_membership_revision = self.liked_membership_revision.wrapping_add(1);
        self.retain_scope(|scope| !matches!(scope, CatalogSearchScope::Liked));
    }

    pub(super) fn invalidate_playlists(&mut self) {
        self.playlist_membership_revision = self.playlist_membership_revision.wrapping_add(1);
        self.retain_scope(|scope| !matches!(scope, CatalogSearchScope::Playlist(_)));
    }

    pub(super) fn invalidate_all(&mut self) {
        self.entries.clear();
        self.cached_indices = 0;
    }

    fn retain_scope(&mut self, keep: impl Fn(&CatalogSearchScope) -> bool) {
        let mut removed_indices = 0_usize;
        self.entries.retain(|entry| {
            if keep(&entry.key.scope) {
                true
            } else {
                removed_indices = removed_indices.saturating_add(entry.order.index_count());
                false
            }
        });
        self.cached_indices = self.cached_indices.saturating_sub(removed_indices);
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(super) struct AnalysisKey {
    pub(super) song_id: String,
    pub(super) generation: u64,
    pub(super) settings_revision: u64,
}

pub(super) struct AnalysisCacheEntry {
    analysis: CachedSongAnalysis,
    last_access: u64,
}

#[derive(Default)]
pub(super) struct AnalysisCacheState {
    pub(super) entries: HashMap<AnalysisKey, AnalysisCacheEntry>,
    pub(super) flights: HashMap<AnalysisKey, Arc<AnalysisFlight>>,
    next_access: u64,
}

impl AnalysisCacheState {
    pub(super) fn get(&mut self, key: &AnalysisKey) -> Option<CachedSongAnalysis> {
        self.next_access = self.next_access.wrapping_add(1);
        let access = self.next_access;
        let entry = self.entries.get_mut(key)?;
        entry.last_access = access;
        Some(entry.analysis.clone())
    }

    pub(super) fn insert(&mut self, key: AnalysisKey, analysis: CachedSongAnalysis) {
        self.next_access = self.next_access.wrapping_add(1);
        self.entries.insert(
            key,
            AnalysisCacheEntry {
                analysis,
                last_access: self.next_access,
            },
        );
        while self.entries.len() > super::MAX_CACHED_SONG_ANALYSES {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    pub(super) fn touch(&mut self, key: &AnalysisKey) {
        self.next_access = self.next_access.wrapping_add(1);
        if let Some(entry) = self.entries.get_mut(key) {
            entry.last_access = self.next_access;
        }
    }
}

pub(super) struct AnalysisFlight {
    pub(super) settings_revision: u64,
    #[cfg(test)]
    pub(super) waiters: AtomicUsize,
    pub(super) cancelled: AtomicBool,
    pub(super) result: Mutex<Option<Result<CachedSongAnalysis, String>>>,
    completed: Condvar,
    pub(super) latest_viewport_revision: AtomicU64,
    pub(super) independent_request: AtomicBool,
    pub(super) selected_priority: AtomicBool,
    pub(super) queued: AtomicBool,
    pub(super) queued_as_selected: AtomicBool,
}

impl AnalysisFlight {
    fn new(
        settings_revision: u64,
        viewport_revision: Option<u64>,
        priority: AnalysisPriority,
    ) -> Self {
        Self {
            settings_revision,
            #[cfg(test)]
            waiters: AtomicUsize::new(1),
            cancelled: AtomicBool::new(false),
            result: Mutex::new(None),
            completed: Condvar::new(),
            latest_viewport_revision: AtomicU64::new(viewport_revision.unwrap_or_default()),
            independent_request: AtomicBool::new(viewport_revision.is_none()),
            selected_priority: AtomicBool::new(priority == AnalysisPriority::Selected),
            queued: AtomicBool::new(false),
            queued_as_selected: AtomicBool::new(false),
        }
    }

    fn note_request(&self, viewport_revision: Option<u64>) {
        if let Some(revision) = viewport_revision {
            self.latest_viewport_revision
                .fetch_max(revision, Ordering::AcqRel);
        } else {
            self.independent_request.store(true, Ordering::Release);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AnalysisPriority {
    Visible,
    Selected,
}

#[derive(Default)]
pub(super) struct AnalysisAdmissionState {
    pub(super) active: usize,
    pub(super) queued: usize,
    selected_waiters: usize,
    queued_flights: Vec<Weak<AnalysisFlight>>,
}

pub(super) struct AnalysisPermit<'a> {
    runtime: &'a NativeDesktopRuntime,
}

pub(super) struct AnalysisRequest<'a> {
    pub(super) key: AnalysisKey,
    pub(super) path: &'a Path,
    pub(super) title: &'a str,
    pub(super) failure_format_label: &'a str,
    pub(super) settings: &'a ApplicationSettings,
    pub(super) policy: &'a MaterializedTimingPolicy,
    pub(super) viewport_revision: Option<u64>,
    pub(super) priority: AnalysisPriority,
    pub(super) cache_failures: bool,
}

#[cfg(test)]
pub(super) struct AnalysisTestGate {
    remaining: AtomicUsize,
    entered: Arc<std::sync::Barrier>,
    release: Arc<std::sync::Barrier>,
}

impl Drop for AnalysisPermit<'_> {
    fn drop(&mut self) {
        if let Ok(mut admission) = self.runtime.analysis_admission.lock() {
            admission.active = admission.active.saturating_sub(1);
            self.runtime.analysis_admission_changed.notify_all();
        }
    }
}

impl NativeDesktopRuntime {
    pub(super) fn catalog_settings_snapshot(
        &self,
    ) -> Result<(ApplicationSettings, u64, u64, u64), String> {
        let service = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        let search_cache = self
            .catalog_search_cache
            .lock()
            .map_err(|_| "native catalog search cache lock poisoned".to_string())?;
        Ok((
            service.snapshot().clone(),
            search_cache.liked_membership_revision,
            search_cache.playlist_membership_revision,
            self.analysis_revision.load(Ordering::Acquire),
        ))
    }

    fn catalog_viewport_revisions(&self) -> Result<(u64, u64, u64), String> {
        let _settings = self
            .settings
            .lock()
            .map_err(|_| "native settings lock poisoned".to_string())?;
        let search_cache = self
            .catalog_search_cache
            .lock()
            .map_err(|_| "native catalog search cache lock poisoned".to_string())?;
        Ok((
            search_cache.liked_membership_revision,
            search_cache.playlist_membership_revision,
            self.analysis_revision.load(Ordering::Acquire),
        ))
    }

    fn viewport_projection_is_current(
        &self,
        key: &ViewportProjectionKey,
        viewport_revision: u64,
    ) -> Result<bool, String> {
        if self.viewport_revision.load(Ordering::Acquire) != viewport_revision {
            return Ok(false);
        }
        let (liked_revision, playlist_revision, settings_revision) =
            self.catalog_viewport_revisions()?;
        if liked_revision != key.liked_membership_revision
            || playlist_revision != key.playlist_membership_revision
            || settings_revision != key.settings_revision
        {
            return Ok(false);
        }
        let generation = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?
            .index
            .generation();
        Ok(generation == key.generation
            && self.viewport_revision.load(Ordering::Acquire) == viewport_revision)
    }

    pub(super) fn cached_viewport_projection(
        &self,
        key: &ViewportProjectionKey,
        viewport_revision: u64,
    ) -> Result<Option<CatalogViewportDto>, String> {
        let projection = self
            .viewport_projection
            .lock()
            .map_err(|_| "native viewport projection lock poisoned".to_string())?
            .as_ref()
            .filter(|projection| projection.key == *key)
            .cloned();
        let Some(projection) = projection else {
            return Ok(None);
        };
        if projection.response.items.len() > super::MAX_CACHED_VIEWPORT_RESPONSE_ROWS
            || !self.viewport_projection_is_current(key, viewport_revision)?
        {
            return Ok(None);
        }
        #[cfg(test)]
        self.viewport_projection_test_hits
            .fetch_add(1, Ordering::AcqRel);
        Ok(Some(projection.response))
    }

    pub(super) fn shifted_viewport_rows(
        &self,
        key: &ViewportProjectionKey,
        viewport_revision: u64,
    ) -> Result<Option<ShiftedViewportRows>, String> {
        let reusable = {
            let projection = self
                .viewport_projection
                .lock()
                .map_err(|_| "native viewport projection lock poisoned".to_string())?;
            let Some(projection) = projection.as_ref() else {
                return Ok(None);
            };
            let previous = &projection.key;
            let len = key.ordered_song_ids.len();
            if !(2..=super::MAX_CACHED_VIEWPORT_RESPONSE_ROWS).contains(&len)
                || previous.ordered_song_ids.len() != len
                || previous.request_song_ids != key.request_song_ids
                || !key.request_song_ids.is_empty()
                || previous.generation != key.generation
                || previous.settings_revision != key.settings_revision
                || previous.liked_membership_revision != key.liked_membership_revision
                || previous.playlist_membership_revision != key.playlist_membership_revision
                || previous.selected_song_id != key.selected_song_id
                || key.selected_song_id.is_some()
                || projection.response.items.len() != len
                || projection.response.generation != previous.generation
                || projection.response.first_index != previous.first_index
                || projection.response.last_index != previous.last_index
                || projection.response.selected_song_id != previous.selected_song_id
                || !projection
                    .response
                    .items
                    .iter()
                    .zip(&previous.ordered_song_ids)
                    .all(|(row, song_id)| row.song_id == *song_id)
            {
                return Ok(None);
            }
            let forward = previous.first_index.checked_add(1) == Some(key.first_index)
                && previous.last_index.checked_add(1) == Some(key.last_index)
                && previous.ordered_song_ids[1..] == key.ordered_song_ids[..len - 1];
            if forward {
                Some(ShiftedViewportRows {
                    reusable_rows: projection.response.items.iter().skip(1).cloned().collect(),
                    entering_song_id: key.ordered_song_ids[len - 1].clone(),
                    insertion_index: len - 1,
                })
            } else {
                let backward = key.first_index.checked_add(1) == Some(previous.first_index)
                    && key.last_index.checked_add(1) == Some(previous.last_index)
                    && key.ordered_song_ids[1..] == previous.ordered_song_ids[..len - 1];
                backward.then(|| ShiftedViewportRows {
                    reusable_rows: projection
                        .response
                        .items
                        .iter()
                        .take(len - 1)
                        .cloned()
                        .collect(),
                    entering_song_id: key.ordered_song_ids[0].clone(),
                    insertion_index: 0,
                })
            }
        };
        let Some(reusable) = reusable else {
            return Ok(None);
        };
        if !self.viewport_projection_is_current(key, viewport_revision)? {
            return Ok(None);
        }
        Ok(Some(reusable))
    }

    pub(super) fn publish_viewport_projection(
        &self,
        key: ViewportProjectionKey,
        response: CatalogViewportDto,
        viewport_revision: u64,
    ) -> Result<(), String> {
        if response.items.len() > super::MAX_CACHED_VIEWPORT_RESPONSE_ROWS
            || key.request_song_ids.len() > super::MAX_CACHED_VIEWPORT_RESPONSE_ROWS
            || key.ordered_song_ids.len() > super::MAX_CACHED_VIEWPORT_RESPONSE_ROWS
            || !response.accepted
            || !self.viewport_projection_is_current(&key, viewport_revision)?
        {
            return Ok(());
        }
        let mut projection = self
            .viewport_projection
            .lock()
            .map_err(|_| "native viewport projection lock poisoned".to_string())?;
        if self.viewport_revision.load(Ordering::Acquire) == viewport_revision {
            *projection = Some(ViewportResponseProjection { key, response });
        }
        Ok(())
    }

    pub(super) fn invalidate_viewport_projection(&self) {
        if let Ok(mut projection) = self.viewport_projection.lock() {
            *projection = None;
        }
    }

    pub(super) fn touch_analysis_cache(&self, key: &AnalysisKey) -> Result<(), String> {
        self.analysis_cache
            .lock()
            .map_err(|_| "native analysis cache lock poisoned".to_string())?
            .touch(key);
        Ok(())
    }

    fn analysis_cache_get(&self, key: &AnalysisKey) -> Result<Option<CachedSongAnalysis>, String> {
        self.analysis_cache
            .lock()
            .map_err(|_| "native analysis cache lock poisoned".to_string())
            .map(|mut cache| cache.get(key))
    }

    #[cfg(test)]
    pub(super) fn set_analysis_test_gate(
        &self,
        entered: Arc<std::sync::Barrier>,
        release: Arc<std::sync::Barrier>,
        held_analyses: usize,
    ) {
        *self
            .analysis_test_gate
            .lock()
            .expect("analysis test gate lock") = Some(Arc::new(AnalysisTestGate {
            remaining: AtomicUsize::new(held_analyses),
            entered,
            release,
        }));
    }

    #[cfg(test)]
    fn before_song_analysis(&self, song_id: &str) {
        self.analysis_test_runs.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut order) = self.analysis_test_order.lock() {
            order.push(song_id.to_owned());
        }
        let gate = self
            .analysis_test_gate
            .lock()
            .ok()
            .and_then(|gate| gate.as_ref().cloned());
        if let Some(gate) = gate {
            let mut remaining = gate.remaining.load(Ordering::Acquire);
            while remaining > 0 {
                match gate.remaining.compare_exchange_weak(
                    remaining,
                    remaining - 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        gate.entered.wait();
                        gate.release.wait();
                        break;
                    }
                    Err(observed) => remaining = observed,
                }
            }
        }
    }

    pub(super) fn begin_viewport_request(&self) -> Result<u64, String> {
        let mut admission = self
            .analysis_admission
            .lock()
            .map_err(|_| "native analysis admission lock poisoned".to_string())?;
        let revision = self
            .viewport_revision
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.cancel_obsolete_queued_analyses(&mut admission);
        self.analysis_admission_changed.notify_all();
        Ok(revision)
    }

    pub(super) fn analysis_for_request(
        &self,
        request: AnalysisRequest<'_>,
    ) -> Result<CachedSongAnalysis, String> {
        let AnalysisRequest {
            key,
            path,
            title,
            failure_format_label,
            settings,
            policy,
            viewport_revision,
            priority,
            cache_failures,
        } = request;
        if let Some(analysis) = self.analysis_cache_get(&key)? {
            return Ok(analysis);
        }
        let (flight, owner) = {
            let mut cache = self
                .analysis_cache
                .lock()
                .map_err(|_| "native analysis cache lock poisoned".to_string())?;
            if let Some(analysis) = cache.get(&key) {
                return Ok(analysis);
            }
            if let Some(flight) = cache.flights.get(&key).cloned()
                && !flight.cancelled.load(Ordering::Acquire)
            {
                #[cfg(test)]
                flight.waiters.fetch_add(1, Ordering::AcqRel);
                flight.note_request(viewport_revision);
                if priority == AnalysisPriority::Selected
                    && !flight.selected_priority.swap(true, Ordering::AcqRel)
                {
                    let mut admission = self
                        .analysis_admission
                        .lock()
                        .map_err(|_| "native analysis admission lock poisoned".to_string())?;
                    if flight.queued.load(Ordering::Acquire)
                        && !flight.queued_as_selected.swap(true, Ordering::AcqRel)
                    {
                        admission.selected_waiters += 1;
                        self.analysis_admission_changed.notify_all();
                    }
                }
                (flight, false)
            } else {
                let flight = Arc::new(AnalysisFlight::new(
                    key.settings_revision,
                    viewport_revision,
                    priority,
                ));
                cache.flights.insert(key.clone(), Arc::clone(&flight));
                (flight, true)
            }
        };

        if !owner {
            let mut result = flight
                .result
                .lock()
                .map_err(|_| "native analysis flight lock poisoned".to_string())?;
            loop {
                if let Some(result) = result.as_ref() {
                    return result.clone();
                }
                result = flight
                    .completed
                    .wait(result)
                    .map_err(|_| "native analysis flight lock poisoned".to_string())?;
            }
        }

        let result = match self.acquire_analysis_permit(&flight) {
            Ok(_permit) => {
                if self.analysis_flight_is_obsolete(&flight) {
                    Err("catalog analysis request was superseded".into())
                } else {
                    #[cfg(test)]
                    self.before_song_analysis(&key.song_id);
                    let analyzed =
                        self.analyze_song(path, &key.song_id, title, title, settings, policy);
                    let analyzed = if cache_failures {
                        Ok(analyzed.unwrap_or_else(|_| {
                            CachedSongAnalysis::error(failure_format_label.to_owned())
                        }))
                    } else {
                        analyzed
                    };
                    if self.analysis_key_is_current(&key, &flight)
                        && let Ok(analysis) = &analyzed
                        && let Ok(mut cache) = self.analysis_cache.lock()
                    {
                        cache.insert(key.clone(), analysis.clone());
                    }
                    analyzed
                }
            }
            Err(error) => Err(error),
        };
        self.finish_analysis_flight(&key, &flight, result.clone());
        result
    }

    fn acquire_analysis_permit(
        &self,
        flight: &Arc<AnalysisFlight>,
    ) -> Result<AnalysisPermit<'_>, String> {
        let mut admission = self
            .analysis_admission
            .lock()
            .map_err(|_| "native analysis admission lock poisoned".to_string())?;
        if flight.cancelled.load(Ordering::Acquire) || self.analysis_flight_is_obsolete(flight) {
            return Err("catalog analysis request was superseded".into());
        }
        let selected = flight.selected_priority.load(Ordering::Acquire);
        if admission.active < super::MAX_ACTIVE_SONG_ANALYSES
            && (selected || admission.selected_waiters == 0)
        {
            admission.active += 1;
            return Ok(AnalysisPermit { runtime: self });
        }
        if admission.queued >= super::MAX_QUEUED_SONG_ANALYSES {
            #[cfg(test)]
            self.analysis_test_queue_rejections
                .fetch_add(1, Ordering::AcqRel);
            return Err("catalog analysis queue is full".into());
        }
        admission.queued += 1;
        flight.queued.store(true, Ordering::Release);
        flight.queued_as_selected.store(selected, Ordering::Release);
        if selected {
            admission.selected_waiters += 1;
        }
        admission.queued_flights.push(Arc::downgrade(flight));
        self.analysis_admission_changed.notify_all();
        loop {
            if flight.cancelled.load(Ordering::Acquire) || self.analysis_flight_is_obsolete(flight)
            {
                if self.release_queued_analysis_locked(&mut admission, flight) {
                    flight.cancelled.store(true, Ordering::Release);
                }
                self.analysis_admission_changed.notify_all();
                return Err("catalog analysis request was superseded".into());
            }
            if flight.selected_priority.load(Ordering::Acquire)
                && !flight.queued_as_selected.swap(true, Ordering::AcqRel)
            {
                admission.selected_waiters += 1;
            }
            let selected = flight.selected_priority.load(Ordering::Acquire);
            if admission.active < super::MAX_ACTIVE_SONG_ANALYSES
                && (selected || admission.selected_waiters == 0)
            {
                self.release_queued_analysis_locked(&mut admission, flight);
                admission.active += 1;
                self.analysis_admission_changed.notify_all();
                return Ok(AnalysisPermit { runtime: self });
            }
            admission = self
                .analysis_admission_changed
                .wait(admission)
                .map_err(|_| "native analysis admission lock poisoned".to_string())?;
        }
    }

    fn release_queued_analysis_locked(
        &self,
        admission: &mut AnalysisAdmissionState,
        flight: &Arc<AnalysisFlight>,
    ) -> bool {
        let was_queued = flight.queued.swap(false, Ordering::AcqRel);
        if was_queued {
            admission.queued = admission.queued.saturating_sub(1);
            if flight.queued_as_selected.swap(false, Ordering::AcqRel) {
                admission.selected_waiters = admission.selected_waiters.saturating_sub(1);
            }
        }
        admission.queued_flights.retain(|queued| {
            queued
                .upgrade()
                .is_some_and(|queued| !Arc::ptr_eq(&queued, flight))
        });
        was_queued
    }

    fn cancel_obsolete_queued_analyses(&self, admission: &mut AnalysisAdmissionState) {
        let queued_flights = std::mem::take(&mut admission.queued_flights);
        for weak in queued_flights {
            let Some(flight) = weak.upgrade() else {
                continue;
            };
            if self.analysis_flight_is_obsolete(&flight) {
                if self.release_queued_analysis_locked(admission, &flight) {
                    flight.cancelled.store(true, Ordering::Release);
                }
            } else if flight.queued.load(Ordering::Acquire) {
                admission.queued_flights.push(Arc::downgrade(&flight));
            }
        }
    }

    fn analysis_flight_is_obsolete(&self, flight: &AnalysisFlight) -> bool {
        flight.settings_revision != self.analysis_revision.load(Ordering::Acquire)
            || (!flight.independent_request.load(Ordering::Acquire)
                && flight.latest_viewport_revision.load(Ordering::Acquire)
                    != self.viewport_revision.load(Ordering::Acquire))
    }

    fn analysis_key_is_current(&self, key: &AnalysisKey, flight: &AnalysisFlight) -> bool {
        if key.settings_revision != self.analysis_revision.load(Ordering::Acquire)
            || self.analysis_flight_is_obsolete(flight)
        {
            return false;
        }
        self.catalog.lock().is_ok_and(|catalog| {
            catalog.index.generation() == key.generation
                && catalog
                    .index
                    .canonical_path_for_song_id(&key.song_id, Some(key.generation))
                    .is_ok()
        })
    }

    fn finish_analysis_flight(
        &self,
        key: &AnalysisKey,
        flight: &Arc<AnalysisFlight>,
        result: Result<CachedSongAnalysis, String>,
    ) {
        if let Ok(mut state) = flight.result.lock() {
            *state = Some(result);
            flight.completed.notify_all();
        }
        if let Ok(mut cache) = self.analysis_cache.lock()
            && cache
                .flights
                .get(key)
                .is_some_and(|current| Arc::ptr_eq(current, flight))
        {
            cache.flights.remove(key);
        }
    }

    pub(super) fn invalidate_analysis_cache(&self) {
        if let Ok(mut admission) = self.analysis_admission.lock() {
            self.analysis_revision.fetch_add(1, Ordering::AcqRel);
            self.cancel_obsolete_queued_analyses(&mut admission);
            self.analysis_admission_changed.notify_all();
        } else {
            self.analysis_revision.fetch_add(1, Ordering::AcqRel);
        }
        if let Ok(mut cache) = self.analysis_cache.lock() {
            cache.entries.clear();
        }
        self.invalidate_viewport_projection();
    }
}
