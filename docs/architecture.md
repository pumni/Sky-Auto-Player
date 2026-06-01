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
*   **Tempo Scaling & Input Lead:** All timestamps are scaled by tempo and shifted backward by `input_lead_us` to pre-compensate for network and OS latency.
*   **Chord Merging:** Notes falling within the `chord_merge_window_us` are grouped to fire simultaneously, sending fewer events to the OS.
*   **Hold Compression:** If the same key is pressed twice in rapid succession, the scheduler calculates the minimum required gap (`repeat_release_gap_us`) and mathematically shrinks the hold duration of the first note to guarantee the game engine registers the release.

### Step 3: The Real-Time Engine
The `PlaybackEngine` takes the pre-calculated `KeyAction` timeline and enters a highly optimized `while` loop, checking `time.perf_counter_ns()`.

To achieve microsecond accuracy on Windows (where `time.sleep` is notoriously inaccurate), the engine uses a **Hybrid Sleeper**:
1.  **Coarse Sleep:** If the next action is >20ms away, it uses standard OS sleep to yield CPU.
2.  **Yielding:** If <5ms away, it yields the thread (`sleep(0)`).
3.  **Spin-Lock:** For the final few hundred microseconds (`spin_threshold_us`), it busy-waits (spins the CPU) to hit the exact microsecond deadline without context-switching overhead.

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

### Calibration Loop
The Orchestration layer includes a `calibration` module that analyzes the P95 and P99 percentiles of the telemetry lateness.

**How to Calibrate via CLI:**
1. Play a song with `--debug-csv`.
2. Run `python src/main.py --auto-calibrate` to view recommendations based on jitter (e.g., if P99 lateness > 10ms, it will recommend downshifting to the `remote-safe` profile).
3. Run `python src/main.py --save-calibration` to permanently write the recommended profile and FPS offsets to `config.json`.

**How to Calibrate via UI:**
In the Interactive Command Palette, press the `C` key to bring up the Calibration screen. If a recent telemetry log exists, the UI will display the measured jitter and allow you to save the recommended profile with a single keystroke.
