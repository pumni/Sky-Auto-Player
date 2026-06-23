# Kế hoạch: Re-layout thẻ play khi đổi cỡ + fps/config một-nguồn-duy-nhất (Bước 2.7) — bản thực thi

> **Đối tượng đọc:** kỹ sư **thực thi** (điều tra + sửa). Tài liệu tự chứa.
> **Reviewer:** tác nhân giám sát + chủ dự án (nghiệm thu theo "Cổng" §6).
> **Tiền đề:** Bước 2.5/2.6 đã code; smoke thật phát hiện 2 lỗi dưới. Bước 2.6 fix dock được coi là **band-aid** — chỉ căn đúng ở lần hiển thị đầu.
> **Ngày:** 2026-06-08.

---

## 0. TL;DR — 2 lỗi từ smoke thật

- **A. Thẻ play bị che sau khi "nở ra".** Luồng thật: countdown 3s (thẻ nhỏ) → playing (thẻ to ra để chứa progress/debug). Lúc nở ra, thẻ **không được căn lại** → tràn/cụt đáy. Cần **giải pháp tổng thể**: thẻ luôn ghim đáy terminal và `#songs` co/giãn theo *mỗi lần* đổi cỡ, không chỉ lần đầu.
- **B. Timing hiển thị "N/A".** Header app hiện một con số fps (vd 60) nhưng dòng Timing trong debug panel ra "N/A". Nghi vấn cốt lõi của chủ dự án: **tất cả cấu hình (đặc biệt fps) phải được đọc & dùng từ MỘT nguồn duy nhất** — hiện đang có nhiều đường resolve fps lệch nhau. Coder phải **điều tra tận gốc** vì sao, rồi hợp nhất nguồn.

---

## 1. Vấn đề A — Thẻ play phải ghim đáy ở MỌI lần đổi cỡ

### 1.1 Nguyên nhân gốc (đã xác định)
`PlaybackCard` (playback_app.py) là `Static` override `render()`, CSS `dock: bottom; height: auto`. Mọi cập nhật trạng thái đều gọi **`self.refresh()` trơn** (`show_idle/show_error/show_risk/start_countdown/start_playback/toggle_debug/_poll/_tick_countdown`).

`refresh()` chỉ **vẽ lại**, KHÔNG tính lại chiều cao auto của widget docked. Hệ quả:
- Thẻ được đo **một lần** khi hiện lần đầu (mode countdown, ~5 dòng).
- Khi chuyển sang playing (cao hơn) hoặc bật debug (~13 dòng), chiều cao **không được tính lại** → nội dung **tràn khỏi vùng đã reserve → cụt đáy**.
- **Test geometry của Bước 2.6 qua được chỉ vì dùng `countdown_seconds=0`** (vào thẳng playing ở cỡ cuối). Nó KHÔNG mô phỏng lúc "nở ra" — đó là lỗ hổng để band-aid lọt.

### 1.2 Giải pháp tổng thể (bắt buộc — không band-aid)
Khi chiều cao box đổi → **đặt chiều cao tường minh + ép re-layout**, để dock ghim lại đáy và `#songs 1fr` reflow *mỗi lần*. Cách làm xác định (deterministic), KHÔNG dựa vào "auto đo lại đúng lúc":

1. Tách phần dựng dòng trong `render()` thành helper `_compose_lines() -> list[str]` (đã có sẵn logic; chỉ trả về `lines` của `ansi_gradient_box`/`ansi_box`). `render()` = `Text.from_ansi("\n".join(self._compose_lines()))`.
2. Thêm `_rerender()` thay cho mọi `self.refresh()`:
   ```python
   def _rerender(self) -> None:
       lines = self._compose_lines()
       self.styles.height = len(lines)   # set tường minh → dirty layout → dock re-arrange
       self.refresh(layout=True)
   ```
   - Đặt height tường minh `= số dòng box` đảm bảo Textual reserve đúng và `#songs 1fr` co theo. `dock: bottom` giữ mép dưới sát đáy.
   - Bỏ `height: auto` khỏi CSS `#playback-card` (giờ height do code đặt). Giữ `dock: bottom; width: 100%; padding: 0; background: transparent`.
3. Thay **tất cả** `self.refresh()` trong các setter/`_poll`/`_tick_countdown` bằng `self._rerender()`. (Trong `_poll`, gọi `_rerender()` thay `refresh()` để phòng số dòng đổi giữa chừng: warnings/paused/focus_lost thêm-bớt dòng.)
   - Tối ưu nhẹ (tuỳ chọn): chỉ `self.styles.height = h; refresh(layout=True)` khi `h` khác lần trước, else `refresh()`. Không bắt buộc — 10Hz set height là rẻ.

> Nếu vì lý do nào đó `styles.height` tường minh xung đột với dock trên bản Textual hiện tại (≥8.2.7), fallback: giữ `height: auto` + dùng `self.refresh(layout=True)` trong `_rerender` (vẫn ép re-measure). Ưu tiên cách tường minh trước.

### 1.3 Test bắt buộc (`tests/test_textual_playback.py`)
1. **`test_card_anchored_after_countdown_grows` (MỚI — bắt đúng lỗi band-aid):**
   - `countdown_seconds=3`, dry-run, mock engine sleep dài, **mock `is_hotkey_down=False`**.
   - Vào countdown → ghi `card.region.height` (nhỏ) và `songs.region.height`.
   - Đợi qua countdown sang playing (`await pilot.pause(...)` đủ 3s+; hoặc gọi trực tiếp tick countdown). Khi `playback_mode=="playing"`:
     - `card.region.bottom == app.screen.region.bottom - 1` (vẫn ghim đáy sau khi nở).
     - `card.region.bottom <= app.screen.region.bottom` (không clip).
     - `card.region.height > card_height_countdown` (đã nở ra).
     - `songs.region.height < songs_height_countdown` (songs co thêm để nhường chỗ).
     - `songs.region.bottom <= card.region.y` (không chồng).
   - Chạy ở **2 size**: `(100, 30)` và `(60, 24)`.
2. **`test_card_anchored_after_debug_toggle_grows` (MỚI):** ở playing, mock `is_hotkey_down` bật → `card._poll()` (toggle debug, thẻ cao lên) → assert vẫn `bottom == screen.bottom-1`, không clip, songs co thêm.
3. Giữ test dock cũ (countdown=0) làm smoke nhanh.

---

## 2. Vấn đề B — fps/config phải đọc & dùng từ MỘT nguồn duy nhất

> **Yêu cầu chủ dự án (nguyên văn):** *"để AI coding tìm hiểu tại sao lại có vấn đề này, tất cả config cấu hình phải nhất được đọc và sử dụng từ 1 nguồn duy nhất."* → Coder PHẢI điều tra root-cause, KHÔNG vá hiển thị đơn thuần.

### 2.1 Triệu chứng & nghi vấn
- Header app hiện fps (chủ dự án thấy 60); dòng **Timing trong debug panel ra "N/A"**. Hai nơi **lệch nhau** ⇒ fps đang được resolve từ **nhiều nguồn khác nhau**.
- Header chip đọc **thẳng `self.fps`** (app.py `_render_status`:582 — `"auto"` nếu None, else `"{self.fps}fps"`, KHÔNG có fallback 60).
- Dòng Timing đọc `active_policy.fps`/`frame_us`; `FrameTimingPolicy.build(fps=None)` đặt `fps=0, frame_us=0` (scheduler_types.py:189-199) → card render "N/A".

### 2.2 Các điểm resolve fps đã phát hiện (LEADS để điều tra — xác minh từng cái)
| Nơi | Hành vi | Rủi ro lệch nguồn |
|---|---|---|
| `config.py:13` `CONFIG_PATH = Path("config.json")` | **đường tương đối theo CWD** | App chạy từ thư mục khác (repo root vs build/exe vs `src/`) → **đọc file config.json KHÁC nhau** → fps khác nhau. **Đây là nghi phạm số 1 cho single-source.** |
| `main.py:1489/1521` `resolved_fps = args.fps if cli_fps_explicit else (game_fps if game_fps>0 else None)` | gộp `game_fps==0` → `None` | `game_fps=0` (persisted) ⇒ unframed, dù "default" là 60 |
| `config.py` `game_fps` default `60`; `normalize_fps_value(0/None)→0`; `persist_default_fps(None)→0` | "không set" bị lưu thành `0` | đọc lại `0`→`None`→unframed; mất phân biệt "auto" vs "60" |
| `session_context.py:146` `selected_fps = self.fps if self.fps is not None else 60` | **fallback 60** khi resolve profile dict | nhánh này 60, nhưng `FrameTimingPolicy.build(fps=self.fps=None)` lại 0 → **một context 2 giá trị** |
| `app.py:228/701/1153` | `self.fps`→`picker_result.fps`→`session.fps` | nếu mọi hop = `self.fps` thì nhất quán; cần xác minh không hop nào rớt |

### 2.3 Việc của coder (điều tra → hợp nhất)
1. **Tái hiện & định vị:** in/log `self.fps` (header), `picker_result.fps`, `session.fps`, `plan.active_policy.fps`, và **đường dẫn tuyệt đối của config.json đang được load** tại thời điểm bắt đầu phát. Xác định fps "rơi" ở hop nào, và app đang đọc config.json ở **đâu** (có phải khác file chủ dự án nghĩ không).
2. **Một nguồn duy nhất cho config:** neo `CONFIG_PATH` về **một vị trí canonical** (vd cạnh entrypoint/`%APPDATA%`/thư mục dự án — chốt với chủ dự án), không phụ thuộc CWD. Mọi `load_config()` đọc cùng file.
3. **Một đường resolve fps duy nhất:** mọi nơi (header, policy, Timing, engine) lấy fps qua **cùng một hàm/đối tượng** đã resolve (đề xuất: resolve một lần ở session, mọi consumer đọc `session.fps`/`active_policy.fps` — không tự suy lại). Bỏ các fallback "60" rải rác (vd session_context:146) hoặc làm cho chúng nhất quán với `build()`.
4. **Hiển thị Timing trung thực (sau khi nguồn đã nhất quán):** trong `_playing_body`:
   - `fps > 0` → `Timing: {fps}fps ({frame_us}us)  ·  hold/min: {hold}/{min}us`.
   - `fps == 0` (unframed thật) → `Timing: unframed  ·  hold/min: {hold}/{min}us` (KHÔNG còn "N/A (N/A)").
   - **Bất biến hiển thị:** giá trị fps ở Timing **phải == giá trị ở header chip** (cùng nguồn).

### 2.4 ⚠️ Cảnh báo an toàn (đọc kỹ trước khi đổi resolve)
Hợp nhất nguồn có thể **đổi giá trị fps thực dùng** (vd từ unframed `0` thành `60`), mà **fps đổi = timing playback đổi** (`frame_us`, materialize hold) — vùng cực nhạy (memory `realtime-process-isolation`, `timing-one-frame-standard`, `player-dispatch-proven-metronomic`). Do đó:
- Mục tiêu B là **NHẤT QUÁN HOÁ + ĐỌC ĐÚNG NGUỒN**, KHÔNG tự ý đổi hành vi timing.
- Nếu điều tra cho thấy giá trị "đúng" khác giá trị đang chạy (vd thật ra phải 60 chứ không phải unframed), **CHỐT với chủ dự án** giá trị mong muốn trước khi đổi; ghi rõ đây là thay đổi timing có chủ đích, kèm smoke thật.
- KHÔNG sửa `scheduler_types.FrameTimingPolicy.build`, `domain/`, `orchestration/` để "ép" fps trừ khi chủ dự án duyệt. Phạm vi mặc định: **đường đọc/resolve config + hiển thị**, không phải công thức timing.

### 2.5 Test bắt buộc
1. **`test_header_fps_matches_policy_fps`:** trong unified app, sau khi vào playing, `self.fps` (nguồn header) và `plan.active_policy.fps`/`card.active_policy.fps` **bằng nhau** (cả hai 60, hoặc cả hai 0). Bắt chính cái lệch header≠Timing.
2. **`test_timing_line_no_bare_na`:** active_policy fps>0 → render chứa `"{fps}fps"`, KHÔNG chứa `"N/A"`; fps==0 → chứa `"unframed"`, KHÔNG chứa `"N/A"`.
3. **`test_config_single_source` (nếu đụng CONFIG_PATH):** `load_config()` đọc cùng một đường dẫn bất kể CWD (vd `monkeypatch` CWD, assert path không đổi). Không regress đọc/ghi config hiện có.

---

## 3. Bất biến
1. KHÔNG đụng real-time dispatch/scheduler/GC-pause/vòng gửi phím nóng; KHÔNG sửa công thức timing (`scheduler_types`, `domain/`, `orchestration/`) trừ khi chủ dự án duyệt (xem §2.4).
2. Giữ `quiesce()`+guard+auto-focus + in-place state machine (Bước 2.5).
3. Tái dùng: `ansi_gradient_box`/`ansi_box`, `SnapshotRenderer`, hotkey poll (Bước 2.6).
4. KHÔNG regress: `tests/test_textual_picker.py` + `tests/test_textual_playback.py` xanh; `--song` console path nguyên vẹn.
5. Phạm vi file mặc định: `ui/textual_app/playback_app.py`, `ui/textual_app/app.py`, `ui/textual_app/theme_css.py` (CSS), `tests/test_textual_playback.py`, và — chỉ cho §2 — `config.py`/`main.py`/`domain/session_context.py` ở mức **đường đọc & resolve** (không đổi công thức). Liệt kê rõ file đụng trong PR.

---

## 4. Cách chạy test / lint
```
uv run python -m pytest tests/test_textual_playback.py -q
uv run python -m pytest tests/test_textual_picker.py -q
uv run python -m pytest -q          # full: chỉ fail pre-existing 98>=100
uvx ruff check src/sky_music tests/test_textual_playback.py
```

---

## 5. Cổng kiểm duyệt
- [ ] `git diff --name-only`: trong phạm vi §3.5; KHÔNG đụng công thức timing (`scheduler_types`, dispatch, engine hot path) trừ khi có duyệt.
- [ ] **A:** test "nở ra sau countdown" + "debug toggle nở ra" xanh ở 2 size — thẻ luôn `bottom==screen.bottom-1`, không clip, `#songs` co theo *mỗi lần*. (Không chỉ lần đầu.)
- [ ] **B:** điều tra ghi lại root-cause (fps rơi ở hop nào + config.json đọc ở đâu); `test_header_fps_matches_policy_fps` + `test_timing_line_no_bare_na` xanh; nếu đụng CONFIG_PATH thì `test_config_single_source` xanh; KHÔNG còn "N/A (N/A)".
- [ ] Nếu B làm đổi giá trị fps thực dùng → có xác nhận chủ dự án + smoke thật ghi chú rõ.
- [ ] Full suite không tăng fail; `uvx ruff` sạch; picker không regress.
- [ ] **Smoke thật in-game (chủ dự án):** countdown 3s → thẻ **nở ra vẫn ghim đáy, không cụt**; debug panel Timing hiển thị **đúng & khớp header** (không N/A); F2 toggle khi Sky focus vẫn chạy; F9 về picker, F10 thoát.

> Memory: `live-dashboard-decision`, `realtime-process-isolation`, `timing-one-frame-standard`, `player-dispatch-proven-metronomic`. Bài học: hotkey/timing lúc phát BẮT BUỘC smoke tay in-game.
