use serde::{Deserialize, Serialize};
use sky_app_core::catalog::{SUPPORTED_EXTENSIONS, is_valid_song_id};
use sky_app_core::library::ImportedSourceKind;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const CATALOG_CACHE_SCHEMA_VERSION: u32 = 1;
pub const CATALOG_CACHE_MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const CATALOG_CACHE_MAX_ENTRIES: usize = 1_000_000;
pub const CATALOG_CACHE_MAX_SOURCES: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogCacheSourceKind {
    Builtin,
    User,
    Imported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogCacheSource {
    pub source_id: String,
    pub kind: CatalogCacheSourceKind,
    pub import_kind: Option<ImportedSourceKind>,
    pub display_name: String,
    pub available: bool,
    pub song_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogCacheEntry {
    pub source_id: String,
    pub kind: CatalogCacheSourceKind,
    pub canonical_path: String,
    pub song_id: String,
    pub title: String,
    pub file_size: u64,
    pub modified_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogCache {
    pub schema_version: u32,
    pub cache_generation: u64,
    pub sources: Vec<CatalogCacheSource>,
    pub entries: Vec<CatalogCacheEntry>,
}

impl CatalogCache {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != CATALOG_CACHE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported catalog cache schema {}",
                self.schema_version
            ));
        }
        if self.cache_generation == 0 {
            return Err("catalog cache generation must be positive".into());
        }
        if self.sources.len() > CATALOG_CACHE_MAX_SOURCES {
            return Err("catalog cache contains too many sources".into());
        }
        if self.entries.len() > CATALOG_CACHE_MAX_ENTRIES {
            return Err("catalog cache contains too many entries".into());
        }

        let mut sources = HashMap::new();
        for source in &self.sources {
            validate_source_id(&source.source_id, &source.kind)?;
            if matches!(source.kind, CatalogCacheSourceKind::Imported)
                != source.import_kind.is_some()
            {
                return Err("catalog cache source kind metadata is inconsistent".into());
            }
            validate_text(&source.display_name, 1024, "source display name")?;
            if !sources
                .insert((source.kind.clone(), source.source_id.clone()), source)
                .is_none()
            {
                return Err("catalog cache contains duplicate sources".into());
            }
            let mut song_ids = BTreeSet::new();
            for song_id in &source.song_ids {
                if !is_valid_song_id(song_id) || !song_ids.insert(song_id) {
                    return Err("catalog cache contains malformed or duplicate song IDs".into());
                }
            }
        }

        let mut paths = HashMap::new();
        for entry in &self.entries {
            validate_source_id(&entry.source_id, &entry.kind)?;
            if !sources.contains_key(&(entry.kind.clone(), entry.source_id.clone())) {
                return Err("catalog cache entry references an unknown source".into());
            }
            if entry.canonical_path.is_empty()
                || entry.canonical_path.len() > 4_096
                || entry.canonical_path.contains('\0')
                || !Path::new(&entry.canonical_path).is_absolute()
                || !is_supported_path(&entry.canonical_path)
            {
                return Err("catalog cache contains an invalid canonical path".into());
            }
            if !is_valid_song_id(&entry.song_id) {
                return Err("catalog cache contains an invalid song ID".into());
            }
            validate_text(&entry.title, 4_096, "catalog cache title")?;
            let normalized_path = entry.canonical_path.to_ascii_lowercase();
            if let Some(previous_song_id) = paths.insert(normalized_path, &entry.song_id)
                && previous_song_id != &entry.song_id
            {
                return Err("catalog cache maps one path to multiple song IDs".into());
            }
            let source = sources
                .get(&(entry.kind.clone(), entry.source_id.clone()))
                .expect("validated source reference");
            if !source
                .song_ids
                .iter()
                .any(|song_id| song_id == &entry.song_id)
            {
                return Err("catalog cache entry is absent from source membership".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct CatalogCacheStore {
    path: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

impl CatalogCacheStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<CatalogCache>, String> {
        if !self.path.exists() {
            return Ok(None);
        }
        let size = fs::metadata(&self.path)
            .map_err(|error| error.to_string())?
            .len();
        if size > CATALOG_CACHE_MAX_BYTES {
            return Err("catalog cache exceeds the bounded input size".into());
        }
        let bytes = fs::read(&self.path).map_err(|error| error.to_string())?;
        let cache = serde_json::from_slice::<CatalogCache>(&bytes)
            .map_err(|error| format!("catalog cache decode failed: {error}"))?;
        cache.validate()?;
        Ok(Some(cache))
    }

    pub fn save(&self, cache: &CatalogCache) -> Result<(), String> {
        cache.validate()?;
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| "catalog cache write lock poisoned".to_string())?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let encoded = serde_json::to_vec_pretty(cache).map_err(|error| error.to_string())?;
        let temp = self.path.with_extension("json.tmp");
        fs::write(&temp, encoded).map_err(|error| error.to_string())?;
        super::replace_file(&temp, &self.path).inspect_err(|_| {
            let _ = fs::remove_file(&temp);
        })
    }
}

fn validate_source_id(source_id: &str, kind: &CatalogCacheSourceKind) -> Result<(), String> {
    let valid = match kind {
        CatalogCacheSourceKind::Builtin => source_id == "builtin",
        CatalogCacheSourceKind::User => source_id == "user",
        CatalogCacheSourceKind::Imported => is_valid_song_id(source_id),
    };
    if !valid {
        return Err("catalog cache contains an invalid source identity".into());
    }
    Ok(())
}

fn validate_text(value: &str, max_chars: usize, label: &str) -> Result<(), String> {
    if value.is_empty() || value.chars().count() > max_chars || value.contains('\0') {
        return Err(format!("{label} is invalid"));
    }
    Ok(())
}

fn is_supported_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            SUPPORTED_EXTENSIONS
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn entry(source_id: &str, kind: CatalogCacheSourceKind) -> CatalogCacheEntry {
        CatalogCacheEntry {
            source_id: source_id.into(),
            kind,
            canonical_path: r"C:\songs\one.json".into(),
            song_id: "0123456789abcdef0123456789abcdef".into(),
            title: "One".into(),
            file_size: 42,
            modified_unix_ms: 1,
        }
    }

    fn cache() -> CatalogCache {
        CatalogCache {
            schema_version: CATALOG_CACHE_SCHEMA_VERSION,
            cache_generation: 1,
            sources: vec![CatalogCacheSource {
                source_id: "user".into(),
                kind: CatalogCacheSourceKind::User,
                import_kind: None,
                display_name: "User library".into(),
                available: true,
                song_ids: vec!["0123456789abcdef0123456789abcdef".into()],
            }],
            entries: vec![entry("user", CatalogCacheSourceKind::User)],
        }
    }

    #[test]
    fn cache_validation_rejects_schema_and_path_identity_errors() {
        let mut invalid_schema = cache();
        invalid_schema.schema_version += 1;
        assert!(invalid_schema.validate().is_err());

        let mut invalid_path = cache();
        invalid_path.entries[0].canonical_path = "relative/song.json".into();
        assert!(invalid_path.validate().is_err());

        let mut invalid_membership = cache();
        invalid_membership.entries[0].song_id = "fedcba9876543210fedcba9876543210".into();
        assert!(invalid_membership.validate().is_err());
    }

    #[test]
    fn cache_store_round_trips_atomically() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-catalog-cache-{suffix}"));
        let store = CatalogCacheStore::new(root.join("cache/catalog-index.json"));
        store.save(&cache()).expect("save cache");
        assert_eq!(store.load().expect("load cache"), Some(cache()));
        assert!(!store.path().with_extension("json.tmp").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cache_store_fails_safe_for_truncated_and_schema_mismatch_data() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("sky-catalog-cache-corrupt-{suffix}"));
        let store = CatalogCacheStore::new(root.join("cache/catalog-index.json"));
        fs::create_dir_all(root.join("cache")).expect("cache root");
        fs::write(store.path(), b"{\"schema_version\":1").expect("truncated cache");
        assert!(store.load().is_err());
        let mut mismatch = cache();
        mismatch.schema_version += 1;
        fs::write(
            store.path(),
            serde_json::to_vec(&mismatch).expect("schema mismatch JSON"),
        )
        .expect("schema mismatch cache");
        assert!(store.load().is_err());
        let _ = fs::remove_dir_all(root);
    }
}
