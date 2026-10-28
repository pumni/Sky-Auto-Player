use super::*;
use std::path::PathBuf;

fn package_b_native_fixture(
    label: &str,
    size: usize,
) -> (
    PathBuf,
    std::sync::Arc<NativeDesktopRuntime>,
    u64,
    Vec<String>,
) {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-package-b-{label}-{size}-{suffix}"));
    let songs_root = root.join("songs");
    fs::create_dir_all(&songs_root).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    for index in 0..size {
        fs::write(
            songs_root.join(format!("song-{index:05}.json")),
            format!(
                "{{\"name\":\"Bench Song {index:05}\",\"songNotes\":[{{\"time\":0,\"key\":\"Key0\"}}]}}"
            ),
        )
        .expect("fixture song");
    }
    let runtime = std::sync::Arc::new(
        NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime"),
    );
    let generation = catalog_generation(&runtime);
    let ids = runtime
        .catalog
        .lock()
        .expect("catalog lock")
        .index
        .snapshot()
        .items
        .into_iter()
        .map(|row| row.song_id)
        .collect();
    (root, runtime, generation, ids)
}

fn package_b_viewport(
    generation: u64,
    index: usize,
    selected_song_id: Option<String>,
    song_ids: Vec<String>,
) -> crate::commands::CatalogViewportRequest {
    crate::commands::CatalogViewportRequest {
        generation,
        first_index: index as u64,
        last_index: index as i64,
        selected_song_id,
        song_ids,
    }
}

fn begin_paused_viewport(
    runtime: &std::sync::Arc<NativeDesktopRuntime>,
    request: crate::commands::CatalogViewportRequest,
) -> (
    std::thread::JoinHandle<Result<crate::commands::CatalogViewportDto, String>>,
    std::sync::Arc<std::sync::Barrier>,
) {
    let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
    let release = std::sync::Arc::new(std::sync::Barrier::new(2));
    runtime.set_analysis_test_gate(entered.clone(), release.clone(), 1);
    let runtime = std::sync::Arc::clone(runtime);
    let worker = std::thread::spawn(move || runtime.set_viewport(request));
    entered.wait();
    (worker, release)
}

fn wait_for_analysis_flight_waiters(
    runtime: &NativeDesktopRuntime,
    song_id: &str,
    generation: u64,
    settings_revision: u64,
    expected: usize,
) {
    let key = super::super::AnalysisKey {
        song_id: song_id.to_owned(),
        generation,
        settings_revision,
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let joined = runtime
            .analysis_cache
            .lock()
            .expect("analysis cache lock")
            .flights
            .get(&key)
            .is_some_and(|flight| flight.waiters.load(Ordering::Acquire) >= expected);
        if joined {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "analysis waiter did not join in time"
        );
        std::thread::yield_now();
    }
}

#[test]
#[ignore = "opt-in Package B native viewport benchmark"]
fn opt_in_package_b_native_viewport_benchmark() {
    use std::time::{Duration, Instant};

    fn percentile(values: &mut [f64], fraction: f64) -> f64 {
        values.sort_by(f64::total_cmp);
        let index = ((values.len() - 1) as f64 * fraction).ceil() as usize;
        values[index]
    }

    fn print_distribution(size: usize, window: &str, mode: &str, values: &mut [f64]) {
        let min = values.iter().copied().fold(f64::INFINITY, f64::min);
        let max = values.iter().copied().fold(0.0, f64::max);
        let p50 = percentile(values, 0.50);
        let p90 = percentile(values, 0.90);
        println!(
            "package_b_viewport size={size} window={window} mode={mode} samples={} p50_us={p50:.3} p90_us={p90:.3} min_us={min:.3} max_us={max:.3}",
            values.len()
        );
    }

    println!(
        "package_b_bench layer=native-viewport fixture=package-b-native-files-v1 seed=0x560 size_set=96,1000,10000 profile=release warmup=1 cold_samples=7 warm_sample_ms=25"
    );
    for size in [96_usize, 1_000, 10_000] {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-package-b-viewport-{size}-{suffix}"));
        let songs_root = root.join("songs");
        fs::create_dir_all(&songs_root).expect("songs root");
        fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
        for index in 0..size {
            fs::write(
                songs_root.join(format!("song-{index:05}.json")),
                format!(
                    "{{\"name\":\"Bench Song {index:05}\",\"songNotes\":[{{\"time\":0,\"key\":\"Key0\"}}]}}"
                ),
            )
            .expect("benchmark song");
        }

        let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
        let generation = catalog_generation(&runtime);
        let width = size.min(200);
        let last_start = size.saturating_sub(width);
        for (window_name, first) in [
            ("first", 0),
            ("middle", last_start / 2),
            ("late", last_start),
        ] {
            let last = first + width - 1;
            let request = serde_json::json!({
                "generation": generation,
                "firstIndex": first,
                "lastIndex": last,
                "selectedSongId": null,
                "songIds": []
            });
            let run_viewport = || {
                runtime
                    .dispatch("catalog.set_viewport", request.clone())
                    .expect("viewport benchmark")
            };

            let warmup = run_viewport();
            assert_eq!(warmup["items"].as_array().map(Vec::len), Some(width));
            let mut cold = Vec::with_capacity(7);
            for _ in 0..7 {
                runtime.invalidate_analysis_cache();
                let started = Instant::now();
                std::hint::black_box(run_viewport());
                cold.push(started.elapsed().as_secs_f64() * 1_000_000.0);
            }
            print_distribution(size, window_name, "cold", &mut cold);

            std::hint::black_box(run_viewport());
            let mut warm = Vec::with_capacity(9);
            for _ in 0..9 {
                let started = Instant::now();
                let mut iterations = 0_u64;
                while started.elapsed() < Duration::from_millis(25) {
                    std::hint::black_box(run_viewport());
                    iterations += 1;
                }
                warm.push(started.elapsed().as_secs_f64() * 1_000_000.0 / iterations as f64);
            }
            print_distribution(size, window_name, "warm-repeated", &mut warm);
        }
        runtime.shutdown();
        fs::remove_dir_all(&root).expect("remove benchmark fixture");
    }
}

#[test]
fn native_catalog_search_order_cache_is_bounded_lru_and_source_invalidated() {
    let (root, runtime, generation, ids) = package_b_native_fixture("search-cache", 96);
    for index in 0..9 {
        runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({
                    "query": format!("Bench Song {index:02}"),
                    "offset": index * 2,
                    "limit": 200,
                    "generation": generation,
                    "source": {"kind":"smart","id":"all"}
                }),
            )
            .expect("all source fuzzy page");
    }
    {
        let cache = runtime
            .catalog_search_cache
            .lock()
            .expect("catalog search cache");
        assert_eq!(cache.entries.len(), 8);
        assert!(cache.cached_indices <= 100_000);
        assert!(
            !cache
                .entries
                .iter()
                .any(|entry| entry.key.query == "Bench Song 00")
        );
        assert!(
            cache
                .entries
                .iter()
                .any(|entry| entry.key.query == "Bench Song 08")
        );
    }
    assert_eq!(
        runtime
            .catalog_search_order_test_computations
            .load(Ordering::Acquire),
        9
    );
    let first_page = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query":"song",
                "offset":0,
                "limit":20,
                "generation":generation,
                "source":{"kind":"smart","id":"all"}
            }),
        )
        .expect("first reusable fuzzy page");
    let second_page = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query":"song",
                "offset":20,
                "limit":20,
                "generation":generation,
                "source":{"kind":"smart","id":"all"}
            }),
        )
        .expect("second reusable fuzzy page");
    assert_eq!(first_page["total"], 96);
    assert_eq!(first_page["items"].as_array().map(Vec::len), Some(20));
    assert_eq!(second_page["items"].as_array().map(Vec::len), Some(20));
    let first_ids = first_page["items"]
        .as_array()
        .expect("first page rows")
        .iter()
        .filter_map(|row| row["song_id"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        second_page["items"]
            .as_array()
            .expect("second page rows")
            .iter()
            .all(|row| !first_ids.contains(row["song_id"].as_str().expect("song ID")))
    );
    assert_eq!(
        runtime
            .catalog_search_order_test_computations
            .load(Ordering::Acquire),
        10,
        "different pages of one normalized query must reuse one exact result order"
    );

    let liked_query = "song";
    let liked_search = |offset| {
        runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({
                    "query": liked_query,
                    "offset": offset,
                    "limit": 20,
                    "generation": generation,
                    "source": {"kind":"smart","id":"liked"}
                }),
            )
            .expect("liked page")
    };
    let first_liked = liked_search(0);
    assert_eq!(first_liked["total"], 0);
    let later_liked = liked_search(20);
    assert_eq!(later_liked["items"].as_array().map(Vec::len), Some(0));
    assert!(
        runtime
            .catalog_search_cache
            .lock()
            .expect("search cache")
            .entries
            .iter()
            .any(|entry| matches!(entry.key.scope, super::super::CatalogSearchScope::Liked))
    );
    runtime
        .dispatch(
            "catalog.set_liked",
            serde_json::json!({"songId":ids[0],"liked":true,"generation":generation}),
        )
        .expect("like membership change");
    {
        let cache = runtime
            .catalog_search_cache
            .lock()
            .expect("catalog search cache");
        assert!(
            !cache.entries.iter().any(|entry| {
                matches!(entry.key.scope, super::super::CatalogSearchScope::Liked)
            })
        );
        assert!(cache.entries.iter().all(|entry| {
            !matches!(entry.key.scope, super::super::CatalogSearchScope::All)
                || entry.key.generation == generation
        }));
    }

    let playlist = runtime
        .dispatch(
            "library.create_playlist",
            serde_json::json!({"name":"Cache test"}),
        )
        .expect("create playlist");
    let playlist_id = playlist["id"].as_str().expect("playlist ID").to_owned();
    runtime
        .dispatch(
            "library.add_songs",
            serde_json::json!({"playlistId":playlist_id,"songIds":[ids[0]]}),
        )
        .expect("add first song");
    let playlist_search = |offset| {
        runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({
                    "query": liked_query,
                    "offset": offset,
                    "limit": 20,
                    "generation": generation,
                    "source": {"kind":"playlist","id":playlist_id}
                }),
            )
            .expect("playlist page")
    };
    assert_eq!(playlist_search(0)["total"], 1);
    assert_eq!(
        playlist_search(1)["items"].as_array().map(Vec::len),
        Some(0)
    );
    runtime
        .dispatch(
            "library.add_songs",
            serde_json::json!({"playlistId":playlist_id,"songIds":[ids[1]]}),
        )
        .expect("playlist membership change");
    assert!(
        !runtime
            .catalog_search_cache
            .lock()
            .expect("search cache")
            .entries
            .iter()
            .any(|entry| matches!(
                entry.key.scope,
                super::super::CatalogSearchScope::Playlist(_)
            ))
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_catalog_search_order_cache_caps_total_indices_at_one_hundred_thousand() {
    let mut catalog = super::super::CatalogIndex::default();
    let snapshot = catalog
        .replace_entries((0..20_000).map(|index| {
            sky_app_core::catalog::CatalogSourceEntry::path_derived(
                format!("C:/package-b-index-cap/{index:05}.json"),
                format!("Index Cap Song {index:05}"),
            )
        }))
        .expect("large synthetic index");
    let order = catalog
        .search_order_with_allowed_ids(
            &sky_app_core::catalog::WRatioRanker,
            "",
            Some(snapshot.generation),
            None,
        )
        .expect("empty exact order");
    assert_eq!(order.index_count(), 20_000);

    let mut cache = super::super::CatalogSearchCache::default();
    for revision in 0..6 {
        cache.insert(
            super::super::CatalogSearchKey {
                generation: snapshot.generation,
                scope: super::super::CatalogSearchScope::All,
                membership_revision: revision,
                query: String::new(),
            },
            order.clone(),
        );
        assert!(cache.cached_indices <= 100_000);
    }
    assert_eq!(cache.entries.len(), 5);
    assert_eq!(cache.cached_indices, 100_000);
    assert!(
        !cache
            .entries
            .iter()
            .any(|entry| entry.key.membership_revision == 0)
    );
}

#[test]
fn native_analysis_cache_uses_a_128_entry_lru() {
    let mut cache = super::super::AnalysisCacheState::default();
    for index in 0..128 {
        cache.insert(
            super::super::AnalysisKey {
                song_id: format!("{index:032x}"),
                generation: 1,
                settings_revision: 0,
            },
            super::super::CachedSongAnalysis {
                metadata: super::super::CatalogMetadata {
                    format_label: "JSON".into(),
                    duration_us: Some(index as u64),
                    note_count: Some(1),
                    risk_level: "low".into(),
                    state: "ready",
                },
                detail: None,
            },
        );
    }
    let retained = super::super::AnalysisKey {
        song_id: format!("{:032x}", 0),
        generation: 1,
        settings_revision: 0,
    };
    assert!(cache.get(&retained).is_some());
    for index in 128..140 {
        cache.insert(
            super::super::AnalysisKey {
                song_id: format!("{index:032x}"),
                generation: 1,
                settings_revision: 0,
            },
            super::super::CachedSongAnalysis {
                metadata: super::super::CatalogMetadata::error("JSON".into()),
                detail: None,
            },
        );
    }
    assert_eq!(cache.entries.len(), 128);
    assert!(cache.entries.contains_key(&retained));
    assert!(!cache.entries.contains_key(&super::super::AnalysisKey {
        song_id: format!("{:032x}", 1),
        generation: 1,
        settings_revision: 0,
    }));
}

#[test]
fn native_analysis_cache_recomputes_safely_after_lru_eviction() {
    let (root, runtime, generation, ids) = package_b_native_fixture("analysis-eviction", 129);
    let detail = |song_id: &str| {
        runtime
            .detail(crate::commands::CatalogDetailRequest {
                song_id: song_id.to_owned(),
                generation: Some(generation),
            })
            .expect("detail analysis")
    };
    for song_id in ids.iter().take(128) {
        detail(song_id);
    }
    detail(&ids[0]); // Keep the first entry hot; the second becomes least recent.
    detail(&ids[128]);

    let evicted_key = super::super::AnalysisKey {
        song_id: ids[1].clone(),
        generation,
        settings_revision: runtime.analysis_revision.load(Ordering::Acquire),
    };
    assert_eq!(runtime.analysis_test_runs.load(Ordering::Acquire), 129);
    assert_eq!(
        runtime
            .analysis_cache
            .lock()
            .expect("analysis cache")
            .entries
            .len(),
        128
    );
    assert!(
        !runtime
            .analysis_cache
            .lock()
            .expect("analysis cache")
            .entries
            .contains_key(&evicted_key)
    );

    detail(&ids[1]);
    assert_eq!(
        runtime.analysis_test_runs.load(Ordering::Acquire),
        130,
        "requesting an evicted analysis must recompute and repopulate it"
    );
    assert!(
        runtime
            .analysis_cache
            .lock()
            .expect("analysis cache")
            .entries
            .contains_key(&evicted_key)
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_analysis_admission_caps_two_active_workers_and_sixty_four_queued_ids() {
    const REQUESTS: usize = 67;
    let (root, runtime, generation, ids) = package_b_native_fixture("analysis-queue", REQUESTS);
    let entered = std::sync::Arc::new(std::sync::Barrier::new(3));
    let release = std::sync::Arc::new(std::sync::Barrier::new(3));
    runtime.set_analysis_test_gate(entered.clone(), release.clone(), 2);
    let handles = ids
        .iter()
        .cloned()
        .map(|song_id| {
            let runtime = std::sync::Arc::clone(&runtime);
            std::thread::spawn(move || {
                runtime.detail(crate::commands::CatalogDetailRequest {
                    song_id,
                    generation: Some(generation),
                })
            })
        })
        .collect::<Vec<_>>();

    entered.wait();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let admission = runtime
            .analysis_admission
            .lock()
            .expect("analysis admission lock");
        let queue_rejections = runtime
            .analysis_test_queue_rejections
            .load(Ordering::Acquire);
        if admission.queued == super::super::MAX_QUEUED_SONG_ANALYSES && queue_rejections == 1 {
            assert_eq!(admission.active, super::super::MAX_ACTIVE_SONG_ANALYSES);
            break;
        }
        drop(admission);
        assert!(
            Instant::now() < deadline,
            "analysis queue did not reach its bound"
        );
        std::thread::yield_now();
    }
    release.wait();

    let failures = handles
        .into_iter()
        .map(|handle| handle.join().expect("analysis request thread"))
        .filter(Result::is_err)
        .count();
    assert_eq!(failures, 1, "the 65th distinct queued ID must fail closed");
    assert_eq!(
        runtime.analysis_test_runs.load(Ordering::Acquire),
        66,
        "only two active plus sixty-four queued IDs may run"
    );
    let admission = runtime
        .analysis_admission
        .lock()
        .expect("analysis admission lock");
    assert_eq!(admission.active, 0);
    assert_eq!(admission.queued, 0);
    drop(admission);
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn viewport_analysis_coalesces_song_revisions_and_runs_selected_first() {
    let (root, runtime, generation, ids) = package_b_native_fixture("analysis-order", 3);
    let selected = ids[1].clone();
    let visible = ids.clone();
    let first = runtime
        .set_viewport(package_b_viewport(
            generation,
            0,
            Some(selected.clone()),
            visible,
        ))
        .expect("selected viewport");
    assert!(first.accepted);
    assert_eq!(
        *runtime.analysis_test_order.lock().expect("analysis order"),
        vec![ids[1].clone(), ids[0].clone(), ids[2].clone()]
    );

    runtime.invalidate_analysis_cache();
    let baseline_runs = runtime.analysis_test_runs.load(Ordering::Acquire);
    let settings_revision = runtime.analysis_revision.load(Ordering::Acquire);
    let request = package_b_viewport(generation, 0, None, vec![ids[0].clone()]);
    let (first_request, release) = begin_paused_viewport(&runtime, request.clone());
    let second_runtime = std::sync::Arc::clone(&runtime);
    let second_request = std::thread::spawn(move || second_runtime.set_viewport(request));
    wait_for_analysis_flight_waiters(&runtime, &ids[0], generation, settings_revision, 2);
    release.wait();

    let stale = first_request
        .join()
        .expect("first viewport thread")
        .expect("first response");
    let current = second_request
        .join()
        .expect("second viewport thread")
        .expect("second response");
    assert!(!stale.accepted);
    assert!(stale.items.is_empty());
    assert!(current.accepted);
    assert_eq!(
        runtime.analysis_test_runs.load(Ordering::Acquire) - baseline_runs,
        1,
        "overlapping requests for one song and revision must analyze once"
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn viewport_discards_analysis_after_like_and_viewport_churn() {
    let (root, runtime, generation, ids) = package_b_native_fixture("analysis-like", 2);
    let (delayed, release) = begin_paused_viewport(
        &runtime,
        package_b_viewport(generation, 0, None, vec![ids[0].clone()]),
    );
    runtime
        .dispatch(
            "catalog.set_liked",
            serde_json::json!({"songId":ids[0],"liked":true,"generation":generation}),
        )
        .expect("like while analysis is paused");
    release.wait();
    let stale = delayed
        .join()
        .expect("viewport worker")
        .expect("viewport response");
    assert!(!stale.accepted);
    assert!(stale.items.is_empty());
    let latest = runtime
        .set_viewport(package_b_viewport(
            generation,
            0,
            None,
            vec![ids[0].clone()],
        ))
        .expect("latest liked viewport");
    assert!(latest.accepted);
    assert!(latest.items[0].liked);
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);

    let (root, runtime, generation, ids) = package_b_native_fixture("analysis-churn", 2);
    let (old_viewport, release) = begin_paused_viewport(
        &runtime,
        package_b_viewport(generation, 0, None, vec![ids[0].clone()]),
    );
    let current = runtime
        .set_viewport(package_b_viewport(
            generation,
            1,
            None,
            vec![ids[1].clone()],
        ))
        .expect("new viewport while old analysis is paused");
    assert!(current.accepted);
    assert_eq!(current.items[0].song_id, ids[1]);
    release.wait();
    let stale = old_viewport
        .join()
        .expect("old viewport worker")
        .expect("old viewport response");
    assert!(!stale.accepted);
    assert!(stale.items.is_empty());
    assert!(
        !runtime
            .analysis_cache
            .lock()
            .expect("analysis cache")
            .entries
            .contains_key(&super::super::AnalysisKey {
                song_id: ids[0].clone(),
                generation,
                settings_revision: runtime.analysis_revision.load(Ordering::Acquire),
            })
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn analysis_results_are_discarded_after_settings_change_and_catalog_reload() {
    let (root, runtime, generation, ids) = package_b_native_fixture("analysis-settings", 1);
    let old_revision = runtime.analysis_revision.load(Ordering::Acquire);
    let (delayed, release) = begin_paused_viewport(
        &runtime,
        package_b_viewport(generation, 0, None, vec![ids[0].clone()]),
    );
    runtime
        .dispatch(
            "settings.patch",
            serde_json::json!({"playbackDefaults":{"tempoScale":1.05}}),
        )
        .expect("settings patch while analysis is paused");
    assert!(runtime.analysis_revision.load(Ordering::Acquire) > old_revision);
    release.wait();
    let stale = delayed
        .join()
        .expect("viewport worker")
        .expect("viewport response");
    assert!(!stale.accepted);
    assert!(stale.items.is_empty());
    assert!(
        runtime
            .analysis_cache
            .lock()
            .expect("analysis cache")
            .entries
            .is_empty()
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);

    let (root, runtime, generation, ids) = package_b_native_fixture("analysis-reload", 1);
    let (delayed, release) = begin_paused_viewport(
        &runtime,
        package_b_viewport(generation, 0, None, vec![ids[0].clone()]),
    );
    let reloaded: serde_json::Value = runtime
        .dispatch("catalog.reload", serde_json::json!({}))
        .expect("catalog reload while analysis is paused");
    let reloaded_generation = reloaded["generation"].as_u64().expect("new generation");
    assert!(reloaded_generation > generation);
    release.wait();
    let stale = delayed
        .join()
        .expect("viewport worker")
        .expect("viewport response");
    assert!(!stale.accepted);
    assert!(stale.items.is_empty());
    assert!(
        runtime
            .analysis_cache
            .lock()
            .expect("analysis cache")
            .entries
            .keys()
            .all(|key| key.generation != generation)
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn catalog_cache_reconciliation_requires_exact_identity_and_path() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-cache-identity-path-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");

    let active_id = "0123456789abcdef0123456789abcdef";
    let old_id = "fedcba9876543210fedcba9876543210";
    let active_path = root.join("songs/active-missing.json");
    let active_path = active_path.to_string_lossy().into_owned();
    let mismatched_path = root.join("songs/other-missing.json");
    let mismatched_path = mismatched_path.to_string_lossy().into_owned();
    let cache_entry =
        |song_id: &str, canonical_path: &str, file_size, modified_unix_ms| CatalogCacheEntry {
            source_id: "builtin".into(),
            kind: CatalogCacheSourceKind::Builtin,
            canonical_path: canonical_path.into(),
            song_id: song_id.into(),
            title: format!("Cached {song_id}"),
            file_size,
            modified_unix_ms,
        };
    let previous = CatalogCache {
        schema_version: CATALOG_CACHE_SCHEMA_VERSION,
        cache_generation: 1,
        source_fingerprint: "a".repeat(64),
        sources: vec![CatalogCacheSource {
            source_id: "builtin".into(),
            kind: CatalogCacheSourceKind::Builtin,
            import_kind: None,
            display_name: "Built-in library".into(),
            available: true,
            song_ids: vec![active_id.into(), old_id.into()],
        }],
        entries: vec![
            cache_entry(active_id, &mismatched_path, 999, 999),
            cache_entry(old_id, &active_path, 888, 888),
            cache_entry(active_id, &active_path, 123, 111),
            cache_entry(active_id, &active_path, 456, 222),
        ],
    };
    *runtime.cached_catalog.lock().expect("cached catalog") = Some(previous.clone());

    let mut composition = sky_native_adapters::CatalogComposition::default();
    composition
        .entries
        .push(sky_app_core::catalog::CatalogSourceEntry::stable(
            active_path.clone(),
            "Active title",
            sky_app_core::catalog::StableSongId::new(active_id).expect("active ID"),
        ));
    composition.builtin_membership.insert(active_id.into());
    composition.library_membership.insert(active_id.into());
    composition.builtin_status =
        sky_native_adapters::BuiltinCatalogStatus::Available { song_count: 1 };
    let reconciled = runtime
        .catalog_cache_from_composition(&composition, &"b".repeat(64))
        .expect("exact cache identity match");
    assert_eq!(reconciled.entries.len(), 1);
    assert_eq!(reconciled.entries[0].song_id, active_id);
    assert_eq!(reconciled.entries[0].canonical_path, active_path);
    assert_eq!(reconciled.entries[0].file_size, 123);
    assert_eq!(reconciled.entries[0].modified_unix_ms, 111);
    assert_eq!(reconciled.entries[0].title, "Active title");

    let retired_path = root.join("songs/retired-id.json");
    let retired_path = retired_path.to_string_lossy().into_owned();
    let retired = CatalogCache {
        entries: vec![cache_entry(old_id, &retired_path, 77, 77)],
        ..previous
    };
    *runtime.cached_catalog.lock().expect("cached catalog") = Some(retired);
    let mut replacement = sky_native_adapters::CatalogComposition::default();
    let replacement_id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    replacement
        .entries
        .push(sky_app_core::catalog::CatalogSourceEntry::stable(
            retired_path.clone(),
            "Replacement identity",
            sky_app_core::catalog::StableSongId::new(replacement_id).expect("replacement ID"),
        ));
    replacement.builtin_membership.insert(replacement_id.into());
    replacement.library_membership.insert(replacement_id.into());
    replacement.builtin_status =
        sky_native_adapters::BuiltinCatalogStatus::Available { song_count: 1 };
    assert!(
        runtime
            .catalog_cache_from_composition(&replacement, &"c".repeat(64))
            .is_err(),
        "a retired ID at the same path must not supply replacement metadata"
    );

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn catalog_reload_completes_while_prepare_model_is_paused_and_supersedes_it() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-prepare-catalog-race-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(
        root.join("songs/demo.json"),
        r#"{"name":"Demo","songNotes":[{"time":0,"key":"Key0"}]}"#,
    )
    .expect("song");
    let runtime = Arc::new(NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime"));
    let generation = catalog_generation(&runtime);
    let search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({"query":"demo","offset":0,"limit":10,"generation":generation}),
        )
        .expect("search");
    let song_id = search["items"][0]["song_id"]
        .as_str()
        .expect("song ID")
        .to_owned();
    let request = crate::commands::PlaybackPrepareRequest {
        song_id: song_id.clone(),
        generation,
        config: PlaybackConfigDto {
            hold_frames: 1.0,
            timing_margin_us: 800,
            tempo_scale: 1.0,
            fps: 60,
            dry_run: true,
        },
    };
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let entered_hook = Arc::clone(&entered);
    let release_hook = Arc::clone(&release);
    *runtime
        .playback
        .prepare_before_model_test_hook
        .lock()
        .expect("model hook") = Some(Arc::new(move || {
        entered_hook.wait();
        release_hook.wait();
    }));

    let prepare_runtime = Arc::clone(&runtime);
    let prepare = thread::spawn(move || prepare_runtime.prepare_playback(request));
    entered.wait();
    let overlapping_search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query":"demo",
                "offset":0,
                "limit":10,
                "generation":generation
            }),
        )
        .expect("search while playback preparation is paused");
    assert_eq!(overlapping_search["items"][0]["song_id"], song_id);
    // This direct lock probe is deterministic: catalog query work does not
    // retain the catalog mutex across model analysis, and reload can proceed.
    assert!(runtime.catalog.try_lock().is_ok());
    let reload_runtime = Arc::clone(&runtime);
    let (started_tx, started_rx) = mpsc::channel();
    let (reload_tx, reload_rx) = mpsc::channel();
    let reload = thread::spawn(move || {
        started_tx.send(()).expect("reload started marker");
        let result = reload_runtime.dispatch("catalog.reload", Value::Object(Default::default()));
        reload_tx.send(result).expect("reload result");
    });
    started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("reload thread started");
    let reload_result = reload_rx.recv_timeout(Duration::from_secs(5));
    if reload_result.is_err() {
        release.wait();
        let _ = prepare.join();
        let _ = reload.join();
        runtime.shutdown();
        let _ = fs::remove_dir_all(root);
        panic!("catalog reload waited for paused whole-stream analysis");
    }
    reload_result
        .expect("reload result received")
        .expect("catalog reload");
    reload.join().expect("catalog reload thread");
    let new_generation = catalog_generation(&runtime);
    assert!(new_generation > generation);
    release.wait();
    assert!(
        prepare
            .join()
            .expect("Prepare worker")
            .expect_err("catalog generation invalidates old source snapshot")
            .contains("superseded")
    );
    assert!(
        runtime
            .playback
            .prepared
            .lock()
            .expect("prepared cache")
            .is_empty()
    );

    let latest = runtime
        .prepare_playback(crate::commands::PlaybackPrepareRequest {
            song_id,
            generation: new_generation,
            config: PlaybackConfigDto {
                hold_frames: 1.0,
                timing_margin_us: 800,
                tempo_scale: 1.0,
                fps: 60,
                dry_run: true,
            },
        })
        .expect("new-generation Prepare succeeds");
    assert!(latest.prepared_id.is_some());
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}
