# Audit luồng dispatch và lõi tính thời điểm gửi

- Ngày: 2026-07-22
- Môi trường: CPython `3.14.3 free-threading build`, `sys._is_gil_enabled() == False`.
- Phạm vi: parser/scheduler, coordinator, dispatch loop, supervisor/thread, wait strategy, telemetry, Win32 `SendInput`, cleanup và wiring CLI/Textual.
- Nguồn kết luận kỹ thuật: mã nguồn, test và các phép tái hiện/đo runtime. Không dùng tài liệu kiến trúc hay tri thức trong `docs/` để suy ra hành vi.
- Hai báo cáo AI cũ chỉ được đọc sau khi phân tích độc lập, để kiểm tra và hợp nhất nhận định.
- **Consumption (2026-07-23):** actionable work extracted into
  [plan/2026-07-23_dispatch-core-correctness-hardening-plan.md](plan/2026-07-23_dispatch-core-correctness-hardening-plan.md).
  That plan **rejects H2 as a defect** (completion-anchor / same-key equality drop is
  design-correct per `timing-principles.md`). Prefer the plan over re-deriving fixes from
  this audit alone.

## 1. Kết luận điều hành

Lõi có nhiều quyết định đúng cho một trình phát ưu tiên độ chính xác: timeline tuyệt đối dựa trên `perf_counter_ns`, key-up neo theo lúc key-down hoàn thành, active state bounded theo polyphony, có waitable timer/event/spin tail, và đường gửi cuối chỉ dùng Win32 `SendInput`.

Tuy nhiên, dự án **chưa đạt best practice hoàn chỉnh cho Python 3.14 free-threaded**. Sáu vấn đề mức cao là:

1. Note đầu có thể được gửi sau khi focus đã mất vì fresh focus gate chỉ chạy từ key-down thứ hai.
2. Hai lần nhấn cùng phím ở đúng biên minimum-hold được scheduler chấp nhận nhưng runtime có thể loại lần thứ hai do latency backend.
3. Khi high-resolution timer không dùng được, command event có thể bị bỏ qua hoàn toàn, không chỉ phản hồi chậm.
4. Ngoại lệ trong supervisor có thể để dispatch thread sống khi tài nguyên đang bị đóng.
5. Ba Win32 API event wait thiếu prototype `ctypes`; `CreateEventW` có rủi ro cắt cụt HANDLE trên Windows 64-bit.
6. Một số biên đầu vào chưa strict: allow-list process rỗng, boolean được nhận như số, timing không-finite/ngoài miền, scan code chưa kiểm tra đầy đủ.

Spin tail, latency probe và completion anchoring là chi phí có chủ đích cho accuracy, không phải lãng phí mặc định. Phần nên tối ưu là schedule materialization, prewarm sai hình dạng runtime, telemetry giữ toàn bộ record rồi tạo thêm summary, và full-GC đồng bộ — nhưng chỉ sau khi sửa correctness.

## 2. Luồng thực thi đọc từ code

```text
song/CLI config
  -> parse + validate
  -> compile_schedule (authored absolute actions)
  -> PlaybackEngine.play
       -> prewarm SendInput shapes
       -> initial focus wait
       -> RealtimePlaybackScope (GC/MMCSS/timer policy)
       -> PlaybackSupervisor
            -> control thread polls command/focus
            -> dispatch thread runs DispatchLoop
                 -> coordinator chọn authored/pending deadline
                 -> wait strategy: timer/event/sleep/spin
                 -> drain pending releases rồi authored batches
                 -> backend.send_key_batch -> SendInput
                 -> ghi completion và lập minimum-hold release
       -> cleanup pressed keys, timer/cache, telemetry, GC
```

Scheduler tạo intent theo authored timeline; runtime mới biết latency thật của `SendInput`. Vì vậy invariant ở ranh scheduler/runtime phải được kiểm tra theo completion time, không chỉ hai timestamp authored.

## 3. Bảng phát hiện

| ID | Mức | Phát hiện | Bằng chứng |
|---|---|---|---|
| H1 | Cao | Fresh focus gate bỏ qua key-down đầu | Tái hiện được |
| H2 | Cao | Same-key đúng biên minimum-hold bị runtime loại | Tái hiện được |
| H3 | Cao | Fallback wait bỏ qua command event | Tái hiện được |
| H4 | Cao | Supervisor exception có thể để thread sống | Tái hiện được |
| H5 | Cao | Win32 event API thiếu `ctypes` prototypes | Xác minh trực tiếp |
| H6 | Cao/P0 | Validation tại nhiều biên chưa nghiêm | Xác minh trực tiếp |
| M1 | Trung bình | Warmup có thể chạy sau pending deadline | Tái hiện được |
| M2 | Trung bình | Adaptive lead snapshot không nhất quán trong một drain | Tái hiện được |
| M3 | Trung bình-thấp | `now_us` cũ có tác động hẹp, không phải lỗi tổng quát như báo cáo cũ | Phân tích call path |
| M4 | Trung bình | Prewarm không phủ multi-key key-up | Xác minh trực tiếp |
| M5 | Trung bình | Một số option không nối nhất quán vào production | Xác minh caller |
| L1 | Thấp/cần làm rõ | State lock-free cần hợp đồng rõ trong free-threaded build | Rủi ro thiết kế |
| L2 | Thấp | Layer boundary và compatibility shim còn nợ kỹ thuật | Xác minh import/caller |

## 4. Phát hiện mức cao

### H1 — Key-down đầu có thể lọt qua sau khi mất focus

`PlaybackSupervisor` khởi tạo `SharedFocusSignal(True)` rồi cho dispatch thread chạy. Fresh focus check trong `DispatchLoop` chỉ chạy nếu `_first_down_dispatched` đã là `True` (`core/loop.py`, vùng 565–670). Với action ở `t=0`, dispatch thread có thể gửi trước vòng poll focus đầu của control thread.

Repro cho focus guard trả `True` ở initial engine check rồi `False`:

```text
result=finished
focus_calls=2
down_observations=[('sky-music-dispatch', False)]
```

Backend đã key-down khi focus guard báo false. Đây là lỗi correctness/an toàn đầu vào, không chỉ là caveat của shared signal. Fresh focus gate phải áp dụng cho **mọi** key-down, gồm action đầu.

### H2 — Scheduler và runtime bất đồng ở biên same-key

Scheduler coi hai lần nhấn cùng phím cách đúng `min_hold_us` là hợp lệ và có thể tạo key-up/key-down cùng timestamp. Runtime neo release vào **lúc key-down hoàn thành** cộng minimum hold. Khi backend có latency, key-up thực bị đẩy muộn và key-down kế tiếp xung đột.

Repro với delta `1000 us`, min hold `1000 us`, backend latency `100 us`:

```text
scheduler_impossible=0
scheduler_risky=0
backend_calls=[down 0->100, up 1100->1200]
runtime_dropped_conflict=1
```

Scheduler cần margin cho backend/completion anchor, hoặc runtime cần policy xác định để trì hoãn lần nhấn kế thay vì silent drop.

### H3 — Command bị mất trong degraded wait path

`HybridWaitStrategy.wait_until_us()` chỉ dùng `command_event` khi event wait bật **và** có timer handle (`infrastructure/wait_strategy.py`, vùng 83–91). Nếu high-resolution timer unavailable, fallback sleep/spin không quan sát event. Dispatch loop lại chỉ poll queue định kỳ khi `command_event is None` (`core/loop.py`, vùng 1092–1105).

Repro ép timer unavailable, phát quit trong bài 200 ms:

```text
quit_was_produced=True
result=finished
elapsed≈0.220s
```

Lệnh không được xử lý trước khi bài tự hết. Fallback phải chờ event hoặc poll queue theo bounded interval; `WAIT_FAILED` cũng phải là nhánh lỗi riêng.

### H4 — Ngoại lệ supervisor có thể để dispatch thread sống

Cleanup cuối supervisor đóng event handle nhưng không bảo đảm cancel/join dispatch thread khi vòng control ném ngoại lệ. Repro với `controls.poll()` ném lỗi và dispatch stub bị block:

```text
raised='control failure'
dispatch_thread_alive_after_supervisor_exception=True
```

Outer engine cleanup sau đó có thể đóng timer/cache khi thread còn dùng chúng. Cần structured shutdown trong `finally`: cancel, signal event, join có timeout, rồi mới teardown tài nguyên chung.

### H5 — Thiếu prototype cho Win32 event API

Introspection cho thấy `CreateEventW`, `SetEvent`, `WaitForMultipleObjects` đều có `argtypes=None`, `restype=ctypes.c_long`. Mặc định này không an toàn cho pointer-sized HANDLE trên Windows 64-bit; `CreateEventW` có thể truncate. Code cũng chưa tách `WAIT_FAILED` khỏi kết quả wait khác.

Cần khai báo đầy đủ `argtypes/restype`, kiểm tra lỗi tại platform boundary và test handle vượt 32 bit.

### H6 — Strict validation chưa kín

- `set_expected_process_names()` lọc chuỗi rỗng; input `","` thành allow-list rỗng. `get_sky_window()` khi đó có thể nhận title bất kỳ nếu không có fallback rõ.
- JSON timestamp nhận `bool` vì `bool` là subclass của `int`.
- Một số config dùng `bool(value)`, nên chuỗi `"false"` thành `True`.
- Timing config chưa đồng nhất kiểm tra `isfinite`, giới hạn trên và miền hợp lệ; `VALID_FPS` không luôn dùng để reject.
- Native scan-code seam chủ yếu loại duplicate, chưa xác nhận chặt kiểu/range trước khi tạo `INPUT`.

Cần validator typed tại boundary: reject allow-list rỗng sau normalize, reject boolean cho numeric field, dùng `math.isfinite`, đặt bound rõ và validate scan code trước `SendInput`. Không thêm cơ chế input nào khác `SendInput`.

## 5. Phát hiện trung bình và thấp

### M1 — Warmup có thể làm trễ pending release đã đến hạn

Trong `_drain_due`, budget warmup được tính từ `next_authored_us` trước `pop_due_pending()`. Nếu pending release đã đến hạn nhưng authored action kế còn xa, warmup vẫn chạy trước pending release.

```text
effective_release_deadline_us=100000
warmup_budgets_us=[200]
release_emit_time_us=100200
release_added_lateness_us=200
```

Nhận định của báo cáo acceptance rằng warmup không thể chạy sau deadline là sai. Budget phải dựa trên `min(next authored, next pending)` và bằng 0 nếu effective deadline đã tới.

### M2 — Adaptive lead thay đổi giữa các batch đã pop

`pop_due_authored()` materialize tất cả batch due bằng lead snapshot trước vòng send. Sau batch đầu, estimator có thể cập nhật lead; batch sau ghi telemetry theo lead mới dù quyết định pop dùng lead cũ.

```text
(scheduled=1000, now=1000, lead_recorded=200, late=0)
(scheduled=1100, now=1000, lead_recorded=0, late=-100)
```

Comment tại `core/loop.py` 1191–1194 nói lead được đọc fresh cho từng yield, nhưng tuple đã materialize nên comment không đúng semantics. Nên pop/send từng batch hoặc mang lead snapshot kèm batch.

### M3 — `now_us` cũ có thật, nhưng báo cáo cũ xếp hạng quá cao

`_drain_due(now_us)` giữ scalar `now_us` cho một số quyết định. Tuy nhiên telemetry send đọc clock mới, focus gate đọc signal/probe mới, pending release mới sau key-up dùng elapsed time mới, và backend completion được đo lại. Tác động còn lại chủ yếu ở optional late-pulse-drop và snapshot batch due; late-pulse-drop không thấy production caller truyền vào. Đây là vấn đề cần làm sạch/test, nhưng chưa đủ bằng chứng để gọi High tổng quát.

### M4 — Prewarm không phủ hình dạng key-up thực tế

Prewarm tạo đủ batch key-down nhưng key-up chủ yếu theo singleton. Runtime có thể release nhiều phím cùng lúc, dẫn tới cache miss, lock và cấp phát mảng `ctypes` ngay đường nóng. Nên prewarm exact unique shapes cho cả down/up với cap bounded.

### M5 — Production wiring không nhất quán

- CLI lưu `RUNTIME_STATE.spin_floor_us`, nhưng constructor path không truyền nhất quán; Textual dùng `value or 700`, làm giá trị hợp lệ `0` thành `700`.
- `lead_cache_path` có ở engine/test nhưng không thấy production caller cấp.
- Textual không truyền đủ một số telemetry field liên quan min-hold margin.
- `late_pulse_drop_threshold_us` có test/engine support nhưng không thấy production caller.

Đây là dead/partial configuration: tăng nhánh code, test surface và hiểu nhầm vận hành.

### L1 — Lock-free state trong free-threaded Python cần hợp đồng rõ

`SharedFocusSignal` và một số snapshot state dựa vào assignment/read đơn giản giữa thread. H1 là lỗi logic độc lập, không cần giả định data race. Ngoài lỗi đó, code no-GIL nên chỉ rõ synchronization/ownership thay vì dựa vào trực giác từ GIL cũ. Không nên thêm lock vào hot path mù quáng; event, atomic snapshot hoặc owner-thread message cần được benchmark.

### L2 — Nợ kiến trúc và pattern cũ

- Domain/session import policy từ infrastructure.
- Domain scheduler types đọc cache file, trộn model với I/O.
- `layouts` và một số CLI/doctor path chạm `ctypes` ngoài platform package.
- Compatibility shims và broad `except Exception` làm khó chứng minh cleanup invariant.
- Một vài queue path dùng `empty()` rồi `get_nowait()`, là check-then-act thừa và dễ race.

Các điểm này chưa trực tiếp làm sai timing trong phép thử, nên không refactor lớn trước H1–H6.

## 6. CPU: phần nên giữ và phần có thể giảm

### Nên giữ vì phục vụ accuracy

- Absolute monotonic deadlines.
- Completion-anchored minimum hold.
- Waitable timer kết hợp spin tail ngắn.
- Latency measurement và bounded adaptive estimator.
- Cleanup key state cuối phiên.

### Có thể giảm mà ít ảnh hưởng accuracy

- Tránh full schedule transformations/materialization lặp lại; cân nhắc stream/chunk bounded.
- Prewarm exact shapes một lần, bounded, trước perf anchor.
- Đo lại nhu cầu full `gc.collect()`; hiện code collect trước realtime và sau playback, đồng thời disable GC trong realtime.
- Không flush/serialize telemetry lớn đồng bộ trên đường gửi.
- Xóa production-dead options sau khi audit caller, thay vì giữ nhánh không dùng.

Không nên giảm spin tail hay timer precision chỉ để hạ CPU trước khi có latency distribution và missed-deadline benchmark. Với ưu tiên accuracy, phần CPU này là hợp lý.

## 7. RAM: số đo và diễn giải

Đo bằng `tracemalloc` trên Python 3.14 free-threaded hiện tại:

| Trường hợp | 10k | 50k | 100k |
|---|---:|---:|---:|
| Actions đầu vào | 1.68 MiB | 8.40 MiB | 16.79 MiB |
| Tăng thêm khi compile | 2.73 MiB | 13.71 MiB | 27.44 MiB |
| Tổng xấp xỉ sau compile | 4.41 MiB | 22.11 MiB | 44.24 MiB |

Compiled representation tăng tuyến tính, khoảng `1.63x` phần memory của actions gốc trong phép đo. Đây là duplication tuyến tính, **không phải leak**.

Telemetry khi giữ record và tạo summary:

| Records | Sau record | Sau summary | Peak |
|---|---:|---:|---:|
| 10k | 5.01 MiB | 14.67 MiB | 16.54 MiB |
| 50k | 25.19 MiB | 73.44 MiB | 83.08 MiB |

Array cache với 8,192 shapes: khoảng `6.33 MiB` current, `6.89 MiB` peak. Cache có clear cuối phiên trong normal outer cleanup nên bounded; nhưng early return trước outer `try/finally` có thể giữ schedule/cache nếu engine object còn được giữ.

Quit trong initial focus wait cho kết quả:

```text
result=quit
runtime_schedule_retained=True
array_cache_entries_retained=3
```

Đây là missed cleanup/bounded retention, chưa đủ bằng chứng gọi unbounded leak.

## 8. Đánh giá theo Python 3.14 free-threaded

Điểm tốt:

- Runtime thật đúng CPython 3.14.3 free-threaded và GIL tắt.
- Dispatch/control tách thread, phù hợp mục tiêu không để UI giữ hot path.
- Type hints, dataclass/slots và protocol được dùng rộng.

Điểm chưa đạt:

- `requires-python` chỉ biểu đạt version, không biểu đạt free-threaded ABI; môi trường đúng phụ thuộc pin/audit/build gate của repo.
- Shared mutable state cần ownership/synchronization contract rõ hơn trong no-GIL runtime.
- `ctypes` prototypes phải rõ kiểu, nhất là HANDLE pointer-size.
- Numeric validation phải xử lý `bool <: int`, NaN và infinity.

Không có cơ sở gọi code “lạc hậu” chỉ vì dùng thread, `ctypes` hoặc spin. Pattern lạc hậu thực sự là ngầm dựa vào GIL, Win32 call không prototype, check-then-act queue và broad exception không giữ cleanup invariant.

## 9. Đối chiếu hai báo cáo AI cũ

### Bản `dispatch-core-code-audit-2026-07-22.md` cũ

Bản cũ đúng một phần về fallback event, stale `now_us`, warmup, memory tuyến tính và synchronization concern. Nhưng:

- Xếp stale `now_us` mức cao là quá rộng; nhiều phép đo/gate đã refresh time.
- Không phát hiện H1, H2, H4, H5 và các lỗ validation cụ thể.
- Gọi metrics trước chord stagger là sai về `max_polyphony`: code cố ý đo logical/authored chord trước stagger. Chỉ executable timing/risk metric cần tách tên hoặc tính thêm sau transform.
- Gọi retention là leak là quá mạnh khi chưa chứng minh tăng không giới hạn.

### Bản acceptance

Bản acceptance đúng khi bác bỏ `max_polyphony` trước stagger là bug và nhấn mạnh accuracy-first. Nhưng hai kết luận quan trọng bị runtime repro bác bỏ:

- Warmup **có thể** chạy sau pending deadline vì budget chỉ nhìn next authored.
- Degraded event path không chỉ chậm; command có thể bị bỏ qua đến hết bài.

Bản này cũng hạ focus race thành caveat, trong khi H1 cho thấy key-down đầu thực sự lọt qua sau focus loss. Cả hai bản thiếu benchmark memory; số đo trong báo cáo hợp nhất thay thế suy đoán tĩnh đó.

## 10. Thứ tự sửa đề xuất

1. **P0/correctness:** H1 focus gate, H6 validation, H5 Win32 prototypes/error handling.
2. **Runtime safety:** H4 structured cancellation/join, H3 fallback event/queue polling.
3. **Timing invariant:** H2 same-key completion margin, M1 effective-deadline warmup, M2 lead snapshot.
4. **Resource discipline:** telemetry streaming/chunking, exact-shape prewarm, early-return cleanup, đo lại GC/spin.
5. **Maintenance:** production wiring và layer/shim cleanup theo commit riêng, không trộn behavior fix.

## 11. Regression gates cần thêm

- Action `t=0` mất focus trước first control poll không được gửi.
- Same-key tại equality boundary với backend latency > 0 không bị silent drop.
- Quit/pause/focus wake hoạt động khi timer handle unavailable.
- Supervisor exception luôn cancel/join dispatch thread trước resource teardown.
- HANDLE > 32 bit không truncate; `WAIT_FAILED` được báo lỗi.
- Empty process allow-list, boolean numeric, NaN/Inf/out-of-range timing và invalid scan code đều bị reject.
- Pending release đã due không cho warmup chạy.
- Mỗi batch ghi đúng lead snapshot dùng để quyết định due.
- Compile/telemetry/cache benchmark có ngưỡng tuyến tính/bounded, không đặt ngưỡng tuyệt đối thiếu căn cứ.

## 12. Xác minh đã thực hiện

Relevant test suite:

```text
172 passed in 32.26s
```

Các gate toàn dự án đã chạy sau khi viết báo cáo:

```text
uv run ruff check .                                      -> All checks passed
uv run pyright                                           -> 0 errors, 0 warnings
uv run --env-file .env python scripts/audit_security_mandates.py
                                                          -> [OK] No forbidden Windows API references in src/.
```

Các test hiện có xanh nhưng chưa phủ những interleaving/degraded path trên. Ngoài test, từng repro H1–H4, M1–M2, early-return retention và benchmark RAM đã chạy bằng `uv run` trên interpreter pin của dự án.

## 13. Kết luận cuối

Thiết kế timing nền tảng tốt và có định hướng accuracy-first rõ. Rủi ro chính không nằm ở CPU cho spin/timer, mà ở seam giữa authored time và completion time, supervisor và dispatch thread, event handle và fallback wait, cùng native/config validation.

Kết luận công tâm: **chưa đạt best practice hoàn chỉnh, nhưng không cần viết lại lõi**. Một chuỗi sửa nhỏ kèm controlled-clock/thread regression tests có giá trị hơn refactor rộng hoặc tối ưu CPU sớm. Chỉ sau khi H1–H6 và M1–M2 được khóa bằng test mới nên tối ưu materialization, telemetry và GC; mọi tối ưu phải giữ nguyên `SendInput`-only và độ chính xác deadline.
