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
updated: '2026-08-08'
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
    label: Timing principles
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/timing-principles.md
  - category: distribution
    label: Distribution and update documentation
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/distribution-and-update.md
---

## Song not appearing in the Library

**Cause**: The file extension is not recognised, or the Library has not been reloaded.

**Fix**:

1. Confirm the file extension is `.json`, `.skysheet`, or `.txt`.
2. Press **Reload songs** in the desktop GUI without restarting. In the Textual fallback, press
   `Ctrl+R`.
3. If still missing, check that the file is directly inside the `songs/` folder, not in a subfolder.

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

**Fix**: Close background applications. Consider enabling a tuning preset for weak PCs
via the `--tuning` flag. See the [full FAQ](https://pumni.github.io/Sky-Auto-Player/faq/).

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

**Fix**: Switch to the game window before pressing the play hotkey. Sky Auto Player sends
keystrokes to the active window; if the game is not focused, keystrokes go elsewhere.

**Cause B — Hotkey not set**: The play hotkey is not bound or conflicts with another application.

**Fix**: Open the command palette (`/`) and check hotkey bindings.

**Cause C — Wrong key layout**: The key layout profile does not match the keyboard layout
in use inside Sky.

**Fix**: Verify the layout profile in settings matches what Sky shows.

## Windows SmartScreen warning on launch

**Cause**: Sky Auto Player is not code-signed. Windows SmartScreen shows this warning for
any unsigned executable downloaded from the internet.

**Fix**: Click **More info → Run anyway**. To verify the binary is authentic, check the
SHA256 checksum of the downloaded ZIP against the `.sha256` file published on the
[releases page](https://github.com/pumni/Sky-Auto-Player/releases/latest).

## Updating manually

**Cause**: Public Windows releases use an unsigned portable model. The bundled updater must be
launched by the user and the installation folder must be writable; there is no system installer.

**Fix**:

1. Retry **Update and Restart** from the in-app update notice.
2. If staging fails, open the [official releases page](https://github.com/pumni/Sky-Auto-Player/releases).
3. Download the canonical ZIP, `.zip.sha256`, and `MANIFEST.json`.
4. Verify the ZIP SHA256 when desired, extract into a new folder, and copy your
   `config.json`, `.env`, `songs/`, and `logs/` folders.

## The HUD shows high timing jitter

**Cause**: The HUD's jitter metric reflects _sender-side_ dispatch variance. High values
indicate OS scheduling pressure on your machine.

**Fix**: Close background applications. Try a tuning preset. Note that sender-side jitter
does not directly translate to audible timing errors — the game's sampling rate is often
the larger factor.

## Known limitations

These issues cannot be fixed from the player side and are documented as known constraints:

- **Game-side timing**: The game decides when to register keystrokes. Sky Auto Player
  cannot override the game's input polling rate.
- **Very fast passages**: Extremely dense arpeggios at high BPM may exceed the game's
  ability to register individual notes, regardless of how precisely they are sent.
- **Windows only**: MMCSS real-time scheduling and `SendInput` are Windows APIs. macOS
  and Linux are not supported and are not on the roadmap.
- **Unofficial project**: Sky Auto Player is not affiliated with thatgamecompany.
  Automated playback may conflict with Sky's Terms of Service.
