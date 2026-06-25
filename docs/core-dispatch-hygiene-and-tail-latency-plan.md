# Core dispatch hygiene + tail-latency investigation plan (Python 3.14)

Status: PROPOSED — ready for an executing AI/engineer.
Owner of acceptance: Claude (nghiệm thu theo §8 gates).
Scope author context: viết sau 3 vòng phân tích lõi xử lý/lập lịch/gửi phím (đo thực nghiệm trên
3.14.3, máy dev). Plan này **bổ trợ** `docs/main-path-cleanup-and-build-quality-plan.md` (Pillar A/B/C ở
đó lo realtime switch-interval + tuning preset + build); plan này lo **vệ sinh mã mạch dispatch /
backend / scheduler-glue** còn lại + **một track đo tail-latency** để quyết định free-threaded.

> [!IMPORTANT]
> **Kết luận nền (đã đo, không tranh luận lại):** phần Python trên critical-span `spin-end → SendInput`
> có thể cắt được **≈ 0.6µs/note**, trong khi SendInput thật là **10–60µs** và jitter chi phối là
> **system-side** (telemetry: player metronomic ±45µs; `send-split-analysis-gate`, `player-dispatch-proven-metronomic`).
> ⇒ **KHÔNG mục nào trong §3–§4 cải thiện timing.** Chúng là *code quality* thuần. Ai thực thi phải
> trình bày đúng như vậy trong mô tả PR — cấm gắn nhãn "tối ưu hiệu năng".

---

## 0. Hard constraints (đọc trước khi sửa gì)

Kế thừa nguyên văn §0 của `main-path-cleanup-and-build-quality-plan.md`, cộng thêm:

- **KHÔNG đổi timing semantics.** Không sửa giá trị lead/floor/spin/onset, không đổi thứ tự dispatch,
  không đổi điều kiện feasibility same-key. Mọi edit ở §4 phải **output-identical** (chứng minh bằng
  test hiện có + telemetry `sender_clean=true`, 0 drop).
- **Tuân thủ `AGENTS.md`:** `SendInput` only; type hints bắt buộc; `@dataclass(frozen=True, slots=True)`
  cho domain model; `uv run` cho mọi lệnh; đổi nhỏ + có test; không rewrite rộng; không thêm dependency.
- **Phân tầng rủi ro bắt buộc:** §3 (không chạm runtime send/dispatch behavior) làm trước và độc lập.
  §4 (chạm mã trên send/dispatch path, dù behavior-preserving) là **OPTIONAL**, mỗi mục một commit,
  chỉ proceed sau khi §3 xanh và maintainer duyệt. §5 (đo tail) độc lập, có thể song song.
- **Evidence hierarchy** (INDEX.md §0): game-observed > telemetry > codebase > docs. Plan này là tài
  liệu — nếu mâu thuẫn code/telemetry thì code/telemetry thắng.

Mã nền tham chiếu (xác minh lại line trước khi sửa — code thắng doc):
- Backend gửi phím: `src/sky_music/infrastructure/backend.py`
  (`_TrackedKeyState` ~`:55-142`, `_decide_down/_decide_up` ~`:61-83`, `WinSendInputBackend` ~`:145-185`,
  `release_all` ~`:196`).
- Dispatch loop: `src/sky_music/orchestration/dispatch_loop.py`
  (`PlaybackState` ~`:49`, `DispatchHealthMonitor.record_input_path_send_duration` ~`:142-166`,
  `get_current_leads` ~`:232-237`, `_dispatch_down_batch` ~`:353`, `_dispatch_pending_releases` ~`:418`,
  `_drain_due` ~`:794`).
- Coordinator/scheduler-glue: `src/sky_music/orchestration/runtime_dispatch.py`
  (`pop_due_pending` ~`:198-218`).
- Engine: `src/sky_music/orchestration/engine.py`
  (`SendLatencyEstimator` ~`:44-101`, `_probe_timer_wake_error`/`probe_spin_threshold` ~`:301-353`,
  imports `:4`).
- Win32 send: `src/sky_music/platform/win32/inputs.py` (`ctypes.sizeof(INPUT)` ~`:374,:421`).
- Đo: `tests/measure_stutter.py` (after-send, cần audio), `scripts/audit_pipeline_bench.py` (structural,
  fake clock).

---

## 1. Evidence base (số đo thực, 3.14.3, GIL on, máy dev)

Dùng để xếp ưu tiên và để bác các đề xuất sai. Ai thực thi nên tái lập bằng một script bench nhỏ
(`scripts/` riêng, không commit vào hot path) trước khi tin.

| Thao tác | ns/op | Ghi chú |
|---|---:|---|
| `perf_counter_ns()` | 74 | |
| `perf_counter_ns() // 1000` (`now_us`) | 111–141 | `//1000` ~37ns |
| `ctypes.sizeof(INPUT)` mỗi send | 53 | cache còn 31 → tiết kiệm ~22ns/send (≈0.05% của SendInput) |
| `estimator.get_lead_us()` | 192 | `get_current_leads` = 2× = **378** |
| `_decide_down` 1 phím (`tuple([...])`×2) | 308 | `tuple(dict.fromkeys)` chiếm 239 |
| `_decide_down` single-pass loop | **499** (chord 3) vs 636 cũ | nhanh ~21% trên chord; **`tuple(genexpr)` = 1016, CHẬM 60% — cấm dùng** |
| spin iter `now_us()<target` vs `perf_counter_ns()<target` | 157 vs 87 | chỉ giảm overshoot ~70ns, bị 2–3µs dispatch sau nuốt → vô nghĩa |
| `dataclass` slots vs no-slots: 5 attr read | **89.2 vs 89.4** | đọc thuộc tính **không nhanh hơn** trên 3.14 — slots chỉ lợi memory |
| `try/except` không raise vs không | 42.9 vs 39.0 | ~4ns (noise) — zero-cost exceptions 3.11+ |

---

## 2. Scheduling — đã review, KHÔNG đổi (ghi để plan đầy đủ)

`compile_runtime_intents` (precompile timeline + generation pairing) và `RuntimeDispatchCoordinator`
(`pop_due_authored`, `split_down_intents`, no-early-conflict guard) đã được rà:
- Không có O(n²); schedule precompiled một lần; mọi thao tác per-drain là O(polyphony ≤ ~10).
- `next_pending_release_us`/`pop_due_pending` quét `min`/sort trên tập pending nhỏ (1–3) — vi mô, **không**
  đổi sang heapq (over-engineering cho N nhỏ; thêm bề mặt lỗi state).
- Onset = completion-anchor, lead đối xứng, guard chống early-pop: là **timing semantics đã chốt** →
  cấm chạm (§0).

**Quyết định: scheduling layer giữ nguyên.** Mọi việc dưới đây thuộc dispatch-glue/backend/typing.

---

## 3. Phase 1 — Hygiene KHÔNG chạm runtime send/dispatch behavior (LÀM TRƯỚC)

Mỗi mục một commit nhỏ. Không mục nào đổi giá trị runtime. Rủi ro thấp nhất.

### 3.1 — Modernize typing (3.14 best practice, 0 runtime cost)
`from __future__ import annotations` đã có ở các file này nên annotation là **string, không eval** —
đổi thuần thẩm mỹ/nhất quán, KHÔNG phải perf.
- `engine.py:4`: `from typing import Optional, Tuple` → bỏ; dùng `tuple[...]`, `X | None`. Sửa các chữ ký
  trong `PlaybackEngine.__init__` (`Optional[Clock]`→`Clock | None`, `Tuple[KeyAction, ...]`→`tuple[...]`, …).
- `dispatch_loop.py:4`: `Callable` đang import từ `typing` → chuyển sang `from collections.abc import Callable`.
  `dispatch_loop.py:16`: `Optional` → thay bằng `| None` tại các điểm dùng (`probe_callback`, `PlaybackState`
  field types nếu có).
- **Giữ** `from __future__ import annotations` (cần cho 3.11 forward-ref; xem bài lint sai khi đòi gỡ).
- Validate: `uv run pyright` sạch (không đổi nghĩa kiểu), `uv run ruff check .`.

### 3.2 — `SendLatencyEstimator.kind` dùng `Literal`/`ActionKind` thay `str`
`engine.py:74,92`: `def update(self, kind: str, ...)` / `get_lead_us(self, kind: str)` →
`kind: ActionKind` (`from sky_music.domain.scheduler_types import ActionKind`). Type safety, 0 runtime.
- **KHÔNG** đổi `if/elif` sang `match/case` (đo: không nhanh hơn; là khẩu vị; tránh churn vô ích).

### 3.3 — Gộp `_probe_timer_wake_error` + `probe_spin_threshold` (DRY)
`engine.py:301-353`: hai hàm gần trùng (10 mẫu sleep 2ms, `max`, clamp `max(300,min(3000,p_max+200))`),
chỉ khác telemetry key prefix (`probe_*` vs `reprobe_*`) và `enable_adaptive_spin` flag. Refactor:
```python
def _measure_spin_threshold(self, sleeper: Sleeper, *, prefix: str) -> int:
    wake_errors: list[int] = []
    for _ in range(10):
        t0 = self.clock.now_us()
        sleeper.sleep(0.002)
        wake_errors.append((self.clock.now_us() - t0) - 2_000)
    threshold = max(300, min(3_000, max(wake_errors) + 200))
    self.effective_spin_threshold_us = threshold
    # ... ghi telemetry theo prefix ...
    return threshold
```
Hai public method trở thành wrapper mỏng gọi helper với `prefix="probe"` / `"reprobe"`.
- **Giữ nguyên** telemetry key cũ (test phụ thuộc `probe_*` vs `reprobe_*` để phân biệt). Đo lại: clamp
  và giá trị threshold phải **y hệt** (đây là timing-adjacent — output-identical bắt buộc).
- Validate: `uv run pytest -k "probe or spin or reprobe"`.

### 3.4 — `PlaybackState` thêm `slots=True` (memory/consistency, KHÔNG phải tốc độ)
`dispatch_loop.py:49`: `@dataclass` → `@dataclass(slots=True)`. Class bị mutate (`update_pause_time`,
`rebase_epoch`) nên **không** `frozen`. Mọi attribute đã khai báo (`start_perf, pause_time_us,
manual_pause_started_us, focus_pause_started_us, epoch_us`); `__post_init__` set `epoch_us` — tương thích
slots. Lý do ghi trong commit: **đúng project-rule + chặn typo gán nhầm attribute**, KHÔNG phải tốc độ
(đo: read 89.2 vs 89.4ns = bằng nhau).
- Rủi ro: nếu có chỗ gán attribute động ngoài danh sách field → AttributeError. Grep `state\.` để chắc
  không ai set field lạ. Validate: `uv run pytest -k "playback_state or dispatch or pause or focus"`.

### 3.5 — `release_all`: bỏ `import time` cục bộ
`backend.py:196`: trong `release_all` có `import time` trong khi module đã `import time` ở `:3`. Xóa dòng
cục bộ (no-op gây nhầm). Đây là cold path (panic/pause/end), 0 ảnh hưởng hot. Validate: pytest backend.

### 3.6 — (tùy chọn) `_TrackedKeyState`/`WinSendInputBackend` `__slots__`
`backend.py:55`: base dùng annotation trần (KHÔNG tạo class attr — không có shared-state, xem §6.B).
Thêm `__slots__` cho `_TrackedKeyState` + cả subclass (`WinSendInputBackend`, `DryRunBackend` có thêm
`inputs_module/_send_fn/...`, `history`). Lợi **memory** (instance đơn → gần vô nghĩa) — xếp **thấp**,
chỉ làm nếu muốn đồng bộ phong cách. Cẩn thận liệt kê đủ slot cho mọi field mỗi subclass.

---

## 4. Phase 2 — OPTIONAL: chạm mã trên send/dispatch path (behavior-preserving)

> [!WARNING]
> Các mục này sửa code **trên** send/dispatch path mà §0 của plan kia coi là frozen. Net perf ≈ 0
> (đã đo). Chỉ proceed nếu: (a) §3 đã merge xanh, (b) maintainer đồng ý đánh đổi "diff trên frozen path"
> lấy "code sạch hơn", (c) chứng minh **output-identical**. Nếu phân vân → **bỏ qua cả §4**.

### 4.1 — `_decide_down`/`_decide_up`: single-pass thay `tuple([...])`×N
`backend.py:61-83`. Hiện duyệt `unique_scan_codes` 2 lần (down) / 3 lần (up).
```python
def _decide_down(self, scan_codes):
    active = self.active_keys
    to_send: list[int] = []
    duplicates: list[int] = []
    for sc in dict.fromkeys(scan_codes):   # giữ nguyên dedup + thứ tự insert
        (duplicates if sc in active else to_send).append(sc)
    return tuple(to_send), tuple(duplicates)
```
- **CẤM** `tuple(genexpr)` (đo: chậm 60%). Single-pass: chord 3 phím 636→499ns.
- Output **phải** y hệt: cùng dedup (insertion-order của `dict.fromkeys`), cùng phân nhóm. Fast-path
  `if not self.active_keys` cũ có thể giữ hoặc bỏ (loop tự đúng khi `active` rỗng) — nếu bỏ, xác nhận
  test bao phủ ca rỗng.
- Validate: `uv run pytest tests/ -k "backend or decide or duplicate or tracked"`; chạy 1 playback thật
  → telemetry `sender_clean=true`, `backend_skipped_*` không đổi so với baseline.

### 4.2 — Xóa recompute `get_current_leads` thừa trong dispatch helpers
`dispatch_loop.py`: `_drain_due` (~`:801`) đã tính `lead_down/lead_up`, nhưng `_dispatch_down_batch`
(~`:400`) và `_dispatch_pending_releases` (~`:454`) gọi LẠI `get_current_leads()` chỉ để lấy
`applied_lead_us` cho telemetry (378ns dead/note). Truyền lead xuống thay vì gọi lại:
- Đổi chữ ký `_dispatch_down_batch(self, batch, state, *, lead_down)` và
  `_dispatch_pending_releases(self, releases, state, *, lead_up)`; `_drain_due` truyền giá trị đã có.
- `applied_lead_us` phải **y hệt** giá trị cũ (cùng nguồn estimator, cùng thời điểm — thực tế chặt hơn vì
  bỏ khả năng estimator đổi giữa 2 lần gọi). Đây là telemetry field, không đổi dispatch behavior.
- Validate: `uv run pytest -k "dispatch or lead or telemetry"`; so summary `applied_lead_us` stats vs
  baseline trên cùng song/seed (deterministic fake clock qua `audit_pipeline_bench.py`).

### 4.3 — (rất thấp) cache `ctypes.sizeof(INPUT)` + bind `SendInput`
`inputs.py:374,421`: `ctypes.sizeof(INPUT)` mỗi send (53ns) → hằng module `_INPUT_SIZE`. Bind
`_SendInput = user32.SendInput` ở module-level bỏ attr lookup/lần. Tiết kiệm ~22ns/send ≈ **0.05%** của
SendInput → **dưới noise, gần như không đáng**. Chỉ làm kèm nếu đã mở `inputs.py` vì lý do khác.
- Cẩn thận: `_send_scan_code_batch_impl` và `send_input_batch` đều dùng — sửa cả hai. Test monkeypatch
  `user32`/`SendInput` trong suite có thể vỡ nếu bind cứng ở import-time → kiểm `rg -n "SendInput" tests/`
  trước; nếu test patch module attr, **giữ** `user32.SendInput` (đừng bind) và chỉ cache sizeof.

---

## 5. Phase 3 — Track đo tail-latency: free-threaded 3.14t (đòn bẩy THẬT duy nhất)

Mục tiêu: trả lời **bằng số** câu "free-threaded `python3.14t` có cắt được đuôi max-lateness do GIL
handoff giữa Textual UI và dispatch thread không?" — thứ duy nhất còn có thể chạm jitter thật. Đây là
**điều tra**, kết quả có thể là "không đáng, đóng lại".

### 5.1 — Vì sao có thể có đuôi (giả thuyết cần kiểm)
Trong spin window (GIL giữ), nếu spin > switch-interval (đã tune 1ms; reprobe có thể nâng tới 3ms) thì
UI thread có thể giành GIL một lát ngay sát mốc → kéo dài onset tối đa ~1 switch-interval ở ca xấu.
SendInput đã nhả GIL qua ctypes nên p50/p99 không bị; nghi ngờ chỉ ở **max/p99.9**. Free-threaded triệt
tiêu lớp này nhưng đánh đổi ~5–10% overhead single-thread + cần wheel free-threaded cho `rapidfuzz`,
`textual`.

### 5.2 — Harness cần dựng (`scripts/measure_dispatch_tail.py`, mới)
`measure_stutter.py` cần audio thật; `audit_pipeline_bench.py` dùng fake clock (không phản ánh GIL
contention). Cần harness mới đo **dispatch lateness phân phối dưới tải UI song song, KHÔNG inject phím**:
1. Backend giả `SyntheticLatencyBackend(InputBackend)`: thay vì `SendInput`, busy-spin/`sleep` một khoảng
   lấy từ phân phối send_duration thật trong telemetry đã thu (p50≈477µs in-game; tail p99≈953µs,
   max≈1695µs — `send-split-analysis-gate`). KHÔNG gọi Win32 → an toàn, không bắn phím.
   - Lưu ý: `_should_use_dispatch_thread` loại `DryRunBackend` theo tên class → đặt tên khác để vẫn chạy
     thread thật (`PerfCounterClock` + `RealSleeper`).
2. Một thread "UI load" mô phỏng Textual render: vòng lặp CPU-bound Python (vd cập nhật dict/format
   string) ở nhịp ~10–60Hz để tạo GIL contention tương đương dashboard live.
3. Chạy engine thật (event_wait, adaptive lead/spin, MMCSS auto) trên một song dày note; thu `lateness_us`,
   `visible_lateness_us`, `dispatch_lateness_us` từ telemetry (`--debug-csv`).
4. Output: p50/p95/p99/**p99.9/max** của 3 metric, kèm `idle_gap`/`pre_send_spin`.

### 5.3 — Ma trận chạy (cùng máy, cùng song/seed, ≥5 run mỗi ô, lấy median của max)
| Build | UI load | switch-interval |
|---|---|---|
| stock 3.14 (GIL) | off | default (1ms tuned) |
| stock 3.14 (GIL) | on | 1ms |
| stock 3.14 (GIL) | on | 5ms (đối chứng) |
| `python3.14t` (free-threaded) | on | n/a (skip nhờ §A1 plan kia) |

Cài 3.14t qua `uv python install 3.14t` (hoặc tương đương) trong môi trường **riêng**; cần wheel
free-threaded cho `textual`/`rapidfuzz` — nếu thiếu, ghi nhận "blocked by ecosystem" và dừng (đó cũng là
một kết luận hợp lệ).

### 5.4 — Tiêu chí quyết định
- Nếu `(GIL,on,1ms).max − (3.14t).max` **≥ ~500µs lặp lại được** và 3.14t không làm xấu p50/p99 quá
  ngưỡng khó chịu → mở plan riêng đánh giá chuyển free-threaded (build artifact thứ 2 — KHÔNG thuộc plan
  này; xem out-of-scope plan kia §5).
- Nếu chênh < ~100µs hoặc trong noise → **đóng**: ghi vào `docs/timing-experiments.md` rằng GIL handoff
  không phải nguồn đuôi đáng kể; switch-interval-tuning hiện tại là đủ. Đây là kết quả **kỳ vọng** dựa
  trên p99 80–104µs hiện có.

### 5.5 — Không làm trong track này
Không dựng pipeline build 3.14t, không sub-interpreters (PEP 734), không đổi default. Chỉ **đo + kết luận**.

---

## 6. REJECT list — đề xuất KHÔNG được thực hiện (kèm bằng chứng)

Các đề xuất từng được nêu (bài lint bên ngoài) nhưng **sai sự thật / có hại**; ghi ở đây để người sau
không lặp lại:

- **A. `int(round(0.95*(L-1)))` → `int(0.95*(L-1))`** ("round dư thừa"): **REJECT — đổi hành vi.** Đo:
  L=8 → 7 vs 6; **L=64 → 60 vs 59** (deque `maxlen=64`, chạy mỗi send vì `input_path_warn_us=3000`). Đây
  là đổi index ngưỡng p95 của bộ phát hiện input-path-degraded, không phải cleanup. Giữ nguyên.
  (`dispatch_loop.py:159`).
- **B. "`_TrackedKeyState` annotation trần là class-var → shared state giữa instances"**: **REJECT — sai
  semantics.** `active_keys: set[int]` không gán → chỉ vào `__annotations__`, KHÔNG tạo class attribute,
  KHÔNG có shared mutable state. Quên init → `AttributeError` (đã init đúng trong `__init__`). Không cần
  "sửa". (Việc thêm `__slots__` ở §3.6 là vì memory, không vì nguy cơ bịa này.)
- **C. Cache `state.get_elapsed_us` xuyên vòng `_wait_until_runtime_deadline`**: **REJECT — phá vòng
  chờ.** Các lần đọc đó cách nhau bởi `wait_until_us`/poll (thời gian đã trôi); cache lại sẽ chờ sai.
  Không phải redundancy.
- **D. `tuple([listcomp])` → `tuple(genexpr)`**: **REJECT** — đo chậm **60%** (1016 vs 636ns). Nếu làm
  §4.1 thì dùng **single-pass loop**, không phải genexpr.
- **E. Tối ưu spin loop (`now_us`→`perf_counter_ns` trong `spin_until_us`)**: **REJECT (vô giá trị).**
  Spin là busy-wait tới mốc cố định; iter rẻ hơn chỉ giảm overshoot ~70ns, bị 2–3µs dispatch sau nuốt.
- **F. `try/except` trong `_emit` là "hot path overhead"**: **REJECT lý do** — zero-cost exceptions 3.11+
  (đo 4ns noise). Có thể đơn giản hóa logic re-resolve `_send_fn_module` cho dễ đọc, nhưng KHÔNG vì perf
  và KHÔNG được đổi hành vi monkeypatch (test thay `inputs_module` dựa vào nhánh re-resolve).
- **G. `heapq` cho `pending_by_generation`**: **REJECT** — N≤~10, sort 1–3 phần tử là noise; thêm cấu
  trúc = thêm bề mặt lỗi state.
- **H. Đổi `if/elif` sang `match/case` "để nhanh hơn"**: **REJECT lý do** — không nhanh hơn; chỉ khẩu vị,
  bỏ để tránh churn.

---

## 7. Thứ tự thực thi & PR boundaries

1. **PR-1 (§3.1–3.3, 3.5):** typing modernize + estimator Literal + probe DRY + bỏ import time. Không
   chạm runtime behavior. Diff trung bình, dễ review.
2. **PR-2 (§3.4, tùy chọn 3.6):** `PlaybackState slots` (+ backend `__slots__` nếu muốn). Tách riêng vì
   slots có rủi ro AttributeError cần test riêng.
3. **PR-3 (§4, OPTIONAL):** chỉ khi maintainer duyệt chạm frozen path. Mỗi mục (4.1 / 4.2 / 4.3) một
   commit, kèm chứng minh output-identical + 1 playback thật telemetry sạch.
4. **PR-4 (§5):** harness `measure_dispatch_tail.py` + báo cáo kết quả vào `docs/timing-experiments.md`.
   Không đổi code sản phẩm.

Mỗi PR: `uv run pytest`, `uv run ruff check .`, `uv run pyright` xanh (AGENTS.md). Diff nhỏ, focused.

---

## 8. Acceptance gates (Claude nghiệm thu)

- **G-1 (typing/DRY):** `pyright`/`ruff` sạch; `engine.py`/`dispatch_loop.py` không còn `Optional`/`Tuple`
  từ `typing`; `Callable` từ `collections.abc`; `from __future__ import annotations` GIỮ nguyên.
- **G-2 (probe DRY):** telemetry key `probe_*`/`reprobe_*` không đổi; giá trị `effective_spin_threshold_us`
  trên cùng input **y hệt** trước/sau; test probe/reprobe pass.
- **G-3 (slots):** `PlaybackState` có `slots=True`; full suite pass; không AttributeError ở pause/focus/
  rebase paths.
- **G-4 (§4 nếu làm):** với cùng song qua `audit_pipeline_bench.py` (fake clock, deterministic),
  chuỗi dispatch (scan codes, kind, thứ tự, `sent/skipped`, `applied_lead_us`) **byte-identical** trước/sau;
  1 playback thật → `sender_clean=true`, `before_send_missing_down_count=0`, drop counts không đổi.
- **G-5 (tail):** `measure_dispatch_tail.py` chạy được, sinh bảng p50…max cho ma trận §5.3; báo cáo +
  quyết định (mở plan free-threaded HAY đóng) ghi vào `docs/timing-experiments.md` với số cụ thể.
- **G-regress:** `uv run pytest`, `uv run ruff check .`, `uv run pyright` đều sạch ở mọi PR.
- **G-no-perf-claim:** mọi mô tả PR của §3–§4 nêu rõ "code quality, net timing ≈ 0 (đo §1)", không gắn
  nhãn tối ưu hiệu năng.

---

## 9. Risks & rollback

- **R1 — diff trên frozen send-path (§4) gây hồi quy tinh vi.** Mitigation: output-identical gate G-4 +
  telemetry sạch; rollback = revert commit đơn lẻ (mỗi mục tách commit).
- **R2 — `slots=True` vỡ vì gán attribute động ngoài field.** Mitigation: grep `state\.<attr>` toàn repo
  trước; G-3 full suite. Rollback: bỏ `slots`.
- **R3 — bind `SendInput`/sizeof (§4.3) vỡ test monkeypatch.** Mitigation: kiểm `rg -n "SendInput" tests/`
  trước; nếu test patch attr thì chỉ cache sizeof. Rollback: revert.
- **R4 — 3.14t thiếu wheel `textual`/`rapidfuzz`.** Không phải lỗi: ghi "blocked by ecosystem", đóng track
  với kết luận tạm.
- **R5 — over-reach:** ai đó coi plan này là "giấy phép tối ưu send-path". KHÔNG. §0 + §6 là rào.

---

## 10. Decisions (điền khi thực thi)

- [ ] §4 (frozen-path hygiene) có được maintainer duyệt làm không? (mặc định: KHÔNG, vì net perf ≈ 0).
- [ ] §3.6 backend `__slots__`: làm hay bỏ (chỉ lợi memory instance đơn).
- [ ] §4.3 cache sizeof/bind SendInput: test có patch `user32.SendInput` không? → quyết định bind hay chỉ
  cache.
- [ ] §5 kết quả tail: mở plan free-threaded hay đóng? (số cụ thể + ngày).
- [ ] Cập nhật `docs/INDEX.md` §2 trỏ tới plan này khi bắt đầu thực thi.
