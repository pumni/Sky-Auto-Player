# KẾ HOẠCH CẢI TIẾN TOÀN DIỆN RUST REAL-TIME DISPATCH CORE

## 0. Mục đích tài liệu

Tài liệu này là đặc tả triển khai cho hệ thống lõi gửi phím Rust của Sky Auto Player.

Coding agent phải xem đây là một kế hoạch kỹ thuật có tính ràng buộc, không phải danh sách gợi ý. Mọi thay đổi phải ưu tiên theo thứ tự:

1. **Tính đúng và độ ổn định của toàn bộ phiên phát.**
2. **Độ chính xác về thời gian tại đúng boundary cần đo.**
3. **Tính toàn vẹn của chord và trạng thái phím.**
4. **Khả năng kiểm chứng bằng test và benchmark.**
5. **CPU, RAM và khả năng tận dụng Rust.**

Không được ưu tiên micro-optimization nếu các invariant về timing, chord hoặc key lifecycle chưa được chứng minh.

---

# 1. Nguyên tắc bắt buộc khi triển khai

## 1.1 Không giả định code hiện tại là tối ưu

Coding agent phải phản biện từng cơ chế hiện có.

Không được giữ lại một thiết kế chỉ vì:

* Nó đã tồn tại lâu.
* Nó có nhiều test.
* Nó được mô tả là “best practice” trong tài liệu.
* Benchmark mock hiện tại đang pass.
* Rust implementation đang tương đương Python oracle.

Code và tài liệu hiện tại là dữ liệu đầu vào, không phải kết luận cuối cùng.

## 1.2 Không đánh đồng các loại thời gian

Trong toàn bộ implementation và telemetry phải tách rõ:

```text
authored timestamp
dispatch deadline
worker wake time
SendInput call start
SendInput call completion
OS input delivery
game-observed input
game frame/audio onset
```

Không được gọi `SendInput` completion là “game onset”.

Không được tuyên bố game nhận note đúng timestamp nếu chỉ có dữ liệu phía sender.

## 1.3 Không làm mega-PR

Mỗi phase phải được triển khai bằng một hoặc nhiều PR nhỏ, có thể review độc lập.

Mỗi PR phải:

* Có một mục tiêu kỹ thuật chính.
* Không trộn refactor cấu trúc với thay đổi timing semantics nếu không bắt buộc.
* Có test chứng minh behavior cũ hoặc behavior mới.
* Có benchmark trước và sau nếu PR chạm hot path.
* Có cập nhật tài liệu tương ứng.
* Có phương án rollback hoặc kill switch nếu thay đổi ảnh hưởng production timing.

## 1.4 Không tối ưu bằng trực giác

Mọi tối ưu hot path phải có:

```text
baseline
patch
benchmark trên cùng môi trường
so sánh kết quả
kết luận giữ hoặc revert
```

Mock backend chỉ được dùng để kiểm tra state machine và regression sơ bộ.

Không dùng mock backend làm bằng chứng cuối cùng cho real Windows timing.

## 1.5 Python vẫn là oracle trong giai đoạn chuyển đổi

Không xóa Python fallback hoặc differential oracle trong kế hoạch này.

Rust phải được chứng minh:

* Tương đương hoặc tốt hơn về semantics.
* Không tạo note lifecycle khác biệt ngoài những thay đổi đã được đặc tả.
* Có timing tốt hơn trên fixed-host benchmark.
* Có failure handling rõ hơn.

---

# 2. Chuẩn bị trước khi sửa code

## Phase PRE-0 — Khóa baseline và lập bản đồ hiện trạng

### Mục tiêu

Tạo một baseline có thể tái lập trước khi bắt đầu refactor.

### Công việc

1. Ghi lại:

   * Git commit hiện tại.
   * Rust toolchain.
   * Python version và ABI.
   * Windows version.
   * CPU.
   * Power plan.
   * Timer mode.
   * MMCSS tier thực tế.
   * Game FPS được dùng để test.
   * Cấu hình adaptive spin/lead.
   * Build profile.

2. Chạy toàn bộ gate hiện có:

```powershell
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo check --manifest-path rust/Cargo.toml --workspace --all-targets --all-features
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --workspace --all-features
uv run pytest -m "not slow"
uv run --env-file .env python scripts/bench_native_acceptance.py `
  --actions 512 `
  --repeats 5 `
  --polyphony 1,2,3,5,8,15 `
  --label before-overhaul `
  --output artifacts/native-before-overhaul.json
```

3. Chạy benchmark real `SendInput` trên một window do ứng dụng sở hữu hoặc fixed-host riêng.

4. Chạy ít nhất một bài nhạc dài trên:

   * Python backend.
   * Rust backend.
   * Cùng cấu hình.
   * Cùng máy.
   * Cùng bài.
   * Cùng số lần lặp.

5. Lưu telemetry gốc:

   * Down completion error.
   * Up completion error.
   * Wake overshoot.
   * Send duration.
   * Bookkeeping duration.
   * Spin time.
   * RSS.
   * CPU time.
   * Conflict/drop/retry counters.
   * Drift đầu bài so với cuối bài.

### Artifact phải tạo

```text
docs/perf-baselines/2026-08-rust-overhaul-before.md
artifacts/native-before-overhaul.json
artifacts/python-before-overhaul.json
artifacts/environment-before-overhaul.json
```

### Tiêu chí hoàn thành

Không bắt đầu thay đổi semantics trước khi baseline được commit hoặc lưu ngoài repository với checksum rõ ràng.

---

# 3. Kiến trúc mục tiêu

Sau toàn bộ kế hoạch, lõi nên có kiến trúc logic sau:

```text
Python authored actions
        │
        ▼
Strict PyO3 boundary validation
        │
        ▼
Physical-feasibility compiler
        │
        ├── Reject impossible same-key overlap
        ├── Canonicalize chord order
        ├── Define same-timestamp packet semantics
        └── Produce compact immutable schedule
        │
        ▼
Native worker-owned runtime
        │
        ├── QPC tick-domain deadlines
        ├── Fixed-slot key state
        ├── Bounded release recovery
        ├── Dedicated wait strategy
        ├── Chord-aware lead predictor
        └── No Python/PyO3 work in RT path
        │
        ▼
Physical transaction builder
        │
        ▼
SendInput
        │
        ├── Full success
        ├── Zero-progress bounded retry
        ├── Partial insertion → fatal integrity loss
        └── Structured Win32 error evidence
        │
        ▼
Worker-local metrics
        │
        ▼
Throttled coherent snapshot publication
```

---

# 4. P0 — Loại bỏ schedule vật lý bất khả thi

## Phase P0.1 — Compile-time same-key feasibility validation

### Vấn đề

Compiler hiện có thể ghép nhiều generation trên cùng physical scan code bằng FIFO. Trong khi đó, bàn phím vật lý chỉ có hai trạng thái:

```text
key up
key down
```

Hai generation cùng active trên một phím không thể biểu diễn chính xác.

Việc phát hiện conflict ở runtime là quá muộn.

### File chính

```text
rust/crates/sky_dispatch_core/src/compile.rs
rust/crates/sky_dispatch_core/src/model.rs
rust/crates/sky_dispatch_core/src/testing.rs
src/sky_music/orchestration/core/coordinator.py
tests/
docs/timing-principles.md
docs/rt-dispatch-architecture.md
```

### Thay đổi bắt buộc

Thay:

```rust
[VecDeque<GenerationId>; MAX_KEYS]
```

bằng state compile-time có tối đa một open generation trên mỗi physical slot:

```rust
#[derive(Clone, Copy, Debug)]
struct OpenGeneration {
    generation_id: GenerationId,
    down_action_index: u32,
    down_scheduled_us: u64,
}

let mut open_generation_by_slot: [Option<OpenGeneration>; MAX_KEYS];
```

### Quy tắc compile

#### Down khi key đang Up

Hợp lệ:

```text
Up → Down
```

Tạo generation mới.

#### Down khi key đã Down

Fatal compile error:

```rust
CompileError::OverlappingSameKeyDown {
    scan_code,
    first_down_action_index,
    second_down_action_index,
    first_scheduled_us,
    second_scheduled_us,
}
```

Không được đẩy lỗi này xuống runtime conflict policy.

#### Up khi key đang Down

Hợp lệ.

Đóng đúng generation hiện tại.

#### Up khi key đang Up

Trong phase này giữ compatibility hiện tại:

* Không tạo generation.
* Đánh dấu stale/unmatched Up.
* Runtime có thể suppress.
* Compiler phải đếm và xuất diagnostic.

Không đổi stale Up thành fatal error trong cùng PR trừ khi toàn bộ parser/scheduler upstream đã chứng minh không thể tạo stale Up.

### Test bắt buộc

1. Một Down và một Up hợp lệ.
2. Down–Down cùng key trước Up phải bị reject.
3. Down chord, sau đó Down chord khác chứa một key vẫn active phải bị reject.
4. Down key A và Down key B độc lập vẫn hợp lệ.
5. Up không có Down không tạo generation.
6. Một key được tái sử dụng sau Up hợp lệ.
7. Property test:

   * Sau compile thành công, mỗi physical slot có tối đa một open generation.
   * Không generation nào được ghép với nhiều Up.
   * Generation ID tăng đơn điệu.
8. Differential test Rust/Python:

   * Cùng input hợp lệ phải tạo cùng lifecycle.
   * Input overlap phải bị reject ở cả production path và oracle path.

### Tiêu chí hoàn thành

Không còn runtime-authored conflict nào xuất phát từ một schedule đã được compiler chấp nhận trong clean playback.

`authored_conflict_events` phải bằng zero đối với mọi schedule production hợp lệ.

---

## Phase P0.2 — Định nghĩa chính thức same-timestamp semantics

### Vấn đề

Up và Down cùng authored timestamp có thể dẫn đến:

```text
SendInput(Up)
SendInput(Down)
```

Down bị trễ bởi toàn bộ syscall trước đó.

### Quyết định cần đặc tả

Một timestamp có thể chứa:

```text
0..N Up intents
0..1 Down chord
```

Không cho phép nhiều Down chord độc lập cùng timestamp.

### Mô hình dữ liệu đề xuất

Thêm khái niệm packet:

```rust
struct CompiledPacket {
    scheduled_us: u64,
    release_start: u32,
    release_len: u8,
    down_start: u32,
    down_len: u8,
    down_source_action_index: Option<u32>,
}
```

Không nhất thiết phải thay toàn bộ schedule ngay trong PR đầu tiên.

Có thể triển khai theo hai bước:

#### Bước A

* Compiler group metadata theo timestamp.
* Runtime vẫn gửi Up và Down riêng.
* Telemetry ghi rõ:

  * `same_timestamp_release_before_down`.
  * `head_of_line_delay_us`.

#### Bước B

Sau khi benchmark và fault injection đầy đủ, cân nhắc mixed physical transaction.

### Mixed transaction chỉ được triển khai khi

1. Semantics partial insertion được định nghĩa.
2. Fault injection kiểm tra mọi prefix từ `0..total_events`.
3. Có benchmark chứng minh:

   * Giảm Down completion error.
   * Không tăng stuck-key risk.
   * Không làm recovery phức tạp đến mức không review được.

### Semantics partial cho mixed transaction

Giả sử packet có:

```text
U releases
D downs
```

Các trường hợp:

#### Inserted = U + D

Success hoàn toàn.

#### Inserted < U

* Một phần release có thể đã được insert.
* Không được gửi Down chord.
* Cập nhật release state theo prefix quan sát.
* Trong strict mode: terminal error.
* Trong normal fidelity mode: ưu tiên terminal error thay vì tiếp tục timeline mơ hồ.

#### Inserted = U

* Release hoàn tất.
* Zero Down inserted.
* Có thể retry toàn bộ Down chord đúng một lần nếu:

  * Return semantics đã được test.
  * Deadline chưa vượt retry threshold.
* Nếu không đủ điều kiện: drop/abort theo strict fidelity contract.

#### U < inserted < U + D

* Một phần Down chord đã insert.
* Chord integrity mất vĩnh viễn.
* Rollback Down prefix.
* Full cleanup.
* Terminal error.

### Tiêu chí hoàn thành

Có tài liệu và test mô tả rõ mọi trường hợp cùng timestamp.

Không để behavior phụ thuộc tình cờ vào thứ tự loop hiện tại.

---

# 5. P0 — Chord integrity và key lifecycle

## Phase P0.3 — Hợp nhất chord integrity thành invariant fatal

### Mục tiêu

Không cho phép bất kỳ production policy nào tiếp tục playback sau partial Down insertion.

### File chính

```text
rust/crates/sky_dispatch_win32/src/input.rs
rust/crates/sky_player_rs/src/engine.rs
rust/crates/sky_player_rs/src/lib.rs
src/sky_music/orchestration/native_dispatch.py
tests/
```

### Quy tắc mới

```text
partial Down insertion
    → chord_integrity_lost = true
    → rollback inserted prefix
    → full-instrument cleanup
    → terminal error
```

Không phụ thuộc:

* `strict_timing`.
* `same_key_conflict_policy`.
* Telemetry enabled hay disabled.
* Số phím rollback thành công.

`strict_timing` chỉ nên điều khiển completion SLO, không điều khiển tính toàn vẹn chord.

### Refactor kết quả gửi

Tách rõ:

```rust
enum DownSendOutcome {
    Complete {
        completed_ticks: QpcTicks,
    },
    ZeroProgress {
        error: Option<u32>,
        completed_ticks: QpcTicks,
    },
    IntegrityLost {
        inserted_prefix: u8,
        rolled_back: u8,
        rollback_residue: u8,
        first_error: Option<u32>,
        last_error: Option<u32>,
    },
}
```

Tránh một struct quá nhiều boolean có thể tạo trạng thái không hợp lệ như:

```text
success = true
chord_integrity_lost = true
```

Dùng enum để Rust type system loại trạng thái mâu thuẫn.

### Test bắt buộc

Với chord size `n = 1..15`, inject return count:

```text
0
1
2
...
n
n + giá trị bất thường
```

Kiểm tra:

* Full success chỉ khi count được clamp thành `n`.
* Zero progress retry cả chord, không retry remainder.
* Partial progress không bao giờ gửi phần Down còn lại.
* Partial progress luôn terminal.
* Rollback residue được phản ánh đúng.
* Cleanup cuối phiên luôn chạy.
* Không còn key active trong mock success cleanup.

---

## Phase P0.4 — Canonical chord order

### Mục tiêu

Một chord giống nhau luôn tạo cùng thứ tự physical input.

### Quy tắc

Sau khi map scan code sang physical slot, chord phải được canonicalize theo slot:

```text
slot 0 → slot 14
```

Không phụ thuộc:

* Thứ tự note trong source JSON.
* Thứ tự parser.
* Thứ tự HashMap upstream.
* Thứ tự user input.

### Lợi ích

* Partial prefix luôn deterministic.
* Calibration có thể so sánh qua nhiều phiên.
* Input receipt spread có thể phân tích theo vị trí.
* Test đơn giản hơn.
* Cache estimator không học từ nhiều permutation của cùng chord.

### Test

* Hai action có cùng tập scan code nhưng thứ tự khác nhau tạo cùng compiled chord.
* Generation mapping vẫn đúng.
* Telemetry lưu canonical order.
* Python oracle dùng cùng canonical order.

---

# 6. P0 — Tách Rust worker khỏi UI stall

## Phase P0.5 — Thiết kế lại supervisor lease

### Vấn đề

Heartbeat hiện đi cùng vòng poll UI/control.

Nếu Python hoặc renderer stall quá timeout, Rust worker có thể abort dù scheduler và `SendInput` vẫn khỏe.

### Nguyên tắc

UI health không được đồng nghĩa với worker ownership health.

### Thiết kế đề xuất

#### Phương án mặc định

Tạo dedicated heartbeat thread phía supervisor:

```python
class NativeHeartbeatThread:
    interval_s = 0.1
```

Thread này chỉ làm:

```python
while not stop_event.wait(interval_s):
    session.heartbeat()
```

Không:

* Render.
* Poll control.
* Đọc telemetry lớn.
* Chạy JSON serialization.
* Làm filesystem I/O.

#### Lease timeout

Đổi default từ 500 ms sang một giá trị chịu được scheduler stall hợp lý, ví dụ:

```text
2–5 giây
```

Giá trị cuối cùng phải dựa trên benchmark.

Không dùng timeout quá ngắn chỉ để tạo cảm giác fail-fast.

#### Tùy chọn khác

Cho phép disable lease:

```text
supervisor_lease_timeout_us = 0
```

trong in-process production nếu xác định rằng lease không cung cấp guarantee cleanup khi process chết hoàn toàn.

Cần ghi rõ:

* Worker thread trong cùng process không thể cleanup sau hard process termination.
* Lease chỉ phát hiện supervisor/control stall, không phát hiện process đã biến mất.
* Nếu cần watchdog process độc lập.

### File chính

```text
src/sky_music/orchestration/native_dispatch.py
rust/crates/sky_player_rs/src/engine.rs
rust/crates/sky_player_rs/src/lib.rs
tests/
docs/rt-dispatch-architecture.md
```

### Test

1. Renderer stall 1 giây không làm playback abort nếu heartbeat thread khỏe.
2. Heartbeat thread bị dừng thì worker abort sau timeout.
3. Quit/join dừng heartbeat thread sạch.
4. Exception trong supervisor không để heartbeat thread rò rỉ.
5. Không gọi heartbeat sau session terminal.
6. Không giữ reference làm session không được drop.

### Tiêu chí hoàn thành

Một UI stall không ảnh hưởng timeline hoặc làm Rust worker kết thúc.

---

# 7. P1 — Định nghĩa đúng timing contract

## Phase P1.1 — Chuẩn hóa vocabulary và telemetry schema

### Mục tiêu

Không còn field mơ hồ về “lateness”.

### Đổi tên logic

Tối thiểu phải tách:

```text
dispatch_start_error
send_completion_error
delivery_proxy_error
game_observed_error
```

Nếu chưa đo được delivery hoặc game-observed:

```text
delivery_proxy_error = unavailable
game_observed_error = unavailable
```

Không dùng zero để biểu diễn unavailable.

### Telemetry record đề xuất

```rust
struct NativeTelemetryRecord {
    authored_us: u64,

    wait_target_us: u64,
    wake_us: u64,
    wake_error_us: i64,

    send_started_us: u64,
    send_completed_us: u64,
    send_duration_us: u64,

    sender_completion_error_us: i64,

    delivery_first_us: Option<u64>,
    delivery_last_us: Option<u64>,
    delivery_last_error_us: Option<i64>,
    intra_chord_delivery_spread_us: Option<u64>,

    applied_lead_us: u64,
    lead_components: LeadComponents,

    runtime_outcome: RuntimeOutcome,
}
```

Không nhất thiết lưu tất cả trong production full-rate telemetry.

Có thể lưu component aggregate trong summary mode.

### Schema migration

* Tăng telemetry schema version.
* Python adapter phải kiểm tra version.
* Các field cũ có thể giữ một release với deprecation.
* Update script phân tích telemetry.
* Update docs và test schema.

---

## Phase P1.2 — Xác định ba contract riêng

### Contract 1 — Sender completion

```text
SendInput call completion gần authored timestamp.
```

Đây là contract có thể đo trực tiếp trên mọi máy.

### Contract 2 — OS delivery proxy

```text
Phím cuối của chord được nhận bởi app-owned Raw Input target gần authored timestamp.
```

Đây là proxy cho input delivery, không phải game.

### Contract 3 — Game-observed frame

```text
Toàn bộ chord có mặt trước hoặc trong game frame mục tiêu.
```

Đây là contract cuối cùng nhưng có thể cần harness riêng hoặc quan sát gián tiếp.

### Quy tắc báo cáo

Mọi benchmark/report phải ghi:

```text
evidence_scope:
    sender_completion
    raw_input_delivery_proxy
    game_observed
    audio_observed
```

Không so sánh trực tiếp số liệu từ hai scope khác nhau như cùng một metric.

---

# 8. P1 — Chuyển hot timing path sang QPC ticks

## Phase P1.3 — Tick-domain refactor

### Mục tiêu

Giảm phép chia/chuyển đổi và tránh quantization không cần thiết.

### Boundary

Giữ microseconds tại:

* Python API.
* User config.
* JSON.
* Telemetry output.
* Domain schedule trước native preparation.

Sau khi native session được prepare, chuyển deadline sang QPC ticks một lần.

### Kiểu dữ liệu

```rust
#[repr(transparent)]
struct QpcTicks(u64);

#[repr(transparent)]
struct TimelineTicks(u64);

#[repr(transparent)]
struct DurationTicks(u64);
```

Không dùng `u64` trần cho nhiều loại thời gian trong cùng API.

### Lộ trình hai bước

#### Bước 1

* Precompute authored batch timestamp ticks.
* Precompute fixed `min_hold_ticks`.
* Precompute configured lead cap ticks.
* Wait strategy nhận tick deadline trực tiếp.
* Chuyển về microseconds chỉ khi publish telemetry.

#### Bước 2

* Chuyển coordinator pending release fields sang timeline ticks.
* Loại phần lớn `qpc_ticks_to_us` trong outer loop.
* Clock pause/recovery offset hoạt động trong ticks.
* Chỉ giữ public compatibility helper `_us` ngoài RT path.

### Invariant

```text
Một timestamp chỉ được convert us → ticks đúng một lần trong preparation.
```

Ngoại lệ:

* Dynamic duration từ config update không được phép trong phiên phát.
* Telemetry convert ticks → us khi xuất dữ liệu.

### QPC failure behavior

Không để RT helper trả `QpcTicks(0)` âm thầm.

Sau startup admission:

```rust
fn qpc_now_ticks_rt() -> Result<QpcTicks, QpcRuntimeError>
```

Hoặc dùng invariant nội bộ và terminalize session nếu API thất bại.

Clock failure phải:

* Full cleanup.
* Structured terminal error.
* Không tiếp tục với timestamp zero.

### Test

1. Roundtrip monotonic.
2. Deadline không trễ do hai sample khác nhau.
3. Pause offset chính xác trong ticks.
4. Recovery offset chỉ áp dụng một lần.
5. Long timeline không overflow.
6. Conversion rounding không làm release trước min-hold.
7. Differential simulation tick/us cho schedule ngắn.

### Benchmark

So sánh:

* Worker CPU time.
* Instructions nếu có profiler.
* p50/p95 bookkeeping.
* Wake target error.
* Sender completion error.

---

# 9. P1 — Chord-aware input delivery calibration

## Phase P1.4 — Viết calibration harness native Rust

### Mục tiêu

Đo delivery proxy theo chord size thay vì chỉ một key.

### Module đề xuất

```text
rust/crates/sky_dispatch_win32/src/calibration.rs
rust/crates/sky_player_rs/src/calibration.rs
```

Hoặc binary/tool riêng nếu không muốn đưa calibration vào production module.

### Polyphony bắt buộc

```text
1, 2, 3, 5, 8, 15
```

Có thể đo đủ 1–15 trong advanced mode.

### Mỗi sample phải ghi

```text
call_started_ticks
call_completed_ticks

first_receipt_ticks
last_receipt_ticks

receipt_count
expected_receipt_count

first_receipt_latency
last_receipt_latency
intra_chord_spread
```

### Correlation

Mỗi calibration packet phải có sequence ID nội bộ.

Không được ghép sample chỉ bằng scan code nếu các sample có thể overlap.

Phải phát hiện:

* Duplicate receipt.
* Missing receipt.
* Reordered receipt.
* Unexpected scan code.
* Unexpected Down/Up.
* Timeout.
* Focus loss.

### Dataset

Tách bucket:

```text
kind: Down | Up
polyphony: 1..15
class: Hot | Cold
```

Có thể thêm metadata:

```text
Windows build
CPU model
QPC frequency
power state
MMCSS mode
timer mode
sample count
sampled_at
```

### Sample count

Không dùng 200 mẫu để tuyên bố tail guarantee.

Mức đề xuất:

```text
quick calibration: 500 mẫu/bucket
full calibration: 5.000+ mẫu/bucket
soak/reference-host: 50.000+ mẫu quan trọng
```

Quick calibration phục vụ user setup.

Full/reference calibration phục vụ tuning estimator và release gate.

### Output schema

```json
{
  "version": 2,
  "evidence_kind": "injected_raw_input_delivery_proxy",
  "host_fingerprint": "...",
  "buckets": {
    "down": {
      "1": {
        "hot": {
          "call_duration": {},
          "first_receipt": {},
          "last_receipt": {},
          "spread": {}
        }
      }
    }
  }
}
```

### Không được làm

* Không gọi dữ liệu này là Sky latency.
* Không dùng receipt của key đầu làm đại diện cho cả chord.
* Không bỏ sample lỗi khỏi báo cáo mà không đếm.
* Không merge hot/cold trước khi estimator có policy rõ ràng.

---

# 10. P1 — Thiết kế lại adaptive lead

## Phase P1.5 — Lead predictor có component rõ ràng

### Mục tiêu

Không để một estimator duy nhất học lẫn tất cả loại latency.

### Cấu trúc đề xuất

```rust
struct LeadComponents {
    syscall_us: u64,
    delivery_proxy_us: u64,
    wake_reserve_us: u64,
    cold_reserve_us: u64,
    residual_bias_us: i64,
}

struct LeadEstimate {
    applied_us: u64,
    components: LeadComponents,
    saturated: bool,
    confidence: LeadConfidence,
}
```

### Công thức khởi điểm

```text
predicted_sender_completion =
    syscall_quantile(kind, polyphony, class)

predicted_delivery =
    calibrated_last_receipt_quantile(kind, polyphony, class)

scheduler_reserve =
    wake_error_reserve

lead =
    predicted_sender_completion
    + predicted_delivery
    + scheduler_reserve
    + cold_reserve
    + residual_bias
```

Sau đó clamp:

```text
0 <= lead <= max_lead
```

### Không dùng lead để chữa catastrophic tail

Phải phân biệt:

```text
systematic latency
random jitter
catastrophic preemption
```

* Systematic latency: bù bằng lead.
* Random timer jitter: giảm bằng wait/spin/priority.
* Catastrophic tail: telemetry, degrade hoặc strict abort.
* Không tăng lead toàn cục lên 10–15 ms chỉ vì một số outlier hiếm.

### Fast và slow model

Thay rolling window nhỏ duy nhất bằng hai tầng:

```text
fast model:
    phản ứng nhanh
    window ngắn hoặc histogram có decay

slow tail reserve:
    lịch sử dài hơn
    decay chậm
    chống quên tail hiếm quá nhanh
```

Lead sử dụng:

```text
max(calibrated_prior, fast_quantile, slow_tail_reserve)
```

### Data structure

Ưu tiên histogram bounded thay vì sort ring trên worker.

Ví dụ:

```rust
const BUCKET_WIDTH_US: u64 = 25;
const BUCKET_COUNT: usize = 256;
```

Bao phủ khoảng 0–6,4 ms, có overflow bucket.

Nếu latency thực tế lớn hơn, điều chỉnh dựa trên baseline.

### Complexity

* Update O(1).
* Query quantile O(number of buckets), bounded.
* Không allocation.
* Không sort.
* Không clone window.

### Residual

Residual phải riêng theo:

```text
Down/Up
Hot/Cold
```

Có clamp để không oscillate.

Không update residual từ:

* Retry.
* Partial insertion.
* Deferred release.
* Mixed-source release.
* Focus transition.
* Wait failure.
* Cleanup.
* Telemetry mode đặc biệt làm perturb timing.

### Cache migration

Tăng estimator state version.

Cần:

* Import version cũ.
* Migrate bảo thủ.
* Không dùng cache nếu host fingerprint không phù hợp.
* Có checksum/schema validation.
* Corrupt cache phải bị bỏ, không panic.

### Confidence state

```rust
enum LeadConfidence {
    PriorOnly,
    Warming,
    Learned,
    Saturated,
}
```

Telemetry phải ghi confidence.

---

# 11. P1 — Release timing và min-hold

## Phase P1.6 — Xác minh hold theo completion và delivery

### Invariant sender-side

```text
release_send_completion
    >= down_send_completion + min_hold
```

Không chỉ:

```text
release_call_start >= down_call_start + min_hold
```

### Delivery-aware margin

Calibration hiện dùng delivery asymmetry để tăng hold margin. Giữ ý tưởng này nhưng:

* Tách rõ margin sender-side và delivery-side.
* Không tái sử dụng onset lead như hold margin.
* Down và Up phải có profile riêng.
* Margin phải dựa trên tail phù hợp.

Công thức cần được kiểm chứng lại bằng chord-aware data.

Ví dụ khởi điểm:

```text
delivery_hold_shortening_risk =
    p99(down_last_receipt_after_completion)
    - p50(up_last_receipt_after_completion)
```

Nhưng coding agent phải kiểm tra dấu và semantics bằng dataset thực.

### Test

* Không release sớm dưới mọi lead combination.
* Lead Up lớn không vượt `release_not_before`.
* Cold Up và hot Down vẫn giữ min-hold.
* Recovery retry không làm hold ngắn hơn.
* Delivery margin không bị áp dụng hai lần.

---

# 12. P2 — Tối ưu hot path

Chỉ bắt đầu phase này sau khi P0 và timing contract P1 đã ổn định.

## Phase P2.1 — Worker-local metrics

### Vấn đề

Worker hiện publish nhiều atomic field sau mỗi dispatch.

### Thiết kế

Worker giữ:

```rust
struct WorkerMetricsLocal {
    // plain integer fields
}
```

Shared side giữ:

```rust
struct PublishedMetrics {
    snapshot: parking_lot::Mutex<EngineSnapshotData>,
    last_publish_ticks: AtomicU64,
}
```

### Publication

Publish khi:

* Đã qua 10–25 ms từ lần trước.
* Pause/resume.
* Error.
* Integrity loss.
* Terminal.
* Supervisor yêu cầu forced snapshot.

Worker dùng:

```rust
if let Some(mut guard) = snapshot.try_lock() {
    *guard = local.to_snapshot();
}
```

Không block RT worker nếu reader đang giữ lock.

Các terminal flags vẫn dùng atomics riêng.

### Mục tiêu

* Không atomic-store hàng chục field mỗi chord.
* Snapshot đủ mới cho UI.
* Error/terminal vẫn được publish ngay.
* Không tạo unsafe mới.

### Benchmark

* Bookkeeping p50/p95/p99.
* Cache contention.
* Snapshot freshness.
* Worker CPU.
* Supervisor snapshot latency.

---

## Phase P2.2 — Fixed-slot runtime state

### Mục tiêu

Loại `HashMap` và dynamic generation tracking khỏi hot path khi có thể.

### Dữ liệu vật lý

Instrument chỉ có 15 slots.

Dùng:

```rust
struct SlotState {
    generation_id: Option<GenerationId>,
    down_completed_ticks: TimelineTicks,
    release_state: ReleaseState,
}
```

```rust
struct RuntimeSlots {
    slots: [SlotState; MAX_KEYS],
    active_mask: u16,
    pending_mask: u16,
}
```

### Generation status

Nếu generation status chi tiết chỉ cần cho telemetry cuối phiên:

* Duy trì counter tổng theo state.
* Active generation ID nằm trong slot.
* Không cần `HashMap<GenerationId, Status>` cho mọi generation nếu generation đã terminal.

Có thể lưu terminal counts:

```rust
struct GenerationCounters {
    scheduled: u64,
    active: u64,
    release_pending: u64,
    released: u64,
    cancelled: u64,
    dropped: u64,
}
```

Nếu cần trace từng generation khi telemetry full:

* Bật optional trace structure ngoài production hot mode.
* Không bắt production mode trả chi phí cho diagnostic mode.

### Test

* Counters không âm.
* Tổng terminal + nonterminal bằng generation_count.
* Mỗi slot tối đa một generation.
* Release đúng generation.
* Cancel cleanup đúng counter.
* Differential với implementation cũ trên random valid schedule.

---

## Phase P2.3 — Borrowed schedule view

### Vấn đề

Mỗi dispatch materialize/copy intent từ flat arena sang runtime structs.

### Thiết kế

```rust
struct BatchView<'a> {
    batch: &'a CompiledBatch,
    intents: &'a [CompactIntent],
}
```

Coordinator làm việc trực tiếp với compact slice.

Chỉ tạo stack buffer scan code:

```rust
struct ScanCodeBatch {
    values: [u16; MAX_KEYS],
    len: u8,
}
```

Không cần `SmallVec` cho đường production thông thường.

### Điều kiện

* Không làm API lifetime quá phức tạp khiến correctness khó review.
* Nếu borrowed API gây coupling lớn, có thể giữ `SmallVec` nhưng loại clone và materialization không cần thiết trước.

### Benchmark

* CPU per dispatch.
* Allocation count.
* Bookkeeping.
* Binary size.
* Code complexity.

---

## Phase P2.4 — Precompute physical input templates

### Thiết kế

Tạo template cho 15 scan code:

```text
Down INPUT template
Up INPUT template
```

Physical transaction builder copy template vào fixed stack array.

### Không làm

* Không dùng heap.
* Không tạo `Vec<INPUT>` mỗi send.
* Không gọi conversion scan code → virtual key nếu `KEYEVENTF_SCANCODE` path không cần.
* Không thay đổi `dwExtraInfo` signature semantics.

### Benchmark

Đo riêng:

* Build INPUT array.
* `SendInput` syscall.
* Tổng backend call.
* Chord size 1–15.

Chỉ giữ patch nếu phần build input giảm đáng kể hoặc làm code rõ hơn.

---

## Phase P2.5 — Telemetry modes

### Mode đề xuất

```text
Off
Summary
Ring
FullTrace
```

#### Off

* Không materialize record.
* Không clone chord/generation list.
* Chỉ correctness counters tối thiểu.

#### Summary

* Histogram và aggregate.
* Không lưu từng event.

#### Ring

* Bounded N record gần nhất.
* Dùng cho debugging production.

#### FullTrace

* Chỉ dùng benchmark/diagnostic.
* Preallocate.
* Có capacity hard limit.
* Báo truncated.

### Python API

Thay boolean:

```text
telemetry_enabled
```

bằng mode rõ ràng nhưng giữ compatibility một release.

### RAM gate

Benchmark riêng cho:

* Off.
* Summary.
* Ring 4.096.
* FullTrace 200.000.

---

# 13. P2 — Wait strategy, spin và priority

## Phase P2.6 — Đo spin duty cycle

### Metric mới

```text
spin_time_us
playback_wall_time_us
spin_duty_cycle_ppm
worker_cpu_time_us
process_cpu_time_us
```

### Adaptive spin

Không chỉ dựa trên p95 của sample nhỏ.

Cần xem xét:

* Cold/hot.
* Power mode.
* MMCSS tier.
* Timer backend.
* Wake outlier.
* CPU load.

### Guardrail

```text
effective_spin_threshold_us <= configured hard cap
```

Hard cap không được tăng âm thầm do estimator state.

### Reprobe

Reprobe giữa bài chỉ được thực hiện khi:

* Còn đủ gap an toàn.
* Không có pending release gần deadline.
* Không có command/focus transition.
* Không làm worker bỏ lỡ authored deadline.
* Partial probe bị hủy sạch nếu điều kiện thay đổi.

### Benchmark

So sánh các threshold:

```text
300
500
700
800
1000
1500
3000 µs
```

Đo:

* Wake error.
* Completion error.
* Spin duty.
* CPU.
* Tail dưới load.

Không chọn threshold thấp nhất hoặc cao nhất theo cảm tính.

---

## Phase P2.7 — MMCSS policy benchmark

### Policies

```text
MMCSS Pro Audio
MMCSS Low Latency
MMCSS Audio
MMCSS Games
THREAD_PRIORITY_HIGHEST
Normal/off
```

### Test load

* Idle desktop.
* CPU stress.
* GPU load.
* Disk I/O.
* Battery/power saving.
* Background browser/video.
* Long playback.

### Kết luận cần đưa ra

Có bằng chứng cho:

```text
Auto fallback policy
```

Nếu `THREAD_PRIORITY_HIGHEST` không ổn định dưới load, không dùng nó làm fallback mặc định.

`TIME_CRITICAL` tiếp tục chỉ là expert explicit mode.

---

# 14. P2 — RAM và Python/native boundary

## Phase P2.8 — Giảm peak memory khi prepare session

### Vấn đề

Trong giai đoạn prepare có thể tồn tại đồng thời:

* Python action objects.
* Native tuple list.
* PyO3 parsed vector.
* Rust compiled schedule.
* Runtime giữ lại `_actions`.

### Thay đổi

1. Kiểm tra `_actions` có được dùng sau constructor không.
2. Nếu không dùng:

   * Không giữ `_actions`.
   * Hoặc set `None` sau native session prepare.
3. Chuyển Python actions bằng iterator/chunk nếu PyO3 API cho phép mà không làm contract phức tạp.
4. Reserve chính xác Rust vectors.
5. Không duplicate reason string không cần thiết.
6. Đo peak working set trong:

   * Parse.
   * Native conversion.
   * Compile.
   * Playback.
   * Teardown.

### Test

* Large synthetic schedule.
* 100k action.
* 1M action cap.
* Peak RSS.
* Memory sau `clear_session`.
* Không giữ object graph ngoài ý muốn.

---

# 15. P3 — Test architecture

## Phase P3.1 — Fault-injection backend

### API đề xuất

```rust
enum InjectedSendOutcome {
    Full {
        latency_ticks: u64,
    },
    Zero {
        latency_ticks: u64,
        win32_error: u32,
    },
    Prefix {
        inserted: u8,
        latency_ticks: u64,
        win32_error: u32,
    },
    Stall {
        duration_ticks: u64,
    },
}
```

Script có thể chỉ định outcome theo call index.

### Coverage

* Down.
* Up.
* Rollback.
* Cleanup.
* Full-instrument release.
* Recovery retry.
* Focus loss.
* Quit during wait.
* Quit during recovery.
* Supervisor lease expiration.

---

## Phase P3.2 — Property-based testing

Thêm `proptest` cho core.

### Invariant bắt buộc

1. Không key nào active sau terminal cleanup.
2. Mỗi generation có đúng một terminal state.
3. Không commit partial Down.
4. Release không trước min-hold.
5. Một late dispatch không tự động dịch deadline tiếp theo.
6. Pause overlap chỉ cộng một interval.
7. Recovery offset chỉ cộng một lần.
8. Compiler thành công thì không có same-key overlap.
9. Canonical chord order ổn định.
10. Tổng generation counters bằng generation_count.
11. Bounded queue/window không tăng capacity ngoài thiết kế.
12. Corrupt estimator cache không panic.
13. QPC conversion không làm deadline đi lùi.
14. Mọi partial prefix `0..n` tạo outcome đúng.

---

## Phase P3.3 — Differential Rust/Python oracle

### Input

Sinh random valid authored schedule với:

* Chord size 1–15.
* Same timestamp Up/Down hợp lệ.
* Repeated key sau release.
* Pause intervals.
* Focus transitions.
* Fixed send latency.
* Injected zero-progress.
* Release retry.

### So sánh

* Generation lifecycle.
* Sent scan code order.
* Release suppression.
* Min-hold.
* Final state.
* Timeline offset.
* Terminal outcome.

### Cho phép khác biệt

Chỉ cho phép khác biệt đã ghi trong migration manifest.

Ví dụ:

```text
Rust rejects overlap at compile time
Python oracle cũ rejects ở validation wrapper
```

Không được silently snapshot-update test để che behavior khác.

---

# 16. P3 — Benchmark và acceptance mới

## Phase P3.4 — Sender-side benchmark

Bắt buộc tách:

```text
Down / Up
polyphony
Hot / Cold
idle / load
```

Metrics:

```text
signed error
absolute error
late-only
early-only
p50
p95
p99
p99.9 nếu đủ mẫu
max
```

Ngoài ra:

```text
wake error
send duration
bookkeeping
spin duty
worker CPU
RSS
```

---

## Phase P3.5 — Drift benchmark

Với bài dài hoặc synthetic timeline dài:

```text
error_i = completion_i - authored_i
```

Tính:

* Linear regression slope.
* Median phút đầu.
* Median phút cuối.
* Difference đầu/cuối.
* Burst sau pause/recovery.
* Autocorrelation hoặc rolling median nếu cần.

### Gate

Không dùng chỉ `max_lateness`.

Một scheduler có thể không drift nhưng có một tail lớn, hoặc drift đều nhưng max chưa lớn ở bài ngắn.

---

## Phase P3.6 — Delivery-proxy benchmark

Dùng calibration window:

```text
last_receipt_error
first_receipt_error
intra_chord_spread
missing receipt
duplicate receipt
reorder
```

Theo chord size.

### Gate correctness

```text
missing = 0
duplicate = 0
mismatch = 0
```

Trong clean controlled run.

---

## Phase P3.7 — Soak test

### Kịch bản

* 30 phút.
* 2 giờ.
* Synthetic dense chords.
* Bài nhạc thật.
* CPU stress.
* Long idle gap.
* Focus loss riêng.
* UI stall riêng.
* Telemetry Off và Summary.

### Pass criteria

Clean run:

```text
keys_dropped = 0
chord_split_events = 0
failed_release_count = 0
rollback_residue_keys = 0
authored_conflict_events = 0
nonterminal_generations = 0
unexpected_terminal_error = none
```

Memory:

```text
RSS không tăng tuyến tính theo số event
```

Timing:

* Không cumulative drift.
* Không burst sau release recovery.
* p95/p99 không regress so với baseline đã phê duyệt.
* Rust phải tốt hơn Python trên ít nhất các sender-side metrics đã xác định trước.

---

# 17. CI và release gates

## Phase P3.8 — Chia gate theo mức bằng chứng

### Pull request CI

Chạy:

* fmt.
* check.
* clippy.
* unit.
* property tests bounded.
* Python tests.
* Mock fault-injection acceptance.
* Schema compatibility.
* Differential oracle.

### Scheduled Windows benchmark

Chạy trên fixed host hoặc self-hosted runner:

* Real `SendInput`.
* App-owned target.
* Chord-aware calibration.
* Sender benchmark.
* CPU/RSS.
* Soak ngắn.

### Release qualification

Chạy:

* Soak dài.
* Python/Rust A/B.
* Delivery-proxy benchmark.
* Target game validation nếu có harness.
* Power/load matrix.

### Không giữ CI ceiling quá rộng làm release proof

CI mock ceiling chỉ là sanity gate.

Release gate phải dùng fixed-host baseline có version và environment fingerprint.

---

# 18. Migration và backward compatibility

## Estimator cache

* Bump version.
* Import version cũ.
* Migrate bảo thủ.
* Nếu host/config không tương thích: bỏ cache.
* Không panic.
* Telemetry ghi cache loaded/migrated/dropped.

## Telemetry schema

* Bump version.
* Python adapter kiểm tra.
* Script cũ được cập nhật.
* Field cũ có deprecation window.
* Không đổi nghĩa field mà giữ nguyên tên.

## Config

Thêm hoặc chuẩn hóa:

```text
telemetry_mode
supervisor_lease_timeout_us
heartbeat_interval_us
lead_model_mode
delivery_calibration_mode
max_spin_threshold_us
```

Mọi config timing mới:

* Có default.
* Có validation.
* Có kill switch.
* Có telemetry ghi effective value.

## Python fallback

Giữ Python backend trong suốt rollout.

Không tự động fallback từ Rust sang Python giữa phiên phát.

Admission failure trước playback có thể:

* Fail closed.
* Hoặc chọn Python nếu user/config đã explicit cho phép.

Integrity loss giữa phiên không được chuyển backend rồi tiếp tục timeline.

---

# 19. Tài liệu phải cập nhật

Mỗi phase phải cập nhật tối thiểu các tài liệu liên quan:

```text
docs/timing-principles.md
docs/rt-dispatch-architecture.md
docs/perf-baselines/
docs/rust-dispatch-migration/
```

Tài liệu phải mô tả:

* Boundary được đo.
* Invariant.
* Failure behavior.
* Thread ownership.
* Cache schema.
* Telemetry schema.
* Benchmark scope.
* Kill switches.
* Known limitations.

Không để tài liệu gọi OS delivery proxy là game timing.

---

# 20. Thứ tự PR khuyến nghị

## Milestone A — Correctness foundation

### PR A1

Compile-time same-key overlap rejection.

### PR A2

Canonical chord order và compiler diagnostics.

### PR A3

Typed `DownSendOutcome` và fatal partial chord invariant.

### PR A4

Same-timestamp packet metadata và head-of-line telemetry.

### PR A5

Dedicated heartbeat và supervisor lease redesign.

Điều kiện kết thúc Milestone A:

```text
Không schedule bất khả thi.
Không partial chord continuation.
UI stall không giết worker.
```

---

## Milestone B — Timing truth

### PR B1

Telemetry vocabulary/schema mới.

### PR B2

QPC checked runtime behavior.

### PR B3

Tick-domain schedule preparation.

### PR B4

Tick-domain coordinator/pending release.

### PR B5

Rust chord calibration harness.

Điều kiện kết thúc Milestone B:

```text
Sender completion và delivery proxy được tách hoàn toàn.
Hot path dùng tick domain.
Có dữ liệu chord-aware.
```

---

## Milestone C — Lead model

### PR C1

Lead component types và calibrated prior.

### PR C2

Fast histogram model.

### PR C3

Slow tail reserve.

### PR C4

Estimator cache vNext và migration.

### PR C5

Lead saturation/confidence telemetry.

Điều kiện kết thúc Milestone C:

```text
Lead không còn là một con số black-box.
Không sort rolling window trong hot path.
```

---

## Milestone D — Runtime optimization

### PR D1

Worker-local metrics và throttled snapshot.

### PR D2

Fixed-slot generation lifecycle.

### PR D3

Borrowed compact batch view.

### PR D4

Fixed stack transaction buffers.

### PR D5

Telemetry modes.

### PR D6

Python/native peak-memory cleanup.

Điều kiện kết thúc Milestone D:

```text
Hot path không allocation trong production clean mode.
Không HashMap generation lookup trên đường thường.
Snapshot không gây atomic traffic mỗi chord.
```

---

## Milestone E — Production proof

### PR E1

Fault-injection framework.

### PR E2

Property tests.

### PR E3

Differential oracle.

### PR E4

Fixed-host sender benchmark.

### PR E5

Delivery-proxy benchmark.

### PR E6

Soak and release workflow.

---

# 21. Tiêu chí Definition of Done toàn dự án

Dự án chỉ được coi là hoàn tất kế hoạch khi đáp ứng toàn bộ:

## Correctness

* Compiler từ chối overlapping physical key generation.
* Không production mode nào tiếp tục sau partial Down insertion.
* Không còn authored runtime conflict trong clean valid schedule.
* Full cleanup chạy trên mọi terminal path.
* Không stuck key trong controlled success environment.
* Generation counters luôn nhất quán.

## Timing

* Không cumulative drift.
* Deadline mapping dùng cùng một clock sample.
* Worker hot timing path hoạt động chủ yếu trong QPC ticks.
* Sender completion và delivery proxy có metric riêng.
* Lead predictor có component và confidence rõ.
* Chord lead dựa trên polyphony.
* Last key receipt được dùng khi đánh giá chord readiness.

## Stability

* UI stall không làm playback dừng nếu owner heartbeat còn khỏe.
* Focus loss có outcome và continuity status rõ.
* Release recovery không gây catch-up burst.
* Timer/QPC failure terminalize có cleanup.

## Performance

* Production telemetry Off/Summary không allocation theo event.
* Worker CPU và spin duty được đo.
* RSS không tăng tuyến tính.
* Không regression p95/p99 trên fixed-host baseline.
* Rust chứng minh tốt hơn Python trên metric được định nghĩa trước.

## Engineering quality

* `cargo clippy -D warnings` pass.
* Không thêm `unsafe` ngoài Win32 boundary nếu không có lý do đặc biệt.
* Mọi `unsafe` có safety contract.
* Cache/schema có version.
* Test fault injection đủ prefix.
* Tài liệu đúng với implementation.

---

# 22. Các điều coding agent tuyệt đối không được làm

1. Không tăng `max_lead_us` để che tail mà không phân loại nguyên nhân.
2. Không tăng spin lên vài millisecond chỉ để benchmark sender đẹp hơn.
3. Không gọi `SendInput` completion là game onset.
4. Không tiếp tục bài sau partial chord.
5. Không đưa schedule overlap vào runtime rồi trông chờ conflict policy.
6. Không làm mixed Up/Down transaction trước khi có prefix-failure tests.
7. Không dùng một benchmark mock để tuyên bố production accuracy.
8. Không xóa Python oracle quá sớm.
9. Không thêm lock blocking vào RT worker.
10. Không tạo background allocation trong telemetry Off.
11. Không dùng process priority class.
12. Không dùng `TIME_CRITICAL` làm auto fallback.
13. Không bỏ overflow check chỉ để microbenchmark nhanh hơn.
14. Không dùng `panic=abort` nếu điều đó loại cleanup guarantee.
15. Không cập nhật baseline chỉ vì patch không pass.
16. Không thay đổi semantics và optimization trong cùng PR nếu có thể tách.
17. Không giữ boolean soup khi enum có thể loại trạng thái bất hợp lệ.
18. Không thêm cache state mà không có migration và validation.
19. Không để QPC failure biến thành timestamp zero.
20. Không tuyên bố hoàn thành trước khi có soak test.

---

# 23. Mẫu báo cáo bắt buộc cho mỗi PR

```text
## Problem

Lỗi hoặc hạn chế cụ thể là gì?

## Existing behavior

Code hiện tại làm gì?

## New invariant

Sau patch, điều gì luôn đúng?

## Design

Data structure và control flow mới.

## Failure semantics

Điều gì xảy ra khi API hoặc invariant thất bại?

## Files changed

Danh sách file.

## Tests

Test mới và test cũ liên quan.

## Benchmark

Môi trường, lệnh chạy, before/after.

## Compatibility

Schema/cache/config/API ảnh hưởng gì?

## Rollback

Có thể disable hoặc revert bằng cách nào?

## Known limitations

Boundary nào vẫn chưa đo được?
```

---

# 24. Lệnh validation tối thiểu sau mỗi phase

```powershell
cargo fmt --manifest-path rust/Cargo.toml --all -- --check

cargo check `
  --manifest-path rust/Cargo.toml `
  --workspace `
  --all-targets `
  --all-features

cargo clippy `
  --manifest-path rust/Cargo.toml `
  --workspace `
  --all-targets `
  --all-features `
  -- -D warnings

cargo test `
  --manifest-path rust/Cargo.toml `
  --workspace `
  --all-features

uv run pytest -m "not slow"

uv run --env-file .env python scripts/bench_native_acceptance.py `
  --actions 512 `
  --repeats 5 `
  --polyphony 1,2,3,5,8,15 `
  --output artifacts/current-native-acceptance.json
```

Các PR timing hoặc Win32 phải chạy thêm fixed-host benchmark phù hợp.

---

# 25. Kết quả kiến trúc mong muốn cuối cùng

Sau khi hoàn tất, Rust core phải có các đặc điểm:

```text
Immutable physically-valid schedule
Fixed-size physical key state
One native timing owner
QPC tick-domain deadlines
No Python in real-time path
No runtime allocation in clean production mode
No partial chord continuation
Bounded and typed recovery
Chord-aware calibrated lead
Separate sender/delivery/game timing evidence
Low-frequency coherent telemetry publication
Property-tested lifecycle
Fixed-host regression proof
Long-session soak proof
```

Mục tiêu cuối cùng không chỉ là Rust chạy nhanh hơn Python.

Mục tiêu là tạo một lõi mà:

* Những schedule không thể phát đúng sẽ bị từ chối trước khi bắt đầu.
* Những failure không thể phục hồi về mặt âm nhạc sẽ dừng có kiểm soát.
* Những loại latency khác nhau được đo và xử lý đúng bản chất.
* Timing không drift theo độ dài bài.
* Chord không bị tách âm thầm.
* UI không can thiệp vào worker timing.
* Hiệu năng có bằng chứng định lượng.
* Mọi kết luận về độ chính xác đều ghi đúng boundary quan sát.
