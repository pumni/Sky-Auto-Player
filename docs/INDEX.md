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
- `docs/releases/` — Release-specific acceptance evidence and verification records.
- `docs/perf-baselines/` — Measured performance baselines for specific environments and revisions.
- `docs/evidence/` — Raw experimental and verification evidence.

## Historical evidence and superseded contracts

- [history/timing-experiments.md](history/timing-experiments.md) — Historical timing experiments and calibration procedures (non-normative).
- [history/protocol-9-raw-input.md](history/protocol-9-raw-input.md) — Historical protocol-9 Raw Input observer mechanics (superseded).
- [history/v3/distribution-and-update.md](history/v3/distribution-and-update.md) — Historical v3 portable ZIP and standalone updater mechanics (superseded).
- `docs/history/migrations/` — Past transition summaries (`wave2` through `wave6`, `architecture-target.md`, `v4-clean-distribution-execution.md`, retirement ledgers) preserving past transition evidence.
- Completed plans, superseded audits, and historical implementation work orders live in Git history (`git log`).
