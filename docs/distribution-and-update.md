# Distribution and Update Model

This is the normative contract for the unsigned, portable Windows package and
its user-triggered native updater. It tracks `[project].version` in
`pyproject.toml` and is independent of the Rust playback dispatcher.

## 1. Distribution

Sky Auto Player has one canonical portable release package:

```text
Sky-Auto-Player-v<version>.zip
Sky-Auto-Player-v<version>.zip.sha256
MANIFEST.json
```

The ZIP expands to one folder containing the PyInstaller application,
`native_calibration.exe`, and the canonical `Sky-Auto-Player-Updater.exe`.
There are no BAT/PowerShell updater scripts, system installer, legacy
executable name, bridge ZIP, or second bundle. Runtime-owned paths are kept
outside the public package when they are not part of the application itself.

Public binaries are intentionally unsigned. Authenticode is recorded as
`N/A — intentionally unsigned`; no PFX, certificate secret, signing step, or
verified-publisher claim is required. The exact ZIP SHA256, exact manifest,
clean-worktree/native provenance, and GitHub build attestation are the release
integrity/provenance evidence.

Prerelease tags (`vX.Y.ZrcN`, `vX.Y.Z.devN`, and equivalent PEP 440 forms) are
published as GitHub prereleases for beta-channel validation. Stable tags are
published only after the repository's exact-artifact, manifest, provenance,
fresh-install, and Defender qualification gates pass. Published tags and
assets are immutable; fixes require a new version.

## 2. Runtime ownership

Python owns update checking, stable/beta selection, the modal, update-notice
state, and the fixed manual fallback URL. It does not download, extract,
replace, delete, or restart application files itself.

When the user chooses **Update and Restart**, Python first validates the
currently installed `MANIFEST.json` and the updater's exact size/SHA256. It
copies the updater into an allow-listed random run directory under
`%LOCALAPPDATA%\Sky-Auto-Player\update-runs\`, starts it with `shell=False`,
and exits the UI. The Rust updater then:

1. waits for the parent app without terminating or injecting into it;
2. fetches the exact target release over the GitHub HTTPS allow-list;
3. verifies the ZIP sidecar, release `MANIFEST.json`, archive paths, and every
   staged file before mutation;
4. prepares and applies a transactional managed-file update while preserving
   user-owned paths; and
5. writes a durable result and restarts the canonical app only after a verified
   success or verified rollback.

The modal also offers **Open GitHub Releases**, **Remind me later**, and
**Skip this version**. The manual path opens only:
`https://github.com/pumni/Sky-Auto-Player/releases`.

The updater is intentionally non-elevating. A portable installation must be
in a user-writable directory; the package does not install a service or invoke
UAC.

## 3. Release selection and network

Python remains the user-facing selector. Stable excludes prereleases; beta may
include them. The checker uses:

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
_internal/**/sky_player_rs*.pyd
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

Before the first mutation, complete backups are created and a flushed
`prepared` journal is atomically written under:

```text
<install>\.sky-update-transaction\
    journal.json
    backup\
```

Rollback removes newly managed files, restores backups, and verifies restored
hashes. If recovery cannot be proven, backup material remains and the app is
not restarted. A later run recovers a prepared transaction before starting a
new update; malformed journals fail closed. Journal and result JSON use
same-directory temporary files, flush, and atomic replace.

## 6. Result and restart handoff

The updater writes a bounded result record to:

```text
%LOCALAPPDATA%\Sky-Auto-Player\update-state\last-result.json
```

The app consumes it once on the next start and reports stable statuses such as
`success`, `rolled_back`, and `failure`, together with a stable error code.
Logs do not contain secrets, signed redirect query strings, song contents, or
arbitrary personal file listings.

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

## 8. Security boundary

The updater is not a game integration. It does not modify game files, read or
write game memory, bypass anti-cheat, inject DLLs, attach debuggers, install
hooks, or send keyboard/mouse input. Playback input remains exclusively behind
the Windows `SendInput` backend, and the updater has no dependency on that
dispatcher.
