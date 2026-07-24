# Distribution and Update Model

This document explains the distribution model, update architecture, and lifecycle rules for Sky Auto Player. It tracks the `pyproject.toml` `[project].version` (currently 2.4.2) and is the reference for contributors handling packaging or update logic. Update this header whenever the release version bumps — the rules below apply to every shipped release, not a specific point release.

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

- **Pre-mutation SHA256 Verification:** The updater downloads the zip and compares its hash against the sidecar *before* touching any files in the installation directory.
- **TEMP Staging:** Updates are extracted to a temporary staging folder (`%TEMP%\SkyPlayerUpdate-*`).
- **Write Permission Checks:** The updater validates it has write access to the target installation directory before attempting any copies.
- **Mandatory MANIFEST.json Verification:** After staging, the updater reads `MANIFEST.json` from the zip and verifies every file's SHA256 against the manifest's per-file hashes. **A zip without `MANIFEST.json` (or with an empty `files` array, or with any mismatched hash) is refused before any install mutation** — this is a fail-closed defense-in-depth invariant. Every official release since the `release.yml --manifest` gate carries `MANIFEST.json`; an absent manifest implies either a regressed build pipeline or a stripped zip, neither of which the updater will install.
- **Transactional Copy & Rollback:** Binaries are copied over in a transactional sequence. If any part of the operation fails, a rollback routine automatically reverts to the backup state.
- **Preserve-list (Data Safety):** The updater explicitly skips modifying `config.json` (except for allowed patch fields) and completely ignores the `songs/` folder. User profiles and song libraries are never touched.
- **Process Guard & Dual-Name Resolution:** The updater refuses to run if it detects either `Sky-Auto-Player.exe` or the legacy `Sky-Player.exe` is currently running, avoiding file locking issues. It also dynamically resolves the primary executable, preferring `Sky-Auto-Player.exe` but gracefully falling back to `Sky-Player.exe` for older installations.

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
- **Failed Copy:** If a file cannot be overwritten, the fallback rollback routine restores the previous binaries.
- **Manual Retry:** If an update fails, the user simply runs `updater.bat` again, or safely downloads the latest zip manually from GitHub and extracts it over their folder (skipping `config.json` and `songs/`).

## 6. Phase Contracts

This model is the implementation of the `mpv-pattern` update design. For historical phase definitions and the exact implementation contracts, refer to the [distribution-mpv-pattern-plan.md](2026-07-18_distribution-mpv-pattern-plan.md).

## 7. Explicit Non-Goals in 2.4.0

The following features were intentionally excluded from the 2.4.0 release and deferred:
- **System Installer:** An optional system installer (`sky-auto-player-install.bat`) for Start Menu shortcuts and `.skysheet` file associations is deferred to a future minor release (Phase 4).
- **Code Signing:** Authenticode EV signing for SmartScreen bypass is managed on a separate track.
