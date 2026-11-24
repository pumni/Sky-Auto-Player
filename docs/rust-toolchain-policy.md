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
`rust/rust-toolchain.toml`; commands started at the repository root, `apps/desktop/`, or
`apps/website/` do not. The repository resolver reads the nested pin and supplies the corresponding
`RUSTUP_TOOLCHAIN` for root-level Cargo commands. The `setup-rust-pinned` composite action
uses the same source for CI installation and exports that value to later steps.

## Rust lint policy

The required Rust validation has two complementary lanes. The existing
workspace Clippy pass uses all targets and all features with `-D warnings`.
It protects test and qualification code but does not establish production
lint cleanliness: enabling `test-support` can hide unused production items.

The production lint pass in `cargo xtask check rust` checks each first-party
library independently with no default features, library targets only, the
`dist` profile, locked dependencies and `-D warnings`. It also checks the
standalone xtask binary. `cargo xtask check desktop-native` checks the desktop
library and binary independently of test features in both desktop-runtime
and packaged production configurations, with `-D warnings`. The packaged
lint uses the `dist` profile with the existing no-frontend-assets check
configuration; the actual packaged artifact is still built and verified by
the existing package lane. Neither lint lane replaces the other.

Fix unused production items at their ownership boundary: delete genuinely
unused code or restrict test-only items and their imports to the features
that consume them. Do not add blanket `allow(dead_code)`, `allow(unused)`,
or warning-level compiler flags simply to pass a gate. Existing justified
lint exceptions must be reviewed in context; a green gate is not evidence
that every suppression is justified. Architecture audit advisories are a
separate policy and are not compiler warnings.

## Upgrade policy

1. Start from the current `origin/main` and record the exact base commit.
2. Change the compiler only in `rust/rust-toolchain.toml`; do not run `cargo update`.
3. Run the complete pinned `cargo xtask check all` gate, including production
   lint, all-target/all-feature Clippy with `-D warnings`, and locked tests.
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
