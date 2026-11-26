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

**Fix**: In the Player Dock's **Playback profile**, open **Advanced timing** and select the
FPS configured in Sky, if available. Supported values are 30, 60, 90, 120, 144, 165, and 240.
The player's sender-side timing does not reveal when Sky samples input.

**Cause B — Tempo mismatch**: The tempo multiplier is not 1.0.

**Fix**: In the Player Dock's **Playback profile**, set **Tempo** to 1.0× to use the sheet's
authored tempo without an additional tempo multiplier.

**Cause C — System load**: Heavy CPU load can increase Windows scheduling jitter and sender-side
dispatch lateness. The calibrated wake guard is fixed; it does not continuously adjust a lead to
compensate for changing load.

**Fix**: Close background applications and use **Diagnostics** to review sender-side dispatch
measurements. Playback profile controls can change authored tempo, hold and FPS inputs, but they
cannot change Windows scheduling or the game's input sampling. See the
[full FAQ](https://pumni.github.io/Sky-Auto-Player/faq/).

> **Note**: Sky Auto Player controls when keystrokes are _sent_. When the game _processes_
> them depends on the game's own sampling loop and Windows scheduling — these factors are
> outside the player's control.

## Hold notes not registering (releasing too early)

**Cause**: The hold duration at the configured FPS is shorter than the game's minimum
registration time for that key.

**Fix**: In the Player Dock's **Playback profile**, try **Base Hold** at 1.25 or 1.5 frames.
The frame-aware hold floor and timing margin affect sender-side release eligibility; they do not
guarantee when Sky registers a key.

## Playback stops or notes do not register

**Cause — Foreground target**: New musical packets require fresh checks of the foreground window
and retained target identity. A foreground event is only a hint to recheck; it cannot authorize
a send. When authority is lost, the engine prevents new musical packets.

**Fix**: Focus the selected Sky target before starting or resuming playback. If the app stops on
focus or target loss, restore the target and follow the Player Dock's playback state before
starting again. Bounded safety cleanup can separately attempt releases for keys already recorded
in the release journal. For notes that are sent but not registered, review the sender-side timing
limits described above and in the [playback guide](https://pumni.github.io/Sky-Auto-Player/guides/timing-engine/).

## Windows SmartScreen warning on launch

**Cause**: The canonical installer is intentionally unsigned for Authenticode under the current policy. Windows may show Unknown Publisher or a SmartScreen warning.

**Fix**: Confirm the download came from the [official Sky Auto Player GitHub Releases page](https://github.com/pumni/Sky-Auto-Player/releases/latest). The Tauri updater verifies the separate `.exe.sig` sidecar; it does not provide Authenticode publisher identity. If you cannot confirm the source, do not run the installer.

## Updating manually

**Cause**: The official Tauri updater could not stage or execute the current-user installer.

**Fix**:

1. Retry **Update and Restart** from the in-app update notice.
2. If it still fails, uninstall only after preserving app data, then download the current
   canonical NSIS installer from the [official Sky Auto Player GitHub Releases page](https://github.com/pumni/Sky-Auto-Player/releases/latest).
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
