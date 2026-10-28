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

fn viewport_song_ids(response: &crate::commands::CatalogViewportDto) -> Vec<String> {
    response
        .items
        .iter()
        .map(|item| item.song_id.clone())
        .collect()
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
fn viewport_projection_rejects_like_playlist_settings_and_generation_changes() {
    let (root, runtime, generation, ids) = viewport_projection_fixture("revision", 2);
    let request = viewport_request(generation, 0, 1, None);
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
        .set_viewport(viewport_request(reloaded_generation, 0, 1, None))
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
