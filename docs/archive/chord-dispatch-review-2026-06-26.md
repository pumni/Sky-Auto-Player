# Đánh giá kiến trúc lõi & đường gửi phím hợp âm

> **Ngày:** 2026-06-26
> **Phạm vi:** Luồng xử lý từ `uv run play` → scheduler → coordinator → dispatch loop → backend `SendInput`.
> **Trọng tâm:** Tính đồng thời của hợp âm, anti-pattern, dead code, mức tối ưu theo Python 3.14 (free-threaded).
> **Trạng thái:** Báo cáo đánh giá — chưa thực hiện thay đổi mã nguồn nào.

---

## 1. Kiến trúc & luồng từ `uv run play`

`play` ánh xạ tới `main:main` (`pyproject.toml:41`). Chuỗi thực thi:

```
main()  src/main.py
 ├─ parse args → configure_from_args → RUNTIME_STATE (+ mirror các global "legacy")
 ├─ enable_high_precision_timers()       winmm.timeBeginPeriod(1)
 ├─ chọn bài (Textual picker) → PlaybackOverrides
 └─ play_selected_song → PlaybackEngine.play()          orchestration/engine.py
       ├─ build_key_actions()         domain/scheduler.py        (XÂY timeline)
       ├─ compile_runtime_intents()   orchestration/runtime_dispatch.py  (gắn generation/key)
       ├─ RealtimeProcessScope (GC pause + GIL switch-interval tuning)
       ├─ PlaybackSupervisor.run()    orchestration/playback_supervisor.py
       │     └─ thread "sky-music-dispatch"  ← real-time
       │           DispatchLoop.run()  orchestration/dispatch_loop.py  (wait → drain → execute)
       │                 └─ WinSendInputBackend.key_down/up   infrastructure/backend.py
       │                       └─ inputs.send_scan_code_batch_trusted → user32.SendInput
       └─ control thread: poll hotkeys, render Textual HUD
```

**Nhận xét:** Phân tách trách nhiệm sạch và đúng với `AGENTS.md`:
- **scheduler thuần** (không phụ thuộc wall-clock, unit-test được) →
- **coordinator** quản lý trạng thái per-key generation và điều kiện release →
- **dispatch loop** lo timing (wait → drain → execute) →
- **backend** cô lập hoàn toàn sau interface `InputBackend` (Protocol).

---

## 2. Đánh giá đường gửi phím hợp âm (yêu cầu trọng tâm)

**Kết luận: thiết kế đúng và đạt mức đồng thời tối đa mà Windows cho phép.**

Một hợp âm đi qua **một lời gọi `SendInput` duy nhất** với mảng nhiều struct `INPUT`:

1. `domain/scheduler.py:277-295` — gom sự kiện theo `(at_us, kind)`: các nốt cùng mốc thời gian + cùng loại gộp thành **một** `KeyAction` với `scan_codes` là tuple.
2. `orchestration/dispatch_loop.py:440-446` — `_dispatch_down_batch` dựng **một** `KeyAction` cho toàn bộ intent "playable" → **một** `key_down`.
3. `infrastructure/backend.py:170-172` → `inputs.send_scan_code_batch_trusted` → `platform/win32/inputs.py:425` `user32.SendInput(n, input_array, ...)` — **một** syscall.

Đây là cách đúng: `SendInput` với một mảng được Windows serialize nguyên khối, không thread nào chen input vào giữa lô. Phần release cũng được gộp (`_dispatch_pending_releases`, `dispatch_loop.py:462`) → một `key_up` cho mọi phím đến hạn cùng lúc.

**Các điểm thiết kế tốt hỗ trợ tính đồng thời:**
- Cache `_INPUT_CACHE` / `_ARRAY_CACHE` (`inputs.py:400-423`) loại bỏ chi phí dựng struct ctypes lặp lại — chi phí Python lớn nhất cho hợp âm.
- Adaptive lead **bucket theo polyphony** (`engine.py:43-107`, `SendLatencyEstimator`): hợp âm 5 phím được "bắn sớm" nhiều hơn nốt đơn để bù độ trễ `SendInput` dài hơn → căn để phím **hiển thị** đúng mốc đã định. Có cả mô hình tuyến tính `send ≈ a + b·N` để ngoại suy cho cỡ hợp âm hiếm gặp.
- Sort up-trước-down tại cùng mốc thời gian (`scheduler.py:298`) tránh đè phím khi một nốt nhả và nốt khác bấm cùng thời điểm.

### Các điểm có thể làm SUY GIẢM tính đồng thời — cần lưu ý

- **Retry khi `SendInput` gửi thiếu** (`inputs.py:429-436`): nếu `SendInput` trả về `sent < n` (partial send), phần phím còn lại được gửi bằng **lời gọi `SendInput` thứ hai** qua `send_input_batch`, có thể kèm `_retry_wait_seconds(0.002)` → hợp âm **bị tách**, lệch tới ~2ms. Trường hợp này hiếm (chỉ khi input bị chặn/UIPI mismatch), nhưng đây là **chỗ duy nhất** phá vỡ tính nguyên khối của hợp âm. → **Đề xuất:** thêm telemetry/cảnh báo rõ ràng khi partial-send xảy ra.
- **Không có "cửa sổ lượng tử hóa" (quantization window) cho hợp âm**: việc gom chỉ theo `at_us` **bằng nhau tuyệt đối** (`scheduler.py:279`). Nếu dữ liệu nguồn có jitter vài µs giữa các nốt mà người soạn coi là một hợp âm, chúng sẽ **tách thành nhiều `SendInput`**. Với sheet Sky chuẩn (timestamp trùng khít) thì không sao, nhưng đây là giả định ngầm, không được bảo vệ bằng tolerance. → **Đề xuất:** cân nhắc tùy chọn `chord_quantization_us`.

---

## 3. Anti-pattern

| # | Vị trí | Vấn đề |
|---|--------|--------|
| A1 | `main.py:113-131` `_sync_legacy_runtime_globals` | **14 global mirror** từ `RUNTIME_STATE` tồn tại **chỉ để test assert** (`main.TIMING_POLICY`, `main.SLEEP_POLICY`, `main.USE_DISPATCH_THREAD`, …). Production không đọc chúng (xác minh bằng grep — chỉ thư mục `tests/` đọc). Vi phạm trực tiếp mandate "Avoid globals in new code". Nên cho test assert trên `RUNTIME_STATE` rồi xóa mirror. |
| A2 | `dispatch_loop.py` (vd. dòng 554, 886) | Tham số hot-path khai báo `Any`: `command_source: Any`, `focus_signal: Any`, `progress_sink: Any` dù đã có Protocol `CommandSource` / `FocusSignal` / `ProgressSink` ngay trong `playback_supervisor.py:28-51`. Mất type-safety vô cớ — nên import qua `TYPE_CHECKING`. |
| A3 | `dispatch_loop.py:213,234` | `estimator: Any = _NullEstimator()` rồi lại `estimator if estimator is not None else _NullEstimator()`, trong khi `engine.py:390` truyền `None` để… được default lại. Vòng vo; truyền thẳng `_NullEstimator()` hoặc bỏ nhánh re-default. |
| A4 | `dispatch_loop.py:518` | `tuple(g_id for g_id in gen_ids_list)` ≡ `tuple(gen_ids_list)`. Generator thừa. |
| A5 | `backend.py:218-310` | ~8 khối `try: debug_log(...) except Exception: pass` lồng nhau trong `release_all`. Nên có helper `_safe_debug_log` thay vì lặp lại pattern nuốt lỗi. |
| A6 | `engine.py:408-411` & `dispatch_loop.py:270-278` | Logic mean/variance/stdev **chép tay 2 lần**. Còn `import math as _math` cục bộ (`engine.py:397`) trong khi nơi khác dùng `math` module-level. Nên dùng `statistics.fmean` / `pstdev`, gỡ import cục bộ. |
| A7 | `scheduler.py:298` | `sort(key=lambda a: (a.at_us, a.kind == "down"))` dựa vào `False < True` để xếp up-trước-down — đúng nhưng ẩn ý (bool-as-int smell). Có comment kèm, nhưng vẫn nên cân nhắc khóa sắp xếp tường minh. |

---

## 4. Dead code

- **`deferred_by_us`** (`dispatch_loop.py:333`, được truyền tại `:521`): tính ở `_dispatch_pending_releases:502` và truyền vào `_execute_action`, nhưng **thân `_execute_action` (336-393) không hề dùng**. Giá trị chỉ cần để chọn `runtime_outcome` (dòng 519) — việc truyền vào hàm là chết. Gỡ tham số.
- **`command_event`** trong `_process_wait_states` (`dispatch_loop.py:617`): khai báo nhưng thân hàm không tham chiếu; được luồn qua `_service_control_state` rồi rơi vào ngõ cụt. Chuỗi tham số chết (nó chỉ thực sự được dùng tại `wait_strategy.wait_until_us`, đường khác — `dispatch_loop.py:797-804`).
- **Các global ở A1** (`TIMING_POLICY`, `SLEEP_POLICY`, `VERBOSE_HUD`, `USE_DISPATCH_THREAD`, `ENABLE_TIMER_GUARD`, `ENABLE_WAITABLE_TIMER`, `ENABLE_GC_PAUSE`): chết về mặt production (chỉ test đọc).

---

## 5. Chưa tối ưu theo Python 3.14 (free-threaded)

**Phần tận dụng free-threaded đã tốt:**
- `RealtimeProcessScope` (`realtime.py:123-130`) đúng đắn **bỏ qua** tuning switch-interval khi không có GIL.
- `_ns_based` (`wait_strategy.py:44`) né phép chia `// 1000` trong vòng spin nóng bằng cách so sánh trực tiếp `perf_counter_ns()`.
- Lý do pin `3.14+freethreaded` (tách thread spin khỏi thread Textual để khử tranh chấp GIL) hợp lý và được tài liệu hóa trong `pyproject.toml`.

**Điểm còn thiếu / nên cải thiện:**
- **`pyproject.toml:14` `requires-python = ">=3.11,<3.15"`** mâu thuẫn với `.python-version = 3.14+freethreaded` và `AGENTS.md` ("Python 3.14.3"). Hệ quả thực tế: **ruff auto-detect target = py311** (không có `[tool.ruff] target-version`), nên các idiom 3.12–3.14 không được lint/upgrade. → Đặt `[tool.ruff] target-version = "py314"` và siết cận dưới `requires-python` nếu thực sự chỉ chạy 3.14.
- **Vòng spin thuần Python** `while time.perf_counter_ns() < tgt_ns: pass` (`wait_strategy.py:46`): trên free-threaded chiếm trọn 1 core. Đây là chủ đích cho độ chính xác và hiện chấp nhận được vì chỉ có 1 dispatch thread.
- Manual variance ở A6 có thể dùng module `statistics` — thuần clean-code (không nằm hot-path).

---

## 6. Khuyến nghị ưu tiên

1. **(An toàn hợp âm)** Thêm telemetry/cảnh báo rõ khi `SendInput` partial-send xảy ra (`inputs.py:429`) — điểm duy nhất phá vỡ tính đồng thời. Cân nhắc tùy chọn `chord_quantization_us` ở scheduler để gom nốt lệch vài µs.
2. **(Clean code, rủi ro thấp)** Gỡ `deferred_by_us` và `command_event` chết; sửa A3/A4. Bắt đầu từ nhóm này vì có thể giữ test xanh ngay.
3. **(Kiến trúc)** Thay mirror global (A1) bằng assert trên `RUNTIME_STATE`; thay `Any` bằng Protocol (A2).
4. **(Config)** Đồng bộ `requires-python` với `.python-version` và thêm `ruff target-version = "py314"`.

---

## 7. Đào sâu: mô hình polyphonic linear lead (commit `712c02e`)

> Bối cảnh: phiên `712c02e feat(orchestration): add per-batch lead and polyphonic estimator`
> (25/06/2026) đã thêm bucket theo polyphony + mô hình tuyến tính cho `SendLatencyEstimator`.
> Plan doc gốc (`docs/chord-polyphony-lead-and-nogil-plan.md`) đã bị gỡ khỏi cây hiện tại; test
> `tests/test_adaptive_lead.py` còn nguyên và phủ rất kỹ ý đồ thiết kế.

### 7.1. Có HAI bài toán "đồng thời" — linear giải bài toán thứ hai

- **(a) Đồng thời *nội bộ* một hợp âm**: các phím cùng hợp âm chạm game cùng lúc → giải bằng **một `SendInput`** (mục 2). Lead **không** ảnh hưởng tới (a).
- **(b) Đồng thời *giữa các sự kiện trên cùng nhịp***: hợp âm 5 phím và nốt đơn, dù soạn ở cùng mốc, sẽ **chạm game lệch nhau** nếu cùng *bắt đầu* `SendInput` tại mốc đó, vì syscall 5 phím **hoàn tất muộn hơn** syscall 1 phím. → Đây là bài toán mà polyphonic linear sinh ra để giải.

Mô hình linear **không** làm hợp âm "đồng thời hơn với chính nó", mà làm **mọi cỡ sự kiện cùng đáp đúng nhịp của nó**, để hợp âm lớn không tụt nhịp hệ thống so với nốt đơn (groove đều).

### 7.2. Cơ chế áp lead — "onset = thời điểm hoàn tất dispatch"

- `runtime_dispatch.py:170` `next_authored_us` trả `max(0, scheduled_us − lead)`; với `lead_for_batch` thì lead = `_down_lead_for_batch(batch)` (`dispatch_loop.py:255-263`), tính theo **đúng số phím** (`len(batch.intents)`).
- Vòng wait tới deadline-đã-dời-sớm rồi mới bắn → `SendInput` **khởi phát** tại `scheduled − send_duration_dự_đoán`, **hoàn tất** tại `scheduled`.
- Test `test_dispatch_completion_lands_on_schedule_with_warm_estimator` chốt: estimator ấm + send-duration cố định → mọi completion rơi đúng timestamp.

### 7.3. Đúng đắn mô hình `send ≈ a + b·N`

- **Guard `denom <= 0`** (`engine.py:168-170`): mọi mẫu chung một polyphony → độ dốc không xác định → trả `None`. Đúng toán học; test `test_estimator_nearest_bucket_when_linear_undefined`.
- **Chuỗi fallback** (`engine.py:175-197`): bucket ấm → ngoại suy linear → bucket seeded ≤ N → EMA tổng → 0. Năm test `test_estimator_*` phủ từng nhánh.
- **Warm-start** (`engine.py:118-125`): cỡ hợp âm lần đầu được seed từ linear rồi fold mẫu thật (`0.2·sample + 0.8·linear`); test `test_estimator_warm_start_uses_first_sample` (840 = 0.2·1000 + 0.8·800). Đây là giá trị thật của lớp linear: hợp âm hiếm được led đúng ngay lần đầu.

### 7.4. Điểm cần soi (rủi ro/khiếm khuyết thật)

| # | Vị trí | Vấn đề |
|---|--------|--------|
| L1 | `dispatch_loop.py:247-253` `get_current_leads` | `lead_down` tính với `n_keys=1` mặc định nhưng **luôn bị `lead_for_batch` ghi đè** ở `run()` (`:906`) và `_drain_due` (`:865`); chỉ còn dùng làm `applied_lead_us` telemetry. Chỉ `lead_up` thực sự được tiêu thụ. Hàm trả về một nửa giá trị bị vứt → đáng dọn. |
| L2 | `dispatch_loop.py:252` & `:262-263` | `onset_bias_us` được cộng ở **hai đường** (`get_current_leads` + `_down_lead_for_batch`). Đường thực thi là cái sau; cộng hai nơi là bẫy bảo trì. |
| L3 | `engine.py:143-148,158-173` | Bất đối xứng "quên": bucket EMA thích nghi (α=0.2) còn tích lũy linear **nhớ trọn đời** (không decay) → linear là trung bình vòng đời, tụt hậu nếu độ trễ máy trôi. Tác động bị chặn (linear chỉ dùng warm-start/cỡ chưa thấy). Số học an toàn với float64. |
| L4 | `engine.py:63` `_MAX_POLY = 6` | Hợp âm ≥7 phím dùng lead bucket 6 → under-led. Sky 15 phím về lý thuyết có thể vượt 6; repertoire thực thì hiếm. Trần ngầm cần biết. |
| L5 | `engine.py:129` (seed 5 mẫu) | Cold-start: 5 sự kiện đầu mỗi bucket cho lead = 0 → **hợp âm đầu của mỗi cỡ vẫn đáp muộn** đúng bằng send-duration, cho tới khi warm-start (cần ≥2 cỡ seeded) hoặc đủ 5 mẫu. Đánh đổi cố ý ("cold estimate tệ hơn không có"). |
| L6 | bản chất dự đoán | Lead dùng lịch sử, không phải đo of-this-send → jitter send-duration để lại lateness dư; EMA làm mượt, `visible_lateness_us` đo phần còn lại. Không sửa được về bản chất. |
| L7 | `engine.py` `max_lead_us=2_000` | Trần 2ms so với send thực ~vài chục µs → gần như **không bao giờ chạm** ngoài điều kiện bệnh lý (test phải cố tình bơm 5000µs). Hiểu là lan can an toàn, không phải knob tinh chỉnh nóng. |

### 7.5. Lớp linear có "đáng đồng tiền"?

Cửa sổ hữu ích = **lần đầu xuất hiện mỗi cỡ hợp âm** + cỡ chưa từng thấy. Bài thực có rất ít cỡ phân biệt (≈1–6), nên sau dăm nhịp đầu mọi bucket đã ấm và linear gần như không còn được gọi → payoff tập trung ở đầu bài. Chi phí kiểm soát tốt (tích lũy O(1), 5 test). **Không overengineering tới mức nên gỡ**, nhưng là lớp "biên" lợi ích hẹp. Thứ đáng dọn là **rác xung quanh** (L1 + L2), không phải bản thân mô hình.

---

## 8. Đào sâu: khoảng trễ "tới deadline-đã-led → `SendInput` thực sự chạy"

### 8.1. Dòng thời gian thực của một lần bắn

```
spin kết thúc tại target_system_us (= epoch + (scheduled − lead))   wait_strategy.spin_until_us
        │   ← KHOẢNG TRỐNG: prologue Python thuần (CHƯA được bù)
        ▼
_wait_until_runtime_deadline return → unpack tuple                  dispatch_loop.py:910-921
now_us = state.get_elapsed_us(clock)                                :925  (đọc clock)
_drain_due:                                                         :851
   get_current_leads()           ← đọc estimator (lần 2)            :858
   pop_due_pending(...)          ← duyệt dict pending               :860
   pop_due_authored(..., _down_lead_for_batch)  ← đọc estimator/batch   :864
   _down_lead_for_batch(batch)   ← đọc estimator (lần 3, cùng batch)    :876
   _dispatch_down_batch:
       split_down_intents(...)   ← duyệt intents + set membership   :420
       build KeyAction(tuple(...))                                  :440
       _execute_action:
           send_start_raw = clock.now_us()    ← đọc clock           :336
           send_start_us  = get_elapsed_us()  ← đọc clock           :337
           backend.key_down(scan_codes) ───────► SendInput   ◄── BẮN THẬT
```

### 8.2. Lead bù `send_duration`, KHÔNG bù prologue

`estimator.update("down", result.send_duration_us, …)` (`:453`) học **chỉ thời lượng lời gọi backend** (`send_end_us − send_start_us`). Toàn bộ **prologue** giữa spin-end và `send_start` **không được lead bù**. Định lượng:

- spin xong khi `elapsed = scheduled − lead`;
- `send_start ≈ (scheduled − lead) + prologue`;
- completion `≈ scheduled − lead + prologue + send_duration`;
- vì `lead ≈ send_duration` → **`visible_lateness ≈ prologue`**.

Độ trễ dư game thấy = đúng độ dài prologue. Spin căn ke rất sắc (ns-based, `wait_strategy.py:46`); vấn đề nằm ở **công việc Python sau spin**.

### 8.3. Bù prologue đã có knob sẵn — `onset_bias_us`

`--onset-bias-us` ("additive fixed lead applied to key-down/onset dispatches only") chính là để quay tay phần bias prologue. `_down_lead_for_batch` cộng `onset_bias_us` cho down (`:262-263`). Vậy residual ở 8.2 **không phải lỗi cấu trúc** — nó được giao cho onset_bias xử lý theo từng máy. Thiết kế đúng chỗ.

### 8.4. Prologue bị thổi phồng bởi tính toán lặp (chỗ đáng dọn)

Vì residual = prologue, **mọi công việc thừa trong prologue trực tiếp làm phím trễ thêm**. Dư thừa hiện có:
- `get_current_leads()` gọi ở `run()` (`:904`) **rồi gọi lại** trong `_drain_due` (`:858`).
- `_down_lead_for_batch(batch)` đọc estimator **nhiều lần cho cùng một batch** (`:865` qua callback, `:876`, rồi truyền tiếp).
- `lead_down` (n_keys=1) tính rồi bị `lead_for_batch` ghi đè (L1) — tính vô ích ngay trong cửa sổ nóng.

→ Dọn đám này **rút ngắn prologue = giảm `visible_lateness` thật**, không chỉ thẩm mỹ. Cải thiện timing có thật, không overengineer.

### 8.5. Down trùng nhịp với release kế thừa send_duration của release

`_drain_due` bắn pending releases **trước** (`:860-862`), rồi mới authored downs. Tại nhịp có cả up lẫn down cùng đến hạn, `SendInput` của down phải đợi `SendInput` của up xong → down nhận thêm trọn một `send_duration` trễ mà lead không lường. Độ trễ **có cấu trúc**, nhưng up-trước-down là bắt buộc đúng và hai `KeyAction` khác `kind` không gộp được → chấp nhận được.

### 8.6. Vi mô trong đường nóng nhất

`getattr(self.wait_strategy, "spin_until_us", None)` (`:773`) tra cứu chuỗi **mỗi deadline**; nhánh fallback `while self.clock.now_us() < target_system_us: pass` (`:777`) là **dead** trong production (`HybridWaitStrategy` luôn có `spin_until_us`). Gọi thẳng `self.wait_strategy.spin_until_us(...)` bỏ cả getattr lẫn nhánh chết.

### 8.7. Những thứ đã ĐÚNG (không sửa nhầm)

- Spin ns-based, không chia trong vòng lặp (`wait_strategy.py:44-47`).
- `spin_threshold` (≈500–800µs, adaptive mean+3σ) đủ rộng để nuốt jitter thức giấc của sleeper → phần cuối luôn spin.
- Prologue chạy dưới MMCSS/TIME_CRITICAL (`rt_priority.py`), GC tạm dừng, no-GIL → khử ba nguồn preempt lớn nhất; residual prologue gần như là **bias hằng số ~vài chục µs** (dưới một frame), không phải jitter lớn.

### Kết luận (mục 8)

Đường spin đã sắc; khoảng trễ chưa-bù-được = **prologue Python giữa spin-end và `SendInput`**, và nó *bằng* `visible_lateness`. Lead bù `send_duration`, prologue giao cho `onset_bias_us`. **Đòn bẩy cải thiện thật và an toàn duy nhất ở đây là rút ngắn prologue** bằng cách dọn tính toán lead lặp (8.4 + L1/L2) — vừa clean code vừa giảm trễ thật.

---

## Phụ lục — Bằng chứng tính đồng thời (chuỗi gọi một hợp âm)

```
scheduler.build_key_actions
  → group by (at_us, kind)                      # N nốt cùng mốc → 1 KeyAction
compile_runtime_intents
  → 1 RuntimeActionBatch (N intents)
DispatchLoop._drain_due → _dispatch_down_batch
  → 1 KeyAction(scan_codes = N phím)
  → backend.key_down(scan_codes)                # 1 lời gọi
WinSendInputBackend._emit
  → send_scan_code_batch_trusted
  → user32.SendInput(N, array, size)            # 1 syscall nguyên khối  ✅
```
