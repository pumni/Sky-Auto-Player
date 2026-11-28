use crate::commands::{
    LibraryCreatePlaylistRequest, LibraryNavigationDto, LibraryPlaylistIdRequest,
    LibraryPlaylistImportResultDto, LibraryPlaylistSongsRequest, LibraryPlaylistSummaryDto,
    LibraryRenamePlaylistRequest,
};
use crate::native_runtime::catalog::catalog_error;
use crate::native_runtime::{NativeDesktopRuntime, NativeLibraryImportRequest, opaque_native_id};
use sha2::{Digest, Sha256};
use sky_app_core::library::{ImportedSourceKind, ImportedSourceRef, LibraryError};
use sky_native_adapters::BuiltinCatalogStatus;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

impl NativeDesktopRuntime {
    fn invalidate_playlist_search_orders(&self) -> Result<(), String> {
        self.catalog_search_cache
            .lock()
            .map_err(|_| "native catalog search cache lock poisoned".to_string())?
            .invalidate_playlists();
        self.invalidate_viewport_projection();
        Ok(())
    }
    pub(in crate::native_runtime) fn list_playlists(&self) -> Result<LibraryNavigationDto, String> {
        self.ensure_catalog_loaded()?;
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let manifest = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?;
        let generation = catalog.index.generation();
        let builtin_available = matches!(
            &catalog.builtin_status,
            BuiltinCatalogStatus::Available { .. }
        );
        let builtin_song_count = if builtin_available {
            catalog
                .index
                .count_allowed_ids(&catalog.builtin_membership, Some(generation))
                .map_err(catalog_error)? as u64
        } else {
            0
        };
        let playlists = manifest
            .snapshot()
            .collections
            .iter()
            .map(|collection| {
                let ids = collection.song_ids.iter().cloned().collect::<BTreeSet<_>>();
                LibraryPlaylistSummaryDto {
                    id: collection.id.clone(),
                    name: collection.name.clone(),
                    song_count: catalog
                        .index
                        .count_allowed_ids(&ids, Some(generation))
                        .unwrap_or_default() as u64,
                }
            })
            .collect();
        Ok(LibraryNavigationDto {
            builtin_available,
            builtin_song_count,
            playlists,
        })
    }

    fn playlist_summary(
        &self,
        collection: sky_app_core::library::Collection,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        self.ensure_catalog_loaded()?;
        let catalog = self
            .catalog
            .lock()
            .map_err(|_| "native catalog lock poisoned".to_string())?;
        let ids = collection.song_ids.iter().cloned().collect::<BTreeSet<_>>();
        Ok(LibraryPlaylistSummaryDto {
            id: collection.id,
            name: collection.name,
            song_count: catalog
                .index
                .count_allowed_ids(&ids, Some(catalog.index.generation()))
                .map_err(catalog_error)? as u64,
        })
    }

    pub(in crate::native_runtime) fn create_playlist(
        &self,
        request: LibraryCreatePlaylistRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let id = opaque_native_id()?;
        let mut manifest = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?;
        let collection = manifest
            .create_collection(id, request.name)
            .map_err(library_error)?;
        drop(manifest);
        self.playlist_summary(collection)
    }

    pub(in crate::native_runtime) fn rename_playlist(
        &self,
        request: LibraryRenamePlaylistRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let collection = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?
            .rename_collection(&request.playlist_id, request.name)
            .map_err(library_error)?;
        self.playlist_summary(collection)
    }

    pub(in crate::native_runtime) fn delete_playlist(
        &self,
        request: LibraryPlaylistIdRequest,
    ) -> Result<bool, String> {
        let mut manifest = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?;
        let deleted = manifest
            .delete_collection(&request.playlist_id)
            .map_err(library_error)?;
        if deleted {
            self.invalidate_playlist_search_orders()?;
        }
        Ok(deleted)
    }

    pub(in crate::native_runtime) fn add_playlist_songs(
        &self,
        request: LibraryPlaylistSongsRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let mut manifest = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?;
        let collection = manifest
            .add_songs(&request.playlist_id, &request.song_ids)
            .map_err(library_error)?;
        self.invalidate_playlist_search_orders()?;
        drop(manifest);
        self.playlist_summary(collection)
    }

    pub(in crate::native_runtime) fn remove_playlist_songs(
        &self,
        request: LibraryPlaylistSongsRequest,
    ) -> Result<LibraryPlaylistSummaryDto, String> {
        let mut manifest = self
            .library_manifest
            .lock()
            .map_err(|_| "native library manifest lock poisoned".to_string())?;
        let collection = manifest
            .remove_songs(&request.playlist_id, &request.song_ids)
            .map_err(library_error)?;
        self.invalidate_playlist_search_orders()?;
        drop(manifest);
        self.playlist_summary(collection)
    }

    pub(in crate::native_runtime) fn import_paths(
        &self,
        request: NativeLibraryImportRequest,
        kind: ImportedSourceKind,
    ) -> Result<LibraryPlaylistImportResultDto, String> {
        const MAX_IMPORT_SELECTIONS: usize = 1_024;
        if request.paths.len() > MAX_IMPORT_SELECTIONS {
            return Err("invalid_params: import selection is too large".into());
        }

        let mut imports = Vec::with_capacity(request.paths.len());
        for raw_path in request.paths {
            let path = PathBuf::from(raw_path);
            let canonical = fs::canonicalize(&path)
                .map_err(|_| "import source cannot be resolved".to_owned())?;
            let valid = match kind {
                ImportedSourceKind::File => {
                    canonical.is_file() && is_supported_catalog_path(&canonical)
                }
                ImportedSourceKind::Folder => canonical.is_dir(),
            };
            if !valid {
                return Err("invalid_params: import source has an unsupported type".into());
            }
            let canonical_path = canonical.to_string_lossy().into_owned();
            imports.push(ImportedSourceRef {
                source_id: imported_source_id(&canonical_path, kind),
                canonical_path,
                kind,
            });
        }

        // Resolve the selected native paths before touching the manifest. The
        // resulting IDs are opaque and path-free at the application boundary.
        let composition = self
            .catalog_composer
            .compose(&imports)
            .map_err(catalog_error)?;
        let song_ids = imports
            .iter()
            .filter_map(|import| composition.imported_membership.get(&import.source_id))
            .flat_map(|ids| ids.iter().cloned())
            .collect::<BTreeSet<_>>();
        let song_ids = song_ids.into_iter().collect::<Vec<_>>();

        let (playlist, has_new_import) = {
            let mut manifest = self
                .library_manifest
                .lock()
                .map_err(|_| "native library manifest lock poisoned".to_string())?;
            let has_new_import = imports.iter().any(|import| {
                !manifest.snapshot().imports.iter().any(|existing| {
                    existing
                        .canonical_path
                        .eq_ignore_ascii_case(&import.canonical_path)
                })
            });
            let playlist = manifest
                .import_and_add_songs(&request.playlist_id, imports, &song_ids)
                .map_err(library_error)?;
            self.invalidate_playlist_search_orders()?;
            (playlist, has_new_import)
        };

        let generation = if has_new_import {
            self.reload()?.generation
        } else {
            self.ensure_catalog_loaded()?
        };
        Ok(LibraryPlaylistImportResultDto {
            playlist: self.playlist_summary(playlist)?,
            imported_song_count: song_ids.len() as u64,
            catalog_generation: generation,
        })
    }
}
fn library_error(error: LibraryError) -> String {
    format!("library operation failed: {error}")
}

fn is_supported_catalog_path(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            sky_app_core::catalog::SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn imported_source_id(canonical_path: &str, kind: ImportedSourceKind) -> String {
    let mut hasher = Sha256::new();
    hasher.update(match kind {
        ImportedSourceKind::File => b"file\0".as_slice(),
        ImportedSourceKind::Folder => b"folder\0".as_slice(),
    });
    hasher.update(canonical_path.as_bytes());
    hasher
        .finalize()
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
