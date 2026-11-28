use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LibraryPlaylistSummaryDto {
    pub id: String,
    pub name: String,
    pub song_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LibraryNavigationDto {
    pub builtin_available: bool,
    pub builtin_song_count: u64,
    pub playlists: Vec<LibraryPlaylistSummaryDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LibraryCreatePlaylistRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LibraryRenamePlaylistRequest {
    pub playlist_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LibraryPlaylistSongsRequest {
    pub playlist_id: String,
    pub song_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct LibraryPlaylistIdRequest {
    pub playlist_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LibraryPlaylistImportResultDto {
    pub playlist: LibraryPlaylistSummaryDto,
    pub imported_song_count: u64,
    pub catalog_generation: u64,
}
