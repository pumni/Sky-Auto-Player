---
key: security-boundaries
locale: en
slug: security-boundaries
title: Security Boundaries
description: >-
  Sky Auto Player's three non-negotiable security mandates: SendInput-only input simulation,
  no game tampering, and strict input validation. Source code is public and CI-audited.
summary: >-
  Sky Auto Player uses only the Windows SendInput API for keystrokes. It never reads game
  memory, modifies game files, hooks processes, or injects code. Three security mandates are
  enforced in CI on every commit.
category: technical-safety
order: 1
published: '2026-08-08'
updated: '2026-08-08'
lastReviewedVersion: '3.0.0'
draft: false
related:
  - how-it-works
  - windows-setup
  - timing-engine
evidence:
  - category: security
    label: SECURITY.md — full security policy
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/SECURITY.md
  - category: security
    label: Security audit script (CI gate)
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/scripts/audit_security_mandates.py
  - category: implementation
    label: Windows platform layer — only place SendInput lives
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/src/sky_music/platform/
  - category: distribution
    label: Updater — SHA256 verify, preserve-list, HTTPS allow-list
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/installer/updater.ps1
---

## Three security mandates

Sky Auto Player is built on three non-negotiable rules enforced in CI on every push and
pull request. Violating any of them causes an immediate build failure.

### 1. No game tampering

Sky Auto Player **never**:

- Reads or writes any other process's memory
- Modifies, patches, or inspects game files
- Injects DLLs into any process
- Attaches a debugger to any process
- Installs Windows hooks (`SetWindowsHookEx`, `SetWinEventHook`, or similar) on any target
- Bypasses anti-cheat systems

These restrictions apply to all code in the repository — not just code that targets the game.

### 2. SendInput only

The only mechanism for simulating keyboard input is `user32.SendInput`. Legacy
`keybd_event` / `mouse_event` calls and any third-party input library (`python-keyboard`,
`pynput`, etc.) are forbidden. The Windows platform layer (`src/sky_music/platform/`) is
the only place in the codebase where `SendInput` and Win32 types may live.

### 3. Strict input validation

Every CLI argument, config field, song file, hotkey binding, and timing parameter is
validated through a typed data structure before reaching the dispatch engine. Malformed
inputs are rejected with a clear error — not silently coerced.

## CI enforcement

An automated audit script (`scripts/audit_security_mandates.py`) scans both the Python
source under `src/` and the Rust source under `rust/` on every push and pull request.
Any commit that introduces a forbidden API call (hook, memory read, remote thread, debug
attach) fails CI immediately. Historical exceptions, if any, are listed in
`.config/security_audit_baseline.json` with a justification and a tracking reference.

To run the audit locally:

```powershell
uv run --env-file .env python scripts/audit_security_mandates.py
```

## Source code

Sky Auto Player is open source under the GNU General Public License v3.0. The full source
is available at [github.com/pumni/Sky-Auto-Player](https://github.com/pumni/Sky-Auto-Player).
You can audit the code, build from source, or verify that the release binary matches
the source.

## Release verification

Every release produces three files:

- `Sky-Auto-Player-v<version>.zip` — the application archive
- `Sky-Auto-Player-v<version>.zip.sha256` — the SHA256 checksum
- `MANIFEST.json` — a manifest listing all asset checksums

Verify the archive before extracting:

```powershell
(Get-FileHash "Sky-Auto-Player-v3.0.0.zip" -Algorithm SHA256).Hash
# Compare with the content of the .sha256 file
```

## Updater security

The external updater (`updater.bat` → `installer/updater.ps1`) enforces:

- HTTPS connections only, to an explicit allow-list of GitHub hosts
- SHA256 verification of the downloaded archive before any file is overwritten
- Transactional copy with rollback if any step fails
- A process guard that refuses to run while `Sky-Auto-Player.exe` is locked

The updater never replaces `config.json` or the `songs/` folder.

## Terms of Service notice

Automated playback may conflict with Sky: Children of the Light's Terms of Service. Sky
Auto Player is an unofficial community project and is not affiliated with or endorsed by
thatgamecompany. Use responsibly and at your own risk.

## Reporting a vulnerability

Email **pumni.dev@gmail.com**. Do not open a public issue for reproducer steps. Expect an
acknowledgement within 7 days and a triage decision within 14 days.
