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
 ├─ compile_runtime_intents → RuntimeSchedule        per-key generations (core/coordinator.py)
 ├─ SendLatencyEstimator                             per-kind EMA of SendInput durations
 ├─ wake-error probe (pre start_perf)                derives effective spin threshold
 ├─ RealtimeProcessScope (infrastructure/realtime)   gc.collect→gc.disable + setswitchinterval(1ms)
 ├─ DispatchLoop (orchestration/core/loop.py)        wait → drain → execute; RT thread only
 │   ├─ RuntimeDispatchCoordinator (core/coordinator.py)  active/pending state machine, floors, guards
 │   ├─ PlaybackState (core/state.py)                single-interval pause SM + display snapshot
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

### 2.1 The `orchestration/core/` package (the isolated dispatch seam)

The real-time core lives in `orchestration/core/` — a platform-free package that is the exact
seam the future Rust worker (`rust-migration-plan.md`) replaces. `orchestration/dispatch_loop.py`
and `orchestration/runtime_dispatch.py` remain as thin re-export shims for backward compatibility.

- `core/loop.py` — `DispatchLoop` + `DispatchHealthMonitor`.
- `core/coordinator.py` — `RuntimeDispatchCoordinator` (schedule → batches, generation tracking).
- `core/state.py` — `PlaybackState`.
- `core/ports.py` — the typed Protocols the core depends on: `InputBackend`, `Clock`, `Sleeper`,
  `WaitStrategy`, `CommandSource`, `FocusSignal`, `FocusController`, `ProgressSink`,
  `LeadEstimator`, plus the `PlaybackCommand` StrEnum.

**Boundary rule (enforced by `tests/test_core_boundary.py`):** no module under `core/` imports
`sky_music.platform.*`, `sky_music.ui.*`, `sky_music.infrastructure.focus`, or
`sky_music.orchestration.engine`. Platform access is injected as ports/hooks: the engine wires a
cheap foreground-HWND probe (`cheap_focus_probe`), a diagnostics `debug_log` hook
(`diagnostics_log`), and the unfocused-send counter (`unfocused_send_hook`) at loop construction.

**Pause state machine (`PlaybackState`).** One contiguous-interval owner: a `pause_reasons` set
(`{"manual","focus"}`) + one anchor (`pause_interval_started_us`). Entering pause from an empty set
captures the anchor; a second concurrent reason does not move it; only the last exiting reason
accumulates the interval into `pause_time_us` exactly once, attributed to the first reason that
opened it. This replaced the old dual-anchor model that double-counted overlap and made elapsed run
backwards. Cross-thread display reads go through `elapsed_snapshot_us()` — a single atomically
reassigned `(epoch_us, pause_anchor, paused)` tuple (single-writer dispatch thread; readers never
tear).

### 2.2 Focus-check ownership (who calls what, at what cadence)

| Caller | Check | Cadence |
|---|---|---|
| Supervisor periodic sample; polled pause gate; `run()`-entry; `engine.play()` pre-start wait | **Full** `FocusGuard.is_active()` — `GetForegroundWindow` + `GetWindowThreadProcessId` + `OpenProcess` + process-name validation | 20–50 ms (human-facing) |
| `DispatchLoop` Phase-2 pre-down gate | shared runtime `FocusSignal` (`SharedFocusSignal`, sampled by the supervisor) **plus**, in threaded mode, a fresh injected cheap HWND-only recheck (`is_foreground_cached_hwnd`: `GetForegroundWindow()==sky`, no `OpenProcess`) | per down batch |
| `DispatchHealthMonitor.focus_is_active` (post-send diagnostic) | runtime `FocusSignal` if set, else injected cheap HWND-only probe (`is_foreground_cached_hwnd`) — no process lookup | 2 ms TTL |

The dispatch thread never issues the full process-name check; a live HWND cannot change the process
behind it, so the cheap compare is safe and staleness is bounded by the full checks' 20–50 ms cadence.
The pre-down gate's fresh HWND recheck (wired only in threaded mode, where the `SharedFocusSignal`
is 20–50 ms stale) closes the alt-tab race in which a down would inject into the window the user just
switched to; it short-circuits so the one `GetForegroundWindow` call runs only when the cheap signal
already says active. In direct mode the gate's `DirectFocusSignal` already wraps the authoritative
`FocusGuard.is_active()` fresh on every down, so no extra probe is wired.

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
