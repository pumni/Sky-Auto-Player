# Distribution and Update Model

This document explains the distribution model, update architecture, and lifecycle rules for Sky Player as of version 2.4.0. It is a reference for contributors handling packaging or update logic.

## 1. Model Overview

Sky Player is a fully portable application distributed as a zip file. It does not use a system installer and does not rely on the registry for its core functionality. All application files, including user profiles (`config.json`) and downloaded songs (`songs/`), live together in a single directory.

To ensure stability and eliminate in-use file replacement complexities, **in-app auto-update is intentionally excluded**. Instead, Sky Player uses a two-piece update model:
- **In-app notification**: The application checks for updates on GitHub in the background and presents a banner when a new version is available.
- **External updater**: The actual update is applied by closing the app and running an external `updater.bat` script, avoiding the risks of patching running binaries.

## 2. Release Artefact Contract

Our CI pipeline (defined in `.github/workflows/release.yml`) builds exactly three assets on every tag push matching `v*`:
1. `Sky-Player-v<version>.zip` — The portable application.
2. `Sky-Player-v<version>.zip.sha256` — The cryptographic sidecar.
3. `MANIFEST.json` — Release metadata.

**Crucial Invariant:** The Git tag version must perfectly match the version specified in `pyproject.toml`. The release workflow enforces this and will fail the build if they diverge.

## 3. Updater Behaviour

The external updater (`updater.bat` delegating to `installer/updater.ps1`) enforces a strict lifecycle to protect user data and ensure successful upgrades:

- **Pre-mutation SHA256 Verification:** The updater downloads the zip and compares its hash against the sidecar *before* touching any files in the installation directory.
- **TEMP Staging:** Updates are extracted to a temporary staging folder (`%TEMP%\SkyPlayerUpdate-*`).
- **Write Permission Checks:** The updater validates it has write access to the target installation directory before attempting any copies.
- **Transactional Copy & Rollback:** Binaries are copied over in a transactional sequence. If any part of the operation fails, a rollback routine automatically reverts to the backup state.
- **Preserve-list (Data Safety):** The updater explicitly skips modifying `config.json` (except for allowed patch fields) and completely ignores the `songs/` folder. User profiles and song libraries are never touched.
- **Process Guard:** The updater refuses to run if it detects `Sky-Player.exe` is currently running, avoiding file locking issues.

## 4. Channel Switching

Users can subscribe to different update channels:
- By default, users are on the `stable` channel.
- Users can switch to `beta` through the "Update Settings" in the app or manually editing `update.channel` in `config.json`.
- The external updater also accepts a command-line override: `updater.bat -Channel beta`.
- Both the in-app checker (`include_prerelease`) and the external updater use the same channel definition to find the appropriate GitHub Release.

## 5. Recovery

Because the update process is heavily guarded, recovery is straightforward:
- **Corrupt Zip:** A downloaded zip with a mismatched SHA256 will never be extracted to the installation directory.
- **Failed Copy:** If a file cannot be overwritten, the fallback rollback routine restores the previous binaries.
- **Manual Retry:** If an update fails, the user simply runs `updater.bat` again, or safely downloads the latest zip manually from GitHub and extracts it over their folder (skipping `config.json` and `songs/`).

## 6. Phase Contracts

This model is the implementation of the `mpv-pattern` update design. For historical phase definitions and the exact implementation contracts, refer to the [distribution-mpv-pattern-plan.md](2026-07-18_distribution-mpv-pattern-plan.md).

## 7. Explicit Non-Goals in 2.4.0

The following features were intentionally excluded from the 2.4.0 release and deferred:
- **System Installer:** An optional system installer (`sky-player-install.bat`) for Start Menu shortcuts and `.skysheet` file associations is deferred to a future minor release (Phase 4).
- **Code Signing:** Authenticode EV signing for SmartScreen bypass is managed on a separate track.
