# Sky Auto Player — Rust Dispatch Migration Guide

**Mục tiêu:** chuyển toàn bộ lõi phát nhạc thời gian thực và gửi phím sang Rust, giữ nguyên UI/Textual và luồng ứng dụng Python hiện tại thông qua PyO3.

- Baseline mã nguồn đã phân tích: `main@45fa3c83b1642fb6c91caa2c13b3ce7d1a6cad52`
- Ngày rà soát: `2026-07-30`
- Rust toolchain mục tiêu: **Rust 1.97.1**, edition 2024
- PyO3 mục tiêu: **0.29.0**
- Maturin mục tiêu: **1.13.3**
- Windows bindings: **windows-sys 0.61.2**
- Python đích: **CPython 3.14 free-threaded (`3.14t`)**

> Quy tắc ưu tiên: hành vi hiện tại trong `src/` và test đang chạy là chuẩn. Tài liệu cũ chỉ là tham khảo. Không “dịch cú pháp” Python sang Rust; phải tái tạo đúng state machine, timing semantics và ownership contract.

## Cách dùng bộ tài liệu

AI coding phải đọc theo thứ tự:

1. `00_BASELINE_AND_SCOPE.md`
2. `01_CURRENT_DISPATCH_FLOW.md`
3. `02_BEHAVIORAL_INVARIANTS.md`
4. `03_TARGET_RUST_ARCHITECTURE.md`
5. `04_PYO3_CONTRACT.md`
6. `05_RUST_CRATE_AND_DATA_DESIGN.md`
7. `06_WIN32_SENDINPUT_PORT.md`
8. `07_MIGRATION_PHASES.md`
9. `08_TEST_AND_BENCHMARK_PLAN.md`
10. `09_BUILD_PACKAGING.md`
11. `10_TELEMETRY_UI_INTEGRATION.md`
12. `11_RISKS_ROLLBACK.md`
13. `12_AI_CODING_RUNBOOK.md`
14. `13_DEFINITION_OF_DONE.md`

`references/` chứa sơ đồ và bản đồ nguồn. `templates/` là scaffold định hướng, không được coi là code hoàn chỉnh cho đến khi `cargo check`, Clippy, test Rust và toàn bộ pytest đều qua.

## Kết luận kiến trúc

Biên thay thế cuối cùng là package `src/sky_music/orchestration/core/` **cộng** các phần timing/Win32 backend mà core gọi tới. Không nên chỉ port `SendInput`: làm vậy vẫn để Python nằm giữa deadline và syscall, giữ lại phần lớn jitter, allocation và lifecycle phức tạp.

Kiến trúc cuối:

```text
Textual/UI + Python application orchestration
        │ commands, focus samples, immutable schedule/config
        ▼
PyO3 adapter (`sky_player_rs.DispatchSession`)
        │ bounded command channel + atomics + snapshots
        ▼
Dedicated Rust dispatch worker
        ├─ runtime schedule/generations
        ├─ pause/focus state machine
        ├─ adaptive lead + wait strategy
        ├─ high-resolution waitable timer/event/MMCSS
        ├─ tracked SendInput backend
        └─ bounded telemetry
```

## Các quyết định không được đảo ngược tùy tiện

- Không Python callback trên worker thời gian thực.
- Không `abi3` cho wheel CPython 3.14t; build wheel riêng theo interpreter.
- Không Tokio/async runtime trong dispatch core.
- Không process priority class và không hard CPU affinity.
- Không hook, DLL injection, đọc/ghi memory tiến trình khác hoặc API nào ngoài input công khai/Win32 lifecycle cần thiết.
- Note-on partial: tối đa **một retry ngay lập tức, không sleep**, sau đó drop tail.
- Note-off/panic: ưu tiên giải phóng đầy đủ, được retry có giới hạn.
- Mốc hold là completion-to-completion, không phải call-entry.
- Rust worker là owner duy nhất của backend, timer handle, active key state và coordinator trong lúc chạy.

## Consolidated production status

- Rust is the only production dispatch implementation.
- The active binary has no Python sender, runtime fallback, backend selector, or
  `SKY_USE_PYTHON_DISPATCH` switch. Rollback means rolling back the application
  release.
- `DispatchSession` owns scheduling, timing, focus, SendInput, cleanup, and
  native telemetry. Python receives `snapshot_lite` during playback and one
  final `session_report` after termination.
- Sender telemetry ends at `SendInput` completion; `game_observed.available=false`
  remains intentional.
