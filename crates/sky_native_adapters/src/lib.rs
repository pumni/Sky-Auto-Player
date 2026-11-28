//! Outer adapters for application services.
//!
//! This crate owns filesystem enumeration and persisted config compatibility.
//! It does not contain Tauri delivery policy, realtime playback, or updater
//! transaction logic.

mod atomic_file;
mod catalog;
pub mod catalog_cache;
mod library_store;
pub mod paths;
mod settings_store;

pub use catalog::{
    BuiltinCatalogFailureCode, BuiltinCatalogSource, BuiltinCatalogStatus, CatalogComposer,
    CatalogComposition, CatalogCompositionEvent, CatalogCompositionSource, CatalogScanMetrics,
    FileCatalogSource, ImportedSourceCatalogStatus,
};
pub use catalog_cache::{
    CATALOG_CACHE_SCHEMA_VERSION, CatalogCache, CatalogCacheEntry, CatalogCacheSource,
    CatalogCacheSourceKind, CatalogCacheStore,
};
pub use library_store::JsonLibraryManifestStore;
pub use paths::{AppPaths, AppResources, V4_APP_IDENTIFIER, snapshot_directory};
pub use settings_store::JsonSettingsStore;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod catalog_tests;
