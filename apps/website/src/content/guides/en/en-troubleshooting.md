---
key: troubleshooting
locale: en
slug: troubleshooting
title: Troubleshooting
description: >-
  Common issues with Sky Auto Player: song not appearing, notes out of sync, holds
  not registering, SmartScreen warnings, and update failures — with solutions.
summary: >-
  Symptom-based troubleshooting for Sky Auto Player: Library issues, playback timing
  problems, hold registration, SmartScreen, and update failures. Covers known limitations
  that cannot be fixed from the player side.
category: support
order: 1
published: '2026-08-08'
updated: '2026-11-26'
draft: false
related:
  - windows-setup
  - sheet-formats
  - timing-engine
evidence:
  - category: implementation
    label: README — FAQ section
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/README.md
  - category: architecture
    label: Playback engine architecture
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-engine.md
  - category: distribution
    label: Distribution and update documentation
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/distribution-and-update.md
---

## Song not appearing in the Library

**Cause**: The file extension is not recognised, or the sheet has not been imported into a playlist.

**Fix**:

1. Confirm the file extension is `.json`, `.skysheet`, or `.txt`.
2. Create or select a playlist and use **Add songs** to import the file or its folder.
3. Confirm the source still exists at the original path; imports reference files in place.

## "Failed to parse" or parse error on launch

**Cause**: The file content is not valid JSON, or the sheet uses a schema version the
current parser does not support.

**Fix**:

1. Open the file in a text editor. Confirm it contains a JSON object starting with `{`.
2. Re-export the sheet from the [Sky Music editor](https://specy.github.io/skyMusic/).
3. If the error persists, open a [GitHub issue](https://github.com/pumni/Sky-Auto-Player/issues)
   and attach the error message (not the sheet file itself).

## Notes sound late or ahead of the music

**Cause A — FPS mismatch**: The FPS setting in Sky Auto Player does not match the FPS
configured inside Sky.

**Fix**: Open the per-song settings and set the FPS to match Sky's frame rate (30 or 60).

**Cause B — Tempo mismatch**: The tempo multiplier is not 1.0.

**Fix**: Reset the tempo multiplier to 1.0 in per-song settings.

**Cause C — System load**: Heavy CPU load causes Windows scheduling jitter, which
increases sender-side latency beyond what the adaptive lead can compensate.

**Fix**: Close background applications. Use **Diagnostics** to review sender-side dispatch measurements. Adjust only options available in the selected song's settings; they cannot change Windows scheduling or the game's input sampling. See the [full FAQ](https://pumni.github.io/Sky-Auto-Player/faq/).

> **Note**: Sky Auto Player controls when keystrokes are _sent_. When the game _processes_
> them depends on the game's own sampling loop and Windows scheduling — these factors are
> outside the player's control.

## Hold notes not registering (releasing too early)

**Cause**: The hold duration at the configured FPS is shorter than the game's minimum
registration time for that key.

**Fix**: In per-song settings, switch from **1.0 frames** to **1.25 frames** or
**1.5 frames**. The FPS setting must match the FPS inside Sky for hold timing to be correct.

## Game does not receive any input

**Cause A — Focus**: The game window does not have focus when playback starts.

**Fix**: Switch to the game window before pressing the play hotkey. Sky Auto Player sends input only when checks confirm the selected Sky target is in the foreground. Restore focus to Sky and retry; input is not routed to an arbitrary active window.

**Cause B — Hotkey not set**: The play hotkey is not bound or conflicts with another application.

**Fix**: Open the command palette (`/`) and check hotkey bindings.

**Cause C — Wrong key layout**: The key layout profile does not match the keyboard layout
in use inside Sky.

**Fix**: Verify the layout profile in settings matches what Sky shows.

## Windows SmartScreen warning on launch

**Cause**: The canonical installer is intentionally unsigned for Authenticode under the current policy. Windows may show Unknown Publisher or a SmartScreen warning.

**Fix**: Confirm the download came from the [official Sky Auto Player GitHub Releases page](https://github.com/pumni/Sky-Auto-Player/releases). The Tauri updater verifies the separate `.exe.sig` sidecar; it does not provide Authenticode publisher identity. If you cannot confirm the source, do not run the installer.

## Updating manually

**Cause**: The official Tauri updater could not stage or execute the current-user installer.

**Fix**:

1. Retry **Update and Restart** from the in-app update notice.
2. If it still fails, uninstall only after preserving app data, then download the current
   canonical NSIS installer from the [official Sky Auto Player GitHub Releases page](https://github.com/pumni/Sky-Auto-Player/releases).
3. Do not use a v3 ZIP, `MANIFEST.json`, or standalone updater executable for v4.

## The HUD shows high timing jitter

**Cause**: The HUD's jitter metric reflects _sender-side_ dispatch variance. High values
indicate OS scheduling pressure on your machine.

**Fix**: Close resource-heavy applications and review the sender-side measurements again. These measurements do not show when Sky registers input or produces sound.

## Known limitations

These issues cannot be fixed from the player side and are documented as known constraints:

- **Game-side timing**: The game decides when to register keystrokes. Sky Auto Player
  cannot override the game's input polling rate.
- **Very fast passages**: Extremely dense arpeggios at high BPM may exceed the game's
  ability to register individual notes, regardless of how precisely they are sent.
- **Windows 11 x64**: The current packaged app requires Windows 11 64-bit. Windows 10,
  macOS, and Linux packages are not supported.
- **Unofficial project**: Sky Auto Player is not affiliated with thatgamecompany.
  Automated playback may conflict with Sky's Terms of Service.
