# 11 — Risk register, rollback và anti-patterns

## 1. Risk: “port nhanh” làm thay đổi semantics

Dấu hiệu:

- release anchor dùng send start;
- pending release pop sau down;
- retry note-on dùng generic loop;
- focus chỉ kiểm tra ở supervisor;
- terminal counts giữ per-song Vec;
- command event index sau timer.

Mitigation: differential fake-clock tests trước real Windows.

## 2. Risk: PyO3 re-enables GIL

Nguyên nhân:

- module `gil_used=true`;
- non-thread-safe global/unsafe assumption;
- dependency extension không free-thread-safe.

Mitigation: import smoke kiểm tra runtime GIL flag trước/sau; explicit module annotation; audit `Send + Sync`.

## 3. Risk: use abi3 sai với 3.14t

Mitigation: no abi3 feature; validate wheel tag; build bằng exact `sys.executable`.

## 4. Risk: callbacks gây deadlock/jitter

Mitigation: no callback contract; Python pull snapshot/result only.

## 5. Risk: shutdown timeout/double-close

Mitigation:

- lifecycle atomics;
- one owner per handle;
- bounded join;
- POISONED state giữ resources alive;
- tests simulate blocked sender/waiter;
- engine không reuse poisoned session.

## 6. Risk: Rust panic bỏ key active

Mitigation: catch worker boundary, best-effort tracked + full-instrument release, external watchdog vẫn hoạt động.

## 7. Risk: new thread priority gây system instability

Mitigation: exact current MMCSS/fallback; no process class/affinity; telemetry acquisition outcome; kill switch.

## 8. Risk: float rounding estimator drift

Mitigation: Python-compatible round-to-even implementation/golden values; cache schema parity.

## 9. Risk: “zero allocation” nhưng unsafe pointer cache

Mitigation: frozen owned packet slices; no long-lived raw pointers; max 15 stack/SmallVec; benchmark before pointer tricks.

## 10. Risk: telemetry conversion OOM sau long song

Native capacity bounded 200k. Python conversion có thể vẫn lớn; support iterator/chunked take **sau terminal** nếu needed, nhưng giữ retain-first order. Không callback mid-play.

## 11. Risk: Python and Rust versions drift

Embed schema/build commit; prepare handshake fail fast. Adapter checks supported API version.

## 12. Rollback mechanism

Trong phases 3–7:

```text
SKY_DISPATCH_IMPL=python|rust
```

Hoặc internal config equivalent. Không expose như permanent user knob nếu không cần. Telemetry luôn record implementation.

Rollback criteria:

- any stuck-key regression;
- semantic diff unexplained;
- p99 timing regression > agreed bound;
- free-threaded GIL re-enabled;
- frozen build import failure;
- handle/thread leak;
- shutdown timeout increase.

Rollback là chọn Python implementation tại composition root, không revert nửa dependency tree.

## 13. Forbidden anti-patterns

- Tokio/async for microsecond dispatch.
- Python object stored in core/worker.
- `Arc<Mutex<RuntimeKernel>>` shared giữa Python và worker.
- unbounded channel/telemetry queue.
- per-note JSON/dict/string formatting.
- `unsafe impl Send/Sync` để ép compiler im lặng.
- `mem::forget` handles để “fix” shutdown.
- `unwrap/expect` trong worker trên external input/Win32 result.
- sleep/yield trong final spin.
- retry down có backoff.
- direct UI imports of native module across many files.
- xóa Python oracle trước differential soak.
