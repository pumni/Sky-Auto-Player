# 12 — Runbook giao cho AI coding

## 1. System prompt chung

```text
Bạn đang migration dispatch core của pumni/Sky-Auto-Player sang Rust/PyO3.
Baseline được thiết kế theo main@45fa3c83b1642fb6c91caa2c13b3ce7d1a6cad52.

Đọc toàn bộ docs/rust-dispatch-migration theo thứ tự. Current src/tests thắng mọi proposal cũ.
Giữ nguyên mọi invariant trong 02_BEHAVIORAL_INVARIANTS.md.
Không gọi Python từ Rust dispatch worker. Không dùng abi3 cho CPython 3.14t.
Không dùng Tokio. Không hooks/injection/process memory; chỉ public SendInput và Win32 lifecycle APIs hiện có.
Mỗi patch phải nhỏ, có tests, cargo fmt/check/clippy/test và pytest.
Không sửa test expectation để che semantic difference.
Khi gặp conflict giữa docs và source, dừng phần code bị ảnh hưởng, trích symbol/current behavior và cập nhật plan trong PR.
```

## 2. Quy trình bắt buộc cho mỗi task

1. `git rev-parse HEAD` và inventory files/symbols liên quan.
2. Viết bảng “current behavior → target behavior → tests”.
3. Implement pure/native core trước binding.
4. Viết unit/property/differential tests.
5. Chạy Rust gates.
6. Build native wheel bằng 3.14t.
7. Chạy pytest targeted rồi full.
8. Review unsafe, locks, allocations, lifecycle.
9. Ghi PR summary: invariants touched, measurements, rollback.

## 3. Prompt Phase 0

```text
Thực hiện Phase 0 trong 07_MIGRATION_PHASES.md. Chỉ thay docs/tests contract, không thêm production Rust behavior. Xác định mọi tài liệu mâu thuẫn với migration, freeze telemetry schema/outcomes/defaults, bổ sung invariant tests còn thiếu. Kết quả phải là Python suite xanh và một source map cập nhật theo HEAD.
```

## 4. Prompt Phase 1

```text
Tạo Rust workspace 3 crates theo 05. Dùng Rust 1.97.1, edition 2024, PyO3 0.29.0, windows-sys 0.61.2. Extension module sky_player_rs phải import trên CPython 3.14t và khai báo gil_used=false. Không abi3. Chỉ expose build_info và lifecycle smoke; chưa port dispatch.
```

## 5. Prompt Phase 2

```text
Port generation compiler, RuntimeDispatchCoordinator, PlaybackClockState và SendLatencyEstimator vào sky_dispatch_core. Không Windows/PyO3 trong core. Tạo fake-clock differential harness đối chiếu baseline Python. Preserve O(polyphony) live status, scalar authored drain, release-first priority, completion anchor và Python-compatible rounding.
```

## 6. Prompt Phase 3

```text
Port tracked SendInput backend vào sky_dispatch_win32 và temporary PyO3 RustInputBackend adapter. Implement exact asymmetric retry algorithms, timestamp immediately after native return, possibly-active/failed-release tracking, release_all verification và full-instrument panic release. Dùng fake native API cho unit tests; real SendInput tests chỉ trên Windows marker.
```

## 7. Prompt Phase 4

```text
Implement Rust worker lifecycle, QPC clock, waitable timer, command event, MMCSS/power guard, adaptive spin/cold guard/reprobe. Worker dry-run only. Use bounded command channel, atomic focus/HWND, latest-wins progress snapshot. No Python callback/PyObject on worker. Test command event priority, pause overlap, focus wake, cooperative shutdown và simulated join timeout.
```

## 8. Prompt Phase 5

```text
Compose Rust core + Win32 sender thành full DispatchSession behind feature flag. Python adapter pushes actions/config/commands/focus and pulls snapshots/results. Preserve all telemetry fields/outcomes and lifecycle ownership. Production default vẫn Python. Run differential corpus and Windows E2E.
```

## 9. Prompt Phase 6–8

```text
Hoàn thiện UI/telemetry/build/PyInstaller parity, then Rust default with explicit Python fallback, then remove Python production hot path only after all gates in 13 pass. Do not combine these phases in one PR.
```

## 10. Review checklist cho AI reviewer

```text
[ ] Any Python callback/PyObject reachable from worker?
[ ] Any lock held across SendInput/wait/join/Python conversion?
[ ] Command event index 0?
[ ] Down partial exactly one immediate retry/no sleep?
[ ] Up zero-progress bound/reset exact?
[ ] Completion timestamp first operation after native return?
[ ] Release floor completion-anchored?
[ ] Pending releases drained first?
[ ] Activate only landed prefix?
[ ] Focus fresh HWND check immediately pre-down?
[ ] Worker sole owner of backend/timer/coordinator?
[ ] Join timeout leaves resources intact/poisoned?
[ ] Telemetry bounded retain-first, no disk I/O?
[ ] Free-thread module does not enable GIL?
[ ] No abi3 feature?
[ ] Unsafe blocks documented and isolated?
[ ] Full Python/Rust/Windows/build gates passed?
```

## 11. Output format yêu cầu cho agent

Mỗi agent response/PR description phải có:

- Scope;
- Current behavior evidence (files/symbols);
- Design;
- Invariants preserved;
- Intentional differences;
- Tests run/results;
- Performance evidence;
- Unsafe/concurrency audit;
- Rollback;
- Remaining work.
