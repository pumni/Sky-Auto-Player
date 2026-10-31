# Rust toolchain policy

`rust/rust-toolchain.toml` is the single production Rust toolchain source. Historical
migration notes and release acceptance records preserve the versions they used; they do
not define the current build contract.

## Current contract

- Production compiler: the exact stable patch in `rust/rust-toolchain.toml`.
- Workspace MSRV: Rust `1.99`, declared independently by `rust/Cargo.toml`.
- Edition: Rust `2024`.
- Target: `x86_64-pc-windows-msvc`.
- Dependency resolution: `rust/Cargo.lock` is committed and native product builds use
  Cargo's locked mode.
- Root-level Cargo commands use
  `node scripts/toolchain-pin.mjs exec -- cargo ...`; `read rust` prints the canonical
  compiler pin and `verify rust` checks the active Rustup, rustc, and Cargo versions.
- Native packaged-app provenance checks continue to compare embedded compiler metadata
  with the actual compiler used by the source-bound build.

Rustup searches for a toolchain file from the process's current directory upward through
its parents. It does not search child directories. Commands started inside `rust/` find
`rust/rust-toolchain.toml`; commands started at the repository root, `desktop/`, or
`site/` do not. The repository resolver reads the nested pin and supplies the corresponding
`RUSTUP_TOOLCHAIN` for root-level Cargo commands. The `setup-rust-pinned` composite action
uses the same source for CI installation and exports that value to later steps.

## Upgrade policy

1. Start from the current `origin/main` and record the exact base commit.
2. Change the compiler only in `rust/rust-toolchain.toml`; do not run `cargo update`.
3. Run format, locked check, locked Clippy with `-D warnings`, and locked tests for the
   complete workspace, all targets, and all features.
4. Review Windows-specific runtime changes, especially TLS destructor behavior for the
   `sky_dispatch_win32` thread-local waitable timer.
5. Raise the workspace MSRV only after the compatibility pass succeeds. Do not add a new
   stable API merely to demonstrate compiler adoption.
6. Build and verify the exact native desktop artifact, including Rust version, target,
   and source commit provenance.

Compiler migration and dependency upgrades are separate changes. A dependency or lockfile
change requires its own review and evidence.

## Branch and release policy

`main` tracks the explicitly pinned current stable compiler and its deliberate MSRV.
Release branches may freeze their compiler and MSRV; they must not be rewritten as part of
a `main` migration unless a backport is explicitly requested. A reproducible regression
preserves its evidence and rolls back the compiler/MSRV change without lowering timing or
security gates.

## Qualification policy

Green CI establishes compiler/build/test compatibility only. The final native production
binary still requires the existing Rust, repository-tooling, static, security, package, and
updater gates, followed by the physical 10,000-boundary qualification and 100,000-boundary
rare-tail soak. Those runs require the isolated project-owned target window and must
preserve their raw JSON evidence. No compiler migration may claim final real-time
acceptance without those reports.
