---
key: windows-setup
locale: en
slug: windows-setup
title: Windows Setup and First Launch
description: >-
  Step-by-step setup guide for Sky Auto Player on Windows 10 or 11: download the Tauri installer,
  install, add songs, and configure the hotkeys.
summary: >-
  Download the canonical Tauri NSIS installer, install for the current user, and use the desktop
  Library to import sheets.
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
- No system Python required — the packaged build is native and standalone
- No administrator rights required for the current-user install
- No system-wide installer or service

## Download and install

1. Go to the [dedicated v4 release authority](https://github.com/pumni/Sky-Auto-Player-Releases/releases).
2. Download the canonical Tauri NSIS installer and its `.exe.sig` sidecar.
3. Run the installer and keep the default current-user location.
4. Launch **Sky Auto Player** from the installed shortcut.

## First launch

On first launch the application creates its app-data state under the Windows user profile.
You do not need to edit it manually — all supported settings are available from the desktop
app's **Settings** dialog.

The canonical installer is Authenticode-qualified. If Windows reports an installer integrity or
publisher error, discard it and download the exact package again from the v4 release authority.

## Adding songs

1. Export a sheet from the [Sky Music editor](https://specy.github.io/skyMusic/) as JSON, `.skysheet`, or TXT.
2. Import the file through the desktop Library.
3. Use the Library refresh control if the file was added through an external file picker.

## Desktop shortcuts

| Shortcut | Action                            |
| -------- | --------------------------------- |
| `/`      | Focus song search                 |
| `Ctrl+F` | Focus song search                 |
| `Esc`    | Close the active safe overlay     |
| `q`      | No quit action in the desktop GUI |

## Updating

Sky Auto Player checks the configured v4 channel through the dedicated release authority and
shows a banner when an update is available. **Update and Restart** uses the official Tauri
updater through the Rust-owned `UpdateService`; it verifies the Tauri `.sig` and runs the
current-user NSIS installer. V4 has no bundled `Sky-Auto-Player-Updater.exe`, portable ZIP
updater, or custom `MANIFEST.json.sig` contract.

## Uninstalling

Use Windows **Installed apps** to uninstall the current-user package. Application data under
the user profile is a separate boundary and can be removed after preserving anything you want.
