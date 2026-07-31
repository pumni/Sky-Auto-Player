# RT Dispatch Architecture (Python oracle + native Rust worker)

Status: CURRENT — Rust owns the eligible native real-time sender path; Python remains the
deterministic oracle and rollback path.
History: built by `archive/2026-06_rt-pipeline-extreme-optimization-plan.md`; A/B numbers in
`perf-baselines/2026-06-baseline.md`.

## 1. The single ground truth

The game registers a key press iff the key is observed held for **at least 1 game frame**
(completion-to-completion). Every mechanism below is subordinate to that invariant; see
`timing-principles.md` §0/§3.

## 2. Component map

```
PlaybackEngine (orchestration/engine.py)            facade: wiring + lifecycle only
 ├─ native_dispatch.RustDispatchRuntime              single Python/native composition seam
 │   └─ sky_player_rs.DispatchSession                worker owns core/wait/backend/telemetry
 ├─ Python rollback/oracle path:
 │   ├─ compile_runtime_intents → RuntimeSchedule    per-key generations (core/coordinator.py)
 ├─ SendLatencyEstimator                             per-kind EMA of SendInput durations
 ├─ wake-error probe (pre start_perf)                derives effective spin threshold
 ├─ RealtimeProcessScope (infrastructure/realtime)   gc.collect→gc.disable + setswitchinterval(1ms)
 ├─ DispatchLoop (orchestration/core/loop.py)        wait → drain → execute; RT thread only
 │   ├─ RuntimeDispatchCoordinator (core/coordinator.py)  active/pending state machine, floors, guards
 │   ├─ PlaybackState (core/state.py)                single-interval pause SM + display snapshot
 │   ├─ HybridWaitStrategy (infrastructure/wait_strategy.py)
 │   ├─ DispatchHealthMonitor                        focus cache, backend-health cache, input-path p95
 │   └─ InputBackend → send_scan_code_batch_trusted  cached INPUT arrays → user32.SendInput
 │   └─ PlaybackSupervisor (orchestration/playback_supervisor.py)  control thread:
     command queue + command event, focus polling, progress consumption/publishing,
     DispatchThreadPriorityScope (infrastructure/rt_priority.py) on the dispatch thread
```

Threading contract: the dispatch thread owns the backend and all timing; the supervisor (control)
thread owns controls/focus/rendering and must never call into the backend
(`test_threaded_dispatch_keeps_all_backend_calls_on_dispatch_thread`).

### 2.1 The `orchestration/core/` package (the isolated dispatch seam)

The Python reference core lives in `orchestration/core/`; the native worker replaces this seam
by default on the eligible real Windows path. `orchestration/dispatch_loop.py` and
`orchestration/runtime_dispatch.py` remain thin re-export shims and the differential oracle.
The native adapter must not duplicate coordinator state, calculate deadlines, or call SendInput.

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
Cross-thread display reads go through `elapsed_snapshot_us()` — protected by a documented synchronization primitive (threading.Lock) to ensure free-threaded safety without tearing.

### 2.2 Focus-check ownership (who calls what, at what cadence)

| Caller | Check | Cadence |
|---|---|---|
| Supervisor periodic sample; polled pause gate; `run()`-entry; `engine.play()` pre-start wait | **Full** `FocusGuard.is_active()` — `GetForegroundWindow` + `GetWindowThreadProcessId` + `OpenProcess` + process-name validation | 20–50 ms (human-facing) |
| `DispatchLoop` Phase-2 pre-down gate | shared runtime `FocusSignal` (`SharedFocusSignal`, sampled by the supervisor) **plus**, in threaded mode, a fresh injected cheap HWND-only recheck (`is_foreground_cached_hwnd`: `GetForegroundWindow()==sky`, no `OpenProcess`) | every down batch (including the first note) |
| `DispatchHealthMonitor.focus_is_active` (post-send diagnostic) | runtime `FocusSignal` if set, else injected cheap HWND-only probe (`is_foreground_cached_hwnd`) — no process lookup | 2 ms TTL |

Neither dispatch worker issues the full process-name check; a live HWND cannot change the process
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
- **Wake-error probe** (`enable_adaptive_spin`): 30 × 2 ms probe sleeps run strictly *before*
  `start_perf` (same rule as `gc.collect`), deriving
  `effective_spin_threshold = clamp(spin_floor_us, 3000, p95_wake_error + 200)` µs (default
  `spin_floor_us = 700`, cap 3000 µs). The probe also retains p50/p99/max diagnostics. A later
  reprobe uses a robust `median + 6 × MAD + 200 µs` candidate over its small cooperative sample,
  raises the threshold immediately, and lowers it by at most 50 µs per update. This keeps one timer
  outlier from forcing a 3 ms spin window while retaining fast protection when the timer path degrades.
- **Cross-session EMA lead cache (Phase D):** `SendLatencyEstimator` exports/imports per-kind
  EMA state via `.cache/lead_estimator.json` so the first note benefits from the last session's
  warm lead. Corrupt/version-mismatched cache is silently dropped. Loaded flag recorded in
  `runtime_options.lead_cache_loaded`.
- **Idle-gap core warmup (Phase 1/6):** When the gap since last `SendInput` completion ≥ 20 ms, a
  `core_warmup_budget_us` (default 200 µs, capped at 500 µs) is added directly to the
  `effective_spin_threshold` for the final precision wait. This expands the busy-spin window right
  before the deadline to warm the CPU core without adding a separate blocking sleep cycle.
- **Mid-song spin re-probe (Phase H):** During inter-note gaps ≥ 0.5 s, if ≥ 30 s have elapsed
  since the last reprobe, the dispatch thread starts an eight-sample cooperative attempt. It takes
  at most one 2 ms sample per outer wait iteration and services command/focus state between
  samples; pause, focus, stop, or an unsafe deadline discards the partial attempt. A candidate is
  committed only after all eight samples using robust `median + 6 × MAD + 200 µs`, floor/cap and
  asymmetric hysteresis. Kill switch: `enable_spin_reprobe`
  (auto-off when `enable_adaptive_spin = False`). Applied thresholds are recorded in
  `runtime_options.reprobe_applied_thresholds`.

**Adaptive pre-play probe context.** In threaded playback, the 30-sample, 2 ms wake-error probe
runs on the dispatch thread after the timer and priority scopes have entered and before the final
epoch rebase. The result is applied to the loop before its first wait. Direct playback probes in
its execution context before creating the playback anchor. A probe failure preserves the configured
threshold and records the degradation; it does not abort playback. `p95 + 200 µs`, the configured
floor/cap, and the existing kill switch remain unchanged.

**Calibration evidence boundary.** The latency calibration cache is an
`injected_raw_input_delivery_proxy`: the app injects through Windows `SendInput` into an app-owned
window and observes its `WM_INPUT` delivery. It does not measure Sky process polling, render-frame
timing, or audio onset. `sampled_at` is UTC metadata and `evidence_kind` identifies this boundary;
no freshness TTL is applied by the loader.

## 4. Wait strategy

`HybridWaitStrategy.wait_until_us` picks, in order:
1. `remaining ≤ spin_threshold` → busy-spin to target.
2. Sleeper declares `is_high_resolution` (capability flag, e.g. `WaitableTimerSleeper`):
   - event mode (command event handle present): arm the waitable timer for
     `remaining − guard` and block in `WaitForMultipleObjects(command_event, timer)` — zero
     polling; commands/focus transitions wake the thread instantly. If both handles are ready,
     the command handle is selected first; then the loop re-polls before entering the spin guard.
   - polled mode: 2 ms-capped sleeps towards `target − guard` so the loop can poll between steps.
3. Fallback ladder (RealSleeper): coarse (≤20 ms, −5 ms buffer) → 1 ms ticks → yield → spin. (In event mode, the degraded 2ms polled sleep still monitors the command event to ensure prompt wake).

Polling is governed by the *presence* of the command event, not a flag: the supervisor creates the
event before the dispatch thread starts (so no early command can lose its wake-up) and signals it
on commands and focus transitions. In event mode the supervisor also publishes the periodic
"playing" progress (the loop sleeps whole inter-note gaps); pause/focus states are still published
by the loop itself. Direct (non-threaded) mode always runs polled.

The Rust equivalent is `sky_dispatch_win32::wait::HybridWaiter`. It creates the event before the
worker starts, orders `[command_event, timer]`, uses raw QPC ticks in the final wait/spin loop,
and keeps handle ownership in RAII wrappers. Microseconds are used only at the coordinator and
telemetry boundary. Its adaptive probe runs after timer/priority acquisition and before the
playback epoch. If the high-resolution timer is unavailable or disabled, a worker-owned 1 ms
timer-resolution guard is acquired and restored; the effective wait mode is exposed as
`runtime_options.wait_strategy_acquired`. The Python strategy remains the deterministic oracle
and rollback path.

Test seam: deterministic tests inject a `HybridWaitStrategy` subclass whose `spin_until_us`
advances their fake clock (`wait_strategy` parameter on `PlaybackEngine`). Production code never
special-cases fake clocks.

## 5. Dispatch-thread priority ladder

`DispatchThreadPriorityScope(mode)` applied on the dispatch thread, reverted on exit:
`auto` tries MMCSS (`AvSetMmThreadCharacteristicsW`: "Pro Audio" → "Low Latency" → "Audio" →
"Games", plus `AvSetMmThreadPriority(HIGH)`) then `SetThreadPriority` HIGHEST →
off. (`TIME_CRITICAL` is strictly an explicit expert mode, never an auto fallback). **Never a process priority class** (user mandate). The acquired
tier is recorded in telemetry `runtime_options.rt_priority_acquired`.
The Rust `MmcssGuard::acquire` implements the same per-thread ladder and restoration; it never
changes process priority or affinity. A separate worker-owned RAII guard disables per-thread
EcoQoS/execution-speed throttling on supported Windows versions and restores the prior state at
worker teardown.

### 5.1 Native rollout boundary

The native implementation is default on only for the real Windows backend with the production
clock/sleeper/thread mode. Dry-run, fake-clock tests, and explicit
`SKY_USE_PYTHON_DISPATCH=1` use the Python oracle. Missing or incompatible native metadata falls
back to Python and records a runtime warning; `SKY_REQUIRE_RUST_DISPATCH=1` is the fail-closed
validation switch. The Python dispatcher remains available as the rollback path.

## 6. Production defaults & kill switches

All graduated ON (config/RUNTIME_STATE layer; library/engine constructor defaults stay off so
deterministic tests are unaffected):

| Feature | Default | Kill switch |
|---|---|---|
| MMCSS/priority ladder | `rt_priority_mode: auto` (config) | `--rt-priority-mode off` |
| Adaptive dispatch lead | `enable_adaptive_lead: true` (config) | `--no-adaptive-lead` |
| Lead EMA cross-session cache | `.cache/lead_estimator.json` | `lead_cache_path = None` |
| Adaptive spin threshold | `enable_adaptive_spin: true` (config) | `--no-adaptive-spin` |
| Mid-song spin re-probe | `enable_spin_reprobe: true` when adaptive spin on | set `enable_adaptive_spin: false` |
| Idle-gap core warmup | `core_warmup_budget_us = 200`, threshold 20 ms | `core_warmup_budget_us = 0` |
| Device margin | `.cache/input_latency.json` → `min_hold_margin_us`; default 500 | profile override |
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

## 8. Telemetry and Memory Hygiene

To protect the dispatch hot path, the dispatch thread never synchronously writes telemetry files to disk or allocates unbounded arrays. If `TelemetryLogger` reaches its hard record cap (`_TELEMETRY_MAX_BUFFER`), it retains the first records, stops accepting new records, and increments exact truncation/drop markers. Final CSV export is performed off the dispatch hot path during playback lifecycle teardown.

The Rust path reserves its retain-first record buffer before the playback epoch. The compiled
schedule uses a flat `CompactIntent` arena plus small batch headers; only the current batch is
materialized into full intent views. Runtime active/release ownership uses 15-key arrays and
bitmasks rather than hash tables. Per-record scan/generation fields use inline storage and
reasons remain compact IDs on the worker; bounded summary counters and 50 µs histograms remain
available in every native run, including summary-only runs where the retain-first event buffer is
disabled. Reason strings and JSON are materialized only after terminal join. This preserves the
existing CSV fields and outcome strings without per-send disk I/O while making production health
histograms available without retaining every event.

**Prewarm observability.** Before a threaded playback dispatch loop starts, the
engine prewarms the platform INPUT cache in two passes under the cache cap
(imported from `inputs.ARRAY_CACHE_MAX`). The first pass reserves singleton-up
slots for every distinct scan code in the schedule — singleton releases are the
common case on the dispatch hot path (a held-down chord resolves per-down,
respectively multitimbral up keyups also express as singletons) and must not be
admitted only opportunistically. The second pass then admits authored multi-key
shapes by descending frequency (`Counter` of the schedule's shape histogram),
up to the remaining budget. This keeps the platform cache's bounded lifecycle
counters — unique down/up shapes, prewarmed `INPUT` slots, approximate payload
bytes, schedule shape frequency, prewarm duration, lazy cache misses/first-hit
build duration, and entries/slots cleared at teardown — meaningful as admission
diagnostics. The counters remain diagnostic only; they do not impose a payload
budget or change cache representation. Working-set and RSS claims remain
benchmark evidence, not an inference from cache-entry or `gc.collect()` counts.

**Windows acceptance benchmark.** Run `scripts/bench_native_acceptance.py` on the
baseline and follow-up revisions with the same `--actions`, `--repeats`, power state,
priority mode, and background-load conditions. The report must retain sender-side
completion-error p50/p95/p99/max, spin CPU time, peak RSS, command-interrupt latency,
drop/release counters, and outcome. This mock-backend harness does not measure game
sampling, frame observation, audio onset, or real `SendInput` delivery; those remain
separate Windows test-window evidence gates.
