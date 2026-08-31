//! Pure application/domain services for the Rust-first migration.
//!
//! Every module in this crate is backed by a current production behavior and
//! is deliberately independent from delivery, filesystem, network, Python,
//! and Windows APIs. Concrete effects are expressed as inward ports and are
//! implemented by `sky_native_adapters` or existing outer services.

#![forbid(unsafe_code)]

pub mod catalog;
pub mod settings;
pub mod update;
