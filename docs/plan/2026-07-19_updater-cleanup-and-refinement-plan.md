# Updater Cleanup and Refinement Plan

**Date**: 2026-07-19
**Status**: Proposal

## 1. Context and Problem Statement

The current external update script (`installer/updater.ps1`) implements a robust `mpv-pattern` transactional copy, moving files from a staging directory (`%TEMP%`) to the installation root. However, the `Copy-UpdateTree` function currently only iterates over the source files and overwrites/copies them into the destination.

**The Orphaned Files Bug**: 
When a new release of Sky Player removes a file (for example, dropping an unused Python library from PyInstaller's `_internal/` directory or removing an obsolete script), the updater does **not** delete this file from the user's installation directory. Over time, these orphaned files accumulate. While often harmless, leftover binaries (especially `.pyc` or `.pyd` files in `_internal/`) can cause unpredictable runtime bugs if they are inadvertently loaded.

## 2. Proposed Solution

We will leverage `MANIFEST.json` as the ultimate source of truth for what should exist in the installation directory. 

### Phase 1: Implement `Remove-OrphanedFiles` (Manifest-driven Cleanup)
Before or during `Copy-UpdateTree`, the updater will:
1. Read the parsed `MANIFEST.json` to get the canonical list of files for the new version.
2. Scan all files in the target installation directory (`$DestRoot`).
3. If a file exists in `$DestRoot` but **does not exist** in the `MANIFEST.json` paths:
   - Check if it belongs to the preserve-list (e.g., `config.json`, `songs\`, `logs\`, `*.bak`).
   - If it is not in the preserve-list, **back it up** (using the existing rollback backup mechanism).
   - Delete the orphaned file from `$DestRoot`.
4. If any error occurs during the subsequent copy phase, the rollback mechanism will restore these deleted files alongside the overwritten ones.

### Phase 2: Zip-Slip Defense Documentation
While `update_installer.py` explicitly guards against Zip-Slip path traversal (e.g., `..\..\Windows\System32`), `updater.ps1` relies on `[System.IO.Compression.ZipFile]::ExtractToDirectory` or `Expand-Archive`, which can have varying levels of native protection depending on the Windows version.

However, the current architecture inherently defeats Zip-Slip due to the **fail-closed `MANIFEST.json` integrity gate**. If an attacker tricks the archive into extracting a file outside of `$StagingRoot`:
1. `Test-ManifestIntegrity` iterates over every file declared in the manifest.
2. It constructs the path using `Join-Path $StagingRoot $file.path`.
3. If the file escaped `$StagingRoot`, `Test-Path` returns `$false`, and the script immediately aborts before any mutation occurs.

*Action*: We will simply add a comment in `updater.ps1` explicitly documenting that `Test-ManifestIntegrity` serves as the primary Zip-Slip mitigation, preventing any confusion during future security audits.

## 3. Execution Contract

- **Target**: `installer/updater.ps1`
- **Safety**: The preserve-list MUST continue to strictly protect `config.json` and the `songs/` directory.
- **Rollback**: Any file deleted during the orphaned cleanup MUST be backed up to the `$backupDir` and successfully restored if `Copy-UpdateTree` fails.

## 4. Next Steps
Once approved, apply the modifications to `installer/updater.ps1` and verify with a simulated update containing a dummy obsolete file.
