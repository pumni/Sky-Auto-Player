# 03 — Kiến trúc Rust đích

> HISTORICAL: This migration note records the target proposal. The current
> production contract is Rust-only; see `README.md`.

## 1. Boundary

Python chỉ giữ application orchestration. Rust session nhận dữ liệu immutable, tự spawn worker, và expose thread-safe control/snapshot methods.

```text
Python main/UI thread
 ├─ parse/config/scheduler
 ├─ target HWND discovery + full validation
 ├─ hotkey polling
 ├─ renderer at ~30 Hz
 └─ telemetry persistence after join
          │
          │ PyO3 methods, all short/nonblocking except join
          ▼
Arc<SessionShared>
 ├─ bounded command TX
 ├─ AtomicBool focus_active
 ├─ AtomicIsize target_hwnd
 ├─ auto-reset interrupt event
 ├─ progress snapshot lock/version
 └─ worker JoinHandle/result
          │
          ▼
Rust dispatch worker — sole owner
 ├─ RuntimeKernel
 ├─ QPC clock/epoch/pause state
 ├─ HybridWait
 ├─ Win32Sender + tracked state
 ├─ adaptive estimator
 ├─ telemetry Vec (bounded retain-first)
 └─ timer/MMCSS/power-throttling RAII guards
```

## 2. Session object model

`DispatchSession` PyO3 class chứa `Arc<SessionShared>`. Không expose `&mut self` cho Python; tất cả methods dùng `&self`, vì Python 3.14t có thể gọi đồng thời.

```rust
struct SessionShared {
    lifecycle: AtomicU8,
    command_tx: Sender<Command>,
    focus_active: AtomicBool,
    target_hwnd: AtomicIsize,
    progress: Mutex<ProgressSnapshot>,
    progress_version: AtomicU64,
    result: Mutex<Option<WorkerResult>>,
    join: Mutex<Option<JoinHandle<()>>>,
    interrupt_event: OwnedEvent,
}
```

`OwnedEvent` có thể được tạo trước worker để không mất wake-up startup. Chỉ close khi JoinHandle đã kết thúc.

## 3. Worker ownership

Worker move vào:

- prepared schedule;
- packet table;
- backend state;
- estimator state;
- immutable config;
- timer handle/MMCSS guards;
- telemetry buffer.

Không để các object trên vừa nằm trong worker vừa mutable qua PyO3. Shared side chỉ chứa command/focus/snapshot/result.

## 4. Command channel

Dùng bounded `crossbeam_channel` (đề xuất capacity 64). `send_command`:

1. strict parse enum ở PyO3 edge;
2. `try_send`;
3. signal Win32 event;
4. trả `accepted: bool` hoặc typed error.

Không dùng unbounded channel. Khi full:

- panic/quit không được silent drop: fallback set atomic urgent bits và signal event;
- repeated pause/refocus có thể coalesce có chủ đích nhưng phải có test;
- phase đầu đơn giản nhất: capacity 64 và return error để Python retry ở poll kế tiếp.

## 5. Focus state

Python supervisor vẫn làm full process-name validation. Nó gọi:

```python
session.update_focus(active, hwnd)
```

Rust worker:

- đọc `focus_active` trước down;
- nếu true và `hwnd != 0`, gọi `GetForegroundWindow() == hwnd` ngay trước SendInput;
- không OpenProcess/process-name query trên hot path;
- focus transition signal interrupt event để đánh thức wait.

## 6. RuntimeKernel

```rust
struct RuntimeKernel {
    config: RuntimeConfig,
    schedule: Box<[RuntimeBatch]>,
    cursor: usize,
    key_registry: KeyRegistry,
    active: [Option<ActiveGeneration>; MAX_KEYS],
    pending: SmallVec<[PendingRelease; MAX_KEYS]>,
    terminal_counts: GenerationCounts,
    generation_count: u64,
    playback: PlaybackClockState,
    estimator: SendLatencyEstimator,
    telemetry: TelemetryBuffer,
    counters: RuntimeCounters,
}
```

Vì instrument tối đa 15 keys, fixed/small inline storage thường tốt hơn HashMap/BinaryHeap. Không tối ưu mù: benchmark `SmallVec + linear min` so với heap/map, nhưng default thiết kế phải allocation-free sau prepare.

## 7. Prepared schedule

Target cuối Python gửi authored `KeyAction` DTO; Rust thực hiện generation compile. Internal batch:

```rust
struct RuntimeBatch {
    source_action_index: u32,
    kind: ActionKind,
    scheduled_us: u64,
    reason_id: u16,
    intents: SmallVec<[RuntimeIntent; 8]>,
    packet_id: PacketId,
}
```

Để tránh String mỗi event:

- reason table intern ở prepare;
- scan codes validate và map sang key slot;
- authored packet shape intern thành `PacketTable`;
- prebuild singleton up packet cho mọi allowed key.

## 8. Packet table

Không cần Python OrderedDict cache trên worker. Prepare tạo frozen packet table:

```rust
struct InputPacket {
    inputs: SmallVec<[INPUT; 15]>,
}
```

Sau freeze, Vec/SmallVec không mutate. Mỗi send lấy slice pointer tạm thời; không lưu raw pointer lâu dài. Dynamic remainder dùng slice của scan-code list và stack/SmallVec packet; max 15 nên không heap.

## 9. Wait architecture

`HybridWait` là concrete struct, không nhất thiết dynamic trait trên production path. Trait chỉ dùng cho test injection:

```rust
trait Clock { fn now_us(&self) -> u64; }
trait InputSink { fn emit(...) -> SendResult; }
trait Waiter { fn wait(...) -> WakeReason; }
```

Production có monomorphized worker generic hoặc concrete types. FakeClock/FakeSender/FakeWaiter chạy cargo tests không cần Python/Windows.

## 10. Progress và telemetry

### Live progress

Latest-wins snapshot, bounded một object:

- elapsed/status/counters/health;
- worker publish tối đa khi status đổi hoặc theo bounded cadence;
- Python pulls at renderer cadence.

### Detailed telemetry

Worker-owned retain-first `Vec<TelemetryRecord>` capacity 200,000. Không lock trên record path. Sau worker terminal, session chuyển buffer vào result; Python `take_telemetry()` lấy một lần và serialize.

Nếu cần mid-song telemetry inspect, expose aggregate atomics/snapshot, không expose detail Vec đang mutate.

## 11. Error model

Rust domain errors:

```rust
PrepareError
StartError
CommandError
WaitError
SendInputError
ReleaseError
LifecycleError
```

PyO3 map sang custom exception classes hoặc `PyValueError`/`PyRuntimeError` ở edge. Hot worker không tạo PyErr.

Worker result chứa:

```rust
struct WorkerResult {
    outcome: PlaybackOutcome,
    error: Option<WorkerErrorReport>,
    release: ReleaseAllOutcome,
    generation_counts: GenerationCounts,
    telemetry: TelemetryBuffer,
    estimator_state: EstimatorState,
}
```

## 12. Panic strategy

- Workspace release profile `panic = "unwind"` trong migration để `catch_unwind` ở worker boundary có thể chạy release.
- Không `panic=abort` cho đến khi có external watchdog guarantees và quyết định riêng.
- Worker closure `catch_unwind(AssertUnwindSafe(...))`.
- On panic: best-effort full release, store structured panic report, set terminal lifecycle, signal completion event.
- Không bắt panic quanh từng note; chỉ boundary.
