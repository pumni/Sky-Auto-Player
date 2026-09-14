# Sky Auto Player — Repository Guide

Windows 11 music-sheet player for Sky: Children of the Light. The desktop application reads JSON,
skysheet, and TXT sheets and emits gameplay keyboard input through Windows `SendInput`.

## External mutation boundary

Do not create, publish, close, merge, retarget, or otherwise mutate remote branches, pull requests,
issues, releases, or other externally visible project state unless the user's current request
explicitly authorizes that external action. Ordinary local work (source edits, local investigation,
targeted tests, formatting) within the requested scope does not require separate authorization.

## Security and input boundary

- Never modify game files, read or write another process's memory, bypass anti-cheat, inject code,
  attach a debugger, or install process/input hooks (`SetWindowsHookEx`, `SetWinEventHook`, etc.).
- Gameplay keystroke simulation uses Windows `SendInput` only. Never introduce `python-keyboard`,
  `pynput`, `keybd_event`, `mouse_event`, or other injection mechanisms.
- Validate external and user inputs; fail closed on ambiguous or malformed security-sensitive data.
- `SECURITY.md` is canonical for the security contract; `cargo xtask check static` enforces it.

## Repository map

- `desktop/src/` — React / TypeScript UI and bridge projections.
- `desktop/src-tauri/` — Tauri shell, commands, and composition root.
- `rust/crates/sky_app_core/` — Pure application and domain policies.
- `rust/crates/sky_native_adapters/` — Concrete OS, filesystem, and process adapters.
- `rust/crates/sky_player/` — Playback application and real-time runtime service.
- `rust/crates/sky_dispatch_core/` — Platform-independent dispatch logic.
- `rust/crates/sky_dispatch_win32/` — Windows `SendInput` and native dispatch boundary.
- `rust/xtask/` — Canonical verification, artifact qualification, and release tooling.
- `tests/` — Direct regression, golden, and Windows behavior tests.
- `scripts/` — Narrow host and CI scripts where Rust is not a better fit.
- `site/` — Marketing and documentation website (Astro/Pages).
- `docs/` — Canonical technical documentation (see `docs/INDEX.md`).

## Stable architecture invariants

- Domain and orchestration crates stay independent of Win32 implementation details and direct
  `SendInput`; see `docs/architecture.md` and boundary tests for exact ownership.
- The supported product and canonical repository verification are Rust/Bun native. Python must not
  be reintroduced into the product runtime or repository tooling.
- Native updater and release integrity contracts follow `docs/distribution-and-update.md`.
- Source, tests, and configuration are primary evidence. Consult `docs/INDEX.md` to locate current
  canonical documentation on demand.

## Verification

Canonical repository verification:

```powershell
cargo xtask check all
```

Narrower checks for focused development:

```powershell
cargo xtask check static
cargo xtask check rust
cargo xtask check desktop
```

Packaging, Windows timing/latency acceptance, release, and benchmark workflows are specialized
checks run when changes touch those boundaries.
