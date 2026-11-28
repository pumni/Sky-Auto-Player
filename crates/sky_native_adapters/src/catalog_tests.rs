use crate::{
    AppResources, BuiltinCatalogFailureCode, BuiltinCatalogSource, BuiltinCatalogStatus,
    CatalogComposer, CatalogCompositionEvent, FileCatalogSource, JsonLibraryManifestStore,
};
use sky_app_core::catalog::{BuiltinCatalogManifest, CatalogError, SongSource};
use sky_app_core::library::{
    ImportedSourceKind, ImportedSourceRef, LIBRARY_MANIFEST_VERSION, LibraryManifestStore,
    LibraryManifestV1,
};
use std::fs;

#[test]
fn catalog_source_handles_missing_directory_as_empty() {
    let source = FileCatalogSource::new(std::env::temp_dir().join("sky-missing-catalog-directory"));
    assert!(source.entries().expect("missing root").is_empty());
}

#[test]
fn catalog_source_rejects_non_directory_and_ignores_unsupported_files() {
    let root = std::env::temp_dir().join(format!("sky-catalog-{}", std::process::id()));
    fs::create_dir_all(&root).expect("temp root");
    fs::write(root.join("song.txt"), "notes").expect("song");
    fs::write(root.join("ignored.csv"), "ignored").expect("ignored");
    let entries = FileCatalogSource::new(&root).entries().expect("entries");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].canonical_path.ends_with("song.txt"));
    let file = root.join("not-a-directory");
    fs::write(&file, "file").expect("file");
    assert!(matches!(
        FileCatalogSource::new(file).entries(),
        Err(CatalogError::SourceUnavailable(_))
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn library_manifest_round_trips_and_imported_folders_are_composed() {
    let root = std::env::temp_dir().join(format!(
        "sky-library-manifest-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let songs = root.join("songs");
    let imported = root.join("imported/nested");
    fs::create_dir_all(&songs).expect("songs");
    fs::create_dir_all(&imported).expect("imported");
    fs::write(songs.join("primary.json"), "{}").expect("primary");
    fs::write(imported.join("local.txt"), "notes").expect("imported song");
    fs::write(imported.join("second.skysheet"), "notes").expect("second imported song");
    fs::write(imported.join("ignored.csv"), "ignored").expect("unsupported import");

    let canonical = fs::canonicalize(root.join("imported")).expect("canonical folder");
    let manifest = LibraryManifestV1 {
        version: LIBRARY_MANIFEST_VERSION,
        imports: vec![ImportedSourceRef {
            source_id: "a".repeat(32),
            canonical_path: canonical.to_string_lossy().into_owned(),
            kind: ImportedSourceKind::Folder,
        }],
        collections: Vec::new(),
    };
    let store = JsonLibraryManifestStore::new(root.join("library-manifest.json"));
    store.save(&manifest).expect("manifest save");
    assert_eq!(store.load().expect("manifest load"), manifest);

    let composer = CatalogComposer::new(
        BuiltinCatalogSource::new(AppResources::from_builtin_catalog_root(
            root.join("missing-builtin"),
        )),
        FileCatalogSource::new(&songs),
    );
    let entries = composer
        .compose(&manifest.imports)
        .expect("composed entries")
        .entries;
    assert_eq!(entries.len(), 3);
    assert!(
        entries
            .iter()
            .any(|entry| entry.canonical_path.ends_with("local.txt"))
    );

    let composition = CatalogComposer::new(
        BuiltinCatalogSource::new(AppResources::from_builtin_catalog_root(
            root.join("missing-builtin"),
        )),
        FileCatalogSource::new(&songs),
    )
    .compose(&[
        manifest.imports[0].clone(),
        ImportedSourceRef {
            source_id: "b".repeat(32),
            canonical_path: root.join("disconnected").to_string_lossy().into_owned(),
            kind: ImportedSourceKind::Folder,
        },
    ])
    .expect("catalog composition");
    assert_eq!(composition.library_membership.len(), 1);
    assert_eq!(composition.imported_status.len(), 2);
    assert_eq!(composition.imported_status[0].song_count, 2);
    assert!(composition.imported_status[0].available);
    assert_eq!(composition.imported_membership[&"a".repeat(32)].len(), 2);
    assert_eq!(composition.imported_status[1].song_count, 0);
    assert!(!composition.imported_status[1].available);
    assert!(composition.imported_membership[&"b".repeat(32)].is_empty());

    let duplicate = CatalogComposer::new(
        BuiltinCatalogSource::new(AppResources::from_builtin_catalog_root(
            root.join("missing-builtin"),
        )),
        FileCatalogSource::new(&songs),
    )
    .compose(&[ImportedSourceRef {
        source_id: "c".repeat(32),
        canonical_path: songs.to_string_lossy().into_owned(),
        kind: ImportedSourceKind::Folder,
    }])
    .expect("duplicate composition");
    assert_eq!(duplicate.entries.len(), 2);
    assert_eq!(duplicate.imported_status[0].song_count, 1);
    assert_eq!(duplicate.imported_membership[&"c".repeat(32)].len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn catalog_observer_composition_matches_production_composition() {
    let root = std::env::temp_dir().join(format!(
        "sky-catalog-observer-parity-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let builtin_root = root.join("builtin-songs");
    let builtin_sheets = builtin_root.join("sheets");
    let user_root = root.join("user-songs");
    let imported_root = root.join("imported");
    fs::create_dir_all(&builtin_sheets).expect("builtin sheets");
    fs::create_dir_all(&user_root).expect("user songs");
    fs::create_dir_all(&imported_root).expect("imported songs");

    let builtin_song = builtin_sheets.join("builtin.json");
    fs::write(
        &builtin_song,
        br#"{"name":"Builtin","songNotes":[{"time":0,"key":"1Key0"}]}"#,
    )
    .expect("builtin song");
    let builtin_manifest = BuiltinCatalogManifest {
        schema_version: 1,
        songs: vec![sky_app_core::catalog::BuiltinSongManifestEntry {
            id: "0123456789abcdef0123456789abcdef".into(),
            path: "sheets/builtin.json".into(),
            title: "Builtin".into(),
            sha256: sha256_bytes(&fs::read(&builtin_song).expect("read builtin song")),
        }],
        retired_songs: Vec::new(),
    };
    fs::write(
        builtin_root.join("manifest.json"),
        serde_json::to_vec(&builtin_manifest).expect("builtin manifest JSON"),
    )
    .expect("builtin manifest");
    fs::write(user_root.join("user.json"), "{}").expect("user song");
    let imported_song = imported_root.join("imported.txt");
    fs::write(&imported_song, "notes").expect("imported song");

    let imports = vec![
        ImportedSourceRef {
            source_id: "a".repeat(32),
            canonical_path: fs::canonicalize(&imported_song)
                .expect("canonical imported song")
                .to_string_lossy()
                .into_owned(),
            kind: ImportedSourceKind::File,
        },
        ImportedSourceRef {
            source_id: "b".repeat(32),
            canonical_path: root.join("unavailable").to_string_lossy().into_owned(),
            kind: ImportedSourceKind::Folder,
        },
    ];
    let composer = CatalogComposer::new(
        BuiltinCatalogSource::new(AppResources::from_builtin_catalog_root(builtin_root)),
        FileCatalogSource::new(user_root),
    );

    let production = composer.compose(&imports).expect("production composition");
    let mut events = Vec::new();
    let observed = composer
        .compose_with_observer(&imports, |event| events.push(event))
        .expect("observed composition");

    assert_eq!(production.entries, observed.entries);
    assert_eq!(production.library_membership, observed.library_membership);
    assert_eq!(production.builtin_membership, observed.builtin_membership);
    assert_eq!(production.user_membership, observed.user_membership);
    assert_eq!(production.imported_membership, observed.imported_membership);
    assert_eq!(production.imported_status, observed.imported_status);
    assert_eq!(production.builtin_status, observed.builtin_status);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, CatalogCompositionEvent::SourceStarted(_)))
            .count(),
        4
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, CatalogCompositionEvent::SourceFinished { .. }))
            .count(),
        4
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn builtin_source_is_manifest_driven_and_fails_as_one_source() {
    let root = std::env::temp_dir().join(format!("sky-v4-builtin-{}", std::process::id()));
    let resource_root = root.join("builtin-songs");
    let sheets = resource_root.join("sheets");
    let user_root = root.join("user-songs");
    fs::create_dir_all(&sheets).expect("sheets");
    fs::create_dir_all(&user_root).expect("user songs");
    let builtin_path = sheets.join("stable.json");
    fs::write(
        &builtin_path,
        br#"{"name":"Stable","songNotes":[{"time":0,"key":"1Key0"}]}"#,
    )
    .expect("builtin song");
    fs::write(
        user_root.join("user.json"),
        br#"{"name":"User","songNotes":[{"time":0,"key":"1Key1"}]}"#,
    )
    .expect("user song");
    let digest = sha256_bytes(&fs::read(&builtin_path).expect("read builtin"));
    let manifest = BuiltinCatalogManifest {
        schema_version: 1,
        songs: vec![sky_app_core::catalog::BuiltinSongManifestEntry {
            id: "0123456789abcdef0123456789abcdef".into(),
            path: "sheets/stable.json".into(),
            title: "Stable".into(),
            sha256: digest,
        }],
        retired_songs: Vec::new(),
    };
    fs::write(
        resource_root.join("manifest.json"),
        serde_json::to_vec(&manifest).expect("manifest json"),
    )
    .expect("manifest");
    let composition = CatalogComposer::new(
        BuiltinCatalogSource::new(AppResources::from_builtin_catalog_root(resource_root)),
        FileCatalogSource::new(&user_root),
    )
    .compose(&[])
    .expect("composition");
    assert_eq!(composition.builtin_membership.len(), 1);
    assert_eq!(composition.user_membership.len(), 1);
    assert_eq!(composition.library_membership.len(), 2);
    assert_eq!(composition.entries.len(), 2);
    assert_eq!(
        composition.builtin_status,
        BuiltinCatalogStatus::Available { song_count: 1 }
    );

    let missing_resource_root = root.join("missing-resource-builtin");
    fs::create_dir_all(&missing_resource_root).expect("missing resource root");
    let mut missing_manifest = manifest;
    missing_manifest.songs[0].path = "sheets/missing.json".into();
    fs::write(
        missing_resource_root.join("manifest.json"),
        serde_json::to_vec(&missing_manifest).expect("missing resource manifest json"),
    )
    .expect("missing resource manifest");
    let missing = CatalogComposer::new(
        BuiltinCatalogSource::new(AppResources::from_builtin_catalog_root(
            missing_resource_root,
        )),
        FileCatalogSource::new(&user_root),
    )
    .compose(&[])
    .expect("missing built-in source must not break user composition");
    assert_eq!(missing.entries.len(), 1);
    assert!(missing.builtin_membership.is_empty());
    assert_eq!(
        missing.builtin_status,
        BuiltinCatalogStatus::Unavailable {
            code: BuiltinCatalogFailureCode::MissingResource
        }
    );
    let _ = fs::remove_dir_all(&root);
}

fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
