# Kế hoạch: Bottom-dock thẻ play + F2 global + bỏ nền thẻ (Bước 2.6) — bản thực thi

> **Đối tượng đọc:** kỹ sư **thực thi**. Tài liệu tự chứa.
> **Reviewer:** tác nhân giám sát + chủ dự án (nghiệm thu theo "Cổng" §7). Không merge khi chưa qua cổng.
> **Tiền đề:** Bước 2.5 (in-place + HUD-box render) đã code & qua cổng tự động; chờ smoke thật. Bước 2.6 là **đánh bóng UI/UX** từ smoke thật, KHÔNG đổi luồng playback.
> **Ngày:** 2026-06-08.

---

## 0. TL;DR — 3 vấn đề từ smoke thật

1. **Thẻ play bị khuất đáy.** Thẻ nằm trong luồng dọc NGAY DƯỚI bảng `#songs` (vốn `height: 1fr`), nên bị đẩy xuống và **rơi khỏi/khuất một phần đáy viewport**. Mong muốn: **dock thẻ xuống đáy terminal**, bảng `#songs` **co lại** nhường chỗ.
2. **F2 (debug) không bấm được lúc phát.** F2 hiện là Textual binding → **chết khi Sky giữ focus** lúc phát (DEFECT D1). Mong muốn: F2 thành **global như F8/F9** (bấm được cả khi đang phát, game đang focus).
3. **Thẻ play có nền tối riêng.** `#playback-card` có `background: #080e1c`, lệch với các thẻ khác (`#songs`/`#detail` đều `background: transparent`). Mong muốn: **bỏ nền** cho đồng nhất.

---

## 1. Bối cảnh kiến trúc (đã khảo sát — đọc kỹ trước khi sửa)

### 1.1 Layout picker (`SkyPickerApp`)
- `compose()` (app.py:329-350) dựng `Container#root` chứa, theo thứ tự dọc:
  `GradientHeader#appbar` → `SearchInput#search` → `SongTable#songs` → `DetailPanel#detail` → **`PlaybackCard#playback-card`** (mặc định `display:none`) → `CustomFooter`.
- CSS nền (theme_css.py `BASE_CSS`):
  ```
  #root   { height: 100%; layout: vertical; padding: 1 2; }
  #appbar { height: 3; }
  #search { height: 3; margin: 1 0 0 0; }
  #songs  { height: 1fr; padding: 0 2; }      /* nuốt toàn bộ chiều cao dư */
  #detail { height: auto; min-height: 5; max-height: 9; margin: 1 0 0 0; }
  AppFooter { height: 1; margin: 1 0 0 0; }
  ```
- CSS thẻ play (app.py:187-193, sau Bước 2.5):
  ```
  #playback-card { width: 100%; height: auto; padding: 0; background: #080e1c; }
  ```
- Khi phát, `_show_playback_card(mode)` (app.py:1091) đặt `#detail`.display=none, `CustomFooter`.display=none, `#playback-card`.display=block. `_restore_picker_after_playback()` (app.py:1102) đảo lại.
- **Nguyên nhân khuất đáy:** `#songs` là `1fr` còn thẻ `height:auto` đứng SAU nó trong luồng dọc bình thường — thẻ KHÔNG được ghim vào đáy viewport. Tùy chiều cao terminal và chiều cao nội dung thẻ (idle 2 dòng → debug ~13 dòng), các dòng dưới của thẻ (viền đáy + hàng hotkey) **render vượt mép dưới** → khuất. Sửa đúng hướng user: **ghim thẻ xuống đáy (`dock: bottom`) để `#songs` 1fr tự co lại phía trên.**

### 1.2 Hệ thống hotkey playback (global, độc lập focus)
- `PlaybackControls` (infrastructure/hotkeys.py:56) giữ các `HotkeyBinding`: `pause`/`skip`/`quit`/`refocus`/`panic`. Mặc định (config.py:19-23): **F8/F9/F10/F6/Ctrl+Alt+Backspace**.
- `PlaybackControls.poll()` (hotkeys.py:77) dùng `is_hotkey_down()` → `is_virtual_key_down()` (Win32 `GetAsyncKeyState`, **system-wide**, đúng kể cả khi game focus), có **edge-detection** (`_was_down`), trả về 1 action/lần gọi theo thứ tự ưu tiên.
- Engine `PlaybackEngine` nhận `controls=self.controls` (app.py:1251) và **renderer dùng chung** `renderer` (app.py:1252 + card.start_playback renderer=renderer, app.py:1282 → cùng 1 `SnapshotRenderer`). Engine poll controls trong vòng lặp → `_handle_commands()` (engine.py:411) xử lý pause/skip/quit/refocus/panic.
- **F2 hiện KHÔNG nằm trong `PlaybackControls`.** Nó chỉ được bắt ở `SkyPickerApp.handle_playback_card_key()` (app.py:1073-1075) khi `mode=="playing"` → `card.toggle_debug()`. Đây là **Textual key event** → khi Sky giữ focus, terminal không nhận phím → F2 chết (DEFECT D1, đã ghi memory).
- Card debug: `PlaybackCard.debug_mode` (cờ cục bộ), `toggle_debug()` (playback_app.py:346), khởi tạo `debug_mode=self.verbose_hud` (app.py:1289 — fix B "debug mở sẵn").

### 1.3 Render thẻ (sau Bước 2.5)
- `PlaybackCard` là `Static` có `render()->Text`, tái dùng `ansi_gradient_box`/`ansi_box`; `_poll()` (playback_app.py) chạy `set_interval(0.1)` (10Hz) đọc `renderer.get_snapshot()` rồi `refresh()`. **Đây là điểm cắm lý tưởng để poll F2 global ở phía UI** (xem §4).

---

## 2. Bất biến (vi phạm = fail review)

1. **KHÔNG đụng** real-time dispatch / scheduler / GC-pause / vòng gửi phím nóng. Cụ thể: **không sửa `_handle_commands`, không sửa vòng lặp engine, không sửa `runtime_dispatch`** (xem §4 chọn phương án UI-poll chính là để tránh việc này).
2. Giữ nguyên `quiesce()` + guard `_picker_cleanup_failed` + auto-focus pre-playback (Bước 2.5) — không chạm ngữ nghĩa.
3. Giữ tương thích CLI `--song`/non-TTY (console `ProgressRenderer` + `PlaybackScreen` cũ) — không xoá, không đổi.
4. Tái dùng tối đa: `is_hotkey_down`/`HotkeyBinding`/`parse_hotkey` (đọc & gọi), `SnapshotRenderer`, state machine Bước 2.5.
5. Coding standards (AGENTS.md): type hints, frozen dataclass khi hợp lý, không global mới, có test, `uv run`.
6. KHÔNG regress: `tests/test_textual_picker.py` + `tests/test_textual_playback.py` xanh.

**Phạm vi file cho phép chạm:** `src/sky_music/ui/textual_app/app.py`, `src/sky_music/ui/textual_app/playback_app.py`, `src/sky_music/ui/textual_app/theme_css.py` (chỉ CSS layout nếu cần), `tests/test_textual_playback.py`. **Tuỳ chọn** (chỉ nếu chọn cấu hình hoá F2): `src/sky_music/infrastructure/hotkeys.py` (thêm field) — xem §4.3.

---

## 3. Vấn đề 1 — Dock thẻ play xuống đáy, `#songs` co lại

### 3.1 Thay đổi CSS (app.py:187-193)
```css
#playback-card {
    dock: bottom;       /* ghim đáy #root; reserve chiều cao của thẻ */
    width: 100%;
    height: auto;       /* đo theo nội dung box → luôn flush đáy, không thừa dòng trống */
    padding: 0;
    background: transparent;   /* gộp luôn Vấn đề 3 (§5) */
}
```
- `dock: bottom` + `height: auto`: Textual ≥8.2.7 hỗ trợ docked auto-height. Thẻ chiếm đúng số dòng nội dung ở đáy; các sibling không-dock (`#appbar`, `#search`, `#songs 1fr`) chia phần còn lại phía trên → **`#songs` tự co**.
- Khi `display:none` (mode picker), thẻ không tham gia layout → `#detail`/footer flow bình thường. `_restore_picker_after_playback` không cần đổi.

### 3.2 Rủi ro & xử lý
- **Nếu bản Textual hiện tại dock+auto-height bị lỗi** (thẻ cao 0 hoặc tràn): fallback đặt `height` cố định đủ chứa mode cao nhất (debug ~13 dòng + 2 viền = ~15) → `height: 15;` kèm `dock: bottom`. Nhược điểm: mode ngắn để lại khoảng trống dưới box. Ưu tiên thử `height: auto` trước; chỉ fallback khi test §3.3 thất bại.
- **`_box_width()` fallback 72** (playback_app.py): khi `self.size.width` chưa biết ở frame đầu, trả 72 → nếu terminal hẹp hơn 72, box có thể wrap (tăng chiều cao). Đảm bảo render đo lại sau layout: `_box_width` đã `min(size.width,100)` nên sau mount là đúng; **thêm guard** trả về `min(72, <giá trị an toàn>)` không vượt `size.width` khi đã biết. (Kiểm bằng test terminal hẹp §3.3.)

### 3.3 Dọn CSS chết (tuỳ chọn, nên làm)
Sau Bước 2.5 thẻ không còn widget con `#song-name`/`#progress-bar`/`#time-info`/`#status-info`/`#countdown-timer`/`#warning-info`/`#hotkeys-info`. Các rule CSS này (app.py:194-223) **đã chết** (không match widget nào trong `SkyPickerApp`). Xoá để sạch — KHÔNG xoá các rule cùng tên trong `playback_app.py` (chúng thuộc `PlaybackScreen`/`CountdownScreen` của luồng `--song`).

### 3.4 Test (Pilot geometry — `tests/test_textual_playback.py`)
Thêm `test_playback_card_docked_at_bottom`:
```python
async with app.run_test(size=(100, 30)) as pilot:
    await pilot.pause()
    songs_h_before = app.query_one("#songs").region.height
    await pilot.press("enter")          # vào playing (mock engine sleep dài, dry-run)
    await pilot.pause(0.2)
    assert app.playback_mode == "playing"
    card = app.query_one("#playback-card")
    songs = app.query_one("#songs")
    screen = app.screen
    # 1) Hiển thị trọn — không bị clip đáy:
    assert card.region.bottom <= screen.region.bottom
    # 2) Ghim đáy — mép dưới thẻ sát đáy vùng nội dung #root (padding bottom = 1):
    assert card.region.bottom == screen.region.bottom - 1
    # 3) #songs co lại nhường chỗ (thấp hơn trước) và nằm TRÊN thẻ (không chồng):
    assert songs.region.bottom <= card.region.top
    assert songs.region.height < songs_h_before
```
Thêm biến thể terminal hẹp `size=(60, 24)` để bắt lỗi wrap của `_box_width` (assert card không clip). Dùng `mock_prepare_playback` + `MockPlaybackEngine.play` sleep dài (mẫu sẵn trong file) để giữ mode playing trong lúc assert; kết `await pilot.press("f9")` để thoát sạch.

---

## 4. Vấn đề 2 — F2 debug toggle thành GLOBAL (đúng khi game focus)

> **Nguyên tắc:** không đụng engine/dispatch (Bất biến §2.1). Poll F2 ở **phía UI**, dùng chính Win32 `is_virtual_key_down` (system-wide như F8/F9).

### 4.1 Phương án CHỌN — poll trong vòng `_poll` 10Hz của thẻ (Option A, khuyến nghị)
`PlaybackCard._poll()` đã chạy `set_interval(0.1)` lúc playing. Thêm vào đó việc **edge-detect F2 global** rồi tự toggle debug. Cơ chế giống F8/F9 (GetAsyncKeyState toàn cục) nhưng **không chạm engine/dispatch**.

**playback_app.py:**
```python
from sky_music.infrastructure.hotkeys import is_hotkey_down, parse_hotkey  # đọc & gọi, không sửa

# trong __init__:
self._debug_hotkey = None        # gán ở start_playback
self._debug_was_down = False

# trong start_playback(...):  (sau khi set debug_mode)
controls = getattr(self.app, "controls", None)
self._debug_hotkey = getattr(controls, "toggle_debug", None) or parse_hotkey("f2")
self._debug_was_down = False

# trong _poll(self):
self._poll_debug_hotkey()
snap = self.renderer.get_snapshot()
...

def _poll_debug_hotkey(self) -> None:
    hk = self._debug_hotkey
    if hk is None:
        return
    down = is_hotkey_down(hk)        # Win32 GetAsyncKeyState — toàn cục
    if down and not self._debug_was_down:
        self.toggle_debug()          # đã có; flip debug_mode + refresh()
    self._debug_was_down = down
```
- **Tần suất:** 10Hz đủ cho người bấm/giữ ~100ms; debug toggle không phải timing-critical. (F2 nhấp quá nhanh <100ms có thể trượt — chấp nhận; user giữ nhẹ là bắt.)
- **Phạm vi:** chỉ chạy khi mode playing (interval khởi ở `start_playback`, dừng ở `_safe_finish` qua `_poll_timer.stop()`), đúng như F8/F9 chỉ tác dụng lúc phát.

### 4.2 Gỡ Textual binding F2 (tránh double-toggle)
Trong `SkyPickerApp.handle_playback_card_key` (app.py:1073-1075), **xoá** nhánh:
```python
if key == "f2":
    self.query_one("#playback-card", PlaybackCard).toggle_debug()
    return True
```
Giữ swallow `up/down/enter` ở mode playing. Lý do: F2 giờ chỉ do Win32-poll xử lý; nếu vẫn còn Textual binding thì lúc terminal có focus (vd dry-run) sẽ toggle 2 lần (Textual + Win32) = vô hiệu.

### 4.3 (Tuỳ chọn) Cấu hình hoá phím debug
Để hiển thị đúng phím & cho đổi sau, thêm field vào `PlaybackControls` (hotkeys.py):
```python
toggle_debug: HotkeyBinding = field(default_factory=lambda: parse_hotkey("f2"))
```
(đặt sau `panic`, trước `enabled`/`_was_down`). Khi đó `start_playback` lấy `controls.toggle_debug`. **KHÔNG bắt buộc** cho lần này — nếu bỏ qua, card fallback `parse_hotkey("f2")`. Nếu làm: KHÔNG thêm vào `poll()` (debug do UI poll, không qua engine command), chỉ là nguồn binding + để hàng hotkey hiển thị đúng. Cập nhật hàng hotkey trong `_playing_body` dùng `self._debug_hotkey.display` thay chuỗi "F2" cứng (nhỏ, đẹp).

### 4.4 Test (`tests/test_textual_playback.py`)
1. **`test_f2_debug_toggle_is_global` (unit qua mock Win32):**
   ```python
   import sky_music.ui.textual_app.playback_app as pb
   # vào playing, debug_mode=False
   state = {"down": False}
   monkeypatch.setattr(pb, "is_hotkey_down", lambda hk: state["down"])
   card = app.query_one("#playback-card", PlaybackCard)
   assert card.debug_mode is False
   state["down"] = True;  card._poll(); assert card.debug_mode is True   # cạnh lên → toggle
   card._poll();          assert card.debug_mode is True                 # giữ → không lặp
   state["down"] = False; card._poll(); assert card.debug_mode is True   # nhả → giữ nguyên
   state["down"] = True;  card._poll(); assert card.debug_mode is False  # bấm lần 2 → tắt
   ```
   (mock `is_hotkey_down` ở reference trong `playback_app`, KHÔNG gọi Win32 thật.)
2. **`test_textual_f2_no_longer_toggles` (regression binding):** vào playing, `await pilot.press("f2")` → `card.debug_mode` **không đổi** (binding Textual đã gỡ; Pilot không bắn Win32).
3. **Cập nhật `test_playback_screen_toggle_debug` cũ:** nó đang `pilot.press("f2")` để toggle — giờ phải đổi sang **lái qua `is_hotkey_down` mock + `card._poll()`** (như test 1), vì F2 không còn là Textual key. Giữ tinh thần "verbose_hud→debug mở sẵn" trong `test_playback_screen_debug_mode_initial_state`.

> **Lưu ý nghiệm thu:** §4 KHÔNG được chạm `orchestration/`. Nếu coder chọn Option B (route "debug" qua engine `_handle_commands` cho "giống F8/F9 thật") thì **vi phạm Bất biến §2.1** trừ khi chủ dự án chấp thuận mở rộng phạm vi — mặc định **dùng Option A**.

---

## 5. Vấn đề 3 — Bỏ nền thẻ play (đồng nhất các thẻ khác)

Đã gộp vào CSS §3.1: `background: transparent;` (thay `#080e1c`). `#songs`/`#detail` đều transparent → thẻ play hoà nền terminal như các thẻ khác. Box gradient chỉ vẽ ký tự foreground, không phụ thuộc nền nên không vỡ.

### 5.1 Test
- **Parity:** `assert app.query_one("#playback-card").styles.background == app.query_one("#songs").styles.background` (cả hai transparent).
- **(Tuỳ chọn) snapshot:** dùng `pytest-textual-snapshot` (`snap_compare`) chụp màn playing để bắt regression thị giác viền gradient + nền trong suốt. Sinh baseline 1 lần, commit SVG.

---

## 6. Cách chạy test
```
uv run python -m pytest tests/test_textual_playback.py -q
uv run python -m pytest tests/test_textual_picker.py -q      # không regress
uv run python -m pytest -q                                   # full: chỉ fail pre-existing 98>=100 (dữ liệu workspace, ngoài diff)
uvx ruff check src/sky_music/ui/textual_app tests/test_textual_playback.py
```
(`ruff`/`pyright` không có trong venv; dùng `uvx ruff`. `uvx pyright` nếu cần.)

---

## 7. Cổng kiểm duyệt (reviewer chạy trước khi đóng dấu)
- [ ] `git diff --name-only`: chỉ các file trong phạm vi §2 (+ `hotkeys.py` nếu chọn §4.3). **KHÔNG có** `orchestration/`, `infrastructure/` (trừ hotkeys field tuỳ chọn), `platform/`, `domain/`, `config.py`, `hud.py`.
- [ ] **Vấn đề 1:** test dock xanh — thẻ `card.region.bottom <= screen.region.bottom` (không clip) **và** sát đáy **và** `#songs` co lại (`region.height` giảm, không chồng thẻ). Thử cả terminal hẹp.
- [ ] **Vấn đề 2:** test global-toggle xanh (mock `is_hotkey_down`, edge-detect đúng 1 lần/nhấn); test Pilot `press("f2")` KHÔNG toggle (binding đã gỡ); **KHÔNG đụng engine/dispatch** (đọc diff xác nhận).
- [ ] **Vấn đề 3:** nền thẻ == nền `#songs` (transparent); box gradient còn nguyên.
- [ ] `quiesce()`+guard+auto-focus (Bước 2.5) giữ nguyên; in-place state machine không đổi ngữ nghĩa.
- [ ] Full suite không tăng fail; `uvx ruff` sạch; picker không regress.
- [ ] **Smoke phát THẬT in-game (chủ dự án):** ấn play → thẻ play **ghim đáy, không khuất**, `#songs` co lại, **nền trong suốt như các thẻ khác** → đang phát (Sky focus) bấm **F2 vẫn bật/tắt được debug panel** (đây là phần Pilot KHÔNG thay được) → F9 về picker (UI khôi phục), F10 thoát.

## 8. Bàn giao
Có thể tách: (2.6a) layout dock + bỏ nền (CSS thuần + test geometry/parity) — nhỏ, smoke nhanh; (2.6b) F2 global (UI-poll + gỡ binding + test mock). Mỗi PR kèm mô tả + pytest + smoke + xác nhận bất biến §2. Reviewer chạy §7. Smoke thật của chủ dự án trước khi đóng dấu.

> Memory liên quan: `live-dashboard-decision`, `realtime-process-isolation`, `player-dispatch-proven-metronomic`. Bài học Bước 3: **tính năng hotkey lúc phát BẮT BUỘC smoke tay in-game** (Pilot/mock không phủ được focus).
