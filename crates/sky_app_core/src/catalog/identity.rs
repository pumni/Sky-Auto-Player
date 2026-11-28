use super::{CatalogError, MAX_PAGE_SIZE, MAX_QUERY_LENGTH, SONG_ID_LENGTH, SUPPORTED_EXTENSIONS};
use sha2::{Digest, Sha256};
use std::path::Path;
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

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

pub(super) fn unicode_casefold(value: &str) -> String {
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

pub(super) fn is_supported_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

pub(super) fn normalize_query(query: &str) -> Result<String, CatalogError> {
    if query.chars().count() > MAX_QUERY_LENGTH {
        return Err(CatalogError::QueryTooLong);
    }
    Ok(normalize_search_text(query).trim().to_owned())
}

pub(super) fn validate_window(
    query: &str,
    _offset: usize,
    limit: usize,
) -> Result<(), CatalogError> {
    let _ = normalize_query(query)?;
    if !(1..=MAX_PAGE_SIZE).contains(&limit) {
        return Err(CatalogError::InvalidLimit);
    }
    Ok(())
}
