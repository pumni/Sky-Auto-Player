use super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

fn viewport_projection_fixture(
    label: &str,
    size: usize,
) -> (PathBuf, Arc<NativeDesktopRuntime>, u64, Vec<String>) {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("sky-viewport-projection-{label}-{size}-{suffix}"));
    let songs_root = root.join("songs");
    fs::create_dir_all(&songs_root).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    for index in 0..size {
        fs::write(
            songs_root.join(format!("song-{index:05}.json")),
            format!(
                "{{\"name\":\"Projection Song {index:05}\",\"songNotes\":[{{\"time\":0,\"key\":\"Key0\"}}]}}"
            ),
        )
        .expect("fixture song");
    }
    let runtime = Arc::new(NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime"));
    let generation = catalog_generation(&runtime);
    let catalog = runtime.catalog.lock().expect("catalog lock");
    let ids = catalog
        .index
        .snapshot()
        .items
        .into_iter()
        .map(|row| row.song_id)
        .collect();
    drop(catalog);
    (root, runtime, generation, ids)
}

fn viewport_request(
    generation: u64,
    first: usize,
    last: usize,
    selected_song_id: Option<String>,
) -> crate::commands::CatalogViewportRequest {
    crate::commands::CatalogViewportRequest {
        generation,
        first_index: first as u64,
        last_index: last as i64,
        selected_song_id,
        song_ids: Vec::new(),
    }
}

fn explicit_viewport_request(
    generation: u64,
    first: usize,
    last: usize,
    selected_song_id: Option<String>,
    song_ids: Vec<String>,
) -> crate::commands::CatalogViewportRequest {
    crate::commands::CatalogViewportRequest {
        generation,
        first_index: first as u64,
        last_index: last as i64,
        selected_song_id,
        song_ids,
    }
}

fn viewport_song_ids(response: &crate::commands::CatalogViewportDto) -> Vec<String> {
    response
        .items
        .iter()
        .map(|item| item.song_id.clone())
        .collect()
}

fn assert_viewport_parity(
    actual: &crate::commands::CatalogViewportDto,
    fresh: &crate::commands::CatalogViewportDto,
) {
    assert_eq!(
        serde_json::to_value(actual).expect("serialize actual viewport"),
        serde_json::to_value(fresh).expect("serialize fresh viewport")
    );
}

#[test]
fn viewport_projection_reuses_only_the_exact_last_accepted_window() {
    let (root, runtime, generation, ids) = viewport_projection_fixture("shift", 201);
    let first_request = viewport_request(generation, 0, 199, None);
    let first = runtime
        .set_viewport(first_request.clone())
        .expect("first viewport");
    assert!(first.accepted);
    assert_eq!(viewport_song_ids(&first), ids[..200]);
    assert_eq!(runtime.analysis_test_runs.load(Ordering::Acquire), 200);
    {
        let projection = runtime
            .viewport_projection
            .lock()
            .expect("viewport projection lock");
        let projection = projection.as_ref().expect("accepted projection");
        assert_eq!(projection.response.items.len(), 200);
        assert_eq!(projection.key.ordered_song_ids, ids[..200]);
        assert!(projection.key.request_song_ids.is_empty());
        assert!(projection.key.selected_song_id.is_none());
        assert!(projection.response.items.len() <= super::super::MAX_CACHED_VIEWPORT_RESPONSE_ROWS);
    }
    assert_eq!(
        runtime
            .analysis_cache
            .lock()
            .expect("analysis cache")
            .entries
            .len(),
        super::super::MAX_CACHED_SONG_ANALYSES
    );

    let runs_after_cold = runtime.analysis_test_runs.load(Ordering::Acquire);
    let repeated = runtime
        .set_viewport(first_request.clone())
        .expect("exact repeated viewport");
    assert!(repeated.accepted);
    assert_eq!(viewport_song_ids(&repeated), ids[..200]);
    assert_eq!(
        runtime.analysis_test_runs.load(Ordering::Acquire),
        runs_after_cold
    );
    assert_eq!(
        runtime
            .viewport_projection_test_hits
            .load(Ordering::Acquire),
        1
    );

    let shifted_request = viewport_request(generation, 1, 200, None);
    let shifted = runtime
        .set_viewport(shifted_request.clone())
        .expect("one-row shifted viewport");
    assert!(shifted.accepted);
    assert_eq!(viewport_song_ids(&shifted), ids[1..201]);
    let runs_after_shift = runtime.analysis_test_runs.load(Ordering::Acquire);
    assert_eq!(
        runs_after_shift - runs_after_cold,
        1,
        "a validated one-row shift reanalyzes only the entering row"
    );
    assert_eq!(
        runtime
            .viewport_projection_test_hits
            .load(Ordering::Acquire),
        1
    );

    let repeated_shift = runtime
        .set_viewport(shifted_request)
        .expect("exact repeated shifted viewport");
    assert!(repeated_shift.accepted);
    assert_eq!(
        runtime.analysis_test_runs.load(Ordering::Acquire),
        runs_after_shift
    );
    assert_eq!(
        runtime
            .viewport_projection_test_hits
            .load(Ordering::Acquire),
        2
    );

    let back = runtime
        .set_viewport(first_request.clone())
        .expect("viewport shifted back");
    assert!(back.accepted);
    assert_eq!(viewport_song_ids(&back), ids[..200]);
    let runs_after_back = runtime.analysis_test_runs.load(Ordering::Acquire);
    assert_eq!(
        runs_after_back - runs_after_shift,
        1,
        "a validated shift back reanalyzes only its entering row"
    );
    assert_eq!(
        runtime
            .viewport_projection_test_hits
            .load(Ordering::Acquire),
        2
    );
    let repeated_back = runtime
        .set_viewport(first_request)
        .expect("exact repeated viewport after shift back");
    assert!(repeated_back.accepted);
    assert_eq!(
        runtime.analysis_test_runs.load(Ordering::Acquire),
        runs_after_back
    );
    assert_eq!(
        runtime
            .viewport_projection_test_hits
            .load(Ordering::Acquire),
        3
    );
    assert!(
        runtime
            .analysis_cache
            .lock()
            .expect("analysis cache")
            .entries
            .len()
            <= super::super::MAX_CACHED_SONG_ANALYSES
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn viewport_projection_retains_no_more_than_200_row_summaries() {
    let (root, runtime, generation, ids) = viewport_projection_fixture("boundary", 201);
    let oversized = runtime
        .set_viewport(viewport_request(generation, 0, 200, None))
        .expect("201-row viewport");
    assert!(oversized.accepted);
    assert_eq!(oversized.items.len(), 201);
    assert!(
        runtime
            .viewport_projection
            .lock()
            .expect("viewport projection lock")
            .is_none()
    );

    let bounded = runtime
        .set_viewport(viewport_request(generation, 0, 199, None))
        .expect("200-row viewport");
    assert!(bounded.accepted);
    assert_eq!(viewport_song_ids(&bounded), ids[..200]);
    let projection = runtime
        .viewport_projection
        .lock()
        .expect("viewport projection lock")
        .clone()
        .expect("bounded accepted response projection");
    assert_eq!(projection.response.items.len(), 200);
    assert_eq!(projection.key.ordered_song_ids, ids[..200]);
    assert!(projection.response.items.len() <= super::super::MAX_CACHED_VIEWPORT_RESPONSE_ROWS);
    assert!(
        runtime
            .analysis_cache
            .lock()
            .expect("analysis cache")
            .entries
            .len()
            <= super::super::MAX_CACHED_SONG_ANALYSES
    );

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn explicit_unselected_viewports_reuse_adjacent_windows_and_match_fresh_hydration() {
    let (root, runtime, generation, ids) = viewport_projection_fixture("explicit-unselected", 64);
    let first_request = explicit_viewport_request(generation, 0, 19, None, ids[..20].to_vec());
    let first = runtime
        .set_viewport(first_request.clone())
        .expect("first explicit unselected viewport");
    assert!(first.accepted);
    assert_eq!(viewport_song_ids(&first), ids[..20]);
    assert_eq!(first.selected_song_id, None);
    assert_eq!(runtime.analysis_test_runs.load(Ordering::Acquire), 20);

    let repeated = runtime
        .set_viewport(first_request.clone())
        .expect("repeat explicit unselected viewport");
    assert_viewport_parity(&repeated, &first);
    let runs_after_repeat = runtime.analysis_test_runs.load(Ordering::Acquire);

    let shifted_request = explicit_viewport_request(generation, 1, 20, None, ids[1..=20].to_vec());
    let shifted = runtime
        .set_viewport(shifted_request.clone())
        .expect("shift explicit unselected viewport");
    assert!(shifted.accepted);
    assert_eq!(viewport_song_ids(&shifted), ids[1..=20]);
    assert_eq!(shifted.selected_song_id, None);
    assert_eq!(
        runtime.analysis_test_runs.load(Ordering::Acquire) - runs_after_repeat,
        1,
        "an adjacent explicit-ID shift analyzes only its entering row"
    );

    let back = runtime
        .set_viewport(first_request.clone())
        .expect("shift explicit unselected viewport back");
    assert!(back.accepted);
    assert_eq!(viewport_song_ids(&back), ids[..20]);
    assert_eq!(back.selected_song_id, None);
    assert_eq!(
        runtime.analysis_test_runs.load(Ordering::Acquire),
        runs_after_repeat + 1,
        "shift-back reuses all rows still in the heavyweight cache"
    );

    runtime.invalidate_analysis_cache();
    let fresh_shifted = runtime
        .set_viewport(shifted_request)
        .expect("fresh explicit unselected shifted viewport");
    assert_viewport_parity(&shifted, &fresh_shifted);
    runtime.invalidate_analysis_cache();
    let fresh_back = runtime
        .set_viewport(first_request)
        .expect("fresh explicit unselected viewport after shift-back");
    assert_viewport_parity(&back, &fresh_back);
    assert!(
        runtime
            .analysis_cache
            .lock()
            .expect("analysis cache")
            .entries
            .len()
            <= super::super::MAX_CACHED_SONG_ANALYSES
    );

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn explicit_selected_viewports_reuse_adjacent_windows_and_match_fresh_hydration() {
    let (root, runtime, generation, ids) = viewport_projection_fixture("explicit-shift", 301);
    for width in [20_usize, 40, 200] {
        for (placement, selected_index) in
            [("first", 0_usize), ("last", width - 1), ("outside", 250)]
        {
            runtime.invalidate_analysis_cache();
            let selected_song_id = ids[selected_index].clone();
            let first_request = explicit_viewport_request(
                generation,
                0,
                width - 1,
                Some(selected_song_id.clone()),
                ids[..width].to_vec(),
            );
            let first = runtime
                .set_viewport(first_request.clone())
                .unwrap_or_else(|error| panic!("{placement} first window failed: {error}"));
            assert!(first.accepted, "{placement} first window accepted");
            assert_eq!(viewport_song_ids(&first), ids[..width]);
            assert_eq!(
                first.selected_song_id.as_deref(),
                Some(selected_song_id.as_str())
            );
            assert!(first.items.len() <= super::super::MAX_CACHED_VIEWPORT_RESPONSE_ROWS);

            let selected_key = super::super::AnalysisKey {
                song_id: selected_song_id.clone(),
                generation,
                settings_revision: runtime.analysis_revision.load(Ordering::Acquire),
            };
            assert!(
                runtime
                    .analysis_cache
                    .lock()
                    .expect("analysis cache")
                    .entries
                    .contains_key(&selected_key)
            );
            let runs_after_first = runtime.analysis_test_runs.load(Ordering::Acquire);
            let selected_detail = runtime
                .detail(crate::commands::CatalogDetailRequest {
                    song_id: selected_song_id.clone(),
                    generation: Some(generation),
                    conversion_mode: None,
                    target_pitch: None,
                })
                .expect("selected detail after first explicit viewport");
            assert_eq!(selected_detail.song_id, selected_song_id);
            assert_eq!(
                runtime.analysis_test_runs.load(Ordering::Acquire),
                runs_after_first
            );

            let repeated_first = runtime
                .set_viewport(first_request.clone())
                .expect("repeat explicit viewport");
            assert_viewport_parity(&repeated_first, &first);
            let runs_after_repeat = runtime.analysis_test_runs.load(Ordering::Acquire);

            let shifted_request = explicit_viewport_request(
                generation,
                1,
                width,
                Some(selected_song_id.clone()),
                ids[1..=width].to_vec(),
            );
            let shifted = runtime
                .set_viewport(shifted_request.clone())
                .unwrap_or_else(|error| panic!("{placement} shifted window failed: {error}"));
            assert!(shifted.accepted, "{placement} shifted response accepted");
            assert_eq!(viewport_song_ids(&shifted), ids[1..=width]);
            assert_eq!(
                shifted.selected_song_id.as_deref(),
                Some(selected_song_id.as_str())
            );
            assert_eq!(
                runtime.analysis_test_runs.load(Ordering::Acquire) - runs_after_repeat,
                1,
                "a new entering row is analyzed once when the selected detail is cached"
            );
            let projection = runtime
                .viewport_projection
                .lock()
                .expect("viewport projection lock")
                .clone()
                .expect("accepted explicit shifted projection");
            assert_eq!(projection.key.request_song_ids, ids[1..=width]);
            assert_eq!(projection.key.ordered_song_ids, ids[1..=width]);
            assert_eq!(projection.response.items.len(), width);
            assert!(
                projection.response.items.len() <= super::super::MAX_CACHED_VIEWPORT_RESPONSE_ROWS
            );
            assert!(
                runtime
                    .analysis_cache
                    .lock()
                    .expect("analysis cache")
                    .entries
                    .contains_key(&selected_key)
            );

            let runs_after_shift = runtime.analysis_test_runs.load(Ordering::Acquire);
            let selected_after_shift = runtime
                .detail(crate::commands::CatalogDetailRequest {
                    song_id: selected_song_id.clone(),
                    generation: Some(generation),
                    conversion_mode: None,
                    target_pitch: None,
                })
                .expect("selected detail after explicit shift");
            assert_eq!(selected_after_shift.song_id, selected_song_id);
            assert_eq!(
                runtime.analysis_test_runs.load(Ordering::Acquire),
                runs_after_shift
            );

            let repeated_shift = runtime
                .set_viewport(shifted_request.clone())
                .expect("repeat explicit shifted viewport");
            assert_viewport_parity(&repeated_shift, &shifted);
            let runs_after_shift_repeat = runtime.analysis_test_runs.load(Ordering::Acquire);
            let back = runtime
                .set_viewport(first_request.clone())
                .expect("explicit viewport shifted back");
            assert!(back.accepted, "{placement} shift-back response accepted");
            assert_eq!(viewport_song_ids(&back), ids[..width]);
            assert_eq!(
                back.selected_song_id.as_deref(),
                Some(selected_song_id.as_str())
            );
            let expected_back_analyses = if width == 200 && placement != "first" {
                1
            } else {
                0
            };
            assert_eq!(
                runtime.analysis_test_runs.load(Ordering::Acquire) - runs_after_shift_repeat,
                expected_back_analyses,
                "shift-back only analyzes its entering row when it has been evicted"
            );

            runtime.invalidate_analysis_cache();
            let fresh_shifted = runtime
                .set_viewport(shifted_request)
                .expect("fresh explicit shifted viewport");
            assert_viewport_parity(&shifted, &fresh_shifted);
            runtime.invalidate_analysis_cache();
            let fresh_back = runtime
                .set_viewport(first_request)
                .expect("fresh explicit viewport after shift-back");
            assert_viewport_parity(&back, &fresh_back);
            assert!(
                runtime
                    .analysis_cache
                    .lock()
                    .expect("analysis cache")
                    .entries
                    .len()
                    <= super::super::MAX_CACHED_SONG_ANALYSES
            );
        }
    }
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stale_explicit_shift_cannot_publish_over_the_latest_viewport_intent() {
    let (root, runtime, generation, ids) = viewport_projection_fixture("explicit-stale", 64);
    let first =
        explicit_viewport_request(generation, 0, 19, Some(ids[0].clone()), ids[..20].to_vec());
    assert!(
        runtime
            .set_viewport(first)
            .expect("initial explicit viewport")
            .accepted
    );

    let shifted = explicit_viewport_request(
        generation,
        1,
        20,
        Some(ids[0].clone()),
        ids[1..=20].to_vec(),
    );
    let entered = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    runtime.set_analysis_test_gate(Arc::clone(&entered), Arc::clone(&release), 1);
    let shifted_runtime = Arc::clone(&runtime);
    let stale_worker = std::thread::spawn(move || shifted_runtime.set_viewport(shifted));
    entered.wait();
    let shifted_revision = runtime.viewport_revision.load(Ordering::Acquire);

    let latest_request = explicit_viewport_request(
        generation,
        2,
        21,
        Some(ids[0].clone()),
        ids[2..=21].to_vec(),
    );
    let latest_runtime = Arc::clone(&runtime);
    let latest_worker = std::thread::spawn(move || latest_runtime.set_viewport(latest_request));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while runtime.viewport_revision.load(Ordering::Acquire) <= shifted_revision {
        assert!(
            std::time::Instant::now() < deadline,
            "latest explicit viewport did not supersede the shifted request"
        );
        std::thread::yield_now();
    }
    release.wait();

    let latest = latest_worker
        .join()
        .expect("latest viewport worker")
        .expect("latest viewport response");
    let stale = stale_worker
        .join()
        .expect("stale shifted viewport worker")
        .expect("stale shifted viewport response");
    assert!(latest.accepted);
    assert!(!stale.accepted);
    let projection = runtime
        .viewport_projection
        .lock()
        .expect("viewport projection lock")
        .clone()
        .expect("latest accepted viewport projection");
    assert_eq!(projection.key.first_index, 2);
    assert_eq!(projection.key.last_index, 21);
    assert_eq!(projection.key.request_song_ids, ids[2..=21]);
    assert_eq!(projection.key.ordered_song_ids, ids[2..=21]);
    assert_eq!(viewport_song_ids(&projection.response), ids[2..=21]);
    assert!(projection.response.accepted);
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn viewport_projection_rejects_like_playlist_settings_and_generation_changes() {
    let (root, runtime, generation, ids) = viewport_projection_fixture("revision", 2);
    let request = explicit_viewport_request(generation, 0, 1, Some(ids[0].clone()), ids.clone());
    let first = runtime
        .set_viewport(request.clone())
        .expect("first viewport");
    assert!(first.accepted);
    assert_eq!(
        runtime
            .viewport_projection_test_hits
            .load(Ordering::Acquire),
        0
    );
    assert!(
        runtime
            .set_viewport(request.clone())
            .expect("warm viewport")
            .accepted
    );
    assert_eq!(
        runtime
            .viewport_projection_test_hits
            .load(Ordering::Acquire),
        1
    );

    runtime
        .dispatch(
            "catalog.set_liked",
            serde_json::json!({"songId":ids[0],"liked":true,"generation":generation}),
        )
        .expect("Like membership change");
    assert!(
        runtime
            .viewport_projection
            .lock()
            .expect("projection lock")
            .is_none()
    );
    let liked = runtime
        .set_viewport(request.clone())
        .expect("viewport after Like change");
    assert!(liked.accepted);
    assert!(liked.items[0].liked);
    assert_eq!(liked.selected_song_id.as_deref(), Some(ids[0].as_str()));
    assert_eq!(
        runtime
            .viewport_projection_test_hits
            .load(Ordering::Acquire),
        1
    );

    let playlist = runtime
        .dispatch(
            "library.create_playlist",
            serde_json::json!({"name":"Projection revision"}),
        )
        .expect("create playlist");
    let playlist_id = playlist["id"].as_str().expect("playlist ID");
    runtime
        .dispatch(
            "library.add_songs",
            serde_json::json!({"playlistId":playlist_id,"songIds":[ids[1]]}),
        )
        .expect("change playlist membership");
    assert!(
        runtime
            .viewport_projection
            .lock()
            .expect("projection lock")
            .is_none()
    );
    let playlist_changed = runtime
        .set_viewport(request.clone())
        .expect("viewport after playlist membership change");
    assert!(playlist_changed.accepted);
    assert_eq!(
        runtime
            .viewport_projection_test_hits
            .load(Ordering::Acquire),
        1
    );

    runtime
        .dispatch(
            "settings.patch",
            serde_json::json!({"playbackDefaults":{"tempoScale":1.05}}),
        )
        .expect("change analysis settings");
    assert!(
        runtime
            .viewport_projection
            .lock()
            .expect("projection lock")
            .is_none()
    );
    let analyses_before_settings_refresh = runtime.analysis_test_runs.load(Ordering::Acquire);
    let settings_changed = runtime
        .set_viewport(request.clone())
        .expect("viewport after settings change");
    assert!(settings_changed.accepted);
    assert!(runtime.analysis_test_runs.load(Ordering::Acquire) > analyses_before_settings_refresh);
    assert_eq!(
        runtime
            .viewport_projection_test_hits
            .load(Ordering::Acquire),
        1
    );

    let reload: serde_json::Value = runtime
        .dispatch("catalog.reload", serde_json::json!({}))
        .expect("catalog reload");
    let reloaded_generation = reload["generation"].as_u64().expect("new generation");
    assert!(reloaded_generation > generation);
    assert!(
        runtime
            .viewport_projection
            .lock()
            .expect("projection lock")
            .is_none()
    );
    assert!(
        runtime.set_viewport(request).is_err(),
        "old generation is rejected"
    );
    let reloaded = runtime
        .set_viewport(explicit_viewport_request(
            reloaded_generation,
            0,
            1,
            Some(ids[0].clone()),
            ids,
        ))
        .expect("new generation viewport");
    assert!(reloaded.accepted);
    assert_eq!(reloaded.generation, reloaded_generation);
    assert_eq!(
        runtime
            .viewport_projection_test_hits
            .load(Ordering::Acquire),
        1
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn selected_analysis_stays_in_the_bounded_lru_and_detail_remains_independent() {
    let (root, runtime, generation, ids) = viewport_projection_fixture("selected", 201);
    let start = runtime.analysis_test_runs.load(Ordering::Acquire);
    for (label, selected_index) in [("first", 0), ("last", 199), ("outside", 200)] {
        runtime.invalidate_analysis_cache();
        let request = viewport_request(generation, 0, 199, Some(ids[selected_index].clone()));
        let response = runtime
            .set_viewport(request)
            .unwrap_or_else(|error| panic!("{label} selected viewport failed: {error}"));
        assert!(response.accepted, "{label} selected response accepted");
        assert_eq!(response.items.len(), 200);
        assert_eq!(
            response.selected_song_id.as_deref(),
            Some(ids[selected_index].as_str())
        );
        assert_eq!(viewport_song_ids(&response), ids[..200]);
        let settings_revision = runtime.analysis_revision.load(Ordering::Acquire);
        let selected_key = super::super::AnalysisKey {
            song_id: ids[selected_index].clone(),
            generation,
            settings_revision,
        };
        {
            let cache = runtime.analysis_cache.lock().expect("analysis cache");
            assert!(cache.entries.len() <= super::super::MAX_CACHED_SONG_ANALYSES);
            assert!(cache.entries.contains_key(&selected_key));
        }
        let analyses_after_viewport = runtime.analysis_test_runs.load(Ordering::Acquire);
        let detail = runtime
            .detail(crate::commands::CatalogDetailRequest {
                song_id: ids[selected_index].clone(),
                generation: Some(generation),
                conversion_mode: None,
                target_pitch: None,
            })
            .unwrap_or_else(|error| panic!("{label} selected detail failed: {error}"));
        assert_eq!(detail.song_id, ids[selected_index]);
        assert_eq!(
            runtime.analysis_test_runs.load(Ordering::Acquire),
            analyses_after_viewport,
            "{label} selected detail should reuse the heavyweight entry retained by the 128-item LRU"
        );
        let repeated_detail = runtime
            .detail(crate::commands::CatalogDetailRequest {
                song_id: ids[selected_index].clone(),
                generation: Some(generation),
                conversion_mode: None,
                target_pitch: None,
            })
            .expect("repeat selected detail");
        assert_eq!(repeated_detail.song_id, detail.song_id);
        assert_eq!(
            runtime.analysis_test_runs.load(Ordering::Acquire),
            analyses_after_viewport
        );
        let projection = runtime
            .viewport_projection
            .lock()
            .expect("projection lock")
            .clone()
            .expect("accepted compact projection");
        assert_eq!(projection.response.items.len(), 200);
        if selected_index == 200 {
            assert!(
                projection
                    .response
                    .items
                    .iter()
                    .all(|row| row.song_id != ids[selected_index])
            );
        } else {
            assert!(
                projection
                    .response
                    .items
                    .iter()
                    .any(|row| row.song_id == ids[selected_index])
            );
        }
        assert!(
            runtime
                .analysis_cache
                .lock()
                .expect("analysis cache")
                .entries
                .len()
                <= 128
        );
    }
    assert!(runtime.analysis_test_runs.load(Ordering::Acquire) > start);
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}
