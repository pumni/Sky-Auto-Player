---
key: how-it-works
locale: en
slug: how-it-works
title: How Sky Auto Player Works
description: >-
  Learn how Sky Auto Player reads sheet files, schedules notes with frame-aligned timing,
  and sends keystrokes through the Windows SendInput API — without touching the game.
summary: >-
  Sky Auto Player reads user-provided sheet files, schedules every note against a timing
  model, and delivers keystrokes through the Windows SendInput API. It never reads game
  memory, modifies game files, or injects code.
category: getting-started
order: 1
published: '2026-08-08'
updated: '2026-08-08'
lastReviewedVersion: '3.0.0'
draft: false
showDiagram: true
related:
  - sheet-formats
  - timing-engine
  - security-boundaries
evidence:
  - category: architecture
    label: Four-layer architecture overview
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/architecture.md
  - category: security
    label: Security mandates (SendInput only, no game tampering)
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/SECURITY.md
  - category: implementation
    label: README — workflow and technical summary
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/README.md
  - category: architecture
    label: Real-time dispatch architecture
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/rt-dispatch-architecture.md
---

## What the software does

Sky Auto Player reads a music sheet file that you provide, schedules every note, chord, and
hold against a timing model, and delivers keystrokes to the active window through the Windows
`SendInput` API — the same channel used by any standard keyboard macro. It does not
read game memory, inspect game files, modify the game, inject code, or attach a debugger.

The intended workflow is:

1. Export a sheet from the [Sky Music editor](https://specy.github.io/skyMusic/) as JSON, `.skysheet`, or JSON-compatible TXT.
2. Drop the file into the `songs/` folder next to `Sky-Auto-Player.exe`.
3. Launch Sky Auto Player, select the song, switch to the game, and press your play hotkey.

Sky Auto Player sends keystrokes; the game observes them on its own schedule. The timing
precision on the game side depends on Windows scheduling, the game's input sampling rate, and
your hardware — not on the player itself.

## How note scheduling works

Sky Auto Player does not replay a fixed sequence of delays. Instead, it schedules each note
as an event on a timeline that advances against the sheet's tempo and the FPS you configure
inside Sky. The core loop:

- **Contiguous chord batches** — all notes in a chord are submitted in one `SendInput` call to
  minimise sender-side skew between keys in the same chord.
- **Hold semantics** — long notes are held for their full notated duration. A hold note does
  not get clipped short because the next note is close.
- **Hold-frame alignment** — the hold duration is expressed as 1.0, 1.25, or 1.5 game frames
  at the FPS you select. The default is 1.0 frames.
- **Latency adaptation** — the scheduler measures sender-side completion error on the current
  machine and adjusts its lead time accordingly. This reduces systematic drift; it does not
  guarantee game-side timing accuracy, which also depends on Windows and the game's sampling loop.

## Architecture overview

The code follows a four-layer design:

| Layer              | Responsibility                                                                    |
| ------------------ | --------------------------------------------------------------------------------- |
| **Domain**         | Pure timing models, schedules, and profiles. No I/O.                              |
| **Orchestration**  | Scheduler and coordinator. No wall-clock or SendInput.                            |
| **Infrastructure** | Focus tracking, hotkey listener, real-time sleeper, MMCSS registration.           |
| **Platform**       | Windows backend: `SendInput`, waitable timer, MMCSS. Only place Win32 types live. |

A Rust timing worker owns compilation, dispatch, focus, `SendInput` calls, cleanup, and native
telemetry. Python owns the UI and application flow. The dispatch loop and Textual TUI run on
separate threads using the free-threaded Python 3.14 build — they do not contend on the GIL.

## What Sky Auto Player does not do

- Read or write game memory
- Modify, patch, or inspect game files
- Inject DLLs or attach a debugger to any process
- Install Windows hooks (`SetWindowsHookEx`, etc.)
- Bypass anti-cheat systems
- Guarantee in-game timing accuracy — sender-side timing is optimised, but the game decides
  when to register keystrokes

## Limitations

- Windows 10 and 11 (64-bit) only. MMCSS real-time scheduling is a Windows API.
- Automated playback may conflict with Sky's Terms of Service. Use responsibly and at your own risk.
- Sky Auto Player is an unofficial community project, not affiliated with or endorsed by thatgamecompany.
