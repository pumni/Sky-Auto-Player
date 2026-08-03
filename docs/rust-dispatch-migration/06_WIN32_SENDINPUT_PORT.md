# 06 — Port Win32 SendInput, wait và real-time scope

> HISTORICAL: This migration note predates the final Rust-owned Win32 path.
> The current production contract is Rust-only; see `README.md`.

## 1. windows-sys features

Tối thiểu rà soát các feature:

```toml
windows-sys = { version = "=0.61.2", features = [
  "Win32_Foundation",
  "Win32_UI_Input_KeyboardAndMouse",
  "Win32_UI_WindowsAndMessaging",
  "Win32_System_Threading",
  "Win32_System_Performance",
  "Win32_Media_Multimedia",
  "Win32_System_Power",
] }
```

Tên feature/API phải xác nhận bằng `cargo check`; không thêm broad feature set tùy tiện.

## 2. INPUT packet

Mỗi keyboard input:

```text
type = INPUT_KEYBOARD
wVk = 0
wScan = scan_code
flags = KEYEVENTF_SCANCODE | optional KEYEVENTF_KEYUP
time = 0
dwExtraInfo = SKY_PLAYER_SIGNATURE (0x5C1B9111)
```

Rust wrapper nhận `&[u16]`, xây/lookup contiguous `[INPUT]`, gọi một SendInput.

Soundness checklist:

- slice sống suốt call;
- pointer đúng alignment;
- count <= slice length <= UINT max;
- `cbSize == size_of::<INPUT>()`;
- no mutation/concurrent move trong call;
- returned count clamp vào requested range trước slice indexing.

## 3. Clock

Dùng `QueryPerformanceCounter/Frequency` hoặc Rust monotonic source đã benchmark. Để parity và tránh semantic drift, QPC wrapper là khuyến nghị.

`completed_us` sample ngay sau `SendInput` return. `GetLastError` chỉ đọc sau timestamp, giống current priority: timestamp first, error second.

## 4. Send result

```rust
struct PlatformSendResult {
    requested: u8,
    inserted: u8,
    completed_us: u64,
    win32_error: u32,
}
```

Higher backend result:

```rust
struct InputSendResult {
    sent: SmallVec<[u16; 15]>,
    skipped_duplicates: SmallVec<[u16; 15]>,
    success: bool,
    error: Option<SendErrorKind>,
    send_completed_us: u64,
}
```

## 5. Exact retry algorithms

### Down

```text
first = SendInput(all)
if first == n: return full
remaining = all[first:]
second = SendInput(remaining)       # immediate
landed = first + second
record partial/chord split/retried/dropped
return prefix all[..landed]
```

Không sleep; không retry thứ ba; không dùng generic release retry helper.

### Up

```text
sent_total = SendInput(all)
zero_progress = 0
while sent_total < n:
    r = SendInput(remainder)
    if r > 0:
        sent_total += r
        zero_progress = 0
    else:
        zero_progress += 1
        if zero_progress >= 3: error
        sleep 2 ms
```

Completion timestamp là final attempted call.

## 6. Tracked backend transitions

Down single key common path:

```text
if active → skipped duplicate
possibly = true
emit
if sent → active = true
possibly = false
```

Nếu emit wrapper trả error, emergency key-up best effort; nếu cleanup fail, failed_release=true.

Up:

- active/possible false → skip;
- sent → clear active/possible/failed;
- unsent → failed_release true.

State arrays worker-owned nên không cần lock.

## 7. release_all

Union active/possible/failed. Current behavior cần parity:

1. tối đa 3 send passes, sleep 15 ms giữa failures;
2. verify physical state qua `GetAsyncKeyState` khi có VK mapping;
3. nếu stuck: retry 50 ms rồi 100 ms;
4. return `ReleaseAllOutcome` với attempted/stuck/inconclusive;
5. full instrument mode gửi toàn bộ 15 KEYUP sau tracked release.

Không tuyên bố `GetAsyncKeyState` là ground truth tuyệt đối; `verification_inconclusive` phải giữ.

## 8. Focus

Rust chỉ port cheap check:

```rust
GetForegroundWindow() == target_hwnd
```

Window discovery, title/process validation và refocus policy vẫn Python. Handle có thể stale; supervisor cập nhật/clear. Worker không tự enumerate window trên hot path.

## 9. Waitable timer/event

- Create high-resolution timer với `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` khi supported.
- Relative due time dùng 100 ns negative intervals; conversion overflow-safe.
- Auto-reset command event.
- `WaitForMultipleObjects([command, timer], FALSE, INFINITE)`.
- Nếu API fail, return typed degraded reason và fallback 2 ms sleep ladder.

## 10. Spin

```rust
while qpc_now_ticks() < target_ticks {
    std::hint::spin_loop(); // benchmark against empty loop
}
```

Current Python uses empty pass. Rust `spin_loop()` có thể có CPU pause semantics; benchmark tail latency trước khi chọn. Không mặc định rằng PAUSE instruction luôn tốt hơn. Freeze choice bằng benchmark report.

## 11. Priority/power

Port current ladder:

1. MMCSS task candidates: Pro Audio → Low Latency → Audio → Games;
2. nếu không có, thread priority highest;
3. disable thread power throttling/EcoQoS best effort;
4. no process priority class;
5. no hard affinity;
6. revert/restore on worker exit.

Mọi failure observable qua runtime options/diagnostics, không fatal trừ handle corruption.

## 12. Timer resolution

High-res waitable timer path không cần process-wide `timeBeginPeriod(1)` theo current design. Chỉ dùng timer-resolution guard cho fallback sleeper, và balance begin/end đúng scope.

## 13. Windows integration harness

Tạo test executable/window do dự án sở hữu để nhận Raw Input hoặc key state. Test được phép xác nhận:

- order/down-up;
- batch count/partial simulation qua fake API;
- sender completion timestamps;
- cleanup.

Không gọi đây là game onset. Game sampling/audio vẫn unobserved.
