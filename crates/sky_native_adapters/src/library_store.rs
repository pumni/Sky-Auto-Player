use super::atomic_file;
use sky_app_core::library::{
    LIBRARY_MANIFEST_VERSION, LibraryError, LibraryManifestStore, LibraryManifestV1,
};
use sky_app_core::settings::SettingsError;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
/// Native-only persistence for user library data.  The canonical paths in
/// this document are never projected to the frontend; they are resolved only
/// while composing the native catalog.
#[derive(Clone)]
pub struct JsonLibraryManifestStore {
    path: PathBuf,
    write_lock: Arc<Mutex<()>>,
}

impl JsonLibraryManifestStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl LibraryManifestStore for JsonLibraryManifestStore {
    fn load(&self) -> Result<LibraryManifestV1, LibraryError> {
        if !self.path.exists() {
            return Ok(LibraryManifestV1 {
                version: LIBRARY_MANIFEST_VERSION,
                ..Default::default()
            });
        }
        let text = fs::read_to_string(&self.path)
            .map_err(|error| LibraryError::Storage(error.to_string()))?;
        serde_json::from_str(&text).map_err(|error| LibraryError::Storage(error.to_string()))
    }

    fn save(&self, manifest: &LibraryManifestV1) -> Result<(), LibraryError> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| LibraryError::Storage("library manifest write lock poisoned".into()))?;
        let encoded = serde_json::to_vec_pretty(manifest)
            .map_err(|error| LibraryError::Storage(error.to_string()))?;
        atomic_file::atomic_replace(&self.path, &encoded)
            .map_err(|error| LibraryError::Storage(SettingsError::Storage(error).to_string()))
    }
}
