# ADR-0002: Tauri Desktop UI with Shared Python Application Services (historical)

Status: superseded by [ADR-0003](ADR-0003-rust-first-clean-architecture.md)
only for Python application-service ownership and the Python Core sidecar.

Date: 2026-08-29

Original decision record: [ADR-0002 at the frozen WP6 checkpoint](https://github.com/pumni/Sky-Auto-Player/blob/d4e580cb46c164b3c564d8d99b2bc5f7835724d6/docs/adr/ADR-0002-tauri-desktop-ui.md).

## Context

The v4 desktop needed to share song discovery, metadata, settings, and playback
preparation with the existing Textual picker without duplicating policy. The
migration had to preserve timing and native-input boundaries, proceed
incrementally, and avoid a risky big-bang rewrite.

## Decision

The accepted 2026-08-29 design used Tauri 2 with React/TypeScript and a Python
Core sidecar for shared services over bounded, versioned newline-delimited JSON
on inherited stdin/stdout. It was a local pipe protocol, never an HTTP server,
localhost socket, or remote-content bridge. ADR-0003 later superseded Python
service ownership only; the Tauri UI, incremental migration rule, Rust authority
for realtime behavior, and native-input safety boundaries remain valid.

Rust remains the sole authority for realtime scheduling and physical playback;
the Win32 `SendInput` boundary stays isolated in `sky_dispatch_win32`. React
renders state and submits high-level intent only.

Textual was retained during incremental migration. The later Phase 9 product
cutover made packaged Tauri/React the canonical surface; it did not change the
realtime or input boundaries.

## Consequences

Shared preparation, catalog, metadata, settings, and DTO policy were to remain
outside presentation packages; each migration step required direct tests before
the corresponding Textual implementation was reduced.

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
