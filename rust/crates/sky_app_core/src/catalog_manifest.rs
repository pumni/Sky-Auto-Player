use crate::catalog::{SUPPORTED_EXTENSIONS, is_valid_song_id};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

pub const BUILTIN_MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltinCatalogManifest {
    pub schema_version: u32,
    pub songs: Vec<BuiltinSongManifestEntry>,
    #[serde(default)]
    pub retired_songs: Vec<RetiredBuiltinSong>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltinSongManifestEntry {
    pub id: String,
    pub path: String,
    pub title: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetiredBuiltinSong {
    pub id: String,
    pub last_title: String,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum BuiltinManifestError {
    #[error("unsupported built-in manifest schema version")]
    UnsupportedSchema,
    #[error("built-in manifest contains a malformed song ID")]
    InvalidId,
    #[error("built-in manifest reuses a song ID")]
    DuplicateId,
    #[error("built-in manifest reuses a resource path")]
    DuplicatePath,
    #[error("built-in manifest active and retired IDs overlap")]
    ActiveRetiredOverlap,
    #[error("built-in manifest contains an invalid relative path")]
    InvalidPath,
    #[error("built-in manifest contains an unsupported song extension")]
    UnsupportedExtension,
    #[error("built-in manifest contains an empty title")]
    EmptyTitle,
    #[error("built-in manifest contains an invalid SHA-256")]
    InvalidHash,
}

impl BuiltinCatalogManifest {
    pub fn validate(&self) -> Result<(), BuiltinManifestError> {
        if self.schema_version != BUILTIN_MANIFEST_SCHEMA_VERSION {
            return Err(BuiltinManifestError::UnsupportedSchema);
        }
        let mut active_ids = BTreeSet::new();
        let mut active_paths = BTreeSet::new();
        for song in &self.songs {
            if !is_valid_song_id(&song.id) {
                return Err(BuiltinManifestError::InvalidId);
            }
            if !active_ids.insert(&song.id) {
                return Err(BuiltinManifestError::DuplicateId);
            }
            if !active_paths.insert(song.path.to_ascii_lowercase()) {
                return Err(BuiltinManifestError::DuplicatePath);
            }
            validate_builtin_path(&song.path)?;
            if song.title.trim().is_empty() {
                return Err(BuiltinManifestError::EmptyTitle);
            }
            if song.sha256.len() != 64
                || !song
                    .sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            {
                return Err(BuiltinManifestError::InvalidHash);
            }
        }
        let mut retired_ids = BTreeSet::new();
        for song in &self.retired_songs {
            if !is_valid_song_id(&song.id) || !retired_ids.insert(&song.id) {
                return Err(BuiltinManifestError::DuplicateId);
            }
            if active_ids.contains(&song.id) {
                return Err(BuiltinManifestError::ActiveRetiredOverlap);
            }
            if song.last_title.trim().is_empty() {
                return Err(BuiltinManifestError::EmptyTitle);
            }
        }
        Ok(())
    }
}

fn validate_builtin_path(path: &str) -> Result<(), BuiltinManifestError> {
    if path.is_empty() || path.contains('\0') || path.contains('\\') {
        return Err(BuiltinManifestError::InvalidPath);
    }
    let path = Path::new(path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(BuiltinManifestError::InvalidPath);
    }
    let extension = path.extension().and_then(|value| value.to_str());
    if !extension.is_some_and(|extension| {
        SUPPORTED_EXTENSIONS
            .iter()
            .any(|supported| extension.eq_ignore_ascii_case(supported))
    }) {
        return Err(BuiltinManifestError::UnsupportedExtension);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active(path: &str, id: &str) -> BuiltinSongManifestEntry {
        BuiltinSongManifestEntry {
            id: id.into(),
            path: path.into(),
            title: "Song".into(),
            sha256: "a".repeat(64),
        }
    }

    #[test]
    fn active_and_retired_id_validation_is_disjoint_and_strict() {
        let manifest = BuiltinCatalogManifest {
            schema_version: 1,
            songs: vec![active(
                "sheets/song.json",
                "0123456789abcdef0123456789abcdef",
            )],
            retired_songs: vec![RetiredBuiltinSong {
                id: "fedcba9876543210fedcba9876543210".into(),
                last_title: "Retired".into(),
            }],
        };
        assert!(manifest.validate().is_ok());

        let mut overlapping = manifest.clone();
        overlapping.retired_songs[0].id = overlapping.songs[0].id.clone();
        assert_eq!(
            overlapping.validate(),
            Err(BuiltinManifestError::ActiveRetiredOverlap)
        );

        let mut duplicate_path = manifest;
        duplicate_path.songs.push(active(
            "sheets/SONG.JSON",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ));
        assert_eq!(
            duplicate_path.validate(),
            Err(BuiltinManifestError::DuplicatePath)
        );
    }

    #[test]
    fn manifest_rejects_unsafe_paths_and_invalid_hashes() {
        let mut manifest = BuiltinCatalogManifest {
            schema_version: 1,
            songs: vec![active(
                "sheets/song.json",
                "0123456789abcdef0123456789abcdef",
            )],
            retired_songs: Vec::new(),
        };
        manifest.songs[0].path = "../song.json".into();
        assert_eq!(manifest.validate(), Err(BuiltinManifestError::InvalidPath));

        manifest.songs[0].path = "sheets/song.json".into();
        manifest.songs[0].sha256 = "A".repeat(64);
        assert_eq!(manifest.validate(), Err(BuiltinManifestError::InvalidHash));
    }
}
