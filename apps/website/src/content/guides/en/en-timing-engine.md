---
key: timing-engine
locale: en
slug: timing-engine
title: The Timing Engine
description: >-
  How Sky Auto Player schedules notes, applies sender-side hold and release floors, and
  why game-side timing depends on Windows and the game's sampling loop.
summary: >-
  A fixed 15-slot executor schedules prepared notes and sends healthy chords in one packet.
  Sender timing and game-side sampling remain separate.
category: playback-timing
order: 1
published: '2026-08-08'
updated: '2026-11-18'
draft: false
related:
  - how-it-works
  - security-boundaries
  - troubleshooting
evidence:
  - category: architecture
    label: Playback engine contract
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-engine.md
  - category: test
    label: Playback verification
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-verification.md
---

## What timing means

Sky Auto Player controls when it calls Windows SendInput. Windows scheduling,
the game's input polling loop, and hardware affect when the game observes a
key. Sender completion is not proof that the game registered a note.

## Note and chord scheduling

The desktop prepares the sheet before starting a native worker. A fixed
15-slot executor tracks musical presses and releases. A healthy chord uses
one prepared packet. When independent releases and presses are due together,
the packet orders every Up before every Down.

The production waiter uses calibrated host timing with QPC deadlines. Its
wake guard remains fixed during playback; it does not continuously adjust
lead time to hide drift.

## Hold and release floors

Let T be the ceiling of 1,000,000 divided by configured FPS, in microseconds.
The sender floors are:

- H = ceiling(hold_frames × T) + margin.
- G = T + margin.

An Up cannot occur before both its authored release time and successful Down
completion plus H. A same-key Down cannot occur before successful Up completion
plus G. These are sender-side limits; they do not choose the game frame that
samples a key.

## Limits

Dense passages may exceed the game's sampling rate even when sender deadlines
are met. Timing results describe the Windows sender path, not audio or
gameplay acceptance. The supported desktop application targets Windows 11.

For current implementation contracts and verification commands, see
[playback-engine.md](https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-engine.md)
and
[playback-verification.md](https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-verification.md).
