# WASAPI Loopback Live Measurement Plan

Status: active implementation plan

Owner role split:
- Implementer: another AI coding agent.
- Reviewer / acceptance owner: this thread.

## 0. Why

`tests/measure_stutter.py` is the after-send ground-truth tool: it correlates the notes the runtime
*sent* (telemetry CSV) against the notes actually *heard* (audio onsets), and is the only way to
tell a before-send fault (scheduler/timer/GIL/GC) from an after-send fault (focus/UIPI/frame
sampling). Today the audio half is **manual**: record the game in Audacity, export 16-bit PCM WAV,
then run the analyzer by hand.

External research (GPT-5.5 deep report, 2026-06) recommends **WASAPI loopback** to capture the
system render stream programmatically. This plan automates only the audio-capture half so the
two-layer verification becomes a near-one-command flow. It does **not** touch playback timing.

## 1. Scope

In scope:

- A new audio-capture module + a "live" CLI under `tests/` that records the default render endpoint
  via WASAPI loopback to a 16-bit PCM WAV.
- A non-behavioural refactor of `tests/measure_stutter.py` to expose its analysis (onset detection,
  alignment, matching, the three verdicts) as importable functions the live tool reuses.
- An **optional/dev** dependency for the audio library.
- `docs/INDEX.md` update.

Out of scope (hard — rejection if violated):

- ANY change under `src/` — dispatch loop, timing, SendInput backend, scheduler, profiles, sleeper,
  `RealtimeProcessScope`, MMCSS. This is diagnostic tooling off the hot path.
- Changing the existing `measure_stutter.py` CLI behaviour or printed output (internal refactor only;
  the CLI must stay byte-for-byte backward compatible on the same inputs).
- Adding the audio library to **core** runtime dependencies.
- Reimplementing onset detection, offset search, or matching.

## 2. Current State

- `measure_stutter.py` (pure stdlib): `read_wav_mono` (16/32/8-bit PCM), `detect_onsets`
  (energy-novelty), `load_sent_downs`, `best_offset`, `match`, `fmt_t`. The three verdicts
  (MISSING / STUTTER GAPS / GAME-ONLY JITTER) and the validity/sender gates are computed and printed
  **inside `main()`**, so they are not yet reusable.
- Telemetry CSV is produced by `uv run play ... --debug-csv` into `logs/playback_telemetry_*.csv`.
  It now also carries `idle_gap_us` / `pre_send_spin_us` (sender-warmup columns) — irrelevant to this
  tool but present.
- Alignment is offset-search based (`best_offset`), so capture does **not** need to start at the same
  instant as playback — over-capturing a window that contains the whole song is sufficient.

## 3. Design Goals

1. No manual Audacity step: capture + analyze in one tool.
2. Single source of truth: reuse `measure_stutter.py`'s analysis verbatim.
3. Graceful degradation: a clear, actionable error when the library/device/loopback is unavailable —
   never an opaque stack trace, never a silent empty WAV.
4. Output WAV must be readable by the **existing** `read_wav_mono` (16-bit PCM, mono or stereo).
5. Zero runtime impact: nothing under `src/` imports the capture code; the player runs without the
   optional dependency installed.
6. Small surface; optional dependency only.

## 4. Required Changes

### 4.1 Refactor `measure_stutter.py` for reuse (no output change)

Extract the verdict computation + printing out of `main()` into one function, e.g.:

```python
def analyze_and_report(
    sent: list[float],
    heard: list[float],
    *,
    fps: int,
    tol_ms: float = 120.0,
    gap_ms: float = 30.0,
    top: int = 15,
) -> int: ...
```

`main()` becomes: parse args → load `heard` (wav/labels) → load `sent` (csv) → `analyze_and_report(...)`.
All existing functions stay public. The printed text/numbers for a given (wav, csv, flags) must be
identical to the current tool (verified by a golden test in §6).

### 4.2 New capture module `tests/audio_loopback.py`

Public function:

```python
def capture_loopback_to_wav(
    out_path: Path,
    *,
    stop_event: threading.Event | None = None,
    max_seconds: float | None = None,
    samplerate: int = 48_000,
) -> Path: ...
```

- Captures the **default render endpoint** via WASAPI loopback.
- Recommended library: **`soundcard`** (`get_microphone(default_speaker().name, include_loopback=True)`)
  as primary; `PyAudioWPatch` is an acceptable alternative. The implementer picks ONE and documents it
  in the module docstring with the exact install command.
- Writes a **16-bit PCM WAV** (convert float32 → int16, clamp) so `read_wav_mono` reads it unchanged.
  Do NOT emit IEEE-float WAV (format 3) — `read_wav_mono` cannot parse it.
- Stops on `stop_event` or after `max_seconds`, whichever first.
- If the library is missing, no default render device exists, or loopback is unsupported: raise a
  `RuntimeError` whose message names the cause and the fix (install command / enable device).

### 4.3 New live CLI `tests/measure_stutter_live.py`

Default mode is **capture-only orchestration** (do not subprocess-drive the interactive player; it has
focus/preflight prompts):

1. Start loopback capture in a background thread (stop on Enter, or `--duration N`).
2. Print: "Play your song now with `--debug-csv`; press Enter when it finishes."
3. Stop capture, write the WAV under `logs/`.
4. Auto-locate the newest `logs/playback_telemetry_*.csv` (or accept `--csv PATH`).
5. Call `analyze_and_report(...)` and print the same three verdicts. Echo the WAV path so the run can
   be re-analyzed later with `measure_stutter.py` directly.

Stretch (optional, NOT required for acceptance): a `--launch` flag that subprocess-runs
`uv run play --song ... --debug-csv` only if preflight prompts can be suppressed cleanly.

### 4.4 Dependency wiring

Add the chosen audio library as an **optional/dev** dependency (uv `--group dev`, or
`[project.optional-dependencies] measure = [...]`). It must NOT land in core runtime deps. Document the
install command in the script docstring and `docs/INDEX.md`.

## 5. Implementation Phases

- **Phase 0 — baseline:** run the current `measure_stutter.py` on an existing (wav, csv) pair (or a
  synthetic pair from §6); save the exact stdout as the golden.
- **Phase 1 — refactor:** extract `analyze_and_report`; prove byte-identical stdout on the Phase-0 input.
- **Phase 2 — capture:** implement `audio_loopback.py`; capture a few seconds of system audio while
  music plays; confirm the WAV opens in `read_wav_mono` and the onset count is plausible.
- **Phase 3 — live CLI:** wire capture → auto-CSV discovery → `analyze_and_report`.
- **Phase 4 — docs + optional dep.**

## 6. Test Matrix (how the reviewer verifies)

Automated (must be added and green):

- **Import test:** `from measure_stutter import analyze_and_report, detect_onsets, best_offset, match, load_sent_downs`.
- **Golden regression:** a synthetic 16-bit PCM WAV with known onset times + a matching synthetic CSV
  → `analyze_and_report` produces the expected matched/missing/gap counts; the refactor does not change
  output vs a captured pre-refactor golden on the same input.
- **Round-trip:** generate a WAV whose onsets line up with the CSV `actual_us` → assert the aligner
  matches them all (missing == 0) and the offset search converges.
- **Capture module import/probe:** `import audio_loopback` works; a device-probe helper returns the
  default loopback name. This test is `skip`/`xfail` when no audio device is present (CI).

Manual (reviewer runs on Windows 11):

- With Sky audible, run `measure_stutter_live.py`, play a **controlled percussive probe** with
  well-separated notes (`--debug-csv`), confirm the `[VALIDITY GATE]` passes (onset/sent ratio
  0.7–1.5) and the three verdicts print.
- Disable the audio device → the tool prints a clear actionable error, not a traceback.
- `uv run play ...` still works in an environment where the optional audio dep is **not** installed.

## 7. Acceptance Criteria (reviewer accepts only if all hold)

- No file under `src/` changed; no dispatch/timing/SendInput/scheduler/profile/`RealtimeProcessScope`
  touched.
- `measure_stutter.py` CLI output is unchanged on identical inputs (golden passes).
- Audio library is optional/dev only; core playback runs without it.
- The live tool reproduces the same verdicts as the manual flow on the same run.
- Loopback writes a 16-bit PCM WAV that `read_wav_mono` reads with no changes to that reader.
- Library/device/loopback missing → clear, actionable error.
- The full existing suite is still green (the 6 pre-existing `FrameTimingPolicy` failures are unrelated
  and out of scope — do not "fix" them here).

## 8. Rejection Criteria

- Any edit to the runtime dispatch/timing/SendInput path.
- Audio library added to core runtime deps.
- Onset detection / alignment reimplemented instead of reused.
- `measure_stutter.py` output changed for the existing CLI.
- A manual/Audacity step remains, OR the WAV is IEEE-float that `read_wav_mono` cannot parse.
- Silent failure / empty WAV when loopback is unsupported.

## 9. Handoff Prompt For Implementer

```text
Read AGENTS.md and docs/2026-06_wasapi-loopback-measurement-plan.md. Implement ONLY the audio-capture
automation for the after-send measurement; do NOT touch anything under src/ (no dispatch, timing,
SendInput, scheduler, profiles, RealtimeProcessScope). Refactor tests/measure_stutter.py to expose an
analyze_and_report() (no output change), add tests/audio_loopback.py that captures the default WASAPI
loopback render endpoint to a 16-bit PCM WAV readable by read_wav_mono, and add
tests/measure_stutter_live.py that captures + auto-finds the newest telemetry CSV + runs the analysis.
Use soundcard (or PyAudioWPatch) as an OPTIONAL/dev dependency only. Add the import/golden/round-trip
tests in the plan's §6. Fail loudly and clearly when loopback is unavailable. Run uv run pytest and
report the golden-regression result.
```

## 10. Reviewer (acceptance) Checklist

```text
- git diff --stat shows no src/ changes.
- rg -n "ThreadPoolExecutor|SendInput|spin_threshold|RealtimeProcessScope|sleep_step" src tests/measure_stutter*.py audio_loopback.py  -> only expected matches.
- uv run python tests/measure_stutter.py <golden.wav> <golden.csv> --fps 144  -> output == pre-refactor golden.
- uv run pytest tests/test_measure_stutter*.py -q  -> green (import/golden/round-trip).
- uv run pytest -q  -> green except the known FrameTimingPolicy pre-existing failures.
- Fresh venv WITHOUT the audio extra: uv run play --song <x> --dry-run  -> still works (no import error).
- Manual: live tool on a clean percussive probe reproduces the 3 verdicts; disabled-device case errors clearly.
```
