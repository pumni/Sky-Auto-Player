//! Pure catalog index and generation semantics.
//!
//! Directory enumeration and canonicalization are outer-adapter concerns. The
//! core receives canonical path strings, owns opaque IDs, stable ordering,
//! generation checks, and bounded path-free rows. Fuzzy ranking is an explicit
//! port because the current Python implementation uses RapidFuzz WRatio; the
//! core must not silently substitute a different algorithm.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

pub const SUPPORTED_EXTENSIONS: [&str; 3] = ["json", "skysheet", "txt"];
pub const MAX_PAGE_SIZE: usize = 200;
pub const MAX_QUERY_LENGTH: usize = 1024;
pub const FUZZY_SCORE_CUTOFF: f64 = 60.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSourceEntry {
    pub canonical_path: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogRow {
    pub song_id: String,
    pub title: String,
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
    #[error("catalog offset must be a non-negative bounded integer")]
    InvalidOffset,
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
    #[error("malformed or unknown song ID")]
    UnknownSongId,
    #[error("fuzzy ranking is not available for this shadow index")]
    FuzzyRankingUnavailable,
}

pub trait FuzzyRanker {
    fn rank(&self, query: &str, search_keys: &[String], score_cutoff: f64) -> Vec<usize>;
}

pub trait SongSource {
    fn entries(&self) -> Result<Vec<CatalogSourceEntry>, CatalogError>;
}

pub struct CatalogIndex {
    generation: u64,
    entries: Vec<CatalogEntry>,
    by_id: HashMap<String, usize>,
}

impl Default for CatalogIndex {
    fn default() -> Self {
        Self {
            generation: 0,
            entries: Vec::new(),
            by_id: HashMap::new(),
        }
    }
}

impl CatalogIndex {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn replace_entries(
        &mut self,
        sources: impl IntoIterator<Item = CatalogSourceEntry>,
    ) -> Result<CatalogSnapshot, CatalogError> {
        let mut entries: Vec<CatalogEntry> = Vec::new();
        let mut by_id: HashMap<String, usize> = HashMap::new();
        let mut seen_paths = HashMap::<String, ()>::new();
        for source in sources {
            if source.canonical_path.is_empty() {
                return Err(CatalogError::EmptyPath);
            }
            if !is_supported_path(&source.canonical_path) {
                return Err(CatalogError::UnsupportedExtension);
            }
            if seen_paths
                .insert(source.canonical_path.clone(), ())
                .is_some()
            {
                continue;
            }
            let normalized_path = normalized_canonical_path(&source.canonical_path);
            let song_id = song_id_for_canonical_path(&source.canonical_path);
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
                .then_with(|| a.row.title.to_lowercase().cmp(&b.row.title.to_lowercase()))
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
        self.check_generation(generation)?;
        validate_window(query, offset, limit)?;
        let normalized = normalize_query(query)?;
        let indices = if normalized.is_empty() {
            (0..self.entries.len()).collect::<Vec<_>>()
        } else if normalized.chars().count() == 1 {
            self.entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    entry.search_key.contains(&normalized).then_some(index)
                })
                .collect()
        } else {
            let keys = self
                .entries
                .iter()
                .map(|entry| entry.search_key.clone())
                .collect::<Vec<_>>();
            let mut ranked = Vec::new();
            for index in ranker.rank(&normalized, &keys, FUZZY_SCORE_CUTOFF) {
                if index < self.entries.len() && !ranked.contains(&index) {
                    ranked.push(index);
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
        if song_id.len() != 32
            || !song_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(CatalogError::UnknownSongId);
        }
        self.by_id
            .get(song_id)
            .map(|index| self.entries[*index].canonical_path.as_str())
            .ok_or(CatalogError::UnknownSongId)
    }

    fn check_generation(&self, generation: Option<u64>) -> Result<(), CatalogError> {
        if generation.is_some_and(|value| value != self.generation) {
            Err(CatalogError::StaleGeneration)
        } else {
            Ok(())
        }
    }
}

pub fn normalize_search_text(value: &str) -> String {
    value
        .nfkd()
        .filter(|character| !is_combining_mark(*character))
        .map(|character| match character {
            'đ' | 'Đ' => 'd',
            other => other,
        })
        .collect::<String>()
        .to_lowercase()
}

pub fn song_id_for_canonical_path(path: &str) -> String {
    let digest = Sha256::digest(normalized_canonical_path(path).as_bytes());
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Match Python's `normcase(normpath(resolve(...)))` for the canonical path
/// supplied by the outer filesystem adapter. The adapter performs strict
/// filesystem canonicalization; this function only performs deterministic
/// separator/case normalization before hashing and collision checks.
pub fn normalized_canonical_path(path: &str) -> String {
    #[cfg(windows)]
    {
        path.replace('/', "\\").to_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.replace('\\', "/")
    }
}

fn is_supported_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn normalize_query(query: &str) -> Result<String, CatalogError> {
    if query.len() > MAX_QUERY_LENGTH {
        return Err(CatalogError::QueryTooLong);
    }
    Ok(normalize_search_text(query).trim().to_owned())
}

fn validate_window(query: &str, offset: usize, limit: usize) -> Result<(), CatalogError> {
    let _ = normalize_query(query)?;
    if offset > 1_000_000_000 {
        return Err(CatalogError::InvalidOffset);
    }
    if !(1..=MAX_PAGE_SIZE).contains(&limit) {
        return Err(CatalogError::InvalidLimit);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoFuzzy;
    impl FuzzyRanker for NoFuzzy {
        fn rank(&self, _query: &str, _keys: &[String], _score_cutoff: f64) -> Vec<usize> {
            Vec::new()
        }
    }

    fn entry(path: &str, title: &str) -> CatalogSourceEntry {
        CatalogSourceEntry {
            canonical_path: path.into(),
            title: title.into(),
        }
    }

    #[test]
    fn ids_and_generation_are_stable_and_path_free() {
        let mut catalog = CatalogIndex::default();
        let snapshot = catalog
            .replace_entries([entry("C:/songs/Cà Phê.json", "Cà Phê")])
            .expect("index");
        assert_eq!(snapshot.generation, 1);
        assert_eq!(snapshot.items[0].song_id.len(), 32);
        assert_eq!(
            catalog
                .canonical_path_for_song_id(&snapshot.items[0].song_id, Some(1))
                .unwrap(),
            "C:/songs/Cà Phê.json"
        );
        assert_eq!(
            catalog
                .search_substrings("ca phe", 0, 10, Some(1))
                .unwrap()
                .total,
            1
        );
        assert!(
            matches!(catalog.search(&NoFuzzy, "ca phe", 0, 10, Some(1)), Ok(page) if page.total == 0)
        );
    }

    #[test]
    fn stale_generation_and_bad_windows_fail_closed() {
        let mut catalog = CatalogIndex::default();
        catalog
            .replace_entries([entry("C:/songs/a.txt", "Alpha")])
            .expect("index");
        assert_eq!(
            catalog.search_substrings("", 0, 10, Some(2)).unwrap_err(),
            CatalogError::StaleGeneration
        );
        assert_eq!(
            catalog.search_substrings("", 0, 201, Some(1)).unwrap_err(),
            CatalogError::InvalidLimit
        );
    }
}
