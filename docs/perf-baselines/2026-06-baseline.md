# Phase 0 Baseline Performance Metrics

Captured: 2026-06-11
Environment: Windows 11
Python: 3.14.3

## 1. Default Song: Renai Circulation (2202 notes, 2172 actions)
Profile: balanced @60fps, min_hold_us=17000, hold_us=17000

### Preparation Stage (best of N, ms)
- `parse_song_file`          : 2.472 ms
- `repo.load` (cached)       : 0.001 ms
- `repo.load` (cold/cleared) : 2.356 ms
- `build_key_actions`        : 10.613 ms
- `validate_key_actions`     : 1.251 ms
- `compile_runtime_intents`  : 6.721 ms

### Dispatch Path (pure coordinator CPU, no sleep modelling)
- Full timeline drain: 19.984 ms over 2172 batches
- Average CPU per batch: 9.201 us/batch

### Structural Fidelity (full engine, fake clock)
- Down events dispatched: 1085
- Down IOI median (whole): 250000 us (min=125000, max=5500000)
- Down timeline drift: 0 us
- Lateness (p50/p95/p99/max): 0 / 0 / 0 / 0 us
- Late >2ms / >5ms / >10ms: 0 / 0 / 0
- Runtime conflict drops: 0
- Confirmed hold shortfall: 0
- Sent down / up: 2202 / 2202

### Micro-benchmarks (best of N, us per call)
- `_record_input_path_send_duration` : 0.743 us
- `telemetry.record` (enabled)       : 2.208 us
- `send_scan_code_batch` (3 keys)    : 6.441 us

---

## 2. Song: blue.json (1030 notes, 990 actions)
Profile: balanced @60fps, min_hold_us=17000, hold_us=17000

### Preparation Stage (best of N, ms)
- `parse_song_file`          : 1.124 ms
- `repo.load` (cached)       : 0.001 ms
- `repo.load` (cold/cleared) : 1.128 ms
- `build_key_actions`        : 5.487 ms
- `validate_key_actions`     : 0.563 ms
- `compile_runtime_intents`  : 3.127 ms

### Dispatch Path (pure coordinator CPU, no sleep modelling)
- Full timeline drain: 9.387 ms over 990 batches
- Average CPU per batch: 9.482 us/batch

### Structural Fidelity (full engine, fake clock)
- Down events dispatched: 492
- Down IOI median (whole): 304000 us (min=76000, max=1824000)
- Down timeline drift: 0 us
- Lateness (p50/p95/p99/max): 0 / 0 / 0 / 0 us
- Late >2ms / >5ms / >10ms: 0 / 0 / 0
- Runtime conflict drops: 0
- Confirmed hold shortfall: 0
- Sent down / up: 1030 / 1030

### Micro-benchmarks (best of N, us per call)
- `_record_input_path_send_duration` : 0.750 us
- `telemetry.record` (enabled)       : 2.225 us
- `send_scan_code_batch` (3 keys)    : 6.654 us

---

## 3. Live Telemetry Baseline (balanced@60fps)
Song: `blue.json` (1030 notes, 990 actions)
Run ID: `20260611-010835-3110`
Timestamp: 2026-06-11 01:12:04

### Lateness Metrics
- **lateness_us** (delay of SendInput execution start vs scheduled):
  - min: 20.0 us
  - p50: 34.0 us
  - p95: 6470.0 us
  - p99: 11086.0 us
  - max: 13920.0 us
  - avg: 720.58 us
  - over 2ms / 5ms / 10ms: 50 / 31 / 10 events
- **visible_lateness_us** (delay of SendInput completion vs scheduled):
  - min: 245.0 us
  - p50: 448.0 us
  - p95: 7719.0 us
  - p99: 11846.0 us
  - max: 14503.0 us
  - avg: 1283.93 us
  - over 2ms / 5ms / 10ms: 55 / 39 / 17 events

### Send Duration & Spin Metrics
- **send_duration_us** (SendInput call execution duration):
  - min: 220.0 us
  - p50: 376.0 us
  - p95: 938.0 us
  - p99: 6160.0 us
  - max: 13539.0 us
  - avg: 587.29 us
- **pre_send_spin_us** (active spin time prior to dispatch):
  - min: 20.0 us
  - p50: 754.0 us
  - p95: 843.0 us
  - p99: 861.0 us
  - max: 956.0 us
  - avg: 578.14 us

### Invariants & Diagnostics
- **observed_hold_us** (actual press completion to release completion hold time):
  - min / p50 / p95 / p99 / max: 17238.0 / 17580.0 / 23828.0 / 27074.0 / 30568.0 us
- **observed_hold_below_frame_count**: 0 (no frame floor violations)
- **confirmed_hold_shortfall_count**: 0
- **down_timeline_drift_us**: -547 us (stable absolute timeline)
- **deferred_release_count**: 492
- **runtime_conflict_dropped_down_count**: 0
- **runtime_backend_dropped_down_count**: 0
- **expired_dropped_down_count**: 0


---

## 3. Post Phase 1-6 + acceptance round-1 fixes (2026-06-11)

Default song (Renai Circulation), same environment. Gate comparison vs §1:

### Micro-benchmarks (best of N, us per call)
- `record_input_path_send_duration` (DispatchHealthMonitor): 0.353 us  (baseline 0.743 -> **-52%**)
- `telemetry.record` (enabled, deferred formatting)        : 0.469 us  (baseline 2.208 -> **-79%**)
- `send_scan_code_batch` (3 keys, cached array)            : 4.992 us  (baseline 6.441 -> **-22%**;
  the mocked SendInput call dominates this bench, diluting the array-cache gain)

### Dispatch path (pure coordinator CPU)
- 9.436 us/batch (baseline 9.201; within noise — this path bypasses telemetry/SendInput, where
  the Phase-1 wins live)

### Structural fidelity
- drift 0, lateness 0/0/0/0, conflict drops 0, hold shortfall 0, sent 2202/2202 — unchanged.
