# Distribution and Update Model

This document is the normative contract for portable distribution and manual
updates. It tracks the version in `pyproject.toml` and is independent of the
Rust real-time playback dispatcher.

## 1. Distribution

Sky Auto Player is portable and has exactly one release package:

```text
Sky-Auto-Player-v<version>.zip
Sky-Auto-Player-v<version>.zip.sha256
MANIFEST.json
```

The ZIP contains `Sky-Auto-Player.exe`, `native_calibration.exe`,
`MANIFEST.json`, `README.md`, `config.json`, `songs/`, and
`_internal/`. It contains no native updater, BAT/PowerShell updater, system
installer, legacy executable name, bridge ZIP, or second bundle.

The tag version must equal `[project].version`. Release packaging is
fail-closed when the worktree is dirty or native provenance does not match.
Public Windows binaries are intentionally unsigned: there is currently no
trusted Authenticode publisher identity and no PFX/certificate secret is
required. `MANIFEST.json` hashes the exact unsigned bytes that are packaged.
SHA256, the exact manifest, and GitHub build provenance remain the release
integrity/provenance evidence.

Prerelease tags (`vX.Y.ZrcN`) are published directly as GitHub prereleases for
beta-channel validation. Stable tags (`vX.Y.Z`) are created as draft releases;
they are promoted and published to the stable channel only after exact-artifact,
manifest, provenance, fresh-install, and Defender qualification pass.
Authenticode is recorded as `N/A — intentionally unsigned`, not as a passing
signature check.
Published release tags and assets are immutable; fixes require a new version.

## 2. Runtime ownership

The Python app owns update checking, stable/beta selection, the update modal,
and opening the fixed official GitHub Releases page. It never downloads,
extracts, replaces, or deletes installed application files.

The Rust `sky_updater` binary is retained as a separately tested,
fail-closed security component, but it is not copied into public packages and
is not reachable from the application UI. It must continue to reject unsigned
release binaries; this release model does not weaken that boundary.

When the user selects **Open GitHub Releases**:

1. no playback shutdown is required;
2. the app opens only
   `https://github.com/pumni/Sky-Auto-Player/releases`;
3. the user downloads the canonical ZIP manually and may verify its SHA256 and
   `MANIFEST.json`;
4. the user extracts the new release into a new folder and copies preserved
   user state as needed.

## 3. Release selection and network

Python remains the user-facing selector. Stable excludes prereleases; beta may
include them. The checker requests release metadata from:

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

The checker requires the exact canonical ZIP, SHA256 sidecar, and
`MANIFEST.json` asset names before it reports an update. It does not download
or install any asset. The manual download target is always the fixed official
Releases page above, never a URL supplied by release metadata.

## 4. Archive and manifest safety

For source-only updater tests, extraction occurs outside the install root only
after every archive entry is validated. The updater rejects absolute,
drive-qualified, UNC, traversal,
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

Project-owned PE files are, at minimum, the app, calibration binary, and
`_internal/**/sky_player_rs*.pyd`. Public binaries are currently unsigned by
policy. There is no runtime signature-bypass flag, and the source-only Rust
updater continues to require its configured trusted publisher when its own
security tests exercise automatic installation.

## 5. Managed and preserved files

Automatic update mutation is disabled. These paths are user-owned and must be
preserved by a manual migration:

```text
config.json
.env
songs/**
logs/**
```

Users should extract a release into a new folder and copy the preserved paths
as needed. The app does not delete, replace, or orphan-clean anything in the
old install. The manifest and SHA256 checks are evidence for the downloaded
package, not permission for the app to mutate an existing installation.

The source-only updater's transaction plan is computed before mutation; the
public app never executes it:

```text
files_to_replace
files_to_add
managed_orphans_to_delete
backup_paths
```

Preserved-path matching is Windows case-insensitive. A package that places a
managed file below a preserved directory is rejected.

## 6. Manual update procedure

1. Open the official
   `https://github.com/pumni/Sky-Auto-Player/releases` page.
2. Download the matching `Sky-Auto-Player-v<version>.zip`,
   `.zip.sha256`, and `MANIFEST.json`.
3. Verify the ZIP SHA256 and exact manifest when desired.
4. Extract the ZIP into a new folder.
5. Copy the preserved user-owned paths listed above.
6. Start `Sky-Auto-Player.exe` from the new folder.

There is no automatic extraction, application-file replacement, restart
handoff, transaction journal, rollback, or native `Update now` command in
the public unsigned package.

## 7. Source-only updater security tests (not public)

The following transaction details belong only to the separately tested Rust
updater source. The updater is not shipped and is not reachable from the app.

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

## 8. Source-only updater validation (not public)

The parent-process and dry-run contract below is retained for source-level
security tests only; it is not an application command or public update path.

The updater validates an absolute existing install root, canonical primary
executable, valid PEP 440 current/target versions, strictly increasing target,
exact channel, nonzero parent PID, and recognizable installed layout before
network activity. It opens the parent only with minimum wait/query rights,
never terminates it, and fails without mutation on a bounded timeout. Where
practical, the parent image path must resolve to the expected installed app.

`--dry-run` may fetch, verify, validate, and stage. It never recovers a
transaction, creates a backup, mutates the install root, or restarts. An
unresolved transaction is reported nonzero.

## 9. Security boundary

The updater does not modify game files, read/write game memory, inject DLLs,
attach debuggers, install hooks, bypass anti-cheat, or send input. The Rust
playback dispatcher remains protected and is not a dependency of the updater.

The old `updater.bat`, `installer/updater.ps1`, Pester tests/actions, old-name
resolution, and bridge release assets are removed from the active architecture.
