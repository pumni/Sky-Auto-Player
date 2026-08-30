# Phase 7 desktop evidence

Phase 7 adds the Core-backed Settings surface, all five GUI themes, and the
typed update/check/handoff surface. It does not activate final Tauri packaging
or change `bundle.active` (which remains `false`).

## UI evidence

The deterministic Playwright/axe suite covers the supported 920×620 minimum
viewport and verifies:

- Settings persistence controls and focus-safe modal behavior;
- `aurora`, `minimalist`, `slate`, `cyberpunk`, and `classic` through
  `html[data-theme]` and the Core-shaped mock bridge;
- update-available indicator, typed update dialog, and handoff-ready state;
- existing Library, Song Detail, diagnostics, calibration, paging, keyboard,
  and reduced-motion behavior.

The managed execution session did not expose a visible interactive Tauri window
for a new capture, so the Phase 7 browser images are not labeled as real-Tauri
captures. Existing real-window evidence remains in `desktop-nonphysical/` and
`desktop-playback/`. No physical input was synthesized for this evidence.

## Source and provenance

- Desktop implementation source: public `repo_head` `fc549b6`.
- Native/updater provenance: the checked-in Rust `sky_updater` source and its
  deterministic test fixture; no final release binary is claimed here.
- Exact packaged-artifact qualification remains a Phase 8 gate.
