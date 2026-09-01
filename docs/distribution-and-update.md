# Distribution and Update Model

This is the normative contract for the unsigned, portable Windows package and
its user-triggered native updater. It tracks the native desktop Cargo version
and is independent of the Rust playback dispatcher.

## 1. Distribution

Sky Auto Player has one canonical portable release package:

```text
Sky-Auto-Player-v<version>.zip
Sky-Auto-Player-v<version>.zip.sha256
MANIFEST.json
```

The ZIP expands to one folder containing the native Tauri application,
`native_calibration.exe`, and the canonical `Sky-Auto-Player-Updater.exe`.
There are no BAT/PowerShell updater scripts, system installer, legacy
executable name, bridge ZIP, or second bundle. Runtime-owned paths are kept
outside the public package when they are not part of the application itself.

Public binaries are intentionally unsigned. Authenticode is recorded as
`N/A — intentionally unsigned`; no PFX, certificate secret, signing step, or
verified-publisher claim is required. The exact ZIP SHA256, exact manifest,
clean-worktree/native provenance, and GitHub build attestation are the release
integrity/provenance evidence.

The GitHub repository and its release pipeline are the runtime trust root.
SHA256 and `MANIFEST.json` verify the bytes and file set delivered by that
release; they are integrity checks, not independent publisher authentication.

Prerelease tags (`vX.Y.ZrcN`, `vX.Y.Z.devN`, and equivalent PEP 440 forms) are
created as draft GitHub releases and published as prereleases for beta-channel
validation only after qualification. Stable tags are published only after the
repository's exact-artifact, manifest, provenance, fresh-install, and Defender
qualification gates pass. Published tags and assets are immutable; fixes
require a new version.

Release publication is draft-first. A version tag runs the release workflow,
which builds and attests the exact ZIP and manifest, then creates a
draft GitHub Release. The draft is the qualification input: it is published
as a prerelease or stable release only after exact-artifact and platform
qualification pass. Assets are not replaced and the tag is not moved between
draft creation and publication; a failed qualification requires a new version
or RC instead.

## 2. Runtime ownership

The Native Desktop Runtime owns update checking, stable/beta selection,
update-notice state, and the fixed manual fallback URL. It does not download,
extract, replace, delete, or restart application files itself. The native
updater owns the visible progress window and the durable update lifecycle.

When the user chooses **Update and Restart**, the Native Runtime first validates the
currently installed `MANIFEST.json` and the updater's exact size/SHA256. It
copies the updater into an allow-listed random run directory under
`%LOCALAPPDATA%\Sky-Auto-Player\update-runs\`, starts it with `shell=False`,
and waits for a bounded ready/rejected handoff at
`<run_root>\handoff.json`. The ready record is published only after the native
progress UI, per-install lock, and active-update state are established. The
desktop exits the UI only after the ready handshake; a rejected handshake leaves the
app running. The Rust updater then:

1. waits for the parent app without terminating or injecting into it;
2. reports `Fetching`, `Verifying`, `Extracting`, `Preflight`, `Backing up`,
   `Installing`, `Cleaning up`, and `Restarting` through the native progress UI;
3. fetches the exact target release over the GitHub HTTPS allow-list;
4. verifies the ZIP sidecar, release `MANIFEST.json`, archive paths, and every
   staged file before mutation;
5. prepares and applies a transactional managed-file update while preserving
   user-owned paths; and
6. writes a durable result and restarts the canonical app only after a verified
   success or verified rollback.

The updater writes schema-1 result records with bounded provenance fields
(`phase`, `operation`, `path`, and `os_error`). Post-commit cleanup is
best-effort: failures become bounded warnings and `cleanup_pending: true`; they
never roll back a verified installation. Restart is still attempted after such
warnings, and the app reports them after consuming the result.

If installation succeeds but automatic restart fails, the new installation is
kept, `last-result.json` is overwritten with `status: "failure"` and
`error_code: "RESTART_FAILED"`, and the updater does not roll the installation
back solely because the process could not be started.

The modal also offers **Open GitHub Releases**, **Remind me later**, and
**Skip this version**. The manual path opens only:
`https://github.com/pumni/Sky-Auto-Player/releases`.

The updater is intentionally non-elevating. A portable installation must be
in a user-writable directory; the package does not install a service or invoke
UAC.

Before any parent wait, recovery, download, verification, or installation
work, the updater acquires an exclusive per-install lock at
`%LOCALAPPDATA%\Sky-Auto-Player\update-locks\<sha256(canonical-root)>.lock`.
The OS handle is held through result persistence and restart. A second updater
returns `UPDATE_ALREADY_RUNNING` immediately and does not create or touch a
transaction directory.

The canonical-root hash uses the Windows verbatim canonical form (`\\?\` for
drive paths and `\\?\UNC\` for UNC paths) so the Rust updater's lock/active
identity and the native desktop startup guard derive the same `install_id`.

The active lifecycle record is bounded and atomic at:

```text
%LOCALAPPDATA%\Sky-Auto-Player\update-state\active-update.json
```

It records the canonical install identity, updater PID, run ID, run root, and
current phase. It is created only after the progress window and lock exist,
updated at phase boundaries, and removed before the updater exits. On packaged
Windows startup, the native shell validates that any active PID is a live canonical
`Sky-Auto-Player-Updater.exe` inside the expected `update-runs\run-<32 hex>`
directory; a valid active update makes startup exit cleanly with the bounded
output `Sky Auto Player is currently updating to vX (Phase). The updater window
will restart the app automatically.`

## 3. Release selection and network

The desktop GUI is the canonical user-facing selector. Stable excludes prereleases;
beta may include them. The checker uses:

```text
stable: https://api.github.com/repos/pumni/Sky-Auto-Player/releases/latest
beta:   https://api.github.com/repos/pumni/Sky-Auto-Player/releases?per_page=10
```

The selected release must be non-draft, tag/version-matching, channel
compatible, and contain exactly these canonical assets:

```text
Sky-Auto-Player-v<target>.zip
Sky-Auto-Player-v<target>.zip.sha256
MANIFEST.json
```

The native updater uses HTTPS only and checks redirects against:
`api.github.com`, `github.com`, `objects.githubusercontent.com`, and
`release-assets.githubusercontent.com`. Userinfo, HTTP URLs, arbitrary API
bases, arbitrary mirrors, shell downloads, and TLS-verification bypasses are
rejected. Release metadata never supplies the manual browser destination.

## 4. Archive and manifest safety

The Rust updater downloads outside the install root and validates every ZIP
entry before extraction. It rejects absolute, drive-qualified, UNC,
traversal, alternate-data-stream, symlink, duplicate, case-colliding,
file/directory collision, reserved-device, and trailing-dot/space paths.
Windows case-insensitive path identity is used throughout. Explicit directory
entries are valid parents for files; a file used as a parent is not valid.

Release and updater version ordering uses the same PEP 440 semantics as Python
`packaging.version`, including development, prerelease, post, padding, and
local versions.

`MANIFEST.json` is schema version `2`. It records the exact app ID, target
version, canonical executable, clean-worktree/native provenance, and a unique
list of every shipped file except the manifest itself. Each entry's size and
SHA256 must match, and the staged file set must equal the manifest file set.
The manifest must include at least:

```text
Sky-Auto-Player.exe
native_calibration.exe
Sky-Auto-Player-Updater.exe
```

The native updater verifies unsigned project-owned files by SHA256. It has no
runtime signature-bypass flag because Authenticode is not part of this public
unsigned release contract.

## 5. Managed and preserved files

The updater's transaction plan distinguishes managed application files from
these user-owned paths, which are never replaced or deleted by an update:

```text
config.json
.env
songs/**
logs/**
```

Preserved-path matching is Windows case-insensitive. A package that places a
managed file below a preserved directory is rejected. The transaction journal
is computed before mutation and records replacements, additions, managed
orphans, and backups; preserved paths are excluded from those mutations.

Before the first mutation, the updater preflights every existing managed
destination using the same replace-compatible Windows access requirements as
the atomic operation. A sharing violation returns `INSTALL_TARGET_BUSY` with
the relative path and Win32 error code; no transaction is created and no
installation file is modified.

Complete backups are then created and a flushed `prepared` journal is
atomically written under:

```text
<install>\.sky-update-transaction\
    journal.json
    backup\
```

Managed payloads are copied to same-volume temporary files beside each
destination, flushed, hash-verified, and atomically replaced (`ReplaceFileW` on
Windows). Existing destinations are replaced with a same-directory emergency
backup name and `ReplaceFileW` flags `0`; the failure path reconciles the
destination, emergency backup, and temporary file before any cleanup. A
temporary or emergency backup is never removed while the canonical destination
is missing or ambiguous. The Windows preflight requests read/delete/
synchronize access while sharing read/write/delete, matching the replacement
operation rather than requiring write access. Apply ordering is explicit:
normal files, primary executable, calibration executable, updater executable,
and `MANIFEST.json` last.

Rollback is restore-first. Each verified backup is prepared and atomically
restored without deleting its current destination; the updater is restored
first, followed by the primary executable and calibration binary. Pure-new
additions are removed only after every old backup has been restored and
verified. If any restore or cleanup step fails, the current destination and
transaction material remain for the next recovery attempt; the app is not
restarted. A later run recovers a `Prepared` transaction before starting a new
update; malformed journals fail closed. Journal and result JSON use
same-directory temporary files, flush, and atomic replace.

## 6. Result and restart handoff

The updater writes a bounded result record to:

```text
%LOCALAPPDATA%\Sky-Auto-Player\update-state\last-result.json
```

The update notice cache is separate from the durable result and is stored at:

```text
%LOCALAPPDATA%\Sky-Auto-Player\update-state\pending-release.json
```

It contains only validated release identity and bounded plain-text notes; URLs
are never persisted. The cache is written atomically and is cleared when the
user skips the version or a successful update is consumed. A missing or
mismatched cache triggers a fresh metadata check rather than an empty-notes
modal.

The app consumes it once on the next start and reports stable statuses such as
`success`, `rolled_back`, and `failure`, together with a stable error code.
The error code distinguishes at least `IO_FAILURE`, `JSON_FAILURE`,
`NETWORK_FAILURE`, `INSTALL_TARGET_BUSY`, `UPDATE_ALREADY_RUNNING`,
`INSTALL_ATOMIC_REPLACE_FAILED`, and `ROLLBACK_ATOMIC_REPLACE_FAILED`.
Failure results may additionally include bounded `phase`, `operation`, `path`,
and `os_error` fields. Logs do not contain secrets, signed redirect query
strings, song contents, or arbitrary personal file listings.

The parent PID is used only for bounded waiting. The updater never terminates
the parent, attaches a debugger, reads its memory, injects code, installs a
hook, or sends input. Restart uses the canonical app executable only after
installed project-owned files pass the manifest integrity check.

## 7. Manual fallback

If the native update cannot be staged or launched, the UI offers the official
Releases page. The user may:

1. download the canonical ZIP, sidecar, and `MANIFEST.json`;
2. verify the ZIP SHA256 and manifest;
3. extract into a new user-writable folder;
4. copy the preserved paths listed above; and
5. start `Sky-Auto-Player.exe` from the new folder.

The public package contains no legacy `updater.bat`,
`installer/updater.ps1`, Pester updater workflow, old-name resolution, or
bridge release asset.

The feature-gated local release source and deterministic fault-injection
`sky_updater_e2e.exe` are test-only artifacts and are rejected by the public
release-tree guard. E2E fault checkpoints are path-based (for example,
`apply:after-replace:Sky-Auto-Player-Updater.exe` and
`rollback:after-restore:Sky-Auto-Player-Updater.exe`) so critical windows are
tested after the updater replacement, not by an incidental file index. The
Windows E2E harness records Defender antivirus/real-time status and exclusion
paths before and after each acceptance run. It fails closed if Defender is not
enabled, if the exclusion set is not readable, or if the exclusion set changes,
and records any detections observed during the run in the evidence bundle. The
main harness and every updater/app scenario remain non-elevated; only the
feature-local Defender snapshot helper uses two explicit UAC prompts. Evidence
records `harness_elevated: false` and `defender_snapshot_elevated: true`. The
v3.4.4 → v3.4.5 is still initiated by the v3.4.4 updater. The progress UI and
lifecycle fixes in v3.4.5 apply only to updates initiated from v3.4.5 onward.
If the v3.4.4 → v3.4.5 update reports failure or does not restart the app, use
the canonical ZIP manual bridge. Installations whose existing updater predates
this transaction hardening must be moved manually to v3.4.5 before native
self-update is trusted again.

## 8. Security boundary

The updater is not a game integration. It does not modify game files, read or
write game memory, bypass anti-cheat, inject DLLs, attach debuggers, install
hooks, or send keyboard/mouse input. Playback input remains exclusively behind
the Windows `SendInput` backend, and the updater has no dependency on that
dispatcher.
