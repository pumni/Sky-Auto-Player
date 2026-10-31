//! Pure catalog index and generation semantics.
//!
//! Directory enumeration and canonicalization are outer-adapter concerns. The
//! core receives canonical path strings, owns opaque IDs, stable ordering,
//! generation checks, and bounded path-free rows. Fuzzy ranking is an explicit
//! port because the checked-in search fixtures define its WRatio contract; the
//! core must not silently substitute a different algorithm.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};
mod search_order;

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

/// Bounded WRatio-compatible ranker for the native index.
///
/// The accepted WRatio contract is a composition of normalized Levenshtein, partial,
/// token-sort, and token-set ratios.  The native implementation keeps the
/// same score selection/cutoff policy while operating on the already
/// normalized catalog keys.  It is deliberately allocation-bounded by the
/// catalog query and entry caps; it is not used from the realtime thread.
#[derive(Debug, Default, Clone, Copy)]
pub struct WRatioRanker;

impl FuzzyRanker for WRatioRanker {
    fn rank(&self, query: &str, search_keys: &[String], score_cutoff: f64) -> Vec<usize> {
        let mut matches = search_keys
            .iter()
            .enumerate()
            .filter_map(|(index, key)| {
                let score = if key.contains(query) {
                    100.0
                } else {
                    wratio_score(query, key)
                };
                (score >= score_cutoff).then_some((index, score))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        matches.into_iter().map(|(index, _)| index).collect()
    }
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

pub fn normalize_search_text(value: &str) -> String {
    let decomposed = value
        .nfkd()
        .filter(|character| !is_combining_mark(*character))
        .map(|character| match character {
            'đ' | 'Đ' => 'd',
            other => other,
        })
        .collect::<String>();
    unicode_casefold(&decomposed)
}

pub fn is_valid_song_id(song_id: &str) -> bool {
    song_id.len() == SONG_ID_LENGTH
        && song_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn unicode_casefold(value: &str) -> String {
    value.case_fold().collect()
}

pub fn song_id_for_canonical_path(path: &str) -> String {
    let digest = Sha256::digest(normalized_canonical_path(path).as_bytes());
    digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Normalize the canonical path supplied by the outer filesystem adapter.
/// The adapter performs strict filesystem canonicalization; this function
/// only applies deterministic separator/case normalization before hashing
/// and collision checks.
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
    if query.chars().count() > MAX_QUERY_LENGTH {
        return Err(CatalogError::QueryTooLong);
    }
    Ok(normalize_search_text(query).trim().to_owned())
}

fn validate_window(query: &str, _offset: usize, limit: usize) -> Result<(), CatalogError> {
    let _ = normalize_query(query)?;
    if !(1..=MAX_PAGE_SIZE).contains(&limit) {
        return Err(CatalogError::InvalidLimit);
    }
    Ok(())
}

/// Return the accepted WRatio-compatible score for two normalized catalog
/// keys. This is a non-realtime search operation and is intentionally kept
/// separate from the realtime/player crates.
pub fn wratio_score(query: &str, candidate: &str) -> f64 {
    if query.is_empty() || candidate.is_empty() {
        return 0.0;
    }
    let query_len = query.chars().count();
    let candidate_len = candidate.chars().count();
    let length_ratio = query_len.max(candidate_len) as f64 / query_len.min(candidate_len) as f64;
    let end_ratio = ratio(query, candidate);
    if length_ratio < 1.5 {
        return end_ratio.max(token_ratio(query, candidate) * 0.95);
    }
    let partial_scale = if length_ratio <= 8.0 { 0.9 } else { 0.6 };
    let partial = end_ratio.max(partial_ratio(query, candidate) * partial_scale);
    partial.max(partial_token_ratio(query, candidate) * 0.95 * partial_scale)
}

fn ratio(left: &str, right: &str) -> f64 {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    if left.is_empty() && right.is_empty() {
        return 100.0;
    }
    indel_ratio(&left, &right)
}

fn partial_ratio(left: &str, right: &str) -> f64 {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    let (short, long) = if left.len() <= right.len() {
        (&left, &right)
    } else {
        (&right, &left)
    };
    if short.is_empty() {
        return 0.0;
    }
    let mut best = partial_ratio_impl(short, long);
    if left.len() == right.len() && best < 100.0 {
        best = best.max(partial_ratio_impl(long, short));
    }
    best
}

/// The bounded short-needle candidate selection uses the same Indel
/// score. The upstream implementation uses a bit-parallel optimization; the
/// direct candidate evaluation keeps the score semantics while staying off
/// the realtime path.
fn partial_ratio_impl(short: &[char], long: &[char]) -> f64 {
    debug_assert!(short.len() <= long.len());
    let mut best: f64 = 0.0;
    for end in 1..short.len() {
        best = best.max(indel_ratio(short, &long[..end]));
    }
    for start in 0..long.len().saturating_sub(short.len()) {
        best = best.max(indel_ratio(short, &long[start..start + short.len()]));
    }
    for start in long.len().saturating_sub(short.len())..long.len() {
        best = best.max(indel_ratio(short, &long[start..]));
    }
    best
}

fn token_sort_ratio(left: &str, right: &str) -> f64 {
    ratio(&sorted_tokens(left), &sorted_tokens(right))
}

fn token_set_ratio(left: &str, right: &str) -> f64 {
    let (left, right) = token_sets(left, right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(&right).cloned().collect::<Vec<_>>();
    let left_rest = left.difference(&right).cloned().collect::<Vec<_>>();
    let right_rest = right.difference(&left).cloned().collect::<Vec<_>>();
    if intersection.is_empty() {
        return ratio(&join_tokens(&left), &join_tokens(&right));
    }
    if left_rest.is_empty() || right_rest.is_empty() {
        return 100.0;
    }
    let sect_len = join_tokens(&intersection).chars().count();
    let left_rest = join_tokens(&left_rest);
    let right_rest = join_tokens(&right_rest);
    let left_len = sect_len + usize::from(sect_len != 0) + left_rest.chars().count();
    let right_len = sect_len + usize::from(sect_len != 0) + right_rest.chars().count();
    let diff_ratio = ratio(&left_rest, &right_rest);
    let left_ratio = 100.0
        - 100.0 * (usize::from(sect_len != 0) + left_rest.chars().count()) as f64
            / (sect_len + left_len) as f64;
    let right_ratio = 100.0
        - 100.0 * (usize::from(sect_len != 0) + right_rest.chars().count()) as f64
            / (sect_len + right_len) as f64;
    diff_ratio.max(left_ratio).max(right_ratio)
}

fn partial_token_sort_ratio(left: &str, right: &str) -> f64 {
    partial_ratio(&sorted_tokens(left), &sorted_tokens(right))
}

fn partial_token_set_ratio(left: &str, right: &str) -> f64 {
    let (left, right) = token_sets(left, right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    if left.intersection(&right).next().is_some() {
        return 100.0;
    }
    let left_rest = left.difference(&right).cloned().collect::<Vec<_>>();
    let right_rest = right.difference(&left).cloned().collect::<Vec<_>>();
    partial_ratio(&join_tokens(left_rest), &join_tokens(right_rest))
}

fn token_ratio(left: &str, right: &str) -> f64 {
    token_set_ratio(left, right).max(token_sort_ratio(left, right))
}

fn partial_token_ratio(left: &str, right: &str) -> f64 {
    let result = partial_token_sort_ratio(left, right);
    let (left_set, right_set) = token_sets(left, right);
    if left_set.is_empty() || right_set.is_empty() {
        return result;
    }
    if left_set.intersection(&right_set).next().is_some() {
        return 100.0;
    }
    let left_difference = left_set.difference(&right_set).cloned().collect::<Vec<_>>();
    let right_difference = right_set.difference(&left_set).cloned().collect::<Vec<_>>();
    let left_token_count = left.split_whitespace().count();
    let right_token_count = right.split_whitespace().count();
    if left_token_count == left_difference.len() && right_token_count == right_difference.len() {
        return result;
    }
    result.max(partial_token_set_ratio(left, right))
}

fn token_sets(left: &str, right: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    (
        left.split_whitespace().map(str::to_owned).collect(),
        right.split_whitespace().map(str::to_owned).collect(),
    )
}

fn join_tokens<T, I>(tokens: I) -> String
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    tokens
        .into_iter()
        .map(|token| token.as_ref().to_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn indel_ratio(left: &[char], right: &[char]) -> f64 {
    let length_sum = left.len() + right.len();
    if length_sum == 0 {
        return 100.0;
    }
    200.0 * lcs_length(left, right) as f64 / length_sum as f64
}

fn lcs_length(left: &[char], right: &[char]) -> usize {
    let mut previous = vec![0usize; right.len() + 1];
    let mut current = vec![0usize; right.len() + 1];
    for left_char in left {
        for (index, right_char) in right.iter().enumerate() {
            current[index + 1] = if left_char == right_char {
                previous[index] + 1
            } else {
                previous[index + 1].max(current[index])
            };
        }
        std::mem::swap(&mut previous, &mut current);
        current.fill(0);
    }
    previous[right.len()]
}

fn sorted_tokens(value: &str) -> String {
    let mut tokens = value.split_whitespace().collect::<Vec<_>>();
    tokens.sort_unstable();
    tokens.join(" ")
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "catalog_perf_tests.rs"]
mod perf_tests;
