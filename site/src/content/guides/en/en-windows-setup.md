---
key: windows-setup
locale: en
slug: windows-setup
title: Windows Setup and First Launch
description: >-
  Step-by-step setup guide for Sky Auto Player on Windows 10 or 11: download, extract,
  verify the checksum, add songs, and configure the hotkeys.
summary: >-
  Download the ZIP from GitHub releases, extract it anywhere — no installer, no admin rights.
  Drop song files into songs/, launch Sky-Auto-Player.exe, and use hotkeys to control playback.
category: getting-started
order: 3
published: '2026-08-08'
updated: '2026-08-08'
draft: false
related:
  - sheet-formats
  - troubleshooting
  - security-boundaries
evidence:
  - category: implementation
    label: README — quick start and requirements
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/README.md
  - category: distribution
    label: Distribution and update documentation
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/distribution-and-update.md
---

## Requirements

- Windows 10 or 11, 64-bit
- No system Python required — the packaged build ships its own runtime
- No administrator rights required for normal use
- No registry entries, no system-wide installer

## Download and extract

1. Go to the [latest release](https://github.com/pumni/Sky-Auto-Player/releases/latest) on GitHub.
2. Download `Sky-Auto-Player-v<version>.zip`.
3. _(Optional)_ Download `Sky-Auto-Player-v<version>.zip.sha256` and verify the checksum
   before extracting:

   ```powershell
   # In PowerShell, in the folder where you downloaded the zip:
   (Get-FileHash "Sky-Auto-Player-v<version>.zip" -Algorithm SHA256).Hash
   # Compare the output with the contents of the .sha256 file
   ```

4. Extract the ZIP to any folder, for example `C:\Sky-Auto-Player\`.
5. Run `Sky-Auto-Player.exe`.

## First launch

On first launch the application creates a `config.json` file in the installation folder.
You do not need to edit this file manually — all settings are accessible from within the TUI.

If Windows shows a SmartScreen prompt ("Windows protected your PC"), click
**More info → Run anyway**. The binary is not code-signed; SmartScreen shows this warning
for any unsigned executable downloaded from the internet.

## Adding songs

1. Export a sheet from the [Sky Music editor](https://specy.github.io/skyMusic/) as JSON, `.skysheet`, or TXT.
2. Copy or move the file into the `songs/` folder next to `Sky-Auto-Player.exe`.
3. In the picker, press `Ctrl+R` to reload the list.

## Hotkeys

| Hotkey      | Action                  |
| ----------- | ----------------------- |
| `/`         | Open command palette    |
| `F8`        | Pause / resume playback |
| `F9`        | Skip to next section    |
| `F10`       | Stop playback           |
| `q` / `Esc` | Quit                    |
| `Ctrl+R`    | Reload song list        |

## Updating

Sky Auto Player checks GitHub for new releases when it starts and shows a banner when one
is available. To apply an update:

1. Close Sky Auto Player.
2. Run `updater.bat` in the installation folder.
3. Reopen `Sky-Auto-Player.exe`.

The updater verifies the SHA256 checksum of the downloaded archive before touching any file.
If verification fails, it rolls back and leaves your installation unchanged. It never
replaces `config.json` or your `songs/` folder.

## Uninstalling

Delete the installation folder. Sky Auto Player does not write to the registry or any
location outside its own folder (except the user-level `%LOCALAPPDATA%\uv` cache if
you built from source with `uv`).
