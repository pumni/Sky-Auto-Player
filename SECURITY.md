# Security Policy

## Canonical security contract

This file is the source of truth for Sky Auto Player's security boundary. Agent guidance may
summarize these rules, but it does not own or redefine them. The executable enforcement gate is
`cargo xtask check static`.

Sky Auto Player is a Windows 11 desktop tool that reads music-sheet files and simulates keyboard
keypresses through the public Windows `SendInput` API so users can play music sheets in
[Sky: Children of the Light](https://www.thatskygame.com/) hands-free.

The entire system is built on three non-negotiable mandates.

### 1. No game or process tampering

Sky Auto Player **never**:

- modifies or patches game files;
- reads or writes another process's memory;
- installs Windows hooks (`SetWindowsHookEx`, `SetWinEventHook`, etc.), regardless of target;
- injects DLLs or code into another process;
- attaches a debugger to another process;
- bypasses anti-cheat or interferes with anti-cheat operation.

The hook prohibition applies to input/keyboard hooks as well as game-targeted hooks.

### 2. `SendInput` only for gameplay input

The only supported mechanism for gameplay keystroke simulation is `user32.SendInput`. Legacy
`keybd_event` / `mouse_event` calls and third-party input injection modules such as
`python-keyboard`, `pynput`, and similar tooling are forbidden.

Platform APIs used for observation, timing, UI, calibration, packaging, or updater behavior do not
authorize a second gameplay-input mechanism and must remain inside their documented boundaries.

### 3. Strict validation and fail-closed security behavior

CLI arguments, configuration, song inputs, hotkey bindings, timing values, updater inputs, and other
external data must be validated before reaching sensitive execution boundaries. Malformed,
unsupported, ambiguous, or integrity-invalid security-sensitive input is rejected rather than
silently coerced into a permissive behavior.

## Auditing

The mandates above are enforced by review and by the Rust xtask security audit, which scans the
native product source for forbidden hooks, process-memory APIs, remote-thread/debug tooling,
legacy input APIs, and disallowed Win32 surfaces. It also enforces the explicit approved
`windows_sys` module boundary. New references fail the audit.
Historical exceptions, if any, are recorded in `.config/security_audit_baseline.json` with their
justification and tracking reference.

Run the security gate locally with:

```powershell
cargo xtask check static
```

The normal repository verification entry point (`cargo xtask check static`) includes this
audit in its `static` group.

## Update and release integrity

The v4 release policy is `unsigned-zero-budget`: shipped project PE files and the canonical NSIS
installer are intentionally unsigned for Authenticode. Windows may show Unknown Publisher or a
SmartScreen warning; this is a publisher-identity/UX trade-off, not a bypass of update trust.
The Rust-owned `UpdateService` selects the fixed v4 authority and the official Tauri updater
verifies the detached Ed25519 signature over the exact NSIS update artifact before installation.
V4 has no bundled custom updater executable, portable ZIP updater contract, or `MANIFEST.json.sig`
protocol. It must also preserve its HTTPS allow-list, exact artifact SHA-256, bounded provenance,
and user-triggered execution model. The current normative contract is
[`docs/distribution-and-update.md`](docs/distribution-and-update.md).

Changes to updater trust, release provenance, allowed download origins, preserved user data, or
transaction integrity are security-sensitive and require direct tests/evidence for that boundary.

## Reporting a Vulnerability

If you discover a way to bypass these mandates or abuse Sky Auto Player through memory tampering,
hooks, DLL/code injection, anti-cheat evasion, update-integrity bypass, or a comparable security
issue:

- Email **pumni.dev@gmail.com** and encrypt sensitive material at the PGP key linked from the
  publisher profile.
- Do **not** open a public issue containing reproducer steps.
- Expect an acknowledgement within 7 days and a triage decision within 14 days.

Reports are appreciated; coordinated disclosure is the norm.

## Out of Scope

- Sky Auto Player must never be used to violate Thatgamecompany's
  [Sky Terms of Service](https://www.thatskygame.com/terms-of-service/). Automated playback may
  itself be prohibited; the user assumes that risk.
- Behavior caused by running the binary outside its intended environment (for example unsupported
  Windows builds, broken permissions, or simulated anti-cheat environments) is out of scope.

## Recognition

Credit is given in the next release `CHANGELOG.md` entry unless the reporter opts out.
