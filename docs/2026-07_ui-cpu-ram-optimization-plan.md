# Kế hoạch: Tối ưu CPU/RAM hệ thống UI Textual (bản thực thi)

> **Đối tượng đọc:** AI/kỹ sư **thực thi** tối ưu UI. Tài liệu tự chứa — không cần hội thoại trước.
> **Reviewer:** tác nhân giám sát + chủ dự án (nghiệm thu theo cổng cuối mỗi phase).
> **Nguồn audit:** deep-dive UI 2026-07-17 (code `src/sky_music/ui/` as-of date).
> **Ngày:** 2026-07-17 · **Status:** active implementation plan · **Revision:** v1

---

## 0. TL;DR

UI Textual đã **đúng hướng kiến trúc** (engine off UI thread, metadata progressive + LRU,
unified in-place playback, free-threaded). Nút thắt hiệu năng khi **đang dùng** nằm ở
**presentation hot path**, không phải scheduler/SendInput:

| Ưu tiên | Vấn đề | Tác động |
|---|---|---|
| **P0** | `PlaybackCard` poll 10Hz luôn full recompose + `refresh(layout=True)` | CPU UI thread lúc play |
| **P1** | ANSI bridge (`ansi_*_box` → `Text.from_ansi`) + gradient O(width) mỗi tick | CPU + allocation churn |
| **P1** | `GradientHeader` parse/blend mỗi `render()` | CPU idle + mỗi đổi status |
| **P2** | Full `DataTable.clear()` + re-add rows; metadata update mọi row | CPU search/reload/library lớn |
| **P2** | Metadata warm/analyze **toàn library**, không visible-first | CPU nền picker; cạnh tranh free-threaded |
| **P3** | Ba surface playback trùng lặp + module >1k dòng | Maintainability → khó tối ưu an toàn |

**Mục tiêu đo được (không claim Task Manager magic):**

1. **Playing (dry-run dài ≥60s, verbose HUD off):** số lần `refresh(layout=True)` trên
   `PlaybackCard` **≤ 2/s** trung bình (hiện ~10/s); paint content-only khi height ổn định.
2. **Playing debug off:** không gọi `sorted()` trên latency ring mỗi poll (chỉ khi debug
   stats thực sự hiển thị và throttle).
3. **Picker idle sau warm:** không regress RSS peak sau multi-song so với baseline
   `scripts/mem_after_play.py` (nếu có) / không tăng holdout metadata unbounded.
4. **Library 100 bài (repo hiện tại):** search debounce giữ nguyên; full-table rebuild sau
   search vẫn chấp nhận; **không** bắt buộc virtualization ở Phase A–B.
5. **Gate test:** `tests/test_textual_*.py`, `test_hud_ui.py`, `test_picker_metadata_*`,
   `test_command_palette_*`, `--selftest-textual` xanh. **Không** đổi golden timing /
   dispatch latency targets.

---

## 1. Bất biến (vi phạm = fail review ngay)

Từ [AGENTS.md](../AGENTS.md) + timing docs:

1. **P0 Security:** Không game memory, injection, hooks ngoài control path hiện có.
   Chỉ `SendInput` cho note simulation.
2. **Không đụng** hot path dispatch/scheduler timing formulas:
   - `src/sky_music/domain/scheduler.py`, `scheduler_types.py` (công thức hold/frame)
   - `src/sky_music/orchestration/core/*`, `runtime_dispatch.py`, `dispatch_loop.py`
   - `src/sky_music/platform/win32/inputs.py` send path
3. **Không đổi** hợp đồng `SongPickerResult`, CLI flags public, hành vi binding phím
   (F8/F9/F10/F2) trừ khi phase ghi rõ và có test.
4. **Không đổi** visual parity *trừ khi* phase B chủ đích migrate ANSI→native và test
   snapshot/geometry vẫn pass (dock bottom, countdown grow).
5. **Free-threaded:** Mọi mutation metadata cache giữ lock hiện có (`_cache_lock`).
   Không invent lock-free cache. Snapshot latency ring giữ lock-free writer.
6. **Surgical PRs:** một concern chính / PR; không refactor unrelated.
7. **`uv run`** mọi lệnh Python; không `pip install`.
8. **Engine vẫn off UI thread** (`@work(thread=True, exclusive=True)`). Không `engine.play()`
   trên UI thread.

---

## 2. Scope

### In scope

| Area | Paths |
|---|---|
| Playback card hot path | `src/sky_music/ui/textual_app/playback_app.py` (`PlaybackCard`, `SnapshotRenderer`) |
| Header gradient | `src/sky_music/ui/textual_app/display_widgets.py` |
| ANSI helpers (chỉ nếu reuse/cache) | `src/sky_music/ui/text_render.py` |
| Picker table/search/metadata UI | `src/sky_music/ui/textual_app/screens/picker.py` |
| Metadata coordinator | `src/sky_music/ui/textual_app/workers.py` |
| Metadata caches (dirty keys only nếu cần) | `src/sky_music/ui/picker_metadata.py` |
| App shell (cache widget refs, bỏ double work) | `src/sky_music/ui/textual_app/app.py` |
| Optional unify surface (Phase C) | `playback_app.py` (`PlaybackApp`/`PlaybackScreen`), `cli/console_playback.py` |
| Tests | `tests/test_textual_playback.py`, `test_textual_picker.py`, tests mới focused |
| Docs | file này; [INDEX.md](INDEX.md) link active |

### Out of scope

- Scheduler / SendInput / adaptive lead / MMCSS / focus dual-release
- Rust migration (`rust-migration-plan.md`)
- Redesign visual theme tokens / thêm theme mới
- Thay Textual bằng framework khác
- Virtualization DataTable full rewrite (chỉ đánh giá; implement chỉ nếu Phase B đo được hitch >16ms trên library ≥500)
- Classic/prompt_toolkit revival
- RAM plan items đã cover ngoài UI (`telemetry.records`, `gc.disable` RT) — xem
  [archive/2026-07_ram-memory-hygiene-plan.md](archive/2026-07_ram-memory-hygiene-plan.md)

### Liên quan đã ship (đừng re-do)

| Item | Evidence |
|---|---|
| Metadata LRU caps | `picker_metadata.py` `_METADATA_CACHE_MAX=2048` … |
| Progressive metadata batches | `workers.py` warm 25 / risk 10 |
| Search debounce 150ms | `picker.py` `set_timer(0.15, …)` |
| `release_song_data` + unmount guard | `playback_app.py` `on_unmount` / `run_engine` |
| Latency ring `maxlen=512` + lock-free snapshot | `SnapshotRenderer` |
| Dock bottom + explicit height | `_rerender` set `styles.height` (plan 2.7) |

---

## 3. Kiến trúc hiện tại (baseline coder phải hiểu)

### 3.1 Luồng chính (unified)

```text
main.run_sky_app_unified()
  → SkyPickerApp(unified_mode=True)
       get_default_screen → PickerScreen
         compose: GradientHeader | Search | SongTable | Detail | PlaybackCard | Footer
       confirm → start_playback_workflow
         → PlaybackCard.start_countdown / start_playback
         → @work engine.play()  (thread)
         → set_interval(0.1, _poll)  → _rerender() mỗi tick nếu có snapshot
```

### 3.2 Surface playback (debt)

| Surface | Dùng khi | Render model |
|---|---|---|
| **`PlaybackCard`** | Unified app (primary) | Single `Static`, ANSI strings → `Text.from_ansi` |
| **`PlaybackApp`** | `run_playback_textual` / console | Nhiều `Static`, `update()` 10Hz |
| **`PlaybackScreen`** | Legacy screen path | Static + debug panel |
| **`hud.ProgressRenderer`** | Console HUD non-Textual | Rich Live |

Phase A–B **tối ưu `PlaybackCard` trước** (user path chính). Phase C mới unify/deprecate.

### 3.3 Hot path tốn CPU (code pointers)

**A. Always layout (P0)**

```python
# playback_app.py — PlaybackCard
def _rerender(self) -> None:
    lines = self._compose_lines()
    self.styles.height = len(lines)
    self.refresh(layout=True)   # ← full layout tree mỗi lần

def _poll(self) -> None:
    snap = self.renderer.get_snapshot()
    if snap is not None:
        self._snapshot = snap
        self._rerender()        # ← không dirty-check
```

Timer: `set_interval(0.1, self._poll)` trong `start_playback`.

**B. Full recompose mỗi tick**

`_compose_lines` → `_playing_body` → `ansi_gradient_box` / `ansi_box` → join →
`Text.from_ansi` trong `render()`. Gradient: `Color.parse` + blend **per border cell**.

**C. Debug stats mỗi frame khi debug**

```python
if self.debug_mode:
    stats = self.renderer.debug_stats()  # sorted() + variance
```

**D. Header**

`GradientHeader.render()`: parse stops + O(width) blend mỗi refresh (status chips đổi
thường xuyên khi đổi profile/tempo/fps).

**E. Picker table**

`_render_table`: `table.clear()` + `add_row` mọi filtered row.
`refresh_metadata_rows`: `update_cell` **mọi** filtered row + `_render_detail()` mỗi batch.

**F. Metadata coordinator**

`_warm_and_process_all_paths(all_paths)` — không ưu tiên visible/cursor.

### 3.4 RAM UI-relevant

- Caches đã LRU (đừng đụng policy trừ Phase B visible-first dirty tracking).
- `PlaybackCard` giữ `engine` ref đến `_safe_finish` — đã release; giữ nguyên contract.
- Trùng surface code: chi phí **binary/import**, không leak runtime nếu path không mount.

---

## 4. Design principles (cho mọi PR)

1. **Measure then cut.** Mỗi phase có assertion/test hoặc counter nội bộ chứng minh
   giảm work (layout calls, paint skips). Không “cảm giác nhanh hơn”.
2. **Dirty paint > lower FPS blindly.** Ưu tiên skip work vô ích; chỉ hạ poll rate nếu
   vẫn mượt progress bar (target visual ~4–10 Hz content update).
3. **Layout only when geometry changes.** `refresh(layout=True)` chỉ khi `height`/mode
   đổi số dòng.
4. **Preserve dock geometry.** Tests `test_card_anchored_*` / layout grow sau countdown
   **bắt buộc xanh**.
5. **No timing side effects.** UI poll không gọi focus OpenProcess, không lock engine,
   không `debug_stats` heavy trên path non-debug.
6. **Free-threaded safe.** Không share mutable list không bảo vệ giữa UI và dispatch.
7. **One surface at a time.** Phase A không rewrite `PlaybackApp` trừ mirror bugfix
   tối thiểu.
8. **Feature flags optional.** Nếu thay đổi hành vi paint gây tranh cãi UX, thêm
   constant module-level (vd `_PLAYBACK_POLL_S = 0.1`) dễ revert — **không** thêm CLI
   flag user-facing trừ khi chủ dự án yêu cầu.

---

## 5. PR Plan (DAG)

```text
PR-A  PlaybackCard dirty paint + conditional layout     ──┐
PR-B  GradientHeader + ANSI box gradient cache         ──┼──► PR-D optional unify
PR-C  Picker table/metadata incremental + visible-first ─┘
```

| PR | Title | Depends | Risk | Size | Priority |
|---|---|---|---|---|---|
| **PR-A** | PlaybackCard CPU: dirty-check, conditional layout, debug throttle | — | Medium | M | P0 |
| **PR-B** | Gradient/ANSI render cache | — | Low | S | P1 |
| **PR-C** | Picker incremental metadata + visible-first coordinator | — | Medium | M | P2 |
| **PR-D** | Unify playback surfaces (native widgets) | A (+ ideally B) | Higher | L | P3 |

**Parallelism:** **A ∥ B ∥ C** an toàn nếu không đụng cùng hunk `playback_app.py` /
`picker.py`. **D** sau A (và nên sau B).

**Không gộp A+D** trong một PR.

---

## 6. PR-A — PlaybackCard dirty paint + conditional layout (P0)

### 6.1 Goals

- Giảm layout thrash lúc playing từ ~10 layout/s xuống **chỉ khi số dòng box đổi**.
- Giảm full recompose khi snapshot “không đổi meaningfully”.
- Giữ progress bar mượt (update content ≥4 Hz khi `current` đổi).
- Không đổi engine, bridge, bindings.

### 6.2 Changes (chi tiết implement)

#### A1. Snapshot dirty signature

Trong `PlaybackCard`, thêm state:

```python
self._last_paint_sig: tuple[Any, ...] | None = None
self._last_line_count: int = -1
```

Định nghĩa signature từ snapshot + mode + debug (ví dụ):

```python
def _paint_signature(self, snap: PlaybackSnapshot | None) -> tuple[Any, ...]:
    if snap is None:
        return (self._mode, self.debug_mode, self._countdown_remaining, ...)
    # Quantize time so sub-100ms jitter không force repaint mỗi poll
    current_q = int(snap.current * 10)  # 0.1s buckets — điều chỉnh nếu test UX
    stats_sig: tuple[Any, ...] = ()
    if self.debug_mode:
        # Chỉ lấy counters nhẹ; p50/p95 throttle riêng (A3)
        c = self.renderer.counters_snapshot()
        stats_sig = (c.late_5ms, c.active_keys, c.stuck_keys, c.keys_dropped)
    else:
        c = self.renderer.counters_snapshot()
        stats_sig = (c.late_5ms, c.active_keys, c.stuck_keys, c.keys_dropped)
    return (
        self._mode,
        self.debug_mode,
        snap.status,
        current_q,
        int(snap.total * 10),
        snap.input_path_degraded,
        stats_sig,
        self._countdown_remaining,
        self._risk_selected,
        # ... fields risk/error title only when those modes
    )
```

**Chốt quantize:** bắt đầu `0.1s` (`int(current * 10)`). Nếu smoke thấy bar “nhảy” thô,
đổi `* 20` (50ms). **Không** update mỗi microsecond.

#### A2. `_rerender` tách paint vs layout

Thay:

```python
def _rerender(self) -> None:
    lines = self._compose_lines()
    self.styles.height = len(lines)
    self.refresh(layout=True)
```

Bằng:

```python
def _rerender(self, *, force: bool = False) -> None:
    sig = self._paint_signature(self._snapshot)
    if not force and sig == self._last_paint_sig:
        return
    self._last_paint_sig = sig
    lines = self._compose_lines()
    line_count = len(lines)
    if line_count != self._last_line_count:
        self._last_line_count = line_count
        self.styles.height = line_count
        self.refresh(layout=True)
    else:
        self.refresh()  # content only — dock geometry giữ nguyên
```

**Bắt buộc `force=True`** trong: `show_idle`, `show_error`, `show_risk`,
`start_countdown`, `start_playback`, `toggle_debug`, mode transitions, resize nếu có
hook.

`_poll` dùng `_rerender()` không force.

#### A3. Throttle `debug_stats()` (p50/p95/σ)

Hiện `_playing_body` khi `debug_mode`:

```python
stats = self.renderer.debug_stats()  # sorted every paint
```

Đổi:

- Giữ `counters_snapshot()` cho late counters / keys (rẻ).
- Cache `DebugStats` full percentiles với TTL **0.5s** (hoặc 1s) trên card:

```python
self._debug_stats_cache: DebugStats | None = None
self._debug_stats_mono: float = 0.0  # time.monotonic()
```

Chỉ gọi `debug_stats()` khi `monotonic() - self._debug_stats_mono >= 0.5`.

**Không** thêm lock mới trên renderer hot path writer.

#### A4. Poll interval (tuỳ chọn, cùng PR nếu test OK)

- Default giữ `0.1` **hoặc** đổi `0.15` nếu A1–A2 đủ skip.
- Nếu đổi: constant `_PLAYBACK_POLL_S = 0.1` ở đầu class/module; comment rationale.
- **Không** poll chậm hơn `0.25` — progress bar sẽ cảm giác lag.

Debug hotkey edge-detect (`_poll_debug_hotkey`) vẫn chạy mỗi poll — rẻ (`is_hotkey_down`).

#### A5. Invalidate signature khi cần

Khi `toggle_debug`, risk selection, countdown tick: `self._last_paint_sig = None` hoặc
`force=True`.

### 6.3 Files allowed

- `src/sky_music/ui/textual_app/playback_app.py` (chủ yếu `PlaybackCard`, có thể
  helper nhỏ trên `SnapshotRenderer` nếu cần cache hook — **không** đổi
  `update_counters` hot path semantics)
- `tests/test_textual_playback.py` (tests mới + giữ geometry)
- Optional: `tests/test_playback_card_paint.py` nếu file test quá lớn

**Không** sửa `app.py` workflow trừ import-only.

### 6.4 Tests bắt buộc

1. **Giữ** mọi test dock/geometry hiện có (`test_card_anchored_*`, countdown grow,
   debug toggle height).
2. **`test_rerender_skips_when_signature_unchanged`:** mock snapshot identical
   quantize → gọi `_poll` 2 lần → assert `refresh` layout call count không tăng 2 lần
   (monkeypatch `refresh` / đếm `styles.height` writes).
3. **`test_rerender_layout_when_line_count_changes`:** toggle debug hoặc inject
   warning line → height update + layout path.
4. **`test_progress_quantized_still_updates`:** current 1.00 → 1.15 (vượt bucket) →
   paint xảy ra.
5. **`test_debug_stats_throttled`:** với debug on, 5× `_playing_body` trong <0.5s →
   `debug_stats` mock call count ≤ 2 (hoặc 1 + force).

### 6.5 Manual smoke

```powershell
uv run python -m app --dry-run
# chọn bài dài; quan sát progress mượt; F2 debug; resize terminal; pause F8
```

### 6.6 Gate PR-A

```powershell
uv run ruff check src/sky_music/ui/textual_app/playback_app.py tests/test_textual_playback.py
uv run pyright
uv run pytest tests/test_textual_playback.py -q
```

### 6.7 Acceptance PR-A

- [ ] Conditional layout landed; default path no longer unconditional `layout=True`
- [ ] Dirty signature skips no-op paints
- [ ] Debug percentiles throttled
- [ ] Geometry tests green
- [ ] No engine/dispatch file touched

---

## 7. PR-B — GradientHeader + ANSI gradient cache (P1)

### 7.1 Goals

- Loại `Color.parse` lặp mỗi `render()` / mỗi `ansi_gradient_box` call.
- Cache mảng hex (hoặc ANSI) theo `(stops_tuple, width)`.

### 7.2 Changes

#### B1. `GradientHeader` — cache gradient row

Trong `display_widgets.py`:

```python
self._grad_cache_key: tuple[tuple[str, ...], int] | None = None
self._grad_hex: list[str] = []
```

Khi `render()`:

```python
key = (tuple(self._stops), width)
if key != self._grad_cache_key:
    self._grad_cache_key = key
    self._grad_hex = _build_gradient_hex(self._stops, width)
# dùng self._grad_hex[i] thay g(i)
```

Invalidate cache trong `set_theme` (stops đổi).

Helper pure (module-level hoặc `text_render.py`):

```python
def build_horizontal_gradient_hex(stops: Sequence[str], width: int) -> list[str]:
    ...
```

Unit-test pure function — không cần Textual pilot.

#### B2. `ansi_gradient_box` — cache tương tự

`text_render.ansi_gradient_box` hiện parse stops mỗi call. Options (chọn 1):

- **(Preferred)** Cache `functools.lru_cache(maxsize=64)` trên
  `_gradient_ansi_row(stops: tuple[str, ...], width: int) -> tuple[str, ...]`
  (ANSI codes per column), dùng trong top/bottom border loop.
- Hoặc pass precomputed row từ `PlaybackCard` (nhiều API hơn — tránh nếu không cần).

**Lưu ý:** `lru_cache` trên free-threaded: OK nếu pure/immutable keys; stops là tuple
str hex.

#### B3. Optional micro: precompute theme ANSI in `PlaybackCard`

`_playing_body` gọi `hex_to_ansi(preset.*)` nhiều lần mỗi paint. Cache trên card khi
theme set:

```python
self._ansi_accent = hex_to_ansi(preset.accent)
...
```

Invalidate khi `theme_name` đổi (hiếm lúc play).

### 7.3 Files

- `src/sky_music/ui/textual_app/display_widgets.py`
- `src/sky_music/ui/text_render.py`
- `src/sky_music/ui/textual_app/playback_app.py` (chỉ B3 optional)
- `tests/test_text_render.py` (mới hoặc extend) + optional header unit test

### 7.4 Tests

1. `test_build_horizontal_gradient_hex_length` — `len == width`, deterministic.
2. `test_gradient_cache_stable` — 2× same inputs → equal lists; width change → new.
3. `test_ansi_gradient_box_unchanged_visual_smoke` — snapshot string length/border
   chars cho width cố định (không cần pixel).

### 7.5 Gate PR-B

```powershell
uv run ruff check src/sky_music/ui/textual_app/display_widgets.py src/sky_music/ui/text_render.py
uv run pyright
uv run pytest tests/test_text_render.py tests/test_textual_picker.py -q
```

### 7.6 Acceptance PR-B

- [ ] Header không `Color.parse` trong tight loop mỗi cell mỗi frame
- [ ] `ansi_gradient_box` không re-parse stops mỗi call (cache hit path)
- [ ] Visual smoke picker header + playing gradient OK

---

## 8. PR-C — Picker incremental metadata + visible-first (P2)

### 8.1 Goals

- `refresh_metadata_rows` chỉ `update_cell` khi giá trị cell thực sự đổi.
- Detail panel chỉ rebuild khi selected path/meta signature đổi.
- Metadata coordinator **ưu tiên** visible/near-cursor paths trước full library.
- Không đổi schema `SongUiMetadata` / SQLite schema.

### 8.2 Changes

#### C1. Incremental cell update

Trong `PickerScreen.refresh_metadata_rows`:

```python
# pseudo
new_vals = _metadata_cells(metadata)
if table.get_cell(row_key, "time") != new_vals.duration:  # or track side cache
    table.update_cell(...)
```

**Thực tế Textual DataTable:** `get_cell` có thể đắt/khó so sánh `Text`. Preferred:

```python
self._row_meta_sig: dict[str, tuple[str, str, str, str]] = {}
sig = (duration, notes, risk, suggested)
if self._row_meta_sig.get(row_key) == sig:
    continue
self._row_meta_sig[row_key] = sig
table.update_cell(...)
```

Clear `_row_meta_sig` trong `_render_table` (full rebuild).

#### C2. Detail dirty-check

```python
self._detail_sig: tuple[str, ...] | None = None
# sig = (path_str, analyzed, risk, note_count, ...)
```

Skip `detail.update(...)` nếu sig trùng.

#### C3. Visible-first coordinator

Trong `MetadataCoordinator._warm_and_process_all_paths`:

1. Giữ API `refresh(paths: list[Path])` — full list từ picker.
2. Thêm optional prioritization:
   - Extend `MetadataApp` Protocol với method optional
     `get_metadata_priority_paths() -> list[Path]` default `[]`.
   - `PickerScreen` implement: paths của **filtered rows trong viewport**
     (nếu lấy được từ DataTable scroll offset) **else** filtered[:30] + selected ±10.
3. Process order:
   ```text
   priority_paths (stable unique)  →  remaining paths
   ```
4. Vẫn batch 25/10; vẫn cancel via `request_id`.
5. **Không** spawn thêm thread/process.

Nếu viewport API Textual khó/fragile: **fallback** `filtered[:40]` + current selection
neighbors — ghi rõ trong code comment; đủ cho library ~100.

#### C4. Search fuzzy limit (optional cùng PR)

`rank_song_choices`:

```python
matches = process.extract(..., limit=min(200, len(choices)) or None)
```

Với library repo (~100) không đổi hành vi. Thêm test: query empty vẫn full list;
single-char path giữ nguyên.

**Chỉ làm nếu** không phá test search ranking hiện có. Nếu ranking test expect full
ordering beyond 200 — skip C4 hoặc `limit=None` khi `len(choices) <= 300`.

#### C5. Double MetadataCoordinator (audit-only / micro-fix)

`SkyPickerApp` non-unified tạo `metadata`; `PickerScreen` cũng tạo. Main path
`unified_mode=True` → app.metadata `None`.

- Document trong comment `app.py` / `workers.py`.
- Nếu non-unified path vẫn ship: **không** start app-level coordinator khi picker
  screen owns one — surgical fix nếu test cover non-unified.

### 8.3 Files

- `src/sky_music/ui/textual_app/screens/picker.py`
- `src/sky_music/ui/textual_app/workers.py`
- `src/sky_music/ui/textual_app/app.py` (C5 only nếu cần)
- `tests/test_textual_picker.py`, `tests/test_picker_metadata_optimizations.py`

### 8.4 Tests

1. **`test_refresh_metadata_skips_unchanged_rows`:** populate sig cache → refresh →
   mock `update_cell` call count == 0 khi meta identical.
2. **`test_refresh_metadata_updates_changed_risk`:** after store analyzed meta →
   cells update risk from `...` to `LOW`/`HIGH`.
3. **`test_detail_skips_identical_sig`:** spy `DetailPanel.update` / Static.update.
4. **`test_coordinator_priority_order`:** inject priority paths; assert first batch
   includes them (mock `compute_song_ui_metadata_payloads` recording order) — unit
   test coordinator với fake app protocol.
5. Giữ tests fuzzy / reload / responsive columns.

### 8.5 Gate PR-C

```powershell
uv run ruff check src/sky_music/ui/textual_app/screens/picker.py src/sky_music/ui/textual_app/workers.py
uv run pyright
uv run pytest tests/test_textual_picker.py tests/test_picker_metadata_optimizations.py -q
```

### 8.6 Acceptance PR-C

- [ ] Row meta signature cache prevents redundant `update_cell`
- [ ] Detail not rewritten on every metadata batch if selection unchanged
- [ ] Coordinator processes priority paths first (tested)
- [ ] No SQLite schema / metadata dataclass field renames without migration story

---

## 9. PR-D — Unify playback surfaces (P3, optional large)

> Chỉ làm sau PR-A ổn định trên máy chủ dự án (smoke play thật). Đây là
> maintainability + CPU dài hạn, **không** chặn A–C.

### 9.1 Goals

- Một presentation path cho progress UI trong Textual.
- Loại bỏ (hoặc re-export thin) duplicate `PlaybackApp` / `PlaybackScreen` poll loops.
- Prefer **native Static widgets** (model `PlaybackApp._update_ui`) cho primary path
  **hoặc** giữ single `Static` nhưng build `rich.text.Text` **trực tiếp** (không ANSI
  round-trip).

### 9.2 Strategy (chọn một — coder ghi quyết định trong PR body)

| Option | Pros | Cons |
|---|---|---|
| **D-opt1** Migrate `PlaybackCard` body → multi-`Static` children như `PlaybackApp` | Best Textual practice; partial updates trivial | Geometry/CSS dock rewrite; more tests |
| **D-opt2** Keep single Static; compose `Text` with styles, drop `ansi_*` | Smaller diff; still kill from_ansi | Gradient border hard hơn (per-cell styles on Text) |
| **D-opt3** Make `PlaybackApp` thin wrapper around `PlaybackCard` for console | Dedup entry | Card vẫn ANSI nếu chưa opt1/2 |

**Recommended:** **D-opt2 first** (kill ANSI round-trip) **or D-opt1** if owner wants
long-term cleanliness. Do **not** implement both in one PR.

### 9.3 Steps (D-opt2 sketch)

1. `_compose_lines` → `_compose_text() -> Text` using Rich styles (`style=accent`) instead
   of `hex_to_ansi`.
2. `render()` return that `Text` directly (no `from_ansi`).
3. Border: either Textual CSS `border: round` on `#playback-card` (already in theme CSS
   for some paths) **or** keep box-drawing chars with solid accent (drop gradient) —
   **owner sign-off** if gradient removed.
4. Console `run_playback_textual` delegates to shared helper or pushes equivalent card.
5. Delete dead `CountdownScreen` if unused (grep first).
6. Snapshot tests / pilot geometry.

### 9.4 Invariants PR-D

- Unified workflow in `app.py` unchanged externally.
- Hotkeys F8/F9/F10/F2 identical.
- `SnapshotRenderer` API stable (`render`, `update_counters`, `get_snapshot`,
  `debug_stats`, `counters_snapshot`, `finish`).

### 9.5 Gate PR-D

```powershell
uv run ruff check .
uv run pyright
uv run pytest tests/test_textual_playback.py tests/test_textual_picker.py tests/test_hud_ui.py -q
uv run python -m app --selftest-textual
```

---

## 10. Measurement & baselines

### 10.1 Before any PR (optional but recommended)

Trên Windows dev machine:

```powershell
# Functional baseline
uv run pytest tests/test_textual_playback.py tests/test_textual_picker.py -q

# Memory (if script present)
uv run python scripts/mem_after_play.py
```

Ghi tay vào PR description: Python version (`3.14t`), theme, terminal (Windows Terminal).

### 10.2 Instrumentation (dev-only, không ship user-facing)

Trong PR-A, **được phép** counters sau (behind `if __debug__` hoặc env
`SKY_UI_PAINT_STATS=1`):

```python
self._paint_count = 0
self._layout_count = 0
self._skip_count = 0
```

Log một dòng khi playback finish nếu env set. **Xoá hoặc keep gated** — không spam
production logs.

### 10.3 Success metrics (soft)

| Metric | Baseline (estimate) | Target after A+B |
|---|---|---|
| `layout=True` / s playing steady | ~10 | ≤ 0.5 (mode stable) |
| Full compose / s playing steady | ~10 | ≤ 5–10 but **skips** dominate when paused |
| `debug_stats` / s debug on | ~10 | ≤ 2 |
| Picker metadata `update_cell` / batch | O(filtered) | O(changed only) |

Không fail CI nếu không có perf harness — tests correctness + counters unit tests đủ.

---

## 11. Risk register

| Risk | Mitigation |
|---|---|
| Progress bar trông giật do quantize thô | Giảm bucket (50ms); smoke UX |
| Dock clip regression khi bỏ layout | Giữ geometry tests; force layout on line_count change |
| Free-threaded race on paint sig | sig chỉ UI thread; snapshot copy via existing lock |
| Visible-first làm chậm full-library warm | Vẫn process remaining sau priority; cancel on play |
| ANSI→Text visual drift (PR-D) | Snapshot/pilot; owner visual approve |
| Scope creep into engine | Invariant §1 — reject PR that touches core/ |

---

## 12. Validation matrix (altitude)

| Change scope | Command |
|---|---|
| PR-A only | `uv run ruff check … && uv run pyright && uv run pytest tests/test_textual_playback.py` |
| PR-B only | + `tests/test_text_render.py` / picker smoke |
| PR-C only | + `tests/test_textual_picker.py` `test_picker_metadata_optimizations.py` |
| PR-D / multi | Full UI tests + `--selftest-textual` |
| Touch app entry | `uv run python -m app --selftest-textual` |

Broader if unsure:

```powershell
uv run ruff check . && uv run pyright && uv run pytest
```

---

## 13. File ownership cheat-sheet

| File | PR-A | PR-B | PR-C | PR-D |
|---|---|---|---|---|
| `textual_app/playback_app.py` | **Primary** | optional B3 | — | **Primary** |
| `textual_app/display_widgets.py` | — | **Primary** | — | — |
| `ui/text_render.py` | — | **Primary** | — | maybe |
| `textual_app/screens/picker.py` | — | — | **Primary** | maybe hide/show |
| `textual_app/workers.py` | — | — | **Primary** | — |
| `textual_app/app.py` | — | — | C5 only | workflow glue only |
| `ui/hud.py` | — | — | — | only if console parity |
| `domain/*` / `orchestration/core/*` | **FORBIDDEN** | **FORBIDDEN** | **FORBIDDEN** | **FORBIDDEN** |

---

## 14. Implementation checklist (copy vào PR)

### PR-A

- [ ] `_paint_signature` + skip path
- [ ] Conditional `layout=True`
- [ ] Debug stats throttle
- [ ] Force paths on mode change
- [ ] Tests §6.4
- [ ] Smoke dry-run play
- [ ] Gate §6.6 green

### PR-B

- [ ] Header gradient cache
- [ ] `ansi_gradient_box` / shared pure gradient helper + cache
- [ ] Tests §7.4
- [ ] Gate §7.5 green

### PR-C

- [ ] `_row_meta_sig` incremental updates
- [ ] Detail sig skip
- [ ] Priority paths in coordinator
- [ ] Tests §8.4
- [ ] Gate §8.5 green

### PR-D (optional)

- [ ] Owner chose D-opt1 / D-opt2 / D-opt3
- [ ] Dead code grep + remove/reexport
- [ ] Geometry + selftest green
- [ ] Visual sign-off

---

## 15. Non-goals / explicit rejects

Coder **không** được:

1. “Tối ưu” bằng cách `gc.disable()` trên UI thread.
2. Hạ poll xuống 1Hz để “đỡ CPU” mà không dirty-check (UX regression).
3. `table.clear()` mỗi metadata batch (đã từng là anti-pattern; giữ `update_cell`).
4. ProcessPool metadata trở lại chỉ vì “song song” (Windows spawn + PyInstaller cost —
   coordinator thread đã intentional).
5. Đổi `FUZZY_SCORE_CUTOFF` hoặc ranking scorer không có test.
6. Refactor `SkyPickerApp` property delegates “cho đẹp” trong PR perf (trừ khi chặn
   compile).

---

## 16. Order of work for a single agent session

Nếu một agent làm end-to-end:

1. Đọc §1–§3 + `PlaybackCard` / `_poll` / `_rerender` thực tế (line numbers có thể lệch —
   grep symbols, đừng tin số dòng mù).
2. Implement **PR-A only** → tests → stop for review nếu large.
3. **PR-B** (nhanh, independent).
4. **PR-C**.
5. **PR-D** chỉ khi user yêu cầu explicit.

Không open PR-D trong cùng session với A nếu chưa green A.

---

## 17. Appendix — Symbol index (grep targets)

```text
PlaybackCard
_rerender
_poll
_compose_lines
_playing_body
debug_stats
counters_snapshot
SnapshotRenderer
GradientHeader
ansi_gradient_box
Text.from_ansi
refresh_metadata_rows
_render_table
MetadataCoordinator
_warm_and_process_all_paths
rank_song_choices
run_sky_app_unified
start_playback
```

---

## 18. Document control

| Field | Value |
|---|---|
| Status | **Active** — ready for implementation |
| Supersedes | UI perf findings in ad-hoc chat 2026-07-17 (this file is SoT for work) |
| Related archive | `archive/live-dashboard-step2.*`, `archive/2026-07_ram-memory-hygiene-plan.md`, `archive/2026-06_ui-overhaul-textual-plan.md` |
| Does **not** supersede | `timing-principles.md`, `rt-dispatch-architecture.md` |

Khi toàn bộ PR-A–C (và D nếu làm) ship: chuyển file này sang `docs/archive/` với stamp
`ARCHIVED` + outcome table (layout/s before/after nếu đo được).

---

## 19. Cổng nghiệm thu toàn plan (owner)

- [ ] Playing: layout thrash hết (steady-state)
- [ ] Playing: progress/status vẫn mượt; F2 debug usable
- [ ] Picker: search/reload/metadata progressive vẫn đúng; không flicker row
- [ ] Multi-song session: không leak UI-visible (engine release vẫn chạy)
- [ ] `ruff` + `pyright` + UI pytest + `--selftest-textual` green
- [ ] Zero edits under `orchestration/core/` / `domain/scheduler*` / `platform/win32/inputs.py`
