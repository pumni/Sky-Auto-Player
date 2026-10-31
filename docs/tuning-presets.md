# Native tuning presets

The supported desktop is the native Tauri application. Playback scheduling,
focus admission, timing, diagnostics, and calibration are implemented by the
Rust services and the qualified native player; there is no alternate
runtime or terminal UI preset to select.

## Where to tune

Use the current settings UI for supported playback and telemetry preferences.
For development-only investigations, use the focused native tests and
fixtures under `rust/crates/sky_player`, `rust/crates/sky_dispatch_*`, and
`desktop/src-tauri`. Keep changes to realtime timing behind the existing
no-allocation, assembly, focus, release-all, and player qualification gates.

The production invariants remain fixed:

- QPC-backed authored deadlines and the qualified wait strategy;
- `SendInput` as the only gameplay input boundary;
- `Auto` attempts the MMCSS `Games` task with high task priority, then uses
  `THREAD_PRIORITY_HIGHEST` if MMCSS acquisition fails. It does not register
  the `Pro Audio` task. The supervisor lease remains an independent fail-safe;
- the production waiter requires a high-resolution waitable timer at startup.
  Six startup wake samples initialize a heuristic; they do not establish a
  percentile, service level, or receiver-timing guarantee;
- bounded nonblocking realtime queues and emergency release-all cleanup;
- exact-HWND foreground admission and the native final focus gate.

QPC is the monotonic time domain for schedule waits and sender evidence. The
waitable timer receives a negative relative interval in 100-nanosecond units;
QPC ticks are not converted or passed as FILETIME. `timeBeginPeriod` can affect
waitable-timer resolution on some paths, but does not increase QPC precision.

The application's supported product target is Windows 11 x64. The production
waiter uses `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`, which Microsoft documents
as supported starting with Windows 10 version 1803. Production treats failure
to create that timer as a startup error rather than falling back to coarse
sleep polling. See [ADR-0018](adr/ADR-0018-resumable-release-barrier-and-final-lease-age.md)
for the timing, lifecycle, and evidence contract.

## Release and runtime checks

Release and repository verification use the Rust `cargo xtask` tooling.
Historical Textual/free-threaded/PyInstaller tuning notes are retained only as
evidence in historical records and are not supported product instructions.
