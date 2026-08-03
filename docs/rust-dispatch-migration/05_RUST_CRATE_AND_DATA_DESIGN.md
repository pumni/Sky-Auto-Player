# 05 — Crate layout và data-oriented design

> HISTORICAL: This migration note records an earlier design review. The
> current production contract is Rust-only; see `README.md`.

## 1. Workspace đề xuất

```text
rust/
├─ Cargo.toml
├─ rust-toolchain.toml
├─ crates/
│  ├─ sky_dispatch_core/
│  │  ├─ src/model.rs
│  │  ├─ src/compile.rs
│  │  ├─ src/coordinator.rs
│  │  ├─ src/playback_clock.rs
│  │  ├─ src/estimator.rs
│  │  ├─ src/telemetry.rs
│  │  ├─ src/worker.rs
│  │  └─ src/testing.rs
│  ├─ sky_dispatch_win32/
│  │  ├─ src/clock.rs
│  │  ├─ src/input.rs
│  │  ├─ src/wait.rs
│  │  ├─ src/focus.rs
│  │  ├─ src/priority.rs
│  │  └─ src/handles.rs
│  └─ sky_player_rs/
│     ├─ src/dto.rs
│     ├─ src/errors.rs
│     ├─ src/session.rs
│     └─ src/lib.rs
└─ benches/
```

Dependency direction:

```text
sky_dispatch_core  ← traits/primitives only, no PyO3, no Windows
sky_dispatch_win32 → implements core traits, Windows-only unsafe seam
sky_player_rs      → PyO3 + composes core/win32
```

Core phải `cargo test` được trên non-Windows. Win32 crate dùng `cfg(windows)` và test fake/system wrappers.

## 2. Avoid dynamic graph/heap

Không port trực tiếp:

- Python dict per generation;
- deque per scan code;
- strings per pending release;
- heap allocations mỗi action;
- Arc<Mutex> cho mọi component.

### Key registry

Vì allowed keys nhỏ:

```rust
struct KeyRegistry {
    scan_codes: SmallVec<[u16; 15]>,
}
```

`slot_for(scan)` có thể linear scan; benchmark trước khi thêm HashMap. Runtime arrays index bằng slot.

### Generation compile

Cần FIFO unmatched downs per key. Dùng `SmallVec<VecDeque<u64>>` trong prepare được phép vì không RT. Sau compile, queue drop.

### Live status

```rust
struct GenerationCounts { scheduled, active, pending, released, ... }
```

Không giữ terminal status array cho toàn song. Scheduled implicit = generation_count - terminals - live.

## 3. Pending release structure

Polyphony <=15. `SmallVec<[PendingRelease; 15]>` có lợi:

- không heap trong common case;
- remove by swap/shift có bound nhỏ;
- min scan O(15), predictable;
- dễ preserve tie-break `(effective, source_index, scan_code)`.

Nếu benchmark cho thấy BinaryHeap tốt hơn, heap phải preallocate capacity và tie ordering test đầy đủ.

## 4. Action/batch data

Use integer IDs:

```rust
type GenerationId = u64;
type ReasonId = u16;
type PacketId = u32;
type KeySlot = u8;
```

`RuntimeIntent` giữ key slot + generation id, không duplicate scan code string/object.

## 5. Estimator

Port exact behavior:

- seed count 5;
- down EMA bucket theo polyphony, clamp bucket;
- nearest seeded bucket <= N, rồi total fallback;
- up scalar EMA;
- residual down-only, positive contribution, sample clamp và max residual 500 µs;
- total lead clamp max 2000 µs mặc định;
- import/export schema version 2 tương thích JSON hiện tại trong parity phase.

Dùng f64 để parity với Python float/round. Rust `f64::round()` khác Python bankers rounding ở .5; phải implement Python-compatible round-to-even hoặc freeze golden cases. Không bỏ qua điểm này.

## 6. Time types

Không trộn đơn vị. Dùng newtypes:

```rust
struct Micros(u64);
struct PerfTicks(i64);
```

QPC conversion phải tránh overflow:

```text
us = ticks * 1_000_000 / frequency
```

Có thể dùng i128 trung gian. Frequency cache once.

Timeline elapsed dùng saturating arithmetic ở boundary nhưng internal invariant violations phải error/test, không che hết bằng saturating math.

## 7. TelemetryRecord layout

Native record giữ tuples nhỏ inline:

```rust
struct TelemetryRecord {
    event_index: u32,
    dispatch_id: u64,
    kind: ActionKind,
    scheduled_us: u64,
    actual_us: u64,
    completed_us: u64,
    lateness_us: i64,
    visible_lateness_us: i64,
    send_duration_us: u32,
    send_duration_pure_us: u32,
    bookkeeping_us: u32,
    dispatch_lateness_us: i64,
    scan_codes: SmallVec<[u16; 8]>,
    sent_scan_codes: SmallVec<[u16; 8]>,
    skipped_scan_codes: SmallVec<[u16; 8]>,
    generation_ids: SmallVec<[u64; 8]>,
    outcome: RuntimeOutcome,
    reason_id: ReasonId,
    ...
}
```

Không format `"1;2;3"` trên worker. Python/file writer format sau join.

## 8. Unsafe policy

`sky_dispatch_core`: `#![forbid(unsafe_code)]`.

`sky_player_rs`: ideally `#![forbid(unsafe_code)]`; PyO3 macros tự sinh unsafe bên ngoài source không phải lý do cho handwritten unsafe.

`sky_dispatch_win32`: `#![deny(unsafe_op_in_unsafe_fn)]`; mỗi unsafe block có:

- pointer nguồn/lifetime;
- length/capacity;
- struct layout từ windows-sys;
- handle validity/ownership;
- thread ownership.

Không tạo generic “unsafe utils”. Mỗi wrapper nhỏ và typed.

## 9. Handle wrappers

RAII wrappers:

```rust
OwnedHandle
OwnedWaitableTimer
OwnedEvent
MmcssRegistration
TimerResolutionGuard
PowerThrottlingGuard
```

Nhưng Drop chỉ chạy khi worker đã thực sự own và terminate. Session shared event tạo trước worker cần ownership transfer rõ; tránh double-close.

## 10. Logging

Worker không gọi Python logger. Có bounded diagnostic ring riêng hoặc enum `DiagnosticEvent` trong telemetry. Win32 errors lưu numeric code + short static context; format human-readable sau playback.

## 11. Release profile

Khuyến nghị ban đầu:

```toml
[profile.release]
lto = "thin"
codegen-units = 1
opt-level = 3
panic = "unwind"
strip = "symbols"
overflow-checks = true
```

Không bật `target-cpu=native` cho binary phân phối rộng. Có thể benchmark PGO riêng sau parity.
