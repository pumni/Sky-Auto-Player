//! Pure catalog index and generation semantics.
//!
//! Directory enumeration and canonicalization are outer-adapter concerns. The
//! core receives canonical path strings, owns opaque IDs, stable ordering,
//! generation checks, and bounded path-free rows. Fuzzy ranking is an explicit
//! port because the checked-in search fixtures define its WRatio contract; the
//! core must not silently substitute a different algorithm.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
mod identity;
mod ranking;
mod search_order;

use identity::{is_supported_path, normalize_query, unicode_casefold, validate_window};
pub use identity::{
    is_valid_song_id, normalize_search_text, normalized_canonical_path, song_id_for_canonical_path,
};
pub use ranking::{WRatioRanker, wratio_score};

pub const SUPPORTED_EXTENSIONS: [&str; 3] = ["json", "skysheet", "txt"];
pub const SONG_ID_LENGTH: usize = 32;
pub const MAX_PAGE_SIZE: usize = 200;
pub const MAX_QUERY_LENGTH: usize = 1024;
pub const FUZZY_SCORE_CUTOFF: f64 = 60.0;
pub use crate::catalog_manifest::{
    BUILTIN_MANIFEST_SCHEMA_VERSION, BuiltinCatalogManifest, BuiltinManifestError,
    BuiltinSongManifestEntry, RetiredBuiltinSong,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSourceEntry {
    pub canonical_path: String,
    pub title: String,
    pub identity: CatalogSourceIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogSourceIdentity {
    PathDerived,
    Stable(StableSongId),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StableSongId(String);

impl StableSongId {
    pub fn new(value: impl Into<String>) -> Result<Self, CatalogError> {
        let value = value.into();
        if !is_valid_song_id(&value) {
            return Err(CatalogError::UnknownSongId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl CatalogSourceEntry {
    pub fn path_derived(canonical_path: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            canonical_path: canonical_path.into(),
            title: title.into(),
            identity: CatalogSourceIdentity::PathDerived,
        }
    }

    pub fn stable(
        canonical_path: impl Into<String>,
        title: impl Into<String>,
        song_id: StableSongId,
    ) -> Self {
        Self {
            canonical_path: canonical_path.into(),
            title: title.into(),
            identity: CatalogSourceIdentity::Stable(song_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogRow {
    pub song_id: String,
    pub title: String,
}

/// Trusted in-process lookup returned to outer adapters.  The canonical path
/// never crosses the delivery DTO boundary; it is only used for file access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntryView {
    pub row: CatalogRow,
    pub canonical_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogEntry {
    row: CatalogRow,
    canonical_path: String,
    search_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogPage {
    pub items: Vec<CatalogRow>,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
    pub generation: u64,
}

/// Exact in-memory row order for one normalized query and catalog generation.
/// The native adapter may keep this order in a bounded, source-scoped cache;
/// it is never persisted or treated as identity authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSearchOrder {
    generation: u64,
    normalized_query: String,
    indices: Vec<usize>,
}

impl CatalogSearchOrder {
    pub fn index_count(&self) -> usize {
        self.indices.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogSnapshot {
    pub items: Vec<CatalogRow>,
    pub generation: u64,
    pub total: usize,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum CatalogError {
    #[error("catalog query must be text")]
    InvalidQuery,
    #[error("catalog query exceeds {MAX_QUERY_LENGTH} characters")]
    QueryTooLong,
    #[error("catalog limit must be between 1 and {MAX_PAGE_SIZE}")]
    InvalidLimit,
    #[error("catalog generation is stale")]
    StaleGeneration,
    #[error("catalog generation overflowed")]
    GenerationOverflow,
    #[error("catalog source path is empty")]
    EmptyPath,
    #[error("catalog source is unavailable: {0}")]
    SourceUnavailable(String),
    #[error("unsupported catalog source extension")]
    UnsupportedExtension,
    #[error("song ID collision for distinct canonical paths")]
    IdCollision,
    #[error("catalog source declares conflicting identities for one canonical path")]
    IdentityPathConflict,
    #[error("malformed or unknown song ID")]
    UnknownSongId,
    #[error("fuzzy ranking is not available for this shadow index")]
    FuzzyRankingUnavailable,
    #[error("catalog search order does not match the requested query")]
    SearchOrderMismatch,
}

pub trait FuzzyRanker {
    fn rank(&self, query: &str, search_keys: &[String], score_cutoff: f64) -> Vec<usize>;
}

pub trait SongSource {
    fn entries(&self) -> Result<Vec<CatalogSourceEntry>, CatalogError>;
}

#[derive(Default)]
pub struct CatalogIndex {
    generation: u64,
    entries: Vec<CatalogEntry>,
    by_id: HashMap<String, usize>,
}

impl CatalogIndex {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn replace_entries(
        &mut self,
        sources: impl IntoIterator<Item = CatalogSourceEntry>,
    ) -> Result<CatalogSnapshot, CatalogError> {
        let mut entries: Vec<CatalogEntry> = Vec::new();
        let mut by_id: HashMap<String, usize> = HashMap::new();
        let mut seen_paths = HashMap::<String, String>::new();
        for source in sources {
            if source.canonical_path.is_empty() {
                return Err(CatalogError::EmptyPath);
            }
            if !is_supported_path(&source.canonical_path) {
                return Err(CatalogError::UnsupportedExtension);
            }
            let normalized_path = normalized_canonical_path(&source.canonical_path);
            let song_id = match &source.identity {
                CatalogSourceIdentity::PathDerived => {
                    song_id_for_canonical_path(&source.canonical_path)
                }
                CatalogSourceIdentity::Stable(stable) => stable.as_str().to_owned(),
            };
            if let Some(previous_id) = seen_paths.get(&normalized_path) {
                if previous_id != &song_id {
                    return Err(CatalogError::IdentityPathConflict);
                }
                continue;
            }
            seen_paths.insert(normalized_path.clone(), song_id.clone());
            if let Some(previous) = by_id.get(&song_id) {
                if normalized_canonical_path(&entries[*previous].canonical_path) != normalized_path
                {
                    return Err(CatalogError::IdCollision);
                }
                continue;
            }
            let entry = CatalogEntry {
                row: CatalogRow {
                    song_id: song_id.clone(),
                    title: source.title.clone(),
                },
                canonical_path: source.canonical_path,
                search_key: normalize_search_text(&source.title),
            };
            by_id.insert(song_id, entries.len());
            entries.push(entry);
        }
        entries.sort_by(|a, b| {
            a.search_key
                .cmp(&b.search_key)
                .then_with(|| unicode_casefold(&a.row.title).cmp(&unicode_casefold(&b.row.title)))
        });
        by_id.clear();
        for (index, entry) in entries.iter().enumerate() {
            by_id.insert(entry.row.song_id.clone(), index);
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(CatalogError::GenerationOverflow)?;
        self.entries = entries;
        self.by_id = by_id;
        Ok(self.snapshot())
    }

    pub fn snapshot(&self) -> CatalogSnapshot {
        let items = self
            .entries
            .iter()
            .map(|entry| entry.row.clone())
            .collect::<Vec<_>>();
        CatalogSnapshot {
            total: items.len(),
            items,
            generation: self.generation,
        }
    }

    pub fn search<R: FuzzyRanker>(
        &self,
        ranker: &R,
        query: &str,
        offset: usize,
        limit: usize,
        generation: Option<u64>,
    ) -> Result<CatalogPage, CatalogError> {
        self.search_with_allowed_ids(ranker, query, offset, limit, generation, None)
    }

    pub fn search_with_allowed_ids<R: FuzzyRanker>(
        &self,
        ranker: &R,
        query: &str,
        offset: usize,
        limit: usize,
        generation: Option<u64>,
        allowed_song_ids: Option<&BTreeSet<String>>,
    ) -> Result<CatalogPage, CatalogError> {
        self.check_generation(generation)?;
        validate_window(query, offset, limit)?;
        let normalized = normalize_query(query)?;
        if normalized.chars().count() <= 1 {
            if normalized.is_empty() && allowed_song_ids.is_none() {
                let total = self.entries.len();
                let start = offset.min(total);
                let end = start.saturating_add(limit).min(total);
                let items = self.entries[start..end]
                    .iter()
                    .map(|entry| entry.row.clone())
                    .collect();
                return Ok(CatalogPage {
                    items,
                    offset,
                    limit,
                    total,
                    generation: self.generation,
                });
            }

            if let Some(ids) = allowed_song_ids
                && ids.len() <= self.entries.len() / 4
            {
                let mut indices = Vec::with_capacity(ids.len());
                for song_id in ids {
                    if let Some(index) = self.by_id.get(song_id) {
                        let entry = &self.entries[*index];
                        if normalized.is_empty() || entry.search_key.contains(&normalized) {
                            indices.push(*index);
                        }
                    }
                }
                indices.sort_unstable();
                let total = indices.len();
                let items = indices
                    .into_iter()
                    .skip(offset)
                    .take(limit)
                    .map(|index| self.entries[index].row.clone())
                    .collect();
                return Ok(CatalogPage {
                    items,
                    offset,
                    limit,
                    total,
                    generation: self.generation,
                });
            }

            let mut items = Vec::with_capacity(limit.min(self.entries.len()));
            let mut total = 0;
            for entry in &self.entries {
                if allowed_song_ids.is_some_and(|ids| !ids.contains(&entry.row.song_id))
                    || (!normalized.is_empty() && !entry.search_key.contains(&normalized))
                {
                    continue;
                }
                if total >= offset && items.len() < limit {
                    items.push(entry.row.clone());
                }
                total += 1;
            }
            return Ok(CatalogPage {
                items,
                offset,
                limit,
                total,
                generation: self.generation,
            });
        }
        let candidates = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                allowed_song_ids.is_none_or(|ids| ids.contains(&entry.row.song_id))
            })
            .collect::<Vec<_>>();
        let indices = if normalized.is_empty() {
            candidates
                .iter()
                .map(|(index, _)| *index)
                .collect::<Vec<_>>()
        } else if normalized.chars().count() == 1 {
            candidates
                .iter()
                .filter_map(|entry| entry.1.search_key.contains(&normalized).then_some(entry.0))
                .collect()
        } else {
            let keys = candidates
                .iter()
                .map(|(_, entry)| entry.search_key.clone())
                .collect::<Vec<_>>();
            let mut ranked = Vec::new();
            let mut seen = vec![false; candidates.len()];
            for candidate_index in ranker.rank(&normalized, &keys, FUZZY_SCORE_CUTOFF) {
                if let Some((index, _)) = candidates.get(candidate_index)
                    && !seen[candidate_index]
                {
                    seen[candidate_index] = true;
                    ranked.push(*index);
                }
            }
            ranked
        };
        let total = indices.len();
        let items = indices
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|index| self.entries[index].row.clone())
            .collect();
        Ok(CatalogPage {
            items,
            offset,
            limit,
            total,
            generation: self.generation,
        })
    }

    pub fn song_ids_in_range(
        &self,
        first: usize,
        last: usize,
        generation: Option<u64>,
    ) -> Result<Vec<String>, CatalogError> {
        self.check_generation(generation)?;
        if first > last || last >= self.entries.len() {
            return Err(CatalogError::InvalidLimit);
        }
        Ok(self.entries[first..=last]
            .iter()
            .map(|entry| entry.row.song_id.clone())
            .collect())
    }

    pub fn count_allowed_ids(
        &self,
        allowed_song_ids: &BTreeSet<String>,
        generation: Option<u64>,
    ) -> Result<usize, CatalogError> {
        self.check_generation(generation)?;
        if allowed_song_ids.len() <= self.entries.len() / 4 {
            return Ok(allowed_song_ids
                .iter()
                .filter(|song_id| self.by_id.contains_key(*song_id))
                .count());
        }
        Ok(self
            .entries
            .iter()
            .filter(|entry| allowed_song_ids.contains(&entry.row.song_id))
            .count())
    }

    pub fn search_substrings(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
        generation: Option<u64>,
    ) -> Result<CatalogPage, CatalogError> {
        self.check_generation(generation)?;
        validate_window(query, offset, limit)?;
        let normalized = normalize_query(query)?;
        if normalized.chars().count() > 1 {
            let items = self
                .entries
                .iter()
                .filter(|entry| entry.search_key.contains(&normalized))
                .map(|entry| entry.row.clone())
                .collect::<Vec<_>>();
            let total = items.len();
            return Ok(CatalogPage {
                items: items.into_iter().skip(offset).take(limit).collect(),
                offset,
                limit,
                total,
                generation: self.generation,
            });
        }
        let items = self
            .entries
            .iter()
            .filter(|entry| normalized.is_empty() || entry.search_key.contains(&normalized))
            .map(|entry| entry.row.clone())
            .collect::<Vec<_>>();
        let total = items.len();
        Ok(CatalogPage {
            items: items.into_iter().skip(offset).take(limit).collect(),
            offset,
            limit,
            total,
            generation: self.generation,
        })
    }

    pub fn canonical_path_for_song_id(
        &self,
        song_id: &str,
        generation: Option<u64>,
    ) -> Result<&str, CatalogError> {
        self.check_generation(generation)?;
        if !is_valid_song_id(song_id) {
            return Err(CatalogError::UnknownSongId);
        }
        self.by_id
            .get(song_id)
            .map(|index| self.entries[*index].canonical_path.as_str())
            .ok_or(CatalogError::UnknownSongId)
    }

    pub fn entry_for_song_id(
        &self,
        song_id: &str,
        generation: Option<u64>,
    ) -> Result<CatalogEntryView, CatalogError> {
        self.check_generation(generation)?;
        let path = self
            .canonical_path_for_song_id(song_id, generation)?
            .to_owned();
        let index = self.by_id.get(song_id).ok_or(CatalogError::UnknownSongId)?;
        Ok(CatalogEntryView {
            row: self.entries[*index].row.clone(),
            canonical_path: path,
        })
    }

    fn check_generation(&self, generation: Option<u64>) -> Result<(), CatalogError> {
        if generation.is_some_and(|value| value != self.generation) {
            Err(CatalogError::StaleGeneration)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "catalog_perf_tests.rs"]
mod perf_tests;
