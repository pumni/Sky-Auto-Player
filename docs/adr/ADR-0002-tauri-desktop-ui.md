# ADR-0002: Tauri Desktop UI with Shared Python Application Services

Status: accepted

Date: 2026-08-29

## Context

The v4 desktop UI needs to share application behavior with the existing Textual
picker without duplicating song discovery, metadata analysis, settings policy,
or playback preparation. The migration must preserve the current timing and
native-input boundaries while leaving a known-good fallback available.

## Decision

The desktop presentation will use Tauri 2 with React and TypeScript. A Python
Core sidecar will own shared application services and communicate with the
desktop shell over bounded newline-delimited JSON on inherited stdin/stdout
pipes. The protocol is local and versioned; it is not an HTTP server, localhost
socket, or remote-content bridge.

Rust remains the sole authority for real-time scheduling and physical playback
dispatch. The production Win32 `SendInput` boundary remains isolated in
`sky_dispatch_win32`. React will render state and submit high-level intent only.

Textual remains a supported fallback and consumes the same extracted Python
orchestration services. Migration proceeds incrementally; there is no big-bang
rewrite and no early removal of the Textual implementation.

## Consequences

- Playback preparation, catalog access, metadata coordination, settings, and
  desktop DTOs live outside presentation packages.
- Compatibility imports remain temporarily at the old Textual paths while
  callers migrate.
- Phase 0/1 introduce no Tauri runtime, protocol worker, or physical playback
  behavior change.
- Each later phase must add direct tests for behavior moved into shared
  orchestration before reducing the corresponding Textual implementation.

## Baseline evidence

Captured before Phase 1 changes on 2026-08-29 from the clean `main` checkout:

- `uv run python scripts/check.py`: PASS
- Python: 944 passed, 16 skipped, 1 expected xfail in 83.97s
- Rust format, check, clippy, and workspace tests: PASS
- Rust test totals: 50 core, 1 golden, 3 properties, 223 Win32, 237 player,
  20 no-allocation dispatch, 53 updater, plus release/update integration tests
- Static security audit: PASS; no forbidden Windows API references

The skipped tests require the optional test-support native wheel or a supplied
packaging harness. The expected xfail is the existing Windows Textual layout
timing case.

## Security boundary

This decision does not authorize game tampering, process-memory access,
debugging, hooks, code injection, anti-cheat bypass, arbitrary shell/filesystem
access from the frontend, or any gameplay-input mechanism other than the
existing Rust `SendInput` boundary. The Python Core and desktop shell must fail
closed at their respective admission and transport boundaries.
