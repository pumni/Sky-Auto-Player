# 02 — Behavioral invariants bắt buộc giữ nguyên

Mọi invariant dưới đây phải có ít nhất một Rust unit test và một differential/integration test tương ứng.

## A. Timeline và hold

**I-01 Completion anchor**

```text
release_not_before_us = down_send_completed_us + min_hold_us
```

Không dùng down call-entry, wall-clock trước syscall hay telemetry completion sau bookkeeping.

**I-02 Release floor wins over lead**

```text
due_up = max(scheduled_up_us - lead_up_us, release_not_before_us)
```

**I-03 Elapsed freezes khi pause**

Một contiguous pause interval chỉ được cộng một lần dù manual và focus overlap.

**I-04 Epoch rebase ordering**

Adaptive probe/priority/timer setup trước rebase; `run` ngay sau rebase.

**I-05 Sender-side metric honesty**

`visible_lateness_us` là SendInput-return minus schedule, không được đặt tên/hiển thị như game onset.

## B. Generation lifecycle

**I-06 Stable per-key generation**

Down/up cùng scan code được pair FIFO theo authored order.

**I-07 Runtime live state bounded**

Live status memory O(polyphony), terminal generations fold thành counters.

**I-08 No early conflict**

Lead không được pop down trước authored time nếu key còn active/pending.

**I-09 Activate only landed keys**

Unsent tail của partial chord chuyển `DROPPED_BACKEND`, không được invent active state.

**I-10 Stale up suppressed**

Up không match active generation không phát KEYUP vào một generation mới.

**I-11 Pending release priority**

Release due được drain trước authored down tại cùng wake.

**I-12 Authored order stable**

Scalar drain tăng cursor đơn điệu; không reorder batch cùng timestamp.

## C. SendInput

**I-13 Strict scan validation**

- integer thật, không chấp nhận bool;
- range u16;
- thuộc allowlist instrument/layout đã chuẩn bị;
- batch 1..15;
- không duplicate trên trusted path.

**I-14 Contiguous chord attempt**

Mỗi chord attempt là một `SendInput(n, ptr, sizeof(INPUT))`, không loop từng key ở healthy path.

**I-15 Note-on retry asymmetric**

Partial down: đúng một immediate remainder retry, zero sleep; sau đó drop.

**I-16 Note-off completion policy**

Up/release: cố hoàn tất remainder, tối đa ba zero-progress liên tiếp trước error; progress dương reset counter.

**I-17 Timestamp immediately after native return**

QPC sample phải là operation quan sát đầu tiên sau `SendInput` return, trước diagnostics/state mutation.

**I-18 Dedupe semantics**

Duplicate down và already-up là success idempotent với `skipped_duplicates`.

**I-19 Exception window tracked**

Trước down emit, key ở possibly-active; chỉ clear sau result/cleanup. Rust RAII phải không làm mất trạng thái khi FFI lỗi/panic.

**I-20 Panic release full instrument**

Tracked release trước, sau đó KEYUP toàn bộ 15 scan code instrument.

## D. Focus và commands

**I-21 No down while unfocused**

Khi require-focus, pre-down đọc atomic focus signal và fresh cached-HWND compare.

**I-22 Dual release on focus transition**

Release lúc focus loss và release idempotent sau restore grace.

**I-23 Command wins simultaneous deadline**

Trong `WaitForMultipleObjects`, command event nằm index 0.

**I-24 Backend thread affinity**

Mọi backend call và key-state mutation trong playback xảy ra trên cùng Rust worker.

**I-25 No Python call on worker**

Không log callback, telemetry callback, focus callback hay PyObject access từ RT worker.

## E. Wait/priority

**I-26 Final spin không sleep/yield**

Spin loop chỉ đọc QPC/performance counter và compare.

**I-27 High-resolution timer fallback**

Nếu high-res timer không có, degrade rõ ràng sang sleep ladder/timer resolution guard; không giả vờ success.

**I-28 Adaptive threshold clamp**

`max(spin_floor, min(3000, max_wake_error + 200))`.

**I-29 Cold guard/reprobe kill switches**

Tắt adaptive spin phải tắt reprobe; current knobs giữ behavior.

**I-30 MMCSS lifecycle**

Register/revert cùng worker; fallback ladder giữ nguyên; không process priority class/hard affinity.

## F. Telemetry và lifecycle

**I-31 Retain-first bounded telemetry**

Capacity hiện tại 200,000. Khi full, giữ record đầu, drop record mới và tăng exact counters. Không đổi sang drop-oldest trong parity phase.

**I-32 Stable field semantics**

Các field CSV hiện tại phải giữ tên/đơn vị/outcome trước khi có schema version mới.

**I-33 No disk I/O on worker**

Worker chỉ giữ native records/counters. Python serialize sau join.

**I-34 Authoritative join**

Không Drop/close timer/event/cache/state khi worker còn sống.

**I-35 Abort order**

Release first, cancel coordinator second, count abort third.

**I-36 Panic containment**

Rust panic trong worker phải được bắt ở thread boundary, chạy best-effort release, publish terminal error; không unwind qua PyO3/FFI.

## G. Assertions tổng hợp

Sau mọi terminal path:

```text
active_keys == empty OR release outcome explicitly reports stuck keys
pending_releases == empty OR worker is shutdown-timeout/poisoned
sum(generation_status_counts) == generation_count
accepted_telemetry + dropped_telemetry == attempted_telemetry
all Win32 handles closed iff worker_terminated == true
```
