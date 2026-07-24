# Distribution and Update Model

This document explains the distribution model, update architecture, and lifecycle rules for Sky Auto Player. It tracks the `pyproject.toml` `[project].version` (currently 2.4.4) and is the reference for contributors handling packaging or update logic. Update this header whenever the release version bumps — the rules below apply to every shipped release, not a specific point release.

## 1. Model Overview

Sky Auto Player is a fully portable application distributed as a zip file. It does not use a system installer and does not rely on the registry for its core functionality. All application files, including user profiles (`config.json`) and downloaded songs (`songs/`), live together in a single directory.

To ensure stability and eliminate in-use file replacement complexities, **in-app auto-update is intentionally excluded**. Instead, Sky Auto Player uses a two-piece update model:
- **In-app notification**: The application checks for updates on GitHub in the background and presents a banner when a new version is available.
- **External updater**: The actual update is applied by closing the app and running an external `updater.bat` script, avoiding the risks of patching running binaries.

## 2. Release Artefact Contract

Our CI pipeline (defined in `.github/workflows/release.yml`) builds exactly five assets on every tag push matching `v*` during the legacy bridge window:
1. `Sky-Auto-Player-v<version>.zip` — The canonical portable application.
2. `Sky-Auto-Player-v<version>.zip.sha256` — The cryptographic sidecar.
3. `MANIFEST.json` — Canonical release metadata.
4. `Sky-Player-v<version>.zip` — The legacy bridge portable application.
5. `Sky-Player-v<version>.zip.sha256` — The cryptographic sidecar for the legacy bridge zip.

**Note on Sunset (D3):** The legacy bridge assets (`Sky-Player-*`) are published to support users migrating from pre-2.4.2 versions. This dual-publish bridge is a temporary transition path and will be sunset no earlier than the first 2.5.0 release (with at least 30 days notice in the CHANGELOG).

**Crucial Invariant:** The Git tag version must perfectly match the version specified in `pyproject.toml`. The release workflow enforces this and will fail the build if they diverge.

## 3. Updater Behaviour

The external updater (`updater.bat` delegating to `installer/updater.ps1`) enforces a strict lifecycle to protect user data and ensure successful upgrades:

- **Pre-mutation SHA256 Verification:** The updater requires the sidecar to contain exactly one SHA256 bound to the selected zip filename, then compares the downloaded zip before touching any install file.
- **TEMP Staging:** Updates are extracted only to `%TEMP%\sky-update-*`, never directly into the install directory. Before extraction, every zip entry is checked for rooted/traversal paths, case-colliding duplicates, alternate data streams, and symbolic links.
- **Write Permission + Process Gates:** The updater validates write access and refuses to recover or mutate files while either `Sky-Auto-Player.exe` or legacy `Sky-Player.exe` is running from the target directory. `-ForceClose` must stop every matching target process before recovery continues.
- **Exact MANIFEST.json Verification:** The embedded manifest is mandatory. Its app ID, selected release version, primary executable name/hash, every payload hash, and the exact staged file set must match. Unsafe paths, duplicate/case-colliding paths, missing files, executable mismatches, and unmanifested extra files all fail closed before install mutation.
- **Durable Transaction Journal:** Before deleting or overwriting anything, the updater creates `<install>\.sky-update-transaction\journal.json` and a complete backup of every affected existing file. The journal is atomically committed before mutation. A later updater run automatically rolls back any `prepared` transaction left by process termination, power loss, or restart.
- **Rollback Retention:** A failed restore never deletes the remaining backup. The updater reports the durable recovery directory and refuses a new transaction until recovery succeeds. A `committed` journal means all copied files passed a post-copy SHA256 check; it is safe to clean without rollback.
- **Preserve-list (Data Safety):** `config.json`, `.env`, `songs/`, and `logs/` are never replaced or orphan-cleaned. After a successful binary transaction, only `update.last_check_ts` and `update.last_notified_version` are patched into `config.json` through a same-directory atomic replace.
- **Dry-run Contract:** `-DryRun` performs release selection, asset download, outer SHA256, process gate, safe extraction, and exact manifest verification. It performs no install recovery or mutation; an unresolved transaction must first be recovered by a normal updater run.
- **Dual-Name Resolution:** The updater prefers `Sky-Auto-Player.exe` and falls back to `Sky-Player.exe` for legacy installations; release asset and manifest executable names must agree.

### 3.1. `installer/updater.ps1` encoding invariant

The script **MUST start with a UTF-8 BOM** (`EF BB BF`).  `updater.bat` falls back to `powershell.exe` (Windows PowerShell 5.1, the inbox shell on every Windows machine) when `pwsh` is not installed.  PS 5.1 reads BOM-less `.ps1` files with the system ANSI codepage (Windows-1252 on en-US hosts), so any non-ASCII byte — em-dash `—` (`E2 80 94`), `§` (`C2 A7`), smart quotes — gets mis-decoded as `â€"` / `Â§` and breaks the parser, fail-closing the entire external update path.

### 3.2. Pre-2.4.2 Migration (The Bridge)

Users on older installations named "Sky-Player" (v2.4.1 or earlier) can seamlessly migrate to the new "Sky Auto Player" identity. To migrate, users simply run their existing `updater.bat` once after v2.4.2 is published. The old updater will download the legacy bridge zip, which contains the new `Sky-Auto-Player.exe` and the new updater scripts. Subsequent updates will then follow the canonical `Sky-Auto-Player-v*.zip` path automatically.

## 4. Channel Switching

Users can subscribe to different update channels:
- By default, users are on the `stable` channel.
- Users can switch to `beta` through the "Update Settings" in the app or manually editing `update.channel` in `config.json`.
- The external updater also accepts a command-line override: `updater.bat -Channel beta`.
- Both the in-app checker and the external updater use the same channel definition to find the appropriate GitHub Release. The authoritative policy for each channel is defined in `src/sky_music/domain/update_policy.py`:

| Channel | Pre-releases | GitHub API endpoint |
|---------|--------------|---------------------|
| stable  | Excluded     | `/releases/latest`  |
| beta    | Included     | `/releases?per_page=10` |

The stable channel never surfaces rc/beta/alpha/dev tags; the beta channel includes them and picks the highest non-draft version.

## 5. Recovery

Because the update process is heavily guarded, recovery is straightforward:
- **Corrupt Zip:** A downloaded zip with a mismatched SHA256 will never be extracted to the installation directory.
- **Failed Copy:** The rollback routine restores the previous binaries from the durable journal. If any restore is blocked, the backup is retained under `.sky-update-transaction` and the next normal updater run retries recovery before checking versions.
- **Interrupted Process / Power Loss:** A `prepared` journal is rolled back on the next normal run; a `committed` journal is cleaned without undoing the verified update.
- **Manual Retry:** If an update fails, resolve the reported lock/permission problem and run `updater.bat` again. Do not manually delete `.sky-update-transaction`; it may contain the only recoverable copy of an old file. Manual zip extraction remains a last resort and must skip `config.json`, `.env`, `songs/`, and `logs/`.

## 6. Phase Contracts

This model is the implementation of the `mpv-pattern` update design. For historical phase definitions and the exact implementation contracts, refer to the [distribution-mpv-pattern-plan.md](2026-07-18_distribution-mpv-pattern-plan.md).

## 7. Explicit Non-Goals in 2.4.0

The following features were intentionally excluded from the 2.4.0 release and deferred:
- **System Installer:** An optional system installer (`sky-auto-player-install.bat`) for Start Menu shortcuts and `.skysheet` file associations is deferred to a future minor release (Phase 4).
- **Code Signing:** Authenticode EV signing for SmartScreen bypass is managed on a separate track.
