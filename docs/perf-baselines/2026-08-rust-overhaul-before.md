# Performance Baseline: Before Rust Overhaul (Aug 2026)

This document captures the performance and efficiency baselines of the system prior to the complete Rust orchestration overhaul. The implementation plan that originally framed this baseline has been retired from the active documentation tree and remains available through Git history. These measurements are historical evidence, not current repository instructions.

## 1. Dispatch Efficiency (Python vs. Rust prototype)

Tested using `songs/Arthur Warrell - We Wish You A Merry Christmas.json` (180 notes, duration 36.5s) using `bench_rust_vs_python.py`.

| Metric | Python Engine | Rust Engine |
| --- | --- | --- |
| **process_cpu_us (p50)** | 1,218,750 | 0 |
| **idle_wake_count (p50)** | 306,170 | 252 |
| **send_duration_us (p50)** | 26 | 4 |
| **send_duration_us (p99)** | 67 | 17 |
| **visible_lateness_us (p99)** | 33,846 | 34,106 |

### Observation
The legacy Python busy-looping sleeper triggers over 300,000 idle wakes to play a 36-second song, consuming ~1.2 seconds of pure CPU time. The Rust engine, utilizing high-precision waitable timers (`WaitableTimer`), drops idle wakes down to just 252 (one per actual event batch), virtually eliminating CPU overhead (recorded as 0us process CPU). `send_duration_us` is also drastically lower due to the reduced FFI boundary crossings per key action in Rust.

## 2. Polyphony Latency Under Heavy Load

Tested using `test_measure_dispatch_tail.py` (simulating overlapping chords).

| Metric | Polyphony 1 | Polyphony 8 | Polyphony 15 |
| --- | --- | --- | --- |
| **completion_error_us.absolute.p50** | 335 | 376 | 296 |
| **completion_error_us.absolute.p95** | 884 | 971 | 1,036 |
| **completion_error_us.absolute.p99** | 1,288 | 1,234 | 2,145 |
| **completion_error_us.absolute.max** | 6,826 | 2,919 | 35,004 |
| **completion_error_us.signed.p50 (down)** | -459 | -416 | -293 |
| **completion_error_us.signed.p99 (down)** | 1,054 | 817 | 2,230 |
| **spin_cpu_time_us (p50)** | 521,784 | 687,292 | 458,093 |
| **peak_rss_bytes (p50)** | 68 MB | 81 MB | 96 MB |

### Observation
At extreme load (polyphony 15), the system demonstrates a significant degradation in tail latency (max error of 35ms), directly exposing the performance ceiling of iterating over many `SendInput` actions within the Python loop. `WaitableTimer` handles this more efficiently in the Rust engine, but this data is the Python/FFI boundary performance before the overhaul moves `SendInput` batching inside the Rust core entirely.

## Conclusion
The overhaul is justified by the massive reduction in idle wakes and process CPU usage. The legacy Python busy loop is highly inefficient. Bringing the orchestration and platform backend entirely into Rust is expected to stabilize the polyphony tail latency by avoiding the GIL and Python object allocation overhead entirely during the active dispatch window.
