# Native Timing & Infrastructure Experiments

The former Python/Textual experiment commands are retired. This document is a
record of native timing evidence and must not be used to launch `src/main.py`
or any other Python product entrypoint.

This document records the open experiments and calibration procedures for Sky Auto Player's timing infrastructure. For historical experiments (O1 through O10.4) and retired parameters, refer to the archived document [2026-06_timing-experiments.md](archive/2026-06_timing-experiments.md).

---

## 0. Setup and Tooling

### 0.1 Build the Test Songs
Use the committed native test songs under `songs/`; the retired Python helper
is no longer part of the supported workflow.
```bash
cargo test --manifest-path rust/Cargo.toml -p sky_player --locked
```

For the active experiments below, the relevant test song is:
* `TEST_metro_alt_120`: Alternate keys with steady pacing, used for measuring sleeper lateness and CPU impact.

### 0.2 Recording Game Audio (Audacity Loopback)
To record game audio accurately on Windows without background noise:
1. Open Audacity. In **Audio Setup**, select **Host = Windows WASAPI**.
2. Select **Recording Device = "<your speakers> (loopback)"**.
3. Set **Project Rate = 48000 Hz**, channels = **Mono**.
4. In-game, select a percussive instrument with a short decay.
5. Press record in Audacity, wait ~1 second, start the playback, and stop recording when finished.

---

## 1. Active Experiments

### O10.5 Measuring `spin_threshold_us`
* **Objective:** Measure the fixed global sleeper spin threshold that balances thread lateness against CPU consumption. This parameter is independent of hold selection.
* **Tooling Limits:** Current summary telemetry logs lateness but does not record CPU time or tag `spin_threshold_us`. Therefore, run conditions and CPU measurements must be tracked manually.
* **Warning against Dry-Runs:** Do **NOT** use `--dry-run` to measure spin threshold performance. `_should_use_dispatch_thread()` returns `False` for `DryRunBackend`, which forces execution onto the main thread using `RealSleeper` instead of the production dispatch thread and `WaitableTimerSleeper`. Measuring on dry-runs runs the wrong sleeper path, exaggerating the benefits of spinning. All measurements must be performed on the real threaded path.
* **Protocol:**
  1. Keep power plan, user-selected game FPS, hold selection, and background processes constant. Use `TEST_metro_alt_120`.
  2. Run the 4 levels in randomized/alternate order (do not run all `0` runs before moving to next levels).
  3. Perform at least 7 runs per level on the real threaded backend. Record the median and worst-run p95/p99 lateness.
     ```bash
     cargo test --manifest-path rust/Cargo.toml -p sky_player --locked -- spin_threshold
     ```
  4. **Metrics:** Down-event IOI std, lateness p95/p99, count of events exceeding 1ms/2ms, worst-case latency spike, and process CPU time.
* **Null Hypothesis / Decision Rule:** Choose the lowest spin threshold that matches the p95/p99 latency performance of the higher thresholds without a significant worst-case regression.
  * *Null Hypothesis:* If the p95/p99 lateness and send jitter at `spin_threshold_us = 0` are approximately equal to or show no significant regression compared to `1200`, the spin threshold knob has no gameplay effect and should be retired or fixed to `0` (avoiding CPU busy-waiting).
  * If process CPU telemetry is unavailable, the results remain inconclusive.
* **Report Template:**
  ```text
  O10.5 spin threshold | Phase live | Runs per level: __ | randomized rounds: yes/no
  __ us: median down-IOI std __; median p95/p99 __/__ us; worst max __; CPU time __
  Decision: global spin_threshold_us = __ / INCONCLUSIVE (missing CPU) / RETIRE (null hypothesis confirmed)
  ```

### O10.6 Measuring `focus_restore_grace_us`
* **Status:** **BLOCKED BY OBSERVABILITY — do not run manually until instrumentation is added.**
* **Rationale:** The engine pauses the playback timeline when focus is lost, waits for the grace period, and adds the combined pause duration to `pause_time_us`. Since telemetry uses playback time with pauses subtracted, the CSV cannot measure post-focus restoration latency or the exact gap before the first key dispatch.
* **Warning against Manual Alt-Tab:** Manual focus switches are highly noisy and cannot isolate the grace period. Calibrating this parameter requires programmatic focus loss/restore scenarios (using a test script calling `focusWindow()` or native Windows focus APIs to toggle focus in a loop for 20+ cycles to eliminate human timing jitter).
* **Prerequisite Instrumentation:** Code must first log wall-clock timestamps for `focus_lost`, `focus_active_detected`, `grace_complete`, and `first_send_after_focus`, and write the configured grace value to the run summary.
* **Protocol (once instrumented):**
  1. Verify timeline pausing and burst-prevention deterministically with a mock clock/focus guard.
  2. Run automated focus toggle scripts at 0, 25, 50, 100, and 150 ms in randomized order (minimum 20 focus cycles per level).
  3. Record audio to verify that the first post-focus note is registered by the game, and use telemetry to measure the actual wall-clock gap.
  4. Select a single **global grace value**; production uses `100_000` µs and it is independent of hold selection.
* **Decision Rule & Alternatives:**
  * *Decision Rule:* If testing shows that setting `focus_restore_grace_us = 0` yields 100% registration of the first post-focus note, the grace parameter has no value and should be removed entirely.
  * *Alternatives:* If the game drops notes due to focus-switch delays (such as `SetForegroundWindow` race conditions), we should investigate implementing a programmatic focus-confirmation loop using Windows APIs rather than relying on a blind time sleep.
* **Report Template:**
  ```text
  O10.6 focus grace | cycles/level: __ | randomized: yes/no
  Grace __ ms: accepted first notes __/__ | active->first-send p50/p95 __/__ ms
  Decision: global focus_restore_grace_us = __ / INCONCLUSIVE / REMOVE (grace=0 passes) / CONFIRMATION_LOOP (race detected)
  ```

### O10.7 Retired Python UI-contention investigation
* **Status:** Historical evidence. The Textual UI and Python dispatch path are retired.
* **Tooling:** Automated using [measure_dispatch_tail.py](file:///d:/Dev/Sky%20Player/scripts/measure_dispatch_tail.py) simulating 60Hz UI load (GIL contention) and SendInput latency (empirical distribution: p50≈477µs, p99≈953µs, max≈1695µs).
* **Matrix results (2026-06-25):**
  - **Load Off / Def:** p50 lateness: -567.0µs | p99 lateness: 62.0µs | max lateness: 68.0µs
  - **Load On / 1ms:** p50 lateness: -568.0µs | p99 lateness: 40.0µs | max lateness: 334.0µs
  - **Load On / 5ms:** p50 lateness: -555.0µs | p99 lateness: 60.0µs | max lateness: 128.0µs
* **Decision:** **REJECT/CLOSE free-threaded python migration**. The maximum lateness change under maximum GIL contention (60Hz UI thread load) is only +266µs (334.0µs vs 68.0µs), well below the 500µs decision threshold. The current `switch-interval` tuning (1ms) is highly sufficient to handle GIL contention. No free-threaded Python migration is necessary.

---

## 2. Phase G Validation Results (Detailed Record)

*This section records the detailed measurement results of Phase G on 2026-06-06 to confirm the reliability of the completion-anchor mechanism.*

### Tier-2 Gate (per-block, not per-song)
At `144fps`, one frame is `6945 us`; the current `1.0`-frame production hold is `7445 us` with the default 500 µs device margin. A same-key block passes the "must be 12/12 sent" gate when:
$$\text{headroom} = \text{interval} - \text{min\_hold} > \text{real machine dispatch jitter}$$
(measured ~2.5 ms spike on the dev machine). Therefore blocks **8 ms+** (headroom $\ge 1$ ms) are valid gates; block **7 ms** (~55 us headroom) sits right on the floor edge, so any drop is a physical limit of tempo/hold selection, not a logic error in the anchor.

Actual measurement (run1/run2 with start-anchor fix): every 8 ms+ block achieved 12/12; only the 7 ms block dropped 4 notes — matching the theoretical prediction exactly. This proves the anchor operates correctly at the sender level for every interval whose headroom exceeds system jitter.

### Detailed Measurement Results (2026-06-06)
Tested with song `TEST_repeat_clean_144` (same-key blocks at intervals 20/24/30/40/55/70 ms), a `1.0`-frame hold at 144 FPS, 2 real play-throughs with game audio recording:
* **Sender gate:** Both runs achieved `intended_down = sent_down = 72`, `dropped_conflict/expired/suppressed_stale_up = 0` $\rightarrow$ sent data is completely clean, recorded audio is valid ground truth.
* **Game onsets:** Both runs registered **72/72** notes, all 6 blocks achieved 12/12 notes $\rightarrow$ zero dropped notes, zero lost blocks.
* **Sender IOI per block:** Varied from 0.0027 to 0.0243 ms (well below the 0.05–0.07 ms detection threshold).
* **Game-only jitter per block:** Std from 1.88 to 5.77 ms (entirely from game/audio internal latency and the onset splitter, not from the sender side).

**Conclusion:** The completion-anchor mechanism does not drop same-key notes in practice for intervals whose headroom lies above the machine's jitter amplitude. No additional fixed margin needs to be added beyond the device-delivery allowance.
