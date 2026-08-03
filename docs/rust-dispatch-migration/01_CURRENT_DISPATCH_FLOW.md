# 01 — Phân tích luồng dispatch hiện tại

> HISTORICAL: This migration note describes the superseded pre-consolidation
> flow. The current production contract is Rust-only; see `README.md`.

## 1. Luồng tổng thể

```text
Song / scheduler KeyAction[]
  → PlaybackEngine.play()
  → compile_runtime_intents(actions)
  → RuntimeSchedule(batches + generation ids)
  → prewarm Win32 INPUT arrays
  → create realtime sleeper
  → PlaybackSupervisor
      ├─ control/UI thread: hotkey poll, focus sample, progress render
      └─ dispatch thread: DispatchLoop.run()
           → next_deadline
           → waitable timer/event + final busy-spin
           → drain pending releases first
           → drain authored batch one-by-one
           → backend.key_down/key_up
           → platform.send_scan_code_batch_trusted
           → user32.SendInput
           → completion timestamp
           → coordinator transition + telemetry
```

## 2. Composition và ownership trước playback

`PlaybackEngine.play()` thực hiện theo thứ tự quan trọng:

1. reset send/prewarm diagnostics;
2. rebuild `RuntimeSchedule` nếu session trước đã release data;
3. reset/revalidate Sky HWND khi focus bắt buộc;
4. prewarm INPUT packet shapes;
5. chờ focus ban đầu trước khi dựng timeline;
6. tạo `RuntimeDispatchCoordinator`;
7. chọn direct hoặc threaded dispatch;
8. tạo high-resolution sleeper khi có thể;
9. dựng `DispatchLoop` với backend, clock, wait strategy, estimator, focus hooks;
10. supervisor khởi chạy worker;
11. chỉ sau khi worker đã kết thúc mới clear timer/cache/schedule và persist lead cache.

Điểm dễ sai khi port: start epoch phải được rebase **sau** khi worker đã vào timer/priority scope và adaptive probe hoàn tất. Không được làm thêm công việc sau rebase trước `run()` vì sẽ làm các note ở t=0 bị trễ/compress.

## 3. Compile runtime generations

`compile_runtime_intents()` đi qua authored actions theo thứ tự và cấp `generation_id` cho từng key-down. Với mỗi scan code có một FIFO unmatched-down queue; key-up lấy generation đầu hàng. Unmatched key-up nhận `None` và về sau bị suppress.

Một authored chord trở thành `RuntimeActionBatch` chứa nhiều `RuntimeKeyIntent`, nhưng vẫn giữ:

- source action index;
- kind;
- scheduled timestamp;
- reason;
- per-key generation id.

Đây không phải metadata phụ. Generation là cơ sở để phân biệt một up cũ với một down mới cùng scan code.

## 4. Coordinator state machine

Trạng thái logic:

```text
SCHEDULED
  ├─ down sent       → ACTIVE
  ├─ same-key clash  → DROPPED_CONFLICT
  ├─ too late/focus  → DROPPED_EXPIRED
  └─ abort           → CANCELLED

ACTIVE
  └─ authored up     → RELEASE_PENDING

RELEASE_PENDING
  ├─ up landed       → RELEASED
  ├─ backend failed  → DROPPED_BACKEND / still tracked for safety
  └─ abort           → CANCELLED
```

Implementation hiện tại giữ live entries O(polyphony), không giữ status object cho toàn bộ bài. Terminal states được fold vào counter. Rust phải giữ đặc tính memory này.

### Deadline

Deadline tiếp theo là min của:

- authored batch tiếp theo, có per-batch lead;
- pending release sớm nhất, có up lead nhưng không được sớm hơn hold floor.

### No-early-conflict guard

Một down batch không được pop sớm do dispatch lead nếu scan code của nó vẫn active hoặc pending. Nếu pop sớm, nó sẽ bị hiểu thành same-key conflict và mất note. Khi guard block, deadline authored trả về thời điểm gốc, không phải `scheduled - lead`.

## 5. Wait path

### Event mode

Supervisor tạo auto-reset command event **trước** worker. Worker dùng high-resolution waitable timer và `WaitForMultipleObjects` với thứ tự:

```text
[command_event, timer_handle]
```

Do Windows trả index nhỏ nhất khi cả hai signal, command thắng deadline tại cạnh đồng thời. Rust phải giữ đúng thứ tự này.

### Poll mode

Không có event thì dispatch loop thức theo các bước tối đa 2 ms để poll command/focus. Direct mode luôn poll vì không có supervisor thread signal event.

### Final spin

Khi còn trong `spin_threshold_us`, loop busy-spin tới deadline. Không `sleep(0)`, không yield, không Python call, không logging.

### Adaptive spin

- pre-play probe: 30 lần sleep 2 ms trên dispatch context;
- threshold = clamp(max error + 200 µs, floor, 3000 µs);
- mid-song reprobe: chỉ trong gap >= 0.5 s, cách lần trước >= 30 s, 8 sample, hysteresis 50 µs;
- cold gap >= 20 ms: mở rộng final spin thêm 200 µs mặc định, cap 500 µs.

## 6. Drain ordering

Sau wake:

1. `pop_due_pending()` và gửi release trước;
2. pop **một** authored batch mỗi vòng;
3. nếu authored up, chuyển active generation sang pending rồi re-check release có due ngay;
4. nếu authored down, chạy focus gate/conflict split/send/activate;
5. observe result ngay, không materialize cả fan overdue trước lần gửi đầu.

Scalar drain là tối ưu correctness lẫn latency: catch-up burst không được scan/allocate toàn bộ overdue tuple trước khi gửi batch đầu.

## 7. Down dispatch

Thứ tự chính xác:

1. kiểm tra late-drop threshold;
2. focus gate:
   - shared focus signal từ supervisor;
   - khi signal active, fresh `GetForegroundWindow() == cached_sky_hwnd` để đóng race;
3. nếu mất focus: release trước, cancel generations, ghi `blocked_unfocused`;
4. split playable/conflicting intents;
5. strict conflict có thể raise; degraded mode drop conflict;
6. tạo contiguous scan-code batch;
7. gọi backend `key_down`;
8. lấy native completion timestamp;
9. update estimator bằng pure SendInput duration;
10. activate **chỉ** prefix thật sự đã landed;
11. release floor = `down_completion + min_hold`.

## 8. Up dispatch

Authored up không gửi ngay một cách mù quáng. Nó yêu cầu release của generation đang active. Pending release có:

```text
effective_release = max(scheduled_release - lead_up,
                        down_completion + min_hold)
```

Single release là fast path. Multi-release được gom theo due time nhưng completion result vẫn quyết định generation nào được terminalize.

## 9. Backend tracking

Backend có ba tập:

- `active_keys`: đã biết SendInput landed;
- `possibly_active_keys`: đang ở cửa sổ emit/exception không chắc chắn;
- `failed_release_keys`: release thất bại cần panic cleanup.

Dedupe rules:

- duplicate down: không gửi lại;
- up của key không active/possible: idempotent skip;
- partial down: chỉ sent prefix chuyển active;
- partial up: phần chưa gửi vẫn bị giữ để `release_all` reclaim.

## 10. Native SendInput policy

### Note-on

- call 1 với toàn chord;
- nếu partial: call 2 ngay lập tức cho remainder;
- không sleep;
- sau call 2, tail còn thiếu bị drop;
- telemetry phân biệt `partial_note_on`, `keys_retried`, `keys_dropped`, `chord_split_events`.

### Note-off và safety release

- phải cố hoàn tất remainder;
- progress reset zero-progress counter;
- ba lần liên tiếp không tiến triển thì raise;
- zero-progress retry sleep 2 ms;
- panic/release_all có thêm multi-pass + physical-state verification.

## 11. Focus flow

Có ba cấp:

1. full process-name/window validation trên control thread, cadence chậm;
2. cached HWND compare trên control thread, cadence 20–50 ms;
3. pre-down fresh HWND compare trên dispatch thread, rất rẻ.

Khi focus mất:

- first KEYUP/release ngay lúc mất;
- enter focus pause, elapsed freeze;
- khi focus trở lại, grace window xử lý command;
- second idempotent KEYUP;
- đóng pause interval và rebase elapsed qua accumulated pause.

## 12. Command flow

Commands: pause, resume/toggle, skip, quit, panic, refocus.

- quit/skip trả terminal result;
- panic full-instrument release và publish status;
- pause dùng unified abort helper rồi enter manual pause;
- resume chỉ đóng manual reason; nếu focus reason còn, timeline vẫn pause;
- refocus trên threaded mode chủ yếu do supervisor xử lý để tránh Win32 focus policy trên RT worker.

## 13. Teardown

Dispatch thread có authoritative liveness flag. Nếu join timeout 5 s:

- trả `shutdown_timeout`;
- không close command event/timer;
- engine không clear INPUT cache;
- không drop coordinator/schedule;
- không `gc.collect()`;
- không persist estimator;
- engine bị xem là poisoned cho lần play tiếp theo.

Đây là ownership contract bắt buộc, không phải cleanup optimization.
