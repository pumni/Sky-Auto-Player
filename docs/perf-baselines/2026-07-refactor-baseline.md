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

## Phase 6 after table

Structural after-state (post Phases 1–4). The "after" column is by construction — these are
enforced by code + tests, not live-Sky measurements (this harness never needed a game session):

| Metric | Phase 0 (before) | After Phase 1–4 |
|---|---|---|
| Dispatch-thread focus syscalls per send | `focus_is_active()` → OpenProcess on 2 ms TTL (finding A4) | **0** — runtime `FocusSignal` under threaded dispatch; cheap HWND-only compare (`is_foreground_cached_hwnd`, injected) as direct-mode fallback; full process-name check only at supervisor 20–50 ms cadence |
| Spin threshold reprobe apply policy | ratchet-up only (`new > current`, finding A5) | **symmetric** (`_maybe_apply_reprobe_threshold`, both directions, clamp `[700, 3000]`); applied thresholds appended to `runtime_options["reprobe_applied_thresholds"]` |
| Overshoot samples | contaminated by already-late branch | only true timer-wake overshoot sampled (already-late branch no longer records) |
| Telemetry mid-song CSV flush | every 10 000 records synchronously on RT thread (finding A3) | off the hot path — `flush_if_large` from paused/wait branches + `record_pause`; hard cap `_TELEMETRY_MAX_BUFFER=200_000` as pathological fallback |
| Hot-path platform import in `_execute_action` | `from sky_music.platform.win32 import inputs` per unfocused send | removed — `unfocused_send_hook` injected at loop construction; the whole loop is now platform-import-free (`orchestration/core/` boundary, `tests/test_core_boundary.py`) |

### Dispatch-tail latency (synthetic backend, `scripts/measure_dispatch_tail.py`, 15 s truncated)

Measured post-Phase-4 (2026-07-17) with the synthetic p50≈477 µs / max≈1695 µs send-duration model.
All values microseconds; `lateness` is completion − scheduled (negative = early, expected under
adaptive lead), `visible`/`dispatch` are the on-time metrics.

| Metric | Load Off / default | Load On / 1 ms | Load On / 5 ms |
|---|---|---|---|
| p50 lateness | −557 | −538 | −585 |
| p95 lateness | 56 | 56 | 52 |
| p99 lateness | 103 | 106 | 357 |
| max lateness | 127 | 118 | 415 |
| p99 visible/dispatch | 766 | 821 | 1085 |
| max visible/dispatch | 1050 | 1023 | 1113 |

> The tool's `SyntheticLatencyBackend._emit` was updated in Phase 6 to the current `_emit`
> tuple contract `(actually_sent, send_completed_us)` — it had drifted since the Phase 1 A6a
> clock-injection change and could not run before. Numbers are synthetic (no live Sky), useful
> as a relative CPU/tail floor, not an absolute latency claim.
