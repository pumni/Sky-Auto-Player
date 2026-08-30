# Target Desktop Architecture

This document describes the Rust-first migration target. The current runtime
remains documented in [architecture.md](architecture.md) until each boundary
has passed its phase acceptance gate.

```text
React / TypeScript
        │ Tauri invoke + Channel
        ▼
sky_desktop_shell  ── composition root and delivery adapter
        │
        ├── sky_app_core ── domain, use cases, and inward ports
        ├── sky_player ──── playback adapter
        ├── sky_updater ─── hardened update adapter
        └── dispatch adapters ── sky_dispatch_core + sky_dispatch_win32

xtask ── developer/release tooling only
```

## Dependency rules

- `sky_app_core` has no Tauri, Win32, PyO3, WebView, or concrete filesystem/HTTP
  dependency.
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
