# Distribution and Update Model

This document is the normative contract for portable distribution and updates.
It tracks the version in `pyproject.toml` and is independent of the Rust
real-time playback dispatcher.

## 1. Distribution

Sky Auto Player is portable and has exactly one release package:

```text
Sky-Auto-Player-v<version>.zip
Sky-Auto-Player-v<version>.zip.sha256
MANIFEST.json
```

The ZIP contains `Sky-Auto-Player.exe`, `Sky-Auto-Player-Updater.exe`,
`native_calibration.exe`, `MANIFEST.json`, `README.md`, `config.json`,
`songs/`, and `_internal/`. No BAT/PowerShell updater, system installer,
legacy executable name, bridge ZIP, or second bundle is supported.

The tag version must equal `[project].version`. Release packaging is
fail-closed when the worktree is dirty, native provenance does not match, or
the required signing provider is unavailable. The release workflow provisions
an encrypted PFX identity into the ephemeral runner certificate store from
`AUTHENTICODE_PFX_BASE64` and `AUTHENTICODE_PFX_PASSWORD`, verifies the exact
`AUTHENTICODE_CERT_THUMBPRINT` has a private key, signs project-owned PE files,
and only then generates the manifest. These are CI secrets; the PFX is never
committed or retained in the workspace.

## 2. Runtime ownership

The Python app owns update checking, stable/beta selection, the update modal,
playback shutdown, and launching the updater. It never extracts an archive or
replaces its own running files.

The Rust `sky_updater` binary owns exact-tag release fetch, bounded download,
SHA256 verification, archive validation, staging, manifest and Authenticode
verification, durable transaction, rollback, result reporting, and restart.
It has no Python, Textual, playback, or `SendInput` dependency.

When the user selects **Update now**:

1. playback performs normal graceful stop and mandatory key cleanup;
2. Python copies the bundled updater to
   `%LOCALAPPDATA%\Sky-Auto-Player\update-runs\<run-id>\`;
3. Python verifies the copied updater hash and launches it without a shell,
   passing the install root, parent PID, current/target versions, channel, and
   restart intent;
4. the app exits only after launch succeeds;
5. the updater waits for the parent to exit, then refetches the exact target
   GitHub tag and applies the transaction;
6. the updater writes the structured result atomically, then restarts only a
   verified app (including the verified old app after a successful rollback).

The first native-updater release may require users of older releases to
download and extract manually once. There is no migration code or compatibility
promise for the old updater.

## 3. Release selection and network

Python remains the user-facing selector. Stable excludes prereleases; beta may
include them. The native updater receives only a validated `stable` or `beta`
channel and independently requests:

```text
https://api.github.com/repos/pumni/Sky-Auto-Player/releases/tags/v<target>
```

It requires an exact tag, a non-draft release, a policy-compatible prerelease
flag, and exactly these assets:

```text
Sky-Auto-Player-v<target>.zip
Sky-Auto-Player-v<target>.zip.sha256
MANIFEST.json
```

All requests are HTTPS. Every redirect is checked against this allow-list:
`api.github.com`, `github.com`, `objects.githubusercontent.com`, and
`release-assets.githubusercontent.com`. Userinfo, HTTP URLs, arbitrary API
bases, arbitrary mirrors, shell downloads, and TLS-verification bypasses are
rejected.

Downloads are bounded streams. The default bounds are 1 MiB for API JSON,
16 KiB for a SHA sidecar, 4 MiB for an external manifest, 256 MiB compressed
ZIP, 512 MiB total uncompressed data, 256 MiB per entry, and 20,000 entries.

The sidecar must contain exactly one meaningful record for the expected ZIP
filename, with exactly 64 hexadecimal characters. The ZIP hash is checked
before extraction or installation mutation.

## 4. Archive and manifest safety

Extraction occurs outside the install root only after every archive entry is
validated. The updater rejects absolute, drive-qualified, UNC, traversal,
alternate-data-stream, symlink, duplicate, case-colliding, file/directory
collision, reserved-device, and trailing-dot/space paths.

ZIP path identity follows Windows case-insensitive semantics. Explicit directory
entries are valid parents for files; a file used as a parent, duplicate path,
or case-folded file/directory collision is rejected. Release and updater version
ordering uses the same PEP 440 semantics as Python `packaging.version`,
including post-zero, development, prerelease, release-padding, and local
versions.

`MANIFEST.json` uses schema version `2` and contains the exact app ID,
target version, canonical executable, clean-worktree/provenance fields, and a
unique list of every staged file except the manifest itself. That single
self-reference exception is explicit: the manifest is not hashed in its own
`files` list. Every listed size and SHA256 must match, and the staged file set
must equal the manifest set plus `MANIFEST.json`.

Project-owned PE files are, at minimum, the app, updater, calibration binary,
and `_internal/**/sky_player_rs*.pyd`. Production requires a valid trusted
Authenticode chain from the configured project publisher. Development builds
may be unsigned only through a compile-time debug/test path; there is no
production runtime bypass.

## 5. Managed and preserved files

These paths are never overwritten and never orphan-deleted:

```text
config.json
.env
songs/**
logs/**
```

Everything else is managed only when it is present in the old or new manifest.
The updater never deletes arbitrary unmanifested user files. A missing or
corrupt installed manifest fails closed for orphan deletion and, once the
native architecture is established, for automatic update.

The transaction plan is computed before mutation:

```text
files_to_replace
files_to_add
managed_orphans_to_delete
backup_paths
```

Preserved-path matching is Windows case-insensitive. A package that places a
managed file below a preserved directory is rejected.

## 6. Durable transaction and recovery

Transaction state is kept at:

```text
<install>\.sky-update-transaction\
    journal.json
    backup\
```

Before the first install mutation the updater creates complete backups for all
overwritten/deleted files, records new paths that rollback must remove, and
atomically flushes a `prepared` journal. Rollback first verifies that every
recorded backup exists and matches its SHA256, and preflights every managed
target/ancestor for Windows reparse points. Only then may it copy or delete
managed paths. Installed hashes are checked before writing `committed`.

Any failure after `prepared` removes new managed paths, restores backups, and
verifies restored hashes. If recovery cannot be proven, backup material is
retained and the app is not restarted. A later run handles `prepared` before a
new update; a `committed` journal is verified/cleaned. Malformed journals fail
closed.

Journal and result JSON use same-directory temporary files, flush, and atomic
replace. Results are written to:

```text
%LOCALAPPDATA%\Sky-Auto-Player\update-state\last-result.json
```

with stable statuses such as `success`, `rolled_back`, and `failure`, plus a
stable error code. Logs must not contain secrets, signed redirect query
strings, song contents, or arbitrary personal file listings.

## 7. Parent process and dry-run

The updater validates an absolute existing install root, canonical primary
executable, valid PEP 440 current/target versions, strictly increasing target,
exact channel, nonzero parent PID, and recognizable installed layout before
network activity. It opens the parent only with minimum wait/query rights,
never terminates it, and fails without mutation on a bounded timeout. Where
practical, the parent image path must resolve to the expected installed app.

`--dry-run` may fetch, verify, validate, and stage. It never recovers a
transaction, creates a backup, mutates the install root, or restarts. An
unresolved transaction is reported nonzero.

## 8. Security boundary

The updater does not modify game files, read/write game memory, inject DLLs,
attach debuggers, install hooks, bypass anti-cheat, or send input. The Rust
playback dispatcher remains protected and is not a dependency of the updater.

The old `updater.bat`, `installer/updater.ps1`, Pester tests/actions, old-name
resolution, and bridge release assets are removed from the active architecture.
