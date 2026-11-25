---
key: how-it-works
locale: en
slug: how-it-works
title: How Sky Auto Player Works
description: >-
  Learn how Sky Auto Player prepares a sheet, schedules notes, and sends keyboard input
  through the Windows SendInput API.
summary: >-
  Sky Auto Player prepares user-provided sheets and schedules them in a native Windows
  desktop player. Sender timing and game-side sampling are separate.
category: getting-started
order: 1
published: '2026-08-08'
updated: '2026-11-26'
draft: false
showDiagram: true
related:
  - sheet-formats
  - timing-engine
  - security-boundaries
evidence:
  - category: architecture
    label: System architecture
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/architecture.md
  - category: security
    label: Security policy
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/SECURITY.md
  - category: architecture
    label: Playback engine contract
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-engine.md
---

## What it does

Sky Auto Player reads a sheet file you provide, prepares its notes and chords,
and schedules keyboard input for the Windows desktop. The game observes that
input on its own schedule, so sender timing cannot guarantee which frame
registers a note.

A typical workflow is:

1. Export a sheet from the [Sky Music editor](https://specy.github.io/skyMusic/) as JSON, **.skysheet**, or JSON-compatible TXT.
2. Import it through the desktop Library.
3. Select the song, switch to Sky, and start playback with the configured control.

The supported desktop application is the Tauri app for Windows 11. It provides
the Library, Song Detail, Player Dock, Diagnostics, Settings, and Update
surfaces.

## From sheet to playback

The desktop prepares and validates a sheet before playback begins. A native
Rust worker then dispatches the prepared schedule against QPC deadlines. A
fixed 15-slot executor tracks key holds and same-key release gaps. A healthy
chord is sent as one prepared packet; when independent releases and new notes
are due together, releases are ordered before new presses.

Before each musical packet, the native sender freshly checks the current
foreground window and retained target identity. A foreground event is only a
hint to recheck focus and cannot authorize a send. Bounded safety cleanup may
separately attempt releases for keys already recorded in its journal. The
Tauri shell coordinates pause and stop, joins the worker, and records
retirement before publishing the result to the UI.

For the exact timing, cleanup, and recovery rules, see the
[playback engine contract](https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-engine.md).

## Scope and limitations

Sky Auto Player sends musical packets through Windows SendInput after fresh
foreground and target identity checks. Bounded journal-limited safety cleanup
follows a separate path. It does not control the game's own input sampling
loop or guarantee an in-game result. The full security boundary, including
prohibited process and input techniques, is defined in the
[security policy](https://github.com/pumni/Sky-Auto-Player/blob/main/SECURITY.md).

Automated playback may conflict with Sky's Terms of Service. Sky Auto Player is
an unofficial community project and is not affiliated with or endorsed by
thatgamecompany.
