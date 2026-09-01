# Native tuning presets

The supported desktop is the native Tauri application. Playback scheduling,
focus admission, timing, diagnostics, and calibration are implemented by the
Rust services and the qualified native player; there is no Python or terminal
UI preset to select.

## Where to tune

Use the current settings UI for supported playback and telemetry preferences.
For development-only investigations, use the focused native tests and
fixtures under `rust/crates/sky_player`, `rust/crates/sky_dispatch_*`, and
`desktop/src-tauri`. Keep changes to realtime timing behind the existing
no-allocation, assembly, focus, release-all, and player qualification gates.

The production invariants remain fixed:

- QPC-backed authored deadlines and the qualified wait strategy;
- `SendInput` as the only gameplay input boundary;
- MMCSS/priority acquisition and the supervisor lease;
- bounded nonblocking realtime queues and emergency release-all cleanup;
- exact-HWND foreground admission and the native final focus gate.

## Release and runtime checks

The temporary release tooling is documented in
[`docs/wave5-legacy-python-retirement.md`](wave5-legacy-python-retirement.md).
It may use Python as repository tooling until Wave 6 replaces it with
`cargo xtask`; the packaged application itself has no Python runtime.

Historical Textual/free-threaded/PyInstaller tuning notes are retained only in
archived migration records and are not supported product instructions.
