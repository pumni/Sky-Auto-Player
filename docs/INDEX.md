# Sky Auto Player Documentation Router

Use this page to find the smallest current document set needed for a task. It is a router, not a
bundle to load in full.

## Evidence hierarchy

When sources disagree, prefer current observable evidence in this order:

1. **Observed game behavior** — captured onsets/audio and reproducible behavior.
2. **Deterministic native telemetry and frozen test vectors** — measured implementation evidence.
3. **Current source, direct tests, enforced configuration, and CI checks** — the executable codebase.
4. **Current documentation below** — explanatory contracts that must be updated when intentional
   code changes make them inaccurate.

Historical plans, audit reports, issue text, and obsolete implementation notes do not override these
sources.

## Active architecture and behavior

- [architecture.md](architecture.md) — Native Tauri/Rust layering, dependency direction, and
  component ownership.
- [architecture-target.md](architecture-target.md) — superseded Rust-first migration target and
  dependency invariants; use `architecture.md` for the current runtime.
- [wave2-native-application-services.md](wave2-native-application-services.md) — historical Wave 2
  application-core, adapter, ownership, event, and deliberate cutover evidence.
- [wave3-native-desktop-ownership.md](wave3-native-desktop-ownership.md) — historical Wave 3 native
  command ownership and composition evidence; superseded by the current native boundary.
- [rt-dispatch-architecture.md](rt-dispatch-architecture.md) — current native real-time dispatch
  contract and runtime boundary.
- [timing-principles.md](timing-principles.md) — timing semantics, targets, measurement domains, and
  fail-closed behavior.
- [hold-frame-model.md](hold-frame-model.md) — user-selected FPS and authored hold materialization.
- [library-v1-contract.md](library-v1-contract.md) — native manifest, collection, and local import
  contract for the stacked Library V1 work.

For implementation work, read relevant source and direct tests alongside the matching document. Do
not open every architecture page by default.

## Security, distribution, and toolchain

- [../SECURITY.md](../SECURITY.md) — canonical security policy and disclosure process.
- [distribution-and-update.md](distribution-and-update.md) — public package, native updater,
  integrity/provenance, rollback, and release contract.
- [v4-tauri-packaging.md](v4-tauri-packaging.md) — v4 local Tauri NSIS/updater artifact qualification
  and current-user packaging contract.
- [v4-release-authority.md](v4-release-authority.md) — v4 immutable release namespace, static
  metadata, channel isolation, and read-only acceptance contract.
- [v4-updater-key-custody.md](v4-updater-key-custody.md) — v4 updater trust root custody, backup,
  loss, compromise, rotation, and recovery runbook.
- [v4-authenticode-provider-seam.md](v4-authenticode-provider-seam.md) — v4 production Authenticode
  provider seam specification and contract.
- [v4-release-execution-topology.md](v4-release-execution-topology.md) — v4 production release
  execution topology, candidate lifecycle, and provenance boundary.
- [rust-toolchain-policy.md](rust-toolchain-policy.md) — Rust compiler/toolchain policy.
- [wave5-legacy-python-retirement.md](wave5-legacy-python-retirement.md) — historical evidence for
  the completed product/runtime Python retirement; not a tooling instruction.
- [wave6-rust-xtask-release-ci.md](wave6-rust-xtask-release-ci.md) — Rust xtask commands and the
  Rust/Bun canonical check/release boundary and zero-Python guard.

Build and release details live in `rust/xtask/`, `desktop/`, `.github/workflows/`, and direct
tests. Tauri NSIS is the canonical v4 package producer; `cargo xtask` verifies and qualifies
the resulting artifacts and does not assemble a portable v4 ZIP.

## Development and verification

The repository-level verification entry point is:

```powershell
cargo xtask check all
```

Use `static`, `tests`, or `rust` as an optional group during focused development. Packaging,
Windows timing acceptance, release, and benchmark scripts are specialized evidence paths; use them
when a task touches those boundaries.

## Decisions and bounded evidence

- `docs/adr/` contains explicit architecture decision records. Consult an ADR only when the task
  touches the decision it records.
- `docs/releases/` contains release-specific acceptance/history, not startup context.
- `docs/perf-baselines/` contains named performance evidence. A baseline is evidence for the
  environment and revision it records, not a universal instruction.
- Completed plans, superseded audits, and historical briefs live in Git history (`git log`).

## Documentation maintenance

Keep active docs about the current system. Convert durable decisions into ADRs or current
contracts; retire completed plans to Git history. Keep this router concise.
