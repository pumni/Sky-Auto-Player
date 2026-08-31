//! Pure catalog index and generation semantics.
//!
//! Directory enumeration and canonicalization are outer-adapter concerns. The
//! core receives canonical path strings, owns opaque IDs, stable ordering,
//! generation checks, and bounded path-free rows. Fuzzy ranking is an explicit
//! port because the current Python implementation uses RapidFuzz WRatio; the
//! core must not silently substitute a different algorithm.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use unicode_casefold::UnicodeCaseFold;
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
    #[error("malformed or unknown song ID")]
    UnknownSongId,
    #[error("fuzzy ranking is not available for this shadow index")]
    FuzzyRankingUnavailable,
}

pub trait FuzzyRanker {
    fn rank(&self, query: &str, search_keys: &[String], score_cutoff: f64) -> Vec<usize>;
}

/// Bounded WRatio-compatible ranker for the native index.
///
/// RapidFuzz's WRatio is a composition of normalized Levenshtein, partial,
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
            let normalized_path = normalized_canonical_path(&source.canonical_path);
            if seen_paths.insert(normalized_path.clone(), ()).is_some() {
                continue;
            }
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

/// Return the RapidFuzz-compatible WRatio score for two normalized catalog
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

/// RapidFuzz's bounded short-needle candidate selection using the same Indel
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

    #[test]
    fn normalization_uses_unicode_casefold_not_only_lowercase() {
        assert_eq!(normalize_search_text("Straße"), "strasse");
        assert_eq!(normalize_search_text("ΟΣ"), normalize_search_text("οσ"));
        assert_eq!(normalize_search_text("ĐÀN"), "dan");
    }

    #[test]
    fn query_bounds_count_unicode_scalars_and_allow_large_offsets() {
        let mut catalog = CatalogIndex::default();
        catalog
            .replace_entries([entry("C:/songs/a.json", "Alpha")])
            .expect("index");
        let valid = "é".repeat(MAX_QUERY_LENGTH);
        assert!(catalog.search_substrings(&valid, 0, 10, Some(1)).is_ok());
        let invalid = "é".repeat(MAX_QUERY_LENGTH + 1);
        assert_eq!(
            catalog
                .search_substrings(&invalid, 0, 10, Some(1))
                .unwrap_err(),
            CatalogError::QueryTooLong
        );
        assert!(
            catalog
                .search_substrings("", 1_000_000_001, 10, Some(1))
                .is_ok()
        );
    }

    #[test]
    fn wratio_ranker_preserves_stable_ties_after_cutoff() {
        let ranker = WRatioRanker;
        let keys = vec!["sky child".into(), "sky child".into(), "unrelated".into()];
        assert_eq!(
            ranker.rank("sky child", &keys, FUZZY_SCORE_CUTOFF),
            vec![0, 1]
        );
    }

    #[test]
    fn wratio_matches_rapidfuzz_reference_vectors() {
        let cases = [
            (("this is a test", "this is a test!"), 96.55172413793103),
            (("fuzzy was a bear", "fuzzy fuzzy was a bear"), 95.0),
            (
                (
                    "fuzzy was a bear but not a dog",
                    "fuzzy was a bear but not a cat",
                ),
                90.0,
            ),
            (("abcd", "xxabceyy"), 67.5),
            (("strasse", "straße"), 76.92307692307692),
            (("sky child", "sky children of the light"), 90.0),
            (("alpha beta", "beta alpha"), 95.0),
            (("abc", "xyzabcq"), 90.0),
            (("a b c", "a a b c"), 95.0),
            (("xabcdy", "abcd"), 90.0),
            (("abc", "zab"), 66.66666666666667),
            (("abc", "qabc"), 85.71428571428572),
            (("a b", "x a b y"), 90.0),
            (("foo bar", "foo baz qux"), 85.5),
            (("testing", "test"), 90.0),
            (("aa bb aa", "bb aa"), 90.0),
            (("a", "bbbbba"), 90.0),
        ];
        for ((left, right), expected) in cases {
            let actual = wratio_score(left, right);
            assert!(
                (actual - expected).abs() < 1e-9,
                "{left:?} vs {right:?}: expected {expected}, got {actual}"
            );
        }
    }
}
