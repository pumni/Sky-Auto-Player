use super::{
    CatalogError, CatalogIndex, CatalogPage, CatalogSourceEntry, FUZZY_SCORE_CUTOFF, FuzzyRanker,
    WRatioRanker, normalize_query, validate_window,
};
use std::collections::BTreeSet;

fn entry(path: &str, title: &str) -> CatalogSourceEntry {
    CatalogSourceEntry::path_derived(path, title)
}

fn measured_catalog(
    size: usize,
) -> (
    CatalogIndex,
    BTreeSet<String>,
    BTreeSet<String>,
    BTreeSet<String>,
) {
    let tags = ["Moon Child", "Star Light", "Dawn River", "Blue Horizon"];
    let sources = (0..size).map(|index| {
        let path = format!("C:/catalog-query-v1/{index:05}.json");
        let title = format!("{} Song {index:05}", tags[index % tags.len()]);
        entry(&path, &title)
    });
    let mut catalog = CatalogIndex::default();
    let snapshot = catalog.replace_entries(sources).expect("benchmark catalog");
    let all = snapshot
        .items
        .iter()
        .map(|row| row.song_id.clone())
        .collect::<BTreeSet<_>>();
    let liked = snapshot
        .items
        .iter()
        .enumerate()
        .filter(|(index, _)| index % 4 == 0)
        .map(|(_, row)| row.song_id.clone())
        .collect();
    let playlist = snapshot
        .items
        .iter()
        .enumerate()
        .filter(|(index, _)| index % 5 < 2)
        .map(|(_, row)| row.song_id.clone())
        .collect();
    (catalog, all, liked, playlist)
}

fn preoptimization_search<R: FuzzyRanker>(
    catalog: &CatalogIndex,
    ranker: &R,
    query: &str,
    offset: usize,
    limit: usize,
    generation: Option<u64>,
    allowed_song_ids: Option<&BTreeSet<String>>,
) -> Result<CatalogPage, CatalogError> {
    catalog.check_generation(generation)?;
    validate_window(query, offset, limit)?;
    let normalized = normalize_query(query)?;
    let candidates = catalog
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| allowed_song_ids.is_none_or(|ids| ids.contains(&entry.row.song_id)))
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
        for candidate_index in ranker.rank(&normalized, &keys, FUZZY_SCORE_CUTOFF) {
            if let Some((index, _)) = candidates.get(candidate_index)
                && !ranked.contains(index)
            {
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
        .map(|index| catalog.entries[index].row.clone())
        .collect();
    Ok(CatalogPage {
        items,
        offset,
        limit,
        total,
        generation: catalog.generation,
    })
}

fn assert_matches_preoptimization_search(
    catalog: &CatalogIndex,
    query: &str,
    allowed_song_ids: Option<&BTreeSet<String>>,
) {
    const PAGE_SIZE: usize = 8;
    let ranker = WRatioRanker;
    let generation = Some(catalog.generation());
    let total = preoptimization_search(
        catalog,
        &ranker,
        query,
        0,
        PAGE_SIZE,
        generation,
        allowed_song_ids,
    )
    .expect("reference probe")
    .total;
    let mut offsets = [
        0,
        total / 2,
        total.saturating_sub(PAGE_SIZE),
        total,
        total.saturating_add(7),
    ]
    .to_vec();
    offsets.sort_unstable();
    offsets.dedup();
    for offset in offsets {
        let expected = preoptimization_search(
            catalog,
            &ranker,
            query,
            offset,
            PAGE_SIZE,
            generation,
            allowed_song_ids,
        )
        .expect("pre-optimization page");
        let actual = catalog
            .search_with_allowed_ids(
                &ranker,
                query,
                offset,
                PAGE_SIZE,
                generation,
                allowed_song_ids,
            )
            .expect("optimized page");
        assert_eq!(actual, expected, "query={query:?}, offset={offset}");
    }
}

fn preoptimization_count_allowed_ids(
    catalog: &CatalogIndex,
    allowed_song_ids: &BTreeSet<String>,
    generation: Option<u64>,
) -> Result<usize, CatalogError> {
    catalog.check_generation(generation)?;
    Ok(catalog
        .entries
        .iter()
        .filter(|entry| allowed_song_ids.contains(&entry.row.song_id))
        .count())
}

#[test]
fn catalog_length_and_generation_match_snapshot_metadata() {
    let empty = CatalogIndex::default();
    assert_eq!((empty.generation(), empty.len()), (0, 0));

    let (catalog, _, _, _) = measured_catalog(100);
    let snapshot = catalog.snapshot();
    assert_eq!(catalog.generation(), snapshot.generation);
    assert_eq!(catalog.len(), snapshot.total);
}

#[test]
fn short_query_pagination_matches_preoptimization_at_high_cardinality() {
    for size in [100, 1_000, 10_000] {
        let (catalog, all, liked, playlist) = measured_catalog(size);
        for allowed_ids in [None, Some(&all), Some(&liked), Some(&playlist)] {
            for query in ["", "O"] {
                assert_matches_preoptimization_search(&catalog, query, allowed_ids);
            }
        }
    }

    let (catalog, all, liked, playlist) = measured_catalog(1_000);
    for allowed_ids in [None, Some(&all), Some(&liked), Some(&playlist)] {
        assert_matches_preoptimization_search(&catalog, "moon child", allowed_ids);
    }
    let mut liked_with_unknown = liked;
    liked_with_unknown.insert("not-a-catalog-song-id".into());
    for query in ["", "O"] {
        assert_matches_preoptimization_search(&catalog, query, Some(&liked_with_unknown));
    }
}

#[test]
fn cached_search_orders_match_the_wratio_reference_across_scopes_and_pages() {
    const PAGE_SIZE: usize = 200;
    let ranker = WRatioRanker;
    for size in [0, 1, 96, 100, 101, 199, 200, 201, 1_000, 10_000] {
        let (catalog, all, liked, playlist) = measured_catalog(size);
        for allowed_ids in [None, Some(&all), Some(&liked), Some(&playlist)] {
            for query in ["", "o", "  Moon Child  ", "Dawn R"] {
                let generation = Some(catalog.generation());
                let expected_total = preoptimization_search(
                    &catalog,
                    &ranker,
                    query,
                    0,
                    PAGE_SIZE,
                    generation,
                    allowed_ids,
                )
                .expect("reference search")
                .total;
                let order = catalog
                    .search_order_with_allowed_ids(&ranker, query, generation, allowed_ids)
                    .expect("exact search order");
                assert_eq!(order.index_count(), expected_total);
                for offset in [
                    0,
                    expected_total / 2,
                    expected_total.saturating_sub(PAGE_SIZE),
                    expected_total,
                    expected_total.saturating_add(1),
                ] {
                    let expected = preoptimization_search(
                        &catalog,
                        &ranker,
                        query,
                        offset,
                        PAGE_SIZE,
                        generation,
                        allowed_ids,
                    )
                    .expect("reference page");
                    let actual = catalog
                        .page_from_search_order(&order, query, offset, PAGE_SIZE, generation)
                        .expect("cached page");
                    assert_eq!(
                        actual, expected,
                        "size={size}, query={query:?}, offset={offset}"
                    );
                }
            }
        }
    }
}

#[test]
fn allowed_song_counts_match_preoptimization_for_sparse_and_dense_sets() {
    for size in [100, 1_000, 10_000] {
        let (catalog, all, liked, playlist) = measured_catalog(size);
        for allowed_ids in [&all, &liked, &playlist] {
            assert_eq!(
                catalog.count_allowed_ids(allowed_ids, Some(catalog.generation())),
                preoptimization_count_allowed_ids(
                    &catalog,
                    allowed_ids,
                    Some(catalog.generation())
                )
            );
        }
        let unknown = BTreeSet::from(["not-a-catalog-song-id".to_owned()]);
        assert_eq!(
            catalog.count_allowed_ids(&unknown, Some(catalog.generation())),
            Ok(0)
        );
    }
}

fn median_microseconds(mut operation: impl FnMut()) -> f64 {
    use std::time::{Duration, Instant};

    const SAMPLES: usize = 7;
    const SAMPLE_DURATION: Duration = Duration::from_millis(20);

    operation();
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let mut iterations = 0_u64;
        while started.elapsed() < SAMPLE_DURATION {
            operation();
            iterations += 1;
        }
        samples.push(started.elapsed().as_secs_f64() * 1_000_000.0 / iterations as f64);
    }
    samples.sort_by(f64::total_cmp);
    samples[SAMPLES / 2]
}

#[derive(Debug)]
struct TimingDistribution {
    min_us: f64,
    p50_us: f64,
    p90_us: f64,
    max_us: f64,
}

fn timing_distribution(mut operation: impl FnMut()) -> TimingDistribution {
    use std::time::{Duration, Instant};

    const SAMPLES: usize = 9;
    const SAMPLE_DURATION: Duration = Duration::from_millis(25);

    operation();
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let mut iterations = 0_u64;
        while started.elapsed() < SAMPLE_DURATION {
            operation();
            iterations += 1;
        }
        samples.push(started.elapsed().as_secs_f64() * 1_000_000.0 / iterations as f64);
    }
    samples.sort_by(f64::total_cmp);
    TimingDistribution {
        min_us: samples[0],
        p50_us: samples[samples.len() / 2],
        p90_us: samples[(samples.len() * 9 / 10).min(samples.len() - 1)],
        max_us: samples[samples.len() - 1],
    }
}

/// Reproducible Package B query benchmark. Run before and after the catalog
/// changes with the same pinned Rust toolchain and release profile:
/// `cargo test --manifest-path rust/Cargo.toml -p sky_app_core --release --locked
/// opt_in_package_b_catalog_query_benchmark -- --ignored --nocapture`.
/// Fixtures, scopes, queries, and page offsets are deterministic; timing is
/// diagnostic output only and is never asserted.
#[test]
#[ignore = "opt-in Package B catalog query benchmark"]
fn opt_in_package_b_catalog_query_benchmark() {
    const PAGE_SIZE: usize = 200;
    let ranker = WRatioRanker;
    println!(
        "package_b_bench layer=catalog-index fixture=package-b-catalog-fixture-v1 seed=0x560 size_set=96,1000,10000 profile=release warmup=1 sample_ms=25 samples=9"
    );

    for size in [96, 1_000, 10_000] {
        let (catalog, _all, liked, playlist) = measured_catalog(size);
        let scopes = [
            ("all", None),
            ("liked", Some(&liked)),
            ("playlist", Some(&playlist)),
        ];
        for (scope_name, allowed_ids) in scopes {
            for (query_name, query) in [
                ("empty", ""),
                ("one-char", "o"),
                ("fuzzy-normalized", "  Moon Child  "),
            ] {
                let generation = Some(catalog.generation());
                let total = catalog
                    .search_with_allowed_ids(&ranker, query, 0, PAGE_SIZE, generation, allowed_ids)
                    .expect("benchmark probe")
                    .total;
                let windows = [
                    ("first", 0),
                    ("middle", total / 2),
                    ("late", total.saturating_sub(PAGE_SIZE)),
                ];
                for (window_name, offset) in windows {
                    let timing = timing_distribution(|| {
                        std::hint::black_box(
                            catalog
                                .search_with_allowed_ids(
                                    &ranker,
                                    query,
                                    offset,
                                    PAGE_SIZE,
                                    generation,
                                    allowed_ids,
                                )
                                .expect("benchmark search"),
                        );
                    });
                    println!(
                        "package_b_query size={size} scope={scope_name} query={query_name} window={window_name} offset={offset} limit={PAGE_SIZE} p50_us={:.3} p90_us={:.3} min_us={:.3} max_us={:.3}",
                        timing.p50_us, timing.p90_us, timing.min_us, timing.max_us
                    );
                }
            }
        }
    }
}

fn bench_viewport_refresh(index: &CatalogIndex, first: usize, width: usize) {
    let generation = index.generation();
    let total = index.len();
    let last = (first + width - 1).min(total - 1);
    let ids = index
        .song_ids_in_range(first, last, Some(generation))
        .expect("viewport IDs");
    std::hint::black_box((total, ids));
}

fn bench_preoptimization_viewport_refresh(index: &CatalogIndex, first: usize, width: usize) {
    let snapshot = index.snapshot();
    let last = (first + width - 1).min(snapshot.total - 1);
    let ids = index
        .song_ids_in_range(first, last, Some(snapshot.generation))
        .expect("legacy viewport IDs");
    std::hint::black_box((snapshot.total, ids));
}

/// Run with `cargo test --manifest-path rust/Cargo.toml -p sky_app_core
/// --release --locked opt_in_catalog_performance_measurement -- --ignored
/// --nocapture`. This measures pure `CatalogIndex` query/count and index-range
/// operations only; it does not include native IPC readiness, DTO projection,
/// file parsing, or analysis-cache hydration. Native warmed-route behavior is
/// covered by the desktop fixture tests. Reports medians only; there are no
/// timing assertions.
#[test]
#[ignore = "opt-in bounded catalog performance measurement"]
fn opt_in_catalog_performance_measurement() {
    const PAGE_SIZE: usize = 16;
    const VIEWPORT_WIDTH: usize = 48;
    let ranker = WRatioRanker;

    for size in [100, 1_000, 10_000] {
        let (catalog, all, liked, playlist) = measured_catalog(size);
        println!(
            "catalog_perf_fixture id=catalog-query-v1 layer=catalog-index size={size} all={} liked={} playlist={} profile=release",
            all.len(),
            liked.len(),
            playlist.len()
        );
        let workloads: [(&str, &str, Option<&BTreeSet<String>>); 8] = [
            ("all-empty-filtered", "", Some(&all)),
            ("all-empty", "", None),
            ("liked-empty", "", Some(&liked)),
            ("playlist-empty", "", Some(&playlist)),
            ("all-one-char-filtered", "o", Some(&all)),
            ("all-one-char", "o", None),
            ("all-fuzzy-filtered", "moon child", Some(&all)),
            ("all-fuzzy", "moon child", None),
        ];

        for (name, query, allowed_ids) in workloads {
            let total = preoptimization_search(
                &catalog,
                &ranker,
                query,
                0,
                PAGE_SIZE,
                Some(catalog.generation()),
                allowed_ids,
            )
            .expect("benchmark search probe")
            .total;
            let mut offsets = [0, total / 2, total.saturating_sub(PAGE_SIZE)].to_vec();
            offsets.sort_unstable();
            offsets.dedup();
            for offset in offsets {
                let position = if offset == 0 {
                    "first"
                } else if offset == total.saturating_sub(PAGE_SIZE) {
                    "late"
                } else {
                    "middle"
                };
                let generation = Some(catalog.generation());
                let legacy = median_microseconds(|| {
                    std::hint::black_box(
                        preoptimization_search(
                            &catalog,
                            &ranker,
                            query,
                            offset,
                            PAGE_SIZE,
                            generation,
                            allowed_ids,
                        )
                        .expect("legacy benchmark search"),
                    );
                });
                let current = median_microseconds(|| {
                    std::hint::black_box(
                        catalog
                            .search_with_allowed_ids(
                                &ranker,
                                query,
                                offset,
                                PAGE_SIZE,
                                generation,
                                allowed_ids,
                            )
                            .expect("benchmark search"),
                    );
                });
                println!(
                    "catalog_perf size={size} workload={name} page={position} offset={offset} limit={PAGE_SIZE} warmup=1 sample_ms=20 samples=7 legacy_median_us={legacy:.3} current_median_us={current:.3}"
                );
            }
        }

        for (name, allowed_ids) in [("all-total", &all), ("liked-total", &liked)] {
            let generation = Some(catalog.generation());
            let legacy = median_microseconds(|| {
                std::hint::black_box(
                    preoptimization_count_allowed_ids(&catalog, allowed_ids, generation)
                        .expect("legacy benchmark count"),
                );
            });
            let current = median_microseconds(|| {
                std::hint::black_box(
                    catalog
                        .count_allowed_ids(allowed_ids, generation)
                        .expect("benchmark count"),
                );
            });
            println!(
                "catalog_perf size={size} workload={name} warmup=1 sample_ms=20 samples=7 legacy_median_us={legacy:.3} current_median_us={current:.3}"
            );
        }

        let max_first = size.saturating_sub(VIEWPORT_WIDTH);
        let mut viewport_offsets = [0, max_first / 2, max_first].to_vec();
        viewport_offsets.sort_unstable();
        viewport_offsets.dedup();
        for first in viewport_offsets {
            let position = if first == 0 {
                "first"
            } else if first == max_first {
                "late"
            } else {
                "middle"
            };
            let legacy = median_microseconds(|| {
                bench_preoptimization_viewport_refresh(&catalog, first, VIEWPORT_WIDTH);
            });
            let current = median_microseconds(|| {
                bench_viewport_refresh(&catalog, first, VIEWPORT_WIDTH);
            });
            println!(
                "catalog_perf size={size} workload=index-viewport-refresh page={position} first={first} width={VIEWPORT_WIDTH} warmup=1 sample_ms=20 samples=7 legacy_median_us={legacy:.3} current_median_us={current:.3}"
            );
        }
    }
}
