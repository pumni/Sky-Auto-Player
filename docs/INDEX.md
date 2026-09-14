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

- [architecture.md](architecture.md) — Native Tauri/Rust layering, dependency direction, and component ownership.
- [rt-dispatch-architecture.md](rt-dispatch-architecture.md) — Native real-time dispatch contract and runtime boundary.
- [timing-principles.md](timing-principles.md) — Timing semantics, targets, measurement domains, and fail-closed behavior.
- [hold-frame-model.md](hold-frame-model.md) — User-selected FPS and authored hold materialization.
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

- `docs/adr/` — Architecture Decision Records for specific structural choices.
- `docs/releases/` — Release-specific acceptance evidence and verification records.
- `docs/perf-baselines/` — Measured performance baselines for specific environments and revisions.
- Completed plans, superseded audits, and historical implementation work orders live in Git history (`git log`).
- Historical migration summaries (`wave2-native-application-services.md`, `wave3-native-desktop-ownership.md`, `wave4-final-native-ownership.md`, `wave5-legacy-python-retirement.md`, `wave6-rust-xtask-release-ci.md`, `architecture-target.md`) preserve past transition evidence and are not current operational contracts.
