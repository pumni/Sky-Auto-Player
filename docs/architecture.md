# System Architecture & Calibration

Sky Player is built on a modern, strictly-layered **Domain-Driven Design (DDD)**. The architecture separates the abstract concept of music from the harsh, real-time realities of OS thread scheduling and game engine polling constraints.

## 1. High-Level Architecture

The codebase is divided into four distinct layers:

1.  **Domain (`sky_music/domain/`):** Pure Python, zero side-effects. Contains immutable models (`Song`, `Note`), the strict JSON parser, and the Ahead-Of-Time (AOT) microsecond `scheduler`.
2.  **Orchestration (`sky_music/orchestration/`):** The real-time heart of the app. Contains the `PlaybackEngine` (which consumes the timeline) and the `Telemetry` & `Calibration` modules.
3.  **Infrastructure (`sky_music/infrastructure/`):** Bridging code. Includes focus tracking, hotkey listeners, and the `PreciseSleeper` utility.
4.  **Platform (`sky_music/platform/win32/`):** OS-specific implementations. Translates abstract actions into `SendInput` API calls using physical hardware scan codes.

---

## 2. The Playback Pipeline

The journey from a JSON file to a piano sound in-game follows a strict pipeline:

### Step 1: Parsing & Resolution
The `parser` reads the JSON file, strictly validating timestamps and schemas. Unmapped keys or negative timestamps instantly halt execution with clear errors. Keys are resolved into physical **Scan Codes** (ignoring OS keyboard language layouts like AZERTY/QWERTY).

### Step 2: The AOT Scheduler (`build_key_actions`)
Instead of calculating delays on the fly, the entire song is mapped out onto an absolute timeline in **microseconds** *before* playback begins.
*   **Tempo Scaling:** All timestamps are scaled by `tempo_scale` and converted to microseconds. Notes are emitted at their exact source time — the player generates the whole timeline against no external reference, so there is no uniform "input lead" shift (it was proven a no-op and removed; see `timing-architecture-audit.md`).
*   **Visibility Hold (`hold_us` / `min_hold_us`):** Each note is held down long enough to survive the game's per-frame input sampling. With FPS selected, built-ins materialise purely as `ceil(profile_frames * frame_us)`; with no FPS they use conservative `*_unframed_us` values. Explicit `_us` overrides remain an expert escape hatch. This is the only timing lever the scheduler enforces.
*   **Same-Key Feasibility:** If the same key repeats faster than `min_hold_us`, the previous hold is compressed down to `min_hold_us` (never below). If the authored interval is below `min_hold_us` the repeat is physically infeasible: `strict` mode rejects and recommends a slower tempo, `degraded` mode keeps `min_hold_us` and reports the overlap. There is no separate repeat-gap/chord-merge/frame-align knob — all three were removed after measurement showed they did not change real-song playback.
*   **Event Grouping:** Notes sharing the exact same timestamp are grouped into a single `SendInput` batch (chords). Notes a few ms apart go out at their own time; the game samples them on the same frame anyway.

### Step 3: The Real-Time Engine
The `PlaybackEngine` compiles the pre-calculated `KeyAction` timeline into per-key runtime
generations, then enters a highly optimized `while` loop checking `time.perf_counter_ns()`.

The runtime coordinator preserves authored down deadlines while enforcing the resolved
`min_hold_us` from the down dispatch start:

```text
release_not_before = down_dispatch_started + min_hold_us
effective_release = max(scheduled_release, release_not_before)
```

Releases are deferred per key, so protecting one note's hold does not block unrelated downs.
Generation identity also prevents a stale up from releasing a later same-key note after a conflict,
pause, focus loss, or panic release. In degraded mode, a runtime-infeasible same-key down is
explicitly dropped while other playable chord keys continue.

To achieve microsecond accuracy on Windows (where `time.sleep` is notoriously inaccurate), the engine uses a **Hybrid Sleeper** (`PreciseSleeper`) that steps toward each deadline:
1.  **Coarse Sleep:** If the next action is >20ms away, it OS-sleeps in chunks (capped at 20ms, waking ~5ms early) so the loop can still poll hotkeys/pause.
2.  **Medium / Yield:** Between ~5ms and the spin threshold it sleeps in 1ms ticks, then yields the thread (`sleep(0)`).
3.  **Spin-Lock:** For the final `spin_threshold_us` it busy-waits (spins the CPU) to hit the exact microsecond deadline without context-switching overhead.

The focus check that pauses playback on alt-tab is memoised on a short TTL so its heavy Win32 calls stay out of this spin phase.

---

## 3. Backend Safety & Anti-Cheat Compliance

The application interacts with the game exclusively through legitimate, public Windows APIs (`User32.SendInput`). It **does not** read game memory, inject DLLs, or hook global processes, making it safe for general use.

### Active State Tracking
The `WinSendInputBackend` maintains strict state tracking (`active_keys`). 
*   **Duplicate-Down Protection:** If a scheduled `DOWN` event fires for a key that the system thinks is already down, it is ignored to prevent sticky buffers.
*   **Multi-Pass Emergency Release:** When the user pauses or alt-tabs, the backend fires an immediate `release_all()`. To counteract OS queue blocking, it executes a 3-pass verification using `GetAsyncKeyState` to ensure the key actually bounced back up.

---

## 4. Telemetry & Auto-Calibration

Because every PC and network environment has different latency profiles, the engine logs its performance to help users find the perfect `FrameTimingPolicy`.

### Telemetry Logs
When running with the `--debug-csv` flag (or globally enabled in settings), a CSV is dumped to the `logs/` directory.
*   `lateness_us`: The delay between when a note was scheduled to play vs. when the OS actually fired it.
*   `send_duration_us`: How long the `SendInput` call blocked the thread.
*   `sent_scan_codes` / `skipped_scan_codes`: What the backend actually accepted or skipped.
*   `note_hold_duration_us`: Observed hold measured start-to-start, matching the runtime visibility
    contract.
*   `confirmed_hold_lower_bound_us`: Advisory worst-case diagnostic measured from down dispatch
    start through matching up dispatch completion.
*   `runtime_outcome` / `deferred_by_us`: Whether an intent was sent, deferred, suppressed, or
    dropped by an explicit runtime conflict decision.

### Calibration Loop
The Orchestration layer includes a `calibration` module that analyzes the P95 and P99 percentiles of the telemetry lateness.

**How to Calibrate via CLI:**
1. Play a song with `--debug-csv`.
2. Run `python src/main.py --auto-calibrate` to view recommendations based on jitter. Schedule stress and fast repeats recommend `local-precise` plus a tempo reduction; dense polyphony may recommend `audience-safe`.
3. Run `python src/main.py --save-calibration` to permanently write the recommended profile and FPS offsets to `config.json`.

**How to Calibrate via UI:**
In the Interactive Command Palette, press the `C` key to bring up the Calibration screen. If a recent telemetry log exists, the UI will display the measured jitter and allow you to save the recommended profile with a single keystroke.
