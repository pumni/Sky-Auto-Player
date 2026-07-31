# 2026-08 Rust Real-Time Dispatch Core Overhaul - PRE-0 Baseline

## Môi trường (Environment)

* Git commit hiện tại: 27379119dadaca843f8f1fbb03fb7edf3c788498
* Rust toolchain: rustc 1.97.1 (8bab26f4f 2026-07-14)
* Python version và ABI: 3.14.3 free-threading build (main, Feb 12 2026, 00:41:00) [MSC v.1944 64 bit (AMD64)] GIL_enabled=False
* Windows version: Microsoft Windows 11 Home
* CPU: AMD Ryzen 5 5500U with Radeon Graphics
* Power plan: Power Scheme GUID: 381b4222-f694-41f0-9685-ff5bb260df2e  (Balanced)

## Kết quả Gates

* `cargo fmt`, `check`, `clippy`, `test`: Pass (0 warnings, test success)
* `pytest -m "not slow"`: Pass (991 passed, 1 skipped)

## Dữ liệu Benchmark

Các json telemetry gốc được lưu tại:
* `artifacts/native-before-overhaul.json` (Real SendInput performance prior to overhaul)
* `artifacts/python-before-overhaul.json` (Python vs Rust comparative stats for a full song run)

Baseline này đại diện cho hành vi của scheduler cũ (vẫn có thể ghép nhiều generation trên cùng scan code, có conflict policy động, v.v.).

*Không bắt đầu thay đổi semantics trước khi baseline này được review/commit.*
