# Security Policy

## Scope

Sky Auto Player is a Windows 11 desktop tool that reads JSON song files and simulates keyboard keypresses through the public Windows `SendInput` API so that users can play music sheets in [Sky: Children of the Light](https://www.thatskygame.com/) hands-free.

**The entire system is built on three non-negotiable security mandates.** They live in [`AGENTS.md`](./AGENTS.md) as P0 rules and are enforced by `scripts/audit_security_mandates.py`.

### 1. NO GAME TAMPERING

Sky Auto Player **never**:

- modifies or patches game files;
- reads or writes any other process's memory;
- installs Windows hooks (`SetWindowsHookEx`, `SetWinEventHook`, etc.), regardless of target;
- injects DLLs into any other process;
- attaches a debugger to any other process;
- bypasses anti-cheat.

The only hooks explicitly forbidden by the P0 audit, including keyboard hooks on any process — not just the game.

### 2. SENDINPUT ONLY

The only mechanism used to send keystrokes is `user32.SendInput` (and the legacy `keybd_event` / `mouse_event` siblings). No third-party keyboard module (`python-keyboard`, `pynput`, etc.) is loaded.

### 3. STRICT VALIDATION

Every CLI argument, config field, song file, hotkey binding, and timing profile is validated through a typed dataclass before reaching the dispatch engine. Malformed inputs are rejected with a clear error rather than silently coerced.

## Auditing

The P0 mandates are enforced both **by review** (every PR must pass CI) and **by automation** (`scripts/audit_security_mandates.py` runs as a CI gate on every push and pull request). Any new code that adds a forbidden API call — hook, memory read, remote thread, debug attach — fails CI immediately. Historical exceptions, if any, are listed in `.config/security_audit_baseline.json` with a justification and a tracking reference.

To run the audit locally:

```powershell
uv run --env-file .env python scripts/audit_security_mandates.py
```

## Reporting a Vulnerability

If you discover a way to bypass these mandates or abuse Sky Auto Player in a way that violates the P0 rules (memory tampering, hooks, DLL injection, anti-cheat evasion, etc.):

- Email **pumni.dev@gmail.com** and encrypt sensitive material at the PGP key linked from the publisher profile.
- Do **not** open a public issue for reproducer steps.
- Expect an acknowledgement within 7 days and a triage decision within 14 days.

Reports are appreciated; coordinated disclosure is the norm.

## Out of Scope

- Sky Auto Player must never be used to violate Thatgamecompany's [Sky Terms of Service](https://www.thatskygame.com/terms-of-service/). Automated playback may itself be prohibited; the user assumes that risk.
- Behaviour caused by running the binary outside its intended environment (e.g. on Windows builds we don't support, with broken permissions, or with simulated anti-cheat) is out of scope.

## Recognition

Credit is given in the next release `CHANGELOG.md` entry unless the reporter opts out.
