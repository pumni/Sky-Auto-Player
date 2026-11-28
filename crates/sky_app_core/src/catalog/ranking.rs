use super::FuzzyRanker;
use std::collections::BTreeSet;

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
