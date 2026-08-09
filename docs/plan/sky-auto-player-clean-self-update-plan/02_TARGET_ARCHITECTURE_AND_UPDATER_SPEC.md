# Target Architecture and Native Updater Specification

## 1. Distribution model

Every release has one canonical application distribution:

```text
GitHub Release
├── Sky-Auto-Player-vX.Y.Z.zip
├── Sky-Auto-Player-vX.Y.Z.zip.sha256
└── MANIFEST.json
```

The ZIP contains:

```text
Sky-Auto-Player/
├── Sky-Auto-Player.exe
├── Sky-Auto-Player-Updater.exe
├── native_calibration.exe
├── MANIFEST.json
├── README.md
├── config.json
├── songs/
└── _internal/
    ├── python runtime
    ├── sky_player_rs*.pyd
    ├── Textual/Rich runtime
    └── ...
```

There is no system installer, BAT launcher, PowerShell updater, old executable name, or second distribution ZIP.

## 2. Runtime ownership

### `Sky-Auto-Player.exe`

Owns:

- Textual UI;
- configuration;
- song parsing;
- playback orchestration;
- update availability checks;
- update modal;
- graceful playback shutdown before update;
- copying/launching the updater runner;
- reading updater result on next startup;
- cleaning stale updater-run directories.

Does not own:

- application payload installation;
- archive extraction into install root;
- replacement of its own running binaries;
- rollback;
- managed-orphan deletion during update.

### `Sky-Auto-Player-Updater.exe`

A small Windows-native Rust binary dedicated to:

- exact target release fetch;
- HTTPS download;
- redirect allow-list enforcement;
- outer ZIP SHA256;
- archive safety validation;
- staging extraction;
- manifest validation;
- Authenticode validation;
- parent-process exit wait;
- transaction planning;
- durable backup/journal;
- install;
- post-copy hash verification;
- rollback;
- result reporting;
- restart.

It does not import Python, Textual, playback scheduler code, or `SendInput`.

`sky_updater` must not depend on `sky_player_rs`.

## 3. Python layering

Recommended new/changed files:

```text
src/sky_music/
├── domain/
│   └── update_checker.py
├── orchestration/
│   └── update_service.py
├── infrastructure/
│   ├── hotkeys.py
│   └── update_launcher.py
└── platform/
    └── win32/
        ├── global_hotkeys.py
        └── window_target.py
```

`update_launcher.py` may use normal Python filesystem/process APIs, but must not contain Win32 `ctypes`.

Low-level Win32 binding belongs under `platform/win32/`.

## 4. User update sequence

```text
Background check
      │
      ▼
Update available
      │
      ▼
User selects Update now
      │
      ▼
Graceful playback stop + mandatory key cleanup
      │
      ▼
Copy bundled updater to
%LOCALAPPDATA%\Sky-Auto-Player\update-runs\<run-id>\
      │
      ▼
Launch temp updater
      │
      ▼
Main app exits
      │
      ▼
Updater waits for parent exit
      │
      ▼
Fetch exact target GitHub release by tag
      │
      ▼
Download ZIP + SHA sidecar
      │
      ▼
Verify → stage → manifest → signature
      │
      ▼
Prepare durable transaction
      │
      ▼
Install managed files
      │
      ▼
Verify installed hashes
  ┌───┴────┐
  │        │
success   fail
  │        │
commit   rollback
  │        │
  └───┬────┘
      ▼
Write updater result
      │
      ▼
Restart valid app
```

## 5. Why updater runs from `%LOCALAPPDATA%`

The installed updater is itself a managed application file. Running it from the install root makes replacing it fragile on Windows.

The app copies:

```text
<install>\Sky-Auto-Player-Updater.exe
```

to:

```text
%LOCALAPPDATA%\Sky-Auto-Player\update-runs\<random-run-id>\
    Sky-Auto-Player-Updater.exe
```

The temporary copy performs the transaction and can safely replace the installed updater.

Do not implement self-delete tricks, delayed `cmd.exe`, PowerShell cleanup, or shell scripts.

Stale run directories are cleaned by a later app startup using a conservative age policy.

## 6. Managed vs preserved files

### Preserved state — never overwrite or orphan-delete

At minimum:

```text
config.json
.env
songs/**
logs/**
```

### Managed payload

Everything else managed by the release manifest, including:

```text
Sky-Auto-Player.exe
Sky-Auto-Player-Updater.exe
native_calibration.exe
_internal/**
README.md
MANIFEST.json
```

The updater must not delete arbitrary unmanifested user files.

Managed orphan removal is based on:

```text
old installed manifest managed set
-
new staged manifest managed set
-
preserved paths
```

Never use "delete everything not in the new ZIP".

## 7. Update release selection

Python remains the user-facing selector because current code already owns stable/beta policy and PEP 440 comparison.

When user chooses update, pass to native updater:

- install root;
- parent PID;
- current version;
- target version;
- channel;
- restart intent.

The native updater independently refetches the **exact target release by tag**, instead of trusting a browser URL from Python.

Recommended GitHub API path:

```text
/repos/pumni/Sky-Auto-Player/releases/tags/v<target-version>
```

Validate:

- exact tag/target match;
- non-draft release;
- stable channel rejects prerelease target;
- exact expected assets;
- manifest version equals target.

## 8. Runtime updater workspace

Use:

```text
%LOCALAPPDATA%\Sky-Auto-Player\
├── updater.log
├── update-state\
│   └── last-result.json
└── update-runs\
    └── <run-id>\
        ├── Sky-Auto-Player-Updater.exe
        ├── download\
        └── staging\
```

Durable transaction state lives temporarily in install root:

```text
<install>\.sky-update-transaction\
├── journal.json
└── backup\
```

## 9. Rust crate structure

Add fourth workspace member:

```text
rust/crates/sky_updater/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── main.rs
    ├── cli.rs
    ├── error.rs
    ├── github.rs
    ├── http.rs
    ├── archive.rs
    ├── manifest.rs
    ├── signature.rs
    ├── transaction.rs
    ├── process.rs
    ├── install.rs
    ├── recovery.rs
    ├── result.rs
    └── restart.rs
```

Keep `main.rs` thin. Put testable logic in library modules.

## 10. Dependency policy

Prefer:

- Rust stdlib;
- existing workspace-compatible `serde` / `serde_json`;
- `windows-sys` for native Windows APIs;
- a focused SHA-256 crate;
- a focused ZIP parser/extractor.

For HTTP, prefer Windows WinHTTP via `windows-sys` rather than a large async/network runtime.

If Rust must compare PEP 440 versions for downgrade protection, use a maintained PEP 440 implementation and pin it. Do not silently substitute SemVer.

Every new dependency needs a short PR justification.

## 11. CLI contract

Recommended production CLI:

```text
Sky-Auto-Player-Updater.exe
  --install-root <absolute-path>
  --parent-pid <u32>
  --current-version <pep440-version>
  --target-version <pep440-version>
  --channel <stable|beta>
  --restart
```

Diagnostics:

```text
--dry-run
--help
--version
```

Do not expose production flags for:

- arbitrary download URL;
- arbitrary API base URL;
- disabling TLS verification;
- disabling SHA verification;
- disabling signature verification;
- disabling archive path checks;
- arbitrary executable to restart.

Tests should inject fake transports/filesystems through library seams, not unsafe runtime flags.

## 12. Startup validation

Before network activity validate:

- install root is absolute and exists;
- primary executable resolves under install root;
- current/target versions parse;
- target is greater than current;
- channel is exactly stable/beta;
- stable rejects prerelease target;
- parent PID is nonzero;
- installed layout is recognizable.

Production updater should verify its **own Authenticode signature** before mutation.

A development signature bypass, if necessary, must be compile-time/test-only, never a production runtime flag.

## 13. Parent process handling

Open parent with minimum rights needed to wait/query.

Do not terminate it.

Wait for clean exit with a bounded timeout.

If timeout expires:

- fail without install mutation;
- write result;
- do not force-kill the app.

Where practical, verify parent image path corresponds to the expected installed `Sky-Auto-Player.exe`.

## 14. Network contract

HTTPS only.

Initial GitHub API host:

```text
api.github.com
```

Allowed redirect/download hosts:

```text
api.github.com
github.com
objects.githubusercontent.com
release-assets.githubusercontent.com
```

Rules:

- reject `http://`;
- reject userinfo URLs;
- validate every redirect destination;
- cap redirect count;
- reject redirect outside allow-list;
- set explicit connect/send/receive timeouts;
- fixed User-Agent;
- avoid logging signed redirect query strings.

Preferred transport: WinHTTP through `windows-sys`.

Do not shell out to `curl`, PowerShell, BITS scripts, browser, or `certutil`.

## 15. Exact release assets

Expected names:

```text
Sky-Auto-Player-v<target-version>.zip
Sky-Auto-Player-v<target-version>.zip.sha256
MANIFEST.json
```

Do not select "first zip".

## 16. Download bounds

Use bounded streaming downloads.

Suggested conservative code constants:

```text
release API JSON:      <= 1 MiB
SHA sidecar:           <= 16 KiB
external manifest:     <= 4 MiB
ZIP compressed size:   <= 256 MiB
ZIP entries:           <= 20,000
total uncompressed:    <= 512 MiB
single entry:          <= 256 MiB
```

If current legitimate release evidence needs larger bounds, adjust deliberately and document it.

Do not allocate whole ZIP into memory.

## 17. SHA sidecar parser

Require exactly one meaningful record bound to the expected ZIP filename.

Checksum:

- exactly 64 hex chars;
- exact expected filename;
- no competing second record.

Verify ZIP SHA256 before extraction.

## 18. Staging and ZIP safety

Extract only outside install root after validating **all entries**.

Reject:

- absolute paths;
- drive-qualified paths;
- UNC paths;
- `..` traversal;
- normalization outside staging;
- alternate data streams (`:` in filename component);
- symlink entries;
- duplicate normalized paths;
- case-insensitive collisions;
- file/directory collisions;
- Windows reserved device names (`CON`, `NUL`, `AUX`, `COM1`, etc.);
- trailing-dot/space ambiguity;
- entries or archive exceeding configured bounds.

After extraction, revalidate staged paths before manifest acceptance.

## 19. Manifest schema

Introduce explicit schema version, recommended `2`.

Recommended shape:

```json
{
  "schema_version": 2,
  "app": "Sky-Auto-Player",
  "version": "X.Y.Z",
  "executable": "Sky-Auto-Player.exe",
  "git_head": "...",
  "dirty_worktree": false,
  "native_build_commit": "...",
  "build_time_utc": "...",
  "files": [
    {
      "path": "Sky-Auto-Player.exe",
      "size": 123,
      "sha256": "..."
    }
  ]
}
```

Validate:

- exact app ID;
- exact target version;
- exact primary executable;
- clean-worktree release;
- valid SHA for every file;
- unique normalized paths;
- staged exact file set equals manifest file set, with one explicitly documented rule for whether manifest hashes itself;
- every size/hash matches.

Do not have ambiguous self-hash behavior.

## 20. Authenticode validation

After staging hash validation and before install mutation, validate project-owned PE files:

```text
Sky-Auto-Player.exe
Sky-Auto-Player-Updater.exe
native_calibration.exe
_internal/**/sky_player_rs*.pyd
```

Use Windows trust APIs such as `WinVerifyTrust`.

Require:

- valid signature;
- trusted chain;
- signer matches configured project publisher policy.

Do not weaken this to "has any signature".

Document signer/certificate rollover policy before enabling rigid key pinning.

## 21. Installed manifest and orphan policy

Read installed `MANIFEST.json`.

Use it to determine prior managed payload.

Do not treat arbitrary files in install root as managed.

If installed manifest is missing/corrupt:

- fail closed for destructive orphan cleanup;
- do not broadly delete unknown files.

Once native updater architecture is established, auto-update should require a valid installed manifest baseline.

## 22. Preserve policy

Never overwrite/delete:

```text
config.json
.env
songs/
logs/
```

Use Windows case-insensitive normalized path matching.

If new manifest attempts to place managed files below a preserved directory, fail as packaging error.

## 23. Transaction plan

Compute before mutation:

```text
files_to_replace
files_to_add
managed_orphans_to_delete
backup_paths
```

`managed_orphans_to_delete` is derived only from old/new manifests.

## 24. Durable journal

Use:

```text
<install>\.sky-update-transaction\
├── journal.json
└── backup\
```

Journal states at minimum:

```text
prepared
committed
```

Before first install mutation:

1. create complete backup for existing files that will be overwritten/deleted;
2. record new paths that rollback must remove;
3. atomically write/flush `prepared` journal;
4. only then mutate install root.

## 25. Atomic metadata writes

For journal/result/critical JSON:

```text
write temp file in same directory
flush
atomic rename/replace
```

Do not rely on partially written JSON.

## 26. Install and post-install verify

For every target:

- ensure path remains under install root;
- copy from verified staging;
- never execute staged binaries during install;
- delete only managed orphans in plan.

Before commit:

- hash every installed managed file;
- compare to new manifest;
- verify primary executable/updater exist;
- recheck critical signatures if appropriate.

Any failure after `prepared` triggers rollback.

## 27. Rollback

On failure after `prepared`:

1. remove new managed paths recorded by journal;
2. restore backed-up files;
3. verify restored hashes if recorded;
4. preserve recovery material if any restore fails;
5. do not start a half-updated app.

If rollback succeeds:

- write `rolled_back` result;
- restart old app only if old install verifies.

If rollback cannot fully verify:

- fail closed;
- keep recovery material;
- do not auto-launch app.

## 28. Interrupted transaction recovery

At updater startup, before a new transaction:

- `prepared` → rollback/recover first;
- `committed` → verify/cleanup;
- malformed journal → fail closed and preserve files.

No new transaction until recovery is resolved.

## 29. Result and logging

Structured result:

```text
%LOCALAPPDATA%\Sky-Auto-Player\update-state\last-result.json
```

Example:

```json
{
  "schema_version": 1,
  "status": "success",
  "from_version": "3.2.0",
  "target_version": "3.3.0",
  "timestamp_utc": "2026-08-09T00:00:00Z",
  "error_code": null,
  "message": null
}
```

Stable error codes should include categories such as:

```text
INVALID_ARGUMENT
PARENT_TIMEOUT
NETWORK_FAILURE
REDIRECT_REJECTED
RELEASE_NOT_FOUND
RELEASE_POLICY_REJECTED
ASSET_MISSING
CHECKSUM_INVALID
CHECKSUM_MISMATCH
ARCHIVE_UNSAFE
MANIFEST_INVALID
MANIFEST_HASH_MISMATCH
SIGNATURE_INVALID
INSTALL_ROOT_INVALID
TRANSACTION_RECOVERY_REQUIRED
BACKUP_FAILED
INSTALL_COPY_FAILED
POST_INSTALL_VERIFY_FAILED
ROLLBACK_FAILED
RESTART_FAILED
```

Do not log secrets, environment dumps, signed redirect URLs, song contents, or arbitrary personal file listings.

## 30. Dry-run contract

`--dry-run` may:

- fetch release;
- download;
- verify checksum;
- validate archive;
- extract staging;
- validate manifest;
- validate signatures.

It must not:

- mutate/recover install root;
- create install backup;
- copy/delete install files;
- restart app.

If unresolved transaction exists, report it and exit nonzero.

## 31. Fault-injection requirements

Tests must inject failures at:

- download interrupted;
- malformed sidecar;
- checksum mismatch;
- ZIP traversal;
- case collision;
- symlink entry;
- missing manifest;
- extra staged file;
- manifest hash mismatch;
- invalid signature;
- backup failure;
- copy fails after first file;
- orphan delete failure;
- post-copy hash mismatch;
- rollback restore failure;
- process termination after `prepared`;
- leftover `committed` journal;
- parent never exits.

Every test asserts filesystem safety, not only returned error.
