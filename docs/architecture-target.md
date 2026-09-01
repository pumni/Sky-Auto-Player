# Target Desktop Architecture (superseded by Wave 5)

Wave 5 completed this target. For the current supported product boundary, use
[`architecture.md`](architecture.md) and
[`wave5-legacy-python-retirement.md`](wave5-legacy-python-retirement.md).

This document describes the Rust-first migration target. It is retained as
migration history; the current runtime and ownership are documented in
[architecture.md](architecture.md).

```text
React / TypeScript
        │ Tauri invoke + Channel
        ▼
sky_desktop_shell  ── composition root and delivery adapter
        │
        ├── sky_app_core ── pure application/domain home; ports added just-in-time
        ├── sky_player ──── playback adapter
        ├── sky_updater ─── hardened update adapter
        └── dispatch adapters ── sky_dispatch_core + sky_dispatch_win32

xtask ── developer/release tooling only
```

## Dependency rules

- `sky_app_core` has no Tauri, Win32, PyO3, WebView, or concrete filesystem/HTTP
  dependency. It remains an architecture-only foundation until a subsystem
  migration supplies current-behavior evidence and parity fixtures for each
  model or port introduced.
- `sky_player` remains a pure Rust production engine; the temporary Python
  bridge is isolated and deleted after parity.
- `sky_dispatch_core` remains platform-neutral; `sky_dispatch_win32` owns QPC,
  focus, timing, and the only `SendInput` syscall.
- `sky_desktop_shell` owns Tauri commands, composition, concrete adapters, and
  UI event translation; it does not become a second business-logic layer.
- `sky_updater` keeps its existing HTTPS allow-list, exact artifact checks,
  transaction/rollback, preserve-list, recovery, and provenance behavior.

## Migration invariants

- migrate one behavioral seam per PR and keep `main` releasable;
- preserve the frontend `DesktopBridge` command/DTO/event contract during core
  migration;
- keep hard realtime timing in the native scheduler, never in JavaScript or
  async timers;
- do not delete a Python surface until callers, parity tests, and package
  self-tests prove it is no longer required by the canonical desktop path;
- exact portable qualification remains mandatory on `main` and release tags.

## Intended production end state

The shipped desktop has no Python process, CPython runtime, PyO3/maturin
production boundary, PyInstaller sidecar, or custom Rust-to-Python IPC.
