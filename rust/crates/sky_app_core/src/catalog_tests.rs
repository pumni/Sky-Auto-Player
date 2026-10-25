use super::{
    CatalogError, CatalogIndex, CatalogSourceEntry, FUZZY_SCORE_CUTOFF, FuzzyRanker,
    MAX_QUERY_LENGTH, StableSongId, WRatioRanker, normalize_search_text, wratio_score,
};

struct NoFuzzy;
impl FuzzyRanker for NoFuzzy {
    fn rank(&self, _query: &str, _keys: &[String], _score_cutoff: f64) -> Vec<usize> {
        Vec::new()
    }
}

fn entry(path: &str, title: &str) -> CatalogSourceEntry {
    CatalogSourceEntry::path_derived(path, title)
}

#[test]
fn stable_identity_survives_resource_path_and_content_changes() {
    let stable_id = StableSongId::new("0123456789abcdef0123456789abcdef").unwrap();
    let mut catalog = CatalogIndex::default();
    let first = catalog
        .replace_entries([CatalogSourceEntry::stable(
            r"C:\install\builtin-songs\sheets\song.json",
            "Song",
            stable_id.clone(),
        )])
        .unwrap();
    let second = catalog
        .replace_entries([CatalogSourceEntry::stable(
            r"D:\different-install\builtin-songs\sheets\renamed.json",
            "Song (corrected)",
            stable_id,
        )])
        .unwrap();
    assert_eq!(first.items[0].song_id, second.items[0].song_id);
    assert_eq!(second.items[0].title, "Song (corrected)");
}

#[test]
fn identity_conflicts_are_rejected_instead_of_deduplicated() {
    let stable_a = StableSongId::new("0123456789abcdef0123456789abcdef").unwrap();
    let stable_b = StableSongId::new("fedcba9876543210fedcba9876543210").unwrap();
    let mut catalog = CatalogIndex::default();
    let same_path = [
        CatalogSourceEntry::stable("C:/songs/a.json", "A", stable_a.clone()),
        CatalogSourceEntry::stable("C:/songs/a.json", "A", stable_b),
    ];
    assert_eq!(
        catalog.replace_entries(same_path),
        Err(CatalogError::IdentityPathConflict)
    );
    let different_path = [
        CatalogSourceEntry::stable("C:/songs/a.json", "A", stable_a),
        CatalogSourceEntry::stable(
            "C:/songs/b.json",
            "B",
            StableSongId::new("0123456789abcdef0123456789abcdef").unwrap(),
        ),
    ];
    assert_eq!(
        catalog.replace_entries(different_path),
        Err(CatalogError::IdCollision)
    );
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
fn wratio_matches_reference_vectors() {
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
