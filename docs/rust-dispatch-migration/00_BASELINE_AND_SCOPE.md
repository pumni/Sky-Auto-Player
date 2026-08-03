# 00 — Baseline, phạm vi và quyết định thiết kế

> HISTORICAL: This migration note records the pre-consolidation design. The
> current production contract is Rust-only; see `README.md` in this directory.

## 1. Baseline bất biến

Mọi patch phải được so sánh với commit:

```text
45fa3c83b1642fb6c91caa2c13b3ce7d1a6cad52
```

Trước khi coding, AI phải xác nhận `git rev-parse HEAD`. Nếu HEAD đã đi tiếp, phải chạy lại bước inventory và cập nhật `references/SOURCE_MAP.md`; không được giả định symbol/contract vẫn giống baseline.

## 2. Phạm vi “toàn bộ lõi gửi phím”

### Chuyển sang Rust ở đích cuối

- `orchestration/core/loop.py`
- `orchestration/core/coordinator.py`
- phần timing của `orchestration/core/state.py`
- `SendLatencyEstimator` trong `orchestration/engine.py`
- `infrastructure/wait_strategy.py`
- real-time sleeper/waitable timer/event và dispatch thread priority scope
- `_TrackedKeyState` + `WinSendInputBackend`
- hot path trong `platform/win32/inputs.py`:
  - INPUT packet creation/prewarm
  - `SendInput`
  - retry policy
  - completion timestamp
  - tracked release/panic release
  - cheap foreground HWND compare
- bounded dispatch telemetry và live progress counters
- authoritative worker start/stop/join/teardown ownership

### Giữ ở Python

- Textual UI, renderer và picker
- CLI/config/profile selection
- parsing song và domain scheduler cấp cao trong giai đoạn đầu
- full target-window discovery/process-name validation
- watchdog process hiện tại trong migration; có thể thay sau bằng binary Rust riêng nhưng không nằm trong critical path
- CSV/JSON file writing sau playback
- calibration UI và game-observed evidence pipeline

### Có thể chuyển sau khi core ổn định

- `compile_runtime_intents`: nên chuyển vào Rust ở target cuối để Python chỉ gửi authored `KeyAction`; nhưng nên đi qua một phase parity với compiled schedule hiện tại.
- telemetry summary aggregation: có thể giữ Python để không đổi schema, trong khi raw records/counters do Rust tạo.

## 3. Các tài liệu cũ cần xử lý trước code

`docs/rust-migration-plan.md` là proposal cũ, có nhiều ý tốt nhưng không còn đúng hoàn toàn:

- dependency PyO3 cũ;
- callback telemetry Python từ executor thread làm ownership và shutdown phức tạp;
- còn trường `onset_bias_us` đã bị loại bỏ;
- pseudocode wait đặt timer trước command event, trong khi code hiện tại cố ý đặt command event ở index 0 để command thắng khi đồng thời signal;
- mô hình pending heap/HashMap chưa tận dụng giới hạn polyphony nhỏ;
- chưa phản ánh lifecycle teardown hardening ngày 2026-07-29.

`docs/PORTING_GUIDE.md` hiện có mục nói signal-dispatch hot path chưa được phép chuyển. PR đầu tiên phải cập nhật mục này hoặc thay bằng liên kết đến bộ tài liệu mới; nếu không AI khác sẽ nhận chỉ dẫn mâu thuẫn.

## 4. Kiến trúc migration được chọn

Dùng **strangler migration** với hai implementation sau một Python adapter ổn định:

```python
DispatchRuntime = PythonDispatchRuntime | RustDispatchRuntime
```

- Python implementation là oracle trong giai đoạn differential testing.
- Rust implementation bật bằng config nội bộ/feature flag.
- Không rải `if rust_available` xuyên UI/engine. Chỉ chọn implementation tại composition root.
- Sau khi Rust là default và ổn định, Python core được giữ một release làm fallback rồi xóa có chủ đích.

## 5. Version và packaging

### Pin khuyến nghị

```toml
[workspace.package]
edition = "2024"
rust-version = "1.97"

[dependencies]
pyo3 = { version = "=0.29.0", features = ["extension-module"] }
windows-sys = { version = "=0.61.2", features = [ ... ] }
thiserror = "2"
smallvec = "1"
crossbeam-channel = "0.5"
parking_lot = "0.12"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

`Cargo.lock` phải commit. Toolchain pin `1.97.1-x86_64-pc-windows-msvc` để reproducible.

### Không dùng `abi3` cho 3.14t

CPython 3.14 free-threaded chưa có limited stable ABI dùng được cho extension wheel kiểu `abi3`. Build phải dùng chính interpreter 3.14t và tạo wheel version-specific, ví dụ tag thuộc họ `cp314-cp314t-win_amd64`.

## 6. Định nghĩa “full migration”

Migration chỉ hoàn thành khi:

- giữa deadline wake và `SendInput` không chạy Python;
- active/pending generation state nằm hoàn toàn trong Rust;
- wait timer/event/MMCSS và `SendInput` cùng một Rust worker;
- pause/focus/command được worker xử lý bằng state native;
- Python chỉ push command/focus và pull snapshot;
- UI không cần biết implementation là Python hay Rust;
- pytest hiện tại, differential tests, Rust tests và Windows integration tests đều qua;
- Python dispatch core không còn nằm trên production path.
