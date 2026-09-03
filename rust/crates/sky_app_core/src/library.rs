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

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum LibraryError {
    #[error("liked song ID is malformed")]
    InvalidSongId,
    #[error("liked songs collection exceeds {MAX_LIKED_SONGS} entries")]
    TooManyLikedSongs,
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
}
