# 07 — Kế hoạch triển khai theo PR/phase

> HISTORICAL: This migration plan has been superseded by the completed
> consolidation. The current production contract is Rust-only; see `README.md`.

Mỗi phase phải merge được độc lập, có rollback rõ, và không thay đổi default production trước phase 6.

## Phase 0 — Freeze contract và loại bỏ doc conflict

**Mục tiêu:** tạo oracle ổn định.

Tasks:

- copy bộ guide này vào repo, ví dụ `docs/rust-dispatch-migration/`;
- cập nhật `docs/INDEX.md`;
- sửa `PORTING_GUIDE.md` mục cấm dispatch migration;
- đánh dấu `docs/rust-migration-plan.md` superseded hoặc thêm resolution header;
- thêm `RUST_DISPATCH_SCHEMA_VERSION = 1`;
- snapshot telemetry field list/outcome enums/config defaults;
- thêm tests cho invariants còn thiếu trước khi port.

Gate: Python suite xanh, không behavior change.

## Phase 1 — Workspace và PyO3 smoke trên 3.14t

- tạo workspace 3 crates;
- build `sky_player_rs` bằng exact CPython 3.14t;
- `#[pymodule(gil_used=false)]`;
- expose `build_info()` trả versions/ABI flags;
- CI `cargo fmt`, check, clippy, test;
- smoke import dưới free-threaded runtime và assert module không re-enable GIL;
- build wheel version-specific, no abi3.

Gate: import từ app/test environment, PyInstaller selftest load extension.

## Phase 2 — Pure Rust model/coordinator differential simulator

- port types, generation compile, coordinator, pause clock, estimator;
- fake clock/fake sender; chưa gọi Windows;
- export test-only function chạy schedule và trả trace;
- Python differential harness feed cùng actions/scripted results vào Python core và Rust core;
- compare ordered outcomes, generation counts, release floor, estimator.

Gate: golden corpus và property tests parity.

## Phase 3 — Rust Win32 sender behind Python loop

- port INPUT packet/SendInput/tracked backend/release_all;
- expose temporary `RustInputBackend` Python class implement surface hiện tại;
- Python DispatchLoop vẫn chạy, chọn backend bằng internal flag;
- differential fake SendInput scripts;
- Windows integration test.

Mục đích: isolate Win32 correctness. Không tuyên bố performance migration hoàn tất.

Gate: all backend tests + stuck-key safety + current pytest.

## Phase 4 — Rust wait/priority worker, dry-run first

- port QPC, timer/event, wait strategy, adaptive probe, cold guard, reprobe;
- worker chạy with FakeSender/DryRun;
- PyO3 session/channel/snapshot/lifecycle;
- supervisor adapter push commands/focus;
- no Python callback worker;
- test simultaneous command/deadline, pause overlap, join timeout simulation.

Gate: deterministic worker traces parity.

## Phase 5 — Full Rust dispatch worker with real sender

- compose coordinator + wait + sender;
- worker owns all hot state;
- Rust detail telemetry buffer;
- Python engine adds `RustDispatchRuntime` behind flag;
- maintain Python fallback;
- mirror release/panic/focus semantics;
- full-instrument panic test.

Gate: Windows E2E songs, no backend call outside worker, no stuck keys.

## Phase 6 — UI/telemetry/build parity

- snapshot → renderer mapping;
- convert/take native telemetry after join;
- summary schema comparison;
- lead cache import/export compatibility;
- PyInstaller wheel/`.pyd` inclusion;
- doctor/selftest show native module status/toolchain/ABI;
- performance and memory baselines.

Gate: frozen app startup/playback/selftests.

## Phase 7 — Rust default, Python fallback

- default Rust on supported Windows 3.14t;
- fallback automatically when the native extension is missing/incompatible, with
  `SKY_USE_PYTHON_DISPATCH=1` as the explicit diagnostic rollback switch;
  Rust is fail-closed by default, while `SKY_REQUIRE_RUST_DISPATCH=1` remains
  accepted for backwards-compatible deployment configuration;
- production telemetry records backend implementation/version;
- soak testing across song corpus/FPS/profiles/focus/pause/panic;
- publish migration notes.

Gate: two release cycles or agreed soak window without Rust-specific blocker.

## Phase 8 — Remove Python production hot core

- remove Python DispatchLoop/backend from normal path;
- retain fake/oracle code under tests or archived reference only as needed;
- remove ctypes INPUT caches and Python wait strategy production wiring;
- simplify free-threaded dependency assumptions only after evidence;
- update AGENTS/architecture.

Gate: no production import path reaches old core; all docs current.

## Patch sizing rule

Một PR không đồng thời:

- thay DTO schema;
- đổi timing policy;
- đổi retry policy;
- đổi telemetry schema;
- đổi packaging.

Mỗi PR phải nêu “behavior intentionally unchanged” hoặc list exact intentional differences.
