# Kế hoạch đại tu UI Picker → Textual (bản thực thi)

> **Đối tượng đọc:** AI/kỹ sư **thực thi** refactor. Tài liệu này tự chứa — không cần
> ngữ cảnh hội thoại trước đó. Mọi quyết định chiến lược đã chốt.
> **Reviewer:** một tác nhân riêng kiểm duyệt theo "Cổng kiểm duyệt" ở cuối mỗi phase.
> Không merge/đổi mặc định khi chưa qua cổng.

---

## 0. TL;DR

Thay **UI chọn bài** (picker) hiện tại (prompt_toolkit) bằng app **Textual** (trên Rich),
thêm **rapidfuzz** cho fuzzy search. Triển khai **theo phase, sau cờ `--ui`**, giữ picker
prompt_toolkit cũ làm fallback cho tới khi đạt parity và ổn định.

**Tuyệt đối không** chạm phần playback (engine, scheduler, timing, SendInput, HUD lúc chơi).
Ranh giới: TUI chạy → user chọn → trả `SongPickerResult` → **thoát** → engine cũ phát y nguyên.

---

## 1. Bất biến (vi phạm = fail review ngay)

1. **KHÔNG sửa** bất kỳ file nào trong:
   - `src/sky_music/domain/` (parser, scheduler, analyzer, validation, session_context, song_repository, domain)
   - `src/sky_music/orchestration/` (engine, telemetry, calibration)
   - `src/sky_music/infrastructure/`, `src/sky_music/platform/`
   - `src/sky_music/ui/hud.py` (HUD lúc playback)
   - `src/sky_music/config.py`, `src/sky_music/layouts.py`
   > Được phép **đọc & gọi** các module này, không được **đổi** chúng.
2. **Giữ nguyên** `src/sky_music/ui/picker_metadata.py` và `src/sky_music/ui/text_render.py`.
   Tái sử dụng, không viết lại pipeline metadata.
3. **Giữ tương thích CLI**: mọi flag hiện có (`--song`, `--list`, `--theme`, `--dry-run`,
   `--timing-profile`, `--tempo-scale`, `--fps`, …) hoạt động y như cũ. Fallback không-TTY giữ nguyên.
4. **Không đổi** chữ ký / hành vi của `play_selected_song`, `prompt_song_selection` trừ chỗ
   *điều hướng backend UI* (mục Phase 1.1).
5. **`SongPickerResult`** là hợp đồng đầu ra duy nhất giữa UI và playback — không đổi shape.
6. Coding standards (AGENTS.md): Python 3.11+ (repo đang pin 3.14), **type hints bắt buộc**,
   ưu tiên `dataclass(frozen=True, slots=True)`, không global mới, có test.
7. Mỗi phase phải **xanh test** + **smoke** trước khi sang phase sau. Không gộp phase.

---

## 2. Quyết định đã chốt

| Hạng mục | Quyết định |
|---|---|
| Framework | **Textual** (trên Rich) |
| Search | **rapidfuzz** (ranked fuzzy) ở Phase 4 |
| Cutover | **Phased, sau cờ `--ui textual\|classic\|auto`**, mặc định `classic` đến khi ổn |
| CSS | **Inline** (`CSS = """..."""` trong App) — KHÔNG file `.tcss` (né lỗi PyInstaller `_MEIPASS`) |
| Metadata pipeline | **Tái dùng** `picker_metadata.py` (2 tầng: raw tức thì + risk lazy + SQLite) |
| Risk compute | Giữ `ProcessPoolExecutor(max_workers=1)` (tránh GIL Windows), submit từ thread worker |

---

## 3. Bối cảnh kiến trúc hiện tại (đọc kỹ trước khi code)

### 3.1 Luồng chọn bài hiện tại
`src/main.py::main()` → vòng lặp → `prompt_song_selection(...)` (main.py:~1086) →
nếu `songs.HAS_PROMPT_TOOLKIT` gọi `songs.choose_song_interactively(...)` (picker.py) →
trả `SongPickerResult | None` → `play_selected_song(...)` phát.

`prompt_song_selection` chữ ký:
```python
def prompt_song_selection(profile="balanced", tempo=1.0, dry_run=False,
                          fps=None, scan_code_mode="physical") -> "SongPickerResult | None"
```

`choose_song_interactively` (entry picker hiện tại, picker.py) chữ ký:
```python
def choose_song_interactively(
    theme_name: str | None = None,
    initial_profile: str = "balanced",
    initial_tempo: float = 1.0,
    initial_fps: int | None = None,
    initial_dry_run: bool = False,
    scan_code_mode: str = "physical",
) -> SongPickerResult | None
```

### 3.2 Hợp đồng đầu ra (KHÔNG đổi)
`src/sky_music/ui/picker.py`:
```python
@dataclass(frozen=True, slots=True)
class SongPickerResult:
    song_path: Path
    action: Literal["play", "dry_run"]
    profile_name: str
    tempo_scale: float
    fps: int | None = None
    verbose_hud: bool | None = None
    telemetry_enabled: bool | None = None
```

### 3.3 Pipeline metadata (TÁI DÙNG — `picker_metadata.py`)
Đã được tầng hoá 2 lớp:
- **Tầng raw** (≈1ms, policy-independent): Time, Notes, Density, gaps, chord size.
- **Tầng risk** (≈5ms, scheduler): Risk, Suggested profile/tempo, polyphony thật.

`SongUiMetadata` (frozen dataclass) có cờ `analyzed: bool` — `False` = mới có raw,
`True` = đã phân tích risk. Render phải gate Risk/Suggested theo `analyzed`.

API cần dùng (tất cả ở `sky_music.ui.picker_metadata`):
```python
peek_cached_song_ui_metadata(path, session=None, cfg=None) -> SongUiMetadata | None  # render-safe, không I/O đĩa
hydrate_and_fill_raw_metadata(paths, session=None, cfg=None) -> int   # cache-worker: SQLite + raw, trả số dòng repaint được
compute_song_ui_metadata_payloads(path_values: list[str], session_payload: dict, cfg=None) -> list[dict]  # CHẠY TRONG ProcessPool
store_computed_song_ui_metadata_payloads(payloads, session, cfg=None) -> int  # nạp kết quả ProcessPool vào cache
session_to_worker_payload(session) -> dict   # picklable cho ProcessPool
warm_persistent_metadata_cache(limit=6000) -> int   # nạp toàn bộ SQLite vào RAM (chạy nền)
clear_metadata_cache(clear_persistent=False) -> None
```
`SongUiMetadata` fields: `path, name, duration_seconds, note_count, max_polyphony,
min_note_gap_ms, min_same_key_gap_ms, risk("low|medium|high|error"), recommended_profile,
recommended_tempo_scale, warnings, average_notes_per_second, peak_notes_per_second_1s,
impossible_repeats, max_chord_size, chords_count, timing_stress_rate, analyzed`.

> **Mẫu điều phối đúng (thay generation/coalesce thủ công cũ):**
> 1. Khi danh sách hiển thị/đổi: 1 thread worker `exclusive` chạy `hydrate_and_fill_raw_metadata(visible_paths)` → repaint.
> 2. Với path còn `peek(...) is None or not .analyzed`: submit `compute_song_ui_metadata_payloads`
>    vào `ProcessPoolExecutor(1)`; khi xong → `store_computed_...` → repaint.
> 3. Dùng `@work(exclusive=True, group=...)` để Textual tự huỷ batch cũ (thay cho `metadata_generation`).

### 3.4 Dữ liệu bài hát & sort (đã đúng — tái dùng)
```python
from sky_music.ui.picker_helpers import get_song_choices  # -> list[Path], đã sort A→Z theo stem chuẩn dấu
from sky_music.ui.picker_theme import remove_accents       # chuẩn hoá dấu để build search key
```
Search key mỗi bài: `remove_accents(path.stem).casefold()`. Tiêu đề hiển thị = `path.stem`.

### 3.5 Cấu hình, theme, options (đọc giá trị, đừng hardcode lại)
```python
from sky_music.config import (load_config, save_config, canonical_profile_name,
    display_profile_name, persist_default_profile, persist_default_tempo,
    persist_default_fps, persist_calibration_defaults, CLI_PROFILE_NAMES)
from sky_music.domain.session_context import PlaybackSessionContext
from sky_music.ui.picker_theme import THEME_PRESETS  # aurora, minimalist, slate, cyberpunk, classic
```
Hằng options lấy từ `picker.py`: `PROFILES_INFO`, `TEMPO_OPTIONS`, `FPS_OPTIONS`, `commands`
(import lại hoặc nhân bản sang module mới — nếu nhân bản phải có test chống lệch).
Calibration: `sky_music.orchestration.calibration.{load_latest_telemetry_summary,
calibration_input_from_summary, calibrate_profile}`.

### 3.6 Render helper dùng chung (cho màn print() còn lại)
`sky_music.ui.text_render`: `cell_width, truncate_cells, pad_cells, fit_cells,
clamp_terminal_width (60..100), ansi_box, strip_ansi, visible_width`. Textual không cần
các hàm này (Rich tự lo), nhưng giữ cho fallback/diagnostics.

---

## 4. Kiến trúc đích (Textual)

```
src/sky_music/ui/textual_app/
├── __init__.py          # export choose_song_interactively_textual(...)
├── app.py               # SkyPickerApp(App): bindings, screen stack, exit→SongPickerResult
├── styles.py            # CSS = """...""" (inline). 5 biến thể theme map từ THEME_PRESETS
├── state.py             # PickerModel: reactive state (query, profile, tempo, fps, dry_run, ...)
├── workers.py           # MetadataCoordinator: thread worker(hydrate/raw) + ProcessPool(risk)
├── widgets/
│   ├── song_table.py    # SongTable(DataTable): cột Title/Time/Notes/Risk/Suggested, virtual scroll
│   ├── detail_panel.py  # DetailPanel(Static): raw stats + risk (gate theo analyzed)
│   └── status_bar.py    # header status (profile/tempo/fps/dry/hud/telem/theme/songs)
└── screens/
    ├── picker.py        # PickerScreen
    ├── command_palette.py  # dùng Textual CommandPalette/SystemCommands cho "/"
    └── modals.py        # ProfileSelect/TempoSelect/FpsSelect/ThemeSelect/Preview/Calibration/Help
```

Entry mới (cùng "ý nghĩa" với bản cũ, trả về cùng kiểu):
```python
# textual_app/__init__.py
def choose_song_interactively_textual(
    theme_name: str | None = None,
    initial_profile: str = "balanced",
    initial_tempo: float = 1.0,
    initial_fps: int | None = None,
    initial_dry_run: bool = False,
    scan_code_mode: str = "physical",
) -> "SongPickerResult | None":
    ...  # app = SkyPickerApp(...); return app.run()
```

Nguyên tắc:
- **State = reactive** → bỏ `update_ui()` dựng 4 control thủ công và máy trạng thái string-literal.
- **Layout = CSS inline** → bỏ toán cell-width/box (Rich lo).
- **Worker** cập nhật UI qua `self.app.call_from_thread(...)`; **không** set reactive trực tiếp từ thread.
- `@work(thread=True, exclusive=True, group="meta")` cho hydrate/raw; ProcessPool cho risk.

---

## 5. Các phase

> Mỗi phase: **một PR/diff riêng**, kèm test + ghi chú smoke. Reviewer chỉ duyệt 1 phase/lần.

### Phase 0 — Nền tảng & CHỨNG MINH ĐÓNG GÓI *(cổng quyết định — rủi ro tập trung ở đây)*
**Mục tiêu:** chứng minh Textual chạy + đóng gói PyInstaller được trên Windows trước khi đầu tư.

**Việc:**
1. `uv add textual rapidfuzz`; `uv add --dev pytest-textual-snapshot`.
2. Xác minh Python compat: `uv run python -c "import textual, rapidfuzz; print('ok')"`.
   - Nếu Textual **không** hỗ trợ Python 3.14 (repo pin `requires-python>=3.14`): **DỪNG**, báo reviewer.
     Phương án: nới `requires-python` hoặc chờ — reviewer quyết.
3. Tạo `src/sky_music/ui/textual_app/app.py` với app **hello tối thiểu**, **CSS inline**, 1 màn
   hiển thị "Sky Player" + thoát bằng `q`. Không file `.tcss`.
4. Thêm `build_app.py` (nếu cần) cờ PyInstaller cho Textual: thử trước **không** thêm gì; nếu
   `--onedir` lỗi thiếu module, thêm `--collect-all textual --collect-all rich` (hoặc hiddenimports).
   Ghi lại chính xác cờ đã dùng vào doc này (mục Phụ lục B).
5. `uv run python -m sky_music.ui.textual_app.app` chạy được trên Windows Terminal.
6. `uv run python src/build_app.py` → chạy `dist/Sky-Player/Sky-Player.exe` → mở app hello → **không** `StylesheetError`.

**Cổng kiểm duyệt P0:**
- [ ] exe build & chạy app Textual hello trên Windows Terminal, không lỗi CSS/_MEIPASS.
- [ ] `import textual, rapidfuzz` OK trên interpreter dự án.
- [ ] Không sửa file thuộc danh sách bất biến (mục 1).
- [ ] Cờ PyInstaller đã dùng được ghi lại ở Phụ lục B.
> **Nếu P0 fail đóng gói → dừng toàn bộ, đánh giá lại lựa chọn Textual.**

---

### Phase 1 — Parity cơ bản sau cờ `--ui`
**Mục tiêu:** `--ui textual` chọn được bài và phát qua engine cũ.

**Việc:**
1. **Cờ `--ui`** trong `src/main.py`:
   - `build_arg_parser`: thêm
     ```python
     disp.add_argument("--ui", choices=["auto","textual","classic"], default="classic",
                       help="song picker backend (default: classic prompt_toolkit)")
     ```
   - `prompt_song_selection`: chọn backend:
     ```python
     # auto = textual nếu stdout.isatty() và phát hiện terminal hỗ trợ; ngược lại classic
     if ui_mode == "textual" or (ui_mode == "auto" and _supports_textual()):
         return songs_textual.choose_song_interactively_textual(...)
     # else: nhánh classic hiện có (giữ nguyên)
     ```
   - `_supports_textual()`: tối thiểu `sys.stdout.isatty()`; có thể kiểm `os.environ.get("WT_SESSION")`
     (Windows Terminal). Mặc định thận trọng → classic.
   - Truyền `ui_mode` từ `args.ui` xuống (qua `configure_from_args`/tham số). Giữ mặc định `classic`.
2. **`SkyPickerApp` + `PickerScreen`** tối thiểu:
   - `Input` (search, debounce ~80ms) + `SongTable(DataTable)` cột `# / Title / Time / Notes / Risk / Suggested`
     + `Footer`.
   - Nguồn dữ liệu: `get_song_choices(force_refresh=True)` (đã ABC). Build search keys bằng `remove_accents`.
   - Lọc: Phase 1 dùng substring trên search key (rapidfuzz để Phase 4).
3. **MetadataCoordinator** (`workers.py`):
   - thread worker `@work(thread=True, exclusive=True, group="hydrate")`:
     gọi `hydrate_and_fill_raw_metadata(visible_paths, session, cfg)` → `call_from_thread` cập nhật bảng.
   - ProcessPool risk: với path `peek(...) is None or not .analyzed` → submit
     `compute_song_ui_metadata_payloads([str(p)...], session_to_worker_payload(session), cfg)`;
     done → `store_computed_song_ui_metadata_payloads(...)` → `call_from_thread` repaint.
   - Khi đổi query/selection/profile: để Textual huỷ batch cũ bằng `exclusive=True` cùng group.
   - Render cell theo `peek_cached_song_ui_metadata`: nếu `None`→`—`; có nhưng `not analyzed`→Time/Notes thật,
     Risk/Suggested = `…`; `analyzed`→đủ.
4. **Enter** → dựng `SongPickerResult` (đúng shape mục 3.2) → `app.exit(result)`.
   `Esc`/`q` ở picker → `app.exit(None)`.
5. **Session**: tạo `PlaybackSessionContext` từ initial_profile/tempo/fps/scan_code_mode để khớp cache key
   (giống `picker.py::picker_session`).

**Test (bắt buộc):** `tests/test_textual_picker.py` dùng `App.run_test()` + `Pilot`:
- [ ] mở app → bảng có đúng số bài = `len(get_song_choices())`.
- [ ] gõ query lọc đúng; `↓`+`Enter` → `SongPickerResult.song_path` đúng bài đang chọn.
- [ ] `Esc` → trả `None`.
- [ ] (nếu khả thi) sau khi raw hydrate, cột Time/Notes khác `—`.

**Cổng kiểm duyệt P1:**
- [ ] `--ui classic` hành vi y hệt trước (regression check: `--list`, chọn bài, phát).
- [ ] `--ui textual` chọn bài → `play_selected_song` phát bình thường (smoke thủ công, mô tả trong PR).
- [ ] Pilot test xanh; toàn bộ `uv run pytest` không thêm fail mới (so với baseline timing fails đã biết).
- [ ] Không sửa file bất biến; `SongPickerResult` nguyên shape.
- [ ] Worker cập nhật UI **chỉ** qua `call_from_thread` (đọc code xác nhận).

---

### Phase 2 — Parity đầy đủ (panel + palette + preview/detail)
**Việc:**
- `ModalScreen`: ProfileSelect / TempoSelect / FpsSelect / ThemeSelect — đọc options từ
  `PROFILES_INFO / TEMPO_OPTIONS / FPS_OPTIONS / THEME_PRESETS`; áp dụng → persist qua
  `persist_default_profile/tempo/fps` + `save_theme`.
- **Command palette** `/`: dùng Textual `CommandPalette`/`Provider` cho danh sách `commands`
  (preview, profile, tempo, fps, calibration, dry_run, hud, telemetry, reload, theme, help).
- **Preview / Detail**: hiện raw stats tức thì, risk khi `analyzed` (gate y Phase 1).
- **Status bar**: profile/tempo/fps/dry/hud/telem/theme/songs (reactive).
- Toggle HUD/telemetry/dry-run cập nhật `AppConfig` qua `save_config` (đọc cách dùng ở `picker.py`).

**Cổng kiểm duyệt P2:** ✅ **QUA CỔNG** (2026-06-04, sau 1 vòng changes-requested)
- [x] Mọi chức năng `commands` của picker cũ có mặt và tương đương (palette 11 lệnh).
- [x] Đổi profile/tempo/fps/theme **persist** đúng (qua `persist_default_*` + `save_theme`).
- [x] Pilot test cho mỗi modal + palette (mở, chọn, áp dụng, đóng) — `tests/test_textual_picker.py` 12 passed.
- [x] ~~Theme map đủ 5 biến thể, không vỡ layout~~ → **chỉ persist tên theme**, CSS chưa recolor. Hạ xuống non-blocker, **NỢ KỸ THUẬT phải trả trước Phase 5** (xem dưới).

> **Lịch sử kiểm duyệt P2:** vòng 1 trả lại 2 blocker key-handling:
> - **Blocker 1 (arrow +2):** `App.on_key` xử lý up/down song song với DataTable native → double-move. **Đã vá:** gỡ hẳn up/down khỏi `on_key` (giờ chỉ enter/escape/q); arrow thuần native. Regression test `test_table_arrow_moves_one_row_from_initial_focus` (table focus → down → cursor 0→1).
> - **Blocker 2 (phím tắt/nav chết sau modal):** Esc/Enter rò từ modal lên `App.on_key` → exit ngầm + mất focus. **Đã vá:** `OptionModal`/`InfoModal.on_key` `event.stop()` esc/enter; `on_screen_resume`→`_focus_table` khôi phục focus về bảng. Regression test `test_shortcuts_and_arrow_survive_modal_close`.
> - **Scoping phím tắt:** `p/t/f/y/v/d/h/f3/ctrl+r//` chuyển vào `SongTable.BINDINGS` (chỉ sống khi bảng focus); gõ trong `#search` không kích shortcut. Một cơ chế duy nhất, không còn xử lý chồng trong `on_key`.
> - **Tái kiểm:** đúng repro của cổng đều xanh; `git diff HEAD` trên tập bất biến rỗng (kể cả `picker_metadata.py`).
>
> **NỢ KỸ THUẬT chặn Phase 5 — theme-recolor:** `_apply_theme` (app.py) mới lưu tên + cập nhật status bar; màu UI giữ nguyên (CSS inline cố định). Phải map 5 `THEME_PRESETS` → Textual design tokens / biến thể CSS **trước khi cutover ở Phase 5** (đã ghi điều kiện vào Cổng P5).

---

### Phase 3 — Phần còn lại
**Việc:** calibration screen (telemetry summary → recommend → persist), help screen, reload
(`Ctrl+R`, gọi `get_song_choices(force_refresh=True)` + `clear_metadata_cache()`), warm cache nền
(`warm_persistent_metadata_cache`).

**Cổng P3:** ✅ **QUA CỔNG** (2026-06-04, 1 vòng)
- [x] calibration/help/reload tương đương cũ — `_apply_calibration` khớp từng dòng với classic picker (picker.py:1360-1368); reload = `clear_metadata_cache` + `force_refresh` + repaint + reschedule.
- [x] warm cache không chặn first paint — `on_mount` render đồng bộ trước, rồi mới `run_worker(_warm_metadata_cache, thread=True)`; UI callback qua `call_from_thread`.
- [x] test — warmup / reload / calibration-apply (`tests/test_textual_picker.py` 14 passed; full suite 187 passed/0 fail).
> Lưu ý: executor chưa chạy picker tương tác thật lượt này (Pilot + smoke CLI). Warmup-non-blocking chứng minh bằng thứ tự code → chấp nhận; xác minh thủ công 1 lần trước P5. Nợ theme-recolor (từ P2) vẫn chặn P5.

---

### Phase 4 — rapidfuzz + đánh bóng
**Việc:** thay lọc substring bằng rapidfuzz ranked (ví dụ `rapidfuzz.process.extract` trên search keys,
ngưỡng + sắp xếp theo score, giữ thứ tự ABC khi query rỗng). Mouse, cuộn mượt, highlight match.

**Cổng P4:** ✅ **QUA CỔNG** (2026-06-04, 1 vòng)
- [x] query rỗng vẫn A→Z — `rank_song_choices` trả `list(choices)`; test xác nhận `== choices`.
- [x] gõ sai 1-2 ký tự vẫn hợp lý — `process.extract`+`fuzz.WRatio` cutoff 60; 1 ký tự→substring né nhiễu; substring boost 100 + tie-break giữ A→Z.
- [x] benchmark <16ms/keystroke với 107+ bài — test assert `max<0.016s`; đo thực tế max 0.196ms (~80× margin).
- [x] test ranking — 3 ranking test + benchmark + guard "gõ chữ trong search không kích shortcut".
- Highlight substring qua `get_match_span` (reuse picker_theme, read-only). `on_input_changed` dùng ranker (reset_cursor=True).
> Lưu ý: chưa chạy picker tương tác thật (Pilot+CLI). Trước P5 chạy `--ui textual` xác minh fuzzy realtime + highlight. Nợ theme-recolor (P2) vẫn chặn P5. Full suite 191 passed/0 fail.

---

### Phase 5 — Cutover
**Việc:** đổi mặc định `--ui` sang `auto` (textual khi terminal hỗ trợ); đánh dấu picker
prompt_toolkit **deprecated** (giữ code + cờ `classic`); cập nhật README/help. Gỡ prompt_toolkit picker
ở PR sau khi ổn định thực địa.

**Cổng P5:** ✅ **QUA CỔNG** (2026-06-04, vòng 2) — **CUTOVER DUYỆT, DỰ ÁN HOÀN TẤT (P0→P5 đều qua cổng).** Reviewer tự chạy `dist\Sky-Player\Sky-Player.exe --selftest-textual` → "Textual selftest OK…" exit 0 trên đúng artifact frozen → xác nhận exe nạp Textual+rapidfuzz+parse CSS thật. Full suite 197 passed/0 fail; bất biến rỗng.
- [x] **NỢ TỪ P2 — theme-recolor:** đã trả. `_apply_theme_class` add/remove class trên `self.screen` (mount + đổi live); CSS riêng 5 preset; `_normalize_theme_name` kẹp preset hợp lệ; test `screen.has_class("theme-minimalist")` chứng minh recolor.
- [x] `auto` chọn textual đúng môi trường — default `auto`, `_supports_textual()` yêu cầu TTY + (WT_SESSION|vscode trên Windows); fallback classic + non-TTY giữ nguyên.
- [x] **BLOCKER đã vá: Smoke đóng gói exe đi qua picker.** Thêm flag ẩn `--selftest-textual`: import `rapidfuzz` + `sky_music.ui.textual_app`, dựng `SkyPickerApp`, chạy `run_test()` headless, kiểm table render + theme class. Đã chạy `dist\Sky-Player\Sky-Player.exe --selftest-textual` → exit 0, không lỗi CSS/_MEIPASS/ModuleNotFoundError.
- [x] README/help cập nhật. Không gỡ prompt_toolkit trong cùng PR cutover.
> Blocker đóng gói đã có bằng chứng tự động hoá; chờ reviewer tái kiểm vòng 2 trước khi đóng dấu P5.

**Trạng thái triển khai P5:** sẵn sàng tái kiểm (2026-06-04)
- Theme recolor: `TEXTUAL_THEME_TOKENS` cover đủ 5 `THEME_PRESETS`; `SkyPickerApp`/modal gắn class theme, CSS đổi background/foreground/border/cursor/modal, highlight match lấy màu theo theme. Test cover đủ preset + đổi theme sang `minimalist` gắn class thật.
- Cutover: `--ui` default = `auto`; `_supports_textual()` yêu cầu TTY, Windows chỉ bật khi `WT_SESSION` hoặc VS Code terminal. Test cover non-TTY, Windows Terminal, terminal yếu.
- Packaging: release build mặc định collect Textual/Rich + hidden imports `sky_music.ui.textual_app*`; `--textual-proof` vẫn giữ.
- Frozen picker smoke: flag ẩn `--selftest-textual` import `rapidfuzz` + `sky_music.ui.textual_app`, dựng `SkyPickerApp`, chạy `app.run_test()` headless, kiểm table render + theme class; `dist\Sky-Player\Sky-Player.exe --selftest-textual` → exit 0.
- Smoke đã chạy: source `--help`, `--list --ui auto`, dry-run playback; exe `dist/Sky-Player/Sky-Player.exe --help`, `--list --ui auto`, dry-run playback đều exit 0. P5 packaging smoke phải luôn có ít nhất một lệnh đi qua đường picker (`--selftest-textual` hoặc smoke tương tác `--ui textual`).
- Test: `uv run pytest -q tests\test_textual_picker.py tests\test_cli.py` → 41 passed; `uv run pytest -q` → 197 passed.
- Bất biến: diff trên domain/orchestration/infrastructure/platform/hud/config/layouts/picker_metadata/text_render/picker rỗng.
> Chưa drive được picker tương tác thật qua terminal tự động trong môi trường này; cần reviewer smoke thủ công `.\dist\Sky-Player\Sky-Player.exe --ui textual` hoặc `uv run python src\main.py --ui textual`: mở picker, đổi theme, fuzzy search, Enter chọn bài.

---

## 6. Yêu cầu kiểm thử xuyên suốt
- Test domain hiện có **phải giữ xanh** (trừ các fail timing đã biết do `config.py` đang dở — xem mục 8).
- Mỗi phase thêm test Textual (`run_test`/`Pilot`), không giảm độ phủ.
- Không test nào được khởi động playback thật / SendInput.

## 7. Yêu cầu đóng gói
- **Inline CSS** (không `.tcss`). Nếu buộc dùng file CSS → phải xử lý `sys._MEIPASS` + `--add-data`
  (tránh; ưu tiên inline).
- Cập nhật `src/build_app.py` nếu thiếu module: ưu tiên `--collect-all textual --collect-all rich`.
- Phải có 1 lần smoke exe ở **P0** và **P5**.

## 8. Trạng thái baseline đã biết (đừng nhầm là do bạn gây ra)
- `uv run pytest` hiện có **13 fail thuộc nhóm timing** (`test_session_context`, `test_empirical_floors`,
  `test_scheduler_new`, `test_engine_refactor`) do refactor timing **đang dở** trong `config.py`
  (uncommitted). Chúng **không liên quan** UI. Kiểm chứng: `git stash push -- src/sky_music/config.py`
  rồi chạy lại các test đó → xanh; `git stash pop` khôi phục. **Tuyệt đối không "sửa" các test này.**
- Baseline đếm: **158 passed, 13 failed**. Phase của bạn không được làm tăng số fail.

## 9. Quy trình kiểm duyệt (reviewer)
1. Executor mở **1 PR/diff cho 1 phase**, kèm: mô tả việc, kết quả `uv run pytest` (đếm pass/fail),
   mô tả smoke (lệnh + quan sát), và xác nhận "không chạm file bất biến".
2. Reviewer chạy checklist "Cổng kiểm duyệt" của phase đó + đối chiếu danh sách bất biến (mục 1).
3. Reviewer chạy `git diff --name-only` để chắc không có file domain/orchestration/infra/config bị đổi.
4. Reviewer chạy thử `--ui classic` (regression) và `--ui textual` (tính năng mới).
5. Chỉ qua cổng mới sang phase sau. Phát hiện vi phạm bất biến → trả lại, không merge.

## 10. Thứ tự & phụ thuộc
P0 → P1 → P2 → P3 → P4 → P5 (tuần tự, không song song). P0 là cổng "go/no-go" cho cả dự án.

---

## Phụ lục A — Bản đồ tính năng picker cũ → Textual
| Picker cũ (prompt_toolkit) | Textual |
|---|---|
| `update_ui()` dựng 4 control | reactive attrs + `watch_*` |
| máy trạng thái `current_view` string | `Screen`/`ModalScreen` stack |
| `build_box`/cell-width thủ công | CSS + Rich render |
| ProcessPool + `metadata_generation` + coalesce | `@work(exclusive=True)` + ProcessPool giữ nguyên |
| themes dict → `Style.from_dict` | CSS variants map từ `THEME_PRESETS` |
| `/` mở commands view | Textual CommandPalette |
| `_format_song_row_fast` | `DataTable` rows + cập nhật cell |

## Phụ lục B — Cờ PyInstaller thực tế đã dùng (điền ở P0)
> _Executor điền sau khi P0 build thành công:_
> - Lệnh: `uv run python src\build_app.py --textual-proof`
> - hiddenimports/collect: không cần; build thành công không dùng `--collect-all textual --collect-all rich`
> - Ghi chú _MEIPASS (nếu có): không có lỗi `StylesheetError` hoặc lỗi `_MEIPASS`; CSS đang inline trong `SkyPickerApp.CSS`

## Phụ lục C — Lệnh hay dùng
```bash
uv run pytest -q                          # test
uv run python src/main.py --ui textual    # chạy picker Textual
uv run python src/main.py --ui classic    # fallback cũ
uv run python src/main.py --list          # liệt kê (ABC)
uv run python src/build_app.py            # đóng gói exe
git diff --name-only                      # kiểm file đã đổi (review)
```
