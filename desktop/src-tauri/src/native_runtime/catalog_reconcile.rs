use super::NativeDesktopRuntime;
use sky_app_core::catalog::{CatalogSourceIdentity, song_id_for_canonical_path};
use sky_native_adapters::{
    BuiltinCatalogStatus, CATALOG_CACHE_SCHEMA_VERSION, CatalogCache, CatalogCacheEntry,
    CatalogCacheSource, CatalogCacheSourceKind, CatalogComposition,
};
use std::collections::HashMap;
use std::fs;
use std::time::UNIX_EPOCH;

impl NativeDesktopRuntime {
    pub(super) fn catalog_cache_from_composition(
        &self,
        composition: &CatalogComposition,
        source_fingerprint: &str,
    ) -> Result<CatalogCache, String> {
        // Size and mtime are persisted for cache metadata and diagnostics only.
        // Reconciliation always comes from a complete CatalogComposer pass, so
        // a same-size, same-mtime content mutation cannot be hidden by a
        // metadata-only validity shortcut.
        let mut sources = vec![
            CatalogCacheSource {
                source_id: "builtin".into(),
                kind: CatalogCacheSourceKind::Builtin,
                import_kind: None,
                display_name: "Built-in library".into(),
                available: matches!(
                    composition.builtin_status,
                    BuiltinCatalogStatus::Available { .. }
                ),
                song_ids: composition.builtin_membership.iter().cloned().collect(),
            },
            CatalogCacheSource {
                source_id: "user".into(),
                kind: CatalogCacheSourceKind::User,
                import_kind: None,
                display_name: "User library".into(),
                available: true,
                song_ids: composition.user_membership.iter().cloned().collect(),
            },
        ];
        for status in &composition.imported_status {
            sources.push(CatalogCacheSource {
                source_id: status.source_id.clone(),
                kind: CatalogCacheSourceKind::Imported,
                import_kind: Some(status.kind),
                display_name: status.display_name.clone(),
                available: status.available,
                song_ids: composition
                    .imported_membership
                    .get(&status.source_id)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
            });
        }
        let previous_cache = self.cached_catalog_snapshot()?;
        let previous_entries_by_identity_path = previous_cache.as_ref().map(|cache| {
            let mut entries = HashMap::with_capacity(cache.entries.len());
            for cached in &cache.entries {
                entries
                    .entry((cached.song_id.clone(), cached.canonical_path.clone()))
                    .or_insert(cached);
            }
            entries
        });
        let mut entries = Vec::with_capacity(composition.entries.len());
        for entry in &composition.entries {
            let song_id = match &entry.identity {
                CatalogSourceIdentity::PathDerived => {
                    song_id_for_canonical_path(&entry.canonical_path)
                }
                CatalogSourceIdentity::Stable(song_id) => song_id.as_str().to_owned(),
            };
            let (source_id, kind) = cache_source_for_song(composition, &song_id);
            let previous_entry = previous_entries_by_identity_path
                .as_ref()
                .and_then(|cache| {
                    cache
                        .get(&(song_id.clone(), entry.canonical_path.clone()))
                        .copied()
                });
            let metadata = fs::metadata(&entry.canonical_path).ok();
            let (file_size, modified_unix_ms) = if let Some(metadata) = metadata {
                let modified_unix_ms = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
                    .or_else(|| previous_entry.map(|cached| cached.modified_unix_ms))
                    .ok_or_else(|| "catalog cache metadata timestamp unavailable".to_string())?;
                (metadata.len(), modified_unix_ms)
            } else if let Some(previous_entry) = previous_entry {
                (previous_entry.file_size, previous_entry.modified_unix_ms)
            } else {
                return Err("catalog cache metadata unavailable".into());
            };
            entries.push(CatalogCacheEntry {
                source_id,
                kind,
                canonical_path: entry.canonical_path.clone(),
                song_id,
                title: entry.title.clone(),
                file_size,
                modified_unix_ms,
            });
        }
        let cache_generation = self
            .cached_catalog
            .lock()
            .ok()
            .and_then(|cache| {
                cache
                    .as_ref()
                    .map(|cache| cache.cache_generation.saturating_add(1).max(1))
            })
            .unwrap_or(1);
        Ok(CatalogCache {
            schema_version: CATALOG_CACHE_SCHEMA_VERSION,
            cache_generation,
            source_fingerprint: source_fingerprint.to_owned(),
            sources,
            entries,
        })
    }

    pub(super) fn cached_catalog_snapshot(&self) -> Result<Option<CatalogCache>, String> {
        self.cached_catalog
            .lock()
            .map_err(|_| "native catalog cache lock poisoned".to_string())
            .map(|cache| cache.clone())
    }
}

fn cache_source_for_song(
    composition: &CatalogComposition,
    song_id: &str,
) -> (String, CatalogCacheSourceKind) {
    if composition.builtin_membership.contains(song_id) {
        return ("builtin".into(), CatalogCacheSourceKind::Builtin);
    }
    if composition.user_membership.contains(song_id) {
        return ("user".into(), CatalogCacheSourceKind::User);
    }
    if let Some(source_id) = composition
        .imported_membership
        .iter()
        .filter_map(|(source_id, song_ids)| song_ids.contains(song_id).then_some(source_id))
        .min()
    {
        return (source_id.clone(), CatalogCacheSourceKind::Imported);
    }
    ("user".into(), CatalogCacheSourceKind::User)
}

#[cfg(test)]
mod package_b_benchmark_tests {
    use super::*;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    #[test]
    #[ignore = "opt-in Package B catalog reconciliation benchmark"]
    fn opt_in_package_b_catalog_reconciliation_benchmark() {
        fn report(size: usize, values: &mut [f64]) {
            values.sort_by(f64::total_cmp);
            println!(
                "package_b_reconcile size={size} samples={} p50_us={:.3} p90_us={:.3} min_us={:.3} max_us={:.3}",
                values.len(),
                values[values.len() / 2],
                values[(values.len() * 9 / 10).min(values.len() - 1)],
                values[0],
                values[values.len() - 1]
            );
        }

        println!(
            "package_b_bench layer=native-cache-reconcile fixture=package-b-reconcile-paths-v1 seed=0x560 size_set=96,1000,10000 profile=release warmup=1 samples=7"
        );
        for size in [96_usize, 1_000, 10_000] {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let root =
                std::env::temp_dir().join(format!("sky-package-b-reconcile-{size}-{suffix}"));
            fs::create_dir_all(root.join("songs")).expect("songs root");
            fs::write(root.join("config.json"), "{\"schema_version\":3}\n").expect("config");
            let runtime = NativeDesktopRuntime::from_install_root(root.clone()).expect("runtime");

            let mut composition = CatalogComposition::default();
            let mut previous_entries = Vec::with_capacity(size);
            let mut user_ids = Vec::with_capacity(size);
            for index in 0..size {
                let path = format!("C:\\package-b-reconcile-v1\\{index:05}.json");
                let song_id = song_id_for_canonical_path(&path);
                composition
                    .entries
                    .push(sky_app_core::catalog::CatalogSourceEntry::path_derived(
                        path.clone(),
                        format!("Reconcile Song {index:05}"),
                    ));
                composition.user_membership.insert(song_id.clone());
                composition.library_membership.insert(song_id.clone());
                user_ids.push(song_id.clone());
                previous_entries.push(CatalogCacheEntry {
                    source_id: "user".into(),
                    kind: CatalogCacheSourceKind::User,
                    canonical_path: path,
                    song_id,
                    title: format!("Reconcile Song {index:05}"),
                    file_size: 32,
                    modified_unix_ms: 1,
                });
            }
            let previous_cache = CatalogCache {
                schema_version: CATALOG_CACHE_SCHEMA_VERSION,
                cache_generation: 1,
                source_fingerprint: "a".repeat(64),
                sources: vec![
                    CatalogCacheSource {
                        source_id: "builtin".into(),
                        kind: CatalogCacheSourceKind::Builtin,
                        import_kind: None,
                        display_name: "Built-in library".into(),
                        available: false,
                        song_ids: Vec::new(),
                    },
                    CatalogCacheSource {
                        source_id: "user".into(),
                        kind: CatalogCacheSourceKind::User,
                        import_kind: None,
                        display_name: "User library".into(),
                        available: true,
                        song_ids: user_ids,
                    },
                ],
                entries: previous_entries,
            };
            *runtime.cached_catalog.lock().expect("cached catalog") = Some(previous_cache.clone());

            let source_fingerprint = "b".repeat(64);
            let measure = || {
                std::hint::black_box(
                    runtime
                        .catalog_cache_from_composition(&composition, &source_fingerprint)
                        .expect("catalog cache composition"),
                );
            };
            measure();
            let mut samples = Vec::with_capacity(7);
            for _ in 0..7 {
                let started = Instant::now();
                measure();
                samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);
            }
            report(size, &mut samples);
            *runtime
                .cached_catalog
                .lock()
                .expect("restore cached catalog") = Some(previous_cache);
            runtime.shutdown();
            fs::remove_dir_all(&root).expect("remove benchmark fixture");
        }
    }
}
