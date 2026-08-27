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

## Historical evidence

`archive/`, `perf-baselines/`, and historical release evidence are not startup context and are not
repository instructions. Consult them only when a current task needs that evidence. Completed plans,
migration playbooks, and implementation choreography are retained in Git history rather than kept in
the active documentation tree.

Durable architectural decisions belong in `adr/` or the current reference documents above. Do not
turn this directory into a generated context pack or duplicate active architecture across
agent-specific documents.

## Website history

The former static website files under `docs/` were removed after the Astro-based site became the
production GitHub Pages source. Website implementation and generated artifacts belong under `site/`
and its Pages workflows, not this technical-documentation directory.
