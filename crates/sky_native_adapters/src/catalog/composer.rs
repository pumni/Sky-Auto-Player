use super::builtin_source::{BuiltinCatalogSource, BuiltinCatalogStatus};
use super::file_source::{
    FileCatalogSource, entries_from_directory, entries_from_directory_with_metrics,
    entries_from_file, entries_from_file_with_metrics, import_display_name,
};
use sky_app_core::catalog::{
    CatalogError, CatalogSourceEntry, SongSource, song_id_for_canonical_path,
};
use sky_app_core::library::{ImportedSourceKind, ImportedSourceRef};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Instant;
#[derive(Debug, Clone, Default)]
pub struct CatalogComposition {
    pub entries: Vec<CatalogSourceEntry>,
    pub library_membership: BTreeSet<String>,
    pub builtin_membership: BTreeSet<String>,
    pub user_membership: BTreeSet<String>,
    pub imported_membership: HashMap<String, BTreeSet<String>>,
    pub imported_status: Vec<ImportedSourceCatalogStatus>,
    pub builtin_status: BuiltinCatalogStatus,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CatalogScanMetrics {
    pub directories_visited: u64,
    pub files_visited: u64,
    pub supported_files: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogCompositionSource {
    Builtin,
    User,
    Imported { index: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogCompositionEvent {
    SourceStarted(CatalogCompositionSource),
    SourceFinished {
        source: CatalogCompositionSource,
        metrics: CatalogScanMetrics,
        duration_ms: u64,
    },
}

pub struct CatalogComposer {
    builtin: BuiltinCatalogSource,
    user: FileCatalogSource,
}

impl CatalogComposer {
    pub fn new(builtin: BuiltinCatalogSource, user: FileCatalogSource) -> Self {
        Self { builtin, user }
    }

    pub fn compose(
        &self,
        imports: &[ImportedSourceRef],
    ) -> Result<CatalogComposition, CatalogError> {
        let (builtin_entries, builtin_membership, builtin_status) = self.builtin.load();
        let user_entries = self.user.entries()?;
        let user_membership = user_entries
            .iter()
            .map(|entry| song_id_for_canonical_path(&entry.canonical_path))
            .collect::<BTreeSet<_>>();
        let mut composition = CatalogComposition {
            entries: builtin_entries,
            builtin_membership: builtin_membership.clone(),
            user_membership: user_membership.clone(),
            library_membership: builtin_membership
                .union(&user_membership)
                .cloned()
                .collect(),
            builtin_status,
            ..Default::default()
        };
        composition.entries.extend(user_entries);
        for import in imports {
            let path = PathBuf::from(&import.canonical_path);
            let display_name = import_display_name(&path, import.kind);
            if !path.exists() {
                record_missing_import(&mut composition, import, display_name);
                continue;
            }
            let imported = match import.kind {
                ImportedSourceKind::File => entries_from_file(&path),
                ImportedSourceKind::Folder => entries_from_directory(&path, true),
            };
            let imported = match imported {
                Ok(entries) => entries,
                Err(_) => {
                    record_missing_import(&mut composition, import, display_name);
                    continue;
                }
            };
            let membership = imported
                .iter()
                .map(|entry| song_id_for_canonical_path(&entry.canonical_path))
                .collect::<BTreeSet<_>>();
            composition
                .imported_status
                .push(ImportedSourceCatalogStatus {
                    source_id: import.source_id.clone(),
                    kind: import.kind,
                    display_name,
                    song_count: membership.len(),
                    available: true,
                });
            composition
                .imported_membership
                .insert(import.source_id.clone(), membership);
            composition.entries.extend(imported);
        }
        composition
            .entries
            .sort_by(|left, right| left.canonical_path.cmp(&right.canonical_path));
        Ok(composition)
    }

    pub fn compose_with_observer<F>(
        &self,
        imports: &[ImportedSourceRef],
        mut observer: F,
    ) -> Result<CatalogComposition, CatalogError>
    where
        F: FnMut(CatalogCompositionEvent),
    {
        self.compose_internal(imports, Some(&mut observer))
    }

    fn compose_internal(
        &self,
        imports: &[ImportedSourceRef],
        mut observer: Option<&mut dyn FnMut(CatalogCompositionEvent)>,
    ) -> Result<CatalogComposition, CatalogError> {
        let source_started = observer.is_some().then(Instant::now);
        notify_composition_observer(
            &mut observer,
            CatalogCompositionEvent::SourceStarted(CatalogCompositionSource::Builtin),
        );
        let (builtin_entries, builtin_membership, builtin_status) = self.builtin.load();
        let builtin_metrics = if observer.is_some() {
            CatalogScanMetrics {
                directories_visited: u64::from(
                    self.builtin.resources().builtin_catalog_root().is_dir(),
                ),
                files_visited: builtin_entries.len() as u64,
                supported_files: builtin_entries.len() as u64,
            }
        } else {
            CatalogScanMetrics::default()
        };
        notify_composition_observer(
            &mut observer,
            CatalogCompositionEvent::SourceFinished {
                source: CatalogCompositionSource::Builtin,
                metrics: builtin_metrics,
                duration_ms: elapsed_ms(source_started),
            },
        );
        notify_composition_observer(
            &mut observer,
            CatalogCompositionEvent::SourceStarted(CatalogCompositionSource::User),
        );
        let source_started = observer.is_some().then(Instant::now);
        let (user_entries, user_metrics) = if observer.is_some() {
            match self.user.entries_with_metrics() {
                Ok(result) => result,
                Err(error) => {
                    notify_composition_observer(
                        &mut observer,
                        CatalogCompositionEvent::SourceFinished {
                            source: CatalogCompositionSource::User,
                            metrics: CatalogScanMetrics::default(),
                            duration_ms: elapsed_ms(source_started),
                        },
                    );
                    return Err(error);
                }
            }
        } else {
            (self.user.entries()?, CatalogScanMetrics::default())
        };
        notify_composition_observer(
            &mut observer,
            CatalogCompositionEvent::SourceFinished {
                source: CatalogCompositionSource::User,
                metrics: user_metrics,
                duration_ms: elapsed_ms(source_started),
            },
        );
        let user_membership = user_entries
            .iter()
            .map(|entry| song_id_for_canonical_path(&entry.canonical_path))
            .collect::<BTreeSet<_>>();
        let mut composition = CatalogComposition {
            entries: builtin_entries,
            builtin_membership: builtin_membership.clone(),
            user_membership: user_membership.clone(),
            library_membership: builtin_membership
                .union(&user_membership)
                .cloned()
                .collect(),
            builtin_status,
            ..Default::default()
        };
        composition.entries.extend(user_entries);
        for (index, import) in imports.iter().enumerate() {
            let source = CatalogCompositionSource::Imported { index };
            let source_started = observer.is_some().then(Instant::now);
            notify_composition_observer(
                &mut observer,
                CatalogCompositionEvent::SourceStarted(source),
            );
            let path = PathBuf::from(&import.canonical_path);
            let display_name = import_display_name(&path, import.kind);
            if !path.exists() {
                record_missing_import(&mut composition, import, display_name);
                notify_composition_observer(
                    &mut observer,
                    CatalogCompositionEvent::SourceFinished {
                        source,
                        metrics: CatalogScanMetrics::default(),
                        duration_ms: elapsed_ms(source_started),
                    },
                );
                continue;
            }
            let imported = match (import.kind, observer.is_some()) {
                (ImportedSourceKind::File, true) => entries_from_file_with_metrics(&path),
                (ImportedSourceKind::Folder, true) => {
                    entries_from_directory_with_metrics(&path, true)
                }
                (ImportedSourceKind::File, false) => {
                    entries_from_file(&path).map(|entries| (entries, CatalogScanMetrics::default()))
                }
                (ImportedSourceKind::Folder, false) => entries_from_directory(&path, true)
                    .map(|entries| (entries, CatalogScanMetrics::default())),
            };
            let (imported, metrics) = match imported {
                Ok(result) => result,
                Err(_) => {
                    record_missing_import(&mut composition, import, display_name);
                    notify_composition_observer(
                        &mut observer,
                        CatalogCompositionEvent::SourceFinished {
                            source,
                            metrics: CatalogScanMetrics::default(),
                            duration_ms: elapsed_ms(source_started),
                        },
                    );
                    continue;
                }
            };
            let membership = imported
                .iter()
                .map(|entry| song_id_for_canonical_path(&entry.canonical_path))
                .collect::<BTreeSet<_>>();
            composition
                .imported_status
                .push(ImportedSourceCatalogStatus {
                    source_id: import.source_id.clone(),
                    kind: import.kind,
                    display_name,
                    song_count: membership.len(),
                    available: true,
                });
            composition
                .imported_membership
                .insert(import.source_id.clone(), membership);
            composition.entries.extend(imported);
            notify_composition_observer(
                &mut observer,
                CatalogCompositionEvent::SourceFinished {
                    source,
                    metrics,
                    duration_ms: elapsed_ms(source_started),
                },
            );
        }
        composition
            .entries
            .sort_by(|left, right| left.canonical_path.cmp(&right.canonical_path));
        Ok(composition)
    }
}

fn elapsed_ms(started: Option<Instant>) -> u64 {
    started
        .map(|started| started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn notify_composition_observer(
    observer: &mut Option<&mut dyn FnMut(CatalogCompositionEvent)>,
    event: CatalogCompositionEvent,
) {
    if let Some(observer) = observer.as_deref_mut() {
        observer(event);
    }
}

fn record_missing_import(
    composition: &mut CatalogComposition,
    import: &ImportedSourceRef,
    display_name: String,
) {
    composition
        .imported_status
        .push(ImportedSourceCatalogStatus {
            source_id: import.source_id.clone(),
            kind: import.kind,
            display_name,
            song_count: 0,
            available: false,
        });
    composition
        .imported_membership
        .insert(import.source_id.clone(), BTreeSet::new());
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSourceCatalogStatus {
    pub source_id: String,
    pub kind: ImportedSourceKind,
    pub display_name: String,
    pub song_count: usize,
    pub available: bool,
}
