# ADR-0005: Rust `xtask` Owns Exact Portable Release Assembly

Status: accepted

Date: 2026-08-31

## Context

The exact portable release is currently assembled by Python and includes the
PyO3/PyInstaller production chain. Retaining a Python toolchain solely for
release orchestration would preserve avoidable setup and packaging cost after
the desktop runtime becomes Rust-native.

## Decision

Introduce a repository-local Rust `xtask` for version checks, distribution
assembly, manifest/provenance generation, verification, and release evidence.
The existing `sky_updater` remains the production updater and is not replaced
by this decision.

## Non-goals

- do not switch the portable ZIP to MSI/NSIS as part of this ADR;
- do not claim bit-for-bit reproducible PE files without evidence;
- do not remove exact updater, rollback, provenance, or artifact-integrity gates.

## Required parity

The Rust tool must preserve exact release-tree verification, hashes, provenance,
updater fault/recovery qualification, and the draft-release/attestation flow
before the Python release path is deleted.
