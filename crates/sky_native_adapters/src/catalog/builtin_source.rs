use super::super::paths::AppResources;
use sky_app_core::catalog::{BuiltinCatalogManifest, CatalogSourceEntry, StableSongId};
use std::collections::BTreeSet;
use std::fs;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuiltinCatalogFailureCode {
    ResourceUnavailable,
    MalformedManifest,
    MissingResource,
    InvalidResource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuiltinCatalogStatus {
    Available { song_count: usize },
    Unavailable { code: BuiltinCatalogFailureCode },
}

impl Default for BuiltinCatalogStatus {
    fn default() -> Self {
        Self::Unavailable {
            code: BuiltinCatalogFailureCode::ResourceUnavailable,
        }
    }
}

pub struct BuiltinCatalogSource {
    resources: AppResources,
}

impl BuiltinCatalogSource {
    pub fn new(resources: AppResources) -> Self {
        Self { resources }
    }

    pub fn resources(&self) -> &AppResources {
        &self.resources
    }

    pub(super) fn load(
        &self,
    ) -> (
        Vec<CatalogSourceEntry>,
        BTreeSet<String>,
        BuiltinCatalogStatus,
    ) {
        let root = self.resources.builtin_catalog_root();
        if !root.is_dir() {
            return (
                Vec::new(),
                BTreeSet::new(),
                BuiltinCatalogStatus::Unavailable {
                    code: BuiltinCatalogFailureCode::ResourceUnavailable,
                },
            );
        }
        let manifest_path = self.resources.builtin_catalog_manifest_path();
        let manifest = match fs::read(&manifest_path).map_err(|_| ()).and_then(|bytes| {
            serde_json::from_slice::<BuiltinCatalogManifest>(&bytes).map_err(|_| ())
        }) {
            Ok(manifest) => manifest,
            Err(()) => {
                return (
                    Vec::new(),
                    BTreeSet::new(),
                    BuiltinCatalogStatus::Unavailable {
                        code: BuiltinCatalogFailureCode::MalformedManifest,
                    },
                );
            }
        };
        if manifest.validate().is_err() {
            return (
                Vec::new(),
                BTreeSet::new(),
                BuiltinCatalogStatus::Unavailable {
                    code: BuiltinCatalogFailureCode::MalformedManifest,
                },
            );
        }
        let canonical_root = match fs::canonicalize(root) {
            Ok(root) => root,
            Err(_) => {
                return (
                    Vec::new(),
                    BTreeSet::new(),
                    BuiltinCatalogStatus::Unavailable {
                        code: BuiltinCatalogFailureCode::ResourceUnavailable,
                    },
                );
            }
        };
        let mut entries = Vec::with_capacity(manifest.songs.len());
        let mut membership = BTreeSet::new();
        for item in manifest.songs {
            let path = root.join(&item.path);
            let canonical = match fs::canonicalize(&path) {
                Ok(path) if path.starts_with(&canonical_root) && path.is_file() => path,
                _ => {
                    return (
                        Vec::new(),
                        BTreeSet::new(),
                        BuiltinCatalogStatus::Unavailable {
                            code: BuiltinCatalogFailureCode::MissingResource,
                        },
                    );
                }
            };
            let stable_id = match StableSongId::new(item.id) {
                Ok(id) => id,
                Err(_) => {
                    return (
                        Vec::new(),
                        BTreeSet::new(),
                        BuiltinCatalogStatus::Unavailable {
                            code: BuiltinCatalogFailureCode::InvalidResource,
                        },
                    );
                }
            };
            membership.insert(stable_id.as_str().to_owned());
            entries.push(CatalogSourceEntry::stable(
                canonical.to_string_lossy(),
                item.title,
                stable_id,
            ));
        }
        let status = BuiltinCatalogStatus::Available {
            song_count: entries.len(),
        };
        (entries, membership, status)
    }
}
