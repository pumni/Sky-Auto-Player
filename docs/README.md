# Technical Documentation

This directory holds current technical contracts, operating runbooks,
task-specific research methods, and revision-bound records for Sky Auto Player.

Start at [INDEX.md](INDEX.md) **when a task needs documentation**. It routes
by responsibility and distinguishes authoritative current contracts from
historical studies. Do not treat the whole `docs/` tree as an agent instruction
file or load every document at session startup.

## How to read or change a document

- **Current contracts/runbooks** describe intended supported behavior or a
  controlled operating procedure. Verify claims with current code, tests,
  configuration, and the relevant security/release policy.
- **Research/reference methods** describe how to estimate, test, or analyze
  behavior. Their outputs do not automatically prove native or game-side results.
- **Historical records** capture measurements, implementation phases, old
  decisions, acceptance results, or migration evidence at recorded revisions.
  Preserve their provenance rather than rewriting them to match HEAD.

`adr/` holds architectural decision records; `evidence/` holds bounded
observations and audit reports; `releases/` holds release-specific records.
Some historical records remain at the top of `docs/` for stable links.

For repository documentation edits, see [AGENTS.md](AGENTS.md); for instruction
and context audits, see
[agent-context-maintenance.md](agent-context-maintenance.md). The public
Astro website lives in [site/](../site/).
