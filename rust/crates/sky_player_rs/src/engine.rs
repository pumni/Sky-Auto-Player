//! Compatibility facade for the legacy `sky_player_rs::engine` path.
//!
//! The implementation lives in the pure Rust `sky_player` crate. Keeping
//! this facade preserves the paths used by the existing wheel and native
//! regression tests while the PyO3 layer remains a temporary adapter.

pub use sky_player::engine::*;
