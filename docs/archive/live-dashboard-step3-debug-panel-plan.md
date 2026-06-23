# Kế hoạch: Debug Panel (Bước 3) — bản thực thi

> **Đối tượng đọc:** kỹ sư **thực thi**. Tài liệu tự chứa.
> **Reviewer:** tác nhân giám sát (nghiệm thu theo "Cổng" cuối tài liệu). Không merge khi chưa qua cổng.
> **Tiền đề:** Bước 1 + Bước 2 đã đóng dấu. Đã có `SnapshotRenderer`/`PlaybackScreen` (snapshot + poll 10Hz) và app gộp.
> **Ngày:** 2026-06-07. Sự thật hiện hành: `architecture.md`, `timing-principles.md`.

---

## 0. TL;DR

Thêm **Debug panel** cho `PlaybackScreen` với **toggle Normal/Debug**. Normal = như hiện tại (song/progress/time/status/warnings). Debug = thêm backend health (active/stuck keys), phân phối trễ (late>2/5/10ms, max, **p50/p95/jitter**), timing (fps/frame_us, hold/min, profile/tempo).

**Chỉ dùng dữ liệu renderer đã nhận** (`render()` args + `update_counters`) + policy truyền lúc dựng. **KHÔNG đụng engine/telemetry.** `send_duration_us`, `dropped_conflict`, `observed_hold`, MMCSS/timer-resolution/queue-depth nằm trong `engine.telemetry` (bất biến) → **ngoài phạm vi** (xem §7).

Giữ nguyên mô hình an toàn: engine ghi → UI poll 10Hz; không thêm tải lên đường dispatch real-time.

---

## 1. Bất biến (vi phạm = fail review ngay)

1. **KHÔNG sửa** `src/sky_music/domain/`, `src/sky_music/orchestration/` (gồm `engine.py`, `telemetry.py`, `runtime_dispatch.py`), `src/sky_music/infrastructure/`, `src/sky_music/platform/`, `src/sky_music/ui/hud.py`, `config.py`, `layouts.py`. Được **đọc & gọi**, không **đổi**.
2. **KHÔNG đổi hợp đồng** engine→renderer (`render`/`update_counters`/`finish` chữ ký giữ nguyên — engine gọi chúng, ta không sửa engine).
3. **KHÔNG thêm tải/khoá lên đường dispatch real-time.** Xem §4 (ràng buộc thread của `update_counters`).
4. Giữ tương thích: `ProgressRenderer` console (`hud.py`) không đổi; luồng `--song`/non-TTY không đổi.
5. Coding standards (AGENTS.md): Python 3.14, type hints, `@dataclass(frozen=True, slots=True)` ưu tiên, không global mới, có test, `uv run`.

---

## 2. Bối cảnh — nguồn dữ liệu khả dụng (KHÔNG đụng engine)

### 2.1 Từ `SnapshotRenderer.render(...)` (đã nhận, một phần chưa hiển thị)
- `current/total/status` — đang hiển thị.
- `input_path_degraded` — đang hiển thị (banner).
- `backend_health: BackendHealth | None` — **CHƯA hiển thị.** Có `active_count` (active keys) và `failed_release_count` (stuck keys) — xem `infrastructure/backend.py::BackendHealth.snapshot()`.

### 2.2 Từ `SnapshotRenderer.update_counters(lateness_us)`
- Hiện chỉ cập nhật `max_lateness_us`. Có thể tích luỹ thêm: đếm late>2/5/10ms (mô phỏng `ProgressRenderer.update_counters`, hud.py:91-103) và **ring buffer sample** để tính p50/p95/jitter.

### 2.3 Từ policy/plan (truyền lúc dựng PlaybackScreen)
- `profile`, `tempo`, `fps`, `frame_us`, `hold_us`, `min_hold_us` — lấy từ `PlaybackPlan.active_policy` + `session`. Hiện PlaybackScreen mới nhận `theme/song/total/violations` → cần truyền thêm policy/profile/tempo.

### 2.4 Tiền lệ — console verbose HUD đã hiện gì (hud.py:251-282)
`backend healthy/stuck · late >2ms/>5ms/>10ms · active keys · Timing: fps (frame_us) · hold/min`. Debug panel Textual nên **đạt parity + thêm p50/p95/jitter**.

---

## 3. Việc cần làm

### 3.1 Mở rộng `SnapshotRenderer` (`ui/textual_app/playback_app.py`)
- `update_counters(lateness_us)`: giữ `max_lateness_us`; thêm đếm `late_2ms/late_5ms/late_10ms`; thêm **ring buffer bounded** các sample lateness gần đây (vd `collections.deque(maxlen=4096)`) để tính percentile. Chi phí O(1)/sample.
- `render(...)`: lưu `backend_health` vào snapshot (đã có field `backend_health` trên `PlaybackSnapshot` — chỉ cần hiển thị).
- Thêm method đọc thống kê cho UI: `debug_stats() -> DebugStats` (frozen dataclass) tính p50/p95/jitter/max + late bands + active/stuck keys **tại thời điểm poll** (UI thread). Percentile: copy buffer dưới lock ngắn rồi `sorted`/`statistics` ngoài lock; deque ≤4096 → sort ~µs, chạy 10Hz vô hại. `jitter` = stdev (hoặc p95−p50, executor chọn, ghi rõ).

### 3.2 `PlaybackScreen` (`ui/textual_app/playback_app.py`)
- **Toggle Normal/Debug:** một phím **KHÔNG trùng** hotkey global (F6 refocus / F8 / F9 / F10 / panic). Đề xuất `F2` hoặc `d`. Mặc định Normal; nhớ trạng thái trong phiên screen.
- **Debug section** (chỉ hiện khi Debug): 3 dòng —
  1. `backend {healthy|stuck:N} · active keys: N`
  2. `late >2ms:N >5ms:N >10ms:N · max {x}ms · p50 {x}ms · p95 {x}ms · jitter {x}ms`
  3. `Timing: {fps}fps ({frame_us}us) · hold/min {hold}/{min}us · {profile} {tempo}×`
- Vòng `_poll` (10Hz đã có) cập nhật các dòng này qua `debug_stats()` khi Debug bật. Normal mode KHÔNG gọi `debug_stats()` (khỏi tốn sort).
- Truyền thêm `active_policy`/`profile`/`tempo` vào constructor `PlaybackScreen` từ `execute_playback_plan` (đọc từ `plan`).

### 3.3 Footer/hint
- Thêm gợi ý phím toggle vào dòng hotkey của PlaybackScreen (vd `... · F2 debug`).

---

## 4. An toàn timing (ràng buộc cứng)

| Nguy cơ | Biện pháp |
|---|---|
| Thêm khoá lên đường dispatch real-time | **Executor PHẢI xác minh** `update_counters` chạy trên thread NÀO. Trong engine, counters được **gom batch** qua `SnapshotProgressSink` và tiêu thụ ở `_consume_progress_updates` (engine.py:1004-1016) — tức **off hot-path** (consumer/render thread), KHÔNG phải dispatch thread. Nếu đúng vậy: deque.append + lock ngắn an toàn. Nếu (bất ngờ) chạy trên dispatch thread: KHÔNG dùng lock — giữ lock-free như Bước 1. Ghi kết luận xác minh vào PR. |
| Sort percentile làm chậm | Chỉ tính trong `debug_stats()` ở UI poll 10Hz, deque bounded ≤4096; Normal mode không tính. Không tính trong `update_counters`. |
| Đụng engine/telemetry | Không. Chỉ dùng dữ liệu renderer đã nhận + policy. |

> Tham chiếu memory: `player-dispatch-proven-metronomic`, `realtime-process-isolation`, `live-dashboard-decision`.

---

## 5. Test (bắt buộc) — bổ sung `tests/test_textual_playback.py`
- `SnapshotRenderer.debug_stats()`: bơm chuỗi lateness đã biết → p50/p95/max/jitter + late bands đúng; buffer rỗng → giá trị an toàn (0/None), không chia 0.
- Toggle: Pilot mở PlaybackScreen (engine giả bơm snapshot+counters), nhấn phím toggle → Debug section xuất hiện/biến mất; Normal mode không hiển thị debug.
- Debug render từ `backend_health` giả (active/stuck) đúng.
- KHÔNG test nào chạy playback thật/SendInput/hotkey global.

---

## 6. Cổng kiểm duyệt (reviewer)
- [ ] `git diff --name-only`: chỉ `ui/textual_app/playback_app.py` + test (+ doc). **Bất biến §1 rỗng** (đặc biệt `engine.py`, `telemetry.py`, `hud.py`, domain/orchestration/infra/platform/config).
- [ ] Engine→renderer hợp đồng không đổi; không đụng engine/telemetry.
- [ ] **Xác minh thread `update_counters`** ghi trong PR; không thêm khoá lên dispatch thread real-time.
- [ ] `update_counters` O(1)/sample; percentile chỉ tính ở poll Debug; Normal mode không tính.
- [ ] Toggle dùng phím KHÔNG trùng hotkey global; mặc định Normal.
- [ ] Test xanh; `uv run pytest` không tăng fail so baseline; ruff sạch; (pyright nếu chạy được).
- [ ] **Smoke phát thật (chủ dự án):** bật Debug lúc phát → p50/p95/jitter/late/active-keys cập nhật hợp lý; nhạc KHÔNG nấc thêm khi bật Debug (xác nhận sort 10Hz vô hại).

## 6b. Kết quả nghiệm thu (reviewer, 2026-06-07)

**Trạng thái: ✅ ĐẠT CẤP CODE — còn 1 ruff vặt + smoke phát thật in-game của chủ dự án.**

- [x] Phạm vi: thay đổi Bước 3 ở `playback_app.py` + test. `git status` xác nhận **KHÔNG** đụng `engine.py`/`telemetry.py`/domain/orchestration/infra/platform/hud/config. (main.py/app.py/modals.py/__init__.py còn `M` là dư Bước 1/2 chưa commit.)
- [x] Hợp đồng engine→renderer không đổi; không đụng engine/telemetry.
- [x] **Thread `update_counters`:** chạy trên worker thread (consumer off hot-path), KHÔNG phải dispatch real-time. Thiết kế lock-free: `deque(maxlen=4096).append` atomic; `list(deque)` an toàn trong CPython (vòng C giữ GIL liên tục → append thread khác không chen vào → không "deque mutated during iteration"). Đúng.
- [x] `update_counters` O(1)/sample; percentile/stdev chỉ tính trong `debug_stats()` gọi ở `_update_ui` **chỉ khi `debug_mode`** (line 485). Normal mode không sort.
- [x] Toggle **F2** (`PlaybackScreen.BINDINGS`), không trùng F6/F8/F9/F10/panic; mặc định Normal; footer hint đổi theo mode.
- [x] Test: `test_textual_playback` 11 passed; full suite **313 passed, 1 failed** (benchmark env; flaky `test_live_cli_execution` pass) — không regress.
- [ ] **Ruff:** `tests/test_textual_playback.py:595` unused `BackendHealth` (auto-fix). `playback_app.py` sạch.
- [x] Ruff: đã vá unused `BackendHealth`.
- [ ] **Smoke phát THẬT in-game → PHÁT HIỆN DEFECT (2026-06-07).**

### 🔴 DEFECT D1 — F2 không toggle được lúc phát thật (focus)
Chủ dự án bấm F2 lúc **game Sky đang focus** → panel không hiện. **Root cause:** F2 là **Textual binding** (`PlaybackScreen.BINDINGS`), chỉ kích hoạt khi *terminal* có focus. Lúc phát thật game giữ focus, terminal nền → Textual không nhận F2. Đây đúng là lý do F8/F9/F10 phải là **global hotkey** (hook hệ thống), không phải binding.
> **Bài học quy trình:** Pilot mock smoke (`pilot.press("f2")`) KHÔNG tái hiện được "game giữ focus" nên báo pass sai. **Mọi tính năng phụ thuộc hotkey lúc phát BẮT BUỘC smoke tay in-game thật, không chấp nhận Pilot.**

### Fix đã chốt với chủ dự án — Hướng B (không phá bất biến infrastructure)
Quyết định Debug **trước khi phát**, panel hiện sẵn khi vào playback:
1. `PlaybackScreen.__init__`: thêm `debug_mode: bool = False`; `self.debug_mode = debug_mode` (bỏ hardcode False). `on_mount` đã gọi `_update_debug_visibility()` → panel hiện ngay nếu True.
2. `execute_playback_plan` (app.py): truyền `debug_mode=self.verbose_hud` khi dựng `PlaybackScreen` — **tái dùng toggle HUD sẵn có** của picker (phím `h`/command, persist `cfg.verbose_hud`), terminal còn focus lúc bật → không cần global hotkey mới.
3. Giữ F2 binding (vẫn hữu ích khi terminal focus: dry-run, hoặc khi đã alt-tab/pause).
4. (polish) cập nhật nhãn/help toggle HUD ở picker để nói rõ nó bật Debug panel lúc phát.
5. Test: `PlaybackScreen(debug_mode=True)` → `#debug-panel` visible từ đầu; `debug_mode=False` → ẩn.
> Hướng D (global hotkey F2 toggle real-time) cần sửa `infrastructure/hotkeys` (bất biến) → task riêng nếu sau này cần; chủ dự án chọn làm B trước.

## 7. Ngoài phạm vi (cân nhắc Bước 4)
`send_duration_us`, `dropped_conflict`, `observed_hold_us`, MMCSS status, timer resolution, queue depth, event-loop delay — chỉ có trong `engine.telemetry` (bất biến). Muốn hiển thị phải: (a) review thread-safety đọc `engine.telemetry` live, hoặc (b) thêm điểm phơi dữ liệu — đều cần đụng tầng orchestration → **một phase riêng có spec + gate timing**, không gộp vào Bước 3.

## 8. Quy trình bàn giao
1 PR cho Bước 3, kèm: mô tả việc, kết luận xác minh thread `update_counters`, `pytest` đếm pass/fail, mô tả smoke, xác nhận không chạm bất biến §1. Reviewer chạy §6 + `git diff --name-only`. Smoke thật của chủ dự án trước khi đóng dấu (như Bước 1/2).
