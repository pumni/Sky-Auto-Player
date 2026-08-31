//! Pure application/domain home for the Rust-first migration.
//!
//! The crate is intentionally an architecture-only foundation in this phase.
//! Its first real model or port must be introduced together with a concrete
//! subsystem migration, current-behavior evidence, and a parity fixture. This
//! keeps speculative contracts from becoming a second application-service
//! owner while the existing runtime remains canonical.

#![forbid(unsafe_code)]
