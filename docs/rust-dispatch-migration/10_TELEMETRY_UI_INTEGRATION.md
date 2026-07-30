# 10 — Telemetry, progress và UI integration

## 1. Giữ UI contract, thay nguồn dữ liệu

Renderer hiện cần:

- elapsed/total;
- status;
- backend health;
- input path degraded;
- progress counters;
- finish message.

`RustDispatchRuntime` pull `session.snapshot()` theo cadence supervisor/UI và gọi cùng renderer methods. UI không import native DTO trực tiếp; adapter đổi thành existing Python dataclasses/protocol shape.

## 2. Snapshot semantics

Latest-wins, không queue từng frame. Snapshot fields:

```text
version
elapsed_us
total_us
status
force
input_path_degraded
backend_health
counters
finished_message | None
```

Worker publish khi:

- status transition;
- terminal;
- panic/focus/pause event;
- aggregate counters batch ready.

Supervisor có thể tự tính periodic playing elapsed từ Rust snapshot epoch, nhưng đơn giản và an toàn hơn là pull elapsed native 30 Hz.

## 3. Backend health

Rust owns:

```text
active_count
possibly_active_count
failed_release_count
last_error_code/context
min_same_key_up_gap_us
impossible_same_key_repeats
send_while_unfocused
keys_dropped
chord_split_events
```

Python adapter map sang `BackendHealth` hiện tại. Không expose internal Vec/sets.

## 4. Detailed telemetry schema

Giữ `_CSV_FIELDS` hiện tại:

```text
song,event_index,dispatch_id,kind,scheduled_us,actual_us,
dispatch_completed_us,evidence_scope,lateness_us,visible_lateness_us,
send_duration_us,send_duration_pure_us,bookkeeping_us,
dispatch_lateness_us,scan_codes,sent_scan_codes,skipped_scan_codes,
generation_ids,runtime_outcome,deferred_by_us,pre_send_spin_us,
idle_gap_us,reason,applied_lead_us
```

Rust raw record không cần `song` string mỗi row; Python writer inject song name khi materialize.

## 5. Outcome enum

Freeze at least:

```text
sent
deferred_release
partial_note_on
dropped_conflict
dropped_expired
blocked_unfocused
suppressed_stale_up
```

Terminal playback:

```text
finished
quit
skipped
shutdown_timeout
error
```

Không đổi spelling/case trước schema version 2.

## 6. Retain-first policy

Current logger giữ 200,000 record đầu. Rust phải tương thích:

```text
attempted += 1
if len < capacity: push; accepted += 1
else: dropped += 1; truncated = true
```

Không flush disk từ worker. `flush_if_large` Python cũ trở thành no-op/soft signal cho native path; save sau join.

## 7. Pure send vs bookkeeping

Trong Rust, bookkeeping có thể nhỏ hơn nhiều. Vẫn giữ fields:

- `send_duration_pure_us`: call entry → final SendInput return timestamp;
- `bookkeeping_us`: completion timestamp → end of native dispatch bookkeeping;
- `send_duration_us`: tổng;
- `visible_lateness_us`: completion - scheduled;
- `dispatch_lateness_us`: call-entry lateness + pure send duration.

Nếu timestamp precision khiến bookkeeping 0 thường xuyên, vẫn đúng; không xóa field.

## 8. Lead cache

Giữ JSON version 2 ban đầu để cache cũ dùng được. Adapter:

- đọc file Python;
- strict validate native importer;
- truyền native string/struct vào prepare;
- sau terminal lấy estimator export;
- chỉ write nếu worker terminated và engine không poisoned;
- atomic temp replace như hiện tại.

## 9. Summary

Phase đầu tiếp tục dùng Python `TelemetryLogger.get_summary()` bằng cách ingest native records sau join. Sau khi parity ổn, có thể viết native summary nhưng phải differential compare toàn schema.

Evidence fields bắt buộc:

```text
timing_semantics.onset_definition = sendinput_return
game_observed.available = false
game_acceptance_unknown = true
```

## 10. Debug diagnostics

Không gọi Python debug callback worker. Native diagnostics đi vào:

- counters;
- bounded `DiagnosticEvent` list;
- terminal error report.

Python format/log sau snapshot hoặc join. Chỉ critical OS debugging có thể dùng `OutputDebugStringW` best effort, nhưng mặc định tắt và không trong normal note path.
