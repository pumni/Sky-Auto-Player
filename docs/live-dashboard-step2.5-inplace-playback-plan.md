# Kế hoạch: Playback In-Place + Auto-focus (Bước 2.5) — bản thực thi

> **Đối tượng đọc:** kỹ sư **thực thi**. Tài liệu tự chứa.
> **Reviewer:** tác nhân giám sát (nghiệm thu theo "Cổng" cuối). Không merge khi chưa qua cổng.
> **Tiền đề:** Bước 1+2 đóng dấu; Bước 3 (Debug panel) code xong (chưa smoke). Bước 2.5 **thay cách trình bày playback** (push-screen → in-place) và **gộp Debug panel vào thẻ play mới**. Tái dùng tối đa logic đã có.
> **Ngày:** 2026-06-07.

---

## 0. TL;DR — 2 vấn đề từ smoke thật

1. **Mất auto-focus Sky (regression).** Luồng cũ gọi `ensure_sky_ready()`→`inputs.focusWindow()` (picker_helpers.py:96-105) tự đưa Sky lên foreground khi phát. Luồng unified (`execute_playback_plan`) KHÔNG gọi → user phải tự alt-tab. Engine chỉ `focus()` khi lệnh refocus (F6), không focus chủ động lúc bắt đầu.
2. **Push-screen nuốt UI picker.** `push_screen(PlaybackScreen)` + CountdownScreen + risk OptionModal thay toàn bộ layout. Yêu cầu: giữ **header + table** (khóa lại), biến **vùng detail+footer thành "thẻ play"** giữ thẩm mỹ cũ (gradient border). Risk + countdown + playback + debug **đều render in-place** trong thẻ play. Không còn screen/modal riêng cho các pha này.

**Chốt thiết kế (với chủ dự án):** tất cả in-place; gộp Debug vào redesign; trong lúc phát **khóa search + điều hướng table** (chỉ hotkey phát hoạt động, vì Sky giữ focus).

---

## 1. Bất biến (vi phạm = fail review ngay)
1. **KHÔNG sửa** `domain/`, `orchestration/` (engine/telemetry/runtime_dispatch), `infrastructure/`, `platform/`, `ui/hud.py`, `config.py`, `layouts.py`. Được **đọc & gọi** (gồm `infrastructure.focus`, `platform.win32.inputs.focusWindow`, `PlaybackEngine`), không **đổi**.
2. KHÔNG đụng dispatch real-time/scheduler/GC-pause; `PlaybackEngine` params giữ nguyên cách lấy động (RUNTIME_STATE).
3. **Giữ bảo đảm cách ly timing:** `quiesce()` + guard `_picker_cleanup_failed` trước khi phát (đã có ở `execute_playback_plan`) phải được giữ NGUYÊN ngữ nghĩa trong form in-place.
4. Giữ tương thích CLI: `--song`/non-TTY giữ `play_selected_song` console.
5. KHÔNG regress picker: `tests/test_textual_picker.py` xanh.
6. Tái dùng (KHÔNG viết lại): `SnapshotRenderer`/`debug_stats`/`PlaybackSnapshot`/`DebugStats`, logic dựng engine + params trong `execute_playback_plan`, `quiesce/rearm`, `_get_main_module`, `prepare_playback/rebuild_with`, `risk decision` mapping.
7. Coding standards (AGENTS.md): type hints, frozen dataclass, không global mới, test, `uv run`.

---

## 2. Bối cảnh — layout picker hiện tại (`SkyPickerApp.compose`, app.py:274-292)
```
Container#root
├── GradientHeader#appbar      # giữ
├── SearchInput#search         # giữ, KHÓA lúc phát
├── SongTable#songs            # giữ, KHÓA điều hướng lúc phát
├── DetailPanel#detail   ─┐
└── CustomFooter          ┘ → THAY bằng "thẻ play" lúc phát (khôi phục khi xong)
```
Playback hiện ở `PlaybackScreen` (playback_app.py) + `CountdownScreen`; risk ở `OptionModal` (app.py:1050-1114). Bước 2.5 chuyển các pha này thành **mode in-place** của `SkyPickerApp`.

---

## 3. Kiến trúc đích — playback là MODE in-place của picker

### 3.1 State machine trong `SkyPickerApp`
`mode ∈ {picker, risk, countdown, playing}`. Thẻ play = một widget (vd `PlaybackCard(Container)`) chiếm vùng detail+footer; ẩn `#detail`/footer khi không ở mode picker.

Luồng (thay `start_playback_workflow`/`execute_playback_plan` push-screen):
1. **confirm** (unified) → `prepare_playback`. Lỗi → hiện thông báo lỗi **trong thẻ play** + "Esc về picker" (không modal).
2. `risk != low` → mode `risk`: thẻ play hiện cảnh báo + 5 lựa chọn (proceed/switch_profile/scale_tempo/dry_run/cancel) **trong thẻ** (list focusable; **terminal còn focus ở pha này** nên chọn được bằng ↑↓/Enter hoặc phím số). Map quyết định → `rebuild_with` y như hiện tại.
3. proceed → **`quiesce()` + guard `_picker_cleanup_failed`** (giữ nguyên; fail → lỗi in-place + `rearm`, KHÔNG phát).
4. **Auto-focus Sky** (Vấn đề 1): gọi `Win32SkyFocusGuard().focus()` (hoặc `inputs.focusWindow()`) khi vào countdown — đưa Sky lên foreground tự động (chỉ khi không dry-run).
5. mode `countdown`: đếm ngược **trong thẻ play** (thay CountdownScreen).
6. mode `playing`: thẻ play chạy `engine.play()` ở `@work(thread=True)` + `set_interval(0.1)` poll (cơ chế Bước 1/3). Hiển thị: tên bài, progress bar, time/ETA, status, violations, **Debug panel khi `verbose_hud`** (tái dùng `debug_stats`), hotkey hints. **Khóa** `#search` + điều hướng `#songs`.
7. result finish/skip → mode `picker`: khôi phục `#detail`/footer, mở khóa, `rearm()`. `quit` → `app.exit(0)`.

### 3.2 Thẻ play — thẩm mỹ
- Giữ **gradient border** như UI cũ. Tái dùng token theme (`t.gradient`/`GradientHeader` pattern) hoặc CSS border gradient; mục tiêu nhìn như HUD console cũ (`ansi_gradient_box`).
- Layout thẻ giống PlaybackScreen hiện tại (song/progress/time/status/warning/debug-panel/hotkeys) nhưng nhúng in-place, KHÔNG full-screen.

### 3.3 Khóa tương tác lúc phát (mode playing)
- `SearchInput#search`: `disabled=True` (hoặc bỏ focus + chặn input).
- `SongTable#songs`: vô hiệu điều hướng (disable cursor up/down/enter) — table vẫn hiển thị (read-only).
- Lý do: Sky giữ focus lúc phát nên Textual không nhận phím; nhưng khi focus_lost/pause terminal có thể nhận → khóa để không đổi bài giữa chừng.

### 3.4 Auto-focus (Vấn đề 1) — chi tiết
- Dùng `from sky_music.infrastructure.focus import Win32SkyFocusGuard` rồi `.focus()` (nội bộ gọi `inputs.focusWindow()`), hoặc gọi thẳng `inputs.focusWindow()`. **Chỉ gọi**, không sửa hai module này.
- Gọi 1 lần khi chuyển sang countdown/playing (không dry-run). Sau đó terminal mất focus là **đúng mong đợi** (game cần focus) — đây cũng là lý do F2 không toggle được lúc phát (xem Bước 3 fix B: `verbose_hud`→debug mở sẵn).

---

## 4. An toàn timing (giữ nguyên đảm bảo Bước 2)
| Nguy cơ | Biện pháp |
|---|---|
| Worker picker chạy lúc phát | `quiesce()` + guard `_picker_cleanup_failed` trước khi dựng engine (giữ nguyên; in-place không đổi ngữ nghĩa). `rearm()` khi về picker. |
| UI làm bẩn dispatch | Engine→UI snapshot + poll 10Hz; `call_from_thread` chỉ lúc exit; Debug sort chỉ ở poll khi `verbose_hud`. |
| Auto-focus | Chỉ **gọi** `focusWindow()` (1 lần, pre-playback), không đụng infrastructure/platform. |

> Memory: `realtime-process-isolation`, `player-dispatch-proven-metronomic`, `live-dashboard-decision`.

---

## 5. Di trú & dọn dẹp
- `PlaybackScreen`/`CountdownScreen` (Screen) → chuyển logic vào `PlaybackCard` widget + state machine SkyPickerApp. Có thể giữ `run_playback_textual`/`PlaybackApp` cho luồng `--song` console (Bước 1) — KHÔNG xoá nếu còn dùng; xác nhận call sites trước khi gỡ.
- `OptionModal` risk → list in-place trong thẻ. `InfoModal` lỗi → thông báo in-place.
- `_transitioning_to_playback` guard giữ (chống double-confirm).

---

## 6. Test (bắt buộc)
- Pilot: confirm → (risk) thẻ play hiện cảnh báo + chọn proceed → countdown in-place → playing (engine giả) → finish → **về mode picker, `#detail`/footer khôi phục, search/table mở khóa lại**.
- `quiesce` fail (mock) → lỗi in-place + rearm, KHÔNG vào playing.
- Khóa: ở mode playing, `#search` disabled + điều hướng table vô hiệu (Pilot gửi ↓ không đổi cursor).
- Debug: `verbose_hud=True` → thẻ play hiện debug panel từ đầu; số liệu từ `debug_stats`.
- Auto-focus: trừu tượng hoá qua FocusGuard để test gọi `.focus()` đúng 1 lần pre-playback (inject Noop/mock; KHÔNG gọi Win32 thật trong test).
- `tests/test_textual_picker.py` vẫn xanh.

---

## 7. Cổng kiểm duyệt
- [ ] `git diff --name-only`: chỉ `ui/textual_app/*` (+ main.py nếu cần) + test. **Bất biến §1 rỗng** (engine/telemetry/infra/platform/hud/config/domain).
- [ ] Auto-focus: chỉ **gọi** `focus()`/`focusWindow()` (đọc code xác nhận không sửa infra/platform); gọi pre-playback, không khi dry-run, không trong pha risk (terminal cần focus để chọn).
- [ ] **Cách ly timing:** `quiesce()` + guard giữ nguyên trước khi dựng engine; không worker picker sống lúc phát.
- [ ] In-place: header+table còn hiển thị; detail+footer→thẻ play (gradient); KHÔNG push screen/modal cho playback/countdown/risk. Khôi phục đúng khi về picker.
- [ ] Khóa search+điều hướng lúc phát; mở lại khi về picker.
- [ ] Debug panel hoạt động trong thẻ (verbose_hud mở sẵn — Bước 3 fix B).
- [ ] Test xanh; picker không regress; ruff sạch; full suite không tăng fail.
- [ ] **Smoke phát THẬT in-game (chủ dự án):** ấn play → **Sky tự lên foreground** (không phải alt-tab tay) → thẻ play hiện in-place (header+table vẫn thấy) → progress/debug chạy, nhạc không nấc → F9 về picker (UI khôi phục) → F10 thoát. (Pilot KHÔNG thay được smoke focus thật.)

## 8. Quy trình bàn giao
Có thể tách 2 PR: (2.5a) **auto-focus fix** (nhỏ, độc lập, smoke nhanh) trước; (2.5b) **in-place redesign + gộp Debug**. Mỗi PR kèm mô tả + pytest + smoke + xác nhận bất biến. Reviewer chạy §7 + `git diff --name-only`. Smoke thật của chủ dự án trước khi đóng dấu.
