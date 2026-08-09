//! Testable native updater components.
//!
//! The updater is intentionally independent from `sky_player_rs`. The public
//! modules contain validation and transaction logic; platform modules only
//! provide bounded Win32 process/network/signature seams.

pub mod archive;
pub mod cli;
pub mod error;
pub mod github;
pub mod http;
pub mod install;
pub mod manifest;
pub mod process;
pub mod recovery;
pub mod restart;
pub mod result;
pub mod signature;
pub mod transaction;
pub mod version;

pub const APP_NAME: &str = "Sky-Auto-Player";
pub const PRIMARY_EXE: &str = "Sky-Auto-Player.exe";
pub const UPDATER_EXE: &str = "Sky-Auto-Player-Updater.exe";
pub const MANIFEST_NAME: &str = "MANIFEST.json";
pub const SCHEMA_VERSION: u32 = 2;

pub const API_MAX_BYTES: usize = 1024 * 1024;
pub const SIDECAR_MAX_BYTES: usize = 16 * 1024;
pub const MANIFEST_MAX_BYTES: usize = 4 * 1024 * 1024;
pub const ZIP_MAX_COMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
pub const ZIP_MAX_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;
pub const ZIP_MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
pub const ZIP_MAX_ENTRIES: usize = 20_000;
