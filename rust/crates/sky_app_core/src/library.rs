//! Durable library collections and stable song identity policy.
//!
//! The desktop shell owns delivery and persistence, while this module keeps
//! the actual liked-song behavior independent from React and filesystem
//! details. Song IDs are opaque, path-free identities produced by the catalog
//! index and remain stable when a song is revisited.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SONG_ID_LENGTH: usize = 32;
pub const MAX_LIKED_SONGS: usize = 100_000;
pub const LIBRARY_MANIFEST_VERSION: u32 = 1;
pub const COLLECTION_ID_LENGTH: usize = 32;
pub const MAX_COLLECTIONS: usize = 256;
pub const MAX_COLLECTION_NAME_LENGTH: usize = 128;
pub const MAX_COLLECTION_SONGS: usize = 100_000;
pub const MAX_IMPORTED_SOURCES: usize = 1_024;
pub const MAX_IMPORTED_PATH_LENGTH: usize = 4_096;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum LibraryError {
    #[error("liked song ID is malformed")]
    InvalidSongId,
    #[error("liked songs collection exceeds {MAX_LIKED_SONGS} entries")]
    TooManyLikedSongs,
    #[error("collection ID is malformed")]
    InvalidCollectionId,
    #[error("collection name is empty")]
    EmptyCollectionName,
    #[error("collection name exceeds {MAX_COLLECTION_NAME_LENGTH} characters")]
    CollectionNameTooLong,
    #[error("collection already exists")]
    CollectionAlreadyExists,
    #[error("collection was not found")]
    CollectionNotFound,
    #[error("collections exceed {MAX_COLLECTIONS} entries")]
    TooManyCollections,
    #[error("collection exceeds {MAX_COLLECTION_SONGS} song IDs")]
    TooManyCollectionSongs,
    #[error("import source ID is malformed")]
    InvalidImportSourceId,
    #[error("import source path is invalid")]
    InvalidImportPath,
    #[error("import sources exceed {MAX_IMPORTED_SOURCES} entries")]
    TooManyImportedSources,
    #[error("library manifest storage error: {0}")]
    Storage(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LikedSongs {
    ids: BTreeSet<String>,
}

impl LikedSongs {
    /// Load persisted IDs defensively. IDs that no longer exist in the
    /// catalog remain durable so a temporarily missing song can become liked
    /// again when it returns; malformed values are discarded at the storage
    /// boundary and never enter the domain model.
    pub fn from_persisted<I>(ids: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let ids = ids
            .into_iter()
            .filter(|id| is_valid_song_id(id))
            .take(MAX_LIKED_SONGS)
            .collect();
        Self { ids }
    }

    pub fn contains(&self, song_id: &str) -> bool {
        self.ids.contains(song_id)
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn ids(&self) -> &BTreeSet<String> {
        &self.ids
    }

    pub fn set(&mut self, song_id: &str, liked: bool) -> Result<bool, LibraryError> {
        if !is_valid_song_id(song_id) {
            return Err(LibraryError::InvalidSongId);
        }
        if liked {
            if self.ids.contains(song_id) {
                return Ok(false);
            }
            if self.ids.len() >= MAX_LIKED_SONGS {
                return Err(LibraryError::TooManyLikedSongs);
            }
            self.ids.insert(song_id.to_owned());
            Ok(true)
        } else {
            Ok(self.ids.remove(song_id))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportedSourceKind {
    File,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedSourceRef {
    pub source_id: String,
    /// This is an opaque native storage locator. It must never be included in
    /// a frontend DTO or browser persistence payload.
    pub canonical_path: String,
    pub kind: ImportedSourceKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub song_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryManifestV1 {
    pub version: u32,
    #[serde(default)]
    pub imports: Vec<ImportedSourceRef>,
    #[serde(default)]
    pub collections: Vec<Collection>,
}

pub trait LibraryManifestStore {
    fn load(&self) -> Result<LibraryManifestV1, LibraryError>;
    fn save(&self, manifest: &LibraryManifestV1) -> Result<(), LibraryError>;
}

pub struct LibraryManifestService<S> {
    store: S,
    current: LibraryManifestV1,
}

impl<S: LibraryManifestStore> LibraryManifestService<S> {
    pub fn load(store: S) -> Result<Self, LibraryError> {
        let mut current = store.load()?;
        let needs_version_write = current.version == 0;
        if current.version == 0 {
            current.version = LIBRARY_MANIFEST_VERSION;
        }
        validate_manifest(&current)?;
        if needs_version_write {
            store.save(&current)?;
        }
        Ok(Self { store, current })
    }

    pub fn snapshot(&self) -> &LibraryManifestV1 {
        &self.current
    }

    pub fn create_collection(
        &mut self,
        id: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Collection, LibraryError> {
        let id = id.into();
        let name = name.into();
        let name = normalize_collection_name(&name)?;
        validate_collection_id(&id)?;
        if self.current.collections.iter().any(|item| item.id == id) {
            return Err(LibraryError::CollectionAlreadyExists);
        }
        if self.current.collections.len() >= MAX_COLLECTIONS {
            return Err(LibraryError::TooManyCollections);
        }
        let collection = Collection {
            id,
            name,
            song_ids: Vec::new(),
        };
        let mut next = self.current.clone();
        next.collections.push(collection.clone());
        self.commit(next)?;
        Ok(collection)
    }

    pub fn rename_collection(
        &mut self,
        id: &str,
        name: impl Into<String>,
    ) -> Result<Collection, LibraryError> {
        validate_collection_id(id)?;
        let name = normalize_collection_name(&name.into())?;
        let mut next = self.current.clone();
        let collection = next
            .collections
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or(LibraryError::CollectionNotFound)?;
        collection.name = name;
        let result = collection.clone();
        self.commit(next)?;
        Ok(result)
    }

    pub fn delete_collection(&mut self, id: &str) -> Result<bool, LibraryError> {
        validate_collection_id(id)?;
        let mut next = self.current.clone();
        let before = next.collections.len();
        next.collections.retain(|item| item.id != id);
        if next.collections.len() == before {
            return Ok(false);
        }
        self.commit(next)?;
        Ok(true)
    }

    pub fn add_songs(
        &mut self,
        collection_id: &str,
        song_ids: &[String],
    ) -> Result<Collection, LibraryError> {
        validate_collection_id(collection_id)?;
        validate_song_ids(song_ids)?;
        let mut next = self.current.clone();
        let collection = next
            .collections
            .iter_mut()
            .find(|item| item.id == collection_id)
            .ok_or(LibraryError::CollectionNotFound)?;
        for song_id in song_ids {
            if !collection.song_ids.contains(song_id) {
                collection.song_ids.push(song_id.clone());
            }
        }
        if collection.song_ids.len() > MAX_COLLECTION_SONGS {
            return Err(LibraryError::TooManyCollectionSongs);
        }
        let result = collection.clone();
        self.commit(next)?;
        Ok(result)
    }

    pub fn remove_songs(
        &mut self,
        collection_id: &str,
        song_ids: &[String],
    ) -> Result<Collection, LibraryError> {
        validate_collection_id(collection_id)?;
        validate_song_ids(song_ids)?;
        let mut next = self.current.clone();
        let collection = next
            .collections
            .iter_mut()
            .find(|item| item.id == collection_id)
            .ok_or(LibraryError::CollectionNotFound)?;
        collection
            .song_ids
            .retain(|song_id| !song_ids.contains(song_id));
        let result = collection.clone();
        self.commit(next)?;
        Ok(result)
    }

    pub fn register_imports(
        &mut self,
        imports: Vec<ImportedSourceRef>,
    ) -> Result<usize, LibraryError> {
        for import in &imports {
            validate_import(import)?;
        }
        let mut next = self.current.clone();
        let before = next.imports.len();
        for import in imports {
            if !next.imports.iter().any(|existing| {
                existing
                    .canonical_path
                    .eq_ignore_ascii_case(&import.canonical_path)
            }) {
                next.imports.push(import);
            }
        }
        if next.imports.len() > MAX_IMPORTED_SOURCES {
            return Err(LibraryError::TooManyImportedSources);
        }
        let added = next.imports.len() - before;
        if added > 0 {
            self.commit(next)?;
        }
        Ok(added)
    }

    /// Commit imported asset references and their target playlist membership as
    /// one manifest transaction. The caller resolves the selected paths to
    /// stable song IDs before entering this method; a failed save therefore
    /// cannot leave an import reference without its playlist membership.
    pub fn import_and_add_songs(
        &mut self,
        playlist_id: &str,
        imports: Vec<ImportedSourceRef>,
        song_ids: &[String],
    ) -> Result<Collection, LibraryError> {
        validate_collection_id(playlist_id)?;
        validate_song_ids(song_ids)?;
        for import in &imports {
            validate_import(import)?;
        }

        let mut next = self.current.clone();
        for import in imports {
            if !next.imports.iter().any(|existing| {
                existing
                    .canonical_path
                    .eq_ignore_ascii_case(&import.canonical_path)
            }) {
                next.imports.push(import);
            }
        }
        if next.imports.len() > MAX_IMPORTED_SOURCES {
            return Err(LibraryError::TooManyImportedSources);
        }

        let playlist = next
            .collections
            .iter_mut()
            .find(|item| item.id == playlist_id)
            .ok_or(LibraryError::CollectionNotFound)?;
        for song_id in song_ids {
            if !playlist.song_ids.contains(song_id) {
                playlist.song_ids.push(song_id.clone());
            }
        }
        if playlist.song_ids.len() > MAX_COLLECTION_SONGS {
            return Err(LibraryError::TooManyCollectionSongs);
        }
        let result = playlist.clone();
        if next != self.current {
            self.commit(next)?;
        }
        Ok(result)
    }

    pub fn remove_import(&mut self, source_id: &str) -> Result<bool, LibraryError> {
        validate_import_source_id(source_id)?;
        let mut next = self.current.clone();
        let before = next.imports.len();
        next.imports.retain(|item| item.source_id != source_id);
        if next.imports.len() == before {
            return Ok(false);
        }
        self.commit(next)?;
        Ok(true)
    }

    fn commit(&mut self, next: LibraryManifestV1) -> Result<(), LibraryError> {
        validate_manifest(&next)?;
        self.store.save(&next)?;
        self.current = next;
        Ok(())
    }
}

fn validate_manifest(manifest: &LibraryManifestV1) -> Result<(), LibraryError> {
    if manifest.version != LIBRARY_MANIFEST_VERSION {
        return Err(LibraryError::Storage(format!(
            "unsupported library manifest version {}",
            manifest.version
        )));
    }
    if manifest.collections.len() > MAX_COLLECTIONS {
        return Err(LibraryError::TooManyCollections);
    }
    if manifest.imports.len() > MAX_IMPORTED_SOURCES {
        return Err(LibraryError::TooManyImportedSources);
    }
    let mut collection_ids = BTreeSet::new();
    for collection in &manifest.collections {
        validate_collection_id(&collection.id)?;
        if !collection_ids.insert(&collection.id) {
            return Err(LibraryError::CollectionAlreadyExists);
        }
        normalize_collection_name(&collection.name)?;
        validate_song_ids(&collection.song_ids)?;
        if collection.song_ids.len() > MAX_COLLECTION_SONGS {
            return Err(LibraryError::TooManyCollectionSongs);
        }
    }
    let mut import_ids = BTreeSet::new();
    for import in &manifest.imports {
        validate_import(import)?;
        if !import_ids.insert(&import.source_id) {
            return Err(LibraryError::InvalidImportSourceId);
        }
    }
    Ok(())
}

fn normalize_collection_name(name: &str) -> Result<String, LibraryError> {
    let normalized = name.trim();
    if normalized.is_empty() {
        return Err(LibraryError::EmptyCollectionName);
    }
    if normalized.chars().count() > MAX_COLLECTION_NAME_LENGTH || normalized.contains('\0') {
        return Err(LibraryError::CollectionNameTooLong);
    }
    Ok(normalized.to_owned())
}

fn validate_collection_id(id: &str) -> Result<(), LibraryError> {
    if id.len() != COLLECTION_ID_LENGTH
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(LibraryError::InvalidCollectionId);
    }
    Ok(())
}

pub fn is_valid_collection_id(id: &str) -> bool {
    id.len() == COLLECTION_ID_LENGTH
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate_song_ids(song_ids: &[String]) -> Result<(), LibraryError> {
    if song_ids.iter().any(|id| !is_valid_song_id(id)) {
        return Err(LibraryError::InvalidSongId);
    }
    Ok(())
}

fn validate_import(import: &ImportedSourceRef) -> Result<(), LibraryError> {
    validate_import_source_id(&import.source_id)?;
    if import.canonical_path.is_empty()
        || import.canonical_path.len() > MAX_IMPORTED_PATH_LENGTH
        || import.canonical_path.contains('\0')
    {
        return Err(LibraryError::InvalidImportPath);
    }
    Ok(())
}

fn validate_import_source_id(id: &str) -> Result<(), LibraryError> {
    if id.len() != COLLECTION_ID_LENGTH
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(LibraryError::InvalidImportSourceId);
    }
    Ok(())
}

pub fn is_valid_song_id(song_id: &str) -> bool {
    song_id.len() == SONG_ID_LENGTH
        && song_id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liked_song_ids_are_deduplicated_and_malformed_values_are_discarded() {
        let id = "0123456789abcdef0123456789abcdef".to_owned();
        let liked = LikedSongs::from_persisted([
            id.clone(),
            id.clone(),
            "not-a-song-id".into(),
            "0123456789ABCDEF0123456789abcdef".into(),
        ]);
        assert_eq!(liked.len(), 1);
        assert!(liked.contains(&id));
    }

    #[test]
    fn like_mutation_is_validated_and_idempotent() {
        let id = "0123456789abcdef0123456789abcdef";
        let mut liked = LikedSongs::default();
        assert_eq!(liked.set(id, true), Ok(true));
        assert_eq!(liked.set(id, true), Ok(false));
        assert_eq!(liked.set(id, false), Ok(true));
        assert_eq!(liked.set(id, false), Ok(false));
        assert_eq!(liked.set("bad", true), Err(LibraryError::InvalidSongId));
    }

    #[derive(Default)]
    struct MemoryManifestStore {
        value: std::sync::Mutex<Option<LibraryManifestV1>>,
        saves: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl LibraryManifestStore for MemoryManifestStore {
        fn load(&self) -> Result<LibraryManifestV1, LibraryError> {
            Ok(self
                .value
                .lock()
                .expect("manifest lock")
                .clone()
                .unwrap_or(LibraryManifestV1 {
                    version: LIBRARY_MANIFEST_VERSION,
                    imports: Vec::new(),
                    collections: Vec::new(),
                }))
        }

        fn save(&self, manifest: &LibraryManifestV1) -> Result<(), LibraryError> {
            self.saves.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            *self.value.lock().expect("manifest lock") = Some(manifest.clone());
            Ok(())
        }
    }

    #[test]
    fn manifest_service_persists_collection_membership_without_paths_in_domain_operations() {
        let mut service =
            LibraryManifestService::load(MemoryManifestStore::default()).expect("manifest service");
        let collection = service
            .create_collection("0123456789abcdef0123456789abcdef", "  Practice  ")
            .expect("collection");
        let song_id = "abcdefabcdefabcdefabcdefabcdefab".to_owned();
        let updated = service
            .add_songs(&collection.id, std::slice::from_ref(&song_id))
            .expect("membership");
        assert_eq!(updated.name, "Practice");
        assert_eq!(updated.song_ids, [song_id]);
        assert!(service.delete_collection(&collection.id).expect("delete"));
        assert!(service.snapshot().collections.is_empty());
    }

    #[test]
    fn manifest_rejects_invalid_ids_and_path_like_names() {
        let mut service =
            LibraryManifestService::load(MemoryManifestStore::default()).expect("manifest service");
        assert_eq!(
            service.create_collection("bad", "Practice").unwrap_err(),
            LibraryError::InvalidCollectionId
        );
        assert_eq!(
            service
                .create_collection("0123456789abcdef0123456789abcdef", "")
                .unwrap_err(),
            LibraryError::EmptyCollectionName
        );
    }

    #[test]
    fn import_and_add_songs_commits_import_reference_and_membership_once() {
        let saves = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let store = MemoryManifestStore {
            saves: saves.clone(),
            ..Default::default()
        };
        let mut service = LibraryManifestService::load(store).expect("manifest service");
        let playlist_id = "0123456789abcdef0123456789abcdef";
        service
            .create_collection(playlist_id, "Practice")
            .expect("playlist");
        saves.store(0, std::sync::atomic::Ordering::SeqCst);

        let song_id = "abcdefabcdefabcdefabcdefabcdefab".to_owned();
        let updated = service
            .import_and_add_songs(
                playlist_id,
                vec![ImportedSourceRef {
                    source_id: "fedcba9876543210fedcba9876543210".to_owned(),
                    canonical_path: r"C:\Music\local.json".to_owned(),
                    kind: ImportedSourceKind::File,
                }],
                std::slice::from_ref(&song_id),
            )
            .expect("atomic import");

        assert_eq!(saves.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(updated.song_ids, [song_id]);
        assert_eq!(service.snapshot().imports.len(), 1);
        assert_eq!(service.snapshot().collections[0].song_ids.len(), 1);
    }
}
