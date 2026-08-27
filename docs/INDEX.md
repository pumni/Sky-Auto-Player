# Sky Auto Player Documentation Router

Use this page to find the smallest current document set needed for a task. Do not treat this index as
an instruction bundle to preload.

## Evidence hierarchy

When sources disagree, prefer current observable evidence in this order:

1. **Observed game behavior** — captured onsets/audio and reproducible behavior.
2. **Deterministic native telemetry and frozen test vectors** — measured implementation evidence.
3. **Current source, direct tests, enforced configuration, and CI checks** — the executable codebase.
4. **Active documentation below** — explanatory contracts that must be updated when the code changes
   intentionally.

Historical plans, audit reports, baselines, issue text, and archived documents do not override these
sources.

## Active architecture and behavior

- [architecture.md](architecture.md) — Python/Rust layering, dependency direction, and component
  ownership.
- [rt-dispatch-architecture.md](rt-dispatch-architecture.md) — current native real-time dispatch
  contract and runtime boundary.
- [timing-principles.md](timing-principles.md) — timing semantics, targets, measurement domains, and
  fail-closed behavior.
- [hold-frame-model.md](hold-frame-model.md) — user-selected FPS and authored hold materialization.

For implementation work, read the relevant source and direct tests alongside the matching document;
do not read every architecture page by default.

## Security, distribution, and toolchain

- [../SECURITY.md](../SECURITY.md) — canonical security policy and disclosure process.
- [distribution-and-update.md](distribution-and-update.md) — public package, native updater,
  integrity/provenance, rollback, and release contract.
- [rust-toolchain-policy.md](rust-toolchain-policy.md) — Rust compiler/toolchain policy.

Build and release implementation details also live in `src/build_app.py`, `Sky-Auto-Player.spec`,
`.github/workflows/`, and their direct tests. Treat those executable surfaces as current-state
evidence rather than duplicating them into agent instructions.

## Development and verification

The repository-level verification entry point is:

```powershell
uv run python scripts/check.py
```

Use `static`, `tests`, or `rust` as an optional group during focused development. Packaging,
Windows timing acceptance, release, and benchmark scripts are specialized evidence paths; use them
when a task touches those boundaries.

## Decisions and historical evidence

- `docs/adr/` contains explicit architecture decision records. Consult an ADR when the current task
  touches the decision it records.
- `docs/releases/` contains release-specific acceptance/history, not startup context.
- `docs/perf-baselines/` contains named performance evidence. A baseline is evidence for the
  environment and revision it records, not a universal instruction.
- `docs/plan/`, top-level dated plan/review documents, and `docs/archive/` are historical working
  material. They may describe superseded code or implementation choreography and are **not active
  repository authority** unless the current human task explicitly adopts one.

Do not maintain a catalog of every historical plan here. Use repository search or Git history when a
current investigation genuinely requires historical rationale.

## Documentation maintenance

Keep active docs about the current system, keep this router concise, and move obsolete explanatory
material out of the active path instead of adding warnings and more routing layers. A new agent or
context framework is not a documentation substitute; source, tests, focused docs, and executable
verification are the default context system.