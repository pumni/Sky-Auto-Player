# RT Dispatch Architecture (post Phase-6 decomposition)

Status: CURRENT — graduated to production defaults 2026-06-11.
History: built by `archive/2026-06_rt-pipeline-extreme-optimization-plan.md`; A/B numbers in
`perf-baselines/2026-06-baseline.md`.

## 1. The single ground truth

The game registers a key press iff the key is observed held for **at least 1 game frame**
(completion-to-completion). Every mechanism below is subordinate to that invariant; see
`timing-principles.md` §0/§3.

## 2. Component map

```
PlaybackEngine (orchestration/engine.py)            facade: wiring + lifecycle only
 ├─ compile_runtime_intents → RuntimeSchedule        per-key generations (runtime_dispatch.py)
 ├─ SendLatencyEstimator                             per-kind EMA of SendInput durations
 ├─ wake-error probe (pre start_perf)                derives effective spin threshold
 ├─ RealtimeProcessScope (infrastructure/realtime)   gc.collect→gc.disable + setswitchinterval(1ms)
 ├─ DispatchLoop (orchestration/dispatch_loop.py)    wait → drain → execute; RT thread only
 │   ├─ RuntimeDispatchCoordinator                   active/pending state machine, floors, guards
 │   ├─ HybridWaitStrategy (infrastructure/wait_strategy.py)
 │   ├─ DispatchHealthMonitor                        focus cache, backend-health cache, input-path p95
 │   └─ InputBackend → send_scan_code_batch_trusted  cached INPUT arrays → user32.SendInput
 └─ PlaybackSupervisor (orchestration/playback_supervisor.py)  control thread:
     command queue + command event, focus polling, progress consumption/publishing,
     DispatchThreadPriorityScope (infrastructure/rt_priority.py) on the dispatch thread
```

Threading contract: the dispatch thread owns the backend and all timing; the supervisor (control)
thread owns controls/focus/rendering and must never call into the backend
(`test_threaded_dispatch_keeps_all_backend_calls_on_dispatch_thread`).

## 3. Timing semantics

- **Onset = dispatch completion.** The adaptive lead (per-kind EMA of `send_duration_us`, seeded
  by the average of the first 5 samples, clamped to 2 ms) pops work early so that SendInput
  *completion* lands on `scheduled_us`. `lateness_us` may legitimately be negative;
  `visible_lateness_us` is the on-time metric. Live A/B: down-onset median +420 µs → −3 µs.
- **Lead is symmetric** (downs and pending releases) and floor-clamped: a release becomes due at
  `max(scheduled_release − lead_up, release_not_before)` where
  `release_not_before = down_dispatch_completed + min_hold` — the 1-frame floor always wins.
- **No-early-conflict guard**: a down batch is never popped before its authored time while any of
  its scan codes is active or pending release (an early pop would become a dropped note).
  `next_authored_us` is guard-aware so a blocked batch reports its authored time as the deadline
  (no busy-loop while waiting for the blocking release).
- **Wake-error probe** (`enable_adaptive_spin`): ~10×2 ms probe sleeps run strictly *before*
  `start_perf` (same rule as `gc.collect`), deriving
  `effective_spin_threshold = clamp(max_error + 200, 300, 3000)` µs.

## 4. Wait strategy

`HybridWaitStrategy.wait_until_us` picks, in order:
1. `remaining ≤ spin_threshold` → busy-spin to target.
2. Sleeper declares `is_high_resolution` (capability flag, e.g. `WaitableTimerSleeper`):
   - event mode (command event handle present): arm the waitable timer for
     `remaining − guard` and block in `WaitForMultipleObjects(timer, command_event)` — zero
     polling; commands/focus transitions wake the thread instantly; then spin the guard.
   - polled mode: 1 ms-capped sleeps towards `target − guard` so the loop can poll between steps.
3. Fallback ladder (RealSleeper): coarse (≤20 ms, −5 ms buffer) → 1 ms ticks → yield → spin.

Polling is governed by the *presence* of the command event, not a flag: the supervisor creates the
event before the dispatch thread starts (so no early command can lose its wake-up) and signals it
on commands and focus transitions. In event mode the supervisor also publishes the periodic
"playing" progress (the loop sleeps whole inter-note gaps); pause/focus states are still published
by the loop itself. Direct (non-threaded) mode always runs polled.

Test seam: deterministic tests inject a `HybridWaitStrategy` subclass whose `spin_until_us`
advances their fake clock (`wait_strategy` parameter on `PlaybackEngine`). Production code never
special-cases fake clocks.

## 5. Dispatch-thread priority ladder

`DispatchThreadPriorityScope(mode)` applied on the dispatch thread, reverted on exit:
`auto` tries MMCSS (`AvSetMmThreadCharacteristicsW`: "Pro Audio" → "Low Latency" → "Audio" →
"Games", plus `AvSetMmThreadPriority(HIGH)`) then `SetThreadPriority` TIME_CRITICAL → HIGHEST →
off. Explicit modes pin one rung. **Never a process priority class** (user mandate). The acquired
tier is recorded in telemetry `runtime_options.rt_priority_acquired`.

## 6. Production defaults & kill switches

All graduated ON (config/RUNTIME_STATE layer; library/engine constructor defaults stay off so
deterministic tests are unaffected):

| Feature | Default | Kill switch |
|---|---|---|
| MMCSS/priority ladder | `rt_priority_mode: auto` (config) | `--rt-priority-mode off` |
| Adaptive dispatch lead | `enable_adaptive_lead: true` (config) | `--no-adaptive-lead` |
| Adaptive spin threshold | `enable_adaptive_spin: true` (config) | `--no-adaptive-spin` |
| Event-driven waits | on (runtime) | `--no-event-wait` |
| GIL switch interval 1 ms | on | `--no-switch-interval-tuning` |
| GC pause, timer guard, waitable timer, dispatch thread | on (pre-existing) | `--no-gc-pause`, `--no-timer-guard`, `--no-waitable-timer`, `--no-dispatch-thread` |

Note: the legacy `rt_time_critical` config key was dead and is ignored/dropped; only
`rt_priority_mode` matters. Long-running app instances re-save their in-memory config on exit —
close old instances before editing `config.json` by hand.

## 7. Live A/B evidence (blue @ local-precise/144, 2026-06-11)

| Arm | down visible_lateness p50/p99/max (µs) | lateness p99 | drops |
|---|---|---|---|
| Control ×2 | +420 / 1781 / 5740 and +407 / 2076 / 7697 | 553 / 1446 | 0 |
| Adaptive lead | **−3** / 760 / 2915 | 561 | 0 |
| Event wait | +401 / **1026** / 4408 | **104** | 0 |
| Adaptive spin | +409 / 1029 / **2458** | **80** | 0 |

Control showed 492/492 releases floor-deferred (correct at 1-frame zero margin); with lead only 8
needed deferral and the floor mechanism remained active — holds never dropped below 1 frame.

Post-graduation validation (run 075636, full production stack incl. `mmcss:Pro Audio`, 84-note
song): down-onset visible p50 **+16 µs**, zero drops, 8 floor-deferrals, send max 1.8 ms — the
cleanest send tail of all runs.
