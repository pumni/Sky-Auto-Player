# Technical Documentation

This directory contains canonical technical specifications, architectural contracts, operational runbooks, and release records for Sky Auto Player.

The documentation router is:

- [`INDEX.md`](INDEX.md) — The sole canonical router mapping system concerns to authoritative documents.

The public website lives in [`site/`](../site/) and is built with Astro.

## Structure

- `adr/` — Architecture Decision Records for structural decisions.
- `releases/` — Release-specific qualification and acceptance records.

Some completed experiments, migration records, raw evidence, and performance baselines remain in
this tree as bounded records. They describe the revisions and conditions stated in each record;
they are not standing instructions or a source of current implementation behavior. For current
behavior, use source, tests, and configuration at the checked-out SHA. Use [`INDEX.md`](INDEX.md) to
find the current contract for a task. Git history preserves earlier revisions of tracked records.
