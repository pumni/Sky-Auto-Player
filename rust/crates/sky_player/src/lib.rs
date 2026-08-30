//! Pure Rust playback engine.
//!
//! This crate contains the authoritative scheduler/session implementation.
//! Delivery adapters such as the temporary `sky_player_rs` wheel depend on
//! it, while the engine itself remains independent of Python and UI stacks.

pub mod engine;
