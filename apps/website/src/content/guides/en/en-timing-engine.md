---
key: timing-engine
locale: en
slug: timing-engine
title: The Timing Engine
description: >-
  How Sky Auto Player schedules notes, aligns chords to dispatch frames, handles holds,
  adapts to machine latency, and what "timing accuracy" actually means in this context.
summary: >-
  Sky Auto Player's timing engine schedules notes against a frame-aligned timeline, submits
  chords in one SendInput batch, holds notes for their full duration, and measures
  sender-side latency to reduce systematic drift. Game-side accuracy depends on Windows
  and the game's own sampling loop.
category: playback-timing
order: 1
published: '2026-08-08'
updated: '2026-08-08'
draft: false
related:
  - how-it-works
  - security-boundaries
  - troubleshooting
evidence:
  - category: architecture
    label: Playback engine architecture
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-engine.md
  - category: architecture
    label: Playback verification and qualification protocol
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/playback-verification.md
  - category: implementation
    label: README — timing summary
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/README.md
---

## What "timing" means here

Sky Auto Player controls the _sender side_ — when it calls `SendInput`. The _game side_ —
when the game registers and processes a keystroke — depends on the Windows message queue,
the game's input polling rate, and your hardware. These two things are different.

The timing engine is designed to minimise sender-side error. It does not — and cannot —
guarantee that the game receives keystrokes at exactly the intended musical time.

## Note scheduling

Instead of replaying a fixed list of delays, Sky Auto Player compiles notes into an
ordered stream driven by the sheet's tempo, your target FPS, and hold parameters.
A 15-slot executor manages key-hold obligations ($H$) and release gaps ($G$)
deterministically without dual cursors.

## Chord alignment

Notes that belong to the same chord are submitted in a single `SendInput` call containing
all the keys in that chord. When disjoint key releases and down chords coincide, they are
batched together (with Up events preceding Down events in the same call) to eliminate
sender-side skew.

## Hold semantics and timing floors

A hold note (sustained key) is held for at least the minimum hold floor $H$:
$$H = \lceil \text{hold\_frames} \times T \rceil + \text{margin}$$
where $T = \lceil 1{,}000{,}000 / \text{FPS} \rceil$. Retriggering the same key respects
the release gap floor $G = T$ before sending the next Down.

## Hold-frame selection

The hold duration is expressed as a multiple of one game frame at the configured FPS:

| Selection            | Duration at 60 FPS |
| -------------------- | ------------------ |
| 1.0 frames (default) | ~16.7 ms           |
| 1.25 frames          | ~20.8 ms           |
| 1.5 frames           | ~25.0 ms           |

Choose 1.25 or 1.5 if short holds are not registering reliably on your machine.
The FPS setting must match the FPS configured inside Sky for hold timing to be correct.

## Calibration and robust wake guard

Sky Auto Player measures Windows kernel timer wake errors during startup host calibration,
deriving a robust wake guard ($\text{median} + 6 \times \text{MAD}$). During playback, the
production waiter uses this frozen wake guard to combine efficient kernel-timer sleep
with a microsecond-accurate QPC spin loop, eliminating drift without uncalibrated runtime jitter.

This is **sender-side timing only**. It reduces systematic delay caused by OS scheduling
jitter on your machine. It does not correct for network latency or game frame pacing.
The telemetry displayed in the Diagnostics panel reports the measured sender-side result,
not an audible or in-game guarantee.

## Rust dispatch worker

A dedicated Rust worker thread owns the dispatch loop: compilation of the note schedule,
the high-resolution timing wait, focus tracking, `SendInput` calls, cleanup, and native
telemetry. The Tauri/React shell renders the desktop UI, while native Rust services own
the playback state and safety policies.

## Known limitations

- Timing accuracy on the game side is not controlled by Sky Auto Player.
- Very fast passages (extremely high BPM or dense arpeggios) may exceed what the game's
  sampling rate can resolve, regardless of how precisely keystrokes are sent.
- MMCSS real-time scheduling requires Windows 10 or 11 and is not available on other platforms.
