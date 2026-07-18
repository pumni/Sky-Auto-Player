# Performance Baselines: Core Send Overhaul (2026-07)

These metrics capture the static defaults and limits governing the high-precision sleep-spin loop and the adaptive dispatch estimator.

| Constant / metric | Value | Source |
|---|---|---|
| `spin_floor_us` default | 700 µs | `engine.py` |
| Spin cap | 3000 µs | probe clamp in `_probe_timer_wake_error` |
| Residual max | 500 µs | `SendLatencyEstimator._MAX_RESIDUAL_US` |
| Lead max | 2000 µs | `SendLatencyEstimator` (init max_lead_us) |
| Seed samples | 5 | `SendLatencyEstimator._SEED_SAMPLES` |
| Default `min_hold_margin_us` | 500 µs | `scheduler_types.py` |
| Polled sleep cap | 2000 µs | `wait_strategy.py` |
| Poll interval | 2000 µs | `loop.py` |

Live Sky performance numbers are TBD (measure using `scripts/measure_dispatch_tail.py` when live audio loopback calibration is enabled).
