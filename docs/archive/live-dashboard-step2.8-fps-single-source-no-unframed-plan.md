# Kế hoạch: fps một-nguồn-duy-nhất + DIỆT "unframed" (Bước 2.8) — bản thực thi

> **Đối tượng đọc:** kỹ sư **thực thi**. Tài liệu tự chứa.
> **Reviewer:** tác nhân giám sát + chủ dự án (nghiệm thu §6).
> **Tiền đề:** Bước 2.7 Part B đi **sai hướng** — nó hợp nhất hiển thị nhưng chốt vào giá trị `unframed` (do config `game_fps=0`). Chủ dự án đính chính yêu cầu (xem §0).
> **Ngày:** 2026-06-08.

---

## 0. Yêu cầu (chủ dự án chốt — nguyên văn ý)

- **Dự án KHÔNG có khái niệm "unframed"/"auto"/`fps=0`.** Trạng thái `fps=0` **không bao giờ được phép xảy ra**.
- **Lần đầu mở app: default = 60 fps.**
- Người dùng **chỉ chọn được các giá trị fps có trong menu** (30/60/90/120/144/165/240) — KHÔNG có lựa chọn "Auto".
- **Toàn bộ config phải đọc & dùng từ MỘT nguồn duy nhất**, tránh các thành phần lấy từ nhiều nguồn gây lệch/bug.

> Bước 2.7 đã làm đúng phần **CONFIG_PATH tuyệt đối/CWD-independent** (giữ nguyên). Bước 2.8 sửa phần **fps**: diệt unframed, default 60, một resolver duy nhất; và đồng bộ hiển thị.

---

## 1. "unframed" đang bị nướng vào đâu (đã khảo sát — sửa hết các điểm này)

| Nơi | Hiện trạng | Cần đổi |
|---|---|---|
| `ui/picker.py:43-51` `FPS_OPTIONS` | phần tử đầu `(None, "Auto (No forced sync)")` | **XOÁ dòng Auto**. Chỉ giữ 30/60/90/120/144/165/240. |
| `ui/textual_app/app.py:762-773` `action_open_fps`/`_apply_fps` | dựng option "Auto" từ `value is None`; `self.fps = None if value=="auto" else int` | Bỏ nhánh Auto; `self.fps` luôn là int hợp lệ từ menu. |
| `config.py:208` `normalize_fps_value` | *"0 means frame-aware scaling is disabled"*; `None/<=0 → 0` | `None/<=0 → DEFAULT_FPS (60)`. **Không bao giờ trả 0.** |
| `config.py` `AppConfig.game_fps` | default `60` (đúng) nhưng có thể bị persist `0` | Đảm bảo **load/migrate**: `game_fps<=0 → 60` ngay khi đọc; không giữ 0 trong AppConfig. |
| `main.py:1489/1521` `resolved_fps = args.fps if cli else (game_fps if >0 else None)` | nhánh `else None` → unframed | Bỏ `None`: luôn ra int (qua resolver §2). |
| `domain/session_context.py` `fps: int|None`, `with_fps` chuẩn hoá `<=0→None`; `:146 selected_fps = self.fps if not None else 60` | cho phép `None` | `session.fps` luôn concrete (default 60); KHÔNG None. (Xem §2 — đi qua resolver.) |
| `domain/scheduler_types.py:161-201` `FrameTimingPolicy.build(fps=None)` → `fps=0, frame_us=0` | nhánh unframed | **KHÔNG sửa công thức** (domain timing nhạy). Chỉ **đảm bảo không bao giờ truyền None/0 vào** (resolver chặn ở thượng nguồn). Nhánh None thành dead-code phòng thủ — để yên. |
| `ui/textual_app/app.py:582` header chip | `"unframed" if self.fps is None else ...` (2.7 đổi từ "auto") | `self.fps` không còn None → **luôn `{fps}fps`**. Bỏ chuỗi "unframed". |
| `ui/textual_app/playback_app.py` Timing line | `unframed` khi `fps==0` (2.7) | `fps` không còn 0 → **luôn `{fps}fps ({frame_us}us)`**. Bỏ nhánh "unframed". |
| `orchestration/calibration.py:129` `fps=inp.fps if inp.fps>0 else None` | có thể ra None | Cho ra default 60 thay vì None (hoặc đi qua resolver). |

---

## 2. Kiến trúc đích — MỘT nguồn fps duy nhất

### 2.1 Một resolver
Thêm **một** điểm chuẩn hoá (vd trong `config.py`):
```python
DEFAULT_GAME_FPS = 60
VALID_FPS = (30, 60, 90, 120, 144, 165, 240)

def resolve_game_fps(value: int | None) -> int:
    """Nguồn DUY NHẤT trả fps hiệu lực — không bao giờ 0/None/unframed."""
    if value is None or int(value) <= 0:
        return DEFAULT_GAME_FPS
    return int(value)   # (tuỳ chọn: snap về VALID_FPS gần nhất nếu cần)
```
- **Mọi consumer fps đọc qua đây** (hoặc qua `cfg.game_fps` đã được resolve sẵn): header chip, `session.fps`, `resolve_effective_policy`→`build`, engine `fps=`, Timing line, calibration, picker default. KHÔNG component nào tự suy `else None`/`>0 else 0` nữa.
- **Chốt single-source:** `AppConfig.game_fps` sau `load_config()` LUÔN là int hợp lệ (chạy `resolve_game_fps` lúc load). Đó là nguồn sự thật; `session`/policy/UI dẫn xuất từ nó, không đọc đường khác.

### 2.2 Migrate config cũ
`config.json` hiện trường (local, gitignored) có `game_fps: 0`. Khi load: `resolve_game_fps(0) → 60`. Lần save kế tiếp ghi 60 (hết 0 vĩnh viễn). Không cần script riêng — load-time resolve là đủ.

### 2.3 Picker chỉ giá trị hợp lệ
- `FPS_OPTIONS` bỏ Auto. `action_open_fps` highlight giá trị hiện tại; `_apply_fps` luôn `int(value)` → `persist_default_fps` (giờ không bao giờ ra 0).

---

## 3. ⚠️ Tác động timing (đọc kỹ — vùng nhạy)
- Với config hiện tại của chủ dự án (`game_fps=0`), sau 2.8 fps **= 60** → `FrameTimingPolicy.build(fps=60)` → `frame_us=16667` + hold **lượng tử hoá theo frame** (khác hold thô unframed) → **độ dài giữ phím & frame model THAY ĐỔI thật**. Đây là hành vi ĐÚNG theo chủ dự án (0 lẽ ra không tồn tại), nhưng:
- **KHÔNG sửa công thức** trong `scheduler_types.build`/`domain/`/dispatch/engine hot path. Chỉ chặn không cho 0/None chảy vào.
- **Bắt buộc smoke feel nhạc thật** sau khi sửa (memory `timing-one-frame-standard`, `realtime-process-isolation`, `player-dispatch-proven-metronomic`).

---

## 4. Bất biến
1. KHÔNG đổi công thức timing (`scheduler_types`, `domain/`), KHÔNG đụng dispatch/scheduler/GC/engine hot path. Chỉ chặn input fps ở thượng nguồn + UI + config resolve.
2. CONFIG_PATH single-source (2.7) giữ nguyên. (Lưu ý build: `Path(__file__).parents[2]` có thể sai trong PyInstaller — ghi chú, xử lý nếu đụng build.)
3. Một resolver fps duy nhất; KHÔNG thêm đường suy fps song song.
4. KHÔNG regress: `tests/test_textual_picker.py`, `tests/test_textual_playback.py`, `tests/test_session_context.py`, `tests/test_calibration.py` xanh; `--song` console path nguyên vẹn.
5. Phạm vi file: `config.py`, `ui/picker.py`, `ui/textual_app/app.py`, `ui/textual_app/playback_app.py`, `domain/session_context.py` (chỉ đường resolve, KHÔNG công thức), `orchestration/calibration.py` (chỉ default), tests. Liệt kê rõ trong PR.

---

## 5. Test bắt buộc
1. **`test_resolve_game_fps_never_zero`:** `resolve_game_fps(0)==60`, `(None)==60`, `(-5)==60`, `(144)==144`. Không bao giờ trả 0/None.
2. **`test_config_load_migrates_zero_fps`:** config.json có `game_fps=0` → `load_config().game_fps == 60`.
3. **`test_fps_menu_has_no_auto`:** `None not in [v for v,_ in FPS_OPTIONS]`; mọi value là int dương.
4. **`test_no_unframed_string_anywhere`:** header chip + Timing line với fps mặc định → chứa `"60fps"`, **KHÔNG** chứa `"unframed"`/`"auto"`/`"N/A"`.
5. **`test_header_fps_matches_policy_fps`** (giữ/cập nhật từ 2.7): header `self.fps` == `card.active_policy.fps` == giá trị menu; cả hai concrete.
6. **`test_config_path_is_cwd_independent`** (giữ từ 2.7).
7. Cập nhật `test_timing_line_no_bare_na` (2.7): bỏ case unframed; thay bằng "luôn `{fps}fps`".

---

## 6. Cổng kiểm duyệt
- [ ] `git diff --name-only` trong phạm vi §4.5; KHÔNG đụng công thức timing/dispatch/engine hot path.
- [ ] **Single-source:** chỉ MỘT resolver fps; `AppConfig.game_fps` sau load luôn concrete (≥ default); mọi consumer (header/session/policy/engine/Timing/picker/calibration) đọc cùng nguồn — đọc diff xác nhận không còn `else None`/`>0 else 0` rải rác.
- [ ] **Diệt unframed:** `FPS_OPTIONS` không còn Auto; `resolve_game_fps` & `normalize_fps_value` không bao giờ ra 0; config 0 → 60 khi load; KHÔNG còn chuỗi "unframed"/"auto"/"N/A" ở header/Timing.
- [ ] Default lần đầu = 60 (config trống → game_fps 60).
- [ ] Test §5 xanh; full suite không tăng fail; `uvx ruff` sạch; picker/session/calibration không regress.
- [ ] **Smoke thật in-game (chủ dự án):** mở app → fps hiển thị **60** ở header (và Timing khi debug) — **không còn "unframed"**; đổi fps trong menu (chỉ thấy số, không có Auto) → header/Timing đổi theo, **khớp nhau**; **nghe lại feel nhạc** (vì timing đổi từ unframed→60fps); F2/F9/F10 + dock đáy vẫn OK.

> Memory: `live-dashboard-decision`, `timing-one-frame-standard`, `realtime-process-isolation`, `player-dispatch-proven-metronomic`.
