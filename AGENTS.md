# Sky Auto Player — Repository Guide

Windows 11 music-sheet player for Sky: Children of the Light. The desktop application reads JSON,
skysheet, and TXT sheets and emits gameplay keyboard input through Windows `SendInput`.

## Git workflow and external mutation boundary

Work through task-scoped branches rather than committing directly to `main`.

When the user has explicitly assigned an implementation task, the normal workflow may include,
without a second confirmation:

- creating or updating a task-specific remote branch;
- pushing commits that stay within the assigned task scope;
- opening or updating a Draft pull request for that branch;
- updating PR descriptions, checklists, and task evidence that belong to the same assigned work.

Keep externally visible changes narrow and attributable to the assigned task. Do not use an
implementation request as authorization for unrelated issue, branch, PR, release, or repository
maintenance.

The following actions still require explicit user authorization in the current session:

- merging or auto-merging a pull request;
- force-pushing or rewriting history on shared/protected branches;
- deleting remote branches;
- retargeting a pull request to a different base branch;
- publishing, editing, or deleting releases/tags;
- changing repository settings, branch protection, secrets, permissions, or CI administration;
- creating, closing, or materially changing unrelated issues or project-management state.

Before pushing task work, preserve unrelated local changes, sync the intended base branch, and record
the base SHA used for the task. Keep pull requests in Draft while implementation or required
verification is incomplete.

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
