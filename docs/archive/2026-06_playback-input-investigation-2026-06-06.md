> ARCHIVED 2026-06 — historical plan/audit. Không phải tài liệu hiện hành.
> Contract & sự thật hiện tại: ../timing-principles.md và ../architecture.md.
> CẢNH BÁO lệch code đã biết: quan sát về FPS toggle và các lo ngại pipeline cũ (đã fold vào principles).

# Playback/Input Investigation Notes - 2026-06-06

This note records fresh runtime observations from the current playback/input path. Treat older timing
docs as historical context, not as unquestioned ground truth.

## Fresh Observation: Game FPS Toggle Is Not A Reliable Fix

User-observed sequence:

1. Game and player were run at 144 FPS and some notes were not accepted by the game.
2. Game and player were switched to 60 FPS and missing notes disappeared.
3. Game and player were switched back to 144 FPS and missing notes also disappeared.
4. A later retest showed that toggling the game FPS again did not reliably clear the missing-note
   symptom.

Important interpretation:

- If the game is truly running at 144 FPS, a player-side 60 FPS setting would produce a longer hold
  window (~16.7 ms), so it should be safer, not more likely to miss notes.
- Therefore the earlier 144 FPS misses are unlikely to be explained simply by the player accidentally
  using a 60 FPS timing profile.
- The stronger hypothesis is that toggling the game from 144 -> 60 -> 144 may sometimes reset or
  resynchronise an internal game/input/timing state.
- Because the retest did not reliably clear the symptom, FPS toggling must not be treated as a
  standard workaround or product behavior.
- This means 144 FPS is not inherently proven unreliable, but the system needs a clean architecture
  and better evidence before relying on any game-state workaround. The symptom may be sensitive to
  the game's current internal input sampling phase, FPS limiter state, focus/window state, or frame
  pacing state.

If the issue reproduces, preserve the state before changing settings:

- Save the telemetry summary and CSV.
- Note the game FPS setting and whether the game was recently toggled between FPS modes.
- Optionally test toggling only the game FPS 144 -> 60 -> 144, keeping the same player settings, but
  treat the result as diagnostic only.
- If missing notes disappear while player telemetry remains unchanged, that run suggests an after-
  SendInput game/input sampling factor. If missing notes persist, the toggle result should not be
  over-interpreted.

## Current Play/Input Pipeline Concerns

These are architecture findings from the current code, independent of the FPS-toggle observation.

### Backend ownership is not fully isolated

The threaded design intends the dispatch thread to be the only thread touching the input backend.
However, `PlaybackEngine.__init__` attaches the backend to the renderer, and `ProgressRenderer.render`
calls `backend.get_health()` during HUD rendering. In threaded playback, that means the main/render
thread can read backend state while the dispatch thread is sending input.

This does not directly prove note loss, but it violates the intended backend-owner rule and makes the
input path less clean than the design goal.

Relevant files:

- `src/sky_music/orchestration/engine.py`
- `src/sky_music/ui/hud.py`
- `src/sky_music/infrastructure/backend.py`

### Two threads do not fully isolate Python CPU work

The threaded path moves focus polling and HUD rendering off the real-time dispatch loop, which is the
right direction. But Python threads still share the GIL. A renderer or control path that does CPU-heavy
Python work can still delay the dispatch thread even though it runs on a separate OS thread.

A local probe showed that a CPU-bound renderer can disturb down-dispatch intervals. The existing
threaded test uses `time.sleep()` to simulate slow rendering, but `sleep()` releases the GIL, so that
test does not cover CPU-bound terminal/render work.

This means the threaded architecture is an improvement, but not a complete hard real-time isolation.

### Telemetry proves send-side behavior, not game acceptance

Telemetry can show that the player scheduled a note, called the backend, and `SendInput` accepted the
event into the Windows input stream. It cannot prove the game sampled the key-down state or produced
audio.

Therefore:

- Clean lateness does not prove the game accepted every note.
- `sent_down_count == expected` rules out internal scheduler/backend drops, but not game-side misses.
- Audio/onset validation is still required for after-send failures.

### 1-frame hold can be probabilistic

At 144 FPS, one frame is about 6.94 ms. If the game's input/music sampling tick is not perfectly
aligned with that short key-down window, some notes can be accepted and others can be missed without
any audible timeline drift. Longer holds fix the symptom because they cover more sampling phases, but
they also reduce the sharp 144 FPS feel.

The research goal should therefore be:

- Do not blindly increase hold globally.
- Identify whether the game can be phase-aligned or reset into a stable 144 FPS input state.
- If needed, find the minimum reliable hold or a targeted mitigation that preserves the local-precise
  feel as much as possible.

## Suggested Next Evidence

When missing notes reappear:

1. Run a simple probe such as `TEST_metro_alt_200` with `--debug-csv`.
2. Record clean game audio with a percussive instrument and muted background audio.
3. Analyze sent downs vs heard onsets.
4. Repeat after only toggling the game's FPS mode 144 -> 60 -> 144.
5. Compare telemetry and audio. If telemetry is the same but audio acceptance changes, the decisive
   variable is game/input state, not the scheduler.
