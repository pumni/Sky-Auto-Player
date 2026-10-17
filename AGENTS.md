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

## Mandatory naming policy

- Names for active source paths, types, functions, tests, scripts, command arguments, environment
  variables, and configuration fields must describe their lasting behavior or responsibility.
- Do not encode the implementation sequence in maintained names with phase, sprint, work-order, or
  optimization-wave markers such as `r3_`, `p4_`, `phase8_`, `wo04_`, or `h4_`.
- Keep a numbered name only when it is a real version/schema identifier or immutable historical or
  external contract. Document that exception narrowly; do not add a compatibility alias without
  evidence that an external consumer needs it.
- A domain state machine may use “phase” when it describes an actual runtime state. The naming
  audit in `cargo xtask check static` enforces the implementation-name rule.

## Validation policy

- During editing, run focused tests for the changed boundary.
- Before review, run `cargo xtask check all` once against the final relevant source state. It
  contains the static, Rust, desktop, and native desktop checks; do not automatically run those
  groups individually and then repeat them through `check all`.
- Repeat a passed gate only when a later change invalidates that gate's inputs or coverage.
- Do not repeat full cold/warm performance baselines or native acceptance without a new behavior or
  performance change.
- Record the exact command, HEAD and relevant input identity, exit code, and coverage. Report only
  checks that actually ran; explicitly identify checks that were skipped.
- Run packaging, Windows timing/latency acceptance, release, and benchmark workflows only when the
  changed boundary requires them.
