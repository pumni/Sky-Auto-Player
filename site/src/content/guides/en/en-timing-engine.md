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
lastReviewedVersion: '3.0.0'
draft: false
related:
  - how-it-works
  - security-boundaries
  - troubleshooting
evidence:
  - category: architecture
    label: Timing principles
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/timing-principles.md
  - category: architecture
    label: Hold-frame timing model
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/hold-frame-model.md
  - category: architecture
    label: Real-time dispatch architecture
    url: https://github.com/pumni/Sky-Auto-Player/blob/main/docs/rt-dispatch-architecture.md
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

Instead of replaying a fixed list of delays, Sky Auto Player places every note on a
timeline driven by the sheet's tempo and the FPS setting you configure inside Sky.
When the scheduler reaches a note's position on that timeline, it dispatches the keystroke.

The scheduler is **pure**: it has no wall-clock dependency and no `SendInput` calls. It
produces a schedule of events; the infrastructure and platform layers execute those events.
This separation makes the timing logic unit-testable against a controlled clock.

## Chord alignment

Notes that belong to the same chord are submitted in a single `SendInput` call containing
all the keys in that chord. This eliminates sender-side skew between notes that should
sound simultaneously. Whether the game processes them simultaneously depends on the
game's own sampling loop.

## Hold semantics

A hold note (sustained key) is pressed at the note's start time and released after its
full notated duration. The next note starting close behind does not cause the hold to be
clipped short — the hold and the next note are submitted as separate `SendInput` events
on their own timeline positions.

## Hold-frame selection

The hold duration is expressed as a multiple of one game frame at the configured FPS:

| Selection            | Duration at 60 FPS |
| -------------------- | ------------------ |
| 1.0 frames (default) | ~16.7 ms           |
| 1.25 frames          | ~20.8 ms           |
| 1.5 frames           | ~25.0 ms           |

Choose 1.25 or 1.5 if short holds are not registering reliably on your machine.
The FPS setting must match the FPS configured inside Sky for hold timing to be correct.

## Latency adaptation

Sky Auto Player measures the time between when a `SendInput` call is scheduled and when
it actually completes, building a running profile of sender-side completion error. It
uses this measurement to adjust the lead time of future dispatches — sending slightly
earlier to compensate for observed delay.

This is **sender-side compensation only**. The adaptation reduces systematic drift caused
by OS scheduling jitter on your machine. It does not correct for network latency, game
frame pacing, or any factor on the game's side. The telemetry displayed in the HUD
reports the measured sender-side result, not a universal timing guarantee.

## Rust dispatch worker

A Rust worker owns the dispatch loop: compilation of the note schedule, the
high-resolution timing wait, focus tracking, `SendInput` calls, cleanup, and native
telemetry. Python owns the TUI and application flow. They run on separate threads; the
free-threaded Python 3.14 build ensures the dispatch loop does not contend with the
Textual UI on the GIL.

## Known limitations

- Timing accuracy on the game side is not controlled by Sky Auto Player.
- Very fast passages (extremely high BPM or dense arpeggios) may exceed what the game's
  sampling rate can resolve, regardless of how precisely keystrokes are sent.
- MMCSS real-time scheduling requires Windows 10 or 11 and is not available on other platforms.
