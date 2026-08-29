# Phase 6 desktop evidence

Phase 6 adds diagnostics and calibration coverage without introducing a new
physical-input path.

The automated UI evidence runs through the deterministic DesktopBridge fixture:

- Diagnostics drawer: Performance, Timing, Events, and Logs tabs.
- Calibration dialog: idle, running, cancelled, success, and failure states.
- axe checks at the supported minimum viewport of 920×620.

The Phase 6 real-window capture was attempted with `cd desktop; bun run
tauri:dev` on Windows. The Tauri shell and Rust Core launch path completed,
but this managed execution session exposed no visible interactive window to the
capture tooling. Therefore no new Phase 6 PNGs are represented as real-window
evidence, and no mock screenshots are mislabeled as Tauri captures.

Existing real-Tauri screenshots remain in `desktop-nonphysical/` and
`desktop-playback/`. The Phase 6 native/backend behavior is covered by the
Rust, Python, and deterministic test-backend suites; no physical SendInput was
used for automated evidence.
