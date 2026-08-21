# Rust toolchain policy

This is the current source of truth for the native Rust toolchain. Historical
migration notes and release acceptance records retain the versions used when
they were written; they do not define the current build contract.

## Current contract

- Production compiler: Rust `1.98.0`, pinned by `rust/rust-toolchain.toml`.
- Workspace MSRV: Rust `1.98`, declared by `rust/Cargo.toml`.
- Edition: Rust `2024`.
- Target: `x86_64-pc-windows-msvc`.
- Dependency resolution: `rust/Cargo.lock` is committed and must remain
  unchanged during a compiler-only migration. CI, release, and native wheel
  builds use Cargo's locked mode.
- Every root-level native build, including `build_app`, explicitly exports
  `RUSTUP_TOOLCHAIN=1.98.0`; the shipped wheel, calibration binary, and updater
  are each built with `--locked`.
- Native wheel and packaged-app provenance checks require the embedded
  `rustc_version` metadata to start with the exact pinned compiler prefix.

The nested toolchain file is discovered reliably when commands run from
`rust/`. Root-level commands must set `RUSTUP_TOOLCHAIN=1.98.0` explicitly,
as the CI and release workflows do.

## Upgrade policy

1. Start from the current `origin/main` and record the exact base commit.
2. Pin the candidate compiler without running `cargo update`.
3. Run format, locked check, locked Clippy with `-D warnings`, and locked tests
   for the complete workspace, all targets, and all features.
4. Review Windows-specific runtime changes, especially TLS destructor behavior
   for the `sky_dispatch_win32` thread-local waitable timer.
5. Raise the workspace MSRV only after the compatibility pass succeeds. Do not
   add a new stable API merely to demonstrate compiler adoption.
6. Build and verify the exact free-threaded native wheel, including Rust
   version, target ABI, and source commit provenance.

Compiler migration and dependency upgrades are separate changes. A dependency
or lockfile change requires its own review and evidence.

## Branch and release policy

`main` tracks the explicitly pinned current stable compiler and its deliberate
MSRV. Release branches may freeze their compiler and MSRV; they must not be
rewritten as part of a `main` migration unless a backport is explicitly
requested. A reproducible regression preserves its evidence and rolls back the
compiler/MSRV change without lowering timing or security gates.

## Qualification policy

Green CI establishes compiler/build/test compatibility only. The final native
1.98 production binary still requires the existing Rust, Python, static,
security, package, and updater gates, followed by the physical 10,000-boundary
qualification and 100,000-boundary rare-tail soak. Those runs require the
isolated project-owned target window and must preserve their raw JSON evidence.
No compiler migration may claim final real-time acceptance without those
reports.
