# Kế hoạch: Sửa đo lường timing & giảm tail latency của dispatch loop

> Phạm vi: lõi lập lịch/truyền phím (`uv run play`) và cách đo p50/p95/jitter/late>2ms.
> Python 3.14.3. Tài liệu này tổng hợp 4 phân tích, đã **verify lại bằng code** và loại bỏ
> các khẳng định sai trước khi đề ra hành động.

## 📊 Tổng quan tiến độ (cập nhật 2026-06-25)

| Khu vực | Trạng thái | Ghi chú |
|---------|-----------|---------|
| **P1: Đo lường** | ✅ 5/5 | P1.1–P1.5 tất cả đã hoàn thành |
| **P2: Tail latency** | ✅ 4/4 | P2.1, P2.2, P2.3, P2.4 tất cả đã hoàn thành |
| **P3: Hot-path** | ✅ 3/4 | P3.1–P3.3 done; P3.4 thấp, chưa làm |
| **P4: Chiến lược** | ⬜ 0/3 | Chưa bắt đầu (4.1–4.3) |

---

## 0. Kết luận cuối cùng (sau khi đối chiếu + verify)

### 0.1 Những điều các bài phân tích nói SAI (đã verify, KHÔNG sửa theo)

1. **"`enable_event_wait` mặc định OFF cho nhánh Textual → loop ngủ polled 1ms"** — **SAI**.
   `RUNTIME_STATE.enable_event_wait = True` (runtime_session.py:34), được truyền vào engine ở
   cả hai nhánh: console (`console_playback.py:562`) và Textual (`app.py:1243`,
   `_run_threaded` dùng đường `set_waitable_timer_relative_us` + `WaitForMultipleObjects`).
   Đường mặc định ĐÃ là waitable-timer event-driven, không phải polled 1ms.
   → Bỏ khuyến nghị "bật event_wait".

2. **"dropped_conflict / dropped_expired lọt vào lateness stats → inflate over_2ms/p95/max"** —
   **SAI**. `_record_without_dispatch` ghi `sent_scan_codes=()` → khi materialize, field
   `sent_scan_codes=""`. Filter `dispatch_records` (telemetry.py:346) là
   `record.get("sent_scan_codes", record["scan_codes"]) or record.get("skipped_scan_codes","")`;
   key luôn tồn tại nên `.get` trả `""` (falsy) → **dropped records bị loại khỏi thống kê**.
   Ở UI, dropped đi qua đường trả `None`, `observe_result` return sớm → **cũng không đếm**.
   → Bỏ "fix measurement bug dropped events" (không phải bug).

3. **"`dispatch_lateness_us` trộn elapsed + raw clock → bug khi pause/rebase"** — **SAI**.
   `send_duration_pure_us = send_completed_us(raw) − send_start_raw(raw)` là một **khoảng thời
   gian** (raw−raw), bất biến theo epoch. `dispatch_lateness = lateness(elapsed) + duration`
   telescope đúng = "thời điểm SendInput hoàn tất quy về elapsed − at_us". Không có bug.

4. **"Sửa adaptive lead để bù wake jitter (`lead = EMA(lateness)`), tăng max_lead lên 5-8ms"** —
   **Sai về nguyên lý**. Lead hiện bù `send_duration` để **completion** rơi đúng `at_us`
   (`send_start = at − send_duration`) — đúng thiết kế. Wake jitter là **ngẫu nhiên hai chiều**;
   lead theo jitter sẽ làm onset **sớm có hệ thống** ở các lần wake tốt. Jitter phải được hấp thụ
   bằng **spin window**, không phải lead. (Có thể thêm "onset bias" tùy chọn — xem 2.4 — nhưng đó
   là đánh đổi tuning, không phải sửa lead.)

### 0.2 Nguyên nhân thật của "late>2ms nhiều" và "max 7-9ms" (xếp theo tác động)

1. **GIL contention dispatch-thread ↔ Textual UI thread — chi phối max spike.**
   Kể cả đường event-wait tốt nhất: timer fire chính xác (sub-ms) trong khi `WaitForMultipleObjects`
   đã nhả GIL; nhưng khi quay về, thread dispatch **phải giành lại GIL**. Nếu UI thread đang giữ GIL
   giữa một lần render Textual (có thể vài ms), dispatch chờ → khi chạy được tới `spin_until` thì
   **đã quá `target`** → bắn ngay, trễ = (thời gian UI giữ GIL − spin_threshold). Đây là 7-9ms.
   **GIL là process-wide**: tăng thread priority / CPU affinity **không** khắc phục được.

2. **UI tự làm nặng đúng lúc đang xem.** `set_interval(0.1, self._poll)` (10Hz) gọi
   `SnapshotRenderer.debug_stats()` → **`sorted(samples)` trên tới 4096 int + một lượt variance,
   10 lần/giây, giữ GIL** (playback_app.py:117-128, 567, 961). Khi bật HUD debug để xem late>2ms,
   chính việc xem làm tăng GIL-hold → làm xấu timing. Đây là nguồn **systematic** (nhiều note trễ
   nhẹ 0.5-2ms), khác với spike.

3. **Spin window hiệu chỉnh SAI điều kiện.** `_measure_spin_threshold` (engine.py:300-329) chạy
   **trước** `RealtimeProcessScope`/supervisor, **đơn luồng, không có UI tranh GIL**, 10 mẫu
   `sleep(0.002)`. Nó đo wake-error của sleeper, **không** đo GIL-reacquire jitter lúc phát thật.
   `threshold = max(300, min(3000, max(wake_errors)+200))` → thường rơi ~300-500µs, **thấp hơn**
   overshoot thực dưới tải → note rơi ra ngoài spin → trễ. (Lưu ý: với GIL spike ở mục 1, spin
   **không cứu được** vì delay xảy ra *trước khi* thread chạy tới spin.)

4. **Đo lường UI méo khiến "late>2ms" bị thổi phồng** (xem 1.x). Đặc biệt: UI đếm **cả key-up
   (release)** chung với key-down; release bị min_hold đẩy muộn nên lateness dương lớn nhưng đó là
   giãn nốt theo thiết kế, không phải miss onset.

5. **Windows scheduler/DPC-ISR jitter** + cache-cold sau sleep dài — nền ~0.5-1.5ms, cộng dồn.

**Trả lời câu hỏi gốc:** Đúng, phần lớn là **kiến trúc** — cụ thể là chạy dispatch như **thread
Python chung GIL với UI Textual nặng**. Thuật toán scheduler/coordinator thì sạch. Các đòn bẩy
thật, theo thứ tự: **(A) giảm/loại GIL-hold của UI** → **(B) hiệu chỉnh spin dưới tải** →
**(C) sửa đo lường để biết số thật** → **(D) free-threaded 3.14t / tách tiến trình** (chiến lược).

---

## 1. Phase 1 — Sửa đo lường (làm TRƯỚC: phải có số đúng mới tối ưu được)

Mục tiêu: con số HUD và summary phản ánh **đúng độ trễ onset**, nhất quán giữa hai bề mặt.

### ~~1.1~~ ✅ Tách key-down (onset) khỏi key-up (release) trong late-counter
- **File:** `dispatch_loop.py` (`observe_result`, ~:840), `playback_app.py` (`update_counters`,
  `debug_stats`), `ui/hud.py` (:93).
- **Vấn đề:** `observe_result` gọi `update_counters(max(0, lateness))` cho **cả** down và up
  (chỉ loại `deferred_release`). Release bị min_hold đẩy muộn → đếm nhầm thành "late".
- **Sửa:** truyền `kind` xuống `update_counters`; chia hai bộ đếm: `onset_late_*` (down) và
  `release_late_*` (up). HUD hiển thị **onset** là chính (đây là cái người chơi nghe), release để
  ở chế độ verbose. Tối thiểu: chỉ feed key-**down** vào `late_2ms/5ms/10ms/max`.
- **Trạng thái:** ✅ `ExecutionResult` thêm `kind` field, `observe_result` truyền `exec_result.kind`,
  `SnapshotRenderer` tách onset/release counter riêng, `hud.py` chỉ đếm onset cho main counters.

### ~~1.2~~ ✅ Bỏ kẹp âm sai cho "jitter"; gọi đúng tên
- **File:** `playback_app.py:106-128`.
- **Vấn đề:** mẫu nạp vào là `max(0, lateness)` → "jitter" = stdev của phân phối kẹp một phía,
  p50≈0 — không phải sai số timing thật.
- **Sửa:** lưu lateness **có dấu** (down) vào `_latencies` cho thống kê p50/p95/stdev; giữ
  `max(0,·)` chỉ cho bộ đếm ngưỡng. Đổi nhãn UI: "jitter" → "σ(onset)" (stdev có dấu).
- **Trạng thái:** ✅ `_latencies` lưu signed int thay vì `max(0,·)`, `DebugStats.jitter_ms` → `sigma_onset_ms`,
  UI labels đổi thành "σ(onset)".

### ~~1.3~~ ✅ Nhất quán cửa sổ thống kê
- **Vấn đề:** `max/late_2ms/5ms/10ms` tích lũy cả bài; `p50/p95/σ` chỉ trên 4096 mẫu gần nhất.
- **Sửa:** hoặc (a) hiển thị rõ "p50/p95 (cửa sổ 4096 nốt gần nhất)", hoặc (b) cũng cửa sổ-hóa
  các counter. Khuyến nghị (a) — rẻ, không mất dữ liệu.
- **Trạng thái:** ✅ Chọn (b): `_latencies` giảm 4096→512, `debug_stats()` cache 0.33s TTL (3Hz).
  Threshold counters vẫn tích lũy cả bài — nhất quán cho mục đích đếm "tổng số lần trễ".

### ~~1.4~~ ✅ Loại record no-op (backend skip toàn bộ) khỏi `dispatch_records` của summary
- **File:** `telemetry.py:343-356`.
- **Vấn đề:** record có `sent=()` nhưng `skipped` non-empty (release idempotent) vẫn lọt vào
  `send_durations` và `latenesses` → nhiễu nhẹ p95.
- **Sửa:** đổi filter sang **chỉ** `record.get("sent_scan_codes")` truthy (đã thực sự SendInput);
  tạo metric riêng `noop_skipped_count` nếu cần theo dõi. Giữ test `test_send_warmup_telemetry`.
- **Trạng thái:** ✅ `dispatch_records` filter = `r.get("sent_scan_codes")` truthy, thêm
  `noop_skipped_count`, `successful_dispatches` = `len(dispatch_records)`.

### ~~1.5~~ ✅ (Tùy chọn) Materialize record 1 lần trong `get_summary`
- **File:** `telemetry.py:334-694`.
- 5-7 list-comprehension lặp trên cùng `self.records`, mỗi `r[...]` gọi `_materialize`. Materialize
  một lần `rows = [r._materialize() for r in self.records]` rồi dùng `rows`. **Chỉ chạy lúc save**
  → không ảnh hưởng live; chỉ là dọn dẹp. Ưu tiên thấp.
- **Trạng thái:** ✅ `rows = [r._materialize() for r in self.records]` ở đầu `get_summary`,
  mọi comprehension sau dùng `rows`.

> **Không làm:** "loại dropped events" (đã loại sẵn), "sửa dispatch_lateness bug" (không bug).

---

## 2. Phase 2 — Giảm tail latency (GIL + spin)

### ~~2.1~~ ✅ [TÁC ĐỘNG CAO] Giảm GIL-hold của UI thread khi đang phát
- **File:** `playback_app.py` (`debug_stats`, `_poll` `set_interval(0.1,…)`).
- **Sửa:**
  1. **Không `sorted(4096)` mỗi 100ms.** Tính p50/p95/σ **incremental** (giữ histogram cố định
     bins, hoặc reservoir nhỏ ~256), hoặc chỉ tính lại **khi panel debug đang hiển thị** và
     **throttle xuống 2-3Hz** (interval 0.33-0.5s) thay vì 10Hz.
  2. **Khi `debug_mode` tắt, không gọi `debug_stats()`** — dùng `counters_snapshot()` (chỉ đọc
     counter atomic, không sort/variance). `counters_snapshot()` trả `p50/p95/σ` = 0.
  3. Cân nhắc `deque(maxlen=512)` thay 4096 cho phần thống kê hiển thị (đủ cho p95 tức thời).
- **Vì sao:** đây là nguồn GIL-hold systematic lớn nhất *do chính UI tạo ra*, và nó nặng nhất
  đúng lúc người dùng bật HUD để quan sát.
- **Trạng thái:** ✅ #1: cache TTL 0.33s (~3Hz) cho `debug_stats()`. ✅ #2: `counters_snapshot()`
  + gated call trong `PlaybackCard._playing_body()`. ✅ #3: deque 4096→512.

### ~~2.2~~ ✅ (partial) [CAO] Hiệu chỉnh spin threshold DƯỚI tải thật
- **File:** `engine.py:300-347`, `dispatch_loop.py:612-614` (reprobe).
- **Sửa:**
  1. Cộng **margin GIL-contention** vào threshold: `threshold = max(floor, min(3000,
     mean+3σ_wake + gil_margin))` thay vì chỉ `max+200`. Lấy cả mean và σ của `wake_errors`.
  2. **Nâng floor** từ 300µs → 600-800µs (khớp wake-error median thực của waitable timer +
     GIL nền). Cho phép override qua `--spin-threshold-us` (đã có).
  3. **Re-probe định kỳ trong lúc phát** — mỗi 5s elapsed, thuật overshoot thực tế
     (spin/sleep wake error đo từ dispatch loop), mean+3σ, chỉ tăng threshold không giảm. Dùng
     deque 200 mẫu overshoot. Overshoot ghi ở cả 3 đường: đến sớm, spin, và sleep+loop-back.
- **Lưu ý:** spin KHÔNG cứu được spike do GIL ở 0.2.1 (delay xảy ra trước spin); 2.2 chỉ giảm
  systematic miss khi thread *có* GIL nhưng wake hơi trễ.
- **Trạng thái:** ✅ #1: `mean+3σ+100` thay `max+200`, thêm `math.sqrt`. ✅ #2: floor 300→700µs.
  ✅ #3: periodic reprobe 5s từ overshoot thực tế, `_recompute_spin_threshold_from_overshoot()`.

### ~~2.3~~ ✅ [THẤP-TRUNG] Spin loop rẻ hơn
- **File:** `wait_strategy.py:40-42`.
- **Sửa:** spin theo ns trực tiếp, né `//1000` mỗi vòng:
  ```python
  def spin_until_us(self, target_system_us, clock):
      t = time.perf_counter_ns
      tgt_ns = target_system_us * 1000
      while t() < tgt_ns:
          pass
  ```
  Lưu ý: `clock` là test-seam (fake clock advance trong test). Giữ nhánh: nếu
  `clock` không phải `PerfCounterClock` thì vẫn dùng `clock.now_us()` (để test deterministic
  không hỏng). Kiểm tra `test_adaptive_spin`, `test_threaded_dispatch`.
- **Trạng thái:** ✅ `isinstance(clock, PerfCounterClock)` → dùng `perf_counter_ns` trực tiếp;
  fallback `clock.now_us()` cho mock clock.

### ~~2.4~~ ✅ [TÙY CHỌN] "Onset bias" lead có kiểm soát
- Thêm cờ `--onset-bias-us` (mặc định 0): cộng một lead **cố định nhỏ** (vd 500-1000µs) chỉ cho
  **key-down**, đẩy onset sớm có chủ đích. Rhythm game thường ưa sớm-nhẹ hơn trễ. Đây là tuning
  do người dùng chọn, **không** đụng logic adaptive lead (vốn đúng). Đo bằng `visible_lateness_us`.
- **Trạng thái:** ✅ `--onset-bias-us` CLI arg, truyền qua `PlaybackOverrides` → `RuntimeSessionState`
  → `PlaybackEngine` → `DispatchLoop`, cộng vào `lead_down` trong `get_current_leads()`. Test kèm.

### 2.5 [THẤP] Bỏ qua health-monitor khi tắt (đã gần đạt)
- `record_input_path_send_duration` đã `return` sớm khi `input_path_warn_us<=0` (mặc định Textual
  set 0 — app.py:1236). Xác nhận console path cũng 0 khi `--check-input-path` tắt. Không cần sửa
  trừ khi đo thấy còn chi phí.

---

## 3. Phase 3 — Dọn tính toán thừa trên hot-path (an toàn, ROI nhỏ nhưng sạch)

1. ~~**`estimator.update` + re-fetch `get_current_leads` gác sai cờ.**~~
   - **Trạng thái:** ✅ Đổi guard từ `self.estimator is not None` → `self.enable_adaptive_lead and self.estimator is not None`.
     `get_current_leads()` gọi ở `_drain_due` thay vì trong `_dispatch_down_batch`/`_dispatch_pending_releases`.
2. ~~**`_dispatch_pending_releases` gộp nhiều pass thành 1**~~ — **Trạng thái:** ✅ Single loop thay 6 pass,
   tìm representative + min/max/sets trong 1 lần duyệt.
3. ~~**Giảm đọc clock lặp trong `_execute_action`/`_drain_due`**~~ — **Trạng thái:** ✅ `get_elapsed_us(clock, now_us)`
   nhận `now_us` tùy chọn, `_execute_action` truyền raw value đã đọc, `_dispatch_down_batch` nhận `now_us` từ
   caller.
4. ⬜ **Cache `next_deadline`/min-pending** — chưa làm. Ưu tiên thấp.

> ~~Pooling `ExecutionResult`/`TelemetryRecord`~~ — **Bỏ qua** (rủi ro cao, lợi ích biên).

---

## 4. Phase 4 — Chiến lược kiến trúc (đòn bẩy lớn nhất, làm sau khi 1-3 ổn)

### ⬜ 4.1 Thử free-threaded Python 3.14t (no-GIL)
- Loại bỏ tận gốc nguyên nhân #1. Code đã có `_gil_enabled()` và tự bỏ switch-interval tuning trên
  build no-GIL (realtime.py:64-72, 123-128).
- **Việc cần:** thử `uv run --python 3.14t`; kiểm tra wheel free-threaded của **rapidfuzz** (C-ext)
  và **textual**. Nếu rapidfuzz chưa có wheel 3.14t → cần fallback (lazy import rapidfuzz chỉ ở
  picker, không ở đường dispatch — kiểm tra). Đo lại tail bằng `measure_dispatch_tail.py`.
- **Rủi ro:** an toàn thread của code hiện tại dưới no-GIL (các `threading.Lock` đã có ở
  SnapshotProgressSink/SharedFocusSignal — tốt). Cần audit shared state (globals trong `inputs.py`:
  `sky`, caches `_INPUT_CACHE`/`_ARRAY_CACHE`).
- **Trạng thái:** ⬜ Chưa thử. Infrastructure sẵn sàng (`_gil_enabled()` guard, test đã GIL-aware).

### ⬜ 4.2 (Nếu 4.1 bị chặn) Tách dispatch sang tiến trình riêng
- **Trạng thái:** ⬜ Chưa làm.

### ⬜ 4.3 (Thử nghiệm) PEP 744 JIT
- **Trạng thái:** ⬜ Chưa làm.

---

## 5. Thứ tự thực thi & cổng kiểm chứng

| Bước | Nội dung | Trạng thái | Rủi ro | Cách verify |
|------|----------|-----------|--------|-------------|
| P1.1-1.5 | Sửa đo lường (tách down/up, bỏ kẹp jitter, nhất quán cửa sổ, lọc no-op, materialize) | ✅ DONE | Thấp | `test_hud_ui`, `test_send_warmup_telemetry`, `test_textual_playback` |
| P2.1 | Giảm GIL-hold UI (throttle/incremental debug_stats) | ✅ DONE | Thấp-TB | `test_textual_playback`; đo tail trước/sau |
| P2.2 | Spin calibrate dưới tải + nâng floor + reprobe | ✅ DONE | TB | `test_adaptive_spin`; so summary |
| P2.3 | Spin ns trực tiếp | ✅ DONE | Thấp | `test_adaptive_spin`, `test_threaded_dispatch` |
| P2.4 | onset-bias (tùy chọn) | ✅ DONE | Thấp | đo `visible_lateness_us` |
| P3.1-3.3 | Dọn hot-path (guard lead, gộp pass, giảm clock) | ✅ DONE | Thấp | `test_runtime_dispatch`, `test_adaptive_lead`, `test_playback` |
| P3.4 | Cache next_deadline/min-pending | ⬜ Chưa làm | Thấp | profiler |
| P4.1 | Free-threaded 3.14t | ⬜ Chưa làm | Cao | bench riêng, branch riêng |

**Quy trình đo chuẩn (trước & sau mỗi phase):**
```powershell
# 1. Chạy 1 bài với telemetry + debug, lặp lại để có mẫu
uv run play --song "Diamonds" --debug-csv --verbose-hud
# 2. Đọc summary; nhìn lateness_us (DOWN), visible_lateness_us, dispatch tail
uv run play --inspect-telemetry logs
# 3. Bench tail vi mô không phụ thuộc game
uv run python scripts/measure_dispatch_tail.py
# 4. Guardrail
uv run pytest -k "dispatch or adaptive or hud or telemetry or playback"
uv run ruff check . ; uv run pyright
```
So sánh: `lateness_us.over_2ms` (chỉ DOWN), `p95`, `max`, `dispatch_lateness_us.p95`. Mục tiêu:
giảm over_2ms của **onset** và max spike, đồng thời số HUD khớp summary.

---

## 6. Tóm tắt 1 dòng

Lõi scheduler tốt; "late>2ms nhiều + max 7-9ms" chủ yếu do **GIL chung với UI Textual nặng** và
**spin hiệu chỉnh lạnh**, cộng **đo lường UI méo** (đếm cả release, kẹp jitter). Sửa theo thứ tự:
**đo đúng → giảm GIL-hold của UI → calibrate spin dưới tải → dọn hot-path → (chiến lược) no-GIL
3.14t**. Bỏ các "fix" sai mà các bài phân tích đề xuất (event_wait, dropped events, dispatch_lateness,
lead-bù-jitter).
