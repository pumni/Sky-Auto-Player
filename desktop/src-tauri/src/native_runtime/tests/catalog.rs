use super::*;
#[test]
fn catalog_cache_reconciles_add_change_delete_and_matches_clean_compose() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-catalog-cache-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let keep = root.join("songs/keep.json");
    let deleted = root.join("songs/deleted.json");
    fs::write(&keep, b"keep").expect("keep song");
    fs::write(&deleted, b"deleted").expect("deleted song");
    write_catalog_cache(
        &root,
        vec![
            user_cache_entry(&keep, "Cached keep", 4),
            user_cache_entry(&deleted, "Cached deleted", 7),
        ],
    );

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let cached = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({"query":"","offset":0,"limit":20,"source":{"kind":"smart","id":"all"}}),
            )
            .expect("cached search");
    assert!(
        cached["items"]
            .as_array()
            .expect("cached items")
            .iter()
            .any(|item| item["title"] == "Cached keep")
    );

    fs::write(&keep, vec![b'k'; 41]).expect("changed song");
    fs::remove_file(&deleted).expect("delete song");
    fs::write(root.join("songs/added.json"), b"added").expect("add song");
    let claim = runtime
        .claim_catalog_load()
        .expect("reconcile claim")
        .expect("reconcile work");
    runtime.run_catalog_load(claim);

    let reconciled = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({"query":"","offset":0,"limit":20,"source":{"kind":"smart","id":"all"}}),
            )
            .expect("reconciled search");
    let reconciled_items = reconciled["items"].as_array().expect("reconciled items");
    assert!(reconciled_items.iter().any(|item| item["title"] == "keep"));
    assert!(reconciled_items.iter().any(|item| item["title"] == "added"));
    assert!(
        !reconciled_items
            .iter()
            .any(|item| item["title"] == "deleted")
    );

    let persisted = CatalogCacheStore::new(root.join("cache/catalog-index.json"))
        .load()
        .expect("load reconciled cache")
        .expect("reconciled cache");
    assert_eq!(persisted.entries.len(), 2);
    assert_eq!(
            persisted
                .entries
                .iter()
                .find(|entry| entry.canonical_path
                    == fs::canonicalize(&keep).unwrap().to_string_lossy())
                .expect("changed cache entry")
                .file_size,
            41
        );
    assert!(
        !persisted
            .entries
            .iter()
            .any(|entry| entry.canonical_path.ends_with("deleted.json"))
    );

    runtime.shutdown();
    fs::remove_file(root.join("cache/catalog-index.json")).expect("remove cache");
    let clean_runtime =
        NativeDesktopRuntime::from_install_root(root.clone()).expect("clean runtime");
    let clean = clean_runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({"query":"","offset":0,"limit":20,"source":{"kind":"smart","id":"all"}}),
            )
            .expect("clean search");
    assert_eq!(clean["items"], reconciled["items"]);
    clean_runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unavailable_import_preserves_cached_entries_and_manifest_intent() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-catalog-unavailable-{suffix}"));
    let imported_root = root.join("imported");
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::create_dir_all(&imported_root).expect("import root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(imported_root.join("imported.json"), b"imported").expect("imported song");
    let source_id = "a".repeat(32);
    fs::write(
            root.join("library-manifest.json"),
            serde_json::json!({
                "version": 1,
                "imports": [{"source_id": source_id, "canonical_path": fs::canonicalize(&imported_root).unwrap(), "kind": "folder"}],
                "collections": []
            })
            .to_string(),
        )
        .expect("manifest");

    let initial_runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let _ = catalog_generation(&initial_runtime);
    initial_runtime.shutdown();
    fs::remove_dir_all(&imported_root).expect("temporarily unavailable import");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("cached runtime");
    let _cached = runtime
            .dispatch(
                "catalog.search",
                serde_json::json!({"query":"","offset":0,"limit":20,"source":{"kind":"smart","id":"all"}}),
            )
            .expect("cached unavailable search");
    assert_eq!(
        runtime
            .catalog
            .lock()
            .expect("catalog lock")
            .index
            .snapshot()
            .total,
        1
    );
    let claim = runtime
        .claim_catalog_load()
        .expect("reconcile claim")
        .expect("reconcile work");
    runtime.run_catalog_load(claim);
    let status = runtime
        .catalog
        .lock()
        .expect("catalog lock")
        .imported_status
        .first()
        .cloned()
        .expect("import status");
    assert!(!status.available);
    assert_eq!(status.song_count, 1);
    assert_eq!(
        runtime
            .catalog
            .lock()
            .expect("catalog lock")
            .index
            .snapshot()
            .total,
        1
    );
    assert!(
        String::from_utf8(fs::read(root.join("library-manifest.json")).unwrap())
            .unwrap()
            .contains(&source_id)
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn duplicate_imports_share_cached_entries_without_losing_source_status() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-catalog-duplicates-{suffix}"));
    let imported_root = root.join("imported");
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::create_dir_all(&imported_root).expect("import root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(imported_root.join("one.json"), b"one").expect("song");
    let canonical = fs::canonicalize(&imported_root).expect("canonical import");
    fs::write(
            root.join("library-manifest.json"),
            serde_json::json!({
                "version": 1,
                "imports": [
                    {"source_id": "b".repeat(32), "canonical_path": canonical, "kind": "folder"},
                    {"source_id": "c".repeat(32), "canonical_path": fs::canonicalize(&imported_root).unwrap(), "kind": "folder"}
                ],
                "collections": []
            })
            .to_string(),
        )
        .expect("manifest");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let _ = catalog_generation(&runtime);
    let catalog = runtime.catalog.lock().expect("catalog lock");
    assert_eq!(catalog.imported_status.len(), 2);
    drop(catalog);
    assert_eq!(
        runtime
            .catalog
            .lock()
            .expect("catalog lock")
            .index
            .snapshot()
            .total,
        1
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cache_source_fingerprint_rejects_removed_import_before_hydration() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("sky-native-catalog-cache-removed-import-{suffix}"));
    let imported_root = root.join("imported");
    let source_id = "d".repeat(32);
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::create_dir_all(&imported_root).expect("import root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(imported_root.join("old.json"), b"old").expect("imported song");
    fs::write(
            root.join("library-manifest.json"),
            serde_json::json!({
                "version": 1,
                "imports": [{"source_id": source_id, "canonical_path": fs::canonicalize(&imported_root).unwrap(), "kind": "folder"}],
                "collections": []
            })
            .to_string(),
        )
        .expect("manifest");

    let initial_runtime =
        NativeDesktopRuntime::from_install_root(root.clone()).expect("initial runtime");
    let _ = catalog_generation(&initial_runtime);
    initial_runtime.shutdown();
    fs::write(
        root.join("library-manifest.json"),
        "{\"version\":1,\"imports\":[],\"collections\":[]}\n",
    )
    .expect("removed import manifest");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    assert!(matches!(
        *runtime.catalog_load.lock().expect("catalog state"),
        super::CatalogLoadState::Uninitialized
    ));
    assert_eq!(
        runtime
            .catalog
            .lock()
            .expect("catalog lock")
            .index
            .snapshot()
            .total,
        0
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cache_source_fingerprint_tracks_imports_added_during_runtime() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("sky-native-catalog-cache-runtime-import-{suffix}"));
    let imported_root = root.join("imported");
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::create_dir_all(&imported_root).expect("import root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(imported_root.join("added.json"), b"added").expect("imported song");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let _ = catalog_generation(&runtime);
    let created = runtime
        .dispatch(
            "library.create_playlist",
            serde_json::json!({"name":"Imported"}),
        )
        .expect("create playlist");
    let playlist_id = created["id"].as_str().expect("playlist ID").to_owned();
    let imported = runtime
        .dispatch(
            "library.import_local_folder_to_playlist",
            serde_json::json!({
                "playlistId": playlist_id,
                "paths": [imported_root]
            }),
        )
        .expect("import folder");
    assert_eq!(imported["imported_song_count"], 1);

    let persisted = CatalogCacheStore::new(root.join("cache/catalog-index.json"))
        .load()
        .expect("load persisted cache")
        .expect("persisted cache");
    assert_eq!(
        persisted.source_fingerprint,
        runtime
            .catalog_source_fingerprint()
            .expect("source fingerprint")
    );
    runtime.shutdown();

    let restarted =
        NativeDesktopRuntime::from_install_root(root.clone()).expect("restarted runtime");
    assert!(matches!(
        *restarted.catalog_load.lock().expect("catalog state"),
        super::CatalogLoadState::Cached { .. }
    ));
    let search = restarted
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query":"added",
                "offset":0,
                "limit":10
            }),
        )
        .expect("cached imported search");
    assert_eq!(search["total"], 1);
    restarted.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stale_reconcile_cannot_overwrite_newer_import_reload() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root =
        std::env::temp_dir().join(format!("sky-native-catalog-cache-reconcile-race-{suffix}"));
    let imported_root = root.join("imported");
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::create_dir_all(&imported_root).expect("import root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(imported_root.join("race.json"), b"race").expect("imported song");

    let initial_runtime =
        NativeDesktopRuntime::from_install_root(root.clone()).expect("initial runtime");
    let _ = catalog_generation(&initial_runtime);
    initial_runtime.shutdown();

    let runtime =
        Arc::new(NativeDesktopRuntime::from_install_root(root.clone()).expect("cached runtime"));
    let created = runtime
        .dispatch(
            "library.create_playlist",
            serde_json::json!({"name":"Race"}),
        )
        .expect("create playlist");
    let playlist_id = created["id"].as_str().expect("playlist ID").to_owned();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    runtime.set_catalog_compose_barrier(Arc::clone(&entered), Arc::clone(&release));
    let claim = runtime
        .claim_catalog_load()
        .expect("reconcile claim")
        .expect("reconcile work");
    let worker_runtime = Arc::clone(&runtime);
    let worker = thread::spawn(move || worker_runtime.run_catalog_load(claim));
    entered.wait();

    let imported = runtime
        .dispatch(
            "library.import_local_folder_to_playlist",
            serde_json::json!({
                "playlistId": playlist_id,
                "paths": [imported_root]
            }),
        )
        .expect("import during reconcile");
    assert_eq!(imported["imported_song_count"], 1);
    release.wait();
    worker.join().expect("reconcile worker");

    let persisted = CatalogCacheStore::new(root.join("cache/catalog-index.json"))
        .load()
        .expect("load persisted cache")
        .expect("persisted cache");
    assert_eq!(
        persisted.source_fingerprint,
        runtime
            .catalog_source_fingerprint()
            .expect("source fingerprint")
    );
    runtime.shutdown();

    let restarted =
        NativeDesktopRuntime::from_install_root(root.clone()).expect("restarted runtime");
    assert!(matches!(
        *restarted.catalog_load.lock().expect("catalog state"),
        super::CatalogLoadState::Cached { .. }
    ));
    let search = restarted
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query":"race",
                "offset":0,
                "limit":10
            }),
        )
        .expect("cached imported search");
    assert_eq!(search["total"], 1);
    restarted.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn duplicate_catalog_start_is_single_flight() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-catalog-single-flight-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");

    assert!(runtime.claim_catalog_load().expect("first claim").is_some());
    assert!(
        runtime
            .claim_catalog_load()
            .expect("duplicate claim")
            .is_none()
    );
    assert!(matches!(
        *runtime.catalog_load.lock().expect("catalog load state"),
        super::CatalogLoadState::Loading
    ));

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn catalog_worker_cannot_mutate_after_shutdown_claims_closing() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-catalog-shutdown-race-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");

    let claim = runtime
        .claim_catalog_load()
        .expect("claim")
        .expect("rebuild work");
    runtime.shutdown();
    runtime.run_catalog_load(claim);

    assert!(matches!(
        *runtime.catalog_load.lock().expect("catalog load state"),
        super::CatalogLoadState::Closing
    ));
    assert_eq!(
        runtime.catalog.lock().expect("catalog").index.generation(),
        0
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn catalog_viewport_hydrates_metadata_and_liked_songs_persist_by_song_id() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-library-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(
        root.join("songs/hydrate.json"),
        r#"{"name":"Hydrate","songNotes":[{"time":0,"key":"Key0"},{"time":100,"key":"Key1"}]}"#,
    )
    .expect("song");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let generation = catalog_generation(&runtime);
    let search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query": "",
                "offset": 0,
                "limit": 10,
                "generation": generation,
                "source": {"kind":"smart","id":"all"}
            }),
        )
        .expect("search");
    let song_id = search["items"][0]["song_id"]
        .as_str()
        .expect("song ID")
        .to_owned();
    assert_eq!(search["items"][0]["metadata_state"], "pending");
    assert_eq!(search["items"][0]["format_label"], "JSON");

    let viewport_request = serde_json::json!({
        "generation": generation,
        "firstIndex": 0,
        "lastIndex": 0,
        "selectedSongId": null,
        "songIds": [song_id.clone()]
    });
    let viewport = runtime
        .dispatch("catalog.set_viewport", viewport_request.clone())
        .expect("viewport hydration");
    assert_eq!(viewport["items"][0]["metadata_state"], "ready");
    assert_eq!(viewport["items"][0]["note_count"], 2);
    assert!(viewport["items"][0]["duration_us"].as_u64().is_some());
    let repeated_viewport = runtime
        .dispatch("catalog.set_viewport", viewport_request)
        .expect("cached viewport hydration");
    assert_eq!(repeated_viewport["items"], viewport["items"]);

    let hydrated_detail = runtime
        .detail(crate::commands::CatalogDetailRequest {
            song_id: song_id.clone(),
            generation: Some(generation),
        })
        .expect("detail from hydrated analysis");
    assert_eq!(hydrated_detail.format_label, "JSON");
    let unavailable_path = root.join("songs/hydrate-unavailable.json");
    fs::rename(root.join("songs/hydrate.json"), &unavailable_path).expect("hide source");
    let cached_detail = runtime
        .detail(crate::commands::CatalogDetailRequest {
            song_id: song_id.clone(),
            generation: Some(generation),
        })
        .expect("cached detail after source unavailable");
    assert_eq!(
        serde_json::to_value(cached_detail).expect("cached detail JSON"),
        serde_json::to_value(hydrated_detail).expect("hydrated detail JSON")
    );
    fs::rename(&unavailable_path, root.join("songs/hydrate.json")).expect("restore source");

    let liked = runtime
        .dispatch(
            "catalog.set_liked",
            serde_json::json!({
                "songId": song_id,
                "liked": true,
                "generation": generation
            }),
        )
        .expect("like");
    assert_eq!(liked["liked"], true);
    assert_eq!(liked["total"], 1);
    runtime.shutdown();

    let restarted = NativeDesktopRuntime::from_install_root(root.clone()).expect("restart");
    let bootstrap = restarted.bootstrap().expect("restart bootstrap");
    let liked_search = restarted
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query": "",
                "offset": 0,
                "limit": 10,
                "generation": bootstrap.catalog_generation,
                "source": {"kind":"smart","id":"liked"}
            }),
        )
        .expect("liked search");
    assert_eq!(liked_search["total"], 1);
    assert_eq!(liked_search["items"][0]["liked"], true);
    assert_eq!(liked_search["items"][0]["metadata_state"], "pending");
    restarted.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn warmed_native_catalog_routes_avoid_full_snapshot_clones() {
    const CATALOG_SIZE: usize = 10_000;

    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-catalog-readiness-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(
        root.join("config.json"),
        serde_json::json!({"schema_version": 3}).to_string(),
    )
    .expect("config");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let _ = catalog_generation(&runtime);
    let (generation, first_song_id) = {
        let mut catalog = runtime.catalog.lock().expect("catalog lock");
        let snapshot = catalog
            .index
            .replace_entries((0..CATALOG_SIZE).map(|index| {
                let path = format!("C:/catalog-readiness-v1/{index:05}.json");
                sky_app_core::catalog::CatalogSourceEntry::path_derived(
                    path,
                    format!("Ready Song {index:05}"),
                )
            }))
            .expect("synthetic catalog");
        catalog.library_membership = snapshot
            .items
            .iter()
            .map(|row| row.song_id.clone())
            .collect();
        let first_song_id = snapshot.items[0].song_id.clone();
        (snapshot.generation, first_song_id)
    };
    *runtime.catalog_load.lock().expect("catalog load lock") =
        CatalogLoadState::Ready { generation };
    runtime
        .analysis_cache
        .lock()
        .expect("analysis cache lock")
        .insert(
            super::super::AnalysisKey {
                song_id: first_song_id.clone(),
                generation,
                settings_revision: 0,
            },
            super::super::CachedSongAnalysis {
                metadata: super::super::CatalogMetadata {
                    format_label: "JSON".into(),
                    duration_us: Some(0),
                    note_count: Some(0),
                    risk_level: "low".into(),
                    state: "ready",
                },
                detail: None,
            },
        );

    let source = include_str!("../catalog.rs");
    let ensure_start = source
        .find("pub(super) fn ensure_catalog_loaded(")
        .expect("readiness function");
    let ensure_end = source[ensure_start..]
        .find("pub(super) fn reload(")
        .map(|offset| ensure_start + offset)
        .expect("next catalog method");
    let ensure_source = &source[ensure_start..ensure_end];
    assert!(ensure_source.contains("Result<u64, String>"));
    assert!(ensure_source.contains("return Ok(*generation);"));
    assert!(!ensure_source.contains(".snapshot()"));

    let search_request = serde_json::json!({
        "query": "",
        "offset": 0,
        "limit": 16,
        "generation": generation,
        "source": {"kind": "smart", "id": "all"}
    });
    let viewport_request = serde_json::json!({
        "generation": generation,
        "firstIndex": 0,
        "lastIndex": 0,
        "selectedSongId": null,
        "songIds": [first_song_id]
    });

    for state in [
        CatalogLoadState::Ready { generation },
        CatalogLoadState::Cached { generation },
        CatalogLoadState::Reconciling { generation },
    ] {
        let expected_state = state.clone();
        *runtime.catalog_load.lock().expect("catalog load lock") = state;
        let search = runtime
            .dispatch("catalog.search", search_request.clone())
            .expect("warmed high-cardinality search");
        assert_eq!(search["generation"], generation);
        assert_eq!(search["total"], CATALOG_SIZE);
        assert_eq!(search["items"].as_array().expect("search items").len(), 16);

        let viewport = runtime
            .dispatch("catalog.set_viewport", viewport_request.clone())
            .expect("warmed viewport");
        assert_eq!(viewport["generation"], generation);
        assert_eq!(viewport["items"][0]["metadata_state"], "ready");
        let repeated = runtime
            .dispatch("catalog.set_viewport", viewport_request.clone())
            .expect("repeated warmed viewport");
        assert_eq!(repeated["items"], viewport["items"]);
        assert_eq!(
            *runtime.catalog_load.lock().expect("catalog load lock"),
            expected_state
        );
    }

    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_playlist_import_commits_membership_and_import_reference_together() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-library-v1-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::create_dir_all(root.join("imports")).expect("imports root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(root.join("songs/primary.json"), "{}").expect("primary song");
    fs::write(root.join("imports/local.txt"), "notes").expect("import song");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let listed = runtime
        .dispatch("library.list_playlists", serde_json::json!({}))
        .expect("list playlists");
    assert_eq!(listed["playlists"].as_array().expect("playlists").len(), 0);
    let initial_generation = catalog_generation(&runtime);
    let primary_id = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query":"primary",
                "offset":0,
                "limit":10,
                "generation":initial_generation,
                "source":{"kind":"smart","id":"all"}
            }),
        )
        .expect("primary search")["items"][0]["song_id"]
        .as_str()
        .expect("primary song ID")
        .to_owned();

    let created = runtime
        .dispatch(
            "library.create_playlist",
            serde_json::json!({"name":"Practice"}),
        )
        .expect("create playlist");
    let playlist_id = created["id"].as_str().expect("playlist ID").to_owned();
    assert_eq!(created["name"], "Practice");
    runtime
        .dispatch(
            "library.rename_playlist",
            serde_json::json!({"playlistId":playlist_id,"name":"Morning"}),
        )
        .expect("rename playlist");
    runtime
        .dispatch(
            "library.add_songs",
            serde_json::json!({
                "playlistId":playlist_id,
                "songIds":[primary_id]
            }),
        )
        .expect("add song to playlist");
    let listed = runtime
        .dispatch("library.list_playlists", serde_json::json!({}))
        .expect("list updated playlists");
    assert_eq!(listed["playlists"][0]["name"], "Morning");
    assert_eq!(listed["playlists"][0]["song_count"], 1);
    assert!(listed["playlists"][0].get("song_ids").is_none());

    let imported = runtime
        .dispatch(
            "library.import_local_files_to_playlist",
            serde_json::json!({
                "playlistId":playlist_id,
                "paths":[root.join("imports/local.txt")]
            }),
        )
        .expect("import local file to playlist");
    assert_eq!(imported["imported_song_count"], 1);
    assert_eq!(imported["playlist"]["name"], "Morning");
    assert_eq!(imported["playlist"]["song_count"], 2);
    assert!(imported.get("canonical_path").is_none());

    let generation = imported["catalog_generation"].as_u64().expect("generation");
    let search = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query":"local",
                "offset":0,
                "limit":10,
                "generation":generation,
                "source":{"kind":"playlist","id":playlist_id}
            }),
        )
        .expect("search playlist local song");
    assert_eq!(search["total"], 1);

    let all_songs = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query":"local",
                "offset":0,
                "limit":10,
                "generation":generation,
                "source":{"kind":"smart","id":"all"}
            }),
        )
        .expect("search primary catalog");
    assert_eq!(all_songs["total"], 0);
    assert!(root.join("imports/local.txt").exists());
    runtime
        .dispatch(
            "library.delete_playlist",
            serde_json::json!({"playlistId":playlist_id}),
        )
        .expect("delete playlist");
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn native_missing_import_does_not_break_all_songs_or_navigation() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-library-missing-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(root.join("songs/primary.json"), "{}").expect("primary song");
    let missing_path = root.join("disconnected-drive");
    let source_id = "d".repeat(32);
    fs::write(
        root.join("library-manifest.json"),
        serde_json::json!({
            "version": 1,
            "imports": [{
                "source_id": source_id.clone(),
                "canonical_path": missing_path.to_string_lossy().to_string(),
                "kind": "folder"
            }],
            "collections": []
        })
        .to_string(),
    )
    .expect("missing import manifest");

    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
    let listed = runtime
        .dispatch("library.list_playlists", serde_json::json!({}))
        .expect("list playlists with missing import");
    assert_eq!(listed["playlists"].as_array().map(Vec::len), Some(0));
    assert!(
        !listed
            .to_string()
            .contains(&missing_path.to_string_lossy().to_string())
    );

    let generation = catalog_generation(&runtime);
    let all_songs = runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({
                "query":"",
                "offset":0,
                "limit":10,
                "generation":generation,
                "source":{"kind":"smart","id":"all"}
            }),
        )
        .expect("all songs search");
    assert_eq!(all_songs["total"], 1);
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn settings_and_catalog_changes_invalidate_prepared_native_playback() {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("sky-native-state-{suffix}"));
    fs::create_dir_all(root.join("songs")).expect("songs root");
    fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
    fs::write(
        root.join("songs/demo.json"),
        r#"{"name":"Demo","songNotes":[{"time":0,"key":"Key0"}]}"#,
    )
    .expect("song");
    let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");
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
    let prepared = runtime
            .dispatch(
                "playback.prepare",
                serde_json::json!({
                    "songId": song_id,
                    "generation": generation,
                    "config": {"hold_frames":1.0,"timing_margin_us":800,"tempo_scale":1.0,"fps":60,"dry_run":true}
                }),
            )
            .expect("prepare");
    let prepared_id = prepared["prepared_id"]
        .as_str()
        .expect("prepared ID")
        .to_owned();
    runtime
        .patch_settings(crate::commands::SettingsPatch {
            appearance_mode: None,
            palette: None,
            telemetry_enabled: None,
            verbose_hud: None,
            playback_defaults: Some(crate::commands::PlaybackPatch {
                hold_frames: None,
                timing_margin_us: None,
                tempo_scale: Some(0.95),
                fps: None,
            }),
            auto_play: None,
            update_preferences: None,
        })
        .expect("native settings invalidation seam");
    assert!(
        runtime
            .dispatch(
                "playback.start",
                serde_json::json!({"preparedId":prepared_id,"decisions":[]}),
            )
            .is_err()
    );

    let prepared = runtime
            .dispatch(
                "playback.prepare",
                serde_json::json!({
                    "songId": song_id,
                    "generation": generation,
                    "config": {"hold_frames":1.0,"timing_margin_us":800,"tempo_scale":1.0,"fps":60,"dry_run":true}
                }),
            )
            .expect("prepare after settings patch");
    let prepared_id = prepared["prepared_id"]
        .as_str()
        .expect("prepared ID")
        .to_owned();
    runtime
        .dispatch("catalog.reload", Value::Object(Default::default()))
        .expect("reload");
    assert!(
        runtime
            .dispatch(
                "playback.start",
                serde_json::json!({"preparedId":prepared_id,"decisions":[]}),
            )
            .is_err()
    );
    runtime.shutdown();
    let _ = fs::remove_dir_all(root);
}
