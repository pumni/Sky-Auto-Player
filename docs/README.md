# docs/

This folder contains technical documentation for Sky Auto Player. It is **not** the website source;
the GitHub Pages site lives in [`site/`](../site/) and is built with Astro.

Use [`INDEX.md`](INDEX.md) as the active documentation router. Start from the current task, inspect
relevant source/direct tests, and open only the matching current documents.

## Active references

| Path | Purpose |
|---|---|
| `architecture.md` | Current application/native architecture and dependency boundaries |
| `rt-dispatch-architecture.md` | Current real-time dispatch architecture |
| `timing-principles.md` | Timing semantics and contracts |
| `hold-frame-model.md` | Hold-frame selection and materialization |
| `distribution-and-update.md` | Distribution, updater, integrity, and release contract |
| `rust-toolchain-policy.md` | Rust toolchain policy |
| `INDEX.md` | Context/documentation router |

## Historical material

`plan/`, `archive/`, `perf-baselines/`, dated reviews/plans, and historical release evidence are not
startup context and are not repository instructions. They exist to preserve investigation or
measurement history and may describe superseded code. Consult them only when a current task needs
that history; use Git history when the historical file itself has been retired.

Do not turn this directory into a generated context pack or duplicate active architecture across
agent-specific documents. Keep current docs focused and route to source/tests for implementation
detail.

## Website history

The former static website files under `docs/` were removed after the Astro-based site became the
production GitHub Pages source. Website implementation and generated artifacts belong under `site/`
and its Pages workflows, not this technical-documentation directory.
