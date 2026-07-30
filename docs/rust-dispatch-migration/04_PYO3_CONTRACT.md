# 04 — PyO3 contract và Python adapter

## 1. Module declaration

Dùng PyO3 0.29 và khai báo free-thread safety rõ ràng:

```rust
#[pyo3::pymodule(gil_used = false)]
mod sky_player_rs {
    // registration only
}
```

PyO3 0.28+ mặc định coi module thread-safe, nhưng annotation explicit là review guard. Không đặt `gil_used = true`, vì import sẽ bật lại GIL và phá invariant runtime của dự án.

## 2. Không dùng API cũ

- Thay `py.allow_threads(...)` bằng `py.detach(...)`.
- `Python<'py>` trên free-threaded build nghĩa là thread đang attached, không phải lock toàn cục.
- Không giữ `Bound<'py, PyAny>` hoặc `PyObject` trong worker/core.

## 3. Public API đề xuất

```python
class DispatchSession:
    @classmethod
    def prepare(
        cls,
        actions: Sequence[ActionDTO],
        config: RuntimeConfigDTO,
        allowed_scan_codes: Sequence[int],
        estimator_state_json: str | None,
    ) -> DispatchSession: ...

    def start(self) -> None: ...
    def send_command(self, command: str) -> bool: ...
    def update_focus(self, active: bool, hwnd: int | None) -> None: ...
    def snapshot(self) -> ProgressSnapshotDTO: ...
    def try_result(self) -> PlaybackResultDTO | None: ...
    def join(self, timeout_ms: int | None = None) -> PlaybackResultDTO: ...
    def take_telemetry(self) -> list[TelemetryRecordDTO]: ...
    def emergency_release(self, full_instrument: bool = True) -> ReleaseAllDTO: ...
    def close(self) -> None: ...
```

`prepare` có thể là constructor nhưng classmethod làm lifecycle rõ: prepared → running → terminal → closed.

## 4. DTO contract

### ActionDTO

Version 1 đơn giản, parse một lần:

```python
@dataclass(frozen=True, slots=True)
class ActionDTO:
    source_action_index: int
    kind: Literal["down", "up"]
    at_us: int
    scan_codes: tuple[int, ...]
    reason: str
```

Rust tự compile generation ở target cuối.

Validation:

- `source_action_index`: 0..u32::MAX, monotonic theo input;
- `kind`: exact enum;
- `at_us`: 0..u64::MAX, nondecreasing;
- `scan_codes`: 1..15, strict ints, no duplicate, allowed;
- `reason`: bounded UTF-8 length, ví dụ <=128 bytes;
- tổng actions/generations có cap chống input độc hại/accidental OOM.

### RuntimeConfigDTO

Bao gồm primitives, không truyền Python policy object:

```text
min_hold_us
focus_restore_grace_us
late_pulse_drop_threshold_us | None
same_key_conflict_policy
spin_threshold_us
spin_floor_us
core_warmup_budget_us
enable_adaptive_spin
enable_spin_reprobe
enable_event_wait
enable_waitable_timer
rt_priority_mode
dispatch_lead_us
total_time_us
telemetry_enabled
telemetry_capacity
```

Cấu hình phải được version hóa (`schema_version=1`).

### Snapshot

Để giảm allocation, có hai lựa chọn:

1. `#[pyclass(frozen)] ProgressSnapshotDTO` — rõ type, ít dict churn;
2. tuple cố định — nhanh hơn nhưng khó maintain.

Khuyến nghị pyclass frozen với primitive fields.

## 5. Detach rules

### Phải `py.detach`

- `join()` chờ worker;
- heavy prepare after Python DTOs đã extract thành Rust-owned structs;
- emergency release có backoff/sleep;
- benchmark/selftest chạy lâu.

### Không cần detach

- `send_command`;
- `update_focus`;
- `snapshot`;
- `try_result`;
- parse DTO ngắn khi đang đọc Python objects.

Ví dụ:

```rust
fn join(&self, py: Python<'_>, timeout_ms: Option<u64>) -> PyResult<PyPlaybackResult> {
    let native = py.detach(|| self.inner.join(timeout_ms))?;
    Python::attach(|py| native.into_py_dto(py))
}
```

Thực tế method đã có `py` attached; sau `detach` closure trả native data, rồi convert trong cùng call khi attached trở lại.

## 6. Free-threaded pyclass design

Không dùng mutable pyclass borrow như state store. Mẫu đúng:

```rust
#[pyclass(frozen)]
struct DispatchSession {
    inner: Arc<SessionShared>,
}
```

Methods `&self`. Interior mutation qua atomics/channel/Mutex ngắn. Không chạy Python code khi giữ Rust lock.

Lock ordering phải được document:

```text
join_lock → result_lock
progress_lock độc lập
không bao giờ giữ lock khi SetEvent, SendInput, join hoặc Python conversion
```

## 7. No callback rule

Cấm API kiểu:

```python
session.start(telemetry_callback)
```

Lý do:

- worker phải attach Python runtime;
- có thể deadlock với GC global sync ở free-threaded Python;
- callback latency chèn vào timing;
- teardown phải quản lý PyObject lifetime;
- exception Python có thể phá worker ownership.

Thay bằng push native state → Python pull snapshot/result.

## 8. Python adapter

Tạo một file duy nhất, ví dụ:

```text
src/sky_music/orchestration/native_dispatch.py
```

Responsibilities:

- import/availability check;
- convert `KeyAction` → DTO;
- choose Rust/Python runtime;
- map commands/outcomes;
- pull progress và feed renderer;
- convert native telemetry vào `TelemetryLogger` hoặc direct CSV writer sau join;
- preserve exception messages at UI boundary.

Adapter không được:

- duplicate coordinator state;
- read active key sets;
- call SendInput;
- compute deadlines;
- perform retry;
- mutate Rust lifecycle internals.

## 9. Lifecycle API rules

State transitions:

```text
NEW → PREPARED → RUNNING → TERMINAL → CLOSED
                       └→ POISONED (join timeout)
```

- `start` twice: error;
- command before start: error hoặc explicit queue-before-start behavior, không mơ hồ;
- `take_telemetry` trước terminal: error;
- `close` while running: cooperative quit + join; nếu timeout thì POISONED và không close handles;
- `__del__` chỉ best effort, không dựa vào destructor để correctness;
- context manager có thể thêm sau, nhưng explicit lifecycle là chuẩn.

## 10. Module import and wheel

Không dùng abi3 feature. Build bằng exact interpreter:

```powershell
uv run --python 3.14t maturin build   --manifest-path rust/Cargo.toml   --release   --interpreter "$env:VIRTUAL_ENV\Scripts\python.exe"
```

Tên module top-level `sky_player_rs` giữ packaging đơn giản và tương thích kế hoạch cũ. Python package import thông qua adapter, không import trực tiếp khắp codebase.
