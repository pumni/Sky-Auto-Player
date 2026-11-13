# Sky Auto Player — Agent Guide

Windows 11 Tauri 2/React desktop with Rust-native sheet playback for Sky.

## Non-negotiable boundaries

- Gameplay input uses Windows `SendInput` only. No game-memory access,
  file tampering, hooks, injection, debugger attach, or anti-cheat bypass.
  See [SECURITY.md](SECURITY.md).
- Keep domain/orchestration separate from Win32 effects. Preserve native
  timing, focus, and cleanup; browser mocks do not qualify native timing.
  For related work read [architecture](docs/architecture.md) and
  [playback](docs/playback-engine.md).
- Preserve updater/signing/release trust; consult
  [release authority](docs/v4-release-authority.md) and
  [distribution](docs/distribution-and-update.md) when it is in scope.
- Rust and Bun are the product toolchains. No Python product runtime;
  hosted Python ClamAV is a narrow release-only exception.

## Context on demand

- `apps/desktop/` and `apps/website/` have separate Bun manifests and
  lockfiles; the Rust workspace is in `rust/`.
- Start with task-relevant source, tests, and config at the checked-out SHA.
  Use [docs/INDEX.md](docs/INDEX.md) to find a *relevant* contract, not to
  preload the documentation tree for routine edits.
- Active contracts state intended invariants; code/tests show implementation.
  Investigate conflicts rather than freezing a historical design.
  [Historical records](docs/HISTORY.md) apply only to their pinned revisions.

## Work and verification

- Evaluate alternatives for substantial refactors; preserve correctness and
  live identifiers, not an exact module layout or old naming scheme.
- Local edits and disposable tests are within scope. Remote branch, PR,
  issue, credential, release, and deployment writes need authorization.
- Use focused tests and relevant lint/format while iterating. Report exact
  commands, outcomes, skips, and limitations for intermediate review.
- For final merge readiness, run `cargo xtask check all` on final relevant
  source and verify required CI on exact PR HEAD; do not repeatedly run
  full suites on unchanged inputs or claim an unrun/failed gate passed.
- Update the owning contract when its behavior changes; avoid duplicate
  instruction layers.
