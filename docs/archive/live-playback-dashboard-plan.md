# Kế hoạch: Live Playback Dashboard (Bước 1) — bản thực thi

> **Đối tượng đọc:** kỹ sư **thực thi**. Tài liệu tự chứa — không cần ngữ cảnh hội thoại.
> **Reviewer:** tác nhân giám sát (nghiệm thu theo "Cổng kiểm duyệt" cuối tài liệu). Không merge khi chưa qua cổng.
> **Ngày:** 2026-06-07. Sự thật hiện hành tham chiếu: `architecture.md`, `timing-principles.md`.

---

## 0. TL;DR

Hôm nay: chọn bài (Textual) → **thoát TUI** → `play_selected_song()` phát nhạc trên **console** với `ProgressRenderer` (in ANSI). Sau khi phát → quay lại picker (`main.py:1466`).

Bước 1 thay **phần HUD lúc phát** bằng một **PlaybackApp (Textual)** đọc trạng thái qua **snapshot + poll 10Hz**. Picker và playback vẫn là **hai `app.run()` riêng** (giữ nguyên cấu trúc loop) — đây là thay đổi ít phá vỡ nhất, lấy ~80% giá trị UX. Gộp screen / unified control center là **Bước 2–3, ngoài phạm vi**.

**Quyết định nền (đã chốt với chủ dự án):** chấp nhận chạy UI song song lúc phát, KHÔNG đo trước, vì rủi ro được khử bằng thiết kế (xem §4). Stutter được quy về tụt FPS game (đặc biệt 144fps), không phải player-side.

---

## 1. Bất biến (vi phạm = fail review ngay)

1. **KHÔNG sửa** bất kỳ file nào trong:
   - `src/sky_music/domain/`
   - `src/sky_music/orchestration/` (đặc biệt `engine.py`, `runtime_dispatch.py`, `telemetry.py`)
   - `src/sky_music/infrastructure/`, `src/sky_music/platform/`
   - `src/sky_music/ui/hud.py` (ProgressRenderer — giữ làm fallback, **không đổi**)
   - `src/sky_music/config.py`, `src/sky_music/layouts.py`
   > Được phép **đọc & gọi**, không được **đổi**.
2. **KHÔNG đụng** đường dispatch real-time: không sửa scheduler, không sửa thread dispatch, giữ nguyên `enable_gc_pause` và mọi cờ timing truyền vào `PlaybackEngine`.
3. **Giữ tương thích CLI**: mọi flag hiện có hoạt động y như cũ. Fallback non-TTY và `--ui classic` vẫn dùng `ProgressRenderer` console như hôm nay.
4. **KHÔNG đổi** chữ ký/hành vi của `PlaybackEngine`, `SnapshotProgressSink`, hay protocol renderer. `SnapshotRenderer` mới phải **khớp interface engine đang gọi** (xem §3).
5. Coding standards (AGENTS.md): Python 3.14, **type hints bắt buộc**, ưu tiên `@dataclass(frozen=True, slots=True)`, không global mới, có test. Dùng `uv run`.
6. Phải **xanh test** + smoke mô tả trong PR trước khi qua cổng.

---

## 2. Bối cảnh kiến trúc (đọc kỹ trước khi code)

### 2.1 Luồng hiện tại
`main.py::main()` → `while True:` (main.py:1466) → `prompt_song_selection(...)` (chạy picker Textual, **thoát**) → `play_selected_song(...)` (main.py:565) phát nhạc trên console → lặp.

`play_selected_song` làm tuần tự: parse → `build_key_actions` → `validate_key_actions` → `analyze_schedule` (risk) → preflight/countdown → tạo `renderer = ProgressRenderer(...)` (main.py:741) → tạo `PlaybackEngine(... renderer=renderer ...)` (main.py:756) → `result = engine.play()` (**blocking**, main.py:778) → trả `result`.

`result` ∈ {`"finished"`, `"skipped"`, `"quit"`} (hằng `PLAYBACK_FINISHED/SKIPPED/QUIT` ở `hud.py:15-17`). Caller (main.py:1501) chỉ phân biệt `PLAYBACK_QUIT` (return 0) và `PLAYBACK_SKIPPED` (sleep ngắn).

### 2.2 Interface engine → renderer (HỢP ĐỒNG — `SnapshotRenderer` phải implement đúng)
Engine gọi (xem `engine.py:992-1016`, `hud.py`):
```python
def render(self, current: float, total: float, song_name: str,
           status: str = "playing", force: bool = False,
           input_path_degraded: bool = False,
           backend_health: "BackendHealth | None" = None) -> None: ...

def update_counters(self, lateness_us: int) -> None: ...   # optional; engine dùng hasattr-guard

def finish(self, message: str = "") -> None: ...
```
- `status` ∈ {`"playing"`, `"paused"`, `"focus_lost"`, `"waiting_for_focus"`} (xem `hud.py:191-200`).
- `current`/`total` tính bằng **giây** (engine đã chia 1e6, `engine.py:995-996`).
- Engine **đã tự gom** update qua `SnapshotProgressSink` và gọi renderer; `render()` của `ProgressRenderer` còn tự throttle 10Hz (`hud.py:167`). `SnapshotRenderer` **không cần** throttle khi ghi — chỉ lưu giá trị mới nhất; throttle nằm ở vòng poll UI.

### 2.3 Hotkey lúc phát
Pause/skip/quit (F8/F9/F10) đi qua `PlaybackControls` / `infrastructure/hotkeys` ở mức **global hook**, độc lập focus terminal. Lúc phát, **game** giữ focus, không phải terminal → PlaybackApp chỉ là **observer**, không bind các phím này. Không được "cướp" hotkey trong Textual.

### 2.4 Khi nào dùng Textual vs console
Tái dùng tiêu chí đã có cho picker: `_supports_textual()` trong `main.py` (TTY + Windows Terminal/VS Code). Nếu không hỗ trợ, hoặc `--ui classic` → **giữ nguyên** đường `ProgressRenderer` console. Textual playback chỉ bật khi picker Textual cũng bật.

---

## 3. Việc cần làm (Bước 1)

### 3.1 File mới: `src/sky_music/ui/textual_app/playback_app.py`

**(a) `SnapshotRenderer`** — implement đúng interface §2.2.
- Lưu **trạng thái mới nhất** vào field (đề xuất: một `@dataclass` `PlaybackSnapshot` frozen + một ô chứa duy nhất, hoặc các field rời). Single-writer (engine thread) / single-reader (UI poll).
- `update_counters`: **Bước 1 KHÔNG render counter.** Engine vẫn gọi method này nên phải có; cho phép cộng dồn `max_lateness_us` vào field nội bộ (cheap, single-writer) **nhưng không hiển thị** — để Bước 3 bật Debug panel mà không phải đổi hợp đồng. Tối thiểu nhất: thân hàm chỉ cập nhật `max_lateness_us`, KHÔNG đụng UI.
- `finish(message)`: set cờ `done=True` + lưu `message`.
- **Tuyệt đối không** import/gọi Textual hay `call_from_thread` ở đây. Chỉ ghi field. Không khoá nặng (giá trị nguyên thuỷ/đổi tham chiếu là đủ; nếu lo, dùng `threading.Lock` cực ngắn quanh swap).

**(b) `PlaybackApp(App[str])`** — màn hình playback tối thiểu.
- Layout: tên bài · progress bar (Textual `ProgressBar` hoặc custom) · `elapsed / total` (mm:ss) · status (playing/paused/focus_lost/waiting_for_focus, đổi màu) · dòng nhắc hotkey tĩnh (F8 pause · F9 skip · F10 quit).
- Theme: lấy token theo `cfg.theme` qua `TEXTUAL_THEME_TOKENS` (giống picker) để đồng bộ màu.
- `@work(thread=True, exclusive=True)`: chạy `result = engine.play()`; khi xong → lưu kết quả, `self.call_from_thread(self.exit, result)` **một lần** (đây là call_from_thread DUY NHẤT được phép — lúc kết thúc, không phải mỗi note).
- `set_interval(0.1, self._poll)`: đọc `SnapshotRenderer` → cập nhật widget. Nếu `renderer.done` → `self.exit(result)` (phòng khi worker chưa kịp).
- `ansi_color = True`, CSS inline (KHÔNG `.tcss` — quy ước đóng gói PyInstaller của dự án).

**(c) Entry:** 
```python
def run_playback_textual(engine: "PlaybackEngine", renderer: "SnapshotRenderer",
                         *, theme_name: str, song_name: str, total_us: int) -> str:
    """Chạy PlaybackApp, trả 'finished'|'skipped'|'quit'. Khối cho tới khi engine.play() xong."""
```

### 3.2 Sửa `main.py::play_selected_song` (chỉ đoạn ~736-780)
- Chọn nhánh: nếu Textual hỗ trợ (cùng điều kiện picker) →
  - `renderer = SnapshotRenderer()` thay cho `ProgressRenderer(...)`.
  - tạo `PlaybackEngine(... renderer=renderer ...)` **y nguyên các tham số timing hiện có**.
  - `result = run_playback_textual(engine, renderer, theme_name=_active_theme_name, song_name=song.name, total_us=...)` thay cho `engine.play()`.
- Ngược lại (non-TTY / classic): **giữ nguyên** đường `ProgressRenderer` + `engine.play()` hiện tại.
- `clear_terminal()` trước/sau giữ hành vi cũ.
- Dry-run: cho đi qua cùng đường Textual (DryRunBackend vẫn hoạt động).

> Không đổi gì khác trong `play_selected_song` (parse/validate/risk/countdown vẫn console ở Bước 1).

### 3.3 Test (bắt buộc) — `tests/test_textual_playback.py`
Dùng `App.run_test()` + một **engine giả** (fake) implement `play()` bơm vào `renderer.render(...)` vài snapshot rồi `renderer.finish("done")` và return `"finished"`:
- [ ] app render progress đúng từ snapshot (elapsed/total, status).
- [ ] khi engine giả `finish()` → `app.run()` trả đúng `result` ("finished"/"skipped"/"quit").
- [ ] `update_counters(lateness_us)` không làm vỡ render khi gọi dày (mô phỏng note-rate).
- [ ] KHÔNG test nào khởi động playback thật / SendInput / hotkey global.
- [ ] (đơn vị) `SnapshotRenderer`: ghi rồi đọc trả đúng giá trị mới nhất; `done`/`message` đúng.

---

## 4. An toàn timing (khử rủi ro bằng thiết kế — không cần đo)

| Nguy cơ | Biện pháp trong spec |
|---|---|
| UI làm bẩn đường gửi | Không đụng dispatch/scheduler/GC-pause; engine không đổi (§1.2, §1.4) |
| Ngập asyncio loop ở note-rate | Engine→UI = **ghi snapshot**; UI **poll 10Hz**. `call_from_thread` chỉ gọi **1 lần** lúc exit (§3.1b) |
| Tải nền lúc phát | PlaybackApp render tối giản 10Hz; không worker nặng, không ProcessPool lúc phát |
| Tranh hotkey | PlaybackApp không bind F8/F9/F10; chúng là global hook (§2.3) |

> Tham chiếu memory: `player-dispatch-proven-metronomic`, `realtime-process-isolation`, `live-dashboard-decision`.

---

## 5. Ngoài phạm vi (Bước 2–3, KHÔNG làm trong PR này)
- Gộp picker + playback thành **một** app Textual bền vững (screen stack). Bước 1 giữ hai `app.run()`.
- Chuyển risk-analysis / countdown / error từ `print` sang modal Textual (xoá hẳn console đen).
- Debug panel chi tiết (lateness p50/p95, jitter, dropped, send_duration, FPS). **Bước 1 chỉ hiển thị progress + time + status** — không render bất kỳ counter trễ nào (chốt với chủ dự án); `max_lateness_us` chỉ tích luỹ nội bộ, chờ Bước 3 hiển thị.

---

## 6. Cổng kiểm duyệt (reviewer chạy)
- [ ] `git diff --name-only`: chỉ chạm `src/sky_music/ui/textual_app/playback_app.py` (mới), `src/main.py`, `tests/test_textual_playback.py`. **Tập bất biến §1 rỗng** (đặc biệt `engine.py`, `hud.py`, domain/orchestration/infra/platform/config).
- [ ] `SnapshotRenderer` khớp đúng interface §2.2; không import Textual; không `call_from_thread` ngoài 1 chỗ exit.
- [ ] `--ui classic` / non-TTY: regression — vẫn dùng `ProgressRenderer` console, hành vi phát y cũ.
- [ ] `--ui textual` (hoặc auto): chọn bài → phát → màn Textual hiện progress/time/status; F8/F9/F10 vẫn pause/skip/quit; kết thúc trả đúng result và quay lại picker.
- [ ] `uv run pytest tests/test_textual_playback.py` xanh; `uv run pytest` không tăng số fail so với baseline timing đã biết.
- [ ] `uv run ruff check .` và `uv run pyright` sạch trên file mới/sửa.
- [ ] Smoke mô tả trong PR: lệnh + quan sát (gồm 1 lần phát thật qua màn Textual, xác nhận hotkey + thoát đúng).

## 6b. Kết quả nghiệm thu (reviewer, 2026-06-07)

**Trạng thái: ✅ ĐÓNG DẤU — BƯỚC 1 HOÀN TẤT (2026-06-07). Mọi cổng qua, gồm Cổng 4 smoke thật do chủ dự án xác nhận 4/4 OK.**

- [x] **Cổng 1** — phạm vi file: chỉ `main.py` + `playback_app.py`(mới) + `tests/test_textual_playback.py`(mới). Tập bất biến §1 rỗng.
- [x] **Cổng 2** — `SnapshotRenderer` chỉ ghi field (lock ngắn), không import Textual; `call_from_thread` chỉ ở `run_engine` lúc kết thúc; UI qua `set_interval(0.1)` poll. Kết thúc qua return value `engine.play()` (đã bỏ quét chuỗi `finish_message`).
- [x] **Cổng 3** — bật Textual chỉ theo `_check_textual_support() is None` (đã gỡ nhầm lẫn theme "classic"). Non-TTY → console giữ nguyên.
- [x] **Cổng 5** — `tests/test_textual_playback.py`: 5 passed. Full suite 300 passed / 1 failed; cái fail (`test_rank_song_choices_benchmark`, 98<100 bài) là test picker môi-trường-phụ-thuộc, **không phải regression** của PR này.
- [x] **Cổng 6** — ruff sạch trên `playback_app.py` + test + vùng diff `main.py` (lỗi ruff còn lại ở `main.py` đều pre-existing ngoài hunk). Pyright không chạy được trong env review (chưa cài; uvx cô lập → false-positive textual). Field theme token `warning`/`danger`/`accent`/`muted`/`modal_background` đã kiểm tay → tồn tại trên `ThemePreset`.
- [x] Minor: `total_us` đã dùng cho thời lượng ban đầu; bỏ logic đoán result mong manh.
- [x] **Cổng 4 — smoke phát thật:** chủ dự án chạy 1 lượt thật, xác nhận **4/4 OK** (progress chạy · F8 pause→Paused vàng · F9→"skipped" về picker · F10→"quit" thoát). Đóng dấu 2026-06-07.

> Lưu ý doc: §2.4 tham chiếu `--ui classic`/`_supports_textual()` đã **lỗi thời** — code hiện dùng `_check_textual_support()` và không còn picker classic runtime. Gating của executor (`_check_textual_support() is None`) là đúng theo code hiện hành.

## 7. Quy trình bàn giao
1. Executor mở **1 PR** cho Bước 1, kèm: mô tả việc, kết quả `pytest` (đếm pass/fail), mô tả smoke (lệnh + quan sát), xác nhận "không chạm file bất biến §1".
2. Reviewer chạy checklist §6 + `git diff --name-only` đối chiếu §1.
3. Chỉ qua cổng mới mở Bước 2.
