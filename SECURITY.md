# Security Policy

## Canonical security contract

This file is the source of truth for Sky Auto Player's security boundary. Project documentation
and guidance may summarize these rules, but they do not own or redefine them. The executable
enforcement gate is `cargo xtask check static`.

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

The current pre-provider v4 release policy is the temporary `unsigned-zero-budget` state: shipped
project PE files and the canonical NSIS installer are intentionally unsigned for Authenticode.
Production GA should use an approved real Authenticode signer when a provider is available; PR CI
may continue to use unsigned fixtures and must not receive production signing credentials. Windows
may show Unknown Publisher or a SmartScreen warning while the temporary policy is active; this is a
publisher-identity/UX trade-off, not a bypass of update trust.
The Rust-owned `UpdateService` selects the fixed v4 authority and the official Tauri updater
verifies the detached Ed25519 signature over the exact NSIS update artifact before installation.
V4 has no bundled custom updater executable, portable ZIP updater contract, or `MANIFEST.json.sig`
protocol. It must also preserve its HTTPS allow-list, exact artifact SHA-256, bounded provenance,
and user-triggered execution model. The current normative contract is
[`docs/distribution-and-update.md`](docs/distribution-and-update.md).

Changes to updater trust, release provenance, allowed download origins, preserved user data, or
transaction integrity are security-sensitive and require direct tests/evidence for that boundary.

### Protected v4 release authority

Production updater signing secrets belong only to the protected `v4-production-release`
Environment and the `sign_qualify` job in `.github/workflows/release-v4.yml`. The pinned Rust
control executable is built and source/hash checked before the secrets are mapped; the signing step
invokes that executable directly and does not run Cargo. Every Bun child in the secret-bearing
signing phase, including the Tauri CLI version probe and local-key challenge, clears inherited
environment state and receives only the bounded Windows runtime allowlist plus the temporary key
path/passphrase required by Tauri. Git identity subprocesses use the same runtime-only policy. A
spawned-child regression verifies the actual environment contains no synthetic signing secrets,
GitHub tokens, or unrelated secrets. This reduces secret propagation to children; the protected
ephemeral hosted job remains the signing trust boundary.

The production updater public root remains Key ID `19AABD2E7838818C`. Never place updater signing
secrets in repository/organization scope, pull-request CI, test-key proof jobs, artifacts, command
arguments, or logs. The separate metadata App private key is consumed only by its token-mint action
in the normal release workflow or the manually dispatched metadata-recovery workflow; its token is
scoped to this repository and `contents: write`.

`.github/workflows/v4-metadata-recovery.yml` is limited to an already immutable, valid stable
release. A read-only plan binds the original successful release run, release ID/tag/source SHA,
asset inventory and digests, version-bound production signature, attestations, GitHub Latest,
stable/beta metadata, and the public endpoint. Apply requires the operator-reviewed plan hash,
explicit confirmation, and protected Environment approval. It can write only
`channels/stable/latest.json` once, then requires exact readback, unchanged beta/Latest state, and
public endpoint convergence. It cannot rebuild, sign, replace assets, change a release/tag, or roll
metadata backward. Unknown write results are read back and never blindly retried. Beta auto-update
remains disabled. The operator procedure and recovery limits are in
[`v4-release-authority.md`](docs/v4-release-authority.md).

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
