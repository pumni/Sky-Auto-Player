# Sky Auto Player Documentation Router

This router identifies the canonical documentation owner for each system concern.

## Evidence hierarchy

When sources disagree, prefer current observable evidence in this order:

1. **Observed game behavior** — captured onsets/audio and reproducible behavior.
2. **Deterministic native telemetry and frozen test vectors** — measured implementation evidence.
3. **Current source, direct tests, enforced configuration, and CI checks** — executable repository truth.
4. **Current contracts below** — explanatory specifications updated alongside code changes.

Historical plans, audit reports, issue text, and obsolete work orders do not override these sources.

## Active architecture and runtime

- [architecture.md](architecture.md) — Native Tauri/Rust layering, dependency direction, module ownership, and unsafe boundary.
- [rt-dispatch-architecture.md](rt-dispatch-architecture.md) — Native real-time dispatch contract, thread split, hot-path constraints, and runtime boundary.
- [timing-principles.md](timing-principles.md) — Timing semantics, targets, measurement domains, and fail-closed behavior.
- [hold-frame-model.md](hold-frame-model.md) — User-selected FPS and authored hold materialization.
- [tuning-presets.md](tuning-presets.md) — Native settings, playback preferences, and runtime tuning boundaries.
- [library-v1-contract.md](library-v1-contract.md) — Native manifest, collection, and local import contract.

## Security, distribution, and toolchain

- [../SECURITY.md](../SECURITY.md) — Canonical security policy and disclosure process.
- [distribution-and-update.md](distribution-and-update.md) — Public package, native updater, integrity/provenance, and release contract.
- [v4-tauri-packaging.md](v4-tauri-packaging.md) — v4 local Tauri NSIS/updater artifact qualification and current-user packaging contract.
- [v4-release-authority.md](v4-release-authority.md) — One-repository v4 release/update runbook, immutable GitHub Releases, and channel contract.
- [v4-updater-key-custody.md](v4-updater-key-custody.md) — v4 updater trust root custody, backup, loss, compromise, rotation, and recovery runbook.
- [v4-authenticode-provider-seam.md](v4-authenticode-provider-seam.md) — v4 production Authenticode provider seam specification.
- [v4-release-execution-topology.md](v4-release-execution-topology.md) — v4 production release execution topology, candidate lifecycle, and provenance boundary.
- [rust-toolchain-policy.md](rust-toolchain-policy.md) — Rust compiler and workspace toolchain policy.
- [desktop-startup-benchmark.md](desktop-startup-benchmark.md) — Opt-in startup markers, milestone definitions, and packaged benchmark methodology.

## Development and verification

The repository-level verification entry point is:

```powershell
cargo xtask check all
```

Use `static`, `rust`, or `desktop` for focused local development:

```powershell
cargo xtask check static
cargo xtask check rust
cargo xtask check desktop
```

Packaging, Windows timing acceptance, release, and benchmark scripts are specialized evidence paths used when a change touches those boundaries.

## Decisions and bounded evidence

- `docs/adr/` — Architecture Decision Records for specific structural choices:
  - [adr/ADR-0001-packetized-native-input-dispatch.md](adr/ADR-0001-packetized-native-input-dispatch.md)
  - [adr/ADR-0002-tauri-desktop-ui.md](adr/ADR-0002-tauri-desktop-ui.md)
  - [adr/ADR-0003-rust-first-clean-architecture.md](adr/ADR-0003-rust-first-clean-architecture.md)
  - [adr/ADR-0004-direct-tauri-rust-ipc.md](adr/ADR-0004-direct-tauri-rust-ipc.md)
  - [adr/ADR-0005-rust-xtask-release.md](adr/ADR-0005-rust-xtask-release.md)
  - [adr/ADR-0006-v4-distribution-installation-update.md](adr/ADR-0006-v4-distribution-installation-update.md)
  - [adr/ADR-0007-single-repository-v4-release-architecture.md](adr/ADR-0007-single-repository-v4-release-architecture.md)
  - [adr/ADR-0008-v4-builtin-song-catalog.md](adr/ADR-0008-v4-builtin-song-catalog.md)
  - [adr/ADR-0009-v4-github-latest-policy.md](adr/ADR-0009-v4-github-latest-policy.md)
  - [adr/ADR-0010-physical-timing-feasibility.md](adr/ADR-0010-physical-timing-feasibility.md)
  - [adr/ADR-0011-normal-playback-down-continuity.md](adr/ADR-0011-normal-playback-down-continuity.md)
  - [adr/ADR-0012-user-configurable-normal-down-continuity.md](adr/ADR-0012-user-configurable-normal-down-continuity.md)
- `docs/releases/` — Release-specific qualification and acceptance records.
- Completed experiments, past migration records, raw measurement evidence, and historical performance baselines have been retired from the active documentation tree and remain preserved in Git history (`git log`).
