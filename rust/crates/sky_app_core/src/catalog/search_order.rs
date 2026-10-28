use super::{
    CatalogError, CatalogIndex, CatalogPage, CatalogSearchOrder, FUZZY_SCORE_CUTOFF, FuzzyRanker,
    normalize_query, validate_window,
};
use std::collections::BTreeSet;

impl CatalogIndex {
    /// Validate a requested window before an adapter spends time constructing
    /// or looking up a reusable result order.
    pub fn validate_search_window(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
        generation: Option<u64>,
    ) -> Result<(), CatalogError> {
        self.check_generation(generation)?;
        validate_window(query, offset, limit)
    }

    /// Whether a query enters the fuzzy ranker after the canonical search
    /// normalization. Short queries retain the direct substring/paging path.
    pub fn is_fuzzy_search_query(query: &str) -> Result<bool, CatalogError> {
        Ok(normalize_query(query)?.chars().count() > 1)
    }

    /// Build the exact full result order for page reuse. Results retain the
    /// same source membership, fuzzy cutoff, and ranker order as
    /// `search_with_allowed_ids`; duplicate ranker indices are ignored in
    /// first-seen order using a bounded bitmap.
    pub fn search_order_with_allowed_ids<R: FuzzyRanker>(
        &self,
        ranker: &R,
        query: &str,
        generation: Option<u64>,
        allowed_song_ids: Option<&BTreeSet<String>>,
    ) -> Result<CatalogSearchOrder, CatalogError> {
        self.check_generation(generation)?;
        let normalized_query = normalize_query(query)?;
        let candidates = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                allowed_song_ids.is_none_or(|ids| ids.contains(&entry.row.song_id))
            })
            .collect::<Vec<_>>();
        let indices = if normalized_query.chars().count() <= 1 {
            candidates
                .iter()
                .filter_map(|(index, entry)| {
                    (normalized_query.is_empty() || entry.search_key.contains(&normalized_query))
                        .then_some(*index)
                })
                .collect()
        } else {
            let keys = candidates
                .iter()
                .map(|(_, entry)| entry.search_key.clone())
                .collect::<Vec<_>>();
            let mut seen = vec![false; candidates.len()];
            let mut ranked = Vec::with_capacity(candidates.len());
            for candidate_index in ranker.rank(&normalized_query, &keys, FUZZY_SCORE_CUTOFF) {
                if let Some((index, _)) = candidates.get(candidate_index)
                    && !seen[candidate_index]
                {
                    seen[candidate_index] = true;
                    ranked.push(*index);
                }
            }
            ranked
        };
        Ok(CatalogSearchOrder {
            generation: self.generation,
            normalized_query,
            indices,
        })
    }

    /// Project one page from an exact search order. Query and generation
    /// checks make it safe to reject stale or accidentally cross-key reuse.
    pub fn page_from_search_order(
        &self,
        order: &CatalogSearchOrder,
        query: &str,
        offset: usize,
        limit: usize,
        generation: Option<u64>,
    ) -> Result<CatalogPage, CatalogError> {
        self.check_generation(generation)?;
        validate_window(query, offset, limit)?;
        if order.generation != self.generation {
            return Err(CatalogError::StaleGeneration);
        }
        let normalized = normalize_query(query)?;
        if normalized != order.normalized_query {
            return Err(CatalogError::SearchOrderMismatch);
        }
        let total = order.indices.len();
        let items = order
            .indices
            .iter()
            .skip(offset)
            .take(limit)
            .map(|index| self.entries[*index].row.clone())
            .collect();
        Ok(CatalogPage {
            items,
            offset,
            limit,
            total,
            generation: self.generation,
        })
    }
}
