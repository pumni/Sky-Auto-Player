# Active Timing & Infrastructure Experiments

This document records the open experiments and calibration procedures for Sky Player's timing infrastructure. For historical experiments (O1 through O10.4) and retired parameters, refer to the archived document [2026-06_timing-experiments.md](file:///d:/Dev/Sky%20Player/docs/archive/2026-06_timing-experiments.md).

---

## 0. Setup and Tooling

### 0.1 Build the Test Songs
Run this command once to generate the test songs under `songs/`:
```bash
uv run python tests/make_test_song.py
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
* **Objective:** Select a global sleeper spin threshold that balances thread lateness against CPU consumption. This is a global engine parameter and does not vary by timing profile.
* **Tooling Limits:** Current summary telemetry logs lateness but does not record CPU time or tag `spin_threshold_us`. Therefore, run conditions and CPU measurements must be tracked manually.
* **Protocol:**
  1. Keep power plan, game FPS, profile, and background processes constant. Use `TEST_metro_alt_120`.
  2. Run the 4 levels in randomized/alternate order (do not run all `0` runs before moving to next levels).
  3. Phase A uses `--dry-run` to isolate sleeper performance; Phase B confirms the best two levels with real `SendInput` dispatch.
  4. Perform at least 7 runs per level. Record the median and worst-run p95/p99 lateness.
     ```bash
     uv run python -m main --song TEST_metro_alt_120 --fps 144 --spin-threshold-us 0 --debug-csv --dry-run
     uv run python -m main --song TEST_metro_alt_120 --fps 144 --spin-threshold-us 500 --debug-csv --dry-run
     uv run python -m main --song TEST_metro_alt_120 --fps 144 --spin-threshold-us 800 --debug-csv --dry-run
     uv run python -m main --song TEST_metro_alt_120 --fps 144 --spin-threshold-us 1200 --debug-csv --dry-run
     ```
  5. **Metrics:** Down-event IOI std, lateness p95/p99, count of events exceeding 1ms/2ms, worst-case latency spike, and process CPU time.
* **Decision Rule:** Choose the lowest spin threshold that matches the p95/p99 latency performance of the higher thresholds without a significant worst-case regression. If process CPU telemetry is unavailable, the results remain inconclusive.
* **Report Template:**
  ```text
  O10.5 spin threshold | Phase dry/live | Runs per level: __ | randomized rounds: yes/no
  __ us: median down-IOI std __; median p95/p99 __/__ us; worst max __; CPU time __
  Decision: global spin_threshold_us = __ / INCONCLUSIVE (missing CPU)
  ```

### O10.6 Measuring `focus_restore_grace_us`
* **Status:** **BLOCKED BY OBSERVABILITY — do not run manually until instrumentation is added.**
* **Rationale:** The engine pauses the playback timeline when focus is lost, waits for the grace period, and adds the combined pause duration to `pause_time_us`. Since telemetry uses playback time with pauses subtracted, the CSV cannot measure post-focus restoration latency or the exact gap before the first key dispatch.
* **Prerequisite Instrumentation:** Code must first log wall-clock timestamps for `focus_lost`, `focus_active_detected`, `grace_complete`, and `first_send_after_focus`, and write the configured grace value to the run summary.
* **Protocol (once instrumented):**
  1. Verify timeline pausing and burst-prevention deterministically with a mock clock/focus guard.
  2. Run live probes at 0, 25, 50, 100, and 150 ms in randomized order (minimum 20 focus cycles per level).
  3. Record audio to verify that the first post-focus note is registered by the game, and use telemetry to measure the actual wall-clock gap.
  4. Select a global safety grace value (not profile-specific).
* **Current Fallback:** Maintain the current conservative default value (50/100/150 ms). Do not alter or differentiate this parameter across profiles without instrumented evidence.
* **Report Template:**
  ```text
  O10.6 focus grace | cycles/level: __ | randomized: yes/no
  Grace __ ms: accepted first notes __/__ | active->first-send p50/p95 __/__ ms
  Decision: global focus_restore_grace_us = __ / INCONCLUSIVE
  ```

---

## 2. Phase G Validation Results (Vietnamese Detail Record)

*Mục này ghi lại chi tiết kết quả đo thực tế của Phase G vào ngày 2026-06-06 nhằm xác nhận độ tin cậy của cơ chế completion-anchor.*

### Gate Tầng 2 (per-block, không phải toàn bài)
Ở `local_precise @144fps`, `min_hold = 6945 us`. Một block same-key chỉ là gate "phải 12/12 sent" khi:
$$\text{headroom} = \text{interval} - \text{min\_hold} > \text{jitter dispatch thật của máy}$$
(đo được ~2.5 ms spike trên máy dev). Vì vậy block **8 ms+** (headroom $\ge 1$ ms) là gate hợp lệ; block **7 ms** (~55 us headroom) ở đúng sát mép sàn nên nếu rớt là tín hiệu giới hạn vật lý của tempo/profile, không phải lỗi logic của anchor. 

Đo thực tế (run1/run2 với fix start-anchor): mọi block 8 ms+ đều đạt 12/12; chỉ block 7 ms rớt 4 note — khớp đúng dự đoán lý thuyết. Điều này chứng minh anchor đã hoạt động chính xác ở mức gửi (sender) cho mọi interval có headroom lớn hơn jitter hệ thống.

### Kết quả đo chi tiết (2026-06-06)
Thử nghiệm với bài `TEST_repeat_clean_144` (các block cùng phím có interval 20/24/30/40/55/70 ms), chạy profile `local_precise` ở 144 FPS trên 2 lượt chơi thực tế kèm thu âm game:
* **Sender gate:** Cả 2 lượt đều đạt `intended_down = sent_down = 72`, `dropped_conflict/expired/suppressed_stale_up = 0` $\rightarrow$ dữ liệu gửi đi hoàn toàn sạch, audio thu được là ground truth hợp lệ.
* **Game onsets:** Cả 2 lượt đều nhận **72/72** nốt, toàn bộ 6 block đều đạt 12/12 nốt $\rightarrow$ không mất nốt nào, không mất block nào.
* **Sender IOI per block:** Dao động cực nhỏ từ 0.0027 đến 0.0243 ms (thấp hơn nhiều so với ngưỡng biên 0.05–0.07 ms).
* **Game-only jitter per block:** Đạt std từ 1.88 đến 5.77 ms (hoàn toàn do độ trễ nội bộ của game/audio và bộ tách onset, không phải từ phía gửi).

**Kết luận:** Cơ chế completion-anchor không làm rớt nốt same-key nào trong thực tế đối với các khoảng interval có headroom nằm trên biên độ jitter của máy. Không cần tăng thêm bất kỳ margin cố định nào vào profile.
