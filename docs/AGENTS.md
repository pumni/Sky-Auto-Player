# Documentation-Scope Instructions

These instructions apply to edits under `docs/` in addition to
[../AGENTS.md](../AGENTS.md). The documentation router is [INDEX.md](INDEX.md).

- Classify the document before editing it: **current contract/runbook**,
  **task-specific method/reference**, or **revision-bound historical evidence**
  (including ADRs, qualification results, and release records).
- For a current contract, verify claims against current source, tests,
  configuration, and the applicable security/release constraints. Update the
  router when the responsible current document changes.
- Historical evidence is not a standing instruction or an acceptance for HEAD.
  Preserve recorded versions, SHA identities, measurements, decisions, and
  superseded descriptions. Correct demonstrable factual errors with explicit
  provenance; do not silently rewrite the history as current behavior.
- Do not delete material, rename documented identifiers, or merge distinct
  safety/timing/release contracts just to reduce context size. Move references
  out of the always-read agent guide instead of deleting the information.
- Prefer links to the single owning document over copying its requirements
  across new agent files. Keep task-specific research and audit output
  discoverable but out of automatic agent loading.
- For agent-context audits use
  [agent-context-maintenance.md](agent-context-maintenance.md); keep low-confidence
  or model-specific style preferences as findings until behavior is tested.
