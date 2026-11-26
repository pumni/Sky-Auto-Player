---
key: windows-setup
locale: en
slug: windows-setup
title: Windows Setup and First Launch
description: >-
  Step-by-step setup guide for Sky Auto Player on Windows 11: install the Tauri package,
  import sheets, and find the playback profile and desktop controls.
summary: >-
  Download the canonical Tauri NSIS installer, install for the current user, and use the desktop
  Library to import sheets.
category: getting-started
order: 3
published: '2026-08-08'
updated: '2026-11-26'
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

- Windows 11, 64-bit
- No developer toolchain or separate scripting runtime is required — the packaged build is standalone
- No administrator rights required for the current-user install
- No system-wide installer or service

## Download and install

1. Go to the [official Sky Auto Player GitHub Releases page](https://github.com/pumni/Sky-Auto-Player/releases/latest).
2. Download the canonical Tauri NSIS installer. The `.exe.sig` sidecar is used by the in-app updater and is not needed for manual installation.
3. Run the installer and keep the default current-user location.
4. Launch **Sky Auto Player** from the installed shortcut.

## First launch

On first launch the application creates its app-data state under the Windows user profile.
You do not need to edit it manually. **Settings** manages app-wide preferences, including
playback defaults. The Player Dock's **Playback profile** gives quick access to **Base Hold**,
**Tempo**, and **Advanced timing** (FPS and timing margin).

The canonical installer is intentionally unsigned for Authenticode under the current policy. Windows may show Unknown Publisher or a SmartScreen warning. Download only from the official GitHub Releases page. The Tauri updater verifies the separate .exe.sig sidecar; that sidecar is not required for a manual install.

## Adding songs

1. Export a sheet from the [Sky Music editor](https://specy.github.io/skyMusic/) as JSON, `.skysheet`, or TXT.
2. Create or select a playlist and use **Add songs** to import the local file or its folder.
3. Imports reference the original files; the app does not move or delete them. Keep each source file at its original location.

To start playback, equip a compatible 15-key instrument in Sky. Select your song in Sky Auto Player and use **Play**. Keep the Sky window in focus during playback.

## Desktop shortcuts

| Shortcut | Action                   |
| -------- | ------------------------ |
| `/`      | Focus song search        |
| `Ctrl+F` | Focus song search        |
| `Ctrl+,` | Open Settings            |
| `Ctrl+I` | Toggle the Inspector     |
| `Esc`    | Close the active overlay |

## Updating

Sky Auto Player checks the configured v4 channel through validated metadata on the main repository's
`release-metadata` branch and
shows a banner when an update is available. **Update and Restart** uses the official Tauri
updater through the Rust-owned `UpdateService`; it verifies the Tauri `.sig` and runs the
current-user NSIS installer. V4 has no bundled `Sky-Auto-Player-Updater.exe`, portable ZIP
updater, or custom `MANIFEST.json.sig` contract.

## Uninstalling

Use Windows **Installed apps** to uninstall the current-user package. Application data under
the user profile is a separate boundary and can be removed after preserving anything you want.
