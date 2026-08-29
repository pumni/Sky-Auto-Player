# Desktop playback evidence

These screenshots were captured from the real Windows Tauri WebView through
the production `core_main.py` path on 2026-08-29. No mock bridge was used.

- `idle-real-tauri.png` — fresh Core, no selected song.
- `confirmation-real-tauri.png` — high-risk dry-run preparation with the
  exact risk decision buttons visible.
- `dryrun-real-tauri.png` — completed dry-run lifecycle in the Player Dock.
- `error-real-tauri.png` — native focus rejection surfaced as a user-facing
  playback error.

A real physical `playing`/`paused` screenshot was not captured in this
environment because no Sky target window was available. No SendInput was
synthesized for visual evidence. Physical lifecycle, pause/resume and
emergency cleanup are covered by the test backend and supervisor tests.
