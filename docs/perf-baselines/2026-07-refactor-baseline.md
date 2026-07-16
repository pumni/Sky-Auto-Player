# Core Dispatch Refactor — Perf Baseline (Phase 0)

> **Date:** 2026-07-16  
> **Baseline commit:** `64e0873`  
> **Plan:** [2026-07_core-dispatch-refactor-and-isolation-plan.md](../2026-07_core-dispatch-refactor-and-isolation-plan.md) §3.1 step 3  
> **Purpose:** Compare-points for Phase 3 (CPU floors), not hard CI gates.

## Tool availability

| Tool | Status |
|---|---|
| `scripts/measure_dispatch_tail.py` | **Present** — synthetic latency backend + real wait path; requires wall-clock RT run, not FakeClock. Not automated in this phase. |
| Supervisor wake-rate instrumentation | No dedicated CLI; inferred from `playback_supervisor.py` fixed 10 ms sleep → **100 wake/s** design target. |

## Recorded numbers (Phase 0)

Manual measurement was **not** run against live Sky for this baseline (no game session required for the regression harness). Numbers below are **design-floor / static analysis** so Phase 3 has an explicit before-state to fill in.

| Metric | Value | Source |
|---|---|---|
| Supervisor wake rate | 100 wake/s | `playback_supervisor.py` ~10 ms fixed sleep (plan §6.3: intentionally not event-driven) |
| Adaptive spin threshold floor | 700 µs | `_recompute_spin_threshold_from_overshoot` clamp |
| Adaptive spin threshold cap | 3 000 µs | same |
| Reprobe apply policy (pre-Phase 3) | ratchet-up only (`new > current`) | `dispatch_loop.py` — finding A5 |
| Telemetry mid-song CSV flush (pre-Phase 2) | every 10 000 records on RT thread | `telemetry.py` `_TELEMETRY_FLUSH_CHUNK` |
| Per-send focus refresh (pre-Phase 2) | `focus_is_active()` after every send; OpenProcess on 2 ms TTL | finding A4 |

## How to fill live numbers later

```powershell
# Longest song in songs/ (example — pick the largest .json by duration):
uv run python scripts/measure_dispatch_tail.py

# Optional: compare after Phase 3 with the same command and append a before/after table
# to this file (Phase 6 step 3).
```

## Phase 6 after table (placeholder)

| Metric | Phase 0 (this file) | After Phase 3/6 |
|---|---|---|
| Dispatch-thread syscalls per send (focus) | ~0.5 OpenProcess/s-dense (2 ms TTL) | TBD |
| Spin threshold trajectory with reprobe | ratchet-up only | TBD (symmetric) |
| Telemetry-enabled tail latency | mid-song flush stalls possible | TBD (flush off hot path) |
