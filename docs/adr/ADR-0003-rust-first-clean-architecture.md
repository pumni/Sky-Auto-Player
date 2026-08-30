# ADR-0003: Rust-first Clean Architecture for the Desktop Core

Status: accepted

Date: 2026-08-31

## Context

The Tauri desktop currently delegates application behavior to a Python child
process, which then calls a Rust engine through PyO3. This transitional shape
duplicates IPC, lifecycle, validation, and build boundaries after the Tauri
surface has become the canonical desktop UI.

## Decision

Move authoritative desktop application/domain behavior to a pure Rust
`sky_app_core`; keep Tauri as the delivery adapter and composition root; keep
the native player/dispatch and updater as specialized outer implementations.
Remove Python from the shipped runtime only after parity and safety gates pass.

The migration is incremental and every intermediate `main` remains releasable.

## Relationship to ADR-0002

This ADR amends and supersedes ADR-0002 only regarding the long-term ownership
of shared Python application services and the Python Core sidecar. ADR-0002's
decision to use Tauri for the desktop UI, its incremental migration rule, Rust
authority for realtime work, `SendInput` security boundary, and fail-closed
requirements remain valid. ADR-0002 is not wholly obsolete; its accepted UI
and safety constraints continue to govern this migration.

## Constraints

- preserve gameplay input/timing safety and the Windows `SendInput` boundary;
- preserve updater transaction, rollback, provenance, and startup admission;
- preserve the frontend `DesktopBridge` contract during core migration;
- keep application-core tests independent of Tauri, Win32, PyO3, and concrete I/O.

## Consequences

The target removes one local process and its custom protocol, CPython/PyO3
production coupling, and duplicated application model layers. It requires
temporary dual implementations, behavior-parity fixtures, and a larger Rust
compile graph during migration.

## Alternatives rejected

1. Keep Python Core permanently as a local microservice: unnecessary process
   lifecycle and protocol cost for this desktop product.
2. Big-bang rewrite: excessive regression and rollback risk.
3. Move authoritative orchestration to TypeScript: weakens state ownership and
   separation from timing/security-sensitive work.
